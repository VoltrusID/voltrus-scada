#[allow(unused_imports)]
use super::*;

#[test]
fn test_sanitize_filename() {
    assert_eq!(sanitize_filename("plts_1"), "plts_1");
    assert_eq!(sanitize_filename("my device"), "my_device");
    assert_eq!(sanitize_filename("test/device"), "test_device");
    assert_eq!(sanitize_filename("hello.world"), "hello_world");
    assert_eq!(sanitize_filename(""), "");
    assert_eq!(sanitize_filename("a/b:c@d"), "a_b_c_d");
}

#[test]
fn test_device_export_serialization() {
    let device = DeviceExport {
        id: "plts_1".to_string(),
        name: "PLTS 1".to_string(),
        ip: "192.168.1.10".to_string(),
        port: 502,
        unit_id: 1,
        interval_ms: 1000,
        endianness: "big".to_string(),
        enabled: true,
        registers: vec![RegisterExport {
            name: "voltage".to_string(),
            address: 0,
            register_type: "float32".to_string(),
            register_area: "holding".to_string(),
            scale: 1.0,
            sort_order: 0,
            zero_based: false,
        }],
    };
    let yaml = serde_yaml_neo::to_string(&device).unwrap();
    assert!(yaml.contains("id: plts_1") || yaml.contains("id: \"plts_1\""));
    assert!(yaml.contains("192.168.1.10"));
    assert!(yaml.contains("voltage"));
}

#[test]
fn test_device_export_roundtrip() {
    let device = DeviceExport {
        id: "dev_1".to_string(),
        name: "Device 1".to_string(),
        ip: "10.0.0.1".to_string(),
        port: 502,
        unit_id: 1,
        interval_ms: 500,
        endianness: "little".to_string(),
        enabled: false,
        registers: vec![],
    };
    let yaml = serde_yaml_neo::to_string(&device).unwrap();
    let parsed: DeviceExport = serde_yaml_neo::from_str(&yaml).unwrap();
    assert_eq!(parsed.id, "dev_1");
    assert_eq!(parsed.ip, "10.0.0.1");
    assert_eq!(parsed.port, 502);
    assert!(!parsed.enabled);
}

#[test]
fn test_flow_export_serialization() {
    let flow = FlowExport {
        id: "flow_1".to_string(),
        name: "Test Flow".to_string(),
        enabled: true,
        trigger_config: serde_json::json!({"type": "modbus", "device_id": "*"}),
        nodes: serde_json::json!([{"id": "n1", "type": "filter"}]),
    };
    let yaml = serde_yaml_neo::to_string(&flow).unwrap();
    assert!(yaml.contains("id: flow_1"));
    assert!(yaml.contains("enabled: true"));
}

#[test]
fn test_notification_export_serialization() {
    let notif = NotificationExport {
        source_key: "temp".to_string(),
        channel_type: "email".to_string(),
        config: serde_json::json!({"recipients": ["admin@example.com"]}),
        enabled: true,
    };
    let yaml = serde_yaml_neo::to_string(&notif).unwrap();
    assert!(yaml.contains("source_key: temp"));
    assert!(yaml.contains("channel_type: email"));
}

#[test]
fn test_validate_config_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let nonexistent = tmp.path().join("missing");
    let result = validate_config(nonexistent.to_str().unwrap()).unwrap();
    assert!(result.iter().any(|m| m.contains("SKIP: devices/")));
    assert!(result.iter().any(|m| m.contains("SKIP: screens/")));
    assert!(result.iter().any(|m| m.contains("SKIP: flows/")));
    assert!(result.iter().any(|m| m.contains("SKIP: alarms.yaml")));
}

#[test]
fn test_validate_config_with_files() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    // Create valid device file
    std::fs::create_dir_all(base.join("devices")).unwrap();
    let device = DeviceExport {
        id: "test_dev".to_string(),
        name: "Test".to_string(),
        ip: "192.168.1.1".to_string(),
        port: 502,
        unit_id: 1,
        interval_ms: 1000,
        endianness: "big".to_string(),
        enabled: true,
        registers: vec![],
    };
    std::fs::write(
        base.join("devices/test_dev.yaml"),
        serde_yaml_neo::to_string(&device).unwrap(),
    )
    .unwrap();

    // Create valid flow file
    std::fs::create_dir_all(base.join("flows")).unwrap();
    let flow = FlowExport {
        id: "flow_1".to_string(),
        name: "Flow".to_string(),
        enabled: true,
        trigger_config: serde_json::json!(null),
        nodes: serde_json::json!([]),
    };
    std::fs::write(
        base.join("flows/flow_1.yaml"),
        serde_yaml_neo::to_string(&flow).unwrap(),
    )
    .unwrap();

    let result = validate_config(base.to_str().unwrap()).unwrap();
    assert!(
        result
            .iter()
            .any(|m| m.contains("OK: devices/") && m.contains("test_dev"))
    );
    assert!(
        result
            .iter()
            .any(|m| m.contains("OK: flows/") && m.contains("flow_1"))
    );
}

#[test]
fn test_validate_config_invalid_file() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();

    std::fs::create_dir_all(base.join("devices")).unwrap();
    std::fs::write(base.join("devices/bad.yaml"), "not: valid: yaml: : :").unwrap();

    let result = validate_config(base.to_str().unwrap()).unwrap();
    assert!(
        result
            .iter()
            .any(|m| m.contains("ERROR") && m.contains("bad.yaml"))
    );
}
