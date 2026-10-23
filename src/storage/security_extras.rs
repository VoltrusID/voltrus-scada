use super::Storage;
use crate::models::{
    Annotation, CreateAnnotationRequest, CreateNotificationRequest, CreateShiftHandoverRequest,
    ShiftHandover, UpdateAnnotationRequest, UserNotification,
};
use anyhow::{Context, Result};

impl Storage {
    // ── Annotation CRUD (v0.23.0) ──

    pub async fn create_annotation(
        &self,
        req: &CreateAnnotationRequest,
        author_id: i64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let req = req.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO annotations (tag_id, annotation_text, data_timestamp, author_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![req.tag_id, req.annotation_text, req.data_timestamp, author_id, now, now],
            ).context("Insert annotation failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_annotations(
        &self,
        tag_id: Option<&str>,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
    ) -> Result<Vec<Annotation>> {
        let conn = self.conn.clone();
        let tag_id = tag_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT id, tag_id, annotation_text, data_timestamp, author_id, created_at, updated_at
                 FROM annotations WHERE 1=1"
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(ref t) = tag_id {
                sql.push_str(" AND tag_id = ?");
                params.push(Box::new(t.clone()));
            }
            if let Some(from) = from_ts {
                sql.push_str(" AND data_timestamp >= ?");
                params.push(Box::new(from));
            }
            if let Some(to) = to_ts {
                sql.push_str(" AND data_timestamp <= ?");
                params.push(Box::new(to));
            }
            sql.push_str(" ORDER BY data_timestamp DESC");
            let mut stmt = conn.prepare(&sql).context("Prepare annotations failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(Annotation {
                    id: row.get(0)?,
                    tag_id: row.get(1)?,
                    annotation_text: row.get(2)?,
                    data_timestamp: row.get(3)?,
                    author_id: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            }).context("Query annotations failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Collect annotations failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_annotation(&self, id: i64) -> Result<Option<Annotation>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, tag_id, annotation_text, data_timestamp, author_id, created_at, updated_at
                 FROM annotations WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(Annotation {
                        id: row.get(0)?,
                        tag_id: row.get(1)?,
                        annotation_text: row.get(2)?,
                        data_timestamp: row.get(3)?,
                        author_id: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            ).ok();
            Ok(result)
        }).await.context("Task join failed")?
    }

    pub async fn update_annotation(&self, id: i64, req: &UpdateAnnotationRequest) -> Result<()> {
        let conn = self.conn.clone();
        let req = req.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            if let Some(ref text) = req.annotation_text {
                conn.execute(
                    "UPDATE annotations SET annotation_text = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![text, now, id],
                )
                .context("Update annotation failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_annotation(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM annotations WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete annotation failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Shift Handover CRUD (v0.23.0) ──

    pub async fn create_shift_handover(
        &self,
        req: &CreateShiftHandoverRequest,
        author_id: i64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let req = req.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO shift_handovers (author_id, shift_date, shift_period, what_happened, whats_pending, what_to_watch, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    author_id,
                    req.shift_date,
                    req.shift_period,
                    req.what_happened.unwrap_or_default(),
                    req.whats_pending.unwrap_or_default(),
                    req.what_to_watch.unwrap_or_default(),
                    now,
                ],
            ).context("Insert shift handover failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_shift_handovers(
        &self,
        shift_date: Option<&str>,
        shift_period: Option<&str>,
    ) -> Result<Vec<ShiftHandover>> {
        let conn = self.conn.clone();
        let shift_date = shift_date.map(|s| s.to_string());
        let shift_period = shift_period.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT id, author_id, shift_date, shift_period, what_happened, whats_pending, what_to_watch, created_at
                 FROM shift_handovers WHERE 1=1"
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(ref d) = shift_date {
                sql.push_str(" AND shift_date = ?");
                params.push(Box::new(d.clone()));
            }
            if let Some(ref p) = shift_period {
                sql.push_str(" AND shift_period = ?");
                params.push(Box::new(p.clone()));
            }
            sql.push_str(" ORDER BY shift_date DESC, shift_period ASC");
            let mut stmt = conn.prepare(&sql).context("Prepare shift handovers failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(ShiftHandover {
                    id: row.get(0)?,
                    author_id: row.get(1)?,
                    shift_date: row.get(2)?,
                    shift_period: row.get(3)?,
                    what_happened: row.get(4)?,
                    whats_pending: row.get(5)?,
                    what_to_watch: row.get(6)?,
                    created_at: row.get(7)?,
                })
            }).context("Query shift handovers failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Collect shift handovers failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_shift_handover(&self, id: i64) -> Result<Option<ShiftHandover>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, author_id, shift_date, shift_period, what_happened, whats_pending, what_to_watch, created_at
                 FROM shift_handovers WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(ShiftHandover {
                        id: row.get(0)?,
                        author_id: row.get(1)?,
                        shift_date: row.get(2)?,
                        shift_period: row.get(3)?,
                        what_happened: row.get(4)?,
                        whats_pending: row.get(5)?,
                        what_to_watch: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            ).ok();
            Ok(result)
        }).await.context("Task join failed")?
    }

    // ── User Notification CRUD (v0.23.0) ──

    pub async fn create_notification(&self, req: &CreateNotificationRequest) -> Result<i64> {
        let conn = self.conn.clone();
        let req = req.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO user_notifications (user_id, from_user_id, notification_type, message, reference_id, read, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
                rusqlite::params![
                    req.user_id,
                    req.from_user_id,
                    req.notification_type,
                    req.message,
                    req.reference_id,
                    now,
                ],
            ).context("Insert notification failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_user_notifications(
        &self,
        user_id: i64,
        unread_only: bool,
    ) -> Result<Vec<UserNotification>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if unread_only {
                (
                    "SELECT id, user_id, from_user_id, notification_type, message, reference_id, read, created_at
                     FROM user_notifications WHERE user_id = ?1 AND read = 0
                     ORDER BY created_at DESC",
                    vec![Box::new(user_id)],
                )
            } else {
                (
                    "SELECT id, user_id, from_user_id, notification_type, message, reference_id, read, created_at
                     FROM user_notifications WHERE user_id = ?1
                     ORDER BY created_at DESC",
                    vec![Box::new(user_id)],
                )
            };
            let mut stmt = conn.prepare(sql).context("Prepare notifications failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(UserNotification {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    from_user_id: row.get(2)?,
                    notification_type: row.get(3)?,
                    message: row.get(4)?,
                    reference_id: row.get(5)?,
                    read: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                })
            }).context("Query notifications failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Collect notifications failed")
        }).await.context("Task join failed")?
    }

    pub async fn mark_notification_read(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE user_notifications SET read = 1 WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Mark notification read failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn dismiss_notification(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM user_notifications WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Dismiss notification failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn cleanup_old_notifications(&self) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let cutoff = crate::timeutil::now_secs() - 30 * 24 * 3600;
            let affected = conn
                .execute(
                    "DELETE FROM user_notifications WHERE created_at < ?1",
                    rusqlite::params![cutoff],
                )
                .context("Cleanup old notifications failed")?;
            Ok(affected as u64)
        })
        .await
        .context("Task join failed")?
    }
}
