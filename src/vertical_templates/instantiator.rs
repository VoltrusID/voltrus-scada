use crate::storage::Storage;
use crate::vertical_templates::definitions::{TemplateAlarm, TemplateTag, VerticalTemplate};
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashMap;

/// Largest count a multi-instance pattern accepts (pumps 1..n, lights).
pub const MAX_INSTANCE_COUNT: u32 = 100;
/// Largest number of tags one template may expand to.
pub const MAX_EXPANDED_TAGS: usize = 2000;

/// Instantiate a vertical template into the database.
/// Creates devices, data sources (tags), alarm configs, screen, and flow rules.
pub struct Instantiator<'a> {
    storage: &'a Storage,
    template: VerticalTemplate,
    instance_name: String,
    /// Number of instances for multi-instance tag patterns (e.g. pumps 1..n)
    instance_counts: HashMap<String, u32>,
}

impl<'a> Instantiator<'a> {
    pub fn new(
        storage: &'a Storage,
        template: VerticalTemplate,
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

    /// Default instance counts for standard configurations
    pub fn default_instance_counts() -> HashMap<String, u32> {
        let mut counts = HashMap::new();
        counts.insert("n".into(), 2); // Default 2 instances for pumps, engines, etc.
        counts.insert("light".into(), 4); // 4 navigation lights
        counts.insert("{n}".into(), 2);
        counts.insert("{light}".into(), 4);
        counts
    }

    /// Checks the instance counts before anything is expanded: every count must be
    /// 1..=[`MAX_INSTANCE_COUNT`] and the template may expand to at most
    /// [`MAX_EXPANDED_TAGS`] tags. Without this a count of `u32::MAX` would try to
    /// build billions of tags in memory.
    pub fn check_limits(&self) -> std::result::Result<(), String> {
        for (key, &count) in &self.instance_counts {
            if !(1..=MAX_INSTANCE_COUNT).contains(&count) {
                return Err(format!(
                    "instance_counts.{key} must be 1-{MAX_INSTANCE_COUNT}"
                ));
            }
        }
        let total: usize = self
            .template
            .tags
            .iter()
            .map(|tag| {
                if tag.name_pattern.contains("{n}") {
                    self.instance_counts.get("n").copied().unwrap_or(2) as usize
                } else if tag.name_pattern.contains("{light}") {
                    self.instance_counts.get("light").copied().unwrap_or(4) as usize
                } else {
                    1
                }
            })
            .sum();
        if total > MAX_EXPANDED_TAGS {
            return Err(format!(
                "These instance counts make {total} tags; the limit is {MAX_EXPANDED_TAGS}"
            ));
        }
        Ok(())
    }

    /// Execute the full instantiation
    pub async fn instantiate(&self) -> Result<InstantiationResult> {
        self.check_limits().map_err(anyhow::Error::msg)?;
        let device_id = self.device_id();
        let mut created_tags = Vec::new();
        let mut created_alarms = Vec::new();
        let mut created_flows = Vec::new();
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
                .context("Failed to create device from template")?;
        }

        // 2. Create data sources (tags) from template
        let expanded_tags = self.expand_tag_patterns();
        let mut sort_order = 0;
        for tag in &expanded_tags {
            let resolved_name = tag.name_pattern.clone();
            let key = format!("{}:{}", device_id, resolved_name);

            // Check if data source already exists
            let existing = self.storage.list_data_sources(Some(&device_id)).await?;
            if existing.iter().any(|ds| ds.key == key) {
                continue; // Skip existing tags
            }

            let ds_id = self
                .storage
                .create_data_source(
                    &key,
                    &resolved_name,
                    &tag.unit,
                    "#f97316", // Default orange color
                    tag.min_val,
                    tag.max_val,
                    0.0, // no high warning limit until one is configured
                    0.0, // no high critical limit until one is configured
                    (tag.min_val + tag.max_val) / 2.0, // base_val
                    sort_order,
                    if tag.data_type == "bool" { 0 } else { 1 }, // decimals
                    "modbus",                                    // Default source type
                    &json!({
                        "template_id": self.template.id,
                        "register_area": tag.register_area,
                        "data_type": tag.data_type,
                        "scale": tag.scale,
                        "widget": tag.widget,
                        "group": tag.group,
                        "interval_ms": tag.interval_ms,
                    })
                    .to_string(),
                    &device_id,
                    None,    // equipment_id
                    "gauge", // chart_type
                )
                .await
                .context(format!("Failed to create data source: {}", key))?;

            created_data_source_ids.push(ds_id);
            created_tags.push(resolved_name);
            sort_order += 1;
        }

        // 3. Create screen and assign data sources
        let screen_id = format!(
            "{}_{}",
            self.template.id,
            self.instance_name.to_lowercase().replace(' ', "_")
        );
        if !self.storage.device_exists(&screen_id).await? {
            self.storage
                .create_device(&screen_id, &self.template.dashboard.screen_name, 1)
                .await?;
            for (i, &ds_id) in created_data_source_ids.iter().enumerate() {
                self.storage
                    .add_screen_data_source(&screen_id, ds_id, i as i64)
                    .await
                    .context("Failed to assign data source to screen")?;
            }
        }

        // 4. Create flow rules from template
        for flow in &self.template.flows {
            let flow_id = format!(
                "vt_{}_{}",
                self.template.id,
                flow.name.to_lowercase().replace([' ', '/'], "_")
            );
            // Check if flow already exists
            let existing_flows = self.storage.list_flows().await?;
            if existing_flows.iter().any(|f| f.id == flow_id) {
                continue;
            }

            self.storage
                .create_flow(
                    &flow_id,
                    &flow.name,
                    false, // Disabled by default — engineer must enable
                    &flow.trigger_config.to_string(),
                    &flow.nodes.to_string(),
                )
                .await
                .context(format!("Failed to create flow: {}", flow.name))?;

            created_flows.push(flow.name.clone());
        }

        // 5. Apply alarm thresholds to created data sources
        let expanded_alarms = self.expand_alarm_patterns(&expanded_tags);
        // All tags, then this instance's (the argument of list_data_sources
        // is a screen id, not a device id).
        let all_data_sources: Vec<_> = self
            .storage
            .list_data_sources(None)
            .await?
            .into_iter()
            .filter(|ds| ds.device_id == device_id)
            .collect();
        // Per tag: the columns and source_config to write, built from all of
        // the tag's template alarms, so two alarms on one tag (warning and
        // critical, high and low) don't overwrite each other.
        let mut updates: Vec<(
            i64,
            serde_json::Map<String, serde_json::Value>,
            serde_json::Value,
        )> = Vec::new();
        for alarm in &expanded_alarms {
            let key = format!("{device_id}:{}", alarm.tag_pattern);
            let Some(ds) = all_data_sources.iter().find(|ds| ds.key == key) else {
                continue;
            };
            let at = match updates.iter().position(|(id, ..)| *id == ds.id) {
                Some(at) => at,
                None => {
                    let cfg = serde_json::from_str::<serde_json::Value>(&ds.source_config)
                        .ok()
                        .filter(|v| v.is_object())
                        .unwrap_or_else(|| json!({}));
                    updates.push((ds.id, serde_json::Map::new(), cfg));
                    updates.len() - 1
                }
            };
            let (_, columns, cfg) = &mut updates[at];
            if !apply_template_alarm(alarm, ds.min_val, columns, cfg) {
                continue;
            }
            created_alarms.push(format!(
                "{}: {} {} {}",
                alarm.tag_pattern, alarm.severity, alarm.comparison, alarm.threshold
            ));
        }
        for (id, mut columns, cfg) in updates {
            columns.insert("source_config".into(), json!(cfg.to_string()));
            self.storage
                .update_data_source(id, &serde_json::Value::Object(columns))
                .await
                .context("Failed to update alarm threshold")?;
        }

        // 6. Create OEE shift if template has OEE config
        if self.template.oee_config.is_some() {
            // Ensure at least one shift exists for OEE tracking
            let shifts = self.storage.list_shifts().await?;
            if shifts.is_empty() {
                self.storage
                    .create_shift("Day Shift", 6, 0, 8, 127)
                    .await
                    .ok();
            }
        }

        Ok(InstantiationResult {
            template_id: self.template.id.clone(),
            template_name: self.template.name.clone(),
            device_id,
            screen_id,
            tags_created: created_tags.len(),
            alarms_created: created_alarms.len(),
            flows_created: created_flows.len(),
            tags: created_tags,
            alarms: created_alarms,
            flows: created_flows,
        })
    }

    fn device_id(&self) -> String {
        format!("vt_{}", self.instance_name.to_lowercase().replace(' ', "_"))
    }

    /// Expand tag name patterns like "PMP{n}_RUN" into concrete tag names
    pub fn expand_tag_patterns(&self) -> Vec<TemplateTag> {
        let mut result = Vec::new();
        for tag in &self.template.tags {
            let pattern = &tag.name_pattern;
            // Check if this tag has expandable patterns
            if pattern.contains("{n}") {
                let count = self.instance_counts.get("n").copied().unwrap_or(2);
                for i in 1..=count {
                    let mut expanded = tag.clone();
                    expanded.name_pattern = pattern.replace("{n}", &i.to_string());
                    expanded.description = tag.description.replace("{n}", &i.to_string());
                    result.push(expanded);
                }
            } else if pattern.contains("{light}") {
                let count = self.instance_counts.get("light").copied().unwrap_or(4);
                let light_names = ["PORT", "STBD", "BOW", "STERN"];
                for i in 0..count {
                    let name = light_names.get(i as usize).unwrap_or(&"UNKNOWN");
                    let mut expanded = tag.clone();
                    expanded.name_pattern = pattern.replace("{light}", name);
                    expanded.description = tag.description.replace("{light}", name);
                    result.push(expanded);
                }
            } else {
                result.push(tag.clone());
            }
        }
        result
    }

    /// Expand alarm patterns to match expanded tag names
    pub fn expand_alarm_patterns(&self, expanded_tags: &[TemplateTag]) -> Vec<TemplateAlarm> {
        let mut result = Vec::new();
        for alarm in &self.template.alarms {
            let pattern = &alarm.tag_pattern;
            if pattern.contains("{n}") {
                let count = self.instance_counts.get("n").copied().unwrap_or(2);
                for i in 1..=count {
                    let mut expanded = alarm.clone();
                    expanded.tag_pattern = pattern.replace("{n}", &i.to_string());
                    expanded.message_template =
                        alarm.message_template.replace("{n}", &i.to_string());
                    // Only include if the tag actually exists
                    if expanded_tags
                        .iter()
                        .any(|t| t.name_pattern == expanded.tag_pattern)
                    {
                        result.push(expanded);
                    }
                }
            } else if pattern.contains("{light}") {
                let count = self.instance_counts.get("light").copied().unwrap_or(4);
                let light_names = ["PORT", "STBD", "BOW", "STERN"];
                for i in 0..count {
                    let name = light_names.get(i as usize).unwrap_or(&"UNKNOWN");
                    let mut expanded = alarm.clone();
                    expanded.tag_pattern = pattern.replace("{light}", name);
                    expanded.message_template = alarm.message_template.replace("{light}", name);
                    if expanded_tags
                        .iter()
                        .any(|t| t.name_pattern == expanded.tag_pattern)
                    {
                        result.push(expanded);
                    }
                }
            } else {
                if expanded_tags
                    .iter()
                    .any(|t| t.name_pattern == pattern.as_str())
                {
                    result.push(alarm.clone());
                }
            }
        }
        result
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstantiationResult {
    pub template_id: String,
    pub template_name: String,
    pub device_id: String,
    pub screen_id: String,
    pub tags_created: usize,
    pub alarms_created: usize,
    pub flows_created: usize,
    pub tags: Vec<String>,
    pub alarms: Vec<String>,
    pub flows: Vec<String>,
}

/// Write one template alarm into a tag's limit columns (`columns`) and
/// source_config (`cfg`). Returns false for a comparison that maps to no limit.
///
/// Limit evaluation fires a high limit at value >= limit and a low one at
/// value <= limit. "above" is a high limit and "below" a low one (in
/// source_config; written as warn_val or crit_val it would be a high limit,
/// raised in normal operation). "equal" is for two-state tags: equal to the
/// bottom of the tag's range (a safety contact reading 0) is a low limit
/// there, anything else a high limit. High limits are written with their
/// `<name>_enabled` flag so a limit of 0 or below counts. The tag's deadband
/// is the largest of its alarms' deadbands.
fn apply_template_alarm(
    alarm: &TemplateAlarm,
    min_val: f64,
    columns: &mut serde_json::Map<String, serde_json::Value>,
    cfg: &mut serde_json::Value,
) -> bool {
    let critical = alarm.severity == "critical";
    let high = match alarm.comparison.as_str() {
        "above" => true,
        "below" => false,
        "equal" => alarm.threshold > min_val,
        _ => return false,
    };
    if high {
        let (column, flag) = if critical {
            ("crit_val", "crit_enabled")
        } else {
            ("warn_val", "warn_enabled")
        };
        columns.insert(column.into(), json!(alarm.threshold));
        cfg[flag] = json!(true);
    } else {
        cfg[if critical { "low_crit" } else { "low_warn" }] = json!(alarm.threshold);
    }
    if alarm.deadband > 0.0 {
        let current = cfg.get("deadband").and_then(|v| v.as_f64()).unwrap_or(0.0);
        cfg["deadband"] = json!(current.max(alarm.deadband));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertical_templates::definitions;

    async fn storage() -> Storage {
        Storage::new_in_memory().unwrap()
    }

    fn template_with_n() -> VerticalTemplate {
        definitions::all_templates()
            .into_iter()
            .find(|t| t.tags.iter().any(|tag| tag.name_pattern.contains("{n}")))
            .expect("a template with an {n} pattern")
    }

    #[tokio::test]
    async fn huge_instance_count_is_refused_before_expansion() {
        let storage = storage().await;
        let counts = HashMap::from([("n".to_string(), u32::MAX)]);
        let inst = Instantiator::new(&storage, template_with_n(), "x".into(), counts);
        assert!(inst.check_limits().is_err());
        assert!(inst.instantiate().await.is_err());
    }

    #[tokio::test]
    async fn zero_count_is_refused_and_defaults_pass() {
        let storage = storage().await;
        let zero = HashMap::from([("n".to_string(), 0)]);
        let inst = Instantiator::new(&storage, template_with_n(), "x".into(), zero);
        assert!(inst.check_limits().is_err());
        let inst = Instantiator::new(
            &storage,
            template_with_n(),
            "x".into(),
            Instantiator::default_instance_counts(),
        );
        assert!(inst.check_limits().is_ok());
        let max = HashMap::from([("n".to_string(), MAX_INSTANCE_COUNT)]);
        let inst = Instantiator::new(&storage, template_with_n(), "x".into(), max);
        assert!(inst.check_limits().is_ok());
    }

    #[tokio::test]
    async fn template_tags_alarm_only_on_the_template_limits() {
        let storage = storage().await;
        let template = definitions::all_templates()
            .into_iter()
            .find(|t| t.id == "wastewater_treatment")
            .unwrap();
        Instantiator::new(
            &storage,
            template,
            "ww".into(),
            Instantiator::default_instance_counts(),
        )
        .instantiate()
        .await
        .unwrap();
        let sources = storage.list_data_sources(None).await.unwrap();
        let limits = |suffix: &str| {
            let tag = sources
                .iter()
                .find(|ds| ds.key.ends_with(suffix))
                .unwrap_or_else(|| panic!("no tag {suffix}"));
            crate::alarm_eval::Thresholds::of(tag)
        };
        // pH: critical above 9 and warning below 6, both kept; neutral pH is normal.
        let ph = limits(":PH-1");
        assert_eq!((ph.high_crit, ph.high_warn), (Some(9.0), None));
        assert_eq!((ph.low_warn, ph.low_crit), (Some(6.0), None));
        assert_eq!(ph.condition(7.0), None);
        assert_eq!(ph.condition(5.0), Some(("warning", 6.0, "low")));
        // A low-only alarm is no high limit.
        let cl = limits(":CLO2_RES");
        assert_eq!(
            (cl.high_warn, cl.high_crit, cl.low_warn),
            (None, None, Some(0.2))
        );
        assert_eq!(cl.condition(1.0), None);
        // A tag without template alarms has no limits at all.
        let untouched = sources
            .iter()
            .filter(|ds| crate::alarm_eval::Thresholds::of(ds) == Default::default())
            .count();
        assert!(untouched > 0);
    }

    async fn instantiate(template_id: &str) -> Vec<crate::models::DataSource> {
        let storage = storage().await;
        let template = definitions::all_templates()
            .into_iter()
            .find(|t| t.id == template_id)
            .unwrap();
        Instantiator::new(
            &storage,
            template,
            "t".into(),
            Instantiator::default_instance_counts(),
        )
        .instantiate()
        .await
        .unwrap();
        storage.list_data_sources(None).await.unwrap()
    }

    fn tag<'a>(
        sources: &'a [crate::models::DataSource],
        suffix: &str,
    ) -> &'a crate::models::DataSource {
        sources
            .iter()
            .find(|ds| ds.key.ends_with(suffix))
            .unwrap_or_else(|| panic!("no tag {suffix}"))
    }

    #[tokio::test]
    async fn equal_zero_safety_alarms_fire_when_the_contact_reads_zero() {
        let sources = instantiate("elevator_ski_lift").await;
        for name in [":SAFE_DOOR", ":SAFE_ROPE"] {
            let t = crate::alarm_eval::Thresholds::of(tag(&sources, name));
            assert_eq!(t.condition(0.0), Some(("critical", 0.0, "low")), "{name}");
            assert_eq!(t.condition(1.0), None, "{name}");
        }
        // "equal 1" stays a high limit: fires at 1, not at 0.
        let estop = crate::alarm_eval::Thresholds::of(tag(&sources, ":SAFE_ESTOP"));
        assert_eq!(estop.condition(1.0), Some(("critical", 1.0, "high")));
        assert_eq!(estop.condition(0.0), None);
        // The template's deadband is written with the limit.
        let vib = crate::alarm_eval::Thresholds::of(tag(&sources, ":MTR_VIB"));
        assert_eq!((vib.high_warn, vib.deadband), (Some(10.0), 2.0));

        let ship = instantiate("maritime").await;
        for light in ["PORT", "STBD", "BOW", "STERN"] {
            let t = crate::alarm_eval::Thresholds::of(tag(&ship, &format!(":NAV_{light}")));
            assert_eq!(t.condition(0.0), Some(("warning", 0.0, "low")), "{light}");
            assert_eq!(t.condition(1.0), None, "{light}");
        }
        // Two alarms on one tag (critical 80, warning 50) both stay, with the
        // larger deadband.
        let bilge = crate::alarm_eval::Thresholds::of(tag(&ship, ":BILGE1_LVL"));
        assert_eq!((bilge.high_warn, bilge.high_crit), (Some(50.0), Some(80.0)));
        assert_eq!(bilge.deadband, 10.0);
    }

    #[tokio::test]
    async fn template_alarms_go_to_this_instance_only() {
        let storage = storage().await;
        for name in ["a", "b"] {
            let template = definitions::all_templates()
                .into_iter()
                .find(|t| t.id == "elevator_ski_lift")
                .unwrap();
            Instantiator::new(
                &storage,
                template,
                name.into(),
                Instantiator::default_instance_counts(),
            )
            .instantiate()
            .await
            .unwrap();
        }
        let sources = storage.list_data_sources(None).await.unwrap();
        for device in ["vt_a", "vt_b"] {
            let door = tag(&sources, &format!("{device}:SAFE_DOOR"));
            assert_eq!(
                crate::alarm_eval::Thresholds::of(door).low_crit,
                Some(0.0),
                "{device}"
            );
        }
    }
}
