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
    // The item's address carries the bit offset; a bit write's data byte is
    // only the new state (0x01), as a PLC expects with transport size BIT.
    let bytes = S7Client::encode_value(&S7Value::Bool(true), &tag);
    assert_eq!(bytes, vec![0x01]);
}

// ── Typed writes ──

#[test]
fn test_resolve_applies_configured_type() {
    let t = S7Tag::resolve("DB1.DBD4", "real").unwrap();
    assert_eq!(t.data_type, S7DataType::Real);
    let t = S7Tag::resolve("DB1.DBD4", "dint").unwrap();
    assert_eq!(t.data_type, S7DataType::DInt);
    let t = S7Tag::resolve("DB1.DBD4", "").unwrap();
    assert_eq!(t.data_type, S7DataType::DWord);
    let t = S7Tag::resolve("MW2", "INT").unwrap();
    assert_eq!(t.data_type, S7DataType::Int);
    let t = S7Tag::resolve("Q0.5", "bool").unwrap();
    assert_eq!((t.data_type, t.bit_offset), (S7DataType::Bool, 5));
}

#[test]
fn test_resolve_refuses_type_that_does_not_fit() {
    assert!(S7Tag::resolve("DB1.DBW0", "real").is_err());
    assert!(S7Tag::resolve("DB1.DBX0.0", "byte").is_err());
    assert!(S7Tag::resolve("DB1.DBD0", "bool").is_err());
    assert!(S7Tag::resolve("DB1.DBD0", "lreal").is_err());
}

#[test]
fn test_from_parts() {
    let t = S7Tag::from_parts("DB", 5, 12, 0, "real").unwrap();
    assert_eq!(
        (t.area, t.db_number, t.offset, t.data_type),
        (S7Area::DB, 5, 12, S7DataType::Real)
    );
    let t = S7Tag::from_parts("M", 0, 3, 7, "bool").unwrap();
    assert_eq!(t.bit_offset, 7);
    assert!(S7Tag::from_parts("X", 0, 0, 0, "real").is_err());
    assert!(S7Tag::from_parts("DB", 1, 0, 0, "quad").is_err());
    assert!(S7Tag::from_parts("M", 0, 0, 8, "bool").is_err());
}

fn encoded(address: &str, data_type: &str, value: f64) -> Vec<u8> {
    let tag = S7Tag::resolve(address, data_type).unwrap();
    S7Client::encode_value(&tag.value_for(value).unwrap(), &tag)
}

#[test]
fn test_value_encodings() {
    assert_eq!(encoded("DB1.DBX0.3", "", 1.0), vec![0x01]);
    assert_eq!(encoded("DB1.DBX0.3", "", 0.0), vec![0x00]);
    assert_eq!(encoded("DB1.DBB0", "", 200.0), vec![200]);
    assert_eq!(encoded("DB1.DBW0", "", 0xABCD as f64), vec![0xAB, 0xCD]);
    assert_eq!(encoded("DB1.DBW0", "int", -2.0), vec![0xFF, 0xFE]);
    assert_eq!(
        encoded("DB1.DBD0", "", 70_000.0),
        vec![0x00, 0x01, 0x11, 0x70]
    );
    assert_eq!(
        encoded("DB1.DBD0", "dint", -100_000.0),
        vec![0xFF, 0xFE, 0x79, 0x60]
    );
    assert_eq!(
        encoded("DB1.DBD0", "real", 220.5),
        vec![0x43, 0x5C, 0x80, 0x00]
    );
}

#[test]
fn test_value_ranges() {
    let word = S7Tag::resolve("MW0", "").unwrap();
    assert!(word.value_for(-1.0).is_err());
    assert!(word.value_for(65_536.0).is_err());
    let int = S7Tag::resolve("MW0", "int").unwrap();
    assert!(int.value_for(32_768.0).is_err());
    assert_eq!(int.value_for(-32_768.0).unwrap(), S7Value::Int(-32_768));
    let byte = S7Tag::resolve("MB0", "").unwrap();
    assert_eq!(byte.value_for(9.6).unwrap(), S7Value::Byte(10));
    let real = S7Tag::resolve("MD0", "real").unwrap();
    assert!(real.value_for(f64::INFINITY).is_err());
}

#[test]
fn test_decode_signed_types() {
    let int = S7Tag::resolve("MW0", "int").unwrap();
    assert_eq!(
        S7Client::decode_value(&[0xFF, 0xFE], &int),
        S7Value::Int(-2)
    );
    let dint = S7Tag::resolve("MD0", "dint").unwrap();
    assert_eq!(
        S7Client::decode_value(&[0xFF, 0xFE, 0x79, 0x60], &dint),
        S7Value::DInt(-100_000)
    );
}

#[test]
fn test_write_request_data_item_header() {
    // (address, type) -> data transport size, length field, value bytes
    let cases: [(&str, &str, u8, u16, Vec<u8>); 4] = [
        ("DB1.DBX2.3", "", 0x03, 1, vec![0x01]),
        ("DB1.DBW2", "int", 0x04, 16, vec![0x00, 0x07]),
        ("DB1.DBD2", "dint", 0x04, 32, vec![0x00, 0x00, 0x00, 0x07]),
        ("DB1.DBD2", "real", 0x07, 4, 7.0f32.to_be_bytes().to_vec()),
    ];
    for (address, data_type, transport, length, bytes) in cases {
        let tag = S7Tag::resolve(address, data_type).unwrap();
        let mut client = S7Client::new("127.0.0.1:102", 0, 1);
        let req = client
            .build_write_var_request(&tag, &tag.value_for(7.0).unwrap_or(S7Value::Bool(true)))
            .unwrap();
        // 10-byte job header, function and item count, the 12-byte item, then
        // the data item.
        let item = &req[12..24];
        assert_eq!(item[3], tag.data_type.transport_size(), "{address}");
        let offset_bits = u32::from_be_bytes([0, item[9], item[10], item[11]]);
        assert_eq!(offset_bits, 2 * 8 + u32::from(tag.bit_offset), "{address}");
        let data = &req[24..];
        assert_eq!(data[1], transport, "{address}");
        assert_eq!(u16::from_be_bytes([data[2], data[3]]), length, "{address}");
        assert_eq!(&data[4..], bytes.as_slice(), "{address}");
    }
}

// ── ReadVar framing ──

/// A ReadVar ack as a CPU sends it (S7comm, ROSCTR 3): four items answering
/// DB1.DBX10.3 (BOOL), DB1.DBW12 (INT), DB99.DBB0 (no such DB) and DB1.DBD14
/// (REAL). Bit and integer lengths are in bits, REAL in bytes, and the
/// one-byte BOOL is followed by a fill byte.
const READ_VAR_ACK: [u8; 38] = [
    0x32, 0x03, 0x00, 0x00, 0x00, 0x05, // protocol, ack-data, reserved, PDU ref 5
    0x00, 0x02, 0x00, 0x18, // parameter length 2, data length 24
    0x00, 0x00, // error class / code
    0x04, 0x04, // ReadVar, 4 items
    0xFF, 0x03, 0x00, 0x01, 0x01, 0x00, // BOOL: success, BIT, 1 bit, 1, fill
    0xFF, 0x04, 0x00, 0x10, 0xFF, 0xFE, // INT: success, BYTE/WORD, 16 bits, -2
    0x0A, 0x00, 0x00, 0x00, // object does not exist
    0xFF, 0x07, 0x00, 0x04, 0x43, 0x5C, 0x80, 0x00, // REAL: 4 bytes, 220.5
];

fn ack_tags() -> Vec<S7Tag> {
    vec![
        S7Tag::resolve("DB1.DBX10.3", "").unwrap(),
        S7Tag::resolve("DB1.DBW12", "int").unwrap(),
        S7Tag::resolve("DB99.DBB0", "").unwrap(),
        S7Tag::resolve("DB1.DBD14", "real").unwrap(),
    ]
}

#[test]
fn test_parse_read_var_ack_byte_exact() {
    let values = S7Client::parse_read_var_response(&READ_VAR_ACK, &ack_tags()).unwrap();
    assert_eq!(
        values,
        vec![
            Ok(S7Value::Bool(true)),
            Ok(S7Value::Int(-2)),
            Err(0x0A),
            Ok(S7Value::Real(220.5)),
        ]
    );
}

#[test]
fn test_bit_state_is_bit_zero_whatever_the_offset() {
    let tag = S7Tag::resolve("M0.7", "").unwrap();
    assert_eq!(S7Client::decode_value(&[0x01], &tag), S7Value::Bool(true));
    assert_eq!(S7Client::decode_value(&[0x00], &tag), S7Value::Bool(false));
}

#[test]
fn test_parse_read_var_refuses_short_or_mismatched() {
    let tags = ack_tags();
    assert!(S7Client::parse_read_var_response(&READ_VAR_ACK[..30], &tags).is_err());
    assert!(S7Client::parse_read_var_response(&READ_VAR_ACK, &tags[..3]).is_err());
    let mut refused = READ_VAR_ACK;
    refused[10] = 0x85;
    assert!(S7Client::parse_read_var_response(&refused, &tags).is_err());
}

#[test]
fn test_read_var_request_byte_exact() {
    let mut client = S7Client::new("127.0.0.1:102", 0, 1);
    let tags = ack_tags();
    let req = client.build_read_var_request(&tags[..2]).unwrap();
    assert_eq!(
        req,
        vec![
            0x32, 0x01, 0x00, 0x00, 0x00, 0x01, // job, PDU ref 1
            0x00, 0x1A, 0x00, 0x00, // 26 bytes of parameters, no data part
            0x04, 0x02, // ReadVar, 2 items
            0x12, 0x0A, 0x10, 0x01, 0x00, 0x01, 0x00, 0x01, 0x84, 0x00, 0x00, 0x53, // DBX10.3
            0x12, 0x0A, 0x10, 0x04, 0x00, 0x01, 0x00, 0x01, 0x84, 0x00, 0x00, 0x60, // DBW12
        ]
    );
}

#[test]
fn test_negotiated_pdu_size_offset() {
    // Setup Communication ack: header, F0 00, jobs 1/1, PDU 240.
    let ack = [
        0x32, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00,
        0x01, 0x00, 0x01, 0x00, 0xF0,
    ];
    assert_eq!(S7Client::negotiated_pdu_size(&ack).unwrap(), 240);
    assert!(S7Client::negotiated_pdu_size(&ack[..18]).is_err());
}

#[test]
fn test_plan_read_requests_fits_pdu_and_item_limit() {
    let reals: Vec<S7Tag> = (0..30)
        .map(|i| S7Tag::resolve(&format!("DB1.DBD{}", i * 4), "real").unwrap())
        .collect();
    // 480: the item limit splits 30 tags into 20 + 10.
    assert_eq!(plan_read_requests(&reals, 480), vec![0..20, 20..30]);
    // 100: requests of 12 + 12n bytes hold 7 items.
    let plans = plan_read_requests(&reals, 100);
    assert!(plans.iter().all(|r| r.len() <= 7));
    assert_eq!(plans.iter().map(|r| r.len()).sum::<usize>(), 30);
    // Responses bound the plan when the values are large.
    let strings: Vec<S7Tag> = (0..3)
        .map(|i| S7Tag::from_parts("DB", 1, i * 256, 0, "string").unwrap())
        .collect();
    assert_eq!(plan_read_requests(&strings, 480), vec![0..1, 1..2, 2..3]);
    assert!(plan_read_requests(&[], 240).is_empty());
}

/// A CPU stand-in that negotiates a small PDU, refuses jobs that exceed it
/// and answers each item from DB1, which holds byte i at offset i.
async fn small_pdu_plc(
    pdu: u16,
) -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_packet(s: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
        let mut h = [0u8; 4];
        s.read_exact(&mut h).await.ok()?;
        let mut p = vec![0u8; u16::from_be_bytes([h[2], h[3]]) as usize - 4];
        s.read_exact(&mut p).await.ok()?;
        Some(p)
    }
    fn packet(s7: &[u8]) -> Vec<u8> {
        let mut p = vec![0x03, 0x00, 0, 0, 0x02, 0xF0, 0x80];
        p.extend_from_slice(s7);
        let len = p.len() as u16;
        p[2..4].copy_from_slice(&len.to_be_bytes());
        p
    }
    fn ack(pdu_ref: &[u8], error: u16, params: &[u8], data: &[u8]) -> Vec<u8> {
        let mut f = vec![0x32, 0x03, 0, 0, pdu_ref[0], pdu_ref[1]];
        f.extend_from_slice(&(params.len() as u16).to_be_bytes());
        f.extend_from_slice(&(data.len() as u16).to_be_bytes());
        f.extend_from_slice(&error.to_be_bytes());
        f.extend_from_slice(params);
        f.extend_from_slice(data);
        f
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        read_packet(&mut s).await.unwrap(); // COTP CR
        s.write_all(&[
            0x03, 0x00, 0x00, 0x0B, 0x06, 0xD0, 0x00, 0x01, 0x00, 0x01, 0x00,
        ])
        .await
        .unwrap();
        while let Some(p) = read_packet(&mut s).await {
            let job = &p[3..];
            let pdu_ref = &job[4..6];
            let reply = match job[10] {
                0xF0 => {
                    let mut params = vec![0xF0, 0x00, 0x00, 0x01, 0x00, 0x01];
                    params.extend_from_slice(&pdu.to_be_bytes());
                    ack(pdu_ref, 0, &params, &[])
                }
                0x04 => {
                    counter.fetch_add(1, Ordering::SeqCst);
                    let n = job[11] as usize;
                    let mut data = Vec::new();
                    for i in 0..n {
                        let item = &job[12 + 12 * i..24 + 12 * i];
                        let offset = u32::from_be_bytes([0, item[9], item[10], item[11]]) / 8;
                        let bytes: Vec<u8> = (0..4).map(|k| (offset + k) as u8).collect();
                        data.extend_from_slice(&[0xFF, 0x07, 0x00, 0x04]);
                        data.extend_from_slice(&bytes);
                    }
                    if job.len() > pdu as usize || 14 + data.len() > pdu as usize || n > 20 {
                        ack(pdu_ref, 0x8500, &[0x04, 0x00], &[])
                    } else {
                        ack(pdu_ref, 0, &[0x04, n as u8], &data)
                    }
                }
                _ => break,
            };
            if s.write_all(&packet(&reply)).await.is_err() {
                break;
            }
        }
    });
    (addr, requests)
}

#[tokio::test]
async fn test_read_tags_splits_to_the_negotiated_pdu() {
    let (addr, requests) = small_pdu_plc(100).await;
    let mut client = S7Client::new(&addr.to_string(), 0, 1);
    client.connect().await.unwrap();
    assert_eq!(client.pdu_size(), 100);

    let tags: Vec<S7Tag> = (0..16)
        .map(|i| S7Tag::resolve(&format!("DB1.DBD{}", i * 4), "real").unwrap())
        .collect();
    let values = client.read_tags(&tags).await.unwrap();
    assert_eq!(values.len(), 16);
    for (i, v) in values.iter().enumerate() {
        let o = (i * 4) as u8;
        let expected = f32::from_be_bytes([o, o + 1, o + 2, o + 3]);
        assert_eq!(v, &Ok(S7Value::Real(expected)), "tag {i}");
    }
    assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 3);
}
