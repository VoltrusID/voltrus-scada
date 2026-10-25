//! Wire guards for the Data Manager's REST payloads: each fixture is the
//! shape the server's handler builds (`src/api/data_sources.rs`,
//! `shortcuts.rs`, `modbus_devices.rs`, `models::DataSource`).

use voltrus_scada_native::api::{
    Created, DataSource, ModbusDevice, ModbusDeviceDetail, TagShortcuts,
};

#[test]
fn a_data_source_row_decodes_every_field() {
    // `GET /api/v1/data-sources`: `Json(Vec<models::DataSource>)`.
    let body = r##"[{
        "id": 12, "key": "ac_current_l1", "name": "AC current L1", "unit": "A",
        "color": "#f97316", "min_val": 0.0, "max_val": 120.0, "warn_val": 80.0,
        "crit_val": 85.0, "base_val": 50.0, "sort_order": 3, "decimals": 2,
        "source_type": "modbus",
        "source_config": "{\"modbus_register_name\":\"ac_current_l1\",\"low_warn\":2}",
        "device_id": "inverter-1", "equipment_id": null, "chart_type": "gauge",
        "path": "plant/inverter-1/ac_current_l1", "on_delay_secs": 5, "off_delay_secs": 0
    }]"##;
    let list: Vec<DataSource> = serde_json::from_str(body).unwrap();
    let row = &list[0];
    assert_eq!(row.id, 12);
    assert_eq!(row.sort_order, 3);
    assert_eq!(row.device_id, "inverter-1");
    assert_eq!(row.equipment_id, None);
    assert_eq!(row.path.as_deref(), Some("plant/inverter-1/ac_current_l1"));
    assert_eq!(row.on_delay_secs, 5);
    let config: serde_json::Value = serde_json::from_str(&row.source_config).unwrap();
    assert_eq!(config["low_warn"], 2);
}

#[test]
fn creating_answers_the_new_id() {
    // `(StatusCode::CREATED, Json(json!({"id": id})))`.
    let created: Created = serde_json::from_str(r#"{"id": 41}"#).unwrap();
    assert_eq!(created.id, 41);
}

#[test]
fn shortcuts_map_ids_to_combos_and_skip_cleared_ones() {
    // `get_shortcuts`: `map.insert(s.data_source_id.to_string(), json!(s.shortcut))`;
    // a cleared shortcut is stored as "".
    let shortcuts: TagShortcuts =
        serde_json::from_str(r#"{"12": "Mod+Shift+K", "7": "", "x": "F2", "9": null}"#).unwrap();
    let by_id = shortcuts.by_id();
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[&12], "Mod+Shift+K");
    assert!(
        serde_json::from_str::<TagShortcuts>("{}")
            .unwrap()
            .by_id()
            .is_empty()
    );
}

#[test]
fn modbus_devices_decode_as_listed_and_with_registers() {
    // `list_modbus_devices` builds each row with `json!` plus `register_count`.
    let list: Vec<ModbusDevice> = serde_json::from_str(
        r#"[{
            "id": "inverter-1", "name": "Inverter 1", "ip": "127.0.0.1", "port": 5020,
            "unit_id": 1, "interval_ms": 1000, "endianness": "big", "enabled": true,
            "created_at": 1700000000, "updated_at": 1700000000, "register_count": 18
        }]"#,
    )
    .unwrap();
    assert_eq!(list[0].port, 5020);
    assert_eq!(list[0].register_count, 18);

    // `get_modbus_device`: `json!({"device": device, "registers": registers})`.
    let detail: ModbusDeviceDetail = serde_json::from_str(
        r#"{
            "device": {
                "id": "inverter-1", "name": "Inverter 1", "ip": "127.0.0.1", "port": 5020,
                "unit_id": 1, "interval_ms": 1000, "endianness": "big", "enabled": true,
                "created_at": 1700000000, "updated_at": 1700000000
            },
            "registers": [
                {"id": 3, "device_id": "inverter-1", "name": "ac_current_l1", "address": 40011,
                 "register_type": "float32", "register_area": "holding", "scale": 0.1,
                 "sort_order": 2, "zero_based": false},
                {"id": 4, "device_id": "inverter-1", "name": "frequency", "address": 12,
                 "register_type": "uint16", "register_area": "input", "scale": 1.0,
                 "sort_order": 3}
            ]
        }"#,
    )
    .unwrap();
    assert_eq!(detail.device.id, "inverter-1");
    assert_eq!(detail.device.register_count, 0, "not in the detail");
    assert_eq!(detail.registers.len(), 2);
    assert_eq!(detail.registers[0].address, 40011);
    assert!(
        !detail.registers[1].zero_based,
        "older servers omit zero_based"
    );
}
