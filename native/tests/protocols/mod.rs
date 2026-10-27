//! Data Manager › Protocols against the live plant: every route the six
//! device sections call, each device created, read, changed and deleted
//! again, so the plant ends as it started (its data sources included).
//! No device of these protocols answers on the CI runner, so devices point
//! at 127.0.0.1:1 (refused at once): scans and browses check their error or
//! empty paths, start and stop only spawn and stop pollers.

use std::time::Duration;

use serde::de::IgnoredAny;
use serde_json::{Value, json};
use voltrus_scada_native::api::protocols::{
    ActionAnswer, BacnetDeviceRow, BacnetDiscovery, BacnetObjectRow, CreatedId,
    DNP3_GROUP_VARIATIONS, DiscoveredDnp3, Dnp3DeviceRow, Dnp3PointRow, EnipDeviceRow, EnipTagRow,
    GroupVariation, ModbusDeviceRow, ModbusNetworkScan, ModbusRegisterRow, OpcUaDeviceRow,
    OpcUaNodeRow, PollerStatus, S7Browse, S7DeviceRow, S7TagRow, SimProfile, rows, rows_at,
};
use voltrus_scada_native::api::{DataSource, Method, VoltrusApi};
use voltrus_scada_native::device_file::{
    DeviceFileKind, device_file_kind, parse_bacnet_device_file, parse_modbus_device_file,
};

const SLOW: Duration = Duration::from_secs(60);

async fn get(api: &VoltrusApi, path: &str) -> Value {
    api.get_json(path)
        .await
        .unwrap_or_else(|error| panic!("GET {path}: {error}"))
}

async fn send(api: &VoltrusApi, method: Method, path: &str, body: Value) -> Value {
    api.send_json(method.clone(), path, &body)
        .await
        .unwrap_or_else(|error| panic!("{method} {path}: {error}"))
}

async fn post_empty(api: &VoltrusApi, path: &str) -> Value {
    api.send_empty(Method::POST, path)
        .await
        .unwrap_or_else(|error| panic!("POST {path}: {error}"))
}

async fn delete(api: &VoltrusApi, path: &str) {
    api.send_empty::<IgnoredAny>(Method::DELETE, path)
        .await
        .unwrap_or_else(|error| panic!("DELETE {path}: {error}"));
}

async fn created(api: &VoltrusApi, path: &str, body: Value) -> String {
    let answer: CreatedId = serde_json::from_value(send(api, Method::POST, path, body).await)
        .unwrap_or_else(|error| panic!("POST {path} answers {{ok, id}}: {error}"));
    answer.text()
}

async fn source_keys(api: &VoltrusApi) -> Vec<String> {
    let mut keys: Vec<String> = api
        .get_json::<Vec<DataSource>>("/api/v1/data-sources")
        .await
        .expect("GET /data-sources")
        .into_iter()
        .map(|source| source.key)
        .collect();
    keys.sort();
    keys
}

pub async fn run(url: &str, user: &str, pass: &str) {
    let mut api = VoltrusApi::new(url);
    api.login(user, pass).await.expect("sign in");
    let sources_before = source_keys(&api).await;
    let tag = std::process::id();

    modbus(&api, tag).await;
    opcua(&api, tag).await;
    s7(&api, tag).await;
    enip(&api, tag).await;
    dnp3(&api, tag).await;
    bacnet(&api, tag).await;

    assert_eq!(
        source_keys(&api).await,
        sources_before,
        "the plant's data sources are as they were"
    );
}

async fn modbus(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/modbus-devices";
    let before = rows::<ModbusDeviceRow>(get(api, base).await).expect("the device list's shape");
    println!("modbus: {} devices on the plant", before.len());
    let statuses: Vec<PollerStatus> =
        serde_json::from_value(get(api, &format!("{base}/status")).await)
            .expect("the poller status shape");
    println!("modbus: {} pollers report", statuses.len());

    // Import: a Modbus Poll device file, its registers by explicit offset.
    let id = format!("e2e_native_mb_{tag}");
    let file = json!({
        "id": id, "name": "E2E native Modbus", "ip": "127.0.0.1", "port": 1, "unit_id": 1,
        "interval_ms": 5000, "endianness": "swapped",
        "registers": [
            {"name": "power", "offset": 30000, "register_type": "float32", "register_area": "holding", "scale": 0.5},
            {"name": "alarm_bit", "offset": 7, "register_type": "uint16", "register_area": "coil", "scale": 1}
        ]
    })
    .to_string();
    assert_eq!(device_file_kind(&file), Some(DeviceFileKind::Modbus));
    let parsed = parse_modbus_device_file(&file);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let body = serde_json::to_value(&parsed.devices[0]).unwrap();
    assert_eq!(created(api, base, body).await, id);
    let path = format!("{base}/{id}");

    let detail = get(api, &path).await;
    let registers = rows_at::<ModbusRegisterRow>(detail.clone(), "registers").expect("registers");
    let power: ModbusRegisterRow = serde_json::from_value(registers[0].clone()).unwrap();
    assert_eq!(
        (power.address, power.zero_based),
        (30000, true),
        "an offset is kept as given"
    );
    assert_eq!(registers.len(), 2);
    assert_eq!(detail["device"]["endianness"], "swapped");

    // Edit, then add a register the way the console does (the whole list).
    send(
        api,
        Method::PUT,
        &path,
        json!({"name": "E2E native Modbus (edited)", "interval_ms": 2000, "endianness": "big"}),
    )
    .await;
    let mut inputs: Vec<Value> = registers
        .iter()
        .map(|row| {
            serde_json::from_value::<ModbusRegisterRow>(row.clone())
                .unwrap()
                .input()
        })
        .collect();
    inputs.push(json!({"name": "voltage", "offset": 12, "register_type": "int16", "register_area": "input", "scale": 1.0}));
    send(
        api,
        Method::PUT,
        &format!("{path}/registers"),
        json!({ "registers": inputs }),
    )
    .await;
    let detail = get(api, &path).await;
    assert_eq!(detail["device"]["name"], "E2E native Modbus (edited)");
    let registers = rows_at::<ModbusRegisterRow>(detail, "registers").unwrap();
    assert_eq!(registers.len(), 3);
    let listed = rows::<ModbusDeviceRow>(get(api, base).await).unwrap();
    let row = listed
        .iter()
        .find(|row| row["id"] == id)
        .expect("the device is listed");
    assert_eq!(row["register_count"], 3);

    post_empty(api, &format!("{path}/restart")).await;
    send(api, Method::PUT, &path, json!({"enabled": false})).await;
    send(api, Method::PUT, &path, json!({"enabled": true})).await;

    // Scans: nothing answers on 127.0.0.1:1.
    let scan = api
        .send_slow(
            Method::POST,
            &format!("{base}/scan"),
            Some(
                &json!({"ip": "127.0.0.1", "port": 1, "unit_id": 1, "start_addr": 0, "end_addr": 3,
                "endianness": "big", "register_area": "holding", "scan_delay_ms": 0}),
            ),
            SLOW,
        )
        .await;
    println!("modbus register scan of a refused port: {scan:?}");
    assert!(scan.is_err(), "a refused connection is an error");
    let network: ModbusNetworkScan = serde_json::from_value(
        api.send_slow(
            Method::POST,
            &format!("{base}/scan-network"),
            Some(&json!({"range": "127.0.0.1/32", "port": 1})),
            SLOW,
        )
        .await
        .expect("POST /modbus-devices/scan-network"),
    )
    .expect("the network scan's shape");
    assert_eq!(network.count, network.devices.len());

    // Simulation profiles, and the switch (the server simulates rows of
    // /devices, which a Modbus device has none of).
    let profiles = rows::<SimProfile>(get(api, "/api/v1/sim-profiles").await).expect("profiles");
    let name = format!("e2e native sine {tag}");
    let profile: SimProfile = serde_json::from_value(
        send(
            api,
            Method::POST,
            "/api/v1/sim-profiles",
            json!({"name": name, "data_type": "sinusoidal",
            "config": "{\"amplitude\":10,\"center\":50,\"period_sec\":60,\"noise_pct\":0}"}),
        )
        .await,
    )
    .expect("the created profile");
    assert_eq!(profile.name, name);
    let profile_path = format!("/api/v1/sim-profiles/{}", profile.id);
    let edited: SimProfile = serde_json::from_value(
        send(
            api,
            Method::PUT,
            &profile_path,
            json!({"data_type": "constant", "config": "{\"value\":42}"}),
        )
        .await,
    )
    .unwrap();
    assert_eq!(
        (edited.data_type.as_str(), edited.config.as_str()),
        ("constant", "{\"value\":42}")
    );
    let simulate = api
        .send_json::<_, IgnoredAny>(
            Method::POST,
            &format!("/api/v1/devices/{id}/simulate"),
            &json!({"simulation_mode": true, "sim_profile_id": profile.id, "sim_config": "{\"power\":5}"}),
        )
        .await;
    println!("simulate a Modbus device: {simulate:?}");
    let live = api
        .send_empty::<IgnoredAny>(Method::POST, &format!("/api/v1/devices/{id}/go-live"))
        .await;
    println!("go live: {live:?}");
    delete(api, &profile_path).await;
    let left = rows::<SimProfile>(get(api, "/api/v1/sim-profiles").await).unwrap();
    assert_eq!(left.len(), profiles.len(), "the profile is gone");

    // Remove a register (the whole list less it), then the device.
    let kept: Vec<Value> = registers
        .iter()
        .filter(|row| row["name"] != "voltage")
        .map(|row| {
            serde_json::from_value::<ModbusRegisterRow>(row.clone())
                .unwrap()
                .input()
        })
        .collect();
    send(
        api,
        Method::PUT,
        &format!("{path}/registers"),
        json!({ "registers": kept }),
    )
    .await;
    assert_eq!(
        rows_at::<ModbusRegisterRow>(get(api, &path).await, "registers")
            .unwrap()
            .len(),
        2
    );
    delete(api, &path).await;
    let after = rows::<ModbusDeviceRow>(get(api, base).await).unwrap();
    assert_eq!(after.len(), before.len(), "the device is gone");
    println!("modbus: imported, edited, scanned, simulated and deleted {id}");
}

async fn opcua(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/opcua/devices";
    let before = rows::<OpcUaDeviceRow>(get(api, base).await).expect("the OPC UA list's shape");
    let name = format!("e2e_native_opcua_{tag}");
    let id = created(
        api,
        base,
        json!({"name": name, "url": "opc.tcp://127.0.0.1:1",
        "security_policy": "None", "security_mode": "None", "interval_ms": 1000, "enabled": true}),
    )
    .await;
    let path = format!("{base}/{id}");
    let listed = rows::<OpcUaDeviceRow>(get(api, base).await).unwrap();
    let row = listed
        .iter()
        .find(|row| row["id"].to_string() == id.to_string())
        .expect("listed");
    assert_eq!(row["security_policy"], "None");
    assert_eq!(row["node_count"], 0);
    send(
        api,
        Method::PUT,
        &path,
        json!({"interval_ms": 2000, "security_mode": "Sign"}),
    )
    .await;
    assert!(
        rows::<OpcUaNodeRow>(get(api, &format!("{path}/nodes")).await)
            .unwrap()
            .is_empty()
    );
    created(
        api,
        &format!("{path}/nodes"),
        json!({"node_id": "ns=2;s=Temperature", "name": "temperature",
        "display_name": "Temperature", "data_type": "Double"}),
    )
    .await;
    let nodes = rows::<OpcUaNodeRow>(get(api, &format!("{path}/nodes")).await).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["node_id"], "ns=2;s=Temperature");
    assert!(
        source_keys(api)
            .await
            .contains(&format!("{name}:temperature")),
        "a node makes a data source"
    );
    post_empty(api, &format!("{path}/start")).await;
    post_empty(api, &format!("{path}/stop")).await;
    let browse = api
        .send_slow(
            Method::POST,
            &format!("{path}/browse"),
            None,
            Duration::from_secs(30),
        )
        .await;
    println!(
        "opcua browse of a refused port: {:?}",
        browse.as_ref().map(|_| "answered")
    );
    assert!(browse.is_err());
    delete(api, &format!("{path}/nodes/{}", nodes[0]["id"])).await;
    assert!(
        rows::<OpcUaNodeRow>(get(api, &format!("{path}/nodes")).await)
            .unwrap()
            .is_empty()
    );
    delete(api, &path).await;
    let after = rows::<OpcUaDeviceRow>(get(api, base).await).unwrap();
    assert_eq!(after.len(), before.len(), "the device is gone");
    println!("opcua: created, edited, monitored a node and deleted {name}");
}

async fn s7(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/s7/devices";
    let before = rows::<S7DeviceRow>(get(api, base).await).expect("the S7 list's shape");
    let name = format!("e2e_native_s7_{tag}");
    let id = created(
        api,
        base,
        json!({"name": name, "url": "127.0.0.1:1", "rack": 0, "slot": 1,
        "interval_ms": 1000, "enabled": true}),
    )
    .await;
    let path = format!("{base}/{id}");
    send(api, Method::PUT, &path, json!({"slot": 2})).await;
    let listed = rows::<S7DeviceRow>(get(api, base).await).unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|row| row["name"] == name)
            .expect("listed")["slot"],
        2
    );
    created(
        api,
        &format!("{path}/tags"),
        json!({"tag_address": "DB1.DBD0", "name": "flow",
        "display_name": "Flow", "data_type": "float32"}),
    )
    .await;
    let tags = rows::<S7TagRow>(get(api, &format!("{path}/tags")).await).unwrap();
    assert_eq!(tags[0]["tag_address"], "DB1.DBD0");
    let browse: S7Browse = serde_json::from_value(
        api.send_slow(
            Method::POST,
            &format!("{path}/browse"),
            None,
            Duration::from_secs(30),
        )
        .await
        .expect("POST /s7/devices/{id}/browse"),
    )
    .expect("the S7 browse shape");
    assert!(!browse.connected, "nothing listens on 127.0.0.1:1");
    assert!(browse.areas.iter().any(|area| area.area == "DB"));
    post_empty(api, &format!("{path}/start")).await;
    post_empty(api, &format!("{path}/stop")).await;
    delete(api, &format!("{path}/tags/{}", tags[0]["id"])).await;
    delete(api, &path).await;
    assert_eq!(
        rows::<S7DeviceRow>(get(api, base).await).unwrap().len(),
        before.len()
    );
    println!("s7: created, tagged, browsed and deleted {name}");
}

async fn enip(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/enip/devices";
    let before = rows::<EnipDeviceRow>(get(api, base).await).expect("the EtherNet/IP list's shape");
    let name = format!("e2e_native_enip_{tag}");
    let id = created(
        api,
        base,
        json!({"name": name, "url": "127.0.0.1:1", "slot": 0,
        "interval_ms": 1000, "assembly_instance": 0, "enabled": true}),
    )
    .await;
    let path = format!("{base}/{id}");
    send(api, Method::PUT, &path, json!({"interval_ms": 3000})).await;
    created(
        api,
        &format!("{path}/tags"),
        json!({"tag_name": "Program.Flow", "name": "flow",
        "display_name": "", "data_type": "real32"}),
    )
    .await;
    let tags = rows::<EnipTagRow>(get(api, &format!("{path}/tags")).await).unwrap();
    assert_eq!(tags[0]["tag_name"], "Program.Flow");
    let browse = api
        .send_slow(
            Method::POST,
            &format!("{path}/browse"),
            None,
            Duration::from_secs(30),
        )
        .await;
    assert!(browse.is_err(), "nothing listens on 127.0.0.1:1");
    post_empty(api, &format!("{path}/start")).await;
    post_empty(api, &format!("{path}/stop")).await;
    delete(api, &format!("{path}/tags/{}", tags[0]["id"])).await;
    delete(api, &path).await;
    assert_eq!(
        rows::<EnipDeviceRow>(get(api, base).await).unwrap().len(),
        before.len()
    );
    println!("enip: created, tagged and deleted {name}");
}

async fn dnp3(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/dnp3/devices";
    let before = rows::<Dnp3DeviceRow>(get(api, base).await).expect("the DNP3 list's shape");
    let variations: Vec<GroupVariation> =
        serde_json::from_value(get(api, "/api/v1/dnp3/group-variations").await).unwrap();
    let pairs: Vec<(u8, u8)> = variations
        .iter()
        .map(|gv| (gv.group, gv.variation))
        .collect();
    let ours: Vec<(u8, u8)> = DNP3_GROUP_VARIATIONS
        .iter()
        .map(|(g, v, _)| (*g, *v))
        .collect();
    assert_eq!(
        pairs, ours,
        "the point form offers the server's group/variations"
    );

    let name = format!("e2e_native_dnp3_{tag}");
    let id = created(
        api,
        base,
        json!({"name": name, "ip": "127.0.0.1", "port": 1,
        "master_address": 1, "outstation_address": 10, "enable_unsolicited": true,
        "integrity_poll_interval_secs": 300, "event_scan_interval_ms": null, "enabled": true}),
    )
    .await;
    let path = format!("{base}/{id}");
    send(
        api,
        Method::PUT,
        &path,
        json!({"outstation_address": 11, "event_scan_interval_ms": 500}),
    )
    .await;
    created(
        api,
        &format!("{path}/points"),
        json!({"name": "kw", "display_name": "kW",
        "group_num": 30, "variation": 4, "point_index": 0, "data_type": "float64",
        "scale": 1.0, "deadband": 0.0}),
    )
    .await;
    let mut detail = get(api, &path).await;
    let device: Dnp3DeviceRow = serde_json::from_value(detail["device"].take()).unwrap();
    assert_eq!(
        (device.outstation_address, device.event_scan_interval_ms),
        (11, Some(500))
    );
    let points = rows_at::<Dnp3PointRow>(get(api, &path).await, "points").unwrap();
    assert_eq!(points[0]["group_num"], 30);
    for (action, message) in [("integrity", "Integrity"), ("freeze", "Freeze")] {
        let answer: ActionAnswer =
            serde_json::from_value(post_empty(api, &format!("{path}/{action}")).await).unwrap();
        assert!(answer.message.unwrap_or_default().starts_with(message));
    }
    post_empty(api, &format!("{path}/start")).await;
    post_empty(api, &format!("{path}/stop")).await;
    let found: Vec<DiscoveredDnp3> = serde_json::from_value(
        api.send_slow(
            Method::POST,
            "/api/v1/dnp3/scan",
            Some(&json!({"range": "127.0.0.1/32", "port": 1})),
            SLOW,
        )
        .await
        .expect("POST /dnp3/scan"),
    )
    .unwrap();
    assert!(found.is_empty());
    delete(api, &format!("{path}/points/{}", points[0]["id"])).await;
    delete(api, &path).await;
    assert_eq!(
        rows::<Dnp3DeviceRow>(get(api, base).await).unwrap().len(),
        before.len()
    );
    println!("dnp3: created, pointed, polled and deleted {name}");
}

async fn bacnet(api: &VoltrusApi, tag: u32) {
    let base = "/api/v1/bacnet/devices";
    let before = rows::<BacnetDeviceRow>(get(api, base).await).expect("the BACnet list's shape");

    // Import: a BACnet Explorer device file, the device then its objects.
    let name = format!("e2e_native_bacnet_{tag}");
    let file =
        json!({"devices": [{"name": name, "ip": "127.0.0.1", "port": 1, "device_instance": 7,
        "objects": [{"name": "sat", "object_type": "analogInput", "object_instance": 1}]}]})
        .to_string();
    assert_eq!(device_file_kind(&file), Some(DeviceFileKind::Bacnet));
    let parsed = parse_bacnet_device_file(&file);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let device = &parsed.devices[0];
    let id = created(
        api,
        base,
        json!({"name": device.name, "ip": device.ip, "port": device.port,
        "device_instance": device.device_instance, "interval_ms": device.interval_ms}),
    )
    .await;
    let path = format!("{base}/{id}");
    for object in &device.objects {
        created(
            api,
            &format!("{path}/objects"),
            serde_json::to_value(object).unwrap(),
        )
        .await;
    }
    let listed = rows::<BacnetDeviceRow>(get(api, base).await).unwrap();
    let row = listed
        .iter()
        .find(|row| row["name"] == name)
        .expect("listed");
    assert_eq!(
        (row["object_count"].clone(), row["interval_ms"].clone()),
        (json!(1), json!(1000))
    );

    send(
        api,
        Method::PUT,
        &path,
        json!({"cov_enabled": true, "cov_lifetime": 600}),
    )
    .await;
    created(
        api,
        &format!("{path}/objects"),
        json!({"name": "fan", "display_name": "Fan",
        "object_type": "binaryOutput", "object_instance": 2, "property": "presentValue",
        "cov_increment": 0.5}),
    )
    .await;
    let objects = rows::<BacnetObjectRow>(get(api, &format!("{path}/objects")).await).unwrap();
    assert_eq!(objects.len(), 2, "objects come back as a bare list");
    post_empty(api, &format!("{path}/start")).await;
    post_empty(api, &format!("{path}/stop")).await;
    let browse = api
        .send_slow(
            Method::POST,
            &format!("{path}/browse"),
            None,
            Duration::from_secs(30),
        )
        .await;
    println!(
        "bacnet browse of 127.0.0.1:1: {:?}",
        browse.as_ref().map(|_| "answered")
    );
    let discovery = api
        .send_slow(
            Method::POST,
            "/api/v1/bacnet/discover",
            Some(&json!({"timeout_secs": 1})),
            SLOW,
        )
        .await;
    match discovery {
        Ok(answer) => {
            let found: BacnetDiscovery =
                serde_json::from_value(answer).expect("the discovery shape");
            println!("bacnet discovery: {} devices", found.count);
        }
        Err(error) => println!("bacnet discovery is not possible here: {error}"),
    }
    for object in &objects {
        delete(api, &format!("{path}/objects/{}", object["id"])).await;
    }
    assert!(
        rows::<BacnetObjectRow>(get(api, &format!("{path}/objects")).await)
            .unwrap()
            .is_empty()
    );
    delete(api, &path).await;
    assert_eq!(
        rows::<BacnetDeviceRow>(get(api, base).await).unwrap().len(),
        before.len()
    );
    println!("bacnet: imported, edited and deleted {name}");
}
