use super::*;

#[tokio::test]
async fn test_count_modbus_devices_empty() {
    let storage = Storage::new_in_memory().unwrap();
    let count = storage.count_modbus_devices().await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_count_modbus_devices_with_data() {
    let storage = Storage::new_in_memory().unwrap();
    storage
        .create_modbus_device(
            "dev_count_1",
            "Test Device",
            "192.168.1.1",
            502,
            1,
            1000,
            "big",
            &[],
        )
        .await
        .unwrap();
    let count = storage.count_modbus_devices().await.unwrap();
    assert_eq!(count, 1);
}

fn register_inputs(json: serde_json::Value) -> Vec<crate::models::ModbusRegisterInput> {
    crate::models::ModbusRegisterInput::parse_all(&json).unwrap()
}

#[tokio::test]
async fn test_registers_keep_legacy_references_and_explicit_offsets() {
    let storage = Storage::new_in_memory().unwrap();
    let registers = register_inputs(serde_json::json!({"registers": [
        {"name": "legacy_holding", "address": 40001, "register_type": "uint16"},
        {"name": "legacy_input", "address": 30003, "register_type": "uint16", "register_area": "input"},
        {"name": "plain", "address": 12, "register_type": "uint16"},
        {"name": "high_holding", "offset": 35000, "register_type": "float32"},
        {"name": "high_input", "offset": 65534, "register_type": "uint16", "register_area": "input"},
        {"name": "high_coil", "offset": 40000, "register_type": "uint16", "register_area": "coil"},
    ]}));
    storage
        .create_modbus_device(
            "dev_high", "High", "10.0.0.9", 502, 1, 1000, "big", &registers,
        )
        .await
        .unwrap();

    let stored = storage.list_modbus_registers("dev_high").await.unwrap();
    let by_name = |n: &str| stored.iter().find(|r| r.name == n).unwrap();
    assert!(!by_name("legacy_holding").zero_based);
    assert!(by_name("high_holding").zero_based);
    assert_eq!(by_name("high_holding").address, 35000);

    let device = storage
        .get_modbus_device("dev_high")
        .await
        .unwrap()
        .unwrap();
    let config = device.to_device_config(&stored);
    let wire = |n: &str| {
        config
            .registers
            .iter()
            .find(|r| r.name == n)
            .unwrap()
            .protocol_address()
    };
    assert_eq!(wire("legacy_holding"), 0);
    assert_eq!(wire("legacy_input"), 2);
    assert_eq!(wire("plain"), 12);
    assert_eq!(wire("high_holding"), 35000);
    assert_eq!(wire("high_input"), 65534);
    assert_eq!(wire("high_coil"), 40000);

    // What the API returns can be sent back unchanged.
    let echoed =
        register_inputs(serde_json::json!({ "registers": serde_json::to_value(&stored).unwrap() }));
    storage
        .replace_modbus_registers("dev_high", &echoed)
        .await
        .unwrap();
    let again = storage.list_modbus_registers("dev_high").await.unwrap();
    assert_eq!(
        again
            .iter()
            .map(|r| (r.name.clone(), r.address, r.zero_based))
            .collect::<Vec<_>>(),
        stored
            .iter()
            .map(|r| (r.name.clone(), r.address, r.zero_based))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_register_input_needs_exactly_one_address() {
    let parse = |r: serde_json::Value| {
        crate::models::ModbusRegisterInput::parse_all(&serde_json::json!({"registers": [r]}))
    };
    assert!(
        parse(serde_json::json!({"name": "a"}))
            .unwrap_err()
            .contains("required")
    );
    assert!(
        parse(serde_json::json!({"name": "a", "address": 1, "offset": 1}))
            .unwrap_err()
            .contains("not both")
    );
    assert!(parse(serde_json::json!({"name": "a", "offset": 70000})).is_err());
    assert!(parse(serde_json::json!({"name": "a", "offset": 0})).is_ok());
}

/// The file Modbus Poll's "Export to SCADA > Voltrus SCADA device" writes (client/tests/fixtures has the same file for the
/// importer). Every tag comes back with the address it had in Modbus Poll, including those above 30000.
#[tokio::test]
async fn test_modbus_poll_export_round_trip() {
    let file: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/modbus-poll-big-map.json")).unwrap();
    let registers = register_inputs(file.clone());

    let storage = Storage::new_in_memory().unwrap();
    storage
        .create_modbus_device(
            file["id"].as_str().unwrap(),
            file["name"].as_str().unwrap(),
            file["ip"].as_str().unwrap(),
            file["port"].as_u64().unwrap() as u16,
            file["unit_id"].as_u64().unwrap() as u8,
            file["interval_ms"].as_u64().unwrap(),
            file["endianness"].as_str().unwrap(),
            &registers,
        )
        .await
        .unwrap();

    let device = storage.get_modbus_device("big_map").await.unwrap().unwrap();
    let config = device.to_device_config(&storage.list_modbus_registers("big_map").await.unwrap());
    assert_eq!(config.port, 1502);
    assert_eq!(config.interval_ms, 2000);
    assert!(matches!(
        config.endianness,
        crate::models::Endianness::Swapped
    ));

    use crate::models::RegisterType::*;
    let wire: Vec<_> = config
        .registers
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.register_area.as_str(),
                r.protocol_address(),
                r.reg_type.clone(),
                r.scale,
            )
        })
        .collect();
    assert_eq!(
        wire,
        vec![
            ("Edge_30000", "holding", 30000, Uint16, 1.0),
            ("Float_35000", "holding", 35000, Float32, 0.5),
            ("Count_40000", "holding", 40000, Int32, 1.0),
            ("Input_30001", "input", 30001, Int16, 1.0),
            ("Input_65534", "input", 65534, Uint16, 1.0),
            ("Coil_40000", "coil", 40000, Uint16, 1.0),
            ("Discrete_50000", "discrete", 50000, Uint16, 1.0),
            ("Low_5", "holding", 5, Float64, 1.0),
        ]
    );
}
