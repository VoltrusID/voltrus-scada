mod canonical_values;
mod device_refs;
mod migrations;
mod tables;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn init_schema(conn: &Connection) -> Result<()> {
    tables::create_tables(conn).context("Failed to create tables")?;
    migrations::run_migrations(conn);
    migrations::add_login_failures(conn).context("Failed to add the login_failures table")?;
    device_refs::repair_device_refs(conn);
    canonical_values::repair_canonical_values(conn);
    Ok(())
}
