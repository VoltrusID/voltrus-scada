//! Data Manager › Flows against the live plant: every route the section
//! calls. A flow is created (stopped, so it never fires), read back as
//! stored, edited the way the canvas editor edits it (a node added and
//! wired from the trigger), run and stopped, tested, its runs listed, and
//! deleted.
//!
//! The plant may run either server: one with the graph engine (the
//! `/node-types` catalogue, v2 graphs, dry-run tests) or an older one
//! (straight-line arrays, the stored-flow test). The round trip asks the
//! catalogue first and checks what that server promises; the graph-only
//! routes are skipped against an older one.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_flows::{
    Catalogue, End, FieldKind, Flow, FlowGraph, NodeType, TEST_INPUT, TEST_PAYLOAD, config_id,
    parse_input, refusal,
};

/// A value a required field without a default accepts.
fn placeholder(kind: FieldKind) -> Value {
    match kind {
        FieldKind::Number | FieldKind::DurationMs => json!(1000),
        FieldKind::Bool => json!(false),
        FieldKind::Json | FieldKind::Other => json!({}),
        FieldKind::Expression => json!("metrics.temp > 0"),
        FieldKind::Cron => json!("0 * * * * *"),
        _ => json!("e2e"),
    }
}

/// Fill what `end` lacks: its required fields, with choices first.
fn complete(graph: &mut FlowGraph, end: End, catalogue: &Catalogue) {
    let (config, fields) = match end {
        End::Trigger => {
            let kind = graph.trigger_type().to_owned();
            (
                &mut graph.trigger,
                catalogue.trigger(&kind).map(|t| t.fields.clone()),
            )
        }
        End::Node(index) => {
            let kind = graph.nodes[index]["type"].as_str().unwrap_or("").to_owned();
            (
                &mut graph.nodes[index],
                catalogue.node(&kind).map(|n| n.fields.clone()),
            )
        }
    };
    for field in fields.unwrap_or_default() {
        if field.is_missing(config.get(&field.key)) {
            let value = field
                .choices()
                .first()
                .map_or_else(|| placeholder(field.kind), |choice| choice.value.clone());
            config.insert(field.key.clone(), value);
        }
    }
}

/// A node type with an output and no side effect: the filter if there is
/// one.
fn passing_kind(catalogue: &Catalogue) -> &NodeType {
    catalogue
        .node("filter")
        .filter(|kind| !kind.outputs.is_empty())
        .or_else(|| {
            catalogue
                .nodes
                .iter()
                .find(|kind| !kind.side_effect && !kind.outputs.is_empty() && kind.kind != "switch")
        })
        .expect("a node type that passes messages on")
}

pub(crate) async fn round_trip(api: &VoltrusApi) {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970");
    let now = i64::try_from(elapsed.as_secs()).expect("seconds fit");
    let id = format!("e2e_native_flow_{}", elapsed.as_millis());
    let before = api.flows().await.expect("GET /api/v1/flows");
    assert!(
        !before.iter().any(|flow| flow.id == id),
        "the test flow is new"
    );

    let served = api
        .flow_node_types()
        .await
        .expect("GET /api/v1/flows/node-types (or a 404 from an older server)");
    let graph_server = served.is_some();
    let catalogue = served.unwrap_or_else(Catalogue::builtin);
    println!(
        "flows: {} ({} triggers, {} node types)",
        if graph_server {
            "graph server"
        } else {
            "legacy server: graph-only checks skipped"
        },
        catalogue.triggers.len(),
        catalogue.nodes.len()
    );
    assert!(!catalogue.triggers.is_empty(), "a trigger type");
    assert!(!catalogue.nodes.is_empty(), "node types");
    let texts = |graph: &FlowGraph| {
        if graph_server {
            graph.nodes_json()
        } else {
            graph
                .legacy_json(&catalogue)
                .expect("a straight line the legacy engine runs")
        }
    };

    // Create: a trigger that does not fire while the flow is started for
    // a moment (a Modbus device the plant does not have, or an hourly
    // timer), with one node.
    let trigger = catalogue
        .trigger("modbus")
        .or_else(|| catalogue.trigger("timer"))
        .unwrap_or(&catalogue.triggers[0]);
    let mut graph = FlowGraph::default();
    graph.set_trigger_kind(trigger);
    if graph.trigger_type() == "modbus" {
        graph
            .trigger
            .insert("device_id".into(), json!("e2e-native-no-such-device"));
    }
    if graph.trigger.contains_key("interval_ms") {
        graph.trigger.insert("interval_ms".into(), json!(3_600_000));
    }
    complete(&mut graph, End::Trigger, &catalogue);
    let kind = passing_kind(&catalogue);
    let first_id = graph.fresh_id(1);
    let first = graph.add(kind, first_id.clone(), 260.0, 0.0);
    complete(&mut graph, End::Node(first), &catalogue);
    graph
        .connect(End::Trigger, 0, first, &catalogue)
        .expect("trigger → node");
    graph.validate(&catalogue).expect("a flow the server reads");
    let flow = Flow::blank(&id, "Native e2e flow", false, trigger, now).with_texts(
        "Native e2e flow",
        false,
        graph.trigger_json(),
        texts(&graph),
        now,
    );
    let created = api.create_flow(&flow).await.expect("POST /api/v1/flows");
    assert_eq!(created, id);
    println!("flows: created {id}");

    // Read: the same graph back.
    let read = api.flow(&id).await.expect("GET /api/v1/flows/{id}");
    assert_eq!(read.name, "Native e2e flow");
    assert!(!read.enabled);
    if !graph_server {
        assert_eq!(read.nodes, flow.nodes, "a legacy server stores the text");
    }
    let (stored, _) = FlowGraph::of(&read).expect("the stored graph");
    assert_eq!(stored.nodes.len(), 1);
    assert_eq!(config_id(&stored.nodes[0]), first_id);
    assert_eq!(stored.edges, graph.edges, "trigger → node");
    let listed = api.flows().await.expect("GET /api/v1/flows");
    assert!(listed.iter().any(|flow| flow.id == id), "listed");
    println!("flows: read {id} back");

    // Update: a second node wired straight from the trigger. On a graph
    // server that is a branch; a legacy server runs it first in the line.
    let mut graph = stored;
    let second_id = graph.fresh_id(2);
    let second = graph.add(kind, second_id.clone(), 260.0, 110.0);
    complete(&mut graph, End::Node(second), &catalogue);
    if graph_server {
        graph
            .connect(End::Trigger, 0, second, &catalogue)
            .expect("a second branch");
    } else {
        // The legacy line: trigger → second → first.
        graph.edges.clear();
        graph.connect(End::Trigger, 0, second, &catalogue).unwrap();
        graph.connect(End::Node(second), 0, 0, &catalogue).unwrap();
    }
    graph.validate(&catalogue).expect("a flow the server reads");
    let updated = read.with_texts(
        "Native e2e flow (edited)",
        false,
        graph.trigger_json(),
        texts(&graph),
        now + 1,
    );
    api.update_flow(&updated)
        .await
        .expect("PUT /api/v1/flows/{id}");
    let read = api.flow(&id).await.expect("GET /api/v1/flows/{id}");
    assert_eq!(read.name, "Native e2e flow (edited)");
    let (stored, _) = FlowGraph::of(&read).expect("the stored graph");
    let from_trigger: Vec<&str> = stored
        .outgoing(End::Trigger)
        .into_iter()
        .map(|edge| stored.edges[edge].to.as_str())
        .collect();
    if graph_server {
        assert_eq!(stored.edges.len(), 2);
        assert!(from_trigger.contains(&first_id.as_str()));
        assert!(from_trigger.contains(&second_id.as_str()));
    } else {
        let order: Vec<&str> = stored.nodes.iter().map(config_id).collect();
        assert_eq!(order, [second_id.as_str(), first_id.as_str()]);
        assert_eq!(from_trigger, [second_id.as_str()]);
    }
    println!("flows: added a node and wired it from the trigger");

    // Start and stop: the engine's state; the stored flag stays.
    api.run_flow(&id, true)
        .await
        .expect("POST /api/v1/flows/{id}/start");
    api.run_flow(&id, false)
        .await
        .expect("POST /api/v1/flows/{id}/stop");
    let read = api.flow(&id).await.expect("GET /api/v1/flows/{id}");
    assert!(!read.enabled, "start and stop leave the stored flag");
    println!("flows: started and stopped {id}");

    // Test: a dry run of the draft on a graph server, the stored steps on
    // an older one.
    if graph_server {
        let input = parse_input(TEST_INPUT).expect("the default input");
        let run = api
            .test_flow_draft(&read.trigger_config, &read.nodes, input.clone())
            .await
            .expect("POST /api/v1/flows/test")
            .expect("a graph server dry-runs drafts");
        let ran: Vec<&str> = run.trace.iter().map(|step| step.node_id.as_str()).collect();
        assert_eq!(ran.len(), 2, "both branches ran: {ran:?}");
        assert!(run.ok, "the dry run went through: {:?}", run.error);
        println!("flows: dry-ran {id}: ok {} · {}", run.ok, ran.join(", "));
        // A refusal names its node.
        let mut broken = stored.clone();
        broken
            .edges
            .push(voltrus_scada_native::api::eng_flows::Edge {
                from: first_id.clone(),
                port: 9,
                to: second_id.clone(),
            });
        let error = api
            .test_flow_draft(&read.trigger_config, &broken.nodes_json(), input.clone())
            .await
            .expect_err("a wire from a port the node lacks is refused");
        let refusal = refusal(&error).expect("a 400 about the flow");
        assert_eq!(refusal.node_id.as_deref(), Some(first_id.as_str()));
        println!("flows: refused draft names {}", first_id);
        // Run now: the saved flow, for real, with its trace.
        let live = api
            .run_flow_now(&id, Some(input))
            .await
            .expect("POST /api/v1/flows/{id}/run");
        assert_eq!(live.trace.len(), 2, "the run's trace");
        println!("flows: ran {id} once on demand");
    } else {
        let payload = serde_json::from_str(TEST_PAYLOAD).expect("the default payload");
        let result = api
            .test_flow(&id, payload)
            .await
            .expect("POST /api/v1/flows/{id}/test");
        assert!(result.success);
        assert_eq!(
            result
                .node_results
                .first()
                .map(|step| step.node_id.as_str()),
            Some(second_id.as_str()),
            "the node wired from the trigger ran first"
        );
        println!("flows: tested {id}");
    }

    // Runs: the engine logs only real runs.
    let runs = api
        .flow_runs(&id)
        .await
        .expect("GET /api/v1/flows/{id}/runs");
    assert!(runs.iter().all(|run| run.flow_id == id));
    println!("flows: {} runs logged for {id}", runs.len());

    // Delete: the plant has the flows it had.
    api.delete_flow(&id)
        .await
        .expect("DELETE /api/v1/flows/{id}");
    let after = api.flows().await.expect("GET /api/v1/flows");
    assert!(!after.iter().any(|flow| flow.id == id), "deleted");
    assert_eq!(after.len(), before.len());
    println!("flows: deleted {id}");
}
