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
