use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    // ── Recipe CRUD methods ──

    pub async fn create_recipe(
        &self,
        name: &str,
        category: Option<&str>,
        device_id: &str,
        values: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let category = category.map(|s| s.to_string());
        let device_id = device_id.to_string();
        let values = values.to_string();
        let description = description.map(|s| s.to_string());
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO recipes (name, category, device_id, \"values\", version, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)",
                rusqlite::params![name, category, device_id, values, description, now, now],
            ).context("Insert recipe failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn get_recipe(&self, id: i64) -> Result<Option<crate::models::Recipe>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, category, device_id, \"values\", version, description, created_at, updated_at
                 FROM recipes WHERE id = ?1"
            ).context("Prepare get_recipe failed")?;
            let mut rows = stmt.query_map(rusqlite::params![id], |row| {
                Ok(crate::models::Recipe {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    category: row.get(2)?,
                    device_id: row.get(3)?,
                    values: row.get(4)?,
                    version: row.get(5)?,
                    description: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            }).context("Query get_recipe failed")?;
            match rows.next() {
                Some(row) => Ok(Some(row.context("Row parse get_recipe failed")?)),
                None => Ok(None),
            }
        }).await.context("Task join failed")?
    }

    pub async fn list_recipes(
        &self,
        device_id: Option<&str>,
    ) -> Result<Vec<crate::models::Recipe>> {
        let conn = self.conn.clone();
        let device_id_owned = device_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let recipes = if let Some(ref did) = device_id_owned {
                let mut stmt = conn.prepare(
                    "SELECT id, name, category, device_id, \"values\", version, description, created_at, updated_at
                     FROM recipes WHERE device_id = ?1 ORDER BY updated_at DESC"
                ).context("Prepare list_recipes failed")?;
                let rows = stmt.query_map(rusqlite::params![did], |row| {
                    Ok(crate::models::Recipe {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        category: row.get(2)?,
                        device_id: row.get(3)?,
                        values: row.get(4)?,
                        version: row.get(5)?,
                        description: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                }).context("Query list_recipes failed")?;
                rows.collect::<Result<Vec<_>, _>>().context("Row parse list_recipes failed")?
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, name, category, device_id, \"values\", version, description, created_at, updated_at
                     FROM recipes ORDER BY updated_at DESC"
                ).context("Prepare list_recipes failed")?;
                let rows = stmt.query_map([], |row| {
                    Ok(crate::models::Recipe {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        category: row.get(2)?,
                        device_id: row.get(3)?,
                        values: row.get(4)?,
                        version: row.get(5)?,
                        description: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                }).context("Query list_recipes failed")?;
                rows.collect::<Result<Vec<_>, _>>().context("Row parse list_recipes failed")?
            };
            Ok(recipes)
        }).await.context("Task join failed")?
    }

    pub async fn update_recipe(
        &self,
        id: i64,
        name: Option<&str>,
        category: Option<Option<&str>>,
        values: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<bool> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let category = category.map(|c| c.map(|s| s.to_string()));
        let values = values.map(|s| s.to_string());
        let description = description.map(|d| d.map(|s| s.to_string()));
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let existing: Option<String> = conn
                .query_row(
                    "SELECT \"values\" FROM recipes WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .ok();
            match existing {
                None => Ok(false),
                Some(_existing_values) => {
                    let values_to_save = values.unwrap_or(_existing_values);
                    let mut stmt = conn
                        .prepare(
                            "UPDATE recipes SET name = COALESCE(?1, name),
                         category = COALESCE(?2, category),
                         \"values\" = ?3,
                         description = COALESCE(?4, description),
                         version = version + 1,
                         updated_at = ?5
                         WHERE id = ?6",
                        )
                        .context("Prepare update_recipe failed")?;
                    let affected = stmt
                        .execute(rusqlite::params![
                            name,
                            category,
                            values_to_save,
                            description,
                            now,
                            id
                        ])
                        .context("Execute update_recipe failed")?;
                    Ok(affected > 0)
                }
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_recipe(&self, id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let affected = conn
                .execute("DELETE FROM recipes WHERE id = ?1", rusqlite::params![id])
                .context("Delete recipe failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// Save the current device metric values from cache as a recipe.
    pub async fn save_current_as_recipe(
        &self,
        device_id: &str,
        name: &str,
        category: Option<&str>,
        description: Option<&str>,
        cache: &crate::cache::DeviceCache,
    ) -> Result<i64> {
        let snapshots = cache.get_all().await;
        let metrics = snapshots
            .iter()
            .find(|s| s.device_id == device_id)
            .map(|s| s.metrics.clone())
            .unwrap_or_default();

        let values_json =
            serde_json::to_string(&metrics).context("Failed to serialize current metric values")?;

        self.create_recipe(name, category, device_id, &values_json, description)
            .await
    }
}
