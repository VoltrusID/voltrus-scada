//! The Designer's rules: the binding tests of the web Designer
//! (`client/tests/designerBindings.test.ts`) run against the Rust port, the
//! editor model's undo and redo, and the save compared with what the web
//! Designer's own code saves after the same edits
//! (`tests/fixtures/designer/web-save.json`, written by `web-save.mts`).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value as Json, json};

use super::bindings::{
    Binding, MapCase, PropType, apply_bindings, binding_errors, binding_refs, evaluate_binding,
    ref_index, tag_ref, with_binding,
};
use super::expr::{Cache, ExprError, Value, evaluate, parse, refs, to_fixed};
use super::*;
use crate::mimic::edit::History;
use crate::mimic::{ConnectionRow, EquipmentRow};

fn tags() -> HashMap<&'static str, Option<f64>> {
    HashMap::from([
        ("wtp/level", Some(87.5)),
        ("wtp/run", Some(1.0)),
        ("wtp/fault", Some(0.0)),
        ("wtp/flow", Some(1234.567)),
        ("wtp/none", None),
    ])
}

fn read(reference: &str) -> Option<f64> {
    tags().get(reference).copied().flatten()
}

fn ev(src: &str) -> Value {
    evaluate(&parse(src).expect(src), &read)
}

fn num(x: f64) -> Value {
    Value::Num(x)
}

fn text(s: &str) -> Value {
    Value::Str(s.into())
}

#[test]
fn expressions_arithmetic_precedence_comparison_logic_ternary() {
    assert_eq!(ev("1 + 2 * 3"), num(7.0));
    assert_eq!(ev("(1 + 2) * 3"), num(9.0));
    assert_eq!(ev("10 - 4 - 3"), num(3.0), "left associative");
    assert_eq!(ev("-2 * 3"), num(-6.0));
    assert_eq!(ev("{wtp/level} > 85"), Value::Bool(true));
    assert_eq!(ev("{wtp/level} > 85 ? \"HIGH\" : \"OK\""), text("HIGH"));
    assert_eq!(ev("{wtp/run} && !{wtp/fault}"), Value::Bool(true));
    assert_eq!(ev("{wtp/fault} || {wtp/run} == 1"), Value::Bool(true));
    assert_eq!(ev("\"P-101 \" + text({wtp/flow}, 1)"), text("P-101 1234.6"));
    assert_eq!(ev("round({wtp/flow} / 100, 1)"), num(12.3));
    assert_eq!(ev("clamp(150, 0, 100)"), num(100.0));
    assert_eq!(ev("scale(50, 0, 100, 4, 20)"), num(12.0));
    assert_eq!(ev("bit(5, 2)"), Value::Bool(true));
    assert_eq!(ev("if({wtp/fault}, \"TRIP\", \"ok\")"), text("ok"));
    assert_eq!(ev("1 / 0"), Value::Null, "division by zero has no value");
}

#[test]
fn javascript_text_rules() {
    assert_eq!(ev("\"n=\" + 3"), text("n=3"), "integers without a fraction");
    assert_eq!(ev("\"\" + 0.1 * 3"), text("0.30000000000000004"));
    assert_eq!(ev("\"\" + true"), text("true"));
    assert_eq!(
        ev("\"a\" + {wtp/none}"),
        text("a"),
        "no value joins as empty"
    );
    assert_eq!(ev("\"\" + 1e21"), text("1e+21"));
    assert_eq!(to_fixed(0.5, 0.0), "1", "toFixed: ties away from zero");
    assert_eq!(to_fixed(-2.5, 0.0), "-3");
    assert_eq!(
        to_fixed(1.005, 2.0),
        "1.00",
        "1.005 is below the tie in binary"
    );
    assert_eq!(
        ev("round(-2.5)"),
        num(-2.0),
        "Math.round: halves toward +inf"
    );
    assert_eq!(
        ev("\"1\" == 1"),
        Value::Bool(false),
        "text is never loosely equal"
    );
    assert_eq!(ev("true == 1"), Value::Bool(true));
    assert_eq!(ev("min(3, 1, 2) + max(4, 9)"), num(10.0));
}

#[test]
fn a_missing_tag_or_value_gives_no_value_instead_of_zero() {
    assert_eq!(ev("{wtp/none} + 1"), Value::Null);
    assert_eq!(ev("{nope/x} * 2"), Value::Null);
    assert_eq!(ev("text({wtp/none})"), text(""));
}

#[test]
fn parse_errors_name_the_problem_and_position() {
    for bad in [
        "1 +",
        "(1 + 2",
        "{wtp/level",
        "foo(1)",
        "round()",
        "1 2",
        "\"abc",
        "{}",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
    assert_eq!(
        parse("1 +"),
        Err(ExprError {
            message: "unexpected end of expression".into(),
            pos: 3
        })
    );
    assert_eq!(parse("foo(1)").unwrap_err().message, "unknown name \"foo\"");
    assert_eq!(
        parse("round()").unwrap_err().message,
        "round(x, decimals?) takes 1-2 arguments"
    );
    assert_eq!(parse("1 2").unwrap_err().message, "unexpected \"2\" at 3");
    let found: Vec<String> = refs(&parse("{a/b} + max({c/d}, {a/b})").unwrap())
        .into_iter()
        .collect();
    assert_eq!(found, ["a/b", "c/d"]);
    let mut cache = Cache::default();
    assert!(cache.compile("1 +").is_err());
    assert!(cache.compile("1 + 1").is_ok());
}

#[test]
fn tag_expression_and_map_bindings_evaluate_with_the_property_type() {
    let mut cache = Cache::default();
    let mut eval = |b: &Binding, kind| evaluate_binding(b, &read, Some(kind), &mut cache);
    let tag = |t: &str| Binding::Tag { tag: t.into() };
    assert_eq!(eval(&tag("wtp/run"), PropType::Boolean), Some(json!(true)));
    assert_eq!(
        eval(&tag("wtp/level"), PropType::String),
        Some(json!("87.5"))
    );
    assert_eq!(eval(&tag("wtp/none"), PropType::Number), None);
    let expr = Binding::Expr {
        expr: "{wtp/level} / 100".into(),
    };
    assert_eq!(eval(&expr, PropType::Number), Some(json!(0.875)));
    let map = Binding::Map {
        expr: "{wtp/level}".into(),
        cases: vec![
            MapCase {
                op: ">=".into(),
                v: 95.0,
                out: json!("#f04646"),
            },
            MapCase {
                op: ">=".into(),
                v: 85.0,
                out: json!("#f5b21b"),
            },
        ],
        otherwise: Some(json!("#8b939e")),
    };
    assert_eq!(eval(&map, PropType::Color), Some(json!("#f5b21b")));
    assert_eq!(
        evaluate_binding(&map, &|_| Some(10.0), Some(PropType::Color), &mut cache),
        Some(json!("#8b939e"))
    );
    let state: Binding = serde_json::from_value(json!({
        "kind": "map", "expr": "{wtp/run} == 1", "cases": [{ "op": "==", "v": 1, "out": "RUN" }]
    }))
    .unwrap();
    assert_eq!(
        evaluate_binding(&state, &read, Some(PropType::String), &mut cache),
        Some(json!("RUN"))
    );
}

#[test]
fn apply_bindings_replaces_bound_properties_and_keeps_the_rest() {
    let mut cache = Cache::default();
    let types = |prop: &str| match prop {
        "label" => Some(PropType::String),
        "decimals" => Some(PropType::Number),
        _ => None,
    };
    let mut config = json!({ "label": "fixed", "decimals": 1 })
        .as_object()
        .cloned()
        .unwrap();
    assert_eq!(apply_bindings(&config, &read, &types, &mut cache), None);
    for (prop, binding) in [
        (
            "label",
            json!({ "kind": "expr", "expr": "{wtp/level} > 85 ? \"HIGH\" : \"OK\"" }),
        ),
        ("visible", json!({ "kind": "tag", "tag": "wtp/fault" })),
        ("decimals", json!({ "kind": "expr", "expr": "{wtp/none}" })),
    ] {
        let binding: Binding = serde_json::from_value(binding).unwrap();
        let value = with_binding(&config, prop, Some(&binding));
        config.insert("bindings".into(), value);
    }
    let out = apply_bindings(&config, &read, &types, &mut cache).unwrap();
    assert_eq!(out["label"], json!("HIGH"));
    assert_eq!(out["visible"], json!(false));
    assert_eq!(out["decimals"], json!(1), "no value: the fixed value stays");
    let found: Vec<String> = binding_refs(&config).into_iter().collect();
    assert_eq!(found, ["wtp/fault", "wtp/level", "wtp/none"]);
    // Unbinding the last property removes the bindings key.
    let one = json!({ "a": 1, "bindings": { "label": { "kind": "tag", "tag": "x/y" } } });
    assert_eq!(
        with_binding(one.as_object().unwrap(), "label", None),
        Json::Null
    );
}

#[test]
fn refs_are_device_and_key_and_errors_name_unknown_tags() {
    assert_eq!(tag_ref("wtp", "level"), "wtp/level");
    assert_eq!(tag_ref("", "calc"), "/calc");
    let index = ref_index([(4, "wtp", "level"), (9, "", "calc"), (11, "wtp", "level")]);
    assert_eq!(index["wtp/level"], 4, "the first tag of a ref wins");
    let known = |r: &str| index.contains_key(r);
    assert!(
        binding_errors(
            &Binding::Tag {
                tag: "wtp/level".into()
            },
            &known
        )
        .is_empty()
    );
    assert_eq!(
        binding_errors(
            &Binding::Expr {
                expr: "{wtp/level} + {wtp/gone}".into()
            },
            &known
        ),
        ["Unknown tag {wtp/gone}"]
    );
    assert_eq!(
        binding_errors(&Binding::Expr { expr: "1 +".into() }, &known).len(),
        1
    );
}

// ── The editor model and the save ─────────────────────────────────────────

#[derive(Deserialize)]
struct Input {
    equipment: Vec<EquipmentRow>,
    connections: Vec<ConnectionRow>,
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct WebSave {
    undo_depth: usize,
    equipment: Vec<EquipmentRow>,
    connections: Vec<ConnectionRow>,
}

fn input() -> Input {
    serde_json::from_str(include_str!("../../tests/fixtures/designer/wtp-rows.json")).unwrap()
}

fn widget<'a>(history: &'a History, id: &str) -> &'a Widget {
    history.present().widget(id).expect(id)
}

/// The edits `web-save.mts` makes with the web Designer's model, here
/// through the console's [`History`]; the undo depth after them.
fn edited() -> (History, usize) {
    let input = input();
    let mut h = History::new(mimic::parse(&input.equipment, &input.connections));
    let mut steps = 0;
    let apply = |h: &mut History, edit: Edit, key: Option<&str>| {
        assert!(h.apply(&edit, key), "{edit:?} changes the screen");
    };
    // 1. Drag P-101A.
    apply(
        &mut h,
        Edit::Move(vec![("e012".into(), 200.0, 180.0)]),
        None,
    );
    steps += 1;
    // 2. Resize T-101.
    let resize = Edit::Resize {
        id: "e016".into(),
        x: 448.0,
        y: 250.0,
        width: 120.0,
        height: 230.0,
    };
    apply(&mut h, resize, None);
    steps += 1;
    // 3. Type a label, one key at a time (one step).
    for label in ["XV-101 ", "XV-101 I", "XV-101 Intake"] {
        let typed = set_label(widget(&h, "e011"), label);
        apply(&mut h, typed, Some("e011:label"));
    }
    h.seal();
    steps += 1;
    // 4. Bind the first KPI to another tag: the stored name goes.
    let bind = bind_primary(widget(&h, "e003"), Some(40));
    apply(&mut h, bind, None);
    steps += 1;
    // 5. An expression on T-101's label.
    let expr = Binding::Expr {
        expr: "{wtp/raw_tank_level} > 85 ? \"HIGH\" : \"T-101 \" + text({wtp/raw_tank_level}, 1) + \" %\"".into(),
    };
    let bound = set_binding(widget(&h, "e016"), "label", Some(&expr));
    apply(&mut h, bound, None);
    steps += 1;
    // 6. P-101B shows only while it runs; undone and redone.
    let shows = Binding::Tag {
        tag: "wtp/p101b_run".into(),
    };
    let visible = set_binding(widget(&h, "e013"), "visible", Some(&shows));
    apply(&mut h, visible, None);
    steps += 1;
    assert!(h.undo());
    assert!(!bindings::has_bindings(&widget(&h, "e013").config));
    assert!(h.redo());
    // 7. A state map on the trend's colour.
    let map: Binding = serde_json::from_value(json!({
        "kind": "map", "expr": "{wtp/raw_turbidity}",
        "cases": [{ "op": ">=", "v": 32, "out": "#f04646" }, { "op": ">=", "v": 25, "out": "#f5b21b" }],
        "else": "var(--cyan)"
    }))
    .unwrap();
    let coloured = set_binding(widget(&h, "e033"), "lineColor", Some(&map));
    apply(&mut h, coloured, None);
    steps += 1;
    // 8. A move that is undone: not saved.
    apply(
        &mut h,
        Edit::Move(vec![("e001".into(), 400.0, 400.0)]),
        None,
    );
    assert!(h.undo());
    (h, steps)
}

#[test]
fn undo_and_redo_step_through_designer_edits() {
    let (mut h, steps) = edited();
    assert!(h.dirty());
    let after = h.present().clone();
    let mut undone = 0;
    while h.undo() {
        undone += 1;
    }
    assert_eq!(undone, steps, "one step per edit; the typing is one");
    assert!(!h.dirty(), "all the way back is the loaded screen");
    assert!(!h.can_undo());
    for _ in 0..steps {
        assert!(h.redo());
    }
    assert_eq!(h.present(), &after, "redo replays every edit");
    assert!(h.can_redo(), "the undone move is still there to redo");
    // Rebinding the primary tag and its name are one step.
    let before = h.present().clone();
    let rebind = bind_primary(widget(&h, "e012"), None);
    assert!(h.apply(&rebind, None));
    assert!(widget(&h, "e012").tags.is_empty() && widget(&h, "e012").name.is_empty());
    assert!(h.undo());
    assert_eq!(h.present(), &before);
}

/// Numbers compared as numbers (`24` and `24.0` alike), the rest as is.
fn same_json(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_json(a, b))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

fn parsed(text: &str) -> Json {
    serde_json::from_str(text).expect("JSON text")
}

#[test]
fn the_save_matches_the_web_designer() {
    let web: WebSave =
        serde_json::from_str(include_str!("../../tests/fixtures/designer/web-save.json")).unwrap();
    let names: HashMap<i64, String> = input().tags.into_iter().map(|t| (t.id, t.name)).collect();
    let (h, steps) = edited();
    assert_eq!(steps, web.undo_depth, "the same number of undo steps");
    let (equipment, connections) = rows(h.present(), &|id| names.get(&id).cloned());
    assert_eq!(equipment.len(), web.equipment.len());
    for (ours, theirs) in equipment.iter().zip(&web.equipment) {
        let id = &ours.equipment_id;
        assert_eq!(ours.equipment_id, theirs.equipment_id);
        assert_eq!(ours.equipment_type, theirs.equipment_type, "{id}");
        assert_eq!(ours.equipment_name, theirs.equipment_name, "{id}: name");
        assert_eq!(
            (ours.x, ours.y, ours.width, ours.height, ours.rotation),
            (
                theirs.x,
                theirs.y,
                theirs.width,
                theirs.height,
                theirs.rotation
            ),
            "{id}: box"
        );
        assert!(
            same_json(
                &parsed(&ours.sensor_bindings),
                &parsed(&theirs.sensor_bindings)
            ),
            "{id}: tags"
        );
        assert!(
            same_json(&parsed(&ours.config), &parsed(&theirs.config)),
            "{id}: config\n ours: {}\n web:  {}",
            ours.config,
            theirs.config
        );
    }
    assert_eq!(connections.len(), web.connections.len());
    for (ours, theirs) in connections.iter().zip(&web.connections) {
        assert_eq!(
            (
                &ours.connection_id,
                &ours.from_equipment,
                &ours.to_equipment
            ),
            (
                &theirs.connection_id,
                &theirs.from_equipment,
                &theirs.to_equipment
            )
        );
        assert_eq!(ours.connection_type, theirs.connection_type);
        assert!(same_json(
            &parsed(&ours.waypoints),
            &parsed(&theirs.waypoints)
        ));
    }
}

#[test]
fn a_saved_screen_reads_back_as_edited() {
    let names: HashMap<i64, String> = input().tags.into_iter().map(|t| (t.id, t.name)).collect();
    let (h, _) = edited();
    let (equipment, connections) = rows(h.present(), &|id| names.get(&id).cloned());
    // What the server stores: `save_scada_equipment` takes the fields one
    // by one from the JSON body (`src/api/layouts.rs`).
    let body = serde_json::to_value(mimic::Items {
        items: equipment.clone(),
    })
    .unwrap();
    let stored: Vec<EquipmentRow> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| EquipmentRow {
            screen_id: "wtp".into(),
            equipment_id: item["equipment_id"].as_str().unwrap_or("").into(),
            equipment_type: item["equipment_type"].as_str().unwrap_or("").into(),
            equipment_name: item["equipment_name"].as_str().unwrap_or("").into(),
            x: item["x"].as_f64().unwrap_or(0.0),
            y: item["y"].as_f64().unwrap_or(0.0),
            width: item["width"].as_f64().unwrap_or(100.0),
            height: item["height"].as_f64().unwrap_or(60.0),
            rotation: item["rotation"].as_f64().unwrap_or(0.0),
            sensor_bindings: item["sensor_bindings"].as_str().unwrap_or("[]").into(),
            config: item["config"].as_str().unwrap_or("{}").into(),
        })
        .collect();
    let back = mimic::parse(&stored, &connections);
    let mut expected = h.present().clone();
    for widget in &mut expected.widgets {
        widget.name = saved_name(widget, &|id| names.get(&id).cloned());
    }
    assert_eq!(back, expected, "the rows load back into the same screen");
}

#[test]
fn bound_widgets_draw_their_live_values() {
    let (h, _) = edited();
    let mut cache = Cache::default();
    let tank = widget(&h, "e016");
    let level = |v: f64| move |r: &str| (r == "wtp/raw_tank_level").then_some(v);
    let drawn = bound(tank, &level(42.0), &mut cache).expect("bound");
    assert_eq!(drawn.text("label"), Some("T-101 42.0 %"));
    let drawn = bound(tank, &level(91.0), &mut cache).expect("bound");
    assert_eq!(drawn.text("label"), Some("HIGH"));
    let unbound = bound(tank, &|_| None, &mut cache).expect("bound");
    // As in the browser: the comparison has no value (so is false) and
    // text() of no value is empty.
    assert_eq!(unbound.text("label"), Some("T-101  %"));
    let only_tag = Binding::Expr {
        expr: "{wtp/raw_tank_level} * 1".into(),
    };
    let fixed = Widget {
        config: tank.config.clone(),
        ..tank.clone()
    };
    let rebound = Edit::Config {
        id: fixed.id.clone(),
        key: "bindings".into(),
        value: bindings::with_binding(&fixed.config, "label", Some(&only_tag)),
    };
    let mut layout = h.present().clone();
    assert!(rebound.apply(&mut layout));
    let drawn = bound(layout.widget("e016").unwrap(), &|_| None, &mut cache).unwrap();
    assert_eq!(
        drawn.text("label"),
        Some("T-101 Raw"),
        "no value: the fixed label"
    );
    let pump = widget(&h, "e013");
    assert!(!visible(&bound(pump, &|_| Some(0.0), &mut cache).unwrap()));
    assert!(visible(&bound(pump, &|_| Some(1.0), &mut cache).unwrap()));
    assert!(bound(widget(&h, "e012"), &|_| None, &mut cache).is_none());
    assert!(!locked(pump));
}
