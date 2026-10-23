//! EtherNet/IP PLC simulator — simulates an Allen-Bradley ControlLogix/CompactLogix PLC
//! over EtherNet/IP + CIP on TCP port 44818.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ── ENIP command codes ──

const CMD_REGISTER_SESSION: u16 = 0x0065;
const CMD_UNREGISTER_SESSION: u16 = 0x0066;
const CMD_SEND_RR_DATA: u16 = 0x006F;

// ── CIP service codes ──

const CIP_READ_TAG: u8 = 0x4C;
const CIP_WRITE_TAG: u8 = 0x4D;
const CIP_GET_ATTRIBUTES_ALL: u8 = 0x01;

// ── CIP data type codes ──

const CIP_TYPE_BOOL: u16 = 0x00C1;
#[allow(dead_code)]
const CIP_TYPE_SINT: u16 = 0x00C2;
#[allow(dead_code)]
const CIP_TYPE_INT: u16 = 0x00C3;
const CIP_TYPE_DINT: u16 = 0x00C4;
const CIP_TYPE_REAL: u16 = 0x00CA;

// ── CIP status codes ──

const CIP_STATUS_SUCCESS: u8 = 0x00;

// ── ENIP header size ──

const ENIP_HEADER_SIZE: usize = 24;

// ── Symbol class ID ──

const SYMBOL_CLASS_ID: u8 = 0x6B;

// ── Session handle counter ──

static NEXT_SESSION_HANDLE: AtomicU32 = AtomicU32::new(1);

// ── Tag definition ──

#[derive(Clone)]
struct TagDef {
    name: &'static str,
    type_code: u16,
    #[allow(dead_code)]
    element_size: usize,
    getter: fn(&MeterData) -> Vec<u8>,
}

// ── Meter data ──

struct MeterData {
    voltage_l1: f32,
    voltage_l2: f32,
    voltage_l3: f32,
    current_l1: f32,
    current_l2: f32,
    current_l3: f32,
    power_factor: f32,
    frequency: f32,
    energy_kwh: f32,
    thd_voltage: f32,
    thd_current: f32,
    running: bool,
    alarm_count: i32,
}

impl MeterData {
    fn new() -> Self {
        Self {
            voltage_l1: 220.0,
            voltage_l2: 219.0,
            voltage_l3: 221.0,
            current_l1: 45.0,
            current_l2: 42.0,
            current_l3: 47.0,
            power_factor: 0.92,
            frequency: 50.0,
            energy_kwh: 158732.0,
            thd_voltage: 1.8,
            thd_current: 4.5,
            running: true,
            alarm_count: 0,
        }
    }

    fn tick(&mut self) {
        let mut rng = rand::rng();

        // Voltage: slight drift
        let v_common = rng.random_range(-1.5..1.5);
        self.voltage_l1 =
            (self.voltage_l1 + v_common + rng.random_range(-0.8..0.8)).clamp(210.0, 230.0);
        self.voltage_l2 =
            (self.voltage_l2 + v_common + rng.random_range(-0.8..0.8)).clamp(210.0, 230.0);
        self.voltage_l3 =
            (self.voltage_l3 + v_common + rng.random_range(-0.8..0.8)).clamp(210.0, 230.0);

        // Current: drift
        let i_common = rng.random_range(-2.0..2.0);
        self.current_l1 =
            (self.current_l1 + i_common + rng.random_range(-5.0..5.0)).clamp(5.0, 80.0);
        self.current_l2 =
            (self.current_l2 + i_common + rng.random_range(-5.0..5.0)).clamp(5.0, 80.0);
        self.current_l3 =
            (self.current_l3 + i_common + rng.random_range(-5.0..5.0)).clamp(5.0, 80.0);

        // Power factor: slow drift
        self.power_factor = (self.power_factor + rng.random_range(-0.005..0.005)).clamp(0.80, 0.99);

        // Frequency: tight around 50Hz
        self.frequency = (self.frequency + rng.random_range(-0.005..0.005)).clamp(49.9, 50.1);

        // THD: slight variations
        self.thd_voltage = (self.thd_voltage + rng.random_range(-0.2..0.2)).clamp(0.5, 5.0);
        self.thd_current = (self.thd_current + rng.random_range(-0.3..0.3)).clamp(1.0, 8.0);

        // Energy accumulation
        let total_power: f32 = (self.voltage_l1 * self.current_l1
            + self.voltage_l2 * self.current_l2
            + self.voltage_l3 * self.current_l3)
            * self.power_factor;
        self.energy_kwh += total_power / 3_600_000.0; // W -> kWh (per second tick)
    }
}

// ── Tag table ──

fn build_tag_table() -> Vec<TagDef> {
    vec![
        TagDef {
            name: "Voltage_L1",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.voltage_l1.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Voltage_L2",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.voltage_l2.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Voltage_L3",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.voltage_l3.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Current_L1",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.current_l1.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Current_L2",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.current_l2.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Current_L3",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.current_l3.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "PowerFactor",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.power_factor.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Frequency",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.frequency.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Energy_kWh",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.energy_kwh.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "THD_Voltage",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.thd_voltage.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "THD_Current",
            type_code: CIP_TYPE_REAL,
            element_size: 4,
            getter: |d| d.thd_current.to_le_bytes().to_vec(),
        },
        TagDef {
            name: "Running",
            type_code: CIP_TYPE_BOOL,
            element_size: 1,
            getter: |d| vec![if d.running { 0xFF } else { 0x00 }, 0x00, 0x00, 0x00],
        },
        TagDef {
            name: "Alarm_Count",
            type_code: CIP_TYPE_DINT,
            element_size: 4,
            getter: |d| d.alarm_count.to_le_bytes().to_vec(),
        },
    ]
}

// ── Fault config ──

struct FaultConfig {
    delay: f64,
    disconnect: f64,
    garbage: f64,
}

// ── ENIP header helpers ──

/// Read exactly 24 bytes of ENIP encapsulation header.
/// Returns (command, body_length, session_handle, sender_context).
async fn read_enip_header(
    stream: &mut TcpStream,
) -> std::io::Result<Option<(u16, usize, u32, u64)>> {
    let mut buf = [0u8; ENIP_HEADER_SIZE];
    use tokio::io::AsyncReadExt;
    let n = AsyncReadExt::read_exact(stream, &mut buf).await;
    match n {
        Ok(_n) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let command = u16::from_le_bytes([buf[0], buf[1]]);
    let body_length = u16::from_le_bytes([buf[2], buf[3]]) as usize;
    let session_handle = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let _status = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let sender_context = u64::from_le_bytes([
        buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
    ]);
    let _options = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);

    Ok(Some((command, body_length, session_handle, sender_context)))
}

/// Build an ENIP encapsulation header + body.
fn build_enip_response(
    command: u16,
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ENIP_HEADER_SIZE + body.len());
    buf.extend_from_slice(&command.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(&session_handle.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // status = success
    buf.extend_from_slice(&sender_context.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // options
    buf.extend_from_slice(body);
    buf
}

// ── CIP reply helpers ──

/// Build a CIP success reply header (4 bytes).
fn cip_reply_header(service: u8) -> Vec<u8> {
    vec![service | 0x80, 0x00, CIP_STATUS_SUCCESS, 0x00]
}

/// Build a CIP error reply.
fn cip_error_reply(service: u8, status: u8) -> Vec<u8> {
    vec![service | 0x80, 0x00, status, 0x00]
}

/// Wrap a CIP reply in a SendRRData response frame.
fn build_send_rr_response(session_handle: u32, sender_context: u64, cip_reply: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(12 + cip_reply.len());

    // Interface handle (CIP = 0)
    body.extend_from_slice(&0u32.to_le_bytes());
    // Timeout
    body.extend_from_slice(&0u16.to_le_bytes());
    // Item count = 2
    body.extend_from_slice(&2u16.to_le_bytes());

    // Null address item
    body.extend_from_slice(&0x0000u16.to_le_bytes()); // type ID: null
    body.extend_from_slice(&0u16.to_le_bytes()); // length: 0

    // Unconnected data item
    body.extend_from_slice(&0x00B2u16.to_le_bytes()); // type ID: unconnected data
    body.extend_from_slice(&(cip_reply.len() as u16).to_le_bytes());
    body.extend_from_slice(cip_reply);

    build_enip_response(CMD_SEND_RR_DATA, session_handle, sender_context, &body)
}

// ── Tag name parsing from symbolic path ──

/// Parse a symbolic path segment (0x91) starting at `path[0]`.
/// Returns the tag name as a String.
fn parse_symbolic_tag_name(path: &[u8]) -> Option<String> {
    if path.is_empty() || path[0] != 0x91 {
        return None;
    }
    let name_len = path.get(1).copied()? as usize;
    if path.len() < 2 + name_len {
        return None;
    }
    let name_bytes = &path[2..2 + name_len];
    std::str::from_utf8(name_bytes).ok().map(|s| s.to_string())
}

// ── CIP request handling ──

/// Dispatch a CIP request and return the CIP reply bytes.
fn handle_cip_request(
    cip_request: &[u8],
    data: &Arc<Mutex<MeterData>>,
    tags: &[TagDef],
    should_garbage: bool,
) -> Vec<u8> {
    if cip_request.is_empty() {
        return cip_error_reply(0x00, 0x01);
    }

    let service = cip_request[0];

    if should_garbage {
        tracing::warn!(
            "Fault injection: returning CIP error for service 0x{:02X}",
            service
        );
        return cip_error_reply(service, 0x04); // general error
    }

    match service {
        CIP_READ_TAG => handle_read_tag(cip_request, data, tags),
        CIP_WRITE_TAG => handle_write_tag(cip_request, data, tags),
        CIP_GET_ATTRIBUTES_ALL => handle_get_attribute_all(cip_request, tags),
        _ => {
            tracing::warn!("Unsupported CIP service: 0x{:02X}", service);
            cip_error_reply(service, 0x08) // service not supported
        }
    }
}

/// Handle CIP Read Tag (service 0x4C).
fn handle_read_tag(cip_request: &[u8], data: &Arc<Mutex<MeterData>>, tags: &[TagDef]) -> Vec<u8> {
    // Layout: service(1) + path_size(1) + path(path_size*2) + num_elements(2)
    if cip_request.len() < 4 {
        return cip_error_reply(CIP_READ_TAG, 0x04);
    }

    let path_size_words = cip_request[1] as usize;
    let path_bytes = path_size_words * 2;
    if cip_request.len() < 2 + path_bytes + 2 {
        return cip_error_reply(CIP_READ_TAG, 0x04);
    }

    let tag_name = match parse_symbolic_tag_name(&cip_request[2..2 + path_bytes]) {
        Some(name) => name,
        None => {
            tracing::warn!("Failed to parse symbolic tag name from read request");
            return cip_error_reply(CIP_READ_TAG, 0x04);
        }
    };

    let num_elements =
        u16::from_le_bytes([cip_request[2 + path_bytes], cip_request[3 + path_bytes]]);

    // Find tag — case-insensitive, then fuzzy (e.g. "voltage" matches "Voltage_L1")
    let tag_idx = match tags
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(&tag_name))
        .or_else(|| {
            tags.iter()
                .position(|t| t.name.to_lowercase().contains(&tag_name.to_lowercase()))
        }) {
        Some(idx) => idx,
        None => {
            tracing::warn!("Unknown tag: {}", tag_name);
            return cip_error_reply(CIP_READ_TAG, 0x04);
        }
    };

    let tag = &tags[tag_idx];
    let meter = data.lock().unwrap();
    let value_bytes = (tag.getter)(&meter);
    drop(meter);

    let mut reply = cip_reply_header(CIP_READ_TAG);
    // Type code as u16 LE
    reply.extend_from_slice(&tag.type_code.to_le_bytes());
    // Value data (repeat for num_elements if > 1, but typically 1)
    for _ in 0..num_elements.min(1) {
        reply.extend_from_slice(&value_bytes);
    }

    tracing::debug!(
        "Read tag '{}' -> type=0x{:04X}, {} bytes",
        tag_name,
        tag.type_code,
        value_bytes.len()
    );
    reply
}

/// Handle CIP Write Tag (service 0x4D).
fn handle_write_tag(cip_request: &[u8], _data: &Arc<Mutex<MeterData>>, tags: &[TagDef]) -> Vec<u8> {
    // Layout: service(1) + path_size(1) + path(path_size*2) + type_code(2) + num_elements(2) + value
    if cip_request.len() < 4 {
        return cip_error_reply(CIP_WRITE_TAG, 0x04);
    }

    let path_size_words = cip_request[1] as usize;
    let path_bytes = path_size_words * 2;
    if cip_request.len() < 2 + path_bytes + 4 {
        return cip_error_reply(CIP_WRITE_TAG, 0x04);
    }

    let tag_name = match parse_symbolic_tag_name(&cip_request[2..2 + path_bytes]) {
        Some(name) => name,
        None => {
            tracing::warn!("Failed to parse symbolic tag name from write request");
            return cip_error_reply(CIP_WRITE_TAG, 0x04);
        }
    };

    let _type_code = u16::from_le_bytes([cip_request[2 + path_bytes], cip_request[3 + path_bytes]]);
    let _num_elements =
        u16::from_le_bytes([cip_request[4 + path_bytes], cip_request[5 + path_bytes]]);

    // Check tag exists
    if !tags.iter().any(|t| {
        t.name.eq_ignore_ascii_case(&tag_name)
            || t.name.to_lowercase().contains(&tag_name.to_lowercase())
    }) {
        tracing::warn!("Unknown tag for write: {}", tag_name);
        return cip_error_reply(CIP_WRITE_TAG, 0x04);
    }

    // For the simulator, we accept writes but don't actually update the data model
    // (the meter data ticks on its own). Log it for debugging.
    tracing::debug!(
        "Write tag '{}' accepted (simulator ignores writes)",
        tag_name
    );

    cip_reply_header(CIP_WRITE_TAG)
}

/// Handle CIP GetAttributeAll (service 0x01) for Symbol class tag browsing.
fn handle_get_attribute_all(cip_request: &[u8], tags: &[TagDef]) -> Vec<u8> {
    // Layout: service(1) + path_size(1) + path_bytes
    // Path: 0x20 class_id 0x00 0x24 instance_id 0x00 0x00
    if cip_request.len() < 4 {
        return cip_error_reply(CIP_GET_ATTRIBUTES_ALL, 0x04);
    }

    let path_size_words = cip_request[1] as usize;
    let path_bytes = path_size_words * 2;
    if cip_request.len() < 2 + path_bytes {
        return cip_error_reply(CIP_GET_ATTRIBUTES_ALL, 0x04);
    }

    let path = &cip_request[2..2 + path_bytes];

    // Parse path: expect 0x20 class_id, 0x24 instance_id
    // With padding, path is: [0x20, class_id, 0x00, 0x24, instance_id, 0x00, 0x00]
    // or for 16-bit instance: [0x20, class_id, 0x00, 0x25, 0x00, inst_lo, inst_hi, 0x00]
    let mut pos = 0;
    let mut class_id: u8 = 0;
    let mut instance_id: u32 = 0;

    while pos + 1 < path.len() {
        match path[pos] {
            0x20 => {
                // 8-bit class segment
                if pos + 2 > path.len() {
                    break;
                }
                class_id = path[pos + 1];
                pos += 2;
                // Pad to even
                if pos < path.len() && path[pos] == 0x00 {
                    pos += 1;
                }
            }
            0x24 => {
                // 8-bit instance segment
                if pos + 2 > path.len() {
                    break;
                }
                instance_id = path[pos + 1] as u32;
                pos += 2;
                if pos < path.len() && path[pos] == 0x00 {
                    pos += 1;
                }
            }
            0x25 => {
                // 16-bit instance segment
                if pos + 3 > path.len() {
                    break;
                }
                instance_id = u16::from_le_bytes([path[pos + 1], path[pos + 2]]) as u32;
                pos += 3;
            }
            0x00 => {
                // Padding
                pos += 1;
            }
            _ => {
                pos += 1;
            }
        }
    }

    if class_id != SYMBOL_CLASS_ID {
        tracing::warn!("GetAttributeAll for unknown class 0x{:02X}", class_id);
        return cip_error_reply(CIP_GET_ATTRIBUTES_ALL, 0x08);
    }

    let instance_idx = match instance_id {
        0 => {
            // Class attributes — return minimal info
            let mut reply = cip_reply_header(CIP_GET_ATTRIBUTES_ALL);
            // Minimal class attribute data
            reply.extend_from_slice(&0u16.to_le_bytes()); // revision
            reply.extend_from_slice(&(tags.len() as u16).to_le_bytes()); // max instance
            reply.extend_from_slice(&0u16.to_le_bytes()); // num attributes
            reply.extend_from_slice(&0u16.to_le_bytes()); // list attributes
            return reply;
        }
        id if id as usize <= tags.len() => (id - 1) as usize,
        _ => {
            // Instance beyond range — signal "no more instances"
            tracing::debug!(
                "GetAttributeAll instance {} beyond tag count {}",
                instance_id,
                tags.len()
            );
            return cip_error_reply(CIP_GET_ATTRIBUTES_ALL, 0x08); // object does not exist
        }
    };

    let tag = &tags[instance_idx];
    let name_bytes = tag.name.as_bytes();
    let name_len = name_bytes.len() as u16;

    // Build the response body matching what parse_symbol_instance expects.
    //
    // The client's parse_symbol_instance reads:
    //   data[0..4]  as u32 = name_len  (must be > 0 and < 256)
    //   data[4..4+name_len] as name bytes
    //   data[4+name_len+padding..] as u16 type_word (low byte = type_code, high byte = element count)
    let mut body = Vec::new();

    // Name length as u32 LE
    body.extend_from_slice(&(name_len as u32).to_le_bytes());
    // Name bytes
    body.extend_from_slice(name_bytes);
    // Pad name to even length
    if !name_bytes.len().is_multiple_of(2) {
        body.push(0x00);
    }
    // Type word: lower byte = type code, upper byte = element count (1)
    let type_word = (tag.type_code & 0xFF) | (1u16 << 8);
    body.extend_from_slice(&type_word.to_le_bytes());
    // Some trailing padding to reach comfortable size
    body.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let mut reply = cip_reply_header(CIP_GET_ATTRIBUTES_ALL);
    reply.extend_from_slice(&body);

    tracing::debug!(
        "GetAttributeAll Symbol instance {} -> tag='{}' type=0x{:04X}",
        instance_id,
        tag.name,
        tag.type_code,
    );
    reply
}

// ── Connection handler ──

async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    data: Arc<Mutex<MeterData>>,
    tags: Vec<TagDef>,
    faults: Arc<FaultConfig>,
) {
    let mut session_handle: u32 = 0;

    tracing::info!("ENIP connection from {}", addr);

    loop {
        // Read ENIP header
        let (command, body_length, _req_session, sender_context) =
            match read_enip_header(&mut stream).await {
                Ok(Some(hdr)) => hdr,
                Ok(None) => {
                    tracing::info!("ENIP client {} disconnected (EOF)", addr);
                    break;
                }
                Err(e) => {
                    tracing::warn!("ENIP read error from {}: {}", addr, e);
                    break;
                }
            };

        // Read body
        let mut body_buf = vec![0u8; body_length];
        if body_length > 0
            && let Err(e) = stream.read_exact(&mut body_buf).await
        {
            tracing::warn!("ENIP body read error from {}: {}", addr, e);
            break;
        }

        tracing::debug!(
            "ENIP command=0x{:04X} body_len={} session=0x{:08X} from {}",
            command,
            body_length,
            _req_session,
            addr,
        );

        match command {
            CMD_REGISTER_SESSION => {
                session_handle = NEXT_SESSION_HANDLE.fetch_add(1, Ordering::Relaxed);

                // Build response body: protocol version 1 + options 0
                let mut resp_body = Vec::with_capacity(4);
                resp_body.extend_from_slice(&1u16.to_le_bytes()); // protocol version
                resp_body.extend_from_slice(&0u16.to_le_bytes()); // options

                let packet = build_enip_response(
                    CMD_REGISTER_SESSION,
                    session_handle,
                    sender_context,
                    &resp_body,
                );
                if let Err(e) = stream.write_all(&packet).await {
                    tracing::warn!("Failed to send RegisterSession response to {}: {}", addr, e);
                    break;
                }
                let _ = stream.flush().await;

                tracing::info!(
                    "ENIP session registered: handle=0x{:08X} for {}",
                    session_handle,
                    addr,
                );
            }

            CMD_UNREGISTER_SESSION => {
                tracing::info!(
                    "ENIP UnregisterSession from {} (handle=0x{:08X})",
                    addr,
                    session_handle
                );
                break;
            }

            CMD_SEND_RR_DATA => {
                // Parse the SendRRData body to extract the CIP request
                let cip_request = match extract_cip_from_rr_data(&body_buf) {
                    Some(data) => data,
                    None => {
                        tracing::warn!("Failed to parse SendRRData from {}", addr);
                        break;
                    }
                };

                // Fault injection checks
                let (should_delay, should_disconnect, should_garbage) = {
                    let mut rng = rand::rng();
                    let d = rng.random::<f64>() < faults.delay;
                    let dc = rng.random::<f64>() < faults.disconnect;
                    let g = rng.random::<f64>() < faults.garbage;
                    (d, dc, g)
                };

                if should_disconnect {
                    tracing::warn!("Fault injection: dropping connection to {}", addr);
                    break;
                }

                if should_delay {
                    tracing::warn!("Fault injection: 500ms delay before response to {}", addr);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                // Handle the CIP request
                let cip_reply = handle_cip_request(&cip_request, &data, &tags, should_garbage);

                // Build and send the SendRRData response
                let packet = build_send_rr_response(session_handle, sender_context, &cip_reply);
                if let Err(e) = stream.write_all(&packet).await {
                    tracing::warn!("Failed to send SendRRData response to {}: {}", addr, e);
                    break;
                }
                let _ = stream.flush().await;
            }

            _ => {
                tracing::warn!("Unknown ENIP command 0x{:04X} from {}", command, addr);
                // Send error response
                let packet = build_enip_response(command, session_handle, sender_context, &[]);
                let _ = stream.write_all(&packet).await;
                let _ = stream.flush().await;
            }
        }
    }

    tracing::info!("ENIP connection closed: {}", addr);
}

/// Extract the CIP payload from a SendRRData body.
fn extract_cip_from_rr_data(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 6 {
        return None;
    }

    // interface_handle: u32 (skip)
    // timeout: u16 (skip)
    let _interface_handle = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let _timeout = u16::from_le_bytes([body[4], body[5]]);
    let item_count = u16::from_le_bytes([body[6], body[7]]);

    let mut pos = 8;

    // We expect 2 items: null address + unconnected data
    let mut cip_data: Option<Vec<u8>> = None;

    for _ in 0..item_count {
        if pos + 4 > body.len() {
            break;
        }
        let type_id = u16::from_le_bytes([body[pos], body[pos + 1]]);
        let item_len = u16::from_le_bytes([body[pos + 2], body[pos + 3]]) as usize;
        pos += 4;

        if pos + item_len > body.len() {
            break;
        }

        if type_id == 0x00B2 {
            // Unconnected data item — this is the CIP request
            cip_data = Some(body[pos..pos + item_len].to_vec());
        }

        pos += item_len;
    }

    cip_data
}

// ── CLI ──

fn parse_args() -> (u16, FaultConfig) {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 44818;
    let mut faults = FaultConfig {
        delay: 0.0,
        disconnect: 0.0,
        garbage: 0.0,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("Invalid port, using default 44818");
                        44818
                    });
                i += 2;
            }
            "--delay" => {
                faults.delay = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--disconnect" => {
                faults.disconnect = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--garbage" => {
                faults.garbage = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: enip-sim [--port PORT] [--delay PROB] [--disconnect PROB] [--garbage PROB]"
                );
                eprintln!("Default port: 44818");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                i += 1;
            }
        }
    }
    (port, faults)
}

// ── Main ──

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let (port, faults) = parse_args();
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let data = Arc::new(Mutex::new(MeterData::new()));
    let faults = Arc::new(faults);
    let tags = build_tag_table();

    // Background task: update meter values every second
    {
        let data = data.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut d = data.lock().unwrap();
                d.tick();
                tracing::debug!(
                    "V=[{:.1},{:.1},{:.1}] I=[{:.1},{:.1},{:.1}] PF={:.3} F={:.2}Hz E={:.1}kWh",
                    d.voltage_l1,
                    d.voltage_l2,
                    d.voltage_l3,
                    d.current_l1,
                    d.current_l2,
                    d.current_l3,
                    d.power_factor,
                    d.frequency,
                    d.energy_kwh,
                );
            }
        });
    }

    tracing::info!("EtherNet/IP PLC simulator listening on {}", addr);
    tracing::info!(
        "Fault injection: delay={:.0}% disconnect={:.0}% garbage={:.0}%",
        faults.delay * 100.0,
        faults.disconnect * 100.0,
        faults.garbage * 100.0,
    );
    tracing::info!(
        "Tags: {}",
        tags.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
    );

    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, client_addr) = listener.accept().await?;
        let data = data.clone();
        let faults = faults.clone();
        let tags = tags.clone();

        tokio::spawn(async move {
            handle_connection(stream, client_addr, data, tags, faults).await;
        });
    }
}
