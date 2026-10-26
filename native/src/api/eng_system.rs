//! Data Manager › System: the web client's `SystemPanel.tsx` tabs against
//! the server. Typed mirrors of every answer (host and process figures,
//! the generated documentation, the vertical and FEED templates, the
//! project wizard, the health check and the SMTP settings), the paths, the
//! request bodies and the rules the section applies before sending them
//! (the wizard's steps, the SMTP form's checks, the template instance
//! count).
//!
//! Roles as the server enforces them (`src/api/mod.rs`): every GET here,
//! the template preview and `POST /wizard/create` take any signed-in user;
//! `GET /templates/{id}`, `POST /templates/{id}/instantiate` and the three
//! `/settings/smtp` routes take the operator role (`demo_protected`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ops::encode;

// ── Paths ────────────────────────────────────────────────────────────────

pub const SYSTEM_PATH: &str = "/api/v1/system";
pub const TOPOLOGY_PATH: &str = "/api/v1/docs/topology";
pub const TAG_XREF_PATH: &str = "/api/v1/docs/tag-xref";
pub const AUTOMATION_PATH: &str = "/api/v1/docs/automation";
/// The handover document as one HTML file (the web client's "Export
/// HTML ↗").
pub const DOCS_EXPORT_PATH: &str = "/api/v1/docs/export?format=html";
/// The name the server's `content-disposition` gives the export.
pub const DOCS_EXPORT_FILE: &str = "voltrus-system-docs.html";
pub const TEMPLATE_PREVIEW_PATH: &str = "/api/v1/templates/preview";
pub const FEED_TEMPLATES_PATH: &str = "/api/v1/wizard/feed-templates";
pub const WIZARD_CREATE_PATH: &str = "/api/v1/wizard/create";
pub const HEALTH_PATH: &str = "/api/v1/wizard/health-check";
pub const SMTP_PATH: &str = "/api/v1/settings/smtp";
pub const SMTP_TEST_PATH: &str = "/api/v1/settings/smtp/test";

/// `GET /api/v1/templates`, every industry or one (`?industry=car_wash`).
pub fn templates_path(industry: Option<&str>) -> String {
    match industry {
        Some(industry) if !industry.is_empty() => {
            format!("/api/v1/templates?industry={}", encode(industry))
        }
        _ => "/api/v1/templates".to_owned(),
    }
}

/// `GET /api/v1/templates/{id}` (operator).
pub fn template_path(id: &str) -> String {
    format!("/api/v1/templates/{}", encode(id))
}

/// `POST /api/v1/templates/{id}/instantiate` (operator).
pub fn instantiate_path(id: &str) -> String {
    format!("/api/v1/templates/{}/instantiate", encode(id))
}

/// `GET /api/v1/wizard/feed-templates/{id}`.
pub fn feed_template_path(id: &str) -> String {
    format!("{FEED_TEMPLATES_PATH}/{}", encode(id))
}

// ── Overview: GET /api/v1/system ─────────────────────────────────────────

/// Mirrors `models::SystemResponse`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SystemInfo {
    pub hostname: String,
    pub os: String,
    pub uptime_secs: u64,
    pub cpu_usage: f64,
    pub memory_total_mb: f64,
    pub memory_used_mb: f64,
    pub memory_percent: f64,
    pub disk_total_gb: f64,
    pub disk_used_gb: f64,
    pub disk_percent: f64,
    pub db_size_mb: f64,
    pub process: ProcessInfo,
}

/// Mirrors `models::ProcessInfo`: the Voltrus server process itself.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub memory_mb: f64,
    pub cpu_percent: f64,
    pub uptime_secs: u64,
    pub binary_size_mb: f64,
}

/// How loaded a figure reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warning,
    Critical,
}

impl Level {
    /// Above `critical` is critical, above `warning` a warning.
    pub fn of(value: f64, warning: f64, critical: f64) -> Self {
        if value > critical {
            Self::Critical
        } else if value > warning {
            Self::Warning
        } else {
            Self::Ok
        }
    }

    /// The CPU figure's colour, as the web overview: over 80 % critical,
    /// over 50 % a warning.
    pub fn cpu(percent: f64) -> Self {
        Self::of(percent, 50.0, 80.0)
    }

    /// A usage bar's colour: over 90 % critical, over 70 % a warning.
    pub fn bar(percent: f64) -> Self {
        Self::of(percent, 70.0, 90.0)
    }
}

/// `3d 4h 12m`, the web overview's uptime.
pub fn uptime(secs: u64) -> String {
    format!(
        "{}d {}h {}m",
        secs / 86_400,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60
    )
}

/// The database's size and the line under it: `1.5 GB` over `1536 MB`,
/// or `12.3 MB` over `SQLite file`.
pub fn db_size(mb: f64) -> (String, String) {
    if mb > 1024.0 {
        (format!("{:.1} GB", mb / 1024.0), format!("{mb:.0} MB"))
    } else {
        (format!("{mb:.1} MB"), "SQLite file".to_owned())
    }
}

/// A bar's fill, 0–1.
#[allow(clippy::cast_possible_truncation)]
pub fn fill(percent: f64) -> f32 {
    (percent / 100.0).clamp(0.0, 1.0) as f32
}

// ── Documentation: GET /api/v1/docs/* ────────────────────────────────────

/// Mirrors `system_docs::SystemTopology`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Topology {
    pub devices: Vec<TopologyDevice>,
    pub connections: Vec<TopologyConnection>,
    pub summary: TopologySummary,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TopologyDevice {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub address: String,
    pub enabled: bool,
    pub tag_count: usize,
    pub online: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TopologyConnection {
    pub from: String,
    pub to: String,
    pub protocol: String,
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TopologySummary {
    pub total_devices: usize,
    pub total_tags: usize,
    pub protocols: Vec<ProtocolCount>,
    pub online_count: usize,
    pub offline_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ProtocolCount {
    pub protocol: String,
    pub device_count: usize,
    pub tag_count: usize,
}

/// Mirrors `system_docs::TagXrefReport`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TagXref {
    pub total_tags: usize,
    pub tags: Vec<TagXrefEntry>,
    pub by_device: Vec<DeviceTagSummary>,
    pub by_protocol: Vec<ProtocolCount>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TagXrefEntry {
    pub key: String,
    pub name: String,
    pub device_id: String,
    pub device_name: String,
    pub protocol: String,
    pub unit: String,
    pub source_type: String,
    pub data_type: Option<String>,
    pub address: Option<String>,
    pub has_alarm: bool,
    pub alarm_severity: Option<String>,
    pub alarm_threshold: Option<String>,
    pub warn_val: f64,
    pub crit_val: f64,
    pub path: Option<String>,
}

impl TagXrefEntry {
    /// The alarm cell: its severity, `yes` when the alarm names none, or
    /// `—` without an alarm.
    pub fn alarm_label(&self) -> &str {
        match (&self.alarm_severity, self.has_alarm) {
            (_, false) => "—",
            (Some(severity), true) if !severity.is_empty() => severity.as_str(),
            _ => "yes",
        }
    }

    /// Whether the row matches a search: key, name, device or protocol.
    pub fn matches(&self, query: &str) -> bool {
        matches_any(
            query,
            &[
                self.key.as_str(),
                self.name.as_str(),
                self.device_name.as_str(),
                self.protocol.as_str(),
            ],
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct DeviceTagSummary {
    pub device_id: String,
    pub device_name: String,
    pub protocol: String,
    pub tag_count: usize,
    pub alarmed_count: usize,
}

/// Mirrors `system_docs::AutomationDocs`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Automation {
    pub scripts: Vec<ScriptDoc>,
    pub flows: Vec<FlowDoc>,
    pub summary: AutomationSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ScriptDoc {
    pub id: i64,
    pub name: String,
    pub source_preview: String,
    pub enabled: bool,
    pub syntax_error: Option<String>,
    pub cpu_limit_ms: i64,
    pub memory_limit_bytes: i64,
    pub description: String,
    pub last_execution: Option<ScriptExecution>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ScriptExecution {
    pub status: String,
    pub duration_ms: i64,
    pub error_message: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FlowDoc {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_description: String,
    pub node_count: usize,
    pub node_descriptions: Vec<String>,
    pub description: String,
    pub last_run: Option<FlowRun>,
}

impl FlowDoc {
    /// The actions cell: the node descriptions joined, or `—`.
    pub fn actions(&self) -> String {
        if self.node_descriptions.is_empty() {
            "—".to_owned()
        } else {
            self.node_descriptions.join("; ")
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FlowRun {
    pub success: bool,
    pub duration_ms: i64,
    pub error: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AutomationSummary {
    pub total_scripts: usize,
    pub enabled_scripts: usize,
    pub total_flows: usize,
    pub enabled_flows: usize,
}

/// One row of the Automation view: the flow rules, then the scripts, as
/// the web panel lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationRow {
    Flow(usize),
    Script(usize),
}

impl Automation {
    /// The rows matching `query` (name or description).
    pub fn rows(&self, query: &str) -> Vec<AutomationRow> {
        let flows = self
            .flows
            .iter()
            .enumerate()
            .filter(|(_, flow)| {
                matches_any(
                    query,
                    &[
                        flow.name.as_str(),
                        flow.trigger_description.as_str(),
                        flow.description.as_str(),
                    ],
                )
            })
            .map(|(index, _)| AutomationRow::Flow(index));
        let scripts = self
            .scripts
            .iter()
            .enumerate()
            .filter(|(_, script)| {
                matches_any(query, &[script.name.as_str(), script.description.as_str()])
            })
            .map(|(index, _)| AutomationRow::Script(index));
        flows.chain(scripts).collect()
    }
}

/// A script's status pill: `Error` with a syntax error, else `ON`/`OFF`.
pub fn script_status(script: &ScriptDoc) -> &'static str {
    match (&script.syntax_error, script.enabled) {
        (Some(error), _) if !error.is_empty() => "Error",
        (_, true) => "ON",
        (_, false) => "OFF",
    }
}

/// Case-insensitive search over a row's texts; an empty query matches.
pub fn matches_any(query: &str, texts: &[&str]) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || texts
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
}

// ── Vertical templates: /api/v1/templates ────────────────────────────────

/// `GET /api/v1/templates[?industry=]`: `TemplateListResponse`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateList {
    pub templates: Vec<TemplateSummary>,
    pub industries: Vec<Industry>,
}

impl TemplateList {
    /// An industry's name for its id, or the id.
    pub fn industry_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.industries
            .iter()
            .find(|industry| industry.id == id)
            .map_or(id, |industry| industry.name.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub equipment_type: String,
    pub tag_count: usize,
    pub alarm_count: usize,
    pub flow_count: usize,
    pub has_oee: bool,
}

/// `vertical_templates::definitions::IndustryInfo`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Industry {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// `GET /api/v1/templates/{id}`: the whole `VerticalTemplate`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct VerticalTemplate {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub equipment_type: String,
    pub tags: Vec<TemplateTag>,
    pub alarms: Vec<TemplateAlarm>,
    pub dashboard: TemplateDashboard,
    pub flows: Vec<TemplateFlow>,
    pub oee_config: Option<TemplateOee>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateTag {
    pub name_pattern: String,
    pub description: String,
    pub unit: String,
    pub data_type: String,
    pub register_area: String,
    pub scale: f64,
    pub min_val: f64,
    pub max_val: f64,
    pub group: String,
    pub widget: String,
    pub interval_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateAlarm {
    pub tag_pattern: String,
    pub severity: String,
    pub comparison: String,
    pub threshold: f64,
    pub deadband: f64,
    pub message_template: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateDashboard {
    pub layout_groups: Vec<DashboardGroup>,
    pub screen_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct DashboardGroup {
    pub name: String,
    pub group_type: String,
    pub columns: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateFlow {
    pub name: String,
    pub trigger_type: String,
    pub trigger_config: serde_json::Value,
    pub nodes: serde_json::Value,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplateOee {
    pub availability_tags: Vec<String>,
    pub performance_tags: Vec<String>,
    pub quality_tags: Vec<String>,
    pub ideal_cycle_time_secs: Option<f64>,
}

/// `POST /api/v1/templates/preview`'s body: `PreviewRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewBody {
    pub template_id: String,
    pub instance_counts: BTreeMap<String, u32>,
}

/// `POST /api/v1/templates/{id}/instantiate`'s body: `InstantiateRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstantiateBody {
    pub template_id: String,
    pub name: String,
    pub instance_counts: BTreeMap<String, u32>,
}

/// The web form's instance count: `{"n": count}`.
pub fn instance_counts(count: u32) -> BTreeMap<String, u32> {
    BTreeMap::from([("n".to_owned(), count)])
}

/// The template forms' instance count: empty is the web form's 2, else a
/// whole number from 1 to 20 (the web field's `min`/`max`).
pub fn parse_instances(text: &str) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(2);
    }
    match text.parse::<u32>() {
        Ok(count) if (1..=20).contains(&count) => Ok(count),
        _ => Err("Instances must be a whole number from 1 to 20".to_owned()),
    }
}

impl PreviewBody {
    pub fn new(template_id: &str, instances: &str) -> Result<Self, String> {
        Ok(Self {
            template_id: template_id.to_owned(),
            instance_counts: instance_counts(parse_instances(instances)?),
        })
    }
}

impl InstantiateBody {
    /// The instantiate body; the instance needs a name (the web button
    /// stays disabled without one).
    pub fn new(template_id: &str, name: &str, instances: &str) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name the instance first".to_owned());
        }
        Ok(Self {
            template_id: template_id.to_owned(),
            name: name.to_owned(),
            instance_counts: instance_counts(parse_instances(instances)?),
        })
    }
}

/// The device id the server makes for an instance (`Instantiator::
/// device_id`): `vt_` and the name in lower case, spaces as `_`.
pub fn instance_device_id(name: &str) -> String {
    format!("vt_{}", name.trim().to_lowercase().replace(' ', "_"))
}

/// `POST /api/v1/templates/preview`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplatePreview {
    pub template_id: String,
    pub expanded_tags: Vec<PreviewTag>,
    pub expanded_alarms: Vec<PreviewAlarm>,
    pub flows: Vec<PreviewFlow>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PreviewTag {
    pub name: String,
    pub description: String,
    pub unit: String,
    pub data_type: String,
    pub group: String,
    pub widget: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PreviewAlarm {
    pub tag_pattern: String,
    pub severity: String,
    pub comparison: String,
    pub threshold: f64,
    pub message_template: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct PreviewFlow {
    pub name: String,
    pub description: String,
    pub trigger_type: String,
}

/// `POST /api/v1/templates/{id}/instantiate`'s answer:
/// `InstantiationResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Instantiated {
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

impl Instantiated {
    /// The web panel's line: `Created: 12 tags, 4 alarms, 1 flows on
    /// device "vt_north"`.
    pub fn summary(&self) -> String {
        format!(
            "Created: {} tags, {} alarms, {} flows on device \"{}\"",
            self.tags_created, self.alarms_created, self.flows_created, self.device_id
        )
    }
}

// ── Wizard: /api/v1/wizard/* ─────────────────────────────────────────────

/// `GET /api/v1/wizard/feed-templates`: `{"templates": [...]}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedTemplateList {
    pub templates: Vec<FeedTemplateSummary>,
}

/// `wizard::feed::FeedTemplateSummary`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedTemplateSummary {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub estimated_tag_count: u32,
    pub equipment_units: Vec<String>,
}

/// `GET /api/v1/wizard/feed-templates/{id}`: `wizard::feed::FeedTemplate`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedTemplate {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub equipment_type: String,
    pub estimated_tag_count: u32,
    pub equipment_units: Vec<FeedEquipmentUnit>,
    pub io_list: Vec<FeedIo>,
    pub alarm_strategy: Vec<FeedAlarmStrategy>,
    pub dashboard: FeedDashboard,
    pub network_notes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedEquipmentUnit {
    pub name: String,
    pub quantity: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedIo {
    pub signal: String,
    pub description: String,
    pub io_type: String,
    pub unit: String,
    pub data_type: String,
    pub typical_range: String,
    pub equipment: String,
    pub criticality: String,
    pub notes: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedAlarmStrategy {
    pub equipment: String,
    pub rules: Vec<FeedAlarmRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedAlarmRule {
    pub signal_pattern: String,
    pub severity: String,
    pub condition: String,
    pub setpoint: String,
    pub deadband: String,
    pub description: String,
    pub alarm_class: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedDashboard {
    pub description: String,
    pub recommended_screens: Vec<FeedScreen>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedScreen {
    pub name: String,
    pub layout: String,
    pub sections: Vec<FeedDashboardSection>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct FeedDashboardSection {
    pub title: String,
    pub equipment_group: String,
    pub widget_type: String,
    pub signals: Vec<String>,
}

/// `POST /api/v1/wizard/create`'s body: `wizard::WizardConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WizardConfig {
    pub template_id: Option<String>,
    pub template_source: String,
    pub industry: String,
    pub device: WizardDevice,
    pub instance_counts: BTreeMap<String, u32>,
    pub generate_dashboard: bool,
    pub generate_alarms: bool,
}

/// `wizard::WizardDeviceConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WizardDevice {
    pub id: Option<String>,
    pub protocol: String,
    pub address: String,
    pub port: u16,
    pub unit_id: u8,
    pub auto_scan: bool,
    pub scan: Option<WizardScan>,
}

/// `wizard::WizardScanConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WizardScan {
    pub register_area: String,
    pub start_address: u16,
    pub end_address: u16,
    pub scan_delay_ms: u64,
}

/// `POST /api/v1/wizard/create`'s answer: `wizard::WizardResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct WizardResult {
    pub success: bool,
    pub device_id: String,
    pub tags_created: u32,
    pub alarms_created: u32,
    pub flows_created: u32,
    pub screens_created: u32,
    pub tags_discovered: u32,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}

/// The wizard's industries, as `ProjectWizard.tsx` offers them: the id the
/// server gets and the label.
pub const INDUSTRIES: [(&str, &str); 6] = [
    ("Water/Wastewater", "Water/Wastewater"),
    ("Manufacturing", "Manufacturing / Material Handling"),
    ("Oil & Gas", "Oil & Gas / Industrial"),
    ("Chemical", "Chemical / Pharmaceutical"),
    ("Building Automation", "Building Automation / HVAC"),
    ("Energy", "Energy / Power"),
];

/// A protocol the wizard offers: id, name and default port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    pub id: &'static str,
    pub name: &'static str,
    pub port: u16,
}

pub const PROTOCOLS: [Protocol; 8] = [
    Protocol {
        id: "modbus",
        name: "Modbus TCP",
        port: 502,
    },
    Protocol {
        id: "opcua",
        name: "OPC-UA",
        port: 4840,
    },
    Protocol {
        id: "s7",
        name: "Siemens S7",
        port: 102,
    },
    Protocol {
        id: "ethernet_ip",
        name: "EtherNet/IP",
        port: 44818,
    },
    Protocol {
        id: "dnp3",
        name: "DNP3",
        port: 20000,
    },
    Protocol {
        id: "bacnet",
        name: "BACnet/IP",
        port: 47808,
    },
    Protocol {
        id: "mqtt",
        name: "MQTT",
        port: 1883,
    },
    Protocol {
        id: "simulation",
        name: "Simulation",
        port: 0,
    },
];

pub fn protocol(id: &str) -> Protocol {
    PROTOCOLS
        .into_iter()
        .find(|protocol| protocol.id == id)
        .unwrap_or(PROTOCOLS[0])
}

/// The auto-scan's register areas: id and label.
pub const REGISTER_AREAS: [(&str, &str); 4] = [
    ("holding", "Holding Registers (3x/4x)"),
    ("input", "Input Registers (3x)"),
    ("coil", "Coils (0x)"),
    ("discrete", "Discrete Inputs (1x)"),
];

/// Which kind of template the wizard's second step picks from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TemplateSource {
    #[default]
    Vertical,
    Feed,
}

impl TemplateSource {
    pub fn id(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Feed => "feed",
        }
    }
}

/// The full wizard's steps, in order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WizardStep {
    #[default]
    Industry,
    Template,
    Device,
    Scan,
    Review,
    Result,
}

impl WizardStep {
    /// The stepper's labels (the result is not a numbered step).
    pub const LABELS: [&'static str; 5] = ["Industry", "Template", "Device", "Scan", "Review"];

    pub fn index(self) -> usize {
        match self {
            Self::Industry => 0,
            Self::Template => 1,
            Self::Device => 2,
            Self::Scan => 3,
            Self::Review => 4,
            Self::Result => 5,
        }
    }
}

/// What the full wizard has gathered; text fields as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardDraft {
    pub industry: String,
    pub source: TemplateSource,
    /// The template picked (vertical or FEED); `None` is manual setup.
    pub template: Option<String>,
    pub protocol: &'static str,
    pub address: String,
    pub port: String,
    pub unit_id: String,
    pub device_id: String,
    pub instances: String,
    pub auto_scan: bool,
    pub dashboard: bool,
    pub alarms: bool,
    pub register_area: &'static str,
    pub start: String,
    pub end: String,
}

impl Default for WizardDraft {
    /// The web wizard's starting values.
    fn default() -> Self {
        Self {
            industry: String::new(),
            source: TemplateSource::Vertical,
            template: None,
            protocol: "modbus",
            address: "192.168.1.100".to_owned(),
            port: "502".to_owned(),
            unit_id: "1".to_owned(),
            device_id: String::new(),
            instances: "4".to_owned(),
            auto_scan: true,
            dashboard: true,
            alarms: true,
            register_area: "holding",
            start: "0".to_owned(),
            end: "499".to_owned(),
        }
    }
}

/// The device step's numbers, checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceNumbers {
    pub port: u16,
    pub unit_id: u8,
    pub instances: u32,
}

/// The scan step's range, checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRange {
    pub start: u16,
    pub end: u16,
}

impl ScanRange {
    pub fn count(self) -> u32 {
        u32::from(self.end) - u32::from(self.start) + 1
    }
}

impl WizardDraft {
    /// The protocol is Modbus and the auto-scan is on: the scan step
    /// applies (the server scans Modbus only).
    pub fn scans(&self) -> bool {
        self.auto_scan && self.protocol == "modbus"
    }

    /// A new protocol takes its default port, as the web select does.
    pub fn set_protocol(&mut self, id: &'static str) {
        self.protocol = id;
        self.port = protocol(id).port.to_string();
    }

    pub fn device_numbers(&self) -> Result<DeviceNumbers, String> {
        if self.address.trim().is_empty() && self.protocol != "simulation" {
            return Err("Enter the device's address".to_owned());
        }
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| "Port must be a whole number from 0 to 65535".to_owned())?;
        let unit_id = if self.protocol == "modbus" {
            self.unit_id
                .trim()
                .parse::<u8>()
                .map_err(|_| "Unit ID must be a whole number from 0 to 255".to_owned())?
        } else {
            self.unit_id.trim().parse::<u8>().unwrap_or(1)
        };
        let instances = match self.instances.trim().parse::<u32>() {
            Ok(count) if (1..=100).contains(&count) => count,
            _ => return Err("Instances must be a whole number from 1 to 100".to_owned()),
        };
        Ok(DeviceNumbers {
            port,
            unit_id,
            instances,
        })
    }

    pub fn scan_range(&self) -> Result<ScanRange, String> {
        let parse = |text: &str, what: &str| {
            text.trim()
                .parse::<u16>()
                .map_err(|_| format!("{what} must be a whole number from 0 to 65535"))
        };
        let start = parse(&self.start, "Start address")?;
        let end = parse(&self.end, "End address")?;
        if end < start {
            return Err("The end address comes before the start".to_owned());
        }
        Ok(ScanRange { start, end })
    }

    /// The step after `step`, or why the draft cannot move on.
    pub fn next(&self, step: WizardStep) -> Result<WizardStep, String> {
        match step {
            WizardStep::Industry => {
                if self.industry.is_empty() {
                    Err("Pick an industry".to_owned())
                } else {
                    Ok(WizardStep::Template)
                }
            }
            WizardStep::Template => Ok(WizardStep::Device),
            WizardStep::Device => {
                self.device_numbers()?;
                Ok(if self.scans() {
                    WizardStep::Scan
                } else {
                    WizardStep::Review
                })
            }
            WizardStep::Scan => {
                self.scan_range()?;
                Ok(WizardStep::Review)
            }
            WizardStep::Review | WizardStep::Result => Ok(WizardStep::Result),
        }
    }

    /// The step before `step`; `None` on the first.
    pub fn back(&self, step: WizardStep) -> Option<WizardStep> {
        match step {
            WizardStep::Industry => None,
            WizardStep::Template => Some(WizardStep::Industry),
            WizardStep::Device => Some(WizardStep::Template),
            WizardStep::Scan => Some(WizardStep::Device),
            WizardStep::Review => Some(if self.scans() {
                WizardStep::Scan
            } else {
                WizardStep::Device
            }),
            WizardStep::Result => None,
        }
    }

    /// The body `POST /wizard/create` gets, as the web wizard builds it: a
    /// FEED template is a reference design and goes as no template; the
    /// scan settings go whenever the auto-scan is ticked.
    pub fn config(&self) -> Result<WizardConfig, String> {
        let numbers = self.device_numbers()?;
        let scan = if self.auto_scan {
            let range = if self.scans() {
                self.scan_range()?
            } else {
                self.scan_range()
                    .unwrap_or(ScanRange { start: 0, end: 499 })
            };
            Some(WizardScan {
                register_area: self.register_area.to_owned(),
                start_address: range.start,
                end_address: range.end,
                scan_delay_ms: 10,
            })
        } else {
            None
        };
        let device_id = self.device_id.trim();
        Ok(WizardConfig {
            template_id: match self.source {
                TemplateSource::Vertical => self.template.clone(),
                TemplateSource::Feed => None,
            },
            template_source: self.source.id().to_owned(),
            industry: self.industry.clone(),
            device: WizardDevice {
                id: (!device_id.is_empty()).then(|| device_id.to_owned()),
                protocol: self.protocol.to_owned(),
                address: self.address.trim().to_owned(),
                port: numbers.port,
                unit_id: numbers.unit_id,
                auto_scan: self.scans(),
                scan,
            },
            instance_counts: instance_counts(numbers.instances),
            generate_dashboard: self.dashboard,
            generate_alarms: self.alarms,
        })
    }

    /// The review step's table.
    pub fn review(&self) -> Vec<(&'static str, String)> {
        let protocol = protocol(self.protocol);
        let mut rows = vec![
            ("Industry", self.industry.clone()),
            (
                "Template",
                match (&self.template, self.source) {
                    (Some(id), TemplateSource::Feed) => format!("FEED: {id}"),
                    (Some(id), TemplateSource::Vertical) => id.clone(),
                    (None, _) => "None (manual setup)".to_owned(),
                },
            ),
            ("Protocol", protocol.name.to_owned()),
            (
                "Address",
                format!("{}:{}", self.address.trim(), self.port.trim()),
            ),
        ];
        if self.protocol == "modbus" {
            rows.push(("Unit ID", self.unit_id.trim().to_owned()));
        }
        if !self.device_id.trim().is_empty() {
            rows.push(("Device ID", self.device_id.trim().to_owned()));
        }
        rows.push((
            "Auto-Scan",
            match (self.auto_scan, self.protocol == "modbus") {
                (false, _) => "No".to_owned(),
                (true, true) => format!(
                    "Yes ({} {}-{})",
                    self.register_area,
                    self.start.trim(),
                    self.end.trim()
                ),
                (true, false) => "Not available for this protocol".to_owned(),
            },
        ));
        rows.push(("Dashboard", yes_no(self.dashboard)));
        rows.push(("Alarms", yes_no(self.alarms)));
        rows.push(("Equipment Instances", self.instances.trim().to_owned()));
        rows
    }
}

fn yes_no(value: bool) -> String {
    (if value { "Yes" } else { "No" }).to_owned()
}

/// The quick start's dotted IPv4 address check (`QuickStartGuide.tsx`).
pub fn valid_ipv4(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            (1..=3).contains(&part.len())
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u16>().is_ok_and(|octet| octet <= 255)
        })
}

/// The quick start's body: Modbus at `ip`, scan holding registers 0–499,
/// no template, as `QuickStartGuide.tsx` sends it.
pub fn quick_start_config(ip: &str, port: &str, unit_id: &str) -> Result<WizardConfig, String> {
    let ip = ip.trim();
    if !valid_ipv4(ip) {
        return Err("Enter the PLC's IPv4 address, e.g. 192.168.1.100".to_owned());
    }
    let port = port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| "Port must be a whole number from 1 to 65535".to_owned())?;
    let unit_id = unit_id
        .trim()
        .parse::<u8>()
        .map_err(|_| "Unit ID must be a whole number from 0 to 255".to_owned())?;
    Ok(WizardConfig {
        template_id: None,
        template_source: "vertical".to_owned(),
        industry: String::new(),
        device: WizardDevice {
            id: None,
            protocol: "modbus".to_owned(),
            address: ip.to_owned(),
            port,
            unit_id,
            auto_scan: true,
            scan: Some(WizardScan {
                register_area: "holding".to_owned(),
                start_address: 0,
                end_address: 499,
                scan_delay_ms: 10,
            }),
        },
        instance_counts: instance_counts(4),
        generate_dashboard: true,
        generate_alarms: true,
    })
}

// ── Health: GET /api/v1/wizard/health-check ─────────────────────────────

/// `wizard::health::HealthReport`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HealthReport {
    pub overall_score: u32,
    pub issues: Vec<HealthIssue>,
    pub summary: HealthSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HealthIssue {
    pub severity: HealthSeverity,
    pub category: String,
    pub title: String,
    pub description: String,
    pub suggestion: String,
    pub affected_items: Vec<String>,
    pub count: u32,
}

impl HealthIssue {
    pub fn matches(&self, query: &str) -> bool {
        matches_any(
            query,
            &[
                self.title.as_str(),
                self.description.as_str(),
                self.category.as_str(),
                self.severity.id(),
            ],
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthSeverity {
    Critical,
    Warning,
    #[default]
    Info,
    Pass,
}

impl HealthSeverity {
    pub fn id(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Pass => "pass",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HealthSummary {
    pub total_devices: u32,
    pub total_tags: u32,
    pub total_alarms: u32,
    pub total_flows: u32,
    pub total_screens: u32,
    pub critical_issues: u32,
    pub warning_issues: u32,
    pub info_issues: u32,
    pub tags_without_alarms: u32,
    pub tags_without_units: u32,
    pub unnamed_devices: u32,
    pub devices_without_tags: u32,
    pub duplicate_tag_names: u32,
    pub nonstandard_tag_names: u32,
    pub missing_dashboards: u32,
    pub disabled_devices: u32,
}

impl HealthSummary {
    /// The web panel's six counters, each with the severity it reads in
    /// while not zero.
    pub fn counters(&self) -> [(&'static str, u32, HealthSeverity); 6] {
        let level = |count: u32, severity| {
            if count > 0 {
                severity
            } else {
                HealthSeverity::Pass
            }
        };
        [
            (
                "Tags without Alarms",
                self.tags_without_alarms,
                level(self.tags_without_alarms, HealthSeverity::Warning),
            ),
            (
                "Tags without Units",
                self.tags_without_units,
                level(self.tags_without_units, HealthSeverity::Info),
            ),
            (
                "Unnamed Devices",
                self.unnamed_devices,
                level(self.unnamed_devices, HealthSeverity::Warning),
            ),
            (
                "Empty Devices",
                self.devices_without_tags,
                level(self.devices_without_tags, HealthSeverity::Warning),
            ),
            (
                "Duplicate Names",
                self.duplicate_tag_names,
                level(self.duplicate_tag_names, HealthSeverity::Warning),
            ),
            (
                "Non-standard Names",
                self.nonstandard_tag_names,
                level(self.nonstandard_tag_names, HealthSeverity::Info),
            ),
        ]
    }
}

/// The score's verdict and how it reads: 80 and up healthy, 50 and up
/// room for improvement, below that needs attention.
pub fn health_verdict(score: u32) -> (&'static str, HealthSeverity) {
    if score >= 80 {
        ("Project is healthy", HealthSeverity::Pass)
    } else if score >= 50 {
        ("Room for improvement", HealthSeverity::Warning)
    } else {
        ("Needs attention", HealthSeverity::Critical)
    }
}

// ── SMTP: /api/v1/settings/smtp (operator) ──────────────────────────────

/// The server never returns the password: a configured server answers
/// this mask, and a PUT carrying it (or no password) keeps the stored one.
pub const PASSWORD_MASK: &str = "********";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Encryption {
    #[default]
    Starttls,
    Implicittls,
    None,
}

impl Encryption {
    pub const ALL: [Self; 3] = [Self::Starttls, Self::Implicittls, Self::None];

    pub fn label(self) -> &'static str {
        match self {
            Self::Starttls => "STARTTLS",
            Self::Implicittls => "Implicit TLS",
            Self::None => "None",
        }
    }
}

/// `GET /api/v1/settings/smtp`: `{"configured": false}`, or the settings
/// with the password masked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SmtpSettings {
    pub configured: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub encryption: Option<Encryption>,
}

/// `PUT /api/v1/settings/smtp`'s body: `SmtpUpdateRequest`. A missing
/// password, or the mask, keeps the stored one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SmtpUpdate {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
    pub encryption: Encryption,
}

impl SmtpSettings {
    /// The form's fields for these settings, as the web panel fills them:
    /// the mask stands in the password field of a configured server.
    pub fn draft(&self) -> SmtpDraft {
        if !self.configured {
            return SmtpDraft::default();
        }
        SmtpDraft {
            host: self.host.clone().unwrap_or_default(),
            port: self.port.unwrap_or(587).to_string(),
            username: self.username.clone().unwrap_or_default(),
            password: self
                .password
                .clone()
                .unwrap_or_else(|| PASSWORD_MASK.to_owned()),
            from_address: self.from_address.clone().unwrap_or_default(),
            from_name: self.from_name.clone().unwrap_or_default(),
            encryption: self.encryption.unwrap_or_default(),
        }
    }

    /// The PUT that puts these settings back as they are: every field as
    /// read and the mask for the password, so the stored password stays.
    /// `None` when the server has no SMTP settings (there is no route to
    /// remove them again).
    pub fn restore(&self) -> Option<SmtpUpdate> {
        self.configured.then(|| SmtpUpdate {
            host: self.host.clone().unwrap_or_default(),
            port: self.port.unwrap_or(587),
            username: self.username.clone().unwrap_or_default(),
            password: Some(PASSWORD_MASK.to_owned()),
            from_address: self.from_address.clone(),
            from_name: self.from_name.clone(),
            encryption: self.encryption.unwrap_or_default(),
        })
    }
}

/// The SMTP form as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpDraft {
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub from_address: String,
    pub from_name: String,
    pub encryption: Encryption,
}

impl Default for SmtpDraft {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: "587".to_owned(),
            username: String::new(),
            password: String::new(),
            from_address: String::new(),
            from_name: String::new(),
            encryption: Encryption::Starttls,
        }
    }
}

/// A plausible mail address: one `@` with text on both sides, a dot in
/// the domain, no spaces.
pub fn plausible_email(text: &str) -> bool {
    let text = text.trim();
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !text.chars().any(char::is_whitespace)
}

impl SmtpDraft {
    /// The PUT body, or what to fix first. The host is required (the
    /// server refuses an empty one), the port a number from 1 to 65535 and
    /// the sender address, if given, an address. As the web panel sends
    /// it: an empty password is left out (the server keeps the stored
    /// one), as are an empty sender address and name (the server's
    /// defaults).
    pub fn validate(&self) -> Result<SmtpUpdate, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err("Enter the SMTP host".to_owned());
        }
        if host.chars().any(char::is_whitespace) {
            return Err("The host has no spaces".to_owned());
        }
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| "Port must be a whole number from 1 to 65535".to_owned())?;
        let from_address = self.from_address.trim();
        if !from_address.is_empty() && !plausible_email(from_address) {
            return Err("The from address is not an email address".to_owned());
        }
        let non_empty = |text: &str| (!text.is_empty()).then(|| text.to_owned());
        Ok(SmtpUpdate {
            host: host.to_owned(),
            port,
            username: self.username.trim().to_owned(),
            password: non_empty(&self.password),
            from_address: non_empty(from_address),
            from_name: non_empty(self.from_name.trim()),
            encryption: self.encryption,
        })
    }

    /// Whether the password field still holds the mask: saving keeps the
    /// stored password.
    pub fn keeps_password(&self) -> bool {
        self.password.is_empty() || self.password == PASSWORD_MASK
    }
}

/// `POST /api/v1/settings/smtp/test`'s body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SmtpTestBody {
    pub recipient: String,
}

impl SmtpTestBody {
    pub fn new(recipient: &str) -> Result<Self, String> {
        let recipient = recipient.trim();
        if recipient.is_empty() {
            return Err("Enter a recipient".to_owned());
        }
        if !plausible_email(recipient) {
            return Err("The recipient is not an email address".to_owned());
        }
        Ok(Self {
            recipient: recipient.to_owned(),
        })
    }
}

/// `POST /api/v1/settings/smtp/test`'s answer: `{"ok": true, "message":
/// "Test email sent to …"}`; a failure is a server error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SmtpTestAnswer {
    pub ok: bool,
    pub message: String,
}

impl SmtpTestAnswer {
    pub fn text(&self) -> &str {
        if self.message.is_empty() {
            "Test email sent"
        } else {
            self.message.as_str()
        }
    }
}

// ── Tools ────────────────────────────────────────────────────────────────

/// The desktop tools the System panel launches, by URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub scheme: &'static str,
    pub name: &'static str,
    pub about: &'static str,
}

pub const TOOLS: [Tool; 2] = [
    Tool {
        scheme: "voltrus-plc",
        name: "Voltrus PLC",
        about: "IEC 61131-3 Structured Text editor with softPLC runtime. Write logic, compile, simulate, deploy.",
    },
    Tool {
        scheme: "voltrus-hmi",
        name: "Voltrus HMI",
        about: "Drag-and-drop HMI builder for industrial PLCs. Design operator dashboards, deploy as standalone HTML.",
    },
];

/// `voltrus-plc://open?scada=http%3A%2F%2Fplant%3A3000`: the tool opens
/// with this server as its SCADA context, as the web panel's link.
pub fn deep_link(scheme: &str, server: &str) -> String {
    format!(
        "{scheme}://open?scada={}",
        encode(server.trim_end_matches('/'))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode<T: serde::de::DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).unwrap_or_else(|error| panic!("{error}: {json}"))
    }

    #[test]
    fn system_info_decodes_the_servers_answer() {
        let info: SystemInfo = decode(
            r#"{"hostname":"plant-01","os":"Linux 6.8","uptime_secs":360061,"cpu_usage":12.5,
            "memory_total_mb":16000.0,"memory_used_mb":4000.0,"memory_percent":25.0,
            "disk_total_gb":512.0,"disk_used_gb":128.0,"disk_percent":25.0,"db_size_mb":1536.0,
            "process":{"pid":4242,"memory_mb":48.5,"cpu_percent":1.5,"uptime_secs":3600,"binary_size_mb":31.2}}"#,
        );
        assert_eq!(info.hostname, "plant-01");
        assert_eq!(info.process.pid, 4242);
        assert_eq!(uptime(info.uptime_secs), "4d 4h 1m");
        assert_eq!(
            db_size(info.db_size_mb),
            ("1.5 GB".to_owned(), "1536 MB".to_owned())
        );
        assert_eq!(
            db_size(12.34),
            ("12.3 MB".to_owned(), "SQLite file".to_owned())
        );
    }

    #[test]
    fn levels_follow_the_web_thresholds() {
        assert_eq!(Level::cpu(50.0), Level::Ok);
        assert_eq!(Level::cpu(50.1), Level::Warning);
        assert_eq!(Level::cpu(80.1), Level::Critical);
        assert_eq!(Level::bar(70.0), Level::Ok);
        assert_eq!(Level::bar(71.0), Level::Warning);
        assert_eq!(Level::bar(91.0), Level::Critical);
        assert!((fill(150.0) - 1.0).abs() < f32::EPSILON);
        assert!(fill(-3.0).abs() < f32::EPSILON);
        assert!((fill(25.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn docs_decode_the_servers_answers() {
        let topology: Topology = decode(
            r#"{"devices":[{"id":"inverter","name":"Inverter","protocol":"Modbus TCP",
            "address":"127.0.0.1:5020","enabled":true,"tag_count":14,"online":true}],
            "connections":[{"from":"voltrus","to":"inverter","protocol":"Modbus TCP",
            "address":"127.0.0.1:5020","port":5020}],
            "summary":{"total_devices":1,"total_tags":14,"protocols":[{"protocol":"Modbus TCP",
            "device_count":1,"tag_count":14}],"online_count":1,"offline_count":0}}"#,
        );
        assert_eq!(topology.devices[0].tag_count, 14);
        assert_eq!(topology.connections[0].port, 5020);
        assert_eq!(topology.summary.protocols[0].protocol, "Modbus TCP");

        let xref: TagXref = decode(
            r#"{"total_tags":2,"tags":[
            {"key":"ac_power","name":"AC power","device_id":"inverter","device_name":"Inverter",
             "protocol":"Modbus TCP","unit":"W","source_type":"modbus","data_type":"float32",
             "address":"40001","has_alarm":true,"alarm_severity":"critical",
             "alarm_threshold":"AC power high 59900","warn_val":59500.0,"crit_val":59900.0,"path":null},
            {"key":"irr","name":"Irradiance","device_id":"meter","device_name":"Meter",
             "protocol":"Simulation","unit":"W/m²","source_type":"simulated","data_type":null,
             "address":null,"has_alarm":false,"alarm_severity":null,"alarm_threshold":null,
             "warn_val":0.0,"crit_val":0.0,"path":"site/irr"}],
            "by_device":[{"device_id":"inverter","device_name":"Inverter","protocol":"Modbus TCP",
             "tag_count":1,"alarmed_count":1}],
            "by_protocol":[{"protocol":"Modbus TCP","device_count":0,"tag_count":1}]}"#,
        );
        assert_eq!(xref.tags[0].alarm_label(), "critical");
        assert_eq!(xref.tags[1].alarm_label(), "—");
        assert!(xref.tags[1].matches("METER"));
        assert!(!xref.tags[1].matches("inverter"));
        assert_eq!(xref.by_device[0].alarmed_count, 1);

        let automation: Automation = decode(
            r#"{"scripts":[{"id":3,"name":"Totaliser","source_preview":"let x = 1;","enabled":false,
            "syntax_error":"unexpected token","cpu_limit_ms":50,"memory_limit_bytes":1048576,
            "description":"Reads 2 tags","last_execution":null}],
            "flows":[{"id":"f1","name":"Pump start","enabled":true,
            "trigger_description":"When LT-1 goes above 7","node_count":2,
            "node_descriptions":["Write PMP1_RUN = 1","Notify"],"description":"Flow 'Pump start'",
            "last_run":{"success":true,"duration_ms":4,"error":null,"timestamp":1700000000}}],
            "summary":{"total_scripts":1,"enabled_scripts":0,"total_flows":1,"enabled_flows":1}}"#,
        );
        assert_eq!(automation.flows[0].actions(), "Write PMP1_RUN = 1; Notify");
        assert_eq!(script_status(&automation.scripts[0]), "Error");
        assert_eq!(
            automation.rows(""),
            vec![AutomationRow::Flow(0), AutomationRow::Script(0)]
        );
        assert_eq!(automation.rows("total"), vec![AutomationRow::Script(0)]);
    }

    #[test]
    fn templates_decode_and_bodies_follow_the_web_form() {
        let list: TemplateList = decode(
            r#"{"templates":[{"id":"car_wash","name":"Car Wash System","industry":"car_wash",
            "description":"Tunnel car wash","equipment_type":"carwash","tag_count":14,
            "alarm_count":4,"flow_count":1,"has_oee":true}],
            "industries":[{"id":"car_wash","name":"Car Wash","description":"Tunnel and in-place"}]}"#,
        );
        assert_eq!(list.industry_name("car_wash"), "Car Wash");
        assert_eq!(list.industry_name("maritime"), "maritime");

        let detail: VerticalTemplate = decode(
            r##"{"id":"car_wash","name":"Car Wash System","industry":"car_wash","description":"d",
            "equipment_type":"carwash",
            "tags":[{"name_pattern":"CNV_RUN","description":"Conveyor running","unit":"",
              "data_type":"bool","register_area":"coil","scale":1.0,"min_val":0.0,"max_val":1.0,
              "group":"conveyor","widget":"led","interval_ms":1000}],
            "alarms":[{"tag_pattern":"CHEM1_LVL","severity":"warning","comparison":"below",
              "threshold":10.0,"deadband":1.0,"message_template":"Chemical {n} low"}],
            "dashboard":{"layout_groups":[{"name":"Conveyor","group_type":"motor","columns":2}],
              "screen_name":"Car Wash"},
            "flows":[{"name":"Low chemical tank alert","trigger_type":"tag_threshold",
              "trigger_config":{"tag":"CHEM1_LVL"},"nodes":[],"description":"Alert"}],
            "oee_config":{"availability_tags":["CNV_RUN"],"performance_tags":[],"quality_tags":[],
              "ideal_cycle_time_secs":null}}"##,
        );
        assert_eq!(detail.tags[0].interval_ms, 1000);
        assert!(detail.oee_config.is_some());

        let preview: TemplatePreview = decode(
            r#"{"template_id":"car_wash","expanded_tags":[{"name":"CNV_RUN","description":"c",
            "unit":"","data_type":"bool","group":"conveyor","widget":"led"}],
            "expanded_alarms":[{"tag_pattern":"CHEM1_LVL","severity":"warning","comparison":"below",
            "threshold":10.0,"message_template":"low"}],
            "flows":[{"name":"Low chemical tank alert","description":"Alert","trigger_type":"tag_threshold"}]}"#,
        );
        assert_eq!(preview.expanded_tags.len(), 1);
        assert_eq!(preview.flows[0].trigger_type, "tag_threshold");

        let done: Instantiated = decode(
            r#"{"template_id":"car_wash","template_name":"Car Wash System","device_id":"vt_north",
            "screen_id":"car_wash_north","tags_created":14,"alarms_created":4,"flows_created":1,
            "tags":["CNV_RUN"],"alarms":["CHEM1_LVL: warning below 10"],"flows":["Low chemical tank alert"]}"#,
        );
        assert_eq!(
            done.summary(),
            "Created: 14 tags, 4 alarms, 1 flows on device \"vt_north\""
        );

        assert_eq!(
            serde_json::to_value(PreviewBody::new("car_wash", "").unwrap()).unwrap(),
            serde_json::json!({"template_id": "car_wash", "instance_counts": {"n": 2}})
        );
        assert_eq!(
            serde_json::to_value(InstantiateBody::new("car_wash", "  North bay ", "3").unwrap())
                .unwrap(),
            serde_json::json!({"template_id": "car_wash", "name": "North bay", "instance_counts": {"n": 3}})
        );
        assert!(InstantiateBody::new("car_wash", "  ", "3").is_err());
        assert!(parse_instances("0").is_err());
        assert!(parse_instances("21").is_err());
        assert!(parse_instances("two").is_err());
        assert_eq!(instance_device_id("North Bay"), "vt_north_bay");
        assert_eq!(templates_path(None), "/api/v1/templates");
        assert_eq!(templates_path(Some("")), "/api/v1/templates");
        assert_eq!(
            templates_path(Some("car_wash")),
            "/api/v1/templates?industry=car_wash"
        );
        assert_eq!(
            instantiate_path("car wash"),
            "/api/v1/templates/car%20wash/instantiate"
        );
    }

    #[test]
    fn feed_templates_and_wizard_answers_decode() {
        let list: FeedTemplateList = decode(
            r#"{"templates":[{"id":"feed-pump-station","name":"Booster Pump Station",
            "industry":"Water/Wastewater","description":"d","estimated_tag_count":48,
            "equipment_units":["Wet well","Pumps"]}]}"#,
        );
        assert_eq!(list.templates[0].equipment_units.len(), 2);
        let feed: FeedTemplate = decode(
            r#"{"id":"feed-pump-station","name":"Booster Pump Station","industry":"Water/Wastewater",
            "description":"d","equipment_type":"pump_station","estimated_tag_count":48,
            "equipment_units":[{"name":"Pumps","quantity":"1-4","description":"Duty pumps"}],
            "io_list":[{"signal":"LT-101","description":"Wet well level","io_type":"AI","unit":"m",
              "data_type":"float32","typical_range":"0-10","equipment":"Wet well",
              "criticality":"critical","notes":""}],
            "alarm_strategy":[{"equipment":"Wet well","rules":[{"signal_pattern":"LT-101",
              "severity":"critical","condition":"high-high","setpoint":"90%","deadband":"2%",
              "description":"Overflow","alarm_class":"safety"}]}],
            "dashboard":{"description":"One screen","recommended_screens":[{"name":"Overview",
              "layout":"grid","sections":[{"title":"Level","equipment_group":"Wet well",
              "widget_type":"gauge","signals":["LT-101"]}]}]},
            "network_notes":null}"#,
        );
        assert_eq!(feed.io_list[0].io_type, "AI");
        assert_eq!(feed.alarm_strategy[0].rules[0].setpoint, "90%");
        let result: WizardResult = decode(
            r#"{"success":true,"device_id":"simulation_energy","tags_created":0,"alarms_created":0,
            "flows_created":0,"screens_created":0,"tags_discovered":0,
            "warnings":["No template selected and auto-scan disabled."],"next_steps":[]}"#,
        );
        assert!(result.success);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn wizard_steps_skip_the_scan_unless_modbus_scans() {
        let mut draft = WizardDraft::default();
        assert!(
            draft.next(WizardStep::Industry).is_err(),
            "an industry first"
        );
        draft.industry = "Energy".to_owned();
        assert_eq!(draft.next(WizardStep::Industry), Ok(WizardStep::Template));
        assert_eq!(draft.next(WizardStep::Template), Ok(WizardStep::Device));
        assert_eq!(draft.next(WizardStep::Device), Ok(WizardStep::Scan));
        assert_eq!(draft.back(WizardStep::Review), Some(WizardStep::Scan));
        draft.set_protocol("opcua");
        assert_eq!(draft.port, "4840", "a protocol takes its default port");
        assert_eq!(draft.next(WizardStep::Device), Ok(WizardStep::Review));
        assert_eq!(draft.back(WizardStep::Review), Some(WizardStep::Device));
        assert_eq!(draft.back(WizardStep::Industry), None);
        draft.port = "70000".to_owned();
        assert!(draft.next(WizardStep::Device).is_err());
        draft.set_protocol("modbus");
        draft.start = "500".to_owned();
        draft.end = "10".to_owned();
        assert!(draft.next(WizardStep::Scan).is_err(), "end before start");
        draft.end = "509".to_owned();
        assert_eq!(draft.scan_range().unwrap().count(), 10);
    }

    #[test]
    fn wizard_body_matches_the_web_wizard() {
        let mut draft = WizardDraft {
            industry: "Energy".to_owned(),
            template: Some("hydroelectric".to_owned()),
            ..WizardDraft::default()
        };
        assert_eq!(
            serde_json::to_value(draft.config().unwrap()).unwrap(),
            serde_json::json!({
                "template_id": "hydroelectric",
                "template_source": "vertical",
                "industry": "Energy",
                "device": {
                    "id": null, "protocol": "modbus", "address": "192.168.1.100", "port": 502,
                    "unit_id": 1, "auto_scan": true,
                    "scan": {"register_area": "holding", "start_address": 0, "end_address": 499, "scan_delay_ms": 10}
                },
                "instance_counts": {"n": 4},
                "generate_dashboard": true,
                "generate_alarms": true
            })
        );
        // A FEED template is a reference design: no template goes.
        draft.source = TemplateSource::Feed;
        draft.template = Some("feed-pump-station".to_owned());
        draft.set_protocol("simulation");
        draft.auto_scan = false;
        draft.device_id = " e2e_probe ".to_owned();
        let config = draft.config().unwrap();
        assert_eq!(config.template_id, None);
        assert_eq!(config.template_source, "feed");
        assert_eq!(config.device.id.as_deref(), Some("e2e_probe"));
        assert!(!config.device.auto_scan && config.device.scan.is_none());
        let review = draft.review();
        assert!(review.contains(&("Template", "FEED: feed-pump-station".to_owned())));
        assert!(review.contains(&("Auto-Scan", "No".to_owned())));
    }

    #[test]
    fn quick_start_checks_the_address() {
        assert!(valid_ipv4("192.168.1.100"));
        assert!(!valid_ipv4("192.168.1"));
        assert!(!valid_ipv4("192.168.1.256"));
        assert!(!valid_ipv4("a.b.c.d"));
        let config = quick_start_config("10.0.0.5", "502", "1").unwrap();
        assert_eq!(config.device.address, "10.0.0.5");
        assert!(config.device.auto_scan);
        assert_eq!(config.instance_counts, instance_counts(4));
        assert!(quick_start_config("10.0.0.5", "0", "1").is_err());
        assert!(quick_start_config("10.0.0.5", "502", "300").is_err());
    }

    #[test]
    fn health_decodes_and_reads() {
        let report: HealthReport = decode(
            r#"{"overall_score":64,"issues":[{"severity":"warning","category":"alarms",
            "title":"Tags without alarms","description":"12 tags have no limit",
            "suggestion":"Set limits","affected_items":["ac_power"],"count":12},
            {"severity":"pass","category":"naming","title":"Names","description":"ok",
            "suggestion":"","affected_items":[],"count":0}],
            "summary":{"total_devices":2,"total_tags":28,"total_alarms":3,"total_flows":0,
            "total_screens":1,"critical_issues":0,"warning_issues":1,"info_issues":0,
            "tags_without_alarms":12,"tags_without_units":0,"unnamed_devices":0,
            "devices_without_tags":0,"duplicate_tag_names":0,"nonstandard_tag_names":3,
            "missing_dashboards":0,"disabled_devices":0}}"#,
        );
        assert_eq!(report.issues[0].severity, HealthSeverity::Warning);
        assert_eq!(report.issues[1].severity, HealthSeverity::Pass);
        assert!(report.issues[0].matches("alarm"));
        let counters = report.summary.counters();
        assert_eq!(
            counters[0],
            ("Tags without Alarms", 12, HealthSeverity::Warning)
        );
        assert_eq!(counters[1].2, HealthSeverity::Pass);
        assert_eq!(counters[5].2, HealthSeverity::Info);
        assert_eq!(health_verdict(64).0, "Room for improvement");
        assert_eq!(health_verdict(80).1, HealthSeverity::Pass);
        assert_eq!(health_verdict(10).1, HealthSeverity::Critical);
    }

    #[test]
    fn smtp_settings_decode_and_restore_keeps_the_password() {
        let none: SmtpSettings = decode(r#"{"configured":false}"#);
        assert!(!none.configured);
        assert_eq!(none.draft(), SmtpDraft::default());
        assert!(none.restore().is_none(), "nothing to put back");

        let set: SmtpSettings = decode(
            r#"{"configured":true,"host":"mail.plant.local","port":2525,"username":"ci",
            "password":"********","from_address":"scada@plant.local","from_name":"Plant",
            "encryption":"none"}"#,
        );
        let draft = set.draft();
        assert_eq!(draft.password, PASSWORD_MASK);
        assert!(draft.keeps_password());
        assert_eq!(draft.encryption, Encryption::None);
        assert_eq!(
            serde_json::to_value(set.restore().unwrap()).unwrap(),
            serde_json::json!({
                "host": "mail.plant.local", "port": 2525, "username": "ci",
                "password": "********", "from_address": "scada@plant.local",
                "from_name": "Plant", "encryption": "none"
            })
        );
        // The form's body round-trips the same settings.
        assert_eq!(draft.validate(), Ok(set.restore().unwrap()));
    }

    #[test]
    fn smtp_form_validation_and_body() {
        let mut draft = SmtpDraft::default();
        assert_eq!(draft.validate(), Err("Enter the SMTP host".to_owned()));
        draft.host = " smtp.example.com ".to_owned();
        draft.port = "0".to_owned();
        assert!(draft.validate().is_err());
        draft.port = "465".to_owned();
        draft.from_address = "not-an-address".to_owned();
        assert!(draft.validate().is_err());
        draft.from_address = String::new();
        draft.encryption = Encryption::Implicittls;
        // Empty password, sender address and name are left out.
        assert_eq!(
            serde_json::to_value(draft.validate().unwrap()).unwrap(),
            serde_json::json!({
                "host": "smtp.example.com", "port": 465, "username": "",
                "encryption": "implicittls"
            })
        );
        draft.password = "s3cret".to_owned();
        assert!(!draft.keeps_password());
        assert_eq!(
            draft.validate().unwrap().password.as_deref(),
            Some("s3cret")
        );
        assert!(SmtpTestBody::new("").is_err());
        assert!(SmtpTestBody::new("ops at plant").is_err());
        assert_eq!(
            serde_json::to_value(SmtpTestBody::new(" ops@plant.local ").unwrap()).unwrap(),
            serde_json::json!({"recipient": "ops@plant.local"})
        );
        let answer: SmtpTestAnswer =
            decode(r#"{"ok":true,"message":"Test email sent to ops@plant.local"}"#);
        assert_eq!(answer.text(), "Test email sent to ops@plant.local");
        assert_eq!(SmtpTestAnswer::default().text(), "Test email sent");
        assert!(plausible_email("a@b.co"));
        assert!(!plausible_email("a@b"));
        assert!(!plausible_email("@b.co"));
    }

    #[test]
    fn deep_links_carry_the_server() {
        assert_eq!(
            deep_link("voltrus-plc", "http://plant.local:3000/"),
            "voltrus-plc://open?scada=http%3A%2F%2Fplant.local%3A3000"
        );
        assert_eq!(protocol("bacnet").port, 47808);
        assert_eq!(protocol("nope").id, "modbus");
    }
}
