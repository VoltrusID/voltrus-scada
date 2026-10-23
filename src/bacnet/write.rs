#![allow(dead_code)]
use std::net::SocketAddr;
use std::time::Duration;

use crate::models::WriteResult;

use bacnet_rs::network::Npdu;
use bacnet_rs::object::ObjectIdentifier;
use bacnet_rs::service::{ConfirmedServiceChoice, WritePropertyRequest};

use super::helpers::{object_type_from_str, property_id_from_str};

pub async fn write_property(
    ip: &str,
    port: u16,
    object_type: &str,
    object_instance: u32,
    property: &str,
    value: f64,
) -> WriteResult {
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

        let obj_id = ObjectIdentifier::new(ot, object_instance);

        // Encode property value as BACnet Real (tag 0x44 + 4-byte float)
        let mut property_value = vec![0x44];
        property_value.extend_from_slice(&(value as f32).to_be_bytes());

        let pid_u32: u32 = pid.into();
        let write_req = WritePropertyRequest::new(obj_id, pid_u32, property_value);

        let mut service_data = Vec::new();
        write_req
            .encode(&mut service_data)
            .map_err(|e| e.to_string())?;

        // Send via raw socket
        let socket =
            std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("Socket bind: {e}"))?;

        use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};
        let apdu = Apdu::ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: MaxSegments::Unspecified,
            max_response_size: MaxApduSize::Up1476,
            invoke_id: 1,
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

        // Wait for ack
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
                    if let Ok((_npdu, npdu_len)) = Npdu::decode(&buf[npdu_start..len]) {
                        let apdu_start = npdu_start + npdu_len;
                        if let Ok(Apdu::SimpleAck { .. }) = Apdu::decode(&buf[apdu_start..len]) {
                            return Ok(());
                        }
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
        Ok(Ok(())) => WriteResult {
            success: true,
            device_response: Some("acked".to_string()),
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
