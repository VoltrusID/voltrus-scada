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
    DWord,
    Real,
    String,
}

impl S7DataType {
    pub fn transport_size(&self) -> u8 {
        match self {
            S7DataType::Bool => TS_BIT,
            S7DataType::Byte => TS_BYTE,
            S7DataType::Word => TS_WORD,
            S7DataType::DWord => TS_DWORD,
            S7DataType::Real => TS_REAL,
            S7DataType::String => TS_BYTE, // Strings are transported as byte arrays
        }
    }

    pub fn byte_size(&self) -> u16 {
        match self {
            S7DataType::Bool => 1,
            S7DataType::Byte => 1,
            S7DataType::Word => 2,
            S7DataType::DWord => 4,
            S7DataType::Real => 4,
            S7DataType::String => 1, // Per character, length determines total
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bool" | "bit" => Some(S7DataType::Bool),
            "byte" | "uint8" => Some(S7DataType::Byte),
            "word" | "uint16" | "int16" => Some(S7DataType::Word),
            "dword" | "uint32" | "int32" => Some(S7DataType::DWord),
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
    DWord(u32),
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
            S7Value::DWord(v) => *v as f64,
            S7Value::Real(v) => *v as f64,
            S7Value::String(_) => 0.0,
        }
    }
}
