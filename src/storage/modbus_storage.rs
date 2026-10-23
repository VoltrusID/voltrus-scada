use super::Storage;
use anyhow::{Context, Result};

use crate::models::{DeviceConfig, ModbusDevice, ModbusRegister, ModbusRegisterInput};

impl Storage {
    pub async fn is_yaml_migrated(&self) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT value FROM settings WHERE key = 'modbus_yaml_migrated'")?;
            let result = stmt.query_row([], |row| row.get::<_, String>(0)).ok();
            Ok(result.as_deref() == Some("true"))
        })
        .await
        .context("Task join failed")?
    }

    pub async fn mark_yaml_migrated(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('modbus_yaml_migrated', 'true')",
                [],
            ).context("Mark yaml migrated failed")?;
            Ok(())
        }).await.context("Task join failed")?
    }

    pub async fn import_yaml_devices(&self, devices: &[DeviceConfig]) -> Result<()> {
        let conn = self.conn.clone();
        let devices = devices.to_vec();
        let now = crate::timeutil::now_secs();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;

            for device in &devices {
                let endianness = match device.endianness {
                    crate::models::Endianness::Big => "big",
                    crate::models::Endianness::Little => "little",
                    crate::models::Endianness::Swapped => "swapped",
                };
                tx.execute(
                    "INSERT OR IGNORE INTO modbus_devices (id, name, ip, port, unit_id, interval_ms, endianness, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
                    rusqlite::params![device.id, device.id, device.ip, device.port, device.unit_id, device.interval_ms, endianness, now],
                ).context("Insert modbus device failed")?;

                for (i, reg) in device.registers.iter().enumerate() {
                    let reg_type = match reg.reg_type {
                        crate::models::RegisterType::Int16 => "int16",
                        crate::models::RegisterType::Uint16 => "uint16",
                        crate::models::RegisterType::Int32 => "int32",
                        crate::models::RegisterType::Float32 => "float32",
                        crate::models::RegisterType::Float64 => "float64",
                    };
                    tx.execute(
                        "INSERT OR IGNORE INTO modbus_registers (device_id, name, address, register_type, register_area, scale, sort_order, zero_based) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![device.id, reg.name, reg.address, reg_type, reg.register_area, reg.scale, i as i64, reg.zero_based],
                    ).context("Insert modbus register failed")?;
                }
            }

            tx.commit().context("Commit failed")
        }).await.context("Task join failed")?
    }

    pub async fn count_modbus_devices(&self) -> Result<u32> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM modbus_devices", [], |row| row.get(0))
                .context("Count query failed")?;
            Ok(count as u32)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_modbus_devices(&self) -> Result<Vec<ModbusDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, ip, port, unit_id, interval_ms, endianness, enabled, created_at, updated_at FROM modbus_devices ORDER BY created_at"
            ).context("Prepare failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(ModbusDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    ip: row.get(2)?,
                    port: row.get(3)?,
                    unit_id: row.get(4)?,
                    interval_ms: row.get(5)?,
                    endianness: row.get(6)?,
                    enabled: row.get::<_, i64>(7)? != 0,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            }).context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_modbus_device(&self, id: &str) -> Result<Option<ModbusDevice>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, ip, port, unit_id, interval_ms, endianness, enabled, created_at, updated_at FROM modbus_devices WHERE id = ?1"
            ).context("Prepare failed")?;
            let result = stmt.query_row(rusqlite::params![id], |row| {
                Ok(ModbusDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    ip: row.get(2)?,
                    port: row.get(3)?,
                    unit_id: row.get(4)?,
                    interval_ms: row.get(5)?,
                    endianness: row.get(6)?,
                    enabled: row.get::<_, i64>(7)? != 0,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            }).ok();
            Ok(result)
        }).await.context("Task join failed")?
    }

    pub async fn create_modbus_device(
        &self,
        id: &str,
        name: &str,
        ip: &str,
        port: u16,
        unit_id: u8,
        interval_ms: u64,
        endianness: &str,
        registers: &[ModbusRegisterInput],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.to_string();
        let ip = ip.to_string();
        let endianness = endianness.to_string();
        let registers: Vec<(String, u16, bool, String, String, f64)> = registers
            .iter()
            .map(|r| {
                let (address, zero_based) = r.resolve_address().map_err(|e| anyhow::anyhow!(e))?;
                Ok((
                    r.name.clone(),
                    address,
                    zero_based,
                    r.register_type.clone(),
                    r.register_area.clone(),
                    r.scale,
                ))
            })
            .collect::<Result<_>>()?;
        let now = crate::timeutil::now_secs();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;

            tx.execute(
                "INSERT INTO modbus_devices (id, name, ip, port, unit_id, interval_ms, endianness, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
                rusqlite::params![id, name, ip, port, unit_id, interval_ms, endianness, now],
            ).context("Insert modbus device failed")?;

            for (i, (rname, addr, zero_based, rtype, rarea, scale)) in registers.iter().enumerate() {
                tx.execute(
                    "INSERT INTO modbus_registers (device_id, name, address, register_type, register_area, scale, sort_order, zero_based) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![id, rname, addr, rtype, rarea, scale, i as i64, zero_based],
                ).context("Insert modbus register failed")?;
            }

            tx.commit().context("Commit failed")
        }).await.context("Task join failed")?
    }

    pub async fn update_modbus_device(&self, id: &str, fields: &serde_json::Value) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let now = crate::timeutil::now_secs();
        let fields = fields.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let obj = fields.as_object().context("Expected JSON object")?;

            // Extract all optional values
            let name = obj.get("name").and_then(|v| v.as_str());
            let ip = obj.get("ip").and_then(|v| v.as_str());
            let endianness = obj.get("endianness").and_then(|v| v.as_str());
            let port: Option<u16> = obj.get("port").and_then(|v| v.as_u64()).map(|v| v as u16);
            let unit_id: Option<u8> = obj.get("unit_id").and_then(|v| v.as_u64()).map(|v| v as u8);
            let interval_ms: Option<u64> = obj.get("interval_ms").and_then(|v| v.as_u64());
            let enabled: Option<bool> = obj.get("enabled").and_then(|v| v.as_bool());

            // Build SET clauses and params list
            let mut sets = Vec::new();
            let mut p: Vec<rusqlite::types::Value> = Vec::new();

            if let Some(v) = name {
                sets.push("name = ?");
                p.push(rusqlite::types::Value::from(v.to_string()));
            }
            if let Some(v) = ip {
                sets.push("ip = ?");
                p.push(rusqlite::types::Value::from(v.to_string()));
            }
            if let Some(v) = endianness {
                sets.push("endianness = ?");
                p.push(rusqlite::types::Value::from(v.to_string()));
            }
            if let Some(v) = port {
                sets.push("port = ?");
                p.push(rusqlite::types::Value::from(v as i64));
            }
            if let Some(v) = unit_id {
                sets.push("unit_id = ?");
                p.push(rusqlite::types::Value::from(v as i64));
            }
            if let Some(v) = interval_ms {
                sets.push("interval_ms = ?");
                p.push(rusqlite::types::Value::from(v as i64));
            }
            if let Some(v) = enabled {
                sets.push("enabled = ?");
                p.push(rusqlite::types::Value::from(v as i64));
            }

            if sets.is_empty() {
                return Ok(());
            }
            sets.push("updated_at = ?");
            p.push(rusqlite::types::Value::from(now));
            p.push(rusqlite::types::Value::from(id));

            let sql = format!("UPDATE modbus_devices SET {} WHERE id = ?", sets.join(", "));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                p.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
            conn.execute(&sql, param_refs.as_slice())
                .context("Update modbus device failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_modbus_device(&self, id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM modbus_devices WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete modbus device failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_modbus_registers(&self, device_id: &str) -> Result<Vec<ModbusRegister>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, device_id, name, address, register_type, register_area, scale, sort_order, zero_based FROM modbus_registers WHERE device_id = ?1 ORDER BY sort_order"
            ).context("Prepare failed")?;
            let rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(ModbusRegister {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    name: row.get(2)?,
                    address: row.get(3)?,
                    register_type: row.get(4)?,
                    register_area: row.get(5)?,
                    scale: row.get(6)?,
                    sort_order: row.get(7)?,
                    zero_based: row.get(8)?,
                })
            }).context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    pub async fn replace_modbus_registers(
        &self,
        device_id: &str,
        registers: &[ModbusRegisterInput],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let registers: Vec<(String, u16, bool, String, String, f64)> = registers
            .iter()
            .map(|r| {
                let (address, zero_based) = r.resolve_address().map_err(|e| anyhow::anyhow!(e))?;
                Ok((
                    r.name.clone(),
                    address,
                    zero_based,
                    r.register_type.clone(),
                    r.register_area.clone(),
                    r.scale,
                ))
            })
            .collect::<Result<_>>()?;

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute("DELETE FROM modbus_registers WHERE device_id = ?1", rusqlite::params![device_id])
                .context("Delete registers failed")?;
            for (i, (rname, addr, zero_based, rtype, rarea, scale)) in registers.iter().enumerate() {
                tx.execute(
                    "INSERT INTO modbus_registers (device_id, name, address, register_type, register_area, scale, sort_order, zero_based) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![device_id, rname, addr, rtype, rarea, scale, i as i64, zero_based],
                ).context("Insert register failed")?;
            }
            tx.commit().context("Commit failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_all_device_configs(&self) -> Result<Vec<DeviceConfig>> {
        let devices = self.list_modbus_devices().await?;
        let mut configs = Vec::new();
        for device in &devices {
            if !device.enabled {
                continue;
            }
            let registers = self.list_modbus_registers(&device.id).await?;
            configs.push(device.to_device_config(&registers));
        }
        Ok(configs)
    }
}
