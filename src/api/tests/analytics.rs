use super::*;

#[test]
fn test_analytics_change_percent() {
    let current_avg = 110.0_f64;
    let previous_avg = 100.0_f64;
    let change = ((current_avg - previous_avg) / previous_avg) * 100.0;
    assert!((change - 10.0).abs() < 0.001);
}

#[test]
fn test_analytics_change_percent_zero_division() {
    let current_avg = 5.0_f64;
    let previous_avg = 0.0_f64;
    let change = if previous_avg.abs() > f64::EPSILON {
        ((current_avg - previous_avg) / previous_avg) * 100.0
    } else {
        0.0
    };
    assert_eq!(change, 0.0);
}

#[test]
fn test_analytics_change_percent_negative() {
    let current_avg = 80.0_f64;
    let previous_avg = 100.0_f64;
    let change = ((current_avg - previous_avg) / previous_avg) * 100.0;
    assert!((change - (-20.0)).abs() < 0.001);
}

#[test]
fn test_recipe_values_json_parse() {
    let json = r#"{"temp": 25.0, "pressure": 1.5}"#;
    let values: std::collections::HashMap<String, f64> = serde_json::from_str(json).unwrap();
    assert_eq!(values.len(), 2);
    assert!((values["temp"] - 25.0).abs() < 0.001);
    assert!((values["pressure"] - 1.5).abs() < 0.001);
}

/// SCADA-16 S3: a period without values used to read NULL MIN/MAX/AVG into f64 and answer 500.
#[tokio::test]
async fn test_compare_with_an_empty_period_answers_zeros() {
    let state = seeded_admin_app_state().await;
    state
        .storage
        .insert_batch(&[
            ("dev".to_string(), 1000, "temp".to_string(), 10.0),
            ("dev".to_string(), 1010, "temp".to_string(), 30.0),
        ])
        .await
        .unwrap();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = send_request(
        &app,
        "GET",
        "/api/v1/analytics/compare?device_id=dev&metric=temp\
         &period1_from=1000&period1_to=1100&period2_from=0&period2_to=999",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["current"]["count"], 2);
    assert_eq!(body["current"]["min"], 10.0);
    assert_eq!(body["current"]["max"], 30.0);
    assert_eq!(body["current"]["avg"], 20.0);
    assert_eq!(body["previous"]["count"], 0);
    assert_eq!(body["previous"]["min"], 0.0);
    assert_eq!(body["previous"]["max"], 0.0);
    assert_eq!(body["previous"]["avg"], 0.0);
    assert_eq!(body["previous"]["first_ts"], 0);
    assert_eq!(body["previous"]["last_ts"], 999);

    // Both periods empty is not an error either.
    let (status, body) = send_request(
        &app,
        "GET",
        "/api/v1/analytics/compare?device_id=none&metric=none\
         &period1_from=1&period1_to=2&period2_from=3&period2_to=4",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["current"]["count"], 0);
    assert_eq!(body["change_percent"], 0.0);
}
