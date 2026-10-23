use super::*;

#[tokio::test]
async fn test_insert_and_query() {
    let storage = Storage::new_in_memory().unwrap();

    let records = vec![
        ("dev1".into(), 1000i64, "voltage".into(), 220.5f64),
        ("dev1".into(), 1060i64, "voltage".into(), 221.0f64),
        ("dev1".into(), 1120i64, "voltage".into(), 219.8f64),
    ];
    storage.insert_batch(&records).await.unwrap();

    let points = storage
        .query_history("dev1", "voltage", 0, 2000)
        .await
        .unwrap();

    assert_eq!(points.len(), 3);
    assert_eq!(
        points[0],
        DataPoint {
            timestamp: 1000,
            value: 220.5
        }
    );
    assert_eq!(
        points[1],
        DataPoint {
            timestamp: 1060,
            value: 221.0
        }
    );
    assert_eq!(
        points[2],
        DataPoint {
            timestamp: 1120,
            value: 219.8
        }
    );
}

#[tokio::test]
async fn test_query_filters_by_device_and_metric() {
    let storage = Storage::new_in_memory().unwrap();

    let records = vec![
        ("dev1".into(), 1000, "voltage".into(), 220.0),
        ("dev1".into(), 1000, "current".into(), 5.0),
        ("dev2".into(), 1000, "voltage".into(), 380.0),
    ];
    storage.insert_batch(&records).await.unwrap();

    let points = storage
        .query_history("dev1", "voltage", 0, 2000)
        .await
        .unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].value, 220.0);
}

#[tokio::test]
async fn test_query_time_range() {
    let storage = Storage::new_in_memory().unwrap();

    let records = vec![
        ("dev1".into(), 1000, "voltage".into(), 220.0),
        ("dev1".into(), 2000, "voltage".into(), 221.0),
        ("dev1".into(), 3000, "voltage".into(), 222.0),
    ];
    storage.insert_batch(&records).await.unwrap();

    let points = storage
        .query_history("dev1", "voltage", 1500, 2500)
        .await
        .unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].value, 221.0);
}

#[tokio::test]
async fn test_empty_batch() {
    let storage = Storage::new_in_memory().unwrap();
    let records: Vec<(String, i64, String, f64)> = vec![];
    storage.insert_batch(&records).await.unwrap();
}

#[tokio::test]
async fn test_no_results() {
    let storage = Storage::new_in_memory().unwrap();
    let points = storage
        .query_history("ghost", "voltage", 0, 9999)
        .await
        .unwrap();
    assert!(points.is_empty());
}
