use super::message::Message;
use serde::{Deserialize, Deserializer, Serialize};

fn default_alarm_on() -> String {
    "raised".to_string()
}

/// One data source id or a list of them.
fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<i64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let one = |v: &serde_json::Value| -> Option<i64> {
        v.as_i64()
            .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    };
    let err = || serde::de::Error::custom("data source ids are numbers");
    match &v {
        serde_json::Value::Array(items) => items.iter().map(|i| one(i).ok_or_else(err)).collect(),
        serde_json::Value::Null => Ok(Vec::new()),
        other => one(other).map(|i| vec![i]).ok_or_else(err),
    }
}

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
    /// A tag (data source) of any protocol changed by more than `deadband`.
    #[serde(rename = "tag_change")]
    TagChange {
        #[serde(deserialize_with = "one_or_many", alias = "data_source_id")]
        data_source_ids: Vec<i64>,
        #[serde(default)]
        deadband: Option<f64>,
    },
    /// An alarm was raised, cleared or acknowledged (`on`: one of those or `any`).
    #[serde(rename = "alarm")]
    Alarm {
        #[serde(default = "default_alarm_on")]
        on: String,
        #[serde(default)]
        severity: Option<String>,
        #[serde(default)]
        source_key: Option<String>,
    },
    /// A cron expression in the server's local time.
    #[serde(rename = "schedule")]
    Schedule { cron: String },
    /// `POST /api/v1/flows/{id}/webhook` with the flow's token.
    #[serde(rename = "webhook")]
    Webhook {},
    /// Only the run button / `POST /api/v1/flows/{id}/run`.
    #[serde(rename = "manual")]
    Manual {},
}

/// Every trigger type, in catalogue order.
pub const TRIGGER_TYPES: &[&str] = &[
    "manual",
    "tag_change",
    "modbus",
    "mqtt",
    "alarm",
    "timer",
    "schedule",
    "webhook",
];

impl TriggerConfig {
    pub fn type_name(&self) -> &'static str {
        match self {
            TriggerConfig::Modbus { .. } => "modbus",
            TriggerConfig::Timer { .. } => "timer",
            TriggerConfig::Mqtt { .. } => "mqtt",
            TriggerConfig::TagChange { .. } => "tag_change",
            TriggerConfig::Alarm { .. } => "alarm",
            TriggerConfig::Schedule { .. } => "schedule",
            TriggerConfig::Webhook {} => "webhook",
            TriggerConfig::Manual {} => "manual",
        }
    }

    /// Parse and check a trigger (`trigger_config`, as a JSON value or the
    /// JSON text it is stored as).
    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let value = match value {
            serde_json::Value::String(s) => serde_json::from_str(&s)
                .map_err(|e| format!("trigger_config is not valid JSON: {e}"))?,
            v => v,
        };
        let ty = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        if !TRIGGER_TYPES.contains(&ty.as_str()) {
            return Err(format!(
                "unknown trigger type '{ty}' (expected one of {})",
                TRIGGER_TYPES.join(", ")
            ));
        }
        let trigger: Self =
            serde_json::from_value(value).map_err(|e| format!("trigger ({ty}): {e}"))?;
        trigger.validate()?;
        Ok(trigger)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            TriggerConfig::Timer { interval_ms } if *interval_ms == 0 => {
                Err("trigger (timer): interval_ms must be at least 1".into())
            }
            TriggerConfig::TagChange {
                data_source_ids,
                deadband,
            } => {
                if data_source_ids.is_empty() {
                    return Err("trigger (tag_change): pick at least one data source".into());
                }
                if deadband.is_some_and(|d| d.is_nan() || d < 0.0) {
                    return Err("trigger (tag_change): deadband must be 0 or more".into());
                }
                Ok(())
            }
            TriggerConfig::Alarm { on, severity, .. } => {
                if !matches!(on.as_str(), "raised" | "cleared" | "acknowledged" | "any") {
                    return Err(format!(
                        "trigger (alarm): on must be raised, cleared, acknowledged or any, not {on:?}"
                    ));
                }
                if let Some(s) = severity.as_deref().filter(|s| !s.is_empty())
                    && !matches!(s, "critical" | "warning" | "info")
                {
                    return Err(format!(
                        "trigger (alarm): severity must be critical, warning or info, not {s:?}"
                    ));
                }
                Ok(())
            }
            TriggerConfig::Schedule { cron } => super::cron::Cron::parse(cron)
                .map(|_| ())
                .map_err(|e| format!("trigger (schedule): {e}")),
            _ => Ok(()),
        }
    }

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
            TriggerConfig::Mqtt { topic, .. } => {
                if msg.metadata.get("source") != Some(&"mqtt".to_string()) {
                    return false;
                }
                let msg_topic = msg.metadata.get("topic").map(String::as_str).unwrap_or("");
                mqtt_topic_matches(topic, msg_topic)
            }
            // Fired by their own sources, not by matching a device message.
            _ => false,
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
    fn new_triggers_parse_and_validate() {
        use serde_json::json;
        let ids = |t: &TriggerConfig| match t {
            TriggerConfig::TagChange {
                data_source_ids, ..
            } => data_source_ids.clone(),
            _ => panic!("not tag_change"),
        };
        let t = TriggerConfig::from_value(
            json!({"type": "tag_change", "data_source_ids": [3, 4], "deadband": 0.5}),
        )
        .unwrap();
        assert_eq!(ids(&t), [3, 4]);
        // A single id from a single-tag picker works too.
        let t =
            TriggerConfig::from_value(json!({"type": "tag_change", "data_source_ids": 7})).unwrap();
        assert_eq!(ids(&t), [7]);
        assert!(
            TriggerConfig::from_value(json!({"type": "tag_change", "data_source_ids": []}))
                .is_err()
        );

        TriggerConfig::from_value(json!({"type": "alarm", "on": "any", "severity": "critical"}))
            .unwrap();
        assert!(TriggerConfig::from_value(json!({"type": "alarm", "on": "sometimes"})).is_err());

        TriggerConfig::from_value(json!({"type": "schedule", "cron": "*/5 * * * *"})).unwrap();
        let err =
            TriggerConfig::from_value(json!({"type": "schedule", "cron": "nope"})).unwrap_err();
        assert!(err.contains("schedule"), "{err}");

        TriggerConfig::from_value(json!({"type": "webhook"})).unwrap();
        TriggerConfig::from_value(json!({"type": "manual"})).unwrap();
        // trigger_config as stored: JSON text.
        TriggerConfig::from_value(json!(r#"{"type":"manual"}"#)).unwrap();
        let err = TriggerConfig::from_value(json!({"type": "teleport"})).unwrap_err();
        assert!(err.contains("unknown trigger type"), "{err}");
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
