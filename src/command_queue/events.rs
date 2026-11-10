use super::*;
use crate::ws_types::ServerMessage;

impl CommandQueue {
    pub(super) fn publish_status_event(&self, command_id: &str, status: &str, error: Option<&str>) {
        if let Some(ref ws) = self.ws_manager {
            ws.broadcast_command_ack(ServerMessage::CommandAck {
                command_id: command_id.to_string(),
                status: status.to_string(),
                device_response: error.map(|e| e.to_string()),
                latency_ms: None,
            });
        }
    }

    pub(super) fn publish_ack_event(&self, command_id: &str, response: &str, latency_ms: u64) {
        if let Some(ref ws) = self.ws_manager {
            ws.broadcast_command_ack(ServerMessage::CommandAck {
                command_id: command_id.to_string(),
                status: "acked".to_string(),
                device_response: Some(response.to_string()),
                latency_ms: Some(latency_ms),
            });
        }
    }

    /// Tell clients how the read-back of an acknowledged write ended: status
    /// `readback_confirmed`, `readback_mismatch` or `readback_timeout`, with
    /// the value read back as the device response.
    pub(super) fn publish_readback_event(
        &self,
        command_id: &str,
        outcome: &super::readback::Outcome,
    ) {
        use super::readback::Outcome;
        if let Some(ref ws) = self.ws_manager {
            let value = match outcome {
                Outcome::Confirmed { value, .. } | Outcome::Mismatch { value, .. } => {
                    Some(value.to_string())
                }
                Outcome::Timeout => None,
            };
            ws.broadcast_command_ack(ServerMessage::CommandAck {
                command_id: command_id.to_string(),
                status: format!("readback_{}", outcome.status()),
                device_response: value,
                latency_ms: None,
            });
        }
    }
}
