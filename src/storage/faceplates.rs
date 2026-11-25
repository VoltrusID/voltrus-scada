//! Faceplate definitions (`faceplates`, `faceplate_roles`).

use anyhow::Context;

use super::Storage;
use super::design_history::{self, DesignActor};
use crate::faceplate::{Faceplate, FaceplateRole};

pub(super) fn load(
    conn: &rusqlite::Connection,
    id: Option<&str>,
) -> anyhow::Result<Vec<Faceplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, description FROM faceplates WHERE ?1 IS NULL OR id = ?1 ORDER BY name, id",
    )?;
    let heads = stmt
        .query_map(rusqlite::params![id], |r| {
            Ok(Faceplate {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                description: r.get(3)?,
                roles: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut roles = conn.prepare(
        "SELECT role, label, kind, required FROM faceplate_roles WHERE faceplate_id = ?1 ORDER BY sort_order, id",
    )?;
    let mut out = Vec::with_capacity(heads.len());
    for mut f in heads {
        f.roles = roles
            .query_map(rusqlite::params![f.id], |r| {
                Ok(FaceplateRole {
                    role: r.get(0)?,
                    label: r.get(1)?,
                    kind: r.get(2)?,
                    required: r.get::<_, i64>(3)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        out.push(f);
    }
    Ok(out)
}

impl Storage {
    pub async fn list_faceplates(&self) -> anyhow::Result<Vec<Faceplate>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || load(&conn.lock().unwrap(), None))
            .await
            .context("Task join failed")?
    }

    pub async fn get_faceplate(&self, id: &str) -> anyhow::Result<Option<Faceplate>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            Ok(load(&conn.lock().unwrap(), Some(&id))?.into_iter().next())
        })
        .await
        .context("Task join failed")?
    }

    /// Insert or replace a definition with its roles. `create` refuses an id
    /// that exists (false), an update refuses one that does not (false).
    pub async fn save_faceplate(&self, f: &Faceplate, create: bool) -> anyhow::Result<bool> {
        self.save_faceplate_as(f, create, &DesignActor::system("faceplate"))
            .await
    }

    /// [`Self::save_faceplate`], logged in the design history as `actor`.
    pub async fn save_faceplate_as(
        &self,
        f: &Faceplate,
        create: bool,
        actor: &DesignActor,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let f = f.clone();
        let actor = actor.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction()?;
            let before = design_history::read_project_objects(&tx, design_history::FACEPLATE)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM faceplates WHERE id = ?1)",
                [&f.id],
                |r| r.get(0),
            )?;
            if exists == create {
                return Ok(false);
            }
            let now = crate::timeutil::now_secs();
            if create {
                tx.execute(
                    "INSERT INTO faceplates (id, name, kind, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    rusqlite::params![f.id, f.name, f.kind, f.description, now],
                )?;
            } else {
                tx.execute(
                    "UPDATE faceplates SET name = ?2, kind = ?3, description = ?4, updated_at = ?5 WHERE id = ?1",
                    rusqlite::params![f.id, f.name, f.kind, f.description, now],
                )?;
                tx.execute("DELETE FROM faceplate_roles WHERE faceplate_id = ?1", [&f.id])?;
            }
            for (i, r) in f.roles.iter().enumerate() {
                tx.execute(
                    "INSERT INTO faceplate_roles (faceplate_id, role, label, kind, required, sort_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![f.id, r.role, r.label, r.kind, r.required as i64, i as i64],
                )?;
            }
            let after = design_history::read_project_objects(&tx, design_history::FACEPLATE)?;
            design_history::append(&tx, design_history::diff_objects(&before, &after), &actor)?;
            tx.commit()?;
            Ok(true)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_faceplate(&self, id: &str) -> anyhow::Result<bool> {
        self.delete_faceplate_as(id, &DesignActor::system("faceplate"))
            .await
    }

    /// [`Self::delete_faceplate`], logged in the design history as `actor`.
    pub async fn delete_faceplate_as(&self, id: &str, actor: &DesignActor) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let actor = actor.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction()?;
            let before = design_history::read_project_objects(&tx, design_history::FACEPLATE)?;
            tx.execute("DELETE FROM faceplate_roles WHERE faceplate_id = ?1", [&id])?;
            let n = tx.execute("DELETE FROM faceplates WHERE id = ?1", [&id])?;
            let after = design_history::read_project_objects(&tx, design_history::FACEPLATE)?;
            design_history::append(&tx, design_history::diff_objects(&before, &after), &actor)?;
            tx.commit()?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// Screens whose SCADA symbols open a faceplate, as (screen id, symbol
    /// id): in the draft or in what operators see (the published copy), so a
    /// faceplate a published screen still opens is never deleted under it.
    pub async fn faceplate_users(&self, id: &str) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT screen_id, equipment_id FROM scada_equipment
                 WHERE json_valid(config) AND json_extract(config, '$.faceplate') = ?1
                 UNION
                 SELECT screen_id, equipment_id FROM scada_equipment_published
                 WHERE json_valid(config) AND json_extract(config, '$.faceplate') = ?1
                 ORDER BY 1, 2",
            )?;
            let rows = stmt.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
