#![allow(dead_code)]
// src/compression/bitio.rs — Bit-level I/O for compressed data

/// Bit-level writer for encoding compressed data.
pub struct BitWriter {
    buffer: Vec<u8>,
    current_byte: u8,
    bit_pos: u8, // 0..7, number of bits written in current_byte
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            current_byte: 0,
            bit_pos: 0,
        }
    }

    /// Write `n` bits from `value` (MSB first).
    pub fn write_bits(&mut self, value: u64, mut n: u8) {
        while n > 0 {
            let remaining = 8 - self.bit_pos;
            let bits_to_write = remaining.min(n);
            let shift = n - bits_to_write;
            let bits = ((value >> shift) & ((1u64 << bits_to_write) - 1)) as u8;
            self.current_byte |= bits << (remaining - bits_to_write);
            self.bit_pos += bits_to_write;
            n -= bits_to_write;
            if self.bit_pos == 8 {
                self.buffer.push(self.current_byte);
                self.current_byte = 0;
                self.bit_pos = 0;
            }
        }
    }

    /// Write a single bit.
    pub fn write_bit(&mut self, bit: bool) {
        self.write_bits(if bit { 1 } else { 0 }, 1);
    }

    /// Finalize and return the written bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if self.bit_pos > 0 {
            self.buffer.push(self.current_byte);
        }
        self.buffer
    }
}

/// Bit-level reader for decoding compressed data.
pub struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8, // 0..7, next bit to read in current byte
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    /// Read `n` bits and return as u64 (MSB first).
    pub fn read_bits(&mut self, mut n: u8) -> Option<u64> {
        let mut result: u64 = 0;
        while n > 0 {
            if self.byte_pos >= self.data.len() {
                return None;
            }
            let remaining = 8 - self.bit_pos;
            let bits_to_read = remaining.min(n);
            let shift = remaining - bits_to_read;
            let mask = (((1u32 << bits_to_read) - 1) << shift) as u8;
            let bits = (self.data[self.byte_pos] & mask) >> shift;
            result = (result << bits_to_read) | (bits as u64);
            self.bit_pos += bits_to_read;
            n -= bits_to_read;
            if self.bit_pos == 8 {
                self.byte_pos += 1;
                self.bit_pos = 0;
            }
        }
        Some(result)
    }

    /// Read a single bit.
    pub fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|v| v == 1)
    }

    /// Check if there are more bits to read.
    pub fn has_remaining(&self) -> bool {
        self.byte_pos < self.data.len()
    }
}
