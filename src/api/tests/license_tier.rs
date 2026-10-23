use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_engineering_middleware_allows_access() {
    let tier = crate::license::LicenseTier::Engineering;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Engineering {
            started_at: 0,
            polling_elapsed_secs: 0,
            polling_remaining_secs: 2592000,
            daily_budget_secs: 14400,
            daily_used_secs: 0,
            polling_active: false,
            tier: tier.clone(),
        },
        tier,
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_engineering_polling_stopped_middleware_allows_access() {
    // Verify that EngineeringPollingStopped reports is_valid() == true
    let status = crate::license::LicenseStatus::EngineeringPollingStopped {
        started_at: 0,
        daily_budget_secs: 14400,
        daily_used_secs: 14400,
        message: "polling stopped".to_string(),
    };
    assert!(status.is_valid());

    // Health endpoint is always allowed even for stopped-polling licenses
    let tier = crate::license::LicenseTier::Engineering;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::EngineeringPollingStopped {
            started_at: 0,
            daily_budget_secs: 14400,
            daily_used_secs: 14400,
            message: "polling stopped".to_string(),
        },
        tier,
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ── Comprehensive Tier Enforcement Tests ──
// These tests use test_app_with_license_status() which creates a Router
// with the given tier. Tier gates are enforced at the handler level via
// state.license_tier checks, returning 403 for unauthorized tiers.
// We test with unauthenticated requests — tier gates run before auth checks
// on most endpoints, so we expect either 200/401/403 depending on endpoint.

#[tokio::test]
async fn test_tier_engineering_feature_gates() {
    let tier = crate::license::LicenseTier::Engineering;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Engineering {
            started_at: 0,
            polling_elapsed_secs: 0,
            polling_remaining_secs: 2592000,
            daily_budget_secs: 14400,
            daily_used_secs: 0,
            polling_active: false,
            tier: tier.clone(),
        },
        tier.clone(),
    );

    // SHOULD ALLOW (Engineering) — health and branding are open
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Branding returns defaults for engineering (theme_level=none)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "none");

    // Feature flags should reflect Engineering
    let flags = tier.to_feature_flags();
    assert_eq!(flags["opcua_enabled"], false);
    assert_eq!(flags["email_notifications"], false);
    assert_eq!(flags["sms_notifications"], false);
    assert_eq!(flags["s7_protocol"], false);
    assert_eq!(flags["ethernet_ip"], false);
    assert_eq!(flags["rtsp_video"], false);
    assert_eq!(flags["theme_full"], false);
    assert_eq!(flags["per_user_layouts"], false);
    assert_eq!(flags["white_label"], false);
    assert_eq!(flags["rbac"], false);
    assert_eq!(flags["multi_site"], false);
    assert_eq!(flags["max_devices"], 5);
    assert_eq!(flags["data_retention_days"], 7);
}

#[tokio::test]
async fn test_tier_starter_feature_gates() {
    let tier = crate::license::LicenseTier::Starter;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Valid {
            license_key: "test_key".into(),
            expires_at: None,
            days_remaining: None,
            tier: Some("starter".into()),
            device_limit: Some(50),
        },
        tier.clone(),
    );

    // Branding returns theme_level=basic for Starter
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "basic");

    // OPC-UA blocked (Professional+)
    assert!(!tier.opcua_enabled());

    // Feature flags
    let flags = tier.to_feature_flags();
    assert_eq!(flags["email_notifications"], true);
    assert_eq!(flags["sms_notifications"], false);
    assert_eq!(flags["white_label"], true);
    assert_eq!(flags["theme_full"], false);
    assert_eq!(flags["per_user_layouts"], false);
    assert_eq!(flags["max_devices"], 50);
}

#[tokio::test]
async fn test_tier_professional_feature_gates() {
    let tier = crate::license::LicenseTier::Professional;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Valid {
            license_key: "test_key".into(),
            expires_at: None,
            days_remaining: None,
            tier: Some("professional".into()),
            device_limit: None,
        },
        tier.clone(),
    );

    // Branding returns theme_level=full for Professional
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "full");

    // Feature flags
    let flags = tier.to_feature_flags();
    assert_eq!(flags["opcua_enabled"], true);
    assert_eq!(flags["sms_notifications"], true);
    assert_eq!(flags["pdf_reporting"], true);
    assert_eq!(flags["rbac"], true);
    assert_eq!(flags["multi_site"], true);
    assert_eq!(flags["recipes"], true);
    assert_eq!(flags["s7_protocol"], false);
    assert_eq!(flags["ethernet_ip"], false);
    assert_eq!(flags["rtsp_video"], false);
    assert_eq!(flags["theme_full"], true);
    assert_eq!(flags["per_user_layouts"], true);
    assert_eq!(flags["white_label"], true);
    assert_eq!(flags["max_devices"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_tier_enterprise_feature_gates() {
    let tier = crate::license::LicenseTier::Enterprise;
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Valid {
            license_key: "test_key".into(),
            expires_at: None,
            days_remaining: None,
            tier: Some("enterprise".into()),
            device_limit: None,
        },
        tier.clone(),
    );

    // Branding returns theme_level=full for Enterprise
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "full");

    // All feature flags should be true/max
    let flags = tier.to_feature_flags();
    assert_eq!(flags["opcua_enabled"], true);
    assert_eq!(flags["email_notifications"], true);
    assert_eq!(flags["sms_notifications"], true);
    assert_eq!(flags["s7_protocol"], true);
    assert_eq!(flags["ethernet_ip"], true);
    assert_eq!(flags["rtsp_video"], true);
    assert_eq!(flags["sso_saml"], true);
    assert_eq!(flags["cfr_part_11"], true);
    assert_eq!(flags["theme_full"], true);
    assert_eq!(flags["per_user_layouts"], true);
    assert_eq!(flags["max_devices"], serde_json::Value::Null);
}

#[tokio::test]
async fn test_branding_theme_levels_per_tier() {
    // Engineering: branding returns defaults with theme_level=none
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Engineering {
            started_at: 0,
            polling_elapsed_secs: 0,
            polling_remaining_secs: 2592000,
            daily_budget_secs: 14400,
            daily_used_secs: 0,
            polling_active: false,
            tier: crate::license::LicenseTier::Engineering,
        },
        crate::license::LicenseTier::Engineering,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "none");

    // Starter: theme_level=basic
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Valid {
            license_key: "test".into(),
            expires_at: None,
            days_remaining: None,
            tier: Some("starter".into()),
            device_limit: None,
        },
        crate::license::LicenseTier::Starter,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "basic");

    // Professional: theme_level=full
    let app = test_app_with_license_status(
        crate::license::LicenseStatus::Valid {
            license_key: "test".into(),
            expires_at: None,
            days_remaining: None,
            tier: Some("professional".into()),
            device_limit: None,
        },
        crate::license::LicenseTier::Professional,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/branding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["theme_level"], "full");
}
