//! Modbus/TCP, from the Modbus Application Protocol Specification v1.1b3 and
//! Modbus Messaging on TCP/IP Implementation Guide v1.0b (modbus.org).
//!
//! An ADU is the 7-byte MBAP header (transaction id, protocol id, length,
//! unit id) followed by the PDU (function code and data). A response is
//! decoded with its request when the request was seen, because a response
//! does not repeat the address it answers. [`Session`] keeps the open
//! requests of one connection and pairs them by transaction id.

use std::collections::VecDeque;

use crate::{AddrRange, DecodedFrame, Dir, ErrorInfo, ErrorKind, Field, Proto, Value};

/// The registered Modbus/TCP port.
pub const PORT: u16 = 502;
pub const MBAP_LEN: usize = 7;
/// Largest ADU: MBAP header plus a 253-byte PDU.
pub const MAX_ADU: usize = 260;

/// Length of the ADU at the start of `buf`: `Ok(None)` when more bytes are
/// needed, `Err(())` when the bytes are not a Modbus/TCP header (protocol id
/// not 0, or a length outside 2..=254).
#[allow(clippy::result_unit_err)]
pub fn adu_len(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 6 {
        return Ok(None);
    }
    let protocol = u16::from_be_bytes([buf[2], buf[3]]);
    let length = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    if protocol != 0 || !(2..=254).contains(&length) {
        return Err(());
    }
    let total = 6 + length;
    Ok((buf.len() >= total).then_some(total))
}

/// Cuts one direction of a TCP stream into ADUs, whatever the segment
/// boundaries were.
#[derive(Debug, Default)]
pub struct Splitter {
    buf: Vec<u8>,
}

impl Splitter {
    /// Append `data` and hand every complete ADU to `out`. Bytes that cannot
    /// be a Modbus/TCP header are handed over as they are (they decode as
    /// malformed) and the stream starts again from the next segment.
    pub fn push(&mut self, data: &[u8], mut out: impl FnMut(&[u8])) {
        self.buf.extend_from_slice(data);
        let mut pos = 0;
        loop {
            match adu_len(&self.buf[pos..]) {
                Ok(Some(n)) => {
                    out(&self.buf[pos..pos + n]);
                    pos += n;
                }
                Ok(None) => break,
                Err(()) => {
                    let end = self.buf.len().min(pos + MAX_ADU);
                    out(&self.buf[pos..end]);
                    pos = self.buf.len();
                    break;
                }
            }
        }
        self.buf.drain(..pos);
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

/// What a request asked for; a response is decoded against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub unit: u8,
    pub function: u8,
    /// The data read (function 23: the read half).
    pub range: Option<AddrRange>,
}

/// Parse the parameters of a request ADU.
pub fn request_of(adu: &[u8]) -> Option<Request> {
    let unit = *adu.get(6)?;
    let function = *adu.get(7)?;
    let start = be16(adu, 8).map(u32::from);
    let count = be16(adu, 10).map(u32::from);
    let range = match function {
        1 | 2 | 3 | 4 | 15 | 16 => Some(AddrRange {
            space: space_of(function)?,
            start: start?,
            count: count?,
        }),
        5 | 6 | 22 => Some(AddrRange {
            space: space_of(function)?,
            start: start?,
            count: 1,
        }),
        23 => Some(AddrRange {
            space: "holding",
            start: start?,
            count: count?,
        }),
        _ => None,
    };
    Some(Request {
        unit,
        function,
        range,
    })
}

/// The data table a function code works on.
pub fn space_of(function: u8) -> Option<&'static str> {
    Some(match function {
        1 | 5 | 15 => "coil",
        2 => "discrete",
        4 => "input",
        3 | 6 | 16 | 22 | 23 => "holding",
        _ => return None,
    })
}

pub fn function_name(function: u8) -> &'static str {
    match function {
        1 => "Read Coils",
        2 => "Read Discrete Inputs",
        3 => "Read Holding Registers",
        4 => "Read Input Registers",
        5 => "Write Single Coil",
        6 => "Write Single Register",
        7 => "Read Exception Status",
        8 => "Diagnostics",
        11 => "Get Comm Event Counter",
        12 => "Get Comm Event Log",
        15 => "Write Multiple Coils",
        16 => "Write Multiple Registers",
        17 => "Report Server ID",
        20 => "Read File Record",
        21 => "Write File Record",
        22 => "Mask Write Register",
        23 => "Read/Write Multiple Registers",
        24 => "Read FIFO Queue",
        43 => "Encapsulated Interface Transport",
        _ => "Unknown function",
    }
}

/// Exception codes: the name in the specification, and what it usually
/// means on site.
pub fn exception_text(code: u8) -> (&'static str, &'static str) {
    match code {
        1 => (
            "Illegal function",
            "The device does not support this function code, or not in its current state. Check that the register area matches the device manual: holding registers are read with function 3 and input registers with 4, and some devices cannot write several registers at once (16) or coils at all.",
        ),
        2 => (
            "Illegal data address",
            "The address is outside the device's register map, or the request runs past the end of a block of registers. Most often the address is off by one: manuals number registers from 1 (40001 is wire address 0) while others list 0-based offsets. Check the device manual and the register's zero-based setting; if only a batched read fails, the batch may span a gap between two blocks.",
        ),
        3 => (
            "Illegal data value",
            "The device rejected a value in the request: a quantity outside 1-125 registers or 1-2000 coils, a coil value other than ON (FF00) or OFF (0000), or a value the device does not accept for this register.",
        ),
        4 => (
            "Server device failure",
            "The device hit an unrecoverable error while carrying out the request. Check the device's own diagnostics; the request itself was understood.",
        ),
        5 => (
            "Acknowledge",
            "The device accepted the request but needs a long time to finish it. Poll for completion later instead of repeating the request.",
        ),
        6 => (
            "Server device busy",
            "The device is busy with a long-running command. Retry later; if it persists, poll less often or check what else talks to the device.",
        ),
        7 => (
            "Negative acknowledge",
            "The device cannot perform the programming function that was requested.",
        ),
        8 => (
            "Memory parity error",
            "The device found a parity error reading its extended memory (file records). The device may need service.",
        ),
        10 => (
            "Gateway path unavailable",
            "The gateway has no path to the unit id in the request. Check the unit id and the gateway's routing table.",
        ),
        11 => (
            "Gateway target device failed to respond",
            "The gateway forwarded the request but the device behind it did not answer. Check the unit id, the serial wiring and the baud rate, parity and stop bits on the gateway's serial side.",
        ),
        _ => (
            "Unknown exception",
            "The device answered with an exception code the Modbus specification does not define; see the device manual.",
        ),
    }
}

/// The 1-based reference engineers use (40001 for holding register 0).
pub fn reference(space: &str, addr: u32) -> String {
    let prefix = match space {
        "coil" => 0,
        "discrete" => 1,
        "input" => 3,
        _ => 4,
    };
    if addr < 9999 {
        format!("{prefix}{:04}", addr + 1)
    } else {
        format!("{prefix}{:05}", addr + 1)
    }
}

/// A decoded ADU, before pairing.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub summary: String,
    pub function: Option<u16>,
    pub range: Option<AddrRange>,
    pub fields: Vec<Field>,
    pub error: Option<ErrorInfo>,
}

fn be16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(off)?, *b.get(off + 1)?]))
}

/// Collects fields while reading; a read past the end marks the frame
/// truncated instead of failing.
struct Fields<'a> {
    adu: &'a [u8],
    out: Vec<Field>,
    truncated: bool,
}

impl<'a> Fields<'a> {
    fn u8(&mut self, id: &'static str, label: &'static str, off: usize) -> Option<u8> {
        let v = self.get(off, 1)?[0];
        self.out
            .push(Field::new(id, label, Value::Uint(v.into()), off, 1));
        Some(v)
    }

    fn u8_hex(&mut self, id: &'static str, label: &'static str, off: usize) -> Option<u8> {
        let v = self.get(off, 1)?[0];
        self.out
            .push(Field::new(id, label, Value::Uint(v.into()), off, 1).hex(2));
        Some(v)
    }

    fn u16(&mut self, id: &'static str, label: &'static str, off: usize) -> Option<u16> {
        let b = self.get(off, 2)?;
        let v = u16::from_be_bytes([b[0], b[1]]);
        self.out
            .push(Field::new(id, label, Value::Uint(v.into()), off, 2));
        Some(v)
    }

    fn u16_hex(&mut self, id: &'static str, label: &'static str, off: usize) -> Option<u16> {
        let b = self.get(off, 2)?;
        let v = u16::from_be_bytes([b[0], b[1]]);
        self.out
            .push(Field::new(id, label, Value::Uint(v.into()), off, 2).hex(4));
        Some(v)
    }

    fn bytes(
        &mut self,
        id: &'static str,
        label: &'static str,
        off: usize,
        len: usize,
    ) -> Option<&'a [u8]> {
        let b = self.get(off, len)?;
        self.out
            .push(Field::new(id, label, Value::Bytes(b.to_vec()), off, len));
        Some(b)
    }

    fn get(&mut self, off: usize, len: usize) -> Option<&'a [u8]> {
        let adu = self.adu;
        match adu.get(off..off + len) {
            Some(b) => Some(b),
            None => {
                self.truncated = true;
                None
            }
        }
    }

    /// Register values from `off`, numbered from `first`.
    fn registers(&mut self, off: usize, bytes: usize, first: u32) {
        let avail = self.adu.len().saturating_sub(off);
        if avail < bytes {
            self.truncated = true;
        }
        for i in 0..bytes.min(avail) / 2 {
            let o = off + 2 * i;
            let v = u16::from_be_bytes([self.adu[o], self.adu[o + 1]]);
            let n = first.wrapping_add(i as u32) & 0xFFFF;
            self.out.push(Field::group(
                format!("Register {n} (UINT16): {v}"),
                o,
                2,
                vec![
                    Field::new(
                        "modbus.regnum16",
                        "Register Number",
                        Value::Uint(n.into()),
                        o,
                        2,
                    ),
                    Field::new(
                        "modbus.regval_uint16",
                        "Register Value (UINT16)",
                        Value::Uint(v.into()),
                        o,
                        2,
                    ),
                ],
            ));
        }
    }

    /// `count` bit values packed from `off`, LSB first, numbered from `first`.
    fn bits(&mut self, off: usize, count: usize, first: u32) {
        let avail = self.adu.len().saturating_sub(off) * 8;
        if avail < count {
            self.truncated = true;
        }
        for i in 0..count.min(avail) {
            let o = off + i / 8;
            let v = self.adu[o] >> (i % 8) & 1 == 1;
            let n = first.wrapping_add(i as u32) & 0xFFFF;
            self.out.push(Field::group(
                format!("Bit {n} : {}", u8::from(v)),
                o,
                1,
                vec![
                    Field::new("modbus.bitnum", "Bit Number", Value::Uint(n.into()), o, 1),
                    Field::new("modbus.bitval", "Bit Value", Value::Bool(v), o, 1),
                ],
            ));
        }
    }

    /// Bytes this decoder does not break down further.
    fn rest(&mut self, off: usize) {
        if off < self.adu.len() {
            let len = self.adu.len() - off;
            self.out.push(Field::group(
                format!("Data ({len} bytes, not decoded)"),
                off,
                len,
                Vec::new(),
            ));
        }
    }
}

/// Decode one ADU. `request` is the request a response answers, when known.
pub fn decode(adu: &[u8], dir: Dir, request: Option<&Request>) -> Decoded {
    let mut mbap = Fields {
        adu,
        out: Vec::new(),
        truncated: false,
    };
    let tid = mbap.u16("mbtcp.trans_id", "Transaction Identifier", 0);
    let proto = mbap.u16("mbtcp.prot_id", "Protocol Identifier", 2);
    let len = mbap.u16("mbtcp.len", "Length", 4);
    let unit = mbap.u8("mbtcp.unit_id", "Unit Identifier", 6);
    let mut fields = vec![Field::group(
        format!(
            "Modbus/TCP, transaction {}, unit {}",
            tid.unwrap_or(0),
            unit.unwrap_or(0)
        ),
        0,
        adu.len().min(MBAP_LEN),
        mbap.out,
    )];
    let header_error = if mbap.truncated {
        Some("shorter than the 7-byte MBAP header")
    } else if proto != Some(0) {
        Some("protocol identifier is not 0, so this is not Modbus")
    } else if len.map(|l| l as usize + 6) != Some(adu.len()) {
        Some("MBAP length does not match the frame")
    } else {
        None
    };
    if let Some(why) = header_error {
        return Decoded {
            summary: format!("Malformed Modbus/TCP frame: {why}"),
            function: None,
            range: None,
            fields,
            error: Some(malformed(why)),
        };
    }

    let mut f = Fields {
        adu,
        out: Vec::new(),
        truncated: false,
    };
    let Some(fc_raw) = adu.get(7).copied() else {
        return Decoded {
            summary: "Malformed Modbus/TCP frame: no function code".into(),
            function: None,
            range: None,
            fields,
            error: Some(malformed("no function code")),
        };
    };
    let fc = fc_raw & 0x7F;
    let name = function_name(fc);
    f.out.push(
        Field::new(
            "modbus.func_code",
            "Function Code",
            Value::Uint(fc.into()),
            7,
            1,
        )
        .display(format!("Function Code: {name} ({fc})")),
    );

    let mut error = None;
    let range;
    let summary;
    if dir == Dir::Rx && fc_raw & 0x80 != 0 {
        let code = f.get(8, 1).map(|b| b[0]);
        range = request.and_then(|r| r.range);
        if let Some(code) = code {
            let (title, explanation) = exception_text(code);
            f.out.push(
                Field::new(
                    "modbus.exception_code",
                    "Exception Code",
                    Value::Uint(code.into()),
                    8,
                    1,
                )
                .display(format!("Exception Code: {title} ({code})")),
            );
            summary = format!("{name}: exception {code:02} {title}");
            error = Some(ErrorInfo {
                kind: ErrorKind::Exception,
                code: Some(code.into()),
                title: format!("Exception {code:02}: {title}"),
                explanation: match &range {
                    Some(r) => format!(
                        "{explanation} The request was {name} at wire address {} ({}), {} {}.",
                        r.start,
                        reference(r.space, r.start),
                        r.count,
                        unit_word(r.space, r.count)
                    ),
                    None => explanation.to_string(),
                },
            });
        } else {
            summary = format!("{name}: exception");
        }
    } else if dir == Dir::Tx {
        let req = request_of(adu);
        range = req.and_then(|r| r.range);
        summary = request_fields(&mut f, fc, name);
    } else {
        range = request.and_then(|r| r.range);
        summary = response_fields(&mut f, fc, name, request);
    }

    if f.truncated && error.is_none() {
        error = Some(malformed(
            "the PDU is not as long as its function code needs",
        ));
    }
    fields.push(Field::group(
        format!("Modbus, {name}"),
        7,
        adu.len() - 7,
        f.out,
    ));
    Decoded {
        summary,
        function: Some(fc.into()),
        range,
        fields,
        error,
    }
}

fn unit_word(space: &str, count: u32) -> &'static str {
    match (space, count) {
        ("coil" | "discrete", 1) => "bit",
        ("coil" | "discrete", _) => "bits",
        (_, 1) => "register",
        _ => "registers",
    }
}

fn malformed(why: &str) -> ErrorInfo {
    ErrorInfo {
        kind: ErrorKind::Malformed,
        code: None,
        title: "Malformed frame".into(),
        explanation: format!("The frame does not follow Modbus/TCP: {why}."),
    }
}

fn addr_summary(name: &str, space: &str, start: u16, count: u16) -> String {
    let first = reference(space, start.into());
    if count <= 1 {
        format!("{name} {start} ({first})")
    } else {
        let last = reference(space, u32::from(start) + u32::from(count) - 1);
        format!("{name} {start} x{count} ({first}-{last})")
    }
}

fn request_fields(f: &mut Fields, fc: u8, name: &str) -> String {
    let space = space_of(fc).unwrap_or("holding");
    match fc {
        1..=4 => {
            let start = f.u16("modbus.reference_num", "Reference Number", 8);
            let count = if fc <= 2 {
                f.u16("modbus.bit_cnt", "Bit Count", 10)
            } else {
                f.u16("modbus.word_cnt", "Word Count", 10)
            };
            addr_summary(name, space, start.unwrap_or(0), count.unwrap_or(0))
        }
        5 => {
            let addr = f.u16("modbus.reference_num", "Reference Number", 8);
            let value = be16(f.adu, 10);
            f.bytes("modbus.data", "Data", 10, 2);
            f.u8_hex("modbus.padding", "Padding", 11);
            let state = match value {
                Some(0xFF00) => "ON",
                Some(0x0000) => "OFF",
                _ => "invalid value",
            };
            format!(
                "{} = {state}",
                addr_summary(name, space, addr.unwrap_or(0), 1)
            )
        }
        // A Write Single Register PDU is 5 bytes; any other length is shown
        // as data, not as a register value.
        6 if f.adu.len() > 12 => {
            let addr = f.u16("modbus.reference_num", "Reference Number", 8);
            f.bytes("modbus.data", "Data", 10, f.adu.len() - 10);
            f.truncated = true;
            format!(
                "{} (unexpected length)",
                addr_summary(name, space, addr.unwrap_or(0), 1)
            )
        }
        6 => {
            let addr = f.u16("modbus.reference_num", "Reference Number", 8);
            f.registers(10, 2, addr.unwrap_or(0).into());
            let value = be16(f.adu, 10).unwrap_or(0);
            format!(
                "{} = {value}",
                addr_summary(name, space, addr.unwrap_or(0), 1)
            )
        }
        15 | 16 => {
            let start = f.u16("modbus.reference_num", "Reference Number", 8);
            let count = if fc == 15 {
                f.u16("modbus.bit_cnt", "Bit Count", 10)
            } else {
                f.u16("modbus.word_cnt", "Word Count", 10)
            };
            let bytes = f.u8("modbus.byte_cnt", "Byte Count", 12);
            if let (Some(start), Some(count), Some(bytes)) = (start, count, bytes) {
                if fc == 15 {
                    f.bits(
                        13,
                        usize::from(count).min(usize::from(bytes) * 8),
                        start.into(),
                    );
                } else {
                    f.registers(13, bytes.into(), start.into());
                }
            }
            addr_summary(name, space, start.unwrap_or(0), count.unwrap_or(0))
        }
        22 => {
            let addr = f.u16("modbus.reference_num", "Reference Number", 8);
            let and = f.u16_hex("modbus.and_mask", "AND mask", 10);
            let or = f.u16_hex("modbus.or_mask", "OR mask", 12);
            format!(
                "{} AND 0x{:04x} OR 0x{:04x}",
                addr_summary(name, space, addr.unwrap_or(0), 1),
                and.unwrap_or(0),
                or.unwrap_or(0)
            )
        }
        23 => {
            let rs = f.u16("modbus.read_reference_num", "Read Reference Number", 8);
            let rc = f.u16("modbus.read_word_cnt", "Read Word Count", 10);
            let ws = f.u16("modbus.write_reference_num", "Write Reference Number", 12);
            let wc = f.u16("modbus.write_word_cnt", "Write Word Count", 14);
            if let Some(n) = f.u8("modbus.byte_cnt", "Byte Count", 16) {
                f.bytes("modbus.data", "Data", 17, n.into());
            }
            format!(
                "Read/Write Multiple Registers: read {} x{}, write {} x{}",
                rs.unwrap_or(0),
                rc.unwrap_or(0),
                ws.unwrap_or(0),
                wc.unwrap_or(0)
            )
        }
        43 => {
            let mei = f.u8("modbus.mei", "MEI type", 8);
            if mei == Some(14) {
                let code = f.u8("modbus.read_device_id", "Read Device ID", 9);
                let obj = f.u8("modbus.object_id", "Object ID", 10);
                format!(
                    "Read Device Identification, {} from object {}",
                    device_id_code(code.unwrap_or(0)),
                    obj.unwrap_or(0)
                )
            } else {
                f.rest(9);
                format!("{name}, MEI {}", mei.unwrap_or(0))
            }
        }
        _ => {
            f.rest(8);
            format!("{name} ({fc})")
        }
    }
}

fn device_id_code(code: u8) -> &'static str {
    match code {
        1 => "basic stream",
        2 => "regular stream",
        3 => "extended stream",
        4 => "one object",
        _ => "unknown access",
    }
}

fn response_fields(f: &mut Fields, fc: u8, name: &str, request: Option<&Request>) -> String {
    let start = request.and_then(|r| r.range).map_or(0, |r| r.start);
    match fc {
        1 | 2 => {
            let Some(bytes) = f.u8("modbus.byte_cnt", "Byte Count", 8) else {
                return format!("{name} response");
            };
            let count = request
                .and_then(|r| r.range)
                .map_or(usize::from(bytes) * 8, |r| {
                    (r.count as usize).min(usize::from(bytes) * 8)
                });
            f.bits(9, count, start);
            format!("{name} response, {count} bits")
        }
        3 | 4 => {
            let Some(bytes) = f.u8("modbus.byte_cnt", "Byte Count", 8) else {
                return format!("{name} response");
            };
            f.registers(9, bytes.into(), start);
            format!("{name} response, {} registers", bytes / 2)
        }
        5 | 6 | 22 => {
            let s = request_fields(f, fc, name);
            format!("{s} (echo)")
        }
        15 | 16 => {
            let start = f.u16("modbus.reference_num", "Reference Number", 8);
            let count = if fc == 15 {
                f.u16("modbus.bit_cnt", "Bit Count", 10)
            } else {
                f.u16("modbus.word_cnt", "Word Count", 10)
            };
            format!(
                "{} written",
                addr_summary(
                    name,
                    space_of(fc).unwrap_or("holding"),
                    start.unwrap_or(0),
                    count.unwrap_or(0)
                )
            )
        }
        23 => {
            if let Some(n) = f.u8("modbus.byte_cnt", "Byte Count", 8) {
                f.bytes("modbus.data", "Data", 9, n.into());
                format!("{name} response, {} registers", n / 2)
            } else {
                format!("{name} response")
            }
        }
        43 => {
            let mei = f.u8("modbus.mei", "MEI type", 8);
            if mei != Some(14) {
                f.rest(9);
                return format!("{name} response, MEI {}", mei.unwrap_or(0));
            }
            f.u8("modbus.read_device_id", "Read Device ID", 9);
            f.u8_hex("modbus.conformity_level", "Conformity Level", 10);
            f.u8_hex("modbus.more_follows", "More Follows", 11);
            f.u8("modbus.next_object_id", "Next Object ID", 12);
            let Some(n) = f.u8("modbus.num_objects", "Number of Objects", 13) else {
                return "Read Device Identification response".into();
            };
            let mut objects = Vec::new();
            let mut names = Vec::new();
            let mut off = 14;
            for k in 1..=n {
                let mut o = Fields {
                    adu: f.adu,
                    out: Vec::new(),
                    truncated: false,
                };
                let id = o.u8("modbus.object_id", "Object ID", off);
                let len = o.u8("modbus.objects_len", "Object length", off + 1);
                let (Some(id), Some(len)) = (id, len) else {
                    f.truncated = true;
                    break;
                };
                let value_off = off + 2;
                if id <= 6 {
                    if let Some(b) = o.get(value_off, len.into()) {
                        let s = String::from_utf8_lossy(b).into_owned();
                        names.push(s.clone());
                        o.out.push(Field::new(
                            "modbus.object_str_value",
                            "Object String Value",
                            Value::Str(s),
                            value_off,
                            len.into(),
                        ));
                    }
                } else {
                    o.bytes("modbus.object_value", "Object Value", value_off, len.into());
                }
                f.truncated |= o.truncated;
                let end = (value_off + usize::from(len)).min(f.adu.len());
                objects.push(Field::group(format!("Object #{k}"), off, end - off, o.out));
                off = value_off + usize::from(len);
                if f.truncated {
                    break;
                }
            }
            let end = off.min(f.adu.len());
            f.out.push(Field::group(
                "Objects",
                14.min(end),
                end.saturating_sub(14),
                objects,
            ));
            format!("Read Device Identification response: {}", names.join(", "))
        }
        _ => {
            f.rest(8);
            format!("{name} response ({fc})")
        }
    }
}

/// An open request of a [`Session`].
#[derive(Debug, Clone)]
struct Pending {
    tid: u16,
    unit: u8,
    seq: u64,
    ts_us: i64,
    request: Request,
}

/// Decodes the frames of one connection and pairs each response with its
/// request by transaction id and unit id.
#[derive(Debug, Default)]
pub struct Session {
    pending: VecDeque<Pending>,
}

/// Requests kept open per connection; tokio-modbus and most masters keep one
/// in flight, pipelining masters a few.
const MAX_PENDING: usize = 64;

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one ADU. `seq` identifies the frame (a packet number, or a
    /// counter); a response gets its request's `seq` as `pair_id` and the
    /// time between them as `latency_us`.
    pub fn decode(&mut self, adu: &[u8], dir: Dir, ts_us: i64, seq: u64) -> DecodedFrame {
        let tid = be16(adu, 0);
        let unit = adu.get(6).copied();
        let mut pair_id = None;
        let mut latency_us = None;
        let decoded = match dir {
            Dir::Tx => {
                let d = decode(adu, dir, None);
                if let (Some(tid), Some(request)) = (tid, request_of(adu))
                    && d.error.is_none()
                {
                    // A repeated transaction id replaces the older request.
                    self.pending
                        .retain(|p| !(p.tid == tid && p.unit == request.unit));
                    if self.pending.len() == MAX_PENDING {
                        self.pending.pop_front();
                    }
                    self.pending.push_back(Pending {
                        tid,
                        unit: request.unit,
                        seq,
                        ts_us,
                        request,
                    });
                }
                d
            }
            Dir::Rx => {
                let at = self
                    .pending
                    .iter()
                    .position(|p| Some(p.tid) == tid && Some(p.unit) == unit);
                let req = at.and_then(|i| self.pending.remove(i));
                if let Some(req) = &req {
                    pair_id = Some(req.seq);
                    latency_us = Some(ts_us.saturating_sub(req.ts_us).max(0) as u64);
                }
                decode(adu, dir, req.as_ref().map(|p| &p.request))
            }
        };
        DecodedFrame {
            seq,
            ts_us,
            device_id: None,
            dir,
            proto: Proto::ModbusTcp,
            summary: decoded.summary,
            function: decoded.function,
            range: decoded.range,
            fields: decoded.fields,
            raw: adu.to_vec(),
            pair_id,
            latency_us,
            error: decoded.error,
            tag_refs: Vec::new(),
        }
    }

    /// Forget requests older than `timeout_us` and return their `seq`s: they
    /// were never answered.
    pub fn expire(&mut self, now_us: i64, timeout_us: i64) -> Vec<u64> {
        let mut gone = Vec::new();
        self.pending.retain(|p| {
            let old = now_us - p.ts_us >= timeout_us;
            if old {
                gone.push(p.seq);
            }
            !old
        });
        gone
    }

    /// Forget every open request (the connection was closed).
    pub fn reset(&mut self) -> Vec<u64> {
        self.pending.drain(..).map(|p| p.seq).collect()
    }
}

/// The error a request gets when no response came.
pub fn no_response(timeout_ms: u64) -> ErrorInfo {
    ErrorInfo {
        kind: ErrorKind::NoResponse,
        code: None,
        title: "No response".into(),
        explanation: format!(
            "The device did not answer within {timeout_ms} ms. Check that the unit id is right (a gateway or device ignores requests for other units), that the device is powered and reachable, and that no other master holds its only connection."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adu(tid: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut v = tid.to_be_bytes().to_vec();
        v.extend([0, 0]);
        v.extend(((pdu.len() + 1) as u16).to_be_bytes());
        v.push(unit);
        v.extend(pdu);
        v
    }

    #[test]
    fn pairs_a_read_and_numbers_registers_from_the_request() {
        let mut s = Session::new();
        let req = s.decode(&adu(7, 1, &[3, 0x01, 0xF4, 0, 2]), Dir::Tx, 1_000, 1);
        assert_eq!(req.summary, "Read Holding Registers 500 x2 (40501-40502)");
        assert_eq!(req.range.unwrap().start, 500);
        let resp = s.decode(&adu(7, 1, &[3, 4, 0, 1, 0, 2]), Dir::Rx, 3_500, 2);
        assert_eq!(resp.pair_id, Some(1));
        assert_eq!(resp.latency_us, Some(2_500));
        assert_eq!(resp.range, req.range);
        let nums: Vec<String> = resp
            .walk()
            .filter(|f| f.id == "modbus.regnum16")
            .map(Field::show)
            .collect();
        assert_eq!(nums, ["500", "501"]);
        let v = resp.field("modbus.regval_uint16").unwrap();
        assert_eq!((v.offset, v.len, v.show().as_str()), (9, 2, "1"));
    }

    #[test]
    fn exception_02_explains_the_address_offset() {
        let mut s = Session::new();
        s.decode(&adu(1, 1, &[3, 0x03, 0xE8, 0, 1]), Dir::Tx, 0, 1);
        let resp = s.decode(&adu(1, 1, &[0x83, 2]), Dir::Rx, 10, 2);
        let err = resp.error.clone().unwrap();
        assert_eq!(err.kind, ErrorKind::Exception);
        assert_eq!(err.code, Some(2));
        assert!(err.explanation.contains("off by one"));
        assert!(err.explanation.contains("wire address 1000 (41001)"));
        assert_eq!(resp.function, Some(3));
        assert_eq!(resp.field("modbus.func_code").unwrap().show(), "3");
    }

    #[test]
    fn truncated_and_garbage_frames_never_panic() {
        let mut s = Session::new();
        let full = adu(1, 1, &[16, 0, 1, 0, 2, 4, 0, 1, 0, 2]);
        for n in 0..full.len() {
            for dir in [Dir::Tx, Dir::Rx] {
                let f = s.decode(&full[..n], dir, 0, n as u64);
                assert!(f.error.is_some(), "{n} bytes decoded without an error");
            }
        }
        // Every byte value as function code, both directions, short and long.
        for fc in 0..=255u8 {
            for data in [&[][..], &[0xFF; 3][..], &[0x05; 40][..]] {
                let mut pdu = vec![fc];
                pdu.extend(data);
                let a = adu(2, 1, &pdu);
                s.decode(&a, Dir::Tx, 0, 0);
                s.decode(&a, Dir::Rx, 0, 0);
            }
        }
        // A byte count larger than the frame.
        let f = s.decode(&adu(3, 1, &[3, 200, 0, 1]), Dir::Rx, 0, 0);
        assert_eq!(f.error.unwrap().kind, ErrorKind::Malformed);
    }

    #[test]
    fn splitter_reassembles_and_splits() {
        let a = adu(1, 1, &[3, 0, 0, 0, 1]);
        let b = adu(2, 1, &[4, 0, 0, 0, 1]);
        let mut stream = a.clone();
        stream.extend(&b);
        let mut sp = Splitter::default();
        let mut got = Vec::new();
        for chunk in stream.chunks(5) {
            sp.push(chunk, |f| got.push(f.to_vec()));
        }
        assert_eq!(got, vec![a, b]);
        got.clear();
        sp.push(&[1, 2, 3, 4, 5, 6, 7, 8], |f| got.push(f.to_vec()));
        assert_eq!(got.len(), 1, "garbage is handed over once");
        assert!(sp.buf.is_empty());
    }

    #[test]
    fn unanswered_requests_expire() {
        let mut s = Session::new();
        s.decode(&adu(1, 99, &[3, 0, 0, 0, 1]), Dir::Tx, 0, 10);
        s.decode(&adu(2, 1, &[3, 0, 0, 0, 1]), Dir::Tx, 4_000_000, 11);
        assert_eq!(s.expire(5_000_000, 5_000_000), vec![10]);
        assert_eq!(s.reset(), vec![11]);
    }

    #[test]
    fn references() {
        assert_eq!(reference("holding", 0), "40001");
        assert_eq!(reference("input", 9998), "39999");
        assert_eq!(reference("holding", 9999), "410000");
        assert_eq!(reference("coil", 12), "00013");
    }
}
