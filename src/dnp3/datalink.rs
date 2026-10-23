#![allow(dead_code)]
#![allow(clippy::ptr_arg)]
// src/dnp3/datalink.rs

use anyhow::Result;

// CRC-CCITT lookup table (polynomial 0x1021)
const CRC_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0u16;
    while i < 256 {
        let mut crc = i << 8;
        let mut j = 0;
        while j < 8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

pub fn crc_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0x0000;
    for &byte in data {
        crc = (crc << 8) ^ CRC_TABLE[((crc >> 8) ^ byte as u16) as usize];
    }
    crc
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlByte {
    pub master: bool,
    pub fcb: bool,
    pub fcv: bool,
    pub sequence: u8,
}

impl ControlByte {
    pub fn to_byte(&self) -> u8 {
        let dir = if self.master { 0x80 } else { 0x00 };
        let prm = if self.master { 0x40 } else { 0x00 };
        let fcb = if self.fcb { 0x20 } else { 0x00 };
        let fcv = if self.fcv { 0x10 } else { 0x00 };
        let func = self.sequence & 0x0F;
        dir | prm | fcb | fcv | func
    }

    pub fn from_byte(byte: u8) -> Self {
        Self {
            master: byte & 0x80 != 0,
            fcb: byte & 0x20 != 0,
            fcv: byte & 0x10 != 0,
            sequence: byte & 0x0F,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dnp3Frame {
    pub master: bool,
    pub fcb: bool,
    pub fcv: bool,
    pub sequence: u8,
    pub destination: u16,
    pub source: u16,
    pub data: Vec<u8>,
}

impl Dnp3Frame {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let ctrl = ControlByte {
            master: self.master,
            fcb: self.fcb,
            fcv: self.fcv,
            sequence: self.sequence,
        };
        let user_data_len = 5 + self.data.len();

        if user_data_len > 255 {
            anyhow::bail!(
                "DNP3 data length overflow: {} bytes exceeds maximum of 255",
                user_data_len
            );
        }

        let mut raw = Vec::with_capacity(user_data_len + 10);
        raw.push(0x05);
        raw.push(0x64);
        raw.push(user_data_len as u8);
        raw.push(ctrl.to_byte());
        raw.push((self.destination & 0xFF) as u8);
        raw.push(((self.destination >> 8) & 0xFF) as u8);
        raw.push((self.source & 0xFF) as u8);
        raw.push(((self.source >> 8) & 0xFF) as u8);
        raw.extend_from_slice(&self.data);

        Ok(insert_crc_blocks(&mut raw))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 10 {
            anyhow::bail!("Frame too short: {} bytes", bytes.len());
        }
        if bytes[0] != 0x05 || bytes[1] != 0x64 {
            anyhow::bail!("Invalid start bytes: {:02X} {:02X}", bytes[0], bytes[1]);
        }

        let raw = strip_crc_blocks(bytes)?;

        let len = raw[2] as usize;
        if raw.len() < 3 + len {
            anyhow::bail!(
                "Frame truncated: expected {} bytes, got {}",
                len,
                raw.len() - 3
            );
        }

        let ctrl = ControlByte::from_byte(raw[3]);
        let destination = raw[4] as u16 | ((raw[5] as u16) << 8);
        let source = raw[6] as u16 | ((raw[7] as u16) << 8);
        let data = raw[8..3 + len].to_vec();

        Ok(Self {
            master: ctrl.master,
            fcb: ctrl.fcb,
            fcv: ctrl.fcv,
            sequence: ctrl.sequence,
            destination,
            source,
            data,
        })
    }
}

fn insert_crc_blocks(raw: &mut Vec<u8>) -> Vec<u8> {
    let mut result = Vec::new();
    result.extend_from_slice(&raw[0..3]);

    let user_data = &raw[3..];
    let mut offset = 0;
    while offset < user_data.len() {
        let block_end = std::cmp::min(offset + 16, user_data.len());
        let block = &user_data[offset..block_end];
        result.extend_from_slice(block);
        let crc = crc_ccitt(block);
        result.push((crc & 0xFF) as u8);
        result.push(((crc >> 8) & 0xFF) as u8);
        offset = block_end;
    }

    result
}

fn strip_crc_blocks(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < 6 {
        anyhow::bail!("Frame too short for CRC stripping");
    }

    let mut result = Vec::new();
    result.extend_from_slice(&bytes[0..3]);

    let mut offset = 3;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining <= 2 {
            break;
        }
        let data_len = std::cmp::min(remaining - 2, 16);
        let data = &bytes[offset..offset + data_len];
        let crc_bytes = &bytes[offset + data_len..offset + data_len + 2];
        let expected_crc = crc_ccitt(data);
        let actual_crc = crc_bytes[0] as u16 | ((crc_bytes[1] as u16) << 8);
        if expected_crc != actual_crc {
            anyhow::bail!(
                "CRC mismatch at offset {}: expected {:04X}, got {:04X}",
                offset,
                expected_crc,
                actual_crc
            );
        }
        result.extend_from_slice(data);
        offset += data_len + 2;
    }

    Ok(result)
}

fn verify_crc_blocks(bytes: &[u8]) -> bool {
    strip_crc_blocks(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc_empty() {
        let crc = crc_ccitt(&[]);
        assert_eq!(crc, 0x0000);
    }

    #[test]
    fn test_crc_known_pattern() {
        let crc = crc_ccitt(&[0x05, 0x64]);
        assert_ne!(crc, 0x0000);
    }

    #[test]
    fn test_encode_request_frame() {
        let frame = Dnp3Frame {
            master: true,
            fcb: true,
            fcv: true,
            sequence: 0,
            destination: 10,
            source: 1,
            data: vec![0xC0, 0x01],
        };
        let bytes = frame.encode().unwrap();
        assert_eq!(bytes[0], 0x05);
        assert_eq!(bytes[1], 0x64);
        assert_eq!(bytes[2], (5 + frame.data.len()) as u8);
    }

    #[test]
    fn test_decode_response_frame() {
        let frame = Dnp3Frame {
            master: false,
            fcb: true,
            fcv: true,
            sequence: 0,
            destination: 1,
            source: 10,
            data: vec![],
        };
        let bytes = frame.encode().unwrap();
        let decoded = Dnp3Frame::decode(&bytes).expect("decode should succeed");
        assert_eq!(decoded.master, false);
        assert_eq!(decoded.source, 10);
        assert_eq!(decoded.destination, 1);
    }

    #[test]
    fn test_control_byte_master_request() {
        let ctrl = ControlByte {
            master: true,
            fcb: true,
            fcv: true,
            sequence: 0,
        };
        let byte = ctrl.to_byte();
        assert_eq!(byte & 0x40, 0x40);
        assert_eq!(byte & 0x20, 0x20);
        assert_eq!(byte & 0x01, 0x00);
    }

    #[test]
    fn test_crc_roundtrip() {
        let frame = Dnp3Frame {
            master: true,
            fcb: true,
            fcv: true,
            sequence: 3,
            destination: 100,
            source: 1,
            data: vec![0x01, 0x02, 0x03, 0x04, 0x05],
        };
        let encoded = frame.encode().unwrap();
        let decoded = Dnp3Frame::decode(&encoded).expect("roundtrip decode should succeed");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn test_data_length_overflow() {
        let frame = Dnp3Frame {
            master: true,
            fcb: false,
            fcv: false,
            sequence: 0,
            destination: 10,
            source: 1,
            data: vec![0u8; 251], // 5 + 251 = 256 > 255
        };
        assert!(frame.encode().is_err());
    }
}
