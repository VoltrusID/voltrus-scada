#![allow(dead_code)]
// src/compression/rle.rs — RLE (Run-Length Encoding) for digital/binary tags

use anyhow::{Result, bail};

/// RLE-encode boolean values into (value, count) pairs.
pub fn rle_encode(values: &[bool]) -> Vec<(bool, u32)> {
    if values.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut current = values[0];
    let mut count: u32 = 1;
    for &v in &values[1..] {
        if v == current && count < u32::MAX {
            count += 1;
        } else {
            result.push((current, count));
            current = v;
            count = 1;
        }
    }
    result.push((current, count));
    result
}

/// Decode RLE (value, count) pairs back to booleans.
pub fn rle_decode(runs: &[(bool, u32)]) -> Vec<bool> {
    let total: usize = runs.iter().map(|(_, c)| *c as usize).sum();
    let mut result = Vec::with_capacity(total);
    for (value, count) in runs {
        for _ in 0..*count {
            result.push(*value);
        }
    }
    result
}

/// Serialize RLE runs to binary: per run: 1 byte (0/1) + 4 bytes (u32 LE)
pub fn rle_serialize(runs: &[(bool, u32)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(runs.len() * 5);
    for (value, count) in runs {
        buf.push(if *value { 1 } else { 0 });
        buf.extend_from_slice(&count.to_le_bytes());
    }
    buf
}

/// Deserialize RLE runs from binary.
pub fn rle_deserialize(data: &[u8]) -> Result<Vec<(bool, u32)>> {
    if !data.len().is_multiple_of(5) {
        bail!("Invalid RLE data length: {}", data.len());
    }
    let mut runs = Vec::with_capacity(data.len() / 5);
    for chunk in data.as_chunks::<5>().0 {
        let value = chunk[0] != 0;
        let count = u32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]);
        runs.push((value, count));
    }
    Ok(runs)
}
