use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_import_node_red_requires_admin() {
    let app = test_app();
    let boundary = "----TestBoundary123";
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"flows.json\"\r\nContent-Type: application/json\r\n\r\n[]\r\n--{}--\r\n",
        boundary, boundary
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/import/node-red")
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
async fn test_import_node_red_basic() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let boundary = "----TestBoundary123";
    let json_body = r#"[{"id":"n1","type":"modbus-read","name":"Read Voltage","server":"plc1","fc":3,"unitid":1,"address":"40001","quantity":"2","properties":{"host":"192.168.1.10","port":502}}]"#;
    let body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"flows.json\"\r\nContent-Type: application/json\r\n\r\n{}\r\n--{}--\r\n",
        boundary, json_body, boundary
    );

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/import/node-red")
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
    assert_eq!(json["parse"]["total_nodes"], 1);
    assert_eq!(json["conversion"]["modbus_devices"], 1);
}
