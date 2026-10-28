//! SCADA-17 S20: notification settings validation and the SMTP-less test.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn create(
    app: &Router,
    cookie: &str,
    channel: &str,
    config: &str,
) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "POST",
        "/api/v1/notifications/settings",
        None,
        Some(cookie),
        Some(
            json!({"source_key": "k", "channel_type": channel, "config": config, "enabled": true}),
        ),
    )
    .await
}

async fn update(
    app: &Router,
    cookie: &str,
    id: i64,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "PUT",
        &format!("/api/v1/notifications/settings/{id}"),
        None,
        Some(cookie),
        Some(body),
    )
    .await
}

/// `POST` answered a refused setting with 500 and `PUT` skipped the email and
/// URL shape checks `POST` makes.
#[tokio::test]
async fn test_create_and_update_share_the_setting_checks() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = create(&app, &cookie, "webhook", r#"["ftp://hook"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = create(&app, &cookie, "email", r#"["nobody"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = create(&app, &cookie, "fax", r#"["x"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, hook) = create(&app, &cookie, "webhook", r#"["https://hook.example.com"]"#).await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_i64().unwrap();

    // The same shape checks on update.
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"config": r#"["ftp://hook"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("webhook URL"),
        "{body}"
    );
    let (status, body) = update(&app, &cookie, hook_id, json!({"config": "[]"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Switching the channel checks the stored config against it.
    let (status, body) = update(&app, &cookie, hook_id, json!({"channel_type": "email"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = update(&app, &cookie, hook_id, json!({"channel_type": "fax"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Valid changes, and one that touches neither channel nor config, pass.
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"channel_type": "email", "config": r#"["ops@example.com"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = update(&app, &cookie, hook_id, json!({"enabled": false})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"config": r#"["not-an-address"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = update(&app, &cookie, 9999, json!({"config": r#"["a@b.c"]"#})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // What was refused changed nothing.
    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/notifications/settings",
        None,
        Some(&cookie),
        None,
    )
    .await;
    let row = &list.as_array().unwrap()[0];
    assert_eq!(row["channel_type"], "email");
    assert_eq!(row["config"], r#"["ops@example.com"]"#);
}

/// The test of an email setting without SMTP configured answered 500.
#[tokio::test]
async fn test_notification_test_without_smtp_is_a_client_error() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "email", "config": r#"["ops@example.com"]"#})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "SMTP is not configured");

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "push", "config": "[]"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

const BOT_TOKEN: &str = "123456789:AAEhBP0av28mG8Wv9oQxkXq1Zt2yVbcD3_4";

fn telegram_config() -> String {
    json!({"bot_token": BOT_TOKEN, "chat_ids": ["42"]}).to_string()
}

fn whatsapp_config() -> String {
    json!({
        "access_token": "EAAGsecret",
        "phone_number_id": "106540352242922",
        "recipients": ["+6281234567890"],
        "template_name": "alarm_notice",
        "language": "id",
        "parameters": ["severity", "source", "value", "message", "time"],
    })
    .to_string()
}

async fn integrator_app() -> (Router, crate::storage::Storage) {
    let mut state = seeded_admin_app_state().await;
    state.license_status = std::sync::Arc::new(std::sync::RwLock::new(
        crate::license::LicenseStatus::Valid {
            license_key: "k".to_string(),
            expires_at: None,
            days_remaining: None,
            tier: Some("integrator".to_string()),
            grace: false,
            legacy_white_label: false,
            site: None,
            support_until: None,
        },
    ));
    let storage = state.storage.clone();
    (create_router(state), storage)
}

/// Telegram and WhatsApp need the Integrator tier: a Community install gets
/// 403 on every route that would store or send them.
#[tokio::test]
async fn test_chat_channels_are_gated_to_the_integrator_tier() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    for (channel, config) in [
        ("telegram", telegram_config()),
        ("whatsapp", whatsapp_config()),
    ] {
        let (status, body) = create(&app, &cookie, channel, &config).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let (status, body) = send_request(
            &app,
            "POST",
            "/api/v1/notifications/test",
            None,
            Some(&cookie),
            Some(json!({"channel_type": channel, "config": config})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }

    // Switching an existing rule to a chat channel is gated too.
    let (_, hook) = create(&app, &cookie, "webhook", r#"["https://hook.example.com"]"#).await;
    let (status, body) = update(
        &app,
        &cookie,
        hook["id"].as_i64().unwrap(),
        json!({"channel_type": "telegram", "config": telegram_config()}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn test_chat_configs_are_validated_on_create_and_update() {
    let (app, _) = integrator_app().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = create(&app, &cookie, "telegram", r#"["42"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = create(
        &app,
        &cookie,
        "telegram",
        &json!({"bot_token": BOT_TOKEN, "chat_ids": []}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let bad_phone = whatsapp_config().replace("+6281234567890", "081234567890");
    let (status, body) = create(&app, &cookie, "whatsapp", &bad_phone).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("E.164"), "{body}");

    let (status, created) = create(&app, &cookie, "telegram", &telegram_config()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().unwrap();
    let (status, body) = update(
        &app,
        &cookie,
        id,
        json!({"config": r#"{"chat_ids":["1"]}"#}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// Tokens never come back from the API; sending the mask back keeps the
/// stored token, sending a new one replaces it.
#[tokio::test]
async fn test_chat_secrets_are_masked_and_survive_edits() {
    let (app, storage) = integrator_app().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, created) = create(&app, &cookie, "telegram", &telegram_config()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(!created.to_string().contains(BOT_TOKEN), "{created}");
    let id = created["id"].as_i64().unwrap();
    let (_, wa) = create(&app, &cookie, "whatsapp", &whatsapp_config()).await;
    assert!(!wa.to_string().contains("EAAGsecret"), "{wa}");

    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/notifications/settings",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert!(!list.to_string().contains(BOT_TOKEN), "{list}");
    assert!(!list.to_string().contains("EAAGsecret"), "{list}");
    let masked = list[0]["config"].as_str().unwrap().to_string();
    assert!(masked.contains(crate::notification::chat::SECRET_MASK));

    // Editing the chat ids with the masked token keeps the stored token.
    let edited = masked.replace("\"42\"", "\"42\",\"-100123\"");
    let (status, body) = update(&app, &cookie, id, json!({"config": edited})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = storage.get_all_notification_settings().await.unwrap();
    let cfg = crate::notification::chat::parse_telegram(&stored[0].config).unwrap();
    assert_eq!(cfg.bot_token, BOT_TOKEN);
    assert_eq!(cfg.chat_ids, ["42", "-100123"]);

    // A new token replaces it.
    let new_token = "999999999:BBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    let (status, body) = update(
        &app,
        &cookie,
        id,
        json!({"config": json!({"bot_token": new_token, "chat_ids": ["42"]}).to_string()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = storage.get_all_notification_settings().await.unwrap();
    assert_eq!(
        crate::notification::chat::parse_telegram(&stored[0].config)
            .unwrap()
            .bot_token,
        new_token
    );

    // The mask is never accepted as a token for a new rule.
    let (status, body) = create(&app, &cookie, "telegram", &masked).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

struct EchoChat;

impl crate::edition::ChatSender for EchoChat {
    fn send<'a>(
        &'a self,
        _channel: &'a str,
        config: &'a str,
        _alarm: &'a crate::edition::ChatAlarm,
    ) -> crate::edition::BoxFuture<'a, anyhow::Result<Vec<crate::edition::ChatDelivery>>> {
        Box::pin(async move {
            let token = crate::notification::chat::parse_telegram(config)
                .map(|c| c.bot_token)
                .unwrap_or_default();
            Ok(vec![crate::edition::ChatDelivery {
                target: token,
                error: Some("refused".into()),
            }])
        })
    }
}

/// The editor tests a stored rule with the masked token: the server swaps the
/// stored token back in before sending.
#[tokio::test]
async fn test_chat_test_resolves_masked_secrets_from_the_stored_rule() {
    crate::edition::register_chat_sender(std::sync::Arc::new(EchoChat));
    let (app, _) = integrator_app().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (_, created) = create(&app, &cookie, "telegram", &telegram_config()).await;
    let id = created["id"].as_i64().unwrap();
    let masked = created["config"].as_str().unwrap().to_string();

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "telegram", "config": masked, "setting_id": id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["channel"], "telegram");
    assert_eq!(body["results"][0]["target"], BOT_TOKEN);
    assert_eq!(body["results"][0]["status"], "error");

    // Without a stored rule to resolve it from, the mask is refused.
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "telegram", "config": masked})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
