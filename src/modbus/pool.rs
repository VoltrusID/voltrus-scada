use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

struct PooledConnection {
    ctx: tokio_modbus::client::Context,
    last_used: Instant,
}

/// How long opening a TCP connection to a device may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ModbusPool {
    connections: Arc<tokio::sync::Mutex<HashMap<String, PooledConnection>>>,
    max_idle: Duration,
    /// Resolves a device's `network` to the local address writes bind to,
    /// the same way its poller does.
    networks: Option<crate::network::NetworkManager>,
}

impl ModbusPool {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            max_idle: Duration::from_secs(60),
            networks: None,
        }
    }

    pub fn with_network_manager(mut self, networks: crate::network::NetworkManager) -> Self {
        self.networks = Some(networks);
        self
    }

    /// Local address to bind to for a device on the named network.
    pub async fn bind_address(&self, network: Option<&str>) -> Option<std::net::SocketAddr> {
        match (network, &self.networks) {
            (Some(id), Some(networks)) => networks.get_bind_address(id).await,
            _ => None,
        }
    }

    pub async fn get_or_connect(
        &self,
        device_id: &str,
        addr: std::net::SocketAddr,
        unit_id: u8,
        bind_addr: Option<std::net::SocketAddr>,
    ) -> anyhow::Result<tokio_modbus::client::Context> {
        self.get_or_connect_with(device_id, addr, async move {
            anyhow::Ok(if let Some(local) = bind_addr {
                let stream = crate::network::bind_and_connect(Some(local), addr).await?;
                tokio_modbus::client::tcp::attach_slave(stream, Slave(unit_id))
            } else {
                tcp::connect_slave(addr, Slave(unit_id)).await?
            })
        })
        .await
    }

    /// `get_or_connect` with the connect step given as a future.
    async fn get_or_connect_with(
        &self,
        device_id: &str,
        addr: std::net::SocketAddr,
        connect: impl std::future::Future<Output = anyhow::Result<tokio_modbus::client::Context>>,
    ) -> anyhow::Result<tokio_modbus::client::Context> {
        // The lock covers the map only, never a connect: one unreachable
        // device must not hold up writes to the others.
        let pooled = self.connections.lock().await.remove(device_id);
        if let Some(pooled) = pooled {
            if pooled.last_used.elapsed() < self.max_idle {
                tracing::debug!("Reusing connection for device {device_id}");
                return Ok(pooled.ctx);
            }
            tracing::debug!("Dropping idle connection for device {device_id}");
        }

        tracing::debug!("Connecting to device {device_id} at {addr}");
        tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "connect to {addr} timed out after {}s",
                    CONNECT_TIMEOUT.as_secs()
                )
            })?
    }

    pub async fn return_connection(&self, device_id: &str, ctx: tokio_modbus::client::Context) {
        let mut conns = self.connections.lock().await;
        conns.insert(
            device_id.to_string(),
            PooledConnection {
                ctx,
                last_used: Instant::now(),
            },
        );
    }

    pub async fn remove(&self, device_id: &str) {
        let mut conns = self.connections.lock().await;
        conns.remove(device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_address_comes_from_the_devices_network() {
        let nets = crate::network::NetworkManager::new(vec![crate::models::NetworkConfig {
            id: "ot".into(),
            description: String::new(),
            interface: "lo".into(),
            bind_address: "127.0.0.1".into(),
        }]);
        let pool = ModbusPool::new().with_network_manager(nets);
        assert_eq!(
            pool.bind_address(Some("ot")).await,
            Some("127.0.0.1:0".parse().unwrap())
        );
        assert_eq!(pool.bind_address(Some("other")).await, None);
        assert_eq!(pool.bind_address(None).await, None);
        assert_eq!(ModbusPool::new().bind_address(Some("ot")).await, None);
    }

    /// A connect that never finishes on its own (a device that does not
    /// answer the SYN).
    async fn hanging_connect() -> anyhow::Result<tokio_modbus::client::Context> {
        std::future::pending().await
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_connect_does_not_hold_the_pool_lock() {
        let pool = Arc::new(ModbusPool::new());
        let slow = {
            let pool = pool.clone();
            tokio::spawn(async move {
                pool.get_or_connect_with(
                    "slow",
                    "192.0.2.1:502".parse().unwrap(),
                    hanging_connect(),
                )
                .await
                .map(|_| ())
            })
        };
        // Let the slow connect start and park on its connect.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!slow.is_finished());
        // With paused time the runtime jumps to the next timer: 200 ms comes
        // before the 5 s connect timeout, so this fails if the lock is held.
        tokio::time::timeout(Duration::from_millis(200), pool.remove("other"))
            .await
            .expect("pool lock held during a connect");
        slow.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_connect_times_out() {
        let pool = ModbusPool::new();
        let started = tokio::time::Instant::now();
        let err = pool
            .get_or_connect_with("slow", "192.0.2.1:502".parse().unwrap(), hanging_connect())
            .await
            .map(|_| ())
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
        assert_eq!(started.elapsed(), CONNECT_TIMEOUT);
    }
}
