use crate::storage::Storage;
use crate::vertical_templates::definitions::{TemplateAlarm, TemplateTag, VerticalTemplate};
use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashMap;

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

    /// Execute the full instantiation
    pub async fn instantiate(&self) -> Result<InstantiationResult> {
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
                    tag.min_val, // warn_val defaults to min (no alarm by default)
                    tag.max_val, // crit_val defaults to max (no alarm by default)
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
        let all_data_sources = self.storage.list_data_sources(None).await?;
        for alarm in &expanded_alarms {
            // Find matching data source
            if let Some(ds) = all_data_sources.iter().find(|ds| {
                ds.key.ends_with(&alarm.tag_pattern)
                    || ds.key.contains(&format!(":{}", alarm.tag_pattern))
            }) {
                let update = match alarm.comparison.as_str() {
                    "above" => json!({
                        "warn_val": if alarm.severity == "warning" { alarm.threshold } else { ds.warn_val },
                        "crit_val": if alarm.severity == "critical" { alarm.threshold } else { ds.crit_val },
                    }),
                    "below" => json!({
                        "warn_val": if alarm.severity == "warning" { alarm.threshold } else { ds.warn_val },
                        "crit_val": if alarm.severity == "critical" { alarm.threshold } else { ds.crit_val },
                    }),
                    "equal" => json!({
                        "crit_val": if alarm.severity == "critical" { alarm.threshold } else { ds.crit_val },
                        "warn_val": if alarm.severity == "warning" { alarm.threshold } else { ds.warn_val },
                    }),
                    _ => json!({}),
                };

                if !update.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                    self.storage
                        .update_data_source(ds.id, &update)
                        .await
                        .context("Failed to update alarm threshold")?;
                    created_alarms.push(format!(
                        "{}: {} {} {}",
                        alarm.tag_pattern, alarm.severity, alarm.comparison, alarm.threshold
                    ));
                }
            }
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
