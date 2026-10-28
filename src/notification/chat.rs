//! Config of the chat notification channels (Telegram, WhatsApp).
//!
//! Delivery belongs to the add-on edition (see [`crate::edition::ChatSender`]);
//! the shape of the stored config, its validation and the masking of its
//! secrets live here so the REST API, the project file export and the add-on
//! agree on one definition.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Shown in place of a stored secret in API responses. Sending it back on a
/// `PUT` (or a test) keeps the stored secret.
pub const SECRET_MASK: &str = "********";

/// Alarm fields a WhatsApp template parameter can be bound to, in the order
/// the template's `{{1}}`, `{{2}}`... placeholders take them.
pub const TEMPLATE_FIELDS: [&str; 5] = ["severity", "source", "value", "message", "time"];

/// Most placeholders a template can bind (Meta allows more; alarms have five fields).
const MAX_TEMPLATE_PARAMETERS: usize = 10;

static BOT_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{3,}:[A-Za-z0-9_-]{20,}$").expect("bot token regex"));
static CHAT_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(-?\d{1,20}|@[A-Za-z][A-Za-z0-9_]{4,31})$").expect("chat id regex")
});
static E164: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+[1-9]\d{6,14}$").expect("e164 regex"));
static TEMPLATE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_]{1,512}$").expect("template name regex"));
static LANGUAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]{2,3}(_[A-Z]{2})?$").expect("language regex"));
static PHONE_NUMBER_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{5,32}$").expect("phone number id regex"));

/// Config of a `telegram` rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub bot_token: String,
    /// Numeric chat ids (users, groups, channels) or `@channelname`.
    pub chat_ids: Vec<String>,
}

/// Config of a `whatsapp` rule (WhatsApp Business Cloud API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhatsAppConfig {
    /// Permanent access token of a system user.
    pub access_token: String,
    /// The business phone number's id (not the number itself).
    pub phone_number_id: String,
    /// Recipients in E.164 (`+62812...`).
    pub recipients: Vec<String>,
    /// Name of an approved message template.
    pub template_name: String,
    /// Template language code (`en_US`, `id`).
    pub language: String,
    /// Alarm field for each template placeholder, in order (see [`TEMPLATE_FIELDS`]).
    #[serde(default)]
    pub parameters: Vec<String>,
}

/// Whether `channel` is one of the chat channels.
pub fn is_chat_channel(channel: &str) -> bool {
    matches!(channel, "telegram" | "whatsapp")
}

/// The secret-bearing field of a chat channel's config.
fn secret_field(channel: &str) -> Option<&'static str> {
    match channel {
        "telegram" => Some("bot_token"),
        "whatsapp" => Some("access_token"),
        _ => None,
    }
}

pub fn parse_telegram(config: &str) -> Result<TelegramConfig, String> {
    serde_json::from_str(config).map_err(|e| {
        format!("Telegram config must be a JSON object with 'bot_token' and 'chat_ids': {e}")
    })
}

pub fn parse_whatsapp(config: &str) -> Result<WhatsAppConfig, String> {
    serde_json::from_str(config).map_err(|e| {
        format!(
            "WhatsApp config must be a JSON object with 'access_token', 'phone_number_id', 'recipients', 'template_name', 'language' and 'parameters': {e}"
        )
    })
}

fn check_secret(label: &str, secret: &str) -> Result<(), String> {
    if secret.is_empty() || secret == SECRET_MASK {
        return Err(format!("{label} is required"));
    }
    Ok(())
}

pub fn validate_telegram(config: &TelegramConfig) -> Result<(), String> {
    check_secret("Telegram bot token", &config.bot_token)?;
    if !BOT_TOKEN.is_match(&config.bot_token) {
        return Err(
            "Telegram bot token must look like 123456789:AA... (copy it from @BotFather)".into(),
        );
    }
    if config.chat_ids.is_empty() {
        return Err("Add at least one Telegram chat id".into());
    }
    if let Some(bad) = config.chat_ids.iter().find(|c| !CHAT_ID.is_match(c)) {
        return Err(format!(
            "Not a Telegram chat id: '{bad}' (use a number such as 123456789 or -1001234567890, or @channelname)"
        ));
    }
    Ok(())
}

pub fn validate_whatsapp(config: &WhatsAppConfig) -> Result<(), String> {
    check_secret("WhatsApp access token", &config.access_token)?;
    if !PHONE_NUMBER_ID.is_match(&config.phone_number_id) {
        return Err(
            "WhatsApp phone number id must be the numeric id from the Meta app dashboard".into(),
        );
    }
    if config.recipients.is_empty() {
        return Err("Add at least one WhatsApp recipient".into());
    }
    if let Some(bad) = config.recipients.iter().find(|r| !E164.is_match(r)) {
        return Err(format!(
            "Not an E.164 phone number: '{bad}' (use +, country code and number, for example +6281234567890)"
        ));
    }
    if !TEMPLATE_NAME.is_match(&config.template_name) {
        return Err(
            "Template name must be lowercase letters, digits and underscores, as approved in Meta Business Manager".into(),
        );
    }
    if !LANGUAGE.is_match(&config.language) {
        return Err("Template language must be a code such as 'en_US' or 'id'".into());
    }
    if config.parameters.len() > MAX_TEMPLATE_PARAMETERS {
        return Err(format!(
            "A template can bind at most {MAX_TEMPLATE_PARAMETERS} parameters"
        ));
    }
    if let Some(bad) = config
        .parameters
        .iter()
        .find(|p| !TEMPLATE_FIELDS.contains(&p.as_str()))
    {
        return Err(format!(
            "Unknown template parameter '{bad}' (choose from {})",
            TEMPLATE_FIELDS.join(", ")
        ));
    }
    Ok(())
}

/// Parse and validate the config of a chat channel.
pub fn validate(channel: &str, config: &str) -> Result<(), String> {
    match channel {
        "telegram" => validate_telegram(&parse_telegram(config)?),
        "whatsapp" => validate_whatsapp(&parse_whatsapp(config)?),
        other => Err(format!("'{other}' is not a chat channel")),
    }
}

/// `config` with its secret replaced by [`SECRET_MASK`]. Configs of other
/// channels, and configs that do not parse, come back unchanged (a config that
/// does not parse never held a usable secret).
pub fn mask_config(channel: &str, config: &str) -> String {
    let Some(field) = secret_field(channel) else {
        return config.to_string();
    };
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str(config) else {
        return config.to_string();
    };
    match map.get(field) {
        Some(serde_json::Value::String(s)) if !s.is_empty() => {
            map.insert(field.into(), SECRET_MASK.into());
            serde_json::Value::Object(map).to_string()
        }
        _ => config.to_string(),
    }
}

/// Put the stored secret back into `config` where the client sent the mask.
/// `stored` is the config of the rule being edited, `None` when there is none.
pub fn restore_secret(channel: &str, config: &str, stored: Option<&str>) -> Result<String, String> {
    let Some(field) = secret_field(channel) else {
        return Ok(config.to_string());
    };
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str(config) else {
        return Ok(config.to_string());
    };
    if map.get(field).and_then(|v| v.as_str()) != Some(SECRET_MASK) {
        return Ok(config.to_string());
    }
    let stored_secret = stored
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get(field).and_then(|f| f.as_str().map(str::to_string)))
        .filter(|s| !s.is_empty());
    match stored_secret {
        Some(secret) => {
            map.insert(field.into(), secret.into());
            Ok(serde_json::Value::Object(map).to_string())
        }
        None => Err(format!(
            "'{field}' is masked but no stored value exists; enter it again"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "123456789:AAEhBP0av28mG8Wv9oQxkXq1Zt2yVbcD3_4";

    fn telegram(chat_ids: &[&str]) -> String {
        serde_json::json!({"bot_token": TOKEN, "chat_ids": chat_ids}).to_string()
    }

    fn whatsapp(patch: serde_json::Value) -> String {
        let mut base = serde_json::json!({
            "access_token": "EAAGm0PX4ZCpsBA",
            "phone_number_id": "106540352242922",
            "recipients": ["+6281234567890"],
            "template_name": "alarm_notice",
            "language": "id",
            "parameters": ["severity", "source", "value", "message", "time"],
        });
        for (k, v) in patch.as_object().unwrap() {
            base[k] = v.clone();
        }
        base.to_string()
    }

    #[test]
    fn telegram_accepts_numeric_group_and_channel_ids() {
        assert!(
            validate(
                "telegram",
                &telegram(&["123456789", "-1001234567890", "@plant_ops"])
            )
            .is_ok()
        );
    }

    #[test]
    fn telegram_rejects_bad_tokens_and_ids() {
        assert!(validate("telegram", &telegram(&[])).is_err());
        assert!(validate("telegram", &telegram(&["not an id"])).is_err());
        let bad_token = serde_json::json!({"bot_token": "abc", "chat_ids": ["1"]}).to_string();
        assert!(validate("telegram", &bad_token).is_err());
        assert!(validate("telegram", "[\"123\"]").is_err());
        let masked = serde_json::json!({"bot_token": SECRET_MASK, "chat_ids": ["1"]}).to_string();
        assert!(validate("telegram", &masked).is_err());
    }

    #[test]
    fn whatsapp_requires_e164_template_and_known_parameters() {
        assert!(validate("whatsapp", &whatsapp(serde_json::json!({}))).is_ok());
        assert!(
            validate(
                "whatsapp",
                &whatsapp(serde_json::json!({"recipients": ["081234567890"]}))
            )
            .is_err()
        );
        assert!(validate("whatsapp", &whatsapp(serde_json::json!({"recipients": []}))).is_err());
        assert!(
            validate(
                "whatsapp",
                &whatsapp(serde_json::json!({"template_name": "Alarm Notice"}))
            )
            .is_err()
        );
        assert!(
            validate(
                "whatsapp",
                &whatsapp(serde_json::json!({"language": "indonesian"}))
            )
            .is_err()
        );
        assert!(
            validate(
                "whatsapp",
                &whatsapp(serde_json::json!({"parameters": ["severity", "colour"]}))
            )
            .is_err()
        );
        assert!(validate("whatsapp", &whatsapp(serde_json::json!({"parameters": []}))).is_ok());
    }

    #[test]
    fn masking_hides_only_the_secret_and_restoring_round_trips() {
        let stored = telegram(&["42"]);
        let masked = mask_config("telegram", &stored);
        assert!(!masked.contains(TOKEN));
        assert!(masked.contains(SECRET_MASK));
        assert!(masked.contains("\"42\""));
        // A client that sends the masked config back keeps the stored token.
        let restored = restore_secret("telegram", &masked, Some(&stored)).unwrap();
        assert_eq!(parse_telegram(&restored).unwrap().bot_token, TOKEN);
        // A client that sends a new token replaces it.
        let replaced = serde_json::json!({"bot_token": "999999999:BBBBBBBBBBBBBBBBBBBBBBBB", "chat_ids": ["42"]}).to_string();
        assert_eq!(
            restore_secret("telegram", &replaced, Some(&stored)).unwrap(),
            replaced
        );
        // The mask with nothing stored is an error, never stored as the token.
        assert!(restore_secret("telegram", &masked, None).is_err());
    }

    #[test]
    fn masking_leaves_other_channels_untouched() {
        assert_eq!(mask_config("email", "[\"a@b.c\"]"), "[\"a@b.c\"]");
        let wa = whatsapp(serde_json::json!({}));
        let masked = mask_config("whatsapp", &wa);
        assert!(!masked.contains("EAAGm0PX4ZCpsBA"));
        assert!(masked.contains("106540352242922"));
    }
}
