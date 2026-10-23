use serde::Serialize;

/// JSON payload sent to the browser push service, then displayed by the
/// service worker as a notification.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct AlarmPayload {
    pub title: String,
    pub body: String,
    pub severity: String,
    pub source_key: String,
}

/// Build an `AlarmPayload` from alarm fields.
pub fn build_alarm_payload(
    source_name: &str,
    severity: &str,
    message: &str,
    source_key: &str,
) -> AlarmPayload {
    AlarmPayload {
        title: format!("[{}] {}", severity.to_uppercase(), source_name),
        body: message.to_string(),
        severity: severity.to_string(),
        source_key: source_key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_alarm_payload_critical() {
        let payload = build_alarm_payload(
            "PLTS 1 Voltage",
            "critical",
            "PLTS 1 Voltage reached 250.5V (critical: 240.0V)",
            "plts_1:voltage",
        );

        assert_eq!(payload.title, "[CRITICAL] PLTS 1 Voltage");
        assert_eq!(
            payload.body,
            "PLTS 1 Voltage reached 250.5V (critical: 240.0V)"
        );
        assert_eq!(payload.severity, "critical");
        assert_eq!(payload.source_key, "plts_1:voltage");
    }

    #[test]
    fn test_build_alarm_payload_warning() {
        let payload = build_alarm_payload(
            "Generator Temp",
            "warning",
            "Generator Temp reached 85.0C (warning: 80.0C)",
            "gen_1:temp",
        );

        assert_eq!(payload.title, "[WARNING] Generator Temp");
        assert_eq!(payload.severity, "warning");
    }

    #[test]
    fn test_alarm_payload_serialization() {
        let payload = build_alarm_payload(
            "PLTS 1 Voltage",
            "critical",
            "Voltage high",
            "plts_1:voltage",
        );

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"title\":\"[CRITICAL] PLTS 1 Voltage\""));
        assert!(json.contains("\"severity\":\"critical\""));
        assert!(json.contains("\"source_key\":\"plts_1:voltage\""));
        assert!(json.contains("\"body\":\"Voltage high\""));

        let back: AlarmPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.title, payload.title);
        assert_eq!(back.severity, payload.severity);
    }

    #[test]
    fn test_alarm_payload_json_structure() {
        let payload = AlarmPayload {
            title: "Test".to_string(),
            body: "Body".to_string(),
            severity: "critical".to_string(),
            source_key: "key".to_string(),
        };

        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["title"], "Test");
        assert_eq!(json["body"], "Body");
        assert_eq!(json["severity"], "critical");
        assert_eq!(json["source_key"], "key");
        assert_eq!(json.as_object().unwrap().len(), 4);
    }
}
