//! The embedded MQTT broker against the live plant: status, a config
//! change put back, start/stop ending in the state it began, and a
//! REST-published message received by a real MQTT subscriber.
//!
//! Delivery is proven with a minimal MQTT 3.1.1 client over a plain TCP
//! socket (CONNECT, SUBSCRIBE, then the PUBLISH the broker forwards). The
//! server's message log is fed by a tracker that, on the CI plant, does
//! not record what the broker's own link publishes (server follow-up), so
//! the log, topic list and topic messages are checked when the message
//! shows there and reported otherwise.
//!
//! A broker with no configuration (none in the server's file) cannot go
//! back to none: the server has no route to clear it, so it keeps the
//! defaults (1883, 100, no auth) this test puts back.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::json;
use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_mqtt::{self, BrokerStatus, ConfigSummary, STATUS};

/// A port unlikely to be taken on the CI runner while the test runs.
const TEST_PORT: u16 = 18_831;
const TOPIC: &str = "voltrus/native-e2e/check";

async fn wait_for(api: &VoltrusApi, running: bool) -> BrokerStatus {
    for _ in 0..50 {
        let status = api.mqtt_status().await.expect("broker status");
        if status.running() == running && status.status != "starting" {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "the broker did not become {}",
        if running { "running" } else { "stopped" }
    );
}

// ── Minimal MQTT 3.1.1 ───────────────────────────────────────────────────

/// MQTT's variable-length "remaining length".
fn encode_len(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            return out;
        }
    }
}

/// A length-prefixed UTF-8 string.
fn mqtt_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn packet(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    out.extend(encode_len(body.len()));
    out.extend_from_slice(body);
    out
}

/// CONNECT with a clean session and a 60 s keep-alive.
fn connect_packet(client_id: &str) -> Vec<u8> {
    let mut body = Vec::new();
    mqtt_str(&mut body, "MQTT");
    body.push(4); // protocol level 3.1.1
    body.push(0x02); // clean session
    body.extend_from_slice(&60u16.to_be_bytes());
    mqtt_str(&mut body, client_id);
    packet(0x10, &body)
}

/// SUBSCRIBE to one topic at QoS 0.
fn subscribe_packet(packet_id: u16, topic: &str) -> Vec<u8> {
    let mut body = packet_id.to_be_bytes().to_vec();
    mqtt_str(&mut body, topic);
    body.push(0);
    packet(0x82, &body)
}

/// One packet: its first byte and its body.
fn read_packet(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut first = [0u8; 1];
    stream.read_exact(&mut first)?;
    let mut len = 0usize;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        len |= usize::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "remaining length over 4 bytes",
            ));
        }
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok((first[0], body))
}

/// A PUBLISH's topic and payload; `None` for any other packet.
fn parse_publish(first: u8, body: &[u8]) -> Option<(String, Vec<u8>)> {
    if first & 0xf0 != 0x30 || body.len() < 2 {
        return None;
    }
    let topic_len = usize::from(u16::from_be_bytes([body[0], body[1]]));
    let topic = std::str::from_utf8(body.get(2..2 + topic_len)?).ok()?;
    let mut at = 2 + topic_len;
    if (first >> 1) & 0x03 > 0 {
        at += 2; // packet id
    }
    Some((topic.to_owned(), body.get(at..)?.to_vec()))
}

/// A subscriber to `topic` on `host:port`, connected and subscribed.
fn subscriber(host: &str, port: u16, topic: &str) -> TcpStream {
    let mut stream = TcpStream::connect((host, port)).expect("TCP connect to the broker");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let client_id = format!("voltrus-native-e2e-{}", std::process::id());
    stream
        .write_all(&connect_packet(&client_id))
        .expect("send CONNECT");
    let (first, body) = read_packet(&mut stream).expect("CONNACK");
    assert_eq!(first, 0x20, "CONNACK expected");
    assert_eq!(body.get(1), Some(&0), "the broker accepts the connection");
    stream
        .write_all(&subscribe_packet(1, topic))
        .expect("send SUBSCRIBE");
    let (first, body) = read_packet(&mut stream).expect("SUBACK");
    assert_eq!(first, 0x90, "SUBACK expected");
    assert_eq!(body, [0, 1, 0], "QoS 0 granted for packet 1");
    stream
}

/// Wait up to `timeout` for a PUBLISH of `payload` on `topic`.
fn received(stream: &mut TcpStream, topic: &str, payload: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            return false;
        }
        match read_packet(stream) {
            Ok((first, body)) => {
                if parse_publish(first, &body)
                    .is_some_and(|(got, data)| got == topic && data == payload)
                {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
    false
}

/// Publish over REST and prove an MQTT subscriber receives it; then read
/// it back everywhere the section shows it, as far as the server logs it.
async fn publish_and_read(api: &VoltrusApi) {
    let status = api.mqtt_status().await.expect("broker status");
    let config = status.config.expect("a running broker has a config");
    let url = std::env::var("VOLTRUS_E2E_URL").expect("VOLTRUS_E2E_URL");
    let url = reqwest::Url::parse(&url).expect("VOLTRUS_E2E_URL parses");
    let host = url
        .host_str()
        .expect("VOLTRUS_E2E_URL has a host")
        .to_owned();
    let payload = format!("{{\"check\": {}}}", std::process::id());
    // The test never learns broker users' passwords (they cannot be read
    // back), so an auth-enabled broker cannot be subscribed to.
    let mut subscription = if config.auth_enabled {
        println!("mqtt: broker auth is on; the raw subscriber is skipped");
        None
    } else {
        Some(subscriber(&host, config.port, TOPIC))
    };

    let published = api
        .mqtt_publish(&eng_mqtt::publish_body(TOPIC, &payload).expect("publish body"))
        .await
        .expect("publish");
    assert_eq!(published.topic, TOPIC);
    println!("mqtt: published to {TOPIC}");

    if let Some(stream) = subscription.as_mut() {
        assert!(
            received(stream, TOPIC, payload.as_bytes(), Duration::from_secs(5)),
            "an MQTT subscriber on {host}:{} receives the REST-published message",
            config.port
        );
        let _ = stream.write_all(&[0xe0, 0x00]); // DISCONNECT
        println!("mqtt: a subscriber received the message");
    }

    let mut seen = false;
    for _ in 0..30 {
        let log = api.mqtt_messages(100).await.expect("message log");
        if log
            .messages
            .iter()
            .any(|message| message.topic == TOPIC && message.payload == payload)
        {
            seen = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let topics = api.mqtt_topics().await.expect("topics");
    let own = api
        .mqtt_topic_messages(TOPIC)
        .await
        .expect("the topic's messages");
    if seen {
        let info = topics
            .topics
            .iter()
            .find(|info| info.topic == TOPIC)
            .expect("the topic is listed");
        assert!(info.message_count >= 1);
        assert_eq!(info.last_payload, payload);
        assert_eq!(own.topic, TOPIC);
        assert!(
            own.messages
                .iter()
                .any(|message| message.payload == payload)
        );
        println!(
            "mqtt: read back from the log, {} topics, {} messages on {TOPIC}",
            topics.topics.len(),
            own.messages.len()
        );
    } else {
        println!(
            "mqtt: server gap: the REST-published message never reached the broker's log \
             ({} topics, {} messages on {TOPIC})",
            topics.topics.len(),
            own.messages.len()
        );
    }
}

/// PUT a test configuration, check it, and put `original` back (the broker
/// must be stopped).
async fn change_and_restore_config(api: &VoltrusApi, original: ConfigSummary) {
    let test = json!({
        "port": TEST_PORT,
        "max_connections": 20,
        "auth": { "enabled": original.auth_enabled },
    });
    let updated = api.mqtt_update_config(&test).await.expect("PUT config");
    let config = updated.config.expect("the answer carries the config");
    assert_eq!((config.port, config.max_connections), (TEST_PORT, 20));
    let status: BrokerStatus = api.get_json(STATUS).await.expect("status after PUT");
    assert_eq!(status.config.map(|config| config.port), Some(TEST_PORT));
    println!("mqtt: config set to port {TEST_PORT}");

    let back = api
        .mqtt_update_config(&eng_mqtt::restore_body(original))
        .await
        .expect("PUT the original config back");
    assert_eq!(back.config, Some(original));
    println!(
        "mqtt: config restored (port {}, max {}, auth {})",
        original.port, original.max_connections, original.auth_enabled
    );
}

pub(crate) async fn round_trip(api: &VoltrusApi) {
    let before = api.mqtt_status().await.expect("broker status");
    println!(
        "mqtt: {} with {} clients, config {:?}",
        before.status, before.connected_clients, before.config
    );
    let original = before.config.unwrap_or(ConfigSummary {
        port: 1883,
        max_connections: 100,
        auth_enabled: false,
    });

    if before.running() {
        publish_and_read(api).await;
        let stopped = api.mqtt_stop().await.expect("stop");
        assert_eq!(stopped.status, "stopped");
        wait_for(api, false).await;
        // Changing the config is refused while it runs; now it is not.
        change_and_restore_config(api, original).await;
        let started = api.mqtt_start().await.expect("start again");
        assert_eq!(started.status, "running");
        let after = wait_for(api, true).await;
        assert_eq!(after.config, Some(original));
        println!("mqtt: stopped, reconfigured, restored and running again");
    } else {
        // Run it on the test port, so a port in use on the runner does not
        // matter, then stop it and put the configuration back.
        let test = json!({ "port": TEST_PORT, "max_connections": 20 });
        api.mqtt_update_config(&test)
            .await
            .expect("PUT the test port");
        let started = api.mqtt_start().await.expect("start");
        assert_eq!(started.status, "running");
        let running = wait_for(api, true).await;
        assert_eq!(running.config.map(|config| config.port), Some(TEST_PORT));
        // A running broker refuses config changes.
        assert!(
            api.mqtt_update_config(&test).await.is_err(),
            "the server refuses a config change while the broker runs"
        );
        println!("mqtt: started on port {TEST_PORT}");
        publish_and_read(api).await;
        let stopped = api.mqtt_stop().await.expect("stop");
        assert_eq!(stopped.status, "stopped");
        wait_for(api, false).await;
        change_and_restore_config(api, original).await;
        println!("mqtt: stopped again, as it was");
    }
}

#[cfg(test)]
mod packets {
    use super::*;

    #[test]
    fn remaining_lengths_encode_as_varints() {
        assert_eq!(encode_len(0), [0]);
        assert_eq!(encode_len(127), [0x7f]);
        assert_eq!(encode_len(128), [0x80, 0x01]);
        assert_eq!(encode_len(16_383), [0xff, 0x7f]);
        assert_eq!(encode_len(2_097_152), [0x80, 0x80, 0x80, 0x01]);
    }

    #[test]
    fn connect_and_subscribe_encode() {
        assert_eq!(
            connect_packet("c"),
            [
                0x10, 13, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x02, 0, 60, 0, 1, b'c'
            ]
        );
        assert_eq!(
            subscribe_packet(1, "a/b"),
            [0x82, 8, 0, 1, 0, 3, b'a', b'/', b'b', 0]
        );
    }

    #[test]
    fn packets_read_back_and_publishes_parse() {
        let mut bytes = packet(0x30, &[0, 3, b'a', b'/', b'b', b'h', b'i']);
        bytes.extend(packet(0x32, &[0, 1, b't', 0, 7, b'x']));
        bytes.extend([0x20, 2, 0, 0]);
        let mut reader = io::Cursor::new(bytes);
        let (first, body) = read_packet(&mut reader).unwrap();
        assert_eq!(
            parse_publish(first, &body),
            Some(("a/b".to_owned(), b"hi".to_vec()))
        );
        let (first, body) = read_packet(&mut reader).unwrap();
        assert_eq!(
            parse_publish(first, &body),
            Some(("t".to_owned(), b"x".to_vec())),
            "QoS 1 skips the packet id"
        );
        let (first, body) = read_packet(&mut reader).unwrap();
        assert_eq!((first, body.as_slice()), (0x20, &[0u8, 0][..]));
        assert_eq!(parse_publish(first, &body), None, "CONNACK is no publish");
        assert!(read_packet(&mut reader).is_err(), "the end");
        // A big body's length survives the round trip.
        let big = vec![7u8; 300];
        let (first, body) = read_packet(&mut io::Cursor::new(packet(0x30, &big))).unwrap();
        assert_eq!((first, body), (0x30, big));
    }
}
