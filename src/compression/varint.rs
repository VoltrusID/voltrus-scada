#![allow(dead_code)]
// src/compression/varint.rs — Varint encoding (zigzag + LEB128) for warm tier deltas

use anyhow::Result;

fn zigzag_encode(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

fn zigzag_decode(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

pub fn varint_encode(values: &[i64]) -> Vec<u8> {
    let mut buf = Vec::new();
    for &v in values {
        let mut n = zigzag_encode(v);
        loop {
            let byte = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                buf.push(byte);
                break;
            }
            buf.push(byte | 0x80);
        }
    }
    buf
}

pub fn varint_decode(data: &[u8]) -> Result<Vec<i64>> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let mut n: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if i >= data.len() {
                anyhow::bail!("Truncated varint");
            }
            let byte = data[i];
            i += 1;
            n |= ((byte & 0x7F) as u64) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        values.push(zigzag_decode(n));
    }
    Ok(values)
}
