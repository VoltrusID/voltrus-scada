//! DNP3 application layer (IEEE 1815-2012 clause 4.2.2): fragment headers,
//! object headers and the request builders the master needs.
//!
//! Request:  AC, FC, object headers...
//! Response: AC, FC, IIN1, IIN2, object headers...

use anyhow::Result;

/// Application control octet: FIR | FIN | CON | UNS | SEQ(4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppControl {
    pub fir: bool,
    pub fin: bool,
    pub con: bool,
    pub uns: bool,
    pub seq: u8,
}

impl AppControl {
    /// A single-fragment request; masters never ask for confirmation.
    pub fn request(seq: u8) -> Self {
        Self {
            fir: true,
            fin: true,
            con: false,
            uns: false,
            seq: seq & 0x0F,
        }
    }

    pub fn to_byte(self) -> u8 {
        (if self.fir { 0x80 } else { 0 })
            | (if self.fin { 0x40 } else { 0 })
            | (if self.con { 0x20 } else { 0 })
            | (if self.uns { 0x10 } else { 0 })
            | (self.seq & 0x0F)
    }

    pub fn from_byte(b: u8) -> Self {
        Self {
            fir: b & 0x80 != 0,
            fin: b & 0x40 != 0,
            con: b & 0x20 != 0,
            uns: b & 0x10 != 0,
            seq: b & 0x0F,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionCode {
    Confirm = 0x00,
    Read = 0x01,
    Write = 0x02,
    Select = 0x03,
    Operate = 0x04,
    DirectOperate = 0x05,
    DirectOperateNoAck = 0x06,
    ImmediateFreeze = 0x07,
    ImmediateFreezeNoAck = 0x08,
    FreezeClear = 0x09,
    FreezeClearNoAck = 0x0A,
    ColdRestart = 0x0D,
    WarmRestart = 0x0E,
    EnableUnsolicited = 0x14,
    DisableUnsolicited = 0x15,
    DelayMeasure = 0x17,
    RecordCurrentTime = 0x18,
    Response = 0x81,
    UnsolicitedResponse = 0x82,
}

impl FunctionCode {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0x00 => Self::Confirm,
            0x01 => Self::Read,
            0x02 => Self::Write,
            0x03 => Self::Select,
            0x04 => Self::Operate,
            0x05 => Self::DirectOperate,
            0x06 => Self::DirectOperateNoAck,
            0x07 => Self::ImmediateFreeze,
            0x08 => Self::ImmediateFreezeNoAck,
            0x09 => Self::FreezeClear,
            0x0A => Self::FreezeClearNoAck,
            0x0D => Self::ColdRestart,
            0x0E => Self::WarmRestart,
            0x14 => Self::EnableUnsolicited,
            0x15 => Self::DisableUnsolicited,
            0x17 => Self::DelayMeasure,
            0x18 => Self::RecordCurrentTime,
            0x81 => Self::Response,
            0x82 => Self::UnsolicitedResponse,
            _ => anyhow::bail!("unknown DNP3 function code 0x{v:02X}"),
        })
    }

    pub fn is_response(self) -> bool {
        matches!(self, Self::Response | Self::UnsolicitedResponse)
    }
}

/// Internal indications: IIN1 in the low byte, IIN2 in the high byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Iin(pub u16);

impl Iin {
    pub const ALL_STATIONS: u16 = 0x0001;
    pub const CLASS_1_EVENTS: u16 = 0x0002;
    pub const CLASS_2_EVENTS: u16 = 0x0004;
    pub const CLASS_3_EVENTS: u16 = 0x0008;
    pub const NEED_TIME: u16 = 0x0010;
    pub const LOCAL_CONTROL: u16 = 0x0020;
    pub const DEVICE_TROUBLE: u16 = 0x0040;
    pub const DEVICE_RESTART: u16 = 0x0080;
    pub const NO_FUNC_CODE_SUPPORT: u16 = 0x0100;
    pub const OBJECT_UNKNOWN: u16 = 0x0200;
    pub const PARAMETER_ERROR: u16 = 0x0400;
    pub const EVENT_BUFFER_OVERFLOW: u16 = 0x0800;
    pub const ALREADY_EXECUTING: u16 = 0x1000;
    pub const CONFIG_CORRUPT: u16 = 0x2000;

    pub fn from_bytes(iin1: u8, iin2: u8) -> Self {
        Self(iin1 as u16 | ((iin2 as u16) << 8))
    }

    pub fn to_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }

    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    pub fn set(&mut self, bit: u16, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    pub fn device_restart(self) -> bool {
        self.has(Self::DEVICE_RESTART)
    }

    pub fn need_time(self) -> bool {
        self.has(Self::NEED_TIME)
    }

    pub fn has_events(self) -> bool {
        self.0 & (Self::CLASS_1_EVENTS | Self::CLASS_2_EVENTS | Self::CLASS_3_EVENTS) != 0
    }

    /// IIN2 bits that mean the outstation rejected the request.
    pub fn request_error(self) -> Option<&'static str> {
        if self.has(Self::NO_FUNC_CODE_SUPPORT) {
            Some("function code not supported")
        } else if self.has(Self::OBJECT_UNKNOWN) {
            Some("object unknown")
        } else if self.has(Self::PARAMETER_ERROR) {
            Some("parameter error")
        } else if self.has(Self::ALREADY_EXECUTING) {
            Some("already executing")
        } else {
            None
        }
    }
}

/// Qualifier codes this master emits (spec table 4-5).
pub mod qualifier {
    pub const RANGE_8: u8 = 0x00;
    pub const RANGE_16: u8 = 0x01;
    pub const ALL_OBJECTS: u8 = 0x06;
    pub const COUNT_8: u8 = 0x07;
    pub const COUNT_16: u8 = 0x08;
    pub const COUNT_8_INDEX_8: u8 = 0x17;
    pub const COUNT_16_INDEX_16: u8 = 0x28;
}

/// An application fragment, either direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub control: AppControl,
    pub function: FunctionCode,
    /// Present only on responses.
    pub iin: Option<Iin>,
    pub objects: Vec<u8>,
}

impl Fragment {
    pub fn request(seq: u8, function: FunctionCode, objects: Vec<u8>) -> Self {
        Self {
            control: AppControl::request(seq),
            function,
            iin: None,
            objects,
        }
    }

    /// Confirm for a solicited (uns=false) or unsolicited response.
    pub fn confirm(seq: u8, uns: bool) -> Self {
        Self {
            control: AppControl {
                fir: true,
                fin: true,
                con: false,
                uns,
                seq: seq & 0x0F,
            },
            function: FunctionCode::Confirm,
            iin: None,
            objects: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.objects.len());
        out.push(self.control.to_byte());
        out.push(self.function as u8);
        if let Some(iin) = self.iin {
            out.extend_from_slice(&iin.to_bytes());
        }
        out.extend_from_slice(&self.objects);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            anyhow::bail!("APDU too short: {} bytes", bytes.len());
        }
        let control = AppControl::from_byte(bytes[0]);
        let function = FunctionCode::from_u8(bytes[1])?;
        let (iin, rest) = if function.is_response() {
            if bytes.len() < 4 {
                anyhow::bail!("response APDU without IIN");
            }
            (Some(Iin::from_bytes(bytes[2], bytes[3])), &bytes[4..])
        } else {
            (None, &bytes[2..])
        };
        Ok(Self {
            control,
            function,
            iin,
            objects: rest.to_vec(),
        })
    }
}

/// Object header builders for requests.
pub mod header {
    use super::qualifier;

    pub fn all_objects(group: u8, variation: u8) -> Vec<u8> {
        vec![group, variation, qualifier::ALL_OBJECTS]
    }

    pub fn range(group: u8, variation: u8, start: u16, stop: u16) -> Vec<u8> {
        if stop <= 0xFF {
            vec![
                group,
                variation,
                qualifier::RANGE_8,
                start as u8,
                stop as u8,
            ]
        } else {
            let mut v = vec![group, variation, qualifier::RANGE_16];
            v.extend_from_slice(&start.to_le_bytes());
            v.extend_from_slice(&stop.to_le_bytes());
            v
        }
    }

    pub fn count8(group: u8, variation: u8, count: u8) -> Vec<u8> {
        vec![group, variation, qualifier::COUNT_8, count]
    }

    /// One object with a 16-bit index prefix: the form controls use.
    pub fn single_indexed(group: u8, variation: u8, index: u16, object: &[u8]) -> Vec<u8> {
        let mut v = vec![group, variation, qualifier::COUNT_16_INDEX_16, 1, 0];
        v.extend_from_slice(&index.to_le_bytes());
        v.extend_from_slice(object);
        v
    }
}

/// Class read headers in the order the spec recommends (events before static
/// data, so a static value is never older than an event read after it).
pub fn class_headers(class0: bool, class1: bool, class2: bool, class3: bool) -> Vec<u8> {
    let mut v = Vec::new();
    if class1 {
        v.extend(header::all_objects(60, 2));
    }
    if class2 {
        v.extend(header::all_objects(60, 3));
    }
    if class3 {
        v.extend(header::all_objects(60, 4));
    }
    if class0 {
        v.extend(header::all_objects(60, 1));
    }
    v
}

/// READ Class 1, 2, 3 and 0.
pub fn integrity_poll(seq: u8) -> Fragment {
    Fragment::request(
        seq,
        FunctionCode::Read,
        class_headers(true, true, true, true),
    )
}

/// READ Class 1, 2 and 3.
pub fn event_poll(seq: u8) -> Fragment {
    Fragment::request(
        seq,
        FunctionCode::Read,
        class_headers(false, true, true, true),
    )
}

pub fn set_unsolicited(seq: u8, enable: bool) -> Fragment {
    let fc = if enable {
        FunctionCode::EnableUnsolicited
    } else {
        FunctionCode::DisableUnsolicited
    };
    Fragment::request(seq, fc, class_headers(false, true, true, true))
}

/// WRITE g80v1 index 7 = 0 to clear IIN1.7 DEVICE_RESTART.
pub fn clear_restart(seq: u8) -> Fragment {
    let mut objects = header::range(80, 1, 7, 7);
    objects.push(0x00);
    Fragment::request(seq, FunctionCode::Write, objects)
}

/// WRITE g50v1 (absolute time, ms since 1970 UTC, 48 bits).
pub fn write_time(seq: u8, epoch_ms: u64) -> Fragment {
    let mut objects = header::count8(50, 1, 1);
    objects.extend_from_slice(&epoch_ms.to_le_bytes()[..6]);
    Fragment::request(seq, FunctionCode::Write, objects)
}

/// WRITE g50v3 (time of last recorded time), the LAN procedure's second step.
pub fn write_last_recorded_time(seq: u8, epoch_ms: u64) -> Fragment {
    let mut objects = header::count8(50, 3, 1);
    objects.extend_from_slice(&epoch_ms.to_le_bytes()[..6]);
    Fragment::request(seq, FunctionCode::Write, objects)
}

/// Control Relay Output Block (g12v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crob {
    pub code: u8,
    pub count: u8,
    pub on_ms: u32,
    pub off_ms: u32,
}

impl Crob {
    pub const PULSE_ON: u8 = 0x01;
    pub const PULSE_OFF: u8 = 0x02;
    pub const LATCH_ON: u8 = 0x03;
    pub const LATCH_OFF: u8 = 0x04;
    pub const CLOSE: u8 = 0x40;
    pub const TRIP: u8 = 0x80;

    pub fn latch(on: bool) -> Self {
        Self {
            code: if on { Self::LATCH_ON } else { Self::LATCH_OFF },
            count: 1,
            on_ms: 0,
            off_ms: 0,
        }
    }

    pub fn pulse(on: bool, ms: u32) -> Self {
        Self {
            code: if on { Self::PULSE_ON } else { Self::PULSE_OFF },
            count: 1,
            on_ms: ms,
            off_ms: ms,
        }
    }

    /// Trip/close pair outputs: a pulse-on with the trip or close code bits.
    pub fn trip_close(close: bool, ms: u32) -> Self {
        Self {
            code: Self::PULSE_ON | if close { Self::CLOSE } else { Self::TRIP },
            count: 1,
            on_ms: ms,
            off_ms: 0,
        }
    }

    pub fn encode(&self) -> [u8; 11] {
        let mut b = [0u8; 11];
        b[0] = self.code;
        b[1] = self.count;
        b[2..6].copy_from_slice(&self.on_ms.to_le_bytes());
        b[6..10].copy_from_slice(&self.off_ms.to_le_bytes());
        b[10] = 0; // status: the outstation fills this in its echo
        b
    }
}

/// Analog output command (g41), in one of the wire formats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnalogCommand {
    I32(i32),
    I16(i16),
    F32(f32),
    F64(f64),
}

impl AnalogCommand {
    pub fn variation(&self) -> u8 {
        match self {
            Self::I32(_) => 1,
            Self::I16(_) => 2,
            Self::F32(_) => 3,
            Self::F64(_) => 4,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = match self {
            Self::I32(x) => x.to_le_bytes().to_vec(),
            Self::I16(x) => x.to_le_bytes().to_vec(),
            Self::F32(x) => x.to_le_bytes().to_vec(),
            Self::F64(x) => x.to_le_bytes().to_vec(),
        };
        v.push(0); // status
        v
    }
}

/// A control the master can select/operate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    Crob { index: u16, crob: Crob },
    Analog { index: u16, value: AnalogCommand },
}

impl Control {
    pub fn object_headers(&self) -> Vec<u8> {
        match self {
            Self::Crob { index, crob } => header::single_indexed(12, 1, *index, &crob.encode()),
            Self::Analog { index, value } => {
                header::single_indexed(41, value.variation(), *index, &value.encode())
            }
        }
    }

    pub fn fragment(&self, seq: u8, function: FunctionCode) -> Fragment {
        Fragment::request(seq, function, self.object_headers())
    }
}

/// Control status codes an outstation returns in the echoed object.
pub fn control_status_text(status: u8) -> &'static str {
    match status {
        0 => "success",
        1 => "select timeout",
        2 => "no select",
        3 => "format error",
        4 => "not supported",
        5 => "already active",
        6 => "hardware error",
        7 => "local",
        8 => "too many objects",
        9 => "not authorized",
        10 => "automation inhibit",
        11 => "processing limited",
        12 => "out of range",
        _ => "non-participating or unknown",
    }
}

/// Compare a SELECT/OPERATE echo with the request: the object bytes must be
/// identical apart from the status octet, which is the last octet of both
/// g12v1 and g41 objects. Returns the status.
pub fn check_control_echo(request_objects: &[u8], echo_objects: &[u8]) -> Result<u8> {
    if request_objects.len() != echo_objects.len() || request_objects.is_empty() {
        anyhow::bail!(
            "control echo is {} bytes, the request was {}",
            echo_objects.len(),
            request_objects.len()
        );
    }
    let last = request_objects.len() - 1;
    if request_objects[..last] != echo_objects[..last] {
        anyhow::bail!("control echo does not match the request");
    }
    Ok(echo_objects[last])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn integrity_poll_matches_known_bytes() {
        // Class 1230 read, seq 0: the integrity poll seen in every DNP3
        // capture (opendnp3, Wireshark sample dnp3_read.pcap).
        assert_eq!(
            integrity_poll(0).encode(),
            hex("C0 01 3C 02 06 3C 03 06 3C 04 06 3C 01 06")
        );
    }

    #[test]
    fn event_poll_matches_known_bytes() {
        assert_eq!(
            event_poll(3).encode(),
            hex("C3 01 3C 02 06 3C 03 06 3C 04 06")
        );
    }

    #[test]
    fn request_has_no_iin_and_control_comes_first() {
        let bytes = Fragment::request(5, FunctionCode::Read, vec![]).encode();
        assert_eq!(bytes, vec![0xC5, 0x01]);
    }

    #[test]
    fn response_header_decodes_with_iin() {
        let f = Fragment::decode(&hex("E2 81 90 00 01 02 00 00 00 81")).unwrap();
        assert_eq!(
            f.control,
            AppControl {
                fir: true,
                fin: true,
                con: true,
                uns: false,
                seq: 2
            }
        );
        assert_eq!(f.function, FunctionCode::Response);
        let iin = f.iin.unwrap();
        assert!(iin.device_restart());
        assert!(iin.need_time());
        assert!(!iin.has_events());
        assert_eq!(f.objects, hex("01 02 00 00 00 81"));
        // And it re-encodes byte for byte.
        assert_eq!(f.encode(), hex("E2 81 90 00 01 02 00 00 00 81"));
    }

    #[test]
    fn iin2_errors_are_reported() {
        assert_eq!(
            Iin::from_bytes(0, 0x01).request_error(),
            Some("function code not supported")
        );
        assert_eq!(
            Iin::from_bytes(0, 0x02).request_error(),
            Some("object unknown")
        );
        assert_eq!(Iin::from_bytes(0x8E, 0).request_error(), None);
        assert!(Iin::from_bytes(0x02, 0).has_events());
    }

    #[test]
    fn confirm_frames() {
        assert_eq!(Fragment::confirm(4, false).encode(), vec![0xC4, 0x00]);
        assert_eq!(Fragment::confirm(4, true).encode(), vec![0xD4, 0x00]);
    }

    #[test]
    fn clear_restart_matches_known_bytes() {
        // WRITE g80v1 start 7 stop 7 value 0.
        assert_eq!(clear_restart(1).encode(), hex("C1 02 50 01 00 07 07 00"));
    }

    #[test]
    fn unsolicited_enable_bytes() {
        assert_eq!(
            set_unsolicited(2, true).encode(),
            hex("C2 14 3C 02 06 3C 03 06 3C 04 06")
        );
        assert_eq!(set_unsolicited(2, false).encode()[1], 0x15);
    }

    #[test]
    fn write_time_bytes() {
        let f = write_time(0, 0x0102_0304_0506);
        assert_eq!(f.encode(), hex("C0 02 32 01 07 01 06 05 04 03 02 01"));
    }

    #[test]
    fn crob_latch_on_matches_known_bytes() {
        // SELECT g12v1 qualifier 0x28, index 3, LATCH_ON, count 1.
        let c = Control::Crob {
            index: 3,
            crob: Crob::latch(true),
        };
        assert_eq!(
            c.fragment(7, FunctionCode::Select).encode(),
            hex("C7 03 0C 01 28 01 00 03 00 03 01 00 00 00 00 00 00 00 00 00")
        );
    }

    #[test]
    fn crob_pulse_and_trip_codes() {
        assert_eq!(Crob::pulse(true, 100).encode()[0], 0x01);
        assert_eq!(Crob::trip_close(true, 100).encode()[0], 0x41);
        assert_eq!(Crob::trip_close(false, 100).encode()[0], 0x81);
        assert_eq!(
            &Crob::pulse(false, 1000).encode()[2..6],
            &[0xE8, 0x03, 0, 0]
        );
    }

    #[test]
    fn analog_output_commands() {
        let c = Control::Analog {
            index: 1,
            value: AnalogCommand::I32(-2),
        };
        assert_eq!(
            c.object_headers(),
            hex("29 01 28 01 00 01 00 FE FF FF FF 00")
        );
        let c = Control::Analog {
            index: 0,
            value: AnalogCommand::I16(300),
        };
        assert_eq!(c.object_headers(), hex("29 02 28 01 00 00 00 2C 01 00"));
        let c = Control::Analog {
            index: 0,
            value: AnalogCommand::F32(1.0),
        };
        assert_eq!(
            c.object_headers(),
            hex("29 03 28 01 00 00 00 00 00 80 3F 00")
        );
    }

    #[test]
    fn control_echo_check() {
        let req = Control::Crob {
            index: 0,
            crob: Crob::latch(false),
        }
        .object_headers();
        let mut echo = req.clone();
        assert_eq!(check_control_echo(&req, &echo).unwrap(), 0);
        *echo.last_mut().unwrap() = 4;
        assert_eq!(check_control_echo(&req, &echo).unwrap(), 4);
        echo[7] = 0x03;
        assert!(check_control_echo(&req, &echo).is_err());
        assert!(check_control_echo(&req, &[]).is_err());
    }

    #[test]
    fn range_header_widens_for_large_indices() {
        assert_eq!(header::range(1, 2, 0, 9), hex("01 02 00 00 09"));
        assert_eq!(header::range(1, 2, 0, 300), hex("01 02 01 00 00 2C 01"));
    }
}
