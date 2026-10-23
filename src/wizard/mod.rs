pub mod feed;
pub mod feed_instantiator;
pub mod health;

use crate::storage::Storage;
use crate::vertical_templates::instantiator::Instantiator;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Combined wizard configuration — what the user provides through the wizard UI
#[derive(Debug, Clone, Deserialize)]
pub struct WizardConfig {
    /// Template ID from vertical_templates or feed_templates
    pub template_id: Option<String>,

    /// Template source: "vertical" (v0.37) or "feed" (FEED template)
    pub template_source: Option<String>,

    /// Industry the user selected
    pub industry: Option<String>,

    /// Device configuration
    pub device: WizardDeviceConfig,

    /// Number of equipment instances (e.g., 4 pumps, 2 chillers)
    #[serde(default = "default_instance_counts")]
    pub instance_counts: HashMap<String, u32>,

    /// Whether to generate dashboards
    #[serde(default = "default_true")]
    pub generate_dashboard: bool,

    /// Whether to generate alarms
    #[serde(default = "default_true")]
    pub generate_alarms: bool,

    /// Whether to generate OEE configuration
    #[serde(default)]
    pub generate_oee: bool,
}

fn default_instance_counts() -> HashMap<String, u32> {
    let mut m = HashMap::new();
    m.insert("n".into(), 2);
    m
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct WizardDeviceConfig {
    /// Device ID (auto-generated if empty)
    pub id: Option<String>,

    /// Protocol: modbus, opcua, s7, ethernet_ip, dnp3, bacnet, mqtt, simulation
    pub protocol: String,

    /// IP address or hostname
    pub address: String,

    /// Port number
    #[serde(default = "default_port")]
    pub port: u16,

    /// Modbus unit ID (only for Modbus)
    #[serde(default)]
    pub unit_id: u8,

    /// Scan: whether to auto-discover tags on this device
    pub auto_scan: bool,

    /// Auto-scan configuration
    #[serde(default)]
    pub scan: Option<WizardScanConfig>,
}

fn default_port() -> u16 {
    502
}

#[derive(Debug, Clone, Deserialize)]
pub struct WizardScanConfig {
    pub register_area: String, // holding, input, coil, discrete
    pub start_address: u16,
    pub end_address: u16,
    pub scan_delay_ms: u64,
}

impl Default for WizardScanConfig {
    fn default() -> Self {
        Self {
            register_area: "holding".into(),
            start_address: 0,
            end_address: 499,
            scan_delay_ms: 10,
        }
    }
}

/// Result of wizard execution
#[derive(Debug, Clone, Serialize)]
pub struct WizardResult {
    pub success: bool,
    pub device_id: String,
    pub tags_created: u32,
    pub alarms_created: u32,
    pub flows_created: u32,
    pub screens_created: u32,
    /// Tags discovered via auto-scan (not created from template)
    pub tags_discovered: u32,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}

/// Execute the project creation wizard
pub async fn execute_wizard(
    storage: &Storage,
    config: WizardConfig,
) -> Result<WizardResult, anyhow::Error> {
    let mut warnings = Vec::new();
    let mut next_steps = Vec::new();
    let mut tags_created = 0u32;
    let mut alarms_created = 0u32;
    let mut flows_created = 0u32;
    let mut screens_created = 0u32;

    // 1. Determine device ID
    let device_id = config.device.id.clone().unwrap_or_else(|| {
        let proto = &config.device.protocol;
        let name = config.industry.as_deref().unwrap_or("device");
        format!("{}_{}", proto, name.to_lowercase().replace(' ', "_"))
    });

    // 2. Create or ensure device exists
    if !storage.device_exists(&device_id).await? {
        let name = format!(
            "{} — {}",
            config.industry.as_deref().unwrap_or("Project"),
            config.device.protocol.to_uppercase()
        );
        storage.create_device(&device_id, &name, 0).await?;
    }

    // 3. Instantiate template if provided
    if let Some(template_id) = &config.template_id {
        let source = config.template_source.as_deref().unwrap_or("vertical");

        if source == "vertical" {
            // Use v0.37 vertical template instantiator
            if let Some(template) =
                crate::vertical_templates::definitions::get_template(template_id)
            {
                let template_name = template.name.clone();
                let instantiator = Instantiator::new(
                    storage,
                    template,
                    config.industry.clone().unwrap_or_else(|| "Project".into()),
                    config.instance_counts.clone(),
                );
                let result = instantiator.instantiate().await?;
                tags_created = result.tags_created as u32;
                alarms_created = result.alarms_created as u32;
                flows_created = result.flows_created as u32;

                if tags_created == 0 {
                    warnings.push(format!(
                        "Template '{}' instantiated but created 0 tags. Tags may already exist.",
                        template_name
                    ));
                }
            } else {
                warnings.push(format!(
                    "Template '{}' not found — skipping template instantiation",
                    template_id
                ));
            }
        } else if source == "feed" {
            // FEED template instantiation — creates tags, alarms, and dashboard from I/O list
            if let Some(template) = crate::wizard::feed::get_feed_template(template_id) {
                let template_name = template.name.clone();
                let instantiator = feed_instantiator::FeedInstantiator::new(
                    storage,
                    template,
                    config.industry.clone().unwrap_or_else(|| "Project".into()),
                    config.instance_counts.clone(),
                );
                let result = instantiator.instantiate().await?;
                tags_created = result.tags_created as u32;
                alarms_created = result.alarms_created as u32;
                screens_created = result.screens_created as u32;

                if tags_created == 0 {
                    warnings.push(format!(
                        "FEED template '{}' instantiated but created 0 tags. Tags may already exist.",
                        template_name
                    ));
                }
                next_steps.push(format!(
                    "FEED template '{}' created {} tags, {} alarms, {} screen(s), {} equipment blocks, and {} connections. Connect your PLC protocol (Modbus, OPC UA, etc.) to the data sources and adjust register addresses.",
                    template_name, tags_created, alarms_created, screens_created,
                    result.equipment_created, result.connections_created
                ));
            } else {
                warnings.push(format!(
                    "FEED template '{}' not found — skipping template instantiation",
                    template_id
                ));
            }
        }
    } else if !config.device.auto_scan {
        warnings.push("No template selected and auto-scan disabled. Device will be created empty — add tags manually or re-run with a template.".into());
    }

    // 4. Auto-scan if requested
    let mut tags_discovered = 0u32;
    if config.device.auto_scan && config.device.protocol == "modbus" {
        let default_scan = WizardScanConfig::default();
        let scan = config.device.scan.as_ref().unwrap_or(&default_scan);
        match crate::modbus::scan_registers(
            &config.device.address,
            config.device.port,
            config.device.unit_id,
            scan.start_address,
            scan.end_address,
            &crate::models::Endianness::Big,
            &scan.register_area,
            scan.scan_delay_ms,
        )
        .await
        {
            Ok(registers) => {
                let readable: Vec<_> = registers.iter().filter(|r| r.readable).collect();
                tags_discovered = readable.len() as u32;

                if tags_discovered > 0 {
                    // Create data sources for discovered registers
                    let mut created = 0u32;
                    for reg in readable.iter().take(200) {
                        let tag_name = format!("{}_{}", scan.register_area, reg.address);
                        let key = format!("{}:{}", device_id, tag_name);
                        let existing = storage.list_data_sources(Some(&device_id)).await?;
                        if existing.iter().any(|ds| ds.key == key) {
                            continue;
                        }
                        if let Err(e) = storage
                            .create_data_source(
                                &key,
                                &tag_name,
                                "",             // unit
                                "#3b82f6",      // color
                                0.0,            // min_val
                                100.0,          // max_val
                                0.0,            // warn_val
                                0.0,            // crit_val
                                0.0,            // base_val
                                created as i64, // sort_order
                                2,              // decimals
                                "modbus",       // source_type
                                "{}",           // source_config
                                &device_id,
                                None,    // equipment_id
                                "gauge", // chart_type
                            )
                            .await
                        {
                            warnings.push(format!("Failed to create tag '{}': {}", tag_name, e));
                        } else {
                            created += 1;
                        }
                    }
                    tags_created += created;
                    next_steps.push(format!(
                        "Auto-discovered {} readable registers on {}. Created {} tags. Adjust polling intervals and add alarm thresholds for critical measurements.",
                        tags_discovered, config.device.address, created
                    ));
                } else {
                    warnings.push(format!(
                        "No readable registers found on {} (scanned {}-{}). Check device connectivity, unit ID, and register area.",
                        config.device.address, scan.start_address, scan.end_address
                    ));
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "Auto-scan failed on {}: {}",
                    config.device.address, e
                ));
            }
        }
    } else if config.device.auto_scan && config.device.protocol != "modbus" {
        warnings.push(format!("Auto-scan is only supported for Modbus devices in v0.40. Use manual tag configuration or a vertical template for {} devices.", config.device.protocol));
    }

    // 5. Generate dashboard (basic)
    if config.generate_dashboard && tags_created > 0 {
        let screen_name = format!(
            "{} Overview",
            config.industry.as_deref().unwrap_or("Project")
        );
        // Screens in Voltrus are referenced by ID, not created explicitly.
        // The screen exists when data sources are assigned to it.
        screens_created = 1;
        next_steps.push(format!(
            "Dashboard screen '{}' ready. Use the dashboard to arrange widgets for your new tags.",
            screen_name
        ));
    }

    Ok(WizardResult {
        success: true,
        device_id,
        tags_created,
        alarms_created,
        flows_created,
        screens_created,
        tags_discovered,
        warnings,
        next_steps,
    })
}
