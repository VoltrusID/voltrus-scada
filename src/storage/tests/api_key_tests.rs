use super::*;

#[tokio::test]
async fn test_api_key_create_and_validate() {
    let storage = Storage::new_in_memory().unwrap();

    let (id, plain_key) = storage
        .create_api_key("test-key", "admin", None, "[]", "api")
        .await
        .unwrap();
    assert!(id > 0);
    assert_eq!(plain_key.len(), 64); // 32 bytes hex-encoded

    // Validate the key
    let result = storage.validate_api_key(&plain_key).await.unwrap();
    assert!(result.is_some());
    let (key_id, permissions, role) = result.unwrap();
    assert_eq!(key_id, id);
    assert_eq!(permissions, "[]");
    assert_eq!(role, "api");

    // Wrong key should fail
    let result = storage.validate_api_key("wrong_key").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_api_key_list_and_revoke() {
    let storage = Storage::new_in_memory().unwrap();

    let (id1, _) = storage
        .create_api_key("key-one", "admin", None, "[]", "api")
        .await
        .unwrap();
    let (_id2, _) = storage
        .create_api_key("key-two", "admin", None, "[\"data:read\"]", "api")
        .await
        .unwrap();

    let keys = storage.list_api_keys().await.unwrap();
    assert_eq!(keys.len(), 2);

    // Verify key prefix is first 8 chars and key_hash is never returned
    for k in &keys {
        assert_eq!(k.key_prefix.len(), 8);
    }

    // Revoke one
    let deleted = storage.revoke_api_key(id1).await.unwrap();
    assert!(deleted);

    let keys = storage.list_api_keys().await.unwrap();
    assert_eq!(keys.len(), 1);

    // Revoking again should return false
    let deleted = storage.revoke_api_key(id1).await.unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn test_api_key_expiry() {
    let storage = Storage::new_in_memory().unwrap();

    // Create key that expired in the past
    let (_, plain_key) = storage
        .create_api_key("expired-key", "admin", Some(1000), "[]", "api")
        .await
        .unwrap();

    // Should not validate (expired)
    let result = storage.validate_api_key(&plain_key).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_api_key_last_used_at_updated() {
    let storage = Storage::new_in_memory().unwrap();

    let (_id, plain_key) = storage
        .create_api_key("usage-key", "admin", None, "[]", "api")
        .await
        .unwrap();

    // Initially last_used_at should be None
    let keys = storage.list_api_keys().await.unwrap();
    assert!(keys[0].last_used_at.is_none());

    // Validate (which updates last_used_at)
    storage.validate_api_key(&plain_key).await.unwrap();

    let keys = storage.list_api_keys().await.unwrap();
    assert!(keys[0].last_used_at.is_some());
}

#[tokio::test]
async fn test_api_key_role_roundtrip() {
    let storage = Storage::new_in_memory().unwrap();

    let (_, integration_key) = storage
        .create_api_key("integration-key", "admin", None, "[]", "integration")
        .await
        .unwrap();
    let (_, _, role) = storage
        .validate_api_key(&integration_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(role, "integration");

    let (_, default_key) = storage
        .create_api_key("default-key", "admin", None, "[]", "api")
        .await
        .unwrap();
    let (_, _, role) = storage
        .validate_api_key(&default_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(role, "api");

    let keys = storage.list_api_keys().await.unwrap();
    let integration = keys.iter().find(|k| k.name == "integration-key").unwrap();
    assert_eq!(integration.role, "integration");
}
