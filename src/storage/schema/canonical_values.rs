//! Startup repair: device values stored before the REST routes and the
//! project format checked them, rewritten to the canonical value the pollers
//! already read them as, so a project export imports and validates again.
//!
//! * `modbus_devices.endianness`: "" and other unknown values poll as `big`.
//! * `modbus_registers.register_type`: unknown values poll as `float32`.
//! * `bacnet_objects.object_type`: abbreviations and other spellings the
//!   poller accepts ("AI", "analoginput") become "analogInput". A value the
//!   poller does not read is left as it is.

use rusqlite::Connection;

use crate::device_templates::{MODBUS_TYPES, bacnet_object_type};
use crate::models::Endianness;

pub(super) fn repair_canonical_values(conn: &Connection) {
    match repair(conn) {
        Ok(0) => {}
        Ok(n) => tracing::info!("Rewrote {n} stored device value(s) to their canonical form"),
        Err(e) => tracing::error!("Failed to canonicalise stored device values: {e}"),
    }
}

fn repair(conn: &Connection) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut n = 0;

    let devices: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT id, endianness FROM modbus_devices")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, stored) in devices {
        let canonical = Endianness::from_name(&stored).name();
        if canonical != stored {
            n += tx.execute(
                "UPDATE modbus_devices SET endianness = ?2 WHERE id = ?1",
                rusqlite::params![id, canonical],
            )?;
        }
    }

    let known = MODBUS_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    n += tx.execute(
        &format!(
            "UPDATE modbus_registers SET register_type = 'float32' WHERE register_type NOT IN ({known})"
        ),
        [],
    )?;

    let objects: Vec<(i64, String)> = {
        let mut stmt = tx.prepare("SELECT id, object_type FROM bacnet_objects")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, stored) in objects {
        if let Some(canonical) = bacnet_object_type(&stored).filter(|c| *c != stored) {
            // OR IGNORE: the same object stored under both spellings keeps
            // the alias row rather than failing the whole repair.
            n += tx.execute(
                "UPDATE OR IGNORE bacnet_objects SET object_type = ?2 WHERE id = ?1",
                rusqlite::params![id, canonical],
            )?;
        }
    }

    tx.commit()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_become_what_the_pollers_read() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::init_schema(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO modbus_devices (id, name, ip, port, unit_id, interval_ms, endianness, enabled, created_at, updated_at)
                VALUES ('a', 'a', '1.2.3.4', 502, 1, 1000, '', 1, 0, 0),
                       ('b', 'b', '1.2.3.4', 502, 1, 1000, 'swapped', 1, 0, 0);
             INSERT INTO modbus_registers (device_id, name, address, register_type, register_area, scale, sort_order)
                VALUES ('a', 'x', 0, 'FLOAT', 'holding', 1.0, 0), ('a', 'y', 2, 'int16', 'holding', 1.0, 1);
             INSERT INTO bacnet_devices (name, ip, created_at) VALUES ('ahu', '1.2.3.5', 0);
             INSERT INTO bacnet_objects (device_id, object_type, object_instance, name, created_at)
                VALUES (1, 'AI', 1, 'temp', 0), (1, 'binaryValue', 2, 'run', 0), (1, 'weird', 3, 'w', 0);",
        )
        .unwrap();
        assert_eq!(repair(&conn).unwrap(), 3);
        let one = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(
            one("SELECT endianness FROM modbus_devices WHERE id = 'a'"),
            "big"
        );
        assert_eq!(
            one("SELECT endianness FROM modbus_devices WHERE id = 'b'"),
            "swapped"
        );
        assert_eq!(
            one("SELECT register_type FROM modbus_registers WHERE name = 'x'"),
            "float32"
        );
        assert_eq!(
            one("SELECT register_type FROM modbus_registers WHERE name = 'y'"),
            "int16"
        );
        assert_eq!(
            one("SELECT object_type FROM bacnet_objects WHERE name = 'temp'"),
            "analogInput"
        );
        assert_eq!(
            one("SELECT object_type FROM bacnet_objects WHERE name = 'w'"),
            "weird"
        );
        assert_eq!(repair(&conn).unwrap(), 0, "a second start changes nothing");
    }
}
