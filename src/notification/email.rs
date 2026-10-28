use anyhow::{Context, Result};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::header::ContentType,
    transport::smtp::authentication::Credentials,
};
use serde::Serialize;

use crate::models::SmtpConfig;

// ── Email data types ──

/// Data needed to render an alarm notification email.
#[derive(Debug, Clone, Serialize)]
pub struct AlarmEmailData {
    pub source_key: String,
    pub source_name: String,
    pub severity: String,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
    pub unit: String,
    pub triggered_at: String,
}

// ── Email notifier ──

/// SMTP-based email notifier with async transport and connection pooling.
///
/// The inner `AsyncSmtpTransport` is kept alive for connection reuse.
/// Construct with [`EmailNotifier::new`].
#[derive(Clone)]
pub struct EmailNotifier {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from_address: String,
    from_name: String,
}

impl EmailNotifier {
    /// Build a new `EmailNotifier` from the parsed `SmtpConfig`.
    pub fn new(config: &SmtpConfig) -> Result<Self> {
        let mailer = match config.encryption {
            crate::models::SmtpEncryption::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .with_context(|| {
                        format!("Failed to create STARTTLS transport for {}", config.host)
                    })?
                    .port(config.port)
                    .credentials(Credentials::new(
                        config.username.clone(),
                        config.password.clone(),
                    ))
                    .build()
            }
            crate::models::SmtpEncryption::ImplicitTls => {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                    .with_context(|| format!("Failed to create TLS transport for {}", config.host))?
                    .port(config.port)
                    .credentials(Credentials::new(
                        config.username.clone(),
                        config.password.clone(),
                    ))
                    .build()
            }
            crate::models::SmtpEncryption::None => {
                let addr = format!("{}:{}", config.host, config.port);
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&addr)
                    .credentials(Credentials::new(
                        config.username.clone(),
                        config.password.clone(),
                    ))
                    .build()
            }
        };

        Ok(Self {
            mailer,
            from_address: config.from_address.clone(),
            from_name: config.from_name.clone(),
        })
    }

    /// Send an alarm notification email to the given recipient.
    ///
    /// Uses multipart/alternative with both HTML and plaintext bodies.
    pub async fn send_alarm_email(&self, to: &str, alarm: &AlarmEmailData) -> Result<()> {
        let subject = format!(
            "[{}] {} alarm: {}",
            alarm.severity.to_uppercase(),
            alarm.source_name,
            alarm.message,
        );

        let html_body = render_alarm_html(alarm);
        let text_body = render_alarm_text(alarm);

        let from = format!("{} <{}>", self.from_name, self.from_address)
            .parse()
            .with_context(|| format!("Invalid from address: {}", self.from_address))?;

        let recipient = to
            .parse()
            .with_context(|| format!("Invalid recipient address: {}", to))?;

        let email = Message::builder()
            .from(from)
            .to(recipient)
            .subject(subject)
            .multipart(
                lettre::message::MultiPart::alternative()
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(text_body),
                    )
                    .singlepart(
                        lettre::message::SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html_body),
                    ),
            )
            .context("Failed to build email message")?;

        self.mailer
            .send(email)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send email: {}", e))?;

        tracing::info!(
            "Alarm email sent to {} for {} [{}]",
            to,
            alarm.source_name,
            alarm.severity,
        );

        Ok(())
    }
}

// ── Text template (plaintext fallback for multipart/alternative) ──

fn render_alarm_text(alarm: &AlarmEmailData) -> String {
    format!(
        "ALARM NOTIFICATION\n\
         =================\n\
         Source:    {source_name}\n\
         Severity:  {severity_upper}\n\
         Value:     {value}{unit}\n\
         Threshold: {threshold}{unit}\n\
         Message:   {message}\n\
         Triggered: {triggered_at}\n\
         \n\
         -- Voltrus Monitoring System",
        source_name = alarm.source_name,
        severity_upper = alarm.severity.to_uppercase(),
        value = alarm.value,
        unit = alarm.unit,
        threshold = alarm.threshold,
        message = alarm.message,
        triggered_at = alarm.triggered_at,
    )
}

// ── HTML template (inline, no external files) ──

fn render_alarm_html(alarm: &AlarmEmailData) -> String {
    let severity_color = match alarm.severity.to_lowercase().as_str() {
        "critical" => "#dc3545",
        "warning" => "#ffc107",
        _ => "#6c757d",
    };

    let triggered_at = &alarm.triggered_at;

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #f4f6f9; margin: 0; padding: 20px; }}
  .container {{ max-width: 600px; margin: 0 auto; background: #ffffff; border-radius: 8px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); overflow: hidden; }}
  .header {{ background: {severity_color}; color: #ffffff; padding: 20px 30px; }}
  .header h1 {{ margin: 0; font-size: 20px; }}
  .body {{ padding: 30px; }}
  .row {{ display: flex; margin-bottom: 12px; }}
  .label {{ font-weight: 600; min-width: 120px; color: #555; }}
  .value {{ color: #333; }}
  .footer {{ padding: 15px 30px; background: #f8f9fa; font-size: 12px; color: #999; text-align: center; }}
</style>
</head>
<body>
<div class="container">
  <div class="header">
    <h1>Alarm Notification</h1>
  </div>
  <div class="body">
    <div class="row">
      <span class="label">Source:</span>
      <span class="value">{source_name}</span>
    </div>
    <div class="row">
      <span class="label">Severity:</span>
      <span class="value" style="color: {severity_color}; font-weight: 600;">{severity_upper}</span>
    </div>
    <div class="row">
      <span class="label">Value:</span>
      <span class="value">{value}{unit}</span>
    </div>
    <div class="row">
      <span class="label">Threshold:</span>
      <span class="value">{threshold}{unit}</span>
    </div>
    <div class="row">
      <span class="label">Message:</span>
      <span class="value">{message}</span>
    </div>
    <div class="row">
      <span class="label">Triggered:</span>
      <span class="value">{triggered_at}</span>
    </div>
  </div>
  <div class="footer">
    Voltrus Monitoring System
  </div>
</div>
</body>
</html>"#,
        severity_color = severity_color,
        source_name = html_escape(&alarm.source_name),
        severity_upper = alarm.severity.to_uppercase(),
        value = alarm.value,
        unit = html_escape(&alarm.unit),
        threshold = alarm.threshold,
        message = html_escape(&alarm.message),
        triggered_at = triggered_at,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_alarm() -> AlarmEmailData {
        AlarmEmailData {
            source_key: "plts_1:voltage".to_string(),
            source_name: "PLTS 1 Voltage".to_string(),
            severity: "critical".to_string(),
            value: 250.5,
            threshold: 240.0,
            message: "PLTS 1 Voltage reached 250.5V (critical: 240.0V)".to_string(),
            unit: "V".to_string(),
            triggered_at: "2024-03-09T20:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_render_alarm_html_contains_key_fields() {
        let alarm = sample_alarm();
        let html = render_alarm_html(&alarm);

        assert!(html.contains("PLTS 1 Voltage"));
        assert!(html.contains("CRITICAL"));
        assert!(html.contains("250.5"));
        assert!(html.contains("240.0"));
        assert!(html.contains("Alarm Notification"));
    }

    #[test]
    fn test_render_alarm_html_uses_critical_color() {
        let alarm = sample_alarm();
        let html = render_alarm_html(&alarm);
        assert!(html.contains("#dc3545"));
    }

    #[test]
    fn test_render_alarm_html_uses_warning_color() {
        let mut alarm = sample_alarm();
        alarm.severity = "warning".to_string();
        let html = render_alarm_html(&alarm);
        assert!(html.contains("#ffc107"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("<script>alert('x')</script>"),
            "&lt;script&gt;alert('x')&lt;/script&gt;"
        );
        assert_eq!(
            html_escape("a & b < c > d \"e\""),
            "a &amp; b &lt; c &gt; d &quot;e&quot;"
        );
    }

    #[test]
    fn test_smtp_config_defaults() {
        let yaml = r#"
host: "smtp.example.com"
username: "user"
password: "pass"
"#;
        let config: SmtpConfig = serde_yaml_neo::from_str(yaml).unwrap();
        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.encryption, crate::models::SmtpEncryption::StartTls);
        assert_eq!(config.from_address, "voltrus@example.com");
        assert_eq!(config.from_name, "Voltrus");
    }

    #[test]
    fn test_smtp_encryption_variants() {
        let yaml = r#"
host: "smtp.example.com"
username: "user"
password: "pass"
encryption: "implicittls"
"#;
        let config: SmtpConfig = serde_yaml_neo::from_str(yaml).unwrap();
        assert_eq!(
            config.encryption,
            crate::models::SmtpEncryption::ImplicitTls
        );

        let yaml_none = r#"
host: "smtp.example.com"
username: "user"
password: "pass"
encryption: "none"
"#;
        let config_none: SmtpConfig = serde_yaml_neo::from_str(yaml_none).unwrap();
        assert_eq!(config_none.encryption, crate::models::SmtpEncryption::None);
    }

    #[test]
    fn test_render_alarm_text_contains_key_fields() {
        let alarm = sample_alarm();
        let text = render_alarm_text(&alarm);
        assert!(text.contains("PLTS 1 Voltage"));
        assert!(text.contains("CRITICAL"));
        assert!(text.contains("250.5"));
        assert!(text.contains("240.0"));
        assert!(text.contains("ALARM NOTIFICATION"));
    }
}
