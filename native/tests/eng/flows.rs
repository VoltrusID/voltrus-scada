//! Data Manager › Flows against the live plant: every route the section
//! calls. A flow is created (stopped, on a Modbus device the plant does
//! not have, so it never fires), read back as stored, edited the way the
//! canvas editor edits it (a step added and wired from the trigger), run
//! and stopped, tested, its runs listed, and deleted.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_flows::{
    End, Flow, FlowGraph, TEST_PAYLOAD, node_kind, trigger_kind,
};

pub(super) async fn round_trip(api: &VoltrusApi) {
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

    // Create: the web client's new flow, with one filter.
    let modbus = trigger_kind("modbus").expect("the modbus trigger");
    let mut graph = FlowGraph::default();
    graph.set_trigger_kind(modbus);
    graph
        .trigger
        .insert("device_id".into(), json!("e2e-native-no-such-device"));
    let filter_id = graph.fresh_id(1);
    graph.add(
        node_kind("filter").expect("filter"),
        None,
        filter_id.clone(),
    );
    graph.validate().expect("a flow the server reads");
    let flow = Flow::blank(&id, "Native e2e flow", false, modbus, now).with_graph(
        "Native e2e flow",
        false,
        &graph,
        now,
    );
    let created = api.create_flow(&flow).await.expect("POST /api/v1/flows");
    assert_eq!(created, id);
    println!("flows: created {id}");

    // Read: stored as sent.
    let read = api.flow(&id).await.expect("GET /api/v1/flows/{id}");
    assert_eq!(read.name, "Native e2e flow");
    assert!(!read.enabled);
    assert_eq!(read.trigger_config, flow.trigger_config);
    assert_eq!(read.nodes, flow.nodes);
    let listed = api.flows().await.expect("GET /api/v1/flows");
    assert!(listed.iter().any(|flow| flow.id == id), "listed");
    println!("flows: read {id} back byte for byte");

    // Update: an expression wired straight from the trigger runs first.
    let mut graph = FlowGraph::of(&read).expect("the stored graph");
    let expression_id = graph.fresh_id(2);
    let (index, _) = graph.add(
        node_kind("expression").expect("expression"),
        Some(End::Node(0)),
        expression_id.clone(),
    );
    assert_eq!(index, 1, "added after the filter");
    graph
        .connect(End::Trigger, index)
        .expect("a wire the engine runs")
        .expect("the steps reordered");
    graph.validate().expect("a flow the server reads");
    let updated = read.with_graph("Native e2e flow (edited)", false, &graph, now + 1);
    api.update_flow(&updated)
        .await
        .expect("PUT /api/v1/flows/{id}");
    let read = api.flow(&id).await.expect("GET /api/v1/flows/{id}");
    assert_eq!(read.name, "Native e2e flow (edited)");
    assert_eq!(read.nodes, updated.nodes, "the editor's JSON as stored");
    let stored = FlowGraph::of(&read).expect("the stored graph");
    let order: Vec<&str> = stored
        .nodes
        .iter()
        .map(|node| node["id"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(order, [expression_id.as_str(), filter_id.as_str()]);
    println!("flows: added an expression and wired it from the trigger");

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

    // Test: the stored steps run once on the payload, in array order.
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
        Some(expression_id.as_str()),
        "the expression ran first"
    );
    println!(
        "flows: tested {id}: {}",
        result
            .node_results
            .iter()
            .map(|step| format!("{} {}", step.node_id, step.outcome()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Runs: the engine logs only real runs; this flow never fired.
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
