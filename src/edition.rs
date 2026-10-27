//! Extension points for add-on editions.
//!
//! The open core is complete on its own: a plain `voltrus` build runs every
//! protocol driver, the historian, alarms, dashboards and the REST API with no
//! licence, no device or tag limits and no network calls to Voltrus servers.
//!
//! A downstream build can link this crate as a library and pass an
//! [`Extension`] to [`crate::run`]. The extension may add HTTP routes, background
//! tasks, CLI subcommands and entitlements (the [`LicenseTier`] that unlocks
//! add-on features). The official Voltrus Enterprise add-on is built that way
//! and is not part of this repository.

use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};

use axum::Router;

use crate::api::AppState;
use crate::cache::DeviceCache;
use crate::license::LicenseStatus;
use crate::models::{AppConfig, SmtpConfig};
use crate::notification::email::EmailNotifier;
use crate::poller_manager::PollerManager;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Routes contributed by an extension, merged into the matching core group.
///
/// * `public`: no authentication (like `/api/v1/health`).
/// * `protected`: any authenticated user.
/// * `operator`: admin, demo and integration roles (device and screen configuration).
/// * `admin`: admin users only.
///
/// Extension routes sit behind the same CORS, rate limit and security header
/// layers as core routes.
pub struct ExtensionRoutes {
    pub public: Router<AppState>,
    pub protected: Router<AppState>,
    pub operator: Router<AppState>,
    pub admin: Router<AppState>,
}

impl Default for ExtensionRoutes {
    fn default() -> Self {
        Self {
            public: Router::new(),
            protected: Router::new(),
            operator: Router::new(),
            admin: Router::new(),
        }
    }
}

/// An identity proven by an external provider (for example an OIDC bearer token).
pub struct ExternalIdentity {
    pub username: String,
    pub role: String,
}

/// Everything an extension needs to start its background work.
pub struct StartContext {
    pub config: AppConfig,
    pub config_path: String,
    pub storage: Storage,
    pub cache: DeviceCache,
    pub ws_manager: Arc<WsManager>,
    pub poller_manager: PollerManager,
    pub license_status: Arc<RwLock<LicenseStatus>>,
    pub email_notifier: Option<EmailNotifier>,
}

/// Sends alarm text messages. Registered by an extension that provides SMS.
pub trait SmsSender: Send + Sync {
    /// Email-to-SMS gateway domain for a carrier id, `None` if unsupported.
    fn carrier_gateway(&self, carrier: &str) -> Option<&'static str>;

    fn send_alarm<'a>(
        &'a self,
        smtp: &'a SmtpConfig,
        phone: &'a str,
        carrier: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
}

static SMS_SENDER: OnceLock<Arc<dyn SmsSender>> = OnceLock::new();
static HOSTED_AI: OnceLock<Arc<dyn HostedAi>> = OnceLock::new();

/// Register the SMS sender (first registration wins).
pub fn register_sms_sender(sender: Arc<dyn SmsSender>) {
    let _ = SMS_SENDER.set(sender);
}

/// The registered SMS sender, if an extension provides one.
pub fn sms_sender() -> Option<&'static Arc<dyn SmsSender>> {
    SMS_SENDER.get()
}

/// A hosted AI completion service for the `voltrus_ai` copilot provider.
/// Registered by an add-on edition; Core only talks to the AI providers the
/// operator configures with their own key (`byok_openai`, `byok_anthropic`).
pub trait HostedAi: Send + Sync {
    fn complete<'a>(
        &'a self,
        api_key: &'a str,
        model: &'a str,
        request: crate::ai_copilot::provider::AiRequest,
    ) -> BoxFuture<
        'a,
        Result<crate::ai_copilot::provider::AiResponse, crate::ai_copilot::error::AiError>,
    >;
}

/// Register the hosted AI service (first registration wins).
pub fn register_hosted_ai(service: Arc<dyn HostedAi>) {
    let _ = HOSTED_AI.set(service);
}

/// The registered hosted AI service, if an extension provides one.
pub fn hosted_ai() -> Option<&'static Arc<dyn HostedAi>> {
    HOSTED_AI.get()
}

/// Hooks for add-on editions. Every method has a no-op default, so the
/// open core runs with [`Community`].
pub trait Extension: Send + Sync + 'static {
    /// Short edition name for logs.
    fn name(&self) -> &'static str {
        "community"
    }

    /// Resolve the licence once at startup. An add-on fills the per-site
    /// fields of `LicenseStatus::Valid` (`site`, `support_until`) from the
    /// signed activation blob; Core derives `support` (active, lapsed, none)
    /// from them and exposes them on `/api/license/status` and
    /// `/api/v1/system/features`. Support state is informational: it must
    /// never limit Core. The core default is
    /// [`LicenseStatus::Community`], which enables everything in the core.
    fn resolve_license<'a>(&'a self, _storage: &'a Storage) -> BoxFuture<'a, LicenseStatus> {
        Box::pin(async { LicenseStatus::Community })
    }

    /// Extra CLI help text appended to `--help`.
    fn cli_help(&self) -> &'static str {
        ""
    }

    /// Handle CLI subcommands owned by the extension. Return true when handled.
    fn handle_cli<'a>(&'a self, _args: &'a [String]) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    /// Called once after core state is built and before the server listens.
    /// Spawn background tasks and keep any state the routes need.
    fn start<'a>(&'a self, _ctx: StartContext) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }

    /// Routes to merge into the API router. Called after [`Extension::start`].
    fn routes(&self, _state: &AppState) -> ExtensionRoutes {
        ExtensionRoutes::default()
    }

    /// Validate a `Authorization: Bearer` token issued by an external provider.
    fn authenticate_bearer<'a>(
        &'a self,
        _token: &'a str,
    ) -> BoxFuture<'a, Option<ExternalIdentity>> {
        Box::pin(async { None })
    }

    /// Extra entries for `GET /api/v1/auth/providers`.
    fn auth_providers(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }
}

/// The open core on its own: no add-on, no licence, nothing gated.
pub struct Community;

impl Extension for Community {}
