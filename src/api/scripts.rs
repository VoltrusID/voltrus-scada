use crate::api::*;
use crate::models::*;
use crate::script_engine;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ListExecutionsQuery {
    pub limit: Option<i64>,
}

pub async fn list_scripts(state: State<AppState>) -> Result<Json<Vec<Script>>, StatusCode> {
    state
        .storage
        .list_scripts()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_script(
    state: State<AppState>,
    Json(req): Json<CreateScriptRequest>,
) -> Result<(StatusCode, Json<Script>), StatusCode> {
    let cpu_limit = req.cpu_limit_ms.unwrap_or(1000);
    let mem_limit = req.memory_limit_bytes.unwrap_or(1048576);
    let syntax_error = script_engine::validate_syntax(&req.source);
    let id = state
        .storage
        .create_script(
            &req.name,
            &req.source,
            syntax_error.as_deref(),
            cpu_limit,
            mem_limit,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let script = state
        .storage
        .get_script(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(script)))
}

pub async fn update_script(
    state: State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateScriptRequest>,
) -> Result<Json<Script>, StatusCode> {
    // New source: re-validate and store the result, clearing the old error
    // when it now parses. No source: leave the stored error alone.
    let syntax_error = req.source.as_deref().map(script_engine::validate_syntax);
    state
        .storage
        .update_script(
            id,
            req.name.as_deref(),
            req.source.as_deref(),
            syntax_error.as_ref().map(Option::as_deref),
            req.cpu_limit_ms,
            req.memory_limit_bytes,
            req.enabled,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .storage
        .get_script(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn delete_script(
    state: State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .storage
        .delete_script(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn test_script(
    state: State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<TestScriptRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let script = state
        .storage
        .get_script(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if let Some(ref err) = script.syntax_error {
        return Ok(Json(serde_json::json!({
            "success": false,
            "error": err,
        })));
    }

    let result = script_engine::execute_script(
        &script.source,
        &req.input,
        script.cpu_limit_ms as u64,
        script.memory_limit_bytes as u64,
    );

    let _ = state
        .storage
        .create_script_execution(
            id,
            None,
            Some("test"),
            result.duration_ms as i64,
            if result.success { "success" } else { "error" },
            result.error.as_deref(),
            result.output.as_deref(),
        )
        .await;

    Ok(Json(serde_json::json!({
        "success": result.success,
        "output": result.output,
        "error": result.error,
        "duration_ms": result.duration_ms,
    })))
}

pub async fn list_executions(
    state: State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<ListExecutionsQuery>,
) -> Result<Json<Vec<ScriptExecution>>, StatusCode> {
    let limit = query.limit.unwrap_or(50);
    state
        .storage
        .list_script_executions(id, limit)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
