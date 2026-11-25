//! The Designer's rules: the binding tests of the web Designer
//! (`client/tests/designerBindings.test.ts`) run against the Rust port, the
//! editor model's undo and redo, and the save compared with what the web
//! Designer's own code saves after the same edits
//! (`tests/fixtures/designer/web-save.json`, written by `web-save.mts`).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value as Json, json};

use super::arrange::{Align, Axis};
use super::bindings::{
    Binding, MapCase, PropType, apply_bindings, binding_errors, binding_refs, evaluate_binding,
    ref_index, tag_ref, with_binding,
};
use super::expr::{Cache, ExprError, Value, evaluate, parse, refs, to_fixed};
use super::*;
use crate::mimic::edit::{History, Order, Rect};
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
                out: Some(json!("#f04646")),
            },
            MapCase {
                op: ">=".into(),
                v: 85.0,
                out: Some(json!("#f5b21b")),
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
    let ids = |list: &[&str]| list.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
    // 9. Group the two pumps.
    let grouped = arrange::group(h.present(), &ids(&["e012", "e013"]), "g_pumps").unwrap();
    apply(&mut h, grouped, None);
    steps += 1;
    // 10. Align the valve and the pumps left.
    let aligned =
        arrange::align(h.present(), &ids(&["e011", "e012", "e013"]), Align::Left).unwrap();
    apply(&mut h, aligned, None);
    steps += 1;
    // 11. Distribute them vertically.
    let spaced =
        arrange::distribute(h.present(), &ids(&["e011", "e012", "e013"]), Axis::Vertical).unwrap();
    apply(&mut h, spaced, None);
    steps += 1;
    // 12. Lock the tank; a left align with it moves only the KPI.
    let locked = arrange::lock(h.present(), &ids(&["e016"]), true).unwrap();
    apply(&mut h, locked, None);
    let aligned = arrange::align(h.present(), &ids(&["e016", "e003"]), Align::Left).unwrap();
    apply(&mut h, aligned, None);
    steps += 2;
    // 13. The trend's opacity follows an expression.
    let faded = Binding::Expr {
        expr: "{wtp/raw_turbidity} > 30 ? 1 : 0.5".into(),
    };
    let bound = set_binding(widget(&h, "e033"), "opacity", Some(&faded));
    apply(&mut h, bound, None);
    steps += 1;
    // 14. Delete P-101B: its group of one dissolves.
    let gone = Edit::Remove {
        widgets: ids(&["e013"]),
        connections: Vec::new(),
    };
    apply(&mut h, gone, None);
    assert_eq!(arrange::group_of(widget(&h, "e012")), None);
    steps += 1;
    // 15. Group the valve with P-101A, then bring the group forward one step.
    let grouped = arrange::group(h.present(), &ids(&["e011", "e012"]), "g_intake").unwrap();
    apply(&mut h, grouped, None);
    let forward = Edit::Reorder {
        ids: ids(&["e011"]),
        to: Order::Forward,
    };
    apply(&mut h, forward, None);
    steps += 2;
    // 16. Centre the label over the trend's width, then undo it: not saved.
    let centred = arrange::align(h.present(), &ids(&["e001", "e033"]), Align::Center).unwrap();
    apply(&mut h, centred, None);
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
    // P-101B is deleted in step 14: back to before it.
    let mut h = h;
    while h.present().widget("e013").is_none() {
        assert!(h.undo());
    }
    let pump = widget(&h, "e013");
    assert!(!visible(&bound(pump, &|_| Some(0.0), &mut cache).unwrap()));
    assert!(visible(&bound(pump, &|_| Some(1.0), &mut cache).unwrap()));
    assert!(bound(widget(&h, "e012"), &|_| None, &mut cache).is_none());
    assert!(!locked(pump));
}

// ── The review fixes of the web Designer (#33) ─────────────────────────────

#[test]
fn oversized_or_too_deep_expressions_fail_to_parse() {
    let long = format!("\"{}\"", "a".repeat(expr::MAX_SOURCE));
    assert_eq!(
        parse(&long).unwrap_err().message,
        "expression is longer than 2048 characters"
    );
    let nested = format!("{}1{}", "(".repeat(150), ")".repeat(150));
    assert_eq!(
        parse(&nested).unwrap_err().message,
        "expression is nested deeper than 100 levels"
    );
    // A long left-associative chain is deep too.
    let chain = vec!["1"; 150].join(" + ");
    assert_eq!(
        parse(&chain).unwrap_err().message,
        "expression is nested deeper than 100 levels"
    );
    let negations = format!("{}1", "-".repeat(150));
    assert!(parse(&negations).is_err());
    let wide = format!(
        "max({})",
        vec!["min(1, 2, 3, 4, 5, 6, 7, 8)"; 120].join(", ")
    );
    assert!(
        parse(&wide).is_err(),
        "more than 1000 parts or 16 arguments"
    );
    // A balanced tree of 1023 parts in 2045 characters.
    fn tree(levels: u32) -> String {
        if levels == 0 {
            "1".into()
        } else {
            format!("({}+{})", tree(levels - 1), tree(levels - 1))
        }
    }
    let many = tree(9);
    assert!(many.len() < expr::MAX_SOURCE);
    assert_eq!(
        parse(&many).unwrap_err().message,
        "expression has more than 1000 parts"
    );
    assert!(parse(&tree(8)).is_ok());
    assert!(parse(&vec!["1"; 90].join(" + ")).is_ok());
}

#[test]
fn tag_refs_escape_braces_and_backslashes() {
    assert_eq!(expr::escape_ref("dev/a}b"), "{dev/a\\}b}");
    assert_eq!(expr::escape_ref("dev/a\\b"), "{dev/a\\\\b}");
    let node = parse("{dev/a\\}b} + {dev/a\\\\b}").unwrap();
    assert_eq!(
        refs(&node).into_iter().collect::<Vec<_>>(),
        ["dev/a\\b", "dev/a}b"]
    );
    assert_eq!(
        parse("{dev/a\\}").unwrap_err().message,
        "missing } after tag reference"
    );
    assert_eq!(
        super::bindings::describe(&Binding::Tag {
            tag: "dev/a}b".into()
        }),
        "{dev/a\\}b}"
    );
}

#[test]
fn bit_text_and_round_clamp_their_arguments() {
    assert_eq!(ev("bit(5, 0)"), Value::Bool(true));
    assert_eq!(ev("bit(5, 1)"), Value::Bool(false));
    assert_eq!(ev("bit(-1, 52)"), Value::Bool(true), "two's complement");
    assert_eq!(ev("bit(5, 53)"), Value::Null);
    assert_eq!(ev("bit(5, 1.5)"), Value::Null);
    assert_eq!(ev("text(1.25, 99)"), text("1.25000000000000000000"));
    assert_eq!(ev("text(1.5, -3)"), text("2"));
    assert_eq!(ev("text(1.5, null)"), text("2"));
    assert_eq!(ev("round(1.234567, 50)"), num(1.234_567));
    assert_eq!(ev("round(2.5, -1)"), num(3.0));
}

#[test]
fn a_binding_that_gives_no_number_keeps_the_fixed_value() {
    let mut cache = Cache::default();
    let read = |_: &str| Some(1.0);
    let not_a_number = Binding::Expr {
        expr: "\"abc\" * 2".into(),
    };
    assert_eq!(
        evaluate_binding(&not_a_number, &read, Some(PropType::Number), &mut cache),
        None
    );
    let empty = Binding::Expr {
        expr: "\"\" + \"\"".into(),
    };
    assert_eq!(
        evaluate_binding(&empty, &read, Some(PropType::Number), &mut cache),
        None,
        "empty text is no number"
    );
    // A case without a number never matches; one without a result keeps
    // the fixed value rather than falling to "otherwise".
    let map: Binding = serde_json::from_value(json!({
        "kind": "map", "expr": "1",
        "cases": [{ "op": "==", "v": null, "out": "x" }, { "op": "==", "v": 1 }],
        "else": "else"
    }))
    .unwrap();
    assert_eq!(
        evaluate_binding(&map, &read, Some(PropType::String), &mut cache),
        None
    );
    assert_eq!(binding_errors(&map, &|_| true), ["Case 1 needs a number"]);
    let saved = serde_json::to_value(&map).unwrap();
    assert_eq!(
        saved["cases"][1],
        json!({ "op": "==", "v": 1.0 }),
        "no out written"
    );
}

#[test]
fn tag_pickers_are_never_bound_and_visibility_follows_its_binding() {
    let mut cache = Cache::default();
    let mut config = serde_json::Map::new();
    config.insert(
        "bindings".into(),
        json!({
            "runSensorId": { "kind": "tag", "tag": "wtp/level" },
            "visible": { "kind": "expr", "expr": "{wtp/level} > 50" },
        }),
    );
    let types = |prop: &str| (prop == "runSensorId").then_some(PropType::Tag);
    let read = |_: &str| Some(80.0);
    let out = apply_bindings(&config, &read, &types, &mut cache).unwrap();
    assert!(!out.contains_key("runSensorId"));
    assert_eq!(out["visible"], json!(true));
    assert!(super::bindings::shown_at_runtime(
        &config, &read, &mut cache
    ));
    assert!(!super::bindings::shown_at_runtime(
        &config,
        &|_| Some(10.0),
        &mut cache
    ));
    config.insert("visible".into(), json!(false));
    assert!(
        !super::bindings::shown_at_runtime(&config, &|_| None, &mut cache),
        "no value: the fixed one"
    );
}

#[test]
fn the_parse_cache_drops_the_least_recently_used_text() {
    let mut cache = Cache::default();
    for i in 0..Cache::LIMIT {
        assert!(cache.compile(&format!("{i}")).is_ok());
    }
    // Text 0 is used again, so text 1 is the oldest.
    assert!(cache.compile("0").is_ok());
    assert!(cache.compile("new").is_err());
    assert_eq!(cache.len(), Cache::LIMIT);
    let long = "1".repeat(expr::MAX_SOURCE + 1);
    assert!(cache.compile(&long).is_err());
    assert_eq!(cache.len(), Cache::LIMIT, "oversized text is not kept");
}

#[test]
fn duplicate_refs_are_numbered_in_id_order() {
    let tags = [
        (11, "wtp", "level"),
        (4, "wtp", "level"),
        (9, "", "calc"),
        (20, "wtp", "level"),
    ];
    let refs = super::bindings::refs_by_id(tags);
    assert_eq!(refs[&4], "wtp/level");
    assert_eq!(refs[&11], "wtp/level#2");
    assert_eq!(refs[&20], "wtp/level#3");
    assert_eq!(refs[&9], "/calc");
    let index = ref_index(tags);
    assert_eq!(index["wtp/level#2"], 11);
}

// ── Groups, locks, align and distribute (`model.ts`) ──────────────────────

fn boxed(id: &str, x: f64, y: f64, w: f64, h: f64) -> Widget {
    Widget {
        id: id.into(),
        kind: crate::mimic::WidgetKind::parse("label"),
        name: String::new(),
        tags: Vec::new(),
        signals: Vec::new(),
        x,
        y,
        width: w,
        height: h,
        config: serde_json::Map::new(),
    }
}

fn layout_of(widgets: Vec<Widget>) -> Layout {
    Layout {
        widgets,
        connections: Vec::new(),
    }
}

fn s(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn applied(layout: &Layout, edit: Option<Edit>) -> Layout {
    let mut next = layout.clone();
    assert!(edit.expect("an edit").apply(&mut next));
    next
}

#[test]
fn groups_select_move_and_delete_together() {
    let base = layout_of(vec![
        boxed("a", 0.0, 0.0, 10.0, 10.0),
        boxed("b", 50.0, 0.0, 10.0, 10.0),
        boxed("c", 100.0, 0.0, 10.0, 10.0),
    ]);
    assert_eq!(
        arrange::group(&base, &s(&["a"]), "g"),
        None,
        "one is no group"
    );
    let grouped = applied(&base, arrange::group(&base, &s(&["a", "c"]), "g"));
    assert_eq!(arrange::group_of(grouped.widget("a").unwrap()), Some("g"));
    // Picking one member picks the group; ⇧ toggles the whole group.
    assert_eq!(arrange::expand_to_groups(&grouped, &s(&["c"])), ["a", "c"]);
    assert_eq!(
        arrange::press_selection(&grouped, &[], "a", false),
        ["a", "c"]
    );
    assert_eq!(
        arrange::press_selection(&grouped, &s(&["b"]), "c", true),
        ["b", "a", "c"]
    );
    assert_eq!(
        arrange::press_selection(&grouped, &s(&["a", "b", "c"]), "c", true),
        ["b"]
    );
    // The marquee touching one member takes the group.
    let band = Rect {
        x: 95.0,
        y: -5.0,
        width: 10.0,
        height: 10.0,
    };
    assert_eq!(arrange::marquee(&grouped, band), ["a", "c"]);
    // Regrouping moves members of another group into the new one.
    let regrouped = applied(&grouped, arrange::group(&grouped, &s(&["c", "b"]), "h"));
    assert_eq!(arrange::group_of(regrouped.widget("c").unwrap()), Some("h"));
    // Deleting leaves "a" alone in "g": no group any more.
    let mut deleted = regrouped.clone();
    assert!(
        Edit::Remove {
            widgets: s(&["c"]),
            connections: Vec::new()
        }
        .apply(&mut deleted)
    );
    assert_eq!(arrange::group_of(deleted.widget("a").unwrap()), None);
    assert_eq!(arrange::group_of(deleted.widget("b").unwrap()), None);
    // Ungroup takes the key away.
    let ungrouped = applied(&grouped, arrange::ungroup(&grouped, &s(&["a", "c"])));
    assert!(!ungrouped.widget("a").unwrap().config.contains_key("group"));
    assert_eq!(arrange::ungroup(&ungrouped, &s(&["a"])), None);
}

#[test]
fn locked_widgets_stay_put_and_are_not_band_selected_or_deleted() {
    let base = layout_of(vec![
        boxed("a", 0.0, 0.0, 10.0, 10.0),
        boxed("b", 50.0, 20.0, 10.0, 10.0),
    ]);
    let locked = applied(&base, arrange::lock(&base, &s(&["a"]), true));
    assert_eq!(locked.widget("a").unwrap().config["locked"], json!(true));
    assert_eq!(arrange::lock(&locked, &s(&["a"]), true), None);
    let everything = Rect {
        x: -10.0,
        y: -10.0,
        width: 500.0,
        height: 500.0,
    };
    assert_eq!(arrange::marquee(&locked, everything), ["b"]);
    assert_eq!(arrange::select_all(&locked), ["b"]);
    assert_eq!(arrange::deletable(&locked, &s(&["a", "b"])), ["b"]);
    // Arrows move only the unlocked ones.
    let moved = applied(&locked, arrange::nudge(&locked, &s(&["a", "b"]), 5.0, 0.0));
    assert_eq!(moved.widget("a").unwrap().x, 0.0);
    assert_eq!(moved.widget("b").unwrap().x, 55.0);
    assert_eq!(arrange::nudge(&locked, &s(&["a"]), 5.0, 0.0), None);
    // Align keeps it where it is, but aligns to it.
    let aligned = applied(
        &locked,
        arrange::align(&locked, &s(&["a", "b"]), Align::Top),
    );
    assert_eq!(aligned.widget("b").unwrap().y, 0.0);
    // ⌘L unlocks when everything picked is locked, else locks.
    let both = applied(&locked, arrange::toggle_lock(&locked, &s(&["a", "b"])));
    assert!(arrange::is_locked(both.widget("b").unwrap()));
    let none = applied(&both, arrange::toggle_lock(&both, &s(&["a", "b"])));
    assert!(!none.widget("a").unwrap().config.contains_key("locked"));
    // A pasted copy is unlocked, unnamed, in a group of its own.
    let mut named = applied(&both, arrange::group(&both, &s(&["a", "b"]), "g"));
    named.widgets[0].name = "Pump".into();
    let clip = edit::copy(&named, &s(&["a", "b"]));
    let mut n = 0;
    let Some(Edit::Add { widgets, .. }) = edit::paste(&named, &clip, 20.0, |prefix| {
        n += 1;
        format!("{prefix}{n}")
    }) else {
        panic!("an add");
    };
    assert!(
        widgets
            .iter()
            .all(|w| !arrange::is_locked(w) && w.name.is_empty())
    );
    assert_eq!(arrange::group_of(&widgets[0]), Some("g2"));
    assert_eq!(arrange::group_of(&widgets[1]), Some("g2"));
}

#[test]
fn align_puts_every_edge_on_the_selection_box() {
    let base = layout_of(vec![
        boxed("a", 10.0, 10.0, 20.0, 20.0),
        boxed("b", 100.0, 50.0, 41.0, 10.0),
    ]);
    let ab = s(&["a", "b"]);
    let at = |mode: Align| {
        let next = applied(&base, arrange::align(&base, &ab, mode));
        let a = next.widget("a").unwrap();
        let b = next.widget("b").unwrap();
        ((a.x, a.y), (b.x, b.y))
    };
    assert_eq!(at(Align::Left), ((10.0, 10.0), (10.0, 50.0)));
    assert_eq!(at(Align::Right), ((121.0, 10.0), (100.0, 50.0)));
    // Centres round as JavaScript's Math.round: 10 + (131 - 41) / 2 = 55.
    assert_eq!(at(Align::Center), ((66.0, 10.0), (55.0, 50.0)));
    assert_eq!(at(Align::Top), ((10.0, 10.0), (100.0, 10.0)));
    assert_eq!(at(Align::Bottom), ((10.0, 40.0), (100.0, 50.0)));
    assert_eq!(at(Align::Middle), ((10.0, 25.0), (100.0, 30.0)));
    assert_eq!(arrange::align(&base, &s(&["a"]), Align::Left), None);
    // Already aligned: nothing to do, no undo step.
    let done = applied(&base, arrange::align(&base, &ab, Align::Left));
    assert_eq!(arrange::align(&done, &ab, Align::Left), None);
    // One step: one undo puts both back.
    let mut history = History::new(base.clone());
    assert!(history.apply(&arrange::align(&base, &ab, Align::Top).unwrap(), None));
    assert!(history.undo());
    assert_eq!(history.present(), &base);
}

#[test]
fn distribute_spaces_the_middle_ones_evenly() {
    let base = layout_of(vec![
        boxed("a", 0.0, 0.0, 10.0, 10.0),
        boxed("c", 100.0, 0.0, 10.0, 10.0),
        boxed("b", 20.0, 90.0, 20.0, 10.0),
        boxed("d", 60.0, 0.0, 10.0, 10.0),
    ]);
    let all = s(&["a", "b", "c", "d"]);
    // Span 110, sizes 50: gaps of 20 between neighbours by x.
    let spaced = applied(&base, arrange::distribute(&base, &all, Axis::Horizontal));
    let xs: Vec<f64> = ["a", "b", "d", "c"]
        .iter()
        .map(|id| spaced.widget(id).unwrap().x)
        .collect();
    assert_eq!(xs, [0.0, 30.0, 70.0, 100.0]);
    assert_eq!(
        arrange::distribute(&base, &s(&["a", "c"]), Axis::Horizontal),
        None
    );
    let vertical = applied(
        &base,
        arrange::distribute(&base, &s(&["a", "b", "d"]), Axis::Vertical),
    );
    // By y: a 0, d 0, b 90: d moves between them.
    assert_eq!(vertical.widget("d").unwrap().y, 45.0);
    // A locked middle widget stays; the others still space around it.
    let locked = applied(&base, arrange::lock(&base, &s(&["b"]), true));
    let spaced = applied(
        &locked,
        arrange::distribute(&locked, &all, Axis::Horizontal),
    );
    assert_eq!(spaced.widget("b").unwrap().x, 20.0);
    assert_eq!(spaced.widget("d").unwrap().x, 70.0);
}

#[test]
fn restacking_keeps_a_group_together() {
    let mut grouped = layout_of(vec![
        boxed("a", 0.0, 0.0, 10.0, 10.0),
        boxed("g1", 0.0, 0.0, 10.0, 10.0),
        boxed("b", 0.0, 0.0, 10.0, 10.0),
        boxed("g2", 0.0, 0.0, 10.0, 10.0),
        boxed("c", 0.0, 0.0, 10.0, 10.0),
    ]);
    grouped = applied(&grouped, arrange::group(&grouped, &s(&["g1", "g2"]), "g"));
    let order = |layout: &Layout| {
        layout
            .widgets
            .iter()
            .map(|w| w.id.clone())
            .collect::<Vec<_>>()
    };
    let to = |layout: &Layout, ids: &[&str], to: Order| {
        let mut next = layout.clone();
        Edit::Reorder { ids: s(ids), to }.apply(&mut next);
        next
    };
    // The group is one unit at its back-most member.
    assert_eq!(
        order(&to(&grouped, &["g2"], Order::Front)),
        ["a", "b", "c", "g1", "g2"]
    );
    assert_eq!(
        order(&to(&grouped, &["g1"], Order::Back)),
        ["g1", "g2", "a", "b", "c"]
    );
    assert_eq!(
        order(&to(&grouped, &["g1"], Order::Forward)),
        ["a", "b", "g1", "g2", "c"]
    );
    assert_eq!(
        order(&to(&grouped, &["c"], Order::Backward)),
        ["a", "g1", "g2", "c", "b"]
    );
    assert_eq!(
        order(&to(&grouped, &["b"], Order::Backward)),
        ["a", "b", "g1", "g2", "c"]
    );
    // At the bottom already: only the group is gathered (as the web's).
    assert_eq!(
        order(&to(&grouped, &["a"], Order::Backward)),
        ["a", "g1", "g2", "b", "c"]
    );
    let gathered = to(&grouped, &["a"], Order::Backward);
    let mut same = gathered.clone();
    assert!(
        !Edit::Reorder {
            ids: s(&["a"]),
            to: Order::Backward
        }
        .apply(&mut same)
    );
    assert!(
        !Edit::Reorder {
            ids: s(&["a"]),
            to: Order::Backward
        }
        .apply(&mut same)
    );
}

// ── A tag dropped on the empty stage (`dropChoices`, `pickDrop`) ──────────

#[test]
fn a_dropped_tag_offers_what_suits_it() {
    let shape = |key: &'static str, unit: &'static str, min: f64, max: f64| arrange::TagShape {
        key,
        name: "",
        unit,
        min,
        max,
    };
    let labels = |tag: Option<&arrange::TagShape<'_>>| {
        arrange::drop_choices(tag)
            .iter()
            .map(|c| c.label)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(Some(&shape("p101a_run", "", 0.0, 1.0))),
        [
            "Pump (ISA-101)",
            "Valve (ISA-101)",
            "Motor (ISA-101)",
            "Status light",
            "On/off indicator",
            "Toggle (writes)",
            "Number"
        ]
    );
    assert_eq!(
        labels(Some(&shape("intake_flow", "m³/h", 0.0, 500.0))),
        [
            "Number",
            "Value indicator",
            "Tank (ISA-101)",
            "Control valve (ISA-101)",
            "Bar",
            "Gauge",
            "Trend",
            "Slider (writes)"
        ]
    );
    // A level tag offers its tank first.
    assert_eq!(
        labels(Some(&shape("raw_tank_level", "m", 0.0, 5.0)))[0],
        "Tank (ISA-101)"
    );
    assert_eq!(
        labels(Some(&shape("fill", "%", 0.0, 100.0)))[0],
        "Tank (ISA-101)"
    );
    assert_eq!(labels(None)[0], "Number");
    // Every choice is something the console draws.
    for choice in arrange::drop_choices(None)
        .into_iter()
        .chain(arrange::drop_choices(Some(&shape("x", "", 0.0, 1.0))))
    {
        match choice.placing {
            edit::Placing::Symbol(key) => assert!(crate::mimic::symbol_def(key).is_some(), "{key}"),
            edit::Placing::Widget(kind) => assert!(
                edit::manifest(&crate::mimic::WidgetKind::parse(kind)).is_some(),
                "{kind}"
            ),
        }
    }
}

#[test]
fn a_picked_choice_is_placed_centred_and_bound() {
    let pump = arrange::drop_choices(Some(&arrange::TagShape {
        key: "run",
        name: "Pump run",
        unit: "",
        min: 0.0,
        max: 1.0,
    }))[0];
    let widget = arrange::place_tag(pump, 7, Some("Pump run"), (403.0, 207.0), "w1".into());
    // 90 × 104 centred on (403, 207), on the 10-pixel grid.
    assert_eq!((widget.x, widget.y), (360.0, 160.0));
    assert_eq!(widget.tags, [7]);
    assert_eq!(widget.text("label"), Some("Pump run"));
    assert_eq!(widget.flag("showStatus"), Some(true));
    let number = arrange::drop_choices(None)[0];
    let widget = arrange::place_tag(number, 7, Some("Flow"), (5.0, 5.0), "w2".into());
    assert_eq!((widget.x, widget.y), (0.0, 0.0), "kept on the stage");
    assert_eq!(
        widget.text("label"),
        None,
        "widgets show the tag's own name"
    );
}

#[test]
fn designing_fades_hidden_widgets_and_keeps_some_opacity() {
    let mut w = boxed("a", 0.0, 0.0, 10.0, 10.0);
    assert_eq!(opacity(&w, true), None);
    w.config.insert("opacity".into(), json!(0.05));
    assert_eq!(opacity(&w, true), Some(0.15));
    assert_eq!(opacity(&w, false), Some(0.05));
    w.config.insert("opacity".into(), json!(3));
    assert_eq!(opacity(&w, false), Some(1.0));
    w.config.insert("visible".into(), json!(false));
    assert_eq!(opacity(&w, true), Some(0.3));
    // Every property but a tag picker binds.
    let valve = boxed("v", 0.0, 0.0, 10.0, 10.0);
    let props = properties(&valve);
    assert_eq!(props.first().map(|p| p.key), Some("label"));
    assert!(props.iter().any(|p| p.key == "visible" && p.bindable()));
    assert!(
        props
            .iter()
            .any(|p| p.key == "opacity" && p.kind == PropType::Number)
    );
}

// ── The generated corpus (`tests/fixtures/designer/expr-corpus.json`) ──────
//
// Written by `expr-corpus.mts` from the web Designer's own `expr.ts` and
// `bindings.ts`; CI writes it again and fails when it changed, so these
// tests follow the web code.

#[derive(Deserialize)]
struct Corpus {
    tags: HashMap<String, f64>,
    exprs: Vec<CorpusExpr>,
    bindings: Vec<CorpusBinding>,
    numbers: Vec<(String, f64, String, String)>,
}

#[derive(Deserialize)]
struct CorpusExpr {
    src: String,
    r: String,
}

#[derive(Deserialize)]
struct CorpusBinding {
    b: Json,
    #[serde(rename = "type")]
    kind: Option<String>,
    r: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/designer/expr-corpus.json"
    ))
    .expect("expr-corpus.json")
}

/// A number as the corpus writes it (`String(x)`, `-0`), bit for bit.
fn corpus_number(text: &str) -> f64 {
    if text == "-0" {
        return -0.0;
    }
    let x = expr::js_string_to_number(text);
    assert!(!x.is_nan(), "{text} is a number");
    x
}

fn encode(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Num(x) if *x == 0.0 && x.is_sign_negative() => "number:-0".into(),
        Value::Num(x) => format!("number:{}", expr::js_number(*x)),
        Value::Str(s) => format!("string:{s}"),
        Value::Bool(b) => format!("boolean:{b}"),
    }
}

fn encode_json(value: Option<&Json>) -> String {
    match value {
        None => "undefined".into(),
        Some(Json::Null) => "null".into(),
        Some(Json::Number(n)) => encode(&Value::Num(n.as_f64().unwrap())),
        Some(Json::String(s)) => format!("string:{s}"),
        Some(Json::Bool(b)) => format!("boolean:{b}"),
        Some(other) => format!("other:{other}"),
    }
}

#[test]
fn expressions_match_the_web_corpus() {
    let corpus = corpus();
    let read = |reference: &str| corpus.tags.get(reference).copied();
    let mut failed = Vec::new();
    for case in &corpus.exprs {
        let got = match parse(&case.src) {
            Err(error) => format!("error:{}@{}", error.message, error.pos),
            Ok(node) => encode(&expr::safe_evaluate(&node, &read)),
        };
        if got != case.r {
            failed.push(format!("{:?}: web {:?}, native {got:?}", case.src, case.r));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn bindings_match_the_web_corpus() {
    let corpus = corpus();
    let read = |reference: &str| corpus.tags.get(reference).copied();
    let mut cache = Cache::default();
    let mut failed = Vec::new();
    for case in &corpus.bindings {
        let kind = case.kind.as_deref().map(|kind| match kind {
            "string" => PropType::String,
            "number" => PropType::Number,
            "boolean" => PropType::Boolean,
            other => panic!("type {other}"),
        });
        let binding: Binding = serde_json::from_value(case.b.clone()).expect("a binding");
        let got = evaluate_binding(&binding, &read, kind, &mut cache);
        let got = encode_json(got.as_ref());
        // JSON has no infinite number: an untyped property keeps it as
        // JavaScript writes it, which reads back as the same number.
        let infinite = kind.is_none()
            && matches!(case.r.as_str(), "number:Infinity" | "number:-Infinity")
            && got == case.r.replace("number:", "string:");
        if got != case.r && !infinite {
            failed.push(format!(
                "{} as {:?}: web {:?}, native {got:?}",
                case.b, case.kind, case.r
            ));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn to_fixed_round_and_number_text_match_the_web_corpus() {
    let corpus = corpus();
    assert!(corpus.numbers.len() > 1000);
    let mut failed = Vec::new();
    for (text, digits, fixed, rounded) in &corpus.numbers {
        let x = corpus_number(text);
        if expr::js_number(x) != *text && text != "-0" {
            failed.push(format!("String({text}): native {}", expr::js_number(x)));
        }
        let got = to_fixed(x, *digits);
        if got != *fixed {
            failed.push(format!(
                "({text}).toFixed({digits}): web {fixed}, native {got}"
            ));
        }
        let got = encode(&Value::Num(expr::js_round(x)));
        if got != *rounded {
            failed.push(format!("Math.round({text}): web {rounded}, native {got}"));
        }
    }
    assert!(
        failed.is_empty(),
        "{}",
        failed[..failed.len().min(40)].join("\n")
    );
}

#[test]
fn text_rounds_the_exact_value_and_drops_the_sign_of_zero() {
    assert_eq!(ev("text(1.45, 1)"), text("1.4"), "1.45 is 1.4499…");
    assert_eq!(
        ev("text(2940.055, 2)"),
        text("2940.05"),
        "2940.055 is …054…"
    );
    assert_eq!(
        ev("text(0.125, 2)"),
        text("0.13"),
        "an exact tie: away from zero"
    );
    assert_eq!(ev("text(-0.125, 2)"), text("-0.13"));
    assert_eq!(ev("text(9.995, 2)"), text("9.99"));
    assert_eq!(ev("text(99.5, 0)"), text("100"));
    assert_eq!(ev("text(-{wtp/fault}, 2)"), text("0.00"), "-0");
    assert_eq!(ev("text(-0.0001, 2)"), text("-0.00"), "as the web");
}

#[test]
fn infinite_values_keep_their_meaning_in_every_property_type() {
    let mut cache = Cache::default();
    let read = |_: &str| Some(1e300);
    let big = Binding::Expr {
        expr: "{x} * 1e10".into(),
    };
    let as_kind = |kind, cache: &mut Cache| evaluate_binding(&big, &read, kind, cache);
    assert_eq!(
        as_kind(Some(PropType::String), &mut cache),
        Some(json!("Infinity"))
    );
    assert_eq!(
        as_kind(Some(PropType::Boolean), &mut cache),
        Some(json!(true))
    );
    assert_eq!(as_kind(Some(PropType::Number), &mut cache), None);
    assert_eq!(as_kind(None, &mut cache), Some(json!("Infinity")));
}

#[test]
fn malformed_map_bindings_fall_to_otherwise_as_on_the_web() {
    let mut cache = Cache::default();
    let read = |_: &str| None;
    let otherwise = |raw: Json| {
        let binding: Binding = serde_json::from_value(raw).expect("read leniently");
        evaluate_binding(
            &binding,
            &read,
            Some(PropType::String),
            &mut Cache::default(),
        )
    };
    assert_eq!(
        otherwise(json!({ "kind": "map", "expr": "1", "cases": "nope", "else": "E" })),
        Some(json!("E"))
    );
    assert_eq!(
        otherwise(
            json!({ "kind": "map", "expr": "1", "cases": [null, { "v": 1, "out": "A" }], "else": "E" })
        ),
        Some(json!("E")),
        "no op: compared with <"
    );
    let mut out = |out: Json, kind| {
        let binding: Binding = serde_json::from_value(
            json!({ "kind": "map", "expr": "1", "cases": [{ "op": "==", "v": 1, "out": out }] }),
        )
        .unwrap();
        evaluate_binding(&binding, &read, kind, &mut cache)
    };
    assert_eq!(
        out(json!([1, 2]), Some(PropType::String)),
        Some(json!("1,2"))
    );
    assert_eq!(
        out(json!({ "a": 1 }), Some(PropType::String)),
        Some(json!("[object Object]"))
    );
    assert_eq!(out(json!([5]), Some(PropType::Number)), Some(json!(5.0)));
    assert_eq!(out(json!([]), Some(PropType::Boolean)), Some(json!(true)));
}

#[test]
fn number_text_follows_javascript() {
    let n = |s: &str| Value::Str(s.into()).num();
    assert_eq!(n("0x10"), 16.0);
    assert_eq!(n("0b11"), 3.0);
    assert_eq!(n("0o17"), 15.0);
    assert!(n("inf").is_nan());
    assert!(n("infinity").is_nan());
    assert!(n("nan").is_nan());
    assert!(n("-0x10").is_nan());
    assert_eq!(n("-Infinity"), f64::NEG_INFINITY);
    assert_eq!(n("\u{feff}12"), 12.0, "U+FEFF is white space to JavaScript");
    assert!(n("\u{85}12").is_nan(), "U+0085 is not");
    assert_eq!(n(" \t"), 0.0);
    assert_eq!(n("5."), 5.0);
    assert!(n("1_0").is_nan());
}

#[test]
fn error_positions_count_utf16_units() {
    let error = parse("\"😀\" 1").unwrap_err();
    assert_eq!(
        (error.message.as_str(), error.pos),
        ("unexpected \"1\" at 6", 5)
    );
    assert_eq!(parse("\u{feff}1"), parse("1"), "U+FEFF is white space");
    assert_eq!(parse("\u{85} 1").unwrap_err().pos, 0, "U+0085 is not");
}

#[test]
fn the_parse_cache_fits_a_large_screen() {
    let mut cache = Cache::default();
    cache.fit(2000);
    assert_eq!(cache.limit(), 2000);
    for i in 0..2000 {
        assert!(cache.compile(&format!("{i} + 1")).is_ok());
    }
    assert_eq!(cache.len(), 2000);
    cache.fit(10);
    assert_eq!(cache.limit(), Cache::LIMIT, "never fewer than the default");
    assert_eq!(cache.len(), Cache::LIMIT, "the oldest went");
    assert!(cache.compile("1999 + 1").is_ok());
}

#[test]
fn bound_widgets_are_evaluated_again_only_when_a_value_or_the_config_changes() {
    use std::cell::Cell;

    let input = input();
    let mut layout = mimic::parse(&input.equipment, &input.connections);
    let id = layout.widgets[0].id.clone();
    let widget = &mut layout.widgets[0];
    widget.config.insert(
        "bindings".into(),
        json!({ "label": { "kind": "expr", "expr": "\"L \" + text({wtp/level}, 1)" } }),
    );
    let widget = widget.clone();
    let reads = Cell::new(0);
    let level = Cell::new(1.25);
    let read = |reference: &str| {
        reads.set(reads.get() + 1);
        (reference == "wtp/level").then(|| level.get())
    };
    let mut cache = BoundCache::default();
    cache.frame();
    let label = |cache: &mut BoundCache, widget: &Widget| {
        cache
            .bound(widget, &read)
            .and_then(|w| w.text("label").map(str::to_owned))
    };
    assert_eq!(label(&mut cache, &widget).as_deref(), Some("L 1.3"));
    let first = cache.widgets[&id].out.clone();
    // Same value: the drawn widget is the one made before.
    cache.frame();
    assert_eq!(label(&mut cache, &widget).as_deref(), Some("L 1.3"));
    assert_eq!(cache.widgets[&id].out, first);
    level.set(2.0);
    assert_eq!(label(&mut cache, &widget).as_deref(), Some("L 2.0"));
    // A move keeps the parse, and draws at the new place.
    let mut moved = widget.clone();
    moved.x += 10.0;
    assert_eq!(cache.bound(&moved, &read).map(|w| w.x), Some(moved.x));
    // A new expression is parsed again.
    let mut edited = moved.clone();
    edited.config.insert(
        "bindings".into(),
        json!({ "label": { "kind": "expr", "expr": "\"M \" + {wtp/level}" } }),
    );
    assert_eq!(label(&mut cache, &edited).as_deref(), Some("M 2"));
    // Unbound: nothing kept.
    let mut plain = edited.clone();
    plain.config.remove("bindings");
    assert!(cache.bound(&plain, &read).is_none());
    assert!(cache.is_empty());
    assert!(reads.get() > 0);
    // What is no longer drawn goes after a while.
    assert!(cache.bound(&edited, &read).is_some());
    for _ in 0..=BoundCache::KEEP_FRAMES + 64 {
        cache.frame();
    }
    assert!(cache.is_empty());
}

// ── Loading and saving against a scripted server ───────────────────────────

/// A server that answers each connection with the next of `answers`
/// (status, body) and reports every request's line and body.
fn scripted(
    answers: Vec<(u16, &'static str)>,
) -> (
    crate::api::VoltrusApi,
    std::sync::mpsc::Receiver<(String, String)>,
) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
    let url = format!("http://{}", listener.local_addr().expect("address"));
    let (report, requests) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in answers {
            let Ok((mut connection, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if connection.read(&mut byte).unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let mut request_body = vec![0_u8; length];
            let _ = connection.read_exact(&mut request_body);
            let line = head.lines().next().unwrap_or_default().to_owned();
            let _ = report.send((line, String::from_utf8_lossy(&request_body).into_owned()));
            let _ = write!(
                connection,
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let mut api = crate::api::VoltrusApi::new(&url);
    api.token = Some("t".into());
    (api, requests)
}

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn lines(requests: &std::sync::mpsc::Receiver<(String, String)>) -> Vec<(String, String)> {
    requests.try_iter().collect()
}

const EMPTY_ITEMS: &str = r#"{"items":[]}"#;

#[test]
fn a_failed_layout_read_is_an_error_not_an_older_server() {
    for (status, body) in [
        (500, r#"{"error":"database is locked"}"#),
        (401, r#"{"error":"signed out"}"#),
        (403, r#"{"error":"not allowed"}"#),
        (200, r#"{"equipment":[],"connections":[]}"#),
        (200, r#"{"equipment":[],"connections":[],"revision":""}"#),
        (200, r#"{"equipment":"x","connections":[],"revision":"r1"}"#),
    ] {
        let (api, requests) =
            scripted(vec![(status, body), (200, EMPTY_ITEMS), (200, EMPTY_ITEMS)]);
        assert!(block(load(&api, "main")).is_err(), "{status} {body}");
        let seen = lines(&requests);
        assert_eq!(seen.len(), 1, "{status} {body}: no fallback, {seen:?}");
    }
}

#[test]
fn only_a_missing_endpoint_or_a_web_page_falls_back_to_the_two_lists() {
    for (status, body) in [
        (404, r#"{"error":"not found"}"#),
        (405, ""),
        (200, "<!doctype html><html></html>"),
    ] {
        let (api, requests) =
            scripted(vec![(status, body), (200, EMPTY_ITEMS), (200, EMPTY_ITEMS)]);
        let (layout, revision) = block(load(&api, "main")).expect("the two lists");
        assert!(layout.widgets.is_empty());
        assert_eq!(revision, None);
        let seen = lines(&requests);
        assert_eq!(seen.len(), 3, "{status}: {seen:?}");
        assert!(seen[1].0.contains("/scada-equipment "), "{seen:?}");
        assert!(seen[2].0.contains("/scada-connections "), "{seen:?}");
    }
}

#[test]
fn a_known_revision_goes_with_the_older_endpoints_too() {
    let (api, requests) = scripted(vec![
        (404, r#"{"error":"not found"}"#),
        (200, r#"{"ok":true,"revision":"r2"}"#),
        (200, r#"{"ok":true,"revision":"r3"}"#),
    ]);
    let saved = block(save(
        &api,
        "main",
        Vec::new(),
        Vec::new(),
        Some("r1".into()),
    ))
    .unwrap();
    assert_eq!(
        saved,
        Saved::Done {
            revision: Some("r3".into())
        }
    );
    let seen = lines(&requests);
    assert!(seen[0].0.starts_with("PUT ") && seen[0].0.contains("/scada-layout "));
    let body = |i: usize| serde_json::from_str::<Json>(&seen[i].1).unwrap();
    assert_eq!(body(1)["base_revision"], "r1", "{seen:?}");
    assert_eq!(
        body(2)["base_revision"],
        "r2",
        "based on what the first made"
    );

    // A conflict on the first of them is a conflict.
    let (api, _) = scripted(vec![
        (405, ""),
        (
            409,
            r#"{"error":"changed","code":"conflict","revision":"r9"}"#,
        ),
    ]);
    let saved = block(save(
        &api,
        "main",
        Vec::new(),
        Vec::new(),
        Some("r1".into()),
    ))
    .unwrap();
    assert!(matches!(saved, Saved::Conflict { revision, .. } if revision == "r9"));
}

#[test]
fn a_save_answer_without_a_revision_asks_for_it_and_errors_stay_errors() {
    let (api, requests) = scripted(vec![
        (200, r#"{"ok":true}"#),
        (200, r#"{"equipment":[],"connections":[],"revision":"r4"}"#),
    ]);
    let saved = block(save(
        &api,
        "main",
        Vec::new(),
        Vec::new(),
        Some("r1".into()),
    ))
    .unwrap();
    assert_eq!(
        saved,
        Saved::Done {
            revision: Some("r4".into())
        }
    );
    assert_eq!(lines(&requests).len(), 2);

    let (api, requests) = scripted(vec![
        (500, r#"{"error":"disk full"}"#),
        (200, "{}"),
        (200, "{}"),
    ]);
    assert!(
        block(save(
            &api,
            "main",
            Vec::new(),
            Vec::new(),
            Some("r1".into())
        ))
        .is_err()
    );
    assert_eq!(lines(&requests).len(), 1, "no fallback to the two PUTs");

    // No revision known (an older server): the two PUTs, no check.
    let (api, requests) = scripted(vec![(200, "{}"), (200, "{}")]);
    let saved = block(save(&api, "main", Vec::new(), Vec::new(), None)).unwrap();
    assert_eq!(saved, Saved::Done { revision: None });
    let seen = lines(&requests);
    assert!(!seen[0].1.contains("base_revision"), "{seen:?}");
}
