//! Vertical templates: preview and instantiate are configuration actions and
//! refuse instance counts that would expand to more tags than the limits allow.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn app_with_viewer() -> (Router, String, String) {
    let state = seeded_admin_app_state().await;
    let hash = crate::api::auth::hash_password("password-123");
    state
        .storage
        .create_user("victor", &hash, "", "viewer")
        .await
        .unwrap();
    let app = create_router(state);
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let viewer = format!(
        "voltrus_token={}",
        login_cookie(&app, "victor", "password-123").await
    );
    (app, admin, viewer)
}

fn template_id() -> String {
    crate::vertical_templates::definitions::all_templates()
        .into_iter()
        .find(|t| t.tags.iter().any(|tag| tag.name_pattern.contains("{n}")))
        .unwrap()
        .id
}

#[tokio::test]
async fn preview_refuses_huge_counts_and_viewers() {
    let (app, admin, viewer) = app_with_viewer().await;
    let id = template_id();
    let huge = json!({"template_id": id, "instance_counts": {"n": u32::MAX}});
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/templates/preview",
        None,
        Some(&viewer),
        Some(huge.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewers may not preview");
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/templates/preview",
        None,
        Some(&admin),
        Some(huge),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/templates/preview",
        None,
        Some(&admin),
        Some(json!({"template_id": id, "instance_counts": {"n": 3}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["expanded_tags"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn instantiate_refuses_huge_counts() {
    let (app, admin, _) = app_with_viewer().await;
    let id = template_id();
    let (status, body) = send_request(
        &app,
        "POST",
        &format!("/api/v1/templates/{id}/instantiate"),
        None,
        Some(&admin),
        Some(json!({"template_id": id, "name": "Big", "instance_counts": {"n": 100_000}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
