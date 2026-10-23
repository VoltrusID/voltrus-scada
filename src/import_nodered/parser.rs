use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

/// Helper that deserializes a value that can be either a number or a string containing a number.
fn deserialize_flex_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self};

    // Use a helper enum to accept either string or number
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexF64 {
        F64(f64),
        I64(i64),
        Str(String),
        Null,
    }

    match FlexF64::deserialize(deserializer) {
        Ok(FlexF64::F64(v)) => Ok(Some(v)),
        Ok(FlexF64::I64(v)) => Ok(Some(v as f64)),
        Ok(FlexF64::Str(v)) => v
            .parse::<f64>()
            .map(Some)
            .map_err(|_| de::Error::custom(format!("cannot parse '{}' as f64", v))),
        Ok(FlexF64::Null) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Top-level Node-RED flows.json structure (array of nodes).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeRedFlow {
    #[serde(default)]
    pub nodes: Vec<NodeRedNode>,
}

/// A single node in a Node-RED flow.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(non_snake_case)]
pub struct NodeRedNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: Option<String>,
    #[serde(default)]
    pub z: String,
    #[serde(default)]
    pub wires: Vec<Vec<String>>,
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub func: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub qos: Option<u8>,
    #[serde(default)]
    pub broker: Option<String>,
    #[serde(default)]
    pub fc: Option<u8>,
    #[serde(default)]
    pub unitid: Option<u8>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub gmin: Option<f64>,
    #[serde(default)]
    pub gmax: Option<f64>,
    #[serde(default)]
    pub xaxis: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_flex_f64")]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub timeoutUnits: Option<String>,
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub pt: Option<String>,
    #[serde(default)]
    pub ptt: Option<String>,
    #[serde(default)]
    pub conditions: Option<serde_json::Value>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub payload: Option<String>,
    #[serde(default)]
    pub payloadType: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub links: Option<Vec<String>>,
}

/// Categorization of a Node-RED node for conversion purposes.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeCategory {
    Convertible,
    ManualReview,
    UiDecoration,
    Unsupported,
}

impl NodeRedNode {
    pub fn category(&self) -> NodeCategory {
        match self.node_type.as_str() {
            "modbus-read" | "modbus-write" | "modbus-response" | "modbus-flex-getter"
            | "modbus-flex-write" | "modbus-client" | "modbus-server" | "opcua-client"
            | "opcua-item" | "opcua-browser" | "s7-client" | "s7-in" | "s7-out" | "enip-read"
            | "enip-write" | "bacnet-client" => NodeCategory::Convertible,
            "mqtt in" | "mqtt out" | "mqtt-broker" => NodeCategory::Convertible,
            "switch" | "change" | "delay" | "trigger" | "link in" | "link out" | "link-call"
            | "complete" | "catch" | "status" | "exec" | "template" => NodeCategory::Convertible,
            "ui_gauge" | "ui_chart" | "ui_text" | "ui_button" | "ui_slider" | "ui_dropdown"
            | "ui_switch" | "ui_led" | "ui_notification" | "ui_toast" | "ui_table" | "ui_group"
            | "ui_tab" | "ui_spacer" => NodeCategory::Convertible,
            "function" | "python" | "java" => NodeCategory::ManualReview,
            "inject" | "debug" => NodeCategory::UiDecoration,
            "comment" | "tab" | "subflow" | "subflow:" => NodeCategory::UiDecoration,
            _ => NodeCategory::Unsupported,
        }
    }
}

/// Result of parsing a flows.json file.
#[derive(Debug, Clone, Serialize)]
pub struct ParseResult {
    pub total_nodes: usize,
    pub convertible: usize,
    pub manual_review: usize,
    pub ui_decoration: usize,
    pub unsupported: usize,
    pub nodes: Vec<NodeRedNode>,
    pub warnings: Vec<String>,
}

/// Parse a flows.json string into structured data.
pub fn parse_flows(json_str: &str) -> Result<ParseResult, String> {
    let flow: NodeRedFlow = if json_str.trim().starts_with('[') {
        let nodes: Vec<NodeRedNode> = serde_json::from_str(json_str)
            .map_err(|e| format!("Failed to parse flows.json: {}", e))?;
        NodeRedFlow { nodes }
    } else {
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse flows.json: {}", e))?
    };

    let total_nodes = flow.nodes.len();
    let mut convertible = 0;
    let mut manual_review = 0;
    let mut ui_decoration = 0;
    let mut unsupported = 0;
    let mut warnings = Vec::new();

    for node in &flow.nodes {
        match node.category() {
            NodeCategory::Convertible => convertible += 1,
            NodeCategory::ManualReview => {
                manual_review += 1;
                let name = node.name.as_deref().unwrap_or(&node.id);
                warnings.push(format!(
                    "Node '{}' ({}) requires manual review — {} logic must be reimplemented",
                    name, node.id, node.node_type
                ));
            }
            NodeCategory::UiDecoration => ui_decoration += 1,
            NodeCategory::Unsupported => {
                unsupported += 1;
                let name = node.name.as_deref().unwrap_or(&node.id);
                warnings.push(format!(
                    "Node '{}' ({}) type '{}' is not supported and will be skipped",
                    name, node.id, node.node_type
                ));
            }
        }
    }

    Ok(ParseResult {
        total_nodes,
        convertible,
        manual_review,
        ui_decoration,
        unsupported,
        nodes: flow.nodes,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_modbus_flow() {
        let json = r#"[
            {"id":"n1","type":"tab","label":"Flow 1"},
            {"id":"n2","type":"modbus-read","name":"Read Voltage","server":"m1","fc":3,"unitid":1,"address":"40001","quantity":"2"},
            {"id":"n3","type":"debug","name":"Debug Output"}
        ]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.total_nodes, 3);
        assert_eq!(result.convertible, 1);
        assert_eq!(result.manual_review, 0);
        assert_eq!(result.ui_decoration, 2);
        assert_eq!(result.unsupported, 0);
    }

    #[test]
    fn test_parse_function_node() {
        let json = r#"[{"id":"n1","type":"function","name":"My Function","func":"return msg;"}]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.manual_review, 1);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("manual review"));
    }

    #[test]
    fn test_parse_unsupported_node() {
        let json = r#"[{"id":"n1","type":"unknown-plugin","name":"Weird Node"}]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.unsupported, 1);
        assert!(result.warnings[0].contains("not supported"));
    }

    #[test]
    fn test_parse_mqtt_nodes() {
        let json = r#"[
            {"id":"n1","type":"mqtt in","name":"Subscribe","topic":"sensor/#","broker":"b1"},
            {"id":"n2","type":"mqtt out","name":"Publish","topic":"cmd/valve","qos":1,"broker":"b1"}
        ]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.convertible, 2);
    }

    #[test]
    fn test_parse_object_format() {
        let json = r#"{"nodes":[{"id":"n1","type":"tab","label":"Flow 1"},{"id":"n2","type":"modbus-read","name":"Read"}]}"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.total_nodes, 2);
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = "not valid json";
        let result = parse_flows(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_node_category_classification() {
        let modbus = NodeRedNode {
            id: "n1".into(),
            node_type: "modbus-read".into(),
            name: Some("Read".into()),
            z: String::new(),
            wires: Vec::new(),
            properties: HashMap::new(),
            func: None,
            server: None,
            topic: None,
            qos: None,
            broker: None,
            fc: None,
            unitid: None,
            address: None,
            quantity: None,
            label: None,
            gmin: None,
            gmax: None,
            xaxis: None,
            timeout: None,
            timeoutUnits: None,
            property: None,
            pt: None,
            ptt: None,
            conditions: None,
            endpoint: None,
            item: None,
            payload: None,
            payloadType: None,
            template: None,
            info: None,
            links: None,
        };
        assert_eq!(modbus.category(), NodeCategory::Convertible);

        let func = NodeRedNode {
            id: "n2".into(),
            node_type: "function".into(),
            name: Some("Func".into()),
            z: String::new(),
            wires: Vec::new(),
            properties: HashMap::new(),
            func: None,
            server: None,
            topic: None,
            qos: None,
            broker: None,
            fc: None,
            unitid: None,
            address: None,
            quantity: None,
            label: None,
            gmin: None,
            gmax: None,
            xaxis: None,
            timeout: None,
            timeoutUnits: None,
            property: None,
            pt: None,
            ptt: None,
            conditions: None,
            endpoint: None,
            item: None,
            payload: None,
            payloadType: None,
            template: None,
            info: None,
            links: None,
        };
        assert_eq!(func.category(), NodeCategory::ManualReview);

        let unknown = NodeRedNode {
            id: "n3".into(),
            node_type: "rare-custom-node".into(),
            name: None,
            z: String::new(),
            wires: Vec::new(),
            properties: HashMap::new(),
            func: None,
            server: None,
            topic: None,
            qos: None,
            broker: None,
            fc: None,
            unitid: None,
            address: None,
            quantity: None,
            label: None,
            gmin: None,
            gmax: None,
            xaxis: None,
            timeout: None,
            timeoutUnits: None,
            property: None,
            pt: None,
            ptt: None,
            conditions: None,
            endpoint: None,
            item: None,
            payload: None,
            payloadType: None,
            template: None,
            info: None,
            links: None,
        };
        assert_eq!(unknown.category(), NodeCategory::Unsupported);
    }

    #[test]
    fn test_parse_switch_and_delay_nodes() {
        let json = r#"[
            {"id":"n1","type":"switch","name":"Check","property":"payload","pt":"msg","ptt":"number"},
            {"id":"n2","type":"delay","name":"Wait","timeout":"5","timeoutUnits":"seconds"}
        ]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.convertible, 2);
    }

    #[test]
    fn test_parse_ui_gauge_chart_text() {
        let json = r#"[
            {"id":"n1","type":"ui_gauge","name":"Pressure","gmin":0,"gmax":100,"label":"PSI"},
            {"id":"n2","type":"ui_chart","name":"Trend"},
            {"id":"n3","type":"ui_text","name":"Status","label":"Online"}
        ]"#;
        let result = parse_flows(json).unwrap();
        assert_eq!(result.convertible, 3);
    }
}
