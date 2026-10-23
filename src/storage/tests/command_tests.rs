use super::*;

#[tokio::test]
async fn test_command_crud_lifecycle() {
    let storage = Storage::new_in_memory().unwrap();

    // Insert
    let cmd = Command {
        id: "cmd_test1".to_string(),
        data_source_id: 1,
        device_id: 1,
        protocol: "modbus".to_string(),
        value: "100.5".to_string(),
        status: "pending".to_string(),
        policy: "immediate".to_string(),
        scheduled_at: None,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: 1000,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test_user".to_string(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };
    storage.insert_command(&cmd).await.unwrap();

    // Get
    let fetched = storage.get_command("cmd_test1").await.unwrap().unwrap();
    assert_eq!(fetched.status, "pending");
    assert_eq!(fetched.value, "100.5");
    assert_eq!(fetched.created_by, "test_user");

    // Update to acked
    storage
        .update_command_status("cmd_test1", "acked", None, "system", None)
        .await
        .unwrap();
    let fetched = storage.get_command("cmd_test1").await.unwrap().unwrap();
    assert_eq!(fetched.status, "acked");
    assert!(fetched.acked_at.is_some());
    assert!(fetched.resolved_at.is_some());
}

#[tokio::test]
async fn test_command_audit_trail() {
    let storage = Storage::new_in_memory().unwrap();

    let cmd = Command {
        id: "cmd_audit1".to_string(),
        data_source_id: 1,
        device_id: 1,
        protocol: "modbus".to_string(),
        value: "50".to_string(),
        status: "pending".to_string(),
        policy: "immediate".to_string(),
        scheduled_at: None,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: 1000,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test_user".to_string(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };
    storage.insert_command(&cmd).await.unwrap();

    storage
        .update_command_status(
            "cmd_audit1",
            "queued",
            Some("device offline"),
            "system",
            Some("retry"),
        )
        .await
        .unwrap();
    storage
        .update_command_status("cmd_audit1", "acked", None, "system", None)
        .await
        .unwrap();

    let audit = storage.get_command_audit("cmd_audit1").await.unwrap();
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0].old_status, "pending");
    assert_eq!(audit[0].new_status, "queued");
    assert_eq!(audit[1].old_status, "queued");
    assert_eq!(audit[1].new_status, "acked");
}

#[tokio::test]
async fn test_count_recent_writes() {
    let storage = Storage::new_in_memory().unwrap();
    let now = crate::timeutil::now_secs();

    for i in 0..5 {
        let cmd = Command {
            id: format!("cmd_rate{}", i),
            data_source_id: 42,
            device_id: 1,
            protocol: "modbus".to_string(),
            value: "10".to_string(),
            status: "acked".to_string(),
            policy: "immediate".to_string(),
            scheduled_at: None,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at: now,
            sent_at: None,
            acked_at: None,
            resolved_at: None,
            created_by: "test".to_string(),
            readback_status: None,
            readback_value: None,
            readback_attempts: 0,
            readback_ts: None,
        };
        storage.insert_command(&cmd).await.unwrap();
    }

    let count = storage.count_recent_writes(42, 60).await.unwrap();
    assert_eq!(count, 5);

    // Different data source should be 0
    let count2 = storage.count_recent_writes(99, 60).await.unwrap();
    assert_eq!(count2, 0);
}

#[tokio::test]
async fn test_list_commands_with_filters() {
    let storage = Storage::new_in_memory().unwrap();
    let now = crate::timeutil::now_secs();

    // Insert commands with different statuses
    for i in 0..3 {
        let cmd = Command {
            id: format!("cmd_list_acked_{}", i),
            data_source_id: 1,
            device_id: 1,
            protocol: "modbus".to_string(),
            value: "10".to_string(),
            status: "acked".to_string(),
            policy: "immediate".to_string(),
            scheduled_at: None,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at: now,
            sent_at: None,
            acked_at: None,
            resolved_at: None,
            created_by: "test".to_string(),
            readback_status: None,
            readback_value: None,
            readback_attempts: 0,
            readback_ts: None,
        };
        storage.insert_command(&cmd).await.unwrap();
    }

    let failed_cmd = Command {
        id: "cmd_list_failed".to_string(),
        data_source_id: 2,
        device_id: 2,
        protocol: "modbus".to_string(),
        value: "20".to_string(),
        status: "failed".to_string(),
        policy: "immediate".to_string(),
        scheduled_at: None,
        retry_count: 3,
        max_retries: 3,
        error_message: Some("timeout".to_string()),
        created_at: now,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test".to_string(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };
    storage.insert_command(&failed_cmd).await.unwrap();

    // Filter by status
    let query = CommandQuery {
        status: Some("acked".to_string()),
        ..Default::default()
    };
    let results = storage.list_commands(&query).await.unwrap();
    assert_eq!(results.len(), 3);

    // Filter by device
    let query2 = CommandQuery {
        device: Some("2".to_string()),
        ..Default::default()
    };
    let results2 = storage.list_commands(&query2).await.unwrap();
    assert_eq!(results2.len(), 1);
    assert_eq!(results2[0].status, "failed");
}

#[tokio::test]
async fn test_get_queued_commands() {
    let storage = Storage::new_in_memory().unwrap();

    // Insert a pending command
    let cmd = Command {
        id: "cmd_q1".to_string(),
        data_source_id: 1,
        device_id: 1,
        protocol: "modbus".to_string(),
        value: "10".to_string(),
        status: "pending".to_string(),
        policy: "immediate".to_string(),
        scheduled_at: None,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: 1000,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test".to_string(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };
    storage.insert_command(&cmd).await.unwrap();

    // Insert a queued command
    let cmd2 = Command {
        id: "cmd_q2".to_string(),
        data_source_id: 1,
        device_id: 1,
        protocol: "modbus".to_string(),
        value: "20".to_string(),
        status: "queued".to_string(),
        policy: "immediate".to_string(),
        scheduled_at: None,
        retry_count: 1,
        max_retries: 3,
        error_message: None,
        created_at: 2000,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: "test".to_string(),
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };
    storage.insert_command(&cmd2).await.unwrap();

    let queued = storage.get_queued_commands().await.unwrap();
    assert_eq!(queued.len(), 2);
}
