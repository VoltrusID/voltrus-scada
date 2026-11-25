use std::collections::HashSet;

use serde_json::{Value, json};

use super::resolve::{ModbusDevice, Status, World, apply_patch, resolve, widget_patch};
use super::*;

fn pump() -> EquipmentType {
    library::get("pump").unwrap().equipment_type
}

fn inst(v: Value) -> EquipmentInstance {
    serde_json::from_value(v).unwrap()
}

/// The WTP showcase: `wtp` holds p101a_run, p101b_run, p101b_fault, p301a_run, p301a_speed.
fn wtp() -> World {
    let mut w = World::default();
    let keys = [
        "p101a_run",
        "p101b_run",
        "p101b_fault",
        "p301a_run",
        "p301a_speed",
    ];
    for (i, k) in keys.iter().enumerate() {
        w.tags
            .insert(("wtp".into(), k.to_string()), json!(i as i64 + 1));
    }
    w.devices.insert("wtp".into());
    w.modbus.insert(
        "wtp".into(),
        ModbusDevice {
            registers: keys.iter().map(|k| k.to_string()).collect(),
            linked: false,
        },
    );
    w
}

#[test]
fn the_library_is_valid_and_names_known_symbols() {
    let entries = library::entries();
    assert_eq!(entries.len(), 7);
    for e in entries {
        let t = &e.equipment_type;
        assert!(t.problems().is_empty(), "{}: {:?}", t.id, t.problems());
        let fp = e.faceplate.expect("every starter type opens a faceplate");
        assert!(fp.problems().is_empty(), "{:?}", fp.problems());
        for tag in &t.tags {
            assert!(
                fp.roles.iter().any(|r| r.role == tag.role),
                "{}: role {} not on faceplate {}",
                t.id,
                tag.role,
                fp.id
            );
        }
    }
}

#[test]
fn prefixes_come_from_the_name() {
    assert_eq!(default_prefix("P-101A"), "p101a");
    assert_eq!(default_prefix("FCV 320"), "fcv320");
    let i = inst(json!({"name": "P-501A", "type": "pump", "device": "wtp", "prefix": "p301a"}));
    assert_eq!(i.prefix(), "p301a");
}

#[test]
fn patterns_expand_and_unknown_placeholders_are_named() {
    let vars = Vars {
        prefix: "p101a",
        name: "P-101A",
        device: "wtp",
        role: "run",
        label: "Running",
    };
    assert_eq!(
        expand("{prefix}_{role}", &vars, &|_| None).unwrap(),
        "p101a_run"
    );
    assert_eq!(
        expand("{name} {label} on {device}", &vars, &|_| None).unwrap(),
        "P-101A Running on wtp"
    );
    let e = expand("{prefix}_{colour}", &vars, &|_| None).unwrap_err();
    assert!(e.contains("{colour}"), "{e}");
    assert!(expand("{prefix", &vars, &|_| None).is_err());
}

#[test]
fn an_instance_binds_existing_tags_by_pattern_and_falls_back() {
    let t = pump();
    let i = inst(json!({"name": "P-101B", "type": "pump", "device": "wtp"}));
    let r = resolve(&t, &i, &wtp());
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    let s = |role: &str| r.role(role).unwrap().status;
    assert_eq!(s("run"), Status::Bound);
    assert_eq!(s("fault"), Status::Bound);
    // No p101b_cmd: the command writes the run tag.
    assert_eq!(s("cmd"), Status::Fallback);
    assert_eq!(r.role("cmd").unwrap().key, "p101b_run");
    assert_eq!(r.role("cmd").unwrap().tag_id, Some(json!(2)));
    // Optional and absent.
    assert_eq!(s("speed"), Status::Missing);
    assert_eq!(r.role("run").unwrap().reference, "wtp/p101b_run");

    let p = widget_patch(&t, &i, &r);
    assert_eq!(p.sensor_id, Some(json!(2)));
    assert_eq!(p.config["faultSensorId"], json!(3));
    assert_eq!(p.config["faceplate"], "pump");
    assert_eq!(
        p.config["faceplateMap"],
        json!({"run": 2, "fault": 3, "cmd": 2})
    );
    assert_eq!(p.config["symbol"], "isa_pump_centrifugal");
    assert_eq!(p.config["showStatus"], true);
    assert!(p.remove.contains(&"runSensorId".to_string()));
}

#[test]
fn a_prefix_and_an_override_reach_other_keys() {
    let t = pump();
    let i = inst(
        json!({"name": "P-501A", "type": "pump", "device": "wtp", "prefix": "p301a",
        "overrides": {"fault": {"skip": true}, "speed": {"unit": "rpm", "max": 1500}}}),
    );
    let r = resolve(&t, &i, &wtp());
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.role("fault").unwrap().status, Status::Skipped);
    let speed = r.role("speed").unwrap();
    assert_eq!(speed.status, Status::Bound);
    assert_eq!(speed.attrs.unit.as_deref(), Some("rpm"));
    assert_eq!(speed.attrs.max, Some(1500.0));
    assert_eq!(speed.attrs.min, Some(0.0), "type's min kept");
}

#[test]
fn a_base_on_a_modbus_device_creates_registers() {
    let t = pump();
    let mut w = wtp();
    w.modbus.insert("plc2".into(), ModbusDevice::default());
    w.devices.insert("plc2".into());
    let i = inst(json!({"name": "P-201", "type": "pump", "device": "plc2", "base": 100}));
    let r = resolve(&t, &i, &w);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    for (role, addr) in [("run", 100), ("fault", 102), ("cmd", 104), ("speed", 106)] {
        let x = r.role(role).unwrap();
        assert_eq!(x.status, Status::Created, "{role}");
        assert_eq!(x.register.as_ref().unwrap().address, addr);
        assert_eq!(x.tag_id, None);
    }
    assert_eq!(
        r.role("cmd").unwrap().key,
        "p201_cmd",
        "own register, no fallback"
    );
}

#[test]
fn problems_name_what_is_wrong() {
    let t = pump();
    let w = wtp();
    // Required run tag missing, no base to create it.
    let r = resolve(
        &t,
        &inst(json!({"name": "P-999", "type": "pump", "device": "wtp"})),
        &w,
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("required role 'run'") && p.contains("p999_run")),
        "{:?}",
        r.problems
    );
    // Unknown device.
    let r = resolve(
        &t,
        &inst(json!({"name": "P-1", "type": "pump", "device": "nope"})),
        &w,
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("device 'nope' does not exist"))
    );
    // A base on a device that is not Modbus.
    let mut w2 = w.clone();
    w2.devices.insert("opc".into());
    let r = resolve(
        &t,
        &inst(json!({"name": "P-1", "type": "pump", "device": "opc", "base": 0})),
        &w2,
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("base needs a Modbus device")),
        "{:?}",
        r.problems
    );
    // A linked device cannot get registers.
    let mut w3 = w.clone();
    w3.modbus.get_mut("wtp").unwrap().linked = true;
    let r = resolve(
        &t,
        &inst(json!({"name": "P-7", "type": "pump", "device": "wtp", "base": 500})),
        &w3,
    );
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("follows a device template")),
        "{:?}",
        r.problems
    );

    let i = inst(
        json!({"name": "a/b", "type": "Pump!", "device": "", "overrides": {"nope": {}, "run": {"skip": true}}}),
    );
    let p = i.problems(Some(&t)).join("\n");
    assert!(p.contains("must not contain '/'"), "{p}");
    assert!(p.contains("type 'Pump!'"), "{p}");
    assert!(p.contains("device must not be empty"), "{p}");
    assert!(
        p.contains("overrides.nope: type 'pump' has no role 'nope'"),
        "{p}"
    );
    assert!(
        p.contains("overrides.run.skip: role 'run' is required"),
        "{p}"
    );
}

#[test]
fn type_problems_name_the_field() {
    let mut t = pump();
    t.id = "Pump".into();
    t.symbol = "teapot".into();
    t.tags[1].role = "run".into();
    t.tags[2].kind = "button".into();
    t.tags[2].fallback = Some("nope".into());
    t.tags[3].key = "{prefix}_{colour}".into();
    t.bindings.fault = Some("trip".into());
    t.symbol_config.insert("faceplateMap".into(), json!({}));
    t.flows =
        vec![json!({"id": "x", "name": "X", "trigger": {}, "nodes": [{"tag": "{ref:ghost}"}]})];
    let p = t.problems().join("\n");
    for want in [
        "id 'Pump'",
        "symbol 'teapot' is not a known symbol",
        "tags[1].role 'run' is listed twice",
        "tags[2].kind 'button' is not one of",
        "tags[2].fallback names an unknown role 'nope'",
        "tags[3].key: '{prefix}_{colour}' uses an unknown placeholder {colour}",
        "bindings.fault names an unknown role 'trip'",
        "symbol_config.faceplateMap",
        "flows[0].nodes[0].tag",
    ] {
        assert!(p.contains(want), "missing {want:?} in\n{p}");
    }
    let e = serde_json::from_value::<EquipmentType>(
        json!({"id": "a", "name": "A", "symbol": "isa_pump", "colour": 1}),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("colour"), "{e}");
}

#[test]
fn flows_are_copied_per_instance_with_refs() {
    let mut t = pump();
    t.flows = vec![json!({
        "id": "trip-stop", "name": "{name}: stop on trip", "enabled": true,
        "trigger": {"type": "tag_change", "tags": ["{ref:fault}"]},
        "nodes": [{"type": "write_out", "id": "n1", "tag": "{ref:cmd}", "value": 0}]
    })];
    assert!(t.problems().is_empty(), "{:?}", t.problems());
    let r = resolve(
        &t,
        &inst(json!({"name": "P-101B", "type": "pump", "device": "wtp"})),
        &wtp(),
    );
    assert_eq!(r.flows.len(), 1);
    let f = &r.flows[0];
    assert_eq!(f["id"], "p101b-trip-stop");
    assert_eq!(f["name"], "P-101B: stop on trip");
    assert_eq!(f["trigger"]["tags"][0], "wtp/p101b_fault");
    assert_eq!(
        f["nodes"][0]["tag"], "wtp/p101b_run",
        "cmd falls back to run"
    );
    // P-101A has no fault tag: the flow is left out with a note.
    let r = resolve(
        &t,
        &inst(json!({"name": "P-101A", "type": "pump", "device": "wtp"})),
        &wtp(),
    );
    assert!(r.flows.is_empty());
    assert!(r.notes[0].contains("trip-stop"), "{:?}", r.notes);
}

#[test]
fn applying_a_patch_twice_changes_nothing_and_keeps_a_label() {
    let t = pump();
    let i = inst(json!({"name": "P-101B", "type": "pump", "device": "wtp"}));
    let p = widget_patch(&t, &i, &resolve(&t, &i, &wtp()));
    let mut kind = "pump".to_string();
    let mut sensors = vec![json!(99)];
    let mut config = json!({"equipment": "P-101B", "label": "Intake B", "runSensorId": 7, "z": 3})
        .as_object()
        .cloned()
        .unwrap();
    assert!(apply_patch(&p, &mut kind, &mut sensors, &mut config));
    assert_eq!(kind, "symbol");
    assert_eq!(sensors, vec![json!(2)]);
    assert_eq!(config["label"], "Intake B", "a label is kept");
    assert_eq!(config["z"], 3, "other keys are kept");
    assert!(!config.contains_key("runSensorId"));
    assert!(
        !apply_patch(&p, &mut kind, &mut sensors, &mut config),
        "idempotent"
    );
    let mut empty = serde_json::Map::new();
    apply_patch(&p, &mut kind, &mut sensors, &mut empty);
    assert_eq!(empty["label"], "P-101B");
}

#[test]
fn symbol_keys_include_the_library_files() {
    // build.rs lists symbols/*.json; code symbols are always known.
    assert!(symbols::is_known("isa_pump"));
    assert!(symbols::is_known("pump"));
    assert!(!symbols::is_known("teapot"));
    let all: HashSet<&str> = symbols::all().into_iter().collect();
    for k in symbols::SYMBOL_FILES {
        assert!(all.contains(k));
    }
}
