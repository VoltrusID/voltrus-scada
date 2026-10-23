use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    // ── Sim Profile CRUD methods ──

    pub async fn list_sim_profiles(&self) -> anyhow::Result<Vec<crate::models::SimProfile>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, data_type, config, created_at FROM sim_profiles ORDER BY name")
                .context("Prepare list_sim_profiles failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::models::SimProfile {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        data_type: row.get(2)?,
                        config: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                })
                .context("Query list_sim_profiles failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row parse list_sim_profiles failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_sim_profile(
        &self,
        id: i64,
    ) -> anyhow::Result<Option<crate::models::SimProfile>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, data_type, config, created_at FROM sim_profiles WHERE id = ?1")
                .context("Prepare get_sim_profile failed")?;
            let mut rows = stmt
                .query_map(rusqlite::params![id], |row| {
                    Ok(crate::models::SimProfile {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        data_type: row.get(2)?,
                        config: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                })
                .context("Query get_sim_profile failed")?;
            match rows.next() {
                Some(row) => Ok(Some(row.context("Row parse get_sim_profile failed")?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_sim_profile(
        &self,
        name: &str,
        data_type: &str,
        config: &str,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let data_type = data_type.to_string();
        let config = config.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sim_profiles (name, data_type, config, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![name, data_type, config, now],
            )
            .context("Insert sim_profile failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_sim_profile(
        &self,
        id: i64,
        name: Option<&str>,
        data_type: Option<&str>,
        config: Option<&str>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let data_type = data_type.map(|s| s.to_string());
        let config = config.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let existing = conn
                .query_row(
                    "SELECT name, data_type, config FROM sim_profiles WHERE id = ?1",
                    rusqlite::params![id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .ok();
            match existing {
                None => Ok(false),
                Some((cur_name, cur_data_type, cur_config)) => {
                    let name = name.unwrap_or(cur_name);
                    let data_type = data_type.unwrap_or(cur_data_type);
                    let config = config.unwrap_or(cur_config);
                    let affected = conn
                        .execute(
                            "UPDATE sim_profiles SET name = ?1, data_type = ?2, config = ?3 WHERE id = ?4",
                            rusqlite::params![name, data_type, config, id],
                        )
                        .context("Update sim_profile failed")?;
                    Ok(affected > 0)
                }
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_sim_profile(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let affected = conn
                .execute(
                    "DELETE FROM sim_profiles WHERE id = ?1",
                    rusqlite::params![id],
                )
                .context("Delete sim_profile failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }

    // ── Device simulation get/set ──

    pub async fn set_device_simulation(
        &self,
        device_id: &str,
        sim_mode: bool,
        sim_profile_id: Option<i64>,
        sim_config: Option<&str>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let sim_config = sim_config.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let affected = conn
                .execute(
                    "UPDATE devices SET simulation_mode = ?1, sim_profile_id = ?2, sim_config = ?3 WHERE id = ?4",
                    rusqlite::params![sim_mode as i32, sim_profile_id, sim_config, device_id],
                )
                .context("Update device simulation failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_device_simulation(
        &self,
        device_id: &str,
    ) -> anyhow::Result<Option<(bool, Option<i64>, Option<String>)>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT simulation_mode, sim_profile_id, sim_config FROM devices WHERE id = ?1",
                rusqlite::params![device_id],
                |row| {
                    let mode: i32 = row.get(0)?;
                    Ok((mode != 0, row.get(1)?, row.get(2)?))
                },
            );
            match result {
                Ok(row) => Ok(Some(row)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get device simulation failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    // ── Script CRUD methods ──

    pub async fn list_scripts(&self) -> anyhow::Result<Vec<crate::models::Script>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, source, syntax_error, cpu_limit_ms, memory_limit_bytes, enabled, created_at, updated_at FROM scripts ORDER BY name")
                .context("Prepare list_scripts failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::models::Script {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        source: row.get(2)?,
                        syntax_error: row.get(3)?,
                        cpu_limit_ms: row.get(4)?,
                        memory_limit_bytes: row.get(5)?,
                        enabled: row.get::<_, i32>(6)? != 0,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                })
                .context("Query list_scripts failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row parse list_scripts failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_script(&self, id: i64) -> anyhow::Result<Option<crate::models::Script>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, source, syntax_error, cpu_limit_ms, memory_limit_bytes, enabled, created_at, updated_at FROM scripts WHERE id = ?1")
                .context("Prepare get_script failed")?;
            let mut rows = stmt
                .query_map(rusqlite::params![id], |row| {
                    Ok(crate::models::Script {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        source: row.get(2)?,
                        syntax_error: row.get(3)?,
                        cpu_limit_ms: row.get(4)?,
                        memory_limit_bytes: row.get(5)?,
                        enabled: row.get::<_, i32>(6)? != 0,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                })
                .context("Query get_script failed")?;
            match rows.next() {
                Some(row) => Ok(Some(row.context("Row parse get_script failed")?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_script(
        &self,
        name: &str,
        source: &str,
        syntax_error: Option<&str>,
        cpu_limit_ms: i64,
        memory_limit_bytes: i64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let source = source.to_string();
        let syntax_error = syntax_error.map(|s| s.to_string());
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO scripts (name, source, syntax_error, cpu_limit_ms, memory_limit_bytes, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
                rusqlite::params![name, source, syntax_error, cpu_limit_ms, memory_limit_bytes, now, now],
            )
            .context("Insert script failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_script(
        &self,
        id: i64,
        name: Option<&str>,
        source: Option<&str>,
        syntax_error: Option<&str>,
        cpu_limit_ms: Option<i64>,
        memory_limit_bytes: Option<i64>,
        enabled: Option<bool>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let source = source.map(|s| s.to_string());
        let syntax_error = syntax_error.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Check existence
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM scripts WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get::<_, i32>(0),
                )
                .context("Check script existence failed")?
                > 0;
            if !exists {
                return Ok(false);
            }

            // Build dynamic SET clauses
            let mut set_clauses: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref v) = name {
                set_clauses.push("name = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = source {
                set_clauses.push("source = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = syntax_error {
                set_clauses.push("syntax_error = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(v) = cpu_limit_ms {
                set_clauses.push("cpu_limit_ms = ?".to_string());
                params.push(Box::new(v));
            }
            if let Some(v) = memory_limit_bytes {
                set_clauses.push("memory_limit_bytes = ?".to_string());
                params.push(Box::new(v));
            }
            if let Some(v) = enabled {
                set_clauses.push("enabled = ?".to_string());
                params.push(Box::new(v as i32));
            }

            if set_clauses.is_empty() {
                return Ok(false);
            }

            // Always update updated_at
            let now = crate::timeutil::now_secs();
            set_clauses.push("updated_at = ?".to_string());
            params.push(Box::new(now));

            // Add WHERE id param
            params.push(Box::new(id));

            // Build parameter placeholders
            let placeholders: Vec<String> = (1..=params.len()).map(|i| format!("?{}", i)).collect();
            let placeholders_str = placeholders.join(", ");

            let _sql = format!(
                "UPDATE scripts SET {} WHERE id = {}",
                set_clauses.join(", "),
                placeholders_str
            );

            // Recalculate: we need positional params, rebuild with proper numbering
            let mut set_parts: Vec<String> = Vec::new();
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1usize;

            if let Some(ref v) = name {
                set_parts.push(format!("name = ?{}", idx));
                param_values.push(Box::new(v.clone()));
                idx += 1;
            }
            if let Some(ref v) = source {
                set_parts.push(format!("source = ?{}", idx));
                param_values.push(Box::new(v.clone()));
                idx += 1;
            }
            if let Some(ref v) = syntax_error {
                set_parts.push(format!("syntax_error = ?{}", idx));
                param_values.push(Box::new(v.clone()));
                idx += 1;
            }
            if let Some(v) = cpu_limit_ms {
                set_parts.push(format!("cpu_limit_ms = ?{}", idx));
                param_values.push(Box::new(v));
                idx += 1;
            }
            if let Some(v) = memory_limit_bytes {
                set_parts.push(format!("memory_limit_bytes = ?{}", idx));
                param_values.push(Box::new(v));
                idx += 1;
            }
            if let Some(v) = enabled {
                set_parts.push(format!("enabled = ?{}", idx));
                param_values.push(Box::new(v as i32));
                idx += 1;
            }
            set_parts.push(format!("updated_at = ?{}", idx));
            param_values.push(Box::new(now));
            idx += 1;

            let sql = format!(
                "UPDATE scripts SET {} WHERE id = ?{}",
                set_parts.join(", "),
                idx
            );
            param_values.push(Box::new(id));

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let affected = conn
                .execute(&sql, param_refs.as_slice())
                .context("Update script failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_script(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let affected = conn
                .execute("DELETE FROM scripts WHERE id = ?1", rusqlite::params![id])
                .context("Delete script failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }

    // ── Script Execution CRUD methods ──

    pub async fn create_script_execution(
        &self,
        script_id: i64,
        flow_id: Option<&str>,
        trigger_type: Option<&str>,
        duration_ms: i64,
        status: &str,
        error_message: Option<&str>,
        output: Option<&str>,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let flow_id = flow_id.map(|s| s.to_string());
        let trigger_type = trigger_type.map(|s| s.to_string());
        let status = status.to_string();
        let error_message = error_message.map(|s| s.to_string());
        let output = output.map(|s| s.to_string());
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO script_executions (script_id, flow_id, trigger_type, duration_ms, status, error_message, output, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![script_id, flow_id, trigger_type, duration_ms, status, error_message, output, now],
            )
            .context("Insert script_execution failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_script_executions(
        &self,
        script_id: i64,
        limit: i64,
    ) -> anyhow::Result<Vec<crate::models::ScriptExecution>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, script_id, flow_id, trigger_type, duration_ms, status, error_message, output, created_at FROM script_executions WHERE script_id = ?1 ORDER BY created_at DESC LIMIT ?2")
                .context("Prepare list_script_executions failed")?;
            let rows = stmt
                .query_map(rusqlite::params![script_id, limit], |row| {
                    Ok(crate::models::ScriptExecution {
                        id: row.get(0)?,
                        script_id: row.get(1)?,
                        flow_id: row.get(2)?,
                        trigger_type: row.get(3)?,
                        duration_ms: row.get(4)?,
                        status: row.get(5)?,
                        error_message: row.get(6)?,
                        output: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                })
                .context("Query list_script_executions failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row parse list_script_executions failed")
        })
        .await
        .context("Task join failed")?
    }
}
