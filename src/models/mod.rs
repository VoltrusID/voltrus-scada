pub mod alarm_types;
pub mod analytics;
pub mod api_types;
#[cfg(feature = "bacnet")]
pub mod bacnet_types;
pub mod command_types;
pub mod config_types;
#[cfg(feature = "dnp3")]
pub mod dnp3_types;
pub mod notification_types;
pub mod operations;
pub mod protocol_device_types;
pub mod rbac_types;

pub use alarm_types::*;
pub use analytics::*;
pub use api_types::*;
#[cfg(feature = "bacnet")]
pub use bacnet_types::*;
pub use command_types::*;
pub use config_types::*;
#[cfg(feature = "dnp3")]
pub use dnp3_types::*;
pub use notification_types::*;
pub use operations::*;
pub use protocol_device_types::*;
pub use rbac_types::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

// ── Runtime types ──

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeviceSnapshot {
    pub device_id: String,
    pub status: DeviceStatus,
    pub timestamp: i64,
    pub metrics: HashMap<String, f64>,
    #[serde(default)]
    pub simulated: bool,
    /// Acquisition protocol that produced this snapshot ("modbus", "opcua",
    /// "ethernet_ip", "s7", "dnp3", "bacnet", ...). Populated by the poller
    /// manager so flow-engine triggers match on the real source. `None` falls
    /// back to "modbus" for back-compat with pre-protocol snapshots.
    #[serde(default)]
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Online,
    Offline,
}

// ── Application entities ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    #[serde(skip_serializing)]
    pub salt: String,
    #[serde(default = "api_types::default_user_role")]
    pub role: String,
    #[serde(default)]
    pub must_change_password: bool,
    #[serde(skip_serializing)]
    pub failed_attempts: i64,
    #[serde(skip_serializing)]
    pub locked_until: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub device_type: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct DataSource {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub unit: String,
    pub color: String,
    pub min_val: f64,
    pub max_val: f64,
    pub warn_val: f64,
    pub crit_val: f64,
    pub base_val: f64,
    pub sort_order: i64,
    pub decimals: i64,
    pub source_type: String,
    pub source_config: String,
    pub device_id: String,
    pub equipment_id: Option<String>,
    pub chart_type: String,
    pub path: Option<String>,
    /// ISA-18.2 on/off delay seconds for alarm evaluation. 0 = immediate
    /// (historical behavior). Set via PUT /api/v1/data-sources/{id}.
    #[serde(default)]
    pub on_delay_secs: i64,
    #[serde(default)]
    pub off_delay_secs: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ScreenDataSource {
    pub screen_id: String,
    pub data_source_id: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ScreenLayoutRow {
    pub screen_id: String,
    pub breakpoint: String,
    pub layout: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TwinLayout {
    pub screen_id: String,
    pub equipment_id: String,
    pub equipment_type: String,
    pub equipment_name: String,
    pub position_x: f64,
    pub position_z: f64,
    pub rotation_y: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct TwinConnection {
    pub screen_id: String,
    pub from_id: String,
    pub to_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct CanvasLayoutItem {
    pub screen_id: String,
    pub data_source_id: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ── SCADA 2D view types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ScadaEquipment {
    pub screen_id: String,
    pub equipment_id: String,
    pub equipment_type: String,
    pub equipment_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub sensor_bindings: String,
    #[serde(default)]
    pub config: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct ScadaConnection {
    pub screen_id: String,
    pub connection_id: String,
    pub from_equipment: String,
    pub to_equipment: String,
    pub waypoints: String,
    pub connection_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserShortcut {
    pub user_id: i64,
    pub data_source_id: i64,
    pub shortcut: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavShortcut {
    pub user_id: i64,
    pub target: String,
    pub shortcut: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

// ── Audit log types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct AuditLogEntry {
    pub id: i64,
    pub action: String,
    pub username: String,
    pub resource: String,
    pub details: Option<String>,
    pub created_at: i64,
}

// ── Flow types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_config: String,
    pub nodes: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct FlowRun {
    pub id: i64,
    pub flow_id: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub success: bool,
    pub error: Option<String>,
    pub output: Option<String>,
}

// ── AI Copilot types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiSettingsResponse {
    pub provider: String,
    pub api_key_set: bool,
    pub api_key_preview: String,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct RegisterMapRequest {
    pub device_name: String,
    pub file_content: String,
    pub file_type: String,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct DashboardLayoutRequest {
    pub description: String,
    pub screen_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmThresholdRequest {
    pub device_type: String,
    pub tags: Vec<AlarmThresholdTag>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmThresholdTag {
    pub source_key: String,
    pub name: String,
    pub unit: Option<String>,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct TagRenameRequest {
    pub tags: Vec<TagRenameEntry>,
    pub convention: String,
    pub custom_pattern: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct TagRenameEntry {
    pub current: String,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct FlowRuleRequest {
    pub description: String,
    pub available_tags: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct TemplateInstantiateRequest {
    pub template_id: i64,
    pub prefix: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiProposal {
    pub id: i64,
    pub proposal_type: String,
    pub title: String,
    pub context: String,
    pub proposed_config: serde_json::Value,
    pub target_entity_id: Option<String>,
    pub model: String,
    pub tokens_used: i64,
    pub status: String,
    pub reviewed_by: Option<String>,
    pub applied_config: Option<serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EquipmentTemplate {
    pub id: i64,
    pub name: String,
    pub industry: Option<String>,
    pub equipment_type: String,
    pub tag_template: serde_json::Value,
    pub alarm_template: serde_json::Value,
    pub dashboard_template: serde_json::Value,
    pub flow_template: serde_json::Value,
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_count() {
        let r = RegisterConfig {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Uint16,
            scale: 1.0,
            register_area: "holding".into(),
            zero_based: false,
        };
        assert_eq!(r.register_count(), 1);

        let r = RegisterConfig {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
            zero_based: false,
        };
        assert_eq!(r.register_count(), 2);
    }

    #[test]
    fn test_protocol_address() {
        let r = RegisterConfig {
            name: "v".into(),
            address: 40001,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
            zero_based: false,
        };
        assert_eq!(r.protocol_address(), 0);

        let r = RegisterConfig {
            name: "v".into(),
            address: 40003,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "holding".into(),
            zero_based: false,
        };
        assert_eq!(r.protocol_address(), 2);

        let r = RegisterConfig {
            name: "v".into(),
            address: 30005,
            reg_type: RegisterType::Float32,
            scale: 1.0,
            register_area: "input".into(),
            zero_based: false,
        };
        assert_eq!(r.protocol_address(), 4);

        let r = RegisterConfig {
            name: "v".into(),
            address: 10,
            reg_type: RegisterType::Uint16,
            scale: 1.0,
            register_area: "holding".into(),
            zero_based: false,
        };
        assert_eq!(r.protocol_address(), 10);
    }

    #[test]
    fn test_default_scale() {
        assert_eq!(default_scale(), 1.0);
    }

    #[test]
    fn test_data_point_serde() {
        let dp = DataPoint {
            timestamp: 1710000000,
            value: 220.5,
        };
        let json = serde_json::to_string(&dp).unwrap();
        assert!(json.contains("\"timestamp\":1710000000"));
        assert!(json.contains("\"value\":220.5"));

        let back: DataPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, dp);
    }

    #[test]
    fn test_machine_state_serde() {
        let state = MachineState::Running;
        let json = serde_json::to_string(&state).unwrap();
        let expected = r#""running""#;
        assert_eq!(json, expected);
        let back: MachineState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MachineState::Running);
        assert!(MachineState::from_str("running").unwrap() == MachineState::Running);
        assert!(MachineState::from_str("idle").unwrap() == MachineState::Idle);
        assert!(MachineState::from_str("faulted").unwrap() == MachineState::Faulted);
        assert!(MachineState::from_str("setup").unwrap() == MachineState::Setup);
        assert!(MachineState::from_str("maintenance").unwrap() == MachineState::Maintenance);
        assert!(MachineState::from_str("unknown").is_none());
    }

    #[test]
    fn test_create_downtime_event_request_deserialize() {
        let json = r#"{"device_id":"dev1","state":"faulted"}"#;
        let req: CreateDowntimeEventRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.device_id, "dev1");
        assert_eq!(req.state, "faulted");
    }

    #[test]
    fn test_oee_metrics_structure() {
        let metrics = OeeMetrics {
            availability: 0.85,
            performance: 0.90,
            quality: 0.95,
            oee: 0.85 * 0.90 * 0.95,
            total_running_secs: 3600,
            total_planned_secs: 4235,
            downtime_count: 3,
        };
        let json = serde_json::to_string(&metrics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["availability"], 0.85);
        assert_eq!(parsed["total_running_secs"], 3600);
    }

    #[test]
    fn test_permission_constants() {
        assert_eq!(PERM_SCREENS_READ, "screens:read");
        assert_eq!(PERM_SCREENS_WRITE, "screens:write");
        assert_eq!(ALL_PERMISSIONS.len(), 24);
        assert!(ALL_PERMISSIONS.contains(&PERM_USERS_MANAGE));
    }

    #[test]
    fn test_role_serde() {
        let role = Role {
            id: 1,
            name: "Admin".to_string(),
            permissions: r#"["screens:read"]"#.to_string(),
            created_at: 1000,
        };
        let json = serde_json::to_string(&role).unwrap();
        assert!(json.contains("\"name\":\"Admin\""));
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 1);
        assert_eq!(back.name, "Admin");
    }

    #[test]
    fn test_create_role_request_deserialize() {
        let json = r#"{"name":"Operator","permissions":"[\"screens:read\"]"}"#;
        let req: CreateRoleRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Operator");
        assert_eq!(req.permissions, r#"["screens:read"]"#);
    }

    #[test]
    fn test_command_status_roundtrip() {
        for status in [
            CommandStatus::Pending,
            CommandStatus::Queued,
            CommandStatus::Sent,
            CommandStatus::Acked,
            CommandStatus::Failed,
            CommandStatus::Expired,
            CommandStatus::Cancelled,
        ] {
            let s = status.as_str();
            assert_eq!(CommandStatus::from_str(s), Some(status.clone()));
            // serde round-trip
            let json = serde_json::to_string(&status).unwrap();
            let parsed: CommandStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
        assert!(CommandStatus::from_str("unknown").is_none());
        assert!(CommandStatus::Acked.is_terminal());
        assert!(!CommandStatus::Pending.is_terminal());
    }

    #[test]
    fn test_write_policy_update_serde() {
        let json = r#"{"writable": true, "write_min": null}"#;
        let parsed: WritePolicyUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.writable, Some(true));
        assert_eq!(parsed.write_min, Some(None));
        // Fields not in JSON should be None (not provided)
        assert_eq!(parsed.write_max, None);
    }
}
