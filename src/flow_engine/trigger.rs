use super::message::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum TriggerConfig {
    #[serde(rename = "modbus")]
    Modbus {
        device_id: String,
        metric: Option<String>,
        #[serde(default)]
        debounce_ms: Option<u64>,
        #[serde(default)]
        on_change_only: Option<bool>,
    },
    #[serde(rename = "timer")]
    Timer { interval_ms: u64 },
    #[serde(rename = "mqtt")]
    Mqtt {
        topic: String,
        #[serde(default)]
        debounce_ms: Option<u64>,
    },
}

impl TriggerConfig {
    pub fn matches(&self, msg: &Message) -> bool {
        match self {
            TriggerConfig::Modbus {
                device_id, metric, ..
            } => {
                if msg.metadata.get("source") != Some(&"modbus".to_string()) {
                    return false;
                }
                if device_id != "*" {
                    let payload_device_id = msg
                        .payload
                        .get("device_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if payload_device_id != device_id {
                        return false;
                    }
                }
                if let Some(metric_name) = metric.as_deref().filter(|s| !s.is_empty()) {
                    if let Some(metrics) = msg.payload.get("metrics") {
                        if metrics.get(metric_name).is_none() {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            }
            TriggerConfig::Timer { .. } => false,
            TriggerConfig::Mqtt { topic, .. } => {
                if msg.metadata.get("source") != Some(&"mqtt".to_string()) {
                    return false;
                }
                let msg_topic = msg.metadata.get("topic").map(String::as_str).unwrap_or("");
                mqtt_topic_matches(topic, msg_topic)
            }
        }
    }
}

/// MQTT topic-filter match supporting the standard wildcards:
/// - `+` matches exactly one level
/// - `#` matches zero or more remaining levels and must be the final level
///
/// Plain filters match exactly (including an empty filter matching only the
/// empty topic). Mirrors the broker's own subscription filtering.
pub fn mqtt_topic_matches(filter: &str, topic: &str) -> bool {
    let f: Vec<&str> = filter.split('/').collect();
    let t: Vec<&str> = topic.split('/').collect();
    let last = f.len() - 1;
    for (i, fp) in f.iter().enumerate() {
        // '#' is only valid as the final level (MQTT spec). A '#' appearing
        // mid-filter is treated as a literal level, which a normal topic never
        // contains, so such (invalid) filters match nothing.
        if *fp == "#" && i == last {
            return true;
        }
        match t.get(i) {
            None => return false,
            Some(_) if *fp == "+" => continue,
            Some(tp) if tp == fp => continue,
            _ => return false,
        }
    }
    f.len() == t.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::message::Message;

    fn mqtt_msg(topic: &str) -> Message {
        let payload = serde_json::json!({"value": 1});
        let mut m = Message::new(payload);
        m.metadata.insert("source".to_string(), "mqtt".to_string());
        m.metadata.insert("topic".to_string(), topic.to_string());
        m
    }

    #[test]
    fn wildcard_exact() {
        assert!(mqtt_topic_matches("sensors/temp", "sensors/temp"));
        assert!(!mqtt_topic_matches("sensors/temp", "sensors/hum"));
    }

    #[test]
    fn wildcard_single_level() {
        assert!(mqtt_topic_matches("sensors/+/temp", "sensors/01/temp"));
        assert!(mqtt_topic_matches("sensors/+/temp", "sensors/any/temp"));
        assert!(!mqtt_topic_matches("sensors/+/temp", "sensors/01/02/temp"));
        assert!(!mqtt_topic_matches("sensors/+/temp", "sensors/temp"));
    }

    #[test]
    fn wildcard_multi_level() {
        assert!(mqtt_topic_matches("sensors/#", "sensors"));
        assert!(mqtt_topic_matches("sensors/#", "sensors/a"));
        assert!(mqtt_topic_matches("sensors/#", "sensors/a/b/c"));
        assert!(mqtt_topic_matches("#", "anything/here"));
        // '#' only valid as the final level; a filter like 'a/#/b' is treated
        // literally (no '#' special handling mid-filter), so it won't match.
        assert!(!mqtt_topic_matches("a/#/b", "a/x/b"));
    }

    #[test]
    fn mqtt_trigger_matches() {
        let trig = TriggerConfig::Mqtt {
            topic: "sensors/+/temp".to_string(),
            debounce_ms: None,
        };
        assert!(trig.matches(&mqtt_msg("sensors/01/temp")));
        assert!(!trig.matches(&mqtt_msg("sensors/01/hum")));
        // Non-MQTT source never matches an MQTT trigger.
        let mut other = Message::new(serde_json::json!({}));
        other
            .metadata
            .insert("source".to_string(), "modbus".to_string());
        other
            .metadata
            .insert("topic".to_string(), "sensors/01/temp".to_string());
        assert!(!trig.matches(&other));
    }
}
