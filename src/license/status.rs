use super::tier::LicenseTier;
use serde::Serialize;

// ── Licence status ──

/// Why the current tier is what it is.
///
/// Only `Valid` unlocks add-on features. Every other status runs as Community:
/// an invalid, expired or missing licence never blocks the core.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum LicenseStatus {
    /// Open core without an add-on edition.
    Community,
    Valid {
        license_key: String,
        /// `None` for lifetime keys.
        expires_at: Option<String>,
        days_remaining: Option<i64>,
        tier: Option<String>,
        /// True while a lapsed monthly key is inside its 7 day grace period.
        grace: bool,
    },
    Unactivated {
        message: String,
    },
    InvalidFingerprint {
        score: usize,
        required: usize,
        message: String,
    },
    Expired {
        expires_at: String,
        message: String,
    },
    InvalidSignature {
        message: String,
    },
    Corrupted {
        message: String,
    },
}

impl LicenseStatus {
    /// The tier this status unlocks. Anything but `Valid` is Community.
    pub fn tier(&self) -> LicenseTier {
        match self {
            LicenseStatus::Valid { tier, .. } => LicenseTier::from_tier_str(tier.as_deref()),
            _ => LicenseTier::Community,
        }
    }

    pub fn feature_flags(&self) -> serde_json::Value {
        self.tier().to_feature_flags()
    }
}
