use super::Storage;
use anyhow::Context;

impl Storage {
    // ── BACnet device CRUD ──

    pub async fn create_bacnet_device(
        &self,
        name: &str,
        ip: &str,
        port: u16,
        device_instance: u32,
        interval_ms: u64,
        cov_enabled: bool,
        cov_lifetime: u32,
        enabled: bool,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let now = crate::timeutil::now_secs();
        let name = name.to_string();
        let ip = ip.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bacnet_devices (name, ip, port, device_instance, interval_ms, cov_enabled, cov_lifetime, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![name, ip, port, device_instance, interval_ms, cov_enabled as i64, cov_lifetime, enabled as i64, now],
            ).context("Insert bacnet device failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_bacnet_devices(&self) -> anyhow::Result<Vec<crate::models::BacnetDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, ip, port, device_instance, interval_ms, cov_enabled, cov_lifetime, enabled, created_at FROM bacnet_devices ORDER BY created_at",
            )?;
            let devices = stmt.query_map([], |row| {
                Ok(crate::models::BacnetDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    ip: row.get(2)?,
                    port: row.get(3)?,
                    device_instance: row.get(4)?,
                    interval_ms: row.get(5)?,
                    cov_enabled: row.get::<_, i64>(6)? != 0,
                    cov_lifetime: row.get(7)?,
                    enabled: row.get::<_, i64>(8)? != 0,
                    created_at: row.get(9)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(devices)
        }).await.context("Task join failed")?
    }

    pub async fn update_bacnet_device(
        &self,
        id: i64,
        name: Option<&str>,
        ip: Option<&str>,
        port: Option<u16>,
        device_instance: Option<u32>,
        interval_ms: Option<u64>,
        cov_enabled: Option<bool>,
        cov_lifetime: Option<u32>,
        enabled: Option<bool>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let name = name.map(String::from);
        let ip = ip.map(String::from);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let current = conn.query_row(
                "SELECT name, ip, port, device_instance, interval_ms, cov_enabled, cov_lifetime, enabled FROM bacnet_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u16>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, i64>(5)? != 0,
                    row.get::<_, u32>(6)?,
                    row.get::<_, i64>(7)? != 0,
                )),
            );
            let (cur_name, cur_ip, cur_port, cur_inst, cur_interval, cur_cov, cur_cov_life, cur_enabled) = match current {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            let rows = conn.execute(
                "UPDATE bacnet_devices SET name=?1, ip=?2, port=?3, device_instance=?4, interval_ms=?5, cov_enabled=?6, cov_lifetime=?7, enabled=?8 WHERE id=?9",
                rusqlite::params![
                    name.as_deref().unwrap_or(&cur_name),
                    ip.as_deref().unwrap_or(&cur_ip),
                    port.unwrap_or(cur_port),
                    device_instance.unwrap_or(cur_inst),
                    interval_ms.unwrap_or(cur_interval),
                    cov_enabled.unwrap_or(cur_cov) as i64,
                    cov_lifetime.unwrap_or(cur_cov_life),
                    enabled.unwrap_or(cur_enabled) as i64,
                    id,
                ],
            )?;
            Ok(rows > 0)
        }).await.context("Task join failed")?
    }

    pub async fn get_bacnet_device(
        &self,
        id: i64,
    ) -> anyhow::Result<Option<crate::models::BacnetDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, name, ip, port, device_instance, interval_ms, cov_enabled, cov_lifetime, enabled, created_at FROM bacnet_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(crate::models::BacnetDevice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        ip: row.get(2)?,
                        port: row.get(3)?,
                        device_instance: row.get(4)?,
                        interval_ms: row.get(5)?,
                        cov_enabled: row.get::<_, i64>(6)? != 0,
                        cov_lifetime: row.get(7)?,
                        enabled: row.get::<_, i64>(8)? != 0,
                        created_at: row.get(9)?,
                    })
                },
            );
            match result {
                Ok(device) => Ok(Some(device)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }).await.context("Task join failed")?
    }

    pub async fn delete_bacnet_device(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM bacnet_objects WHERE device_id = ?1",
                rusqlite::params![id],
            )?;
            let rows = conn.execute(
                "DELETE FROM bacnet_devices WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }

    // ── BACnet object CRUD ──

    pub async fn create_bacnet_object(
        &self,
        device_id: i64,
        _device_name: &str,
        object_type: &str,
        object_instance: u32,
        property: &str,
        name: &str,
        display_name: &str,
        cov_increment: f64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let now = crate::timeutil::now_secs();
        let object_type = object_type.to_string();
        let property = property.to_string();
        let name = name.to_string();
        let display_name = display_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bacnet_objects (device_id, object_type, object_instance, property, name, display_name, cov_increment, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![device_id, object_type, object_instance, property, name, display_name, cov_increment, now],
            ).context("Insert bacnet object failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_bacnet_objects(
        &self,
        device_id: i64,
    ) -> anyhow::Result<Vec<crate::models::BacnetObject>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, device_id, object_type, object_instance, property, name, display_name, cov_increment, created_at FROM bacnet_objects WHERE device_id = ?1 ORDER BY object_type, object_instance",
            )?;
            let objects = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(crate::models::BacnetObject {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    object_type: row.get(2)?,
                    object_instance: row.get(3)?,
                    property: row.get(4)?,
                    name: row.get(5)?,
                    display_name: row.get(6)?,
                    cov_increment: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(objects)
        }).await.context("Task join failed")?
    }

    pub async fn delete_bacnet_object(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let rows = conn.execute(
                "DELETE FROM bacnet_objects WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }
}
