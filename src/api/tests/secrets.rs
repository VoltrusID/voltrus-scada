//! Secrets at rest: the copilot's LLM key and the generated admin password.

use super::*;
use http::StatusCode;
use serde_json::json;

const LLM_KEY: &str = "sk-test-abcdefghijklmnop-WXYZ";

async fn stored_ai_key(storage: &Storage) -> String {
    storage
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key = 'ai_api_key'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn llm_key_is_encrypted_at_rest_and_never_returned() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let secret = state.auth_secret.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/ai/settings",
        None,
        Some(&cookie),
        Some(json!({"provider": "byok_openai", "api_key": LLM_KEY})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let stored = stored_ai_key(&storage).await;
    assert!(stored.starts_with("enc:v1:"), "{stored}");
    assert!(!stored.contains("abcdefghijklmnop"));

    let (status, body) = send_request(
        &app,
        "GET",
        "/api/v1/ai/settings",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_key_set"], true);
    assert_eq!(body["api_key_preview"], "****WXYZ");
    assert!(!body.to_string().contains("abcdefghijklmnop"));

    // The server itself still reads it back.
    match crate::ai_copilot::key_store::load(&storage, &secret)
        .await
        .unwrap()
    {
        crate::ai_copilot::key_store::StoredKey::Readable(k) => assert_eq!(k, LLM_KEY),
        _ => panic!("key not readable"),
    }
    // Another auth_secret cannot.
    assert!(matches!(
        crate::ai_copilot::key_store::load(&storage, "a-different-secret-0000")
            .await
            .unwrap(),
        crate::ai_copilot::key_store::StoredKey::Unreadable
    ));
}

#[tokio::test]
async fn plaintext_llm_key_from_an_older_version_is_encrypted_at_start() {
    let storage = Storage::new_in_memory().unwrap();
    storage.set_ai_setting("api_key", LLM_KEY).await.unwrap();
    let secret = "test-secret-0123456789";
    assert!(
        crate::ai_copilot::key_store::migrate_plaintext(&storage, secret)
            .await
            .unwrap()
    );
    let stored = stored_ai_key(&storage).await;
    assert!(stored.starts_with("enc:v1:"));
    assert!(
        !crate::ai_copilot::key_store::migrate_plaintext(&storage, secret)
            .await
            .unwrap(),
        "idempotent"
    );
    assert!(matches!(
        crate::ai_copilot::key_store::load(&storage, secret).await.unwrap(),
        crate::ai_copilot::key_store::StoredKey::Readable(k) if k == LLM_KEY
    ));
}

#[tokio::test]
async fn generated_admin_password_goes_to_an_owner_only_file_removed_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("voltrus.db");
    let storage = Storage::new(db.to_str().unwrap()).unwrap();
    storage.seed_database(&[]).await.unwrap();

    let path = dir.path().join(crate::storage::INITIAL_ADMIN_PASSWORD_FILE);
    let password = std::fs::read_to_string(&path).unwrap().trim().to_string();
    assert_eq!(password.len(), 16);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let mut state = seeded_admin_app_state().await;
    state.storage = storage;
    let app = create_router(state);
    let cookie = format!(
        "voltrus_token={}",
        login_cookie(&app, "admin", &password).await
    );
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(&cookie),
        Some(json!({"old_password": password, "new_password": "a-new-password-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !path.exists(),
        "the file goes once the admin has chosen a password"
    );
}
