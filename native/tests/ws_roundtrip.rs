//! Wire-compat guards for the MessagePack WS protocol.
//! Mirrors the server's `src/ws_types.rs` tests — if these and the server's
//! pass against the same bytes, the two enums have not drifted.

use voltrus_scada_native::ws_types::{ClientMessage, ServerMessage, TagUpdateEntry};

fn enc(msg: &ClientMessage) -> Vec<u8> {
    // The wire encoding the client actually uses (see runtime.rs): named
    // struct maps — same encoder as the server's `send_msg`.
    rmp_serde::to_vec_named(msg).unwrap()
}

#[test]
fn client_message_roundtrip() {
    let msg = ClientMessage::Subscribe {
        tag_ids: vec!["PLC01.voltage".to_string(), "PLC01.current".to_string()],
    };
    let decoded: ClientMessage = rmp_serde::from_slice(&enc(&msg)).unwrap();
    match decoded {
        ClientMessage::Subscribe { tag_ids } => {
            assert_eq!(tag_ids.len(), 2);
            assert_eq!(tag_ids[0], "PLC01.voltage");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn auth_first_frame_shape() {
    // Server requires: binary frame, named struct map, tag "type" = "Auth".
    let bytes = enc(&ClientMessage::Auth {
        token: "jwt".to_string(),
    });
    let first = bytes[0];
    // fixmap 0x80|n — Auth has 2 fields (type, token)
    assert_eq!(first, 0x82, "expected fixmap(2), got {first:#02x}");
    let decoded: ClientMessage = rmp_serde::from_slice(&bytes).unwrap();
    assert!(matches!(decoded, ClientMessage::Auth { .. }));
}

#[test]
fn server_tag_update_roundtrip() {
    let msg = ServerMessage::TagUpdate {
        tag_id: "PLC01.voltage".to_string(),
        value: 230.5,
        quality: "good".to_string(),
        ts: 1700000000,
        simulated: false,
    };
    let bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let decoded: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
    match decoded {
        ServerMessage::TagUpdate { tag_id, value, .. } => {
            assert_eq!(tag_id, "PLC01.voltage");
            assert!((value - 230.5).abs() < 0.001);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn server_batch_roundtrip() {
    let msg = ServerMessage::TagUpdateBatch {
        updates: vec![
            TagUpdateEntry {
                tag_id: "t1".to_string(),
                value: 2.0,
                quality: "normal".to_string(),
                ts: 1,
                simulated: true,
            },
            TagUpdateEntry {
                tag_id: "t2".to_string(),
                value: -1.5,
                quality: "bad".to_string(),
                ts: 2,
                simulated: false,
            },
        ],
    };
    let bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let decoded: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
    match decoded {
        ServerMessage::TagUpdateBatch { updates } => {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[1].tag_id, "t2");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn server_alarm_roundtrip() {
    let msg = ServerMessage::Alarm {
        alarm_id: 42,
        tag_id: "PLC01.temp".to_string(),
        severity: "critical".to_string(),
        message: "over limit".to_string(),
        value: 99.0,
        ts: 1700000042,
    };
    let bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let decoded: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
    assert!(matches!(decoded, ServerMessage::Alarm { alarm_id: 42, .. }));
}

#[test]
fn all_client_message_types_roundtrip() {
    let messages = [
        ClientMessage::Auth {
            token: "test-jwt".to_string(),
        },
        ClientMessage::Subscribe { tag_ids: vec![] },
        ClientMessage::Unsubscribe { tag_ids: vec![] },
        ClientMessage::TagWrite {
            tag_id: "t1".to_string(),
            value: 42.0,
            command_id: None,
        },
        ClientMessage::Ping,
    ];
    for msg in &messages {
        let bytes = rmp_serde::to_vec_named(msg).unwrap();
        let _: ClientMessage = rmp_serde::from_slice(&bytes).unwrap();
    }
}

#[test]
fn all_server_message_types_roundtrip() {
    let messages = [
        ServerMessage::AuthOk {
            user: "admin".to_string(),
            permissions: vec![],
        },
        ServerMessage::AuthError {
            message: "bad token".to_string(),
        },
        ServerMessage::TagUpdate {
            tag_id: "t".to_string(),
            value: 1.0,
            quality: "good".to_string(),
            ts: 0,
            simulated: false,
        },
        ServerMessage::TagUpdateBatch {
            updates: vec![TagUpdateEntry {
                tag_id: "t1".to_string(),
                value: 2.0,
                quality: "normal".to_string(),
                ts: 1,
                simulated: true,
            }],
        },
        ServerMessage::CommandAck {
            command_id: "c1".to_string(),
            status: "acked".to_string(),
            device_response: None,
            latency_ms: Some(50),
        },
        ServerMessage::DeviceStatus {
            device_id: "d1".to_string(),
            online: true,
            latency_ms: Some(10),
        },
        ServerMessage::Alarm {
            alarm_id: 1,
            tag_id: "t".to_string(),
            severity: "warning".to_string(),
            message: "high".to_string(),
            value: 99.0,
            ts: 0,
        },
        ServerMessage::Pong,
        ServerMessage::HaStatus {
            role: "primary".to_string(),
            peer_status: "ok".to_string(),
            sync_lag_ms: Some(3),
            node_id: "n1".to_string(),
            peer_id: Some("n2".to_string()),
            failover_ts: None,
        },
    ];
    for msg in &messages {
        let bytes = rmp_serde::to_vec_named(msg).unwrap();
        let _: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
    }
}

#[test]
fn msgpack_smaller_than_json() {
    let msg = ServerMessage::TagUpdate {
        tag_id: "PLC01.voltage".to_string(),
        value: 230.5,
        quality: "good".to_string(),
        ts: 1700000000,
        simulated: false,
    };
    let msgpack_bytes = rmp_serde::to_vec_named(&msg).unwrap();
    let json_bytes = serde_json::to_vec(&msg).unwrap();
    assert!(
        msgpack_bytes.len() < json_bytes.len(),
        "MessagePack should be smaller than JSON"
    );
}

#[test]
fn named_map_decode_preserves_token_value() {
    let tok = "eyJ1c2VybmFtZSI6ImFkbWluIn0.sig";
    let bytes = rmp_serde::to_vec_named(&ClientMessage::Auth {
        token: tok.to_string(),
    })
    .unwrap();
    let decoded: ClientMessage = rmp_serde::from_slice(&bytes).unwrap();
    match decoded {
        ClientMessage::Auth { token } => {
            assert_eq!(token, tok, "token value corrupted by named-map decode")
        }
        _ => panic!("wrong variant"),
    }
}
