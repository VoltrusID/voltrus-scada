use super::*;

impl CommandQueue {
    // -- Address parsing helpers --

    #[cfg(feature = "modbus")]
    pub(super) fn parse_modbus_address(source_config: &str) -> Option<u16> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        // Try "address" field first
        if let Some(addr) = config.get("address").and_then(|v| v.as_u64()) {
            let addr = addr as u16;
            // Convert 5-digit Modbus address to 0-based protocol address
            return Some(if addr >= 40001 {
                addr - 40001
            } else if addr >= 30001 {
                addr - 30001
            } else {
                addr
            });
        }
        // Try "register" field as fallback
        config.get("register").and_then(|v| v.as_u64()).map(|v| {
            let addr = v as u16;
            if addr >= 40001 {
                addr - 40001
            } else if addr >= 30001 {
                addr - 30001
            } else {
                addr
            }
        })
    }

    #[cfg(feature = "s7")]
    pub(super) fn parse_s7_address(source_config: &str) -> Option<(String, u32, u16)> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        let area = config
            .get("area")
            .and_then(|v| v.as_str())
            .unwrap_or("DB")
            .to_string();
        let db_number = config
            .get("db_number")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let offset = config.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
        Some((area, db_number, offset))
    }

    #[cfg(feature = "opcua")]
    pub(super) fn parse_opcua_node_id(source_config: &str) -> Option<String> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        config
            .get("node_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                config
                    .get("nodeId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    }

    #[cfg(feature = "ethernet-ip")]
    pub(super) fn parse_enip_tag_name(source_config: &str) -> Option<String> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        config
            .get("tag_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                config
                    .get("tagName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    }

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

    #[cfg(feature = "bacnet")]
    pub(super) fn parse_bacnet_object_info(source_config: &str) -> Option<(String, u32, String)> {
        let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
        let object_type = config
            .get("object_type")
            .and_then(|v| v.as_str())
            .unwrap_or("analogOutput")
            .to_string();
        let object_instance = config
            .get("object_instance")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let property = config
            .get("property")
            .and_then(|v| v.as_str())
            .unwrap_or("presentValue")
            .to_string();
        Some((object_type, object_instance, property))
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
