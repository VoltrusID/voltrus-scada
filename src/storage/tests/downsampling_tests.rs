use super::*;

#[tokio::test]
async fn test_downsample_raw_to_hourly() {
    let storage = Storage::new_in_memory().unwrap();

    // Insert 2 hours of data, 2 points per hour
    let base_ts: i64 = 3600; // start at 1 hour
    let mut records = Vec::new();
    for hour in 0..2 {
        let ts = base_ts + hour * 3600;
        records.push(("dev1".into(), ts, "voltage".into(), 220.0 + hour as f64));
        records.push((
            "dev1".into(),
            ts + 60,
            "voltage".into(),
            221.0 + hour as f64,
        ));
    }
    storage.insert_batch(&records).await.unwrap();

    // Downsample everything (cutoff far in future)
    let count = storage
        .downsample_raw_to_hourly(base_ts + 7200)
        .await
        .unwrap();
    assert_eq!(count, 2); // 2 hourly buckets

    // Verify aggregated values via direct query
    let conn = storage.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT bucket_start, avg_value, min_value, max_value, sample_count
         FROM telemetry_hourly WHERE device_id = 'dev1' ORDER BY bucket_start",
        )
        .unwrap();
    let rows: Vec<(i64, f64, f64, f64, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(rows.len(), 2);
    // First hour: avg of 220.0 and 221.0 = 220.5
    assert!((rows[0].1 - 220.5).abs() < 0.01);
    assert!((rows[0].2 - 220.0).abs() < 0.01);
    assert!((rows[0].3 - 221.0).abs() < 0.01);
    assert_eq!(rows[0].4, 2);
}

#[tokio::test]
async fn test_downsample_hourly_to_daily() {
    let storage = Storage::new_in_memory().unwrap();

    // Manually insert hourly data on the same day (86400 = day 1)
    let conn = storage.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
         VALUES ('dev1', 'voltage', 86400, 220.0, 219.0, 221.0, 12)", [],
    ).unwrap();
    conn.execute(
        "INSERT INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
         VALUES ('dev1', 'voltage', 90000, 222.0, 221.0, 223.0, 12)", [],
    ).unwrap();
    drop(conn);

    // Both hourly buckets fall on the same day (86400), cutoff well past them
    let count = storage.downsample_hourly_to_daily(100000).await.unwrap();
    assert_eq!(count, 1); // 1 daily bucket

    let conn = storage.conn.lock().unwrap();
    let (avg, min, max, cnt): (f64, f64, f64, i64) = conn.query_row(
        "SELECT avg_value, min_value, max_value, sample_count FROM telemetry_daily WHERE device_id = 'dev1'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();
    // avg of 220.0 and 222.0 = 221.0
    assert!((avg - 221.0).abs() < 0.01);
    assert!((min - 219.0).abs() < 0.01);
    assert!((max - 223.0).abs() < 0.01);
    assert_eq!(cnt, 24);
}

#[tokio::test]
async fn test_delete_raw_older_than() {
    let storage = Storage::new_in_memory().unwrap();
    let records = vec![
        ("dev1".into(), 1000i64, "voltage".into(), 220.0f64),
        ("dev1".into(), 2000i64, "voltage".into(), 221.0f64),
        ("dev1".into(), 3000i64, "voltage".into(), 222.0f64),
    ];
    storage.insert_batch(&records).await.unwrap();

    let deleted = storage.delete_raw_older_than(2000).await.unwrap();
    assert_eq!(deleted, 1);

    let points = storage
        .query_history("dev1", "voltage", 0, 9999)
        .await
        .unwrap();
    assert_eq!(points.len(), 2);
}
