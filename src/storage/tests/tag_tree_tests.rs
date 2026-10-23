use crate::storage::Storage;

#[tokio::test]
async fn test_data_source_path_roundtrip() {
    let storage = Storage::new_in_memory().unwrap();
    let id = storage
        .create_data_source(
            "PT001_TEMP",
            "PT001_TEMP",
            "C",
            "#f97316",
            0.0,
            100.0,
            80.0,
            90.0,
            25.0,
            0,
            2,
            "modbus",
            "{}",
            "plc1",
            None,
            "sparkline",
        )
        .await
        .unwrap();
    let fields = serde_json::json!({"path": "Site A/Area 1/Unit 2/PT001"});
    storage.update_data_source(id, &fields).await.unwrap();
    let ds = storage.get_data_source_by_id(id).await.unwrap().unwrap();
    assert_eq!(ds.path.as_deref(), Some("Site A/Area 1/Unit 2/PT001"));
}
