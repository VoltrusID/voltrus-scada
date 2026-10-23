use super::*;

#[test]
fn test_parse_db_dword() {
    let tag = S7Client::parse_tag_address("DB1.DBD0").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 1);
    assert_eq!(tag.offset, 0);
    assert_eq!(tag.data_type, S7DataType::DWord);
}

#[test]
fn test_parse_db_bool_with_bit() {
    let tag = S7Client::parse_tag_address("DB1.DBX0.1").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 1);
    assert_eq!(tag.offset, 0);
    assert_eq!(tag.bit_offset, 1);
    assert_eq!(tag.data_type, S7DataType::Bool);
}

#[test]
fn test_parse_db_word() {
    let tag = S7Client::parse_tag_address("DB1.DBW0").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 1);
    assert_eq!(tag.offset, 0);
    assert_eq!(tag.data_type, S7DataType::Word);
}

#[test]
fn test_parse_db_byte() {
    let tag = S7Client::parse_tag_address("DB1.DBB10").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 1);
    assert_eq!(tag.offset, 10);
    assert_eq!(tag.data_type, S7DataType::Byte);
}

#[test]
fn test_parse_db_large_offset() {
    let tag = S7Client::parse_tag_address("DB10.DBD100").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 10);
    assert_eq!(tag.offset, 100);
    assert_eq!(tag.data_type, S7DataType::DWord);
}

#[test]
fn test_parse_input_bool() {
    let tag = S7Client::parse_tag_address("I0.0").unwrap();
    assert_eq!(tag.area, S7Area::Input);
    assert_eq!(tag.offset, 0);
    assert_eq!(tag.bit_offset, 0);
    assert_eq!(tag.data_type, S7DataType::Bool);
}

#[test]
fn test_parse_input_bit_nonzero() {
    let tag = S7Client::parse_tag_address("I1.3").unwrap();
    assert_eq!(tag.area, S7Area::Input);
    assert_eq!(tag.offset, 1);
    assert_eq!(tag.bit_offset, 3);
    assert_eq!(tag.data_type, S7DataType::Bool);
}

#[test]
fn test_parse_marker_word() {
    let tag = S7Client::parse_tag_address("MW100").unwrap();
    assert_eq!(tag.area, S7Area::Marker);
    assert_eq!(tag.offset, 100);
    assert_eq!(tag.data_type, S7DataType::Word);
}

#[test]
fn test_parse_output_word() {
    let tag = S7Client::parse_tag_address("QW200").unwrap();
    assert_eq!(tag.area, S7Area::Output);
    assert_eq!(tag.offset, 200);
    assert_eq!(tag.data_type, S7DataType::Word);
}

#[test]
fn test_parse_marker_dword() {
    let tag = S7Client::parse_tag_address("MD100").unwrap();
    assert_eq!(tag.area, S7Area::Marker);
    assert_eq!(tag.offset, 100);
    assert_eq!(tag.data_type, S7DataType::DWord);
}

#[test]
fn test_parse_marker_byte() {
    let tag = S7Client::parse_tag_address("MB50").unwrap();
    assert_eq!(tag.area, S7Area::Marker);
    assert_eq!(tag.offset, 50);
    assert_eq!(tag.data_type, S7DataType::Byte);
}

#[test]
fn test_parse_output_bool() {
    let tag = S7Client::parse_tag_address("Q0.0").unwrap();
    assert_eq!(tag.area, S7Area::Output);
    assert_eq!(tag.offset, 0);
    assert_eq!(tag.bit_offset, 0);
    assert_eq!(tag.data_type, S7DataType::Bool);
}

#[test]
fn test_parse_invalid_address() {
    assert!(S7Client::parse_tag_address("").is_err());
    assert!(S7Client::parse_tag_address("XYZ").is_err());
}

#[test]
fn test_parse_case_insensitive() {
    let tag = S7Client::parse_tag_address("db1.dbw10").unwrap();
    assert_eq!(tag.area, S7Area::DB);
    assert_eq!(tag.db_number, 1);
    assert_eq!(tag.offset, 10);
    assert_eq!(tag.data_type, S7DataType::Word);
}

#[test]
fn test_tpkt_cotp_packet_construction() {
    let client = S7Client::new("127.0.0.1:102", 0, 1);
    let packet = client.build_cotp_cr().unwrap();

    // TPKT header
    assert_eq!(packet[0], 3); // version
    assert_eq!(packet[1], 0); // reserved
    let total_len = u16::from_be_bytes([packet[2], packet[3]]);
    assert_eq!(total_len as usize, packet.len());

    // COTP CR
    assert!(packet[4] > 0); // COTP length
    assert_eq!(packet[5], 0xE0); // CR PDU type
}

#[test]
fn test_s7_setup_comm_construction() {
    let mut client = S7Client::new("127.0.0.1:102", 0, 1);
    let packet = client.build_s7_setup_comm();

    assert_eq!(packet[0], 0x32); // protocol ID
    assert_eq!(packet[1], S7_MSG_TYPE_JOB); // msg type
    assert_eq!(packet[6], 0x00); // param length high byte is 0 since params are small
    // Function code in params
    assert_eq!(packet[10], S7_SETUP_COMM);
}

#[test]
fn test_s7_read_request_construction() {
    let mut client = S7Client::new("127.0.0.1:102", 0, 1);
    let tags = vec![S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 0,
        data_type: S7DataType::DWord,
        length: 1,
        name: None,
    }];
    let packet = client.build_read_var_request(&tags).unwrap();

    assert_eq!(packet[0], 0x32); // protocol ID
    assert_eq!(packet[1], S7_MSG_TYPE_JOB);
    // Params start at byte 10
    assert_eq!(packet[10], S7_READ_VAR); // function code
    assert_eq!(packet[11], 0x01); // item count
}

#[test]
fn test_s7_write_request_construction() {
    let mut client = S7Client::new("127.0.0.1:102", 0, 1);
    let tag = S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 0,
        data_type: S7DataType::Word,
        length: 1,
        name: None,
    };
    let packet = client
        .build_write_var_request(&tag, &S7Value::Word(1234))
        .unwrap();

    assert_eq!(packet[0], 0x32); // protocol ID
    assert_eq!(packet[1], S7_MSG_TYPE_JOB);
    // Params start at byte 10
    assert_eq!(packet[10], S7_WRITE_VAR);
}

#[test]
fn test_s7_value_bool() {
    let v = S7Value::Bool(true);
    assert!((v.to_f64() - 1.0).abs() < f64::EPSILON);
    let v = S7Value::Bool(false);
    assert!((v.to_f64() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_s7_value_word() {
    let v = S7Value::Word(65535);
    assert!((v.to_f64() - 65535.0).abs() < f64::EPSILON);
}

#[test]
fn test_s7_value_real() {
    let v = S7Value::Real(3.14);
    assert!((v.to_f64() - 3.14).abs() < 0.001);
}

#[test]
fn test_s7_value_string() {
    let v = S7Value::String("hello".to_string());
    assert!((v.to_f64() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_s7_value_serde() {
    let v = S7Value::DWord(42);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "42");

    let v = S7Value::Bool(true);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "true");

    let v = S7Value::Real(1.5);
    let json = serde_json::to_string(&v).unwrap();
    assert!(!json.is_empty());

    let v = S7Value::String("test".to_string());
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "\"test\"");
}

#[test]
fn test_s7_area_codes() {
    assert_eq!(S7Area::DB.code(), 0x84);
    assert_eq!(S7Area::Input.code(), 0x81);
    assert_eq!(S7Area::Output.code(), 0x82);
    assert_eq!(S7Area::Marker.code(), 0x83);
    assert_eq!(S7Area::Timer.code(), 0x1D);
    assert_eq!(S7Area::Counter.code(), 0x1C);

    assert_eq!(S7Area::from_code(0x84), Some(S7Area::DB));
    assert_eq!(S7Area::from_code(0xFF), None);
}

#[test]
fn test_s7_data_type_transport_sizes() {
    assert_eq!(S7DataType::Bool.transport_size(), 0x01);
    assert_eq!(S7DataType::Byte.transport_size(), 0x02);
    assert_eq!(S7DataType::Word.transport_size(), 0x04);
    assert_eq!(S7DataType::DWord.transport_size(), 0x06);
    assert_eq!(S7DataType::Real.transport_size(), 0x08);
}

#[test]
fn test_s7_data_type_from_str() {
    assert_eq!(S7DataType::from_str_name("bool"), Some(S7DataType::Bool));
    assert_eq!(S7DataType::from_str_name("Word"), Some(S7DataType::Word));
    assert_eq!(S7DataType::from_str_name("REAL"), Some(S7DataType::Real));
    assert_eq!(S7DataType::from_str_name("float32"), Some(S7DataType::Real));
    assert_eq!(S7DataType::from_str_name("unknown"), None);
}

#[test]
fn test_decode_value_word_be() {
    let data = [0x00, 0xFF]; // 255 big-endian
    let tag = S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 0,
        data_type: S7DataType::Word,
        length: 1,
        name: None,
    };
    let val = S7Client::decode_value(&data, &tag);
    assert_eq!(val, S7Value::Word(255));
}

#[test]
fn test_decode_value_real() {
    let data = f32::to_be_bytes(25.5);
    let tag = S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 0,
        data_type: S7DataType::Real,
        length: 1,
        name: None,
    };
    let val = S7Client::decode_value(&data, &tag);
    match val {
        S7Value::Real(f) => assert!((f - 25.5).abs() < 0.001),
        _ => panic!("Expected Real value"),
    }
}

#[test]
fn test_encode_value_word() {
    let tag = S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 0,
        data_type: S7DataType::Word,
        length: 1,
        name: None,
    };
    let bytes = S7Client::encode_value(&S7Value::Word(0x1234), &tag);
    assert_eq!(bytes, vec![0x12, 0x34]);
}

#[test]
fn test_encode_value_bool_bit_offset() {
    let tag = S7Tag {
        area: S7Area::DB,
        db_number: 1,
        offset: 0,
        bit_offset: 3,
        data_type: S7DataType::Bool,
        length: 1,
        name: None,
    };
    let bytes = S7Client::encode_value(&S7Value::Bool(true), &tag);
    assert_eq!(bytes, vec![0x08]); // 1 << 3
}
