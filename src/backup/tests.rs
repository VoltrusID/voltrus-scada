use super::*;
use std::io::Write;

#[test]
fn test_backup_creates_archive() {
    // Create a temporary directory with a SQLite database.
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("data.db");
    let config_path = tmp_dir.path().join("config.yaml");

    // Create a simple database with a table.
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)", [])
        .unwrap();
    conn.execute("INSERT INTO test (val) VALUES ('hello')", [])
        .unwrap();
    drop(conn);

    // Write a config file.
    let mut f = File::create(&config_path).unwrap();
    writeln!(f, "server_port: 8080").unwrap();
    drop(f);

    // Run backup.
    let output_path = tmp_dir.path().join("backup.tar.gz");
    let opts = BackupOptions {
        config_path: config_path.to_str().unwrap().to_string(),
        db_path: db_path.to_str().unwrap().to_string(),
        output_path: Some(output_path.to_str().unwrap().to_string()),
    };
    let result = run_backup(opts);
    assert!(result.is_ok());

    // Verify archive exists and is non-empty.
    assert!(output_path.exists());
    let meta = fs::metadata(&output_path).unwrap();
    assert!(meta.len() > 0);

    // Extract the archive and verify contents.
    let archive_file = File::open(&output_path).unwrap();
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    let restore_dir = tempfile::tempdir().unwrap();
    archive.unpack(restore_dir.path()).unwrap();

    // Verify expected files exist in the extracted archive.
    assert!(restore_dir.path().join("data.db").exists());
    assert!(restore_dir.path().join("config.yaml").exists());
    assert!(restore_dir.path().join("backup.json").exists());

    // Verify backup.json metadata.
    let content = fs::read_to_string(restore_dir.path().join("backup.json")).unwrap();
    let backup_meta: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(backup_meta["version"], "1.0");
    assert_eq!(backup_meta["voltrus_version"], env!("CARGO_PKG_VERSION"));
    assert!(backup_meta["timestamp"].as_str().unwrap().len() > 0);
    assert!(backup_meta["db_size_bytes"].as_u64().unwrap() > 0);

    // Verify config.yaml content.
    let cfg_content = fs::read_to_string(restore_dir.path().join("config.yaml")).unwrap();
    assert!(cfg_content.contains("server_port: 8080"));

    // Verify the backed-up database is readable and has our data.
    let conn2 = Connection::open(restore_dir.path().join("data.db")).unwrap();
    let val: String = conn2
        .query_row("SELECT val FROM test WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(val, "hello");
}

#[test]
fn test_backup_fails_when_db_missing() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let opts = BackupOptions {
        config_path: tmp_dir
            .path()
            .join("config.yaml")
            .to_str()
            .unwrap()
            .to_string(),
        db_path: tmp_dir
            .path()
            .join("nonexistent.db")
            .to_str()
            .unwrap()
            .to_string(),
        output_path: None,
    };
    let result = run_backup(opts);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Database file not found")
    );
}

#[test]
fn test_backup_without_config_file() {
    // Backup should work even if config file doesn't exist.
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("data.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE t (x)", []).unwrap();
    drop(conn);

    let output_path = tmp_dir.path().join("backup.tar.gz");
    let opts = BackupOptions {
        config_path: tmp_dir
            .path()
            .join("nope.yaml")
            .to_str()
            .unwrap()
            .to_string(),
        db_path: db_path.to_str().unwrap().to_string(),
        output_path: Some(output_path.to_str().unwrap().to_string()),
    };
    let result = run_backup(opts);
    assert!(result.is_ok());
    assert!(output_path.exists());
}

#[test]
fn test_format_size() {
    assert_eq!(format_size(500), "500 B");
    assert_eq!(format_size(2048), "2.00 KB");
    assert_eq!(format_size(3 * 1024 * 1024), "3.00 MB");
    assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GB");
}

#[test]
fn test_restore_from_valid_archive() {
    // Create a backup first, then restore from it.
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("data.db");
    let config_path = tmp_dir.path().join("config.yaml");

    // Create source database with known data.
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)", [])
        .unwrap();
    conn.execute("INSERT INTO test (val) VALUES ('restored')", [])
        .unwrap();
    drop(conn);

    // Write config.
    let mut f = File::create(&config_path).unwrap();
    writeln!(f, "server_port: 8080").unwrap();
    drop(f);

    // Create backup archive.
    let archive_path = tmp_dir.path().join("backup.tar.gz");
    let backup_opts = BackupOptions {
        config_path: config_path.to_str().unwrap().to_string(),
        db_path: db_path.to_str().unwrap().to_string(),
        output_path: Some(archive_path.to_str().unwrap().to_string()),
    };
    assert!(run_backup(backup_opts).is_ok());
    assert!(archive_path.exists());

    // Set up restore target paths (different from source).
    let restore_dir = tempfile::tempdir().unwrap();
    let target_db = restore_dir.path().join("data.db");
    let target_config = restore_dir.path().join("config.yaml");

    // Restore with --force (skip prompt).
    let restore_opts = RestoreOptions {
        archive_path: archive_path.to_str().unwrap().to_string(),
        config_path: target_config.to_str().unwrap().to_string(),
        db_path: target_db.to_str().unwrap().to_string(),
        force: true,
    };
    let result = run_restore(restore_opts);
    assert!(result.is_ok());

    // Verify restored database has the original data.
    assert!(target_db.exists());
    let conn2 = Connection::open(&target_db).unwrap();
    let val: String = conn2
        .query_row("SELECT val FROM test WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(val, "restored");
    drop(conn2);

    // Verify restored config.
    assert!(target_config.exists());
    let cfg = fs::read_to_string(&target_config).unwrap();
    assert!(cfg.contains("server_port: 8080"));
}

#[test]
fn test_restore_fails_on_invalid_archive() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let fake_archive = tmp_dir.path().join("not-a-backup.txt");
    let mut f = File::create(&fake_archive).unwrap();
    writeln!(f, "this is not a gzip file").unwrap();
    drop(f);

    let restore_opts = RestoreOptions {
        archive_path: fake_archive.to_str().unwrap().to_string(),
        config_path: tmp_dir
            .path()
            .join("config.yaml")
            .to_str()
            .unwrap()
            .to_string(),
        db_path: tmp_dir.path().join("data.db").to_str().unwrap().to_string(),
        force: true,
    };
    let result = run_restore(restore_opts);
    assert!(result.is_err());
}

#[test]
fn test_restore_fails_on_missing_archive() {
    let restore_opts = RestoreOptions {
        archive_path: "/nonexistent/backup.tar.gz".to_string(),
        config_path: "config.yaml".to_string(),
        db_path: "data.db".to_string(),
        force: true,
    };
    let result = run_restore(restore_opts);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_restore_missing_data_db_fails() {
    // Create a tar.gz that has backup.json but not data.db.
    let tmp_dir = tempfile::tempdir().unwrap();
    let archive_path = tmp_dir.path().join("incomplete.tar.gz");

    let archive_file = File::create(&archive_path).unwrap();
    let encoder = GzEncoder::new(archive_file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);

    // Add only backup.json, no data.db.
    let meta = BackupMetadata {
        version: "1.0".to_string(),
        timestamp: chrono_timestamp(),
        db_size_bytes: 100,
        voltrus_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let meta_json = serde_json::to_string_pretty(&meta).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(meta_json.len() as u64);
    header.set_cksum();
    builder
        .append_data(&mut header, "backup.json", meta_json.as_bytes())
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();

    let restore_opts = RestoreOptions {
        archive_path: archive_path.to_str().unwrap().to_string(),
        config_path: tmp_dir
            .path()
            .join("config.yaml")
            .to_str()
            .unwrap()
            .to_string(),
        db_path: tmp_dir.path().join("data.db").to_str().unwrap().to_string(),
        force: true,
    };
    let result = run_restore(restore_opts);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("data.db"));
}

#[test]
fn test_parse_major_version() {
    assert_eq!(parse_major_version("0.19.4"), 0);
    assert_eq!(parse_major_version("1.0.0"), 1);
    assert_eq!(parse_major_version("2.3.1"), 2);
    assert_eq!(parse_major_version("invalid"), 0);
    assert_eq!(parse_major_version(""), 0);
}

#[test]
fn test_read_metadata_from_archive() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let db_path = tmp_dir.path().join("data.db");
    let config_path = tmp_dir.path().join("config.yaml");

    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE t (x)", []).unwrap();
    drop(conn);
    let mut f = File::create(&config_path).unwrap();
    writeln!(f, "port: 8080").unwrap();
    drop(f);

    let archive_path = tmp_dir.path().join("backup.tar.gz");
    let opts = BackupOptions {
        config_path: config_path.to_str().unwrap().to_string(),
        db_path: db_path.to_str().unwrap().to_string(),
        output_path: Some(archive_path.to_str().unwrap().to_string()),
    };
    run_backup(opts).unwrap();

    let meta = read_metadata_from_archive(archive_path.to_str().unwrap()).unwrap();
    assert_eq!(meta.version, "1.0");
    assert_eq!(meta.voltrus_version, env!("CARGO_PKG_VERSION"));
    assert!(meta.db_size_bytes > 0);
    assert!(!meta.timestamp.is_empty());
}

#[test]
fn test_restore_creates_pre_restore_backup() {
    // Create an existing database, then restore a different backup.
    // The pre-restore backup should be created automatically.
    let tmp_dir = tempfile::tempdir().unwrap();

    // Create "existing" database with known data.
    let existing_db = tmp_dir.path().join("data.db");
    let existing_config = tmp_dir.path().join("config.yaml");
    let conn = Connection::open(&existing_db).unwrap();
    conn.execute("CREATE TABLE existing (val TEXT)", [])
        .unwrap();
    conn.execute("INSERT INTO existing (val) VALUES ('old_data')", [])
        .unwrap();
    drop(conn);
    let mut f = File::create(&existing_config).unwrap();
    writeln!(f, "server_port: 9999").unwrap();
    drop(f);

    // Create a backup from a different source.
    let backup_dir = tempfile::tempdir().unwrap();
    let source_db = backup_dir.path().join("data.db");
    let source_config = backup_dir.path().join("config.yaml");
    let conn2 = Connection::open(&source_db).unwrap();
    conn2
        .execute("CREATE TABLE test (id INTEGER PRIMARY KEY, val TEXT)", [])
        .unwrap();
    conn2
        .execute("INSERT INTO test (val) VALUES ('new_data')", [])
        .unwrap();
    drop(conn2);
    let mut f2 = File::create(&source_config).unwrap();
    writeln!(f2, "server_port: 8080").unwrap();
    drop(f2);

    let archive_path = tmp_dir.path().join("backup.tar.gz");
    let backup_opts = BackupOptions {
        config_path: source_config.to_str().unwrap().to_string(),
        db_path: source_db.to_str().unwrap().to_string(),
        output_path: Some(archive_path.to_str().unwrap().to_string()),
    };
    run_backup(backup_opts).unwrap();

    // Restore into the same location where the existing DB lives.
    let restore_opts = RestoreOptions {
        archive_path: archive_path.to_str().unwrap().to_string(),
        config_path: existing_config.to_str().unwrap().to_string(),
        db_path: existing_db.to_str().unwrap().to_string(),
        force: true,
    };
    run_restore(restore_opts).unwrap();

    // Verify the database was replaced.
    let conn3 = Connection::open(&existing_db).unwrap();
    let val: String = conn3
        .query_row("SELECT val FROM test WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(val, "new_data");
    drop(conn3);

    // Verify config was replaced.
    let cfg = fs::read_to_string(&existing_config).unwrap();
    assert!(cfg.contains("server_port: 8080"));
}
