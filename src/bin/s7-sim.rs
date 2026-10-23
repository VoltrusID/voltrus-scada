#![allow(clippy::type_complexity)]
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rand::Rng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ── Protocol Constants ──

const TPKT_VERSION: u8 = 3;
const COTP_CR: u8 = 0xE0;
const _COTP_CC: u8 = 0xD0;
const COTP_DT: u8 = 0xF0;

const S7_PROTOCOL_ID: u8 = 0x32;
const S7_MSG_TYPE_JOB: u8 = 0x01;
const S7_MSG_TYPE_ACK_DATA: u8 = 0x03;

const S7_SETUP_COMM: u8 = 0xF0;
const S7_READ_VAR: u8 = 0x04;
const S7_WRITE_VAR: u8 = 0x05;

const AREA_DB: u8 = 0x84;
const AREA_INPUT: u8 = 0x81;
const AREA_OUTPUT: u8 = 0x82;
const AREA_MARKER: u8 = 0x83;

// ── Fault Injection ──

struct FaultConfig {
    delay: f64,
    disconnect: f64,
    garbage: f64,
}

// ── Data Store ──

const DB_SIZE: usize = 256;
const IO_SIZE: usize = 256;

struct DataStore {
    db1: [u8; DB_SIZE],
    inputs: [u8; IO_SIZE],
    outputs: [u8; IO_SIZE],
    markers: [u8; IO_SIZE],
}

impl DataStore {
    fn new() -> Self {
        Self {
            db1: [0u8; DB_SIZE],
            inputs: [0u8; IO_SIZE],
            outputs: [0u8; IO_SIZE],
            markers: [0u8; IO_SIZE],
        }
    }

    fn _get_area(&mut self, area_code: u8, db_number: u16) -> Option<&mut [u8]> {
        match area_code {
            AREA_DB if db_number == 1 => Some(&mut self.db1),
            AREA_DB => None, // only DB1 supported
            AREA_INPUT => Some(&mut self.inputs),
            AREA_OUTPUT => Some(&mut self.outputs),
            AREA_MARKER => Some(&mut self.markers),
            _ => None,
        }
    }

    fn read_bytes(
        &self,
        area_code: u8,
        db_number: u16,
        offset: u16,
        count: u16,
    ) -> Option<Vec<u8>> {
        let area: &[u8] = match area_code {
            AREA_DB if db_number == 1 => &self.db1,
            AREA_DB => return None,
            AREA_INPUT => &self.inputs,
            AREA_OUTPUT => &self.outputs,
            AREA_MARKER => &self.markers,
            _ => return None,
        };
        let start = offset as usize;
        let end = start + count as usize;
        if end <= area.len() {
            Some(area[start..end].to_vec())
        } else if start < area.len() {
            Some(area[start..].to_vec())
        } else {
            None
        }
    }

    fn write_bytes(&mut self, area_code: u8, db_number: u16, offset: u16, data: &[u8]) -> bool {
        let area = match area_code {
            AREA_DB if db_number == 1 => &mut self.db1,
            AREA_DB => return false,
            AREA_INPUT => &mut self.inputs,
            AREA_OUTPUT => &mut self.outputs,
            AREA_MARKER => &mut self.markers,
            _ => return false,
        };
        let start = offset as usize;
        let end = start + data.len();
        if end <= area.len() {
            area[start..end].copy_from_slice(data);
            true
        } else {
            false
        }
    }
}

// ── Meter Data ──

struct MeterData {
    store: DataStore,
}

impl MeterData {
    fn new() -> Self {
        let mut md = Self {
            store: DataStore::new(),
        };
        md.init_meter_values();
        md
    }

    fn init_meter_values(&mut self) {
        let store = &mut self.store;
        // voltage_l1 (offset 0)
        store.db1[0..4].copy_from_slice(&220.0f32.to_be_bytes());
        // voltage_l2 (offset 4)
        store.db1[4..8].copy_from_slice(&219.0f32.to_be_bytes());
        // voltage_l3 (offset 8)
        store.db1[8..12].copy_from_slice(&221.0f32.to_be_bytes());
        // current_l1 (offset 12)
        store.db1[12..16].copy_from_slice(&45.0f32.to_be_bytes());
        // current_l2 (offset 16)
        store.db1[16..20].copy_from_slice(&42.0f32.to_be_bytes());
        // current_l3 (offset 20)
        store.db1[20..24].copy_from_slice(&47.0f32.to_be_bytes());
        // power_factor (offset 24)
        store.db1[24..28].copy_from_slice(&0.92f32.to_be_bytes());
        // frequency (offset 28)
        store.db1[28..32].copy_from_slice(&50.0f32.to_be_bytes());
        // energy_active (offset 32)
        store.db1[32..36].copy_from_slice(&158732.0f32.to_be_bytes());
        // thd_voltage (offset 36)
        store.db1[36..40].copy_from_slice(&1.8f32.to_be_bytes());
        // thd_current (offset 40)
        store.db1[40..44].copy_from_slice(&4.5f32.to_be_bytes());
    }

    fn read_f32(&self, offset: usize) -> f32 {
        let store = &self.store;
        if offset + 4 <= store.db1.len() {
            f32::from_be_bytes([
                store.db1[offset],
                store.db1[offset + 1],
                store.db1[offset + 2],
                store.db1[offset + 3],
            ])
        } else {
            0.0
        }
    }

    fn write_f32(&mut self, offset: usize, val: f32) {
        let store = &mut self.store;
        if offset + 4 <= store.db1.len() {
            store.db1[offset..offset + 4].copy_from_slice(&val.to_be_bytes());
        }
    }

    fn tick(&mut self) {
        let mut rng = rand::rng();

        // Voltage: slight per-phase drift + common mode variation
        let v_common: f32 = rng.random_range(-1.5..1.5);
        for i in 0..3 {
            let offset = i * 4;
            let v = self.read_f32(offset);
            let drift: f32 = rng.random_range(-0.8..0.8);
            self.write_f32(offset, (v + v_common + drift).clamp(210.0, 230.0));
        }

        // Current: drift with load imbalance
        let i_common: f32 = rng.random_range(-2.0..2.0);
        for i in 0..3 {
            let offset = 12 + i * 4;
            let c = self.read_f32(offset);
            let imbalance: f32 = 0.1 * rng.random_range(-5.0..5.0);
            self.write_f32(offset, (c + i_common + imbalance).clamp(5.0, 80.0));
        }

        // Power factor: slow drift
        {
            let pf = self.read_f32(24);
            let drift: f32 = rng.random_range(-0.005..0.005);
            self.write_f32(24, (pf + drift).clamp(0.80, 0.99));
        }

        // Frequency: tight around 50Hz
        {
            let freq = self.read_f32(28);
            let drift: f32 = rng.random_range(-0.005..0.005);
            self.write_f32(28, (freq + drift).clamp(49.9, 50.1));
        }

        // THD voltage: slight variation
        {
            let thd = self.read_f32(36);
            let drift: f32 = rng.random_range(-0.2..0.2);
            self.write_f32(36, (thd + drift).clamp(0.5, 5.0));
        }

        // THD current: slight variation
        {
            let thd = self.read_f32(40);
            let drift: f32 = rng.random_range(-0.3..0.3);
            self.write_f32(40, (thd + drift).clamp(1.0, 8.0));
        }

        // Energy accumulation: total_power / 3600 / 1000 (W -> kWh per second tick)
        let v1 = self.read_f32(0);
        let v2 = self.read_f32(4);
        let v3 = self.read_f32(8);
        let c1 = self.read_f32(12);
        let c2 = self.read_f32(16);
        let c3 = self.read_f32(20);
        let pf = self.read_f32(24);
        let total_power = (v1 * c1 + v2 * c2 + v3 * c3) * pf;
        let energy = self.read_f32(32);
        self.write_f32(32, energy + total_power / 3600000.0);
    }
}

// ── TPKT / COTP helpers ──

/// Read a full TPKT packet: returns the payload after the 4-byte TPKT header.
async fn read_tpkt_packet(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut tpkt = [0u8; 4];
    stream.read_exact(&mut tpkt).await?;

    if tpkt[0] != TPKT_VERSION {
        anyhow::bail!("Invalid TPKT version: 0x{:02X}", tpkt[0]);
    }

    let total_len = u16::from_be_bytes([tpkt[2], tpkt[3]]) as usize;
    if total_len < 4 {
        anyhow::bail!("Invalid TPKT packet length: {}", total_len);
    }

    let payload_len = total_len - 4;
    let mut payload = vec![0u8; payload_len];
    stream.read_exact(&mut payload).await?;

    Ok(payload)
}

/// Wrap S7 frame bytes in COTP DT + TPKT for sending.
fn wrap_s7_in_cotp_dt_tpkt(s7_frame: &[u8]) -> Vec<u8> {
    // COTP DT: [length=2, type=0xF0, EOT=0x80]
    let cotp_dt = [0x02u8, COTP_DT, 0x80];
    let total_len = 4 + cotp_dt.len() + s7_frame.len();
    let mut packet = Vec::with_capacity(total_len);
    packet.push(TPKT_VERSION);
    packet.push(0x00);
    packet.extend_from_slice(&(total_len as u16).to_be_bytes());
    packet.extend_from_slice(&cotp_dt);
    packet.extend_from_slice(s7_frame);
    packet
}

// ── S7 frame builders ──

fn build_s7_setup_comm_response(pdu_ref: u16) -> Vec<u8> {
    // Params: function=0xF0, reserved, max AMQ calling=1, max AMQ called=1, PDU size=480
    let params = [
        S7_SETUP_COMM,
        0x00,
        0x00,
        0x01, // max AMQ calling
        0x00,
        0x01, // max AMQ called
        0x01,
        0xE0, // PDU size = 480
    ];
    // Data: 4 zeros
    let data = [0x00u8; 4];

    let mut frame = Vec::with_capacity(12 + params.len() + data.len());
    frame.push(S7_PROTOCOL_ID);
    frame.push(S7_MSG_TYPE_ACK_DATA);
    frame.extend_from_slice(&[0x00, 0x00]); // reserved
    frame.extend_from_slice(&pdu_ref.to_be_bytes());
    frame.extend_from_slice(&(params.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // error code = 0 (success)
    frame.extend_from_slice(&params);
    frame.extend_from_slice(&data);
    frame
}

fn build_s7_read_var_response(
    pdu_ref: u16,
    items: &[(u8, u8, u16, u16, u8, u16)], // (transport_size, _unused, num_elements, db_number, area_code, offset)
    store: &DataStore,
    should_garbage: bool,
) -> Vec<u8> {
    let item_count = items.len() as u8;

    // Params: [0x04, item_count]
    let params = vec![S7_READ_VAR, item_count];

    // Data: for each item, return_code + transport_size + data_length + data
    let mut data = Vec::new();

    for &(transport_size, _unused, num_elements, db_number, area_code, byte_offset) in items {
        // Determine byte count from transport_size and num_elements
        let byte_count = match transport_size {
            0x01 => (num_elements as usize).div_ceil(8), // bit
            0x02 => num_elements as usize,               // byte
            0x04 => num_elements as usize * 2,           // word
            0x06 => num_elements as usize * 4,           // dword
            0x08 => num_elements as usize * 4,           // real
            _ => num_elements as usize,                  // fallback: byte
        };

        let area_bytes = if should_garbage {
            let mut rng = rand::rng();
            (0..byte_count)
                .map(|_| rng.random::<u8>())
                .collect::<Vec<u8>>()
        } else {
            match store.read_bytes(area_code, db_number, byte_offset, byte_count as u16) {
                Some(b) => b,
                None => vec![0u8; byte_count],
            }
        };

        data.push(0xFF); // return code: success
        data.push(0xFF); // transport size: octet string
        data.extend_from_slice(&(area_bytes.len() as u16).to_be_bytes());
        data.extend_from_slice(&area_bytes);
    }

    let mut frame = Vec::with_capacity(12 + params.len() + data.len());
    frame.push(S7_PROTOCOL_ID);
    frame.push(S7_MSG_TYPE_ACK_DATA);
    frame.extend_from_slice(&[0x00, 0x00]); // reserved
    frame.extend_from_slice(&pdu_ref.to_be_bytes());
    frame.extend_from_slice(&(params.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // error code = 0 (success)
    frame.extend_from_slice(&params);
    frame.extend_from_slice(&data);
    frame
}

fn build_s7_write_var_response(pdu_ref: u16, item_count: u8) -> Vec<u8> {
    // Params: [0x05, item_count]
    let params = [S7_WRITE_VAR, item_count];

    // Data: return code for each item (all success)
    let data = vec![0xFFu8; item_count as usize];

    let mut frame = Vec::with_capacity(12 + params.len() + data.len());
    frame.push(S7_PROTOCOL_ID);
    frame.push(S7_MSG_TYPE_ACK_DATA);
    frame.extend_from_slice(&[0x00, 0x00]); // reserved
    frame.extend_from_slice(&pdu_ref.to_be_bytes());
    frame.extend_from_slice(&(params.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    frame.extend_from_slice(&0x0000u16.to_be_bytes()); // error code = 0 (success)
    frame.extend_from_slice(&params);
    frame.extend_from_slice(&data);
    frame
}

// ── S7 frame parsing ──

/// Parse an S7 read var request, returning (item_count, vec of parsed items).
/// Each item: (transport_size, num_elements, db_number, area_code, byte_offset)
fn parse_read_var_items(s7: &[u8]) -> Option<(u8, Vec<(u8, u16, u16, u8, u16)>)> {
    // S7 header: 10 bytes
    // Params start at byte 10
    if s7.len() < 12 {
        return None;
    }

    let param_len = u16::from_be_bytes([s7[6], s7[7]]) as usize;
    let params_start = 10;

    if s7.len() < params_start + 2 {
        return None;
    }

    let function = s7[params_start];
    if function != S7_READ_VAR {
        return None;
    }

    let item_count = s7[params_start + 1];
    let mut items = Vec::new();
    let mut pos = params_start + 2;

    for _ in 0..item_count {
        if pos + 12 > params_start + param_len {
            break;
        }

        // Item: [0x12, 0x0A, 0x10, transport_size, num_elements(2), db_number(2), area_code, offset(3)]
        let _spec = s7[pos];
        let _len = s7[pos + 1];
        let _syntax = s7[pos + 2];
        let transport_size = s7[pos + 3];
        let num_elements = u16::from_be_bytes([s7[pos + 4], s7[pos + 5]]);
        let db_number = u16::from_be_bytes([s7[pos + 6], s7[pos + 7]]);
        let area_code = s7[pos + 8];
        let offset_bits =
            ((s7[pos + 9] as u32) << 16) | ((s7[pos + 10] as u32) << 8) | (s7[pos + 11] as u32);
        // Convert bit offset to byte offset
        let byte_offset = (offset_bits / 8) as u16;

        items.push((
            transport_size,
            num_elements,
            db_number,
            area_code,
            byte_offset,
        ));
        pos += 12;
    }

    Some((item_count, items))
}

/// Parse a write var request. Returns (item_count, vec of (transport_size, num_elements, db_number, area_code, byte_offset, data_bytes))
fn parse_write_var_items(s7: &[u8]) -> Option<(u8, Vec<(u8, u16, u16, u8, u16, Vec<u8>)>)> {
    if s7.len() < 12 {
        return None;
    }

    let param_len = u16::from_be_bytes([s7[6], s7[7]]) as usize;
    let _data_len = u16::from_be_bytes([s7[8], s7[9]]) as usize;
    let params_start = 10;
    let data_start = params_start + param_len;

    if s7.len() < params_start + 2 {
        return None;
    }

    let function = s7[params_start];
    if function != S7_WRITE_VAR {
        return None;
    }

    let item_count = s7[params_start + 1];

    // Parse items from params
    let mut param_items = Vec::new();
    let mut pos = params_start + 2;

    for _ in 0..item_count {
        if pos + 12 > params_start + param_len {
            break;
        }
        let transport_size = s7[pos + 3];
        let num_elements = u16::from_be_bytes([s7[pos + 4], s7[pos + 5]]);
        let db_number = u16::from_be_bytes([s7[pos + 6], s7[pos + 7]]);
        let area_code = s7[pos + 8];
        let offset_bits =
            ((s7[pos + 9] as u32) << 16) | ((s7[pos + 10] as u32) << 8) | (s7[pos + 11] as u32);
        let byte_offset = (offset_bits / 8) as u16;

        param_items.push((
            transport_size,
            num_elements,
            db_number,
            area_code,
            byte_offset,
        ));
        pos += 12;
    }

    // Parse data for each item
    let mut results = Vec::new();
    let mut data_pos = data_start;

    for (transport_size, num_elements, db_number, area_code, byte_offset) in param_items {
        if data_pos + 4 > s7.len() {
            break;
        }
        let _return_code = s7[data_pos];
        let _ts = s7[data_pos + 1];
        let write_data_len = u16::from_be_bytes([s7[data_pos + 2], s7[data_pos + 3]]) as usize;
        data_pos += 4;

        if data_pos + write_data_len > s7.len() {
            break;
        }
        let data_bytes = s7[data_pos..data_pos + write_data_len].to_vec();
        data_pos += write_data_len;

        results.push((
            transport_size,
            num_elements,
            db_number,
            area_code,
            byte_offset,
            data_bytes,
        ));
    }

    Some((item_count, results))
}

// ── Connection Handler ──

async fn handle_connection(
    mut stream: TcpStream,
    addr: std::net::SocketAddr,
    meter_data: Arc<Mutex<MeterData>>,
    faults: Arc<FaultConfig>,
) -> Result<()> {
    tracing::info!("New S7 connection from {addr}");

    // Phase 1: COTP Connection Request -> Confirm
    let cotp_payload = read_tpkt_packet(&mut stream).await?;
    if cotp_payload.is_empty() {
        anyhow::bail!("Empty COTP payload");
    }

    let pdu_type = cotp_payload[1];
    if pdu_type != COTP_CR {
        tracing::warn!("Expected COTP CR (0xE0), got 0x{:02X}", pdu_type);
        anyhow::bail!("Expected COTP CR, got 0x{:02X}", pdu_type);
    }

    // Send COTP CC (Connection Confirm)
    let cc_response: [u8; 11] = [
        0x03, 0x00, 0x00, 0x0B, // TPKT: version=3, total=11
        0x06, 0xD0, 0x00, 0x01, 0x00, 0x01, 0x00, // COTP CC
    ];
    stream.write_all(&cc_response).await?;
    tracing::debug!("Sent COTP CC to {addr}");

    // Phase 2 & 3: S7 communication loop
    loop {
        let packet = match read_tpkt_packet(&mut stream).await {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!("Client {addr} disconnected: {e}");
                break;
            }
        };

        // Skip COTP DT header (typically [0x02, 0xF0, 0x80])
        if packet.len() < 3 {
            tracing::warn!("COTP DT payload too short from {addr}");
            break;
        }
        let cotp_len = packet[0] as usize;
        if packet.len() < cotp_len + 1 {
            tracing::warn!("COTP header truncated from {addr}");
            break;
        }
        // Verify DT type
        let dt_type = packet[cotp_len - 1];
        if dt_type != COTP_DT {
            tracing::warn!("Expected COTP DT (0xF0), got 0x{:02X} from {addr}", dt_type);
            break;
        }
        let s7_frame = &packet[cotp_len + 1..];

        if s7_frame.len() < 10 {
            tracing::warn!("S7 frame too short ({}) from {addr}", s7_frame.len());
            break;
        }

        if s7_frame[0] != S7_PROTOCOL_ID {
            tracing::warn!("Invalid S7 protocol ID: 0x{:02X} from {addr}", s7_frame[0]);
            break;
        }

        let msg_type = s7_frame[1];
        let pdu_ref = u16::from_be_bytes([s7_frame[4], s7_frame[5]]);
        let _param_len = u16::from_be_bytes([s7_frame[6], s7_frame[7]]) as usize;

        if msg_type != S7_MSG_TYPE_JOB {
            tracing::warn!(
                "Unsupported S7 message type: 0x{:02X} from {addr}",
                msg_type
            );
            continue;
        }

        // Get function code from params
        if s7_frame.len() < 11 {
            tracing::warn!("S7 frame too short for params from {addr}");
            break;
        }
        let function = s7_frame[10];

        // Fault injection checks
        let should_delay = {
            let mut rng = rand::rng();
            rng.random::<f64>() < faults.delay
        };
        let should_disconnect = {
            let mut rng = rand::rng();
            rng.random::<f64>() < faults.disconnect
        };
        let should_garbage = {
            let mut rng = rand::rng();
            rng.random::<f64>() < faults.garbage
        };

        if should_disconnect {
            tracing::warn!("Fault injection: dropping connection from {addr}");
            break;
        }

        if should_delay {
            tracing::warn!("Fault injection: delaying response to {addr}");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let response_frame = match function {
            S7_SETUP_COMM => {
                tracing::debug!("S7 Setup Communication from {addr}");
                build_s7_setup_comm_response(pdu_ref)
            }
            S7_READ_VAR => {
                tracing::debug!("S7 Read Var from {addr} (pdu_ref={pdu_ref})");
                let meter = meter_data.lock().unwrap();
                if let Some((_count, items)) = parse_read_var_items(s7_frame) {
                    let item_tuples: Vec<(u8, u8, u16, u16, u8, u16)> = items
                        .into_iter()
                        .map(|(ts, ne, db, area, off)| (ts, 0u8, ne, db, area, off))
                        .collect();
                    build_s7_read_var_response(pdu_ref, &item_tuples, &meter.store, should_garbage)
                } else {
                    tracing::warn!("Failed to parse Read Var items from {addr}");
                    continue;
                }
            }
            S7_WRITE_VAR => {
                tracing::debug!("S7 Write Var from {addr} (pdu_ref={pdu_ref})");
                if let Some((item_count, items)) = parse_write_var_items(s7_frame) {
                    let mut meter = meter_data.lock().unwrap();
                    for (_ts, _ne, db_number, area_code, byte_offset, data_bytes) in &items {
                        meter
                            .store
                            .write_bytes(*area_code, *db_number, *byte_offset, data_bytes);
                    }
                    drop(meter);
                    build_s7_write_var_response(pdu_ref, item_count)
                } else {
                    tracing::warn!("Failed to parse Write Var items from {addr}");
                    continue;
                }
            }
            _ => {
                tracing::warn!("Unsupported S7 function: 0x{:02X} from {addr}", function);
                continue;
            }
        };

        let response_packet = wrap_s7_in_cotp_dt_tpkt(&response_frame);
        if let Err(e) = stream.write_all(&response_packet).await {
            tracing::warn!("Failed to send response to {addr}: {e}");
            break;
        }
    }

    tracing::info!("S7 connection closed from {addr}");
    let _ = stream.shutdown().await;
    Ok(())
}

// ── CLI ──

fn parse_args() -> (u16, FaultConfig) {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 10200;
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
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("Invalid port, using default 10200");
                        10200
                    });
                i += 2;
            }
            "--delay" => {
                faults.delay = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--disconnect" => {
                faults.disconnect = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--garbage" => {
                faults.garbage = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            other => {
                eprintln!("Unknown argument: {other}");
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
    let addr: std::net::SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let data = Arc::new(Mutex::new(MeterData::new()));
    let faults = Arc::new(faults);

    // Background task: tick meter values every second
    {
        let data = data.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut d = data.lock().unwrap();
                d.tick();
                tracing::debug!(
                    "V=[{:.1},{:.1},{:.1}] I=[{:.1},{:.1},{:.1}] PF={:.3} F={:.2}Hz E={:.1}kWh",
                    d.read_f32(0),
                    d.read_f32(4),
                    d.read_f32(8),
                    d.read_f32(12),
                    d.read_f32(16),
                    d.read_f32(20),
                    d.read_f32(24),
                    d.read_f32(28),
                    d.read_f32(32),
                );
            }
        });
    }

    tracing::info!("S7 PLC simulator listening on {addr}");
    tracing::info!(
        "Fault injection: delay={:.0}% disconnect={:.0}% garbage={:.0}%",
        faults.delay * 100.0,
        faults.disconnect * 100.0,
        faults.garbage * 100.0
    );

    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, client_addr) = listener.accept().await?;
        let data = data.clone();
        let faults = faults.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, client_addr, data, faults).await {
                tracing::error!("Connection error from {client_addr}: {e}");
            }
        });
    }
}
