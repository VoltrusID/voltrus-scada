use anyhow::{Context, Result};
use rusqlite::Connection;

/// Core table definitions executed once at database creation.
pub(super) fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS telemetry (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            metric TEXT NOT NULL,
            value REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_device_time_metric
            ON telemetry(device_id, metric, timestamp);
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            salt TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'user'
        );
        CREATE TABLE IF NOT EXISTS devices (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            type TEXT NOT NULL DEFAULT 'screen',
            sort_order INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS data_sources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            key TEXT NOT NULL,
            name TEXT NOT NULL,
            unit TEXT NOT NULL DEFAULT '',
            color TEXT NOT NULL DEFAULT '#f97316',
            min_val REAL NOT NULL,
            max_val REAL NOT NULL,
            warn_val REAL NOT NULL,
            crit_val REAL NOT NULL,
            base_val REAL NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            decimals INTEGER NOT NULL DEFAULT 1,
            source_type TEXT NOT NULL DEFAULT 'random_walk',
            source_config TEXT NOT NULL DEFAULT '{}',
            device_id TEXT NOT NULL DEFAULT 'default',
            equipment_id TEXT
        );
        CREATE TABLE IF NOT EXISTS screen_data_sources (
            screen_id TEXT NOT NULL,
            data_source_id INTEGER NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (screen_id, data_source_id)
        );
        CREATE TABLE IF NOT EXISTS screen_layouts (
            screen_id TEXT NOT NULL,
            breakpoint TEXT NOT NULL,
            layout TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY (screen_id, breakpoint)
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_shortcuts (
            user_id INTEGER NOT NULL,
            data_source_id INTEGER NOT NULL,
            shortcut TEXT NOT NULL,
            PRIMARY KEY (user_id, data_source_id)
        );
        CREATE TABLE IF NOT EXISTS twin_layouts (
            screen_id TEXT NOT NULL,
            equipment_id TEXT NOT NULL,
            equipment_type TEXT NOT NULL,
            position_x REAL NOT NULL DEFAULT 0,
            position_z REAL NOT NULL DEFAULT 0,
            rotation_y REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (screen_id, equipment_id)
        );
        CREATE TABLE IF NOT EXISTS alarms (
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
            cooldown_until INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_alarms_active
            ON alarms(source_key, severity, acknowledged_at);
        CREATE INDEX IF NOT EXISTS idx_alarms_triggered
            ON alarms(triggered_at DESC);
        CREATE TABLE IF NOT EXISTS push_subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            p256dh TEXT NOT NULL,
            auth_key TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(user, endpoint)
        );
        CREATE TABLE IF NOT EXISTS canvas_layouts (
            screen_id TEXT NOT NULL,
            data_source_id INTEGER NOT NULL,
            x REAL NOT NULL DEFAULT 0,
            y REAL NOT NULL DEFAULT 0,
            width REAL NOT NULL DEFAULT 300,
            height REAL NOT NULL DEFAULT 200,
            PRIMARY KEY (screen_id, data_source_id)
        );
        CREATE TABLE IF NOT EXISTS twin_connections (
            screen_id TEXT NOT NULL,
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            PRIMARY KEY (screen_id, from_id, to_id)
        );
        CREATE TABLE IF NOT EXISTS modbus_devices (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            ip TEXT NOT NULL,
            port INTEGER NOT NULL DEFAULT 502,
            unit_id INTEGER NOT NULL DEFAULT 1,
            interval_ms INTEGER NOT NULL DEFAULT 1000,
            endianness TEXT NOT NULL DEFAULT 'big',
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS modbus_registers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id TEXT NOT NULL REFERENCES modbus_devices(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            address INTEGER NOT NULL,
            register_type TEXT NOT NULL DEFAULT 'float32',
            scale REAL NOT NULL DEFAULT 1.0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            zero_based INTEGER NOT NULL DEFAULT 0,
            UNIQUE(device_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_modbus_registers_device
            ON modbus_registers(device_id);
        CREATE TABLE IF NOT EXISTS scada_equipment (
            screen_id TEXT NOT NULL,
            equipment_id TEXT NOT NULL,
            equipment_type TEXT NOT NULL,
            equipment_name TEXT NOT NULL DEFAULT '',
            x REAL NOT NULL DEFAULT 0,
            y REAL NOT NULL DEFAULT 0,
            width REAL NOT NULL DEFAULT 100,
            height REAL NOT NULL DEFAULT 60,
            rotation REAL NOT NULL DEFAULT 0,
            sensor_bindings TEXT NOT NULL DEFAULT '[]',
            config TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (screen_id, equipment_id)
        );
        CREATE TABLE IF NOT EXISTS scada_connections (
            screen_id TEXT NOT NULL,
            connection_id TEXT NOT NULL,
            from_equipment TEXT NOT NULL,
            to_equipment TEXT NOT NULL,
            waypoints TEXT NOT NULL DEFAULT '[]',
            connection_type TEXT NOT NULL DEFAULT 'power',
            PRIMARY KEY (screen_id, connection_id)
        );
        CREATE TABLE IF NOT EXISTS telemetry_hourly (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id TEXT NOT NULL,
            metric TEXT NOT NULL,
            bucket_start INTEGER NOT NULL,
            avg_value REAL NOT NULL,
            min_value REAL NOT NULL,
            max_value REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            UNIQUE(device_id, metric, bucket_start)
        );
        CREATE INDEX IF NOT EXISTS idx_hourly_dev_metric
            ON telemetry_hourly(device_id, metric, bucket_start);
        CREATE TABLE IF NOT EXISTS telemetry_daily (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id TEXT NOT NULL,
            metric TEXT NOT NULL,
            bucket_start INTEGER NOT NULL,
            avg_value REAL NOT NULL,
            min_value REAL NOT NULL,
            max_value REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            UNIQUE(device_id, metric, bucket_start)
        );
        CREATE INDEX IF NOT EXISTS idx_daily_dev_metric
            ON telemetry_daily(device_id, metric, bucket_start);
        CREATE TABLE IF NOT EXISTS flows (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            enabled INTEGER DEFAULT 1,
            trigger_config TEXT NOT NULL,
            nodes TEXT NOT NULL,
            created_at INTEGER,
            updated_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS flow_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            flow_id TEXT NOT NULL,
            started_at INTEGER,
            duration_ms INTEGER,
            success INTEGER,
            error TEXT,
            output TEXT
        );
        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action TEXT NOT NULL,
            username TEXT NOT NULL,
            resource TEXT NOT NULL,
            details TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_log_created
            ON audit_log(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_audit_log_user
            ON audit_log(username);
        CREATE TABLE IF NOT EXISTS notification_settings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_key TEXT NOT NULL,
            channel_type TEXT NOT NULL CHECK(channel_type IN ('email', 'webhook', 'push')),
            config TEXT NOT NULL DEFAULT '[]',
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_notification_settings_source
            ON notification_settings(source_key);
        CREATE TABLE IF NOT EXISTS api_keys (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            key_hash TEXT NOT NULL UNIQUE,
            key_prefix TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            created_by TEXT NOT NULL,
            last_used_at INTEGER,
            expires_at INTEGER,
            permissions TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_api_keys_hash
            ON api_keys(key_hash);
        CREATE TABLE IF NOT EXISTS opcua_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            security_policy TEXT NOT NULL DEFAULT 'none',
            security_mode TEXT NOT NULL DEFAULT 'none',
            username TEXT,
            password TEXT,
            interval_ms INTEGER NOT NULL DEFAULT 1000,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS opcua_nodes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES opcua_devices(id) ON DELETE CASCADE,
            node_id TEXT NOT NULL,
            name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            data_type TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, node_id)
        );
        CREATE INDEX IF NOT EXISTS idx_opcua_nodes_device ON opcua_nodes(device_id);
        CREATE TABLE IF NOT EXISTS s7_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            rack INTEGER NOT NULL DEFAULT 0,
            slot INTEGER NOT NULL DEFAULT 1,
            interval_ms INTEGER NOT NULL DEFAULT 1000,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS s7_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES s7_devices(id) ON DELETE CASCADE,
            tag_address TEXT NOT NULL,
            name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            data_type TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_s7_tags_device ON s7_tags(device_id);
        CREATE TABLE IF NOT EXISTS enip_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            slot INTEGER NOT NULL DEFAULT 0,
            interval_ms INTEGER NOT NULL DEFAULT 1000,
            enabled INTEGER NOT NULL DEFAULT 1,
            assembly_instance INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS enip_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES enip_devices(id) ON DELETE CASCADE,
            tag_name TEXT NOT NULL,
            name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            data_type TEXT NOT NULL DEFAULT '',
            assembly_offset INTEGER NOT NULL DEFAULT 0,
            assembly_length INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, tag_name)
        );
        CREATE INDEX IF NOT EXISTS idx_enip_tags_device ON enip_tags(device_id);
        CREATE TABLE IF NOT EXISTS voltrus_plc_devices (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            transport TEXT NOT NULL DEFAULT 'grpc',
            interval_ms INTEGER NOT NULL DEFAULT 1000,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS voltrus_plc_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id INTEGER NOT NULL REFERENCES voltrus_plc_devices(id) ON DELETE CASCADE,
            variable_name TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            data_type TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            UNIQUE(device_id, variable_name)
        );
        CREATE INDEX IF NOT EXISTS idx_voltrus_plc_tags_device ON voltrus_plc_tags(device_id);
        CREATE TABLE IF NOT EXISTS hub_sites (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            api_key TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            last_seen INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_hub_sites_enabled ON hub_sites(enabled);
        CREATE TABLE IF NOT EXISTS reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            schedule TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            device_ids TEXT NOT NULL DEFAULT 'all',
            email_recipients TEXT NOT NULL,
            last_generated INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS downtime_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            device_id TEXT NOT NULL,
            state TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            duration_secs INTEGER,
            reason TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_downtime_device ON downtime_events(device_id);
        CREATE INDEX IF NOT EXISTS idx_downtime_started ON downtime_events(started_at);
        CREATE TABLE IF NOT EXISTS shifts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            start_hour INTEGER NOT NULL,
            start_minute INTEGER NOT NULL,
            duration_hours INTEGER NOT NULL,
            days_mask INTEGER NOT NULL DEFAULT 127,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS roles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            permissions TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS electronic_signatures (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id TEXT NOT NULL,
            action TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            meaning TEXT NOT NULL,
            signature_hash TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_electronic_signatures_resource
            ON electronic_signatures(resource_type, resource_id);
        CREATE TABLE IF NOT EXISTS compliance_config (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            config TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS cameras (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            snapshot_on_alarm INTEGER NOT NULL DEFAULT 1,
            retention_hours INTEGER NOT NULL DEFAULT 72,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS camera_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            camera_id INTEGER NOT NULL REFERENCES cameras(id) ON DELETE CASCADE,
            timestamp INTEGER NOT NULL,
            alarm_source TEXT,
            file_path TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_camera_snapshots_camera
            ON camera_snapshots(camera_id, timestamp DESC);
        CREATE TABLE IF NOT EXISTS annotations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tag_id TEXT NOT NULL,
            annotation_text TEXT NOT NULL,
            data_timestamp INTEGER NOT NULL,
            author_id INTEGER NOT NULL REFERENCES users(id),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_annotations_tag_ts
            ON annotations(tag_id, data_timestamp);
        CREATE TABLE IF NOT EXISTS shift_handovers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            author_id INTEGER NOT NULL REFERENCES users(id),
            shift_date TEXT NOT NULL,
            shift_period TEXT NOT NULL CHECK(shift_period IN ('morning', 'afternoon', 'night')),
            what_happened TEXT NOT NULL DEFAULT '',
            whats_pending TEXT NOT NULL DEFAULT '',
            what_to_watch TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_shift_handovers_date_period
            ON shift_handovers(shift_date, shift_period);
        CREATE TABLE IF NOT EXISTS user_notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id),
            from_user_id INTEGER NOT NULL REFERENCES users(id),
            notification_type TEXT NOT NULL CHECK(notification_type IN ('mention', 'shift_handover')),
            message TEXT NOT NULL,
            reference_id INTEGER,
            read INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_notifications_user_read
            ON user_notifications(user_id, read, created_at);

        -- v0.26.0 -- Simulation profiles
        CREATE TABLE IF NOT EXISTS sim_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT UNIQUE NOT NULL,
            data_type TEXT NOT NULL,
            config TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_sim_profiles_name ON sim_profiles(name);

        CREATE TABLE IF NOT EXISTS scripts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT UNIQUE NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            syntax_error TEXT,
            cpu_limit_ms INTEGER NOT NULL DEFAULT 1000,
            memory_limit_bytes INTEGER NOT NULL DEFAULT 1048576,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_scripts_name ON scripts(name);

        CREATE TABLE IF NOT EXISTS script_executions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            script_id INTEGER NOT NULL REFERENCES scripts(id) ON DELETE CASCADE,
            flow_id TEXT,
            trigger_type TEXT,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'success',
            error_message TEXT,
            output TEXT,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_script_executions_script ON script_executions(script_id);
        CREATE INDEX IF NOT EXISTS idx_script_executions_created ON script_executions(created_at);

        CREATE TABLE IF NOT EXISTS config_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT,
            description TEXT,
            snapshot_data TEXT NOT NULL DEFAULT '{}',
            trigger TEXT NOT NULL DEFAULT 'auto',
            created_by TEXT,
            created_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_config_snapshots_created ON config_snapshots(created_at);
        CREATE INDEX IF NOT EXISTS idx_config_snapshots_trigger ON config_snapshots(trigger);

        -- v0.28.0 -- AI Copilot proposals
        CREATE TABLE IF NOT EXISTS ai_proposals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proposal_type TEXT NOT NULL,
            title TEXT NOT NULL,
            context TEXT NOT NULL DEFAULT '',
            proposed_config TEXT NOT NULL DEFAULT '{}',
            target_entity_id TEXT,
            model TEXT NOT NULL DEFAULT '',
            tokens_used INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            reviewed_by TEXT,
            applied_config TEXT,
            created_at INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ai_proposals_type ON ai_proposals(proposal_type);
        CREATE INDEX IF NOT EXISTS idx_ai_proposals_status ON ai_proposals(status);

        -- v0.28.0 -- Equipment templates
        CREATE TABLE IF NOT EXISTS equipment_templates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            industry TEXT,
            equipment_type TEXT NOT NULL DEFAULT '',
            tag_template TEXT NOT NULL DEFAULT '[]',
            alarm_template TEXT NOT NULL DEFAULT '[]',
            dashboard_template TEXT NOT NULL DEFAULT '{}',
            flow_template TEXT NOT NULL DEFAULT '[]'
        );",
    )
    .context("Failed to create schema")?;

    Ok(())
}
