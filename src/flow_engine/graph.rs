//! A flow's nodes and the edges between their output ports.
//!
//! Stored in the `flows.nodes` column as either the legacy (v1) array of nodes,
//! run one after the other, or a v2 object:
//!
//! ```json
//! { "version": 2,
//!   "nodes": [ { "id": "n1", "type": "filter", "x": 120, "y": 80, "condition": "temp > 50" } ],
//!   "edges": [ { "from": "trigger", "port": 0, "to": "n1" } ] }
//! ```
//!
//! `"trigger"` is the virtual source for the flow's trigger. Reading always
//! yields v2; v1 is converted on the way in.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

use super::node::NodeConfig;

/// The id edges use for the flow's trigger.
pub const TRIGGER_ID: &str = "trigger";

/// Ids the HTTP routes under `/api/v1/flows/` use for themselves.
const RESERVED_IDS: &[&str] = &[TRIGGER_ID];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    #[serde(default)]
    pub port: usize,
    pub to: String,
}

/// One node: its parsed configuration plus the object it came from, so layout
/// (`x`, `y`, `name`) and anything an editor adds survive a round trip.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub config: NodeConfig,
    pub raw: Value,
}

impl GraphNode {
    pub fn id(&self) -> &str {
        self.config.id()
    }
}

#[derive(Debug, Clone, Default)]
pub struct FlowGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}

/// Vertical spacing of converted v1 nodes.
const V1_X: i64 = 120;
const V1_Y0: i64 = 80;
const V1_DY: i64 = 120;

fn is_output_type(t: &str) -> bool {
    matches!(
        t,
        "data_source_out" | "http_out" | "alarm_out" | "write_out"
    )
}

/// Why a flow graph was refused, and the node it is about (for editors to mark).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphError {
    pub message: String,
    pub node_id: Option<String>,
}

impl GraphError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            node_id: None,
        }
    }

    pub fn at(node_id: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            node_id: Some(node_id.to_string()),
        }
    }
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<GraphError> for String {
    fn from(e: GraphError) -> Self {
        e.message
    }
}

impl FlowGraph {
    /// Parse stored `nodes` JSON text (v1 or v2) and validate it. Legacy v1
    /// pipelines with missing or repeated node ids get unique ones.
    pub fn parse(text: &str) -> Result<Self, GraphError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| GraphError::new(format!("nodes is not valid JSON: {e}")))?;
        Self::build(value, false)
    }

    /// Parse and validate submitted `nodes` (v1 array, v2 object, or JSON text
    /// holding either). Unlike [`FlowGraph::parse`], v1 node ids must be given
    /// and unique.
    pub fn from_value(value: Value) -> Result<Self, GraphError> {
        Self::build(value, true)
    }

    fn build(value: Value, strict: bool) -> Result<Self, GraphError> {
        let graph = match value {
            Value::String(s) => {
                let inner: Value = serde_json::from_str(&s)
                    .map_err(|e| GraphError::new(format!("nodes is not valid JSON: {e}")))?;
                return Self::build(inner, strict);
            }
            Value::Null => Self::default(),
            Value::Array(items) => Self::from_v1(items, strict)?,
            Value::Object(mut obj) => {
                match obj.get("version") {
                    Some(v) if v.as_u64() == Some(2) => {}
                    None => {}
                    Some(other) => {
                        return Err(GraphError::new(format!(
                            "unsupported nodes version {other}"
                        )));
                    }
                }
                let nodes = match obj.remove("nodes") {
                    Some(Value::Array(a)) => a,
                    None | Some(Value::Null) => Vec::new(),
                    Some(_) => return Err(GraphError::new("nodes.nodes must be an array")),
                };
                let edges: Vec<Edge> = match obj.remove("edges") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(v) => serde_json::from_value(v).map_err(|e| {
                        GraphError::new(format!("edges must be a list of {{from, port, to}}: {e}"))
                    })?,
                };
                let mut parsed = Vec::with_capacity(nodes.len());
                for (i, raw) in nodes.into_iter().enumerate() {
                    parsed.push(parse_node(raw, i)?);
                }
                Self {
                    nodes: parsed,
                    edges,
                }
            }
            _ => {
                return Err(GraphError::new(
                    "nodes must be an array (v1) or an object (v2)",
                ));
            }
        };
        graph.validate()?;
        Ok(graph)
    }

    /// Convert a v1 pipeline: chain the nodes in the order the v1 engine ran
    /// them (processing nodes first, then output nodes, each group in array
    /// order) and lay them out top to bottom. A filter's reject port stays
    /// unconnected; threshold's two ports and script's error port both lead to
    /// the next node, because in v1 those always continued.
    fn from_v1(items: Vec<Value>, strict: bool) -> Result<Self, GraphError> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut raws = Vec::with_capacity(items.len());
        for (i, mut raw) in items.into_iter().enumerate() {
            let Some(obj) = raw.as_object_mut() else {
                return Err(GraphError::new(format!("node {} is not an object", i + 1)));
            };
            if strict {
                let id = obj.get("id").and_then(Value::as_str).unwrap_or("").trim();
                if id.is_empty() {
                    return Err(GraphError::new(format!("node {} has no id", i + 1)));
                }
                if !seen.insert(id.to_string()) {
                    return Err(GraphError::at(
                        id,
                        format!("node ids must be unique: duplicate node id '{id}'"),
                    ));
                }
                raws.push(raw);
                continue;
            }
            // Old writers did not always give ids, or unique ones.
            let mut id = obj
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && !RESERVED_IDS.contains(s))
                .map(str::to_string)
                .unwrap_or_else(|| format!("n{}", i + 1));
            if seen.contains(&id) {
                let base = id.clone();
                let mut n = 2;
                while seen.contains(&id) {
                    id = format!("{base}_{n}");
                    n += 1;
                }
            }
            seen.insert(id.clone());
            obj.insert("id".into(), Value::String(id));
            raws.push(raw);
        }
        let (process, output): (Vec<Value>, Vec<Value>) = raws
            .into_iter()
            .partition(|r| !is_output_type(r.get("type").and_then(Value::as_str).unwrap_or("")));
        let mut nodes = Vec::new();
        for (i, mut raw) in process.into_iter().chain(output).enumerate() {
            if let Some(obj) = raw.as_object_mut() {
                obj.entry("x").or_insert(json!(V1_X));
                obj.entry("y").or_insert(json!(V1_Y0 + V1_DY * i as i64));
            }
            nodes.push(parse_node(raw, i)?);
        }
        let mut edges = Vec::new();
        let mut prev: Option<&GraphNode> = None;
        for node in &nodes {
            match prev {
                None => edges.push(Edge {
                    from: TRIGGER_ID.into(),
                    port: 0,
                    to: node.id().into(),
                }),
                Some(p) => {
                    let ports: &[usize] = match &p.config {
                        NodeConfig::Threshold { .. } | NodeConfig::Script { .. } => &[0, 1],
                        _ => &[0],
                    };
                    for port in ports {
                        edges.push(Edge {
                            from: p.id().into(),
                            port: *port,
                            to: node.id().into(),
                        });
                    }
                }
            }
            prev = Some(node);
        }
        Ok(Self { nodes, edges })
    }

    /// The v2 document.
    pub fn to_value(&self) -> Value {
        json!({
            "version": 2,
            "nodes": self.nodes.iter().map(|n| n.raw.clone()).collect::<Vec<_>>(),
            "edges": self.edges,
        })
    }

    pub fn to_json_string(&self) -> String {
        self.to_value().to_string()
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id() == id)
    }

    /// Targets of the edges leaving `from` on `port`, in edge order.
    pub fn targets(&self, from: &str, port: usize) -> impl Iterator<Item = &str> {
        self.edges
            .iter()
            .filter(move |e| e.from == from && e.port == port)
            .map(|e| e.to.as_str())
    }

    /// Unique ids, edges between known nodes on existing ports, nothing into
    /// the trigger, no cycles, and every node's own configuration.
    pub fn validate(&self) -> Result<(), GraphError> {
        let mut outputs: HashMap<&str, Vec<String>> = HashMap::new();
        for node in &self.nodes {
            let id = node.id();
            let ty = node.config.type_name();
            if id.trim().is_empty() {
                return Err(GraphError::new(format!("a {ty} node has an empty id")));
            }
            if RESERVED_IDS.contains(&id) {
                return Err(GraphError::at(id, format!("node id '{id}' is reserved")));
            }
            if outputs.insert(id, node.config.outputs()).is_some() {
                return Err(GraphError::at(
                    id,
                    format!("node ids must be unique: duplicate node id '{id}'"),
                ));
            }
            node.config
                .validate()
                .map_err(|e| GraphError::at(id, format!("node '{id}' ({ty}): {e}")))?;
        }
        for e in &self.edges {
            // Edge problems are reported on the node the edge leaves (or
            // enters, when it leaves the trigger or an unknown node).
            let blame = if e.from == TRIGGER_ID || !outputs.contains_key(e.from.as_str()) {
                e.to.as_str()
            } else {
                e.from.as_str()
            };
            if e.to == TRIGGER_ID {
                return Err(GraphError::at(
                    &e.from,
                    format!(
                        "edge from '{}' enters the trigger; the trigger has no input",
                        e.from
                    ),
                ));
            }
            if !outputs.contains_key(e.to.as_str()) {
                return Err(GraphError {
                    message: format!("edge from '{}' goes to unknown node '{}'", e.from, e.to),
                    node_id: (e.from != TRIGGER_ID).then(|| e.from.clone()),
                });
            }
            if e.from == TRIGGER_ID {
                if e.port != 0 {
                    return Err(GraphError::at(
                        blame,
                        format!(
                            "edge from the trigger to '{}' uses port {}; the trigger only has port 0",
                            e.to, e.port
                        ),
                    ));
                }
                continue;
            }
            let Some(outs) = outputs.get(e.from.as_str()) else {
                return Err(GraphError::at(
                    blame,
                    format!("edge to '{}' comes from unknown node '{}'", e.to, e.from),
                ));
            };
            if e.port >= outs.len() {
                return Err(GraphError::at(
                    blame,
                    format!(
                        "edge from '{}' uses port {} but the node has {} output(s): {}",
                        e.from,
                        e.port,
                        outs.len(),
                        outs.join(", ")
                    ),
                ));
            }
        }
        if let Some(id) = self.find_cycle() {
            return Err(GraphError::at(
                &id,
                format!("the edges form a cycle through node '{id}'"),
            ));
        }
        Ok(())
    }

    /// A node on a cycle, if the edges have one (Kahn's algorithm: whatever
    /// cannot be ordered is on or behind a cycle).
    fn find_cycle(&self) -> Option<String> {
        let mut indegree: HashMap<&str, usize> =
            self.nodes.iter().map(|n| (n.id(), 0usize)).collect();
        for e in &self.edges {
            if e.from != TRIGGER_ID
                && let Some(d) = indegree.get_mut(e.to.as_str())
            {
                *d += 1;
            }
        }
        let mut ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut done = 0;
        while let Some(id) = ready.pop() {
            done += 1;
            for e in self.edges.iter().filter(|e| e.from == id) {
                if let Some(d) = indegree.get_mut(e.to.as_str()) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push(e.to.as_str());
                    }
                }
            }
        }
        if done == self.nodes.len() {
            return None;
        }
        // Report the first node (in document order) left with inputs.
        self.nodes
            .iter()
            .find(|n| indegree.get(n.id()).is_some_and(|d| *d > 0))
            .map(|n| n.id().to_string())
    }
}

/// Parse one node object; `index` names it in errors when it has no id.
fn parse_node(raw: Value, index: usize) -> Result<GraphNode, GraphError> {
    let id = raw.get("id").and_then(Value::as_str).map(str::to_string);
    let label = id
        .as_deref()
        .map(|s| format!("node '{s}'"))
        .unwrap_or_else(|| format!("node {}", index + 1));
    let fail = |message: String| GraphError {
        message,
        node_id: id.clone(),
    };
    if !raw.is_object() {
        return Err(fail(format!("{label} is not an object")));
    }
    let ty = raw.get("type").and_then(Value::as_str).unwrap_or("");
    if ty.is_empty() {
        return Err(fail(format!("{label} has no type")));
    }
    if !super::node::NODE_TYPES.contains(&ty) {
        return Err(fail(format!("{label} has unknown type '{ty}'")));
    }
    let config: NodeConfig =
        serde_json::from_value(raw.clone()).map_err(|e| fail(format!("{label} ({ty}): {e}")))?;
    Ok(GraphNode { config, raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(nodes: Value, edges: Value) -> Result<FlowGraph, String> {
        FlowGraph::from_value(json!({"version": 2, "nodes": nodes, "edges": edges}))
            .map_err(String::from)
    }

    #[test]
    fn errors_carry_the_node_id() {
        let at = |nodes: Value, edges: Value| {
            FlowGraph::from_value(json!({"version": 2, "nodes": nodes, "edges": edges}))
                .unwrap_err()
                .node_id
        };
        let debug = |id: &str| json!({"id": id, "type": "debug"});
        assert_eq!(
            at(json!([{"id": "f", "type": "filter"}]), json!([])).as_deref(),
            Some("f")
        );
        assert_eq!(
            at(
                json!([debug("a")]),
                json!([{"from": "a", "port": 4, "to": "a"}])
            )
            .as_deref(),
            Some("a")
        );
        assert_eq!(
            at(json!([debug("a")]), json!([{"from": "a", "to": "nope"}])).as_deref(),
            Some("a")
        );
        assert_eq!(
            at(json!([debug("b")]), json!([{"from": "ghost", "to": "b"}])).as_deref(),
            Some("b")
        );
        assert_eq!(
            at(
                json!([debug("a"), debug("b")]),
                json!([{"from": "a", "to": "b"}, {"from": "b", "to": "a"}])
            )
            .as_deref()
            .map(|id| id == "a" || id == "b"),
            Some(true)
        );
        assert_eq!(
            FlowGraph::from_value(json!("not json"))
                .unwrap_err()
                .node_id,
            None
        );
    }

    #[test]
    fn v1_converts_to_a_chain_in_execution_order() {
        let g = FlowGraph::parse(
            &json!([
                {"id": "w", "type": "write_out", "data_source_id": 1},
                {"id": "f", "type": "filter", "condition": "temp > 1"},
                {"id": "t", "type": "threshold", "field": "temp", "rules": [{"gt": 5}]},
                {"id": "e", "type": "expression", "output_field": "x", "expr": "temp * 2"},
            ])
            .to_string(),
        )
        .unwrap();
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id()).collect();
        // Output nodes run last in v1.
        assert_eq!(ids, ["f", "t", "e", "w"]);
        let edge = |f: &str, p: usize, t: &str| Edge {
            from: f.into(),
            port: p,
            to: t.into(),
        };
        assert_eq!(
            g.edges,
            vec![
                edge("trigger", 0, "f"),
                // filter: reject (port 1) has no edge
                edge("f", 0, "t"),
                // threshold continued whether a rule matched or not
                edge("t", 0, "e"),
                edge("t", 1, "e"),
                edge("e", 0, "w"),
            ]
        );
        // Laid out vertically.
        let ys: Vec<i64> = g
            .nodes
            .iter()
            .map(|n| n.raw["y"].as_i64().unwrap())
            .collect();
        assert!(ys.windows(2).all(|w| w[1] > w[0]));
        assert!(g.nodes.iter().all(|n| n.raw["x"] == json!(V1_X)));

        let v = g.to_value();
        assert_eq!(v["version"], 2);
        assert_eq!(v["edges"][0]["from"], "trigger");
        // And the stored v2 reads back to the same graph.
        let again = FlowGraph::from_value(v).unwrap();
        assert_eq!(again.edges, g.edges);
    }

    #[test]
    fn v1_without_ids_or_with_duplicates_gets_unique_ids() {
        let g = FlowGraph::parse(
            &json!([
                {"type": "debug"},
                {"id": "a", "type": "debug"},
                {"id": "a", "type": "debug"},
            ])
            .to_string(),
        )
        .unwrap();
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id()).collect();
        assert_eq!(ids, ["n1", "a", "a_2"]);
    }

    #[test]
    fn empty_v1_is_an_empty_graph() {
        let g = FlowGraph::parse("[]").unwrap();
        assert!(g.nodes.is_empty() && g.edges.is_empty());
    }

    #[test]
    fn layout_and_extra_fields_survive() {
        let g = v2(
            json!([{"id": "d", "type": "debug", "x": 5, "y": 7, "name": "Look", "color": "red"}]),
            json!([{"from": "trigger", "to": "d"}]),
        )
        .unwrap();
        let v = g.to_value();
        assert_eq!(v["nodes"][0]["name"], "Look");
        assert_eq!(v["nodes"][0]["color"], "red");
        assert_eq!(v["nodes"][0]["x"], 5);
        assert_eq!(v["edges"][0]["port"], 0);
    }

    #[test]
    fn validation_errors_name_the_node() {
        let debug = |id: &str| json!({"id": id, "type": "debug"});
        let err = v2(json!([debug("a")]), json!([{"from": "a", "to": "zz"}])).unwrap_err();
        assert!(err.contains("unknown node 'zz'"), "{err}");

        let err = v2(json!([debug("a")]), json!([{"from": "ghost", "to": "a"}])).unwrap_err();
        assert!(err.contains("unknown node 'ghost'"), "{err}");

        let err = v2(
            json!([debug("a")]),
            json!([{"from": "a", "port": 1, "to": "a"}]),
        )
        .unwrap_err();
        assert!(err.contains("port 1") && err.contains("'a'"), "{err}");

        let err = v2(
            json!([{"id": "f", "type": "filter", "condition": "x > 1"}, debug("b")]),
            json!([{"from": "f", "port": 2, "to": "b"}]),
        )
        .unwrap_err();
        assert!(err.contains("pass, reject"), "{err}");

        let err = v2(json!([debug("a"), debug("a")]), json!([])).unwrap_err();
        assert!(err.contains("duplicate node id 'a'"), "{err}");

        let err = v2(json!([debug("a")]), json!([{"from": "a", "to": "trigger"}])).unwrap_err();
        assert!(err.contains("enters the trigger"), "{err}");

        let err = v2(
            json!([debug("a")]),
            json!([{"from": "trigger", "port": 1, "to": "a"}]),
        )
        .unwrap_err();
        assert!(err.contains("only has port 0"), "{err}");

        let err = v2(json!([{"id": "f", "type": "filter"}]), json!([])).unwrap_err();
        assert!(
            err.contains("node 'f'") && err.contains("condition"),
            "{err}"
        );

        let err = v2(
            json!([{"id": "f", "type": "filter", "condition": "(("}]),
            json!([]),
        )
        .unwrap_err();
        assert!(err.contains("node 'f'"), "{err}");

        let err = v2(json!([{"id": "q", "type": "teleport"}]), json!([])).unwrap_err();
        assert!(err.contains("unknown type 'teleport'"), "{err}");

        let err = v2(json!([{"id": "trigger", "type": "debug"}]), json!([])).unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }

    #[test]
    fn cycles_are_rejected() {
        let debug = |id: &str| json!({"id": id, "type": "debug"});
        let err = v2(
            json!([debug("a"), debug("b"), debug("c")]),
            json!([
                {"from": "trigger", "to": "a"},
                {"from": "a", "to": "b"},
                {"from": "b", "to": "c"},
                {"from": "c", "to": "a"},
            ]),
        )
        .unwrap_err();
        assert!(err.contains("cycle"), "{err}");

        let err = v2(json!([debug("a")]), json!([{"from": "a", "to": "a"}])).unwrap_err();
        assert!(err.contains("cycle through node 'a'"), "{err}");

        // A diamond is not a cycle.
        v2(
            json!([debug("a"), debug("b"), debug("c"), debug("d")]),
            json!([
                {"from": "trigger", "to": "a"},
                {"from": "a", "to": "b"},
                {"from": "a", "to": "c"},
                {"from": "b", "to": "d"},
                {"from": "c", "to": "d"},
            ]),
        )
        .unwrap();
    }

    #[test]
    fn disconnected_nodes_are_allowed() {
        v2(json!([{"id": "lonely", "type": "debug"}]), json!([])).unwrap();
    }

    #[test]
    fn switch_ports_follow_its_rules() {
        let sw = json!({"id": "s", "type": "switch", "rules": [
            {"name": "hot", "expression": "temp > 50"},
            {"name": "cold", "expression": "temp < 5"},
        ]});
        // Two rules + otherwise = ports 0..=2.
        v2(
            json!([sw.clone(), {"id": "d", "type": "debug"}]),
            json!([{"from": "s", "port": 2, "to": "d"}]),
        )
        .unwrap();
        let err = v2(
            json!([sw, {"id": "d", "type": "debug"}]),
            json!([{"from": "s", "port": 3, "to": "d"}]),
        )
        .unwrap_err();
        assert!(err.contains("hot, cold, otherwise"), "{err}");
    }
}
