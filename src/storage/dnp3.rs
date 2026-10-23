use super::Storage;
use anyhow::Context;

impl Storage {
    pub async fn create_dnp3_device(
        &self,
        name: &str,
        ip: &str,
        port: u16,
        master_address: u16,
        outstation_address: u16,
        enable_unsolicited: bool,
        integrity_poll_interval_secs: u64,
        event_scan_interval_ms: Option<u64>,
        enabled: bool,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let name = name.to_string();
        let ip = ip.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO dnp3_devices (name, ip, port, master_address, outstation_address, enable_unsolicited, integrity_poll_interval_secs, event_scan_interval_ms, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                rusqlite::params![name, ip, port, master_address, outstation_address, enable_unsolicited as i64, integrity_poll_interval_secs, event_scan_interval_ms, enabled as i64, now],
            ).context("Insert dnp3 device failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_dnp3_devices(&self) -> anyhow::Result<Vec<crate::models::Dnp3Device>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, ip, port, master_address, outstation_address, enable_unsolicited, integrity_poll_interval_secs, event_scan_interval_ms, enabled, created_at, updated_at FROM dnp3_devices ORDER BY created_at",
            )?;
            let devices = stmt.query_map([], |row| {
                Ok(crate::models::Dnp3Device {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    ip: row.get(2)?,
                    port: row.get(3)?,
                    master_address: row.get(4)?,
                    outstation_address: row.get(5)?,
                    enable_unsolicited: row.get::<_, i64>(6)? != 0,
                    integrity_poll_interval_secs: row.get(7)?,
                    event_scan_interval_ms: row.get(8)?,
                    enabled: row.get::<_, i64>(9)? != 0,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(devices)
        }).await.context("Task join failed")?
    }

    pub async fn get_dnp3_device(
        &self,
        id: i64,
    ) -> anyhow::Result<Option<crate::models::Dnp3Device>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, name, ip, port, master_address, outstation_address, enable_unsolicited, integrity_poll_interval_secs, event_scan_interval_ms, enabled, created_at, updated_at FROM dnp3_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(crate::models::Dnp3Device {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        ip: row.get(2)?,
                        port: row.get(3)?,
                        master_address: row.get(4)?,
                        outstation_address: row.get(5)?,
                        enable_unsolicited: row.get::<_, i64>(6)? != 0,
                        integrity_poll_interval_secs: row.get(7)?,
                        event_scan_interval_ms: row.get(8)?,
                        enabled: row.get::<_, i64>(9)? != 0,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
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

    pub async fn update_dnp3_device(
        &self,
        id: i64,
        name: Option<&str>,
        ip: Option<&str>,
        port: Option<u16>,
        master_address: Option<u16>,
        outstation_address: Option<u16>,
        enable_unsolicited: Option<bool>,
        integrity_poll_interval_secs: Option<u64>,
        event_scan_interval_ms: Option<Option<u64>>,
        enabled: Option<bool>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let name = name.map(String::from);
        let ip = ip.map(String::from);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let current = conn.query_row(
                "SELECT name, ip, port, master_address, outstation_address, enable_unsolicited, integrity_poll_interval_secs, event_scan_interval_ms, enabled FROM dnp3_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u16>(2)?,
                    row.get::<_, u16>(3)?,
                    row.get::<_, u16>(4)?,
                    row.get::<_, i64>(5)? != 0,
                    row.get::<_, u64>(6)?,
                    row.get::<_, Option<u64>>(7)?,
                    row.get::<_, i64>(8)? != 0,
                )),
            );
            let (cur_name, cur_ip, cur_port, cur_master, cur_outstation, cur_unsolicited, cur_integrity, cur_event, cur_enabled) = match current {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            let new_name = name.as_deref().unwrap_or(&cur_name);
            let new_ip = ip.as_deref().unwrap_or(&cur_ip);
            let new_port = port.unwrap_or(cur_port);
            let new_master = master_address.unwrap_or(cur_master);
            let new_outstation = outstation_address.unwrap_or(cur_outstation);
            let new_unsolicited = enable_unsolicited.unwrap_or(cur_unsolicited);
            let new_integrity = integrity_poll_interval_secs.unwrap_or(cur_integrity);
            let new_event = event_scan_interval_ms.unwrap_or(cur_event);
            let new_enabled = enabled.unwrap_or(cur_enabled);
            let rows = conn.execute(
                "UPDATE dnp3_devices SET name=?1, ip=?2, port=?3, master_address=?4, outstation_address=?5, enable_unsolicited=?6, integrity_poll_interval_secs=?7, event_scan_interval_ms=?8, enabled=?9, updated_at=?10 WHERE id=?11",
                rusqlite::params![new_name, new_ip, new_port, new_master, new_outstation, new_unsolicited as i64, new_integrity, new_event, new_enabled as i64, now, id],
            )?;
            Ok(rows > 0)
        }).await.context("Task join failed")?
    }

    pub async fn delete_dnp3_device(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let rows = conn.execute(
                "DELETE FROM dnp3_devices WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }

    // ── DNP3 point CRUD ──

    pub async fn create_dnp3_point(
        &self,
        device_id: i64,
        group_num: u8,
        variation: u8,
        point_index: u16,
        name: &str,
        display_name: &str,
        data_type: &str,
        scale: f64,
        deadband: f64,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let name = name.to_string();
        let display_name = display_name.to_string();
        let data_type = data_type.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO dnp3_points (device_id, group_num, variation, point_index, name, display_name, data_type, scale, deadband, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![device_id, group_num, variation, point_index, name, display_name, data_type, scale, deadband, now],
            ).context("Insert dnp3 point failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn list_dnp3_points(
        &self,
        device_id: i64,
    ) -> anyhow::Result<Vec<crate::models::Dnp3Point>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, device_id, group_num, variation, point_index, name, display_name, data_type, scale, deadband, created_at FROM dnp3_points WHERE device_id = ?1 ORDER BY point_index",
            )?;
            let points = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(crate::models::Dnp3Point {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    group_num: row.get(2)?,
                    variation: row.get(3)?,
                    point_index: row.get(4)?,
                    name: row.get(5)?,
                    display_name: row.get(6)?,
                    data_type: row.get(7)?,
                    scale: row.get(8)?,
                    deadband: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(points)
        }).await.context("Task join failed")?
    }

    pub async fn delete_dnp3_point(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let rows = conn.execute(
                "DELETE FROM dnp3_points WHERE id = ?1",
                rusqlite::params![id],
            )?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }
}
