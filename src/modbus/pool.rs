use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_modbus::client::tcp;
use tokio_modbus::prelude::*;

struct PooledConnection {
    ctx: tokio_modbus::client::Context,
    last_used: Instant,
}

pub struct ModbusPool {
    connections: Arc<tokio::sync::Mutex<HashMap<String, PooledConnection>>>,
    max_idle: Duration,
}

impl ModbusPool {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            max_idle: Duration::from_secs(60),
        }
    }

    pub async fn get_or_connect(
        &self,
        device_id: &str,
        addr: std::net::SocketAddr,
        unit_id: u8,
        bind_addr: Option<std::net::SocketAddr>,
    ) -> anyhow::Result<tokio_modbus::client::Context> {
        let mut conns = self.connections.lock().await;
        if let Some(pooled) = conns.remove(device_id) {
            if pooled.last_used.elapsed() < self.max_idle {
                tracing::debug!("Reusing connection for device {device_id}");
                return Ok(pooled.ctx);
            }
            tracing::debug!("Dropping idle connection for device {device_id}");
        }

        tracing::debug!("Connecting to device {device_id} at {addr}");
        let ctx = if let Some(local) = bind_addr {
            let stream = crate::network::bind_and_connect(Some(local), addr).await?;
            tokio_modbus::client::tcp::attach_slave(stream, Slave(unit_id))
        } else {
            tcp::connect_slave(addr, Slave(unit_id)).await?
        };
        Ok(ctx)
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
