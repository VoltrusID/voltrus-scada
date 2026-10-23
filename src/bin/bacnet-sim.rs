//! BACnet/IP device simulator — responds to Who-Is, I-Am, ReadProperty,
//! ReadPropertyMultiple, and WriteProperty requests over UDP.

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::Rng;

// ── BACnet constants ──

const BVLC_TYPE: u8 = 0x81;
const BVLC_ORIGINAL_UNICAST: u8 = 0x0A;
const BVLC_ORIGINAL_BROADCAST: u8 = 0x0B;

const APDU_UNCONFIRMED_REQUEST: u8 = 0x10;
const APDU_CONFIRMED_REQUEST: u8 = 0x00;
const APDU_COMPLEX_ACK: u8 = 0x30;
const APDU_SIMPLE_ACK: u8 = 0x20;

const SVC_WHO_IS: u8 = 0x08;
const SVC_I_AM: u8 = 0x00;
const SVC_READ_PROPERTY: u8 = 0x0C;
const SVC_READ_PROPERTY_MULTIPLE: u8 = 0x0E;
const SVC_WRITE_PROPERTY: u8 = 0x0F;

const OBJ_TYPE_ANALOG_INPUT: u16 = 0;
const OBJ_TYPE_ANALOG_OUTPUT: u16 = 1;
const OBJ_TYPE_ANALOG_VALUE: u16 = 2;
const OBJ_TYPE_BINARY_INPUT: u16 = 3;
const OBJ_TYPE_BINARY_OUTPUT: u16 = 4;
const OBJ_TYPE_BINARY_VALUE: u16 = 5;
const OBJ_TYPE_MULTI_STATE_INPUT: u16 = 13;
const OBJ_TYPE_MULTI_STATE_OUTPUT: u16 = 14;
const OBJ_TYPE_MULTI_STATE_VALUE: u16 = 19;
const OBJ_TYPE_DEVICE: u16 = 8;

const PROP_PRESENT_VALUE: u32 = 85;
const PROP_OBJECT_NAME: u32 = 77;
const PROP_DESCRIPTION: u32 = 28;
const PROP_OBJECT_LIST: u32 = 76;
const PROP_VENDOR_NAME: u32 = 121;
const PROP_VENDOR_ID: u32 = 120;
const PROP_MODEL_NAME: u32 = 70;
const PROP_FIRMWARE_REV: u32 = 44;
const PROP_STATUS_FLAGS: u32 = 111;

// ── Simulated object ──

#[derive(Clone)]
struct SimObject {
    object_type: u16,
    instance: u32,
    name: String,
    description: String,
    value: f64,
    min: f64,
    max: f64,
    drift: f64,
}

impl SimObject {
    fn object_identifier(&self) -> u32 {
        ((self.object_type as u32) << 22) | self.instance
    }

    fn tick(&mut self) {
        if self.drift == 0.0 {
            return;
        }
        let mut rng = rand::rng();
        let delta = rng.random_range(-self.drift..self.drift);
        self.value = (self.value + delta).clamp(self.min, self.max);
    }
}

// ── Device state ──

struct DeviceState {
    device_instance: u32,
    vendor_id: u16,
    vendor_name: String,
    model_name: String,
    firmware: String,
    objects: Vec<SimObject>,
}

impl DeviceState {
    fn new(device_instance: u32) -> Self {
        let objects = vec![
            SimObject {
                object_type: OBJ_TYPE_ANALOG_INPUT,
                instance: 1,
                name: "ZoneTemp_1".into(),
                description: "Zone 1 Temperature".into(),
                value: 22.5,
                min: 18.0,
                max: 28.0,
                drift: 0.3,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_INPUT,
                instance: 2,
                name: "ZoneTemp_2".into(),
                description: "Zone 2 Temperature".into(),
                value: 23.1,
                min: 18.0,
                max: 28.0,
                drift: 0.3,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_INPUT,
                instance: 3,
                name: "SupplyAirTemp".into(),
                description: "Supply Air Temperature".into(),
                value: 15.0,
                min: 10.0,
                max: 25.0,
                drift: 0.2,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_INPUT,
                instance: 4,
                name: "Humidity".into(),
                description: "Relative Humidity".into(),
                value: 55.0,
                min: 30.0,
                max: 80.0,
                drift: 1.0,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_OUTPUT,
                instance: 1,
                name: "CoolingSetpoint".into(),
                description: "Cooling Setpoint".into(),
                value: 24.0,
                min: 16.0,
                max: 30.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_OUTPUT,
                instance: 2,
                name: "DamperPosition".into(),
                description: "Damper Position".into(),
                value: 50.0,
                min: 0.0,
                max: 100.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_ANALOG_VALUE,
                instance: 1,
                name: "OccupiedSetpoint".into(),
                description: "Occupied Setpoint".into(),
                value: 22.0,
                min: 16.0,
                max: 30.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_BINARY_INPUT,
                instance: 1,
                name: "FanStatus".into(),
                description: "Supply Fan Status".into(),
                value: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_BINARY_INPUT,
                instance: 2,
                name: "FilterAlarm".into(),
                description: "Filter Alarm".into(),
                value: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_BINARY_OUTPUT,
                instance: 1,
                name: "FanCmd".into(),
                description: "Fan Command".into(),
                value: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_BINARY_VALUE,
                instance: 1,
                name: "OccupiedMode".into(),
                description: "Occupied Mode".into(),
                value: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_MULTI_STATE_INPUT,
                instance: 1,
                name: "SystemMode".into(),
                description: "System Mode (1=Off 2=Cool 3=Heat 4=Auto)".into(),
                value: 4.0,
                min: 1.0,
                max: 4.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_MULTI_STATE_OUTPUT,
                instance: 1,
                name: "FanSpeed".into(),
                description: "Fan Speed (1=Low 2=Med 3=High)".into(),
                value: 2.0,
                min: 1.0,
                max: 3.0,
                drift: 0.0,
            },
            SimObject {
                object_type: OBJ_TYPE_MULTI_STATE_VALUE,
                instance: 1,
                name: "Priority".into(),
                description: "Priority Level (1-16)".into(),
                value: 8.0,
                min: 1.0,
                max: 16.0,
                drift: 0.0,
            },
        ];

        Self {
            device_instance,
            vendor_id: 999,
            vendor_name: "Voltrus Sim".into(),
            model_name: "BACnet-SIM-100".into(),
            firmware: "1.0.0".into(),
            objects,
        }
    }

    fn find_object(&self, obj_type: u16, instance: u32) -> Option<usize> {
        self.objects
            .iter()
            .position(|o| o.object_type == obj_type && o.instance == instance)
    }

    fn tick(&mut self) {
        for obj in &mut self.objects {
            obj.tick();
        }
    }
}

// ── Encoding helpers ──

fn encode_context_tag(tag_num: u8, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    if data.len() <= 4 {
        buf.push(0x80 | (tag_num << 4) | (data.len() as u8));
        buf.extend_from_slice(data);
    } else {
        buf.push(0x80 | (tag_num << 4) | 0x05);
        buf.push((data.len() >> 8) as u8);
        buf.push((data.len() & 0xFF) as u8);
        buf.extend_from_slice(data);
    }
    buf
}

fn encode_real(value: f32) -> Vec<u8> {
    let mut buf = vec![0x44];
    buf.extend_from_slice(&value.to_be_bytes());
    buf
}

fn encode_unsigned(value: u32) -> Vec<u8> {
    if value <= 0xFF {
        vec![0x21, value as u8]
    } else if value <= 0xFFFF {
        vec![0x22, (value >> 8) as u8, (value & 0xFF) as u8]
    } else {
        let mut buf = vec![0x24];
        buf.extend_from_slice(&value.to_be_bytes());
        buf
    }
}

fn encode_enum(value: u32) -> Vec<u8> {
    if value <= 0xFF {
        vec![0x91, value as u8]
    } else {
        vec![0x92, (value >> 8) as u8, (value & 0xFF) as u8]
    }
}

fn encode_character_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut buf = vec![0x75, bytes.len() as u8];
    buf.extend_from_slice(bytes);
    buf
}

#[allow(dead_code)]
fn encode_object_id(obj_type: u16, instance: u32) -> Vec<u8> {
    let id: u32 = ((obj_type as u32) << 22) | instance;
    let mut buf = vec![0xC4];
    buf.extend_from_slice(&id.to_be_bytes());
    buf
}

fn encode_bvlc(message: &[u8]) -> Vec<u8> {
    let mut bvlc = vec![BVLC_TYPE, BVLC_ORIGINAL_UNICAST, 0x00, 0x00];
    bvlc.extend_from_slice(message);
    let len = bvlc.len() as u16;
    bvlc[2] = (len >> 8) as u8;
    bvlc[3] = (len & 0xFF) as u8;
    bvlc
}

fn encode_npdu() -> Vec<u8> {
    vec![0x01, 0x00]
}

// ── I-Am response ──

fn build_iam_response(device_instance: u32, vendor_id: u16) -> Vec<u8> {
    let mut apdu = vec![APDU_UNCONFIRMED_REQUEST, SVC_I_AM];

    let dev_obj_id: u32 = (OBJ_TYPE_DEVICE as u32) << 22 | device_instance;
    apdu.push(0xC4);
    apdu.extend_from_slice(&dev_obj_id.to_be_bytes());

    // Max APDU length accepted (1476)
    apdu.extend_from_slice(&[0x22, 0x05, 0xC4]);

    // Segmentation: none
    apdu.extend_from_slice(&[0x91, 0x00]);

    // Vendor ID
    apdu.extend_from_slice(&encode_unsigned(vendor_id as u32));

    let mut npdu = encode_npdu();
    npdu.extend_from_slice(&apdu);
    encode_bvlc(&npdu)
}

// ── ReadProperty ──

fn read_property(
    state: &DeviceState,
    obj_type: u16,
    instance: u32,
    property_id: u32,
) -> Option<Vec<u8>> {
    if obj_type == OBJ_TYPE_DEVICE {
        return match property_id {
            PROP_OBJECT_NAME => Some(encode_character_string(&format!(
                "Device_{}",
                state.device_instance
            ))),
            PROP_VENDOR_NAME => Some(encode_character_string(&state.vendor_name)),
            PROP_VENDOR_ID => Some(encode_unsigned(state.vendor_id as u32)),
            PROP_MODEL_NAME => Some(encode_character_string(&state.model_name)),
            PROP_FIRMWARE_REV => Some(encode_character_string(&state.firmware)),
            PROP_OBJECT_LIST => {
                let mut list = vec![0x0E]; // opening tag
                let dev_id: u32 = (OBJ_TYPE_DEVICE as u32) << 22 | state.device_instance;
                list.push(0xC4);
                list.extend_from_slice(&dev_id.to_be_bytes());
                for obj in &state.objects {
                    list.push(0xC4);
                    list.extend_from_slice(&obj.object_identifier().to_be_bytes());
                }
                list.push(0x0F); // closing tag
                Some(list)
            }
            _ => None,
        };
    }

    let idx = state.find_object(obj_type, instance)?;
    let obj = &state.objects[idx];

    match property_id {
        PROP_PRESENT_VALUE => {
            if obj.object_type == OBJ_TYPE_BINARY_INPUT
                || obj.object_type == OBJ_TYPE_BINARY_OUTPUT
                || obj.object_type == OBJ_TYPE_BINARY_VALUE
            {
                Some(encode_enum(if obj.value > 0.5 { 1 } else { 0 }))
            } else if obj.object_type == OBJ_TYPE_MULTI_STATE_INPUT
                || obj.object_type == OBJ_TYPE_MULTI_STATE_OUTPUT
                || obj.object_type == OBJ_TYPE_MULTI_STATE_VALUE
            {
                Some(encode_enum(obj.value as u32))
            } else {
                Some(encode_real(obj.value as f32))
            }
        }
        PROP_OBJECT_NAME => Some(encode_character_string(&obj.name)),
        PROP_DESCRIPTION => Some(encode_character_string(&obj.description)),
        PROP_STATUS_FLAGS => Some(encode_unsigned(0)),
        _ => None,
    }
}

// ── WriteProperty ──

fn write_property(
    state: &mut DeviceState,
    obj_type: u16,
    instance: u32,
    property_id: u32,
    value_data: &[u8],
) -> bool {
    if property_id != PROP_PRESENT_VALUE {
        return false;
    }
    let Some(idx) = state.find_object(obj_type, instance) else {
        return false;
    };
    if value_data.len() < 2 {
        return false;
    }

    let tag = value_data[0];
    let value = match tag {
        0x44 if value_data.len() >= 5 => {
            f32::from_be_bytes([value_data[1], value_data[2], value_data[3], value_data[4]]) as f64
        }
        0x55 if value_data.len() >= 9 => {
            let bytes: [u8; 8] = value_data[1..9].try_into().unwrap();
            f64::from_be_bytes(bytes)
        }
        0x21 if value_data.len() >= 2 => value_data[1] as f64,
        0x91 if value_data.len() >= 2 => value_data[1] as f64,
        _ => return false,
    };

    state.objects[idx].value = value;
    tracing::info!("Write {}:{} = {:.2}", obj_type, instance, value);
    true
}

// ── APDU parsing ──

fn parse_object_identifier(data: &[u8]) -> Option<(u16, u32)> {
    if data.len() < 5 {
        return None;
    }
    let tag = data[0];
    if (tag & 0x0F) != 0x04 {
        return None;
    }
    let id = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    Some(((id >> 22) as u16, id & 0x3FFFFF))
}

fn parse_unsigned(data: &[u8]) -> Option<(u32, usize)> {
    if data.is_empty() {
        return None;
    }
    let tag = data[0];
    let len = (tag & 0x0F) as usize;
    match len {
        1 => Some((*data.get(1)? as u32, 2)),
        2 => Some((((*data.get(1)? as u32) << 8) | *data.get(2)? as u32, 3)),
        4 => Some((
            u32::from_be_bytes([*data.get(1)?, *data.get(2)?, *data.get(3)?, *data.get(4)?]),
            5,
        )),
        _ => None,
    }
}

fn find_value_end(data: &[u8], start: usize) -> usize {
    let mut pos = start;
    while pos < data.len() {
        let b = data[pos];
        if (b & 0x07) == 0x07 {
            return pos;
        } // closing tag
        let len_field = b & 0x0F;
        if len_field == 5 && pos + 2 < data.len() {
            let long_len = ((data[pos + 1] as usize) << 8) | data[pos + 2] as usize;
            pos += 3 + long_len;
        } else {
            pos += 1 + len_field as usize;
        }
    }
    data.len()
}

// ── Message dispatch ──

fn handle_message(
    socket: &UdpSocket,
    state: &Arc<Mutex<DeviceState>>,
    buf: &[u8],
    n: usize,
    src: std::net::SocketAddr,
) {
    if n < 6 || buf[0] != BVLC_TYPE {
        return;
    }
    if buf[1] != BVLC_ORIGINAL_UNICAST && buf[1] != BVLC_ORIGINAL_BROADCAST {
        return;
    }

    let bvlc_len = ((buf[2] as usize) << 8) | buf[3] as usize;
    if n != bvlc_len {
        return;
    }

    let apdu_start = 6; // BVLC(4) + NPDU(2)
    if apdu_start >= n {
        return;
    }
    let apdu = &buf[apdu_start..n];
    if apdu.is_empty() {
        return;
    }

    match apdu[0] & 0xF0 {
        APDU_UNCONFIRMED_REQUEST => {
            if apdu.len() >= 2 && apdu[1] == SVC_WHO_IS {
                handle_who_is(socket, state, src);
            }
        }
        APDU_CONFIRMED_REQUEST => {
            if apdu.len() < 4 {
                return;
            }
            let segmented = (apdu[0] & 0x08) != 0;
            let invoke_id_pos = if segmented { 4 } else { 2 };
            let service_pos = invoke_id_pos + 1;
            if service_pos >= apdu.len() {
                return;
            }
            let invoke_id = apdu[invoke_id_pos];
            let service = apdu[service_pos];
            let svc_data = &apdu[service_pos + 1..];

            match service {
                SVC_READ_PROPERTY => handle_read_prop(socket, state, invoke_id, svc_data, src),
                SVC_READ_PROPERTY_MULTIPLE => handle_rpm(socket, state, invoke_id, svc_data, src),
                SVC_WRITE_PROPERTY => handle_write_prop(socket, state, invoke_id, svc_data, src),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_who_is(socket: &UdpSocket, state: &Arc<Mutex<DeviceState>>, src: std::net::SocketAddr) {
    let s = state.lock().unwrap();
    let response = build_iam_response(s.device_instance, s.vendor_id);
    drop(s);
    if let Err(e) = socket.send_to(&response, src) {
        tracing::warn!("I-Am send failed: {}", e);
    } else {
        tracing::debug!("Sent I-Am to {}", src);
    }
}

fn handle_read_prop(
    socket: &UdpSocket,
    state: &Arc<Mutex<DeviceState>>,
    invoke_id: u8,
    data: &[u8],
    src: std::net::SocketAddr,
) {
    if data.len() < 6 {
        return;
    }
    let Some((obj_type, instance)) = parse_object_identifier(data) else {
        return;
    };
    let prop_result = parse_unsigned(&data[5..]);
    let property_id = prop_result
        .as_ref()
        .map(|(id, _)| *id)
        .unwrap_or(PROP_PRESENT_VALUE);

    let s = state.lock().unwrap();
    let value = read_property(&s, obj_type, instance, property_id);
    drop(s);

    let mut apdu = vec![APDU_COMPLEX_ACK, invoke_id, SVC_READ_PROPERTY];
    apdu.extend_from_slice(&data[..5]); // echo object ID

    match value {
        Some(v) => {
            apdu.extend_from_slice(&encode_context_tag(3, &v));
        }
        None => {
            let err = vec![0x50, invoke_id, SVC_READ_PROPERTY, 0x00, 0x20];
            let mut npdu = encode_npdu();
            npdu.extend_from_slice(&err);
            let _ = socket.send_to(&encode_bvlc(&npdu), src);
            return;
        }
    }

    let mut npdu = encode_npdu();
    npdu.extend_from_slice(&apdu);
    let _ = socket.send_to(&encode_bvlc(&npdu), src);
}

fn handle_rpm(
    socket: &UdpSocket,
    state: &Arc<Mutex<DeviceState>>,
    invoke_id: u8,
    data: &[u8],
    src: std::net::SocketAddr,
) {
    let mut pos = 0;
    let mut apdu = vec![APDU_COMPLEX_ACK, invoke_id, SVC_READ_PROPERTY_MULTIPLE];

    let s = state.lock().unwrap();

    while pos + 5 < data.len() {
        let Some((obj_type, instance)) = parse_object_identifier(&data[pos..]) else {
            break;
        };
        let obj_id_bytes = data[pos..pos + 5].to_vec();
        pos += 5;

        // Skip opening tag for property list
        if pos < data.len() && data[pos] == 0x0E {
            pos += 1;
        }

        apdu.extend_from_slice(&obj_id_bytes);
        apdu.push(0x0E); // opening tag for results

        while pos < data.len() {
            if data[pos] == 0x0F {
                pos += 1;
                break;
            }
            let Some((pid, consumed)) = parse_unsigned(&data[pos..]) else {
                break;
            };
            pos += consumed;

            let value =
                read_property(&s, obj_type, instance, pid).unwrap_or_else(|| encode_unsigned(0));
            apdu.extend_from_slice(&encode_unsigned(pid));
            apdu.extend_from_slice(&encode_context_tag(4, &value));
        }

        apdu.push(0x0F); // closing tag for results
    }
    drop(s);

    let mut npdu = encode_npdu();
    npdu.extend_from_slice(&apdu);
    let _ = socket.send_to(&encode_bvlc(&npdu), src);
}

fn handle_write_prop(
    socket: &UdpSocket,
    state: &Arc<Mutex<DeviceState>>,
    invoke_id: u8,
    data: &[u8],
    src: std::net::SocketAddr,
) {
    if data.len() < 6 {
        return;
    }
    let Some((obj_type, instance)) = parse_object_identifier(data) else {
        return;
    };

    let mut pos = 5;
    let Some((property_id, consumed)) = parse_unsigned(&data[pos..]) else {
        return;
    };
    pos += consumed;

    // Find context tag 3 (property value)
    while pos < data.len() {
        let b = data[pos];
        if (b >> 4) == 0x03 {
            pos += 1;
            break;
        }
        pos += 1;
    }

    let value_start = pos;
    let value_end = find_value_end(data, pos);
    let value_data = &data[value_start..value_end];

    let mut s = state.lock().unwrap();
    let ok = write_property(&mut s, obj_type, instance, property_id, value_data);
    drop(s);

    let apdu = if ok {
        vec![APDU_SIMPLE_ACK, invoke_id, SVC_WRITE_PROPERTY]
    } else {
        vec![0x50, invoke_id, SVC_WRITE_PROPERTY, 0x00, 0x20]
    };

    let mut npdu = encode_npdu();
    npdu.extend_from_slice(&apdu);
    let _ = socket.send_to(&encode_bvlc(&npdu), src);
}

// ── CLI ──

fn parse_args() -> (u16, u32) {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 47809;
    let mut device_instance: u32 = 100;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(47809);
                i += 2;
            }
            "--device" => {
                device_instance = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(100);
                i += 2;
            }
            "--help" | "-h" => {
                eprintln!("Usage: bacnet-sim [--port PORT] [--device INSTANCE_ID]");
                eprintln!("Default port: 47809, device instance: 100");
                std::process::exit(0);
            }
            _ => {
                i += 1;
            }
        }
    }
    (port, device_instance)
}

// ── Main ──

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let (port, device_instance) = parse_args();
    let state = Arc::new(Mutex::new(DeviceState::new(device_instance)));

    let socket = UdpSocket::bind(format!("0.0.0.0:{}", port))?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;

    tracing::info!(
        "BACnet/IP simulator on 0.0.0.0:{} (device instance: {})",
        port,
        device_instance
    );
    {
        let s = state.lock().unwrap();
        for obj in &s.objects {
            tracing::info!(
                "  type={} inst={} ({}) = {:.2}",
                obj.object_type,
                obj.instance,
                obj.name,
                obj.value
            );
        }
    }

    // Background tick for drifting values
    let tick_state = state.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            tick_state.lock().unwrap().tick();
        }
    });

    let mut buf = [0u8; 1500];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, src)) => handle_message(&socket, &state, &buf, n, src),
            Err(_) => continue,
        }
    }
}
