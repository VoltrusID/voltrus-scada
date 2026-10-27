use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

async fn login_status(app: &Router, username: &str, password: &str) -> StatusCode {
    let body = serde_json::json!({"username": username, "password": password});
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn create_user(app: &Router, admin: &str, name: &str, role: &str) -> i64 {
    let (status, created) = send_request(
        app,
        "POST",
        "/api/v1/users",
        None,
        Some(admin),
        Some(serde_json::json!({"username": name, "password": "pw", "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    created["id"].as_i64().unwrap()
}

async fn user_row(app: &Router, admin: &str, id: i64) -> serde_json::Value {
    let (_, users) = send_request(app, "GET", "/api/v1/users", None, Some(admin), None).await;
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .cloned()
        .expect("user listed")
}

/// SCADA-16 S17: there was no way to disable an account.
#[tokio::test]
async fn test_disabled_user_cannot_sign_in_or_use_a_session() {
    let app = test_app_with_seeded_admin().await;
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let id = create_user(&app, &admin, "ops", "viewer").await;
    let ops = format!("voltrus_token={}", login_cookie(&app, "ops", "pw").await);
    let (status, _) = send_request(&app, "GET", "/api/v1/auth/me", None, Some(&ops), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user_row(&app, &admin, id).await["disabled"], false);

    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{id}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"disabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user_row(&app, &admin, id).await["disabled"], true);

    // The signed-in session stops working at once; a new login is refused.
    let (_, me) = send_request(&app, "GET", "/api/v1/auth/me", None, Some(&ops), None).await;
    assert_eq!(me["authenticated"], false);
    let (status, _) = send_request(&app, "GET", "/api/v1/screens", None, Some(&ops), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(login_status(&app, "ops", "pw").await, StatusCode::FORBIDDEN);
    assert_eq!(
        login_status(&app, "ops", "wrong").await,
        StatusCode::UNAUTHORIZED,
        "a wrong password still looks like any failed login"
    );

    // Re-enabling restores access.
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{id}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"disabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(login_status(&app, "ops", "pw").await, StatusCode::OK);
}

/// SCADA-16 S17: demoting or disabling the last enabled admin was allowed.
#[tokio::test]
async fn test_last_enabled_admin_cannot_be_demoted_or_disabled() {
    let app = test_app_with_seeded_admin().await;
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let admin_id = user_row_by_name(&app, &admin, "admin").await;

    let put = |body: serde_json::Value| {
        let app = app.clone();
        let admin = admin.clone();
        async move {
            send_request(
                &app,
                "PUT",
                &format!("/api/v1/users/{admin_id}"),
                None,
                Some(&admin),
                Some(body),
            )
            .await
        }
    };
    let (status, body) = put(serde_json::json!({"role": "viewer"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("last"));
    let (status, _) = put(serde_json::json!({"disabled": true})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A second admin that is disabled does not count as an admin.
    let spare = create_user(&app, &admin, "spare", "admin").await;
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{spare}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"disabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = put(serde_json::json!({"role": "viewer"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // With an enabled second admin, the demotion is fine.
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{spare}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"disabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = put(serde_json::json!({"role": "viewer"})).await;
    assert_eq!(status, StatusCode::OK);
}

async fn user_row_by_name(app: &Router, admin: &str, name: &str) -> i64 {
    let (_, users) = send_request(app, "GET", "/api/v1/users", None, Some(admin), None).await;
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == name)
        .expect("user listed")["id"]
        .as_i64()
        .unwrap()
}

/// SCADA-16 S17: DELETE /users/{id} answered 500 once the user had written an
/// annotation or handover; the records are kept and the user goes away.
#[tokio::test]
async fn test_delete_user_with_annotations_and_handovers() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let id = create_user(&app, &admin, "ops", "demo").await;
    let ops = format!("voltrus_token={}", login_cookie(&app, "ops", "pw").await);

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/annotations",
        None,
        Some(&ops),
        Some(serde_json::json!({"tag_id": "t", "annotation_text": "note", "data_timestamp": 1})),
    )
    .await;
    assert!(status.is_success(), "{status}");
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/shift-handover",
        None,
        Some(&ops),
        Some(serde_json::json!({
            "shift_date": "2026-01-01", "shift_period": "morning", "what_happened": "x"
        })),
    )
    .await;
    assert!(status.is_success(), "{status}");

    let (status, body) = send_request(
        &app,
        "DELETE",
        &format!("/api/v1/users/{id}"),
        None,
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(storage.get_user_by_id(id).await.unwrap().is_none());
    assert_eq!(
        storage
            .list_annotations(None, None, None)
            .await
            .unwrap()
            .len(),
        1
    );
    let (_, users) = send_request(&app, "GET", "/api/v1/users", None, Some(&admin), None).await;
    assert_eq!(users.as_array().unwrap().len(), 1, "only admin is listed");
}

/// SCADA-16 S18: a user's assigned RBAC role could be set but not read back.
#[tokio::test]
async fn test_get_user_role_reads_back_the_assignment() {
    let app = test_app_with_seeded_admin().await;
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let id = create_user(&app, &admin, "ops", "operator").await;
    let uri = format!("/api/v1/users/{id}/role");

    let (status, none) = send_request(&app, "GET", &uri, None, Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        none["role_id"].is_null() && none["role"].is_null(),
        "{none}"
    );

    let (status, role) = send_request(
        &app,
        "POST",
        "/api/v1/roles",
        None,
        Some(&admin),
        Some(serde_json::json!({"name": "Watch", "permissions": "[\"alarms:read\"]"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let role_id = role["id"].as_i64().unwrap();
    let (status, _) = send_request(
        &app,
        "PUT",
        &uri,
        None,
        Some(&admin),
        Some(serde_json::json!({"role_id": role_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, got) = send_request(&app, "GET", &uri, None, Some(&admin), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["user_id"], id);
    assert_eq!(got["role_id"], role_id);
    assert_eq!(got["role"]["name"], "Watch");

    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/users/9999/role",
        None,
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// SCADA-16 S18 design note, pinned: a custom RBAC role is an assignable
/// catalogue entry. It does not widen or narrow what the user's built-in role
/// reaches, in either direction.
#[tokio::test]
async fn test_assigned_custom_role_does_not_change_route_access() {
    let app = test_app_with_seeded_admin().await;
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let viewer = create_user(&app, &admin, "viewer1", "viewer").await;
    let viewer_cookie = format!(
        "voltrus_token={}",
        login_cookie(&app, "viewer1", "pw").await
    );
    let (_, role) = send_request(
        &app,
        "POST",
        "/api/v1/roles",
        None,
        Some(&admin),
        Some(serde_json::json!({"name": "Everything", "permissions": "[\"users:manage\"]"})),
    )
    .await;
    send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{viewer}/role"),
        None,
        Some(&admin),
        Some(serde_json::json!({"role_id": role["id"]})),
    )
    .await;
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/users",
        None,
        Some(&viewer_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// SCADA-16 S19 design note, pinned: an API key's `permissions` text is
/// stored as given; its `role` alone decides access.
#[tokio::test]
async fn test_api_key_role_decides_access_not_permissions_text() {
    let app = test_app_with_seeded_admin().await;
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let make = |role: &'static str, permissions: &'static str| {
        let app = app.clone();
        let admin = admin.clone();
        async move {
            let (status, key) = send_request(
                &app,
                "POST",
                "/api/v1/api-keys",
                None,
                Some(&admin),
                Some(serde_json::json!({"name": role, "role": role, "permissions": permissions})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{key}");
            assert!(
                key["note"].as_str().unwrap().contains("not enforced"),
                "the response says permissions are not enforced: {key}"
            );
            key["key"].as_str().unwrap().to_string()
        }
    };
    // Permissions that name everything do not lift a read-only key ...
    let read_only = make("api", "[\"commands:write\",\"screens:write\"]").await;
    let (status, _) =
        send_request(&app, "GET", "/api/v1/screens", Some(&read_only), None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/settings/alarm-cooldown",
        Some(&read_only),
        None,
        Some(serde_json::json!({"secs": 30})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // ... and an empty list does not restrict an integration key.
    let integration = make("integration", "[]").await;
    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/settings/alarm-cooldown",
        Some(&integration),
        None,
        Some(serde_json::json!({"secs": 30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
