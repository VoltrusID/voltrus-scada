use super::Storage;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Name of the file, next to the database, holding the admin password the
/// server generated on first start. Deleted when the admin changes it.
pub const INITIAL_ADMIN_PASSWORD_FILE: &str = "initial-admin-password";

/// Where the generated admin password is kept for a database at `db_path`;
/// `None` for an in-memory database.
pub fn initial_admin_password_path(db_path: &str) -> Option<PathBuf> {
    if db_path == ":memory:" || db_path.is_empty() {
        return None;
    }
    let dir = Path::new(db_path)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Some(dir.join(INITIAL_ADMIN_PASSWORD_FILE))
}

/// Write `password` to [`initial_admin_password_path`], created with mode 0600
/// (owner only) so the password is never readable by other local accounts,
/// even for an instant.
fn write_initial_admin_password(db_path: &str, password: &str) -> Result<Option<PathBuf>> {
    use std::io::Write;
    let Some(path) = initial_admin_password_path(db_path) else {
        return Ok(None);
    };
    // A stale file from an earlier database would otherwise make create_new fail.
    let _ = std::fs::remove_file(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("create {}", path.display()))?;
    writeln!(file, "{password}").with_context(|| format!("write {}", path.display()))?;
    Ok(Some(path))
}

/// Remove the generated-password file once the admin has chosen a password.
/// Missing file is fine (already removed, or never generated).
pub fn remove_initial_admin_password(db_path: &str) {
    if let Some(path) = initial_admin_password_path(db_path)
        && let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("Could not delete {}: {e}", path.display());
    }
}

impl Storage {
    // ── Seed ──

    pub async fn seed_database(
        &self,
        device_configs: &[crate::models::DeviceConfig],
    ) -> Result<()> {
        let existing = self.get_user_by_username("admin").await?;
        if existing.is_none() {
            use rand::Rng;
            let password: String = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(16)
                .map(char::from)
                .collect();
            let password_hash = crate::api::auth::hash_password(&password);

            self.create_user("admin", &password_hash, "", "admin")
                .await?;
            self.set_must_change_password("admin", true).await?;

            // The password goes to a file only this account can read, not to the
            // log: logs are shipped, rotated and read by people who should not
            // get the admin password.
            match write_initial_admin_password(self.db_path(), &password) {
                Ok(Some(path)) => tracing::warn!(
                    "Generated the admin password and wrote it to {} (readable by this account only). \
                     Sign in as admin with it; you must change it at first sign-in, which deletes the file.",
                    path.display()
                ),
                Ok(None) => tracing::warn!(
                    "Generated an admin password for the in-memory database; it is not saved anywhere"
                ),
                Err(e) => tracing::error!(
                    "Could not write the generated admin password file: {e:#}. \
                     Run `voltrus --reset-admin-password` to set a new one."
                ),
            }
        }

        // Seed default RBAC roles if none exist
        if let Err(e) = self.seed_default_roles().await {
            tracing::warn!("Failed to seed default roles: {}", e);
        }

        // Seed data sources if none exist, matching actual device configs
        let all_data_sources = self.list_data_sources(None).await?;
        if all_data_sources.is_empty() {
            self.seed_device_data_sources(device_configs).await?;
        }

        Ok(())
    }

    /// Seed data sources for the devices defined in config.yaml.
    /// When actual devices are present, their register names are used.
    /// Falls back to generic PLTS defaults when no devices are configured.
    pub(crate) async fn seed_device_data_sources(
        &self,
        device_configs: &[crate::models::DeviceConfig],
    ) -> Result<()> {
        // Determine which devices to seed for: use actual config devices if available,
        // otherwise fall back to legacy plts_1/plts_2
        let devices_to_seed: Vec<(&str, Vec<String>)> = if device_configs.is_empty() {
            vec![
                (
                    "plts_1",
                    vec![
                        "voltage".into(),
                        "current".into(),
                        "power".into(),
                        "frequency".into(),
                        "power_factor".into(),
                        "energy_total".into(),
                    ],
                ),
                (
                    "plts_2",
                    vec![
                        "voltage".into(),
                        "current".into(),
                        "power".into(),
                        "frequency".into(),
                        "power_factor".into(),
                        "energy_total".into(),
                    ],
                ),
            ]
        } else {
            device_configs
                .iter()
                .map(|d| {
                    let regs = d
                        .registers
                        .iter()
                        .map(|r| r.name.clone())
                        .collect::<Vec<_>>();
                    (d.id.as_str(), regs)
                })
                .collect()
        };

        // Seed devices (screens)
        for (i, (device_id, _)) in devices_to_seed.iter().enumerate() {
            if !self.device_exists(device_id).await? {
                let display_name = device_id.replace('_', " ").to_uppercase();
                self.create_device(device_id, &display_name, i as i64)
                    .await?;
            }
        }

        // Default display properties for known register names
        let default_map: std::collections::HashMap<
            String,
            (String, String, String, f64, f64, f64, f64, f64, i64),
        > = [
            (
                "voltage".to_string(),
                (
                    "Voltage".to_string(),
                    "V".to_string(),
                    "#f59e0b".to_string(),
                    0.0,
                    450.0,
                    400.0,
                    420.0,
                    380.0,
                    1i64,
                ),
            ),
            (
                "current".to_string(),
                (
                    "Current".to_string(),
                    "A".to_string(),
                    "#06b6d4".to_string(),
                    0.0,
                    100.0,
                    85.0,
                    95.0,
                    65.0,
                    2i64,
                ),
            ),
            (
                "power".to_string(),
                (
                    "Power".to_string(),
                    "W".to_string(),
                    "#a855f7".to_string(),
                    0.0,
                    50000.0,
                    40000.0,
                    45000.0,
                    24700.0,
                    0i64,
                ),
            ),
            (
                "frequency".to_string(),
                (
                    "Frequency".to_string(),
                    "Hz".to_string(),
                    "#22c55e".to_string(),
                    45.0,
                    55.0,
                    51.0,
                    52.0,
                    50.0,
                    2i64,
                ),
            ),
            (
                "power_factor".to_string(),
                (
                    "Power Factor".to_string(),
                    String::new(),
                    "#ef4444".to_string(),
                    0.0,
                    1.0,
                    0.95,
                    0.98,
                    0.92,
                    3i64,
                ),
            ),
            (
                "energy_total".to_string(),
                (
                    "Energy Total".to_string(),
                    "kWh".to_string(),
                    "#8b5cf6".to_string(),
                    0.0,
                    999999.0,
                    800000.0,
                    900000.0,
                    158732.0,
                    1i64,
                ),
            ),
        ]
        .into_iter()
        .collect();

        // For each device, create data sources from its register names
        let mut all_ds_ids: std::collections::HashMap<&str, Vec<i64>> =
            std::collections::HashMap::new();

        for (device_id, register_names) in &devices_to_seed {
            let mut ids = Vec::new();
            for (sort_idx, reg_name) in register_names.iter().enumerate() {
                let fallback = (
                    reg_name.clone(),
                    String::new(),
                    "#94a3b8".to_string(),
                    0.0,
                    100.0,
                    80.0,
                    90.0,
                    50.0,
                    1i64,
                );
                let (display, unit, color, min, max, warn, crit, base, decimals) =
                    default_map.get(reg_name).unwrap_or(&fallback);

                let id = self
                    .create_data_source(
                        reg_name,
                        display,
                        unit,
                        color,
                        *min,
                        *max,
                        *warn,
                        *crit,
                        *base,
                        sort_idx as i64,
                        *decimals,
                        "modbus",
                        "{}",
                        device_id,
                        None,
                        "sparkline",
                    )
                    .await?;
                ids.push(id);
            }
            all_ds_ids.insert(device_id, ids);
        }

        // Assign data sources to screens
        for (device_id, _) in &devices_to_seed {
            if let Some(ids) = all_ds_ids.get(device_id) {
                for (i, &sid) in ids.iter().enumerate() {
                    self.add_screen_data_source(device_id, sid, i as i64)
                        .await?;
                }
            }
        }

        tracing::info!("Seeded data sources for {} devices", devices_to_seed.len());
        Ok(())
    }
}
