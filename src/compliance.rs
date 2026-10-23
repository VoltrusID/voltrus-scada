//! Part 11 data shapes shared with the storage layer. The behaviour (signature
//! hashing, password policy, endpoints) lives in the Enterprise add-on.

use serde::{Deserialize, Serialize};

// ── Electronic Signature (FDA 21 CFR Part 11) ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ElectronicSignature {
    pub id: i64,
    pub user_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub reason: String,
    pub meaning: String,
    pub signature_hash: String,
    pub created_at: i64,
}

// ── Compliance Configuration ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ComplianceConfig {
    pub auto_logout_minutes: u32,
    pub password_min_length: u8,
    pub password_require_uppercase: bool,
    pub password_require_number: bool,
    pub password_require_special: bool,
    pub password_rotation_days: u32,
    pub authority_to_change: bool,
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            auto_logout_minutes: 15,
            password_min_length: 8,
            password_require_uppercase: true,
            password_require_number: true,
            password_require_special: true,
            password_rotation_days: 90,
            authority_to_change: false,
        }
    }
}
