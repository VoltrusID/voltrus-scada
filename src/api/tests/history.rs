use super::*;
use http::StatusCode;

async fn create_source(app: &Router, cookie: &str, body: serde_json::Value) {
    let (status, _) = send_request(
        app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(cookie),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

async fn history(app: &Router, cookie: &str, query: &str) -> serde_json::Value {
    let (status, body) = send_request(
        app,
        "GET",
        &format!("/api/v1/history?{query}"),
        None,
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

fn floats(v: &serde_json::Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

/// SCADA-16 B4: `agg=minmax` answered the average only.
#[tokio::test]
async fn test_history_minmax_returns_band_for_generated_sources() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    create_source(
        &app,
        &cookie,
        serde_json::json!({"key": "temp", "name": "Temp", "source_type": "random_walk"}),
    )
    .await;

    let body = history(&app, &cookie, "range=30m&agg=minmax").await;
    let series = &body["data"]["temp"];
    let values = floats(&series["values"]);
    let lows = floats(&series["min"]);
    let highs = floats(&series["max"]);
    assert!(!values.is_empty());
    assert_eq!(lows.len(), values.len());
    assert_eq!(highs.len(), values.len());
    for i in 0..values.len() {
        assert!(lows[i] <= values[i] && values[i] <= highs[i], "point {i}");
    }

    // The average line alone carries no band, and neither does plain sampling.
    for query in ["range=30m&agg=avg", "range=30m"] {
        let body = history(&app, &cookie, query).await;
        assert!(body["data"]["temp"].get("min").is_none(), "{query}");
    }
}

/// SCADA-16 B4: a Modbus source ignored `agg`, answering every raw sample.
#[tokio::test]
async fn test_history_aggregates_modbus_telemetry() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    create_source(
        &app,
        &cookie,
        serde_json::json!({
            "key": "pv_power", "name": "PV power", "source_type": "modbus",
            "device_id": "inverter-1",
        }),
    )
    .await;

    // 30m history buckets by 10 s. Three samples share one bucket, a fourth
    // sits in the next.
    let bucket = (crate::timeutil::now_secs() - 600) / 10 * 10;
    let samples = [
        (bucket + 1, 10.0),
        (bucket + 2, 30.0),
        (bucket + 3, 20.0),
        (bucket + 11, 100.0),
    ];
    let rows: Vec<(String, i64, String, f64)> = samples
        .iter()
        .map(|&(ts, v)| ("inverter-1".to_string(), ts, "pv_power".to_string(), v))
        .collect();
    storage.insert_batch(&rows).await.unwrap();

    let sampled = history(&app, &cookie, "range=30m").await;
    assert_eq!(sampled["data"]["pv_power"]["count"], 4, "sampling is raw");

    let avg = history(&app, &cookie, "range=30m&agg=avg").await;
    let series = &avg["data"]["pv_power"];
    assert_eq!(floats(&series["values"]), vec![20.0, 100.0]);
    assert_eq!(series["start"], bucket * 1000);
    assert!(series.get("min").is_none());

    let minmax = history(&app, &cookie, "range=30m&agg=minmax").await;
    let series = &minmax["data"]["pv_power"];
    assert_eq!(floats(&series["values"]), vec![20.0, 100.0]);
    assert_eq!(floats(&series["min"]), vec![10.0, 100.0]);
    assert_eq!(floats(&series["max"]), vec![30.0, 100.0]);
}

/// SCADA-16 B5: `/history` answered every tag whatever `sensor` said.
#[tokio::test]
async fn test_history_filters_by_sensor_and_device() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    for (key, device) in [("a", "plc-1"), ("b", "plc-1"), ("c", "plc-2")] {
        create_source(
            &app,
            &cookie,
            serde_json::json!({
                "key": key, "name": key, "source_type": "random_walk", "device_id": device,
            }),
        )
        .await;
    }

    let keys = |body: &serde_json::Value| -> Vec<String> {
        let mut keys: Vec<String> = body["data"].as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };

    assert_eq!(
        keys(&history(&app, &cookie, "range=30m").await),
        ["a", "b", "c"]
    );

    let one = history(&app, &cookie, "range=30m&sensor=b").await;
    assert_eq!(keys(&one), ["b"]);
    assert_eq!(one["sensors"].as_array().unwrap().len(), 1);

    assert_eq!(
        keys(&history(&app, &cookie, "range=30m&sensor=a,c").await),
        ["a", "c"]
    );
    assert_eq!(
        keys(&history(&app, &cookie, "range=30m&device=plc-1").await),
        ["a", "b"]
    );
    assert_eq!(
        keys(&history(&app, &cookie, "range=30m&sensor=a,c&device=plc-2").await),
        ["c"]
    );
    assert!(keys(&history(&app, &cookie, "range=30m&sensor=nope").await).is_empty());
}
