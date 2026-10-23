use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

const PASSWORD_MASK: &str = "********";

// ── GET /api/v1/settings/smtp ──────────────────────────────────────────

pub async fn get_smtp_config(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let cfg = state.smtp_config.read().await;
    match cfg.as_ref() {
        Some(c) => Ok(Json(json!({
            "configured": true,
            "host": c.host,
            "port": c.port,
            "username": c.username,
            "password": PASSWORD_MASK,
            "from_address": c.from_address,
            "from_name": c.from_name,
            "encryption": c.encryption,
        }))),
        None => Ok(Json(json!({"configured": false}))),
    }
}

// ── PUT /api/v1/settings/smtp ──────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SmtpEncryptionInput {
    Starttls,
    Implicittls,
    None,
}

impl From<SmtpEncryptionInput> for crate::models::notification_types::SmtpEncryption {
    fn from(v: SmtpEncryptionInput) -> Self {
        match v {
            SmtpEncryptionInput::Starttls => Self::StartTls,
            SmtpEncryptionInput::Implicittls => Self::ImplicitTls,
            SmtpEncryptionInput::None => Self::None,
        }
    }
}

#[derive(Deserialize)]
pub struct SmtpUpdateRequest {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    username: String,
    password: Option<String>,
    from_address: Option<String>,
    from_name: Option<String>,
    encryption: Option<SmtpEncryptionInput>,
}

fn default_port() -> u16 {
    587
}

pub async fn update_smtp_config(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<SmtpUpdateRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.host.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("SMTP host must not be empty")));
    }

    let host = req.host.trim().to_string();
    let username = req.username.trim().to_string();

    // Preserve existing password if client sent the mask placeholder
    let password = match &req.password {
        Some(p) if p == PASSWORD_MASK => {
            let cfg = state.smtp_config.read().await;
            cfg.as_ref().map(|c| c.password.clone()).unwrap_or_default()
        }
        Some(p) => p.trim().to_string(),
        None => {
            let cfg = state.smtp_config.read().await;
            cfg.as_ref().map(|c| c.password.clone()).unwrap_or_default()
        }
    };

    use crate::models::notification_types::{SmtpConfig, SmtpEncryption};

    let encryption = req
        .encryption
        .map(SmtpEncryption::from)
        .unwrap_or(SmtpEncryption::StartTls);

    let new_config = SmtpConfig {
        host,
        port: req.port,
        username,
        password,
        from_address: req
            .from_address
            .unwrap_or_else(|| "voltrus@example.com".into()),
        from_name: req.from_name.unwrap_or_else(|| "Voltrus".into()),
        encryption,
    };

    // Persist to config.yaml
    if let Err(e) = persist_smtp_to_yaml(&state.config_path, &new_config) {
        tracing::warn!("Failed to persist SMTP config to file: {}", e);
    }

    // Update in-memory
    {
        let mut cfg = state.smtp_config.write().await;
        *cfg = Some(new_config);
    }

    // Audit log
    state.audit_logger.log(crate::security::AuditEvent {
        action: "smtp.update".to_string(),
        username: auth.username.clone(),
        resource: "smtp".to_string(),
        details: Some("Updated SMTP configuration".to_string()),
    });

    Ok(Json(json!({"ok": true})))
}

fn persist_smtp_to_yaml(
    path: &str,
    smtp: &crate::models::notification_types::SmtpConfig,
) -> Result<(), anyhow::Error> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: serde_yaml_neo::Value = if content.trim().is_empty() {
        serde_yaml_neo::Value::Mapping(Default::default())
    } else {
        serde_yaml_neo::from_str(&content)
            .unwrap_or(serde_yaml_neo::Value::Mapping(Default::default()))
    };

    if let serde_yaml_neo::Value::Mapping(ref mut map) = doc {
        let smtp_yaml = serde_yaml_neo::to_value(smtp)?;
        map.insert(serde_yaml_neo::Value::String("smtp".into()), smtp_yaml);
    }

    let yaml = serde_yaml_neo::to_string(&doc)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

// ── POST /api/v1/settings/smtp/test ────────────────────────────────────

#[derive(Deserialize)]
pub struct TestSmtpRequest {
    recipient: String,
}

pub async fn test_smtp_connection(
    State(state): State<AppState>,
    Json(req): Json<TestSmtpRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.recipient.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Recipient email is required")));
    }

    let cfg = state.smtp_config.read().await;
    let smtp_cfg = cfg
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("SMTP is not configured"))?;

    tracing::info!(
        "Testing SMTP connection to {}:{} (encryption={:?})",
        smtp_cfg.host,
        smtp_cfg.port,
        smtp_cfg.encryption,
    );
    let notifier = crate::notification::email::EmailNotifier::new(smtp_cfg)?;
    let alarm_data = crate::notification::email::AlarmEmailData {
        severity: "info".to_string(),
        source_key: "system.smtp-test".to_string(),
        source_name: "SMTP Test".to_string(),
        value: 0.0,
        threshold: 0.0,
        message: "This is a test email from Voltrus SMTP configuration.".to_string(),
        unit: "".to_string(),
        triggered_at: crate::notification::webhook::format_triggered_at(crate::timeutil::now_secs()),
    };

    notifier
        .send_alarm_email(&req.recipient, &alarm_data)
        .await?;

    Ok(Json(json!({
        "ok": true,
        "message": format!("Test email sent to {}", req.recipient)
    })))
}
