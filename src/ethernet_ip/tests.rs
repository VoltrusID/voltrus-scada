use super::*;

// ── Packet construction tests ──

#[test]
fn test_build_enip_header_register_session() {
    let body = [0u8; 4]; // protocol version + options
    let packet = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &body);
    assert_eq!(packet.len(), ENIP_HEADER_SIZE + 4);
    // Command
    assert_eq!(
        u16::from_le_bytes([packet[0], packet[1]]),
        CMD_REGISTER_SESSION
    );
    // Length
    assert_eq!(u16::from_le_bytes([packet[2], packet[3]]), 4);
    // Session handle
    assert_eq!(
        u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]),
        0
    );
    // Status
    assert_eq!(
        u32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]),
        0
    );
}

#[test]
fn test_build_enip_header_with_session() {
    let body = vec![1, 2, 3, 4];
    let packet = build_enip_header(CMD_SEND_RR_DATA, 0x12345678, 0, &body);
    assert_eq!(packet.len(), ENIP_HEADER_SIZE + 4);
    assert_eq!(u16::from_le_bytes([packet[0], packet[1]]), CMD_SEND_RR_DATA);
    assert_eq!(
        u32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]),
        0x12345678
    );
}

#[test]
fn test_build_read_tag_request() {
    let request = build_read_tag_request("MyTag");
    assert_eq!(request[0], CIP_READ_TAG);
    // Path size should be > 0
    assert!(request[1] > 0);
    // Number of elements = 1
    let len = request.len();
    assert_eq!(u16::from_le_bytes([request[len - 2], request[len - 1]]), 1);
}

#[test]
fn test_build_symbolic_path_short() {
    let path = build_symbolic_path("Tag1");
    assert_eq!(path[0], 0x91); // extended symbolic segment
    assert_eq!(path[1], 4); // length
    assert_eq!(&path[2..6], b"Tag1");
}

#[test]
fn test_build_symbolic_path_padded() {
    let path = build_symbolic_path("Tag"); // 3 bytes -> needs padding
    assert_eq!(path[0], 0x91);
    assert_eq!(path[1], 3);
    assert_eq!(&path[2..5], b"Tag");
    assert_eq!(path[5], 0x00); // padding
}

#[test]
fn test_build_symbolic_path_long() {
    let path = build_symbolic_path("VeryLongTagName123");
    assert_eq!(path[0], 0x91);
    assert_eq!(path[1], 18);
}

// ── CIP data type tests ──

#[test]
fn test_cip_data_type_from_code() {
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_BOOL),
        Some(CipDataType::Bool)
    );
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_SINT),
        Some(CipDataType::Sint)
    );
    assert_eq!(CipDataType::from_code(CIP_TYPE_INT), Some(CipDataType::Int));
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_DINT),
        Some(CipDataType::Dint)
    );
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_REAL),
        Some(CipDataType::Real)
    );
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_STRING),
        Some(CipDataType::String)
    );
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_DWORD),
        Some(CipDataType::Dword)
    );
    assert_eq!(
        CipDataType::from_code(CIP_TYPE_LINT),
        Some(CipDataType::Lint)
    );
    assert_eq!(CipDataType::from_code(0xFF), None);
}

#[test]
fn test_cip_data_type_code_roundtrip() {
    for dt in &[
        CipDataType::Bool,
        CipDataType::Sint,
        CipDataType::Int,
        CipDataType::Dint,
        CipDataType::Real,
        CipDataType::String,
        CipDataType::Dword,
        CipDataType::Lint,
    ] {
        assert_eq!(CipDataType::from_code(dt.code()), Some(dt.clone()));
    }
}

#[test]
fn test_cip_data_type_element_sizes() {
    assert_eq!(CipDataType::Bool.element_size(), 1);
    assert_eq!(CipDataType::Sint.element_size(), 1);
    assert_eq!(CipDataType::Int.element_size(), 2);
    assert_eq!(CipDataType::Dint.element_size(), 4);
    assert_eq!(CipDataType::Real.element_size(), 4);
    assert_eq!(CipDataType::Lint.element_size(), 8);
}

#[test]
fn test_cip_data_type_as_str() {
    assert_eq!(CipDataType::Bool.as_str(), "BOOL");
    assert_eq!(CipDataType::Sint.as_str(), "SINT");
    assert_eq!(CipDataType::Int.as_str(), "INT");
    assert_eq!(CipDataType::Dint.as_str(), "DINT");
    assert_eq!(CipDataType::Real.as_str(), "REAL");
    assert_eq!(CipDataType::String.as_str(), "STRING");
    assert_eq!(CipDataType::Dword.as_str(), "DWORD");
    assert_eq!(CipDataType::Lint.as_str(), "LINT");
}

// ── CIP value encoding/decoding tests ──

#[test]
fn test_encode_cip_value_bool() {
    let (code, data) = encode_cip_value(&CipValue::Bool(true));
    assert_eq!(code, CIP_TYPE_BOOL);
    assert_eq!(data[0], 0xFF);

    let (code, data) = encode_cip_value(&CipValue::Bool(false));
    assert_eq!(code, CIP_TYPE_BOOL);
    assert_eq!(data[0], 0x00);
}

#[test]
fn test_encode_cip_value_int() {
    let (code, data) = encode_cip_value(&CipValue::Int(-1234));
    assert_eq!(code, CIP_TYPE_INT);
    assert_eq!(i16::from_le_bytes([data[0], data[1]]), -1234);
}

#[test]
fn test_encode_cip_value_dint() {
    let (code, data) = encode_cip_value(&CipValue::Dint(42));
    assert_eq!(code, CIP_TYPE_DINT);
    assert_eq!(i32::from_le_bytes([data[0], data[1], data[2], data[3]]), 42);
}

#[test]
fn test_encode_cip_value_real() {
    let (code, data) = encode_cip_value(&CipValue::Real(3.14));
    assert_eq!(code, CIP_TYPE_REAL);
    let f = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    assert!((f - 3.14).abs() < 0.001);
}

#[test]
fn test_encode_cip_value_string() {
    let (code, data) = encode_cip_value(&CipValue::String("Hello".to_string()));
    assert_eq!(code, CIP_TYPE_STRING);
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    assert_eq!(len, 5);
    assert_eq!(&data[4..9], b"Hello");
}

#[test]
fn test_decode_cip_value_bool() {
    let val = decode_cip_value(CIP_TYPE_BOOL, &[0xFF, 0, 0, 0]).unwrap();
    assert_eq!(val, CipValue::Bool(true));

    let val = decode_cip_value(CIP_TYPE_BOOL, &[0x00, 0, 0, 0]).unwrap();
    assert_eq!(val, CipValue::Bool(false));
}

#[test]
fn test_decode_cip_value_int() {
    let data = (-1234i16).to_le_bytes();
    let val = decode_cip_value(CIP_TYPE_INT, &data).unwrap();
    assert_eq!(val, CipValue::Int(-1234));
}

#[test]
fn test_decode_cip_value_dint() {
    let data = 42i32.to_le_bytes();
    let val = decode_cip_value(CIP_TYPE_DINT, &data).unwrap();
    assert_eq!(val, CipValue::Dint(42));
}

#[test]
fn test_decode_cip_value_real() {
    let data = 3.14f32.to_le_bytes();
    let val = decode_cip_value(CIP_TYPE_REAL, &data).unwrap();
    match val {
        CipValue::Real(f) => assert!((f - 3.14).abs() < 0.001),
        _ => panic!("Expected Real"),
    }
}

#[test]
fn test_decode_cip_value_string() {
    let mut data = Vec::new();
    data.extend_from_slice(&5u32.to_le_bytes());
    data.extend_from_slice(b"Hello");
    let val = decode_cip_value(CIP_TYPE_STRING, &data).unwrap();
    assert_eq!(val, CipValue::String("Hello".to_string()));
}

#[test]
fn test_decode_cip_value_unknown_type() {
    let data = vec![0xAA, 0xBB];
    let val = decode_cip_value(0xFE, &data).unwrap();
    assert!(matches!(val, CipValue::Bytes(_)));
}

// ── CipValue conversion tests ──

#[test]
fn test_cip_value_to_f64() {
    assert_eq!(CipValue::Bool(true).to_f64(), Some(1.0));
    assert_eq!(CipValue::Bool(false).to_f64(), Some(0.0));
    assert_eq!(CipValue::Sint(-5).to_f64(), Some(-5.0));
    assert_eq!(CipValue::Int(-100).to_f64(), Some(-100.0));
    assert_eq!(CipValue::Dint(42).to_f64(), Some(42.0));
    assert!((CipValue::Real(3.14).to_f64().unwrap() - 3.14).abs() < 0.001);
    assert_eq!(CipValue::String("hi".into()).to_f64(), None);
    assert_eq!(CipValue::Bytes(vec![]).to_f64(), None);
}

#[test]
fn test_cip_value_display() {
    assert_eq!(CipValue::Bool(true).display_value(), "true");
    assert_eq!(CipValue::Sint(-3).display_value(), "-3");
    assert_eq!(CipValue::Int(-100).display_value(), "-100");
    assert_eq!(CipValue::Dint(42).display_value(), "42");
    assert_eq!(CipValue::Real(3.14).display_value(), "3.1400");
    assert_eq!(CipValue::String("hello".into()).display_value(), "hello");
}

// ── Tag name validation tests ──

#[test]
fn test_validate_tag_name_valid() {
    assert!(validate_tag_name("MyTag").is_ok());
    assert!(validate_tag_name("tag1").is_ok());
    assert!(validate_tag_name("_private").is_ok());
    assert!(validate_tag_name("Tag.Structure").is_ok());
    assert!(validate_tag_name("Array[0]").is_ok());
    assert!(validate_tag_name("a").is_ok());
}

#[test]
fn test_validate_tag_name_invalid() {
    assert!(validate_tag_name("").is_err());
    assert!(validate_tag_name("123tag").is_err()); // starts with digit
    assert!(validate_tag_name("tag name").is_err()); // contains space
    assert!(validate_tag_name("tag-name").is_err()); // contains hyphen
}

#[test]
fn test_validate_tag_name_too_long() {
    let long_name = "A".repeat(256);
    assert!(validate_tag_name(&long_name).is_err());
}

// ── Backoff tests ──

#[test]
fn test_backoff_sequence() {
    let mut b = Backoff::new();
    assert_eq!(b.next(), 1000);
    assert_eq!(b.next(), 2000);
    assert_eq!(b.next(), 4000);
    assert_eq!(b.next(), 8000);
    assert_eq!(b.next(), 16000);
    assert_eq!(b.next(), 32000);
    assert_eq!(b.next(), 60000); // capped at 60000
}

#[test]
fn test_backoff_reset() {
    let mut b = Backoff::new();
    b.next();
    b.next();
    b.reset();
    assert_eq!(b.next(), 1000);
}

// ── Parse read tag response tests ──

#[test]
fn test_parse_read_tag_response_dint() {
    // Simulate a successful DINT read response
    let mut data = vec![CIP_READ_TAG | 0x80, 0x00, CIP_STATUS_SUCCESS, 0x00];
    data.extend_from_slice(&(CIP_TYPE_DINT as u16).to_le_bytes()); // type (u16)
    data.extend_from_slice(&42i32.to_le_bytes()); // value
    let val = parse_read_tag_response(&data).unwrap();
    assert_eq!(val, CipValue::Dint(42));
}

#[test]
fn test_parse_read_tag_response_error() {
    let data = vec![CIP_READ_TAG | 0x80, 0x00, 0x04, 0x00]; // error status
    let result = parse_read_tag_response(&data);
    assert!(result.is_err());
}

#[test]
fn test_parse_read_tag_response_wrong_service() {
    let data = vec![0x99, 0x00, CIP_STATUS_SUCCESS, 0x00];
    let result = parse_read_tag_response(&data);
    assert!(result.is_err());
}

// ── Parse write tag response tests ──

#[test]
fn test_parse_write_tag_response_success() {
    let data = vec![CIP_WRITE_TAG | 0x80, 0x00, CIP_STATUS_SUCCESS, 0x00];
    assert!(parse_write_tag_response(&data).is_ok());
}

#[test]
fn test_parse_write_tag_response_error() {
    let data = vec![CIP_WRITE_TAG | 0x80, 0x00, 0x04, 0x00];
    assert!(parse_write_tag_response(&data).is_err());
}

// ── Parse symbol instance tests ──

#[test]
fn test_parse_symbol_instance_valid() {
    let mut data = Vec::new();
    data.extend_from_slice(&6u32.to_le_bytes()); // name len
    data.extend_from_slice(b"MyTag1"); // name (6 bytes, even -> no padding)
    data.extend_from_slice(&((1u16 << 8) | CIP_TYPE_DINT as u16).to_le_bytes()); // type: 1 element, DINT

    let tag = parse_symbol_instance(&data).unwrap();
    assert_eq!(tag.name, "MyTag1");
    assert_eq!(tag.data_type, CipDataType::Dint);
}

#[test]
fn test_parse_symbol_instance_empty() {
    assert!(parse_symbol_instance(&[]).is_none());
    assert!(parse_symbol_instance(&[0, 0, 0, 0]).is_none());
}
