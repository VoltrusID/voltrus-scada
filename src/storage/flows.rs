use super::Storage;
use anyhow::{Context, Result};

use crate::models::{Flow, FlowRun};

impl Storage {
    // ── Flows ──

    pub async fn create_flow(
        &self,
        id: &str,
        name: &str,
        enabled: bool,
        trigger_config: &str,
        nodes: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.to_string();
        let trigger_config = trigger_config.to_string();
        let nodes = nodes.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO flows (id, name, enabled, trigger_config, nodes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, name, enabled as i64, trigger_config, nodes, now, now],
            )
            .context("Insert flow failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_flow(&self, id: &str) -> Result<Option<Flow>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, enabled, trigger_config, nodes, created_at, updated_at FROM flows WHERE id = ?1")
                .context("Prepare failed")?;
            let result = stmt
                .query_row(rusqlite::params![id], |row| {
                    Ok(Flow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        enabled: row.get::<_, i64>(2)? != 0,
                        trigger_config: row.get(3)?,
                        nodes: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })
                .ok();
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_flows(&self) -> Result<Vec<Flow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, enabled, trigger_config, nodes, created_at, updated_at FROM flows ORDER BY created_at")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Flow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        enabled: row.get::<_, i64>(2)? != 0,
                        trigger_config: row.get(3)?,
                        nodes: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_flow(
        &self,
        id: &str,
        name: &str,
        enabled: bool,
        trigger_config: &str,
        nodes: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.to_string();
        let trigger_config = trigger_config.to_string();
        let nodes = nodes.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE flows SET name = ?1, enabled = ?2, trigger_config = ?3, nodes = ?4, updated_at = ?5 WHERE id = ?6",
                rusqlite::params![name, enabled as i64, trigger_config, nodes, now, id],
            )
            .context("Update flow failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_flow(&self, id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM flows WHERE id = ?1", rusqlite::params![id])
                .context("Delete flow failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Flow runs ──

    pub async fn insert_flow_run(
        &self,
        flow_id: &str,
        duration_ms: u64,
        success: bool,
        error: Option<String>,
        output: Option<serde_json::Value>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let flow_id = flow_id.to_string();
        let error = error.map(|s| s.to_string());
        let output = output.map(|v| v.to_string());
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO flow_runs (flow_id, started_at, duration_ms, success, error, output)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    flow_id,
                    now,
                    duration_ms as i64,
                    success as i64,
                    error,
                    output
                ],
            )
            .context("Insert flow run failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_flow_runs(&self, flow_id: &str, limit: i64) -> Result<Vec<FlowRun>> {
        let conn = self.conn.clone();
        let flow_id = flow_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, flow_id, started_at, duration_ms, success, error, output FROM flow_runs WHERE flow_id = ?1 ORDER BY started_at DESC LIMIT ?2")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![flow_id, limit], |row| {
                    Ok(FlowRun {
                        id: row.get(0)?,
                        flow_id: row.get(1)?,
                        started_at: row.get(2)?,
                        duration_ms: row.get(3)?,
                        success: row.get::<_, i64>(4)? != 0,
                        error: row.get(5)?,
                        output: row.get(6)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn is_in_cooldown(&self, source_key: &str, severity: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let source_key = source_key.to_string();
        let severity = severity.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms
                     WHERE source_key = ?1 AND severity = ?2
                       AND cooldown_until IS NOT NULL AND cooldown_until > ?3",
                    rusqlite::params![source_key, severity, now],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            Ok(count > 0)
        })
        .await
        .context("Task join failed")?
    }

    // ── Push subscription stubs ──

    pub async fn save_push_subscription(
        &self,
        user: &str,
        endpoint: &str,
        p256dh: &str,
        auth_key: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let user = user.to_string();
        let endpoint = endpoint.to_string();
        let p256dh = p256dh.to_string();
        let auth_key = auth_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO push_subscriptions (user, endpoint, p256dh, auth_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(user, endpoint) DO UPDATE SET p256dh=excluded.p256dh, auth_key=excluded.auth_key",
                rusqlite::params![user, endpoint, p256dh, auth_key, now],
            )
            .context("Upsert push subscription failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    pub async fn remove_push_subscription(&self, user: &str, endpoint: &str) -> Result<()> {
        let conn = self.conn.clone();
        let user = user.to_string();
        let endpoint = endpoint.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM push_subscriptions WHERE user = ?1 AND endpoint = ?2",
                rusqlite::params![user, endpoint],
            )
            .context("Delete push subscription failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    /// Get all push subscriptions (for broadcasting alarm notifications).
    pub async fn get_push_subscriptions(&self) -> Result<Vec<crate::models::PushSubscription>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, user, endpoint, p256dh, auth_key, created_at
                     FROM push_subscriptions",
                )
                .context("Prepare get_push_subscriptions failed")?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::models::PushSubscription {
                        id: row.get(0)?,
                        user: row.get(1)?,
                        endpoint: row.get(2)?,
                        p256dh: row.get(3)?,
                        auth_key: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .context("Query get_push_subscriptions failed")?;

            let mut subscriptions = Vec::new();
            for row in rows {
                subscriptions.push(row.context("Row get_push_subscriptions failed")?);
            }
            Ok(subscriptions)
        })
        .await
        .context("Task join failed")?
    }

    /// Get push subscriptions for a specific user.
    pub async fn get_push_subscriptions_by_user(
        &self,
        user: &str,
    ) -> Result<Vec<crate::models::PushSubscription>> {
        let conn = self.conn.clone();
        let user = user.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, user, endpoint, p256dh, auth_key, created_at
                     FROM push_subscriptions WHERE user = ?1",
                )
                .context("Prepare get_push_subscriptions_by_user failed")?;

            let rows = stmt
                .query_map(rusqlite::params![user], |row| {
                    Ok(crate::models::PushSubscription {
                        id: row.get(0)?,
                        user: row.get(1)?,
                        endpoint: row.get(2)?,
                        p256dh: row.get(3)?,
                        auth_key: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .context("Query get_push_subscriptions_by_user failed")?;

            let mut subscriptions = Vec::new();
            for row in rows {
                subscriptions.push(row.context("Row get_push_subscriptions_by_user failed")?);
            }
            Ok(subscriptions)
        })
        .await
        .context("Task join failed")?
    }

    /// Delete a push subscription by ID (for cleaning up stale subscriptions).
    pub async fn delete_push_subscription(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM push_subscriptions WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete push subscription by id failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }
}
