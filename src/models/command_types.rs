#![allow(dead_code)]

// ── Command & Control types (v0.24.0) ──

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandStatus {
    Pending,
    Queued,
    Sent,
    Acked,
    Failed,
    Expired,
    Cancelled,
}

impl CommandStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Acked => "acked",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Acked | Self::Failed | Self::Expired | Self::Cancelled
        )
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "queued" => Some(Self::Queued),
            "sent" => Some(Self::Sent),
            "acked" => Some(Self::Acked),
            "failed" => Some(Self::Failed),
            "expired" => Some(Self::Expired),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Command {
    pub id: String,
    pub data_source_id: i64,
    /// Numeric id of an S7/OPC UA/EtherNet-IP/DNP3/BACnet device; `0` for a
    /// Modbus device, whose id is text (see `device_ref`).
    pub device_id: i64,
    pub protocol: String,
    pub value: String,
    pub status: String,
    pub policy: String,
    pub scheduled_at: Option<i64>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub sent_at: Option<i64>,
    pub acked_at: Option<i64>,
    pub resolved_at: Option<i64>,
    pub created_by: String,
    /// The device's id as the data source names it: a Modbus device's text id,
    /// or the stringified numeric id of the other protocols. Empty on
    /// commands written before this field existed and not yet backfilled.
    #[serde(default)]
    pub device_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readback_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readback_value: Option<f64>,
    #[serde(default)]
    pub readback_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readback_ts: Option<String>,
}

/// The device a data source writes to, as a queued command records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTarget {
    /// Numeric device id; `0` for a Modbus device with a text id.
    pub device_id: i64,
    /// The data source's device id as stored (the Modbus text id, or the
    /// stringified numeric id).
    pub device_ref: String,
    pub protocol: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CommandAuditEntry {
    pub id: i64,
    pub command_id: String,
    pub old_status: String,
    pub new_status: String,
    pub changed_by: String,
    pub reason: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, utoipa::ToSchema)]
pub struct WritePolicy {
    pub writable: bool,
    pub write_min: Option<f64>,
    pub write_max: Option<f64>,
    pub write_max_rate: Option<i64>,
    pub write_requires_reason: bool,
    pub write_deadband: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct WriteRequest {
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
    #[serde(default = "default_write_policy")]
    pub policy: String,
    pub scheduled_at: Option<i64>,
    pub reason: Option<String>,
}

fn default_write_policy() -> String {
    "immediate".to_string()
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CommandQuery {
    pub tag: Option<String>,
    pub status: Option<String>,
    pub device: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Serde helper: deserialize missing JSON field as `None`, JSON `null` as `Some(None)`,
/// and a present value as `Some(Some(value))`. This enables three-state semantics for
/// `Option<Option<T>>` fields (absent / explicitly cleared / explicitly set).
pub(crate) mod option_option {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<T: serde::Serialize, S: Serializer>(
        val: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match val {
            None => s.serialize_none(),
            Some(inner) => s.serialize_some(inner),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Ok(Some(Option::<T>::deserialize(d)?))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct WritePolicyUpdate {
    pub writable: Option<bool>,
    #[serde(default, with = "option_option")]
    pub write_min: Option<Option<f64>>,
    #[serde(default, with = "option_option")]
    pub write_max: Option<Option<f64>>,
    #[serde(default, with = "option_option")]
    pub write_max_rate: Option<Option<i64>>,
    pub write_requires_reason: Option<bool>,
    #[serde(default, with = "option_option")]
    pub write_deadband: Option<Option<f64>>,
}

#[derive(Debug, Clone)]
pub struct WriteResult {
    pub success: bool,
    pub device_response: Option<String>,
    pub error: Option<String>,
    pub latency_ms: u64,
}
