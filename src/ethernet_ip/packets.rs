#![allow(clippy::vec_init_then_push)]
use anyhow::Result;

use super::types::*;

// ── Packet construction helpers ──

pub fn build_enip_header(
    command: u16,
    session_handle: u32,
    sender_context: u64,
    body: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(ENIP_HEADER_SIZE + body.len());
    buf.extend_from_slice(&command.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(&session_handle.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // status
    buf.extend_from_slice(&sender_context.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // options
    buf.extend_from_slice(body);
    buf
}

/// Build a CIP Read Tag request for explicit messaging.
pub fn build_read_tag_request(tag_name: &str) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: Read Tag
    request.push(CIP_READ_TAG);

    // Request path: encode tag name as symbolic segment
    let path = build_symbolic_path(tag_name);
    request.push((path.len() / 2) as u8); // path size in words
    request.extend_from_slice(&path);

    // Number of elements to read
    request.extend_from_slice(&1u16.to_le_bytes());

    request
}

/// Build a CIP Write Tag request.
pub fn build_write_tag_request(tag_name: &str, value: &CipValue) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: Write Tag
    request.push(CIP_WRITE_TAG);

    // Request path
    let path = build_symbolic_path(tag_name);
    request.push((path.len() / 2) as u8);
    request.extend_from_slice(&path);

    // Data type
    let (type_code, data) = encode_cip_value(value);
    request.extend_from_slice(&(type_code as u16).to_le_bytes());

    // Number of elements
    request.extend_from_slice(&1u16.to_le_bytes());

    // Value data
    request.extend_from_slice(&data);

    request
}

/// Build a CIP GetAttributeAll request for a class instance.
pub fn build_get_attribute_all_request(class_id: u8, instance_id: u32) -> Vec<u8> {
    let mut request = Vec::new();

    // CIP service: GetAttributeAll
    request.push(CIP_GET_ATTRIBUTES_ALL);

    // Path: class + instance
    request.push(0x03); // 3 words

    // Class segment: 0x20 = logical class segment, 8-bit
    request.push(0x20);
    request.push(class_id);
    request.push(0x00); // padding

    // Instance segment: 0x24 = logical instance segment, 8-bit
    if instance_id <= 255 {
        request.push(0x24);
        request.push(instance_id as u8);
        request.push(0x00);
        request.push(0x00);
    } else {
        // 16-bit instance
        request[1] = 0x04; // 4 words
        request.push(0x25); // logical instance segment, 16-bit
        request.push(0x00); // padding
        request.extend_from_slice(&(instance_id as u16).to_le_bytes());
    }

    request
}

/// Encode a tag name as a CIP symbolic path segment.
pub fn build_symbolic_path(tag_name: &str) -> Vec<u8> {
    let bytes = tag_name.as_bytes();
    let len = bytes.len();

    let mut path = Vec::with_capacity(len + 2);

    if len <= 255 {
        // Extended symbolic segment: type 0x91
        path.push(0x91);
        path.push(len as u8);
        path.extend_from_slice(bytes);

        // Pad to even length
        if !len.is_multiple_of(2) {
            path.push(0x00);
        }
    } else {
        // Long symbolic segment (unlikely but handle it)
        path.push(0x91);
        path.push(0x00); // placeholder
        path.extend_from_slice(&(len as u16).to_le_bytes());
        path.extend_from_slice(bytes);
        while path.len() % 2 != 0 {
            path.push(0x00);
        }
    }

    path
}

/// Encode a CipValue to (type_code, data bytes).
pub fn encode_cip_value(value: &CipValue) -> (u8, Vec<u8>) {
    match value {
        CipValue::Bool(b) => (
            CIP_TYPE_BOOL,
            vec![if *b { 0xFF } else { 0x00 }, 0x00, 0x00, 0x00],
        ),
        CipValue::Sint(v) => (CIP_TYPE_SINT, vec![*v as u8, 0x00, 0x00, 0x00]),
        CipValue::Int(v) => (CIP_TYPE_INT, v.to_le_bytes().to_vec()),
        CipValue::Dint(v) => (CIP_TYPE_DINT, v.to_le_bytes().to_vec()),
        CipValue::Real(v) => (CIP_TYPE_REAL, v.to_le_bytes().to_vec()),
        CipValue::String(s) => {
            let bytes = s.as_bytes();
            let mut data = Vec::with_capacity(88);
            let len = bytes.len().min(82) as u32;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(&bytes[..len as usize]);
            while data.len() < 88 {
                data.push(0x00);
            }
            (CIP_TYPE_STRING, data)
        }
        CipValue::Bytes(b) => (CIP_TYPE_DWORD, {
            let mut d = b.clone();
            while d.len() < 4 {
                d.push(0x00);
            }
            d
        }),
    }
}

/// Parse a CIP Read Tag response into a CipValue.
pub fn parse_read_tag_response(data: &[u8]) -> Result<CipValue> {
    if data.len() < 4 {
        anyhow::bail!("Read tag response too short ({} bytes)", data.len());
    }

    // CIP reply service (should be 0xCC = Read Tag reply)
    let service = data[0];
    if service != CIP_READ_TAG | 0x80 {
        anyhow::bail!("Unexpected CIP service reply: 0x{:02X}", service);
    }

    // Check for extended status
    let _reserved = data[1];
    let status = data[2];
    if status != CIP_STATUS_SUCCESS && status != CIP_STATUS_PARTIAL {
        let ext_status = if data.len() > 4 { data[3] } else { 0 };
        anyhow::bail!(
            "CIP Read Tag error: status=0x{:02X}, ext=0x{:02X}",
            status,
            ext_status
        );
    }

    if data.len() < 6 {
        anyhow::bail!("Read tag response missing type/data");
    }

    let type_code = u16::from_le_bytes(data[4..6].try_into().unwrap());
    let type_low = type_code as u8;

    // For partial reads, skip to data after the type field
    let data_start = 6;
    let value_data = &data[data_start..];

    decode_cip_value(type_low, value_data)
}

/// Parse a CIP Write Tag response.
pub fn parse_write_tag_response(data: &[u8]) -> Result<()> {
    if data.len() < 4 {
        anyhow::bail!("Write tag response too short");
    }

    let service = data[0];
    if service != CIP_WRITE_TAG | 0x80 {
        anyhow::bail!("Unexpected CIP service reply: 0x{:02X}", service);
    }

    let status = data[2];
    if status != CIP_STATUS_SUCCESS {
        let ext_status = if data.len() > 4 { data[3] } else { 0 };
        anyhow::bail!(
            "CIP Write Tag error: status=0x{:02X}, ext=0x{:02X}",
            status,
            ext_status
        );
    }

    Ok(())
}

/// Decode a CIP value from type code and raw data.
pub fn decode_cip_value(type_code: u8, data: &[u8]) -> Result<CipValue> {
    match type_code {
        CIP_TYPE_BOOL => {
            if data.is_empty() {
                anyhow::bail!("No data for BOOL");
            }
            Ok(CipValue::Bool(data[0] != 0))
        }
        CIP_TYPE_SINT => {
            if data.is_empty() {
                anyhow::bail!("No data for SINT");
            }
            Ok(CipValue::Sint(data[0] as i8))
        }
        CIP_TYPE_INT => {
            if data.len() < 2 {
                anyhow::bail!("Not enough data for INT");
            }
            Ok(CipValue::Int(i16::from_le_bytes(
                data[0..2].try_into().unwrap(),
            )))
        }
        CIP_TYPE_DINT => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for DINT");
            }
            Ok(CipValue::Dint(i32::from_le_bytes(
                data[0..4].try_into().unwrap(),
            )))
        }
        CIP_TYPE_REAL => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for REAL");
            }
            Ok(CipValue::Real(f32::from_le_bytes(
                data[0..4].try_into().unwrap(),
            )))
        }
        CIP_TYPE_STRING => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for STRING header");
            }
            let str_len = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
            let str_data = if data.len() >= 4 + str_len {
                &data[4..4 + str_len]
            } else {
                &data[4..]
            };
            Ok(CipValue::String(
                String::from_utf8_lossy(str_data).to_string(),
            ))
        }
        CIP_TYPE_DWORD => {
            if data.len() < 4 {
                anyhow::bail!("Not enough data for DWORD");
            }
            Ok(CipValue::Bytes(data[0..4].to_vec()))
        }
        CIP_TYPE_LINT => {
            if data.len() < 8 {
                anyhow::bail!("Not enough data for LINT");
            }
            // Store as bytes since we don't have a Lint variant in CipValue
            Ok(CipValue::Bytes(data[0..8].to_vec()))
        }
        _ => {
            // Unknown type — return raw bytes
            Ok(CipValue::Bytes(data.to_vec()))
        }
    }
}

/// Parse a Symbol class instance to extract tag info.
pub fn parse_symbol_instance(data: &[u8]) -> Option<CipTag> {
    // The symbol instance data structure from GetAttributeAll:
    // Attribute 1: name (STRING)
    // Attribute 2: type (UINT)
    // The actual layout varies but we try a best-effort parse.

    if data.len() < 8 {
        return None;
    }

    // Try to read name length (first 4 bytes for CIP STRING len)
    let name_len = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    if name_len == 0 || name_len > 255 || data.len() < 4 + name_len + 2 {
        return None;
    }

    let name_bytes = &data[4..4 + name_len];
    let name = String::from_utf8(name_bytes.to_vec()).ok()?;
    if name.is_empty() {
        return None;
    }

    // After the name, skip padding to even boundary, then read type info
    let type_offset = 4 + name_len + (name_len % 2);
    if data.len() < type_offset + 2 {
        return None;
    }

    // Type word: upper byte has element count flags, lower byte has type code
    let type_word = u16::from_le_bytes(data[type_offset..type_offset + 2].try_into().ok()?);
    let type_code = (type_word & 0xFF) as u8;
    let element_count = (type_word >> 8) & 0xFF;

    let data_type = CipDataType::from_code(type_code)?;

    Some(CipTag {
        name,
        data_type,
        element_count: if element_count == 0 { 1 } else { element_count },
    })
}
