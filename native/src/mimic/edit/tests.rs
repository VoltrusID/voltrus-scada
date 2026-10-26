use serde_json::json;

use super::*;

fn boxed(id: &str, kind: &str, (x, y, width, height): (f64, f64, f64, f64)) -> Widget {
    Widget {
        id: id.to_owned(),
        kind: WidgetKind::parse(kind),
        name: String::new(),
        tags: Vec::new(),
        x,
        y,
        width,
        height,
        config: Map::new(),
    }
}

fn symbol(id: &str, key: &str, config: Value) -> Widget {
    let mut widget = Placing::Symbol(symbol_def(key).expect("a library symbol").key).widget(
        id.to_owned(),
        100.0,
        100.0,
    );
    if let Value::Object(extra) = config {
        widget.config.extend(extra);
    }
    widget
}

fn with_config(mut widget: Widget, config: Value) -> Widget {
    if let Value::Object(extra) = config {
        widget.config.extend(extra);
    }
    widget
}

fn keys(fields: &[Field]) -> Vec<&'static str> {
    fields.iter().map(|field| field.key).collect()
}

fn ids(layout: &Layout) -> Vec<&str> {
    layout.widgets.iter().map(|w| w.id.as_str()).collect()
}

fn line(id: &str, from: &str, to: &str) -> Connection {
    connection(id.to_owned(), from, to, "flow")
}

fn two_and_a_line() -> Layout {
    Layout {
        widgets: vec![
            boxed("a", "gauge", (100.0, 100.0, 100.0, 50.0)),
            boxed("b", "gauge", (400.0, 100.0, 100.0, 50.0)),
        ],
        connections: vec![line("c", "a", "b")],
    }
}

/// Deterministic ids for tests: `w1`, `c2`, …
fn counter() -> impl FnMut(&str) -> String {
    let mut next = 0;
    move |prefix: &str| {
        next += 1;
        format!("{prefix}{next}")
    }
}

#[test]
fn manifests_follow_the_web_registry() {
    assert_eq!(MANIFESTS.len(), WidgetKind::ALL.len());
    for (listed, (kind, wire)) in MANIFESTS.iter().zip(WidgetKind::ALL.iter()) {
        assert_eq!(listed.kind, *wire, "in WidgetKind::ALL's order");
        assert_eq!(manifest(kind), Some(listed));
    }
    let gauge = manifest(&WidgetKind::Gauge).unwrap();
    assert_eq!((gauge.name, gauge.size), ("Gauge", (180.0, 180.0)));
    assert_eq!(keys(gauge.fields), ["showLabels", "arcColor"]);
    assert_eq!(
        manifest(&WidgetKind::SemicircleGauge).unwrap().name,
        "Half Gauge"
    );
    assert_eq!(manifest(&WidgetKind::DataTable).unwrap().max_tags, 99);
    assert_eq!(manifest(&WidgetKind::Area).unwrap().max_tags, 0);
    assert_eq!(manifest(&WidgetKind::Other("x".into())), None);
    // The equipment types are offered as library symbols instead.
    let offered: Vec<&str> = palette_widgets().map(|m| m.kind).collect();
    assert_eq!(offered.len(), 24);
    for equipment in [
        "symbol",
        "pump",
        "motor",
        "valve",
        "tank_level",
        "horizontal_tank",
        "pipe",
    ] {
        assert!(!offered.contains(&equipment), "{equipment}");
    }
    assert!(offered.contains(&"area") && offered.contains(&"progress_bar"));
}

#[test]
fn the_palette_searches_names_and_categories() {
    let valves: Vec<&str> = palette_symbols("VALV").map(|def| def.key).collect();
    assert_eq!(valves, ["valve", "control_valve", "check_valve"]);
    let piping: Vec<&str> = palette_symbols("piping").map(|def| def.key).collect();
    assert_eq!(piping, ["pipe", "pipe_vertical", "pipe_elbow", "pipe_tee"]);
    assert_eq!(palette_symbols("  ").count(), SYMBOLS.len());
    assert!(matches("", &["anything"]));
    assert!(matches("gau", &["Half Gauge"]));
    assert!(!matches("pump", &["Gauge"]));
    // Every symbol sits in one of the palette's categories.
    for def in SYMBOLS {
        assert!(CATEGORIES.contains(&def.category), "{}", def.key);
    }
}

#[test]
fn symbol_schemas_follow_symbol_schema() {
    let pump = symbol("p", "pump", json!({}));
    assert_eq!(
        keys(&schema(&pump)),
        [
            "label",
            "showValue",
            "showStatus",
            "onThreshold",
            "faultSensorId",
            "writable",
            "rotation",
            "flipX"
        ]
    );
    let fields = schema(&pump);
    let writable = fields.iter().find(|f| f.key == "writable").unwrap();
    assert_eq!(writable.init, Init::Flag(false), "new symbols opt in");
    let show_value = fields.iter().find(|f| f.key == "showValue").unwrap();
    assert_eq!(show_value.init, Init::Flag(false));

    // A legacy pump: no value badge, commands on unless turned off.
    let legacy = boxed("lp", "pump", (0.0, 0.0, 100.0, 100.0));
    let fields = schema(&legacy);
    assert!(!keys(&fields).contains(&"showValue"));
    let writable = fields.iter().find(|f| f.key == "writable").unwrap();
    assert_eq!(writable.init, Init::Flag(true));

    let tank = symbol("t", "tank", json!({}));
    assert_eq!(
        keys(&schema(&tank)),
        ["label", "showValue", "showPercent", "rotation", "flipX"]
    );
    // A busbar stretches: no rotation.
    let busbar = symbol("b", "busbar", json!({}));
    assert!(!keys(&schema(&busbar)).contains(&"rotation"));
    let instrument = symbol("i", "instrument", json!({}));
    let fields = schema(&instrument);
    assert!(keys(&fields).contains(&"tagFunction"));
    let mounting = fields.iter().find(|f| f.key == "mounting").unwrap();
    assert!(matches!(mounting.kind, FieldKind::Select(options) if options.len() == 3));
    // A legacy pipe: the flow tag and its own direction setting.
    let pipe = boxed("pp", "pipe", (0.0, 0.0, 200.0, 30.0));
    assert_eq!(
        keys(&schema(&pipe)),
        ["label", "showValue", "runSensorId", "direction"]
    );
    // Other widgets: their manifest's fields.
    let label = boxed("l", "label", (0.0, 0.0, 160.0, 40.0));
    assert_eq!(
        keys(&schema(&label)),
        ["text", "fontSize", "align", "bold", "muted"]
    );
    assert!(schema(&boxed("x", "mystery", (0.0, 0.0, 10.0, 10.0))).is_empty());
}

#[test]
fn fields_show_what_is_drawn() {
    let field = |widget: &Widget, key: &str| {
        let field = schema(widget)
            .into_iter()
            .find(|field| field.key == key)
            .unwrap_or_else(|| panic!("{key}"));
        field_value(widget, &field)
    };
    // Legacy drives: showValue meant the state text.
    let pump = with_config(
        boxed("p", "pump", (0.0, 0.0, 90.0, 90.0)),
        json!({"showValue": false}),
    );
    assert_eq!(field(&pump, "showStatus"), json!(false));
    let pump = boxed("p", "pump", (0.0, 0.0, 90.0, 90.0));
    assert_eq!(field(&pump, "showStatus"), json!(true));
    // Legacy vessels: showPercentage.
    let tank = with_config(
        boxed("t", "tank_level", (0.0, 0.0, 100.0, 160.0)),
        json!({"showPercentage": false}),
    );
    assert_eq!(field(&tank, "showPercent"), json!(false));
    // Unset: the default.
    let gauge = boxed("g", "gauge", (0.0, 0.0, 180.0, 180.0));
    assert_eq!(field(&gauge, "showLabels"), json!(true));
    assert_eq!(field(&gauge, "arcColor"), json!("var(--ok)"));
    let turned = symbol("s", "pump", json!({"rotation": 90}));
    assert_eq!(value_text(&field(&turned, "rotation")), "90");

    assert_eq!(value_text(&json!(90.0)), "90");
    assert_eq!(value_text(&json!("90")), "90");
    assert_eq!(value_text(&json!(0.5)), "0.5");
    assert_eq!(value_text(&Value::Null), "");
    assert!(value_flag(&json!("x")) && !value_flag(&json!(0)) && !value_flag(&Value::Null));
    assert_eq!(value_tag(&json!(12)), Some(12));
    assert_eq!(value_tag(&json!("12")), Some(12));
    assert_eq!(value_tag(&json!(null)), None);
    assert_eq!(number_value(90.0), json!(90));
    assert_eq!(number_value(0.25), json!(0.25));

    let font = FieldKind::Number {
        min: Some(12.0),
        max: Some(72.0),
    };
    assert_eq!(typed_number("100", font), Some(72.0));
    assert_eq!(typed_number(" 20 ", font), Some(20.0));
    assert_eq!(typed_number("", font), None);
    assert_eq!(typed_number("abc", font), None);
    assert_eq!(typed_number("-3", FieldKind::Text), Some(-3.0));
}

#[test]
fn new_widgets_land_on_the_grid_and_the_stage() {
    assert_eq!(snap(14.9), 10.0);
    assert_eq!(snap(15.0), 20.0);
    assert_eq!(clamp_pos(1900.0, 1070.0, 90.0, 90.0), (1830.0, 990.0));
    assert_eq!(clamp_pos(-5.0, -5.0, 10.0, 10.0), (0.0, 0.0));
    // Click-to-add: the centre, cascading 20 pixels, eight steps round.
    assert_eq!(place_at(0, 90.0, 90.0), (920.0, 500.0));
    assert_eq!(place_at(1, 90.0, 90.0), (940.0, 520.0));
    assert_eq!(place_at(8, 90.0, 90.0), place_at(0, 90.0, 90.0));
    assert_eq!(place_at(0, 2000.0, 100.0).0, 0.0, "wider than the stage");
    assert_eq!(drop_at(123.0, 457.0, 90.0, 90.0), (120.0, 460.0));
    assert_eq!(drop_at(1915.0, 5.0, 90.0, 90.0), (1830.0, 10.0));

    let pump = Placing::Symbol("pump").widget("w_1".into(), 10.0, 10.0);
    assert_eq!(pump.kind, WidgetKind::Symbol);
    assert_eq!(pump.text("symbol"), Some("pump"));
    assert_eq!((pump.width, pump.height), (90.0, 90.0));
    assert_eq!(Placing::Symbol("pump").name(), "Centrifugal Pump");
    let gauge = Placing::Widget("gauge").widget("w_2".into(), 1900.0, 0.0);
    assert_eq!((gauge.x, gauge.width), (1740.0, 180.0));
    assert!(gauge.config.is_empty() && gauge.tags.is_empty());
    assert_eq!(Placing::Widget("progress_bar").name(), "Progress Bar");
}

#[test]
fn ids_are_prefix_time_and_five_base36_characters() {
    assert_eq!(id_from("w", 1_700_000_000_000, 0), "w_1700000000000_00000");
    assert_eq!(id_from("c", 1, 35), "c_1_z0000");
    assert_eq!(id_from("w", 1, 36), "w_1_01000");
    let a = fresh_id("w");
    let b = fresh_id("w");
    assert_ne!(a, b);
    let parts: Vec<&str> = a.split('_').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "w");
    assert!(parts[1].parse::<u128>().is_ok());
    assert_eq!(parts[2].len(), 5);
}

#[test]
fn edits_change_the_layout_or_report_nothing_changed() {
    let mut layout = two_and_a_line();
    let moved = Edit::Move(vec![("a".into(), 110.0, 100.0)]);
    assert!(moved.apply(&mut layout));
    assert!(!moved.apply(&mut layout), "already there");
    assert!(
        Edit::Resize {
            id: "b".into(),
            x: 400.0,
            y: 100.0,
            width: 120.0,
            height: 60.0
        }
        .apply(&mut layout)
    );
    assert_eq!(layout.widgets[1].width, 120.0);

    let label = |value: Value| Edit::Config {
        id: "a".into(),
        key: "label".into(),
        value,
    };
    assert!(label(json!("P-1")).apply(&mut layout));
    assert!(!label(json!("P-1")).apply(&mut layout));
    assert!(label(Value::Null).apply(&mut layout), "null clears");
    assert!(!layout.widgets[0].config.contains_key("label"));
    assert!(!label(Value::Null).apply(&mut layout));

    let bind = Edit::Bind {
        id: "a".into(),
        tags: vec![7, 9],
    };
    assert!(bind.apply(&mut layout));
    assert_eq!(layout.widgets[0].tag(), Some(7));
    assert!(!bind.apply(&mut layout));

    let power = Edit::Line {
        id: "c".into(),
        kind: "power".into(),
    };
    assert!(power.apply(&mut layout));
    assert_eq!(layout.connections[0].kind, "power");
    assert!(!power.apply(&mut layout));

    // Removing a widget removes its connections.
    let mut removed = layout.clone();
    assert!(
        Edit::Remove {
            widgets: vec!["b".into()],
            connections: Vec::new()
        }
        .apply(&mut removed)
    );
    assert_eq!(ids(&removed), ["a"]);
    assert!(removed.connections.is_empty());
    // A connection alone.
    let mut cut = layout.clone();
    assert!(
        Edit::Remove {
            widgets: Vec::new(),
            connections: vec!["c".into()]
        }
        .apply(&mut cut)
    );
    assert_eq!(cut.widgets.len(), 2);
    assert!(cut.connections.is_empty());
    assert!(
        !Edit::Remove {
            widgets: vec!["nope".into()],
            connections: Vec::new()
        }
        .apply(&mut cut)
    );
}

#[test]
fn z_order_keeps_the_picked_widgets_order() {
    let mut layout = Layout {
        widgets: vec![
            boxed("a", "label", (0.0, 0.0, 10.0, 10.0)),
            boxed("b", "label", (0.0, 0.0, 10.0, 10.0)),
            boxed("c", "label", (0.0, 0.0, 10.0, 10.0)),
        ],
        connections: Vec::new(),
    };
    let front = |ids: &[&str]| Edit::Reorder {
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        front: true,
    };
    assert!(front(&["a"]).apply(&mut layout));
    assert_eq!(ids(&layout), ["b", "c", "a"]);
    assert!(!front(&["a"]).apply(&mut layout), "already in front");
    assert!(
        Edit::Reorder {
            ids: vec!["a".into(), "c".into()],
            front: false
        }
        .apply(&mut layout)
    );
    assert_eq!(ids(&layout), ["c", "a", "b"]);
    assert!(
        Edit::Many(vec![front(&["c"]), front(&["nope"])]).apply(&mut layout),
        "one of many changed"
    );
    assert_eq!(ids(&layout), ["a", "b", "c"]);
}

#[test]
fn history_undoes_redoes_coalesces_and_knows_what_is_saved() {
    let mut history = History::new(two_and_a_line());
    assert!(!history.dirty() && !history.can_undo() && !history.can_redo());

    let move_a = |x: f64| Edit::Move(vec![("a".into(), x, 100.0)]);
    assert!(history.apply(&move_a(150.0), None));
    assert!(history.dirty() && history.can_undo());
    assert!(!history.apply(&move_a(150.0), None), "no change, no step");
    assert!(history.undo());
    assert!(!history.dirty(), "back to the saved layout");
    assert!(history.can_redo());
    assert!(history.redo());
    assert_eq!(history.present().widgets[0].x, 150.0);
    assert!(!history.redo());

    // Typing in one field is one step.
    for text in ["P", "P-", "P-1"] {
        history.apply(
            &Edit::Config {
                id: "a".into(),
                key: "label".into(),
                value: json!(text),
            },
            Some("a:label"),
        );
    }
    assert_eq!(history.present().widgets[0].text("label"), Some("P-1"));
    assert!(history.undo());
    assert_eq!(history.present().widgets[0].text("label"), None);
    assert_eq!(history.present().widgets[0].x, 150.0);
    // A new edit drops the redo steps.
    assert!(history.apply(&move_a(170.0), Some("drag:1")));
    assert!(!history.can_redo());
    assert!(history.apply(&move_a(180.0), Some("drag:1")));
    assert!(history.apply(&move_a(190.0), Some("drag:2")), "a new drag");
    assert!(history.undo());
    assert_eq!(history.present().widgets[0].x, 180.0);
    assert!(history.undo());
    assert_eq!(history.present().widgets[0].x, 150.0);

    // Saved: clean until the next change.
    let snapshot = history.present().clone();
    history.mark_saved(snapshot);
    assert!(!history.dirty());
    assert!(history.apply(&move_a(10.0), None));
    assert!(history.dirty());
    assert_eq!(history.saved().widgets[0].x, 150.0);

    // At most HISTORY_LIMIT steps.
    let mut long = History::new(two_and_a_line());
    for step in 0..150 {
        assert!(long.apply(&move_a(f64::from(step)), None));
    }
    let mut undone = 0;
    while long.undo() {
        undone += 1;
    }
    assert_eq!(undone, HISTORY_LIMIT);
    // `seal` ends a coalescing run.
    let mut sealed = History::new(two_and_a_line());
    sealed.apply(&move_a(1.0), Some("k"));
    sealed.seal();
    sealed.apply(&move_a(2.0), Some("k"));
    assert!(sealed.undo() && sealed.undo());
    assert!(!sealed.dirty());
}

#[test]
fn moves_snap_to_the_grid_line_up_with_others_and_stay_on_the_stage() {
    let layout = Layout {
        widgets: vec![
            boxed("a", "label", (0.0, 0.0, 100.0, 50.0)),
            boxed("b", "label", (300.0, 200.0, 100.0, 50.0)),
        ],
        connections: Vec::new(),
    };
    let moving = ["a".to_owned()];
    let start = Rect::of(&layout.widgets[0]);
    let both = Snapping {
        grid: true,
        threshold: 6.0,
    };
    // Within 6 pixels of b's left edge (and of the stage's top).
    let snapped = snap_move(&layout, &moving, start, 297.0, 3.0, both);
    assert_eq!((snapped.dx, snapped.dy), (300.0, 0.0));
    let vertical: Vec<f64> = snapped
        .guides
        .iter()
        .filter(|g| g.vertical)
        .map(|g| g.at)
        .collect();
    assert_eq!(
        vertical,
        [300.0, 350.0, 400.0],
        "left, centre, right line up"
    );
    let guide = snapped.guides.iter().find(|g| g.vertical).unwrap();
    assert_eq!(
        (guide.from, guide.to),
        (0.0, 250.0),
        "from a's top to b's bottom"
    );

    // Grid only.
    let grid = Snapping {
        grid: true,
        threshold: 0.0,
    };
    let snapped = snap_move(&layout, &moving, start, 13.0, 27.0, grid);
    assert_eq!((snapped.dx, snapped.dy), (10.0, 30.0));
    assert!(snapped.guides.is_empty());
    // Free (⌘ held).
    let free = Snapping {
        grid: false,
        threshold: 0.0,
    };
    let snapped = snap_move(&layout, &moving, start, 13.0, 27.0, free);
    assert_eq!((snapped.dx, snapped.dy), (13.0, 27.0));
    // Kept on the stage.
    let snapped = snap_move(&layout, &moving, start, 5000.0, -50.0, grid);
    assert_eq!((snapped.dx, snapped.dy), (STAGE_W - 100.0, 0.0));
    // The stage's centre line.
    let centred = snap_move(&layout, &moving, start, 908.0, 600.0, both);
    assert_eq!(centred.dx, 910.0, "the box's centre on x = 960");
}

#[test]
fn resizes_move_the_handles_edges_with_a_least_size() {
    let layout = Layout {
        widgets: vec![
            boxed("a", "label", (100.0, 100.0, 100.0, 100.0)),
            boxed("b", "label", (600.0, 600.0, 100.0, 100.0)),
        ],
        connections: Vec::new(),
    };
    let start = Rect::of(&layout.widgets[0]);
    let grid = Snapping {
        grid: true,
        threshold: 0.0,
    };
    let (rect, guides) = snap_resize(&layout, "a", start, Handle::SouthEast, 23.0, 47.0, grid);
    assert_eq!(
        rect,
        Rect {
            x: 100.0,
            y: 100.0,
            width: 120.0,
            height: 150.0
        }
    );
    assert!(guides.is_empty());
    let (rect, _) = snap_resize(&layout, "a", start, Handle::NorthWest, 200.0, 0.0, grid);
    assert_eq!((rect.x, rect.width), (184.0, MIN_WIDTH));
    let (rect, _) = snap_resize(&layout, "a", start, Handle::West, -500.0, 0.0, grid);
    assert_eq!((rect.x, rect.right()), (0.0, 200.0));
    let (rect, _) = snap_resize(&layout, "a", start, Handle::North, 0.0, 30.0, grid);
    assert_eq!(
        (rect.y, rect.height, rect.x, rect.width),
        (130.0, 70.0, 100.0, 100.0)
    );
    // Lining the right edge up with b's left.
    let both = Snapping {
        grid: true,
        threshold: 6.0,
    };
    let (rect, guides) = snap_resize(&layout, "a", start, Handle::East, 397.0, 0.0, both);
    assert_eq!(rect.right(), 600.0);
    assert!(guides.iter().any(|g| g.vertical && g.at == 600.0));

    assert_eq!(
        hit_handle(start, 200.0, 200.0, 5.0),
        Some(Handle::SouthEast)
    );
    assert_eq!(hit_handle(start, 151.0, 98.0, 5.0), Some(Handle::North));
    assert_eq!(hit_handle(start, 150.0, 150.0, 5.0), None);
    assert_eq!(Handle::West.point(start), (100.0, 150.0));
}

#[test]
fn hits_bands_and_reading_order() {
    let mut layout = two_and_a_line();
    layout
        .widgets
        .push(boxed("top", "label", (150.0, 110.0, 20.0, 20.0)));
    assert_eq!(hit_widget(&layout, 160.0, 120.0), Some(2), "the topmost");
    assert_eq!(hit_widget(&layout, 110.0, 110.0), Some(0));
    assert_eq!(hit_widget(&layout, 5.0, 5.0), None);
    // The connection runs from a's right to b's left at y = 125.
    assert_eq!(hit_connection(&layout, 300.0, 127.0, 4.0), Some(0));
    assert_eq!(hit_connection(&layout, 300.0, 160.0, 4.0), None);
    let pending = pending_route(&layout.widgets[0], 300.0, 400.0);
    assert_eq!(pending.last(), Some(&(300.0, 400.0)));

    let band = Rect::between((520.0, 90.0), (90.0, 160.0));
    assert_eq!(band.x, 90.0);
    assert_eq!(enclosed(&layout, band), ["a", "b", "top"]);
    let band = Rect::between((90.0, 90.0), (210.0, 160.0));
    assert_eq!(enclosed(&layout, band), ["a", "top"], "only wholly inside");

    assert_eq!(step(&layout, None, true).as_deref(), Some("a"));
    assert_eq!(step(&layout, Some("a"), true).as_deref(), Some("top"));
    assert_eq!(
        step(&layout, Some("b"), true).as_deref(),
        Some("a"),
        "round"
    );
    assert_eq!(step(&layout, Some("a"), false).as_deref(), Some("b"));
    assert_eq!(step(&Layout::default(), None, true), None);

    // Nudging keeps the group on the stage.
    let ids = ["a".to_owned(), "b".to_owned()];
    let Some(Edit::Move(moves)) = nudge(&layout, &ids, -500.0, 10.0) else {
        panic!("a move");
    };
    assert_eq!(
        moves,
        [("a".to_owned(), 0.0, 110.0), ("b".to_owned(), 300.0, 110.0)]
    );
    assert_eq!(nudge(&layout, &[], 1.0, 1.0), None);
    assert_eq!(
        bounds(picked(&layout, &ids)),
        Some(Rect {
            x: 100.0,
            y: 100.0,
            width: 400.0,
            height: 50.0
        })
    );
}

#[test]
fn copies_paste_with_fresh_ids_and_their_connections() {
    let layout = two_and_a_line();
    let ids = ["a".to_owned(), "b".to_owned()];
    let mut clip = copy(&layout, &ids);
    assert_eq!(clip.widgets.len(), 2);
    assert_eq!(clip.connections.len(), 1);
    assert!(copy(&layout, &["a".to_owned()]).connections.is_empty());

    let Some(Edit::Add {
        widgets,
        connections,
    }) = paste(&layout, &clip, PASTE_OFFSET, counter())
    else {
        panic!("an add");
    };
    assert_eq!(
        widgets.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["w1", "w2"]
    );
    assert_eq!((widgets[0].x, widgets[0].y), (120.0, 120.0));
    assert_eq!(widgets[0].kind, WidgetKind::Gauge);
    assert_eq!(connections.len(), 1);
    assert_eq!(
        (
            connections[0].id.as_str(),
            connections[0].from.as_str(),
            connections[0].to.as_str()
        ),
        ("c3", "w1", "w2")
    );
    // Taken ids are skipped: the first one offered is taken.
    let mut tries = 0;
    let fresh = |prefix: &str| {
        tries += 1;
        if tries == 1 {
            "a".to_owned()
        } else {
            format!("{prefix}_{tries}")
        }
    };
    let single = copy(&layout, &["a".to_owned()]);
    let Some(Edit::Add { widgets, .. }) = paste(&layout, &single, 20.0, fresh) else {
        panic!("an add");
    };
    assert_eq!(widgets[0].id, "w_2");
    // The next paste lands further on; at the edge it stays on the stage.
    clip.shift(PASTE_OFFSET);
    assert_eq!(clip.widgets[0].x, 120.0);
    let edge = Layout {
        widgets: vec![boxed("e", "label", (1880.0, 1060.0, 40.0, 20.0))],
        connections: Vec::new(),
    };
    let Some(Edit::Add { widgets, .. }) =
        paste(&edge, &copy(&edge, &["e".to_owned()]), 20.0, counter())
    else {
        panic!("an add");
    };
    assert_eq!((widgets[0].x, widgets[0].y), (1880.0, 1060.0));
    assert_eq!(paste(&layout, &Clip::default(), 20.0, counter()), None);
}

#[test]
fn symbols_turn_in_quarters_and_mirror() {
    let layout = Layout {
        widgets: vec![
            symbol("p", "pump", json!({})),
            symbol("bus", "busbar", json!({})),
            boxed("g", "gauge", (0.0, 0.0, 10.0, 10.0)),
        ],
        connections: Vec::new(),
    };
    let all: Vec<String> = ["p", "bus", "g"]
        .iter()
        .map(|id| (*id).to_owned())
        .collect();
    let mut turned = layout.clone();
    assert!(rotate(&layout, &all, 1).unwrap().apply(&mut turned));
    assert_eq!(turned.widgets[0].number("rotation"), Some(90.0));
    assert!(
        !turned.widgets[1].config.contains_key("rotation"),
        "busbars stretch"
    );
    let mut back = layout.clone();
    rotate(&layout, &all, -1).unwrap().apply(&mut back);
    assert_eq!(back.widgets[0].number("rotation"), Some(270.0));
    assert_eq!(rotate(&layout, &["g".to_owned()], 1), None);

    let mut mirrored = layout.clone();
    mirror(&layout, &all).unwrap().apply(&mut mirrored);
    assert_eq!(mirrored.widgets[0].flag("flipX"), Some(true));
    let again = mirror(&mirrored, &all).unwrap();
    again.apply(&mut mirrored);
    assert_eq!(mirrored.widgets[0].flag("flipX"), Some(false));
    assert!(turns(&layout.widgets[0]));
    assert!(!turns(&layout.widgets[1]) && !turns(&layout.widgets[2]));
}

#[test]
fn saving_names_each_widget_like_to_api() {
    let mut layout = two_and_a_line();
    layout.widgets[0]
        .config
        .insert("label".into(), json!("Feed pump"));
    layout.widgets[1].tags = vec![7, 8];
    layout.widgets.push(symbol("s", "valve", json!({})));
    layout
        .widgets
        .push(boxed("o", "mystery", (0.0, 0.0, 10.0, 10.0)));
    let named = named(&layout, |tag| (tag == 7).then(|| "Tank level".to_owned()));
    let names: Vec<&str> = named.widgets.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Feed pump", "Tank level", "Gate Valve", "mystery"]);
    assert_eq!(title(&layout.widgets[1]), "Gauge");
    assert_eq!(max_tags(&layout.widgets[1]), 1);
    assert_eq!(summary(&layout), (4, 1, 2));
    // What the inspector says of a binding.
    assert!(binding_hint(Binding::Level).contains("Fill level"));
}

#[test]
fn an_edited_layout_saves_and_reads_back() {
    // Add, move, connect, undo, redo: the rows read back the same.
    let mut history = History::new(Layout::default());
    let pump = Placing::Symbol("pump").widget("w_1".into(), 100.0, 100.0);
    let tank = Placing::Symbol("tank").widget("w_2".into(), 400.0, 100.0);
    history.apply(
        &Edit::Add {
            widgets: vec![pump, tank],
            connections: Vec::new(),
        },
        None,
    );
    history.apply(&Edit::Move(vec![("w_2".into(), 420.0, 80.0)]), None);
    history.apply(
        &Edit::Add {
            widgets: Vec::new(),
            connections: vec![connection("c_1".into(), "w_1", "w_2", "flow")],
        },
        None,
    );
    history.undo();
    history.redo();
    let layout = history.present().clone();
    let (equipment, connections) = super::super::to_rows(&layout);
    assert_eq!(super::super::parse(&equipment, &connections), layout);
    assert_eq!(LINE_TYPES[0], ("flow", "Process pipe"));
}
