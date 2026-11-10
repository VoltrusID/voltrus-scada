//! Repair of data sources that name their device instead of giving its id.
//!
//! Tags added to S7, OPC UA and EtherNet/IP devices used to create their data
//! source with the device's name as `device_id`. Writes and device lookups
//! need the numeric id, so those tags could not be written. Runs at every
//! start; a repaired row is numeric and is not looked at again.

use rusqlite::Connection;

/// Data source type and the device table its `device_id` refers to.
const PROTOCOL_TABLES: [(&str, &str); 3] = [
    ("s7", "s7_devices"),
    ("opcua", "opcua_devices"),
    ("enip", "enip_devices"),
];

pub(super) fn repair_device_refs(conn: &Connection) {
    for (source_type, table) in PROTOCOL_TABLES {
        if let Err(e) = repair(conn, source_type, table) {
            tracing::warn!("Repairing {source_type} data source device ids failed: {e}");
        }
    }
}

fn repair(conn: &Connection, source_type: &str, table: &str) -> rusqlite::Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let mut stmt = conn.prepare(
        "SELECT id, key, device_id FROM data_sources
         WHERE source_type = ?1 AND (device_id = '' OR device_id GLOB '*[^0-9]*')",
    )?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([source_type], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut find = conn.prepare(&format!("SELECT id FROM {table} WHERE name = ?1 LIMIT 2"))?;
    for (id, key, name) in rows {
        let ids: Vec<i64> = find
            .query_map([&name], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        match ids.as_slice() {
            [device] => {
                conn.execute(
                    "UPDATE data_sources SET device_id = ?1 WHERE id = ?2",
                    rusqlite::params![device.to_string(), id],
                )?;
                tracing::info!("Data source '{key}': device '{name}' is now device id {device}");
            }
            [] => tracing::warn!(
                "Data source '{key}' names {source_type} device '{name}', which does not exist; \
                 left as is"
            ),
            _ => tracing::warn!(
                "Data source '{key}' names {source_type} device '{name}', which several devices \
                 are called; left as is"
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_id_of(conn: &Connection, key: &str) -> String {
        conn.query_row(
            "SELECT device_id FROM data_sources WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn names_become_ids_when_unambiguous() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::init_schema(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO s7_devices (id, name, url, created_at) VALUES (3, 'PLC1', 'x:102', 0);
             INSERT INTO s7_devices (id, name, url, created_at) VALUES (4, 'Twin', 'x:102', 0);
             INSERT INTO s7_devices (id, name, url, created_at) VALUES (5, 'Twin', 'y:102', 0);
             INSERT INTO opcua_devices (id, name, url, created_at) VALUES (7, 'Server', 'opc.tcp://x', 0);",
        )
        .unwrap();
        for (key, source_type, device) in [
            ("s7_PLC1:a", "s7", "PLC1"),
            ("s7_Twin:b", "s7", "Twin"),
            ("s7_Gone:c", "s7", "Gone"),
            ("Server:d", "opcua", "Server"),
            ("ok", "s7", "3"),
            ("modbus_tag", "modbus", "PLC1"),
        ] {
            conn.execute(
                "INSERT INTO data_sources (key, name, unit, color, min_val, max_val, warn_val,
                 crit_val, base_val, sort_order, decimals, source_type, source_config, device_id,
                 chart_type) VALUES (?1, ?1, '', '', 0, 0, 0, 0, 0, 0, 1, ?2, '{}', ?3, 'sparkline')",
                rusqlite::params![key, source_type, device],
            )
            .unwrap();
        }

        repair_device_refs(&conn);
        repair_device_refs(&conn);

        assert_eq!(device_id_of(&conn, "s7_PLC1:a"), "3");
        assert_eq!(device_id_of(&conn, "s7_Twin:b"), "Twin", "ambiguous");
        assert_eq!(device_id_of(&conn, "s7_Gone:c"), "Gone", "no such device");
        assert_eq!(device_id_of(&conn, "Server:d"), "7");
        assert_eq!(device_id_of(&conn, "ok"), "3");
        assert_eq!(
            device_id_of(&conn, "modbus_tag"),
            "PLC1",
            "Modbus ids are text"
        );
    }

    #[cfg(feature = "s7")]
    #[tokio::test]
    async fn new_tags_give_the_device_id() {
        let storage = crate::storage::Storage::new_in_memory().unwrap();
        let device = storage
            .create_s7_device("PLC1", "127.0.0.1:102", 0, 1, 1000, true)
            .await
            .unwrap();
        storage
            .create_s7_tag(device, "PLC1", "DB1.DBD0", "temp", "", "real")
            .await
            .unwrap();
        let ds = storage
            .find_data_source_by_device_and_key(&device.to_string(), "s7_PLC1:temp")
            .await
            .unwrap()
            .expect("data source on the device id");
        let target = storage.get_data_source_device_id(ds.id).await.unwrap();
        assert_eq!(
            target.map(|t| (t.device_id, t.protocol)),
            Some((device, "s7".into()))
        );
    }
}
