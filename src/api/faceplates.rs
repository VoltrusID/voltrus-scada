//! Faceplate definitions (`/api/v1/faceplates`): read by every signed-in user,
//! written by the roles that configure screens. See docs/faceplates.md.

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};

use super::{AppError, AppState, AuthUser, Rejected};
use crate::faceplate::Faceplate;

fn parse(mut body: Value) -> Result<Faceplate, AppError> {
    // What `GET /faceplates/{id}` adds, so a read-modify-write passes.
    if let Some(obj) = body.as_object_mut() {
        obj.remove("used_by");
    }
    let f: Faceplate = serde_json::from_value(body)
        .map_err(|e| AppError::bad_request(format!("Not a faceplate: {e}")))?;
    let problems = f.problems();
    if !problems.is_empty() {
        return Err(AppError::bad_request(format!(
            "Invalid faceplate: {}",
            problems.join("; ")
        )));
    }
    Ok(f)
}

fn audit(state: &AppState, auth: &AuthUser, action: &str, id: &str) {
    state.audit_logger.log(crate::security::AuditEvent {
        action: action.to_string(),
        username: auth.username.clone(),
        resource: format!("faceplate:{id}"),
        details: None,
    });
}

/// GET /api/v1/faceplates
#[utoipa::path(
    get,
    path = "/api/v1/faceplates",
    responses((status = 200, body = Vec<Faceplate>)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_faceplates(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.storage.list_faceplates().await?))
}

/// GET /api/v1/faceplates/{id}
#[utoipa::path(
    get,
    path = "/api/v1/faceplates/{id}",
    params(("id" = String, Path, description = "Faceplate id")),
    responses((status = 200, body = Faceplate), (status = 404)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_faceplate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let f = state
        .storage
        .get_faceplate(&id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("No faceplate '{id}'")))?;
    let used_by: Vec<Value> = state
        .storage
        .faceplate_users(&id)
        .await?
        .into_iter()
        .map(|(screen, symbol)| json!({"screen": screen, "symbol": symbol}))
        .collect();
    let mut body = serde_json::to_value(f).unwrap_or(Value::Null);
    body["used_by"] = Value::Array(used_by);
    Ok(Json(body))
}

/// POST /api/v1/faceplates
#[utoipa::path(
    post,
    path = "/api/v1/faceplates",
    request_body = Faceplate,
    responses(
        (status = 201, body = Faceplate),
        (status = 400, description = "Unknown field or invalid value, named in the error"),
        (status = 409, description = "The id is taken"),
    ),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_faceplate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let f = parse(body)?;
    if !state
        .storage
        .save_faceplate_as(
            &f,
            true,
            &super::design_history::actor(&auth, api_key.is_some(), None, "faceplate"),
        )
        .await?
    {
        return Err(AppError(anyhow::Error::new(Rejected(
            StatusCode::CONFLICT,
            format!("A faceplate '{}' already exists", f.id),
        ))));
    }
    audit(&state, &auth, "create_faceplate", &f.id);
    Ok((StatusCode::CREATED, Json(f)))
}

/// PUT /api/v1/faceplates/{id}
#[utoipa::path(
    put,
    path = "/api/v1/faceplates/{id}",
    params(("id" = String, Path, description = "Faceplate id")),
    request_body = Faceplate,
    responses((status = 200, body = Faceplate), (status = 400), (status = 404)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_faceplate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let f = parse(body)?;
    if f.id != id {
        return Err(AppError::bad_request(format!(
            "id '{}' in the body differs from '{id}' in the path",
            f.id
        )));
    }
    if !state
        .storage
        .save_faceplate_as(
            &f,
            false,
            &super::design_history::actor(&auth, api_key.is_some(), None, "faceplate"),
        )
        .await?
    {
        return Err(AppError::not_found(format!("No faceplate '{id}'")));
    }
    audit(&state, &auth, "update_faceplate", &id);
    Ok(Json(f))
}

/// DELETE /api/v1/faceplates/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/faceplates/{id}",
    params(("id" = String, Path, description = "Faceplate id")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
        (status = 409, description = "SCADA symbols or equipment types still open it"),
    ),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_faceplate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let users = state.storage.faceplate_users(&id).await?;
    if !users.is_empty() {
        let list: Vec<String> = users.iter().map(|(s, w)| format!("{s}/{w}")).collect();
        return Err(AppError(anyhow::Error::new(Rejected(
            StatusCode::CONFLICT,
            format!(
                "Faceplate '{id}' is opened by {} symbol(s) ({}); remove it from them first",
                list.len(),
                list.join(", ")
            ),
        ))));
    }
    let types = state.storage.faceplate_type_users(&id).await?;
    if !types.is_empty() {
        return Err(AppError(anyhow::Error::new(Rejected(
            StatusCode::CONFLICT,
            format!(
                "Faceplate '{id}' is opened by equipment type(s) {}; change their faceplate first",
                types.join(", ")
            ),
        ))));
    }
    if !state
        .storage
        .delete_faceplate_as(
            &id,
            &super::design_history::actor(&auth, api_key.is_some(), None, "faceplate"),
        )
        .await?
    {
        return Err(AppError::not_found(format!("No faceplate '{id}'")));
    }
    audit(&state, &auth, "delete_faceplate", &id);
    Ok(Json(json!({"ok": true, "id": id})))
}
