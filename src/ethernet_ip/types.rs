#![allow(dead_code)]
//! Allen-Bradley EtherNet/IP + CIP protocol client.
//!
//! Implements session registration, tag read/write, and tag browsing
//! for ControlLogix/CompactLogix PLCs over TCP port 44818.

// ── EtherNet/IP command codes ──

pub(crate) const CMD_REGISTER_SESSION: u16 = 0x0065;
pub(crate) const CMD_UNREGISTER_SESSION: u16 = 0x0066;
pub(crate) const CMD_SEND_RR_DATA: u16 = 0x006F;
pub(crate) const _CMD_SEND_UNIT_DATA: u16 = 0x0070;

// ── CIP service codes ──

pub(crate) const CIP_READ_TAG: u8 = 0x4C;
pub(crate) const CIP_READ_TAG_FRAGMENTED: u8 = 0x52;
pub(crate) const CIP_WRITE_TAG: u8 = 0x4D;
pub(crate) const CIP_GET_ATTRIBUTES_ALL: u8 = 0x01;
pub(crate) const _CIP_FORWARD_OPEN: u8 = 0x54;

// ── CIP data type codes ──

pub(crate) const CIP_TYPE_BOOL: u8 = 0xC1;
pub(crate) const CIP_TYPE_SINT: u8 = 0xC2;
pub(crate) const CIP_TYPE_INT: u8 = 0xC3;
pub(crate) const CIP_TYPE_DINT: u8 = 0xC4;
pub(crate) const CIP_TYPE_REAL: u8 = 0xCA;
pub(crate) const CIP_TYPE_STRING: u8 = 0xD0;
pub(crate) const CIP_TYPE_DWORD: u8 = 0xD3;
pub(crate) const CIP_TYPE_LINT: u8 = 0xC5;

// ── CIP status codes ──

pub(crate) const CIP_STATUS_SUCCESS: u8 = 0x00;
pub(crate) const CIP_STATUS_PARTIAL: u8 = 0x06;

// ── Encapsulation header size ──

pub(crate) const ENIP_HEADER_SIZE: usize = 24;

// ── CIP path for Symbol class ──

pub(crate) const SYMBOL_CLASS_ID: u8 = 0x6B;

// ── Public types ──

#[derive(Debug, Clone, PartialEq)]
pub enum CipDataType {
    Bool,
    Sint,
    Int,
    Dint,
    Real,
    String,
    Dword,
    Lint,
}

impl CipDataType {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            CIP_TYPE_BOOL => Some(Self::Bool),
            CIP_TYPE_SINT => Some(Self::Sint),
            CIP_TYPE_INT => Some(Self::Int),
            CIP_TYPE_DINT => Some(Self::Dint),
            CIP_TYPE_REAL => Some(Self::Real),
            CIP_TYPE_STRING => Some(Self::String),
            CIP_TYPE_DWORD => Some(Self::Dword),
            CIP_TYPE_LINT => Some(Self::Lint),
            _ => None,
        }
    }

    pub fn code(&self) -> u8 {
        match self {
            Self::Bool => CIP_TYPE_BOOL,
            Self::Sint => CIP_TYPE_SINT,
            Self::Int => CIP_TYPE_INT,
            Self::Dint => CIP_TYPE_DINT,
            Self::Real => CIP_TYPE_REAL,
            Self::String => CIP_TYPE_STRING,
            Self::Dword => CIP_TYPE_DWORD,
            Self::Lint => CIP_TYPE_LINT,
        }
    }

    pub fn element_size(&self) -> u16 {
        match self {
            Self::Bool => 1,
            Self::Sint => 1,
            Self::Int => 2,
            Self::Dint => 4,
            Self::Real => 4,
            Self::Dword => 4,
            Self::Lint => 8,
            Self::String => 88, // CIP STRING struct: 4 bytes len + 82 bytes data + 2 pad
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Sint => "SINT",
            Self::Int => "INT",
            Self::Dint => "DINT",
            Self::Real => "REAL",
            Self::String => "STRING",
            Self::Dword => "DWORD",
            Self::Lint => "LINT",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CipValue {
    Bool(bool),
    Sint(i8),
    Int(i16),
    Dint(i32),
    Real(f32),
    String(String),
    Bytes(Vec<u8>),
}

impl CipValue {
    /// Convert CIP value to f64 for cache storage.
    pub fn to_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Self::Sint(v) => Some(*v as f64),
            Self::Int(v) => Some(*v as f64),
            Self::Dint(v) => Some(*v as f64),
            Self::Real(v) => Some(*v as f64),
            Self::Bytes(_) => None,
            Self::String(_) => None,
        }
    }

    /// Get a display-friendly string representation.
    pub fn display_value(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Sint(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Dint(v) => v.to_string(),
            Self::Real(v) => format!("{:.4}", v),
            Self::String(s) => s.clone(),
            Self::Bytes(b) => format!("{:02x?}", b),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CipTag {
    pub name: String,
    pub data_type: CipDataType,
    pub element_count: u16,
}
