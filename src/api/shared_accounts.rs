//! Shared sign-ins for a public demo (`shared_accounts` and `login_hint` in the
//! config file).
//!
//! A shared account is one login handed to many people, e.g. `guest` on
//! demo.voltrus.id. Nobody signing in with it may change the account for the
//! others: its password and per-user preferences are read-only, and wrong
//! passwords never lock it (the per-IP login limit still applies). An admin
//! account is never treated as shared.

use axum::Json;
use axum::extract::{Extension, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{AppState, AuthUser};

/// Refusal text for anything a shared account may not change.
pub const SHARED_ACCOUNT_REFUSAL: &str =
    "This is a shared demo account; its password can't be changed";

/// Refusal text for the per-user preference routes.
const SHARED_PREFERENCES_REFUSAL: &str =
    "This is a shared demo account; its settings can't be changed";

/// Public-demo settings from the config file.
#[derive(Debug, Clone, Default)]
pub struct PublicDemo {
    /// Usernames that many people sign in with.
    pub shared_accounts: Vec<String>,
    /// Line shown under the sign-in form (plain text).
    pub login_hint: Option<String>,
}

impl PublicDemo {
    pub fn from_config(config: &crate::models::AppConfig) -> Self {
        Self {
            shared_accounts: config.shared_accounts.clone(),
            login_hint: config.login_hint.clone(),
        }
    }

    /// Whether `username` is listed in `shared_accounts`, whatever its role.
    pub fn is_listed(&self, username: &str) -> bool {
        self.shared_accounts.iter().any(|u| u == username)
    }

    /// Whether the account is treated as shared: listed and not an admin.
    /// Checked at request time, so it follows accounts created or re-roled
    /// after startup.
    pub fn is_shared(&self, username: &str, role: &str) -> bool {
        role != "admin" && self.is_listed(username)
    }
}

/// Refuses to start when a listed account is an admin: an admin login shared
/// with the public would hand out the whole server.
pub async fn check_no_shared_admins(
    storage: &crate::storage::Storage,
    demo: &PublicDemo,
) -> anyhow::Result<()> {
    for name in &demo.shared_accounts {
        if let Some(user) = storage.get_user_by_username(name).await?
            && user.role == "admin"
        {
            anyhow::bail!(
                "shared_accounts lists '{name}', which is an admin account. Give it the viewer role or remove it from shared_accounts."
            );
        }
    }
    Ok(())
}

/// Route layer for self-service routes that change the signed-in account
/// (its UI preferences and shortcuts): refused with 403 for a shared account.
pub async fn deny_shared_accounts_middleware(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    req: Request,
    next: Next,
) -> Response {
    if state.public_demo.is_shared(&auth.username, &auth.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": SHARED_PREFERENCES_REFUSAL})),
        )
            .into_response();
    }
    next.run(req).await
}
