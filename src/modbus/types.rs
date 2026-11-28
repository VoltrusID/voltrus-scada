use crate::models::*;

/// The value of a register from its words (as many as `reg_type.width()`).
///
/// Values are `f64` from here on (cache, history, alarms), so a 64-bit
/// integer is exact only up to 2^53 (9 007 199 254 740 992) in magnitude and
/// rounded to the nearest representable value beyond. An energy counter in Wh
/// gets there after about a million years at 1 GW.
pub fn parse_register(words: &[u16], reg_type: &RegisterType, endianness: &Endianness) -> f64 {
    match reg_type {
        RegisterType::Uint16 => words[0] as f64,
        RegisterType::Int16 => (words[0] as i16) as f64,
        RegisterType::Int32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            raw as i32 as f64
        }
        RegisterType::Uint32 => combine_u16(words[0], words[1], endianness) as f64,
        RegisterType::Int64 => combine_u64(words, endianness) as i64 as f64,
        RegisterType::Uint64 => combine_u64(words, endianness) as f64,
        RegisterType::Float32 => {
            let raw = combine_u16(words[0], words[1], endianness);
            f32::from_bits(raw) as f64
        }
        RegisterType::Float64 => f64::from_bits(combine_u64(words, endianness)),
    }
}

/// Four registers as one 64-bit value, extending the 32-bit orders: `big`
/// is most significant word first (ABCDEFGH), `little` least significant
/// word first with big-endian bytes in each word (GHEFCDAB), `swapped`
/// every byte reversed (HGFEDCBA). The `_dword` orders keep the high dword
/// first and order each dword as `little`/`swapped` (CDABGHEF, DCBAHGFE).
pub(crate) fn combine_u64(words: &[u16], endianness: &Endianness) -> u64 {
    let first = combine_u16(words[0], words[1], endianness) as u64;
    let second = combine_u16(words[2], words[3], endianness) as u64;
    if low_dword_first(endianness) {
        (second << 32) | first
    } else {
        (first << 32) | second
    }
}

fn low_dword_first(endianness: &Endianness) -> bool {
    matches!(endianness, Endianness::Little | Endianness::Swapped)
}

/// The inverse of `combine_u64`.
fn split_u64(raw: u64, endianness: &Endianness) -> Vec<u16> {
    let hi = split_u32((raw >> 32) as u32, endianness);
    let lo = split_u32(raw as u32, endianness);
    if low_dword_first(endianness) {
        [lo, hi].concat()
    } else {
        [hi, lo].concat()
    }
}

pub(crate) fn combine_u16(high: u16, low: u16, endianness: &Endianness) -> u32 {
    match endianness {
        Endianness::Big => ((high as u32) << 16) | (low as u32),
        Endianness::Little | Endianness::LittleDword => ((low as u32) << 16) | (high as u32),
        Endianness::Swapped | Endianness::SwappedDword => {
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
        RegisterType::Uint32 => split_u32(value as u32, endianness),
        RegisterType::Int64 => split_u64(value as i64 as u64, endianness),
        RegisterType::Uint64 => split_u64(value as u64, endianness),
        RegisterType::Float32 => {
            let raw = (value as f32).to_bits();
            split_u32(raw, endianness)
        }
        RegisterType::Float64 => split_u64(value.to_bits(), endianness),
    }
}

/// The words to write for an engineering `value` on `register`: the inverse of
/// a read (`raw * scale`), in the register's type and the device's word order.
/// Integer types round to the nearest step and refuse a value they cannot hold,
/// so a setpoint never wraps or saturates on its way to the device.
pub(crate) fn encode_register(
    value: f64,
    register: &RegisterConfig,
    endianness: &Endianness,
) -> Result<Vec<u16>, String> {
    if !value.is_finite() {
        return Err(format!("Value {value} is not a finite number"));
    }
    if register.scale == 0.0 || !register.scale.is_finite() {
        return Err(format!(
            "Register '{}' has scale {}, so no raw value reads back as {value}",
            register.name, register.scale
        ));
    }
    let raw = value / register.scale;
    let integer = |min: f64, max: f64, type_name: &str| -> Result<f64, String> {
        let rounded = raw.round();
        if rounded < min || rounded > max {
            return Err(format!(
                "Value {value} is raw {raw} on register '{}', outside the {type_name} range {min}..={max}",
                register.name
            ));
        }
        Ok(rounded)
    };
    // `max` is exclusive here: it is the first value past the type's range.
    let integer64 = |min: f64, max: f64, type_name: &str| -> Result<f64, String> {
        let rounded = raw.round();
        if rounded < min || rounded >= max {
            return Err(format!(
                "Value {value} is raw {raw} on register '{}', outside the {type_name} range",
                register.name
            ));
        }
        Ok(rounded)
    };
    Ok(match register.reg_type {
        RegisterType::Uint16 => vec![integer(0.0, u16::MAX as f64, "uint16")? as u16],
        RegisterType::Int16 => {
            vec![integer(i16::MIN as f64, i16::MAX as f64, "int16")? as i16 as u16]
        }
        RegisterType::Int32 => {
            let raw = integer(i32::MIN as f64, i32::MAX as f64, "int32")? as i32;
            split_u32(raw as u32, endianness)
        }
        RegisterType::Uint32 => {
            split_u32(integer(0.0, u32::MAX as f64, "uint32")? as u32, endianness)
        }
        // The f64 bounds are the nearest values to the integer limits: i64::MAX
        // and u64::MAX round up to 2^63 and 2^64, which do not fit, so the upper
        // check is exclusive (an f64 just below them is in range).
        RegisterType::Int64 => {
            let raw = integer64(-(2f64.powi(63)), 2f64.powi(63), "int64")?;
            split_u64(raw as i64 as u64, endianness)
        }
        RegisterType::Uint64 => {
            let raw = integer64(0.0, 2f64.powi(64), "uint64")?;
            split_u64(raw as u64, endianness)
        }
        RegisterType::Float32 => {
            if raw.abs() > f32::MAX as f64 {
                return Err(format!(
                    "Value {value} is raw {raw} on register '{}', outside the float32 range",
                    register.name
                ));
            }
            split_u32((raw as f32).to_bits(), endianness)
        }
        RegisterType::Float64 => value_to_words(raw, &RegisterType::Float64, endianness),
    })
}

fn split_u32(raw: u32, endianness: &Endianness) -> Vec<u16> {
    match endianness {
        Endianness::Big => vec![(raw >> 16) as u16, raw as u16],
        Endianness::Little | Endianness::LittleDword => vec![raw as u16, (raw >> 16) as u16],
        // The inverse of `combine_u16`: low word first, each byte-swapped.
        Endianness::Swapped | Endianness::SwappedDword => {
            let h = (raw >> 16) as u16;
            let l = raw as u16;
            vec![l.swap_bytes(), h.swap_bytes()]
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
