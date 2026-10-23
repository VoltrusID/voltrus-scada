#![allow(dead_code)]
// ── RBAC types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Role {
    pub id: i64,
    pub name: String,
    pub permissions: String, // JSON array: ["screens:read", "screens:write", ...]
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateRoleRequest {
    pub name: String,
    pub permissions: String,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub permissions: Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct AssignRoleRequest {
    pub role_id: i64,
}

// ── Permission constants ──

pub const PERM_SCREENS_READ: &str = "screens:read";
pub const PERM_SCREENS_WRITE: &str = "screens:write";
pub const PERM_DEVICES_READ: &str = "devices:read";
pub const PERM_DEVICES_WRITE: &str = "devices:write";
pub const PERM_ALARMS_READ: &str = "alarms:read";
pub const PERM_ALARMS_ACK: &str = "alarms:ack";
pub const PERM_SETTINGS_READ: &str = "settings:read";
pub const PERM_SETTINGS_WRITE: &str = "settings:write";
pub const PERM_USERS_MANAGE: &str = "users:manage";
pub const PERM_FLOWS_MANAGE: &str = "flows:manage";
pub const PERM_NOTIFICATIONS_MANAGE: &str = "notifications:manage";
pub const PERM_HUB_MANAGE: &str = "hub:manage";
pub const PERM_REPORTS_MANAGE: &str = "reports:manage";
pub const PERM_RECIPES_MANAGE: &str = "recipes:manage";
pub const PERM_COMMANDS_WRITE: &str = "commands:write";
pub const PERM_COMMANDS_READ: &str = "commands:read";
pub const PERM_COMMANDS_CANCEL: &str = "commands:cancel";
pub const PERM_TAGS_WRITE_POLICY: &str = "tags:write_policy";
pub const PERM_SIM_READ: &str = "sim:read";
pub const PERM_SIM_WRITE: &str = "sim:write";
pub const PERM_SCRIPTS_READ: &str = "scripts:read";
pub const PERM_SCRIPTS_WRITE: &str = "scripts:write";
pub const PERM_CONFIG_READ: &str = "config:read";
pub const PERM_CONFIG_WRITE: &str = "config:write";

/// All available permission constants for seeding.
pub const ALL_PERMISSIONS: &[&str] = &[
    PERM_SCREENS_READ,
    PERM_SCREENS_WRITE,
    PERM_DEVICES_READ,
    PERM_DEVICES_WRITE,
    PERM_ALARMS_READ,
    PERM_ALARMS_ACK,
    PERM_SETTINGS_READ,
    PERM_SETTINGS_WRITE,
    PERM_USERS_MANAGE,
    PERM_FLOWS_MANAGE,
    PERM_NOTIFICATIONS_MANAGE,
    PERM_HUB_MANAGE,
    PERM_REPORTS_MANAGE,
    PERM_RECIPES_MANAGE,
    PERM_COMMANDS_WRITE,
    PERM_COMMANDS_READ,
    PERM_COMMANDS_CANCEL,
    PERM_TAGS_WRITE_POLICY,
    PERM_SIM_READ,
    PERM_SIM_WRITE,
    PERM_SCRIPTS_READ,
    PERM_SCRIPTS_WRITE,
    PERM_CONFIG_READ,
    PERM_CONFIG_WRITE,
];

/// Read-only permissions for the Viewer role.
pub const VIEWER_PERMISSIONS: &[&str] = &[
    PERM_SCREENS_READ,
    PERM_DEVICES_READ,
    PERM_ALARMS_READ,
    PERM_SETTINGS_READ,
];

/// Near-admin permissions for the Demo role.
/// Can do almost everything except manage users, roles, API keys, security, and compliance.
pub const DEMO_PERMISSIONS: &[&str] = &[
    PERM_SCREENS_READ,
    PERM_SCREENS_WRITE,
    PERM_DEVICES_READ,
    PERM_DEVICES_WRITE,
    PERM_ALARMS_READ,
    PERM_ALARMS_ACK,
    PERM_SETTINGS_READ,
    PERM_SETTINGS_WRITE,
    PERM_FLOWS_MANAGE,
    PERM_NOTIFICATIONS_MANAGE,
    PERM_HUB_MANAGE,
    PERM_REPORTS_MANAGE,
    PERM_RECIPES_MANAGE,
    PERM_COMMANDS_WRITE,
    PERM_COMMANDS_READ,
    PERM_COMMANDS_CANCEL,
    PERM_TAGS_WRITE_POLICY,
    PERM_SIM_READ,
    PERM_SIM_WRITE,
    PERM_SCRIPTS_READ,
    PERM_SCRIPTS_WRITE,
    PERM_CONFIG_READ,
    PERM_CONFIG_WRITE,
];

/// Read + ack permissions for the Operator role.
pub const OPERATOR_PERMISSIONS: &[&str] = &[
    PERM_SCREENS_READ,
    PERM_DEVICES_READ,
    PERM_ALARMS_READ,
    PERM_ALARMS_ACK,
    PERM_SETTINGS_READ,
    PERM_COMMANDS_WRITE,
    PERM_COMMANDS_READ,
    PERM_SIM_READ,
    PERM_SCRIPTS_READ,
    PERM_CONFIG_READ,
];
