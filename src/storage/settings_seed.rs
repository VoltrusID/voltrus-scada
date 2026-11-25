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

    /// Seed a fresh database: the `admin` user with a random password (written
    /// to [`INITIAL_ADMIN_PASSWORD_FILE`] next to the database, to be changed
    /// on first login), the default roles and the data sources of the
    /// configured devices. Existing data is left alone.
    pub async fn seed_database(
        &self,
        device_configs: &[crate::models::DeviceConfig],
    ) -> Result<()> {
        if let Some(written) = self.seed(device_configs).await? {
            // The password goes to a file only this account can read, not to the
            // log: logs are shipped, rotated and read by people who should not
            // get the admin password.
            match written {
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
        Ok(())
    }

    /// [`Storage::seed_database`] without logging, for setup tools that report
    /// themselves. Returns whether `admin` was created; its password is in
    /// [`INITIAL_ADMIN_PASSWORD_FILE`] then, as with `seed_database`.
    pub async fn seed_database_quietly(
        &self,
        device_configs: &[crate::models::DeviceConfig],
    ) -> Result<bool> {
        match self.seed(device_configs).await? {
            Some(Err(e)) => {
                // `admin` exists now; failing here would not create it again.
                tracing::error!("Could not write the generated admin password file: {e:#}");
                Ok(true)
            }
            Some(Ok(_)) => Ok(true),
            None => Ok(false),
        }
    }

    /// The seed itself. When it created `admin`, returns the outcome of writing
    /// the generated password file (its path, `None` for an in-memory database).
    async fn seed(
        &self,
        device_configs: &[crate::models::DeviceConfig],
    ) -> Result<Option<Result<Option<PathBuf>>>> {
        let mut generated = None;
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
            generated = Some(write_initial_admin_password(self.db_path(), &password));
        }

        // Seed default RBAC roles if none exist
        if let Err(e) = self.seed_default_roles().await {
            tracing::warn!("Failed to seed default roles: {}", e);
        }

        // Seed data sources if none exist, matching actual device configs
        let all_data_sources = self.list_data_sources(None).await?;
        if all_data_sources.is_empty() {
            self.seed_device_data_sources(device_configs).await?;
        } else {
            match self.tags_with_old_seeded_limits().await {
                Ok(old) => old.warn(),
                Err(e) => tracing::warn!("Could not check for seeded alarm limits: {e:#}"),
            }
        }

        Ok(generated)
    }

    /// Tags that still carry, unchanged, alarm limits that earlier versions
    /// (up to v0.52.0) wrote without anyone configuring them. They are
    /// reported, not changed: a limit someone kept on purpose looks the same.
    /// Tags saved with a `<limit>_enabled` flag were edited since and are
    /// left out.
    pub(crate) async fn tags_with_old_seeded_limits(&self) -> Result<OldSeededLimits> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // First start: every register got 80/90, known register names
            // fixed limits; name, colour, range, base and limits as seeded.
            let mut stmt = conn.prepare(
                "SELECT key FROM data_sources
                 WHERE instr(source_config, '_enabled') = 0
                   AND (
                     (name = key AND color = '#94a3b8' AND min_val = 0 AND max_val = 100
                      AND base_val = 50 AND warn_val = 80 AND crit_val = 90)
                     OR (key, name, color, min_val, max_val, warn_val, crit_val, base_val)
                        IN (VALUES
                       ('voltage', 'Voltage', '#f59e0b', 0, 450, 400, 420, 380),
                       ('current', 'Current', '#06b6d4', 0, 100, 85, 95, 65),
                       ('power', 'Power', '#a855f7', 0, 50000, 40000, 45000, 24700),
                       ('frequency', 'Frequency', '#22c55e', 45, 55, 51, 52, 50),
                       ('power_factor', 'Power Factor', '#ef4444', 0, 1, 0.95, 0.98, 0.92),
                       ('energy_total', 'Energy Total', '#8b5cf6', 0, 999999, 800000, 900000,
                        158732))
                   )
                 ORDER BY id",
            )?;
            let mut old = OldSeededLimits::default();
            for key in stmt.query_map([], |row| row.get::<_, String>(0))? {
                old.first_start.push(key?);
            }
            // Vertical and FEED templates: orange, the template's id in
            // source_config, the tag's name (vertical) or "<name> — <description>"
            // (FEED), base in the middle of the range, warning at the range's
            // minimum and critical at its maximum. A template alarm overwrote
            // one of the two limits at most, so either one left in place counts
            // (only a limit above 0 is a limit).
            let mut stmt = conn.prepare(
                "SELECT key, min_val, max_val, warn_val, source_config
                 FROM data_sources
                 WHERE color = '#f97316'
                   AND instr(source_config, '_enabled') = 0
                   AND instr(source_config, '\"template_id\"') > 0
                   AND base_val = (min_val + max_val) / 2
                   AND (name = substr(key, instr(key, ':') + 1)
                        OR substr(name, 1, length(key) - instr(key, ':') + 3)
                           = substr(key, instr(key, ':') + 1) || ' — ')
                   AND ((warn_val = min_val AND warn_val > 0)
                        OR (crit_val = max_val AND crit_val > 0))
                 ORDER BY id",
            )?;
            for row in stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })? {
                let (key, min, max, warn, cfg) = row?;
                // On a two-state tag (a fault bit, a running contact) a
                // critical limit at the top of the range is how a fault is
                // alarmed, so it is no sign of a leftover; only a warning at
                // the minimum is.
                if (warn == min && warn > 0.0) || !is_digital(min, max, &cfg) {
                    old.template_range.push(key);
                }
            }
            // Vertical templates stored a "below" alarm as a high limit.
            let mut stmt = conn.prepare(
                "SELECT key, source_config, warn_val, crit_val FROM data_sources
                 WHERE color = '#f97316' AND instr(source_config, '_enabled') = 0
                   AND instr(source_config, 'template_id') > 0
                 ORDER BY id",
            )?;
            let templates = crate::vertical_templates::definitions::all_templates();
            for row in stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })? {
                let (key, cfg, warn, crit) = row?;
                let Some(template_id) = serde_json::from_str::<serde_json::Value>(&cfg)
                    .ok()
                    .and_then(|v| v.get("template_id")?.as_str().map(str::to_string))
                else {
                    continue;
                };
                let Some(template) = templates.iter().find(|t| t.id == template_id) else {
                    continue;
                };
                let name = key.split_once(':').map_or(key.as_str(), |(_, n)| n);
                let below_as_high = template.alarms.iter().any(|a| {
                    a.comparison == "below"
                        && a.threshold > 0.0
                        && pattern_matches(&a.tag_pattern, name)
                        && a.threshold == if a.severity == "critical" { crit } else { warn }
                });
                if below_as_high {
                    old.below_as_high.push(key);
                }
            }
            Ok(old)
        })
        .await
        .context("Task join failed")?
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

        // Display properties for known register names: name, unit, colour,
        // range, base value and decimals. No alarm limits: a tag alarms only
        // on limits someone configured, never on a default made up here.
        type Display = (&'static str, &'static str, &'static str, f64, f64, f64, i64);
        let default_map: std::collections::HashMap<&str, Display> = [
            ("voltage", ("Voltage", "V", "#f59e0b", 0.0, 450.0, 380.0, 1)),
            ("current", ("Current", "A", "#06b6d4", 0.0, 100.0, 65.0, 2)),
            ("power", ("Power", "W", "#a855f7", 0.0, 50000.0, 24700.0, 0)),
            (
                "frequency",
                ("Frequency", "Hz", "#22c55e", 45.0, 55.0, 50.0, 2),
            ),
            (
                "power_factor",
                ("Power Factor", "", "#ef4444", 0.0, 1.0, 0.92, 3),
            ),
            (
                "energy_total",
                ("Energy Total", "kWh", "#8b5cf6", 0.0, 999999.0, 158732.0, 1),
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
                let (display, unit, color, min, max, base, decimals) = default_map
                    .get(reg_name.as_str())
                    .copied()
                    .unwrap_or((reg_name.as_str(), "", "#94a3b8", 0.0, 100.0, 50.0, 1));

                let id = self
                    .create_data_source(
                        reg_name,
                        display,
                        unit,
                        color,
                        min,
                        max,
                        0.0,
                        0.0,
                        base,
                        sort_idx as i64,
                        decimals,
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

        tracing::info!(
            "Seeded data sources for {} devices. They have no alarm limits: \
             a tag raises alarms once limits are set on it (tag settings or the tags API).",
            devices_to_seed.len()
        );
        Ok(())
    }
}

/// Tags found by [`Storage::tags_with_old_seeded_limits`], by where their
/// unconfigured limits came from (tag keys).
#[derive(Debug, Default, PartialEq)]
pub struct OldSeededLimits {
    /// First start: 80/90 on any register, fixed limits on known names.
    pub first_start: Vec<String>,
    /// Vertical or FEED template: the range's minimum and maximum as limits.
    pub template_range: Vec<String>,
    /// Vertical template "below" alarm stored as a high limit, so it is
    /// raised in normal operation.
    pub below_as_high: Vec<String>,
}

impl OldSeededLimits {
    pub fn is_empty(&self) -> bool {
        self.first_start.is_empty()
            && self.template_range.is_empty()
            && self.below_as_high.is_empty()
    }

    /// One startup warning per kind found.
    fn warn(&self) {
        let list = |keys: &[String]| keys.join(", ");
        if !self.first_start.is_empty() {
            tracing::warn!(
                "{} tag(s) still have the alarm limits earlier versions gave every tag at first start \
                 (e.g. high warning 80, high critical 90), which nobody configured: {}. \
                 Set real limits or clear them; see docs/upgrade-notes.md.",
                self.first_start.len(),
                list(&self.first_start)
            );
        }
        if !self.template_range.is_empty() {
            tracing::warn!(
                "{} template tag(s) still have their range as alarm limits (high warning at the \
                 minimum, high critical at the maximum), which nobody configured: {}. \
                 Set real limits or clear them; see docs/upgrade-notes.md.",
                self.template_range.len(),
                list(&self.template_range)
            );
        }
        if !self.below_as_high.is_empty() {
            tracing::warn!(
                "{} template tag(s) have a low alarm stored as a high limit, so they alarm in \
                 normal operation: {}. Move the limit to the low limit; see docs/upgrade-notes.md.",
                self.below_as_high.len(),
                list(&self.below_as_high)
            );
        }
    }
}

/// Whether a tag is two-state: range 0–1, or a bool, coil or discrete input
/// (`data_type`, `register_area` or FEED `io_type` in its source_config).
fn is_digital(min: f64, max: f64, source_config: &str) -> bool {
    if min == 0.0 && max == 1.0 {
        return true;
    }
    let Ok(cfg) = serde_json::from_str::<serde_json::Value>(source_config) else {
        return false;
    };
    let field = |name: &str| cfg.get(name).and_then(|v| v.as_str()).unwrap_or("");
    matches!(field("data_type"), "bool" | "boolean")
        || matches!(field("register_area"), "coil" | "discrete")
        || matches!(field("io_type"), "DI" | "DO")
}

/// Whether a template tag name such as `TK-2_LVL` matches a pattern such as
/// `TK-{n}_LVL`: the literal parts in order, any text for each `{...}`.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    let mut literals = Vec::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        literals.push(&rest[..open]);
        rest = &rest[open + close + 1..];
    }
    if literals.is_empty() {
        return pattern == name;
    }
    let Some(remaining) = name.strip_prefix(literals[0]) else {
        return false;
    };
    let Some(mut remaining) = remaining.strip_suffix(rest) else {
        return false;
    };
    for literal in &literals[1..] {
        match remaining.find(literal) {
            Some(at) if !literal.is_empty() => remaining = &remaining[at + literal.len()..],
            Some(_) => {}
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod pattern_tests {
    use super::pattern_matches;

    #[test]
    fn template_patterns_match_their_expanded_names() {
        assert!(pattern_matches("TK-{n}_LVL", "TK-12_LVL"));
        assert!(pattern_matches("NAV_{light}", "NAV_PORT"));
        assert!(pattern_matches("CLO2_RES", "CLO2_RES"));
        assert!(!pattern_matches("CLO2_RES", "CLO2_RES2"));
        assert!(!pattern_matches("TK-{n}_LVL", "TK-1_TMP"));
        assert!(!pattern_matches("ENG{n}_OIL_P", "ENG1_TMP"));
    }
}
