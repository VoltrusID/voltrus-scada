use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

use crate::api::tests::*;

#[tokio::test]
async fn test_csv_import_requires_auth() {
    let app = test_app();
    let boundary = "----TestBoundary12345";
    let csv = "name,value,timestamp\nPT001,42.5,1700000000\n";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.csv\"\r\nContent-Type: text/csv\r\n\r\n{}\r\n--{}--\r\n",
        boundary, csv, boundary
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/import/csv")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_csv_import_dry_run() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let csv = "name,value,timestamp\ntag_a,42.5,1700000000\ntag_b,100.0,1700000001\n";
    let boundary = "----TestBoundary12345";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.csv\"\r\nContent-Type: text/csv\r\n\r\n{}\r\n--{}--\r\n",
        boundary, csv, boundary
    );
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/import/csv?dry_run=true")
                .header("cookie", format!("voltrus_token={}", cookie))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
    assert_eq!(json["total_rows"], 2);
    assert_eq!(json["imported"], 2);
    assert_eq!(json["skipped"], 0);
    assert_eq!(json["dry_run"], true);
}

async fn post_import(
    app: &Router,
    cookie: &str,
    uri: &str,
    filename: &str,
    content: &str,
) -> (StatusCode, serde_json::Value) {
    let boundary = "----TestBoundary12345";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n{content}\r\n--{boundary}--\r\n"
    );
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// SCADA-17 S15: `mode=replace` passed validation and was then answered 501.
/// It is no longer an accepted mode (the web wizard no longer offers it); the
/// answer is a 400 that says how to replace readings.
#[tokio::test]
async fn test_import_replace_mode_is_refused_with_400() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let csv = "name,value,timestamp\ntag_a,1,1700000000\n";

    for (uri, file) in [
        ("/api/v1/import/csv?mode=replace", "data.csv"),
        ("/api/v1/import/excel?mode=replace", "data.xlsx"),
    ] {
        let (status, body) = post_import(&app, &cookie, uri, file, csv).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("DELETE /api/v1/telemetry"),
            "{body}"
        );
    }

    // merge, the default, still imports.
    let (status, body) = post_import(
        &app,
        &cookie,
        "/api/v1/import/csv?mode=merge",
        "data.csv",
        csv,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["imported"], 1);
}

/// SCADA-17 S15: imports insert readings and nothing removed them.
#[tokio::test]
async fn test_delete_telemetry_range_removes_imported_readings() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let csv = "name,value,timestamp,device_id\n\
        tag_a,1,1000,plc\ntag_a,2,2000,plc\ntag_a,3,3000,plc\ntag_b,9,2000,plc\ntag_a,7,2000,other\n";
    let (status, body) = post_import(&app, &cookie, "/api/v1/import/csv", "data.csv", csv).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["imported"], 5, "{body}");

    let values = |device: &'static str, metric: &'static str| {
        let storage = storage.clone();
        async move {
            storage
                .query_history(device, metric, 0, 10_000)
                .await
                .unwrap()
                .into_iter()
                .map(|p| p.value)
                .collect::<Vec<_>>()
        }
    };

    // Only tag_a of device plc, only the seconds from 1500 to 2500.
    let (status, body) = send_request(
        &app,
        "DELETE",
        "/api/v1/telemetry?device_id=plc&metric=tag_a&from=1500&to=2500",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"]["raw"], 1, "{body}");
    assert_eq!(values("plc", "tag_a").await, vec![1.0, 3.0]);
    assert_eq!(values("plc", "tag_b").await, vec![9.0]);
    assert_eq!(values("other", "tag_a").await, vec![7.0]);

    // Without `metric` the whole device goes, the range's ends included.
    let (status, body) = send_request(
        &app,
        "DELETE",
        "/api/v1/telemetry?device_id=plc&from=1000&to=3000",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"]["raw"], 3, "{body}");
    assert!(values("plc", "tag_a").await.is_empty());
    assert_eq!(values("other", "tag_a").await, vec![7.0]);

    for bad in [
        "/api/v1/telemetry?device_id=plc&from=5&to=1",
        "/api/v1/telemetry?device_id=&from=1&to=5",
        "/api/v1/telemetry?device_id=plc",
        "/api/v1/telemetry",
    ] {
        let (status, body) = send_request(&app, "DELETE", bad, None, Some(&cookie), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    assert_eq!(values("other", "tag_a").await, vec![7.0]);
}

#[tokio::test]
async fn test_delete_telemetry_range_is_admin_only() {
    let state = seeded_admin_app_state().await;
    let hash = crate::api::auth::hash_password("pw");
    state
        .storage
        .create_user("ops", &hash, "", "demo")
        .await
        .unwrap();
    let app = create_router(state);
    let demo = format!("voltrus_token={}", login_cookie(&app, "ops", "pw").await);

    let (status, _) = send_request(
        &app,
        "DELETE",
        "/api/v1/telemetry?device_id=plc&from=1&to=5",
        None,
        Some(&demo),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
