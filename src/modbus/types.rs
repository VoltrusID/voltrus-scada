use crate::models::*;

pub fn parse_register(words: &[u16], reg_type: &RegisterType, endianness: &Endianness) -> f64 {
    match reg_type {
        RegisterType::Uint16 => words[0] as f64,
        RegisterType::Int16 => (words[0] as i16) as f64,
        RegisterType::Int32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            raw as i32 as f64
        }
        RegisterType::Float32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            f32::from_bits(raw) as f64
        }
        RegisterType::Float64 => {
            let hi = combine_u16(words[0], words[1], endianness);
            let lo = combine_u16(words[2], words[3], endianness);
            f64::from_bits(((hi as u64) << 32) | (lo as u64))
        }
    }
}

pub(crate) fn combine_u16(high: u16, low: u16, endianness: &Endianness) -> u32 {
    match endianness {
        Endianness::Big => ((high as u32) << 16) | (low as u32),
        Endianness::Little => ((low as u32) << 16) | (high as u32),
        Endianness::Swapped => {
            let h = high.swap_bytes();
            let l = low.swap_bytes();
            ((l as u32) << 16) | (h as u32)
        }
    }
}

pub(crate) fn value_to_words(
    value: f64,
    reg_type: &RegisterType,
    endianness: &Endianness,
) -> Vec<u16> {
    match reg_type {
        RegisterType::Uint16 => vec![value as u16],
        RegisterType::Int16 => vec![(value as i16) as u16],
        RegisterType::Int32 => {
            let raw = (value as i32) as u32;
            split_u32(raw, endianness)
        }
        RegisterType::Float32 => {
            let raw = (value as f32).to_bits();
            split_u32(raw, endianness)
        }
        RegisterType::Float64 => {
            let raw = value.to_bits();
            let hi = split_u32((raw >> 32) as u32, endianness);
            let lo = split_u32((raw & 0xFFFFFFFF) as u32, endianness);
            [hi, lo].concat()
        }
    }
}

fn split_u32(raw: u32, endianness: &Endianness) -> Vec<u16> {
    match endianness {
        Endianness::Big => vec![(raw >> 16) as u16, raw as u16],
        Endianness::Little => vec![raw as u16, (raw >> 16) as u16],
        Endianness::Swapped => {
            let h = (raw >> 16) as u16;
            let l = raw as u16;
            vec![h.swap_bytes(), l.swap_bytes()]
        }
    }
}

pub(crate) struct Backoff {
    pub(crate) attempt: u32,
    pub(crate) max_delay_ms: u64,
}

impl Backoff {
    pub(crate) fn new() -> Self {
        Self {
            attempt: 0,
            max_delay_ms: 30_000,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.attempt = 0;
    }

    pub(crate) fn next(&mut self) -> u64 {
        let attempt = self.attempt.min(30);
        let delay = 1000 * 2u64.pow(attempt);
        self.attempt = self.attempt.saturating_add(1);
        delay.min(self.max_delay_ms)
    }
}
