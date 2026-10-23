use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::api::{AppError, AppState};
use crate::models::{CreateRecipeRequest, Recipe};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListRecipesQuery {
    pub device_id: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/recipes",
    params(ListRecipesQuery),
    responses(
        (status = 200, body = Vec<Recipe>),
        (status = 403),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_recipes(
    State(state): State<AppState>,
    Query(q): Query<ListRecipesQuery>,
) -> Result<Json<Vec<Recipe>>, AppError> {
    let recipes = state.storage.list_recipes(q.device_id.as_deref()).await?;
    Ok(Json(recipes))
}

#[utoipa::path(
    post,
    path = "/api/v1/recipes",
    request_body = CreateRecipeRequest,
    responses(
        (status = 201, body = Recipe),
        (status = 400),
        (status = 403),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_recipe(
    State(state): State<AppState>,
    Json(req): Json<CreateRecipeRequest>,
) -> Result<(StatusCode, Json<Recipe>), AppError> {
    let id = state
        .storage
        .save_current_as_recipe(
            &req.device_id,
            &req.name,
            req.category.as_deref(),
            req.description.as_deref(),
            &state.cache,
        )
        .await?;

    let recipe = state
        .storage
        .get_recipe(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Recipe not found after creation"))?;

    Ok((StatusCode::CREATED, Json(recipe)))
}

#[utoipa::path(
    get,
    path = "/api/v1/recipes/{id}",
    params(
        ("id" = i64, Path, description = "Recipe ID")
    ),
    responses(
        (status = 200, body = Recipe),
        (status = 403),
        (status = 404),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_recipe(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Recipe>, AppError> {
    let recipe = state
        .storage
        .get_recipe(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Recipe not found")))?;
    Ok(Json(recipe))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateRecipeRequest {
    pub name: Option<String>,
    pub category: Option<Option<String>>,
    pub description: Option<Option<String>>,
}

#[utoipa::path(
    put,
    path = "/api/v1/recipes/{id}",
    params(
        ("id" = i64, Path, description = "Recipe ID")
    ),
    request_body = UpdateRecipeRequest,
    responses(
        (status = 200, body = Recipe),
        (status = 400),
        (status = 403),
        (status = 404),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_recipe(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRecipeRequest>,
) -> Result<Json<Recipe>, AppError> {
    state
        .storage
        .update_recipe(
            id,
            req.name.as_deref(),
            req.category.as_ref().map(|c| c.as_deref()),
            None,
            req.description.as_ref().map(|d| d.as_deref()),
        )
        .await?;

    let recipe = state
        .storage
        .get_recipe(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Recipe not found after update")))?;
    Ok(Json(recipe))
}

#[utoipa::path(
    delete,
    path = "/api/v1/recipes/{id}",
    params(
        ("id" = i64, Path, description = "Recipe ID")
    ),
    responses(
        (status = 204),
        (status = 403),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_recipe(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    state.storage.delete_recipe(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/recipes/{id}/load",
    params(
        ("id" = i64, Path, description = "Recipe ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
        (status = 404),
    ),
    tag = "Recipes",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn load_recipe(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let recipe = state
        .storage
        .get_recipe(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Recipe not found")))?;

    let command_ids = crate::recipe::load_recipe_to_plc(&recipe, &state.storage).await?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "command_ids": command_ids,
        "message": format!("Recipe load queued: {} commands", command_ids.len()),
    })))
}
