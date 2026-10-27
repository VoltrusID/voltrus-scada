use super::Storage;

#[tokio::test]
async fn admin_password_change_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.db");
    let path = path.to_str().unwrap();

    let storage = Storage::new(path).unwrap();
    storage
        .create_user("admin", "hash", "", "admin")
        .await
        .unwrap();
    storage
        .set_must_change_password("admin", true)
        .await
        .unwrap();
    let id = storage
        .get_user_by_username("admin")
        .await
        .unwrap()
        .unwrap()
        .id;
    storage.update_user_password(id, "new-hash").await.unwrap();
    drop(storage);

    // Opening the database again re-runs the schema migrations.
    let storage = Storage::new(path).unwrap();
    let admin = storage
        .get_user_by_username("admin")
        .await
        .unwrap()
        .unwrap();
    assert!(!admin.must_change_password);
}

/// SCADA-16 S17: DELETE FROM users failed on the foreign keys of
/// annotations, shift handovers and notifications (a 500 in the API).
#[tokio::test]
async fn delete_user_with_authored_records_and_notifications() {
    use crate::models::{
        CreateAnnotationRequest, CreateNotificationRequest, CreateShiftHandoverRequest,
    };

    let storage = Storage::new_in_memory().unwrap();
    let author = storage
        .create_user("author", "h", "", "operator")
        .await
        .unwrap();
    let other = storage
        .create_user("other", "h", "", "operator")
        .await
        .unwrap();

    let annotation = storage
        .create_annotation(
            &CreateAnnotationRequest {
                tag_id: "t".into(),
                annotation_text: "note".into(),
                data_timestamp: 1,
            },
            author,
        )
        .await
        .unwrap();
    let handover = storage
        .create_shift_handover(
            &CreateShiftHandoverRequest {
                shift_date: "2026-01-01".into(),
                shift_period: "morning".into(),
                what_happened: Some("x".into()),
                whats_pending: None,
                what_to_watch: None,
            },
            author,
        )
        .await
        .unwrap();
    for (user_id, from_user_id) in [(other, author), (author, other)] {
        storage
            .create_notification(&CreateNotificationRequest {
                user_id,
                from_user_id,
                notification_type: "mention".into(),
                message: "hi".into(),
                reference_id: None,
            })
            .await
            .unwrap();
    }

    storage
        .delete_user(author)
        .await
        .expect("delete is FK-safe");

    assert!(storage.get_user_by_id(author).await.unwrap().is_none());
    assert!(
        storage
            .list_user_notifications(other, false)
            .await
            .unwrap()
            .is_empty(),
        "notifications from the deleted user go with them"
    );
    let ghost = storage
        .get_user_by_username(crate::storage::DELETED_USER)
        .await
        .unwrap()
        .expect("authored records are kept under the deleted-user account");
    assert!(ghost.disabled);
    assert_eq!(
        storage
            .get_annotation(annotation)
            .await
            .unwrap()
            .unwrap()
            .author_id,
        ghost.id
    );
    assert_eq!(
        storage
            .get_shift_handover(handover)
            .await
            .unwrap()
            .unwrap()
            .author_id,
        ghost.id
    );
    let names: Vec<_> = storage
        .list_users()
        .await
        .unwrap()
        .into_iter()
        .map(|u| u.username)
        .collect();
    assert_eq!(
        names,
        vec!["other"],
        "the deleted-user account is not listed"
    );
}

#[tokio::test]
async fn count_other_active_admins_ignores_disabled_admins() {
    let storage = Storage::new_in_memory().unwrap();
    let a = storage.create_user("a", "h", "", "admin").await.unwrap();
    let b = storage.create_user("b", "h", "", "admin").await.unwrap();
    storage.create_user("c", "h", "", "viewer").await.unwrap();
    assert_eq!(storage.count_other_active_admins(a).await.unwrap(), 1);
    storage.set_user_disabled(b, true).await.unwrap();
    assert_eq!(storage.count_other_active_admins(a).await.unwrap(), 0);
}
