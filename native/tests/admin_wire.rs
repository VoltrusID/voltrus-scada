//! Wire guards for the Data Manager's Admin group, through the console's
//! own serializers and parsers in `api::admin`:
//!
//! - every request body the console builds decodes into a mirror of the
//!   server's request struct (`models::api_types`, `models::rbac_types`,
//!   `api::api_keys`, `models::alarm_types`) that refuses unknown fields,
//!   so a misnamed field (which the server would silently drop) fails;
//! - every answer is produced by serializing a mirror of the server's
//!   model with its serde attributes (`models::User`'s skipped secrets),
//!   or the handler's own `json!` shape, and decoded by the console's type;
//! - the value lists the console offers (user roles, permissions, key
//!   roles) are read from the server's source and must match it.

// The mirrors keep every field the server's types have, read here or not.
#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use voltrus_scada_native::api::admin::{
    ApiKeyList, KEY_ROLES, NewApiKey, NotificationRule, PERMISSIONS, RbacRole, TestOutcome,
    USER_ROLES, User, api_key_body, new_user_body, parse_targets, role_body, rule_body, test_body,
    user_update_body,
};

const SERVER_RBAC: &str = include_str!("../../src/models/rbac_types.rs");
const SERVER_USERS: &str = include_str!("../../src/api/auth/users.rs");
const SERVER_API_KEYS: &str = include_str!("../../src/api/api_keys.rs");

// ── Mirrors of the server's types ────────────────────────────────────────

/// `models::User`, with the server's serde attributes.
#[derive(Serialize)]
struct ServerUser {
    id: i64,
    username: String,
    #[serde(skip_serializing)]
    password_hash: String,
    #[serde(skip_serializing)]
    salt: String,
    role: String,
    must_change_password: bool,
    #[serde(skip_serializing)]
    failed_attempts: i64,
    #[serde(skip_serializing)]
    locked_until: Option<i64>,
}

/// `models::api_types::CreateUserRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: String,
}

/// `models::api_types::UpdateUserRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateUserRequest {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// `models::rbac_types::Role`.
#[derive(Serialize)]
struct ServerRole {
    id: i64,
    name: String,
    permissions: String,
    created_at: i64,
}

/// `models::rbac_types::CreateRoleRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRoleRequest {
    name: String,
    permissions: String,
}

/// `models::rbac_types::UpdateRoleRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRoleRequest {
    name: Option<String>,
    permissions: Option<String>,
}

/// `storage::api_keys::ApiKeyInfo`.
#[derive(Serialize)]
struct ApiKeyInfo {
    id: i64,
    name: String,
    key_prefix: String,
    created_at: i64,
    created_by: String,
    last_used_at: Option<i64>,
    expires_at: Option<i64>,
    permissions: String,
    role: String,
}

/// `api::api_keys::CreateApiKeyRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateApiKeyRequest {
    name: String,
    #[serde(default)]
    expires_at: Option<i64>,
    permissions: String,
    role: String,
}

/// `models::alarm_types::NotificationSetting`.
#[derive(Serialize)]
struct NotificationSetting {
    id: i64,
    source_key: String,
    channel_type: String,
    config: String,
    enabled: bool,
    created_at: i64,
}

/// `models::alarm_types::CreateNotificationSettingRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateNotificationSettingRequest {
    source_key: String,
    channel_type: String,
    config: String,
    enabled: bool,
}

/// `models::alarm_types::UpdateNotificationSettingRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateNotificationSettingRequest {
    source_key: Option<String>,
    channel_type: Option<String>,
    config: Option<String>,
    enabled: Option<bool>,
}

/// `models::alarm_types::TestNotificationRequest`.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestNotificationRequest {
    channel_type: String,
    config: String,
    source_key: Option<String>,
    setting_id: Option<i64>,
}

// ── The server's value lists, read from its source ───────────────────────

/// The double-quoted strings in `text`.
fn quoted(text: &str) -> Vec<&str> {
    text.split('"').skip(1).step_by(2).collect()
}

/// `models::rbac_types::ALL_PERMISSIONS`, its `PERM_*` names resolved.
fn server_permissions() -> Vec<String> {
    let values: HashMap<&str, &str> = SERVER_RBAC
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("pub const PERM_")?;
            let (name, value) = rest.split_once(": &str = ")?;
            Some((name, value.trim().trim_end_matches(';').trim_matches('"')))
        })
        .collect();
    let start = SERVER_RBAC
        .find("pub const ALL_PERMISSIONS")
        .expect("the server lists ALL_PERMISSIONS");
    let list = &SERVER_RBAC[start..];
    let open = list.find("= &[").expect("ALL_PERMISSIONS is a slice") + 4;
    let close = list.find("];").expect("ALL_PERMISSIONS ends");
    list[open..close]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let name = name.strip_prefix("PERM_").unwrap_or(name);
            values
                .get(name)
                .unwrap_or_else(|| panic!("PERM_{name} has a value"))
                .to_string()
        })
        .collect()
}

/// The roles `create_user_handler` and `update_user_handler` accept.
fn server_user_roles() -> BTreeSet<&'static str> {
    SERVER_USERS
        .split("role != \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect()
}

/// `api::api_keys::KEY_ROLES`.
fn server_key_roles() -> Vec<&'static str> {
    let line = SERVER_API_KEYS
        .lines()
        .find(|line| line.contains("const KEY_ROLES"))
        .expect("the server lists KEY_ROLES");
    quoted(line)
}

// ── Users ────────────────────────────────────────────────────────────────

#[test]
fn users_decode_without_secrets() {
    let rows = [
        ServerUser {
            id: 1,
            username: "admin".into(),
            password_hash: "$argon2id$hash".into(),
            salt: "salt".into(),
            role: "admin".into(),
            must_change_password: false,
            failed_attempts: 0,
            locked_until: None,
        },
        ServerUser {
            id: 7,
            username: "ana.shiftlead".into(),
            password_hash: "$argon2id$other".into(),
            salt: "pepper".into(),
            role: "demo".into(),
            must_change_password: true,
            failed_attempts: 2,
            locked_until: Some(1_700_000_000),
        },
    ];
    let wire = serde_json::to_value(&rows).unwrap();
    for row in wire.as_array().unwrap() {
        let keys: BTreeSet<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["id", "username", "role", "must_change_password"]),
            "the secrets never reach the wire"
        );
    }
    let users: Vec<User> = serde_json::from_value(wire).unwrap();
    assert_eq!(
        users[1],
        User {
            id: 7,
            username: "ana.shiftlead".into(),
            role: "demo".into(),
            must_change_password: true,
        }
    );
    assert!(users[0].is_admin() && !users[1].is_admin());
}

#[test]
fn user_bodies_are_what_the_server_reads() {
    let body = new_user_body("  ana  ", "secret-pw", "secret-pw", "operator", &[]).unwrap();
    let request: CreateUserRequest = serde_json::from_value(body).unwrap();
    assert_eq!(
        request,
        CreateUserRequest {
            username: "ana".into(),
            password: "secret-pw".into(),
            role: "operator".into(),
        }
    );
    let both: UpdateUserRequest =
        serde_json::from_value(user_update_body(Some("demo"), Some("new-secret")).unwrap())
            .unwrap();
    assert_eq!(
        both,
        UpdateUserRequest {
            role: Some("demo".into()),
            password: Some("new-secret".into()),
        }
    );
    let password_only: Value = user_update_body(None, Some("new-secret")).unwrap();
    assert!(
        password_only.get("role").is_none(),
        "an unchanged role is left out, not sent as null"
    );
    let password_only: UpdateUserRequest = serde_json::from_value(password_only).unwrap();
    assert_eq!(password_only.role, None);
    assert!(user_update_body(None, None).is_none());
}

#[test]
fn user_roles_are_the_ones_the_server_accepts() {
    let ours: BTreeSet<&str> = USER_ROLES.iter().map(|role| role.id).collect();
    assert_eq!(ours, server_user_roles());
}

// ── Roles ────────────────────────────────────────────────────────────────

#[test]
fn permissions_are_the_servers_in_its_order() {
    assert_eq!(
        PERMISSIONS
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        server_permissions()
    );
}

#[test]
fn role_bodies_round_trip_their_permissions() {
    let chosen = ["alarms:ack".to_owned(), "screens:read".to_owned()];
    let body = role_body(" Shift lead ", &chosen).unwrap();
    let created: CreateRoleRequest = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(created.name, "Shift lead");
    let updated: UpdateRoleRequest = serde_json::from_value(body).unwrap();
    assert_eq!(
        updated.permissions.as_deref(),
        Some(created.permissions.as_str())
    );
    // The permissions string is a JSON array, in the server's order.
    let stored: Vec<String> = serde_json::from_str(&created.permissions).unwrap();
    assert_eq!(stored, ["screens:read", "alarms:ack"]);

    // Stored and listed back, the console reads the same permissions.
    let listed: Vec<RbacRole> = serde_json::from_value(
        serde_json::to_value([ServerRole {
            id: 5,
            name: created.name.clone(),
            permissions: created.permissions.clone(),
            created_at: 1_700_000_000,
        }])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(listed[0].name, "Shift lead");
    assert_eq!(listed[0].created_at, 1_700_000_000);
    assert_eq!(listed[0].permission_list(), stored);
    assert!(role_body("Ops", &["users:everything".to_owned()]).is_err());
}

// ── API keys ─────────────────────────────────────────────────────────────

#[test]
fn key_roles_are_the_ones_the_server_accepts() {
    assert_eq!(
        KEY_ROLES.iter().map(|role| role.id).collect::<Vec<_>>(),
        server_key_roles()
    );
}

#[test]
fn api_key_bodies_are_what_the_server_reads() {
    let permissions = ["screens:read".to_owned()];
    let body = api_key_body(" Script ", "integration", Some(30), 100, &permissions).unwrap();
    let request: CreateApiKeyRequest = serde_json::from_value(body).unwrap();
    assert_eq!(
        request,
        CreateApiKeyRequest {
            name: "Script".into(),
            expires_at: Some(100 + 30 * 86_400),
            permissions: r#"["screens:read"]"#.into(),
            role: "integration".into(),
        },
        "expires_at in seconds, as validate_api_key compares"
    );
    let forever: CreateApiKeyRequest =
        serde_json::from_value(api_key_body("Grafana", "api", None, 100, &[]).unwrap()).unwrap();
    assert_eq!(forever.expires_at, None);
    assert_eq!(forever.permissions, "[]");
}

#[test]
fn api_keys_list_prefixes_and_create_shows_the_key_once() {
    let keys = [
        ApiKeyInfo {
            id: 3,
            name: "Grafana read-only".into(),
            key_prefix: "a1b2c3d4".into(),
            created_at: 1_700_000_100,
            created_by: "admin".into(),
            last_used_at: None,
            expires_at: Some(1_702_592_100),
            permissions: "[]".into(),
            role: "api".into(),
        },
        ApiKeyInfo {
            id: 2,
            name: "Historian".into(),
            key_prefix: "ffee0011".into(),
            created_at: 1_700_000_000,
            created_by: "admin".into(),
            last_used_at: Some(1_700_000_500),
            expires_at: None,
            permissions: "[]".into(),
            role: "integration".into(),
        },
    ];
    // `list_api_keys`: `Json(json!({ "keys": keys }))`.
    let list: ApiKeyList = serde_json::from_value(json!({ "keys": keys })).unwrap();
    assert_eq!(list.keys.len(), 2);
    assert_eq!(list.keys[0].key_prefix, "a1b2c3d4");
    assert_eq!(list.keys[0].created_by, "admin");
    assert_eq!(list.keys[1].last_used_at, Some(1_700_000_500));
    assert_eq!(list.keys[1].role, "integration");
    assert!(list.keys[0].expired(1_702_592_101));
    assert!(!list.keys[1].expired(i64::MAX));

    // `create_api_key`'s answer, built from what the console sent.
    let sent: CreateApiKeyRequest =
        serde_json::from_value(api_key_body("Script", "api", None, 0, &[]).unwrap()).unwrap();
    let plain = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let created: NewApiKey = serde_json::from_value(json!({
        "ok": true,
        "id": 4,
        "name": sent.name,
        "role": sent.role,
        "key": plain,
        "warning": "Store this key securely. It will not be shown again.",
        "note": "Permissions are stored for future use. All valid API keys currently grant full read access."
    }))
    .unwrap();
    assert_eq!((created.id, created.name.as_str()), (4, "Script"));
    assert_eq!(created.key, plain);
    assert!(
        !format!("{created:?}").contains("0123"),
        "Debug hides the key"
    );
}

// ── Notification rules ───────────────────────────────────────────────────

#[test]
fn rule_bodies_are_what_the_server_reads() {
    let text = " https://a.example/x \n\nhttps://b.example/y";
    let body = rule_body(" ac_power ", "webhook", text, false).unwrap();
    let created: CreateNotificationSettingRequest = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(created.source_key, "ac_power");
    assert_eq!(created.channel_type, "webhook");
    assert!(!created.enabled, "a switched-off rule is sent as such");
    let targets: Vec<String> = serde_json::from_str(&created.config).unwrap();
    assert_eq!(targets, parse_targets(text));
    let updated: UpdateNotificationSettingRequest = serde_json::from_value(body).unwrap();
    assert_eq!(updated.config.as_deref(), Some(created.config.as_str()));
    assert_eq!(updated.enabled, Some(false));

    // Stored and listed back, the console reads the same targets.
    let rules: Vec<NotificationRule> = serde_json::from_value(
        serde_json::to_value([
            NotificationSetting {
                id: 2,
                source_key: created.source_key.clone(),
                channel_type: created.channel_type.clone(),
                config: created.config.clone(),
                enabled: created.enabled,
                created_at: 1_700_000_001,
            },
            NotificationSetting {
                id: 1,
                source_key: String::new(),
                channel_type: "email".into(),
                config: r#"["control-room@solar.example"]"#.into(),
                enabled: true,
                created_at: 1_700_000_000,
            },
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(rules[0].targets(), targets);
    assert!(!rules[0].enabled);
    assert_eq!(rules[1].targets(), ["control-room@solar.example"]);

    // The test body for each, as `TestNotificationRequest` reads it.
    let tested: TestNotificationRequest = serde_json::from_value(test_body(&rules[0])).unwrap();
    assert_eq!(
        tested,
        TestNotificationRequest {
            channel_type: "webhook".into(),
            config: created.config.clone(),
            source_key: Some("ac_power".into()),
            setting_id: Some(rules[0].id),
        }
    );
    let every_tag: TestNotificationRequest = serde_json::from_value(test_body(&rules[1])).unwrap();
    assert_eq!(every_tag.source_key, None, "every tag is no source key");
}

#[test]
fn test_outcomes_decode_every_channel() {
    // `test_notification`'s `json!` shapes, per channel.
    let webhook: TestOutcome = serde_json::from_value(json!({
        "channel": "webhook",
        "results": [
            {"url": "http://127.0.0.1:9/voltrus", "status": "error", "error": "error sending request"},
            {"url": "https://hooks.example.com", "status": "sent"}
        ]
    }))
    .unwrap();
    assert_eq!(webhook.sent(), 1);
    assert_eq!(webhook.results[0].target(), "http://127.0.0.1:9/voltrus");
    assert_eq!(
        webhook.results[0].error.as_deref(),
        Some("error sending request")
    );
    let email: TestOutcome = serde_json::from_value(json!({
        "channel": "email",
        "results": [{"to": "ops@example.com", "status": "sent"}]
    }))
    .unwrap();
    assert_eq!(email.results[0].target(), "ops@example.com");
    let whatsapp: TestOutcome = serde_json::from_value(json!({
        "channel": "whatsapp",
        "results": [{"target": "+6281234567890", "status": "error", "error": "WhatsApp error 190"}]
    }))
    .unwrap();
    assert_eq!(whatsapp.results[0].target(), "+6281234567890");
    assert_eq!(whatsapp.sent(), 0);
}
