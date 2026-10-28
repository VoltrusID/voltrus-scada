use tokio_modbus::prelude::*;

use super::types::value_to_words;
use crate::models::*;

pub async fn write_tag(
    pool: &crate::modbus::pool::ModbusPool,
    config: &DeviceConfig,
    register: &RegisterConfig,
    value: f64,
) -> WriteResult {
    if !crate::node_role::is_active() {
        return WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let addr = format!("{}:{}", config.ip, config.port);
    let socket_addr: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(e) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Invalid address: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    // Coil writes
    if register.register_area == "coil" {
        return write_coil(
            pool,
            &config.id,
            socket_addr,
            config.unit_id,
            register.protocol_address(),
            value != 0.0,
            start,
        )
        .await;
    }

    let words = value_to_words(value, &register.reg_type, &config.endianness);

    let ctx_result = pool
        .get_or_connect(&config.id, socket_addr, config.unit_id, None)
        .await;

    let mut ctx = match ctx_result {
        Ok(ctx) => ctx,
        Err(e) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Modbus connection failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    let protocol_addr = register.protocol_address();

    let write_result = if words.len() == 1 {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ctx.write_single_register(protocol_addr, words[0]),
        )
        .await
    } else {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ctx.write_multiple_registers(protocol_addr, &words),
        )
        .await
    };

    match write_result {
        Ok(Ok(_)) => {
            pool.return_connection(&config.id, ctx).await;
            WriteResult {
                success: true,
                device_response: Some(format!(
                    "Wrote {} {} = {} (raw: {:?})",
                    register.name, protocol_addr, value, words
                )),
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        Ok(Err(e)) => {
            pool.remove(&config.id).await;
            WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Modbus write failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        Err(_) => {
            pool.remove(&config.id).await;
            WriteResult {
                success: false,
                device_response: None,
                error: Some("Modbus write timeout (5s)".into()),
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
    }
}

async fn write_coil(
    pool: &crate::modbus::pool::ModbusPool,
    device_id: &str,
    socket_addr: std::net::SocketAddr,
    unit_id: u8,
    address: u16,
    coil_on: bool,
    start: std::time::Instant,
) -> WriteResult {
    let ctx_result = pool
        .get_or_connect(device_id, socket_addr, unit_id, None)
        .await;

    let mut ctx = match ctx_result {
        Ok(ctx) => ctx,
        Err(e) => {
            return WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Modbus connection failed: {e}")),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };

    // Try FC05 (Write Single Coil), fall back to FC0F (Write Multiple Coils)
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ctx.write_single_coil(address, coil_on),
    )
    .await;

    match result {
        Ok(Ok(_)) => {
            pool.return_connection(device_id, ctx).await;
            WriteResult {
                success: true,
                device_response: Some(format!("Wrote coil {} = {}", address, coil_on)),
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        _ => {
            let fallback = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                ctx.write_multiple_coils(address, &[coil_on]),
            )
            .await;

            match fallback {
                Ok(Ok(_)) => {
                    pool.return_connection(device_id, ctx).await;
                    WriteResult {
                        success: true,
                        device_response: Some(format!(
                            "Wrote coil {} = {} (FC0F fallback)",
                            address, coil_on
                        )),
                        error: None,
                        latency_ms: start.elapsed().as_millis() as u64,
                    }
                }
                Ok(Err(e)) => {
                    pool.remove(device_id).await;
                    WriteResult {
                        success: false,
                        device_response: None,
                        error: Some(format!("Modbus coil write failed: {e}")),
                        latency_ms: start.elapsed().as_millis() as u64,
                    }
                }
                Err(_) => {
                    pool.remove(device_id).await;
                    WriteResult {
                        success: false,
                        device_response: None,
                        error: Some("Modbus coil write timeout (5s)".into()),
                        latency_ms: start.elapsed().as_millis() as u64,
                    }
                }
            }
        }
    }
}
