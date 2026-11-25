//! The phone app: pairing, device tokens, and alarm pushes to paired phones.

use super::*;
use axum::body::Body;
use http::Request;
use http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

async fn bearer(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn add_user(state: &AppState, name: &str, role: &str) {
    let pw = crate::api::auth::hash_password("pw1");
    state
        .storage
        .create_user(name, &pw, "", role)
        .await
        .unwrap();
}

/// Sign in as `username` and trade the session for a device token.
async fn pair_by_sign_in(app: &Router, username: &str, password: &str) -> String {
    let cookie = format!(
        "voltrus_token={}",
        login_cookie(app, username, password).await
    );
    let (status, body) = send_request(
        app,
        "POST",
        "/api/v1/mobile/devices",
        None,
        Some(&cookie),
        Some(json!({"name": format!("{username}'s phone"), "platform": "android", "push_token": "ExponentPushToken[abc]"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_qr_code_pairs_one_phone_once() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, pairing) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/pairings",
        None,
        Some(&cookie),
        Some(json!({"origin": "http://192.168.1.20:3000"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pairing}");
    assert_eq!(
        pairing["payload"]["urls"],
        json!(["http://192.168.1.20:3000"])
    );
    // Community: no relay, no push.
    assert_eq!(pairing["push_available"], false);
    assert!(pairing["payload"].get("relay").is_none());
    assert!(
        pairing["link"]
            .as_str()
            .unwrap()
            .starts_with("voltrus-scada://pair?d=")
    );
    let code = pairing["payload"]["code"].as_str().unwrap();

    let pair = json!({"code": code, "name": "Pixel", "platform": "android"});
    let (status, paired) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/pair",
        None,
        None,
        Some(pair.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{paired}");
    assert_eq!(paired["username"], "admin");
    assert_eq!(paired["device"]["push"], false);
    let token = paired["token"].as_str().unwrap();
    assert!(token.starts_with("vdt_"));

    let (status, again) =
        send_request(&app, "POST", "/api/v1/mobile/pair", None, None, Some(pair)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{again}");
    assert_eq!(again["code"], "pairing_invalid");

    let (status, summary) = bearer(&app, "GET", "/api/v1/mobile/summary", token, None).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["user"]["username"], "admin");
    assert_eq!(summary["user"]["can_acknowledge"], true);
    assert_eq!(summary["push"]["licensed"], false);
    assert_eq!(summary["site"], "Voltrus SCADA");
    assert_eq!(summary["relay"], serde_json::Value::Null);
    assert!(summary["urls"].is_array());

    let (status, _) = bearer(&app, "GET", "/api/v1/alarms?status=active", token, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_phone_acts_as_its_user_and_stops_when_removed_or_disabled() {
    let state = seeded_admin_app_state().await;
    add_user(&state, "ops", "operator").await;
    add_user(&state, "watch", "viewer").await;
    let storage = state.storage.clone();
    let app = create_router(state);

    let ops = pair_by_sign_in(&app, "ops", "pw1").await;
    let watch = pair_by_sign_in(&app, "watch", "pw1").await;
    let alarm = storage
        .create_alarm("tag1", "critical", 95.0, 90.0, "High", 0.0)
        .await
        .unwrap();
    let ack = format!("/api/v1/alarms/{alarm}/acknowledge");

    let (status, _) = bearer(&app, "POST", &ack, &watch, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = bearer(&app, "POST", &ack, &ops, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored = storage.get_alarm(alarm).await.unwrap().unwrap();
    assert_eq!(stored.state, "acknowledged");
    assert_eq!(stored.acknowledged_by.as_deref(), Some("ops"));

    // The phone sets its own push filter.
    let (status, device) = bearer(
        &app,
        "PATCH",
        "/api/v1/mobile/device",
        &ops,
        Some(json!({"min_priority": "critical", "push_token": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{device}");
    assert_eq!(device["min_priority"], "critical");
    assert_eq!(device["push"], false);
    let (status, _) = bearer(
        &app,
        "PATCH",
        "/api/v1/mobile/device",
        &ops,
        Some(json!({"min_priority": "urgent"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Disabled user: the phone is refused.
    let id = storage
        .get_user_by_username("ops")
        .await
        .unwrap()
        .unwrap()
        .id;
    storage.set_user_disabled(id, true).await.unwrap();
    let (status, body) = bearer(&app, "GET", "/api/v1/mobile/summary", &ops, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "device_revoked");
    storage.set_user_disabled(id, false).await.unwrap();
    let (status, _) = bearer(&app, "GET", "/api/v1/mobile/summary", &ops, None).await;
    assert_eq!(status, StatusCode::OK);

    // Another user cannot remove the phone; its owner can, and it stops at once.
    let ops_device = storage.list_mobile_devices(Some("ops")).await.unwrap()[0].id;
    let watch_cookie = format!("voltrus_token={}", login_cookie(&app, "watch", "pw1").await);
    let (status, _) = send_request(
        &app,
        "DELETE",
        &format!("/api/v1/mobile/devices/{ops_device}"),
        None,
        Some(&watch_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let ops_cookie = format!("voltrus_token={}", login_cookie(&app, "ops", "pw1").await);
    let (status, _) = send_request(
        &app,
        "DELETE",
        &format!("/api/v1/mobile/devices/{ops_device}"),
        None,
        Some(&ops_cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = bearer(&app, "GET", "/api/v1/mobile/summary", &ops, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A phone removes itself.
    let (status, _) = bearer(&app, "DELETE", "/api/v1/mobile/device", &watch, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = bearer(&app, "GET", "/api/v1/mobile/device", &watch, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_a_signed_in_person_pairs_a_phone() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let (_, key) = storage
        .create_api_key("k", "admin", None, "[]", "integration")
        .await
        .unwrap();
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/pairings",
        Some(&key),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let token = pair_by_sign_in(&app, "admin", "admin").await;
    let (status, _) = bearer(
        &app,
        "POST",
        "/api/v1/mobile/pairings",
        &token,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = bearer(
        &app,
        "POST",
        "/api/v1/mobile/devices",
        &token,
        Some(json!({"platform": "ios"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = bearer(
        &app,
        "GET",
        "/api/v1/mobile/summary",
        "vdt_0000000000000000000000000000000000000000000000000000000000000000",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_site_takes_at_most_fifty_phones() {
    let state = seeded_admin_app_state().await;
    for i in 0..crate::storage::MOBILE_DEVICE_LIMIT {
        state
            .storage
            .create_mobile_device(crate::storage::NewMobileDevice {
                username: "admin".into(),
                name: format!("p{i}"),
                platform: "android".into(),
                min_priority: "warning".into(),
                lang: "id".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .unwrap();
    }
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/devices",
        None,
        Some(&cookie),
        Some(json!({"platform": "android"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "device_limit");
}

#[tokio::test]
async fn mobile_settings_are_configuration() {
    let state = seeded_admin_app_state().await;
    add_user(&state, "ops", "operator").await;
    let app = create_router(state);
    let ops = format!("voltrus_token={}", login_cookie(&app, "ops", "pw1").await);
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let settings = json!({
        "site_name": " Pabrik A ",
        "push_details": false,
        "pinned_tags": ["tag1"],
        "public_urls": ["http://10.8.0.5:3000/"],
    });

    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/mobile/settings",
        None,
        Some(&ops),
        Some(settings.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, saved) = send_request(
        &app,
        "PUT",
        "/api/v1/mobile/settings",
        None,
        Some(&admin),
        Some(settings),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["site_name"], "Pabrik A");
    assert_eq!(saved["public_urls"], json!(["http://10.8.0.5:3000"]));

    let (status, bad) = send_request(
        &app,
        "PUT",
        "/api/v1/mobile/settings",
        None,
        Some(&admin),
        Some(json!({"public_urls": ["file:///etc/passwd"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    // The QR code carries the configured address and name.
    let (_, pairing) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/pairings",
        None,
        Some(&admin),
        Some(json!({"origin": "http://localhost:3000"})),
    )
    .await;
    assert_eq!(pairing["payload"]["site"], "Pabrik A");
    assert_eq!(pairing["payload"]["urls"], json!(["http://10.8.0.5:3000"]));
}

#[tokio::test]
async fn push_targets_follow_priority_and_user_state() {
    let state = seeded_admin_app_state().await;
    add_user(&state, "ops", "operator").await;
    let s = &state.storage;
    let add = |user: &str, min: &str, token: Option<&str>| crate::storage::NewMobileDevice {
        username: user.into(),
        platform: "android".into(),
        min_priority: min.into(),
        lang: "id".into(),
        push_token: token.map(str::to_string),
        ..Default::default()
    };
    let crit_only = s
        .create_mobile_device(add("admin", "critical", Some("t1")))
        .await
        .unwrap()
        .unwrap()
        .0
        .id;
    let all = s
        .create_mobile_device(add("ops", "info", Some("t2")))
        .await
        .unwrap()
        .unwrap()
        .0
        .id;
    s.create_mobile_device(add("ops", "info", None))
        .await
        .unwrap()
        .unwrap();

    let ids = |t: Vec<crate::edition::MobilePushTarget>| {
        let mut v: Vec<i64> = t.into_iter().map(|t| t.device_id).collect();
        v.sort();
        v
    };
    let mut both = vec![crit_only, all];
    both.sort();
    assert_eq!(
        ids(crate::mobile::push_targets(s, Some("critical"))
            .await
            .unwrap()),
        both
    );
    assert_eq!(
        ids(crate::mobile::push_targets(s, Some("warning"))
            .await
            .unwrap()),
        [all]
    );

    let ops = s.get_user_by_username("ops").await.unwrap().unwrap().id;
    s.set_user_disabled(ops, true).await.unwrap();
    assert_eq!(
        ids(crate::mobile::push_targets(s, Some("critical"))
            .await
            .unwrap()),
        [crit_only]
    );
}

/// Captures what the dispatcher hands to the relay.
struct FakeRelay(std::sync::Mutex<Vec<crate::edition::MobileAlarmPush>>);

impl crate::edition::MobilePushSender for FakeRelay {
    fn relay(&self) -> Option<crate::edition::MobileRelay> {
        Some(crate::edition::MobileRelay {
            url: "https://push.example.test".into(),
            site_id: "site-1".into(),
            connected: true,
        })
    }

    fn send<'a>(
        &'a self,
        push: &'a crate::edition::MobileAlarmPush,
    ) -> crate::edition::BoxFuture<'a, anyhow::Result<Vec<crate::edition::MobilePushDelivery>>>
    {
        self.0.lock().unwrap().push(push.clone());
        let deliveries = push
            .targets
            .iter()
            .map(|t| crate::edition::MobilePushDelivery {
                device_id: t.device_id,
                error: None,
                unregistered: t.push_token == "gone",
            })
            .collect();
        Box::pin(async move { Ok(deliveries) })
    }
}

static FAKE_RELAY: std::sync::OnceLock<std::sync::Arc<FakeRelay>> = std::sync::OnceLock::new();

#[tokio::test]
async fn new_alarms_and_acks_are_pushed_on_an_integrator_licence() {
    let relay = FAKE_RELAY
        .get_or_init(|| std::sync::Arc::new(FakeRelay(Default::default())))
        .clone();
    crate::edition::register_mobile_push_sender(relay.clone());

    let state = seeded_admin_app_state().await;
    *state.license_status.write().unwrap() = crate::license::LicenseStatus::Valid {
        license_key: "k".into(),
        expires_at: None,
        days_remaining: None,
        tier: Some("integrator".into()),
        grace: false,
        legacy_white_label: false,
        site: None,
        support_until: None,
    };
    let s = state.storage.clone();
    // A token no other test uses, so pushes from parallel tests are told apart.
    let device = s
        .create_mobile_device(crate::storage::NewMobileDevice {
            username: "admin".into(),
            platform: "ios".into(),
            min_priority: "warning".into(),
            lang: "en".into(),
            push_token: Some("ExponentPushToken[dispatch-test]".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .unwrap()
        .0;
    let gone = s
        .create_mobile_device(crate::storage::NewMobileDevice {
            username: "admin".into(),
            platform: "android".into(),
            min_priority: "info".into(),
            lang: "id".into(),
            push_token: Some("gone".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .unwrap()
        .0;
    crate::mobile::spawn_dispatcher(
        s.clone(),
        state.ws_manager.clone(),
        state.license_status.clone(),
    );
    let app = create_router(state.clone());

    // The QR code now carries the relay.
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (_, pairing) = send_request(
        &app,
        "POST",
        "/api/v1/mobile/pairings",
        None,
        Some(&cookie),
        Some(json!({})),
    )
    .await;
    assert_eq!(pairing["push_available"], true);
    assert_eq!(pairing["payload"]["relay"]["site_id"], "site-1");

    let alarm = s
        .create_alarm("tag1", "critical", 95.0, 90.0, "Pressure high", 0.0)
        .await
        .unwrap();
    state
        .ws_manager
        .broadcast_alarm(crate::ws_types::ServerMessage::Alarm {
            alarm_id: alarm,
            tag_id: "tag1".into(),
            severity: "critical".into(),
            message: "Pressure high".into(),
            value: 95.0,
            ts: 0,
        });
    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/alarms/{alarm}/acknowledge"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let mine = || -> Vec<crate::edition::MobileAlarmPush> {
        relay
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|p| {
                p.targets
                    .iter()
                    .any(|t| t.device_id == device.id && t.push_token.ends_with("[dispatch-test]"))
            })
            .cloned()
            .collect()
    };
    for _ in 0..100 {
        if mine().len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let pushes = mine();
    assert_eq!(pushes.len(), 2, "{pushes:?}");
    assert_eq!(pushes[0].event, "raised");
    assert_eq!(pushes[0].alarm_id, alarm);
    assert_eq!(pushes[0].priority, "critical");
    assert_eq!(pushes[0].message, "Pressure high");
    assert_eq!(pushes[0].site_name, "Voltrus SCADA");
    assert!(pushes[0].details);
    assert_eq!(pushes[1].event, "acknowledged");
    assert_eq!(pushes[1].actor.as_deref(), Some("admin"));

    // The token the push service no longer knows is forgotten.
    let gone = s.get_mobile_device(gone.id).await.unwrap().unwrap();
    assert!(!gone.push);
}
