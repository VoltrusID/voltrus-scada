use serde::{Deserialize, Serialize};

/// A FEED (Front-End Engineering Design) template is a comprehensive I/O list
/// for a specific industry/equipment type. More detailed than v0.37 vertical
/// templates — designed for engineers doing FEED studies before procurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedTemplate {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub equipment_type: String,
    /// Total estimated tag count
    pub estimated_tag_count: u32,
    /// List of equipment units this template covers
    pub equipment_units: Vec<FeedEquipmentUnit>,
    /// Complete I/O list
    pub io_list: Vec<FeedIO>,
    /// Alarm strategy per equipment group
    pub alarm_strategy: Vec<FeedAlarmStrategy>,
    /// Suggested dashboard layout
    pub dashboard: FeedDashboard,
    /// Network architecture notes
    pub network_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedEquipmentUnit {
    pub name: String,
    pub quantity: String, // e.g. "1-4", "per pump", "per tank"
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedIO {
    /// Signal name (ISA-5.1)
    pub signal: String,
    pub description: String,
    /// AI, AO, DI, DO, PI
    pub io_type: String,
    /// Unit of measurement
    pub unit: String,
    /// Data type: bool, uint16, int16, float32, float64
    pub data_type: String,
    /// Typical range (e.g. "0-100", "0-10V", "4-20mA")
    pub typical_range: String,
    /// Equipment group this belongs to
    pub equipment: String,
    /// Criticality: critical, major, minor, info
    pub criticality: String,
    /// Notes for the engineer
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedAlarmStrategy {
    /// Equipment group this strategy applies to
    pub equipment: String,
    /// Alarm rules
    pub rules: Vec<FeedAlarmRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedAlarmRule {
    pub signal_pattern: String,
    pub severity: String,  // critical, major, warning, info
    pub condition: String, // high, low, high-high, low-low, deviation, rate-of-change
    pub setpoint: String,  // e.g. "90%", "2.0 bar", "60°C"
    pub deadband: String,  // e.g. "2%", "0.1 bar"
    pub description: String,
    /// ISA-18.2 alarm class
    pub alarm_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedDashboard {
    pub description: String,
    pub recommended_screens: Vec<FeedScreen>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedScreen {
    pub name: String,
    pub layout: String, // "grid", "canvas", "twin"
    pub sections: Vec<FeedDashboardSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedDashboardSection {
    pub title: String,
    pub equipment_group: String,
    pub widget_type: String, // "gauge", "led", "trend", "table", "alarm_list"
    pub signals: Vec<String>,
}

// ── FEED Template Library ──

/// All available FEED templates
pub fn feed_templates() -> Vec<FeedTemplate> {
    vec![
        water_treatment_plant(),
        pump_station(),
        compressor_station(),
        conveyor_system(),
        batch_reactor(),
        hvac_chiller_plant(),
        power_distribution(),
    ]
}

/// Get a FEED template by ID
pub fn get_feed_template(id: &str) -> Option<FeedTemplate> {
    feed_templates().into_iter().find(|t| t.id == id)
}

/// List FEED template summaries
pub fn feed_summaries() -> Vec<FeedTemplateSummary> {
    feed_templates()
        .into_iter()
        .map(|t| FeedTemplateSummary {
            id: t.id,
            name: t.name,
            industry: t.industry,
            description: t.description,
            estimated_tag_count: t.estimated_tag_count,
            equipment_units: t.equipment_units.iter().map(|u| u.name.clone()).collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedTemplateSummary {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub estimated_tag_count: u32,
    pub equipment_units: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════
// FEED Template Definitions
// ═══════════════════════════════════════════════════════════════════

fn water_treatment_plant() -> FeedTemplate {
    FeedTemplate {
        id: "feed-water-treatment".into(),
        name: "Water Treatment Plant".into(),
        industry: "Water/Wastewater".into(),
        description: "Complete I/O list for a conventional water treatment plant including raw water intake, coagulation/flocculation, sedimentation, filtration, disinfection, and clear well storage.".into(),
        equipment_type: "Water Treatment Plant".into(),
        estimated_tag_count: 280,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Raw Water Intake Pumps".into(), quantity: "2-4".into(), description: "Vertical turbine or submersible pumps at source".into() },
            FeedEquipmentUnit { name: "Rapid Mix / Flash Mixer".into(), quantity: "1-2".into(), description: "Coagulant injection and rapid mixing".into() },
            FeedEquipmentUnit { name: "Flocculators".into(), quantity: "2-4".into(), description: "Slow mix stages with paddle flocculators".into() },
            FeedEquipmentUnit { name: "Sedimentation Basins".into(), quantity: "2-4".into(), description: "Rectangular or circular clarifiers".into() },
            FeedEquipmentUnit { name: "Filter Beds".into(), quantity: "4-8".into(), description: "Rapid gravity sand or multimedia filters".into() },
            FeedEquipmentUnit { name: "Chemical Dosing Systems".into(), quantity: "3-5".into(), description: "Coagulant, polymer, lime, chlorine, fluoride".into() },
            FeedEquipmentUnit { name: "Clear Well".into(), quantity: "1-2".into(), description: "Treated water storage before distribution".into() },
            FeedEquipmentUnit { name: "High-Lift Pumps".into(), quantity: "2-4".into(), description: "Distribution system pumps".into() },
            FeedEquipmentUnit { name: "Backwash System".into(), quantity: "1".into(), description: "Filter backwash pumps and air scour".into() },
            FeedEquipmentUnit { name: "Sludge Handling".into(), quantity: "1".into(), description: "Sludge thickener and disposal".into() },
        ],
        io_list: vec![
            // Raw Water Intake
            FeedIO { signal: "LIT_RAW_001".into(), description: "Raw water source level".into(), io_type: "AI".into(), unit: "m".into(), data_type: "float32".into(), typical_range: "0-20".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "major".into(), notes: "Ultrasonic or radar level transmitter".into() },
            FeedIO { signal: "FIT_RAW_001".into(), description: "Raw water intake flow".into(), io_type: "AI".into(), unit: "m³/h".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "major".into(), notes: "Electromagnetic flow meter".into() },
            FeedIO { signal: "PMP_RAW_001_RUN".into(), description: "Raw water pump 1 running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "critical".into(), notes: "Motor contactor auxiliary contact".into() },
            FeedIO { signal: "PMP_RAW_001_STRT".into(), description: "Raw water pump 1 start command".into(), io_type: "DO".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "critical".into(), notes: "Pump start relay".into() },
            FeedIO { signal: "PMP_RAW_001_FLT".into(), description: "Raw water pump 1 fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "critical".into(), notes: "Motor protection relay trip".into() },
            FeedIO { signal: "PMP_RAW_001_A".into(), description: "Raw water pump 1 current".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-500".into(), equipment: "Raw Water Intake Pumps".into(), criticality: "major".into(), notes: "CT input to PLC analog card".into() },
            // Rapid Mix
            FeedIO { signal: "AIT_TURB_001".into(), description: "Raw water turbidity".into(), io_type: "AI".into(), unit: "NTU".into(), data_type: "float32".into(), typical_range: "0-1000".into(), equipment: "Rapid Mix / Flash Mixer".into(), criticality: "major".into(), notes: "Online turbidity analyzer".into() },
            FeedIO { signal: "MXR_FLASH_001_RUN".into(), description: "Flash mixer running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Rapid Mix / Flash Mixer".into(), criticality: "major".into(), notes: "Mixer motor run status".into() },
            FeedIO { signal: "MXR_FLASH_001_SPD".into(), description: "Flash mixer speed".into(), io_type: "AI".into(), unit: "RPM".into(), data_type: "float32".into(), typical_range: "0-1500".into(), equipment: "Rapid Mix / Flash Mixer".into(), criticality: "minor".into(), notes: "VFD speed feedback".into() },
            FeedIO { signal: "SCADA_PAC_001_DOSE".into(), description: "Alum/PAC dosing rate".into(), io_type: "AI".into(), unit: "L/h".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Chemical Dosing Systems".into(), criticality: "major".into(), notes: "Dosing pump stroke feedback".into() },
            FeedIO { signal: "SCADA_LVL_COAG".into(), description: "Coagulant tank level".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Chemical Dosing Systems".into(), criticality: "major".into(), notes: "Ultrasonic tank level".into() },
            // Flocculation
            FeedIO { signal: "MXR_FLOC_001_RUN".into(), description: "Flocculator 1 running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Flocculators".into(), criticality: "major".into(), notes: "Paddle flocculator motor run".into() },
            FeedIO { signal: "MXR_FLOC_001_SPD".into(), description: "Flocculator 1 speed".into(), io_type: "AI".into(), unit: "RPM".into(), data_type: "float32".into(), typical_range: "0-60".into(), equipment: "Flocculators".into(), criticality: "minor".into(), notes: "Low-speed VFD".into() },
            // Sedimentation
            FeedIO { signal: "LIT_SED_001".into(), description: "Sedimentation basin 1 sludge level".into(), io_type: "AI".into(), unit: "m".into(), data_type: "float32".into(), typical_range: "0-5".into(), equipment: "Sedimentation Basins".into(), criticality: "major".into(), notes: "Sludge blanket level detector".into() },
            FeedIO { signal: "AIT_TURB_SED1".into(), description: "Settled water turbidity".into(), io_type: "AI".into(), unit: "NTU".into(), data_type: "float32".into(), typical_range: "0-50".into(), equipment: "Sedimentation Basins".into(), criticality: "major".into(), notes: "Post-sedimentation turbidity".into() },
            // Filtration
            FeedIO { signal: "FIT_FILT_001".into(), description: "Filter 1 effluent flow".into(), io_type: "AI".into(), unit: "m³/h".into(), data_type: "float32".into(), typical_range: "0-1000".into(), equipment: "Filter Beds".into(), criticality: "major".into(), notes: "Per-filter flow meter".into() },
            FeedIO { signal: "PdIT_FILT_001".into(), description: "Filter 1 head loss".into(), io_type: "AI".into(), unit: "m".into(), data_type: "float32".into(), typical_range: "0-2.5".into(), equipment: "Filter Beds".into(), criticality: "major".into(), notes: "Differential pressure. Triggers backwash at ~2m".into() },
            FeedIO { signal: "AIT_TURB_FLT1".into(), description: "Filter 1 effluent turbidity".into(), io_type: "AI".into(), unit: "NTU".into(), data_type: "float32".into(), typical_range: "0-1".into(), equipment: "Filter Beds".into(), criticality: "critical".into(), notes: "Must be <0.3 NTU per regulation".into() },
            FeedIO { signal: "VLV_FLT1_INL_OPN".into(), description: "Filter 1 inlet valve open".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Filter Beds".into(), criticality: "major".into(), notes: "Limit switch".into() },
            // Disinfection
            FeedIO { signal: "AIT_CL2_001".into(), description: "Chlorine residual".into(), io_type: "AI".into(), unit: "mg/L".into(), data_type: "float32".into(), typical_range: "0-5".into(), equipment: "Chemical Dosing Systems".into(), criticality: "critical".into(), notes: "Online chlorine analyzer. Critical for public health".into() },
            FeedIO { signal: "AIT_pH_001".into(), description: "Treated water pH".into(), io_type: "AI".into(), unit: "pH".into(), data_type: "float32".into(), typical_range: "6.5-8.5".into(), equipment: "Chemical Dosing Systems".into(), criticality: "major".into(), notes: "pH probe".into() },
            // Clear Well & Distribution
            FeedIO { signal: "LIT_CLEAR_001".into(), description: "Clear well level".into(), io_type: "AI".into(), unit: "m".into(), data_type: "float32".into(), typical_range: "0-10".into(), equipment: "Clear Well".into(), criticality: "critical".into(), notes: "Controls distribution pump operation".into() },
            FeedIO { signal: "FIT_DIST_001".into(), description: "Distribution flow".into(), io_type: "AI".into(), unit: "m³/h".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "High-Lift Pumps".into(), criticality: "major".into(), notes: "Total distribution flow".into() },
            FeedIO { signal: "PIT_DIST_001".into(), description: "Distribution pressure".into(), io_type: "AI".into(), unit: "bar".into(), data_type: "float32".into(), typical_range: "0-10".into(), equipment: "High-Lift Pumps".into(), criticality: "critical".into(), notes: "Network pressure at discharge header".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Raw Water Intake Pumps".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "PMP_RAW_{n}_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Pump fault — motor protection trip".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "PMP_RAW_{n}_A".into(), severity: "major".into(), condition: "high".into(), setpoint: "110% FLA".into(), deadband: "5%".into(), description: "Pump overcurrent".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "LIT_RAW_001".into(), severity: "critical".into(), condition: "low-low".into(), setpoint: "0.5m".into(), deadband: "0.1m".into(), description: "Raw water source critically low".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Filter Beds".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "AIT_TURB_FLT{n}".into(), severity: "critical".into(), condition: "high".into(), setpoint: "0.3 NTU".into(), deadband: "0.05".into(), description: "Filter effluent turbidity exceeds regulatory limit".into(), alarm_class: "Process / Regulatory".into() },
                    FeedAlarmRule { signal_pattern: "PdIT_FILT_{n}".into(), severity: "major".into(), condition: "high".into(), setpoint: "2.0m".into(), deadband: "0.2m".into(), description: "Filter head loss — backwash required".into(), alarm_class: "Maintenance".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Chemical Dosing Systems".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "AIT_CL2_001".into(), severity: "critical".into(), condition: "low".into(), setpoint: "0.2 mg/L".into(), deadband: "0.05".into(), description: "Chlorine residual below minimum — public health risk".into(), alarm_class: "Process / Regulatory".into() },
                    FeedAlarmRule { signal_pattern: "AIT_CL2_001".into(), severity: "major".into(), condition: "high".into(), setpoint: "4.0 mg/L".into(), deadband: "0.2".into(), description: "Chlorine residual high — taste/odor complaints".into(), alarm_class: "Process".into() },
                    FeedAlarmRule { signal_pattern: "SCADA_LVL_COAG".into(), severity: "major".into(), condition: "low".into(), setpoint: "10%".into(), deadband: "5%".into(), description: "Coagulant tank low level".into(), alarm_class: "Maintenance".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Clear Well".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "LIT_CLEAR_001".into(), severity: "critical".into(), condition: "low-low".into(), setpoint: "10%".into(), deadband: "5%".into(), description: "Clear well critically low — risk of pump cavitation".into(), alarm_class: "Process".into() },
                    FeedAlarmRule { signal_pattern: "LIT_CLEAR_001".into(), severity: "major".into(), condition: "high-high".into(), setpoint: "95%".into(), deadband: "2%".into(), description: "Clear well overflow risk".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "High-Lift Pumps".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "PIT_DIST_001".into(), severity: "critical".into(), condition: "low".into(), setpoint: "1.0 bar".into(), deadband: "0.2 bar".into(), description: "Distribution pressure critically low — possible main break".into(), alarm_class: "Safety / Process".into() },
                    FeedAlarmRule { signal_pattern: "PIT_DIST_001".into(), severity: "major".into(), condition: "high".into(), setpoint: "8.0 bar".into(), deadband: "0.5 bar".into(), description: "Distribution pressure high — burst risk".into(), alarm_class: "Safety".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Water treatment plant overview with P&ID-style layout".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Plant Overview".into(),
                    layout: "canvas".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Raw Water Intake".into(), equipment_group: "Raw Water Intake Pumps".into(), widget_type: "gauge".into(), signals: vec!["LIT_RAW_001".into(), "FIT_RAW_001".into()] },
                        FeedDashboardSection { title: "Treatment Process".into(), equipment_group: "Filter Beds".into(), widget_type: "trend".into(), signals: vec!["AIT_TURB_001".into(), "AIT_TURB_FLT1".into(), "AIT_CL2_001".into(), "AIT_pH_001".into()] },
                        FeedDashboardSection { title: "Clear Well & Distribution".into(), equipment_group: "Clear Well".into(), widget_type: "gauge".into(), signals: vec!["LIT_CLEAR_001".into(), "FIT_DIST_001".into(), "PIT_DIST_001".into()] },
                        FeedDashboardSection { title: "Alarms".into(), equipment_group: "All".into(), widget_type: "alarm_list".into(), signals: vec![] },
                    ],
                },
                FeedScreen {
                    name: "Filtration Detail".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Filter Status".into(), equipment_group: "Filter Beds".into(), widget_type: "table".into(), signals: vec!["FIT_FILT_{n}".into(), "PdIT_FILT_{n}".into(), "AIT_TURB_FLT{n}".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Typical architecture: PLC at each major process area (intake, treatment, distribution) connected via fiber ring. SCADA server in main control room. Redundant server recommended for critical plants.".into()),
    }
}

fn pump_station() -> FeedTemplate {
    FeedTemplate {
        id: "feed-pump-station".into(),
        name: "Booster Pump Station".into(),
        industry: "Water/Wastewater".into(),
        description: "Standard booster pump station for water or wastewater with wet well, pumps, and discharge monitoring.".into(),
        equipment_type: "Pump Station".into(),
        estimated_tag_count: 85,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Wet Well".into(), quantity: "1".into(), description: "Incoming flow collection".into() },
            FeedEquipmentUnit { name: "Pumps".into(), quantity: "2-6".into(), description: "Submersible or dry-pit centrifugal pumps".into() },
            FeedEquipmentUnit { name: "Discharge Header".into(), quantity: "1".into(), description: "Common discharge manifold".into() },
            FeedEquipmentUnit { name: "Generator".into(), quantity: "1".into(), description: "Backup power".into() },
        ],
        io_list: vec![
            FeedIO { signal: "LIT_WETWELL".into(), description: "Wet well level".into(), io_type: "AI".into(), unit: "m".into(), data_type: "float32".into(), typical_range: "0-10".into(), equipment: "Wet Well".into(), criticality: "critical".into(), notes: "Ultrasonic or submersible level transmitter".into() },
            FeedIO { signal: "FIT_DISCHG".into(), description: "Discharge flow".into(), io_type: "AI".into(), unit: "m³/h".into(), data_type: "float32".into(), typical_range: "0-2000".into(), equipment: "Discharge Header".into(), criticality: "major".into(), notes: "Electromagnetic flow meter".into() },
            FeedIO { signal: "PIT_DISCHG".into(), description: "Discharge pressure".into(), io_type: "AI".into(), unit: "bar".into(), data_type: "float32".into(), typical_range: "0-16".into(), equipment: "Discharge Header".into(), criticality: "major".into(), notes: "Pressure transmitter".into() },
            FeedIO { signal: "PMP_{n}_RUN".into(), description: "Pump N running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Pumps".into(), criticality: "critical".into(), notes: "Contactor auxiliary".into() },
            FeedIO { signal: "PMP_{n}_STRT".into(), description: "Pump N start command".into(), io_type: "DO".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Pumps".into(), criticality: "critical".into(), notes: "Start relay".into() },
            FeedIO { signal: "PMP_{n}_FLT".into(), description: "Pump N fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Pumps".into(), criticality: "critical".into(), notes: "Motor protection trip".into() },
            FeedIO { signal: "PMP_{n}_A".into(), description: "Pump N current".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-500".into(), equipment: "Pumps".into(), criticality: "major".into(), notes: "CT input".into() },
            FeedIO { signal: "GEN_RUN".into(), description: "Generator running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Generator".into(), criticality: "major".into(), notes: "Generator controller output".into() },
            FeedIO { signal: "GEN_FLT".into(), description: "Generator fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Generator".into(), criticality: "critical".into(), notes: "Generator alarm".into() },
            FeedIO { signal: "LIT_FUEL".into(), description: "Fuel tank level".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Generator".into(), criticality: "major".into(), notes: "Fuel level gauge".into() },
            FeedIO { signal: "AIT_H2S".into(), description: "H2S gas detector".into(), io_type: "AI".into(), unit: "ppm".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Wet Well".into(), criticality: "critical".into(), notes: "Safety — confined space gas monitoring".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Wet Well".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "LIT_WETWELL".into(), severity: "critical".into(), condition: "high-high".into(), setpoint: "90%".into(), deadband: "2%".into(), description: "Wet well critical high — risk of overflow".into(), alarm_class: "Safety / Regulatory".into() },
                    FeedAlarmRule { signal_pattern: "LIT_WETWELL".into(), severity: "major".into(), condition: "low-low".into(), setpoint: "0.3m".into(), deadband: "0.1m".into(), description: "Wet well critical low — pump cavitation risk".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "AIT_H2S".into(), severity: "critical".into(), condition: "high".into(), setpoint: "10 ppm".into(), deadband: "2".into(), description: "H2S gas detected — confined space hazard".into(), alarm_class: "Safety".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Pumps".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "PMP_{n}_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Pump fault — motor protection trip".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "PMP_{n}_A".into(), severity: "major".into(), condition: "high".into(), setpoint: "110% FLA".into(), deadband: "5%".into(), description: "Pump overcurrent".into(), alarm_class: "Equipment Protection".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Discharge Header".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "PIT_DISCHG".into(), severity: "critical".into(), condition: "low".into(), setpoint: "0.5 bar".into(), deadband: "0.2 bar".into(), description: "Discharge pressure critically low — possible pipe break".into(), alarm_class: "Process / Safety".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Generator".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "GEN_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Generator fault — no backup power available".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "LIT_FUEL".into(), severity: "major".into(), condition: "low".into(), setpoint: "20%".into(), deadband: "5%".into(), description: "Fuel low — refill required".into(), alarm_class: "Maintenance".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Pump station overview with wet well, pump status, and discharge monitoring".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Pump Station".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Wet Well".into(), equipment_group: "Wet Well".into(), widget_type: "gauge".into(), signals: vec!["LIT_WETWELL".into(), "AIT_H2S".into()] },
                        FeedDashboardSection { title: "Pump Status".into(), equipment_group: "Pumps".into(), widget_type: "led".into(), signals: vec!["PMP_{n}_RUN".into(), "PMP_{n}_FLT".into()] },
                        FeedDashboardSection { title: "Discharge".into(), equipment_group: "Discharge Header".into(), widget_type: "trend".into(), signals: vec!["FIT_DISCHG".into(), "PIT_DISCHG".into()] },
                        FeedDashboardSection { title: "Generator".into(), equipment_group: "Generator".into(), widget_type: "gauge".into(), signals: vec!["GEN_RUN".into(), "LIT_FUEL".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Remote pump stations often connected via cellular modem or radio telemetry. Use Modbus TCP over VPN or MQTT over cellular for remote monitoring.".into()),
    }
}

fn compressor_station() -> FeedTemplate {
    FeedTemplate {
        id: "feed-compressor-station".into(),
        name: "Compressor Station".into(),
        industry: "Oil & Gas / Industrial".into(),
        description: "Air or gas compressor station with multiple compressor units, dryers, receivers, and distribution.".into(),
        equipment_type: "Compressor Station".into(),
        estimated_tag_count: 120,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Compressors".into(), quantity: "2-4".into(), description: "Screw, centrifugal, or reciprocating compressors".into() },
            FeedEquipmentUnit { name: "Air Dryers".into(), quantity: "1-2".into(), description: "Refrigerated or desiccant dryers".into() },
            FeedEquipmentUnit { name: "Receivers".into(), quantity: "1-2".into(), description: "Air receiver tanks".into() },
            FeedEquipmentUnit { name: "Filters".into(), quantity: "2-4".into(), description: "Pre/post filters".into() },
        ],
        io_list: vec![
            FeedIO { signal: "CMP_{n}_RUN".into(), description: "Compressor N running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Compressors".into(), criticality: "critical".into(), notes: "Motor run status".into() },
            FeedIO { signal: "CMP_{n}_FLT".into(), description: "Compressor N fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Compressors".into(), criticality: "critical".into(), notes: "Compressor controller fault".into() },
            FeedIO { signal: "CMP_{n}_LOAD".into(), description: "Compressor N load".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Compressors".into(), criticality: "major".into(), notes: "Capacity utilization".into() },
            FeedIO { signal: "PIT_CMP_{n}_OUT".into(), description: "Compressor N discharge pressure".into(), io_type: "AI".into(), unit: "bar".into(), data_type: "float32".into(), typical_range: "0-16".into(), equipment: "Compressors".into(), criticality: "critical".into(), notes: "Discharge pressure".into() },
            FeedIO { signal: "TIT_CMP_{n}_OUT".into(), description: "Compressor N discharge temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-200".into(), equipment: "Compressors".into(), criticality: "major".into(), notes: "High temp indicates cooling issues".into() },
            FeedIO { signal: "FIT_SYS_OUT".into(), description: "System output flow".into(), io_type: "AI".into(), unit: "Nm³/h".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Receivers".into(), criticality: "major".into(), notes: "Total system flow".into() },
            FeedIO { signal: "PIT_SYS".into(), description: "System pressure".into(), io_type: "AI".into(), unit: "bar".into(), data_type: "float32".into(), typical_range: "0-16".into(), equipment: "Receivers".into(), criticality: "critical".into(), notes: "Receiver/header pressure".into() },
            FeedIO { signal: "TdP_DRY_{n}".into(), description: "Dryer N dew point".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "-70-20".into(), equipment: "Air Dryers".into(), criticality: "major".into(), notes: "Pressure dew point".into() },
            FeedIO { signal: "PdIT_FLT_{n}".into(), description: "Filter N differential pressure".into(), io_type: "AI".into(), unit: "mbar".into(), data_type: "float32".into(), typical_range: "0-500".into(), equipment: "Filters".into(), criticality: "minor".into(), notes: "Filter loading indicator".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Compressors".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "CMP_{n}_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Compressor fault".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "TIT_CMP_{n}_OUT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "180°C".into(), deadband: "5".into(), description: "Compressor discharge temperature high — shutdown risk".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "PIT_SYS".into(), severity: "major".into(), condition: "low".into(), setpoint: "5.0 bar".into(), deadband: "0.5 bar".into(), description: "System pressure low".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Air Dryers".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "TdP_DRY_{n}".into(), severity: "major".into(), condition: "high".into(), setpoint: "3°C".into(), deadband: "1".into(), description: "Dew point high — dryer performance degraded".into(), alarm_class: "Process".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Compressor station overview with compressor status, pressure, flow, and dew point".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Compressor Station".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Compressors".into(), equipment_group: "Compressors".into(), widget_type: "led".into(), signals: vec!["CMP_{n}_RUN".into(), "CMP_{n}_LOAD".into()] },
                        FeedDashboardSection { title: "System Parameters".into(), equipment_group: "Receivers".into(), widget_type: "trend".into(), signals: vec!["PIT_SYS".into(), "FIT_SYS_OUT".into()] },
                        FeedDashboardSection { title: "Air Quality".into(), equipment_group: "Air Dryers".into(), widget_type: "gauge".into(), signals: vec!["TdP_DRY_{n}".into(), "PdIT_FLT_{n}".into()] },
                    ],
                },
            ],
        },
        network_notes: None,
    }
}

fn conveyor_system() -> FeedTemplate {
    FeedTemplate {
        id: "feed-conveyor".into(),
        name: "Conveyor System".into(),
        industry: "Manufacturing / Material Handling".into(),
        description: "Multi-segment conveyor system with motors, belt tracking, speed monitoring, and safety interlocks.".into(),
        equipment_type: "Conveyor".into(),
        estimated_tag_count: 60,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Conveyor Segments".into(), quantity: "3-10".into(), description: "Individual belt or roller conveyor segments".into() },
            FeedEquipmentUnit { name: "Drive Motors".into(), quantity: "1 per segment".into(), description: "VFD-controlled drive motors".into() },
            FeedEquipmentUnit { name: "Safety Devices".into(), quantity: "per segment".into(), description: "E-stops, pull cords, light curtains".into() },
        ],
        io_list: vec![
            FeedIO { signal: "CNV_{n}_RUN".into(), description: "Conveyor N running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Conveyor Segments".into(), criticality: "major".into(), notes: "Run feedback from VFD".into() },
            FeedIO { signal: "CNV_{n}_FLT".into(), description: "Conveyor N fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Conveyor Segments".into(), criticality: "critical".into(), notes: "VFD fault or motor protection trip".into() },
            FeedIO { signal: "MTR_{n}_SPD".into(), description: "Conveyor N speed".into(), io_type: "AI".into(), unit: "m/s".into(), data_type: "float32".into(), typical_range: "0-5".into(), equipment: "Drive Motors".into(), criticality: "major".into(), notes: "Speed feedback from encoder or VFD".into() },
            FeedIO { signal: "MTR_{n}_A".into(), description: "Conveyor N motor current".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Drive Motors".into(), criticality: "major".into(), notes: "CT or VFD current feedback".into() },
            FeedIO { signal: "BLT_{n}_ALGN".into(), description: "Belt N misalignment".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Conveyor Segments".into(), criticality: "major".into(), notes: "Belt alignment sensor".into() },
            FeedIO { signal: "CNV_{n}_ESTP".into(), description: "E-stop N activated".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Safety Devices".into(), criticality: "critical".into(), notes: "Emergency stop".into() },
            FeedIO { signal: "CNV_{n}_CORD".into(), description: "Pull cord N activated".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Safety Devices".into(), criticality: "critical".into(), notes: "Pull cord safety switch".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Conveyor Segments".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "CNV_{n}_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Conveyor fault".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "BLT_{n}_ALGN".into(), severity: "major".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Belt misalignment".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "CNV_{n}_ESTP".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Emergency stop activated".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "CNV_{n}_CORD".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Pull cord activated".into(), alarm_class: "Safety".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Conveyor system overview with segment status, speed, and safety devices".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Conveyor System".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Conveyor Status".into(), equipment_group: "Conveyor Segments".into(), widget_type: "led".into(), signals: vec!["CNV_{n}_RUN".into(), "CNV_{n}_FLT".into()] },
                        FeedDashboardSection { title: "Speed & Current".into(), equipment_group: "Drive Motors".into(), widget_type: "trend".into(), signals: vec!["MTR_{n}_SPD".into(), "MTR_{n}_A".into()] },
                        FeedDashboardSection { title: "Safety".into(), equipment_group: "Safety Devices".into(), widget_type: "led".into(), signals: vec!["CNV_{n}_ESTP".into(), "CNV_{n}_CORD".into(), "BLT_{n}_ALGN".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Conveyor systems often use distributed I/O blocks along the conveyor length connected via fieldbus (Profinet, EtherNet/IP). Each block handles a conveyor segment's sensors and actuators.".into()),
    }
}

fn batch_reactor() -> FeedTemplate {
    FeedTemplate {
        id: "feed-batch-reactor".into(),
        name: "Batch Reactor".into(),
        industry: "Chemical / Pharmaceutical".into(),
        description: "Batch reactor with temperature control, agitation, ingredient dosing, and CIP system.".into(),
        equipment_type: "Batch Reactor".into(),
        estimated_tag_count: 110,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Reactor Vessel".into(), quantity: "1".into(), description: "Jacketed reactor vessel".into() },
            FeedEquipmentUnit { name: "Agitator".into(), quantity: "1".into(), description: "Top or bottom entry agitator".into() },
            FeedEquipmentUnit { name: "Heating/Cooling".into(), quantity: "1".into(), description: "Jacket temperature control with steam/cooling water valves".into() },
            FeedEquipmentUnit { name: "Ingredient Dosing".into(), quantity: "2-6".into(), description: "Liquid/solid ingredient feeds".into() },
            FeedEquipmentUnit { name: "CIP System".into(), quantity: "1".into(), description: "Clean-in-place system".into() },
        ],
        io_list: vec![
            FeedIO { signal: "TIT_REACTOR".into(), description: "Reactor temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-200".into(), equipment: "Reactor Vessel".into(), criticality: "critical".into(), notes: "RTD PT100".into() },
            FeedIO { signal: "TIT_JACKET".into(), description: "Jacket temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-200".into(), equipment: "Heating/Cooling".into(), criticality: "major".into(), notes: "Jacket inlet temperature".into() },
            FeedIO { signal: "PIT_REACTOR".into(), description: "Reactor pressure".into(), io_type: "AI".into(), unit: "bar".into(), data_type: "float32".into(), typical_range: "-1-10".into(), equipment: "Reactor Vessel".into(), criticality: "critical".into(), notes: "Pressure transmitter".into() },
            FeedIO { signal: "LIT_REACTOR".into(), description: "Reactor level/volume".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Reactor Vessel".into(), criticality: "major".into(), notes: "Differential pressure or radar level".into() },
            FeedIO { signal: "AIT_pH_REACT".into(), description: "Reactor pH".into(), io_type: "AI".into(), unit: "pH".into(), data_type: "float32".into(), typical_range: "0-14".into(), equipment: "Reactor Vessel".into(), criticality: "critical".into(), notes: "pH probe".into() },
            FeedIO { signal: "AGTR_RUN".into(), description: "Agitator running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Agitator".into(), criticality: "critical".into(), notes: "Agitator motor run feedback".into() },
            FeedIO { signal: "AGTR_SPD".into(), description: "Agitator speed".into(), io_type: "AI".into(), unit: "RPM".into(), data_type: "float32".into(), typical_range: "0-500".into(), equipment: "Agitator".into(), criticality: "major".into(), notes: "VFD speed feedback".into() },
            FeedIO { signal: "AGTR_A".into(), description: "Agitator current".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Agitator".into(), criticality: "major".into(), notes: "Motor current — indicates viscosity".into() },
            FeedIO { signal: "VLV_STEAM_OPN".into(), description: "Steam valve position".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Heating/Cooling".into(), criticality: "major".into(), notes: "Control valve position feedback".into() },
            FeedIO { signal: "VLV_CW_OPN".into(), description: "Cooling water valve position".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Heating/Cooling".into(), criticality: "major".into(), notes: "Control valve position feedback".into() },
            FeedIO { signal: "WIT_ING_{n}".into(), description: "Ingredient N weight".into(), io_type: "AI".into(), unit: "kg".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Ingredient Dosing".into(), criticality: "major".into(), notes: "Load cell on dosing tank or loss-in-weight feeder".into() },
            FeedIO { signal: "VLV_DOSE_{n}_OPN".into(), description: "Dosing valve N open".into(), io_type: "DO".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Ingredient Dosing".into(), criticality: "major".into(), notes: "Auto ball valve or diaphragm valve".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Reactor Vessel".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "TIT_REACTOR".into(), severity: "critical".into(), condition: "high-high".into(), setpoint: "10% below MAWT".into(), deadband: "2".into(), description: "Reactor temperature approaching maximum allowable".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "PIT_REACTOR".into(), severity: "critical".into(), condition: "high-high".into(), setpoint: "90% of MAWP".into(), deadband: "0.2 bar".into(), description: "Reactor pressure approaching maximum allowable — relief valve".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "AIT_pH_REACT".into(), severity: "major".into(), condition: "deviation".into(), setpoint: "±0.5 pH from target".into(), deadband: "0.1".into(), description: "pH out of target range".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Agitator".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "AGTR_RUN".into(), severity: "critical".into(), condition: "low".into(), setpoint: "1".into(), deadband: "0".into(), description: "Agitator stopped — reaction may not be mixing".into(), alarm_class: "Process".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Batch reactor overview with temperature control, agitation, and dosing".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Reactor Overview".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Reactor Parameters".into(), equipment_group: "Reactor Vessel".into(), widget_type: "gauge".into(), signals: vec!["TIT_REACTOR".into(), "PIT_REACTOR".into(), "LIT_REACTOR".into(), "AIT_pH_REACT".into()] },
                        FeedDashboardSection { title: "Temperature Control".into(), equipment_group: "Heating/Cooling".into(), widget_type: "trend".into(), signals: vec!["TIT_JACKET".into(), "VLV_STEAM_OPN".into(), "VLV_CW_OPN".into()] },
                        FeedDashboardSection { title: "Agitator".into(), equipment_group: "Agitator".into(), widget_type: "gauge".into(), signals: vec!["AGTR_SPD".into(), "AGTR_A".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Batch reactors in pharma typically require 21 CFR Part 11 compliance for batch records. All critical parameters should be recorded with audit trail.".into()),
    }
}

fn hvac_chiller_plant() -> FeedTemplate {
    FeedTemplate {
        id: "feed-hvac-chiller".into(),
        name: "HVAC Chiller Plant".into(),
        industry: "Building Automation / HVAC".into(),
        description: "Central chiller plant with chillers, cooling towers, primary/secondary pumps, and building distribution.".into(),
        equipment_type: "Chiller Plant".into(),
        estimated_tag_count: 160,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Chillers".into(), quantity: "2-4".into(), description: "Centrifugal or screw water-cooled chillers".into() },
            FeedEquipmentUnit { name: "Cooling Towers".into(), quantity: "2-4".into(), description: "Induced draft cooling towers".into() },
            FeedEquipmentUnit { name: "Primary Pumps".into(), quantity: "1 per chiller".into(), description: "Constant-volume chilled water pumps".into() },
            FeedEquipmentUnit { name: "Secondary Pumps".into(), quantity: "2-3".into(), description: "Variable-volume distribution pumps".into() },
            FeedEquipmentUnit { name: "Condenser Pumps".into(), quantity: "1 per chiller".into(), description: "Condenser water pumps".into() },
        ],
        io_list: vec![
            FeedIO { signal: "CH_{n}_RUN".into(), description: "Chiller N running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Chillers".into(), criticality: "critical".into(), notes: "Chiller status".into() },
            FeedIO { signal: "CH_{n}_FLT".into(), description: "Chiller N fault".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Chillers".into(), criticality: "critical".into(), notes: "Chiller alarm contact".into() },
            FeedIO { signal: "TIT_CHW_S".into(), description: "Chilled water supply temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-15".into(), equipment: "Chillers".into(), criticality: "critical".into(), notes: "Main CHW supply header".into() },
            FeedIO { signal: "TIT_CHW_R".into(), description: "Chilled water return temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-20".into(), equipment: "Chillers".into(), criticality: "major".into(), notes: "Main CHW return header".into() },
            FeedIO { signal: "FIT_CHW".into(), description: "Chilled water flow".into(), io_type: "AI".into(), unit: "m³/h".into(), data_type: "float32".into(), typical_range: "0-2000".into(), equipment: "Chillers".into(), criticality: "major".into(), notes: "CHW flow meter".into() },
            FeedIO { signal: "TIT_CW_S".into(), description: "Condenser water supply temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "10-45".into(), equipment: "Cooling Towers".into(), criticality: "major".into(), notes: "To cooling tower".into() },
            FeedIO { signal: "TIT_CW_R".into(), description: "Condenser water return temperature".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "15-50".into(), equipment: "Cooling Towers".into(), criticality: "major".into(), notes: "From cooling tower".into() },
            FeedIO { signal: "CT_{n}_FAN_RUN".into(), description: "Cooling tower N fan running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Cooling Towers".into(), criticality: "major".into(), notes: "Fan VFD status".into() },
            FeedIO { signal: "PMP_CHWP_{n}_RUN".into(), description: "CHW primary pump N running".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Primary Pumps".into(), criticality: "critical".into(), notes: "Primary CHW pump".into() },
            FeedIO { signal: "PMP_CHWS_{n}_SPD".into(), description: "CHW secondary pump N speed".into(), io_type: "AI".into(), unit: "%".into(), data_type: "float32".into(), typical_range: "0-100".into(), equipment: "Secondary Pumps".into(), criticality: "major".into(), notes: "VFD speed for secondary loop".into() },
            FeedIO { signal: "PdIT_CHW".into(), description: "CHW differential pressure".into(), io_type: "AI".into(), unit: "kPa".into(), data_type: "float32".into(), typical_range: "0-500".into(), equipment: "Secondary Pumps".into(), criticality: "major".into(), notes: "Controls secondary pump speed".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Chillers".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "CH_{n}_FLT".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Chiller fault — unit may be down".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "TIT_CHW_S".into(), severity: "major".into(), condition: "high".into(), setpoint: "12°C".into(), deadband: "1".into(), description: "CHW supply temperature high — cooling capacity insufficient".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Secondary Pumps".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "PdIT_CHW".into(), severity: "major".into(), condition: "low".into(), setpoint: "50 kPa".into(), deadband: "10".into(), description: "CHW differential pressure low — distant AHUs not getting flow".into(), alarm_class: "Process".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Chiller plant overview with chiller status, temperatures, flows, and pump operation".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Chiller Plant".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Chillers".into(), equipment_group: "Chillers".into(), widget_type: "led".into(), signals: vec!["CH_{n}_RUN".into(), "CH_{n}_FLT".into()] },
                        FeedDashboardSection { title: "CHW Loop".into(), equipment_group: "Chillers".into(), widget_type: "trend".into(), signals: vec!["TIT_CHW_S".into(), "TIT_CHW_R".into(), "FIT_CHW".into()] },
                        FeedDashboardSection { title: "Condenser Loop".into(), equipment_group: "Cooling Towers".into(), widget_type: "trend".into(), signals: vec!["TIT_CW_S".into(), "TIT_CW_R".into()] },
                        FeedDashboardSection { title: "Pumps".into(), equipment_group: "Secondary Pumps".into(), widget_type: "gauge".into(), signals: vec!["PMP_CHWS_{n}_SPD".into(), "PdIT_CHW".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Chiller plants typically use BACnet/IP or Modbus TCP for chiller integration. Each chiller has its own controller (Carel, Danfoss, Trane) with a BACnet or Modbus gateway.".into()),
    }
}

fn power_distribution() -> FeedTemplate {
    FeedTemplate {
        id: "feed-power-distribution".into(),
        name: "Power Distribution".into(),
        industry: "Energy / Industrial".into(),
        description: "Low/medium voltage power distribution with switchgear, transformers, breakers, and power quality monitoring.".into(),
        equipment_type: "Power Distribution".into(),
        estimated_tag_count: 200,
        equipment_units: vec![
            FeedEquipmentUnit { name: "Main Incomer".into(), quantity: "1-2".into(), description: "MV/LV main incoming supply".into() },
            FeedEquipmentUnit { name: "Transformers".into(), quantity: "1-2".into(), description: "MV/LV or LV/LV transformers".into() },
            FeedEquipmentUnit { name: "Switchgear".into(), quantity: "1 per section".into(), description: "MV or LV switchgear sections".into() },
            FeedEquipmentUnit { name: "Feeders".into(), quantity: "4-20".into(), description: "Individual feeder breakers".into() },
            FeedEquipmentUnit { name: "Generator".into(), quantity: "1".into(), description: "Backup generator with ATS".into() },
        ],
        io_list: vec![
            // Main incomer
            FeedIO { signal: "INC_U_AB".into(), description: "Incomer voltage AB".into(), io_type: "AI".into(), unit: "V".into(), data_type: "float32".into(), typical_range: "0-690".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Line-to-line voltage".into() },
            FeedIO { signal: "INC_U_BC".into(), description: "Incomer voltage BC".into(), io_type: "AI".into(), unit: "V".into(), data_type: "float32".into(), typical_range: "0-690".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Line-to-line voltage".into() },
            FeedIO { signal: "INC_U_CA".into(), description: "Incomer voltage CA".into(), io_type: "AI".into(), unit: "V".into(), data_type: "float32".into(), typical_range: "0-690".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Line-to-line voltage".into() },
            FeedIO { signal: "INC_I_A".into(), description: "Incomer current phase A".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "CT input to power meter".into() },
            FeedIO { signal: "INC_I_B".into(), description: "Incomer current phase B".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "CT input".into() },
            FeedIO { signal: "INC_I_C".into(), description: "Incomer current phase C".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-5000".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "CT input".into() },
            FeedIO { signal: "INC_P".into(), description: "Incomer active power".into(), io_type: "AI".into(), unit: "kW".into(), data_type: "float32".into(), typical_range: "0-50000".into(), equipment: "Main Incomer".into(), criticality: "major".into(), notes: "Total 3-phase active power".into() },
            FeedIO { signal: "INC_Q".into(), description: "Incomer reactive power".into(), io_type: "AI".into(), unit: "kVAr".into(), data_type: "float32".into(), typical_range: "0-50000".into(), equipment: "Main Incomer".into(), criticality: "minor".into(), notes: "Total reactive power".into() },
            FeedIO { signal: "INC_PF".into(), description: "Incomer power factor".into(), io_type: "AI".into(), unit: "".into(), data_type: "float32".into(), typical_range: "0-1".into(), equipment: "Main Incomer".into(), criticality: "minor".into(), notes: "Power factor — penalty if <0.9".into() },
            FeedIO { signal: "INC_F".into(), description: "Incomer frequency".into(), io_type: "AI".into(), unit: "Hz".into(), data_type: "float32".into(), typical_range: "45-65".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Grid frequency".into() },
            FeedIO { signal: "INC_BKR".into(), description: "Incomer breaker closed".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Breaker status".into() },
            FeedIO { signal: "INC_BKR_TRIP".into(), description: "Incomer breaker trip".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Main Incomer".into(), criticality: "critical".into(), notes: "Protection relay trip".into() },
            // Transformer
            FeedIO { signal: "TX_{n}_TEMP_WND".into(), description: "Transformer N winding temp".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-200".into(), equipment: "Transformers".into(), criticality: "critical".into(), notes: "Winding temperature".into() },
            FeedIO { signal: "TX_{n}_TEMP_OIL".into(), description: "Transformer N oil temp".into(), io_type: "AI".into(), unit: "°C".into(), data_type: "float32".into(), typical_range: "0-120".into(), equipment: "Transformers".into(), criticality: "major".into(), notes: "Oil temperature (for oil-filled)".into() },
            // Feeders
            FeedIO { signal: "FDR_{n}_BKR".into(), description: "Feeder N breaker closed".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Feeders".into(), criticality: "major".into(), notes: "Breaker status".into() },
            FeedIO { signal: "FDR_{n}_BKR_TRIP".into(), description: "Feeder N breaker trip".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Feeders".into(), criticality: "critical".into(), notes: "Protection relay trip — equipment downstream may be offline".into() },
            FeedIO { signal: "FDR_{n}_I".into(), description: "Feeder N current".into(), io_type: "AI".into(), unit: "A".into(), data_type: "float32".into(), typical_range: "0-2000".into(), equipment: "Feeders".into(), criticality: "major".into(), notes: "Per-feeder current".into() },
            FeedIO { signal: "FDR_{n}_P".into(), description: "Feeder N power".into(), io_type: "AI".into(), unit: "kW".into(), data_type: "float32".into(), typical_range: "0-10000".into(), equipment: "Feeders".into(), criticality: "minor".into(), notes: "Per-feeder power".into() },
            // Generator (reuse GEN_ signals for consistency)
            FeedIO { signal: "GEN_BKR".into(), description: "Generator breaker closed".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Generator".into(), criticality: "critical".into(), notes: "ATS position feedback".into() },
            FeedIO { signal: "ATS_UTIL_OK".into(), description: "Utility supply healthy".into(), io_type: "DI".into(), unit: "".into(), data_type: "bool".into(), typical_range: "0-1".into(), equipment: "Generator".into(), criticality: "critical".into(), notes: "Normal supply available".into() },
        ],
        alarm_strategy: vec![
            FeedAlarmStrategy {
                equipment: "Main Incomer".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "INC_BKR_TRIP".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Main breaker trip — total site power loss".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "INC_F".into(), severity: "critical".into(), condition: "deviation".into(), setpoint: "±0.5 Hz".into(), deadband: "0.1".into(), description: "Grid frequency deviation — possible islanding".into(), alarm_class: "Safety".into() },
                    FeedAlarmRule { signal_pattern: "INC_U_AB".into(), severity: "major".into(), condition: "deviation".into(), setpoint: "±10%".into(), deadband: "2%".into(), description: "Voltage out of nominal range".into(), alarm_class: "Process".into() },
                    FeedAlarmRule { signal_pattern: "INC_PF".into(), severity: "minor".into(), condition: "low".into(), setpoint: "0.85".into(), deadband: "0.02".into(), description: "Power factor low — utility penalty risk".into(), alarm_class: "Process".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Transformers".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "TX_{n}_TEMP_WND".into(), severity: "critical".into(), condition: "high-high".into(), setpoint: "160°C".into(), deadband: "5".into(), description: "Transformer winding temp critical — forced cooling required".into(), alarm_class: "Equipment Protection".into() },
                    FeedAlarmRule { signal_pattern: "TX_{n}_TEMP_WND".into(), severity: "major".into(), condition: "high".into(), setpoint: "140°C".into(), deadband: "5".into(), description: "Transformer winding temp high".into(), alarm_class: "Equipment Protection".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Feeders".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "FDR_{n}_BKR_TRIP".into(), severity: "critical".into(), condition: "high".into(), setpoint: "1".into(), deadband: "0".into(), description: "Feeder breaker trip".into(), alarm_class: "Safety".into() },
                ],
            },
            FeedAlarmStrategy {
                equipment: "Generator".into(),
                rules: vec![
                    FeedAlarmRule { signal_pattern: "ATS_UTIL_OK".into(), severity: "critical".into(), condition: "low".into(), setpoint: "1".into(), deadband: "0".into(), description: "Utility supply lost — site on generator".into(), alarm_class: "Safety".into() },
                ],
            },
        ],
        dashboard: FeedDashboard {
            description: "Power distribution overview with incomer parameters, transformer monitoring, and feeder status".into(),
            recommended_screens: vec![
                FeedScreen {
                    name: "Power Distribution".into(),
                    layout: "grid".into(),
                    sections: vec![
                        FeedDashboardSection { title: "Incomer".into(), equipment_group: "Main Incomer".into(), widget_type: "gauge".into(), signals: vec!["INC_U_AB".into(), "INC_I_A".into(), "INC_P".into(), "INC_PF".into(), "INC_F".into()] },
                        FeedDashboardSection { title: "Transformer".into(), equipment_group: "Transformers".into(), widget_type: "gauge".into(), signals: vec!["TX_{n}_TEMP_WND".into(), "TX_{n}_TEMP_OIL".into()] },
                        FeedDashboardSection { title: "Feeder Status".into(), equipment_group: "Feeders".into(), widget_type: "led".into(), signals: vec!["FDR_{n}_BKR".into(), "FDR_{n}_I".into()] },
                        FeedDashboardSection { title: "Power Trend".into(), equipment_group: "Main Incomer".into(), widget_type: "trend".into(), signals: vec!["INC_P".into(), "INC_Q".into()] },
                    ],
                },
            ],
        },
        network_notes: Some("Power meters typically communicate via Modbus TCP or Modbus RTU over RS-485. Use Modbus TCP gateways for serial meters. Ensure CT ratios are configured correctly in the power meter for accurate readings.".into()),
    }
}
