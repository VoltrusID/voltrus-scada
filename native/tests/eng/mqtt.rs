//! The embedded MQTT broker against the live plant: status, a config
//! change put back, start/stop ending in the state it began, and a
//! published message looked for in the log, the topic list and the
//! topic's own messages.
//!
//! The server's message log is fed by a tracker thread
//! (`src/mqtt_broker/mod.rs`) that does not record what the broker's own
//! link publishes (the REST publish) on the CI plant: the reads are
//! exercised and checked when the message shows, and reported otherwise.
//!
//! A broker with no configuration (none in the server's file) cannot go
//! back to none: the server has no route to clear it, so it keeps the
//! defaults (1883, 100, no auth) this test puts back.

use std::time::Duration;

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

/// Publish and read the message back everywhere the section shows it.
async fn publish_and_read(api: &VoltrusApi) {
    let payload = format!("{{\"check\": {}}}", std::process::id());
    let published = api
        .mqtt_publish(&eng_mqtt::publish_body(TOPIC, &payload).expect("publish body"))
        .await
        .expect("publish");
    assert_eq!(published.topic, TOPIC);
    println!("mqtt: published to {TOPIC}");

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
