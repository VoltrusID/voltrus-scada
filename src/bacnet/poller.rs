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

    let metrics = tokio::task::spawn_blocking(move || -> Result<HashMap<String, f64>> {
        // Bind socket once and reuse for the entire poll cycle
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").context("BACnet socket bind failed")?;
        socket
            .set_read_timeout(Some(Duration::from_secs(6)))
            .context("BACnet socket set_read_timeout failed")?;

        let points: Vec<(ObjectIdentifier, PropertyIdentifier)> = objects
            .iter()
            .map(|(_, ot, inst, pid)| (ObjectIdentifier::new(*ot, *inst), *pid))
            .collect();
        let mut invoke_id = 0u8;
        let mut decoded = Vec::new();
        for chunk in rpm_chunks(&points, MAX_APDU) {
            read_chunk(&socket, addr, chunk, &mut invoke_id, &mut decoded)?;
        }
        Ok(map_rpm_results(&objects, &decoded))
    })
    .await
    .context("Task join failed")??;

    Ok(metrics)
}

/// Largest APDU we ask for and assume the device sends (BACnet/IP).
const MAX_APDU: usize = 1476;
/// ComplexACK header: PDU type, invoke id, service choice.
const ACK_HEADER: usize = 3;

/// Bytes one point can take in a ReadPropertyMultiple-ACK: object id (5),
/// opening/closing tag 1 (2), property id (up to 4), opening/closing tag 4
/// or 5 (2) and the value or error. Numeric values take at most 9 bytes (a
/// DOUBLE); text properties are given room for a typical name.
fn ack_bytes(property: PropertyIdentifier) -> usize {
    let value = match property {
        PropertyIdentifier::ObjectName
        | PropertyIdentifier::Description
        | PropertyIdentifier::VendorName
        | PropertyIdentifier::ModelName
        | PropertyIdentifier::FirmwareRevision => 130,
        _ => 10,
    };
    5 + 2 + 4 + 2 + value
}

/// Split the points into requests whose answers fit in one unsegmented
/// APDU of `max_apdu` bytes, so the device never has to segment.
fn rpm_chunks(
    points: &[(ObjectIdentifier, PropertyIdentifier)],
    max_apdu: usize,
) -> Vec<&[(ObjectIdentifier, PropertyIdentifier)]> {
    let budget = max_apdu.saturating_sub(ACK_HEADER);
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (i, (_, pid)) in points.iter().enumerate() {
        let need = ack_bytes(*pid);
        if i > start && used + need > budget {
            chunks.push(&points[start..i]);
            start = i;
            used = 0;
        }
        used += need;
    }
    if start < points.len() {
        chunks.push(&points[start..]);
    }
    chunks
}

/// Read one chunk. If the answer would still not fit (the device aborts
/// because it would have to segment, or has a smaller buffer), the chunk is
/// split in half and each half read on its own.
fn read_chunk(
    socket: &std::net::UdpSocket,
    addr: SocketAddr,
    chunk: &[(ObjectIdentifier, PropertyIdentifier)],
    invoke_id: &mut u8,
    out: &mut Vec<super::rpm::RpmObject>,
) -> Result<()> {
    let specs = chunk
        .iter()
        .map(|(oid, pid)| ReadAccessSpecification::new(*oid, vec![PropertyReference::new(*pid)]))
        .collect();
    let mut service_data = Vec::new();
    ReadPropertyMultipleRequest::new(specs).encode(&mut service_data)?;
    *invoke_id = invoke_id.wrapping_add(1);
    match client_send_confirmed(socket, addr, *invoke_id, service_data) {
        Ok(data) => {
            out.extend(
                super::rpm::decode_rpm_ack(&data).context("Invalid ReadPropertyMultiple-ACK")?,
            );
            Ok(())
        }
        Err(e) if chunk.len() > 1 && e.downcast_ref::<TooLarge>().is_some() => {
            let (a, b) = chunk.split_at(chunk.len() / 2);
            read_chunk(socket, addr, a, invoke_id, out)?;
            read_chunk(socket, addr, b, invoke_id, out)
        }
        Err(e) => Err(e),
    }
}

/// The numeric abort reason. bacnet-rs 0.3.0 gives it as a `u8`, 0.3.2 as
/// an `AbortReason` enum that converts to one; Core's lock resolves the
/// first and the Enterprise build the second, so both must compile.
fn abort_code(reason: impl Into<u8>) -> u8 {
    reason.into()
}

/// The device refused because its answer would need segmentation or does
/// not fit its buffer.
#[derive(Debug)]
struct TooLarge(u8);

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BACnet ReadPropertyMultiple answer too large for one APDU (abort reason {})",
            self.0
        )
    }
}

impl std::error::Error for TooLarge {}

/// Match the decoded ACK to the configured points by object and property.
/// A property the device answered with an error, a non-numeric value, or no
/// answer at all is NaN, which the cache stores as bad quality.
fn map_rpm_results(
    objects: &[(
        String,
        bacnet_rs::object::ObjectType,
        u32,
        PropertyIdentifier,
    )],
    decoded: &[super::rpm::RpmObject],
) -> HashMap<String, f64> {
    use super::rpm::RpmValue;
    let mut by_key: HashMap<(u32, u32, u32), f64> = HashMap::new();
    for obj in decoded {
        for prop in &obj.properties {
            if prop.array_index.is_some() {
                continue;
            }
            let v = match prop.value {
                RpmValue::Number(v) => v,
                RpmValue::Unsupported | RpmValue::Error { .. } => f64::NAN,
            };
            by_key
                .entry((obj.object_type, obj.instance, prop.property_id))
                .or_insert(v);
        }
    }
    objects
        .iter()
        .map(|(name, ot, inst, pid)| {
            let key = (u32::from(*ot), *inst, u32::from(*pid));
            (name.clone(), by_key.get(&key).copied().unwrap_or(f64::NAN))
        })
        .collect()
}

/// Send a confirmed request reusing an existing bound socket.
fn client_send_confirmed(
    socket: &std::net::UdpSocket,
    target_addr: SocketAddr,
    invoke_id: u8,
    service_data: Vec<u8>,
) -> Result<Vec<u8>> {
    use bacnet_rs::app::{Apdu, MaxApduSize, MaxSegments};

    let apdu = Apdu::ConfirmedRequest {
        segmented: false,
        more_follows: false,
        // Requests are sized so the answer fits one APDU; a device that
        // cannot answer unsegmented aborts and the request is split.
        segmented_response_accepted: false,
        max_segments: MaxSegments::Unspecified,
        max_response_size: MaxApduSize::Up1476,
        invoke_id,
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
                    match Apdu::decode(&buf[apdu_start..len]) {
                        Ok(
                            Apdu::ComplexAck { invoke_id: id, .. }
                            | Apdu::Error { invoke_id: id, .. }
                            | Apdu::Reject { invoke_id: id, .. }
                            | Apdu::Abort { invoke_id: id, .. },
                        ) if id != invoke_id => continue, // a late answer to an earlier request
                        Ok(Apdu::ComplexAck {
                            segmented: true, ..
                        }) => anyhow::bail!(
                            "BACnet device {target_addr} sent a segmented ReadPropertyMultiple \
                             answer, which is not supported (segmentation was not offered)"
                        ),
                        Ok(Apdu::ComplexAck { service_data, .. }) => return Ok(service_data),
                        // 1 buffer-overflow, 4 segmentation-not-supported,
                        // 11 APDU too long
                        Ok(Apdu::Abort { abort_reason, .. })
                            if matches!(abort_code(abort_reason), 1 | 4 | 11) =>
                        {
                            return Err(TooLarge(abort_code(abort_reason)).into());
                        }
                        Ok(Apdu::Error {
                            error_class,
                            error_code,
                            ..
                        }) => anyhow::bail!(
                            "BACnet ReadPropertyMultiple error (class {error_class}, code {error_code})"
                        ),
                        Ok(Apdu::Reject { reject_reason, .. }) => {
                            anyhow::bail!("BACnet ReadPropertyMultiple rejected: {reject_reason:?}")
                        }
                        Ok(Apdu::Abort { abort_reason, .. }) => {
                            anyhow::bail!(
                                "BACnet ReadPropertyMultiple aborted (reason {abort_reason})"
                            )
                        }
                        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_rs::object::ObjectType;

    #[test]
    fn maps_values_by_object_and_property_and_marks_errors_bad() {
        let objects = vec![
            (
                "temp".to_string(),
                ObjectType::AnalogInput,
                5,
                PropertyIdentifier::PresentValue,
            ),
            (
                "missing".to_string(),
                ObjectType::AnalogInput,
                1,
                PropertyIdentifier::PresentValue,
            ),
            (
                "fan".to_string(),
                ObjectType::BinaryOutput,
                2,
                PropertyIdentifier::PresentValue,
            ),
            (
                "unanswered".to_string(),
                ObjectType::AnalogValue,
                9,
                PropertyIdentifier::PresentValue,
            ),
        ];
        // Device answers in a different order than asked.
        let data = [
            0x0C, 0x01, 0x00, 0x00, 0x02, 0x1E, 0x29, 0x55, 0x4E, 0x91, 0x01, 0x4F,
            0x1F, // BO 2
            0x0C, 0x00, 0x00, 0x00, 0x01, 0x1E, 0x29, 0x55, 0x5E, 0x91, 0x01, 0x91, 0x1F, 0x5F,
            0x1F, // AI 1 error
            0x0C, 0x00, 0x00, 0x00, 0x05, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x41, 0x90, 0x00, 0x00,
            0x4F, 0x1F, // AI 5 = 18.0
        ];
        let decoded = crate::bacnet::rpm::decode_rpm_ack(&data).unwrap();
        let m = map_rpm_results(&objects, &decoded);
        assert_eq!(m["temp"], 18.0);
        assert_eq!(m["fan"], 1.0);
        assert!(m["missing"].is_nan());
        assert!(m["unanswered"].is_nan());
    }

    /// A full poll cycle against a UDP responder that answers with a
    /// standard ReadPropertyMultiple-ACK (Present_Value 85 = 0x55 and all).
    #[tokio::test]
    async fn poll_cycle_reads_a_standard_ack() {
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = device.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let (_, src) = device.recv_from(&mut buf).unwrap();
            let mut msg = vec![0x01, 0x00, 0x30, 0x01, 0x0E];
            msg.extend_from_slice(&[
                0x0C, 0x00, 0x00, 0x00, 0x05, 0x1E, 0x29, 0x55, 0x4E, 0x44, 0x41, 0x90, 0x00, 0x00,
                0x4F, 0x1F,
            ]);
            let mut out = vec![0x81, 0x0A, 0x00, 0x00];
            out.extend_from_slice(&msg);
            let len = out.len() as u16;
            out[2..4].copy_from_slice(&len.to_be_bytes());
            device.send_to(&out, src).unwrap();
        });
        let config = BacnetDeviceConfig {
            id: "bacnet_t".into(),
            ip: "127.0.0.1".into(),
            port,
            device_instance: 1,
            interval_ms: 1000,
            cov_enabled: false,
            cov_lifetime: 0,
            objects: vec![crate::models::BacnetObjectConfig {
                object_type: "analogInput".into(),
                object_instance: 5,
                property: "presentValue".into(),
                name: "temp".into(),
                cov_increment: 0.0,
            }],
        };
        let m = try_poll_cycle(&config).await.unwrap();
        assert_eq!(m["temp"], 18.0);
    }

    fn config_with(port: u16, count: u32) -> BacnetDeviceConfig {
        BacnetDeviceConfig {
            id: "bacnet_t".into(),
            ip: "127.0.0.1".into(),
            port,
            device_instance: 1,
            interval_ms: 1000,
            cov_enabled: false,
            cov_lifetime: 0,
            objects: (1..=count)
                .map(|i| crate::models::BacnetObjectConfig {
                    object_type: "analogInput".into(),
                    object_instance: i,
                    property: "presentValue".into(),
                    name: format!("p{i}"),
                    cov_increment: 0.0,
                })
                .collect(),
        }
    }

    fn send_apdu(device: &std::net::UdpSocket, src: SocketAddr, apdu: &[u8]) {
        let mut out = vec![0x81, 0x0A, 0x00, 0x00, 0x01, 0x00];
        out.extend_from_slice(apdu);
        let len = out.len() as u16;
        out[2..4].copy_from_slice(&len.to_be_bytes());
        device.send_to(&out, src).unwrap();
    }

    /// A device that answers at most `max_points` per request and aborts
    /// with segmentation-not-supported above that. Each value is its
    /// instance number. Returns the port and the request sizes seen.
    fn small_buffer_device(
        max_points: usize,
    ) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<usize>>>) {
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = device.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while let Ok((n, src)) = device.recv_from(&mut buf) {
                let invoke_id = buf[8];
                // Each spec: [0] object id (5), {1 (1), [0] property (2), }1 (1)
                let specs: Vec<&[u8]> = buf[10..n].chunks(9).collect();
                log.lock().unwrap().push(specs.len());
                if specs.len() > max_points {
                    send_apdu(&device, src, &[0x71, invoke_id, 4]);
                    continue;
                }
                let mut apdu = vec![0x30, invoke_id, 0x0E];
                for spec in specs {
                    let instance = u32::from_be_bytes([0, spec[2], spec[3], spec[4]]);
                    apdu.extend_from_slice(&spec[..5]);
                    apdu.extend_from_slice(&[0x1E, 0x29, 0x55, 0x4E, 0x44]);
                    apdu.extend_from_slice(&(instance as f32).to_be_bytes());
                    apdu.extend_from_slice(&[0x4F, 0x1F]);
                }
                send_apdu(&device, src, &apdu);
            }
        });
        (port, seen)
    }

    #[test]
    fn requests_are_split_so_answers_fit_one_apdu() {
        let points: Vec<_> = (0..200)
            .map(|i| {
                (
                    ObjectIdentifier::new(ObjectType::AnalogInput, i),
                    PropertyIdentifier::PresentValue,
                )
            })
            .collect();
        let chunks = rpm_chunks(&points, MAX_APDU);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), 200);
        for c in &chunks {
            let bytes: usize = c.iter().map(|(_, p)| ack_bytes(*p)).sum();
            assert!(ACK_HEADER + bytes <= MAX_APDU, "{bytes}");
        }
        // A point too big for any budget still gets a request of its own.
        assert_eq!(rpm_chunks(&points[..3], 10).len(), 3);
    }

    #[tokio::test]
    async fn many_points_are_read_in_several_requests() {
        let (port, seen) = small_buffer_device(usize::MAX);
        let m = try_poll_cycle(&config_with(port, 150)).await.unwrap();
        assert_eq!(m.len(), 150);
        for i in 1..=150u32 {
            assert_eq!(m[&format!("p{i}")], i as f64);
        }
        let sizes = seen.lock().unwrap().clone();
        assert!(sizes.len() >= 3, "{sizes:?}");
    }

    #[tokio::test]
    async fn a_device_with_a_smaller_buffer_gets_smaller_requests() {
        let (port, seen) = small_buffer_device(10);
        let m = try_poll_cycle(&config_with(port, 40)).await.unwrap();
        assert!(
            (1..=40u32).all(|i| m[&format!("p{i}")] == i as f64),
            "{m:?}"
        );
        let sizes = seen.lock().unwrap().clone();
        assert_eq!(sizes[0], 40, "first try is the full chunk: {sizes:?}");
        assert!(sizes.iter().filter(|n| **n <= 10).count() >= 4, "{sizes:?}");
    }

    #[tokio::test]
    async fn a_segmented_answer_fails_clearly() {
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = device.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let (_, src) = device.recv_from(&mut buf).unwrap();
            // ComplexACK, segmented + more follows, sequence 0, window 1
            send_apdu(&device, src, &[0x3C, buf[8], 0x00, 0x01, 0x0E, 0x0C]);
        });
        let err = try_poll_cycle(&config_with(port, 1)).await.unwrap_err();
        assert!(err.to_string().contains("segmented"), "{err:#}");
    }
}
