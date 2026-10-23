use serde::{Deserialize, Serialize};
use serde_json::json;

// ── Template data structures ──

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub oee_config: Option<TemplateOeeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateTag {
    /// ISA-5.1 compliant tag name pattern, e.g. "PMP{n}_RUN"
    pub name_pattern: String,
    /// Human-readable description
    pub description: String,
    /// Unit of measurement
    pub unit: String,
    /// Data type: bool, uint16, int16, float32, float64
    pub data_type: String,
    /// Default register area for Modbus: coil, discrete, input, holding
    pub register_area: String,
    /// Scale factor
    pub scale: f64,
    /// Min value for display
    pub min_val: f64,
    /// Max value for display
    pub max_val: f64,
    /// Group for dashboard layout (e.g. "pumps", "sensors", "tanks")
    pub group: String,
    /// Icon for dashboard widget: gauge, led, flow, level, pressure, temperature
    pub widget: String,
    /// Default polling interval in ms
    pub interval_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateAlarm {
    /// Tag name pattern this alarm monitors
    pub tag_pattern: String,
    /// Alarm severity: critical, warning, info
    pub severity: String,
    /// Comparison: above, below, equal, not_equal, change
    pub comparison: String,
    /// Threshold value
    pub threshold: f64,
    /// Deadband to prevent alarm floods
    pub deadband: f64,
    /// Human-readable alarm message template
    pub message_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDashboard {
    /// Dashboard layout description for auto-generation
    pub layout_groups: Vec<DashboardGroup>,
    /// Suggested screen name
    pub screen_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardGroup {
    pub name: String,
    pub group_type: String, // "pumps", "tanks", "flow", "pressure", "level", "chemical", "safety", "motor", "navigation", "position", "turbine", "gate"
    pub columns: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateFlow {
    /// Flow rule name
    pub name: String,
    /// Trigger type: tag_threshold, schedule, alarm
    pub trigger_type: String,
    /// Trigger config as JSON
    pub trigger_config: serde_json::Value,
    /// Nodes as JSON
    pub nodes: serde_json::Value,
    /// Description of what this flow does
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateOeeConfig {
    pub availability_tags: Vec<String>,
    pub performance_tags: Vec<String>,
    pub quality_tags: Vec<String>,
    pub ideal_cycle_time_secs: Option<f64>,
}

// ── Vertical Template Definitions ──

pub fn all_templates() -> Vec<VerticalTemplate> {
    vec![
        wastewater_treatment(),
        car_wash(),
        maritime(),
        elevator_ski_lift(),
        hydroelectric(),
    ]
}

pub fn get_template(id: &str) -> Option<VerticalTemplate> {
    all_templates().into_iter().find(|t| t.id == id)
}

pub fn templates_by_industry(industry: &str) -> Vec<VerticalTemplate> {
    all_templates()
        .into_iter()
        .filter(|t| t.industry == industry)
        .collect()
}

/// All available industry categories
pub fn industries() -> Vec<IndustryInfo> {
    vec![
        IndustryInfo {
            id: "water_wastewater".into(),
            name: "Water & Wastewater".into(),
            description: "Municipal water treatment, pump stations, lift stations".into(),
        },
        IndustryInfo {
            id: "car_wash".into(),
            name: "Car Wash".into(),
            description: "Tunnel and in-place car wash systems".into(),
        },
        IndustryInfo {
            id: "maritime".into(),
            name: "Maritime".into(),
            description: "Shipboard controls, bilge pumps, engine monitoring".into(),
        },
        IndustryInfo {
            id: "elevators_lifts".into(),
            name: "Elevators & Lifts".into(),
            description: "Elevators, ski lifts, escalators, people movers".into(),
        },
        IndustryInfo {
            id: "hydroelectric".into(),
            name: "Hydroelectric".into(),
            description: "Turbine control, gate position, grid sync".into(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndustryInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

// ── Wastewater Treatment ──

fn wastewater_treatment() -> VerticalTemplate {
    VerticalTemplate {
        id: "wastewater_treatment".into(),
        name: "Wastewater Treatment Plant".into(),
        industry: "water_wastewater".into(),
        description: "Complete wastewater treatment monitoring: pump stations, flow meters, chlorine dosing, lift stations. Designed for municipal operators with ISA-5.1 tag naming.".into(),
        equipment_type: "wwtp".into(),
        tags: vec![
            // Influent pumps
            TemplateTag { name_pattern: "PMP{n}_RUN".into(), description: "Influent pump {n} running status".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "pumps".into(), widget: "led".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "PMP{n}_FLT".into(), description: "Influent pump {n} fault".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "pumps".into(), widget: "led".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "PMP{n}_AMP".into(), description: "Influent pump {n} current draw".into(), unit: "A".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "pumps".into(), widget: "gauge".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "PMP{n}_HZ".into(), description: "Influent pump {n} VFD frequency".into(), unit: "Hz".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 60.0, group: "pumps".into(), widget: "gauge".into(), interval_ms: 2000 },
            // Flow meters
            TemplateTag { name_pattern: "FT-{n}".into(), description: "Flow transmitter {n}".into(), unit: "L/s".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "flow".into(), widget: "flow".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "FT-{n}_TOT".into(), description: "Flow totalizer {n}".into(), unit: "m³".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 999999.0, group: "flow".into(), widget: "gauge".into(), interval_ms: 5000 },
            // Level sensors
            TemplateTag { name_pattern: "LT-{n}".into(), description: "Level transmitter {n} (wet well / tank)".into(), unit: "m".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 10.0, group: "tanks".into(), widget: "level".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "LT-{n}_HH".into(), description: "Level high-high switch {n}".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "tanks".into(), widget: "led".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "LT-{n}_LL".into(), description: "Level low-low switch {n}".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "tanks".into(), widget: "led".into(), interval_ms: 1000 },
            // Chlorine dosing
            TemplateTag { name_pattern: "CLO2_DOS".into(), description: "Chlorine dioxide dosing rate".into(), unit: "mg/L".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 5.0, group: "chemical".into(), widget: "gauge".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "CLO2_RES".into(), description: "Residual chlorine".into(), unit: "mg/L".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 2.0, group: "chemical".into(), widget: "gauge".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "PH-{n}".into(), description: "pH measurement {n}".into(), unit: "pH".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 14.0, group: "chemical".into(), widget: "gauge".into(), interval_ms: 2000 },
            // Dissolved oxygen
            TemplateTag { name_pattern: "DO-{n}".into(), description: "Dissolved oxygen {n}".into(), unit: "mg/L".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 15.0, group: "chemical".into(), widget: "gauge".into(), interval_ms: 2000 },
        ],
        alarms: vec![
            TemplateAlarm { tag_pattern: "PMP{n}_FLT".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 1.0, deadband: 0.0, message_template: "Pump {n} fault detected".into() },
            TemplateAlarm { tag_pattern: "LT-{n}_HH".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 1.0, deadband: 0.0, message_template: "Level high-high alarm on sensor {n}".into() },
            TemplateAlarm { tag_pattern: "LT-{n}_LL".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 1.0, deadband: 0.0, message_template: "Level low-low alarm on sensor {n}".into() },
            TemplateAlarm { tag_pattern: "LT-{n}".into(), severity: "warning".into(), comparison: "above".into(), threshold: 8.0, deadband: 0.5, message_template: "Level high on sensor {n}: {value}m".into() },
            TemplateAlarm { tag_pattern: "PH-{n}".into(), severity: "critical".into(), comparison: "above".into(), threshold: 9.0, deadband: 0.5, message_template: "pH exceedance on sensor {n}: {value}".into() },
            TemplateAlarm { tag_pattern: "PH-{n}".into(), severity: "warning".into(), comparison: "below".into(), threshold: 6.0, deadband: 0.5, message_template: "pH low on sensor {n}: {value}".into() },
            TemplateAlarm { tag_pattern: "CLO2_RES".into(), severity: "warning".into(), comparison: "below".into(), threshold: 0.2, deadband: 0.05, message_template: "Residual chlorine low: {value} mg/L".into() },
            TemplateAlarm { tag_pattern: "DO-{n}".into(), severity: "warning".into(), comparison: "below".into(), threshold: 2.0, deadband: 0.5, message_template: "Dissolved oxygen low on sensor {n}: {value} mg/L".into() },
            TemplateAlarm { tag_pattern: "PMP{n}_AMP".into(), severity: "warning".into(), comparison: "above".into(), threshold: 85.0, deadband: 5.0, message_template: "Pump {n} high current draw: {value}A".into() },
        ],
        dashboard: TemplateDashboard {
            screen_name: "WWTP Overview".into(),
            layout_groups: vec![
                DashboardGroup { name: "Influent Pumps".into(), group_type: "pumps".into(), columns: 4 },
                DashboardGroup { name: "Flow Meters".into(), group_type: "flow".into(), columns: 2 },
                DashboardGroup { name: "Tank Levels".into(), group_type: "tanks".into(), columns: 3 },
                DashboardGroup { name: "Chemical Analysis".into(), group_type: "chemical".into(), columns: 4 },
            ],
        },
        flows: vec![
            TemplateFlow {
                name: "Auto pump start on high level".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "LT-1", "comparison": "above", "threshold": 7.0, "holdoff_secs": 60}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "High level detected — checking pump availability"},
                    {"type": "action", "action": "webhook", "message": "Wet well level high — auto-start sequence initiated"}
                ]),
                description: "When wet well level exceeds 7.0m, log event and send webhook notification for pump activation review".into(),
            },
            TemplateFlow {
                name: "Low chlorine alarm notification".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "CLO2_RES", "comparison": "below", "threshold": 0.3, "holdoff_secs": 120}),
                nodes: json!([
                    {"type": "action", "action": "email", "subject": "Low Chlorine Alert", "message": "Residual chlorine below 0.3 mg/L"},
                    {"type": "action", "action": "webhook", "message": "Chlorine dosing may need adjustment"}
                ]),
                description: "Notify operators when residual chlorine drops below safe threshold".into(),
            },
        ],
        oee_config: Some(TemplateOeeConfig {
            availability_tags: vec!["PMP1_RUN".into(), "PMP2_RUN".into()],
            performance_tags: vec!["FT-1".into()],
            quality_tags: vec!["CLO2_RES".into(), "PH-1".into(), "DO-1".into()],
            ideal_cycle_time_secs: None,
        }),
    }
}

// ── Car Wash ──

fn car_wash() -> VerticalTemplate {
    VerticalTemplate {
        id: "car_wash".into(),
        name: "Car Wash System".into(),
        industry: "car_wash".into(),
        description: "Tunnel and in-place car wash monitoring: conveyor control, chemical dosing, water recycling, payment integration. Budget-friendly SCADA for car wash operators.".into(),
        equipment_type: "carwash".into(),
        tags: vec![
            // Conveyor
            TemplateTag { name_pattern: "CNV_RUN".into(), description: "Conveyor running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "conveyor".into(), widget: "led".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "CNV_SPD".into(), description: "Conveyor speed".into(), unit: "m/min".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 20.0, group: "conveyor".into(), widget: "gauge".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "CNV_MTR_AMP".into(), description: "Conveyor motor current".into(), unit: "A".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 50.0, group: "conveyor".into(), widget: "gauge".into(), interval_ms: 1000 },
            // Chemical dosing
            TemplateTag { name_pattern: "CHEM{n}_PUMP".into(), description: "Chemical pump {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "chemical".into(), widget: "led".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "CHEM{n}_FLOW".into(), description: "Chemical {n} flow rate".into(), unit: "mL/min".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "chemical".into(), widget: "flow".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "CHEM{n}_LVL".into(), description: "Chemical {n} tank level".into(), unit: "%".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "chemical".into(), widget: "level".into(), interval_ms: 5000 },
            // Water system
            TemplateTag { name_pattern: "WR_INLET_TMP".into(), description: "Water inlet temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 80.0, group: "water".into(), widget: "temperature".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "WR_PRESS".into(), description: "Water pressure".into(), unit: "bar".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 10.0, group: "water".into(), widget: "pressure".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "WR_FLOW".into(), description: "Total water flow".into(), unit: "L/min".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 200.0, group: "water".into(), widget: "flow".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "WR_RECYCLE_PCT".into(), description: "Water recycling percentage".into(), unit: "%".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "water".into(), widget: "gauge".into(), interval_ms: 5000 },
            // Payment
            TemplateTag { name_pattern: "PAY_COUNT".into(), description: "Transaction count today".into(), unit: "".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 9999.0, group: "payment".into(), widget: "gauge".into(), interval_ms: 5000 },
            TemplateTag { name_pattern: "PAY_REVENUE".into(), description: "Revenue today".into(), unit: "$".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 99999.0, group: "payment".into(), widget: "gauge".into(), interval_ms: 10000 },
            // Brushes/blowers
            TemplateTag { name_pattern: "BR{n}_RUN".into(), description: "Brush/blower {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "equipment".into(), widget: "led".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "DRYER{n}_RUN".into(), description: "Dryer {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "equipment".into(), widget: "led".into(), interval_ms: 500 },
        ],
        alarms: vec![
            TemplateAlarm { tag_pattern: "CNV_MTR_AMP".into(), severity: "critical".into(), comparison: "above".into(), threshold: 40.0, deadband: 5.0, message_template: "Conveyor motor overcurrent: {value}A".into() },
            TemplateAlarm { tag_pattern: "WR_PRESS".into(), severity: "warning".into(), comparison: "below".into(), threshold: 2.0, deadband: 0.5, message_template: "Low water pressure: {value} bar".into() },
            TemplateAlarm { tag_pattern: "WR_PRESS".into(), severity: "critical".into(), comparison: "above".into(), threshold: 8.0, deadband: 0.5, message_template: "High water pressure: {value} bar".into() },
            TemplateAlarm { tag_pattern: "CHEM{n}_LVL".into(), severity: "warning".into(), comparison: "below".into(), threshold: 15.0, deadband: 5.0, message_template: "Chemical tank {n} low: {value}%".into() },
            TemplateAlarm { tag_pattern: "WR_INLET_TMP".into(), severity: "warning".into(), comparison: "below".into(), threshold: 30.0, deadband: 5.0, message_template: "Water temperature low: {value}°C".into() },
        ],
        dashboard: TemplateDashboard {
            screen_name: "Car Wash Overview".into(),
            layout_groups: vec![
                DashboardGroup { name: "Conveyor".into(), group_type: "conveyor".into(), columns: 3 },
                DashboardGroup { name: "Chemical Dosing".into(), group_type: "chemical".into(), columns: 3 },
                DashboardGroup { name: "Water System".into(), group_type: "water".into(), columns: 4 },
                DashboardGroup { name: "Brushes & Dryers".into(), group_type: "equipment".into(), columns: 4 },
                DashboardGroup { name: "Revenue".into(), group_type: "payment".into(), columns: 2 },
            ],
        },
        flows: vec![
            TemplateFlow {
                name: "Low chemical tank alert".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "CHEM1_LVL", "comparison": "below", "threshold": 10.0, "holdoff_secs": 300}),
                nodes: json!([
                    {"type": "action", "action": "webhook", "message": "Chemical tank 1 critically low — reorder needed"}
                ]),
                description: "Alert when chemical tank drops below 10% for restocking".into(),
            },
        ],
        oee_config: Some(TemplateOeeConfig {
            availability_tags: vec!["CNV_RUN".into()],
            performance_tags: vec!["CNV_SPD".into(), "WR_FLOW".into()],
            quality_tags: vec!["WR_PRESS".into(), "WR_INLET_TMP".into()],
            ideal_cycle_time_secs: Some(180.0), // 3 min per car
        }),
    }
}

// ── Maritime ──

fn maritime() -> VerticalTemplate {
    VerticalTemplate {
        id: "maritime".into(),
        name: "Maritime / Shipboard Controls".into(),
        industry: "maritime".into(),
        description: "Shipboard monitoring: bilge pumps, engine parameters, navigation lights, tank levels. Edge-friendly, offline-capable design for vessels at sea.".into(),
        equipment_type: "ship".into(),
        tags: vec![
            // Engine monitoring
            TemplateTag { name_pattern: "ENG{n}_RPM".into(), description: "Engine {n} RPM".into(), unit: "RPM".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 3000.0, group: "engine".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "ENG{n}_TMP".into(), description: "Engine {n} temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 120.0, group: "engine".into(), widget: "temperature".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "ENG{n}_OIL_P".into(), description: "Engine {n} oil pressure".into(), unit: "bar".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 10.0, group: "engine".into(), widget: "pressure".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "ENG{n}_COOL_TMP".into(), description: "Engine {n} coolant temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "engine".into(), widget: "temperature".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "ENG{n}_FUEL_FLOW".into(), description: "Engine {n} fuel flow".into(), unit: "L/h".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "engine".into(), widget: "flow".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "ENG{n}_RUN".into(), description: "Engine {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "engine".into(), widget: "led".into(), interval_ms: 500 },
            // Bilge pumps
            TemplateTag { name_pattern: "BILGE{n}_RUN".into(), description: "Bilge pump {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "bilge".into(), widget: "led".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "BILGE{n}_LVL".into(), description: "Bilge {n} water level".into(), unit: "%".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "bilge".into(), widget: "level".into(), interval_ms: 2000 },
            // Navigation lights
            TemplateTag { name_pattern: "NAV_{light}".into(), description: "Navigation light {light} status".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "navigation".into(), widget: "led".into(), interval_ms: 2000 },
            // Tank levels
            TemplateTag { name_pattern: "TK-{n}_LVL".into(), description: "Tank {n} level".into(), unit: "%".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "tanks".into(), widget: "level".into(), interval_ms: 5000 },
            TemplateTag { name_pattern: "TK-{n}_TMP".into(), description: "Tank {n} temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: -20.0, max_val: 80.0, group: "tanks".into(), widget: "temperature".into(), interval_ms: 5000 },
            // GPS
            TemplateTag { name_pattern: "GPS_LAT".into(), description: "GPS latitude".into(), unit: "°".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: -90.0, max_val: 90.0, group: "navigation".into(), widget: "gauge".into(), interval_ms: 5000 },
            TemplateTag { name_pattern: "GPS_LON".into(), description: "GPS longitude".into(), unit: "°".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: -180.0, max_val: 180.0, group: "navigation".into(), widget: "gauge".into(), interval_ms: 5000 },
            TemplateTag { name_pattern: "GPS_SPD".into(), description: "Vessel speed".into(), unit: "knots".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 40.0, group: "navigation".into(), widget: "gauge".into(), interval_ms: 2000 },
        ],
        alarms: vec![
            TemplateAlarm { tag_pattern: "ENG{n}_TMP".into(), severity: "critical".into(), comparison: "above".into(), threshold: 95.0, deadband: 5.0, message_template: "Engine {n} overheating: {value}°C".into() },
            TemplateAlarm { tag_pattern: "ENG{n}_OIL_P".into(), severity: "critical".into(), comparison: "below".into(), threshold: 1.5, deadband: 0.3, message_template: "Engine {n} low oil pressure: {value} bar".into() },
            TemplateAlarm { tag_pattern: "ENG{n}_COOL_TMP".into(), severity: "warning".into(), comparison: "above".into(), threshold: 85.0, deadband: 5.0, message_template: "Engine {n} coolant temp high: {value}°C".into() },
            TemplateAlarm { tag_pattern: "BILGE{n}_LVL".into(), severity: "critical".into(), comparison: "above".into(), threshold: 80.0, deadband: 10.0, message_template: "Bilge {n} high level: {value}%".into() },
            TemplateAlarm { tag_pattern: "BILGE{n}_LVL".into(), severity: "warning".into(), comparison: "above".into(), threshold: 50.0, deadband: 10.0, message_template: "Bilge {n} rising: {value}%".into() },
            TemplateAlarm { tag_pattern: "NAV_{light}".into(), severity: "warning".into(), comparison: "equal".into(), threshold: 0.0, deadband: 0.0, message_template: "Navigation light {light} OFF".into() },
            TemplateAlarm { tag_pattern: "TK-{n}_LVL".into(), severity: "warning".into(), comparison: "below".into(), threshold: 15.0, deadband: 5.0, message_template: "Tank {n} low: {value}%".into() },
        ],
        dashboard: TemplateDashboard {
            screen_name: "Ship Overview".into(),
            layout_groups: vec![
                DashboardGroup { name: "Engine Room".into(), group_type: "engine".into(), columns: 3 },
                DashboardGroup { name: "Bilge System".into(), group_type: "bilge".into(), columns: 2 },
                DashboardGroup { name: "Navigation".into(), group_type: "navigation".into(), columns: 4 },
                DashboardGroup { name: "Tank Levels".into(), group_type: "tanks".into(), columns: 3 },
            ],
        },
        flows: vec![
            TemplateFlow {
                name: "Engine overheat emergency".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "ENG1_TMP", "comparison": "above", "threshold": 100.0, "holdoff_secs": 30}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "ENGINE OVERHEAT EMERGENCY — Engine 1 temperature critical"},
                    {"type": "action", "action": "webhook", "message": "Engine 1 overheat: throttle down and inspect"}
                ]),
                description: "Emergency alert for engine overheating — log and notify bridge".into(),
            },
            TemplateFlow {
                name: "Bilge high level auto-pump".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "BILGE1_LVL", "comparison": "above", "threshold": 60.0, "holdoff_secs": 120}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "Bilge 1 level high — auto-start pump"},
                    {"type": "action", "action": "webhook", "message": "Bilge pump 1 auto-started"}
                ]),
                description: "Log and notify when bilge level rises above threshold".into(),
            },
        ],
        oee_config: Some(TemplateOeeConfig {
            availability_tags: vec!["ENG1_RUN".into(), "ENG2_RUN".into()],
            performance_tags: vec!["ENG1_FUEL_FLOW".into(), "GPS_SPD".into()],
            quality_tags: vec!["ENG1_TMP".into(), "ENG1_OIL_P".into()],
            ideal_cycle_time_secs: None,
        }),
    }
}

// ── Elevator / Ski Lift ──

fn elevator_ski_lift() -> VerticalTemplate {
    VerticalTemplate {
        id: "elevator_ski_lift".into(),
        name: "Elevator / Ski Lift".into(),
        industry: "elevators_lifts".into(),
        description: "Vertical transport monitoring: motor control, safety interlocks, position tracking, emergency stop. Safety-critical with full audit trail.".into(),
        equipment_type: "lift".into(),
        tags: vec![
            // Motor / Drive
            TemplateTag { name_pattern: "MTR_RUN".into(), description: "Main motor running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "motor".into(), widget: "led".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "MTR_SPD".into(), description: "Motor speed".into(), unit: "m/s".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 10.0, group: "motor".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "MTR_AMP".into(), description: "Motor current".into(), unit: "A".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 200.0, group: "motor".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "MTR_TMP".into(), description: "Motor temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 120.0, group: "motor".into(), widget: "temperature".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "MTR_VIB".into(), description: "Motor vibration".into(), unit: "mm/s".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 20.0, group: "motor".into(), widget: "gauge".into(), interval_ms: 1000 },
            // Position
            TemplateTag { name_pattern: "POS_M".into(), description: "Cabin/carriage position".into(), unit: "m".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "position".into(), widget: "level".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "POS_DIR".into(), description: "Direction (0=stopped, 1=up, 2=down)".into(), unit: "".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 2.0, group: "position".into(), widget: "led".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "POS_FLOOR".into(), description: "Current floor/station".into(), unit: "".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 99.0, group: "position".into(), widget: "gauge".into(), interval_ms: 200 },
            // Safety interlocks
            TemplateTag { name_pattern: "SAFE_DOOR".into(), description: "Door closed interlock".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "safety".into(), widget: "led".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "SAFE_BRAKE".into(), description: "Brake engaged".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "safety".into(), widget: "led".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "SAFE_ESTOP".into(), description: "Emergency stop active".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "safety".into(), widget: "led".into(), interval_ms: 100 },
            TemplateTag { name_pattern: "SAFE_OVERSPEED".into(), description: "Overspeed governor tripped".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "safety".into(), widget: "led".into(), interval_ms: 100 },
            TemplateTag { name_pattern: "SAFE_ROPE".into(), description: "Rope/cable tension OK".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "safety".into(), widget: "led".into(), interval_ms: 200 },
            // Load
            TemplateTag { name_pattern: "LOAD_KG".into(), description: "Cabin/carriage load".into(), unit: "kg".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 5000.0, group: "load".into(), widget: "gauge".into(), interval_ms: 500 },
            // Operations
            TemplateTag { name_pattern: "OPS_TRIPS".into(), description: "Trip count today".into(), unit: "".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 9999.0, group: "operations".into(), widget: "gauge".into(), interval_ms: 10000 },
            TemplateTag { name_pattern: "OPS_FAULT".into(), description: "Active fault code".into(), unit: "".into(), data_type: "uint16".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 9999.0, group: "operations".into(), widget: "led".into(), interval_ms: 500 },
        ],
        alarms: vec![
            TemplateAlarm { tag_pattern: "SAFE_ESTOP".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 1.0, deadband: 0.0, message_template: "EMERGENCY STOP ACTIVATED".into() },
            TemplateAlarm { tag_pattern: "SAFE_OVERSPEED".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 1.0, deadband: 0.0, message_template: "OVERSPEED GOVERNOR TRIPPED".into() },
            TemplateAlarm { tag_pattern: "SAFE_DOOR".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 0.0, deadband: 0.0, message_template: "Door not closed during operation".into() },
            TemplateAlarm { tag_pattern: "SAFE_ROPE".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 0.0, deadband: 0.0, message_template: "Rope/cable tension fault".into() },
            TemplateAlarm { tag_pattern: "MTR_TMP".into(), severity: "warning".into(), comparison: "above".into(), threshold: 90.0, deadband: 5.0, message_template: "Motor temperature high: {value}°C".into() },
            TemplateAlarm { tag_pattern: "MTR_AMP".into(), severity: "warning".into(), comparison: "above".into(), threshold: 150.0, deadband: 10.0, message_template: "Motor overcurrent: {value}A".into() },
            TemplateAlarm { tag_pattern: "MTR_VIB".into(), severity: "warning".into(), comparison: "above".into(), threshold: 10.0, deadband: 2.0, message_template: "High vibration: {value} mm/s".into() },
            TemplateAlarm { tag_pattern: "LOAD_KG".into(), severity: "warning".into(), comparison: "above".into(), threshold: 4000.0, deadband: 200.0, message_template: "Overload warning: {value} kg".into() },
            TemplateAlarm { tag_pattern: "OPS_FAULT".into(), severity: "critical".into(), comparison: "above".into(), threshold: 0.0, deadband: 0.0, message_template: "Fault code: {value}".into() },
        ],
        dashboard: TemplateDashboard {
            screen_name: "Lift Overview".into(),
            layout_groups: vec![
                DashboardGroup { name: "Motor / Drive".into(), group_type: "motor".into(), columns: 5 },
                DashboardGroup { name: "Position".into(), group_type: "position".into(), columns: 3 },
                DashboardGroup { name: "Safety Interlocks".into(), group_type: "safety".into(), columns: 5 },
                DashboardGroup { name: "Load & Operations".into(), group_type: "load".into(), columns: 2 },
            ],
        },
        flows: vec![
            TemplateFlow {
                name: "Emergency stop notification".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "SAFE_ESTOP", "comparison": "equal", "threshold": 1.0, "holdoff_secs": 0}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "EMERGENCY STOP — immediate response required"},
                    {"type": "action", "action": "email", "subject": "EMERGENCY STOP ACTIVATED", "message": "E-stop triggered. All operations halted."},
                    {"type": "action", "action": "webhook", "message": "Emergency stop event"}
                ]),
                description: "Immediate notification to all channels when emergency stop is activated".into(),
            },
            TemplateFlow {
                name: "Motor vibration trend alert".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "MTR_VIB", "comparison": "above", "threshold": 8.0, "holdoff_secs": 300}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "Motor vibration above normal — schedule inspection"},
                    {"type": "action", "action": "webhook", "message": "Vibration alert: schedule bearing inspection"}
                ]),
                description: "Trend-based vibration alert for predictive maintenance scheduling".into(),
            },
        ],
        oee_config: Some(TemplateOeeConfig {
            availability_tags: vec!["MTR_RUN".into()],
            performance_tags: vec!["MTR_SPD".into(), "OPS_TRIPS".into()],
            quality_tags: vec!["SAFE_ESTOP".into(), "OPS_FAULT".into()],
            ideal_cycle_time_secs: Some(60.0), // 1 min per trip
        }),
    }
}

// ── Hydroelectric ──

fn hydroelectric() -> VerticalTemplate {
    VerticalTemplate {
        id: "hydroelectric".into(),
        name: "Hydroelectric Generator".into(),
        industry: "hydroelectric".into(),
        description: "Hydroelectric power generation: turbine control, gate position, flow monitoring, grid synchronization. IEC 62443 compliant for critical infrastructure.".into(),
        equipment_type: "hydro".into(),
        tags: vec![
            // Turbine
            TemplateTag { name_pattern: "TRB{n}_RPM".into(), description: "Turbine {n} speed".into(), unit: "RPM".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 600.0, group: "turbine".into(), widget: "gauge".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "TRB{n}_RUN".into(), description: "Turbine {n} running".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "turbine".into(), widget: "led".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "TRB{n}_VIB".into(), description: "Turbine {n} vibration".into(), unit: "mm/s".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 20.0, group: "turbine".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "TRB{n}_TMP".into(), description: "Turbine {n} bearing temperature".into(), unit: "°C".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 120.0, group: "turbine".into(), widget: "temperature".into(), interval_ms: 1000 },
            // Gate control
            TemplateTag { name_pattern: "GATE{n}_POS".into(), description: "Gate {n} position".into(), unit: "%".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 100.0, group: "gate".into(), widget: "level".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "GATE{n}_MTR_AMP".into(), description: "Gate {n} actuator current".into(), unit: "A".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 50.0, group: "gate".into(), widget: "gauge".into(), interval_ms: 500 },
            // Flow / Head
            TemplateTag { name_pattern: "FLOW_RATE".into(), description: "Penstock flow rate".into(), unit: "m³/s".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "flow".into(), widget: "flow".into(), interval_ms: 1000 },
            TemplateTag { name_pattern: "HEAD_M".into(), description: "Hydraulic head".into(), unit: "m".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "flow".into(), widget: "level".into(), interval_ms: 2000 },
            TemplateTag { name_pattern: "TAILWATER_M".into(), description: "Tailwater level".into(), unit: "m".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 50.0, group: "flow".into(), widget: "level".into(), interval_ms: 2000 },
            // Generator / Grid
            TemplateTag { name_pattern: "GEN{n}_MW".into(), description: "Generator {n} output power".into(), unit: "MW".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 500.0, group: "grid".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "GEN{n}_V".into(), description: "Generator {n} voltage".into(), unit: "kV".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 25.0, group: "grid".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "GEN{n}_A".into(), description: "Generator {n} current".into(), unit: "kA".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 50.0, group: "grid".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "GEN{n}_FREQ".into(), description: "Generator {n} frequency".into(), unit: "Hz".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 45.0, max_val: 55.0, group: "grid".into(), widget: "gauge".into(), interval_ms: 200 },
            TemplateTag { name_pattern: "GEN{n}_PF".into(), description: "Generator {n} power factor".into(), unit: "".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "grid".into(), widget: "gauge".into(), interval_ms: 500 },
            TemplateTag { name_pattern: "GRID_SYNC".into(), description: "Grid sync status".into(), unit: "".into(), data_type: "bool".into(), register_area: "coil".into(), scale: 1.0, min_val: 0.0, max_val: 1.0, group: "grid".into(), widget: "led".into(), interval_ms: 200 },
            // Reservoir
            TemplateTag { name_pattern: "RES_LVL".into(), description: "Reservoir level".into(), unit: "m".into(), data_type: "float32".into(), register_area: "holding".into(), scale: 1.0, min_val: 0.0, max_val: 200.0, group: "reservoir".into(), widget: "level".into(), interval_ms: 5000 },
        ],
        alarms: vec![
            TemplateAlarm { tag_pattern: "TRB{n}_VIB".into(), severity: "critical".into(), comparison: "above".into(), threshold: 15.0, deadband: 2.0, message_template: "Turbine {n} excessive vibration: {value} mm/s".into() },
            TemplateAlarm { tag_pattern: "TRB{n}_TMP".into(), severity: "critical".into(), comparison: "above".into(), threshold: 95.0, deadband: 5.0, message_template: "Turbine {n} bearing overtemp: {value}°C".into() },
            TemplateAlarm { tag_pattern: "GEN{n}_FREQ".into(), severity: "critical".into(), comparison: "above".into(), threshold: 51.5, deadband: 0.2, message_template: "Generator {n} overfrequency: {value} Hz".into() },
            TemplateAlarm { tag_pattern: "GEN{n}_FREQ".into(), severity: "critical".into(), comparison: "below".into(), threshold: 48.5, deadband: 0.2, message_template: "Generator {n} underfrequency: {value} Hz".into() },
            TemplateAlarm { tag_pattern: "GEN{n}_MW".into(), severity: "warning".into(), comparison: "above".into(), threshold: 450.0, deadband: 10.0, message_template: "Generator {n} high output: {value} MW".into() },
            TemplateAlarm { tag_pattern: "GEN{n}_V".into(), severity: "warning".into(), comparison: "above".into(), threshold: 22.0, deadband: 1.0, message_template: "Generator {n} overvoltage: {value} kV".into() },
            TemplateAlarm { tag_pattern: "RES_LVL".into(), severity: "warning".into(), comparison: "above".into(), threshold: 180.0, deadband: 5.0, message_template: "Reservoir level high: {value}m — consider gate opening".into() },
            TemplateAlarm { tag_pattern: "RES_LVL".into(), severity: "warning".into(), comparison: "below".into(), threshold: 50.0, deadband: 5.0, message_template: "Reservoir level low: {value}m — generation at risk".into() },
            TemplateAlarm { tag_pattern: "GATE{n}_MTR_AMP".into(), severity: "warning".into(), comparison: "above".into(), threshold: 40.0, deadband: 5.0, message_template: "Gate {n} actuator overcurrent: {value}A".into() },
            TemplateAlarm { tag_pattern: "GRID_SYNC".into(), severity: "critical".into(), comparison: "equal".into(), threshold: 0.0, deadband: 0.0, message_template: "Grid synchronization lost".into() },
        ],
        dashboard: TemplateDashboard {
            screen_name: "Hydroelectric Overview".into(),
            layout_groups: vec![
                DashboardGroup { name: "Turbine".into(), group_type: "turbine".into(), columns: 4 },
                DashboardGroup { name: "Gate Control".into(), group_type: "gate".into(), columns: 2 },
                DashboardGroup { name: "Flow & Head".into(), group_type: "flow".into(), columns: 3 },
                DashboardGroup { name: "Generator / Grid".into(), group_type: "grid".into(), columns: 5 },
                DashboardGroup { name: "Reservoir".into(), group_type: "reservoir".into(), columns: 1 },
            ],
        },
        flows: vec![
            TemplateFlow {
                name: "Frequency deviation response".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "GEN1_FREQ", "comparison": "above", "threshold": 51.0, "holdoff_secs": 10}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "Grid frequency deviation — turbine governor adjusting"},
                    {"type": "action", "action": "webhook", "message": "Frequency deviation alert: governor control active"}
                ]),
                description: "Alert when grid frequency deviates outside normal range — turbine governor should respond automatically".into(),
            },
            TemplateFlow {
                name: "High reservoir level gate advisory".into(),
                trigger_type: "tag_threshold".into(),
                trigger_config: json!({"tag": "RES_LVL", "comparison": "above", "threshold": 185.0, "holdoff_secs": 300}),
                nodes: json!([
                    {"type": "action", "action": "log", "message": "Reservoir level high — gate opening recommended"},
                    {"type": "action", "action": "email", "subject": "High Reservoir Level", "message": "Consider increasing gate opening to manage reservoir level"},
                    {"type": "action", "action": "webhook", "message": "Reservoir high level advisory"}
                ]),
                description: "Notify operators when reservoir exceeds safe level — gate opening recommended".into(),
            },
        ],
        oee_config: Some(TemplateOeeConfig {
            availability_tags: vec!["TRB1_RUN".into(), "GRID_SYNC".into()],
            performance_tags: vec!["GEN1_MW".into(), "FLOW_RATE".into()],
            quality_tags: vec!["GEN1_FREQ".into(), "GEN1_PF".into()],
            ideal_cycle_time_secs: None,
        }),
    }
}
