use anyhow::Result;
use serde::Serialize;

/// Data sent as JSON payload in a webhook notification.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    pub severity: String,
    pub source_key: String,
    pub source_name: String,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
    pub triggered_at: String,
    pub device_info: String,
}

/// Send a webhook alarm notification to the given URL.
///
/// Retries with exponential backoff, same pattern as the HttpOut flow node.
pub async fn send_webhook_alarm(url: &str, payload: &WebhookPayload) -> Result<()> {
    crate::security::outbound::check_url(url).map_err(anyhow::Error::msg)?;
    let client = crate::security::outbound::client(None);
    // Logs and errors (which reach flow runs and the notification test) name
    // the URL without its password, and reqwest errors without their URL.
    let target = url;
    let url = crate::security::redact::mask_url(target);
    let url = url.as_str();
    let max_retries: u32 = 3;

    for attempt in 0..=max_retries {
        let resp = client
            .post(target)
            .header("Content-Type", "application/json")
            .json(payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match resp {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(
                    "Webhook notification sent to {} for {} [{}]",
                    url,
                    payload.source_name,
                    payload.severity,
                );
                return Ok(());
            }
            Ok(resp) => {
                let status = resp.status();
                if attempt < max_retries {
                    let delay = 500 * 2u64.pow(attempt);
                    tracing::warn!(
                        "Webhook to {} returned {}, retrying in {}ms (attempt {}/{})",
                        url,
                        status,
                        delay,
                        attempt + 1,
                        max_retries + 1,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                } else {
                    anyhow::bail!(
                        "Webhook to {} failed with status {} after {} attempts",
                        url,
                        status,
                        max_retries + 1,
                    );
                }
            }
            Err(e) => {
                let e = e.without_url();
                if attempt < max_retries {
                    let delay = 500 * 2u64.pow(attempt);
                    tracing::warn!(
                        "Webhook to {} error: {}, retrying in {}ms (attempt {}/{})",
                        url,
                        e,
                        delay,
                        attempt + 1,
                        max_retries + 1,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                } else {
                    anyhow::bail!(
                        "Webhook to {} failed after {} attempts: {}",
                        url,
                        max_retries + 1,
                        e,
                    );
                }
            }
        }
    }

    Ok(())
}

/// Format an epoch timestamp into a human-readable string using the `time` crate.
pub fn format_triggered_at(epoch_secs: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(epoch_secs) {
        Ok(dt) => dt
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| format!("{} (UTC)", epoch_secs)),
        Err(_) => format!("{} (UTC)", epoch_secs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_webhook_never_shows_the_url_password() {
        let url = format!("http://user:{}@unresolvable.invalid/hook", "topsecretpw");
        let payload = WebhookPayload {
            severity: "warning".into(),
            source_key: "k".into(),
            source_name: "n".into(),
            value: 1.0,
            threshold: 0.0,
            message: "m".into(),
            triggered_at: "t".into(),
            device_info: "d".into(),
        };
        let err = send_webhook_alarm(&url, &payload).await.unwrap_err();
        let shown = format!("{err:#}");
        assert!(shown.contains("unresolvable.invalid"), "{shown}");
        assert!(!shown.contains("topsecretpw"), "{shown}");
    }

    #[test]
    fn test_format_triggered_at_valid() {
        let formatted = format_triggered_at(1710000000);
        assert!(formatted.contains("2024"));
    }

    #[test]
    fn test_format_triggered_at_invalid() {
        let formatted = format_triggered_at(-1);
        assert!(formatted.contains("-1"));
    }

    #[test]
    fn test_webhook_payload_serialization() {
        let payload = WebhookPayload {
            severity: "critical".to_string(),
            source_key: "plts_1:voltage".to_string(),
            source_name: "PLTS 1 Voltage".to_string(),
            value: 250.5,
            threshold: 240.0,
            message: "PLTS 1 Voltage reached 250.5V".to_string(),
            triggered_at: "2024-03-09T20:00:00Z".to_string(),
            device_info: "plts_1".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"severity\":\"critical\""));
        assert!(json.contains("\"source_key\":\"plts_1:voltage\""));
        assert!(json.contains("\"value\":250.5"));
    }
}
