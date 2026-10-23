use super::*;

#[tokio::test]
async fn test_insert_and_list_security_events() {
    let storage = Storage::new_in_memory().unwrap();

    // Insert events
    storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "login_failed".into(),
            severity: "warning".into(),
            source_ip: Some("192.168.1.100".into()),
            user_agent: None,
            username: Some("admin".into()),
            details: Some("Invalid password (attempt 1)".into()),
        })
        .await
        .unwrap();

    storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "auth_401".into(),
            severity: "info".into(),
            source_ip: Some("10.0.0.1".into()),
            user_agent: None,
            username: None,
            details: Some("Unauthorized request".into()),
        })
        .await
        .unwrap();

    storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "config_change".into(),
            severity: "info".into(),
            source_ip: None,
            user_agent: None,
            username: Some("admin".into()),
            details: Some("Updated branding".into()),
        })
        .await
        .unwrap();

    // List all
    let events = storage
        .list_security_events(None, None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);

    // Filter by event_type
    let login_events = storage
        .list_security_events(Some("login_failed"), None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(login_events.len(), 1);
    assert_eq!(login_events[0].event_type, "login_failed");

    // Filter by severity
    let warnings = storage
        .list_security_events(None, Some("warning"), None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].severity, "warning");

    // Pagination
    let page = storage
        .list_security_events(None, None, None, None, 2, 0)
        .await
        .unwrap();
    assert_eq!(page.len(), 2);
    let page2 = storage
        .list_security_events(None, None, None, None, 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 1);
}

#[tokio::test]
async fn test_export_security_events_csv() {
    let storage = Storage::new_in_memory().unwrap();

    storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "login_failed".into(),
            severity: "warning".into(),
            source_ip: Some("192.168.1.100".into()),
            user_agent: Some("Mozilla/5.0".into()),
            username: Some("admin".into()),
            details: Some("Invalid password".into()),
        })
        .await
        .unwrap();

    let csv = storage
        .export_security_events_csv(None, None)
        .await
        .unwrap();

    // Check CSV header
    assert!(
        csv.starts_with(
            "id,event_type,severity,source_ip,user_agent,username,details,created_at\n"
        )
    );
    // Check data row
    assert!(csv.contains("login_failed"));
    assert!(csv.contains("warning"));
    assert!(csv.contains("192.168.1.100"));
    assert!(csv.contains("admin"));
}

#[tokio::test]
async fn test_security_thresholds_crud() {
    let storage = Storage::new_in_memory().unwrap();

    // Get defaults
    let thresholds = storage.get_security_thresholds().await.unwrap();
    assert_eq!(thresholds.failed_login_threshold, 5);
    assert_eq!(thresholds.suspicious_401_threshold, 10);
    assert_eq!(thresholds.rate_limit_threshold, 100);

    // Update
    let updated = crate::models::SecurityThresholds {
        failed_login_threshold: 10,
        failed_login_window_secs: 120,
        suspicious_401_threshold: 20,
        suspicious_401_window_secs: 120,
        rate_limit_threshold: 200,
        rate_limit_window_secs: 30,
    };
    storage.update_security_thresholds(&updated).await.unwrap();

    // Verify
    let thresholds = storage.get_security_thresholds().await.unwrap();
    assert_eq!(thresholds.failed_login_threshold, 10);
    assert_eq!(thresholds.suspicious_401_threshold, 20);
    assert_eq!(thresholds.rate_limit_threshold, 200);
}

#[tokio::test]
async fn test_security_events_append_only() {
    let storage = Storage::new_in_memory().unwrap();

    // Insert multiple events
    for i in 0..5 {
        storage
            .insert_security_event(&crate::models::CreateSecurityEvent {
                event_type: "auth_401".into(),
                severity: "warning".into(),
                source_ip: Some(format!("10.0.0.{}", i)),
                user_agent: None,
                username: None,
                details: None,
            })
            .await
            .unwrap();
    }

    // All events should be present (append-only, no deletion)
    let events = storage
        .list_security_events(None, None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn test_security_events_csv_escapes_special_chars() {
    let storage = Storage::new_in_memory().unwrap();

    storage
        .insert_security_event(&crate::models::CreateSecurityEvent {
            event_type: "login_failed".into(),
            severity: "warning".into(),
            source_ip: Some("192.168.1.1".into()),
            user_agent: Some("Agent with \"quotes\"".into()),
            username: Some("user,test".into()),
            details: Some("Line1\nLine2".into()),
        })
        .await
        .unwrap();

    let csv = storage
        .export_security_events_csv(None, None)
        .await
        .unwrap();
    assert!(csv.contains("user,test"));
    assert!(csv.contains("\\n"));
}
