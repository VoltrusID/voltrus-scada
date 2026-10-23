use crate::models::{ScadaConnection, ScadaEquipment};
use crate::storage::Storage;
use crate::wizard::feed::{FeedIO, FeedTemplate};
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashMap;

/// Instantiate a FEED template into the database.
/// Creates a device, data sources from the I/O list, a screen, and applies alarm thresholds.
pub struct FeedInstantiator<'a> {
    storage: &'a Storage,
    template: FeedTemplate,
    instance_name: String,
    /// Number of instances for multi-instance signal patterns (e.g. pumps 1..n)
    instance_counts: HashMap<String, u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FeedInstantiationResult {
    pub template_id: String,
    pub template_name: String,
    pub device_id: String,
    pub screen_id: String,
    pub tags_created: usize,
    pub alarms_created: usize,
    pub screens_created: usize,
    pub equipment_created: usize,
    pub connections_created: usize,
    pub tags: Vec<String>,
    pub alarms: Vec<String>,
}

impl<'a> FeedInstantiator<'a> {
    pub fn new(
        storage: &'a Storage,
        template: FeedTemplate,
        instance_name: String,
        instance_counts: HashMap<String, u32>,
    ) -> Self {
        Self {
            storage,
            template,
            instance_name,
            instance_counts,
        }
    }

    /// Execute the full instantiation
    pub async fn instantiate(&self) -> Result<FeedInstantiationResult> {
        let device_id = self.device_id();
        let mut created_tags = Vec::new();
        let mut created_alarms = Vec::new();
        let mut created_data_source_ids = Vec::new();

        // 1. Create device if it doesn't exist
        if !self.storage.device_exists(&device_id).await? {
            self.storage
                .create_device(
                    &device_id,
                    &format!("{} — {}", self.instance_name, self.template.name),
                    0,
                )
                .await
                .context("Failed to create device from FEED template")?;
        }

        // 2. Expand I/O list signal patterns and create data sources
        let expanded_ios = self.expand_io_patterns();
        let mut sort_order = 0i64;
        let mut existing_ds_keys: std::collections::HashSet<String> = self
            .storage
            .list_data_sources(Some(&device_id))
            .await?
            .iter()
            .map(|ds| ds.key.clone())
            .collect();
        for io in &expanded_ios {
            let resolved_signal = io.signal.clone();
            let key = format!("{}:{}", device_id, resolved_signal);

            // Check if data source already exists
            if existing_ds_keys.contains(&key) {
                continue;
            }

            let (min_val, max_val) = parse_range(&io.typical_range);
            let decimals = if io.data_type == "bool" { 0 } else { 2 };
            let chart_type = match io.io_type.as_str() {
                "DI" | "DO" => "led",
                _ => "gauge",
            };
            let base_val = (min_val + max_val) / 2.0;

            let ds_id = self
                .storage
                .create_data_source(
                    &key,
                    &format!("{} — {}", resolved_signal, io.description),
                    &io.unit,
                    "#f97316",
                    min_val,
                    max_val,
                    min_val, // warn_val — no alarm by default
                    max_val, // crit_val — no alarm by default
                    base_val,
                    sort_order,
                    decimals,
                    "simulation", // FEED templates are protocol-agnostic; simulation source
                    &json!({
                        "template_id": self.template.id,
                        "feed_template": true,
                        "io_type": io.io_type,
                        "data_type": io.data_type,
                        "equipment": io.equipment,
                        "criticality": io.criticality,
                    })
                    .to_string(),
                    &device_id,
                    Some(&io.equipment),
                    chart_type,
                )
                .await
                .context(format!("Failed to create data source: {}", key))?;

            created_data_source_ids.push(ds_id);
            created_tags.push(resolved_signal.clone());
            existing_ds_keys.insert(key);
            sort_order += 1;
        }

        // 3. Create screen from first recommended screen and assign data sources
        let screens_created =
            if let Some(screen) = self.template.dashboard.recommended_screens.first() {
                let screen_id = self.screen_id(screen);
                if !self.storage.device_exists(&screen_id).await? {
                    self.storage
                        .create_device(&screen_id, &screen.name, 1)
                        .await?;
                    for (i, &ds_id) in created_data_source_ids.iter().enumerate() {
                        self.storage
                            .add_screen_data_source(&screen_id, ds_id, i as i64)
                            .await
                            .context("Failed to assign data source to screen")?;
                    }
                }
                1
            } else {
                0
            };

        let screen_id = self
            .template
            .dashboard
            .recommended_screens
            .first()
            .map(|s| self.screen_id(s))
            .unwrap_or_default();

        // 3b. Generate SCADA equipment and connections from dashboard sections
        let mut equipment_created = 0usize;
        let mut connections_created = 0usize;

        if let Some(screen) = self.template.dashboard.recommended_screens.first() {
            let sid = self.screen_id(screen);
            let sections = &screen.sections;
            let mut equipment_ids: Vec<String> = Vec::new();

            for (idx, section) in sections.iter().enumerate() {
                // Expand signal patterns in section.signals to concrete signal names
                let resolved_signals = self.expand_section_signals(&section.signals);
                let sensor_bindings = json!(resolved_signals).to_string();

                let equipment_id = format!(
                    "eq_{}_{}",
                    section
                        .equipment_group
                        .to_lowercase()
                        .replace([' ', '/'], "_"),
                    idx
                );

                let x = (idx % 3) as f64 * 280.0;
                let y = (idx / 3) as f64 * 200.0;

                let equip = ScadaEquipment {
                    screen_id: sid.clone(),
                    equipment_id: equipment_id.clone(),
                    equipment_type: section.widget_type.clone(),
                    equipment_name: section.title.clone(),
                    x,
                    y,
                    width: 250.0,
                    height: 160.0,
                    rotation: 0.0,
                    sensor_bindings,
                    config: "{}".to_string(),
                };

                self.storage
                    .upsert_scada_equipment(&equip)
                    .await
                    .context("Failed to create SCADA equipment from FEED template")?;

                equipment_ids.push(equipment_id);
                equipment_created += 1;
            }

            // Create connections between sequential equipment (process flow)
            for i in 0..equipment_ids.len().saturating_sub(1) {
                let conn = ScadaConnection {
                    screen_id: sid.clone(),
                    connection_id: format!("conn_{}_{}", sid, i),
                    from_equipment: equipment_ids[i].clone(),
                    to_equipment: equipment_ids[i + 1].clone(),
                    waypoints: "[]".to_string(),
                    connection_type: "process".to_string(),
                };

                self.storage
                    .upsert_scada_connection(&conn)
                    .await
                    .context("Failed to create SCADA connection from FEED template")?;

                connections_created += 1;
            }
        }

        // 4. Apply alarm thresholds from alarm_strategy
        let expanded_alarm_rules = self.expand_alarm_patterns(&expanded_ios);
        let all_data_sources = self.storage.list_data_sources(None).await?;
        for rule in &expanded_alarm_rules {
            // Find matching data source
            if let Some(ds) = all_data_sources.iter().find(|ds| {
                ds.key.ends_with(&rule.signal) || ds.key.contains(&format!(":{}", rule.signal))
            }) {
                let threshold = parse_setpoint(&rule.setpoint);
                let update = match rule.condition.as_str() {
                    "high" | "high-high" | "low" | "low-low" => json!({
                        "crit_val": if rule.severity == "critical" { threshold } else { ds.crit_val },
                        "warn_val": if rule.severity == "major" || rule.severity == "warning" { threshold } else { ds.warn_val },
                    }),
                    "deviation" => json!({
                        "warn_val": if rule.severity == "major" || rule.severity == "warning" { threshold } else { ds.warn_val },
                    }),
                    _ => json!({}),
                };

                if !update.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                    self.storage
                        .update_data_source(ds.id, &update)
                        .await
                        .context("Failed to update alarm threshold")?;
                    created_alarms.push(format!(
                        "{}: {} {} @ {}",
                        rule.signal, rule.severity, rule.condition, rule.setpoint
                    ));
                }
            }
        }

        Ok(FeedInstantiationResult {
            template_id: self.template.id.clone(),
            template_name: self.template.name.clone(),
            device_id,
            screen_id,
            tags_created: created_tags.len(),
            alarms_created: created_alarms.len(),
            screens_created,
            equipment_created,
            connections_created,
            tags: created_tags,
            alarms: created_alarms,
        })
    }

    fn device_id(&self) -> String {
        format!(
            "feed_{}",
            self.instance_name.to_lowercase().replace(' ', "_")
        )
    }

    fn screen_id(&self, screen: &crate::wizard::feed::FeedScreen) -> String {
        format!(
            "feed_{}_{}",
            self.template.id,
            screen.name.to_lowercase().replace(' ', "_")
        )
    }

    /// Expand signal name patterns like "PMP_{n}_RUN" into concrete signal names
    fn expand_io_patterns(&self) -> Vec<FeedIO> {
        let mut result = Vec::new();
        for io in &self.template.io_list {
            let signal = &io.signal;
            if signal.contains("{n}") {
                let count = self.instance_counts.get("n").copied().unwrap_or(2);
                for i in 1..=count {
                    let mut expanded = io.clone();
                    expanded.signal = signal.replace("{n}", &i.to_string());
                    expanded.description = io.description.replace("{n}", &i.to_string());
                    result.push(expanded);
                }
            } else {
                result.push(io.clone());
            }
        }
        result
    }

    /// Expand alarm signal patterns to match expanded I/O signals
    fn expand_alarm_patterns(&self, expanded_ios: &[FeedIO]) -> Vec<ExpandedAlarmRule> {
        let mut result = Vec::new();
        for strategy in &self.template.alarm_strategy {
            for rule in &strategy.rules {
                let pattern = &rule.signal_pattern;
                if pattern.contains("{n}") {
                    let count = self.instance_counts.get("n").copied().unwrap_or(2);
                    for i in 1..=count {
                        let expanded_signal = pattern.replace("{n}", &i.to_string());
                        // Only include if the signal actually exists in expanded I/O
                        if expanded_ios.iter().any(|io| io.signal == expanded_signal) {
                            result.push(ExpandedAlarmRule {
                                signal: expanded_signal,
                                severity: rule.severity.clone(),
                                condition: rule.condition.clone(),
                                setpoint: rule.setpoint.clone(),
                                description: rule.description.clone(),
                            });
                        }
                    }
                } else {
                    // Static signal — only include if it exists in expanded I/O
                    if expanded_ios.iter().any(|io| io.signal == *pattern) {
                        result.push(ExpandedAlarmRule {
                            signal: pattern.clone(),
                            severity: rule.severity.clone(),
                            condition: rule.condition.clone(),
                            setpoint: rule.setpoint.clone(),
                            description: rule.description.clone(),
                        });
                    }
                }
            }
        }
        result
    }

    /// Expand signal patterns in dashboard section signals (e.g. "PMP_{n}_RUN" → ["PMP_1_RUN", "PMP_2_RUN"])
    fn expand_section_signals(&self, patterns: &[String]) -> Vec<String> {
        let mut result = Vec::new();
        for pattern in patterns {
            if pattern.contains("{n}") {
                let count = self.instance_counts.get("n").copied().unwrap_or(2);
                for i in 1..=count {
                    result.push(pattern.replace("{n}", &i.to_string()));
                }
            } else {
                result.push(pattern.clone());
            }
        }
        result
    }
}

/// An alarm rule with its signal pattern expanded to a concrete signal name
struct ExpandedAlarmRule {
    signal: String,
    severity: String,
    condition: String,
    setpoint: String,
    #[allow(dead_code)]
    description: String,
}

/// Parse typical_range strings like "0-100", "0-10V", "4-20mA", "6.5-8.5", "-1-10", "-70-20"
/// Returns (min, max) with sensible defaults if parsing fails.
fn parse_range(range: &str) -> (f64, f64) {
    // Strip common unit suffixes
    let cleaned = range
        .trim()
        .trim_end_matches("V")
        .trim_end_matches("mA")
        .trim_end_matches('%');

    // Find the separator dash — need to handle negative numbers like "-1-10"
    // Strategy: find the last '-' that is preceded by a digit (not at start)
    let parts: Vec<&str> = if let Some(rest) = cleaned.strip_prefix('-') {
        // Leading negative: "-1-10" → split at second dash
        if let Some(pos) = rest.find('-') {
            vec![&cleaned[..pos + 1], &rest[pos + 1..]]
        } else {
            return (-1.0, 1.0); // single negative number, unlikely
        }
    } else {
        cleaned.splitn(2, '-').collect()
    };

    if parts.len() == 2 {
        let min = parts[0].parse::<f64>().unwrap_or(0.0);
        let max = parts[1].parse::<f64>().unwrap_or(100.0);
        if min < max {
            return (min, max);
        }
    }

    // Default range
    (0.0, 100.0)
}

/// Parse setpoint strings like "90%", "2.0 bar", "180°C", "0.3 NTU", "1", "0.5 bar"
/// Returns the numeric value; if parsing fails, returns the default of 0.0.
fn parse_setpoint(setpoint: &str) -> f64 {
    let s = setpoint.trim();

    // Strip leading ± sign (we take the magnitude only)
    let s = s.strip_prefix('\u{00b1}').unwrap_or(s); // ±
    let s = s.strip_prefix("+-").unwrap_or(s);
    let s = s.strip_prefix("-+").unwrap_or(s);

    // Handle percentage like "90%"
    if let Some(stripped) = s.strip_suffix('%')
        && let Ok(val) = stripped.trim().parse::<f64>()
    {
        return val;
    }

    // Try to extract the leading numeric value from strings like "2.0 bar", "180°C", "0.3 NTU"
    let num_str: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();

    if let Ok(val) = num_str.parse::<f64>() {
        return val;
    }

    // Fallback: try parsing the whole string as a number
    s.parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_range_simple() {
        assert_eq!(parse_range("0-100"), (0.0, 100.0));
        assert_eq!(parse_range("0-10"), (0.0, 10.0));
        assert_eq!(parse_range("0-5000"), (0.0, 5000.0));
    }

    #[test]
    fn test_parse_range_with_units() {
        assert_eq!(parse_range("0-10V"), (0.0, 10.0));
        assert_eq!(parse_range("4-20mA"), (4.0, 20.0));
        assert_eq!(parse_range("0-100%"), (0.0, 100.0));
    }

    #[test]
    fn test_parse_range_negative() {
        assert_eq!(parse_range("-1-10"), (-1.0, 10.0));
        assert_eq!(parse_range("-70-20"), (-70.0, 20.0));
        assert_eq!(parse_range("6.5-8.5"), (6.5, 8.5));
    }

    #[test]
    fn test_parse_range_defaults() {
        // Invalid range returns default
        let (min, max) = parse_range("garbage");
        assert_eq!(min, 0.0);
        assert_eq!(max, 100.0);
    }

    #[test]
    fn test_parse_setpoint_percentage() {
        assert_eq!(parse_setpoint("90%"), 90.0);
        assert_eq!(parse_setpoint("10%"), 10.0);
    }

    #[test]
    fn test_parse_setpoint_with_unit() {
        assert_eq!(parse_setpoint("2.0 bar"), 2.0);
        assert_eq!(parse_setpoint("180°C"), 180.0);
        assert_eq!(parse_setpoint("0.3 NTU"), 0.3);
        assert_eq!(parse_setpoint("0.5 bar"), 0.5);
        assert_eq!(parse_setpoint("12°C"), 12.0);
    }

    #[test]
    fn test_parse_setpoint_bare_number() {
        assert_eq!(parse_setpoint("1"), 1.0);
        assert_eq!(parse_setpoint("0"), 0.0);
        assert_eq!(parse_setpoint("0.2"), 0.2);
    }

    #[test]
    fn test_parse_setpoint_complex() {
        assert_eq!(parse_setpoint("110% FLA"), 110.0);
        assert_eq!(parse_setpoint("10% below MAWT"), 10.0);
        assert_eq!(parse_setpoint("90% of MAWP"), 90.0);
        assert_eq!(parse_setpoint("±0.5 Hz"), 0.5);
        assert_eq!(parse_setpoint("±10%"), 10.0);
        assert_eq!(parse_setpoint("±0.5 pH from target"), 0.5);
    }
}
