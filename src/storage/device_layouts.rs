use super::Storage;
use anyhow::{Context, Result};

use crate::models::{CanvasLayoutItem, ScreenLayoutRow, TwinConnection, TwinLayout};

impl Storage {
    // ── Screen Layouts ──

    pub async fn get_layouts(&self, screen_id: &str) -> Result<Vec<ScreenLayoutRow>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT screen_id, breakpoint, layout FROM screen_layouts WHERE screen_id = ?1",
                )
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![screen_id], |row| {
                    Ok(ScreenLayoutRow {
                        screen_id: row.get(0)?,
                        breakpoint: row.get(1)?,
                        layout: row.get(2)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_layouts(&self, screen_id: &str, layouts: &serde_json::Value) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        let layouts = layouts.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM screen_layouts WHERE screen_id = ?1",
                rusqlite::params![screen_id],
            )
            .context("Delete layouts failed")?;
            if let Some(arr) = layouts.as_array() {
                for item in arr {
                    let bp = item.get("breakpoint")
                        .and_then(|v| v.as_str())
                        .unwrap_or("lg");
                    let layout = item.get("layout")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "[]".to_string());
                    tx.execute(
                        "INSERT INTO screen_layouts (screen_id, breakpoint, layout) VALUES (?1, ?2, ?3)",
                        rusqlite::params![screen_id, bp, layout],
                    )
                    .context("Insert layout failed")?;
                }
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Twin Layouts ──

    pub async fn get_twin_layouts(&self, screen_id: &str) -> Result<Vec<TwinLayout>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Try with equipment_name column first, fall back to without
            let mut stmt = conn
                .prepare("SELECT screen_id, equipment_id, equipment_type, COALESCE(equipment_name, ''), position_x, position_z, rotation_y FROM twin_layouts WHERE screen_id = ?1")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![screen_id], |row| {
                    Ok(TwinLayout {
                        screen_id: row.get(0)?,
                        equipment_id: row.get(1)?,
                        equipment_type: row.get(2)?,
                        equipment_name: row.get(3)?,
                        position_x: row.get(4)?,
                        position_z: row.get(5)?,
                        rotation_y: row.get(6)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_twin_layouts(&self, screen_id: &str, layouts: &[TwinLayout]) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        let layouts = layouts.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM twin_layouts WHERE screen_id = ?1",
                rusqlite::params![screen_id],
            )
            .context("Delete twin layouts failed")?;
            for l in &layouts {
                tx.execute(
                    "INSERT INTO twin_layouts (screen_id, equipment_id, equipment_type, equipment_name, position_x, position_z, rotation_y)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![l.screen_id, l.equipment_id, l.equipment_type, l.equipment_name,
                                      l.position_x, l.position_z, l.rotation_y],
                )
                .context("Insert twin layout failed")?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Twin Connections ──

    pub async fn get_twin_connections(&self, screen_id: &str) -> Result<Vec<TwinConnection>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT screen_id, from_id, to_id FROM twin_connections WHERE screen_id = ?1",
                )
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![screen_id], |row| {
                    Ok(TwinConnection {
                        screen_id: row.get(0)?,
                        from_id: row.get(1)?,
                        to_id: row.get(2)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_twin_connections(
        &self,
        screen_id: &str,
        connections: &[TwinConnection],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        let connections = connections.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM twin_connections WHERE screen_id = ?1",
                rusqlite::params![screen_id],
            )
            .context("Delete twin connections failed")?;
            for c in &connections {
                tx.execute(
                    "INSERT INTO twin_connections (screen_id, from_id, to_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![c.screen_id, c.from_id, c.to_id],
                )
                .context("Insert twin connection failed")?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Canvas Layouts ──

    pub async fn get_canvas_layouts(&self, screen_id: &str) -> Result<Vec<CanvasLayoutItem>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT screen_id, data_source_id, x, y, width, height FROM canvas_layouts WHERE screen_id = ?1")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![screen_id], |row| {
                    Ok(CanvasLayoutItem {
                        screen_id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        x: row.get(2)?,
                        y: row.get(3)?,
                        width: row.get(4)?,
                        height: row.get(5)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_canvas_layouts(
        &self,
        screen_id: &str,
        layouts: &[CanvasLayoutItem],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        let layouts = layouts.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM canvas_layouts WHERE screen_id = ?1",
                rusqlite::params![screen_id],
            )
            .context("Delete canvas layouts failed")?;
            for l in &layouts {
                tx.execute(
                    "INSERT INTO canvas_layouts (screen_id, data_source_id, x, y, width, height)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![l.screen_id, l.data_source_id, l.x, l.y, l.width, l.height],
                )
                .context("Insert canvas layout failed")?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }
}
