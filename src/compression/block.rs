// src/compression/block.rs — Self-describing packed block of one tag's samples
//
// The historian stores `(timestamp, value)` samples of one tag in these blocks.
// Layout:
//
//   byte 0        format version (high nibble) | value codec (low nibble)
//   varint        sample count
//   zigzag varint first timestamp
//   8 bytes       first value (f64 bits, little endian)
//   bit stream    delta-of-delta timestamps, then the value payload
//
// Value codecs, chosen per block:
//
//   0 XOR       Gorilla XOR coding, for analog signals
//   1 DIGITAL   run lengths for a signal that only takes 0.0 and 1.0; runs
//               alternate starting from the first value (run-length encoding)
//   2 CONSTANT  every sample has the same bits, no value payload at all

use super::bitio::{BitReader, BitWriter};
use super::gorilla::{
    MAX_BLOCK_SAMPLES, decode_timestamps, decode_xor_values, encode_timestamps, encode_xor_values,
};
use anyhow::{Context, Result, bail};

const FORMAT_VERSION: u8 = 1;
const CODEC_XOR: u8 = 0;
const CODEC_DIGITAL: u8 = 1;
const CODEC_CONSTANT: u8 = 2;

const ZERO_BITS: u64 = 0;
const ONE_BITS: u64 = 0x3FF0_0000_0000_0000;

/// Pack samples into one block. The samples keep the order given (callers
/// pass them sorted by timestamp); duplicates and any `f64` bit pattern are
/// stored exactly.
pub fn encode_series(samples: &[(i64, f64)]) -> Result<Vec<u8>> {
    if samples.is_empty() {
        bail!("cannot encode an empty block");
    }
    if samples.len() > MAX_BLOCK_SAMPLES {
        bail!(
            "block of {} samples exceeds the {MAX_BLOCK_SAMPLES} limit",
            samples.len()
        );
    }
    let first_bits = samples[0].1.to_bits();
    let codec = if samples.iter().all(|s| s.1.to_bits() == first_bits) {
        CODEC_CONSTANT
    } else if samples
        .iter()
        .all(|s| matches!(s.1.to_bits(), ZERO_BITS | ONE_BITS))
    {
        CODEC_DIGITAL
    } else {
        CODEC_XOR
    };

    let mut out = Vec::with_capacity(16 + samples.len());
    out.push(FORMAT_VERSION << 4 | codec);
    write_uvarint(&mut out, samples.len() as u64);
    write_uvarint(&mut out, zigzag(samples[0].0));
    out.extend_from_slice(&first_bits.to_le_bytes());

    if samples.len() > 1 {
        let timestamps: Vec<i64> = samples.iter().map(|s| s.0).collect();
        let mut w = BitWriter::new();
        encode_timestamps(&mut w, &timestamps);
        match codec {
            CODEC_XOR => {
                let values: Vec<f64> = samples.iter().map(|s| s.1).collect();
                encode_xor_values(&mut w, &values);
            }
            CODEC_DIGITAL => encode_runs(&mut w, samples),
            _ => {}
        }
        out.extend_from_slice(&w.finish());
    }
    Ok(out)
}

/// Unpack a block written by [`encode_series`].
pub fn decode_series(data: &[u8]) -> Result<Vec<(i64, f64)>> {
    let header = Header::parse(data)?;
    let Header {
        codec,
        count,
        first_ts,
        first_value,
        payload,
    } = header;
    if count == 1 {
        return Ok(vec![(first_ts, first_value)]);
    }

    let mut r = BitReader::new(payload);
    let timestamps = decode_timestamps(&mut r, first_ts, count)?;
    let values = match codec {
        CODEC_XOR => decode_xor_values(&mut r, first_value, count)?,
        CODEC_DIGITAL => decode_runs(&mut r, first_value, count)?,
        CODEC_CONSTANT => vec![first_value; count],
        other => bail!("unknown value codec {other}"),
    };
    Ok(timestamps.into_iter().zip(values).collect())
}

/// Sample count of a block without decoding it.
pub fn series_len(data: &[u8]) -> Result<usize> {
    Ok(Header::parse(data)?.count)
}

struct Header<'a> {
    codec: u8,
    count: usize,
    first_ts: i64,
    first_value: f64,
    payload: &'a [u8],
}

impl<'a> Header<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        let (&b0, mut rest) = data.split_first().context("empty block")?;
        if b0 >> 4 != FORMAT_VERSION {
            bail!("unsupported block format version {}", b0 >> 4);
        }
        let count = read_uvarint(&mut rest).context("truncated sample count")?;
        if count == 0 || count > MAX_BLOCK_SAMPLES as u64 {
            bail!("invalid block sample count {count}");
        }
        let first_ts = unzigzag(read_uvarint(&mut rest).context("truncated first timestamp")?);
        let (first_bytes, payload) = rest
            .split_first_chunk::<8>()
            .context("truncated first value")?;
        Ok(Self {
            codec: b0 & 0x0F,
            count: count as usize,
            first_ts,
            first_value: f64::from_bits(u64::from_le_bytes(*first_bytes)),
            payload,
        })
    }
}

/// Run lengths of a 0.0/1.0 signal, each stored as `len - 1` in LEB128 bytes.
fn encode_runs(w: &mut BitWriter, samples: &[(i64, f64)]) {
    let mut run = 1u64;
    let mut runs: Vec<u64> = Vec::new();
    for pair in samples.windows(2) {
        if pair[1].1.to_bits() == pair[0].1.to_bits() {
            run += 1;
        } else {
            runs.push(run);
            run = 1;
        }
    }
    runs.push(run);
    for len in runs {
        let mut buf = Vec::with_capacity(4);
        write_uvarint(&mut buf, len - 1);
        for b in buf {
            w.write_bits(b as u64, 8);
        }
    }
}

fn decode_runs(r: &mut BitReader, first_value: f64, count: usize) -> Result<Vec<f64>> {
    let mut values = Vec::with_capacity(count);
    let mut current = first_value.to_bits();
    while values.len() < count {
        let mut n = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = r.read_bits(8).context("truncated run length")?;
            if shift >= 63 {
                bail!("run length varint too long");
            }
            n |= (byte & 0x7F) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        let len = n.saturating_add(1);
        if len > (count - values.len()) as u64 {
            bail!("run of {len} overruns block");
        }
        values.extend(std::iter::repeat_n(f64::from_bits(current), len as usize));
        current = if current == ZERO_BITS {
            ONE_BITS
        } else {
            ZERO_BITS
        };
    }
    Ok(values)
}

fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn unzigzag(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

fn write_uvarint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_uvarint(data: &mut &[u8]) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = data.split_first()?;
        *data = rest;
        if shift == 63 && byte > 1 {
            return None;
        }
        n |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}
