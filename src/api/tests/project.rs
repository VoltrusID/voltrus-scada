use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

const PROJECT: &str = r#"
format: voltrus-project/1
modbus_devices:
- id: plant_a
  name: Plant A
  ip: 10.1.1.1
  registers:
  - {name: voltage, address: 40001}
tags:
- {key: voltage, device: plant_a, name: Voltage, min: 0, max: 450, base: 380, alarm: {warn: 400, crit: 420}}
screens:
- id: plant_a
  name: Plant A
  tags: [{ref: plant_a/voltage}]
  layouts:
    lg: [{tag: plant_a/voltage, x: 0, y: 0, w: 4, h: 3}]
hub_sites:
- {name: Remote, url: 'https://remote.example.com', api_key: hub-secret-key}
"#;

async fn raw(
    app: &Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: &str,
) -> (StatusCode, http::HeaderMap, String) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let resp = app
        .clone()
        .oneshot(
            req.header("content-type", "application/yaml")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn export_is_admin_only_yaml_without_secrets_or_users() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());

    let (status, _, _) = raw(&app, "GET", "/api/v1/project/export", None, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let doc = crate::project_yaml::parse_project_yaml(PROJECT).unwrap();
    crate::project_yaml::apply_import(&state.storage, &doc, crate::project_yaml::Mode::Merge)
        .await
        .unwrap();

    let (status, headers, body) =
        raw(&app, "GET", "/api/v1/project/export", Some(&cookie), "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/yaml")
    );
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("project.yaml")
    );
    assert!(body.starts_with("format: voltrus-project/1\n"), "{body}");
    assert!(body.contains("id: plant_a") && body.contains("ref: plant_a/voltage"));
    assert!(!body.contains("hub-secret-key"), "secret leaked");
    assert!(body.contains("${VOLTRUS_HUB_SITES_REMOTE_API_KEY}"));
    assert!(!body.contains("admin"), "users are not part of a project");
}

#[tokio::test]
async fn import_dry_run_then_apply_then_export_matches() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import?dry_run=true",
        Some(&cookie),
        PROJECT,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["mode"], "merge");
    assert_eq!(plan["totals"]["create"], 4);
    assert!(
        plan["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["section"] == "tags"
                && c["key"] == "plant_a/voltage"
                && c["action"] == "create")
    );
    assert!(!body.contains("hub-secret-key"));
    assert!(
        state
            .storage
            .list_modbus_devices()
            .await
            .unwrap()
            .is_empty(),
        "dry run wrote"
    );

    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import",
        Some(&cookie),
        PROJECT,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(plan["dry_run"], false);
    assert_eq!(state.storage.list_modbus_devices().await.unwrap().len(), 1);
    assert_eq!(
        state.storage.list_data_sources(None).await.unwrap().len(),
        1
    );
    // A pre-import snapshot was taken.
    let snapshots = state.storage.list_config_snapshots(10).await.unwrap();
    assert!(snapshots.iter().any(|s| s.trigger == "import"));

    // Re-importing the export is a no-op, in replace mode too (secrets stay in place).
    let (_, _, exported) = raw(&app, "GET", "/api/v1/project/export", Some(&cookie), "").await;
    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import?mode=replace&dry_run=true",
        Some(&cookie),
        &exported,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(plan["changes"].as_array().unwrap().len(), 0, "{body}");
    assert_eq!(plan["mode"], "replace");
}

#[tokio::test]
async fn invalid_projects_and_modes_are_answered_with_400_and_issues() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let dangling = PROJECT.replace("ref: plant_a/voltage", "ref: plant_a/missing");
    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import",
        Some(&cookie),
        &dangling,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let issues = json["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|i| i["message"]
            .as_str()
            .unwrap()
            .contains("unknown tag 'plant_a/missing'")),
        "{body}"
    );
    assert!(
        state
            .storage
            .list_modbus_devices()
            .await
            .unwrap()
            .is_empty()
    );

    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import",
        Some(&cookie),
        "format: nope\n",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("unsupported format"), "{body}");

    let (status, _, _) = raw(
        &app,
        "POST",
        "/api/v1/project/import",
        Some(&cookie),
        "- not: a mapping\n",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import?mode=wipe",
        Some(&cookie),
        PROJECT,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("unknown import mode"), "{body}");

    // Unset secret on a new entity: rejected with the variable named.
    let needs_env = "format: voltrus-project/1\nhub_sites:\n- {name: X, url: 'https://x', api_key: '${VOLTRUS_TEST_SURELY_UNSET_VAR}'}\n";
    let (status, _, body) = raw(
        &app,
        "POST",
        "/api/v1/project/import",
        Some(&cookie),
        needs_env,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("VOLTRUS_TEST_SURELY_UNSET_VAR"), "{body}");
}
