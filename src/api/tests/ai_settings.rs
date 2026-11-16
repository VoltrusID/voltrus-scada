//! AI copilot settings: the OpenAI-compatible provider (custom base URL,
//! optional key).

use super::*;
use axum::Json;
use axum::http::HeaderMap;
use http::StatusCode;
use serde_json::{Value, json};
use std::sync::Mutex;

async fn admin() -> (Router, String) {
    let app = create_router(seeded_admin_app_state().await);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie)
}

async fn put(app: &Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    send_request(
        app,
        "PUT",
        "/api/v1/ai/settings",
        None,
        Some(cookie),
        Some(body),
    )
    .await
}

async fn get(app: &Router, cookie: &str) -> Value {
    let (status, body) =
        send_request(app, "GET", "/api/v1/ai/settings", None, Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

/// An OpenAI-style server like Ollama's; returns its base URL and the
/// Authorization headers it received.
async fn mock_llm() -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let recorder = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let recorder = recorder.clone();
            async move {
                recorder.lock().unwrap().push(
                    headers
                        .get("authorization")
                        .map(|v| v.to_str().unwrap().to_string()),
                );
                Json(json!({
                    "model": body["model"],
                    "choices": [{"message": {"content": "OK"}}],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/v1/"), seen)
}

#[tokio::test]
async fn openai_compatible_without_key_saves_and_tests() {
    let (app, cookie) = admin().await;
    let (base, seen) = mock_llm().await;

    let (status, body) = put(
        &app,
        &cookie,
        json!({"provider": "byok_openai_compatible", "base_url": base, "default_model": "llama3.2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let settings = get(&app, &cookie).await;
    assert_eq!(settings["provider"], "byok_openai_compatible");
    assert_eq!(settings["base_url"], base);
    assert_eq!(settings["default_model"], "llama3.2");
    assert_eq!(settings["api_key_set"], false);

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/ai/settings/test",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["provider"], "openai_compatible");
    assert_eq!(body["model"], "llama3.2");
    assert_eq!(seen.lock().unwrap().as_slice(), &[None]);
}

#[tokio::test]
async fn openai_compatible_key_is_sent_as_bearer_and_never_returned() {
    let (app, cookie) = admin().await;
    let (base, seen) = mock_llm().await;
    let key = "sk-or-v1-abcdefghijklmnop-WXYZ";

    let (status, _) = put(
        &app,
        &cookie,
        json!({"provider": "byok_openai_compatible", "base_url": base, "default_model": "m", "api_key": key}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let settings = get(&app, &cookie).await;
    assert_eq!(settings["api_key_set"], true);
    assert!(!settings.to_string().contains("abcdefghijklmnop"));

    let (_, body) = send_request(
        &app,
        "POST",
        "/api/v1/ai/settings/test",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(
        seen.lock().unwrap()[0].as_deref(),
        Some(format!("Bearer {key}").as_str())
    );
}

#[tokio::test]
async fn openai_compatible_needs_a_valid_base_url_and_a_model() {
    let (app, cookie) = admin().await;

    for (body, why) in [
        (
            json!({"provider": "byok_openai_compatible", "default_model": "m"}),
            "no base URL",
        ),
        (
            json!({"provider": "byok_openai_compatible", "base_url": "http://localhost:11434/v1"}),
            "no model",
        ),
        (
            json!({"provider": "byok_openai_compatible", "base_url": "ftp://host/v1", "default_model": "m"}),
            "not http",
        ),
        (
            json!({"provider": "byok_openai_compatible", "base_url": "localhost:11434", "default_model": "m"}),
            "no scheme",
        ),
        (json!({"provider": "something_else"}), "unknown provider"),
    ] {
        let (status, _) = put(&app, &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}");
    }
    // Nothing was written by the rejected requests.
    let settings = get(&app, &cookie).await;
    assert_ne!(settings["provider"], "byok_openai_compatible");
    assert!(settings["base_url"].is_null());
}

#[tokio::test]
async fn switching_provider_without_a_new_key_removes_the_old_key() {
    let (app, cookie) = admin().await;
    let (status, _) = put(
        &app,
        &cookie,
        json!({"provider": "byok_openai", "api_key": "sk-openai-abcdefghijklmnop"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(get(&app, &cookie).await["api_key_set"], true);

    // Updating the model alone keeps the key.
    put(&app, &cookie, json!({"default_model": "gpt-4o"})).await;
    assert_eq!(get(&app, &cookie).await["api_key_set"], true);

    // The OpenAI key must not go to a custom base URL.
    let (status, _) = put(
        &app,
        &cookie,
        json!({"provider": "byok_openai_compatible", "base_url": "http://localhost:11434/v1", "default_model": "llama3.2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(get(&app, &cookie).await["api_key_set"], false);

    // Explicit removal, and clearing the base URL.
    put(
        &app,
        &cookie,
        json!({"api_key": "sk-local-abcdefghijklmnop"}),
    )
    .await;
    assert_eq!(get(&app, &cookie).await["api_key_set"], true);
    put(&app, &cookie, json!({"clear_api_key": true})).await;
    assert_eq!(get(&app, &cookie).await["api_key_set"], false);
    put(
        &app,
        &cookie,
        json!({"provider": "byok_anthropic", "base_url": ""}),
    )
    .await;
    assert!(get(&app, &cookie).await["base_url"].is_null());
}

#[tokio::test]
async fn test_endpoint_reports_missing_settings_as_a_failed_test() {
    let (app, cookie) = admin().await;
    put(&app, &cookie, json!({"provider": "byok_openai"})).await;
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/ai/settings/test",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], false);
    assert!(
        body["error"].as_str().unwrap().contains("API key"),
        "{body}"
    );
}
