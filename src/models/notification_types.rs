#![allow(dead_code)]
use serde::{Deserialize, Serialize};

// ── SMTP notification config ──

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpConfig {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default = "default_smtp_from_address")]
    pub from_address: String,
    #[serde(default = "default_smtp_from_name")]
    pub from_name: String,
    #[serde(default = "default_smtp_encryption")]
    pub encryption: SmtpEncryption,
}

fn default_smtp_port() -> u16 {
    587
}

fn default_smtp_from_address() -> String {
    "voltrus@example.com".to_string()
}

fn default_smtp_from_name() -> String {
    "Voltrus".to_string()
}

fn default_smtp_encryption() -> SmtpEncryption {
    SmtpEncryption::StartTls
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SmtpEncryption {
    StartTls,
    ImplicitTls,
    None,
}
