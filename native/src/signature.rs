//! Electronic signatures (21 CFR Part 11). An Enterprise add-on may require
//! the signed-in user to sign an action (a tag write, an alarm
//! acknowledge, …); plain Core never does. The server then refuses the
//! request with what is to be signed, the console asks for the password and
//! a reason, and sends the same request again with a [`Signature`].
//!
//! REST: `428` with `code: signature_required` asks; `403` with
//! `signature_invalid` asks again (a wrong password); `403` with
//! `account_locked` or `signature_not_possible` is a plain failure. The
//! WebSocket write answers a `CommandAck` with status `signature_required`,
//! or `rejected` with `signature_required` set for a wrong password.
//!
//! Every request of the session goes through one path
//! (`api::VoltrusApi`), which asks for a signature when the server wants
//! one and sends the request again with it ([`Signable::Request`]), as the
//! web client's shared fetch does; tag writes, alarm actions and flow runs
//! ask through their own [`Signable`]s.
//!
//! No UI here: the console's prompt is `views::sign`.

use crate::api::ServerError;
use crate::tag_api::WriteMode;
pub use crate::ws_types::{Signature, SignatureRequired};

/// The server wants the action signed: what to state, and why the last
/// signature was refused, if one was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub request: SignatureRequired,
    pub error: Option<String>,
}

/// Whether the server refused a REST request for want of a (valid)
/// signature. Anything else, a locked account included, is a plain failure.
pub fn ask_of(error: &ServerError) -> Option<Ask> {
    let request = error.signature_required.clone()?;
    match (error.status.as_u16(), error.code.as_deref()) {
        (428, _) | (_, Some("signature_required")) => Some(Ask {
            request,
            error: None,
        }),
        (403, Some("signature_invalid")) => Some(Ask {
            request,
            error: Some(error.message.clone()),
        }),
        _ => None,
    }
}

/// [`ask_of`] for an error from the REST client.
pub fn ask(error: &anyhow::Error) -> Option<Ask> {
    ask_of(error.downcast_ref::<ServerError>()?)
}

/// Whether a WebSocket write's answer asks for a signature.
pub fn ask_of_ack(
    status: &str,
    device_response: Option<&str>,
    required: Option<&SignatureRequired>,
) -> Option<Ask> {
    let request = required?.clone();
    match status {
        "signature_required" => Some(Ask {
            request,
            error: None,
        }),
        "rejected" => Some(Ask {
            request,
            error: Some(
                device_response
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or("The signature was refused")
                    .to_owned(),
            ),
        }),
        _ => None,
    }
}

/// A REST answer, or the server's request for a signature.
#[derive(Debug, Clone, PartialEq)]
pub enum Signed<T> {
    Done(T),
    Ask(Ask),
}

/// Turn a refusal that asks for a signature into [`Signed::Ask`]; other
/// failures stay failures.
pub fn signed<T>(result: anyhow::Result<T>) -> anyhow::Result<Signed<T>> {
    match result {
        Ok(value) => Ok(Signed::Done(value)),
        Err(error) => match ask(&error) {
            Some(ask) => Ok(Signed::Ask(ask)),
            None => Err(error),
        },
    }
}

/// An action that can be sent again with a signature.
#[derive(Debug, Clone, PartialEq)]
pub enum Signable {
    /// A write to the tag with this data source id; `reason` is the
    /// write's own (REST only).
    TagWrite {
        tag: i64,
        value: f64,
        mode: WriteMode,
        reason: String,
    },
    Acknowledge(i64),
    AcknowledgeAll,
    Shelve {
        alarm: i64,
        seconds: u64,
        reason: String,
    },
    /// One manual run of the stored flow `id` (a mode switch), on `input`
    /// if given; `name` is for the prompt.
    FlowRun {
        id: String,
        name: String,
        input: Option<serde_json::Value>,
    },
    /// Any other request of the session the server wants signed, waiting
    /// in the shared request path (`api::VoltrusApi`) for the signature or
    /// for the user to decline; `what` describes it.
    Request {
        id: u64,
        what: String,
    },
}

impl Signable {
    /// The action in a few words, for the prompt and its toasts.
    pub fn describe(&self) -> String {
        match self {
            Self::TagWrite { tag, value, .. } => format!("Write {value} to tag {tag}"),
            Self::Acknowledge(alarm) => format!("Acknowledge alarm {alarm}"),
            Self::AcknowledgeAll => "Acknowledge all alarms".to_owned(),
            Self::Shelve { alarm, .. } => format!("Shelve alarm {alarm}"),
            Self::FlowRun { name, .. } => format!("Run flow {name} once"),
            Self::Request { what, .. } => what.clone(),
        }
    }
}

/// The signature for `request` from what the user typed, or why it cannot
/// go: the password and the reason are required; the meaning is the
/// server's, exactly as received.
pub fn signature(
    request: &SignatureRequired,
    password: &str,
    reason: &str,
) -> Result<Signature, &'static str> {
    if password.is_empty() {
        return Err("Enter your password");
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("Enter the reason for signing");
    }
    Ok(Signature {
        password: password.to_owned(),
        meaning: request.meaning.clone(),
        reason: reason.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    fn required() -> SignatureRequired {
        SignatureRequired {
            action: "tag_write".into(),
            meaning: "I approve this setpoint change".into(),
        }
    }

    #[test]
    fn a_428_asks_for_a_signature() {
        let body = r#"{"error": "Signature required", "code": "signature_required",
            "signature_required": {"action": "tag_write", "meaning": "I approve this setpoint change"}}"#;
        let error = ServerError::new(StatusCode::PRECONDITION_REQUIRED, body);
        assert_eq!(
            ask_of(&error),
            Some(Ask {
                request: required(),
                error: None
            })
        );
        let wrapped: anyhow::Error = error.into();
        assert!(ask(&wrapped).is_some());
    }

    #[test]
    fn a_wrong_password_asks_again_with_the_error() {
        let body = r#"{"error": "Invalid password", "code": "signature_invalid",
            "signature_required": {"action": "tag_write", "meaning": "I approve this setpoint change"}}"#;
        let error = ServerError::new(StatusCode::FORBIDDEN, body);
        assert_eq!(
            ask_of(&error),
            Some(Ask {
                request: required(),
                error: Some("Invalid password".into())
            })
        );
    }

    #[test]
    fn a_locked_account_or_an_impossible_signature_stops() {
        for code in ["account_locked", "signature_not_possible"] {
            let body = format!(r#"{{"error": "No", "code": "{code}"}}"#);
            let error = ServerError::new(StatusCode::FORBIDDEN, &body);
            assert_eq!(ask_of(&error), None, "{code}");
            assert_eq!(error.code.as_deref(), Some(code));
            assert_eq!(error.message, "No");
        }
        // A plain refusal is no request for a signature either.
        let error = ServerError::new(StatusCode::FORBIDDEN, r#"{"error": "Forbidden"}"#);
        assert_eq!(ask_of(&error), None);
        let failed = signed::<()>(Err(error.into()));
        assert_eq!(failed.unwrap_err().to_string(), "Forbidden");
        assert_eq!(signed(anyhow::Ok(3)).unwrap(), Signed::Done(3));
    }

    #[test]
    fn a_websocket_answer_asks_for_a_signature() {
        let request = required();
        assert_eq!(
            ask_of_ack("signature_required", None, Some(&request)),
            Some(Ask {
                request: request.clone(),
                error: None
            })
        );
        assert_eq!(
            ask_of_ack("rejected", Some("Invalid password"), Some(&request))
                .and_then(|ask| ask.error),
            Some("Invalid password".into())
        );
        assert_eq!(
            ask_of_ack("rejected", None, Some(&request)).and_then(|ask| ask.error),
            Some("The signature was refused".into())
        );
        // A refusal without a signature request is an ordinary failure.
        assert_eq!(ask_of_ack("rejected", Some("not writable"), None), None);
        assert_eq!(ask_of_ack("acked", None, Some(&request)), None);
    }

    #[test]
    fn the_signature_needs_a_password_and_a_reason() {
        let request = required();
        assert!(signature(&request, "", "because").is_err());
        assert!(signature(&request, "secret", "  ").is_err());
        let signed = signature(&request, "secret", " batch 7 ").unwrap();
        assert_eq!(signed.meaning, request.meaning, "the meaning as received");
        assert_eq!(signed.reason, "batch 7");
        assert!(
            !format!("{signed:?}").contains("secret"),
            "the password stays out of logs"
        );
    }

    #[test]
    fn retries_carry_the_signature() {
        let signature = signature(&required(), "secret", "batch 7").unwrap();
        let mut body = crate::tag_api::write_request(42.0, "", WriteMode::Immediate).unwrap();
        body.signature = Some(signature);
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "value": 42.0,
                "policy": "immediate",
                "signature": {
                    "password": "secret",
                    "meaning": "I approve this setpoint change",
                    "reason": "batch 7"
                }
            })
        );
    }
}
