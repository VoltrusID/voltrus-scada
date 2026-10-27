use super::*;
use http::StatusCode;

/// SCADA-16 B9/S4: the auto-report carried no `period_start`/`period_end` (the
/// web prints them), and its alarm section came from the newest 500 alarms.
#[tokio::test]
async fn test_auto_report_carries_period_window_and_its_alarms() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    storage
        .create_alarm("temp", "critical", 90.0, 80.0, "Temp high", 0.0)
        .await
        .unwrap();
    let now = crate::timeutil::now_secs();

    let (status, report) = send_request(
        &app,
        "GET",
        "/api/v1/shift-handover/auto-report",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let start = report["period_start"].as_i64().expect("period_start");
    let end = report["period_end"].as_i64().expect("period_end");
    assert_eq!(end - start, 8 * 3600, "a shift is eight hours");
    assert!(
        start <= now && now < end,
        "the window is the one in progress"
    );
    assert_eq!(report["alarm_count"], 1);
    assert_eq!(report["alarms"][0]["source_key"], "temp");
}
