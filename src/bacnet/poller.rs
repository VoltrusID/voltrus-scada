#![allow(dead_code)]
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cache::DeviceCache;
use crate::models::{BacnetDeviceConfig, DeviceStatus};

use bacnet_rs::network::Npdu;
use bacnet_rs::object::{ObjectIdentifier, PropertyIdentifier};
use bacnet_rs::service::{
    ConfirmedServiceChoice, PropertyReference, ReadAccessSpecification, ReadPropertyMultipleRequest,
};

use super::helpers::{object_type_from_str, property_id_from_str};

struct Backoff {
    attempt: u32,
}

impl Backoff {
    fn new() -> Self {
        Self { attempt: 0 }
    }
    fn reset(&mut self) {
        self.attempt = 0;
    }
    fn next(&mut self) -> u64 {
        let delay = 1000 * 2u64.pow(self.attempt.min(6));
        self.attempt += 1;
        delay.min(60_000)
    }
}

pub async fn poll_bacnet_device(config: BacnetDeviceConfig, cache: DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_poll_cycle(&config).await {
            Ok(metrics) => {
                backoff.reset();
                cache.set_status(&device_id, DeviceStatus::Online).await;
                if !metrics.is_empty() {
                    cache.update(&device_id, metrics, false).await;
                }
            }
            Err(e) => {
                tracing::warn!("BACnet device {} error: {e}", device_id);
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                let delay = backoff.next();
                tracing::info!("BACnet device {} retrying in {delay}ms", device_id);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                continue;
            }
        }

        tokio::time::sleep(Duration::from_millis(config.interval_ms)).await;
    }
}

async fn try_poll_cycle(config: &BacnetDeviceConfig) -> Result<HashMap<String, f64>> {
    let objects: Vec<(
        String,
        bacnet_rs::object::ObjectType,
        u32,
        PropertyIdentifier,
    )> = config
        .objects
        .iter()
        .filter_map(|o| {
            let ot = object_type_from_str(&o.object_type)?;
            let pid = property_id_from_str(&o.property);
            Some((o.name.clone(), ot, o.object_instance, pid))
        })
        .collect();

    if objects.is_empty() {
        return Ok(HashMap::new());
    }

    let addr = format!("{}:{}", config.ip, config.port)
        .parse::<SocketAddr>()
        .context("Invalid BACnet device address")?;

    // Group by object for RPM (read multiple properties per object)
    let mut read_specs = Vec::new();
    for (_, ot, inst, pid) in &objects {
        let obj_id = ObjectIdentifier::new(*ot, *inst);
        let prop_ref = PropertyReference::new(*pid);
        read_specs.push(ReadAccessSpecification::new(obj_id, vec![prop_ref]));
    }

    let rpm_request = ReadPropertyMultipleRequest::new(read_specs);

    let metrics = tokio::task::spawn_blocking(move || -> Result<HashMap<String, f64>> {
        // Bind socket once and reuse for the entire poll cycle
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").context("BACnet socket bind failed")?;
        socket
            .set_read_timeout(Some(Duration::from_secs(6)))
            .context("BACnet socket set_read_timeout failed")?;

        // Send ReadPropertyMultiple
        let mut service_data = Vec::new();
        rpm_request.encode(&mut service_data)?;

        let response_data = client_send_confirmed(&socket, addr, service_data)?;

        // Parse response - extract property values
        let mut result = HashMap::new();

        // Simple response parsing: scan for known tags
        // PropertyValue encoding: tag byte indicates type
        // 0x44 = Real (4 bytes), 0x55 = Double (8 bytes),
        // 0x21 = Boolean, 0x31 = Unsigned (1-4 bytes), 0x33 = Enum (1-4 bytes)
        let data = &response_data;
        let mut pos = 0;
        let mut obj_idx = 0;

        while pos < data.len() && obj_idx < objects.len() {
            // Look for property value tags
            if pos + 1 < data.len() {
                let tag = data[pos];
                let value = match tag {
                    0x44 if pos + 5 <= data.len() => {
                        // Real (f32)
                        let bytes: [u8; 4] = data[pos + 1..pos + 5]
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("parse f32 bytes"))?;
                        Some(f32::from_be_bytes(bytes) as f64)
                    }
                    0x55 if pos + 9 <= data.len() => {
                        // Double (f64)
                        let bytes: [u8; 8] = data[pos + 1..pos + 9]
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("parse f64 bytes"))?;
                        Some(f64::from_be_bytes(bytes))
                    }
                    0x22 if pos + 3 <= data.len() => {
                        // Unsigned 2-byte
                        Some(((data[pos + 1] as u32) << 8 | data[pos + 2] as u32) as f64)
                    }
                    0x21 if pos + 2 <= data.len() => {
                        // Unsigned 1-byte
                        Some(data[pos + 1] as f64)
                    }
                    0x24 if pos + 5 <= data.len() => {
                        // Unsigned 4-byte
                        let bytes: [u8; 4] = data[pos + 1..pos + 5]
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("parse u32 bytes"))?;
                        Some(u32::from_be_bytes(bytes) as f64)
                    }
                    0x31 => {
                        // Boolean true
                        Some(1.0)
                    }
                    0x30 => {
                        // Boolean false
                        Some(0.0)
                    }
                    0x32 if pos + 3 <= data.len() => {
                        // Enum 2-byte
                        Some(((data[pos + 1] as u32) << 8 | data[pos + 2] as u32) as f64)
                    }
                    _ => None,
                };

                if let Some(v) = value {
                    let (name, _, _, _) = &objects[obj_idx];
                    result.insert(name.clone(), v);
                    obj_idx += 1;
                }
            }
            pos += 1;
        }

        Ok(result)
    })
    .await
    .context("Task join failed")??;

    Ok(metrics)
}

/// Send a confirmed request reusing an existing bound socket.
fn client_send_confirmed(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    service_data: Vec<u8>,
) -> Result<Vec<u8>> {
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
        service_choice: ConfirmedServiceChoice::ReadPropertyMultiple,
        service_data,
    };

    let apdu_data = apdu.encode();
    let mut npdu = Npdu::new();
    npdu.control.expecting_reply = true;
    let npdu_data = npdu.encode();

    let mut message = npdu_data;
    message.extend_from_slice(&apdu_data);

    // BVLC Original-Unicast-NPDU
    let mut bvlc = vec![0x81, 0x0A, 0x00, 0x00];
    bvlc.extend_from_slice(&message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;

    // Socket is already bound by caller
    socket
        .send_to(&bvlc, target_addr)
        .context("BACnet send failed")?;

    let mut buf = [0u8; 1500];
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(6) {
        match socket.recv_from(&mut buf) {
            Ok((len, source)) if source == target_addr => {
                // Parse BVLC header
                if len < 4 || buf[0] != 0x81 {
                    continue;
                }
                // Skip BVLC (4 bytes) and NPDU
                let bvlc_len = ((buf[2] as usize) << 8) | buf[3] as usize;
                if len != bvlc_len {
                    continue;
                }
                let npdu_start = 4;
                if let Ok((_npdu, npdu_len)) = Npdu::decode(&buf[npdu_start..len]) {
                    let apdu_start = npdu_start + npdu_len;
                    if let Ok(Apdu::ComplexAck { service_data, .. }) =
                        Apdu::decode(&buf[apdu_start..len])
                    {
                        return Ok(service_data);
                    }
                }
            }
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e).context("BACnet recv failed"),
        }
    }

    anyhow::bail!("BACnet ReadPropertyMultiple timeout for {}", target_addr)
}
