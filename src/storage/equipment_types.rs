//! Equipment types and instances (`equipment_types`, `equipment_instances`).
//!
//! Writes go through the project import engine (see `api/equipment_types.rs`)
//! so tags, registers, flows and widget bindings change in one transaction;
//! this module reads, deletes and binds saved SCADA rows to instances.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::Storage;
use crate::equipment_types::resolve::{self, ModbusDevice, World};
use crate::equipment_types::{EquipmentInstance, EquipmentType};
use crate::models::ScadaEquipment;

fn json_col(text: Option<String>, default: Value) -> Value {
    text.and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(default)
}

pub fn read_types(conn: &Connection, id: Option<&str>) -> Result<Vec<EquipmentType>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, category, symbol, faceplate, tags, bindings, symbol_config, flows
         FROM equipment_types WHERE ?1 IS NULL OR id = ?1 ORDER BY category, name, id",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![id], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "name": r.get::<_, String>(1)?,
                "description": r.get::<_, String>(2)?,
                "category": r.get::<_, String>(3)?,
                "symbol": r.get::<_, String>(4)?,
                "faceplate": r.get::<_, Option<String>>(5)?,
                "tags": json_col(r.get(6)?, json!([])),
                "bindings": json_col(r.get(7)?, json!({})),
                "symbol_config": json_col(r.get(8)?, json!({})),
                "flows": json_col(r.get(9)?, json!([])),
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for v in rows {
        let id = v["id"].as_str().unwrap_or_default().to_string();
        match serde_json::from_value::<EquipmentType>(v) {
            Ok(t) => out.push(t),
            Err(e) => tracing::warn!("equipment type '{id}' is not readable: {e}"),
        }
    }
    Ok(out)
}

pub fn read_instances(
    conn: &Connection,
    name: Option<&str>,
    type_id: Option<&str>,
) -> Result<Vec<EquipmentInstance>> {
    let mut stmt = conn.prepare(
        "SELECT name, type_id, device_id, base, prefix, description, symbol, overrides
         FROM equipment_instances
         WHERE (?1 IS NULL OR name = ?1) AND (?2 IS NULL OR type_id = ?2)
         ORDER BY name",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![name, type_id], |r| {
            Ok(json!({
                "name": r.get::<_, String>(0)?,
                "type": r.get::<_, String>(1)?,
                "device": r.get::<_, String>(2)?,
                "base": r.get::<_, Option<i64>>(3)?,
                "prefix": r.get::<_, Option<String>>(4)?,
                "description": r.get::<_, String>(5)?,
                "symbol": r.get::<_, Option<String>>(6)?,
                "overrides": json_col(r.get(7)?, json!({})),
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for v in rows {
        let name = v["name"].as_str().unwrap_or_default().to_string();
        match serde_json::from_value::<EquipmentInstance>(v) {
            Ok(i) => out.push(i),
            Err(e) => tracing::warn!("equipment instance '{name}' is not readable: {e}"),
        }
    }
    Ok(out)
}

/// Tags (by integer id), devices and Modbus registers as they are stored.
pub fn read_world(conn: &Connection) -> Result<World> {
    let mut w = World::default();
    {
        let mut stmt = conn.prepare("SELECT id, device_id, key FROM data_sources ORDER BY id")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let (id, device, key): (i64, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
            w.devices.insert(device.clone());
            w.tags.entry((device, key)).or_insert(Value::from(id));
        }
    }
    {
        let mut stmt = conn.prepare("SELECT id, template_id FROM modbus_devices")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let template: Option<String> = r.get(1)?;
            w.devices.insert(id.clone());
            w.modbus.insert(
                id,
                ModbusDevice {
                    registers: Default::default(),
                    linked: template.is_some_and(|t| !t.is_empty()),
                },
            );
        }
        let mut stmt = conn.prepare("SELECT device_id, name FROM modbus_registers")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let (device, name): (String, String) = (r.get(0)?, r.get(1)?);
            if let Some(m) = w.modbus.get_mut(&device) {
                m.registers.insert(name);
            }
        }
    }
    for table in [
        "opcua_devices",
        "s7_devices",
        "enip_devices",
        "dnp3_devices",
        "bacnet_devices",
        "voltrus_plc_devices",
    ] {
        // Optional protocols may have no table in an old database.
        let Ok(mut stmt) = conn.prepare(&format!("SELECT name FROM {table}")) else {
            continue;
        };
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        w.devices.extend(names);
    }
    Ok(w)
}

/// Bind SCADA rows whose `config.equipment` names an instance to its tags.
/// Rows that name no instance, or are bound already, are returned as they
/// are (byte for byte), so saving a materialised screen changes nothing.
pub fn bind_rows(conn: &Connection, rows: &mut [ScadaEquipment]) -> Result<()> {
    let names: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            serde_json::from_str::<Value>(&r.config)
                .ok()?
                .get("equipment")?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    if names.is_empty() {
        return Ok(());
    }
    let types: HashMap<String, EquipmentType> = read_types(conn, None)?
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();
    let instances: HashMap<String, EquipmentInstance> = read_instances(conn, None, None)?
        .into_iter()
        .map(|i| (i.name.clone(), i))
        .collect();
    let world = read_world(conn)?;
    let mut patches: HashMap<String, Option<resolve::WidgetPatch>> = HashMap::new();
    for row in rows.iter_mut() {
        let Ok(Value::Object(mut config)) = serde_json::from_str::<Value>(&row.config) else {
            continue;
        };
        let Some(name) = config
            .get("equipment")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let patch = patches.entry(name.clone()).or_insert_with(|| {
            let inst = instances.get(&name)?;
            let t = types.get(&inst.type_id)?;
            let res = resolve::resolve(t, inst, &world);
            Some(resolve::widget_patch(t, inst, &res))
        });
        let Some(patch) = patch else {
            tracing::warn!(
                "SCADA widget '{}' on screen '{}' names equipment instance '{name}', which does not exist; saved as it is",
                row.equipment_id,
                row.screen_id
            );
            continue;
        };
        let mut sensors: Vec<Value> =
            serde_json::from_str(&row.sensor_bindings).unwrap_or_default();
        let mut kind = row.equipment_type.clone();
        if resolve::apply_patch(patch, &mut kind, &mut sensors, &mut config) {
            row.equipment_type = kind;
            row.sensor_bindings = serde_json::to_string(&sensors)?;
            row.config = serde_json::to_string(&Value::Object(config))?;
        }
    }
    Ok(())
}

/// Outcome of deleting a type.
pub enum TypeDelete {
    Deleted,
    NotFound,
    /// Instances of the type, which keep it.
    InUse(Vec<String>),
}

impl Storage {
    pub async fn list_equipment_types(&self) -> Result<Vec<(EquipmentType, i64)>> {
        self.with_conn(|conn| {
            let types = read_types(conn, None)?;
            let mut stmt =
                conn.prepare("SELECT type_id, COUNT(*) FROM equipment_instances GROUP BY type_id")?;
            let counts: HashMap<String, i64> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            Ok(types
                .into_iter()
                .map(|t| {
                    let n = counts.get(&t.id).copied().unwrap_or(0);
                    (t, n)
                })
                .collect())
        })
        .await
    }

    pub async fn get_equipment_type(&self, id: &str) -> Result<Option<EquipmentType>> {
        let id = id.to_string();
        self.with_conn(move |conn| Ok(read_types(conn, Some(&id))?.into_iter().next()))
            .await
    }

    pub async fn list_equipment_instances(
        &self,
        type_id: Option<&str>,
    ) -> Result<Vec<EquipmentInstance>> {
        let type_id = type_id.map(str::to_string);
        self.with_conn(move |conn| read_instances(conn, None, type_id.as_deref()))
            .await
    }

    pub async fn get_equipment_instance(&self, name: &str) -> Result<Option<EquipmentInstance>> {
        let name = name.to_string();
        self.with_conn(move |conn| Ok(read_instances(conn, Some(&name), None)?.into_iter().next()))
            .await
    }

    /// Types, instances and the world, read under one lock.
    pub async fn equipment_state(
        &self,
    ) -> Result<(Vec<EquipmentType>, Vec<EquipmentInstance>, World)> {
        self.with_conn(|conn| {
            Ok((
                read_types(conn, None)?,
                read_instances(conn, None, None)?,
                read_world(conn)?,
            ))
        })
        .await
    }

    pub async fn delete_equipment_type(&self, id: &str) -> Result<TypeDelete> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let users: Vec<String> = conn
                .prepare("SELECT name FROM equipment_instances WHERE type_id = ?1 ORDER BY name")?
                .query_map([&id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            if !users.is_empty() {
                return Ok(TypeDelete::InUse(users));
            }
            let n = conn
                .execute("DELETE FROM equipment_types WHERE id = ?1", [&id])
                .context("delete equipment type")?;
            Ok(if n == 0 {
                TypeDelete::NotFound
            } else {
                TypeDelete::Deleted
            })
        })
        .await
    }

    pub async fn delete_equipment_instance(&self, name: &str) -> Result<bool> {
        let name = name.to_string();
        self.with_conn(move |conn| {
            Ok(conn
                .execute("DELETE FROM equipment_instances WHERE name = ?1", [&name])
                .context("delete equipment instance")?
                > 0)
        })
        .await
    }

    /// See [`bind_rows`].
    pub async fn bind_equipment_rows(
        &self,
        mut rows: Vec<ScadaEquipment>,
    ) -> Result<Vec<ScadaEquipment>> {
        self.with_conn(move |conn| {
            bind_rows(conn, &mut rows)?;
            Ok(rows)
        })
        .await
    }
}
