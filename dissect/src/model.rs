//! The model every decoder produces.

use serde::{Serialize, Serializer};

/// Who sent a frame, seen from the master (Voltrus, or the client in a capture).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    /// Master to device: a request.
    Tx,
    /// Device to master: a response (or an unsolicited message).
    Rx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Proto {
    ModbusTcp,
}

/// One protocol data unit, decoded.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedFrame {
    /// Position in its stream (the packet number for a capture file); other
    /// frames refer to it through `pair_id`.
    pub seq: u64,
    /// Microseconds since the Unix epoch.
    pub ts_us: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    pub dir: Dir,
    pub proto: Proto,
    pub summary: String,
    /// Function code (Modbus: without the exception bit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<u16>,
    /// The data the frame reads or writes (a response carries its request's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<AddrRange>,
    pub fields: Vec<Field>,
    #[serde(serialize_with = "hex_bytes")]
    pub raw: Vec<u8>,
    /// `seq` of the matching request or response.
    pub pair_id: Option<u64>,
    /// Request to response, on both frames of a pair.
    pub latency_us: Option<u64>,
    pub error: Option<ErrorInfo>,
    /// Filled in by the host: the tags this frame read or wrote.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tag_refs: Vec<TagRef>,
}

impl DecodedFrame {
    /// Every field of the tree, depth first, in wire order.
    pub fn walk(&self) -> impl Iterator<Item = &Field> {
        let mut stack: Vec<&Field> = self.fields.iter().rev().collect();
        std::iter::from_fn(move || {
            let f = stack.pop()?;
            stack.extend(f.children.iter().rev());
            Some(f)
        })
    }

    /// The first field with this id.
    pub fn field(&self, id: &str) -> Option<&Field> {
        self.walk().find(|f| f.id == id)
    }
}

/// A block of protocol addresses, 0-based as on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AddrRange {
    /// Modbus: `coil`, `discrete`, `input` or `holding`.
    pub space: &'static str,
    pub start: u32,
    pub count: u32,
}

impl AddrRange {
    pub fn overlaps(&self, start: u32, end_inclusive: u32) -> bool {
        let last = self.start + self.count.max(1) - 1;
        self.start <= end_inclusive && start <= last
    }
}

/// One node of the decoded tree.
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    /// Wireshark display-filter name, or `""` for a text node that only
    /// groups its children (Wireshark shows those without a filter name too).
    pub id: &'static str,
    pub label: &'static str,
    pub value: Value,
    /// How Wireshark prints the value; used by [`Field::show`].
    #[serde(skip)]
    pub base: Base,
    /// The line shown in the tree.
    pub display: String,
    /// Byte offset of the field in `raw`, and its length.
    pub offset: u32,
    pub len: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Field>,
}

impl Field {
    pub fn new(
        id: &'static str,
        label: &'static str,
        value: Value,
        offset: usize,
        len: usize,
    ) -> Self {
        let mut f = Self {
            id,
            label,
            value,
            base: Base::Dec,
            display: String::new(),
            offset: offset as u32,
            len: len as u32,
            children: Vec::new(),
        };
        f.display = format!("{label}: {}", f.show());
        f
    }

    pub fn hex(mut self, digits: u8) -> Self {
        self.base = Base::Hex(digits);
        self.display = format!("{}: {}", self.label, self.show());
        self
    }

    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = display.into();
        self
    }

    /// A text node grouping `children`.
    pub fn group(
        display: impl Into<String>,
        offset: usize,
        len: usize,
        children: Vec<Field>,
    ) -> Self {
        Self {
            id: "",
            label: "",
            value: Value::None,
            base: Base::Dec,
            display: display.into(),
            offset: offset as u32,
            len: len as u32,
            children,
        }
    }

    /// The value as `tshark -T json` prints it: decimal or `0x`-padded hex
    /// integers, `1`/`0` booleans, colon-separated bytes.
    pub fn show(&self) -> String {
        match (&self.value, self.base) {
            (Value::Uint(v), Base::Hex(d)) => format!("0x{v:0width$x}", width = d as usize),
            (Value::Uint(v), _) => v.to_string(),
            (Value::Int(v), _) => v.to_string(),
            (Value::Bool(b), _) => if *b { "1" } else { "0" }.to_string(),
            (Value::Bytes(b), _) => b
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect::<Vec<_>>()
                .join(":"),
            (Value::Str(s), _) => s.clone(),
            (Value::Float(f), _) => f.to_string(),
            (Value::None, _) => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Base {
    #[default]
    Dec,
    /// Hex with this many digits.
    Hex(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Uint(u64),
    Int(i64),
    Bool(bool),
    Bytes(Vec<u8>),
    Str(String),
    Float(f64),
    None,
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Uint(v) => s.serialize_u64(*v),
            Value::Int(v) => s.serialize_i64(*v),
            Value::Bool(v) => s.serialize_bool(*v),
            Value::Bytes(v) => s.serialize_str(&to_hex(v)),
            Value::Str(v) => s.serialize_str(v),
            Value::Float(v) => s.serialize_f64(*v),
            Value::None => s.serialize_none(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The device answered with a protocol exception.
    Exception,
    /// The frame does not parse.
    Malformed,
    /// A request that no response answered in time.
    NoResponse,
}

/// What went wrong, in words an engineer on site can act on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    /// Protocol error code (Modbus exception code).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    pub title: String,
    pub explanation: String,
}

/// A tag fed by a frame; `offset`/`len` point at its bytes in `raw`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagRef {
    /// The register or point name (the tag's `key`).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub len: Option<u32>,
}

pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

fn hex_bytes<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&to_hex(bytes))
}
