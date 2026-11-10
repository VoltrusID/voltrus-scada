use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use crate::models::WriteResult;

use bacnet_rs::app::Apdu;
use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, ObjectType, PropertyIdentifier};
use bacnet_rs::service::{ConfirmedServiceChoice, WritePropertyRequest};

use super::helpers::{object_type_from_str, property_id_from_str};

/// Priority a queued write takes in a commandable object's priority array when
/// the tag does not set one. 8 is "Manual Operator" (ASHRAE 135, 19.2.1.1),
/// which is what an operator setpoint from the HMI is: above the default 16
/// that most schedules and programs use, below life safety (1-2) and the
/// critical equipment control and minimum on/off slots (5-6).
pub const DEFAULT_WRITE_PRIORITY: u8 = 8;

/// Invoke IDs of confirmed requests in flight, so one write's answer is never
/// taken for another's when several devices or tags are written close together.
static NEXT_INVOKE_ID: AtomicU8 = AtomicU8::new(0);

fn next_invoke_id() -> u8 {
    NEXT_INVOKE_ID.fetch_add(1, Ordering::Relaxed)
}

/// The application-tagged value for writing `value` to `property` of an object
/// of type `object_type`: the datatype the standard gives that property.
/// Present Value is REAL for analog objects, BACnetBinaryPV (ENUMERATED,
/// 0 inactive / 1 active) for binary objects and Unsigned (from 1) for
/// multi-state objects. Any other property is written as REAL.
pub(crate) fn encode_property_value(
    object_type: ObjectType,
    property: PropertyIdentifier,
    value: f64,
) -> Result<Vec<u8>, String> {
    if !value.is_finite() {
        return Err(format!("Value {value} is not a finite number"));
    }
    let present_value = property == PropertyIdentifier::PresentValue;
    match object_type {
        ObjectType::BinaryInput | ObjectType::BinaryOutput | ObjectType::BinaryValue
            if present_value =>
        {
            Ok(vec![0x91, u8::from(value != 0.0)])
        }
        ObjectType::MultiStateInput
        | ObjectType::MultiStateOutput
        | ObjectType::MultiStateValue
            if present_value =>
        {
            let state = value.round();
            if !(1.0..=u32::MAX as f64).contains(&state) {
                return Err(format!(
                    "Value {value} is not a multi-state value (states start at 1)"
                ));
            }
            Ok(encode_unsigned(state as u32))
        }
        _ => {
            if value.abs() > f32::MAX as f64 {
                return Err(format!("Value {value} is outside the REAL range"));
            }
            let mut out = vec![0x44];
            out.extend_from_slice(&(value as f32).to_be_bytes());
            Ok(out)
        }
    }
}

/// Application tag 2 (Unsigned) in the fewest octets that hold `v`.
fn encode_unsigned(v: u32) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let skip = bytes.iter().take(3).take_while(|b| **b == 0).count();
    let len = 4 - skip;
    let mut out = vec![0x20 | len as u8];
    out.extend_from_slice(&bytes[skip..]);
    out
}

/// A priority from 1 to 16, except 6, which the standard reserves for minimum
/// on/off time and a device refuses.
pub fn check_priority(priority: u8) -> Result<u8, String> {
    match priority {
        6 => Err("BACnet priority 6 is reserved for minimum on/off time".into()),
        1..=16 => Ok(priority),
        _ => Err(format!("BACnet priority {priority} is not 1-16")),
    }
}

pub async fn write_property(
    ip: &str,
    port: u16,
    object_type: &str,
    object_instance: u32,
    property: &str,
    value: f64,
    priority: u8,
) -> WriteResult {
    if !crate::node_role::is_active() {
        return WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let addr = format!("{}:{}", ip, port);
    let object_type = object_type.to_string();
    let property = property.to_string();

    let result = tokio::task::spawn_blocking(move || {
        let addr: SocketAddr = addr
            .parse()
            .map_err(|e: std::net::AddrParseError| e.to_string())?;
        let ot = object_type_from_str(&object_type)
            .ok_or_else(|| format!("Unknown object type: {}", object_type))?;
        let pid = property_id_from_str(&property);
        let priority = check_priority(priority)?;

        let obj_id = ObjectIdentifier::new(ot, object_instance);
        let property_value = encode_property_value(ot, pid, value)?;

        let pid_u32: u32 = pid.into();
        let write_req =
            WritePropertyRequest::with_priority(obj_id, pid_u32, property_value, priority);

        let mut service_data = Vec::new();
        write_req
            .encode(&mut service_data)
            .map_err(|e| e.to_string())?;

        let socket =
            std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("Socket bind: {e}"))?;

        use bacnet_rs::app::{MaxApduSize, MaxSegments};
        let invoke_id = next_invoke_id();
        let apdu = Apdu::ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: MaxSegments::Unspecified,
            max_response_size: MaxApduSize::Up1476,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WriteProperty,
            service_data,
        };

        let apdu_data = apdu.encode();
        let mut npdu = Npdu::new();
        npdu.control.expecting_reply = true;
        let npdu_data = npdu.encode();

        let mut msg = npdu_data;
        msg.extend_from_slice(&apdu_data);

        let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
        bvlc.extend_from_slice(&msg);
        let len = bvlc.len() as u16;
        bvlc[2] = (len >> 8) as u8;
        bvlc[3] = (len & 0xFF) as u8;

        socket
            .send_to(&bvlc, addr)
            .map_err(|e| format!("Send: {e}"))?;

        let mut buf = [0u8; 1500];
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("Timeout set: {e}"))?;

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match socket.recv_from(&mut buf) {
                Ok((len, src)) if src == addr => {
                    if len < 4 || buf[0] != 0x81 {
                        continue;
                    }
                    let npdu_start = 4;
                    let Ok((_npdu, npdu_len)) = Npdu::decode(&buf[npdu_start..len]) else {
                        continue;
                    };
                    let Ok(apdu) = Apdu::decode(&buf[npdu_start + npdu_len..len]) else {
                        continue;
                    };
                    if let Some(outcome) = write_outcome(&apdu, invoke_id) {
                        return outcome.map(|()| priority);
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }

        Err("Write ack timeout".to_string())
    })
    .await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(priority)) => WriteResult {
            success: true,
            device_response: Some(format!("acked (priority {priority})")),
            error: None,
            latency_ms,
        },
        Ok(Err(e)) => WriteResult {
            success: false,
            device_response: None,
            error: Some(e),
            latency_ms,
        },
        Err(e) => WriteResult {
            success: false,
            device_response: None,
            error: Some(e.to_string()),
            latency_ms,
        },
    }
}

/// What an APDU says about the write with `invoke_id`: `None` when it answers
/// another request, so the caller keeps waiting.
fn write_outcome(apdu: &Apdu, invoke_id: u8) -> Option<Result<(), String>> {
    match apdu {
        Apdu::SimpleAck { invoke_id: id, .. } if *id == invoke_id => Some(Ok(())),
        Apdu::Error {
            invoke_id: id,
            error_class,
            error_code,
            ..
        } if *id == invoke_id => Some(Err(format!(
            "BACnet error: class {error_class}, code {error_code}"
        ))),
        Apdu::Reject {
            invoke_id: id,
            reject_reason,
        } if *id == invoke_id => Some(Err(format!("BACnet reject: {reject_reason:?}"))),
        Apdu::Abort {
            invoke_id: id,
            abort_reason,
            ..
        } if *id == invoke_id => Some(Err(format!("BACnet abort: reason {abort_reason}"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analog_present_value_is_real() {
        let v = encode_property_value(
            ObjectType::AnalogOutput,
            PropertyIdentifier::PresentValue,
            21.5,
        )
        .unwrap();
        assert_eq!(v, vec![0x44, 0x41, 0xAC, 0x00, 0x00]);
    }

    #[test]
    fn binary_present_value_is_enumerated() {
        let on = encode_property_value(
            ObjectType::BinaryOutput,
            PropertyIdentifier::PresentValue,
            1.0,
        )
        .unwrap();
        let off = encode_property_value(
            ObjectType::BinaryValue,
            PropertyIdentifier::PresentValue,
            0.0,
        )
        .unwrap();
        assert_eq!(on, vec![0x91, 0x01]);
        assert_eq!(off, vec![0x91, 0x00]);
        let any_nonzero = encode_property_value(
            ObjectType::BinaryOutput,
            PropertyIdentifier::PresentValue,
            -3.0,
        )
        .unwrap();
        assert_eq!(any_nonzero, vec![0x91, 0x01]);
    }

    #[test]
    fn multistate_present_value_is_unsigned_from_one() {
        let v = encode_property_value(
            ObjectType::MultiStateValue,
            PropertyIdentifier::PresentValue,
            3.0,
        )
        .unwrap();
        assert_eq!(v, vec![0x21, 0x03]);
        let wide = encode_property_value(
            ObjectType::MultiStateOutput,
            PropertyIdentifier::PresentValue,
            300.0,
        )
        .unwrap();
        assert_eq!(wide, vec![0x22, 0x01, 0x2C]);
        assert!(
            encode_property_value(
                ObjectType::MultiStateOutput,
                PropertyIdentifier::PresentValue,
                0.0
            )
            .is_err()
        );
    }

    #[test]
    fn unsigned_uses_fewest_octets() {
        assert_eq!(encode_unsigned(0), vec![0x21, 0x00]);
        assert_eq!(encode_unsigned(255), vec![0x21, 0xFF]);
        assert_eq!(encode_unsigned(65_536), vec![0x23, 0x01, 0x00, 0x00]);
        assert_eq!(
            encode_unsigned(u32::MAX),
            vec![0x24, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn non_finite_value_is_refused() {
        assert!(
            encode_property_value(
                ObjectType::AnalogValue,
                PropertyIdentifier::PresentValue,
                f64::NAN
            )
            .is_err()
        );
    }

    #[test]
    fn priority_range() {
        assert_eq!(check_priority(DEFAULT_WRITE_PRIORITY), Ok(8));
        assert_eq!(check_priority(1), Ok(1));
        assert_eq!(check_priority(16), Ok(16));
        assert!(check_priority(0).is_err());
        assert!(check_priority(6).is_err());
        assert!(check_priority(17).is_err());
    }

    #[test]
    fn request_carries_priority_and_value() {
        let obj = ObjectIdentifier::new(ObjectType::BinaryOutput, 3);
        let value = encode_property_value(
            ObjectType::BinaryOutput,
            PropertyIdentifier::PresentValue,
            1.0,
        )
        .unwrap();
        let req = WritePropertyRequest::with_priority(obj, 85, value, 8);
        let mut out = Vec::new();
        req.encode(&mut out).unwrap();
        // ... [3E] 91 01 [3F] then context tag 4 = priority 8.
        assert_eq!(&out[out.len() - 6..], &[0x3E, 0x91, 0x01, 0x3F, 0x49, 0x08]);
    }

    #[test]
    fn invoke_ids_roll() {
        let a = next_invoke_id();
        let b = next_invoke_id();
        assert_ne!(a, b);
    }

    #[test]
    fn outcome_matches_invoke_id() {
        let ack = Apdu::SimpleAck {
            invoke_id: 7,
            service_choice: 15,
        };
        assert_eq!(write_outcome(&ack, 7), Some(Ok(())));
        assert_eq!(write_outcome(&ack, 8), None);
        let err = Apdu::Error {
            invoke_id: 7,
            service_choice: ConfirmedServiceChoice::WriteProperty,
            error_class: 2,
            error_code: 40,
        };
        assert!(matches!(write_outcome(&err, 7), Some(Err(_))));
    }
}
