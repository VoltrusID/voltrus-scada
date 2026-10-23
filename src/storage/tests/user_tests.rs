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
