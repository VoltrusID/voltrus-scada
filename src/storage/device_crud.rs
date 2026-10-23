use super::Storage;
use anyhow::{Context, Result};

use crate::models::{DataSource, Device, ScreenDataSource};

impl Storage {
    // ── Devices ──

    pub async fn max_device_sort_order(&self) -> Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let max: Option<i64> = conn
                .query_row("SELECT MAX(sort_order) FROM devices", [], |row| row.get(0))
                .ok();
            Ok(max.unwrap_or(-1))
        })
        .await
        .context("Task join failed")?
    }

    pub async fn device_exists(&self, id: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM devices WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .context("Check device exists failed")?;
            Ok(count > 0)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_devices(&self) -> Result<Vec<Device>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, type, sort_order FROM devices ORDER BY sort_order")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Device {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        device_type: row.get(2)?,
                        sort_order: row.get(3)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_device(&self, id: &str, name: &str, sort_order: i64) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO devices (id, name, sort_order) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, name, sort_order],
            )
            .context("Insert device failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_device(
        &self,
        id: &str,
        name: Option<&str>,
        sort_order: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(ref name) = name {
                conn.execute(
                    "UPDATE devices SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update device name failed")?;
            }
            if let Some(sort_order) = sort_order {
                conn.execute(
                    "UPDATE devices SET sort_order = ?1 WHERE id = ?2",
                    rusqlite::params![sort_order, id],
                )
                .context("Update device sort_order failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_device(&self, id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM devices WHERE id = ?1", rusqlite::params![id])
                .context("Delete device failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Data Sources ──

    pub async fn max_data_source_sort_order(&self) -> Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let max: Option<i64> = conn
                .query_row("SELECT MAX(sort_order) FROM data_sources", [], |row| {
                    row.get(0)
                })
                .ok();
            Ok(max.unwrap_or(-1))
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_data_source_by_id(&self, id: i64) -> Result<Option<DataSource>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, key, name, unit, color, min_val, max_val,
                        warn_val, crit_val, base_val, sort_order, decimals,
                        source_type, source_config, device_id, equipment_id, chart_type, path,
                        on_delay_secs, off_delay_secs
                 FROM data_sources WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(DataSource {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        name: row.get(2)?,
                        unit: row.get(3)?,
                        color: row.get(4)?,
                        min_val: row.get(5)?,
                        max_val: row.get(6)?,
                        warn_val: row.get(7)?,
                        crit_val: row.get(8)?,
                        base_val: row.get(9)?,
                        sort_order: row.get(10)?,
                        decimals: row.get(11)?,
                        source_type: row.get(12)?,
                        source_config: row.get(13)?,
                        device_id: row.get(14)?,
                        equipment_id: row.get(15)?,
                        chart_type: row.get(16)?,
                        path: row.get(17)?,
                        on_delay_secs: row.get(18)?,
                        off_delay_secs: row.get(19)?,
                    })
                },
            );
            match result {
                Ok(ds) => Ok(Some(ds)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get data source by id failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_data_sources(&self, screen_id: Option<&str>) -> Result<Vec<DataSource>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &screen_id {
                Some(sid) => (
                    "SELECT s.id, s.key, s.name, s.unit, s.color, s.min_val, s.max_val,
                            s.warn_val, s.crit_val, s.base_val, s.sort_order, s.decimals,
                            s.source_type, s.source_config, s.device_id, s.equipment_id, s.chart_type, s.path,
                            s.on_delay_secs, s.off_delay_secs
                     FROM data_sources s
                     JOIN screen_data_sources ss ON s.id = ss.data_source_id
                     WHERE ss.screen_id = ?1
                     ORDER BY ss.sort_order",
                    vec![Box::new(sid.clone())],
                ),
                None => (
                    "SELECT id, key, name, unit, color, min_val, max_val,
                            warn_val, crit_val, base_val, sort_order, decimals,
                            source_type, source_config, device_id, equipment_id, chart_type, path,
                            on_delay_secs, off_delay_secs
                     FROM data_sources
                     ORDER BY sort_order",
                    vec![],
                ),
            };
            let mut stmt = conn.prepare(sql).context("Prepare failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(DataSource {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        name: row.get(2)?,
                        unit: row.get(3)?,
                        color: row.get(4)?,
                        min_val: row.get(5)?,
                        max_val: row.get(6)?,
                        warn_val: row.get(7)?,
                        crit_val: row.get(8)?,
                        base_val: row.get(9)?,
                        sort_order: row.get(10)?,
                        decimals: row.get(11)?,
                        source_type: row.get(12)?,
                        source_config: row.get(13)?,
                        device_id: row.get(14)?,
                        equipment_id: row.get(15)?,
                        chart_type: row.get(16)?,
                        path: row.get(17)?,
                        on_delay_secs: row.get(18)?,
                        off_delay_secs: row.get(19)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_data_source(
        &self,
        key: &str,
        name: &str,
        unit: &str,
        color: &str,
        min_val: f64,
        max_val: f64,
        warn_val: f64,
        crit_val: f64,
        base_val: f64,
        sort_order: i64,
        decimals: i64,
        source_type: &str,
        source_config: &str,
        device_id: &str,
        equipment_id: Option<&str>,
        chart_type: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let key = key.to_string();
        let name = name.to_string();
        let unit = unit.to_string();
        let color = color.to_string();
        let source_type = source_type.to_string();
        let source_config = source_config.to_string();
        let device_id = device_id.to_string();
        let equipment_id = equipment_id.map(|s| s.to_string());
        let chart_type = chart_type.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO data_sources (key, name, unit, color, min_val, max_val, warn_val, crit_val,
                 base_val, sort_order, decimals, source_type, source_config, device_id, equipment_id, chart_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                rusqlite::params![key, name, unit, color, min_val, max_val, warn_val, crit_val,
                                  base_val, sort_order, decimals, source_type, source_config,
                                  device_id, equipment_id, chart_type],
            )
            .context("Insert data source failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_data_source(&self, id: i64, fields: &serde_json::Value) -> Result<()> {
        let conn = self.conn.clone();
        let fields = fields.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let obj = fields.as_object().context("fields must be a JSON object")?;
            let allowed = [
                "key",
                "name",
                "unit",
                "color",
                "min_val",
                "max_val",
                "warn_val",
                "crit_val",
                "base_val",
                "sort_order",
                "decimals",
                "source_type",
                "chart_type",
                "path",
                "on_delay_secs",
                "off_delay_secs",
            ];
            let mut parts = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for &col in &allowed {
                if let Some(val) = obj.get(col) {
                    parts.push(format!("{col} = ?"));
                    match val {
                        serde_json::Value::Number(n) => {
                            if let Some(f) = n.as_f64() {
                                params.push(Box::new(f));
                            } else if let Some(i) = n.as_i64() {
                                params.push(Box::new(i));
                            }
                        }
                        serde_json::Value::String(s) => {
                            params.push(Box::new(s.clone()));
                        }
                        serde_json::Value::Null => {
                            params.push(Box::new(Option::<String>::None));
                        }
                        other => {
                            params.push(Box::new(other.to_string()));
                        }
                    }
                }
            }
            if parts.is_empty() {
                return Ok(());
            }
            params.push(Box::new(id));
            let sql = format!("UPDATE data_sources SET {} WHERE id = ?", parts.join(", "));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())
                .context("Update data source failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_data_source(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM data_sources WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete data source failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn reorder_data_sources(&self, order: &[i64]) -> Result<()> {
        let conn = self.conn.clone();
        let order = order.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            for (i, &id) in order.iter().enumerate() {
                conn.execute(
                    "UPDATE data_sources SET sort_order = ?1 WHERE id = ?2",
                    rusqlite::params![i as i64, id],
                )
                .context("Reorder data source failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Screen Data Sources ──

    pub async fn add_screen_data_source(
        &self,
        screen_id: &str,
        data_source_id: i64,
        sort_order: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO screen_data_sources (screen_id, data_source_id, sort_order) VALUES (?1, ?2, ?3)",
                rusqlite::params![screen_id, data_source_id, sort_order],
            )
            .context("Add screen data source failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_screen_data_source_sort(
        &self,
        screen_id: &str,
        data_source_id: i64,
        sort_order: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE screen_data_sources SET sort_order = ?1 WHERE screen_id = ?2 AND data_source_id = ?3",
                rusqlite::params![sort_order, screen_id, data_source_id],
            )
            .context("Update screen data source sort failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn remove_screen_data_source(
        &self,
        screen_id: &str,
        data_source_id: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM screen_data_sources WHERE screen_id = ?1 AND data_source_id = ?2",
                rusqlite::params![screen_id, data_source_id],
            )
            .context("Remove screen data source failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_screen_data_sources_by_screen(&self, screen_id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM screen_data_sources WHERE screen_id = ?1",
                rusqlite::params![screen_id],
            )
            .context("Delete screen data sources by screen failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Tag Tree ──

    pub async fn list_data_sources_for_tree(
        &self,
    ) -> Result<Vec<(i64, String, String, Option<String>, Option<String>)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, device_id, equipment_id, path FROM data_sources ORDER BY path, name")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub fn build_tag_tree(
        tags: &[(i64, String, String, Option<String>, Option<String>)],
        search_term: Option<&str>,
    ) -> Vec<serde_json::Value> {
        use std::collections::BTreeMap;

        let search_lower = search_term.map(|s| s.to_lowercase());

        #[derive(Default)]
        struct TreeNode {
            tag_ids: Vec<i64>,
            children: BTreeMap<String, TreeNode>,
        }

        fn insert_tag(root: &mut BTreeMap<String, TreeNode>, segments: &[&str], tag_id: i64) {
            if segments.is_empty() {
                return;
            }
            let node = root.entry(segments[0].to_string()).or_default();
            if segments.len() == 1 {
                node.tag_ids.push(tag_id);
            } else {
                insert_tag(&mut node.children, &segments[1..], tag_id);
            }
        }

        let mut root = BTreeMap::<String, TreeNode>::new();

        for (id, name, device_id, _equipment_id, path) in tags {
            if let Some(ref term) = search_lower {
                let name_lower = name.to_lowercase();
                let path_lower = path.as_deref().unwrap_or("").to_lowercase();
                let device_lower = device_id.to_lowercase();
                if !name_lower.contains(term)
                    && !path_lower.contains(term)
                    && !device_lower.contains(term)
                {
                    continue;
                }
            }

            let segments: Vec<&str> = match path {
                Some(p) if !p.is_empty() => p.split('/').collect(),
                _ => vec![device_id],
            };

            insert_tag(&mut root, &segments, *id);
        }

        fn node_to_json(name: &str, node: &TreeNode) -> serde_json::Value {
            let children: Vec<serde_json::Value> = node
                .children
                .iter()
                .map(|(n, c)| node_to_json(n, c))
                .collect();

            serde_json::json!({
                "name": name,
                "tagCount": node.tag_ids.len(),
                "tagIds": node.tag_ids,
                "children": children,
            })
        }

        let mut result: Vec<serde_json::Value> = root
            .iter()
            .map(|(name, node)| node_to_json(name, node))
            .collect();
        result.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
        result
    }

    pub async fn update_data_source_path(&self, id: i64, path: Option<&str>) -> Result<()> {
        let conn = self.conn.clone();
        let path = path.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE data_sources SET path = ?1 WHERE id = ?2",
                rusqlite::params![path, id],
            )
            .context("Update data source path failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Screen Data Sources (continued) ──

    /// Count how many distinct screens have data sources assigned.
    pub async fn count_screen_data_sources(&self) -> Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT screen_id) FROM screen_data_sources",
                    [],
                    |row| row.get(0),
                )
                .context("Count screen data sources failed")?;
            Ok(count)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_screen_data_sources(&self, screen_id: &str) -> Result<Vec<ScreenDataSource>> {
        let conn = self.conn.clone();
        let screen_id = screen_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT screen_id, data_source_id, sort_order FROM screen_data_sources WHERE screen_id = ?1 ORDER BY sort_order")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![screen_id], |row| {
                    Ok(ScreenDataSource {
                        screen_id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        sort_order: row.get(2)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
