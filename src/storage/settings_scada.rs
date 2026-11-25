use super::Storage;
use anyhow::{Context, Result};

use crate::models::{ScadaConnection, ScadaEquipment};

impl Storage {
    // ── SCADA 2D Equipment ──

    pub async fn get_scada_equipment(&self, screen_id: &str) -> Result<Vec<ScadaEquipment>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            read_equipment(&conn, &screen_id)
        })
        .await
        .context("Task join failed")?
    }

    /// Upsert a single SCADA equipment item (INSERT OR REPLACE).
    /// Used by FEED instantiator to add equipment without wiping existing items.
    pub async fn upsert_scada_equipment(&self, item: &ScadaEquipment) -> Result<()> {
        let conn = self.conn.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO scada_equipment (screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![item.screen_id, item.equipment_id, item.equipment_type, item.equipment_name,
                                  item.x, item.y, item.width, item.height, item.rotation, item.sensor_bindings, item.config],
            )
            .context("Upsert scada equipment failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── SCADA 2D Connections ──

    pub async fn get_scada_connections(&self, screen_id: &str) -> Result<Vec<ScadaConnection>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            read_connections(&conn, &screen_id)
        })
        .await
        .context("Task join failed")?
    }

    /// Upsert a single SCADA connection (INSERT OR REPLACE).
    /// Used by FEED instantiator to add connections without wiping existing ones.
    pub async fn upsert_scada_connection(&self, c: &ScadaConnection) -> Result<()> {
        let conn = self.conn.clone();
        let c = c.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO scada_connections (screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![c.screen_id, c.connection_id, c.from_equipment, c.to_equipment,
                                  c.waypoints, c.connection_type],
            )
            .context("Upsert scada connection failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// The revision of a screen's 2D layout: sha256 over its equipment and
    /// connection rows in id order. Clients send it back as `base_revision`
    /// so a save over someone else's newer save is refused instead of lost.
    pub async fn scada_layout_revision(&self, screen_id: &str) -> Result<String> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            layout_revision(&conn, &screen_id)
        })
        .await
        .context("Task join failed")?
    }

    /// Equipment, connections and revision of a screen, read under one lock.
    pub async fn get_scada_layout(
        &self,
        screen_id: &str,
    ) -> Result<(Vec<ScadaEquipment>, Vec<ScadaConnection>, String)> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let eq = read_equipment(&conn, &screen_id)?;
            let cn = read_connections(&conn, &screen_id)?;
            let rev = revision_of(&eq, &cn);
            Ok((eq, cn, rev))
        })
        .await
        .context("Task join failed")?
    }

    /// Replace a screen's equipment and/or connections (`None` leaves that
    /// table alone) in one transaction. With `base_revision`, nothing is
    /// written when the screen's current revision differs.
    pub async fn save_scada_layout(
        &self,
        screen_id: &str,
        equipment: Option<Vec<ScadaEquipment>>,
        connections: Option<Vec<ScadaConnection>>,
        base_revision: Option<String>,
    ) -> Result<LayoutSave> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            if let Some(base) = base_revision {
                let current = layout_revision(&tx, &screen_id)?;
                if current != base {
                    return Ok(LayoutSave::Conflict { revision: current });
                }
            }
            if let Some(items) = equipment {
                tx.execute(
                    "DELETE FROM scada_equipment WHERE screen_id = ?1",
                    rusqlite::params![screen_id],
                )
                .context("Delete scada equipment failed")?;
                for item in &items {
                    tx.execute(
                        "INSERT INTO scada_equipment (screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        rusqlite::params![screen_id, item.equipment_id, item.equipment_type, item.equipment_name,
                                          item.x, item.y, item.width, item.height, item.rotation, item.sensor_bindings, item.config],
                    )
                    .context("Insert scada equipment failed")?;
                }
            }
            if let Some(items) = connections {
                tx.execute(
                    "DELETE FROM scada_connections WHERE screen_id = ?1",
                    rusqlite::params![screen_id],
                )
                .context("Delete scada connections failed")?;
                for c in &items {
                    tx.execute(
                        "INSERT INTO scada_connections (screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![screen_id, c.connection_id, c.from_equipment, c.to_equipment,
                                          c.waypoints, c.connection_type],
                    )
                    .context("Insert scada connection failed")?;
                }
            }
            let revision = layout_revision(&tx, &screen_id)?;
            tx.commit().context("Commit failed")?;
            Ok(LayoutSave::Saved { revision })
        })
        .await
        .context("Task join failed")?
    }
}

/// Outcome of [`Storage::save_scada_layout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutSave {
    Saved {
        revision: String,
    },
    /// The screen changed since `base_revision`; nothing was written.
    Conflict {
        revision: String,
    },
}

fn read_equipment(conn: &rusqlite::Connection, screen_id: &str) -> Result<Vec<ScadaEquipment>> {
    let mut stmt = conn
        .prepare("SELECT screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config FROM scada_equipment WHERE screen_id = ?1 ORDER BY equipment_id")
        .context("Prepare failed")?;
    let rows = stmt
        .query_map(rusqlite::params![screen_id], |row| {
            Ok(ScadaEquipment {
                screen_id: row.get(0)?,
                equipment_id: row.get(1)?,
                equipment_type: row.get(2)?,
                equipment_name: row.get(3)?,
                x: row.get(4)?,
                y: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
                rotation: row.get(8)?,
                sensor_bindings: row.get(9)?,
                config: row.get(10)?,
            })
        })
        .context("Query failed")?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("Row mapping failed")
}

fn read_connections(conn: &rusqlite::Connection, screen_id: &str) -> Result<Vec<ScadaConnection>> {
    let mut stmt = conn
        .prepare("SELECT screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type FROM scada_connections WHERE screen_id = ?1 ORDER BY connection_id")
        .context("Prepare failed")?;
    let rows = stmt
        .query_map(rusqlite::params![screen_id], |row| {
            Ok(ScadaConnection {
                screen_id: row.get(0)?,
                connection_id: row.get(1)?,
                from_equipment: row.get(2)?,
                to_equipment: row.get(3)?,
                waypoints: row.get(4)?,
                connection_type: row.get(5)?,
            })
        })
        .context("Query failed")?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("Row mapping failed")
}

fn layout_revision(conn: &rusqlite::Connection, screen_id: &str) -> Result<String> {
    let eq = read_equipment(conn, screen_id)?;
    let cn = read_connections(conn, screen_id)?;
    Ok(revision_of(&eq, &cn))
}

/// sha256 over the rows (already in id order), lowercase hex.
fn revision_of(eq: &[ScadaEquipment], cn: &[ScadaConnection]) -> String {
    use sha2::{Digest, Sha256};
    let body = serde_json::to_vec(&(eq, cn)).unwrap_or_default();
    Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
