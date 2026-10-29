use rusqlite::Connection;

/// Incremental schema migrations applied after initial table creation.
/// Each migration is idempotent (uses `IF NOT EXISTS` / ignores errors on `ALTER TABLE`).
pub(super) fn run_migrations(conn: &Connection) {
    // Migration: add chart_type column
    let _ = conn.execute_batch(
        "ALTER TABLE data_sources ADD COLUMN chart_type TEXT NOT NULL DEFAULT 'sparkline'",
    );

    // Migration: add equipment_name column to twin_layouts
    let _ = conn.execute_batch(
        "ALTER TABLE twin_layouts ADD COLUMN equipment_name TEXT NOT NULL DEFAULT ''",
    );

    // Migration: rename legacy tables from sensor -> data_source
    let _ = conn.execute_batch("ALTER TABLE sensors RENAME TO data_sources");
    let _ = conn.execute_batch("ALTER TABLE screen_sensors RENAME TO screen_data_sources");
    let _ = conn
        .execute_batch("ALTER TABLE screen_data_sources RENAME COLUMN sensor_id TO data_source_id");
    let _ =
        conn.execute_batch("ALTER TABLE user_shortcuts RENAME COLUMN sensor_id TO data_source_id");

    // Migration: add role column to users
    let _ = conn.execute_batch("ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user'");
    let _ = conn.execute_batch("UPDATE users SET role = 'admin' WHERE username = 'admin'");

    // Migration: add cooldown_until column to alarms
    let _ = conn.execute_batch("ALTER TABLE alarms ADD COLUMN cooldown_until INTEGER");

    // Migration: add register_area column to modbus_registers
    let _ = conn.execute_batch(
        "ALTER TABLE modbus_registers ADD COLUMN register_area TEXT NOT NULL DEFAULT 'holding'",
    );

    // Migration: add zero_based column to modbus_registers (address is a 0-based offset, not a 5-digit reference)
    let _ = conn.execute_batch(
        "ALTER TABLE modbus_registers ADD COLUMN zero_based INTEGER NOT NULL DEFAULT 0",
    );

    // Migration: add must_change_password column to users. Only the run that adds the
    // column flags existing admins (they likely have the default "admin" password);
    // doing it on every start would force a password change after each restart.
    if conn
        .execute_batch(
            "ALTER TABLE users ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0",
        )
        .is_ok()
    {
        let _ = conn
            .execute_batch("UPDATE users SET must_change_password = 1 WHERE username = 'admin'");
    }

    // Migration: add account lockout columns to users
    let _ = conn
        .execute_batch("ALTER TABLE users ADD COLUMN failed_attempts INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE users ADD COLUMN locked_until INTEGER");

    // Migration: account disable flag (a disabled user cannot sign in)
    let _ = conn.execute_batch("ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0");

    // Migration: add refresh_tokens table
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS refresh_tokens (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token_hash TEXT NOT NULL UNIQUE,
            username TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            revoked_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_refresh_tokens_username ON refresh_tokens(username);
        CREATE INDEX IF NOT EXISTS idx_refresh_tokens_hash ON refresh_tokens(token_hash);",
    );

    // Migration: add license_activation table
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS license_activation (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            license_key TEXT NOT NULL UNIQUE,
            fingerprint_hash TEXT NOT NULL,
            fingerprint_hostname TEXT NOT NULL,
            fingerprint_cpu TEXT NOT NULL,
            fingerprint_disk_serial TEXT NOT NULL,
            fingerprint_mac TEXT NOT NULL,
            fingerprint_machine_id TEXT NOT NULL,
            activation_blob BLOB NOT NULL,
            activated_at TEXT NOT NULL,
            expires_at TEXT,
            tier TEXT,
            device_limit INTEGER
        );",
    );

    // Migration: add tier and device_limit columns to existing databases
    let _ = conn.execute_batch("ALTER TABLE license_activation ADD COLUMN tier TEXT;");
    let _ = conn.execute_batch("ALTER TABLE license_activation ADD COLUMN device_limit INTEGER;");

    // Migration: per-site licence and annual support contract
    let _ = conn.execute_batch("ALTER TABLE license_activation ADD COLUMN site_id TEXT;");
    let _ = conn.execute_batch("ALTER TABLE license_activation ADD COLUMN site_name TEXT;");
    let _ = conn.execute_batch("ALTER TABLE license_activation ADD COLUMN support_until INTEGER;");

    // Migration: add recipes table
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS recipes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            category TEXT,
            device_id TEXT NOT NULL,
            \"values\" TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            description TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_recipes_device ON recipes(device_id);",
    );

    // Migration: add role_id column to users (roles table is created in main schema above)
    let _ = conn.execute_batch("ALTER TABLE users ADD COLUMN role_id INTEGER REFERENCES roles(id)");

    // Migration: v0.22.0 -- IEC 62443 security audit mode
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS security_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            severity TEXT NOT NULL DEFAULT 'info',
            source_ip TEXT,
            user_agent TEXT,
            username TEXT,
            details TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_security_log_type ON security_log(event_type);
        CREATE INDEX IF NOT EXISTS idx_security_log_created ON security_log(created_at);
        CREATE INDEX IF NOT EXISTS idx_security_log_severity ON security_log(severity);

        CREATE TABLE IF NOT EXISTS security_thresholds (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            failed_login_threshold INTEGER NOT NULL DEFAULT 5,
            failed_login_window_secs INTEGER NOT NULL DEFAULT 60,
            suspicious_401_threshold INTEGER NOT NULL DEFAULT 10,
            suspicious_401_window_secs INTEGER NOT NULL DEFAULT 60,
            rate_limit_threshold INTEGER NOT NULL DEFAULT 100,
            rate_limit_window_secs INTEGER NOT NULL DEFAULT 60
        );
        INSERT OR IGNORE INTO security_thresholds (id) VALUES (1);",
    );

    // Migration: v0.23.0 -- Alarm Management Overhaul
    let _ = conn
        .execute_batch("ALTER TABLE alarms ADD COLUMN priority TEXT NOT NULL DEFAULT 'warning'");
    let _ = conn.execute_batch("ALTER TABLE alarms ADD COLUMN shelved_until INTEGER");
    let _ = conn.execute_batch("ALTER TABLE alarms ADD COLUMN shelved_by TEXT");
    let _ = conn.execute_batch("ALTER TABLE alarms ADD COLUMN shelved_reason TEXT");
    let _ = conn.execute_batch("ALTER TABLE alarms ADD COLUMN deadband REAL NOT NULL DEFAULT 0");
    let _ =
        conn.execute_batch("ALTER TABLE alarms ADD COLUMN state TEXT NOT NULL DEFAULT 'active'");
    let _ =
        conn.execute_batch("ALTER TABLE alarms ADD COLUMN source_name TEXT NOT NULL DEFAULT ''");

    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS alarm_audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alarm_id INTEGER NOT NULL REFERENCES alarms(id) ON DELETE CASCADE,
            action TEXT NOT NULL,
            performed_by TEXT NOT NULL,
            details TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_alarm_audit_alarm_time
            ON alarm_audit_log(alarm_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_alarm_audit_action_time
            ON alarm_audit_log(action, created_at);",
    );

    // Migration: v0.24.0 -- Command & Control tables
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS commands (
            id TEXT PRIMARY KEY,
            data_source_id INTEGER NOT NULL,
            device_id INTEGER NOT NULL,
            protocol TEXT NOT NULL,
            value TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            policy TEXT NOT NULL DEFAULT 'immediate',
            scheduled_at INTEGER,
            retry_count INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3,
            error_message TEXT,
            created_at INTEGER NOT NULL,
            sent_at INTEGER,
            acked_at INTEGER,
            resolved_at INTEGER,
            created_by TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_commands_status ON commands(status);
        CREATE INDEX IF NOT EXISTS idx_commands_data_source ON commands(data_source_id);
        CREATE INDEX IF NOT EXISTS idx_commands_device ON commands(device_id);
        CREATE INDEX IF NOT EXISTS idx_commands_created ON commands(created_at);

        CREATE TABLE IF NOT EXISTS command_audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            command_id TEXT NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
            old_status TEXT NOT NULL,
            new_status TEXT NOT NULL,
            changed_by TEXT NOT NULL,
            reason TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_command_audit_command ON command_audit_log(command_id);

        CREATE TABLE IF NOT EXISTS dnp3_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            ip TEXT NOT NULL,
            port INTEGER NOT NULL DEFAULT 20000,
            master_address INTEGER NOT NULL DEFAULT 1,
            outstation_address INTEGER NOT NULL DEFAULT 10,
            enable_unsolicited INTEGER NOT NULL DEFAULT 1,
            integrity_poll_interval_secs INTEGER NOT NULL DEFAULT 300,
            event_scan_interval_ms INTEGER,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS dnp3_points (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES dnp3_devices(id) ON DELETE CASCADE,
            group_num INTEGER NOT NULL,
            variation INTEGER NOT NULL,
            point_index INTEGER NOT NULL,
            name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            data_type TEXT NOT NULL DEFAULT 'float64',
            scale REAL NOT NULL DEFAULT 1.0,
            deadband REAL NOT NULL DEFAULT 0.0,
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_dnp3_points_device ON dnp3_points(device_id);",
    );

    // Migration: carry the text device id of a command. Modbus ids are text,
    // `device_id` holds only the numeric id of the other protocols.
    if conn
        .execute_batch("ALTER TABLE commands ADD COLUMN device_ref TEXT NOT NULL DEFAULT ''")
        .is_ok()
    {
        let _ = conn.execute_batch(
            "UPDATE commands SET device_ref = COALESCE(
                 (SELECT ds.device_id FROM data_sources ds WHERE ds.id = commands.data_source_id),
                 CAST(device_id AS TEXT))",
        );
    }

    // Migration: v0.31.0 -- BACnet/IP device and object tables
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bacnet_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            ip TEXT NOT NULL,
            port INTEGER NOT NULL DEFAULT 47808,
            device_instance INTEGER NOT NULL DEFAULT 0,
            interval_ms INTEGER NOT NULL DEFAULT 5000,
            cov_enabled INTEGER NOT NULL DEFAULT 0,
            cov_lifetime INTEGER NOT NULL DEFAULT 3600,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS bacnet_objects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES bacnet_devices(id) ON DELETE CASCADE,
            object_type TEXT NOT NULL,
            object_instance INTEGER NOT NULL,
            property TEXT NOT NULL DEFAULT 'presentValue',
            name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            cov_increment REAL NOT NULL DEFAULT 0.0,
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, object_type, object_instance)
        );
        CREATE INDEX IF NOT EXISTS idx_bacnet_objects_device ON bacnet_objects(device_id);",
    );

    // Migration: v0.24.0 -- Write policy columns on data_sources
    let _ = conn
        .execute_batch("ALTER TABLE data_sources ADD COLUMN writable INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN write_min REAL");
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN write_max REAL");
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN write_max_rate INTEGER");
    let _ = conn.execute_batch(
        "ALTER TABLE data_sources ADD COLUMN write_requires_reason INTEGER NOT NULL DEFAULT 0",
    );
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN write_deadband REAL");

    // Migration: v0.26.0 -- Simulation columns on devices
    let _ = conn
        .execute_batch("ALTER TABLE devices ADD COLUMN simulation_mode INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE devices ADD COLUMN sim_profile_id INTEGER");
    let _ = conn.execute_batch("ALTER TABLE devices ADD COLUMN sim_config TEXT");
    // Migration: v0.26.0 -- Simulation column on data_sources
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN sim_value REAL");

    // Migration: v0.32.0 -- UNS path column on data_sources for tag browser tree
    let _ = conn.execute_batch("ALTER TABLE data_sources ADD COLUMN path TEXT DEFAULT NULL");

    // Migration: v0.28.0 -- Custom metrics table for analytics
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_metrics (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            formula TEXT NOT NULL,
            unit TEXT,
            tag_refs TEXT NOT NULL DEFAULT '[]',
            validation_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    );

    // Migration: v0.32.0 -- Audit log search indexes
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_action ON audit_log(action);
         CREATE INDEX IF NOT EXISTS idx_audit_log_username ON audit_log(username);
         CREATE INDEX IF NOT EXISTS idx_audit_log_action_user_time ON audit_log(action, username, created_at);"
    );

    // Migration: fix alarms CHECK constraint to allow 'info' severity
    let _ = conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         DROP TABLE IF EXISTS alarms_new;
         CREATE TABLE alarms_new (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             source_key TEXT NOT NULL,
             severity TEXT NOT NULL CHECK(severity IN ('warning', 'critical', 'info')),
             value REAL NOT NULL,
             threshold REAL NOT NULL,
             message TEXT NOT NULL,
             triggered_at INTEGER NOT NULL,
             resolved_at INTEGER,
             acknowledged_at INTEGER,
             acknowledged_by TEXT,
             cooldown_until INTEGER,
             priority TEXT NOT NULL DEFAULT 'warning',
             shelved_until INTEGER,
             shelved_by TEXT,
             shelved_reason TEXT,
             deadband REAL NOT NULL DEFAULT 0,
             state TEXT NOT NULL DEFAULT 'active',
             source_name TEXT NOT NULL DEFAULT ''
         );
         INSERT INTO alarms_new SELECT * FROM alarms;
         DROP TABLE alarms;
         ALTER TABLE alarms_new RENAME TO alarms;
         CREATE INDEX IF NOT EXISTS idx_alarms_active ON alarms(source_key, severity, acknowledged_at);
         CREATE INDEX IF NOT EXISTS idx_alarms_triggered ON alarms(triggered_at DESC);
         PRAGMA foreign_keys = ON;"
    );

    // Migration: v0.39.0 -- Data egress jobs for scheduled CSV/Parquet export
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS egress_jobs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            schedule TEXT NOT NULL DEFAULT 'daily',
            format TEXT NOT NULL DEFAULT 'csv',
            destination TEXT NOT NULL,
            device_ids TEXT NOT NULL DEFAULT 'all',
            time_range TEXT NOT NULL DEFAULT '24h',
            compression INTEGER NOT NULL DEFAULT 0,
            last_run INTEGER,
            last_status TEXT,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_egress_jobs_enabled ON egress_jobs(enabled);",
    );

    // Migration: v0.39.0 -- Outbound webhooks for external systems
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS outbound_webhooks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            events TEXT NOT NULL DEFAULT '[\"alarm\"]',
            device_filter TEXT NOT NULL DEFAULT '\"all\"',
            headers TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            retry_count INTEGER NOT NULL DEFAULT 3,
            timeout_ms INTEGER NOT NULL DEFAULT 10000,
            last_triggered INTEGER,
            last_status TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_outbound_webhooks_enabled ON outbound_webhooks(enabled);",
    );

    // Migration: EtherNet/IP implicit I/O assembly config (device instance + tag byte offsets)
    let _ = conn.execute_batch(
        "ALTER TABLE enip_devices ADD COLUMN assembly_instance INTEGER NOT NULL DEFAULT 0",
    );
    let _ = conn.execute_batch(
        "ALTER TABLE enip_tags ADD COLUMN assembly_offset INTEGER NOT NULL DEFAULT 0",
    );
    let _ = conn.execute_batch(
        "ALTER TABLE enip_tags ADD COLUMN assembly_length INTEGER NOT NULL DEFAULT 0",
    );

    // Migration: alarm direction (low alarms).
    // Low-alarm thresholds (low_warn/low_crit) live in data_sources.source_config
    // alongside `deadband`, so no new columns are needed there. This migration
    // adds `alarm_type` ('high' default, 'low') to alarms so auto-resolution is
    // direction-aware: a low alarm clears when the value rises back above its
    // threshold (+deadband), not when it drops further below it.
    let _ =
        conn.execute_batch("ALTER TABLE alarms ADD COLUMN alarm_type TEXT NOT NULL DEFAULT 'high'");

    // Migration: persist per-widget config (setpoints, labels, colors, thresholds)
    // for SCADA mimic equipment so operator customizations survive reload.
    // Idempotent — ignored once the column exists.
    let _ = conn
        .execute_batch("ALTER TABLE scada_equipment ADD COLUMN config TEXT NOT NULL DEFAULT '{}'");

    // Migration: API key roles. Adds a per-key role column ('api' default =
    // read-only, unchanged legacy behavior; 'integration' = write-capable key
    // that also passes demo-allowed operational routes). Idempotent — ignored
    // once the column exists.
    let _ = conn.execute_batch("ALTER TABLE api_keys ADD COLUMN role TEXT NOT NULL DEFAULT 'api'");

    // Migration: ISA-18.2 alarm on/off delays. The threshold config lives on
    // the tag that owns it (data_sources), so the delay values do too.
    // 0 disables the delay and preserves the immediate historical behavior.
    let _ = conn.execute_batch(
        "ALTER TABLE data_sources ADD COLUMN on_delay_secs INTEGER NOT NULL DEFAULT 0",
    );
    let _ = conn.execute_batch(
        "ALTER TABLE data_sources ADD COLUMN off_delay_secs INTEGER NOT NULL DEFAULT 0",
    );

    // Migration: append-only alarm journal. One row per lifecycle event
    // (triggered, acknowledged, shelved, unshelved, resolved), written from
    // both the evaluation loop and the management API paths.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS alarm_journal (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alarm_id INTEGER NOT NULL REFERENCES alarms(id) ON DELETE CASCADE,
            event TEXT NOT NULL,
            at INTEGER NOT NULL,
            actor TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_alarm_journal_alarm ON alarm_journal(alarm_id, at);",
    );

    // Migration: shared device template library. `body` is the canonical
    // voltrus-device-template/1 JSON; the other columns repeat what lists and
    // lookups filter on. Built-in templates are generated at startup and not stored.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS device_templates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            template_id TEXT NOT NULL,
            source TEXT NOT NULL,
            version TEXT NOT NULL,
            manufacturer TEXT NOT NULL,
            model TEXT NOT NULL,
            protocol TEXT NOT NULL,
            verified INTEGER NOT NULL DEFAULT 0,
            body TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(template_id, source, version)
        );
        CREATE INDEX IF NOT EXISTS idx_device_templates_manufacturer
            ON device_templates(manufacturer, model);",
    );

    // Migration: packed full-resolution history. The compaction worker turns the
    // raw `telemetry` rows of every complete hour into one Gorilla/RLE block per
    // tag and hour (see compression::block); readers merge both. `seq` numbers the
    // blocks of one tag-hour (only more than one past 4M samples). Databases
    // created before this keep all their raw rows until the worker packs them.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS telemetry_blocks (
            device_id TEXT NOT NULL,
            metric TEXT NOT NULL,
            bucket_start INTEGER NOT NULL,
            seq INTEGER NOT NULL,
            first_ts INTEGER NOT NULL,
            last_ts INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (device_id, metric, bucket_start, seq)
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS idx_telemetry_blocks_bucket
            ON telemetry_blocks(bucket_start);",
    );
    // Migration: let notification_settings hold the chat channels. SQLite
    // cannot alter a CHECK constraint, so the table is rebuilt.
    if let Err(e) = rebuild_notification_settings(conn) {
        tracing::error!("Failed to migrate notification_settings: {e}");
    }
    // Migration: per-block summaries (lowest, highest, sum and newest value) so a
    // long trend bucket that holds a whole block does not decode it. NULL on
    // blocks packed before this; those are decoded as before.
    for col in ["min_v", "max_v", "sum_v", "last_v"] {
        let _ = conn.execute_batch(&format!(
            "ALTER TABLE telemetry_blocks ADD COLUMN {col} REAL"
        ));
    }
}

const NOTIFICATION_CHANNELS: &str = "'email', 'webhook', 'push', 'telegram', 'whatsapp'";

/// Rebuild `notification_settings` with the current `channel_type` CHECK when
/// the stored table predates it. Rows of a channel type that is no longer
/// supported cannot satisfy the CHECK: they move to
/// `notification_settings_retired`, disabled, and one warning says so.
/// Returns the number of rows retired.
fn rebuild_notification_settings(conn: &Connection) -> rusqlite::Result<usize> {
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'notification_settings'",
        [],
        |row| row.get(0),
    )?;
    if sql.contains("'telegram'") && sql.contains("'whatsapp'") {
        return Ok(0);
    }

    let result = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute_batch(&format!(
            "CREATE TABLE notification_settings_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_key TEXT NOT NULL,
                channel_type TEXT NOT NULL CHECK(channel_type IN ({NOTIFICATION_CHANNELS})),
                config TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL
            );
            INSERT INTO notification_settings_new (id, source_key, channel_type, config, enabled, created_at)
                SELECT id, source_key, channel_type, config, enabled, created_at
                FROM notification_settings
                WHERE channel_type IN ({NOTIFICATION_CHANNELS});"
        ))?;
        let retired: usize = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM notification_settings WHERE channel_type NOT IN ({NOTIFICATION_CHANNELS})"
            ),
            [],
            |row| row.get(0),
        )?;
        if retired > 0 {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS notification_settings_retired (
                    id INTEGER PRIMARY KEY,
                    source_key TEXT NOT NULL,
                    channel_type TEXT NOT NULL,
                    config TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL
                );
                INSERT OR REPLACE INTO notification_settings_retired
                    (id, source_key, channel_type, config, enabled, created_at)
                    SELECT id, source_key, channel_type, config, 0, created_at
                    FROM notification_settings
                    WHERE channel_type NOT IN ({NOTIFICATION_CHANNELS});"
            ))?;
        }
        conn.execute_batch(
            "DROP TABLE notification_settings;
            ALTER TABLE notification_settings_new RENAME TO notification_settings;
            CREATE INDEX IF NOT EXISTS idx_notification_settings_source
                ON notification_settings(source_key);
            COMMIT;",
        )?;
        Ok(retired)
    })();

    match result {
        Ok(retired) => {
            if retired > 0 {
                tracing::warn!(
                    "{retired} notification rule(s) used a channel that is no longer supported. They were disabled and moved to the notification_settings_retired table; set up Telegram or WhatsApp rules instead."
                );
            }
            Ok(retired)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = "CREATE TABLE notification_settings (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        source_key TEXT NOT NULL,
        channel_type TEXT NOT NULL CHECK(channel_type IN ('email', 'webhook', 'push')),
        config TEXT NOT NULL DEFAULT '[]',
        enabled INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX idx_notification_settings_source ON notification_settings(source_key);";

    fn legacy_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(LEGACY).unwrap();
        conn.execute_batch(
            "INSERT INTO notification_settings (id, source_key, channel_type, config, enabled, created_at) VALUES
                (1, 'a', 'email', '[\"x@y.z\"]', 1, 10),
                (7, 'b', 'webhook', '[\"https://h\"]', 0, 11);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn rebuild_keeps_rows_and_ids_and_accepts_chat_channels() {
        let conn = legacy_db();
        assert_eq!(rebuild_notification_settings(&conn).unwrap(), 0);
        let rows: Vec<(i64, String, bool)> = conn
            .prepare("SELECT id, channel_type, enabled FROM notification_settings ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i32>(2)? != 0)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            [(1, "email".into(), true), (7, "webhook".into(), false)]
        );
        for channel in ["telegram", "whatsapp"] {
            conn.execute(
                "INSERT INTO notification_settings (source_key, channel_type, config, created_at) VALUES ('', ?1, '{}', 1)",
                [channel],
            )
            .unwrap();
        }
        // Autoincrement continues after the highest migrated id.
        let max: i64 = conn
            .query_row("SELECT MAX(id) FROM notification_settings", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(max, 9);
        assert!(
            conn.execute(
                "INSERT INTO notification_settings (source_key, channel_type, config, created_at) VALUES ('', 'pager', '[]', 1)",
                [],
            )
            .is_err()
        );
        // The source index survived the rebuild.
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_notification_settings_source'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn rebuild_is_a_no_op_on_a_current_table() {
        let conn = Connection::open_in_memory().unwrap();
        crate::storage::schema::init_schema(&conn).unwrap();
        assert_eq!(rebuild_notification_settings(&conn).unwrap(), 0);
        conn.execute(
            "INSERT INTO notification_settings (source_key, channel_type, config, created_at) VALUES ('', 'telegram', '{}', 1)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn retired_channel_rows_are_disabled_and_kept_aside() {
        // A database from before the CHECK existed can hold rows of a retired channel.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE notification_settings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_key TEXT NOT NULL,
                channel_type TEXT NOT NULL,
                config TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL
            );
            INSERT INTO notification_settings (id, source_key, channel_type, config, enabled, created_at) VALUES
                (1, 'a', 'email', '[]', 1, 1),
                (2, 'a', 'pager', '[{\"to\":\"1\"}]', 1, 2);",
        )
        .unwrap();
        assert_eq!(rebuild_notification_settings(&conn).unwrap(), 1);
        let live: i64 = conn
            .query_row("SELECT COUNT(*) FROM notification_settings", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(live, 1);
        let (channel, enabled): (String, i32) = conn
            .query_row(
                "SELECT channel_type, enabled FROM notification_settings_retired WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((channel.as_str(), enabled), ("pager", 0));
        // Running again changes nothing and logs nothing.
        assert_eq!(rebuild_notification_settings(&conn).unwrap(), 0);
    }
}
