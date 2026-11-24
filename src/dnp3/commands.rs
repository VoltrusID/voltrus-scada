//! Tag writes for DNP3: turning a data source's `source_config` into a
//! control, and handing it to the running poller's session.
//!
//! Many outstations accept one master connection only, so a write opened on
//! a second TCP connection would either be refused or knock the poller off.
//! Writes therefore go through the poller's session when one is running.

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};

use crate::dnp3::application::{AnalogCommand, Control, Crob};
use crate::dnp3::master::OperateMode;
use crate::models::WriteResult;

pub struct Dnp3Command {
    pub control: Control,
    pub mode: OperateMode,
    pub reply: oneshot::Sender<WriteResult>,
}

static SESSIONS: LazyLock<DashMap<String, mpsc::Sender<Dnp3Command>>> = LazyLock::new(DashMap::new);

/// Keeps a poller's command channel registered while it lives.
pub struct Registration {
    key: String,
    tx: mpsc::Sender<Dnp3Command>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        SESSIONS.remove_if(&self.key, |_, tx| tx.same_channel(&self.tx));
    }
}

pub fn register(device_key: &str) -> (Registration, mpsc::Receiver<Dnp3Command>) {
    let (tx, rx) = mpsc::channel(8);
    SESSIONS.insert(device_key.to_string(), tx.clone());
    (
        Registration {
            key: device_key.to_string(),
            tx,
        },
        rx,
    )
}

/// Run a control on the poller session for `device_key` (`dnp3_<id>`).
/// `None` when no poller is connected, so the caller can connect itself.
pub async fn operate_via_poller(
    device_key: &str,
    control: Control,
    mode: OperateMode,
) -> Option<WriteResult> {
    let tx = SESSIONS.get(device_key)?.clone();
    let (reply, rx) = oneshot::channel();
    tx.send(Dnp3Command {
        control,
        mode,
        reply,
    })
    .await
    .ok()?;
    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(r)) => Some(r),
        Ok(Err(_)) => Some(WriteResult {
            success: false,
            device_response: None,
            error: Some("DNP3 poller dropped the command (connection lost)".into()),
            latency_ms: 0,
        }),
        Err(_) => Some(WriteResult {
            success: false,
            device_response: None,
            error: Some("DNP3 command timeout (15s)".into()),
            latency_ms: 0,
        }),
    }
}

/// Build the control for writing `value` to the tag described by
/// `source_config`:
///
/// ```json
/// { "point_index": 3, "group": 12, "mode": "sbo", "control": "latch" }
/// { "point_index": 0, "group": 41, "variation": 3, "mode": "direct" }
/// ```
///
/// `group` 10/12 (default) writes a CROB, 40/41 an analog output. `mode` is
/// `sbo` (default) or `direct`. For CROBs, `control` is `latch` (default),
/// `pulse` or `trip_close`, with `pulse_ms` (default 1000). For analog
/// outputs, `variation` 1 (int32), 2 (int16), 3 (float32) or 4 (float64);
/// without it, int32 for whole numbers and float32 otherwise.
pub fn parse_write_target(source_config: &str, value: f64) -> Result<(Control, OperateMode)> {
    let cfg: serde_json::Value = serde_json::from_str(source_config)
        .map_err(|e| anyhow::anyhow!("DNP3 source_config is not JSON: {e}"))?;
    let index = cfg
        .get("point_index")
        .or_else(|| cfg.get("index"))
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| anyhow::anyhow!("DNP3 source_config needs point_index (0-65535)"))?;
    let group = cfg.get("group").and_then(|v| v.as_u64()).unwrap_or(12);
    let mode = match cfg.get("mode").and_then(|v| v.as_str()).unwrap_or("sbo") {
        "sbo" | "select" => OperateMode::SelectBeforeOperate,
        "direct" => OperateMode::DirectOperate,
        m => anyhow::bail!("DNP3 mode '{m}' is not sbo or direct"),
    };
    let control = match group {
        10 | 12 => {
            let on = value != 0.0;
            let ms = cfg
                .get("pulse_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(1000)
                .min(u32::MAX as u64) as u32;
            let crob = match cfg
                .get("control")
                .and_then(|v| v.as_str())
                .unwrap_or("latch")
            {
                "latch" => Crob::latch(on),
                "pulse" => Crob::pulse(on, ms),
                "trip_close" => Crob::trip_close(on, ms),
                c => anyhow::bail!("DNP3 control '{c}' is not latch, pulse or trip_close"),
            };
            Control::Crob { index, crob }
        }
        40 | 41 => {
            let whole = value.fract() == 0.0;
            let variation = cfg
                .get("variation")
                .and_then(|v| v.as_u64())
                .unwrap_or(if whole { 1 } else { 3 });
            let out_of_range = || anyhow::anyhow!("value {value} does not fit g41v{variation}");
            let value = match variation {
                1 if whole && (i32::MIN as f64..=i32::MAX as f64).contains(&value) => {
                    AnalogCommand::I32(value as i32)
                }
                2 if whole && (i16::MIN as f64..=i16::MAX as f64).contains(&value) => {
                    AnalogCommand::I16(value as i16)
                }
                3 if value.is_finite() && value.abs() <= f32::MAX as f64 => {
                    AnalogCommand::F32(value as f32)
                }
                4 if value.is_finite() => AnalogCommand::F64(value),
                1..=4 => return Err(out_of_range()),
                v => anyhow::bail!("g41 variation {v} does not exist"),
            };
            Control::Analog { index, value }
        }
        g => anyhow::bail!("DNP3 group {g} cannot be written; use 12 (binary) or 41 (analog)"),
    };
    Ok((control, mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_sbo_latch() {
        let (c, m) = parse_write_target(r#"{"point_index":3}"#, 1.0).unwrap();
        assert_eq!(m, OperateMode::SelectBeforeOperate);
        assert_eq!(
            c,
            Control::Crob {
                index: 3,
                crob: Crob::latch(true)
            }
        );
        let (c, _) = parse_write_target(r#"{"index":3}"#, 0.0).unwrap();
        assert_eq!(
            c,
            Control::Crob {
                index: 3,
                crob: Crob::latch(false)
            }
        );
    }

    #[test]
    fn analog_variations() {
        let (c, m) =
            parse_write_target(r#"{"point_index":1,"group":41,"mode":"direct"}"#, 42.0).unwrap();
        assert_eq!(m, OperateMode::DirectOperate);
        assert_eq!(
            c,
            Control::Analog {
                index: 1,
                value: AnalogCommand::I32(42)
            }
        );
        let (c, _) = parse_write_target(r#"{"point_index":1,"group":40}"#, 1.5).unwrap();
        assert_eq!(
            c,
            Control::Analog {
                index: 1,
                value: AnalogCommand::F32(1.5)
            }
        );
        assert!(
            parse_write_target(r#"{"point_index":1,"group":41,"variation":2}"#, 70000.0).is_err()
        );
        assert!(parse_write_target(r#"{"point_index":1,"group":30}"#, 1.0).is_err());
        assert!(parse_write_target(r#"{"group":12}"#, 1.0).is_err());
    }

    #[test]
    fn pulse_and_trip_close() {
        let (c, _) =
            parse_write_target(r#"{"point_index":0,"control":"pulse","pulse_ms":250}"#, 1.0)
                .unwrap();
        assert_eq!(
            c,
            Control::Crob {
                index: 0,
                crob: Crob::pulse(true, 250)
            }
        );
        let (c, _) =
            parse_write_target(r#"{"point_index":0,"control":"trip_close"}"#, 0.0).unwrap();
        assert_eq!(
            c,
            Control::Crob {
                index: 0,
                crob: Crob::trip_close(false, 1000)
            }
        );
    }
}
