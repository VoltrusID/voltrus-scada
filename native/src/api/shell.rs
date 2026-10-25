//! Mirrors of what the console's shell asks the server: the user's
//! preferences (`settings/ui-theme`, `settings/ui-lang`), the alarm
//! cooldown, keyboard shortcuts, user notifications, the licence
//! (`/api/license/status`) and the sign-in methods (`auth/providers`).
//! Field names and shapes follow `src/api/{settings,shortcuts,
//! user_notifications,license}.rs` and `src/api/auth/providers.rs`.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::VoltrusApi;

impl VoltrusApi {
    /// `GET path` on a public route (no cookie): the licence and the
    /// sign-in methods, asked before anyone signs in.
    pub async fn get_public<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.http.get(format!("{}{path}", self.base)).send().await?;
        Self::decode(resp).await
    }
}

/// `GET`/`PUT /api/v1/settings/ui-theme`: `{theme}`, one of `light`,
/// `dark` or `system` (the server refuses anything else; `dark` when
/// unset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiTheme {
    pub theme: String,
}

/// The appearance the server keeps per user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    /// Follow the operating system.
    System,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::System];

    /// The server's word; anything unknown reads as the server's default,
    /// dark.
    pub fn parse(text: &str) -> Self {
        match text {
            "light" => Self::Light,
            "system" => Self::System,
            _ => Self::Dark,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }

    /// Dark or not, given whether the system is dark.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => system_dark,
        }
    }
}

/// `GET`/`PUT /api/v1/settings/ui-lang`: `{lang}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiLang {
    pub lang: String,
}

/// The languages the server accepts for `ui-lang` (`en` when unset), with
/// their own names.
pub const LANGUAGES: [(&str, &str); 4] = [
    ("en", "English"),
    ("id", "Bahasa Indonesia"),
    ("zh", "中文"),
    ("de", "Deutsch"),
];

/// `GET /api/v1/settings/alarm-cooldown` (`{secs}`, 300 when unset) and
/// the answer to `PUT` (`{ok, secs}` with `secs` clamped to 0–86400).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct AlarmCooldown {
    pub secs: i64,
}

/// The longest cooldown the server keeps, in seconds.
pub const MAX_COOLDOWN_SECS: i64 = 86_400;

/// `GET /api/v1/shortcuts` (`{data_source_id: combo}`) and
/// `GET /api/v1/nav-shortcuts` (`{target: combo}`, defaults merged with the
/// user's choices, `""` for none). Combos are the web client's
/// `Mod+Shift+K` form. `PUT` takes the same map with `null` (or `""`) to
/// remove an entry.
pub type ShortcutMap = BTreeMap<String, String>;

/// Mirrors `models::UserNotification`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct UserNotification {
    pub id: i64,
    pub user_id: i64,
    pub from_user_id: i64,
    /// e.g. `shift_handover`, `annotation`.
    pub notification_type: String,
    pub message: String,
    pub reference_id: Option<i64>,
    pub read: bool,
    /// Unix seconds.
    pub created_at: i64,
}

/// `GET /api/v1/user-notifications[?unread_only=true]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct UserNotifications {
    pub notifications: Vec<UserNotification>,
    pub count: usize,
}

/// `GET /api/license/status`: the licence status (`status` is the variant
/// name: `Community`, `Valid`, `Unactivated`, `Expired`, …), the tier it
/// resolves to and the feature flags. Like the web client, anything
/// unrecognised reads as Community: the core never depends on it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LicenseStatus {
    pub status: String,
    pub tier: String,
    pub feature_flags: BTreeMap<String, serde_json::Value>,
    pub expires_at: Option<String>,
    pub days_remaining: Option<i64>,
    pub grace: bool,
    pub message: Option<String>,
}

/// The edition a licence unlocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Community,
    Integrator,
    Enterprise,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Community => "Community",
            Self::Integrator => "Integrator",
            Self::Enterprise => "Enterprise",
        }
    }
}

impl LicenseStatus {
    pub fn tier(&self) -> Tier {
        match self.tier.as_str() {
            "integrator" => Tier::Integrator,
            "enterprise" => Tier::Enterprise,
            _ => Tier::Community,
        }
    }

    /// Whether the licence unlocks `flag` (`sso`, `white_label`, …).
    pub fn has(&self, flag: &str) -> bool {
        self.feature_flags.get(flag) == Some(&serde_json::Value::Bool(true))
    }

    /// The unlocked flags, by name.
    pub fn unlocked(&self) -> Vec<&str> {
        self.feature_flags
            .iter()
            .filter(|(_, value)| **value == serde_json::Value::Bool(true))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// One line about the licence beyond its tier, if there is anything to
    /// say: why it does not count, or when it runs out.
    pub fn note(&self) -> Option<String> {
        match self.status.as_str() {
            "" | "Community" => None,
            "Valid" if self.grace => Some("Lapsed: in its grace period".into()),
            "Valid" => match self.days_remaining {
                Some(days) if days <= 30 => Some(match days {
                    1 => "Renews or ends in 1 day".into(),
                    days => format!("Renews or ends in {days} days"),
                }),
                Some(_) => self
                    .expires_at
                    .as_ref()
                    .map(|at| format!("Valid until {}", at.get(..10).unwrap_or(at.as_str()))),
                None => Some("Lifetime licence".into()),
            },
            status => Some(
                self.message
                    .clone()
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| format!("Licence {status}: running as Community")),
            ),
        }
    }
}

/// `GET /api/v1/auth/providers`: the sign-in methods. The core lists the
/// password (`local`) only; an add-on edition appends its own (SSO).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AuthProviders {
    pub providers: Vec<AuthProvider>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthProvider {
    pub provider: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub label: String,
}

fn enabled_by_default() -> bool {
    true
}

/// How the console can use a sign-in method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInMethod {
    /// The username and password form.
    Password,
    /// Listed, but this console cannot use it; why.
    Unavailable(String),
}

impl AuthProvider {
    pub fn name(&self) -> &str {
        if self.label.is_empty() {
            &self.provider
        } else {
            &self.label
        }
    }

    /// The password works here. Other methods (SSO) finish in a browser
    /// that holds the session cookie; the server has no hand-off that
    /// gives a desktop client the session (no loopback or app redirect, no
    /// device code), so they are listed with the reason.
    pub fn method(&self) -> SignInMethod {
        if !self.enabled {
            SignInMethod::Unavailable("Turned off on the server".into())
        } else if self.provider == "local" {
            SignInMethod::Password
        } else {
            SignInMethod::Unavailable(
                "Signs in through the browser; the server offers no hand-off to the desktop console"
                    .into(),
            )
        }
    }
}

impl AuthProviders {
    /// The methods other than the password.
    pub fn others(&self) -> impl Iterator<Item = &AuthProvider> {
        self.providers
            .iter()
            .filter(|provider| provider.provider != "local")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_decode_the_servers_answers() {
        let theme: UiTheme = serde_json::from_str(r#"{"theme":"system"}"#).unwrap();
        assert_eq!(ThemeMode::parse(&theme.theme), ThemeMode::System);
        assert_eq!(ThemeMode::parse("purple"), ThemeMode::Dark, "the default");
        assert!(ThemeMode::System.is_dark(true) && !ThemeMode::System.is_dark(false));
        assert!(!ThemeMode::Light.is_dark(true));
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::parse(mode.as_str()), mode);
        }
        // `PUT` answers `{ok, theme}`; the extra field is ignored.
        let saved: UiTheme = serde_json::from_str(r#"{"ok":true,"theme":"light"}"#).unwrap();
        assert_eq!(saved.theme, "light");
        let lang: UiLang = serde_json::from_str(r#"{"lang":"id"}"#).unwrap();
        assert!(LANGUAGES.iter().any(|(code, _)| *code == lang.lang));
        let cooldown: AlarmCooldown = serde_json::from_str(r#"{"ok":true,"secs":86400}"#).unwrap();
        assert_eq!(cooldown.secs, MAX_COOLDOWN_SECS);
        let nav: ShortcutMap = serde_json::from_str(
            r#"{"screen:all":"Mod+1","screen:system":"Mod+0","view:cards":"","action:alarms":"Mod+Shift+A"}"#,
        )
        .unwrap();
        assert_eq!(nav["action:alarms"], "Mod+Shift+A");
        let tags: ShortcutMap = serde_json::from_str(r#"{"12":"Alt+F2"}"#).unwrap();
        assert_eq!(tags["12"], "Alt+F2");
    }

    #[test]
    fn user_notifications_decode_the_list() {
        let list: UserNotifications = serde_json::from_str(
            r#"{"notifications":[{"id":7,"user_id":2,"from_user_id":1,
                "notification_type":"shift_handover",
                "message":"You were mentioned in a shift handover log for 2026-10-25 (morning)",
                "reference_id":31,"read":false,"created_at":1761379200},
               {"id":8,"user_id":2,"from_user_id":1,"notification_type":"annotation",
                "message":"m","reference_id":null,"read":true,"created_at":1761379300}],
               "count":2}"#,
        )
        .unwrap();
        assert_eq!(list.count, 2);
        assert_eq!(list.notifications[0].reference_id, Some(31));
        assert!(!list.notifications[0].read && list.notifications[1].read);
        assert_eq!(list.notifications[1].reference_id, None);
    }

    #[test]
    fn the_licence_reads_like_the_web_client() {
        // The open core: `LicenseStatus::Community` plus tier and flags.
        let community: LicenseStatus = serde_json::from_str(
            r#"{"status":"Community","tier":"community","feature_flags":{"white_label":false,
                "theme_full":false,"pdf_reporting":false,"sms_notifications":false,
                "multi_site":false,"sso":false,"sso_saml":false,"cfr_part_11":false,
                "iec_62443":false,"ha":false,"rtsp_video":false}}"#,
        )
        .unwrap();
        assert_eq!(community.tier(), Tier::Community);
        assert!(!community.has("sso"));
        assert!(community.unlocked().is_empty());
        assert_eq!(community.note(), None);

        let valid: LicenseStatus = serde_json::from_str(
            r#"{"status":"Valid","license_key":"VT-1","expires_at":"2026-11-03T00:00:00Z",
                "days_remaining":9,"tier":"enterprise","grace":false,
                "feature_flags":{"sso":true,"ha":true,"white_label":true}}"#,
        )
        .unwrap();
        assert_eq!(valid.tier(), Tier::Enterprise);
        assert!(valid.has("sso"));
        assert_eq!(valid.unlocked(), ["ha", "sso", "white_label"]);
        assert_eq!(valid.note().as_deref(), Some("Renews or ends in 9 days"));

        let expired: LicenseStatus = serde_json::from_str(
            r#"{"status":"Expired","expires_at":"2026-01-01","message":"Licence expired on 2026-01-01","tier":"community","feature_flags":{}}"#,
        )
        .unwrap();
        assert_eq!(expired.tier(), Tier::Community);
        assert_eq!(
            expired.note().as_deref(),
            Some("Licence expired on 2026-01-01")
        );

        let odd: LicenseStatus = serde_json::from_str(r#"{"tier":"platinum"}"#).unwrap();
        assert_eq!(odd.tier(), Tier::Community, "unknown tiers are Community");
    }

    #[test]
    fn sign_in_methods_list_the_password_and_say_why_others_wait() {
        // The core's answer.
        let core: AuthProviders = serde_json::from_str(
            r#"{"providers":[{"provider":"local","enabled":true,"label":"Local Authentication"}]}"#,
        )
        .unwrap();
        assert_eq!(core.providers[0].method(), SignInMethod::Password);
        assert_eq!(core.others().count(), 0);

        // An edition's extra entries.
        let edition: AuthProviders = serde_json::from_str(
            r#"{"providers":[{"provider":"local","enabled":true,"label":"Local Authentication"},
                {"provider":"oidc","enabled":true,"label":"Azure AD"},
                {"provider":"saml","enabled":false}]}"#,
        )
        .unwrap();
        let others: Vec<_> = edition.others().collect();
        assert_eq!(others.len(), 2);
        assert_eq!(others[0].name(), "Azure AD");
        assert!(
            matches!(others[0].method(), SignInMethod::Unavailable(why) if why.contains("browser"))
        );
        assert_eq!(others[1].name(), "saml", "no label: the provider id");
        assert_eq!(
            others[1].method(),
            SignInMethod::Unavailable("Turned off on the server".into())
        );
    }
}
