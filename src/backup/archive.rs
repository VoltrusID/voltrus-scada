use super::*;

/// Validate that the given file is a valid gzip archive.
pub(crate) fn validate_archive(path: &str) -> Result<()> {
    if !Path::new(path).exists() {
        anyhow::bail!("Archive not found: {}", path);
    }

    let file = File::open(path).with_context(|| format!("Failed to open archive: {}", path))?;
    let decoder = GzDecoder::new(file);
    // Try to read the first byte to verify gzip validity.
    let mut buf = [0u8; 1];
    let mut reader = std::io::BufReader::new(decoder);
    reader
        .read_exact(&mut buf)
        .context("File is not a valid gzip archive")?;

    Ok(())
}

/// Read and parse backup.json from the archive without fully extracting.
pub(crate) fn read_metadata_from_archive(archive_path: &str) -> Result<BackupMetadata> {
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let mut metadata_content = String::new();
    let mut found = false;

    for entry in archive
        .entries()
        .context("Failed to read archive entries")?
    {
        let mut entry = entry.context("Failed to read archive entry")?;
        let path = entry
            .path()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if path == "backup.json" {
            entry
                .read_to_string(&mut metadata_content)
                .context("Failed to read backup.json from archive")?;
            found = true;
            break;
        }
    }

    if !found {
        anyhow::bail!("backup.json not found in archive");
    }

    let metadata: BackupMetadata =
        serde_json::from_str(&metadata_content).context("Failed to parse backup.json metadata")?;

    Ok(metadata)
}

pub(crate) fn create_sqlite_backup(db_path: &str) -> Result<String> {
    let src = Connection::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path))?;
    let temp_path = format!("{}.backup.tmp", db_path);
    let mut dst = Connection::open(&temp_path)
        .with_context(|| format!("Failed to create temp database: {}", temp_path))?;

    let backup = Backup::new(&src, &mut dst).context("Failed to initialize SQLite backup")?;
    backup
        .run_to_completion(5, std::time::Duration::from_millis(250), None)
        .context("SQLite backup failed")?;

    Ok(temp_path)
}

/// Build a tar.gz archive containing the database backup, config, and metadata.
pub(crate) fn create_archive(
    temp_db_path: &str,
    config_path: &str,
    db_size_bytes: u64,
    output_path: &str,
) -> Result<()> {
    let archive_file = File::create(output_path).context("Failed to create archive file")?;
    let encoder = GzEncoder::new(archive_file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);

    // Add database backup.
    builder
        .append_path_with_name(temp_db_path, "data.db")
        .context("Failed to add database to archive")?;
    println!("Archiving database...");

    // Add config file (if it exists).
    println!("Archiving config...");
    if Path::new(config_path).exists() {
        builder
            .append_path_with_name(config_path, "config.yaml")
            .context("Failed to add config to archive")?;
    } else {
        // Write an empty placeholder so the restore side can find the key.
        let mut empty = File::create("/dev/null").context("Failed to open /dev/null")?;
        writeln!(empty).ok();
        // Use append_data instead so we don't need a real file on disk.
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_cksum();
        builder
            .append_data(&mut header, "config.yaml", std::io::empty())
            .context("Failed to add empty config placeholder")?;
    }

    // Add metadata JSON.
    println!("Archiving metadata...");
    let metadata = BackupMetadata {
        version: "1.0".to_string(),
        timestamp: chrono_timestamp(),
        db_size_bytes,
        voltrus_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("Failed to serialize backup metadata")?;
    let mut meta_header = tar::Header::new_gnu();
    let meta_bytes = metadata_json.as_bytes();
    meta_header.set_size(meta_bytes.len() as u64);
    meta_header.set_cksum();
    builder
        .append_data(&mut meta_header, "backup.json", meta_bytes)
        .context("Failed to add metadata to archive")?;

    // Finish the archive (this also finishes the GzEncoder).
    builder
        .into_inner()
        .context("Failed to finish tar builder")?
        .finish()
        .context("Failed to finish gzip compression")?;

    Ok(())
}
