use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::models::NetworkConfig;
use axum::{Json, extract::State};

#[derive(Clone)]
pub struct NetworkManager {
    networks: Arc<RwLock<HashMap<String, NetworkConfig>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub addresses: Vec<String>,
    pub status: String,
}

impl NetworkManager {
    pub fn new(configs: Vec<NetworkConfig>) -> Self {
        let map: HashMap<String, NetworkConfig> =
            configs.into_iter().map(|c| (c.id.clone(), c)).collect();
        Self {
            networks: Arc::new(RwLock::new(map)),
        }
    }

    pub async fn get_bind_address(&self, network_id: &str) -> Option<SocketAddr> {
        let networks = self.networks.read().await;
        networks
            .get(network_id)
            .and_then(|n| format!("{}:0", n.bind_address).parse().ok())
    }

    pub async fn list_configured(&self) -> Vec<NetworkConfig> {
        let networks = self.networks.read().await;
        networks.values().cloned().collect()
    }

    pub fn detect_interfaces() -> Vec<InterfaceInfo> {
        let mut interfaces = Vec::new();
        let nets = sysinfo::Networks::new_with_refreshed_list();

        for (name, data) in &nets {
            let addresses: Vec<String> = data.ip_networks().iter().map(|n| n.to_string()).collect();
            interfaces.push(InterfaceInfo {
                name: name.clone(),
                addresses,
                status: "up".to_string(),
            });
        }
        interfaces
    }
}

pub async fn bind_and_connect(
    bind_addr: Option<SocketAddr>,
    target: SocketAddr,
) -> std::io::Result<tokio::net::TcpStream> {
    match bind_addr {
        Some(local) => {
            let socket = tokio::net::TcpSocket::new_v4()?;
            socket.bind(local)?;
            socket.connect(target).await
        }
        None => tokio::net::TcpStream::connect(target).await,
    }
}

pub async fn handle_list_networks(
    State(state): State<crate::api::AppState>,
) -> Json<serde_json::Value> {
    let interfaces = NetworkManager::detect_interfaces();
    let configured = state.network_manager.list_configured().await;
    let configured_ids: Vec<String> = configured.iter().map(|c| c.id.clone()).collect();

    Json(serde_json::json!({
        "interfaces": interfaces,
        "configured_networks": configured_ids
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_bind_address_found() {
        let mgr = NetworkManager::new(vec![NetworkConfig {
            id: "scada-lan".to_string(),
            description: String::new(),
            interface: "eth0".to_string(),
            bind_address: "192.168.1.10".to_string(),
        }]);
        let addr = mgr.get_bind_address("scada-lan").await;
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().ip().to_string(), "192.168.1.10");
    }

    #[tokio::test]
    async fn test_get_bind_address_not_found() {
        let mgr = NetworkManager::new(vec![]);
        let addr = mgr.get_bind_address("missing").await;
        assert!(addr.is_none());
    }
}
