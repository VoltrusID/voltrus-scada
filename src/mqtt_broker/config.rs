use std::collections::HashMap;
use std::net::SocketAddr;

use crate::models::MqttBrokerConfig;

/// Build a rumqttd Config from our MqttBrokerConfig.
pub fn build_rumqttd_config(config: &MqttBrokerConfig) -> rumqttd::Config {
    let name = format!("voltrus-mqtt-{}", config.port);
    let listen: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("valid socket address");

    let server = rumqttd::ServerSettings {
        name,
        listen,
        tls: None,
        next_connection_delay_ms: 1,
        connections: rumqttd::ConnectionSettings {
            connection_timeout_ms: 60_000u16,
            max_payload_size: 268_435_456,
            max_inflight_count: 1000,
            auth: None,
            external_auth: None,
            dynamic_filters: true,
        },
    };

    let mut v4 = HashMap::new();
    v4.insert("v4-1".to_string(), server);

    rumqttd::Config {
        id: 0,
        router: rumqttd::RouterConfig {
            max_connections: config.max_connections as usize,
            max_outgoing_packet_count: 1000,
            max_segment_size: 104857600,
            max_segment_count: 10,
            custom_segment: None,
            initialized_filters: None,
            shared_subscriptions_strategy: rumqttd::Strategy::RoundRobin,
        },
        v4: Some(v4),
        v5: None,
        ws: None,
        cluster: None,
        console: None,
        bridge: None,
        prometheus: None,
        metrics: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_config_default() {
        let config = MqttBrokerConfig {
            enabled: true,
            port: 1883,
            max_connections: 500,
            auth: crate::models::MqttAuthConfig {
                mode: "none".to_string(),
                users: vec![],
            },
        };
        let _rumqttd = build_rumqttd_config(&config);
    }
}
