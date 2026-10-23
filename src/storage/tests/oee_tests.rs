use super::*;

#[tokio::test]
async fn test_downtime_event_crud() {
    let storage = Storage::new_in_memory().unwrap();

    // Create open event
    let id = storage
        .create_downtime_event("dev1", "faulted")
        .await
        .unwrap();
    assert!(id > 0);

    // Verify open event
    let open = storage.get_open_downtime_event("dev1").await.unwrap();
    assert!(open.is_some());
    let open = open.unwrap();
    assert_eq!(open.device_id, "dev1");
    assert_eq!(open.state, "faulted");
    assert!(open.ended_at.is_none());

    // Close event
    storage
        .close_downtime_event(id, Some("Machine repaired"))
        .await
        .unwrap();

    // Verify closed
    let open_after = storage.get_open_downtime_event("dev1").await.unwrap();
    assert!(open_after.is_none());

    // List events
    let events = storage
        .list_downtime_events(Some("dev1"), None, None)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason.as_deref(), Some("Machine repaired"));
    assert!(events[0].duration_secs.is_some());
    assert!(events[0].duration_secs.unwrap() >= 0);
}

#[tokio::test]
async fn test_oee_metrics_calculation() {
    let storage = Storage::new_in_memory().unwrap();

    // Create a downtime event that represents 1 hour of downtime
    let id = storage.create_downtime_event("dev1", "idle").await.unwrap();
    storage
        .close_downtime_event(id, Some("Break"))
        .await
        .unwrap();

    // Calculate OEE for a 8-hour window
    let now = crate::timeutil::now_secs();
    let from = now - 8 * 3600;
    let metrics = storage.calculate_oee("dev1", from, now).await.unwrap();

    assert_eq!(metrics.total_planned_secs, 8 * 3600);
    assert!(metrics.total_running_secs > 0);
    assert!(metrics.availability <= 1.0);
    assert!(metrics.availability >= 0.0);
    assert_eq!(metrics.performance, 1.0); // default
    assert_eq!(metrics.quality, 1.0); // default
    assert_eq!(metrics.oee, metrics.availability);
    assert!(metrics.downtime_count >= 1);
}

#[tokio::test]
async fn test_shift_crud() {
    let storage = Storage::new_in_memory().unwrap();

    // Create shift
    let id = storage.create_shift("Morning", 6, 0, 8, 31).await.unwrap(); // Mon-Fri
    assert!(id > 0);

    // List shifts
    let shifts = storage.list_shifts().await.unwrap();
    assert_eq!(shifts.len(), 1);
    assert_eq!(shifts[0].name, "Morning");
    assert_eq!(shifts[0].start_hour, 6);
    assert_eq!(shifts[0].start_minute, 0);
    assert_eq!(shifts[0].duration_hours, 8);
    assert_eq!(shifts[0].days_mask, 31);

    // Delete shift
    storage.delete_shift(id).await.unwrap();
    let shifts = storage.list_shifts().await.unwrap();
    assert_eq!(shifts.len(), 0);
}
