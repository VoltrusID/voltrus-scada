//! Wire guards for the Data Manager's Admin group: each fixture is the shape
//! the server's handler answers with (`src/api/auth/users.rs`,
//! `rbac.rs`, `api_keys.rs`, `notifications.rs`; `models::User`,
//! `models::Role`, `storage::api_keys::ApiKeyInfo`,
//! `models::NotificationSetting`).

use voltrus_scada_native::api::admin::{
    ApiKeyList, NewApiKey, NotificationRule, PERMISSIONS, RbacRole, TestOutcome, User,
    api_key_body, rule_body, test_body,
};

#[test]
fn users_decode_without_secrets() {
    // `list_users`: `Json(Vec<models::User>)`; password_hash, salt,
    // failed_attempts and locked_until are `skip_serializing`.
    let users: Vec<User> = serde_json::from_str(
        r#"[
            {"id": 1, "username": "admin", "role": "admin", "must_change_password": false},
            {"id": 7, "username": "ana.shiftlead", "role": "demo", "must_change_password": true}
        ]"#,
    )
    .unwrap();
    assert_eq!(users[1].username, "ana.shiftlead");
    assert!(users[1].must_change_password);
    assert!(users[0].is_admin() && !users[1].is_admin());
    // `create_user_handler` and `update_user_handler` answer `{"ok", "id"?}`.
    let created: serde_json::Value = serde_json::from_str(r#"{"ok": true, "id": 9}"#).unwrap();
    assert_eq!(created["id"], 9);
}

#[test]
fn roles_carry_their_permissions_as_a_json_string() {
    // `list_roles`: `Json(Vec<models::Role>)`, seeded by `seed_default_roles`.
    let roles: Vec<RbacRole> = serde_json::from_str(
        r#"[
            {"id": 1, "name": "Admin", "permissions": "[\"screens:read\",\"screens:write\"]", "created_at": 1700000000},
            {"id": 4, "name": "Viewer", "permissions": "[\"screens:read\",\"devices:read\",\"alarms:read\",\"settings:read\"]", "created_at": 1700000000}
        ]"#,
    )
    .unwrap();
    assert_eq!(roles[1].permission_list().len(), 4);
    assert!(
        roles
            .iter()
            .flat_map(RbacRole::permission_list)
            .all(|permission| PERMISSIONS.contains(&permission.as_str())),
        "the seeded permissions are all ones the server accepts"
    );
}

#[test]
fn api_keys_list_prefixes_and_create_shows_the_key_once() {
    // `list_api_keys`: `{"keys": [ApiKeyInfo]}`, newest first, no hash.
    let list: ApiKeyList = serde_json::from_str(
        r#"{"keys": [
            {"id": 3, "name": "Grafana read-only", "key_prefix": "a1b2c3d4", "created_at": 1700000100,
             "created_by": "admin", "last_used_at": null, "expires_at": 1702592100,
             "permissions": "[]", "role": "api"},
            {"id": 2, "name": "Historian", "key_prefix": "ffee0011", "created_at": 1700000000,
             "created_by": "admin", "last_used_at": 1700000500, "expires_at": null,
             "permissions": "[]", "role": "integration"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(list.keys.len(), 2);
    assert_eq!(list.keys[0].key_prefix, "a1b2c3d4");
    assert_eq!(list.keys[1].last_used_at, Some(1_700_000_500));
    assert!(list.keys[0].expired(1_702_592_101));
    // `create_api_key`: the plain key with a warning and a note.
    let created: NewApiKey = serde_json::from_str(
        r#"{"ok": true, "id": 4, "name": "Script", "role": "api",
            "key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "warning": "Store this key securely. It will not be shown again.",
            "note": "Permissions are stored for future use. All valid API keys currently grant full read access."}"#,
    )
    .unwrap();
    assert_eq!(created.key.len(), 64);
    assert!(
        !format!("{created:?}").contains("0123"),
        "Debug hides the key"
    );
    let body = api_key_body("Script", "api", Some(1), 100, &[]).unwrap();
    assert_eq!(
        body["expires_at"],
        100 + 86_400,
        "seconds, as validate_api_key compares"
    );
}

#[test]
fn notification_rules_round_trip_their_config_string() {
    // `list_notification_settings`: `Json(Vec<NotificationSetting>)`.
    let rules: Vec<NotificationRule> = serde_json::from_str(
        r#"[
            {"id": 1, "source_key": "", "channel_type": "email",
             "config": "[\"control-room@solar.example\"]", "enabled": true, "created_at": 1700000000},
            {"id": 2, "source_key": "ac_current_l1", "channel_type": "webhook",
             "config": "[\"http://127.0.0.1:9/voltrus\"]", "enabled": false, "created_at": 1700000001}
        ]"#,
    )
    .unwrap();
    assert_eq!(rules[0].targets(), ["control-room@solar.example"]);
    assert!(!rules[1].enabled);
    // What the form sends decodes like a stored rule's config.
    let body = rule_body(
        "ac_power",
        "webhook",
        "https://a.example/x\nhttps://b.example/y",
        true,
    )
    .unwrap();
    let again = NotificationRule {
        config: body["config"].as_str().unwrap().to_owned(),
        ..rules[1].clone()
    };
    assert_eq!(again.targets().len(), 2);
    assert_eq!(test_body(&rules[1])["config"], rules[1].config);
    // `test_notification`: `{"channel", "results": [{"url"|"to", "status", "error"?}]}`.
    let outcome: TestOutcome = serde_json::from_str(
        r#"{"channel": "webhook", "results": [
            {"url": "http://127.0.0.1:9/voltrus", "status": "error", "error": "error sending request"},
            {"url": "https://hooks.example.com", "status": "sent"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(outcome.sent(), 1);
    assert_eq!(outcome.results[0].target(), "http://127.0.0.1:9/voltrus");
    let email: TestOutcome = serde_json::from_str(
        r#"{"channel": "email", "results": [{"to": "ops@example.com", "status": "sent"}]}"#,
    )
    .unwrap();
    assert_eq!(email.results[0].target(), "ops@example.com");
}
