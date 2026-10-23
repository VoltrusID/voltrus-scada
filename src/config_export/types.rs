// ── YAML-serializable config structs ──

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DeviceExport {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u64,
    pub endianness: String,
    pub enabled: bool,
    pub registers: Vec<RegisterExport>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct RegisterExport {
    pub name: String,
    pub address: u16,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
    pub sort_order: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub zero_based: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ScreenExport {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub sort_order: i64,
    pub data_sources: Vec<ScreenDataSourceExport>,
    pub layouts: Vec<ScreenLayoutExport>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ScreenDataSourceExport {
    pub data_source_id: i64,
    pub sort_order: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ScreenLayoutExport {
    pub breakpoint: String,
    pub layout: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FlowExport {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_config: serde_json::Value,
    pub nodes: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AlarmExport {
    pub source_key: String,
    pub source_name: String,
    pub severity: String,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NotificationExport {
    pub source_key: String,
    pub channel_type: String,
    pub config: serde_json::Value,
    pub enabled: bool,
}

/// Sanitize a string for use as a filename.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
