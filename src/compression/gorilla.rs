// src/compression/gorilla.rs — Gorilla-style encoding (Facebook Gorilla paper)
//
// Timestamps use delta-of-delta coding, values use XOR coding of the IEEE-754
// bit patterns. Two details differ from the paper so that any input
// round-trips bit-exactly and so that steady polling stays cheap:
//
// * Timestamp arithmetic wraps, and a dod that does not fit the narrow
//   buckets is stored as a raw 64 bit value, so every `i64` sequence is
//   representable (out-of-order and duplicate timestamps included).
// * A run of identical intervals (dod == 0) of 23 or more samples is stored as
//   one 22 bit token instead of one bit per sample.
//
// NaN, +-inf, -0.0 and subnormals are preserved bit for bit.

use super::bitio::{BitReader, BitWriter};
use anyhow::{Context, Result, bail};

/// Most samples a single block may hold. Bounds the allocation a corrupt
/// header can request when decoding.
pub const MAX_BLOCK_SAMPLES: usize = 4 * 1024 * 1024;

/// Shortest run of zero dods that is cheaper as a run token (22 bits) than as
/// single `0` bits.
const MIN_RUN: usize = 23;
/// Run token carries `count - 1` in 16 bits.
const MAX_RUN: usize = 1 << 16;

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
    if timestamps.len() > MAX_BLOCK_SAMPLES {
        bail!(
            "block of {} samples exceeds the {MAX_BLOCK_SAMPLES} limit",
            timestamps.len()
        );
    }
    let count = timestamps.len();

    if count == 1 {
        return Ok(GorillaBlock {
            first_ts: timestamps[0],
            count,
            first_value: values[0],
            data: Vec::new(),
        });
    }

    let mut writer = BitWriter::new();
    encode_timestamps(&mut writer, timestamps);
    encode_xor_values(&mut writer, values);

    Ok(GorillaBlock {
        first_ts: timestamps[0],
        count,
        first_value: values[0],
        data: writer.finish(),
    })
}

/// Decode a Gorilla block back into timestamps and values.
pub fn gorilla_decode(block: &GorillaBlock) -> Result<(Vec<i64>, Vec<f64>)> {
    let count = block.count;
    if count == 0 {
        bail!("cannot decode block with count 0");
    }
    if count > MAX_BLOCK_SAMPLES {
        bail!("block count {count} exceeds the {MAX_BLOCK_SAMPLES} limit");
    }
    if count == 1 {
        return Ok((vec![block.first_ts], vec![block.first_value]));
    }

    let mut reader = BitReader::new(&block.data);
    let timestamps = decode_timestamps(&mut reader, block.first_ts, count)?;
    let values = decode_xor_values(&mut reader, block.first_value, count)?;
    Ok((timestamps, values))
}

/// Write the delta-of-delta stream for `timestamps[1..]`.
///
/// The implicit delta before the first real one is 0, so the first dod equals
/// the first delta.
pub(crate) fn encode_timestamps(writer: &mut BitWriter, timestamps: &[i64]) {
    let mut dods: Vec<i64> = Vec::with_capacity(timestamps.len().saturating_sub(1));
    let mut prev_delta: i64 = 0;
    for pair in timestamps.windows(2) {
        let delta = pair[1].wrapping_sub(pair[0]);
        dods.push(delta.wrapping_sub(prev_delta));
        prev_delta = delta;
    }

    let mut i = 0;
    while i < dods.len() {
        if dods[i] != 0 {
            encode_dod(writer, dods[i]);
            i += 1;
            continue;
        }
        let run = dods[i..].iter().take_while(|&&d| d == 0).count();
        let mut left = run;
        while left > 0 {
            let take = left.min(MAX_RUN);
            if take >= MIN_RUN {
                writer.write_bits(0b111111, 6);
                writer.write_bits((take - 1) as u64, 16);
            } else {
                for _ in 0..take {
                    writer.write_bit(false);
                }
            }
            left -= take;
        }
        i += run;
    }
}

/// Read `count - 1` dods and rebuild `count` timestamps starting at `first_ts`.
pub(crate) fn decode_timestamps(
    reader: &mut BitReader,
    first_ts: i64,
    count: usize,
) -> Result<Vec<i64>> {
    let mut timestamps = Vec::with_capacity(count);
    timestamps.push(first_ts);
    let mut prev_delta: i64 = 0;
    let mut prev_ts = first_ts;
    let mut remaining = count - 1;
    while remaining > 0 {
        match decode_dod(reader).context("truncated timestamp data")? {
            Dod::Value(dod) => {
                prev_delta = prev_delta.wrapping_add(dod);
                prev_ts = prev_ts.wrapping_add(prev_delta);
                timestamps.push(prev_ts);
                remaining -= 1;
            }
            Dod::ZeroRun(n) => {
                if n > remaining {
                    bail!("timestamp run of {n} overruns block ({remaining} left)");
                }
                for _ in 0..n {
                    prev_ts = prev_ts.wrapping_add(prev_delta);
                    timestamps.push(prev_ts);
                }
                remaining -= n;
            }
        }
    }
    Ok(timestamps)
}

/// Write the XOR stream for `values[1..]` (the first value lives in the header).
pub(crate) fn encode_xor_values(writer: &mut BitWriter, values: &[f64]) {
    // Active "meaningful bits" window; None until the first non-zero xor.
    let mut window: Option<(u8, u8)> = None;

    for pair in values.windows(2) {
        let xor = pair[1].to_bits() ^ pair[0].to_bits();
        if xor == 0 {
            writer.write_bit(false);
            continue;
        }
        writer.write_bit(true);

        let lz = xor.leading_zeros() as u8; // 0..=63, xor != 0
        let tz = xor.trailing_zeros() as u8;

        match window {
            Some((wlz, wtz)) if lz >= wlz && tz >= wtz => {
                // Fits inside the previous window: control bit 0 + window bits.
                writer.write_bit(false);
                writer.write_bits(xor >> wtz, 64 - wlz - wtz);
            }
            _ => {
                let meaningful = 64 - lz - tz; // 1..=64
                writer.write_bit(true);
                writer.write_bits(lz as u64, 6);
                writer.write_bits((meaningful - 1) as u64, 6);
                writer.write_bits(xor >> tz, meaningful);
                window = Some((lz, tz));
            }
        }
    }
}

/// Read `count - 1` XOR-coded values following `first_value`.
pub(crate) fn decode_xor_values(
    reader: &mut BitReader,
    first_value: f64,
    count: usize,
) -> Result<Vec<f64>> {
    let mut values = Vec::with_capacity(count);
    values.push(first_value);

    let mut prev_bits = first_value.to_bits();
    let mut window: Option<(u8, u8)> = None;

    for _ in 1..count {
        if !reader.read_bit().context("truncated value control bit")? {
            values.push(f64::from_bits(prev_bits));
            continue;
        }
        let new_window = reader.read_bit().context("truncated value window bit")?;
        let (lz, tz) = if new_window {
            let lz = reader.read_bits(6).context("truncated leading zeros")? as u8;
            let meaningful = reader.read_bits(6).context("truncated meaningful count")? as u8 + 1;
            if lz as u16 + meaningful as u16 > 64 {
                bail!("corrupt value window: lz {lz} + {meaningful} bits exceeds 64");
            }
            let w = (lz, 64 - lz - meaningful);
            window = Some(w);
            w
        } else {
            window.context("value reuses a window before one was defined")?
        };
        let meaningful = 64 - lz - tz;
        let bits = reader
            .read_bits(meaningful)
            .context("truncated meaningful value bits")?;
        prev_bits ^= bits << tz;
        values.push(f64::from_bits(prev_bits));
    }
    Ok(values)
}

enum Dod {
    Value(i64),
    ZeroRun(usize),
}

/// Encode a non-zero delta-of-delta value into the bit writer.
///
/// Prefix codes: `10`+7, `110`+9, `1110`+12, `11110`+20 bits (each offset so
/// the range is non-negative), `111110`+64 raw bits. `111111` is the run token
/// written by `encode_timestamps`.
fn encode_dod(writer: &mut BitWriter, dod: i64) {
    debug_assert!(dod != 0);
    writer.write_bit(true);
    if (-64..64).contains(&dod) {
        writer.write_bit(false);
        writer.write_bits((dod + 64) as u64, 7);
    } else if (-256..256).contains(&dod) {
        writer.write_bits(0b10, 2);
        writer.write_bits((dod + 256) as u64, 9);
    } else if (-2048..2048).contains(&dod) {
        writer.write_bits(0b110, 3);
        writer.write_bits((dod + 2048) as u64, 12);
    } else if (-(1 << 19)..(1 << 19)).contains(&dod) {
        writer.write_bits(0b1110, 4);
        writer.write_bits((dod + (1 << 19)) as u64, 20);
    } else {
        writer.write_bits(0b11110, 5);
        writer.write_bits(dod as u64, 64);
    }
}

fn decode_dod(reader: &mut BitReader) -> Option<Dod> {
    if !reader.read_bit()? {
        return Some(Dod::Value(0));
    }
    let mut ones = 1u8;
    while ones < 6 && reader.read_bit()? {
        ones += 1;
    }
    // After the leading 1: `ones` counts the 1 bits seen so far (the last
    // read bit was a 0 unless ones reached 6).
    Some(match ones {
        1 => Dod::Value(reader.read_bits(7)? as i64 - 64),
        2 => Dod::Value(reader.read_bits(9)? as i64 - 256),
        3 => Dod::Value(reader.read_bits(12)? as i64 - 2048),
        4 => Dod::Value(reader.read_bits(20)? as i64 - (1 << 19)),
        5 => Dod::Value(reader.read_bits(64)? as i64),
        _ => Dod::ZeroRun(reader.read_bits(16)? as usize + 1),
    })
}
