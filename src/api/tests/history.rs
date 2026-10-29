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

/// SCADA-17 B10: two devices may share a tag key, and `data` keyed by key kept only the later one.
#[tokio::test]
async fn test_history_keyed_by_id_keeps_tags_that_share_a_key() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    for device in ["plc-1", "plc-2"] {
        create_source(
            &app,
            &cookie,
            serde_json::json!({
                "key": "frequency", "name": "Frequency", "source_type": "random_walk",
                "device_id": device,
            }),
        )
        .await;
    }

    let by_key = history(&app, &cookie, "range=30m").await;
    assert_eq!(by_key["sensors"].as_array().unwrap().len(), 2);
    assert_eq!(
        by_key["data"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["frequency"],
        "the key form is ambiguous and stays as it was"
    );

    let by_id = history(&app, &cookie, "range=30m&keyed_by=id").await;
    let mut ids: Vec<String> = by_id["sensors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_i64().unwrap().to_string())
        .collect();
    ids.sort();
    let mut keys: Vec<String> = by_id["data"].as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(ids.len(), 2);
    assert_eq!(keys, ids, "one series per data source id");
    for id in &ids {
        assert!(by_id["data"][id]["count"].as_i64().unwrap() > 0, "{id}");
    }
}

/// Packing history into compressed blocks must not change what the trend shows.
#[tokio::test]
async fn test_history_is_the_same_after_packing() {
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

    let start = (crate::timeutil::now_secs() - 1500) / 10 * 10;
    let rows: Vec<(String, i64, String, f64)> = (0..120)
        .map(|i| {
            (
                "inverter-1".to_string(),
                start + i * 10,
                "pv_power".to_string(),
                50.0 + (i % 17) as f64 * 1.25,
            )
        })
        .collect();
    storage.insert_batch(&rows).await.unwrap();

    let queries = ["range=30m", "range=30m&agg=avg", "range=30m&agg=minmax"];
    let mut before = Vec::new();
    for q in queries {
        before.push(history(&app, &cookie, q).await);
    }
    assert_eq!(before[0]["data"]["pv_power"]["count"], 120);

    // Pack everything, current hour included.
    let report = storage
        .pack_history(crate::timeutil::now_secs() + 2 * 3600, usize::MAX)
        .await
        .unwrap();
    assert!(report.samples == 120);

    for (q, expected) in queries.iter().zip(&before) {
        assert_eq!(&history(&app, &cookie, q).await, expected, "{q}");
    }

    let (status, stats) = send_request(
        &app,
        "GET",
        "/api/v1/compression/stats",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["packed_samples"], 120);
    assert_eq!(stats["hot_rows"], 0);
    assert!(stats["packed_bytes_per_sample"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn test_compression_config_and_deadband_reflect_the_running_setup() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    create_source(
        &app,
        &cookie,
        serde_json::json!({
            "key": "level", "name": "Level", "source_type": "modbus", "device_id": "tank-1",
            "source_config": {
                "history_deadband": 0.5, "history_deadband_type": "percent",
                "history_max_interval_secs": 300,
                "deadband": 3.0
            },
        }),
    )
    .await;
    create_source(
        &app,
        &cookie,
        serde_json::json!({
            "key": "flow", "name": "Flow", "source_type": "modbus", "device_id": "tank-1",
            "source_config": {"deadband": 3.0},
        }),
    )
    .await;

    let (status, entries) = send_request(
        &app,
        "GET",
        "/api/v1/compression/deadband",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Only the tag with history_deadband; the alarm `deadband` key is something else.
    assert_eq!(
        entries,
        serde_json::json!([{
            "device_id": "tank-1", "metric": "level", "deadband_type": "percent",
            "deadband_value": 0.5, "max_interval_secs": 300
        }])
    );

    let (status, config) = send_request(
        &app,
        "GET",
        "/api/v1/compression/config",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(config["pack_enabled"], true);
    assert_eq!(config["pack_after_secs"], 3600);
    assert_eq!(config["raw_keep_secs"], 7_776_000);
    assert_eq!(config["daily_keep_secs"], serde_json::Value::Null);
}

const TAGS: usize = 46;

/// A device with 46 tags: dense data (every 5 s) for the last 40 minutes, a 30 s
/// cadence before that back past 24 h. Tag `i` reads `i` except its newest sample,
/// which reads `1000 + i`, so a series that ends early is visible in the values.
async fn seed_wide_device(
    app: &Router,
    cookie: &str,
    storage: &crate::storage::Storage,
) -> (i64, Vec<String>) {
    let keys: Vec<String> = (0..TAGS).map(|i| format!("tag{i:02}")).collect();
    for key in &keys {
        create_source(
            app,
            cookie,
            serde_json::json!({
                "key": key, "name": key, "source_type": "modbus", "device_id": "plant-1",
            }),
        )
        .await;
    }
    let newest = crate::timeutil::now_secs() / 5 * 5 - 5;
    let mut rows = Vec::new();
    let mut ts = newest;
    while ts > newest - 25 * 3600 {
        for (i, key) in keys.iter().enumerate() {
            let value = if ts == newest {
                1000.0 + i as f64
            } else {
                i as f64
            };
            rows.push(("plant-1".to_string(), ts, key.clone(), value));
        }
        ts -= if ts > newest - 40 * 60 { 5 } else { 30 };
    }
    storage.insert_batch(&rows).await.unwrap();
    (newest, keys)
}

/// Every tag's series reaches both edges of the requested window.
fn assert_whole_window(
    body: &serde_json::Value,
    keys: &[String],
    newest: i64,
    window_secs: i64,
    label: &str,
) {
    for (i, key) in keys.iter().enumerate() {
        let series = &body["data"][key];
        let count = series["count"].as_u64().unwrap_or(0);
        assert!(count > 1, "{label}: {key} has {count} points");
        let first = series["start"].as_i64().unwrap() / 1000;
        let interval = series["interval"].as_u64().unwrap() as i64;
        let last = first + (count as i64 - 1) * interval / 1000;
        // One bucket of slack at either edge; the series spacing is at most that.
        let slack = (window_secs / 100).max(60);
        assert!(
            first <= newest - window_secs + slack,
            "{label}: {key} starts {}s into the window",
            first - (newest - window_secs)
        );
        assert!(
            last >= newest - slack,
            "{label}: {key} ends {}s before the newest sample",
            newest - last
        );
        let values = floats(&series["values"]);
        assert_eq!(values.len() as u64, count, "{label}: {key}");
        let _ = i;
    }
}

#[tokio::test]
async fn test_history_covers_the_whole_window_for_every_tag() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (newest, keys) = seed_wide_device(&app, &cookie, &storage).await;

    let queries = [
        ("range=30m", 1800),
        ("range=30m&detail=full", 1800),
        ("range=30m&agg=avg", 1800),
        ("range=30m&agg=minmax", 1800),
        ("range=24h", 86400),
        ("range=24h&detail=full", 86400),
        ("range=24h&agg=avg", 86400),
        ("range=24h&agg=minmax", 86400),
    ];
    for stage in ["raw", "packed"] {
        if stage == "packed" {
            // Everything older than the last hour moves into blocks.
            let packed = storage
                .pack_history(crate::timeutil::now_secs() - 3600, usize::MAX)
                .await
                .unwrap();
            assert!(packed.samples > 0);
        }
        for (query, window) in queries {
            let body = history(&app, &cookie, query).await;
            assert_whole_window(&body, &keys, newest, window, &format!("{stage} {query}"));
        }
        // The newest sample of every tag is in the sampled answer.
        let body = history(&app, &cookie, "range=30m").await;
        for (i, key) in keys.iter().enumerate() {
            let values = floats(&body["data"][key]["values"]);
            assert_eq!(
                *values.last().unwrap(),
                1000.0 + i as f64,
                "{stage}: {key} newest value"
            );
        }
    }
}

/// Long ranges answered from data of three kinds of age: dense recent samples,
/// a 30 s cadence for the last day, a 10 min cadence back 31 days, and (past
/// the full-resolution window) hourly rows only, from 95 to 120 days ago.
async fn seed_long_device(
    app: &Router,
    cookie: &str,
    storage: &crate::storage::Storage,
) -> (i64, Vec<String>) {
    let keys: Vec<String> = (0..TAGS).map(|i| format!("tag{i:02}")).collect();
    for key in &keys {
        create_source(
            app,
            cookie,
            serde_json::json!({
                "key": key, "name": key, "source_type": "modbus", "device_id": "plant-2",
            }),
        )
        .await;
    }
    let newest = crate::timeutil::now_secs() / 5 * 5 - 5;
    let mut rows = Vec::new();
    let mut ts = newest;
    while ts > newest - 31 * 86400 {
        for (i, key) in keys.iter().enumerate() {
            let value = if ts == newest {
                1000.0 + i as f64
            } else {
                i as f64
            };
            rows.push(("plant-2".to_string(), ts, key.clone(), value));
        }
        ts -= match newest - ts {
            0..=2399 => 5,
            2400..=86_399 => 30,
            _ => 600,
        };
    }
    storage.insert_batch(&rows).await.unwrap();
    let hourly_keys = keys.clone();
    storage
        .with_conn(move |conn| {
            let tx = conn.transaction()?;
            for key in &hourly_keys {
                let mut hour = (newest - 120 * 86400) / 3600 * 3600;
                while hour < newest - 95 * 86400 {
                    tx.execute(
                        "INSERT INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
                         VALUES ('plant-2', ?1, ?2, 5.0, 4.0, 6.0, 720)",
                        rusqlite::params![key, hour],
                    )?;
                    hour += 3600;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    (newest, keys)
}

#[tokio::test]
async fn test_history_long_ranges_are_not_empty_and_cover_the_window() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (newest, keys) = seed_long_device(&app, &cookie, &storage).await;
    let now = crate::timeutil::now_secs();
    let custom_120d = format!("range={}-{}", (now - 120 * 86400) * 1000, now * 1000);

    // The auto detail levels the client sends, and detail=full, for each range.
    let mut queries: Vec<(String, i64)> = Vec::new();
    for (range, secs) in [("24h", 86_400), ("7d", 7 * 86_400), ("30d", 30 * 86_400)] {
        for extra in ["", "&agg=avg", "&agg=minmax", "&detail=full"] {
            queries.push((format!("range={range}{extra}"), secs));
        }
    }
    queries.push((custom_120d.clone(), 120 * 86_400));
    queries.push((format!("{custom_120d}&agg=minmax"), 120 * 86_400));

    for stage in ["raw", "packed"] {
        if stage == "packed" {
            let packed = storage
                .pack_history(crate::timeutil::now_secs() - 3600, usize::MAX)
                .await
                .unwrap();
            assert!(packed.samples > 0);
        }
        for (query, window) in &queries {
            let body = history(&app, &cookie, &format!("{query}&device=plant-2")).await;
            assert_whole_window(&body, &keys, newest, *window, &format!("{stage} {query}"));
        }
    }
}
