#![allow(dead_code)]
// ── S7 Protocol Constants ──

pub(crate) const TPKT_VERSION: u8 = 3;
pub(crate) const COTP_CR: u8 = 0xE0;
pub(crate) const COTP_CC: u8 = 0xD0;
pub(crate) const COTP_DT: u8 = 0xF0;

pub(crate) const S7_PROTOCOL_ID: u8 = 0x32;
pub(crate) const S7_MSG_TYPE_JOB: u8 = 0x01;
pub(crate) const S7_MSG_TYPE_ACK_DATA: u8 = 0x03;

pub(crate) const S7_SETUP_COMM: u8 = 0xF0;
pub(crate) const S7_READ_VAR: u8 = 0x04;
pub(crate) const S7_WRITE_VAR: u8 = 0x05;

// Area codes
pub(crate) const AREA_DB: u8 = 0x84;
pub(crate) const AREA_INPUT: u8 = 0x81;
pub(crate) const AREA_OUTPUT: u8 = 0x82;
pub(crate) const AREA_MARKER: u8 = 0x83;
pub(crate) const AREA_TIMER: u8 = 0x1D;
pub(crate) const AREA_COUNTER: u8 = 0x1C;

// Transport sizes
pub(crate) const TS_BIT: u8 = 0x01;
pub(crate) const TS_BYTE: u8 = 0x02;
pub(crate) const TS_WORD: u8 = 0x04;
pub(crate) const TS_DWORD: u8 = 0x06;
pub(crate) const TS_REAL: u8 = 0x08;

// Transport sizes of a data item (the "return code / transport size / length"
// header in front of written data). BIT and BYTE/WORD/DWORD lengths count
// bits, REAL and octet-string lengths count bytes.
pub(crate) const DATA_TS_BIT: u8 = 0x03;
pub(crate) const DATA_TS_BYTE: u8 = 0x04;
pub(crate) const DATA_TS_REAL: u8 = 0x07;
pub(crate) const DATA_TS_OCTET: u8 = 0x09;

/// Per-item return code of a successful read or write.
pub(crate) const ITEM_OK: u8 = 0xFF;

// S7 error codes
pub(crate) const S7_ERR_NONE: u16 = 0x0000;

// Default PDU size
pub(crate) const DEFAULT_PDU_SIZE: u16 = 480;
pub(crate) const MAX_PDU_SIZE: u16 = 960;

// ── S7 Data Types ──

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7Area {
    DB,
    Input,
    Output,
    Marker,
    Timer,
    Counter,
}

impl S7Area {
    pub fn code(&self) -> u8 {
        match self {
            S7Area::DB => AREA_DB,
            S7Area::Input => AREA_INPUT,
            S7Area::Output => AREA_OUTPUT,
            S7Area::Marker => AREA_MARKER,
            S7Area::Timer => AREA_TIMER,
            S7Area::Counter => AREA_COUNTER,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            AREA_DB => Some(S7Area::DB),
            AREA_INPUT => Some(S7Area::Input),
            AREA_OUTPUT => Some(S7Area::Output),
            AREA_MARKER => Some(S7Area::Marker),
            AREA_TIMER => Some(S7Area::Timer),
            AREA_COUNTER => Some(S7Area::Counter),
            _ => None,
        }
    }
}

impl std::fmt::Display for S7Area {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            S7Area::DB => write!(f, "DB"),
            S7Area::Input => write!(f, "Input"),
            S7Area::Output => write!(f, "Output"),
            S7Area::Marker => write!(f, "Marker"),
            S7Area::Timer => write!(f, "Timer"),
            S7Area::Counter => write!(f, "Counter"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S7DataType {
    Bool,
    Byte,
    Word,
    /// Signed 16-bit (S7 INT).
    Int,
    DWord,
    /// Signed 32-bit (S7 DINT).
    DInt,
    Real,
    String,
}

impl S7DataType {
    pub fn transport_size(&self) -> u8 {
        match self {
            S7DataType::Bool => TS_BIT,
            S7DataType::Byte => TS_BYTE,
            S7DataType::Word | S7DataType::Int => TS_WORD,
            S7DataType::DWord | S7DataType::DInt => TS_DWORD,
            S7DataType::Real => TS_REAL,
            S7DataType::String => TS_BYTE, // Strings are transported as byte arrays
        }
    }

    pub fn byte_size(&self) -> u16 {
        match self {
            S7DataType::Bool => 1,
            S7DataType::Byte => 1,
            S7DataType::Word | S7DataType::Int => 2,
            S7DataType::DWord | S7DataType::DInt => 4,
            S7DataType::Real => 4,
            S7DataType::String => 1, // Per character, length determines total
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bool" | "bit" => Some(S7DataType::Bool),
            "byte" | "uint8" => Some(S7DataType::Byte),
            "word" | "uint16" => Some(S7DataType::Word),
            "int" | "int16" => Some(S7DataType::Int),
            "dword" | "uint32" => Some(S7DataType::DWord),
            "dint" | "int32" => Some(S7DataType::DInt),
            "real" | "float" | "float32" => Some(S7DataType::Real),
            "string" | "str" => Some(S7DataType::String),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S7Tag {
    pub area: S7Area,
    pub db_number: u16,
    pub offset: u16,
    pub bit_offset: u8,
    pub data_type: S7DataType,
    pub length: u16,
    /// Human-readable name from configuration (e.g. "voltage", "current").
    /// Falls back to "{area}_{offset}" when not set.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum S7Value {
    Bool(bool),
    Byte(u8),
    Word(u16),
    Int(i16),
    DWord(u32),
    DInt(i32),
    Real(f32),
    String(String),
}

impl S7Value {
    pub fn to_f64(&self) -> f64 {
        match self {
            S7Value::Bool(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            S7Value::Byte(v) => *v as f64,
            S7Value::Word(v) => *v as f64,
            S7Value::Int(v) => *v as f64,
            S7Value::DWord(v) => *v as f64,
            S7Value::DInt(v) => *v as f64,
            S7Value::Real(v) => *v as f64,
            S7Value::String(_) => 0.0,
        }
    }
}

impl S7Tag {
    /// The tag a configured address and data type name describe, as both the
    /// poller and queued writes use it. The address fixes the area, offset and
    /// width (`DB1.DBD4` is four bytes); `data_type` says how to interpret
    /// those bytes (`real`, `dint` or `dword` for a D address). An empty
    /// `data_type` keeps the address's own unsigned type.
    pub fn resolve(address: &str, data_type: &str) -> anyhow::Result<Self> {
        let mut tag = super::S7Client::parse_tag_address(address)?;
        tag.data_type = Self::interpret(&tag.data_type, data_type)
            .map_err(|e| anyhow::anyhow!("S7 tag '{address}': {e}"))?;
        Ok(tag)
    }

    /// A tag from separate area / DB / offset / bit fields (the older
    /// `source_config` form), with an explicit data type.
    pub fn from_parts(
        area: &str,
        db_number: u16,
        offset: u16,
        bit_offset: u8,
        data_type: &str,
    ) -> anyhow::Result<Self> {
        let area =
            parse_area_name(area).ok_or_else(|| anyhow::anyhow!("Unknown S7 area '{area}'"))?;
        let data_type = S7DataType::from_str_name(data_type)
            .ok_or_else(|| anyhow::anyhow!("Unknown S7 data type '{data_type}'"))?;
        if bit_offset > 7 {
            anyhow::bail!("S7 bit offset {bit_offset} is not 0-7");
        }
        Ok(Self {
            area,
            db_number,
            offset,
            bit_offset: if data_type == S7DataType::Bool {
                bit_offset
            } else {
                0
            },
            length: if data_type == S7DataType::String {
                254
            } else {
                1
            },
            data_type,
            name: None,
        })
    }

    fn interpret(addressed: &S7DataType, configured: &str) -> Result<S7DataType, String> {
        let configured = configured.trim();
        if configured.is_empty() {
            return Ok(addressed.clone());
        }
        let wanted = S7DataType::from_str_name(configured)
            .ok_or_else(|| format!("unknown data type '{configured}'"))?;
        let fits = match addressed {
            S7DataType::Bool => wanted == S7DataType::Bool,
            S7DataType::String => wanted == S7DataType::String,
            _ => wanted != S7DataType::Bool && wanted.byte_size() == addressed.byte_size(),
        };
        if fits {
            Ok(wanted)
        } else {
            Err(format!(
                "data type '{configured}' does not fit a {addressed:?} address"
            ))
        }
    }

    /// The value to send for an engineering value, refusing one the type
    /// cannot hold rather than wrapping it.
    pub fn value_for(&self, value: f64) -> Result<S7Value, String> {
        if !value.is_finite() {
            return Err(format!("Value {value} is not a finite number"));
        }
        let integer = |min: f64, max: f64| -> Result<f64, String> {
            let rounded = value.round();
            if rounded < min || rounded > max {
                Err(format!(
                    "Value {value} is outside the {:?} range {min}..={max}",
                    self.data_type
                ))
            } else {
                Ok(rounded)
            }
        };
        Ok(match self.data_type {
            S7DataType::Bool => S7Value::Bool(value != 0.0),
            S7DataType::Byte => S7Value::Byte(integer(0.0, u8::MAX as f64)? as u8),
            S7DataType::Word => S7Value::Word(integer(0.0, u16::MAX as f64)? as u16),
            S7DataType::Int => S7Value::Int(integer(i16::MIN as f64, i16::MAX as f64)? as i16),
            S7DataType::DWord => S7Value::DWord(integer(0.0, u32::MAX as f64)? as u32),
            S7DataType::DInt => S7Value::DInt(integer(i32::MIN as f64, i32::MAX as f64)? as i32),
            S7DataType::Real => {
                if value.abs() > f32::MAX as f64 {
                    return Err(format!("Value {value} is outside the REAL range"));
                }
                S7Value::Real(value as f32)
            }
            S7DataType::String => S7Value::String(value.to_string()),
        })
    }
}

/// An area name (case-insensitive): "DB", "Input"/"I", "Output"/"Q",
/// "Marker"/"M", "Timer"/"T", "Counter"/"C".
pub(crate) fn parse_area_name(s: &str) -> Option<S7Area> {
    match s.to_lowercase().as_str() {
        "db" | "data" => Some(S7Area::DB),
        "input" | "i" => Some(S7Area::Input),
        "output" | "q" | "o" => Some(S7Area::Output),
        "marker" | "m" => Some(S7Area::Marker),
        "timer" | "t" => Some(S7Area::Timer),
        "counter" | "c" => Some(S7Area::Counter),
        _ => None,
    }
}
