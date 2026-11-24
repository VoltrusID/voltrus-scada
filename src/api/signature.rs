//! Electronic signatures for actions an add-on edition wants signed (21 CFR
//! Part 11 style). The edition decides which actions need one
//! ([`Extension::signature_required`]) and stores the record
//! ([`Extension::record_signature`]); the core asks, checks the signer's
//! password with the sign-in lockout rules, and links the record to the
//! action's audit entry. Plain Core never asks for a signature.
//!
//! Answers (see docs/edition-hooks.md):
//! * 428 `{code: "signature_required", signature_required: {action, meaning}}`
//!   when the request carries no signature (or one for another meaning);
//! * 403 `{code: "signature_invalid", ...}` for a wrong password;
//! * 403 `{code: "account_locked"}` once wrong passwords locked the account;
//! * 403 `{code: "signature_not_possible"}` for principals that cannot sign
//!   (API keys, MCP, flows, accounts without a local password).
//!
//! [`Extension::signature_required`]: crate::edition::Extension::signature_required
//! [`Extension::record_signature`]: crate::edition::Extension::record_signature

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppState, AuthUser};
use crate::edition::{ActionContext, ActionOrigin, SignatureRecord, SignatureRequirement};
use crate::models::SignatureInput;

/// The optional body of an action route that had none: nothing (or only
/// whitespace, whatever the content type) or `{"signature": {...}}`.
pub struct MaybeSigned(pub Option<SignatureInput>);

impl<S: Send + Sync> axum::extract::FromRequest<S> for MaybeSigned {
    type Rejection = axum::response::Response;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self(None));
        }
        let body: crate::models::SignedBody = serde_json::from_slice(&bytes).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid JSON body: {e}")})),
            )
                .into_response()
        })?;
        Ok(Self(body.signature))
    }
}

/// Wrong passwords typed inside a session (signatures, changing the
/// password) before they lock and the user's sessions end (counted apart
/// from sign-in, see storage/login_failures.rs).
pub const MAX_FAILED_ATTEMPTS: i64 = crate::storage::PAIR_MAX_FAILURES;
/// How long signing stays locked.
pub const LOCKOUT_SECS: i64 = crate::storage::PAIR_LOCK_SECS;

/// Why an action that needs a signature was not signed.
#[derive(Debug)]
pub enum SignatureError {
    /// No signature, or one given for another meaning (428).
    Required {
        action: &'static str,
        requirement: SignatureRequirement,
    },
    /// Wrong password (403, the client prompts again).
    Invalid {
        action: &'static str,
        requirement: SignatureRequirement,
    },
    /// The account is locked after too many wrong passwords (403).
    Locked,
    /// This principal cannot sign (403).
    NotPossible(String),
    /// The signature is incomplete, e.g. no reason (400).
    BadRequest(String),
    /// The edition could not record the signature (500).
    Internal(anyhow::Error),
}

impl SignatureError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Required { .. } => StatusCode::PRECONDITION_REQUIRED,
            Self::Invalid { .. } | Self::Locked | Self::NotPossible(_) => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Required { .. } => "signature_required",
            Self::Invalid { .. } => "signature_invalid",
            Self::Locked => "account_locked",
            Self::NotPossible(_) => "signature_not_possible",
            Self::BadRequest(_) => "signature_incomplete",
            Self::Internal(_) => "signature_not_recorded",
        }
    }

    /// The text shown to the caller. Internal errors are logged, not exposed.
    pub fn message(&self) -> String {
        match self {
            Self::Required { .. } => "This action needs an electronic signature".to_string(),
            Self::Invalid { .. } => "The signature password is incorrect".to_string(),
            Self::Locked => {
                "Account locked after too many wrong passwords. Try again later.".to_string()
            }
            Self::NotPossible(m) | Self::BadRequest(m) => m.clone(),
            Self::Internal(e) => {
                tracing::error!("Recording a signature failed: {e:#}");
                "The signature could not be recorded; the action was not carried out".to_string()
            }
        }
    }

    /// The signature the client should prompt for, when it should prompt.
    pub fn prompt(&self) -> Option<(&'static str, &SignatureRequirement)> {
        match self {
            Self::Required {
                action,
                requirement,
            }
            | Self::Invalid {
                action,
                requirement,
            } => Some((action, requirement)),
            _ => None,
        }
    }

    /// The JSON body of the answer.
    pub fn body(&self) -> serde_json::Value {
        let mut body = json!({"error": self.message(), "code": self.code()});
        if let Some((action, requirement)) = self.prompt() {
            body["signature_required"] = json!({
                "action": action,
                "meaning": requirement.meaning,
            });
        }
        body
    }
}

impl IntoResponse for SignatureError {
    fn into_response(self) -> axum::response::Response {
        (self.status(), Json(self.body())).into_response()
    }
}

/// A signature the core verified and the edition recorded.
#[derive(Debug, Clone)]
pub struct Signed {
    /// The edition's id for the record (`None` when it keeps none).
    pub id: Option<String>,
    pub meaning: String,
    pub reason: String,
}

impl Signed {
    /// ` signature=<id> meaning="..."`, appended to the action's audit details.
    pub fn audit_suffix(&self) -> String {
        let id: String = self
            .id
            .as_deref()
            .unwrap_or("-")
            .chars()
            .map(|c| if c.is_whitespace() { '_' } else { c })
            .collect();
        format!(" signature={id} meaning={:?}", self.meaning)
    }
}

/// The origin of a request made by `user`: an API key or a person.
pub fn origin_of(user: &AuthUser, api_key: bool) -> ActionOrigin {
    if api_key || user.username.starts_with("api_key:") {
        ActionOrigin::ApiKey
    } else {
        ActionOrigin::Session
    }
}

/// [`authorize`] for an action of `user` on `resource`.
pub async fn authorize_action(
    state: &AppState,
    user: &AuthUser,
    origin: ActionOrigin,
    action: crate::edition::SignedAction,
    resource: String,
    details: serde_json::Value,
    signature: Option<&SignatureInput>,
) -> Result<Option<Signed>, SignatureError> {
    let ctx = ActionContext {
        action,
        origin,
        username: user.username.clone(),
        role: user.role.clone(),
        resource,
        details,
    };
    authorize(state, &ctx, signature).await
}

/// Log a signed action in the audit log (unsigned ones keep their own
/// entries, or none, as before).
pub fn audit_signed(
    state: &AppState,
    user: &AuthUser,
    action: &str,
    resource: String,
    signed: &Option<Signed>,
) {
    if let Some(signed) = signed {
        state.audit_logger.log(crate::security::AuditEvent {
            action: action.to_string(),
            username: user.username.clone(),
            resource,
            details: Some(format!(
                "{} reason={}",
                signed.audit_suffix().trim_start(),
                signed.reason
            )),
        });
    }
}

/// Ask the edition whether `ctx` needs a signature and, if so, check the one
/// given. `Ok(None)`: no signature needed. `Ok(Some(_))`: signed and recorded;
/// the caller goes ahead and appends [`Signed::audit_suffix`] to its audit entry.
pub async fn authorize(
    state: &AppState,
    ctx: &ActionContext,
    signature: Option<&SignatureInput>,
) -> Result<Option<Signed>, SignatureError> {
    let requirement = state.ext.signature_required(ctx).await;
    authorize_requirement(state, ctx, requirement, signature).await
}

/// [`authorize`] with the requirement already known (a recipe load also asks
/// about each of its tag writes).
pub async fn authorize_requirement(
    state: &AppState,
    ctx: &ActionContext,
    requirement: Option<SignatureRequirement>,
    signature: Option<&SignatureInput>,
) -> Result<Option<Signed>, SignatureError> {
    let Some(requirement) = requirement else {
        return Ok(None);
    };
    let action = ctx.action.as_str();
    if !ctx.origin.can_sign() {
        let who = match ctx.origin {
            ActionOrigin::ApiKey => "an API key",
            ActionOrigin::Mcp => "an MCP client",
            ActionOrigin::Flow => "a flow",
            _ => "this client",
        };
        return Err(SignatureError::NotPossible(format!(
            "This action needs an electronic signature ({}), which {who} cannot give. Sign in as a person to do it.",
            requirement.meaning
        )));
    }
    let Some(signature) = signature.filter(|s| s.meaning.trim() == requirement.meaning.trim())
    else {
        return Err(SignatureError::Required {
            action,
            requirement,
        });
    };
    let reason = signature.reason.trim();
    if reason.is_empty() {
        return Err(SignatureError::BadRequest(
            "A signature needs a reason".to_string(),
        ));
    }
    verify_signer(
        state,
        &ctx.username,
        &signature.password,
        action,
        &requirement,
    )
    .await?;

    let record = SignatureRecord {
        username: ctx.username.clone(),
        role: ctx.role.clone(),
        action: ctx.action,
        origin: ctx.origin,
        resource: ctx.resource.clone(),
        details: ctx.details.clone(),
        meaning: requirement.meaning.clone(),
        reason: reason.to_string(),
        signed_at: crate::timeutil::now_secs(),
    };
    let id = state
        .ext
        .record_signature(&record)
        .await
        .map_err(SignatureError::Internal)?;
    let signed = Signed {
        id,
        meaning: requirement.meaning,
        reason: reason.to_string(),
    };
    state.audit_logger.log(crate::security::AuditEvent {
        action: "signature".to_string(),
        username: ctx.username.clone(),
        resource: ctx.resource.clone(),
        details: Some(format!(
            "action={action}{} reason={}",
            signed.audit_suffix(),
            signed.reason
        )),
    });
    Ok(Some(signed))
}

/// Check the signer's own password ([`check_session_password`]).
async fn verify_signer(
    state: &AppState,
    username: &str,
    password: &str,
    action: &'static str,
    requirement: &SignatureRequirement,
) -> Result<(), SignatureError> {
    let user = state
        .storage
        .get_user_by_username(username)
        .await
        .map_err(SignatureError::Internal)?
        .filter(|u| !u.password_hash.is_empty())
        .ok_or_else(|| {
            SignatureError::NotPossible(
                "This account has no local password to sign with".to_string(),
            )
        })?;
    if user.disabled {
        return Err(SignatureError::NotPossible(
            "This account is disabled".to_string(),
        ));
    }
    let shared = state.public_demo.is_shared(&user.username, &user.role);
    let (attempts, locked) =
        match check_session_password(state, &user.username, password, &user.password_hash, shared)
            .await
            .map_err(SignatureError::Internal)?
        {
            SessionPassword::Right => return Ok(()),
            SessionPassword::Locked => return Err(SignatureError::Locked),
            SessionPassword::Wrong { attempts, locked } => (attempts, locked),
        };
    let _ = state
        .storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "signature_failed".into(),
            severity: "warning".into(),
            source_ip: None,
            user_agent: None,
            username: Some(username.to_string()),
            details: Some(format!(
                "Wrong signature password for {action} (attempt {attempts})"
            )),
        })
        .await;
    if locked {
        return Err(SignatureError::Locked);
    }
    Err(SignatureError::Invalid {
        action,
        requirement: requirement.clone(),
    })
}

/// What [`check_session_password`] found.
pub(crate) enum SessionPassword {
    Right,
    /// Wrong; `attempts` in the window, `locked` when this one locked it.
    Wrong {
        attempts: i64,
        locked: bool,
    },
    /// Locked after too many wrong passwords; the password was not checked.
    Locked,
}

/// Check a password typed inside a signed-in session (an electronic
/// signature, the current password when changing it). Wrong ones are counted
/// for the account under [`crate::storage::SESSION_SOURCE`] (and towards its
/// account-wide count) rather than an address; a shared demo account counts
/// none. The lock ends all the user's sessions, so a stolen session cannot
/// keep guessing; signing in again stays possible.
pub(crate) async fn check_session_password(
    state: &AppState,
    username: &str,
    password: &str,
    password_hash: &str,
    shared: bool,
) -> anyhow::Result<SessionPassword> {
    let now = crate::timeutil::now_secs();
    let attempt = if shared {
        None
    } else {
        match state
            .storage
            .begin_login_attempt(Some(username), crate::storage::SESSION_SOURCE, false, now)
            .await?
        {
            Ok(attempt) => Some(attempt),
            Err(_) => return Ok(SessionPassword::Locked),
        }
    };
    if super::auth::verify_password(password, password_hash) {
        if let Some(attempt) = attempt {
            state.storage.succeed_login_attempt(attempt).await?;
        }
        return Ok(SessionPassword::Right);
    }
    let Some(attempt) = attempt else {
        return Ok(SessionPassword::Wrong {
            attempts: 1,
            locked: false,
        });
    };
    let outcome = state.storage.fail_login_attempt(attempt).await?;
    if outcome.pair_locked {
        let _ = state.storage.revoke_all_user_refresh_tokens(username).await;
        tracing::warn!("Password checks for {username} locked after too many wrong passwords");
    }
    Ok(SessionPassword::Wrong {
        attempts: outcome.pair_failures,
        locked: outcome.pair_locked,
    })
}
