use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::cache::DeviceCache;
#[cfg(feature = "modbus")]
use crate::models::DeviceConfig;
#[cfg(feature = "dnp3")]
use crate::models::Dnp3DeviceConfig;
#[cfg(feature = "ethernet-ip")]
use crate::models::EnipDeviceConfig;
#[cfg(feature = "opcua")]
use crate::models::OpcUaDeviceConfig;
use crate::models::PollerStatus;
#[cfg(feature = "s7")]
use crate::s7::S7DeviceConfig;
use crate::storage::Storage;
use crate::voltrus_plc::VoltrusPlcDeviceConfig;

#[derive(Clone)]
pub struct PollerManager {
    handles: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    cache: DeviceCache,
    storage: Storage,
    network_manager: crate::network::NetworkManager,
}

impl PollerManager {
    pub fn new(
        cache: DeviceCache,
        storage: Storage,
        network_manager: crate::network::NetworkManager,
    ) -> Self {
        Self {
            handles: Arc::new(RwLock::new(HashMap::new())),
            cache,
            storage,
            network_manager,
        }
    }

    /// Check if device is in simulation mode and return config.
    pub async fn get_simulation_config(
        &self,
        device_id: &str,
    ) -> Option<crate::sim_engine::SimConfig> {
        let (sim_mode, profile_id, sim_config) =
            self.storage.get_device_simulation(device_id).await.ok()??;

        if !sim_mode {
            return None;
        }

        // Load profile if specified
        let mut _config_str = sim_config.as_deref().unwrap_or("{}").to_string();

        if let Some(pid) = profile_id
            && let Ok(Some(profile)) = self.storage.get_sim_profile(pid).await
        {
            // Merge profile config with device overrides
            let mut base: serde_json::Value =
                serde_json::from_str(&profile.config).unwrap_or_default();
            if let Some(override_cfg) = &sim_config
                && let Ok(overrides) = serde_json::from_str::<serde_json::Value>(override_cfg)
                && let (Some(base_obj), Some(over_obj)) =
                    (base.as_object_mut(), overrides.as_object())
            {
                for (k, v) in over_obj {
                    base_obj.insert(k.clone(), v.clone());
                }
            }
            _config_str = base.to_string();

            let data_type = profile.data_type.clone();
            let params: std::collections::HashMap<String, serde_json::Value> = base
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            return Some(crate::sim_engine::SimConfig { data_type, params });
        }

        // Parse sim config directly
        let parsed: serde_json::Value = serde_json::from_str(&_config_str).unwrap_or_default();
        let data_type = parsed
            .get("data_type")
            .and_then(|v| v.as_str())
            .unwrap_or("constant")
            .to_string();
        let params: std::collections::HashMap<String, serde_json::Value> = parsed
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, _)| *k != "data_type")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();

        Some(crate::sim_engine::SimConfig { data_type, params })
    }

    #[cfg(feature = "modbus")]
    pub async fn start(&self, config: DeviceConfig) {
        let device_id = config.id.clone();
        self.cache.register_protocol(&device_id, "modbus").await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }

        let bind_addr = if let Some(ref net_id) = config.network {
            self.network_manager.get_bind_address(net_id).await
        } else {
            None
        };

        let cache = self.cache.clone();
        let pool = Arc::new(crate::modbus::ModbusPool::new());
        let sim_config = self.get_simulation_config(&device_id).await;
        let handle = tokio::spawn(async move {
            crate::modbus::poll_device(config, cache, pool, bind_addr, sim_config).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    #[cfg(feature = "opcua")]
    pub async fn start_opcua(&self, config: OpcUaDeviceConfig) {
        let device_id = config.id.clone();
        self.cache.register_protocol(&device_id, "opcua").await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::opcua::poll_opcua_device(config, cache).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    #[cfg(feature = "s7")]
    pub async fn start_s7(&self, config: S7DeviceConfig) {
        let device_id = config.id.clone();
        self.cache.register_protocol(&device_id, "s7").await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::s7::poll_s7_device(config, cache).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    #[cfg(feature = "ethernet-ip")]
    pub async fn start_ethernet_ip(&self, config: EnipDeviceConfig) {
        let device_id = config.id.clone();
        self.cache
            .register_protocol(&device_id, "ethernet_ip")
            .await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::ethernet_ip::poll_enip_device(
                crate::ethernet_ip::EnipDeviceRuntimeConfig {
                    id: config.id,
                    url: config.url,
                    slot: config.slot,
                    interval_ms: config.interval_ms,
                    tags: config.tags,
                },
                cache,
            )
            .await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    #[cfg(feature = "dnp3")]
    pub async fn start_dnp3(&self, config: Dnp3DeviceConfig) {
        let device_id = config.id.clone();
        self.cache.register_protocol(&device_id, "dnp3").await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::dnp3::poll_dnp3_device(config, cache).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    #[cfg(feature = "bacnet")]
    pub async fn start_bacnet(&self, config: crate::models::BacnetDeviceConfig) {
        let device_id = config.id.clone();
        self.cache.register_protocol(&device_id, "bacnet").await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::bacnet::poll_bacnet_device(config, cache).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    pub async fn start_voltrus_plc(&self, config: VoltrusPlcDeviceConfig) {
        let device_id = config.id.clone();
        self.cache
            .register_protocol(&device_id, "voltrus_plc")
            .await;
        if self.is_running(&device_id).await {
            self.stop(&device_id).await;
        }
        let cache = self.cache.clone();
        let handle = tokio::spawn(async move {
            crate::voltrus_plc::poll_voltrus_plc_device(config, cache).await;
        });
        self.handles.write().await.insert(device_id, handle);
    }

    pub async fn stop(&self, device_id: &str) {
        if let Some(handle) = self.handles.write().await.remove(device_id) {
            handle.abort();
        }
    }

    #[cfg(feature = "modbus")]
    pub async fn restart(&self, config: DeviceConfig) {
        self.stop(&config.id).await;
        self.start(config).await;
    }

    pub async fn stop_all(&self) {
        let mut handles = self.handles.write().await;
        for (_, handle) in handles.drain() {
            handle.abort();
        }
    }

    pub async fn get_statuses(&self) -> Vec<PollerStatus> {
        let handles = self.handles.read().await;
        let cache_statuses = self.cache.get_statuses().await;
        handles
            .keys()
            .map(|id| {
                let status = cache_statuses
                    .get(id)
                    .map(|s| s.as_str())
                    .unwrap_or("offline");
                PollerStatus {
                    device_id: id.clone(),
                    running: status == "online",
                    status: status.to_string(),
                }
            })
            .collect()
    }

    pub async fn is_running(&self, device_id: &str) -> bool {
        self.handles.read().await.contains_key(device_id)
    }
}
