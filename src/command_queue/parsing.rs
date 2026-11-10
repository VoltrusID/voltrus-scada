use super::*;

impl CommandQueue {
    // -- Address parsing helpers --

    pub(super) fn parse_ip_port(url: &str, default_port: u16) -> (String, u16) {
        // Strip protocol prefix if present
        let url = url
            .strip_prefix("tcp://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or(url);
        let parts: Vec<&str> = url.split(':').collect();
        let ip = parts[0].to_string();
        let port = parts
            .get(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        (ip, port)
    }

    #[cfg(feature = "dnp3")]
    pub(super) fn parse_dnp3_point_index(source_config: &str) -> Option<u16> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        config
            .get("point_index")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16)
            .or_else(|| {
                config
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u16)
            })
    }

    #[cfg(feature = "mqtt-broker")]
    pub(super) fn parse_mqtt_topic(source_config: &str) -> Option<String> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        config
            .get("topic")
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
            .map(|s| s.to_string())
    }

    /// Build the MQTT publish payload for a command value. If source_config has
    /// a `payload_template`, `{value}` is substituted; otherwise the raw decimal
    /// value string is published.
    #[cfg(feature = "mqtt-broker")]
    pub(super) fn mqtt_payload(source_config: &str, value: f64) -> String {
        let config: serde_json::Value = serde_json::from_str(source_config).unwrap_or_default();
        match config.get("payload_template").and_then(|v| v.as_str()) {
            Some(tpl) => tpl.replace("{value}", &value.to_string()),
            None => value.to_string(),
        }
    }
}
