//! Wire-compat guards for the MessagePack WS protocol.
//! Mirrors the server's `src/ws_types.rs` tests — if these and the server's
//! pass against the same bytes, the two enums have not drifted.

use voltrus_scada_native::ws_types::{
    ClientMessage, ServerMessage, Signature, SignatureRequired, TagUpdateEntry,
};

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
    assert_eq!(first, 0x82, "expected fixmap(2), got {first:#04x}");
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
            signature: None,
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
            signature_required: None,
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
        ServerMessage::AlarmUpdate {
            alarm_id: 1,
            tag_id: "t".to_string(),
            event: "acknowledged".to_string(),
            state: "acknowledged".to_string(),
            actor: Some("op".to_string()),
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

#[test]
fn signatures_are_optional_on_the_wire() {
    // Unsigned: the field is left out, as an older server sends and expects.
    let bytes = enc(&ClientMessage::TagWrite {
        tag_id: "t1".to_string(),
        value: 1.0,
        command_id: Some("c1".to_string()),
        signature: None,
    });
    assert_eq!(bytes[0], 0x84, "type, tag_id, value, command_id");
    let signed = ClientMessage::TagWrite {
        tag_id: "t1".to_string(),
        value: 1.0,
        command_id: Some("c1".to_string()),
        signature: Some(Signature {
            password: "secret".to_string(),
            meaning: "I approve".to_string(),
            reason: "batch 7".to_string(),
        }),
    };
    match rmp_serde::from_slice(&enc(&signed)).unwrap() {
        ClientMessage::TagWrite {
            signature: Some(signature),
            ..
        } => assert_eq!(signature.reason, "batch 7"),
        other => panic!("wrong variant: {other:?}"),
    }
    // An acknowledgement without the field decodes, and one with it keeps it.
    let plain = ServerMessage::CommandAck {
        command_id: "c1".to_string(),
        status: "acked".to_string(),
        device_response: None,
        latency_ms: None,
        signature_required: None,
    };
    let bytes = rmp_serde::to_vec_named(&plain).unwrap();
    assert!(matches!(
        rmp_serde::from_slice(&bytes).unwrap(),
        ServerMessage::CommandAck {
            signature_required: None,
            ..
        }
    ));
    let asks = ServerMessage::CommandAck {
        command_id: "c1".to_string(),
        status: "signature_required".to_string(),
        device_response: None,
        latency_ms: None,
        signature_required: Some(SignatureRequired {
            action: "tag_write".to_string(),
            meaning: "I approve".to_string(),
        }),
    };
    let bytes = rmp_serde::to_vec_named(&asks).unwrap();
    match rmp_serde::from_slice(&bytes).unwrap() {
        ServerMessage::CommandAck {
            signature_required: Some(required),
            ..
        } => assert_eq!(required.meaning, "I approve"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn design_frames_roundtrip_with_binary_payloads() {
    use voltrus_scada_native::ws_types::{DesignLimits, DesignRejectedObject};
    let data = vec![0u8, 1, 2, 255];
    let bytes = enc(&ClientMessage::DesignSync {
        screen: "s1".into(),
        data: data.clone(),
    });
    // The payload travels as MessagePack `bin` (0xc4), not an array of ints.
    assert!(
        bytes.windows(2).any(|w| w == [0xc4, 4]),
        "bin8 of 4 bytes: {bytes:02x?}"
    );
    match rmp_serde::from_slice(&bytes).unwrap() {
        ClientMessage::DesignSync { data: back, .. } => assert_eq!(back, data),
        other => panic!("wrong variant: {other:?}"),
    }
    let messages = [
        ServerMessage::DesignJoined {
            screen: "s1".into(),
            epoch: "e".into(),
            resync: false,
            user: "ana".into(),
            color: "#123456".into(),
            protocol: 1,
            limits: DesignLimits {
                max_update: 1,
                max_rate: 2,
                max_doc: 3,
                max_awareness: 4,
                flush_ms: 5,
                idle_ms: 6,
            },
        },
        ServerMessage::DesignAwareness {
            screen: "s1".into(),
            data: vec![1],
        },
        ServerMessage::DesignError {
            screen: "s1".into(),
            code: "disabled".into(),
            message: "off".into(),
            reload: false,
        },
        ServerMessage::DesignRejected {
            screen: "s1".into(),
            objects: vec![DesignRejectedObject {
                object_type: "widget".into(),
                object_id: "w".into(),
                reason: "x".into(),
            }],
        },
        ServerMessage::DesignFlushed {
            screen: "s1".into(),
            seq: 7,
            at_ms: 1,
            users: vec!["ana".into()],
            acked: Some(3),
        },
        ServerMessage::DesignSessions { sessions: vec![] },
        ServerMessage::DesignComment {
            screen: "s1".into(),
            event: "created".into(),
            comment: serde_json::json!({"id": 1}),
        },
    ];
    for msg in &messages {
        let bytes = rmp_serde::to_vec_named(msg).unwrap();
        let _: ServerMessage = rmp_serde::from_slice(&bytes).unwrap();
    }
}

/// Every collaboration frame of the server's `ws_types.rs` is mirrored here
/// with the same fields.
#[test]
fn design_frames_match_the_servers() {
    let server = include_str!("../../src/ws_types.rs");
    let native = include_str!("../src/ws_types.rs");
    let frames = [
        ("DesignJoin {", &["screen", "protocol", "epoch"][..]),
        ("DesignSync {", &["screen", "data"][..]),
        ("DesignAwareness {", &["screen", "data"][..]),
        ("DesignLeave {", &["screen"][..]),
        ("DesignFlush {", &["screen"][..]),
        (
            "DesignJoined {",
            &["epoch", "resync", "user", "color", "protocol", "limits"][..],
        ),
        ("DesignError {", &["code", "message", "reload"][..]),
        ("DesignRejected {", &["objects"][..]),
        ("DesignFlushed {", &["seq", "at_ms", "users", "acked"][..]),
        ("DesignSessions {", &["sessions"][..]),
        ("DesignComment {", &["event", "comment"][..]),
        (
            "pub struct DesignLimits {",
            &[
                "max_update",
                "max_rate",
                "max_doc",
                "max_awareness",
                "flush_ms",
                "idle_ms",
            ][..],
        ),
        (
            "pub struct DesignRejectedObject {",
            &["object_type", "object_id", "reason"][..],
        ),
        ("pub struct DesignSessionInfo {", &["screen", "users"][..]),
        ("pub struct DesignUser {", &["user", "color"][..]),
    ];
    for (head, fields) in frames {
        for (side, text) in [("server", server), ("native", native)] {
            let at = text
                .find(head)
                .unwrap_or_else(|| panic!("{side} has no `{head}`"));
            let body = &text[at..at + text[at..].find('}').unwrap()];
            for field in fields {
                assert!(
                    body.contains(&format!("{field}:")),
                    "{side} `{head}` has no field {field}"
                );
            }
        }
    }
}

/// The fields newer servers add: kept when present, defaulted from an
/// older server's frames (no `acked`, no `idle_ms`), and left out of the
/// encoding when there is no `acked`.
#[test]
fn design_ack_and_idle_fields_roundtrip_and_default() {
    use voltrus_scada_native::ws_types::DesignLimits;
    let flushed = ServerMessage::DesignFlushed {
        screen: "s1".into(),
        seq: 9,
        at_ms: 1,
        users: vec![],
        acked: Some(12),
    };
    let bytes = rmp_serde::to_vec_named(&flushed).unwrap();
    match rmp_serde::from_slice(&bytes).unwrap() {
        ServerMessage::DesignFlushed { acked, users, .. } => {
            assert_eq!(acked, Some(12));
            assert!(users.is_empty(), "an acknowledgement alone");
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let none = ServerMessage::DesignFlushed {
        screen: "s1".into(),
        seq: 9,
        at_ms: 1,
        users: vec!["ana".into()],
        acked: None,
    };
    let bytes = rmp_serde::to_vec_named(&none).unwrap();
    assert!(
        !bytes.windows(5).any(|w| w == b"acked"),
        "no `acked` key when there is none"
    );

    // An older server's frames, written as maps without the new keys.
    #[derive(serde::Serialize)]
    struct OldFlushed<'a> {
        r#type: &'a str,
        screen: &'a str,
        seq: i64,
        at_ms: i64,
        users: Vec<&'a str>,
    }
    let old = rmp_serde::to_vec_named(&OldFlushed {
        r#type: "DesignFlushed",
        screen: "s1",
        seq: 3,
        at_ms: 1,
        users: vec!["ana"],
    })
    .unwrap();
    match rmp_serde::from_slice(&old).unwrap() {
        ServerMessage::DesignFlushed { acked, seq, .. } => {
            assert_eq!((acked, seq), (None, 3));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    #[derive(serde::Serialize)]
    struct OldLimits {
        max_update: usize,
        max_rate: u32,
        max_doc: usize,
        max_awareness: usize,
        flush_ms: u64,
    }
    let old = rmp_serde::to_vec_named(&OldLimits {
        max_update: 1,
        max_rate: 2,
        max_doc: 3,
        max_awareness: 4,
        flush_ms: 5,
    })
    .unwrap();
    let limits: DesignLimits = rmp_serde::from_slice(&old).unwrap();
    assert_eq!(limits.idle_ms, 0, "0: the default idle limit");
    let back: DesignLimits = rmp_serde::from_slice(
        &rmp_serde::to_vec_named(&DesignLimits {
            idle_ms: 9,
            ..limits
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(back.idle_ms, 9);
}
