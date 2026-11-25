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
use crate::models::AppConfig;
use crate::notification::email::EmailNotifier;
use crate::poller_manager::PollerManager;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Routes contributed by an extension, merged into the matching core group.
///
/// * `public`: no authentication (like `/api/v1/health`).
/// * `protected`: any authenticated user.
/// * `operate`: running the plant: the `operator` role plus admin, demo and
///   integration (tag writes, alarm handling, recipe loads).
/// * `operator`: configuration: admin, demo and integration roles (device and
///   screen configuration). The name predates the `operate` group.
/// * `admin`: admin users only.
///
/// Extension routes sit behind the same CORS, rate limit and security header
/// layers as core routes.
pub struct ExtensionRoutes {
    pub public: Router<AppState>,
    pub protected: Router<AppState>,
    pub operate: Router<AppState>,
    pub operator: Router<AppState>,
    pub admin: Router<AppState>,
}

impl Default for ExtensionRoutes {
    fn default() -> Self {
        Self {
            public: Router::new(),
            protected: Router::new(),
            operate: Router::new(),
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

/// One alarm as a chat channel (Telegram, WhatsApp) delivers it.
#[derive(Debug, Clone)]
pub struct ChatAlarm {
    pub severity: String,
    pub source_key: String,
    pub source_name: String,
    /// `value` rounded to the tag's decimals, as in `value_text`.
    pub value: f64,
    /// `threshold` rounded as in `threshold_text`.
    pub threshold: f64,
    pub unit: String,
    /// The value as the alarm message shows it, unit included (`2.60 m`).
    /// Print this rather than `value` and `unit`, so a chat message reads
    /// the same as the alarm list and the email.
    pub value_text: String,
    /// The limit as the alarm message shows it, unit included.
    pub threshold_text: String,
    pub message: String,
    /// RFC 3339 trigger time.
    pub triggered_at: String,
}

/// What happened to one recipient of a chat notification.
#[derive(Debug, Clone)]
pub struct ChatDelivery {
    /// Chat id (Telegram) or phone number (WhatsApp).
    pub target: String,
    /// `None` when the message was accepted by the provider.
    pub error: Option<String>,
}

/// Delivers alarms over chat channels. Registered by an extension that
/// provides them. `config` is the stored channel config JSON, already
/// validated by [`crate::notification::chat`].
pub trait ChatSender: Send + Sync {
    /// Send `alarm` to every recipient of the channel (`telegram` or
    /// `whatsapp`). An `Err` means the config could not be used at all;
    /// per-recipient failures come back in the deliveries.
    fn send<'a>(
        &'a self,
        channel: &'a str,
        config: &'a str,
        alarm: &'a ChatAlarm,
    ) -> BoxFuture<'a, anyhow::Result<Vec<ChatDelivery>>>;
}

static CHAT_SENDER: OnceLock<Arc<dyn ChatSender>> = OnceLock::new();
static HOSTED_AI: OnceLock<Arc<dyn HostedAi>> = OnceLock::new();

/// Register the chat sender (first registration wins).
pub fn register_chat_sender(sender: Arc<dyn ChatSender>) {
    let _ = CHAT_SENDER.set(sender);
}

/// The registered chat sender, if an extension provides one.
pub fn chat_sender() -> Option<&'static Arc<dyn ChatSender>> {
    CHAT_SENDER.get()
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

// ── Compliance hooks (21 CFR Part 11 style) ──────────────────────────────
//
// Plain Core never asks for any of this: every default keeps today's
// behaviour. An add-on edition implements the `Extension` methods that use
// these types. The full contract is in docs/edition-hooks.md.

/// Where a password is being set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordChangeKind {
    /// The signed-in user changes their own password
    /// (`POST /api/v1/auth/change-password`).
    SelfService,
    /// An admin creates a user (`POST /api/v1/users`).
    AdminCreate,
    /// An admin sets another user's password (`PUT /api/v1/users/{id}`).
    AdminReset,
}

/// A new password to check against the edition's password policy.
#[derive(Clone)]
pub struct PasswordCheck {
    pub username: String,
    /// The role the account has (or will have, for a new user).
    pub role: String,
    /// The new password in clear text. Never log or store it.
    pub new_password: String,
    pub kind: PasswordChangeKind,
}

impl std::fmt::Debug for PasswordCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordCheck")
            .field("username", &self.username)
            .field("role", &self.role)
            .field("new_password", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

/// How long a signed-in session may last. `None` means no limit beyond the
/// core's own (a 15-minute access token renewed by a 7-day sliding refresh
/// token).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct SessionPolicy {
    /// Sign the user out after this many seconds without user activity
    /// (`POST /api/v1/auth/activity`, sent by the client on keyboard, mouse
    /// and touch input).
    pub idle_timeout_secs: Option<u64>,
    /// Sign the user out this many seconds after they signed in, active or not.
    pub absolute_timeout_secs: Option<u64>,
}

/// An action that may need an electronic signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignedAction {
    /// A tag write / setpoint change (REST, WebSocket, MCP, flow, recipe).
    TagWrite,
    /// Acknowledging one alarm or all of them.
    AlarmAck,
    /// Shelving or unshelving an alarm.
    AlarmShelve,
    /// Loading a recipe (one signature covers every write of the load).
    RecipeLoad,
    /// Switching how the plant is run: a device's simulation mode, go-live
    /// and its rollback, starting or stopping a flow, changing a tag's
    /// write policy.
    ModeSwitch,
    /// Publishing a Designer screen (what operators see changes), requesting
    /// a publish, or approving one: `details.stage` is `publish`, `request` or
    /// `approve` (docs/version-history.md). 21 CFR Part 11 editions sign the
    /// request as the author and the approval as the reviewer.
    DesignPublish,
}

impl SignedAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TagWrite => "tag_write",
            Self::AlarmAck => "alarm_ack",
            Self::AlarmShelve => "alarm_shelve",
            Self::RecipeLoad => "recipe_load",
            Self::ModeSwitch => "mode_switch",
            Self::DesignPublish => "design_publish",
        }
    }
}

/// Who or what started an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOrigin {
    /// A person signed in with a session (REST or WebSocket, web or native
    /// client, or an add-on bearer identity).
    Session,
    /// An API key (REST or WebSocket). Keys cannot sign.
    ApiKey,
    /// An MCP tool call (an AI agent acting for a user or a key). Cannot sign.
    Mcp,
    /// A flow's write node. Nobody is there to sign.
    Flow,
    /// A tag write queued by a recipe load. Asked once per tag before the
    /// load starts, so one signature on the load covers it.
    Recipe,
}

impl ActionOrigin {
    /// The name stored with a queued command (`commands.origin`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::ApiKey => "api_key",
            Self::Mcp => "mcp",
            Self::Flow => "flow",
            Self::Recipe => "recipe",
        }
    }

    /// Whether a person is there who can type a password.
    pub fn can_sign(self) -> bool {
        matches!(self, Self::Session | Self::Recipe)
    }
}

/// The action being attempted, given to [`Extension::signature_required`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionContext {
    pub action: SignedAction,
    pub origin: ActionOrigin,
    /// The acting user (`api_key:<id>` for a key, `flow:<id>` for a flow).
    pub username: String,
    pub role: String,
    /// What is acted on: `data_source:<id>`, `alarm:<id>` (`alarm:*` for
    /// acknowledge-all), `recipe:<id>`, `device:<id>:simulation`,
    /// `device:<id>:go_live`, `device:<id>:go_live_rollback`, `flow:<id>`,
    /// `data_source:<id>:write_policy`, `screen:<id>:publish`.
    pub resource: String,
    /// Details per action, listed in docs/edition-hooks.md (a tag write has
    /// `tag_id` (the same id as in `resource`), `key`, `device_id`, `value`
    /// and `previous`, or `recipe_id` instead of `previous` when a recipe load
    /// asks).
    pub details: serde_json::Value,
}

/// A signature the edition asks for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignatureRequirement {
    /// What signing means, shown to the signer and recorded with the
    /// signature ("Approved setpoint change", "Reviewed and acknowledged").
    pub meaning: String,
}

/// A verified signature, given to [`Extension::record_signature`]. The core has
/// checked the signer's password; the edition stores the record.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SignatureRecord {
    pub username: String,
    pub role: String,
    pub action: SignedAction,
    pub origin: ActionOrigin,
    pub resource: String,
    pub details: serde_json::Value,
    pub meaning: String,
    pub reason: String,
    /// Unix seconds.
    pub signed_at: i64,
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

    /// Decide, before any device poller starts, whether this process starts as
    /// the active node (the default) or as the passive node of a redundant
    /// pair. Called once, right after the licence is resolved. An error stops
    /// the process rather than risk two active nodes. See [`crate::node_role`].
    fn resolve_node_role<'a>(
        &'a self,
        _config: &'a AppConfig,
        _storage: &'a Storage,
        _license: &'a LicenseStatus,
    ) -> BoxFuture<'a, anyhow::Result<crate::node_role::Start>> {
        Box::pin(async { Ok(crate::node_role::Start::default()) })
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

    /// Check a new password against the edition's policy (length, character
    /// classes, history, ...), after the core's own 8-character minimum. Asked
    /// on self-service change, admin create and admin reset. `Err` carries a
    /// message shown to the user (answered 400).
    fn validate_new_password<'a>(
        &'a self,
        _check: &'a PasswordCheck,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    /// Called after a password was stored, with its Argon2 hash (for password
    /// history and the change date that expiry counts from).
    fn password_changed<'a>(
        &'a self,
        _username: &'a str,
        _password_hash: &'a str,
        _kind: PasswordChangeKind,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Whether the user's password has expired. Asked at sign-in once the
    /// password is verified; `true` answers the sign-in like
    /// `must_change_password` (plus `password_expired: true`) and sets
    /// `must_change_password` on the account, so the user's sessions may only
    /// change the password until they have (403 `password_change_required`
    /// otherwise). Also asked by `/auth/me` while that flag is set.
    fn password_expired<'a>(&'a self, _user: &'a crate::models::User) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    /// Idle and absolute session limits for this user, read at sign-in and on
    /// every token refresh. The default sets none (core behaviour).
    fn session_policy<'a>(
        &'a self,
        _username: &'a str,
        _role: &'a str,
    ) -> BoxFuture<'a, SessionPolicy> {
        Box::pin(async { SessionPolicy::default() })
    }

    /// Whether `ctx` needs an electronic signature. `None` (the default) lets
    /// the action go ahead as in Core. For origins that cannot sign (API keys,
    /// MCP, flows) `Some` refuses the action.
    fn signature_required<'a>(
        &'a self,
        _ctx: &'a ActionContext,
    ) -> BoxFuture<'a, Option<SignatureRequirement>> {
        Box::pin(async { None })
    }

    /// Store a verified signature. Returns the signature's id, written into
    /// the action's audit entry (`signature=<id>`). An `Err` refuses the action
    /// (answered 500): an action that needs a signature never runs unrecorded.
    fn record_signature<'a>(
        &'a self,
        _record: &'a SignatureRecord,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        Box::pin(async { Ok(None) })
    }
}

static INSTALLED: OnceLock<Arc<dyn Extension>> = OnceLock::new();

/// Make `ext` reachable from code that runs outside a request (flow write
/// nodes). Called once by [`crate::run`].
pub fn install(ext: Arc<dyn Extension>) {
    let _ = INSTALLED.set(ext);
}

/// The edition the process runs with ([`Community`] before [`install`]).
pub fn installed() -> Arc<dyn Extension> {
    INSTALLED
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(Community))
}

/// The open core on its own: no add-on, no licence, nothing gated.
pub struct Community;

impl Extension for Community {}
