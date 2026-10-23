use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct FaultConfig {
    delay: f64,
    disconnect: f64,
    garbage: f64,
}

// ---------------------------------------------------------------------------
// Meter data – identical model to modbus-sim
// ---------------------------------------------------------------------------

struct MeterData {
    voltage: [f32; 3],
    current: [f32; 3],
    power_factor: [f32; 3],
    frequency: f32,
    energy_active: f32,
    thd_voltage: [f32; 3],
    thd_current: [f32; 3],
    load_imbalance: f32,
}

impl MeterData {
    fn new() -> Self {
        Self {
            voltage: [220.0, 219.0, 221.0],
            current: [45.0, 42.0, 47.0],
            power_factor: [0.92, 0.91, 0.93],
            frequency: 50.0,
            energy_active: 158732.0,
            thd_voltage: [1.8, 2.1, 1.6],
            thd_current: [4.5, 5.2, 3.9],
            load_imbalance: 0.1,
        }
    }

    fn tick(&mut self) {
        let mut rng = rand::rng();

        let v_common = rng.random_range(-1.5..1.5);
        for i in 0..3 {
            let drift = rng.random_range(-0.8..0.8);
            self.voltage[i] = (self.voltage[i] + v_common + drift).clamp(210.0, 230.0);
        }

        let i_common = rng.random_range(-2.0..2.0);
        for i in 0..3 {
            let imbalance = self.load_imbalance * rng.random_range(-5.0..5.0);
            self.current[i] = (self.current[i] + i_common + imbalance).clamp(5.0, 80.0);
        }

        for i in 0..3 {
            self.power_factor[i] =
                (self.power_factor[i] + rng.random_range(-0.005..0.005)).clamp(0.80, 0.99);
        }

        self.frequency = (self.frequency + rng.random_range(-0.005..0.005)).clamp(49.9, 50.1);

        for i in 0..3 {
            self.thd_voltage[i] =
                (self.thd_voltage[i] + rng.random_range(-0.2..0.2)).clamp(0.5, 5.0);
            self.thd_current[i] =
                (self.thd_current[i] + rng.random_range(-0.3..0.3)).clamp(1.0, 8.0);
        }

        let total_power: f32 = (0..3)
            .map(|p| self.voltage[p] * self.current[p] * self.power_factor[p])
            .sum();
        self.energy_active += total_power / 3600.0 / 1000.0;
    }
}

// ---------------------------------------------------------------------------
// MQTT protocol helpers
// ---------------------------------------------------------------------------

const CONNECT: u8 = 0x10;
const CONNACK: u8 = 0x20;
const PUBLISH: u8 = 0x30;
const SUBSCRIBE: u8 = 0x80;
const SUBACK: u8 = 0x90;
const PINGREQ: u8 = 0xC0;
const PINGRESP: u8 = 0xD0;
const DISCONNECT: u8 = 0xE0;

fn encode_remaining_length(mut len: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if len == 0 {
            break;
        }
    }
    buf
}

fn decode_remaining_length(buf: &[u8]) -> Option<(usize, usize)> {
    // Returns (value, bytes_consumed)
    let mut value: usize = 0;
    let mut multiplier: usize = 1;
    for (i, &byte) in buf.iter().enumerate() {
        value += ((byte & 0x7F) as usize) * multiplier;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        multiplier *= 128;
        if i >= 3 {
            return None; // malformed – more than 4 bytes
        }
    }
    None // incomplete
}

/// Read a UTF-8 encoded string (2-byte BE length prefix + bytes).
fn read_utf8_string(buf: &[u8], offset: usize) -> Option<(String, usize)> {
    if offset + 2 > buf.len() {
        return None;
    }
    let len = u16::from_be_bytes([buf[offset], buf[offset + 1]]) as usize;
    if offset + 2 + len > buf.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&buf[offset + 2..offset + 2 + len]).to_string();
    Some((s, 2 + len))
}

/// Build a PUBLISH packet (QoS 0) for the given topic and payload.
fn build_publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let topic_bytes = topic.as_bytes();
    let topic_len = topic_bytes.len() as u16;
    let remaining = 2 + topic_bytes.len() + payload.len();
    let mut packet = vec![PUBLISH]; // QoS 0, no dup, no retain
    packet.extend_from_slice(&encode_remaining_length(remaining));
    packet.extend_from_slice(&topic_len.to_be_bytes());
    packet.extend_from_slice(topic_bytes);
    packet.extend_from_slice(payload);
    packet
}

fn build_connack() -> Vec<u8> {
    vec![CONNACK, 0x02, 0x00, 0x00]
}

fn build_suback(packet_id: u16, return_codes: &[u8]) -> Vec<u8> {
    let remaining = 2 + return_codes.len();
    let mut packet = vec![SUBACK];
    packet.extend_from_slice(&encode_remaining_length(remaining));
    packet.extend_from_slice(&packet_id.to_be_bytes());
    packet.extend_from_slice(return_codes);
    packet
}

fn build_pingresp() -> Vec<u8> {
    vec![PINGRESP, 0x00]
}

// ---------------------------------------------------------------------------
// Topic matching
// ---------------------------------------------------------------------------

/// Check whether a subscription filter matches a concrete topic.
///
/// Supports:
///   - exact match ("meter/voltage_l1")
///   - single-level wildcard ("meter/+/voltage")
///   - multi-level wildcard ("meter/#")
fn topic_matches(filter: &str, topic: &str) -> bool {
    let filter_parts: Vec<&str> = filter.split('/').collect();
    let topic_parts: Vec<&str> = topic.split('/').collect();

    let mut fi = 0;
    let mut ti = 0;

    while fi < filter_parts.len() && ti < topic_parts.len() {
        match filter_parts[fi] {
            "#" => return true, // matches everything from here
            "+" => {
                // matches exactly one level
                fi += 1;
                ti += 1;
            }
            p => {
                if p != topic_parts[ti] {
                    return false;
                }
                fi += 1;
                ti += 1;
            }
        }
    }

    // Both must be exhausted, unless filter ends with "#"
    if fi < filter_parts.len() {
        // remaining filter parts must all be empty or "#"
        return filter_parts[fi] == "#";
    }

    fi == filter_parts.len() && ti == topic_parts.len()
}

// ---------------------------------------------------------------------------
// Shared broker state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PublishMsg {
    topic: String,
    payload: Vec<u8>,
}

struct BrokerState {
    tx: broadcast::Sender<PublishMsg>,
    faults: Arc<FaultConfig>,
}

// ---------------------------------------------------------------------------
// Client handler
// ---------------------------------------------------------------------------

async fn handle_client(mut stream: TcpStream, state: Arc<BrokerState>) -> Result<()> {
    let peer = stream.peer_addr()?;
    tracing::info!("Client connected: {peer}");

    let mut buf = Vec::new();
    let subscriptions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut rx = state.tx.subscribe();

    // Helper: read exactly n bytes from stream into buf
    let mut read_buf = [0u8; 4096];

    loop {
        // Try to parse a packet from buf
        if buf.len() >= 2 {
            let packet_type = buf[0] & 0xF0;
            let remaining_result = decode_remaining_length(&buf[1..]);

            if let Some((remaining_len, length_bytes)) = remaining_result {
                let total_packet = 1 + length_bytes + remaining_len;

                if buf.len() >= total_packet {
                    // We have a complete packet
                    let packet: Vec<u8> = buf.drain(..total_packet).collect();
                    let payload = &packet[1 + length_bytes..];

                    match packet_type {
                        CONNECT => {
                            tracing::debug!("CONNECT from {peer}");
                            // Validate protocol name
                            if payload.len() < 10 {
                                tracing::warn!("CONNECT payload too short from {peer}");
                                return Ok(());
                            }
                            // Protocol name: 0x00 0x04 M Q T T
                            if &payload[0..6] != b"\x00\x04MQTT" {
                                tracing::warn!("Invalid protocol name from {peer}");
                                return Ok(());
                            }
                            // Protocol level = 4
                            if payload[6] != 4 {
                                tracing::warn!(
                                    "Unsupported protocol level {} from {peer}",
                                    payload[6]
                                );
                                return Ok(());
                            }
                            // Skip connect flags (1 byte) + keep alive (2 bytes) = 3
                            // Payload starts at offset 10
                            let client_offset = 10;
                            if let Some((client_id, _)) = read_utf8_string(payload, client_offset) {
                                tracing::info!("Client ID: {client_id} ({peer})");
                            }

                            let resp = build_connack();
                            stream.write_all(&resp).await?;
                            stream.flush().await?;
                        }

                        SUBSCRIBE => {
                            tracing::debug!("SUBSCRIBE from {peer}");
                            // Fixed header flags must have reserved bit 1 set (0x02)
                            if payload.len() < 2 {
                                tracing::warn!("SUBSCRIBE payload too short from {peer}");
                                return Ok(());
                            }
                            let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
                            let mut offset = 2;
                            let mut return_codes = Vec::new();

                            while offset + 3 <= payload.len() {
                                if let Some((topic_filter, consumed)) =
                                    read_utf8_string(payload, offset)
                                {
                                    offset += consumed;
                                    if offset < payload.len() {
                                        let requested_qos = payload[offset];
                                        offset += 1;
                                        tracing::info!(
                                            "Subscription: {topic_filter} (QoS {requested_qos}) from {peer}"
                                        );
                                        subscriptions.lock().unwrap().push(topic_filter);
                                        return_codes.push(0x00); // QoS 0 granted
                                    }
                                } else {
                                    break;
                                }
                            }

                            let resp = build_suback(packet_id, &return_codes);
                            stream.write_all(&resp).await?;
                            stream.flush().await?;
                        }

                        PUBLISH => {
                            tracing::debug!("PUBLISH from {peer}");
                            if let Some((topic, consumed)) = read_utf8_string(payload, 0) {
                                let pub_payload = &payload[consumed..];
                                tracing::info!(
                                    "Publish: {topic} ({} bytes) from {peer}",
                                    pub_payload.len()
                                );
                                let _ = state.tx.send(PublishMsg {
                                    topic,
                                    payload: pub_payload.to_vec(),
                                });
                            }
                        }

                        PINGREQ => {
                            tracing::debug!("PINGREQ from {peer}");
                            let resp = build_pingresp();
                            stream.write_all(&resp).await?;
                            stream.flush().await?;
                        }

                        DISCONNECT => {
                            tracing::info!("DISCONNECT from {peer}");
                            return Ok(());
                        }

                        _ => {
                            tracing::warn!("Unknown packet type 0x{:02X} from {peer}", packet_type);
                        }
                    }

                    // After processing the packet, drain any broadcast messages before
                    // reading the next packet from the wire.
                    continue;
                }
                // else: need more data – fall through to read
            }
            // else: incomplete length encoding – fall through to read
        }

        // Before blocking on socket read, drain any queued broadcast messages.
        // We use a non-blocking approach: try_recv in a loop.
        loop {
            let msg = match rx.try_recv() {
                Ok(msg) => msg,
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!("Client {peer} lagged {n} messages");
                    continue;
                }
                Err(broadcast::error::TryRecvError::Closed) => return Ok(()),
            };

            // Determine if this client is subscribed to the topic.
            // The guard is dropped inside this block, well before any .await.
            let is_match = {
                let subs = subscriptions.lock().unwrap();
                subs.iter().any(|f| topic_matches(f, &msg.topic))
            }; // MutexGuard dropped here

            if is_match {
                let packet = build_publish_packet(&msg.topic, &msg.payload);

                // Fault injection – compute decisions in scoped block so
                // rng is dropped before any .await.
                let (should_delay, should_disconnect, should_garbage) = {
                    let mut rng = rand::rng();
                    (
                        rng.random::<f64>() < state.faults.delay,
                        rng.random::<f64>() < state.faults.disconnect,
                        rng.random::<f64>() < state.faults.garbage,
                    )
                }; // ThreadRng dropped here

                if should_delay {
                    tracing::warn!("Fault injection: delay before PUBLISH to {peer}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                if should_disconnect {
                    tracing::warn!("Fault injection: disconnecting {peer}");
                    return Ok(());
                }

                if should_garbage {
                    tracing::warn!("Fault injection: sending garbage to {peer}");
                    let garbage = {
                        let mut rng = rand::rng();
                        let len = rng.random_range(10..100usize);
                        (0..len).map(|_| rng.random::<u8>()).collect::<Vec<u8>>()
                    };
                    let _ = stream.write_all(&garbage).await;
                } else {
                    let _ = stream.write_all(&packet).await;
                }
                let _ = stream.flush().await;
            }
        }

        // Read more data from the client socket.
        let n = tokio::select! {
            result = stream.read(&mut read_buf) => result?,
        };

        if n == 0 {
            tracing::info!("Client disconnected: {peer}");
            return Ok(());
        }

        buf.extend_from_slice(&read_buf[..n]);
    }
}

// ---------------------------------------------------------------------------
// Background publisher – ticks meter data every second
// ---------------------------------------------------------------------------

async fn background_publisher(data: Arc<Mutex<MeterData>>, tx: broadcast::Sender<PublishMsg>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        let d = data.lock().unwrap();

        let avg_pf: f32 = d.power_factor.iter().sum::<f32>() / 3.0;

        let topics: Vec<(&str, String)> = vec![
            (
                "meter/voltage_l1",
                format!(r#"{{"value": {:.1}, "unit": "V"}}"#, d.voltage[0]),
            ),
            (
                "meter/voltage_l2",
                format!(r#"{{"value": {:.1}, "unit": "V"}}"#, d.voltage[1]),
            ),
            (
                "meter/voltage_l3",
                format!(r#"{{"value": {:.1}, "unit": "V"}}"#, d.voltage[2]),
            ),
            (
                "meter/current_l1",
                format!(r#"{{"value": {:.1}, "unit": "A"}}"#, d.current[0]),
            ),
            (
                "meter/current_l2",
                format!(r#"{{"value": {:.1}, "unit": "A"}}"#, d.current[1]),
            ),
            (
                "meter/current_l3",
                format!(r#"{{"value": {:.1}, "unit": "A"}}"#, d.current[2]),
            ),
            (
                "meter/power_factor",
                format!(r#"{{"value": {:.2}}}"#, avg_pf),
            ),
            (
                "meter/frequency",
                format!(r#"{{"value": {:.1}, "unit": "Hz"}}"#, d.frequency),
            ),
            (
                "meter/energy",
                format!(r#"{{"value": {:.1}, "unit": "kWh"}}"#, d.energy_active),
            ),
            (
                "meter/status",
                r#"{"running": true, "alarms": 0}"#.to_string(),
            ),
        ];

        drop(d); // release lock before broadcasting

        for (topic, payload) in &topics {
            let msg = PublishMsg {
                topic: topic.to_string(),
                payload: payload.as_bytes().to_vec(),
            };
            let _ = tx.send(msg);
        }

        tracing::trace!("Published {} meter topics", topics.len());
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("Usage: mqtt-sim [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --port PORT          TCP port to listen on (default: 1883)");
    eprintln!("  --delay PROB         Probability of injecting 500ms delay (0.0-1.0)");
    eprintln!("  --disconnect PROB    Probability of dropping client connection (0.0-1.0)");
    eprintln!("  --garbage PROB       Probability of sending malformed packet (0.0-1.0)");
    eprintln!("  --help               Show this help message");
}

fn parse_args() -> (u16, FaultConfig) {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(0);
    }

    let mut port: u16 = 1883;
    let mut faults = FaultConfig {
        delay: 0.0,
        disconnect: 0.0,
        garbage: 0.0,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid port, using default 1883");
                        1883
                    });
                }
                i += 2;
            }
            "--delay" => {
                if i + 1 < args.len() {
                    faults.delay = args[i + 1].parse().unwrap_or(0.0);
                }
                i += 2;
            }
            "--disconnect" => {
                if i + 1 < args.len() {
                    faults.disconnect = args[i + 1].parse().unwrap_or(0.0);
                }
                i += 2;
            }
            "--garbage" => {
                if i + 1 < args.len() {
                    faults.garbage = args[i + 1].parse().unwrap_or(0.0);
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    (port, faults)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let (port, faults) = parse_args();
    let addr = format!("0.0.0.0:{port}");

    let data = Arc::new(Mutex::new(MeterData::new()));
    let faults = Arc::new(faults);

    // Broadcast channel for published messages.
    // 256-slot capacity; slow clients will get lagged messages dropped.
    let (tx, _) = broadcast::channel::<PublishMsg>(256);

    let state = Arc::new(BrokerState {
        tx: tx.clone(),
        faults: faults.clone(),
    });

    // Background tasks: meter data ticker + publisher
    {
        let data_clone = data.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut d = data_clone.lock().unwrap();
                d.tick();
                tracing::debug!(
                    "V=[{:.1},{:.1},{:.1}] I=[{:.1},{:.1},{:.1}] PF={:.3} F={:.2}Hz E={:.1}kWh",
                    d.voltage[0],
                    d.voltage[1],
                    d.voltage[2],
                    d.current[0],
                    d.current[1],
                    d.current[2],
                    d.power_factor.iter().sum::<f32>() / 3.0,
                    d.frequency,
                    d.energy_active,
                );
            }
        });
    }

    tokio::spawn(background_publisher(data.clone(), tx));

    tracing::info!("MQTT 3-phase energy meter broker simulator listening on {addr}");
    tracing::info!(
        "Fault injection: delay={:.0}% disconnect={:.0}% garbage={:.0}%",
        faults.delay * 100.0,
        faults.disconnect * 100.0,
        faults.garbage * 100.0
    );

    let listener = TcpListener::bind(&addr).await?;

    loop {
        let (socket, peer) = listener.accept().await?;
        tracing::info!("Accepted connection from {peer}");

        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, state).await {
                tracing::error!("Client {peer} error: {e}");
            }
            tracing::info!("Client {peer} handler finished");
        });
    }
}
