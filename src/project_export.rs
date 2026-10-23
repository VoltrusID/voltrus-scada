// ── Project Export / Import ──
//
// Enables the offline engineering workflow: an engineer configures a project on
// their laptop, exports a `.voltrus-scada` archive, and
// imports it on a production server.
//
// Archive layout (tar.gz):
//   manifest.json      – metadata (entity counts, version info)
//   config.sqlite      – consistent SQLite snapshot of the config database

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rusqlite::Connection;
use rusqlite::backup::Backup;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::Path;

// ── Manifest ──

/// Metadata embedded in every `.voltrus-scada` archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    /// Manifest schema version (currently "1.0").
    pub version: String,
    /// ISO-8601 timestamp of when the export was created.
    pub exported_at: String,
    /// Voltrus version that produced the export.
    pub voltrus_version: String,
    /// Number of devices in the exported project.
    pub device_count: usize,
    /// Number of tags in the exported project.
    pub tag_count: usize,
    /// Number of screens in the exported project.
    pub screen_count: usize,
}

/// Tables copied during import.
const IMPORT_TABLES: &[&str] = &[
    "devices",
    "data_sources",
    "screens",
    "alarms",
    "flows",
    "settings",
    "annotations",
    "shift_handover",
    "user_notifications",
    "roles",
    "permissions",
    "role_permissions",
    "user_roles",
    "equipment_templates",
    "egress_jobs",
    "webhooks",
    "api_keys",
    "audit_log",
    "notification_rules",
    "report_schedules",
    "sim_profiles",
    "hub_sites",
    "cameras",
    "recipes",
    "recipe_parameters",
    "script_proposals",
    "ai_settings",
    "naming_conventions",
    "branding_settings",
    "compression_settings",
    "retention_settings",
];

// ── Export ──

/// Create a `.voltrus-scada` archive from the given database.
///
/// The function performs a consistent SQLite backup, counts entities, writes a
/// manifest, and packs everything into a gzipped tar archive.
pub fn export_project(db_path: &str, output_path: &str) -> Result<ProjectManifest> {
    // 1. Consistent SQLite snapshot via the backup API.
    let temp_db =
        create_sqlite_snapshot(db_path).context("Failed to create SQLite snapshot for export")?;

    let result = (|| -> Result<ProjectManifest> {
        // 2. Open the snapshot and count entities.
        let conn = Connection::open(&temp_db)
            .with_context(|| format!("Failed to open snapshot: {}", temp_db))?;

        let device_count = count_rows(&conn, "devices");
        let tag_count = count_rows(&conn, "data_sources");
        let screen_count = count_rows(&conn, "screens");

        let manifest = ProjectManifest {
            version: "1.0".to_string(),
            exported_at: now_iso8601(),
            voltrus_version: env!("CARGO_PKG_VERSION").to_string(),
            device_count,
            tag_count,
            screen_count,
        };

        // 3. Build tar.gz archive.
        let archive_file = File::create(output_path)
            .with_context(|| format!("Failed to create {}", output_path))?;
        let encoder = GzEncoder::new(archive_file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);

        // Add config.sqlite snapshot.
        builder
            .append_path_with_name(&temp_db, "config.sqlite")
            .context("Failed to add config.sqlite to archive")?;

        // Add manifest.json.
        let manifest_json =
            serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;
        let mut header = tar::Header::new_gnu();
        let manifest_bytes = manifest_json.as_bytes();
        header.set_size(manifest_bytes.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, "manifest.json", manifest_bytes)
            .context("Failed to add manifest.json to archive")?;

        builder
            .into_inner()
            .context("Failed to finish tar builder")?
            .finish()
            .context("Failed to finish gzip compression")?;

        Ok(manifest)
    })();

    // Clean up temp snapshot regardless of success/failure.
    let _ = fs::remove_file(&temp_db);

    result
}

// ── Import ──

/// Import a `.voltrus-scada` archive into the target database.
///
/// When `dry_run` is true the archive is validated and the manifest is returned
/// but **no data is written** to the target database.
pub fn import_project(db_path: &str, input_path: &str, dry_run: bool) -> Result<ProjectManifest> {
    // 1. Validate archive exists and is a valid gzip.
    if !Path::new(input_path).exists() {
        bail!("Import archive not found: {}", input_path);
    }

    let file = File::open(input_path)
        .with_context(|| format!("Failed to open import archive: {}", input_path))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // 2. Extract to a temp directory.
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory for extraction")?;
    archive
        .unpack(temp_dir.path())
        .context("Failed to extract archive")?;

    // 3. Read manifest.
    let manifest_path = temp_dir.path().join("manifest.json");
    if !manifest_path.exists() {
        bail!("manifest.json not found in archive — not a valid .voltrus-scada file");
    }
    let manifest_bytes = fs::read(&manifest_path).context("Failed to read manifest.json")?;
    let manifest: ProjectManifest =
        serde_json::from_slice(&manifest_bytes).context("Failed to parse manifest.json")?;

    // 4. Validate config.sqlite exists in archive.
    let snapshot_path = temp_dir.path().join("config.sqlite");
    if !snapshot_path.exists() {
        bail!("config.sqlite not found in archive — not a valid .voltrus-scada file");
    }

    // If dry-run, stop here and return the manifest.
    if dry_run {
        return Ok(manifest);
    }

    // 6. ATTACH the snapshot and copy tables.
    let conn = Connection::open(db_path)
        .with_context(|| format!("Failed to open target database: {}", db_path))?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .context("Failed to set WAL mode")?;

    let attach_sql = format!(
        "ATTACH DATABASE '{}' AS export_db;",
        snapshot_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid snapshot path"))?
    );
    conn.execute_batch(&attach_sql)
        .context("Failed to ATTACH export database")?;

    // Copy each table (create if not exists in target, then replace).
    for table in IMPORT_TABLES {
        // Check if table exists in the export DB.
        let table_exists: bool = conn
            .query_row(
                "SELECT count(*) FROM export_db.sqlite_master WHERE type='table' AND name=?",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !table_exists {
            continue; // Table not present in export — skip silently.
        }

        // Verify the table has a valid schema definition.
        let _create_sql: String = match conn.query_row(
            "SELECT sql FROM export_db.sqlite_master WHERE type='table' AND name=?",
            [table],
            |row| row.get(0),
        ) {
            Ok(sql) => sql,
            Err(_) => continue,
        };

        // Create the table in the main database if it doesn't exist yet.
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS main.\"{table}\" AS SELECT * FROM export_db.\"{table}\" WHERE 0;"
        )).ok(); // Ignore error — table may already exist with different schema.

        // Try the proper CREATE TABLE approach: drop existing and recreate.
        // Actually, safer: delete existing rows and insert from export.
        let _ = conn.execute_batch(&format!("DELETE FROM main.\"{table}\";"));

        // Get column list from the export table.
        let columns: Vec<String> = {
            let mut stmt = conn
                .prepare(&format!("PRAGMA export_db.table_info(\"{table}\")"))
                .context("Failed to get table info")?;
            let col_rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .context("Failed to read column names")?;
            col_rows.filter_map(|c| c.ok()).collect()
        };

        if columns.is_empty() {
            continue;
        }

        let col_list = columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let sel_list = columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");

        let insert_sql = format!(
            "INSERT INTO main.\"{table}\" ({col_list}) SELECT {sel_list} FROM export_db.\"{table}\";"
        );

        if let Err(e) = conn.execute_batch(&insert_sql) {
            tracing::warn!("Skipping table '{table}' during import: {e}");
        }
    }

    conn.execute_batch("DETACH DATABASE export_db;")
        .context("Failed to DETACH export database")?;

    Ok(manifest)
}

// ── Helpers ──

/// Create a consistent snapshot of the SQLite database using the backup API.
fn create_sqlite_snapshot(db_path: &str) -> Result<String> {
    let src = Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path))?;

    let temp_path = format!("{}.export.tmp", db_path);
    let mut dst = Connection::open(&temp_path)
        .with_context(|| format!("Failed to create temp snapshot: {}", temp_path))?;

    let backup = Backup::new(&src, &mut dst).context("Failed to initialize SQLite backup")?;
    backup
        .run_to_completion(5, std::time::Duration::from_millis(250), None)
        .context("SQLite backup failed")?;

    Ok(temp_path)
}

/// Count rows in a table (returns 0 if the table does not exist).
fn count_rows(conn: &Connection, table: &str) -> usize {
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
        row.get::<_, usize>(0)
    })
    .unwrap_or(0)
}

/// Return an ISO-8601 timestamp for the current time in UTC.
fn now_iso8601() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            let secs = d.as_secs();
            // Simple ISO-8601-ish formatting without depending on chrono.
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                1970 + (secs / 31556952) as u32, // approximate year
                1 + ((secs % 31556952) / 2629746) as u32, // approximate month
                1 + ((secs % 2629746) / 86400) as u32, // approximate day
                (secs % 86400) / 3600,
                (secs % 3600) / 60,
                secs % 60,
            )
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let manifest = ProjectManifest {
            version: "1.0".to_string(),
            exported_at: "2026-05-17T12:00:00Z".to_string(),
            voltrus_version: "0.47.0".to_string(),
            device_count: 10,
            tag_count: 50,
            screen_count: 5,
        };

        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: ProjectManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest.version, deserialized.version);
        assert_eq!(manifest.device_count, deserialized.device_count);
        assert_eq!(manifest.tag_count, deserialized.tag_count);
        assert_eq!(manifest.screen_count, deserialized.screen_count);
    }

    #[test]
    fn test_export_and_import_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
        let archive_path = dir
            .path()
            .join("export.voltrus-scada")
            .to_str()
            .unwrap()
            .to_string();
        let import_db_path = dir.path().join("import.db").to_str().unwrap().to_string();

        // Create source database with some test data.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE devices (id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO devices (name) VALUES ('device1');
                 INSERT INTO devices (name) VALUES ('device2');
                 CREATE TABLE data_sources (id INTEGER PRIMARY KEY, tag TEXT);
                 INSERT INTO data_sources (tag) VALUES ('tag1');
                 INSERT INTO data_sources (tag) VALUES ('tag2');
                 INSERT INTO data_sources (tag) VALUES ('tag3');
                 CREATE TABLE screens (id INTEGER PRIMARY KEY, title TEXT);
                 INSERT INTO screens (title) VALUES ('screen1');",
            )
            .unwrap();
        }

        // Export.
        let manifest = export_project(&db_path, &archive_path).unwrap();
        assert_eq!(manifest.device_count, 2);
        assert_eq!(manifest.tag_count, 3);
        assert_eq!(manifest.screen_count, 1);
        assert!(Path::new(&archive_path).exists());

        // Create empty target database with required table schemas.
        {
            let conn = Connection::open(&import_db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE devices (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE data_sources (id INTEGER PRIMARY KEY, tag TEXT);
                 CREATE TABLE screens (id INTEGER PRIMARY KEY, title TEXT);",
            )
            .unwrap();
        }

        // Dry-run import.
        let dry_manifest = import_project(&import_db_path, &archive_path, true).unwrap();
        assert_eq!(dry_manifest.device_count, 2);

        // Verify target DB still empty after dry-run.
        {
            let conn = Connection::open(&import_db_path).unwrap();
            let count: i64 = conn
                .query_row("SELECT count(*) FROM devices", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }

        // Real import.
        let import_manifest = import_project(&import_db_path, &archive_path, false).unwrap();
        assert_eq!(import_manifest.device_count, 2);

        // Verify imported data.
        {
            let conn = Connection::open(&import_db_path).unwrap();
            let dev_count: i64 = conn
                .query_row("SELECT count(*) FROM devices", [], |r| r.get(0))
                .unwrap();
            assert_eq!(dev_count, 2);
            let tag_count: i64 = conn
                .query_row("SELECT count(*) FROM data_sources", [], |r| r.get(0))
                .unwrap();
            assert_eq!(tag_count, 3);
            let screen_count: i64 = conn
                .query_row("SELECT count(*) FROM screens", [], |r| r.get(0))
                .unwrap();
            assert_eq!(screen_count, 1);
        }
    }
}
