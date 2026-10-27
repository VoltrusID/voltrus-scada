use super::Storage;
use anyhow::{Context, Result};

use crate::models::Role;

impl Storage {
    // ── Role management ──

    pub async fn create_role(&self, name: &str, permissions: &str) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let permissions = permissions.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO roles (name, permissions, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![name, permissions, now],
            )
            .context("Insert role failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_roles(&self) -> Result<Vec<Role>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, permissions, created_at FROM roles ORDER BY id")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Role {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        permissions: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_role_by_id(&self, id: i64) -> Result<Option<Role>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn
                .query_row(
                    "SELECT id, name, permissions, created_at FROM roles WHERE id = ?1",
                    rusqlite::params![id],
                    |row| {
                        Ok(Role {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            permissions: row.get(2)?,
                            created_at: row.get(3)?,
                        })
                    },
                )
                .ok();
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_role(
        &self,
        id: i64,
        name: Option<&str>,
        permissions: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let permissions = permissions.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = name {
                conn.execute(
                    "UPDATE roles SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update role name failed")?;
            }
            if let Some(permissions) = permissions {
                conn.execute(
                    "UPDATE roles SET permissions = ?1 WHERE id = ?2",
                    rusqlite::params![permissions, id],
                )
                .context("Update role permissions failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_role(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM roles WHERE id = ?1", rusqlite::params![id])
                .context("Delete role failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn assign_user_role(&self, user_id: i64, role_id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET role_id = ?1 WHERE id = ?2",
                rusqlite::params![role_id, user_id],
            )
            .context("Assign user role failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// The RBAC role assigned to a user (`PUT /users/{id}/role`), if any.
    pub async fn get_user_role_id(&self, user_id: i64) -> Result<Option<i64>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.query_row(
                "SELECT role_id FROM users WHERE id = ?1",
                rusqlite::params![user_id],
                |row| row.get(0),
            )
            .context("Read user role failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_user_role_permissions(&self, user_id: i64) -> Result<Vec<String>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let permissions_json: Option<String> = conn
                .query_row(
                    "SELECT r.permissions FROM users u LEFT JOIN roles r ON u.role_id = r.id WHERE u.id = ?1",
                    rusqlite::params![user_id],
                    |row| row.get(0),
                )
                .ok();
            match permissions_json {
                Some(json_str) => {
                    let perms: Vec<String> = serde_json::from_str(&json_str).unwrap_or_default();
                    Ok(perms)
                }
                None => Ok(vec![]),
            }
        })
        .await
        .context("Task join failed")?
    }

    /// Seed default roles if they don't exist yet.
    pub async fn seed_default_roles(&self) -> Result<()> {
        let existing = self.list_roles().await?;
        if existing.is_empty() {
            let _ = self
                .create_role(
                    "Admin",
                    &serde_json::to_string(crate::models::ALL_PERMISSIONS).unwrap(),
                )
                .await;
            let _ = self
                .create_role(
                    "Demo",
                    &serde_json::to_string(crate::models::DEMO_PERMISSIONS).unwrap(),
                )
                .await;
            let _ = self
                .create_role(
                    "Operator",
                    &serde_json::to_string(crate::models::OPERATOR_PERMISSIONS).unwrap(),
                )
                .await;
            let _ = self
                .create_role(
                    "Viewer",
                    &serde_json::to_string(crate::models::VIEWER_PERMISSIONS).unwrap(),
                )
                .await;
        }
        Ok(())
    }
}
