mod migrations;
mod tables;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn init_schema(conn: &Connection) -> Result<()> {
    tables::create_tables(conn).context("Failed to create tables")?;
    migrations::run_migrations(conn);
    Ok(())
}
