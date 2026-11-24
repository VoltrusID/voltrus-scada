//! Links from Modbus devices to the device template they follow
//! (`modbus_devices.template_id`, `template_version`, `template_source`).

use std::collections::HashMap;

use anyhow::Context;

use super::Storage;
use crate::device_templates::DeviceTemplate;
use crate::device_templates::link::{RegisterRow, TemplateLink};

impl Storage {
    /// The template a Modbus device follows, if it is linked.
    pub async fn modbus_template_link(
        &self,
        device_id: &str,
    ) -> anyhow::Result<Option<TemplateLink>> {
        Ok(self.modbus_template_links().await?.remove(device_id))
    }

    /// Every linked Modbus device and its template.
    pub async fn modbus_template_links(&self) -> anyhow::Result<HashMap<String, TemplateLink>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, template_id, template_version, template_source FROM modbus_devices
                 WHERE template_id IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    TemplateLink {
                        template_id: r.get(1)?,
                        version: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        source: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    },
                ))
            })?;
            rows.collect::<Result<HashMap<_, _>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Ids and names of the Modbus devices that follow `link`, by id.
    pub async fn modbus_devices_linked_to(
        &self,
        link: &TemplateLink,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn.clone();
        let link = link.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name FROM modbus_devices
                 WHERE template_id = ?1 AND template_version = ?2 AND template_source = ?3
                 ORDER BY id",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![link.template_id, link.version, link.source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Link a device to a template, or unlink it (`None`).
    pub async fn set_modbus_template_link(
        &self,
        device_id: &str,
        link: Option<&TemplateLink>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = device_id.to_string();
        let link = link.cloned();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let (t, v, s) = match &link {
                Some(l) => (
                    Some(l.template_id.clone()),
                    Some(l.version.clone()),
                    Some(l.source.clone()),
                ),
                None => (None, None, None),
            };
            let n = conn.execute(
                "UPDATE modbus_devices SET template_id = ?2, template_version = ?3, template_source = ?4,
                 updated_at = ?5 WHERE id = ?1",
                rusqlite::params![id, t, v, s, crate::timeutil::now_secs()],
            )?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// The registers of a Modbus device, as a template would set them.
    pub async fn modbus_register_rows(&self, device_id: &str) -> anyhow::Result<Vec<RegisterRow>> {
        let conn = self.conn.clone();
        let id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT name, address, register_type, register_area, scale FROM modbus_registers
                 WHERE device_id = ?1 ORDER BY sort_order, id",
            )?;
            let rows = stmt.query_map(rusqlite::params![id], |r| {
                Ok(RegisterRow {
                    name: r.get(0)?,
                    offset: r.get(1)?,
                    register_type: r.get(2)?,
                    register_area: r.get(3)?,
                    scale: r.get(4)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Replace the stored body of a template (same id, version and source).
    /// False when there is no such stored template.
    pub async fn update_device_template(&self, t: &DeviceTemplate) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let body = serde_json::to_string(t).context("Serialize template failed")?;
        let t = t.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let n = conn.execute(
                "UPDATE device_templates SET manufacturer = ?4, model = ?5, verified = ?6, body = ?7
                 WHERE template_id = ?1 AND source = ?2 AND version = ?3",
                rusqlite::params![
                    t.template_id,
                    t.source,
                    t.version,
                    t.manufacturer,
                    t.model,
                    t.verified as i64,
                    body
                ],
            )?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }
}
