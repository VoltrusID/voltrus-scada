use super::*;
use axum::body::Body;
use http::Request;
use tower::ServiceExt;

async fn get_status(app: Router) -> (http::StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/license/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn core_runs_as_community_without_a_licence() {
    let (status, body) = get_status(test_app()).await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "Community");
    assert_eq!(body["tier"], "community");
    for (key, value) in body["feature_flags"].as_object().unwrap() {
        assert_eq!(value, false, "{key} must be off in the core");
    }
}

#[tokio::test]
async fn expired_licence_never_blocks_the_core() {
    let app = test_app_with_license_status(crate::license::LicenseStatus::Expired {
        expires_at: "2020-01-01T00:00:00Z".to_string(),
        message: "expired".to_string(),
    });
    let (status, body) = get_status(app.clone()).await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["status"], "Expired");
    assert_eq!(body["tier"], "community");

    // A protected core route answers 401 (needs login), never a licence 403.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/data")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn valid_enterprise_licence_reports_tier_and_flags() {
    let app = test_app_with_license_status(crate::license::LicenseStatus::Valid {
        license_key: "k".to_string(),
        expires_at: None,
        days_remaining: None,
        tier: Some("enterprise".to_string()),
        grace: false,
        legacy_white_label: false,
        site: None,
        support_until: None,
    });
    let (_, body) = get_status(app).await;
    assert_eq!(body["status"], "Valid");
    assert_eq!(body["tier"], "enterprise");
    assert_eq!(body["feature_flags"]["sso"], true);
    assert_eq!(body["feature_flags"]["cfr_part_11"], true);
}

#[tokio::test]
async fn logo_only_white_label_grant_stays_community_with_one_flag() {
    let app = test_app_with_license_status(crate::license::LicenseStatus::Valid {
        license_key: "k".to_string(),
        expires_at: None,
        days_remaining: None,
        tier: Some("starter".to_string()),
        grace: false,
        legacy_white_label: true,
        site: None,
        support_until: None,
    });
    let (_, body) = get_status(app).await;
    assert_eq!(body["status"], "Valid");
    assert_eq!(body["tier"], "community");
    assert_eq!(body["legacy_white_label"], true);
    assert_eq!(body["feature_flags"]["white_label"], true);
    assert_eq!(body["feature_flags"]["theme_full"], false);
    assert_eq!(body["feature_flags"]["multi_site"], false);
}

#[tokio::test]
async fn forbidden_app_error_answers_403_and_other_errors_500() {
    use axum::response::IntoResponse;
    let denied = crate::api::AppError::forbidden("needs an add-on licence").into_response();
    assert_eq!(denied.status(), http::StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(denied.into_body(), 1 << 20)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "needs an add-on licence");

    let failed = crate::api::AppError(anyhow::anyhow!("boom")).into_response();
    assert_eq!(failed.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
}

fn valid(
    site: Option<crate::license::LicenseSite>,
    support_until: Option<i64>,
) -> crate::license::LicenseStatus {
    crate::license::LicenseStatus::Valid {
        license_key: "k".to_string(),
        expires_at: None,
        days_remaining: None,
        tier: Some("integrator".to_string()),
        grace: false,
        legacy_white_label: false,
        site,
        support_until,
    }
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[tokio::test]
async fn community_and_legacy_keys_report_no_site_and_no_support() {
    let (_, body) = get_status(test_app()).await;
    assert_eq!(body["support"], "none");
    assert!(body.get("site").is_none());
    let (_, body) = get_status(test_app_with_license_status(valid(None, None))).await;
    assert_eq!(body["support"], "none");
    assert!(body.get("site").is_none() && body.get("support_until").is_none());
}

#[tokio::test]
async fn per_site_key_reports_site_and_support_transitions() {
    let site = || {
        Some(crate::license::LicenseSite {
            id: "s1".into(),
            name: "Plant A".into(),
        })
    };
    let future = now() + 86_400;
    let (_, body) = get_status(test_app_with_license_status(valid(site(), Some(future)))).await;
    assert_eq!(body["site"]["id"], "s1");
    assert_eq!(body["site"]["name"], "Plant A");
    assert_eq!(body["support_until"], future);
    assert_eq!(body["support"], "active");

    let past = now() - 86_400;
    let (_, body) = get_status(test_app_with_license_status(valid(site(), Some(past)))).await;
    assert_eq!(body["support"], "lapsed");
    // A lapsed contract blocks nothing.
    assert_eq!(body["status"], "Valid");
    assert_eq!(body["feature_flags"]["multi_site"], true);
}

#[tokio::test]
async fn system_features_exposes_site_and_support() {
    let app = test_app_with_license_status(valid(
        Some(crate::license::LicenseSite {
            id: "s1".into(),
            name: "Plant A".into(),
        }),
        Some(now() + 3600),
    ));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/system/features")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["license"]["site"]["name"], "Plant A");
    assert_eq!(body["license"]["support"], "active");
    assert_eq!(body["license"]["tier"], "integrator");
}
