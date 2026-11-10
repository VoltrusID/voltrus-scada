use crate::models::*;

use super::types::{Backoff, combine_u16, parse_register, value_to_words};

#[test]
fn test_parse_uint16() {
    assert_eq!(
        parse_register(&[1000u16], &RegisterType::Uint16, &Endianness::Big),
        1000.0
    );
}

#[test]
fn test_parse_int16_negative() {
    assert_eq!(
        parse_register(&[0xFFFFu16], &RegisterType::Int16, &Endianness::Big),
        -1.0
    );
}

#[test]
fn test_parse_float32_big_endian() {
    let val = parse_register(&[0x435C, 0x8000], &RegisterType::Float32, &Endianness::Big);
    assert!((val - 220.5).abs() < 0.01);
}

#[test]
fn test_parse_float32_little_endian() {
    let val = parse_register(
        &[0x8000, 0x435C],
        &RegisterType::Float32,
        &Endianness::Little,
    );
    assert!((val - 220.5).abs() < 0.01);
}

#[test]
fn test_combine_big_endian() {
    assert_eq!(combine_u16(0x1234, 0x5678, &Endianness::Big), 0x12345678);
}

#[test]
fn test_combine_little_endian() {
    assert_eq!(combine_u16(0x5678, 0x1234, &Endianness::Little), 0x12345678);
}

#[test]
fn test_backoff_sequence() {
    let mut b = Backoff::new();
    assert_eq!(b.next(), 1000);
    assert_eq!(b.next(), 2000);
    assert_eq!(b.next(), 4000);
}

#[test]
fn test_backoff_resets() {
    let mut b = Backoff::new();
    b.next();
    b.next();
    b.reset();
    assert_eq!(b.next(), 1000);
}

#[test]
fn test_backoff_max() {
    let mut b = Backoff::new();
    for _ in 0..20 {
        b.next();
    }
    assert_eq!(b.next(), 30_000);
}

#[test]
fn test_register_count_float64() {
    let reg = RegisterConfig {
        name: "power".into(),
        address: 0,
        reg_type: RegisterType::Float64,
        scale: 1.0,
        register_area: "holding".into(),
        zero_based: false,
    };
    assert_eq!(reg.register_count(), 4);
}

#[test]
fn test_value_to_words_uint16() {
    let words = value_to_words(42.0, &RegisterType::Uint16, &Endianness::Big);
    assert_eq!(words, vec![42]);
}

#[test]
fn test_value_to_words_float32() {
    let words = value_to_words(220.5, &RegisterType::Float32, &Endianness::Big);
    assert_eq!(words, vec![0x435C, 0x8000]);
}

#[test]
fn test_value_to_words_int32() {
    let words = value_to_words(1000.0, &RegisterType::Int32, &Endianness::Big);
    assert_eq!(words, vec![0x0000, 0x03E8]);
}

#[test]
fn test_value_to_words_float64() {
    let words = value_to_words(220.5, &RegisterType::Float64, &Endianness::Big);
    assert_eq!(words, vec![0x406B, 0x9000, 0x0000, 0x0000]);
}

// ── Write encoding: the inverse of a read ──

fn write_reg(reg_type: RegisterType, scale: f64) -> RegisterConfig {
    RegisterConfig {
        name: "sp".into(),
        address: 40001,
        reg_type,
        scale,
        register_area: "holding".into(),
        zero_based: false,
    }
}

/// Encoding then reading back gives the engineering value again.
fn round_trip(value: f64, reg: &RegisterConfig, endianness: &Endianness) -> f64 {
    let words = super::types::encode_register(value, reg, endianness).unwrap();
    assert_eq!(words.len(), reg.register_count() as usize);
    parse_register(&words, &reg.reg_type, endianness) * reg.scale
}

#[test]
fn test_encode_uint16_with_scale() {
    let reg = write_reg(RegisterType::Uint16, 0.1);
    // 12.5 / 0.1 is 124.99999999999999 in f64; it must round, not truncate.
    assert_eq!(
        super::types::encode_register(12.5, &reg, &Endianness::Big).unwrap(),
        vec![125]
    );
    assert!(super::types::encode_register(-1.0, &reg, &Endianness::Big).is_err());
    assert!(super::types::encode_register(6553.6, &reg, &Endianness::Big).is_err());
}

#[test]
fn test_encode_int16_negative() {
    let reg = write_reg(RegisterType::Int16, 1.0);
    assert_eq!(
        super::types::encode_register(-2.0, &reg, &Endianness::Big).unwrap(),
        vec![0xFFFE]
    );
    assert!(super::types::encode_register(40000.0, &reg, &Endianness::Big).is_err());
}

#[test]
fn test_encode_int32_word_orders() {
    let reg = write_reg(RegisterType::Int32, 1.0);
    let enc = |e: &Endianness| super::types::encode_register(-100_000.0, &reg, e).unwrap();
    // -100000 = 0xFFFE7960
    assert_eq!(enc(&Endianness::Big), vec![0xFFFE, 0x7960]);
    assert_eq!(enc(&Endianness::Little), vec![0x7960, 0xFFFE]);
    assert_eq!(enc(&Endianness::Swapped), vec![0x6079, 0xFEFF]);
    for e in [Endianness::Big, Endianness::Little, Endianness::Swapped] {
        assert_eq!(round_trip(-100_000.0, &reg, &e), -100_000.0);
    }
}

#[test]
fn test_encode_float32_scaled_word_orders() {
    let reg = write_reg(RegisterType::Float32, 0.5);
    // 441 / 0.5 = 882.0 = 0x445C8000
    let enc = |e: &Endianness| super::types::encode_register(441.0, &reg, e).unwrap();
    assert_eq!(enc(&Endianness::Big), vec![0x445C, 0x8000]);
    assert_eq!(enc(&Endianness::Little), vec![0x8000, 0x445C]);
    assert_eq!(enc(&Endianness::Swapped), vec![0x0080, 0x5C44]);
    for e in [Endianness::Big, Endianness::Little, Endianness::Swapped] {
        assert_eq!(round_trip(441.0, &reg, &e), 441.0);
    }
}

#[test]
fn test_encode_float64_round_trip() {
    let reg = write_reg(RegisterType::Float64, 2.0);
    for e in [Endianness::Big, Endianness::Little, Endianness::Swapped] {
        assert_eq!(round_trip(1234.5678, &reg, &e), 1234.5678);
    }
}

#[test]
fn test_encode_refuses_bad_scale_and_values() {
    let zero = write_reg(RegisterType::Float32, 0.0);
    assert!(super::types::encode_register(1.0, &zero, &Endianness::Big).is_err());
    let reg = write_reg(RegisterType::Float32, 1.0);
    assert!(super::types::encode_register(f64::NAN, &reg, &Endianness::Big).is_err());
    assert!(super::types::encode_register(1e300, &reg, &Endianness::Big).is_err());
}
