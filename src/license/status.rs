use super::tier::LicenseTier;
use serde::{Deserialize, Serialize};

// ── Licence status ──

/// The site a per-site licence key is issued for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseSite {
    pub id: String,
    pub name: String,
}

/// Annual support contract state, derived from `support_until` and the clock.
/// Informational only: nothing in Core is gated on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportState {
    Active,
    Lapsed,
    None,
}

impl SupportState {
    /// `support_until` is active while `now` is before it, lapsed from it on.
    pub fn at(support_until: Option<i64>, now: i64) -> Self {
        match support_until {
            None => SupportState::None,
            Some(until) if now < until => SupportState::Active,
            Some(_) => SupportState::Lapsed,
        }
    }
}

/// Why the current tier is what it is.
///
/// Only `Valid` unlocks add-on features. Every other status runs as Community:
/// an invalid, expired or missing licence never blocks the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        /// The site this key is issued for (per-site licences). `None` for
        /// keys issued before per-site licensing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<LicenseSite>,
        /// End of the annual support contract, Unix seconds. `None` = no
        /// support contract (lifetime keys, grandfathered keys). Lapsed
        /// support never limits Core or anything already licensed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        support_until: Option<i64>,
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

    /// Support contract state at `now` (Unix seconds). `None` unless `Valid`
    /// with a `support_until`.
    pub fn support_at(&self, now: i64) -> SupportState {
        match self {
            LicenseStatus::Valid { support_until, .. } => SupportState::at(*support_until, now),
            _ => SupportState::None,
        }
    }

    /// Support contract state right now.
    pub fn support(&self) -> SupportState {
        self.support_at(time::OffsetDateTime::now_utc().unix_timestamp())
    }

    /// The per-site and support fields every licence endpoint reports:
    /// `site` (object or null), `support_until` (Unix seconds or null) and the
    /// derived `support` (`active`, `lapsed`, `none`).
    pub fn site_support_json(&self) -> serde_json::Value {
        let (site, support_until) = match self {
            LicenseStatus::Valid {
                site,
                support_until,
                ..
            } => (site.clone(), *support_until),
            _ => (None, None),
        };
        serde_json::json!({
            "site": site,
            "support_until": support_until,
            "support": self.support(),
        })
    }

    pub fn feature_flags(&self) -> serde_json::Value {
        let mut flags = self.tier().to_feature_flags();
        flags["white_label"] = self.white_label().into();
        flags
    }
}
