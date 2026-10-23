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
        /// An add-on edition grants logo-only white-label branding on top of
        /// the resolved tier (no theme editor). Used to keep a retired
        /// entitlement working for existing customers.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        legacy_white_label: bool,
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

    /// Whether white-label branding (logo) is unlocked: by the tier, or by an
    /// add-on edition's logo-only grant.
    pub fn white_label(&self) -> bool {
        self.tier().white_label()
            || matches!(
                self,
                LicenseStatus::Valid {
                    legacy_white_label: true,
                    ..
                }
            )
    }

    pub fn feature_flags(&self) -> serde_json::Value {
        let mut flags = self.tier().to_feature_flags();
        flags["white_label"] = self.white_label().into();
        flags
    }
}
