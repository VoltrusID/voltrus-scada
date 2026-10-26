//! The Data Manager's Admin group against the server: users
//! (`src/api/auth/users.rs`), RBAC roles (`src/api/rbac.rs`,
//! `models::rbac_types`), API keys (`src/api/api_keys.rs`,
//! `storage::api_keys::ApiKeyInfo`) and notification rules
//! (`src/api/notifications.rs`, `models::NotificationSetting`).
//!
//! Typed mirrors of the answers, the request bodies the console sends
//! (checked here the way the server checks them, so a form says what is
//! wrong before it asks), and the rules that keep an admin from locking
//! themselves out. Users, roles and keys are strictly admin routes;
//! notification writes are operator routes, their list any user's.

use std::fmt;

use serde::Deserialize;
use serde_json::{Value, json};

// ── Users ────────────────────────────────────────────────────────────────

/// A row of `GET /api/v1/users` (`models::User`; the hash, salt and lock
/// fields are never serialized).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
}

/// A role the server stores on a user: the only values
/// `create_user_handler` and `update_user_handler` accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserRole {
    pub id: &'static str,
    pub label: &'static str,
    /// What the server lets it do (`auth_middleware.rs`).
    pub reach: &'static str,
}

/// Least to most access. Only `admin` and `demo` pass the server's
/// operator routes; `operator`, `viewer` and `user` read and acknowledge.
pub const USER_ROLES: [UserRole; 5] = [
    UserRole {
        id: "viewer",
        label: "Viewer",
        reach: "Reads and acknowledges alarms",
    },
    UserRole {
        id: "user",
        label: "User",
        reach: "Reads and acknowledges alarms",
    },
    UserRole {
        id: "operator",
        label: "Operator",
        reach: "Reads and acknowledges alarms; the server's operator routes need demo or admin",
    },
    UserRole {
        id: "demo",
        label: "Demo",
        reach: "Configures and operates the plant; not users, roles or API keys",
    },
    UserRole {
        id: "admin",
        label: "Admin",
        reach: "Everything, including users, roles and API keys",
    },
];

pub fn user_role(id: &str) -> Option<UserRole> {
    USER_ROLES.iter().copied().find(|role| role.id == id)
}

/// The console asks for at least this many characters, as the server's own
/// password change does; its admin routes take any non-empty password.
pub const MIN_PASSWORD: usize = 8;

/// A password typed twice: the error to show, if any.
pub fn check_password(password: &str, again: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!(
            "The password needs at least {MIN_PASSWORD} characters"
        ));
    }
    if password != again {
        return Err("The two passwords differ".to_owned());
    }
    Ok(())
}

/// `POST /api/v1/users` (`CreateUserRequest`).
pub fn new_user_body(
    username: &str,
    password: &str,
    again: &str,
    role: &str,
    users: &[User],
) -> Result<Value, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("A user needs a name".to_owned());
    }
    if username.chars().any(char::is_whitespace) {
        return Err("A user name has no spaces".to_owned());
    }
    if users.iter().any(|user| user.username == username) {
        return Err(format!("{username} already exists"));
    }
    if user_role(role).is_none() {
        return Err(format!("The server has no role “{role}”"));
    }
    check_password(password, again)?;
    Ok(json!({ "username": username, "password": password, "role": role }))
}

/// Why `me` may not move `user` to `role`, if they may not: an admin who
/// demotes themselves, or the last admin, loses the Admin pages for good.
pub fn role_change_refused(me: &str, user: &User, role: &str, users: &[User]) -> Option<String> {
    if role == user.role {
        return None;
    }
    if user_role(role).is_none() {
        return Some(format!("The server has no role “{role}”"));
    }
    if !user.is_admin() {
        return None;
    }
    if user.username == me {
        return Some("You cannot take away your own admin role".to_owned());
    }
    let admins = users.iter().filter(|other| other.is_admin()).count();
    (admins <= 1).then(|| format!("{} is the last admin", user.username))
}

/// `PUT /api/v1/users/{id}` (`UpdateUserRequest`): the role when it
/// changed, the new password when one was typed; `None` when nothing did.
pub fn user_update_body(role: Option<&str>, password: Option<&str>) -> Option<Value> {
    let mut body = serde_json::Map::new();
    if let Some(role) = role {
        body.insert("role".into(), json!(role));
    }
    if let Some(password) = password {
        body.insert("password".into(), json!(password));
    }
    (!body.is_empty()).then_some(Value::Object(body))
}

/// Why `me` may not delete `user`, if they may not (the server refuses
/// "cannot delete yourself"; the last admin is the console's own rule).
pub fn delete_refused(me: &str, user: &User, users: &[User]) -> Option<String> {
    if user.username == me {
        return Some("You cannot delete yourself".to_owned());
    }
    let admins = users.iter().filter(|other| other.is_admin()).count();
    (user.is_admin() && admins <= 1).then(|| format!("{} is the last admin", user.username))
}

// ── Roles ────────────────────────────────────────────────────────────────

/// A row of `GET /api/v1/roles` (`models::Role`): `permissions` is a JSON
/// array in a string, `["screens:read", …]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RbacRole {
    pub id: i64,
    pub name: String,
    pub permissions: String,
    pub created_at: i64,
}

impl RbacRole {
    /// The role's permissions; none when the string is not a JSON array.
    pub fn permission_list(&self) -> Vec<String> {
        parse_string_list(&self.permissions)
    }
}

/// Every permission the server accepts on a role (`models::ALL_PERMISSIONS`,
/// in its order); `create_role` and `update_role` refuse any other.
pub const PERMISSIONS: [&str; 24] = [
    "screens:read",
    "screens:write",
    "devices:read",
    "devices:write",
    "alarms:read",
    "alarms:ack",
    "settings:read",
    "settings:write",
    "users:manage",
    "flows:manage",
    "notifications:manage",
    "hub:manage",
    "reports:manage",
    "recipes:manage",
    "commands:write",
    "commands:read",
    "commands:cancel",
    "tags:write_policy",
    "sim:read",
    "sim:write",
    "scripts:read",
    "scripts:write",
    "config:read",
    "config:write",
];

/// The permissions grouped by what they are about (`screens`, `devices`,
/// …), groups in the order the server lists them.
pub fn permission_groups() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut groups: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for permission in PERMISSIONS {
        let area = permission.split(':').next().unwrap_or(permission);
        match groups.iter_mut().find(|(name, _)| *name == area) {
            Some((_, list)) => list.push(permission),
            None => groups.push((area, vec![permission])),
        }
    }
    groups
}

/// `POST /api/v1/roles` and `PUT /api/v1/roles/{id}`: the name and the
/// permissions as the JSON-array string the server stores, in its order.
pub fn role_body(name: &str, permissions: &[String]) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A role needs a name".to_owned());
    }
    if let Some(unknown) = permissions
        .iter()
        .find(|permission| !PERMISSIONS.contains(&permission.as_str()))
    {
        return Err(format!("The server has no permission “{unknown}”"));
    }
    let ordered: Vec<&str> = PERMISSIONS
        .iter()
        .copied()
        .filter(|permission| permissions.iter().any(|chosen| chosen == permission))
        .collect();
    let permissions = serde_json::to_string(&ordered).map_err(|error| error.to_string())?;
    Ok(json!({ "name": name, "permissions": permissions }))
}

// ── API keys ─────────────────────────────────────────────────────────────

/// A row of `GET /api/v1/api-keys` (`{"keys": [ApiKeyInfo]}`): never the
/// key itself, only its first eight characters. Times are Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ApiKey {
    pub id: i64,
    pub name: String,
    pub key_prefix: String,
    pub created_at: i64,
    pub created_by: String,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub permissions: String,
    pub role: String,
}

impl ApiKey {
    pub fn expired(&self, now_secs: i64) -> bool {
        self.expires_at.is_some_and(|at| at < now_secs)
    }
}

/// `GET /api/v1/api-keys`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ApiKeyList {
    pub keys: Vec<ApiKey>,
}

/// `POST /api/v1/api-keys`: the new key in plain text, shown once. Its
/// `Debug` leaves the key out, so it never reaches a log.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NewApiKey {
    pub id: i64,
    pub name: String,
    pub role: String,
    pub key: String,
    pub warning: String,
    pub note: String,
}

impl fmt::Debug for NewApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewApiKey")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("role", &self.role)
            .field("key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// A role a key may hold (`api_keys::KEY_ROLES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRole {
    pub id: &'static str,
    pub label: &'static str,
    pub reach: &'static str,
}

pub const KEY_ROLES: [KeyRole; 2] = [
    KeyRole {
        id: "api",
        label: "Read only",
        reach: "Reads data, history and alarms",
    },
    KeyRole {
        id: "integration",
        label: "Integration",
        reach: "Also passes the operator routes: writes, screens, devices",
    },
];

/// How long a new key lives.
pub const KEY_LIFETIMES: [(&str, Option<i64>); 4] = [
    ("Never expires", None),
    ("30 days", Some(30)),
    ("90 days", Some(90)),
    ("1 year", Some(365)),
];

/// `POST /api/v1/api-keys` (`CreateApiKeyRequest`): `expires_at` in Unix
/// seconds, as `validate_api_key` compares it.
pub fn api_key_body(
    name: &str,
    role: &str,
    lifetime_days: Option<i64>,
    now_secs: i64,
    permissions: &[String],
) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A key needs a name".to_owned());
    }
    if !KEY_ROLES.iter().any(|key_role| key_role.id == role) {
        return Err(format!("A key cannot hold the role “{role}”"));
    }
    let permissions = serde_json::to_string(permissions).map_err(|error| error.to_string())?;
    let mut body = json!({ "name": name, "role": role, "permissions": permissions });
    if let Some(days) = lifetime_days {
        body["expires_at"] = json!(now_secs + days * 86_400);
    }
    Ok(body)
}

// ── Notification rules ───────────────────────────────────────────────────

/// A row of `GET /api/v1/notifications/settings` (`NotificationSetting`):
/// `config` is a JSON array in a string (addresses, URLs or user names;
/// objects for the edition's SMS). An empty `source_key` means every tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NotificationRule {
    pub id: i64,
    pub source_key: String,
    pub channel_type: String,
    pub config: String,
    pub enabled: bool,
    pub created_at: i64,
}

impl NotificationRule {
    /// The targets, one per entry; an SMS target reads `phone (carrier)`.
    pub fn targets(&self) -> Vec<String> {
        let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&self.config) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| match entry {
                Value::String(text) => Some(text.clone()),
                Value::Object(map) => {
                    let phone = map.get("phone").and_then(Value::as_str).unwrap_or("?");
                    Some(match map.get("carrier").and_then(Value::as_str) {
                        Some(carrier) => format!("{phone} ({carrier})"),
                        None => phone.to_owned(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// One line for the table: the target, or how many.
    pub fn targets_summary(&self) -> String {
        let targets = self.targets();
        match targets.len() {
            0 => "none".to_owned(),
            1 => targets[0].clone(),
            count => format!("{} and {} more", targets[0], count - 1),
        }
    }

    /// Whether the console can edit its targets: the core channels, whose
    /// config is a list of strings.
    pub fn editable(&self) -> bool {
        channel(&self.channel_type).is_some()
    }
}

/// A channel the console edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    pub id: &'static str,
    pub label: &'static str,
    /// What one line of the targets is.
    pub target: &'static str,
    pub placeholder: &'static str,
    /// `POST /notifications/test` takes it (push is tested from a browser).
    pub testable: bool,
}

/// The core's channels, as the web client offers them; SMS and other
/// edition channels are listed but edited in the web client.
pub const CHANNELS: [Channel; 3] = [
    Channel {
        id: "email",
        label: "Email",
        target: "one address per line",
        placeholder: "ops@example.com\nshift-lead@example.com",
        testable: true,
    },
    Channel {
        id: "webhook",
        label: "Webhook",
        target: "one URL per line",
        placeholder: "https://hooks.example.com/voltrus",
        testable: true,
    },
    Channel {
        id: "push",
        label: "Push",
        target: "one user name per line",
        placeholder: "admin\noperator",
        testable: false,
    },
];

pub fn channel(id: &str) -> Option<Channel> {
    CHANNELS.iter().copied().find(|channel| channel.id == id)
}

/// The targets typed one per line, blank lines and spaces around dropped.
pub fn parse_targets(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The targets checked as `create_notification_setting` checks them.
pub fn check_targets(channel: &str, targets: &[String]) -> Result<(), String> {
    if targets.is_empty() {
        return Err("Add at least one target".to_owned());
    }
    for target in targets {
        match channel {
            "email" if !target.contains('@') || !target.contains('.') => {
                return Err(format!("Not an email address: {target}"));
            }
            "webhook" if !target.starts_with("http://") && !target.starts_with("https://") => {
                return Err(format!(
                    "A webhook URL starts with http:// or https://: {target}"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// `POST /api/v1/notifications/settings`, and the same fields for `PUT
/// /api/v1/notifications/settings/{id}`.
pub fn rule_body(
    source_key: &str,
    channel_id: &str,
    targets_text: &str,
    enabled: bool,
) -> Result<Value, String> {
    if channel(channel_id).is_none() {
        return Err(format!("The console cannot edit “{channel_id}” rules"));
    }
    let targets = parse_targets(targets_text);
    check_targets(channel_id, &targets)?;
    let config = serde_json::to_string(&targets).map_err(|error| error.to_string())?;
    Ok(json!({
        "source_key": source_key.trim(),
        "channel_type": channel_id,
        "config": config,
        "enabled": enabled,
    }))
}

/// `POST /api/v1/notifications/test` (`TestNotificationRequest`) for a
/// rule as stored.
pub fn test_body(rule: &NotificationRule) -> Value {
    json!({
        "channel_type": rule.channel_type,
        "config": rule.config,
        "source_key": if rule.source_key.is_empty() { None } else { Some(&rule.source_key) },
    })
}

/// One target's outcome in `POST /api/v1/notifications/test`'s answer:
/// `{"to"|"url"|"phone": …, "status": "sent"|"error", "error"?}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TestResult {
    pub to: Option<String>,
    pub url: Option<String>,
    pub phone: Option<String>,
    pub carrier: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

impl TestResult {
    pub fn target(&self) -> String {
        match (&self.to, &self.url, &self.phone) {
            (Some(to), _, _) => to.clone(),
            (_, Some(url), _) => url.clone(),
            (_, _, Some(phone)) => match &self.carrier {
                Some(carrier) => format!("{phone} ({carrier})"),
                None => phone.clone(),
            },
            _ => "?".to_owned(),
        }
    }

    pub fn sent(&self) -> bool {
        self.status == "sent"
    }
}

/// `POST /api/v1/notifications/test`: `{channel, results}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TestOutcome {
    pub channel: String,
    pub results: Vec<TestResult>,
}

impl TestOutcome {
    pub fn sent(&self) -> usize {
        self.results.iter().filter(|result| result.sent()).count()
    }
}

/// A JSON array of strings in a string; empty when it is not one.
fn parse_string_list(text: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(text)
        .map(|list| {
            list.into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: i64, name: &str, role: &str) -> User {
        User {
            id,
            username: name.into(),
            role: role.into(),
            must_change_password: false,
        }
    }

    #[test]
    fn a_new_user_needs_a_free_name_a_known_role_and_a_confirmed_password() {
        let users = [user(1, "admin", "admin")];
        assert_eq!(
            new_user_body("  ana ", "secret-12", "secret-12", "demo", &users).unwrap(),
            json!({ "username": "ana", "password": "secret-12", "role": "demo" })
        );
        assert!(new_user_body("", "secret-12", "secret-12", "demo", &users).is_err());
        assert!(new_user_body("a b", "secret-12", "secret-12", "demo", &users).is_err());
        assert!(new_user_body("admin", "secret-12", "secret-12", "demo", &users).is_err());
        assert!(new_user_body("ana", "secret-12", "secret-12", "integration", &users).is_err());
        assert!(new_user_body("ana", "short", "short", "demo", &users).is_err());
        assert!(new_user_body("ana", "secret-12", "secret-13", "demo", &users).is_err());
    }

    #[test]
    fn an_admin_cannot_demote_or_delete_themselves_or_the_last_admin() {
        let users = [
            user(1, "admin", "admin"),
            user(2, "ana", "demo"),
            user(3, "bo", "admin"),
        ];
        assert!(role_change_refused("admin", &users[0], "viewer", &users).is_some());
        assert!(role_change_refused("admin", &users[0], "admin", &users).is_none());
        assert!(role_change_refused("admin", &users[2], "viewer", &users).is_none());
        assert!(role_change_refused("admin", &users[1], "admin", &users).is_none());
        assert!(role_change_refused("admin", &users[1], "root", &users).is_some());
        let alone = [user(1, "admin", "admin"), user(3, "bo", "admin")];
        assert!(role_change_refused("bo", &alone[0], "viewer", &alone[..1]).is_some());

        assert!(delete_refused("admin", &users[0], &users).is_some());
        assert!(delete_refused("admin", &users[1], &users).is_none());
        assert!(delete_refused("admin", &users[2], &users).is_none());
        assert!(delete_refused("ana", &alone[0], &alone[..1]).is_some());
    }

    #[test]
    fn a_user_update_sends_only_what_changed() {
        assert_eq!(user_update_body(None, None), None);
        assert_eq!(
            user_update_body(Some("viewer"), None),
            Some(json!({ "role": "viewer" }))
        );
        assert_eq!(
            user_update_body(None, Some("new-pass-1")),
            Some(json!({ "password": "new-pass-1" }))
        );
        assert!(check_password("1234567", "1234567").is_err());
        assert!(check_password("12345678", "12345678").is_ok());
    }

    #[test]
    fn roles_store_known_permissions_in_the_servers_order() {
        let body = role_body(
            " Shift lead ",
            &["alarms:ack".into(), "screens:read".into()],
        )
        .unwrap();
        assert_eq!(body["name"], "Shift lead");
        assert_eq!(body["permissions"], r#"["screens:read","alarms:ack"]"#);
        assert_eq!(role_body("x", &[]).unwrap()["permissions"], "[]");
        assert!(role_body(" ", &[]).is_err());
        assert!(
            role_body("x", &["flows:read".into()]).is_err(),
            "the web offers it; the server refuses it"
        );
        let groups = permission_groups();
        assert_eq!(
            groups[0],
            ("screens", vec!["screens:read", "screens:write"])
        );
        assert_eq!(
            groups.iter().map(|(_, list)| list.len()).sum::<usize>(),
            PERMISSIONS.len()
        );
        let role = RbacRole {
            permissions: r#"["alarms:ack", 3]"#.into(),
            ..RbacRole::default()
        };
        assert_eq!(role.permission_list(), ["alarms:ack"]);
        let broken = RbacRole {
            permissions: "nope".into(),
            ..RbacRole::default()
        };
        assert!(broken.permission_list().is_empty());
    }

    #[test]
    fn a_key_expires_in_seconds_and_holds_only_a_key_role() {
        let body = api_key_body("Grafana", "api", Some(30), 1_000, &[]).unwrap();
        assert_eq!(body["expires_at"], 1_000 + 30 * 86_400);
        assert_eq!(body["permissions"], "[]");
        assert_eq!(body["role"], "api");
        let forever = api_key_body("Grafana", "integration", None, 1_000, &[]).unwrap();
        assert!(forever.get("expires_at").is_none());
        assert!(api_key_body("", "api", None, 0, &[]).is_err());
        assert!(api_key_body("x", "admin", None, 0, &[]).is_err());
        let key = ApiKey {
            expires_at: Some(10),
            ..ApiKey::default()
        };
        assert!(key.expired(11) && !key.expired(10));
    }

    #[test]
    fn a_new_keys_debug_never_shows_the_key() {
        let created = NewApiKey {
            key: "f00dfeedcafe".into(),
            ..NewApiKey::default()
        };
        assert!(!format!("{created:?}").contains("f00d"));
    }

    #[test]
    fn rule_targets_are_checked_as_the_server_checks_them() {
        let body = rule_body(
            " ac_power ",
            "email",
            "ops@example.com\n\n  lead@example.com ",
            true,
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "source_key": "ac_power", "channel_type": "email",
                "config": r#"["ops@example.com","lead@example.com"]"#, "enabled": true,
            })
        );
        assert!(rule_body("", "email", "not-an-address", true).is_err());
        assert!(rule_body("", "webhook", "hooks.example.com", true).is_err());
        assert!(rule_body("", "webhook", "https://hooks.example.com", false).is_ok());
        assert!(rule_body("", "push", "admin", true).is_ok());
        assert!(rule_body("", "push", "  \n ", true).is_err());
        assert!(rule_body("", "sms", "+1555", true).is_err());
    }

    #[test]
    fn rules_summarise_their_targets_and_test_as_stored() {
        let rule = NotificationRule {
            id: 4,
            source_key: String::new(),
            channel_type: "sms".into(),
            config: r#"[{"phone":"+15551234","carrier":"att"},{"phone":"+15559876"}]"#.into(),
            enabled: true,
            created_at: 0,
        };
        assert_eq!(rule.targets(), ["+15551234 (att)", "+15559876"]);
        assert_eq!(rule.targets_summary(), "+15551234 (att) and 1 more");
        assert!(!rule.editable());
        assert_eq!(test_body(&rule)["source_key"], Value::Null);
        let keyed = NotificationRule {
            source_key: "ac_power".into(),
            channel_type: "webhook".into(),
            config: r#"["https://a"]"#.into(),
            ..rule
        };
        assert_eq!(test_body(&keyed)["source_key"], "ac_power");
        assert_eq!(keyed.targets_summary(), "https://a");
        assert!(keyed.editable());
    }
}
