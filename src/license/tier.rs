// ── Entitlement tier ──

/// Which add-on features are unlocked.
///
/// `Community` is the open core: everything in this repository works without a
/// licence. `Integrator` and `Enterprise` unlock the add-on features that ship
/// in the separate Enterprise add-on (see `docs/open-core.md`).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LicenseTier {
    #[default]
    Community,
    Integrator,
    Enterprise,
}

impl LicenseTier {
    /// Maps a tier string from a signed activation to a tier.
    ///
    /// Current names: `integrator`, `enterprise`. Keys issued before the open-core
    /// split keep working: `professional`/`pro` map to Integrator, `enterprise`
    /// stays Enterprise, and `edge`/`starter`/`engineering` (and anything unknown)
    /// map to Community because the core now includes everything those tiers had.
    pub fn from_tier_str(s: Option<&str>) -> Self {
        match s.map(|t| t.trim().to_lowercase()).as_deref() {
            Some("integrator") | Some("professional") | Some("pro") => LicenseTier::Integrator,
            Some("enterprise") => LicenseTier::Enterprise,
            _ => LicenseTier::Community,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LicenseTier::Community => "community",
            LicenseTier::Integrator => "integrator",
            LicenseTier::Enterprise => "enterprise",
        }
    }

    fn integrator_or_above(&self) -> bool {
        !matches!(self, LicenseTier::Community)
    }

    fn enterprise_only(&self) -> bool {
        matches!(self, LicenseTier::Enterprise)
    }

    // ── Integrator and above ──

    /// White-label branding (custom logo and product name).
    pub fn white_label(&self) -> bool {
        self.integrator_or_above()
    }

    /// Full theme editor (colors, fonts, layouts).
    pub fn theme_full(&self) -> bool {
        self.integrator_or_above()
    }

    /// Scheduled PDF reports.
    pub fn pdf_reporting(&self) -> bool {
        self.integrator_or_above()
    }

    /// Telegram and WhatsApp alarm notifications.
    pub fn chat_notifications(&self) -> bool {
        self.integrator_or_above()
    }

    /// Multi-site hub.
    pub fn multi_site(&self) -> bool {
        self.integrator_or_above()
    }

    // ── Enterprise only ──

    /// Single sign-on (OIDC bearer tokens).
    pub fn sso(&self) -> bool {
        self.enterprise_only()
    }

    /// SAML 2.0 and LDAP sign-in.
    pub fn sso_saml(&self) -> bool {
        self.enterprise_only()
    }

    /// 21 CFR Part 11 mode (electronic signatures, compliance policy).
    pub fn cfr_part_11(&self) -> bool {
        self.enterprise_only()
    }

    /// IEC 62443 security audit mode (security event log and thresholds).
    pub fn iec_62443(&self) -> bool {
        self.enterprise_only()
    }

    /// Hot standby with automatic failover.
    pub fn ha(&self) -> bool {
        self.enterprise_only()
    }

    /// RTSP video cameras.
    pub fn rtsp_video(&self) -> bool {
        self.enterprise_only()
    }

    /// All add-on flags as a JSON object. Core features have no flag: they are
    /// always on.
    pub fn to_feature_flags(&self) -> serde_json::Value {
        serde_json::json!({
            "white_label": self.white_label(),
            "theme_full": self.theme_full(),
            "pdf_reporting": self.pdf_reporting(),
            "chat_notifications": self.chat_notifications(),
            "multi_site": self.multi_site(),
            "sso": self.sso(),
            "sso_saml": self.sso_saml(),
            "cfr_part_11": self.cfr_part_11(),
            "iec_62443": self.iec_62443(),
            "ha": self.ha(),
            "rtsp_video": self.rtsp_video(),
        })
    }
}
