use super::*;

/// Metadata stored inside the backup archive.
#[derive(Serialize, Deserialize)]
pub(crate) struct BackupMetadata {
    pub version: String,
    pub timestamp: String,
    pub db_size_bytes: u64,
    pub voltrus_version: String,
}

/// Options for the backup operation.
pub struct BackupOptions {
    pub config_path: String,
    pub db_path: String,
    pub output_path: Option<String>,
}

/// Options for the restore operation.
pub struct RestoreOptions {
    pub archive_path: String,
    pub config_path: String,
    pub db_path: String,
    pub force: bool,
}
