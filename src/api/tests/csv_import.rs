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
