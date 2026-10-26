use std::collections::HashMap;

use serde_json::json;

use super::*;

/// What `GET /screens/{id}/scada-equipment` answers for a screen the web
/// client saved (`ScadaView.tsx` `toAPI`): JSON text inside the rows, the
/// stacking order in `config.z`, rows sorted by id rather than by `z`.
const WEB_EQUIPMENT: &str = r#"{"items":[
  {"screen_id":"plant","equipment_id":"w_a","equipment_type":"tank_level","equipment_name":"T-1",
   "x":400,"y":120,"width":110,"height":180,"rotation":0,"sensor_bindings":"[7]",
   "config":"{\"label\":\"T-1\",\"showPercentage\":false,\"z\":2}"},
  {"screen_id":"plant","equipment_id":"w_b","equipment_type":"pump","equipment_name":"P-1",
   "x":100,"y":200,"width":90,"height":90,"rotation":90,"sensor_bindings":"[3]",
   "config":"{\"onThreshold\":\"0.5\",\"rotation\":\"90\",\"showValue\":false,\"z\":0}"},
  {"screen_id":"plant","equipment_id":"w_c","equipment_type":"symbol","equipment_name":"V-1",
   "x":250,"y":220,"width":90,"height":72,"rotation":0,"sensor_bindings":"[5, 9]",
   "config":"{\"symbol\":\"valve\",\"writable\":true,\"faultSensorId\":9,\"z\":1}"},
  {"screen_id":"plant","equipment_id":"w_d","equipment_type":"data_table","equipment_name":"Table",
   "x":600,"y":120,"width":320,"height":200,"rotation":0,"sensor_bindings":"[]",
   "config":"{}"}
]}"#;

const WEB_CONNECTIONS: &str = r#"{"items":[
  {"screen_id":"plant","connection_id":"c_1","from_equipment":"w_b","to_equipment":"w_c",
   "waypoints":"[]","connection_type":"flow"},
  {"screen_id":"plant","connection_id":"c_2","from_equipment":"w_c","to_equipment":"w_a",
   "waypoints":"[[380,256]]","connection_type":"power"}
]}"#;

fn web_layout() -> Layout {
    let equipment: Items<EquipmentRow> = serde_json::from_str(WEB_EQUIPMENT).unwrap();
    let connections: Items<ConnectionRow> = serde_json::from_str(WEB_CONNECTIONS).unwrap();
    parse(&equipment.items, &connections.items)
}

fn values(map: &[(i64, Option<f64>, Level)]) -> impl Values + '_ {
    move |tag: i64| {
        map.iter()
            .find(|(id, _, _)| *id == tag)
            .map(|(_, value, level)| Reading {
                value: *value,
                level: *level,
                min: 0.0,
                max: 100.0,
            })
    }
}

fn widget(kind: &str, config: serde_json::Value, tags: &[i64]) -> Widget {
    Widget {
        id: format!("w_{kind}"),
        kind: WidgetKind::parse(kind),
        name: String::new(),
        tags: tags.to_vec(),
        x: 0.0,
        y: 0.0,
        width: 90.0,
        height: 90.0,
        config: match config {
            serde_json::Value::Object(map) => map,
            _ => Map::new(),
        },
    }
}

#[test]
fn the_servers_rows_parse_in_stacking_order() {
    let layout = web_layout();
    let ids: Vec<&str> = layout.widgets.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["w_b", "w_c", "w_a", "w_d"], "by z, then row order");
    let pump = &layout.widgets[0];
    assert_eq!(pump.kind, WidgetKind::Pump);
    assert_eq!(pump.tags, [3]);
    assert!(!pump.config.contains_key("z"), "z travels in the order");
    assert_eq!(
        pump.number("onThreshold"),
        Some(0.5),
        "numbers saved as text"
    );
    assert_eq!(layout.widgets[1].tags, [5, 9]);
    assert_eq!(layout.widgets[1].tag_setting("faultSensorId"), Some(9));
    assert!(layout.widgets[3].tags.is_empty());
    assert_eq!(layout.connections.len(), 2);
    assert_eq!(layout.connections[0].line(), LineKind::Flow);
    assert_eq!(layout.connections[1].waypoints, [(380.0, 256.0)]);
    assert_eq!(layout.tags(), [3, 5, 9, 7]);
}

#[test]
fn a_layout_saves_back_in_the_servers_shape() {
    let layout = web_layout();
    let (equipment, connections) = to_rows(&layout);
    let body = serde_json::to_value(Items {
        items: equipment.clone(),
    })
    .unwrap();
    let first = &body["items"][0];
    assert_eq!(first["equipment_id"], "w_b");
    assert_eq!(first["equipment_type"], "pump");
    assert_eq!(
        first["rotation"], 90.0,
        "the column mirrors config.rotation"
    );
    assert!(first["sensor_bindings"].is_string() && first["config"].is_string());
    assert!(
        first.get("screen_id").is_none(),
        "the PUT takes it from the path"
    );
    let config: serde_json::Value =
        serde_json::from_str(first["config"].as_str().unwrap()).unwrap();
    assert_eq!(config["z"], 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&connections[1].waypoints).unwrap(),
        json!([[380.0, 256.0]])
    );
    // And reads back the same.
    assert_eq!(parse(&equipment, &connections), layout);
}

#[test]
fn a_feed_templates_rows_load_without_their_signal_names() {
    // `wizard/feed_instantiator.rs` binds signal names and calls its
    // connections "process".
    let equipment = [EquipmentRow {
        equipment_id: "eq_inverter_0".into(),
        equipment_type: "gauge".into(),
        equipment_name: "Inverter".into(),
        width: 250.0,
        height: 160.0,
        sensor_bindings: json!(["dc_power", "ac_power"]).to_string(),
        config: "{}".into(),
        ..EquipmentRow::default()
    }];
    let connections = [ConnectionRow {
        connection_id: "conn_s_0".into(),
        from_equipment: "eq_inverter_0".into(),
        to_equipment: "eq_meter_1".into(),
        waypoints: "[]".into(),
        connection_type: "process".into(),
        ..ConnectionRow::default()
    }];
    let layout = parse(&equipment, &connections);
    assert!(layout.widgets[0].tags.is_empty());
    assert_eq!(layout.connections[0].line(), LineKind::Data);
    // Broken JSON text reads as empty, not as a failed screen.
    let broken = [EquipmentRow {
        equipment_id: "x".into(),
        equipment_type: "mystery".into(),
        sensor_bindings: "not json".into(),
        config: "{".into(),
        ..EquipmentRow::default()
    }];
    let layout = parse(&broken, &[]);
    assert_eq!(layout.widgets[0].kind, WidgetKind::Other("mystery".into()));
    assert_eq!(layout.widgets[0].kind.as_str(), "mystery");
}

/// The CI plant's seed (`native/scripts/mimic-solar-farm.json`) turned into
/// rows the way `ci-server.sh` does, with made-up ids.
fn seeded() -> (Layout, HashMap<String, i64>) {
    let seed: serde_json::Value =
        serde_json::from_str(include_str!("../../scripts/mimic-solar-farm.json")).unwrap();
    let mut ids: HashMap<String, i64> = HashMap::new();
    let mut id_of = |name: &str| {
        let next = i64::try_from(ids.len()).unwrap() + 1;
        *ids.entry(name.to_owned()).or_insert(next)
    };
    let equipment: Vec<EquipmentRow> = seed["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(z, item)| {
            let tags: Vec<i64> = item["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tag| id_of(tag.as_str().unwrap()))
                .collect();
            let mut config = item["config"].as_object().unwrap().clone();
            for value in config.values_mut() {
                if let Some(name) = value.as_str().and_then(|text| text.strip_prefix('@')) {
                    *value = json!(id_of(name));
                }
            }
            config.insert("z".into(), json!(z));
            EquipmentRow {
                screen_id: String::new(),
                equipment_id: item["equipment_id"].as_str().unwrap().into(),
                equipment_type: item["equipment_type"].as_str().unwrap().into(),
                equipment_name: item["equipment_name"].as_str().unwrap().into(),
                x: item["x"].as_f64().unwrap(),
                y: item["y"].as_f64().unwrap(),
                width: item["width"].as_f64().unwrap(),
                height: item["height"].as_f64().unwrap(),
                rotation: 0.0,
                sensor_bindings: json!(tags).to_string(),
                config: serde_json::Value::Object(config).to_string(),
            }
        })
        .collect();
    let connections: Vec<ConnectionRow> = seed["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| ConnectionRow {
            screen_id: String::new(),
            connection_id: item["connection_id"].as_str().unwrap().into(),
            from_equipment: item["from_equipment"].as_str().unwrap().into(),
            to_equipment: item["to_equipment"].as_str().unwrap().into(),
            waypoints: item["waypoints"].to_string(),
            connection_type: item["connection_type"].as_str().unwrap().into(),
        })
        .collect();
    (parse(&equipment, &connections), ids)
}

#[test]
fn the_ci_plants_mimic_is_complete() {
    let (layout, ids) = seeded();
    assert!(layout.widgets.len() >= 40);
    for widget in &layout.widgets {
        assert!(
            !matches!(widget.kind, WidgetKind::Other(_)),
            "{} is a known type",
            widget.id
        );
        if widget.kind == WidgetKind::Symbol {
            let key = widget.text("symbol").unwrap();
            assert!(symbol_def(key).is_some(), "{key} is in the library");
        }
        assert!(
            widget.x >= 0.0 && widget.x + widget.width <= STAGE_W,
            "{}",
            widget.id
        );
        assert!(
            widget.y >= 0.0 && widget.y + widget.height <= STAGE_H,
            "{}",
            widget.id
        );
    }
    for connection in &layout.connections {
        let (Some(a), Some(b)) = (
            layout.widget(&connection.from),
            layout.widget(&connection.to),
        ) else {
            panic!("{} joins two widgets", connection.id);
        };
        let points = route(a, b, &connection.waypoints);
        assert!(points.len() >= 2);
        // Orthogonal: every leg is level or upright.
        for pair in points.windows(2) {
            let (dx, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
            assert!(
                dx.abs() < 1.0 || dy.abs() < 1.0,
                "{}: {pair:?}",
                connection.id
            );
        }
    }
    // The cooling loop flows while the pump runs (heatsink above 30).
    let temperature = ids["sma_inv_1/temperature"];
    let ac_power = ids["sma_inv_1/ac_power"];
    let remote_run = ids["sma_inv_1/remote_run"];
    let hot = [
        (temperature, Some(45.0), Level::Normal),
        (ac_power, Some(56_000.0), Level::Normal),
        (remote_run, Some(1.0), Level::Normal),
    ];
    let cold = [
        (temperature, Some(20.0), Level::Normal),
        (ac_power, Some(0.0), Level::Normal),
        (remote_run, Some(0.0), Level::Normal),
    ];
    let pipe = layout
        .connections
        .iter()
        .find(|c| c.id == "c_p101_hx")
        .unwrap();
    assert!(flowing(pipe, &layout, &values(&hot)));
    assert!(!flowing(pipe, &layout, &values(&cold)));
    // The inverter takes START/STOP on its run register; it runs, so STOP
    // is the one offered, and START once it stopped.
    let inverter = layout.widget("inv").unwrap();
    assert_eq!(inverter.tag(), Some(remote_run));
    let offered = commands(inverter, &values(&hot)).unwrap();
    assert_eq!(
        (offered.on.label.as_str(), offered.on.enabled),
        ("START", false)
    );
    assert_eq!(
        (offered.off.label.as_str(), offered.off.enabled),
        ("STOP", true)
    );
    let stopped = commands(inverter, &values(&cold)).unwrap();
    assert!(stopped.on.enabled && !stopped.off.enabled);
    assert!(!tab_order(&layout).is_empty());
}

#[test]
fn every_type_and_symbol_is_known() {
    for (kind, name) in WidgetKind::ALL {
        assert_eq!(WidgetKind::parse(name), kind);
        assert_eq!(kind.as_str(), name);
    }
    assert_eq!(SYMBOLS.len(), 31);
    let mut keys: Vec<&str> = SYMBOLS.iter().map(|def| def.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), SYMBOLS.len(), "keys are unique");
}

#[test]
fn legacy_widgets_resolve_to_library_symbols() {
    let layout = web_layout();
    let pump = resolve(&layout.widgets[0]).unwrap();
    assert_eq!(pump.def.key, "pump");
    assert!(pump.default_writable);
    assert_eq!(
        pump.show_value,
        Some(false),
        "no value badge on a legacy pump"
    );
    assert!(!pump.show_status, "its showValue was the status text");
    let tank = resolve(&layout.widgets[2]).unwrap();
    assert_eq!(tank.def.key, "tank");
    assert!(!tank.show_percent, "showPercentage: false");
    let valve = resolve(&layout.widgets[1]).unwrap();
    assert!(!valve.default_writable && valve.show_percent);
    let pipe = widget("pipe", json!({"direction": "vertical"}), &[]);
    assert_eq!(resolve(&pipe).unwrap().def.key, "pipe_vertical");
    let unknown = widget("symbol", json!({"symbol": "nope"}), &[]);
    assert_eq!(resolve(&unknown).unwrap().def.key, "pump");
    assert!(resolve(&widget("gauge", json!({}), &[1])).is_none());
}

#[test]
fn footers_and_ports_follow_the_drawing() {
    let plain = widget("symbol", json!({"symbol": "pump"}), &[]);
    let resolved = resolve(&plain).unwrap();
    assert_eq!(footer_height(footer_rows(&plain, &resolved)), 0.0);
    // A 90 box fits the 100 view box at 0.9.
    assert_eq!(port_point(&plain, Side::Left), (1.8, 47.3));
    let labelled = widget(
        "symbol",
        json!({"symbol": "tank", "label": "T-1", "showValue": true}),
        &[4],
    );
    let resolved = resolve(&labelled).unwrap();
    let rows = footer_rows(&labelled, &resolved);
    assert!(rows.label && rows.value && !rows.status);
    assert_eq!(
        footer_height(rows),
        footer::PAD + footer::LABEL + footer::BADGE + footer::GAP
    );
    // A quarter turn clockwise: the left nozzle faces up.
    let turned = widget("symbol", json!({"symbol": "pump", "rotation": 90}), &[]);
    let (x, y) = port_point(&turned, Side::Top);
    assert!(
        (x - 42.75).abs() < 0.11 && (y - 1.8).abs() < 0.11,
        "{x} {y}"
    );
    let flipped = widget("symbol", json!({"symbol": "pump", "flipX": true}), &[]);
    assert_eq!(port_point(&flipped, Side::Right), (88.2, 47.3));
    // Plain widgets attach at their edges' midpoints.
    assert_eq!(
        port_point(&widget("gauge", json!({}), &[]), Side::Bottom),
        (45.0, 90.0)
    );
}

#[test]
fn routes_are_straight_l_or_z_shaped() {
    let mut a = widget("gauge", json!({}), &[]);
    let mut b = widget("gauge", json!({}), &[]);
    b.x = 300.0;
    assert_eq!(route(&a, &b, &[]), [(90.0, 45.0), (300.0, 45.0)]);
    b.y = 100.0;
    assert_eq!(
        route(&a, &b, &[]),
        [(90.0, 45.0), (195.0, 45.0), (195.0, 145.0), (300.0, 145.0)]
    );
    // A busbar is tapped where the other end lines up.
    a = widget("symbol", json!({"symbol": "busbar"}), &[]);
    a.y = 400.0;
    a.width = 600.0;
    a.height = 14.0;
    b.x = 200.0;
    b.y = 100.0;
    let points = route(&b, &a, &[]);
    assert_eq!(points, [(245.0, 190.0), (245.0, 400.0)]);
    let (start, segments) = rounded(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], 10.0).unwrap();
    assert_eq!(start, (0.0, 0.0));
    assert_eq!(
        segments,
        [
            Segment::Line((90.0, 0.0)),
            Segment::Quad {
                control: (100.0, 0.0),
                to: (100.0, 10.0)
            },
            Segment::Line((100.0, 100.0)),
        ]
    );
}

#[test]
fn symbols_take_their_state_from_their_tags() {
    let pump = widget(
        "symbol",
        json!({"symbol": "pump", "faultSensorId": 2}),
        &[1],
    );
    let def = resolve(&pump).unwrap().def;
    let running = [(1, Some(1.0), Level::Normal), (2, Some(0.0), Level::Normal)];
    let live = symbol_live(&pump, def, &values(&running));
    assert_eq!(live.state, EquipState::Running);
    assert!(live.running && live.moving() && !live.blinking());
    let tripped = [
        (1, Some(1.0), Level::Critical),
        (2, Some(1.0), Level::Normal),
    ];
    let live = symbol_live(&pump, def, &values(&tripped));
    assert_eq!(live.state, EquipState::Fault, "the fault tag wins");
    assert!(live.running, "a tripped pump that runs still offers stop");
    assert_eq!(live.alarm, Some(Level::Critical));
    let waiting = [(1, None, Level::Normal)];
    assert_eq!(
        symbol_live(&pump, def, &values(&waiting)).state,
        EquipState::Stale
    );
    let tank = widget("symbol", json!({"symbol": "tank"}), &[1]);
    let def = resolve(&tank).unwrap().def;
    let live = symbol_live(&tank, def, &values(&[(1, Some(25.0), Level::Normal)]));
    assert_eq!((live.state, live.level), (EquipState::Idle, Some(0.25)));
    let pipe = widget("symbol", json!({"symbol": "pipe"}), &[1]);
    let def = resolve(&pipe).unwrap().def;
    let flow = symbol_live(&pipe, def, &values(&[(1, Some(3.0), Level::Normal)]));
    assert_eq!(
        flow.run_state,
        EquipState::Running,
        "a pipe flows on its tag"
    );
    assert_eq!(state_text(def, EquipState::Running), "RUNNING");
    let valve = symbol_def("valve").unwrap();
    assert_eq!(state_text(valve, EquipState::Stopped), "CLOSED");
    let breaker = symbol_def("breaker").unwrap();
    assert_eq!(state_text(breaker, EquipState::Running), "CLOSED");
}

#[test]
fn widget_states_map_like_the_web() {
    assert_eq!(fraction(50.0, 0.0, 200.0), 0.25);
    assert_eq!(fraction(5.0, 3.0, 3.0), 1.0, "an empty range counts as 1");
    assert_eq!(zone(50.0, 0.0, 100.0, 75.0, 90.0), Level::Normal);
    assert_eq!(zone(80.0, 0.0, 100.0, 75.0, 90.0), Level::Warning);
    assert_eq!(zone(95.0, 0.0, 100.0, 75.0, 90.0), Level::Critical);
    assert!(is_on(Some(1.0), 0.5) && !is_on(Some(0.5), 0.5) && !is_on(None, 0.5));
    let reading = |value, level| Reading {
        value,
        level,
        min: 0.0,
        max: 1.0,
    };
    assert_eq!(status_light(None, false, 0.5), Light::Dark);
    assert_eq!(
        status_light(Some(reading(None, Level::Normal)), true, 0.5),
        Light::Dark
    );
    assert_eq!(
        status_light(Some(reading(Some(0.0), Level::Critical)), true, 0.5),
        Light::Off
    );
    assert_eq!(
        status_light(Some(reading(Some(0.0), Level::Critical)), false, 0.5),
        Light::Critical
    );
    let all = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let table = widget("data_table", json!({}), &[9, 2]);
    assert_eq!(shown_tags(&table, &all), [9, 2], "binding order");
    assert_eq!(
        shown_tags(&widget("data_table", json!({}), &[]), &all),
        [1, 2, 3, 4, 5, 6]
    );
    let bars = widget("bar_chart", json!({"maxBars": 2}), &[9, 2, 5]);
    assert_eq!(
        shown_tags(&bars, &all),
        [2, 5],
        "plant order, at most maxBars"
    );
    let trend = widget("multi_trend", json!({}), &[]);
    assert_eq!(shown_tags(&trend, &all), [1, 2, 3, 4]);
    assert_eq!(shown_tags(&widget("gauge", json!({}), &[3]), &all), [3]);
}

#[test]
fn faceplate_commands_follow_the_equipment_and_the_role() {
    let valve = widget("valve", json!({}), &[1]);
    let open = [(1, Some(1.0), Level::Normal)];
    let offered = commands(&valve, &values(&open)).unwrap();
    assert_eq!(
        (
            offered.on.label.as_str(),
            offered.on.enabled,
            offered.off.label.as_str(),
            offered.off.enabled
        ),
        ("OPEN", false, "CLOSE", true)
    );
    let opted_out = widget("valve", json!({"writable": false}), &[1]);
    assert!(commands(&opted_out, &values(&open)).is_none());
    let symbol = widget("symbol", json!({"symbol": "pump"}), &[1]);
    assert!(
        commands(&symbol, &values(&open)).is_none(),
        "symbols opt in"
    );
    let gauge = widget("gauge", json!({}), &[1]);
    assert!(commands(&gauge, &values(&open)).is_none());
    let waiting = [(1, None, Level::Normal)];
    assert!(
        commands(&valve, &values(&waiting)).is_none(),
        "no value, no command"
    );
    let toggle = widget("toggle", json!({"onValue": 5, "offLabel": "HALT"}), &[1]);
    let offered = commands(&toggle, &values(&[(1, Some(0.0), Level::Normal)])).unwrap();
    assert_eq!((offered.on.value, offered.on.enabled), (5.0, true));
    assert_eq!(offered.off.label, "HALT");
    let slider = widget("slider", json!({"step": 5}), &[1]);
    let offered = commands(&slider, &values(&[(1, Some(40.0), Level::Normal)])).unwrap();
    let setpoint = offered.setpoint.unwrap();
    assert_eq!(setpoint.stepped(40.0, 2.0), 50.0);
    assert_eq!(
        setpoint.stepped(98.0, 1.0),
        100.0,
        "kept in the tag's range"
    );
    assert_eq!(
        write_route(true, true),
        Some(WriteMode::Direct),
        "admin: WebSocket"
    );
    assert_eq!(
        write_route(false, true),
        Some(WriteMode::Immediate),
        "operator: REST"
    );
    assert_eq!(write_route(false, false), None);
    let now = Instant::now();
    let armed = Armed {
        widget: "w".into(),
        tag: 1,
        label: "START".into(),
        value: 1.0,
        at: now,
    };
    assert!(!armed.expired(now + Duration::from_secs(5)));
    assert!(armed.expired(now + ARM_TIMEOUT));
    let pump = widget("pump", json!({"runSensorId": 4, "faultSensorId": 6}), &[1]);
    let rows = related(&pump);
    assert_eq!(
        rows.iter().map(|r| (r.label, r.tag)).collect::<Vec<_>>(),
        [("Run feedback", 1), ("Run", 4), ("Fault", 6)]
    );
    assert_eq!(related_text("Fault", true), "TRIPPED");
    assert_eq!(limit(0.0, 0.0, 100.0), None, "0 is not configured");
    assert_eq!(limit(80.0, 0.0, 100.0), Some(80.0));
    assert_eq!(
        low_limits(r#"{"low_warn": 10, "low_crit": 5}"#),
        (Some(10.0), Some(5.0))
    );
}

#[test]
fn the_banner_puts_unacknowledged_critical_alarms_first() {
    let alarm = |id: i64, severity: &str, acknowledged: bool, at: i64| Alarm {
        id,
        severity: severity.into(),
        triggered_at: at,
        acknowledged_at: acknowledged.then_some(at + 1),
        ..Alarm::default()
    };
    let mut cleared = alarm(5, "critical", false, 50);
    cleared.resolved_at = Some(60);
    let mut shelved = alarm(6, "critical", false, 50);
    shelved.shelved_until = Some(9_999);
    let alarms = [
        alarm(1, "warning", false, 10),
        alarm(2, "critical", true, 20),
        alarm(3, "critical", false, 5),
        alarm(4, "warning", false, 30),
        alarm(4, "warning", false, 30),
        cleared,
        shelved,
    ];
    let banner = banner(&alarms);
    assert_eq!(
        banner.alarms.iter().map(|a| a.id).collect::<Vec<_>>(),
        [3, 4, 1, 2]
    );
    assert_eq!(
        (banner.critical, banner.warning, banner.unacknowledged),
        (2, 2, 3)
    );
}

#[test]
fn the_camera_fits_zooms_about_the_pointer_and_pans() {
    let fit = Camera::fit(1000.0, 1000.0, 0.0);
    assert!((fit.zoom - 1000.0 / STAGE_W).abs() < 1e-9);
    assert!((fit.y - (1000.0 - STAGE_H * fit.zoom) / 2.0).abs() < 1e-9);
    let before = fit.to_stage(300.0, 400.0);
    let zoomed = fit.zoomed(2.0, 300.0, 400.0);
    let after = zoomed.to_stage(300.0, 400.0);
    assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
    assert_eq!(fit.zoomed(100.0, 0.0, 0.0).zoom, MAX_ZOOM);
    let one = Camera {
        zoom: 1.0,
        x: 0.0,
        y: 0.0,
    };
    assert_eq!(one.step(true), 1.25);
    assert_eq!(one.step(false), 0.9);
    let far = one.panned(-10_000.0, 0.0, 800.0, 600.0, 100.0);
    assert_eq!(
        far.x,
        100.0 - STAGE_W,
        "a strip of the stage stays on screen"
    );
    assert!(
        one.reveal((10.0, 10.0, 50.0, 50.0), 800.0, 600.0, 8.0)
            .is_none()
    );
    let moved = one
        .reveal((1000.0, 10.0, 50.0, 50.0), 800.0, 600.0, 8.0)
        .unwrap();
    assert_eq!(moved.x, 800.0 - 8.0 - 1050.0);
    let half = one.toward(&moved, 0.5);
    assert!((half.x - moved.x / 2.0).abs() < 1e-9);
}

#[test]
fn the_keyboard_visits_equipment_in_reading_order() {
    let mut layout = Layout::default();
    for (index, (x, y)) in [(500.0, 0.0), (0.0, 0.0), (0.0, 300.0), (500.0, 300.0)]
        .into_iter()
        .enumerate()
    {
        let mut w = widget("pump", json!({}), &[1]);
        w.id = format!("w{index}");
        w.x = x;
        w.y = y;
        layout.widgets.push(w);
    }
    layout.widgets.push(widget("label", json!({}), &[]));
    assert_eq!(tab_order(&layout), [1, 0, 2, 3]);
    let candidates = tab_order(&layout);
    assert_eq!(neighbor(&layout, 1, Direction::Right, &candidates), Some(0));
    assert_eq!(neighbor(&layout, 1, Direction::Down, &candidates), Some(2));
    assert_eq!(neighbor(&layout, 1, Direction::Up, &candidates), None);
    assert_eq!(neighbor(&layout, 3, Direction::Left, &candidates), Some(2));
}
