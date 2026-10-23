use super::*;

/// Run the full backup workflow: SQLite snapshot -> tar.gz archive.
pub fn run_backup(opts: BackupOptions) -> Result<String> {
    // 1. Validate that the database file exists.
    if !Path::new(&opts.db_path).exists() {
        anyhow::bail!(
            "Database file not found: {}. Start the server first to create it.",
            opts.db_path
        );
    }

    // 2. Create a consistent SQLite backup to a temporary file.
    println!("Backing up database...");
    let temp_db = create_sqlite_backup(&opts.db_path)?;
    let db_size = fs::metadata(&temp_db)
        .with_context(|| format!("Failed to stat temp db: {}", temp_db))?
        .len();

    // 3. Determine output path.
    let output_path = match opts.output_path {
        Some(ref p) => p.clone(),
        None => format!("voltrus-backup-{}.tar.gz", chrono_now_timestamp()),
    };

    // 4. Build the tar.gz archive.
    println!("Creating archive: {}", output_path);
    create_archive(&temp_db, &opts.config_path, db_size, &output_path)?;

    // 5. Clean up temp file.
    let _ = fs::remove_file(&temp_db);

    // 6. Report result.
    let archive_size = fs::metadata(&output_path)
        .with_context(|| format!("Failed to stat archive: {}", output_path))?
        .len();
    let size_human = format_size(archive_size);
    println!("Backup complete: {} ({})", output_path, size_human);

    Ok(output_path)
}
