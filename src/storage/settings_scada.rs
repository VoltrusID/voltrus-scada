use super::Storage;
use anyhow::{Context, Result};

use super::design_history::{self, DesignActor};
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
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            let before = design_history::read_objects(&tx, design_history::Scope::Screen(&item.screen_id))?;
            tx.execute(
                "INSERT OR REPLACE INTO scada_equipment (screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![item.screen_id, item.equipment_id, item.equipment_type, item.equipment_name,
                                  item.x, item.y, item.width, item.height, item.rotation, item.sensor_bindings, item.config],
            )
            .context("Upsert scada equipment failed")?;
            let after = design_history::read_objects(&tx, design_history::Scope::Screen(&item.screen_id))?;
            design_history::append(&tx, design_history::diff_objects(&before, &after), &DesignActor::system("wizard"))?;
            tx.commit().context("Commit failed")?;
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
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            let before = design_history::read_objects(&tx, design_history::Scope::Screen(&c.screen_id))?;
            tx.execute(
                "INSERT OR REPLACE INTO scada_connections (screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![c.screen_id, c.connection_id, c.from_equipment, c.to_equipment,
                                  c.waypoints, c.connection_type],
            )
            .context("Upsert scada connection failed")?;
            let after = design_history::read_objects(&tx, design_history::Scope::Screen(&c.screen_id))?;
            design_history::append(&tx, design_history::diff_objects(&before, &after), &DesignActor::system("wizard"))?;
            tx.commit().context("Commit failed")?;
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

    /// The rows a reader sees: the draft, or with `published` the published
    /// copy when the screen has one and publishing is on (else the draft).
    pub async fn get_scada_layout_view(
        &self,
        screen_id: &str,
        published: bool,
    ) -> Result<LayoutView> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            layout_view(&conn, &screen_id, published)
        })
        .await
        .context("Task join failed")?
    }

    /// Replace a screen's equipment and/or connections (`None` leaves that
    /// table alone) in one transaction. With `base_revision` (the rows' hash)
    /// or `base_seq` (the screen's change-log head), nothing is written when
    /// the screen changed since. Symbols naming an equipment instance are
    /// bound to it in the same transaction (docs/equipment-types.md). Every
    /// added, changed or removed object is appended to the change log as one
    /// batch by `actor` (docs/version-history.md).
    pub async fn save_scada_layout(
        &self,
        screen_id: &str,
        equipment: Option<Vec<ScadaEquipment>>,
        connections: Option<Vec<ScadaConnection>>,
        base: LayoutBase,
        actor: &DesignActor,
    ) -> Result<LayoutSave> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        let actor = actor.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            if let Some(conflict) = check_base(&tx, &screen_id, &base)? {
                return Ok(conflict);
            }
            let batch = write_layout_rows(&tx, &screen_id, equipment, connections, &actor)?;
            let revision = layout_revision(&tx, &screen_id)?;
            let seq = design_history::head_seq(&tx, &screen_id)?;
            tx.commit().context("Commit failed")?;
            Ok(LayoutSave::Saved {
                revision,
                seq,
                batch: batch.map(|b| b.batch),
            })
        })
        .await
        .context("Task join failed")?
    }
}

/// A screen's rows as one reader sees them.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LayoutView {
    pub equipment: Vec<ScadaEquipment>,
    pub connections: Vec<ScadaConnection>,
    /// Hash of the draft rows (phase 1's optimistic lock).
    pub revision: String,
    /// Change-log head of the screen (phase 3's optimistic lock).
    pub seq: i64,
    /// `draft` or `published`: which rows `equipment`/`connections` are.
    pub view: &'static str,
}

pub(crate) fn layout_view(
    conn: &rusqlite::Connection,
    screen_id: &str,
    published: bool,
) -> Result<LayoutView> {
    let revision = layout_revision(conn, screen_id)?;
    let seq = design_history::head_seq(conn, screen_id)?;
    if published && let Some(p) = design_history::read_published(conn, screen_id)? {
        return Ok(LayoutView {
            equipment: p.equipment,
            connections: p.connections,
            revision,
            seq,
            view: "published",
        });
    }
    Ok(LayoutView {
        equipment: read_equipment(conn, screen_id)?,
        connections: read_connections(conn, screen_id)?,
        revision,
        seq,
        view: "draft",
    })
}

/// What a save is based on: the revision (rows' hash) and/or the change-log
/// head the writer loaded. Both absent: overwrite, as before phase 3.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutBase {
    pub revision: Option<String>,
    pub seq: Option<i64>,
}

/// `Some(Conflict)` when the screen moved on from `base`.
pub(crate) fn check_base(
    conn: &rusqlite::Connection,
    screen_id: &str,
    base: &LayoutBase,
) -> Result<Option<LayoutSave>> {
    if base.revision.is_none() && base.seq.is_none() {
        return Ok(None);
    }
    let revision = layout_revision(conn, screen_id)?;
    let seq = design_history::head_seq(conn, screen_id)?;
    let stale = base.revision.as_ref().is_some_and(|r| *r != revision)
        || base.seq.is_some_and(|s| s != seq);
    if !stale {
        return Ok(None);
    }
    // Without a base seq (a revision-only client) the last changes stand in.
    let from = match base.seq {
        Some(s) => s.min(seq),
        None => design_history::recent_floor(conn, screen_id, 50)?,
    };
    let since = design_history::batches_since(conn, screen_id, from)?;
    Ok(Some(LayoutSave::Conflict {
        revision,
        seq,
        since,
    }))
}

/// Replace a screen's rows inside the caller's transaction (binding instance
/// symbols first) and log the difference as one batch.
pub(crate) fn write_layout_rows(
    tx: &rusqlite::Connection,
    screen_id: &str,
    equipment: Option<Vec<ScadaEquipment>>,
    connections: Option<Vec<ScadaConnection>>,
    actor: &DesignActor,
) -> Result<Option<design_history::BatchInfo>> {
    let before = design_history::read_objects(tx, design_history::Scope::Screen(screen_id))?;
    if let Some(mut items) = equipment {
        super::equipment_types::bind_rows(tx, &mut items)?;
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
    let after = design_history::read_objects(tx, design_history::Scope::Screen(screen_id))?;
    design_history::append(tx, design_history::diff_objects(&before, &after), actor)
}

/// Outcome of [`Storage::save_scada_layout`].
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutSave {
    Saved {
        revision: String,
        /// The screen's change-log head after the save.
        seq: i64,
        /// The change-log batch, `None` when nothing changed.
        batch: Option<String>,
    },
    /// The screen changed since the base; nothing was written. `since` lists
    /// the batches after the base's `seq` (who changed what).
    Conflict {
        revision: String,
        seq: i64,
        since: Vec<serde_json::Value>,
    },
}

pub(crate) fn read_equipment(
    conn: &rusqlite::Connection,
    screen_id: &str,
) -> Result<Vec<ScadaEquipment>> {
    read_equipment_from(conn, "scada_equipment", Some(screen_id))
}

/// Rows of `table` (`scada_equipment` or `scada_equipment_published`), of
/// one screen or all, in (screen, id) order.
pub(crate) fn read_equipment_from(
    conn: &rusqlite::Connection,
    table: &str,
    screen_id: Option<&str>,
) -> Result<Vec<ScadaEquipment>> {
    let mut stmt = conn
        .prepare(&format!("SELECT screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config FROM {table} WHERE ?1 IS NULL OR screen_id = ?1 ORDER BY screen_id, equipment_id"))
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

pub(crate) fn read_connections(
    conn: &rusqlite::Connection,
    screen_id: &str,
) -> Result<Vec<ScadaConnection>> {
    read_connections_from(conn, "scada_connections", Some(screen_id))
}

/// Rows of `table` (`scada_connections` or `scada_connections_published`).
pub(crate) fn read_connections_from(
    conn: &rusqlite::Connection,
    table: &str,
    screen_id: Option<&str>,
) -> Result<Vec<ScadaConnection>> {
    let mut stmt = conn
        .prepare(&format!("SELECT screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type FROM {table} WHERE ?1 IS NULL OR screen_id = ?1 ORDER BY screen_id, connection_id"))
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

pub(crate) fn layout_revision(conn: &rusqlite::Connection, screen_id: &str) -> Result<String> {
    let eq = read_equipment(conn, screen_id)?;
    let cn = read_connections(conn, screen_id)?;
    Ok(revision_of(&eq, &cn))
}

/// sha256 over the rows (already in id order), lowercase hex.
pub(crate) fn revision_of(eq: &[ScadaEquipment], cn: &[ScadaConnection]) -> String {
    use sha2::{Digest, Sha256};
    let body = serde_json::to_vec(&(eq, cn)).unwrap_or_default();
    Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
