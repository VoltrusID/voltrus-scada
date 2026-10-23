use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rusqlite::Connection;
use rusqlite::backup::Backup;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

mod archive;
mod backup_op;
mod restore;
mod types;
mod utils;

#[cfg(test)]
mod tests;

// Re-export public API for backward compatibility (crate::backup::Foo still works).
pub use backup_op::run_backup;
pub use restore::run_restore;
pub use types::{BackupOptions, RestoreOptions};

// Re-export internal items so sub-modules can use `super::*` to access them.
pub(crate) use archive::{
    create_archive, create_sqlite_backup, read_metadata_from_archive, validate_archive,
};
pub(crate) use types::BackupMetadata;
pub(crate) use utils::{chrono_now_timestamp, chrono_timestamp, format_size, parse_major_version};
