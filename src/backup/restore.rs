use super::*;

/// Run the full restore workflow: validate archive -> extract -> verify -> replace.
pub fn run_restore(opts: RestoreOptions) -> Result<()> {
    // 1. Validate the archive exists and is a valid .tar.gz.
    println!("Validating archive: {}", opts.archive_path);
    validate_archive(&opts.archive_path)?;

    // 2. Open and list archive contents.
    let archive_file = File::open(&opts.archive_path)
        .with_context(|| format!("Failed to open archive: {}", opts.archive_path))?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);

    // Collect entries to inspect them.
    let entries: Vec<(String, u64)> = archive
        .entries()
        .context("Failed to read archive entries")?
        .filter_map(|e| e.ok())
        .map(|entry| {
            let path = entry
                .path()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let size = entry.size();
            (path, size)
        })
        .collect();

    let has_data_db = entries.iter().any(|(p, _)| p == "data.db");
    let has_backup_json = entries.iter().any(|(p, _)| p == "backup.json");
    let has_config = entries.iter().any(|(p, _)| p == "config.yaml");

    if !has_data_db {
        anyhow::bail!("Archive is missing required file: data.db");
    }
    if !has_backup_json {
        anyhow::bail!("Archive is missing required file: backup.json");
    }

    // 3. Read and display backup.json metadata.
    let metadata = read_metadata_from_archive(&opts.archive_path)?;
    println!();
    println!("Backup metadata:");
    println!("  Version:        {}", metadata.version);
    println!("  Timestamp:      {}", metadata.timestamp);
    println!("  Database size:  {}", format_size(metadata.db_size_bytes));
    println!("  Voltrus version: {}", metadata.voltrus_version);
    println!();

    // 4. Version compatibility check.
    let current_version = env!("CARGO_PKG_VERSION");
    let backup_major = parse_major_version(&metadata.voltrus_version);
    let current_major = parse_major_version(current_version);
    if backup_major != current_major {
        println!(
            "WARNING: Major version mismatch — backup was created with voltrus {}, current is {}.",
            metadata.voltrus_version, current_version
        );
        println!("  Restoring may cause issues if the database schema changed.");
        println!();
    }

    // 5. Check for running server (look for the database lock file).
    let lock_path = format!("{}-journal", opts.db_path);
    if Path::new(&lock_path).exists() || Path::new(&opts.db_path).exists() {
        // Try to open the DB exclusively — if it fails, the server likely holds it.
        if let Ok(conn) = Connection::open_with_flags(
            &opts.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            // Check if there are active readers/writers
            let is_wal = Path::new(&format!("{}-wal", opts.db_path)).exists();
            drop(conn);
            if is_wal {
                println!("WARNING: Database WAL file exists. The server may be running.");
                println!("  It is recommended to stop the server before restoring.");
                println!();
            }
        }
    }

    // 6. Confirmation prompt (unless --force).
    if !opts.force {
        println!("This will replace the current database and config at:");
        println!("  Database: {}", opts.db_path);
        println!("  Config:   {}", opts.config_path);
        println!();
        print!("Continue? [y/N] ");
        std::io::stdout()
            .flush()
            .context("Failed to flush stdout")?;

        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("Failed to read user input")?;
        let answer = input.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            println!("Restore cancelled.");
            return Ok(());
        }
    }

    // 7. Create pre-restore backup of current state.
    if Path::new(&opts.db_path).exists() {
        println!("Creating pre-restore backup...");
        let pre_restore_path = format!("voltrus-pre-restore-{}.tar.gz", chrono_now_timestamp());
        let pre_restore_opts = BackupOptions {
            config_path: opts.config_path.clone(),
            db_path: opts.db_path.clone(),
            output_path: Some(pre_restore_path.clone()),
        };
        if let Err(e) = run_backup(pre_restore_opts) {
            println!("WARNING: Failed to create pre-restore backup: {}", e);
            println!("  Continuing anyway. Proceed at your own risk.");
        } else {
            println!("Pre-restore backup saved: {}", pre_restore_path);
        }
    }

    // 8. Extract to a temporary directory.
    println!("Extracting archive...");
    let temp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;

    let archive_file = File::open(&opts.archive_path)
        .with_context(|| format!("Failed to open archive: {}", opts.archive_path))?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .context("Failed to read archive entries")?
    {
        let mut entry = entry.context("Failed to read archive entry")?;
        let path = entry.path().context("Failed to read entry path")?;
        // Reject paths with directory traversal or non-normal components
        let safe = path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !safe {
            anyhow::bail!(
                "Archive contains unsafe path: {}. Refusing to extract.",
                path.display()
            );
        }
        entry
            .unpack_in(temp_dir.path())
            .context("Failed to extract archive entry")?;
    }

    let extracted_db = temp_dir.path().join("data.db");
    let extracted_config = temp_dir.path().join("config.yaml");

    if !extracted_db.exists() {
        anyhow::bail!("Extracted archive is missing data.db");
    }

    // 9. Verify database integrity.
    println!("Verifying database integrity...");
    let conn =
        Connection::open(&extracted_db).with_context(|| "Failed to open extracted database")?;
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("Failed to run integrity check")?;
    drop(conn);

    if result != "ok" {
        anyhow::bail!(
            "Database integrity check failed: {}. Aborting restore.",
            result
        );
    }
    println!("  Integrity check: OK");

    // 10. Verify the restored database opens correctly with a simple query.
    let conn = Connection::open(&extracted_db)
        .context("Failed to re-open extracted database for verification")?;
    // Just verify we can query it — don't assume any particular schema.
    let _: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' LIMIT 1")
        .context("Failed to query extracted database")?
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    drop(conn);

    // 11. Replace the database file.
    println!("Replacing database: {}", opts.db_path);
    if let Some(parent) = Path::new(&opts.db_path).parent() {
        fs::create_dir_all(parent).context("Failed to create database directory")?;
    }
    // Extract to a temp path adjacent to target (same filesystem for atomic rename)
    let temp_target = format!("{}.restoring", opts.db_path);
    fs::copy(&extracted_db, &temp_target)
        .with_context(|| format!("Failed to prepare database at {}", temp_target))?;
    fs::rename(&temp_target, &opts.db_path)
        .with_context(|| format!("Failed to replace database at {}", opts.db_path))?;

    let restored_size = fs::metadata(&opts.db_path)
        .with_context(|| format!("Failed to stat restored database: {}", opts.db_path))?
        .len();
    println!("  Restored database size: {}", format_size(restored_size));

    // 12. Restore config.yaml if present and non-empty in the archive.
    if has_config && extracted_config.exists() {
        let config_content = fs::read_to_string(&extracted_config).unwrap_or_default();
        if !config_content.trim().is_empty() {
            println!("Restoring config: {}", opts.config_path);
            if let Some(parent) = Path::new(&opts.config_path).parent() {
                fs::create_dir_all(parent).context("Failed to create config directory")?;
            }
            fs::copy(&extracted_config, &opts.config_path)
                .with_context(|| format!("Failed to copy config to {}", opts.config_path))?;
        } else {
            println!("Config in archive is empty, skipping config restore.");
        }
    } else {
        println!("No config in archive, skipping config restore.");
    }

    // 13. Final verification — open the restored database.
    println!("Verifying restored database...");
    let verify_conn = Connection::open(&opts.db_path)
        .with_context(|| format!("Failed to open restored database: {}", opts.db_path))?;
    let verify_result: String = verify_conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("Failed to verify restored database")?;
    drop(verify_conn);

    if verify_result != "ok" {
        anyhow::bail!(
            "Restored database integrity check failed: {}",
            verify_result
        );
    }
    println!("  Restored database integrity: OK");

    println!();
    println!("Restore complete. You may start the server.");

    Ok(())
}
