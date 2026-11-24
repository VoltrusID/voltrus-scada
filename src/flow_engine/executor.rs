//! Runs one message through a flow graph.
//!
//! The message enters on the edges leaving the start (the trigger, or a node
//! whose held message is being released), each node's result goes out on the
//! port it chose, and a message sent to several edges is cloned. Nodes are run
//! breadth first; one message may cause at most [`MAX_NODE_RUNS`] node runs.

use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;

use super::graph::FlowGraph;
use super::message::Message;
use super::node::{NodeContext, NodeResult};
use super::state::FlowState;
use crate::cache::DeviceCache;
use crate::storage::Storage;

/// Node runs one trigger message may cause (fan-out included).
pub const MAX_NODE_RUNS: usize = 1000;

/// What a run needs from the server.
#[derive(Clone)]
pub struct ExecEnv {
    pub flow_id: String,
    pub state: FlowState,
    pub storage: Storage,
    pub cache: DeviceCache,
    pub smtp: Option<crate::models::SmtpConfig>,
    pub dry_run: bool,
}

/// One node run, as the trace shows it.
#[derive(Debug, Clone, Serialize)]
pub struct TraceEntry {
    pub node_id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub input: Value,
    /// The payload the node sent on, or (test run of a side-effect node, a
    /// `debug` node, a held message) the node's own report.
    pub output: Value,
    /// The output port the message left on (the first, when several).
    pub port: Option<usize>,
    /// Every port the message left on (a `switch` in `all` mode uses several).
    pub ports: Vec<usize>,
    pub error: Option<String>,
    pub duration_us: u64,
    pub side_effect_skipped: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecResult {
    /// No node failed without its error port taking the message, and the run
    /// cap was not hit.
    pub ok: bool,
    /// The first unhandled error.
    pub error: Option<String>,
    pub trace: Vec<TraceEntry>,
    /// Nodes where a message ended: it left on a port with no edge, or the
    /// node had nowhere to send it.
    pub outputs_reached: Vec<String>,
    /// Payload of the last message that reached the end of a branch.
    #[serde(skip)]
    pub last_payload: Option<Value>,
}

/// Run `msg` through `graph`, starting on the edges that leave `start` on `start_port`.
pub async fn execute(
    graph: &FlowGraph,
    start: &str,
    start_port: usize,
    msg: Message,
    env: &ExecEnv,
) -> ExecResult {
    let mut queue: VecDeque<(String, Message)> = VecDeque::new();
    for to in graph.targets(start, start_port) {
        queue.push_back((to.to_string(), msg.clone()));
    }
    let mut result = ExecResult {
        ok: true,
        error: None,
        trace: Vec::new(),
        outputs_reached: Vec::new(),
        last_payload: None,
    };
    let mut runs = 0usize;
    while let Some((node_id, msg)) = queue.pop_front() {
        if runs >= MAX_NODE_RUNS {
            result.ok = false;
            result.error.get_or_insert_with(|| {
                format!("stopped after {MAX_NODE_RUNS} node runs for one message")
            });
            break;
        }
        runs += 1;
        let Some(node) = graph.node(&node_id) else {
            continue;
        };
        let ctx = NodeContext {
            flow_id: env.flow_id.clone(),
            node_id: node_id.clone(),
            state: env.state.clone(),
            storage: env.storage.clone(),
            cache: env.cache.clone(),
            dry_run: env.dry_run,
            smtp: env.smtp.clone(),
        };
        let input = msg.payload.clone();
        let fallback = msg.clone();
        let started = std::time::Instant::now();
        let outcome = node.config.run(msg, &ctx).await;
        let duration_us = started.elapsed().as_micros() as u64;
        let side_effect_skipped = env.dry_run && node.config.is_side_effect();

        let (routes, report, error): (Vec<(usize, Message)>, Option<Value>, Option<String>) =
            match outcome {
                Ok(out) => {
                    let routes = match out.result {
                        NodeResult::Continue(m) => vec![(0, m)],
                        NodeResult::Port(p, m) => vec![(p, m)],
                        NodeResult::Ports(v) => v,
                        NodeResult::FilterReject => Vec::new(),
                    };
                    (routes, out.report, None)
                }
                Err(e) => match node.config.error_port() {
                    Some(p) => {
                        let mut m = fallback;
                        m.metadata.insert("error".into(), e.clone());
                        m.metadata.insert("error_node".into(), node_id.clone());
                        (vec![(p, m)], None, Some(e))
                    }
                    None => (Vec::new(), None, Some(e)),
                },
            };

        if let Some(e) = &error {
            // Handled when the error port leads somewhere; otherwise the run failed.
            let handled = node
                .config
                .error_port()
                .is_some_and(|p| graph.targets(&node_id, p).next().is_some());
            if !handled {
                result.ok = false;
                result
                    .error
                    .get_or_insert_with(|| format!("node '{node_id}': {e}"));
            }
        }

        let output = match (&report, routes.first()) {
            (Some(r), _) => r.clone(),
            (None, Some((_, m))) => m.payload.clone(),
            (None, None) => Value::Null,
        };
        let ports: Vec<usize> = routes.iter().map(|(p, _)| *p).collect();
        result.trace.push(TraceEntry {
            node_id: node_id.clone(),
            node_type: node.config.type_name().to_string(),
            input,
            output,
            port: ports.first().copied(),
            ports,
            error,
            duration_us,
            side_effect_skipped,
        });

        let mut ended = routes.is_empty();
        for (port, m) in routes {
            let mut targets = graph.targets(&node_id, port).peekable();
            if targets.peek().is_none() {
                ended = true;
                result.last_payload = Some(m.payload.clone());
                continue;
            }
            for to in targets {
                queue.push_back((to.to_string(), m.clone()));
            }
        }
        if ended && !result.outputs_reached.contains(&node_id) {
            result.outputs_reached.push(node_id);
        }
    }
    result
}

/// Largest JSON text kept for one node's input or output in a stored run trace.
pub const TRACE_VALUE_LIMIT: usize = 2048;
/// Node runs kept in one stored trace.
pub const TRACE_ENTRY_LIMIT: usize = 200;

/// Shrink a trace for the run log: long values become a note with their size,
/// and only the first [`TRACE_ENTRY_LIMIT`] entries are kept.
pub fn truncate_trace(trace: &[TraceEntry]) -> Value {
    let cut = |v: &Value| -> Value {
        let text = v.to_string();
        if text.len() <= TRACE_VALUE_LIMIT {
            v.clone()
        } else {
            serde_json::json!({"truncated": true, "bytes": text.len()})
        }
    };
    let entries: Vec<Value> = trace
        .iter()
        .take(TRACE_ENTRY_LIMIT)
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                obj.insert("input".into(), cut(&t.input));
                obj.insert("output".into(), cut(&t.output));
            }
            v
        })
        .collect();
    Value::Array(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(storage: Storage) -> ExecEnv {
        ExecEnv {
            flow_id: "f".into(),
            state: FlowState::new(),
            storage,
            cache: DeviceCache::new(),
            smtp: None,
            dry_run: false,
        }
    }

    fn graph(nodes: Value, edges: Value) -> FlowGraph {
        FlowGraph::from_value(json!({"version": 2, "nodes": nodes, "edges": edges})).unwrap()
    }

    fn msg(payload: Value) -> Message {
        Message::new(payload)
    }

    fn reached(r: &ExecResult) -> Vec<&str> {
        r.outputs_reached.iter().map(String::as_str).collect()
    }

    #[tokio::test]
    async fn http_out_errors_never_show_the_url_password() {
        let url = format!("http://user:{}@unresolvable.invalid/hook", "topsecretpw");
        let g = graph(
            json!([{"id": "h", "type": "http_out", "url": url, "timeout_ms": 2000}]),
            json!([{"from": "trigger", "to": "h"}]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert!(!r.ok);
        let shown = serde_json::to_string(&r.trace).unwrap() + &format!("{:?}", r.error);
        assert!(shown.contains("HttpOut failed"), "{shown}");
        assert!(!shown.contains("topsecretpw"), "{shown}");
    }

    #[tokio::test]
    async fn filter_reject_goes_to_port_1() {
        let g = graph(
            json!([
                {"id": "f", "type": "filter", "condition": "temp > 50"},
                {"id": "hot", "type": "debug"},
                {"id": "cool", "type": "debug"},
            ]),
            json!([
                {"from": "trigger", "to": "f"},
                {"from": "f", "port": 0, "to": "hot"},
                {"from": "f", "port": 1, "to": "cool"},
            ]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let r = execute(&g, "trigger", 0, msg(json!({"temp": 20})), &e).await;
        assert!(r.ok);
        assert_eq!(reached(&r), ["cool"]);
        assert_eq!(r.trace[0].port, Some(1));
        let r = execute(&g, "trigger", 0, msg(json!({"temp": 80})), &e).await;
        assert_eq!(reached(&r), ["hot"]);
        assert_eq!(r.trace[0].port, Some(0));
    }

    #[tokio::test]
    async fn switch_routes_first_or_all_and_otherwise() {
        let nodes = |mode: &str| {
            json!([
                {"id": "s", "type": "switch", "mode": mode, "rules": [
                    {"name": "warm", "expression": "temp > 20"},
                    {"name": "hot", "expression": "temp > 50"},
                ]},
                {"id": "a", "type": "debug"},
                {"id": "b", "type": "debug"},
                {"id": "o", "type": "debug"},
            ])
        };
        let edges = json!([
            {"from": "trigger", "to": "s"},
            {"from": "s", "port": 0, "to": "a"},
            {"from": "s", "port": 1, "to": "b"},
            {"from": "s", "port": 2, "to": "o"},
        ]);
        let e = env(Storage::new_in_memory().unwrap());

        let first = graph(nodes("first"), edges.clone());
        let r = execute(&first, "trigger", 0, msg(json!({"temp": 80})), &e).await;
        assert_eq!(reached(&r), ["a"]);
        let r = execute(&first, "trigger", 0, msg(json!({"temp": 0})), &e).await;
        assert_eq!(reached(&r), ["o"]);

        let all = graph(nodes("all"), edges);
        let r = execute(&all, "trigger", 0, msg(json!({"temp": 80})), &e).await;
        assert_eq!(reached(&r), ["a", "b"]);
        assert_eq!(r.trace[0].ports, vec![0, 1]);
    }

    #[tokio::test]
    async fn fan_out_clones_the_message() {
        let g = graph(
            json!([
                {"id": "x", "type": "expression", "output_field": "a", "expr": "1"},
                {"id": "y", "type": "expression", "output_field": "b", "expr": "2"},
                {"id": "d1", "type": "debug"},
                {"id": "d2", "type": "debug"},
            ]),
            json!([
                {"from": "trigger", "to": "x"},
                {"from": "trigger", "to": "y"},
                {"from": "x", "to": "d1"},
                {"from": "y", "to": "d2"},
            ]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert_eq!(reached(&r), ["d1", "d2"]);
        let d1 = r.trace.iter().find(|t| t.node_id == "d1").unwrap();
        let d2 = r.trace.iter().find(|t| t.node_id == "d2").unwrap();
        // Each branch only sees its own change.
        assert_eq!(d1.input, json!({"a": 1.0}));
        assert_eq!(d2.input, json!({"b": 2.0}));
    }

    #[tokio::test]
    async fn fan_in_runs_the_node_once_per_message() {
        let g = graph(
            json!([
                {"id": "a", "type": "debug"},
                {"id": "b", "type": "debug"},
                {"id": "j", "type": "debug"},
            ]),
            json!([
                {"from": "trigger", "to": "a"},
                {"from": "trigger", "to": "b"},
                {"from": "a", "to": "j"},
                {"from": "b", "to": "j"},
            ]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert_eq!(r.trace.iter().filter(|t| t.node_id == "j").count(), 2);
    }

    #[tokio::test]
    async fn the_run_cap_stops_runaway_fan_out() {
        // Ten layers that each double the messages: 2^10 > MAX_NODE_RUNS.
        let mut nodes = Vec::new();
        let mut edges = vec![json!({"from": "trigger", "to": "l0a"})];
        for layer in 0..10 {
            for s in ["a", "b"] {
                nodes.push(json!({"id": format!("l{layer}{s}"), "type": "debug"}));
            }
            if layer > 0 {
                for from in ["a", "b"] {
                    for to in ["a", "b"] {
                        edges.push(json!({"from": format!("l{}{from}", layer - 1), "to": format!("l{layer}{to}")}));
                    }
                }
            }
        }
        let g = graph(json!(nodes), json!(edges));
        let e = env(Storage::new_in_memory().unwrap());
        let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert!(!r.ok);
        assert_eq!(r.trace.len(), MAX_NODE_RUNS);
        assert!(r.error.unwrap().contains("stopped"));
    }

    #[tokio::test]
    async fn errors_go_to_the_error_port_when_wired() {
        // write_out to a tag that does not exist fails.
        let nodes = json!([
            {"id": "w", "type": "write_out", "data_source_id": 999},
            {"id": "err", "type": "debug"},
        ]);
        let e = env(Storage::new_in_memory().unwrap());

        let unwired = graph(nodes.clone(), json!([{"from": "trigger", "to": "w"}]));
        let r = execute(&unwired, "trigger", 0, msg(json!({"v": 1})), &e).await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("node 'w'"));

        let wired = graph(
            nodes,
            json!([{"from": "trigger", "to": "w"}, {"from": "w", "port": 1, "to": "err"}]),
        );
        let r = execute(&wired, "trigger", 0, msg(json!({"v": 1})), &e).await;
        assert!(r.ok, "{:?}", r.error);
        assert_eq!(reached(&r), ["err"]);
        assert!(r.trace[0].error.is_some());
        let dbg = &r.trace[1].output["debug"]["metadata"];
        assert!(
            dbg["error"].as_str().unwrap().contains("not found"),
            "{dbg}"
        );
    }

    #[tokio::test]
    async fn on_change_passes_only_changes_beyond_the_deadband() {
        let g = graph(
            json!([
                {"id": "c", "type": "on_change", "field": "v", "deadband": 0.5},
                {"id": "yes", "type": "debug"},
                {"id": "no", "type": "debug"},
            ]),
            json!([
                {"from": "trigger", "to": "c"},
                {"from": "c", "port": 0, "to": "yes"},
                {"from": "c", "port": 1, "to": "no"},
            ]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let mut seen = Vec::new();
        for v in [10.0, 10.3, 10.6, 10.6, 9.0] {
            let r = execute(&g, "trigger", 0, msg(json!({"v": v})), &e).await;
            seen.push(r.outputs_reached[0].clone());
        }
        // First value always passes; 10.3 is within 0.5 of 10; 10.6 is 0.6 past 10.
        assert_eq!(seen, ["yes", "no", "yes", "no", "yes"]);
    }

    #[tokio::test]
    async fn rate_limit_drop_mode_uses_the_dropped_port() {
        let g = graph(
            json!([
                {"id": "r", "type": "rate_limit", "max": 2, "duration_ms": 60000},
                {"id": "ok", "type": "debug"},
                {"id": "dropped", "type": "debug"},
            ]),
            json!([
                {"from": "trigger", "to": "r"},
                {"from": "r", "port": 0, "to": "ok"},
                {"from": "r", "port": 1, "to": "dropped"},
            ]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        let mut seen = Vec::new();
        for _ in 0..4 {
            let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
            seen.push(r.outputs_reached[0].clone());
        }
        assert_eq!(seen, ["ok", "ok", "dropped", "dropped"]);
    }

    #[tokio::test]
    async fn rate_limit_keep_latest_holds_the_newest_message() {
        let g = graph(
            json!([
                {"id": "r", "type": "rate_limit", "max": 1, "duration_ms": 50, "mode": "keep_latest"},
                {"id": "ok", "type": "debug"},
            ]),
            json!([{"from": "trigger", "to": "r"}, {"from": "r", "to": "ok"}]),
        );
        let e = env(Storage::new_in_memory().unwrap());
        for n in 1..=3 {
            execute(&g, "trigger", 0, msg(json!({"n": n})), &e).await;
        }
        // 1 passed; 2 then 3 were held, 3 replacing 2.
        assert!(e.state.take_due(std::time::Instant::now()).is_empty());
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        let due = e.state.take_due(std::time::Instant::now());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message.payload["n"], 3);
        // Released messages continue from the node's port 0.
        let r = execute(&g, &due[0].node_id, 0, due[0].message.clone(), &e).await;
        assert_eq!(reached(&r), ["ok"]);
    }

    #[tokio::test]
    async fn delay_waits_in_a_real_run_but_not_in_a_test_run() {
        let g = graph(
            json!([{"id": "d", "type": "delay", "duration_ms": 80}]),
            json!([{"from": "trigger", "to": "d"}]),
        );
        let mut e = env(Storage::new_in_memory().unwrap());
        let t = std::time::Instant::now();
        execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert!(t.elapsed() >= std::time::Duration::from_millis(80));

        e.dry_run = true;
        let t = std::time::Instant::now();
        let r = execute(&g, "trigger", 0, msg(json!({})), &e).await;
        assert!(t.elapsed() < std::time::Duration::from_millis(80));
        assert_eq!(r.trace[0].output["delay_ms"], 80);
    }

    #[test]
    fn stored_traces_are_truncated() {
        let big = json!({"blob": "x".repeat(TRACE_VALUE_LIMIT * 2)});
        let entry = TraceEntry {
            node_id: "n".into(),
            node_type: "debug".into(),
            input: big.clone(),
            output: json!({"small": 1}),
            port: Some(0),
            ports: vec![0],
            error: None,
            duration_us: 1,
            side_effect_skipped: false,
        };
        let v = truncate_trace(&vec![entry; TRACE_ENTRY_LIMIT + 5]);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), TRACE_ENTRY_LIMIT);
        assert_eq!(arr[0]["input"]["truncated"], true);
        assert_eq!(arr[0]["output"]["small"], 1);
        assert_eq!(arr[0]["node_id"], "n");
    }
}
