use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio_modbus::prelude::*;

use crate::cache::DeviceCache;
use crate::models::*;

use super::batch::{ReadBatch, ReadKind, plan_batches, sort_registers};
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

        // Sort registers by area then address, then read each group of neighbours
        // in as few requests as the protocol limits allow.
        let mut sorted_regs: Vec<&RegisterConfig> = config.registers.iter().collect();
        sort_registers(&mut sorted_regs);

        for batch in plan_batches(&sorted_regs) {
            let group = &sorted_regs[batch.first..batch.end];
            let words = read_batch(&mut ctx, &batch).await;

            let response = match words {
                Ok(Ok(Ok(words))) => words,
                Ok(Ok(Err(code))) => {
                    tracing::warn!(
                        "Device {} Modbus exception at addr {}: {:?}",
                        config.id,
                        batch.start,
                        code
                    );
                    for reg in group {
                        metrics.insert(reg.name.clone(), f64::NAN);
                    }
                    continue;
                }
                Ok(Err(e)) => {
                    anyhow::bail!(
                        "Modbus read failed for group starting at {}: {}",
                        batch.start,
                        e
                    );
                }
                Err(_) => {
                    anyhow::bail!("Modbus read timeout for group starting at {}", batch.start);
                }
            };

            for reg in group {
                let offset = (reg.protocol_address() - batch.start) as usize;
                let count = batch.kind.width(reg) as usize;
                if offset + count > response.len() {
                    metrics.insert(reg.name.clone(), f64::NAN);
                    continue;
                }
                let raw = match batch.kind {
                    // A bit is 0 or 1 whatever register type the tag declares.
                    ReadKind::Coil | ReadKind::Discrete => f64::from(response[offset]),
                    ReadKind::Holding | ReadKind::Input => parse_register(
                        &response[offset..offset + count],
                        &reg.reg_type,
                        &config.endianness,
                    ),
                };
                metrics.insert(reg.name.clone(), raw * reg.scale);
            }
        }

        cache.update(&config.id, metrics, false).await;
        tracing::debug!("Device {} polled successfully", config.id);

        pool.return_connection(&config.id, ctx).await;
        tokio::time::sleep(std::time::Duration::from_millis(config.interval_ms)).await;
    }
}

/// The result of one read request: `Ok(Ok(Ok(words)))` is data, `Ok(Ok(Err(_)))`
/// a Modbus exception, `Ok(Err(_))` a transport error, `Err(_)` a timeout.
type ReadOutcome = Result<tokio_modbus::Result<Vec<u16>>, tokio::time::error::Elapsed>;

/// Issue one read. Coils and discrete inputs come back as bits; each becomes a
/// word holding 0 or 1.
async fn read_batch(ctx: &mut tokio_modbus::client::Context, batch: &ReadBatch) -> ReadOutcome {
    let timeout = std::time::Duration::from_secs(5);
    match batch.kind {
        ReadKind::Input => {
            tokio::time::timeout(timeout, ctx.read_input_registers(batch.start, batch.count)).await
        }
        ReadKind::Holding => {
            tokio::time::timeout(
                timeout,
                ctx.read_holding_registers(batch.start, batch.count),
            )
            .await
        }
        ReadKind::Coil => tokio::time::timeout(timeout, ctx.read_coils(batch.start, batch.count))
            .await
            .map(|r| r.map(|r| r.map(bits_to_words))),
        ReadKind::Discrete => {
            tokio::time::timeout(timeout, ctx.read_discrete_inputs(batch.start, batch.count))
                .await
                .map(|r| r.map(|r| r.map(bits_to_words)))
        }
    }
}

fn bits_to_words(bits: Vec<bool>) -> Vec<u16> {
    bits.into_iter().map(u16::from).collect()
}
