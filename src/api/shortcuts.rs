use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

/// GET /api/v1/shortcuts — get shortcuts for the current user
#[utoipa::path(
    get,
    path = "/api/v1/shortcuts",
    responses((status = 200, description = "User keyboard shortcuts map")),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_shortcuts(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_username(&auth.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;
    let shortcuts = state.storage.get_user_shortcuts(user.id).await?;
    // Return as map: {data_source_id: shortcut}
    let mut map = serde_json::Map::new();
    for s in &shortcuts {
        map.insert(s.data_source_id.to_string(), json!(s.shortcut));
    }
    Ok(Json(serde_json::Value::Object(map)))
}

/// PUT /api/v1/shortcuts — save shortcuts, accept map of {data_source_id: shortcut}
/// If value is null or empty string, delete the entry
#[utoipa::path(
    put,
    path = "/api/v1/shortcuts",
    responses((status = 200, description = "Shortcuts saved")),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_shortcuts(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_username(&auth.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;
    let obj = body
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected a JSON object"))?;

    // Collect as owned strings first
    let mut owned_entries: Vec<(i64, Option<String>)> = Vec::new();
    for (data_source_id_str, value) in obj {
        let data_source_id: i64 = data_source_id_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid data_source_id: {}", data_source_id_str))?;
        match value {
            serde_json::Value::Null => {
                owned_entries.push((data_source_id, None));
            }
            serde_json::Value::String(s) if s.is_empty() => {
                owned_entries.push((data_source_id, None));
            }
            serde_json::Value::String(s) => {
                owned_entries.push((data_source_id, Some(s.clone())));
            }
            _ => {
                owned_entries.push((data_source_id, Some(value.to_string())));
            }
        }
    }

    // Convert to references for the storage method
    let entries: Vec<(i64, Option<&str>)> = owned_entries
        .iter()
        .map(|(id, opt)| (*id, opt.as_deref()))
        .collect();

    state.storage.save_user_shortcuts(user.id, &entries).await?;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/v1/nav-shortcuts — get fully resolved navigation shortcuts
/// Returns defaults merged with user overrides.
#[utoipa::path(
    get,
    path = "/api/v1/nav-shortcuts",
    responses((status = 200, description = "Navigation shortcuts map")),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_nav_shortcuts(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_username(&auth.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;

    // Build defaults from screens
    let screens = state.storage.list_devices().await?;
    let mut map = serde_json::Map::new();
    map.insert("screen:all".to_string(), json!("Mod+1"));
    map.insert("screen:system".to_string(), json!("Mod+0"));
    map.insert("view:cards".to_string(), json!(""));
    map.insert("view:canvas".to_string(), json!(""));
    map.insert("view:twin".to_string(), json!(""));
    map.insert("view:scada".to_string(), json!(""));
    map.insert("mode:live".to_string(), json!(""));
    map.insert("mode:history".to_string(), json!(""));
    map.insert("action:alarms".to_string(), json!(""));
    for (i, s) in screens.iter().enumerate() {
        map.insert(format!("screen:{}", s.id), json!(format!("Mod+{}", i + 2)));
    }

    // Apply user overrides
    let user_shortcuts = state.storage.get_user_nav_shortcuts(user.id).await?;
    for s in &user_shortcuts {
        if s.shortcut.is_empty() {
            map.remove(&s.target);
        } else {
            map.insert(s.target.clone(), json!(s.shortcut));
        }
    }

    Ok(Json(serde_json::Value::Object(map)))
}

/// PUT /api/v1/nav-shortcuts — save navigation shortcuts, accept map of {target: shortcut}
#[utoipa::path(
    put,
    path = "/api/v1/nav-shortcuts",
    responses((status = 200, description = "Navigation shortcuts saved")),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_nav_shortcuts(
    State(state): State<AppState>,
    axum::Extension(auth): axum::Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_username(&auth.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;
    let obj = body
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected a JSON object"))?;

    let mut owned_entries: Vec<(String, Option<String>)> = Vec::new();
    for (target, value) in obj {
        match value {
            serde_json::Value::Null => {
                owned_entries.push((target.clone(), None));
            }
            serde_json::Value::String(s) if s.is_empty() => {
                owned_entries.push((target.clone(), None));
            }
            serde_json::Value::String(s) => {
                owned_entries.push((target.clone(), Some(s.clone())));
            }
            _ => {
                owned_entries.push((target.clone(), Some(value.to_string())));
            }
        }
    }

    let entries: Vec<(String, Option<&str>)> = owned_entries
        .iter()
        .map(|(t, opt)| (t.clone(), opt.as_deref()))
        .collect();

    state
        .storage
        .save_user_nav_shortcuts(user.id, &entries)
        .await?;
    Ok(Json(json!({"ok": true})))
}
