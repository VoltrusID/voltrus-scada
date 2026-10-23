#![allow(dead_code)]
// src/compression/gorilla.rs — Gorilla encoding (Facebook Gorilla paper)

use super::bitio::{BitReader, BitWriter};
use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct GorillaBlock {
    pub first_ts: i64,
    pub count: usize,
    pub first_value: f64,
    pub data: Vec<u8>,
}

/// Encode timestamps and values using Gorilla compression.
///
/// Timestamps are encoded with delta-of-delta. Values are encoded with XOR.
pub fn gorilla_encode(timestamps: &[i64], values: &[f64]) -> Result<GorillaBlock> {
    if timestamps.is_empty() || values.is_empty() {
        bail!("empty input: cannot encode zero data points");
    }
    if timestamps.len() != values.len() {
        bail!(
            "mismatched lengths: {} timestamps vs {} values",
            timestamps.len(),
            values.len()
        );
    }
    let count = timestamps.len();
    let first_ts = timestamps[0];
    let first_value = values[0];

    if count == 1 {
        return Ok(GorillaBlock {
            first_ts,
            count,
            first_value,
            data: Vec::new(),
        });
    }

    let mut writer = BitWriter::new();

    // --- Timestamp encoding (delta-of-delta) ---
    // The implicit "previous delta" before the first real delta is 0.
    // For each element i >= 1, dod = delta_i - prev_delta.
    let mut prev_delta: i64 = 0;
    for i in 1..count {
        let delta = timestamps[i] - timestamps[i - 1];
        let dod = delta - prev_delta;
        encode_dod(&mut writer, dod);
        prev_delta = delta;
    }

    // --- Value encoding (XOR) ---
    let mut prev_xor: u64 = 0;
    let mut prev_lz: u8 = 0;
    let mut prev_tz: u8 = 0;

    for i in 1..count {
        let cur_bits = values[i].to_bits();
        let prev_bits = values[i - 1].to_bits();
        let xor = cur_bits ^ prev_bits;

        if xor == 0 {
            writer.write_bit(false);
        } else {
            writer.write_bit(true);

            let lz = xor.leading_zeros() as u8;
            let tz = xor.trailing_zeros() as u8;
            let meaningful_bits = 64 - lz - tz;

            // Try to reuse previous window
            if prev_xor != 0 && lz >= prev_lz && tz >= prev_tz {
                writer.write_bit(false);
                writer.write_bits(xor >> tz, meaningful_bits);
            } else {
                writer.write_bit(true);
                writer.write_bits(lz as u64, 6);
                writer.write_bits((meaningful_bits - 1) as u64, 6);
                writer.write_bits(xor >> tz, meaningful_bits);
            }

            prev_xor = xor;
            prev_lz = lz;
            prev_tz = tz;
        }
    }

    let data = writer.finish();

    Ok(GorillaBlock {
        first_ts,
        count,
        first_value,
        data,
    })
}

/// Decode a Gorilla block back into timestamps and values.
pub fn gorilla_decode(block: &GorillaBlock) -> Result<(Vec<i64>, Vec<f64>)> {
    let count = block.count;
    if count == 0 {
        bail!("cannot decode block with count 0");
    }

    let mut timestamps = Vec::with_capacity(count);
    let mut values = Vec::with_capacity(count);

    timestamps.push(block.first_ts);
    values.push(block.first_value);

    if count == 1 {
        return Ok((timestamps, values));
    }

    let mut reader = BitReader::new(&block.data);

    // --- Timestamp decoding ---
    let mut prev_delta: i64 = 0;
    for i in 1..count {
        let dod = decode_dod(&mut reader).context("truncated timestamp data")?;
        let delta = prev_delta + dod;
        let ts = timestamps[i - 1] + delta;
        timestamps.push(ts);
        prev_delta = delta;
    }

    // --- Value decoding ---
    let mut prev_bits = block.first_value.to_bits();
    let mut prev_lz: u8 = 0;
    let mut prev_tz: u8 = 0;

    for _ in 1..count {
        let xor_nonzero = reader.read_bit().context("truncated value control bit")?;
        if !xor_nonzero {
            // xor == 0, same value as previous
            values.push(f64::from_bits(prev_bits));
        } else {
            let reuse_window = reader.read_bit().context("truncated value window bit")?;

            let xor;
            if reuse_window {
                // New window
                let lz = reader.read_bits(6).context("truncated leading zeros")? as u8;
                let meaningful_count =
                    reader.read_bits(6).context("truncated meaningful count")? as u8 + 1;
                let meaningful_bits = reader
                    .read_bits(meaningful_count)
                    .context("truncated meaningful value bits")?;
                xor = meaningful_bits << (64 - lz - meaningful_count);
                prev_lz = lz;
                prev_tz = 64 - lz - meaningful_count;
            } else {
                // Reuse previous window
                let meaningful_count = 64 - prev_lz - prev_tz;
                let meaningful_bits = reader
                    .read_bits(meaningful_count)
                    .context("truncated meaningful value bits (reuse)")?;
                xor = meaningful_bits << prev_tz;
            }

            let cur_bits = prev_bits ^ xor;
            values.push(f64::from_bits(cur_bits));
            prev_bits = cur_bits;
        }
    }

    Ok((timestamps, values))
}

/// Encode a delta-of-delta value into the bit writer.
fn encode_dod(writer: &mut BitWriter, dod: i64) {
    if dod == 0 {
        // 0
        writer.write_bit(false);
    } else {
        // 1...
        writer.write_bit(true);
        let abs = dod.unsigned_abs();
        if abs < 64 {
            // 10 + 7 bits (signed, offset by 64: maps -64..63 to 0..127)
            writer.write_bit(false);
            writer.write_bits((dod as u64).wrapping_add(64) & 0x7F, 7);
        } else if abs < 256 {
            // 110 + 9 bits (signed, offset by 256: maps -256..255 to 0..511)
            writer.write_bit(true);
            writer.write_bit(false);
            writer.write_bits((dod as u64).wrapping_add(256) & 0x1FF, 9);
        } else if abs < 2048 {
            // 1110 + 12 bits (signed, offset by 2048)
            writer.write_bit(true);
            writer.write_bit(true);
            writer.write_bit(false);
            writer.write_bits((dod as u64).wrapping_add(2048) & 0xFFF, 12);
        } else {
            // 1111 + 32 bits (signed, stored as raw i32 bits)
            writer.write_bit(true);
            writer.write_bit(true);
            writer.write_bit(true);
            writer.write_bits(dod as u32 as u64, 32);
        }
    }
}

/// Decode a delta-of-delta value from the bit reader.
fn decode_dod(reader: &mut BitReader) -> Option<i64> {
    let first = reader.read_bit()?;
    if !first {
        // 0 -> dod == 0
        return Some(0);
    }

    let second = reader.read_bit()?;
    if !second {
        // 10 prefix: 7 bits, signed with offset 64
        let raw = reader.read_bits(7)? as i64;
        Some(raw - 64)
    } else {
        let third = reader.read_bit()?;
        if !third {
            // 110 prefix: 9 bits, signed with offset 256
            let raw = reader.read_bits(9)? as i64;
            Some(raw - 256)
        } else {
            let fourth = reader.read_bit()?;
            if !fourth {
                // 1110 prefix: 12 bits, signed with offset 2048
                let raw = reader.read_bits(12)? as i64;
                Some(raw - 2048)
            } else {
                // 1111 prefix: 32 bits, signed i32
                let raw = reader.read_bits(32)? as u32 as i32 as i64;
                Some(raw)
            }
        }
    }
}
