use crate::models::AppConfig;
use anyhow::{Context, Result};
use std::fs;

pub fn load_config(path: &str) -> Result<AppConfig> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            tracing::info!("Config file {path} not found, using defaults");
            return Ok(serde_yaml_neo::from_str("").unwrap());
        }
    };

    let config: AppConfig = serde_yaml_neo::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {path}"))?;

    validate(&config)?;

    Ok(config)
}

fn validate(config: &AppConfig) -> Result<()> {
    if let Some(ref smtp) = config.smtp {
        anyhow::ensure!(
            !smtp.host.is_empty(),
            "SMTP host is required when smtp is configured"
        );
    }

    if config.devices.is_empty() {
        return Ok(());
    }

    let mut ids = std::collections::HashSet::new();
    for device in &config.devices {
        anyhow::ensure!(!device.id.is_empty(), "Device ID cannot be empty");
        anyhow::ensure!(
            ids.insert(device.id.clone()),
            "Duplicate device ID: {}",
            device.id
        );
        anyhow::ensure!(
            !device.registers.is_empty(),
            "Device {} has no registers",
            device.id
        );

        let mut reg_names = std::collections::HashSet::new();
        for reg in &device.registers {
            anyhow::ensure!(
                reg_names.insert(reg.name.clone()),
                "Duplicate register name '{}' in device {}",
                reg.name,
                device.id
            );
        }
    }

    // Validate HA config
    if config.ha.enabled {
        anyhow::ensure!(!config.ha.node_id.is_empty(), "HA node_id cannot be empty");
        anyhow::ensure!(
            !config.ha.peer_address.is_empty(),
            "HA peer_address is required when HA is enabled"
        );
        anyhow::ensure!(
            ["primary", "standby", "auto"].contains(&config.ha.role.as_str()),
            "HA role must be 'primary', 'standby', or 'auto'"
        );
    }

    // Validate networks (unique IDs)
    let mut net_ids = std::collections::HashSet::new();
    for net in &config.networks {
        anyhow::ensure!(!net.id.is_empty(), "Network ID cannot be empty");
        anyhow::ensure!(
            net_ids.insert(net.id.clone()),
            "Duplicate network ID: {}",
            net.id
        );
    }

    // Validate device network references
    for device in &config.devices {
        if let Some(ref net_id) = device.network {
            anyhow::ensure!(
                net_ids.contains(net_id),
                "Device '{}' references unknown network '{}'",
                device.id,
                net_id
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_valid_config() {
        let yaml = r#"
server_port: 8080
db_path: "test.db"
persist_interval_secs: 10
devices:
  - id: "dev1"
    ip: "192.168.1.10"
    registers:
      - name: "voltage"
        address: 40001
        type: "float32"
        scale: 0.1
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let config = load_config(f.path().to_str().unwrap()).unwrap();

        assert_eq!(config.server_port, 8080);
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].id, "dev1");
        assert_eq!(config.devices[0].ip, "192.168.1.10");
        assert_eq!(config.devices[0].port, 502); // default
        assert_eq!(config.devices[0].registers.len(), 1);
        assert_eq!(config.devices[0].registers[0].scale, 0.1);
    }

    #[test]
    fn test_defaults_applied() {
        let yaml = r#"
devices:
  - id: "dev1"
    ip: "10.0.0.1"
    registers:
      - name: "temp"
        address: 100
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let config = load_config(f.path().to_str().unwrap()).unwrap();

        assert_eq!(config.server_port, 3000);
        assert_eq!(config.db_path, "data.db");
        assert_eq!(config.persist_interval_secs, 5);
        assert_eq!(config.devices[0].port, 502);
        assert_eq!(config.devices[0].unit_id, 1);
        assert_eq!(config.devices[0].interval_ms, 1000);
        assert_eq!(config.devices[0].registers[0].scale, 1.0); // default
    }

    #[test]
    fn test_no_devices_ok() {
        let yaml = "devices: []\n";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let config = load_config(f.path().to_str().unwrap()).unwrap();
        assert!(config.devices.is_empty());
    }

    #[test]
    fn test_duplicate_device_id_fails() {
        let yaml = r#"
devices:
  - id: "dev1"
    ip: "10.0.0.1"
    registers:
      - name: "temp"
        address: 100
  - id: "dev1"
    ip: "10.0.0.2"
    registers:
      - name: "temp"
        address: 100
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let result = load_config(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate device ID")
        );
    }

    #[test]
    fn test_duplicate_register_name_fails() {
        let yaml = r#"
devices:
  - id: "dev1"
    ip: "10.0.0.1"
    registers:
      - name: "temp"
        address: 100
      - name: "temp"
        address: 102
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let result = load_config(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate register name")
        );
    }

    #[test]
    fn test_file_not_found_returns_default() {
        let config = load_config("/nonexistent/config.yaml").unwrap();
        assert!(config.devices.is_empty());
    }

    #[test]
    fn test_smtp_missing_host_fails() {
        let yaml = r#"
smtp:
  host: ""
  username: "user"
  password: "pass"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let result = load_config(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SMTP host is required")
        );
    }

    #[test]
    fn test_smtp_defaults_applied() {
        let yaml = r#"
smtp:
  host: "smtp.example.com"
  username: "user"
  password: "pass"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let config = load_config(f.path().to_str().unwrap()).unwrap();
        let smtp = config.smtp.unwrap();
        assert_eq!(smtp.host, "smtp.example.com");
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.encryption, crate::models::SmtpEncryption::StartTls);
        assert_eq!(smtp.from_address, "voltrus@example.com");
        assert_eq!(smtp.from_name, "Voltrus");
    }

    #[test]
    fn test_smtp_optional() {
        let yaml = "devices: []\n";
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let config = load_config(f.path().to_str().unwrap()).unwrap();
        assert!(config.smtp.is_none());
    }
}
