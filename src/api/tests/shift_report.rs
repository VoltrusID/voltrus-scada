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

/// SCADA-17 B11: handover entries carried only the author's user id.
#[tokio::test]
async fn test_handovers_carry_the_author_username() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/shift-handover",
        None,
        Some(&cookie),
        Some(serde_json::json!({
            "shift_date": "2026-10-25", "shift_period": "night",
            "what_happened": "Inverter 2 tripped",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_i64().unwrap();

    let (status, list) = send_request(
        &app,
        "GET",
        "/api/v1/shift-handover?shift_date=2026-10-25&shift_period=night",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["count"], 1);
    assert_eq!(list["handovers"][0]["author_username"], "admin");
    assert!(list["handovers"][0]["author_id"].as_i64().is_some());

    let (status, one) = send_request(
        &app,
        "GET",
        &format!("/api/v1/shift-handover/{id}"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["author_username"], "admin");
}
