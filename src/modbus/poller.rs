use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_modbus::prelude::*;

use crate::cache::DeviceCache;
use crate::models::*;

use super::pool::ModbusPool;
use super::types::{Backoff, parse_register};

pub async fn poll_device(
    config: DeviceConfig,
    cache: DeviceCache,
    pool: Arc<ModbusPool>,
    bind_addr: Option<std::net::SocketAddr>,
    sim_config: Option<crate::sim_engine::SimConfig>,
) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_poll(&config, &cache, &pool, bind_addr, sim_config.as_ref()).await {
            Ok(()) => backoff.reset(),
            Err(e) => {
                tracing::warn!("Device {device_id} error: {e}");
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                pool.remove(&device_id).await;
                let delay = backoff.next();
                tracing::info!("Device {device_id} retrying in {delay}ms");
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
    }
}

async fn try_poll(
    config: &DeviceConfig,
    cache: &DeviceCache,
    pool: &ModbusPool,
    bind_addr: Option<std::net::SocketAddr>,
    sim_config: Option<&crate::sim_engine::SimConfig>,
) -> Result<()> {
    let addr = format!("{}:{}", config.ip, config.port);
    let socket_addr: std::net::SocketAddr = addr.parse().context("Invalid address")?;

    tracing::info!("Polling device {} at {}", config.id, addr);

    let sim_start = std::time::Instant::now();

    loop {
        if let Some(sim) = sim_config {
            let elapsed = sim_start.elapsed().as_secs_f64();
            let mut metrics = HashMap::new();
            for reg in &config.registers {
                let value = crate::sim_engine::generate_value(sim, elapsed);
                metrics.insert(reg.name.clone(), value);
            }
            cache.update(&config.id, metrics, true).await;
            tracing::debug!("Device {} simulated successfully", config.id);
            tokio::time::sleep(std::time::Duration::from_millis(config.interval_ms)).await;
            continue;
        }

        let mut ctx = pool
            .get_or_connect(&config.id, socket_addr, config.unit_id, bind_addr)
            .await
            .context("Modbus connection failed")?;

        let mut metrics = HashMap::new();

        // Sort registers by area then address for optimal grouping
        let mut sorted_regs: Vec<&RegisterConfig> = config.registers.iter().collect();
        sorted_regs.sort_by_key(|r| (r.register_area.clone(), r.protocol_address()));

        let mut i = 0;
        while i < sorted_regs.len() {
            let first = &sorted_regs[i];
            let area = &first.register_area;
            let start_addr = first.protocol_address();
            let mut end_addr = start_addr + first.register_count();

            let mut group_end = i + 1;
            while group_end < sorted_regs.len()
                && sorted_regs[group_end].register_area == *area
                && sorted_regs[group_end].protocol_address() <= end_addr
            {
                let t = &sorted_regs[group_end];
                let t_end = t.protocol_address() + t.register_count();
                if t_end > end_addr {
                    end_addr = t_end;
                }
                group_end += 1;
            }

            let total = end_addr - start_addr;

            let read_result = if area == "input" {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ctx.read_input_registers(start_addr, total),
                )
                .await
            } else {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ctx.read_holding_registers(start_addr, total),
                )
                .await
            };

            let response = match read_result {
                Ok(Ok(Ok(words))) => words,
                Ok(Ok(Err(code))) => {
                    tracing::warn!(
                        "Device {} Modbus exception at addr {}: {:?}",
                        config.id,
                        start_addr,
                        code
                    );
                    for reg in &sorted_regs[i..group_end] {
                        metrics.insert(reg.name.clone(), f64::NAN);
                    }
                    i = group_end;
                    continue;
                }
                Ok(Err(e)) => {
                    anyhow::bail!(
                        "Modbus read failed for group starting at {}: {}",
                        start_addr,
                        e
                    );
                }
                Err(_) => {
                    anyhow::bail!("Modbus read timeout for group starting at {}", start_addr);
                }
            };

            for reg in &sorted_regs[i..group_end] {
                let offset = (reg.protocol_address() - start_addr) as usize;
                let count = reg.register_count() as usize;
                if offset + count > response.len() {
                    metrics.insert(reg.name.clone(), f64::NAN);
                    continue;
                }
                let value = parse_register(
                    &response[offset..offset + count],
                    &reg.reg_type,
                    &config.endianness,
                ) * reg.scale;
                metrics.insert(reg.name.clone(), value);
            }

            i = group_end;
        }

        cache.update(&config.id, metrics, false).await;
        tracing::debug!("Device {} polled successfully", config.id);

        pool.return_connection(&config.id, ctx).await;
        tokio::time::sleep(std::time::Duration::from_millis(config.interval_ms)).await;
    }
}
