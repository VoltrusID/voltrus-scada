use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::parser::{NodeCategory, ParseResult};

/// A converted Modbus device from Node-RED modbus-read nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedModbusDevice {
    pub device_id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u64,
    pub tags: Vec<ConvertedTag>,
}

/// A converted MQTT data source from Node-RED mqtt-in/out nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedMqttSource {
    pub topic: String,
    pub qos: u8,
    pub name: String,
    pub direction: String, // "in" or "out"
    pub source_node_id: String,
}

/// A converted dashboard widget from Node-RED UI nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedWidget {
    pub widget_type: String, // "gauge", "chart", "text", "button", "slider"
    pub name: String,
    pub label: Option<String>,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
    pub source_node_id: String,
    pub format: Option<String>,
}

/// A converted flow engine node from Node-RED logic nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedFlowNode {
    pub node_type: String, // "switch", "delay", "trigger", "change"
    pub name: String,
    pub config: serde_json::Value,
    pub source_node_id: String,
}

/// A converted tag/register from a Node-RED protocol node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedTag {
    pub name: String,
    pub address: u16,
    pub data_type: String,
    pub register_area: String,
    pub scale: f64,
}

/// A function node flagged for manual review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedFunctionNode {
    pub node_id: String,
    pub name: String,
    pub source_code: Option<String>,
    pub warning: String,
}

/// Complete conversion result.
#[derive(Debug, Clone, Serialize)]
pub struct ConversionResult {
    pub modbus_devices: Vec<ConvertedModbusDevice>,
    pub mqtt_sources: Vec<ConvertedMqttSource>,
    pub widgets: Vec<ConvertedWidget>,
    pub flow_nodes: Vec<ConvertedFlowNode>,
    pub function_nodes: Vec<ConvertedFunctionNode>,
    pub skipped_nodes: Vec<SkippedNode>,
    pub warnings: Vec<String>,
}

/// A node that was skipped during conversion.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedNode {
    pub node_id: String,
    pub node_type: String,
    pub name: Option<String>,
    pub reason: String,
}

/// Convert parsed Node-RED nodes into Voltrus entities.
pub fn convert(parse_result: &ParseResult) -> ConversionResult {
    let mut _modbus_devices: Vec<ConvertedModbusDevice> = Vec::new();
    let mut mqtt_sources: Vec<ConvertedMqttSource> = Vec::new();
    let mut widgets: Vec<ConvertedWidget> = Vec::new();
    let mut flow_nodes: Vec<ConvertedFlowNode> = Vec::new();
    let mut function_nodes: Vec<ConvertedFunctionNode> = Vec::new();
    let mut skipped_nodes: Vec<SkippedNode> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Track unique device IDs to merge modbus-read nodes from the same server
    let mut device_map: HashMap<String, ConvertedModbusDevice> = HashMap::new();

    for node in &parse_result.nodes {
        let cat = node.category();
        if cat == NodeCategory::UiDecoration {
            continue;
        }

        let name = node.name.clone().unwrap_or_else(|| node.id.clone());

        match node.node_type.as_str() {
            // ── Modbus nodes ──
            "modbus-read" | "modbus-flex-getter" => {
                let device_id = node
                    .server
                    .clone()
                    .unwrap_or_else(|| format!("nr_modbus_{}", &node.id[..node.id.len().min(8)]));
                let ip = node
                    .properties
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("192.168.1.1")
                    .to_string();
                let port = node
                    .properties
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(502) as u16;
                let unit_id = node.unitid.unwrap_or(1);
                let fc = node.fc.unwrap_or(3);

                // Parse address
                let addr_str = node.address.as_deref().unwrap_or("0");
                let address: u16 = addr_str.parse().unwrap_or(0);

                // Determine register area from function code
                let (register_area, data_type) = match fc {
                    1 | 2 => ("coil", "uint16"),
                    3 => ("holding", "float32"),
                    4 => ("input", "float32"),
                    _ => ("holding", "float32"),
                };

                let tag_name = name.clone();
                let _quantity = node
                    .quantity
                    .as_deref()
                    .and_then(|q| q.parse::<u16>().ok())
                    .unwrap_or(2);

                let scale = 1.0;

                let tag = ConvertedTag {
                    name: tag_name,
                    address,
                    data_type: data_type.to_string(),
                    register_area: register_area.to_string(),
                    scale,
                };

                let entry =
                    device_map
                        .entry(device_id.clone())
                        .or_insert_with(|| ConvertedModbusDevice {
                            device_id: device_id.clone(),
                            name: format!("NR Modbus {}", device_id),
                            ip: ip.clone(),
                            port,
                            unit_id,
                            interval_ms: 1000,
                            tags: Vec::new(),
                        });
                entry.tags.push(tag);
            }

            "modbus-write" | "modbus-flex-write" => {
                let device_id = node
                    .server
                    .clone()
                    .unwrap_or_else(|| format!("nr_modbus_w_{}", &node.id[..node.id.len().min(8)]));
                let ip = node
                    .properties
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("192.168.1.1")
                    .to_string();
                let port = node
                    .properties
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(502) as u16;
                let unit_id = node.unitid.unwrap_or(1);

                let addr_str = node.address.as_deref().unwrap_or("0");
                let address: u16 = addr_str.parse().unwrap_or(0);

                let tag = ConvertedTag {
                    name: name.clone(),
                    address,
                    data_type: "float32".to_string(),
                    register_area: "holding".to_string(),
                    scale: 1.0,
                };

                let entry =
                    device_map
                        .entry(device_id.clone())
                        .or_insert_with(|| ConvertedModbusDevice {
                            device_id: device_id.clone(),
                            name: format!("NR Modbus {}", device_id),
                            ip: ip.clone(),
                            port,
                            unit_id,
                            interval_ms: 1000,
                            tags: Vec::new(),
                        });
                entry.tags.push(tag);
            }

            // ── MQTT nodes ──
            "mqtt in" => {
                mqtt_sources.push(ConvertedMqttSource {
                    topic: node.topic.clone().unwrap_or_else(|| "#".to_string()),
                    qos: node.qos.unwrap_or(0),
                    name,
                    direction: "in".to_string(),
                    source_node_id: node.id.clone(),
                });
            }

            "mqtt out" => {
                mqtt_sources.push(ConvertedMqttSource {
                    topic: node
                        .topic
                        .clone()
                        .unwrap_or_else(|| "voltrus/out".to_string()),
                    qos: node.qos.unwrap_or(0),
                    name,
                    direction: "out".to_string(),
                    source_node_id: node.id.clone(),
                });
            }

            // ── Dashboard UI nodes ──
            "ui_gauge" => {
                widgets.push(ConvertedWidget {
                    widget_type: "gauge".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: node.gmin,
                    max_val: node.gmax,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            "ui_chart" => {
                widgets.push(ConvertedWidget {
                    widget_type: "chart".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: None,
                    max_val: None,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            "ui_text" => {
                widgets.push(ConvertedWidget {
                    widget_type: "text".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: None,
                    max_val: None,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            "ui_button" => {
                widgets.push(ConvertedWidget {
                    widget_type: "button".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: None,
                    max_val: None,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            "ui_slider" => {
                widgets.push(ConvertedWidget {
                    widget_type: "slider".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: node.gmin,
                    max_val: node.gmax,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            "ui_table" => {
                widgets.push(ConvertedWidget {
                    widget_type: "table".to_string(),
                    name,
                    label: node.label.clone(),
                    min_val: None,
                    max_val: None,
                    source_node_id: node.id.clone(),
                    format: None,
                });
            }

            // ── Flow logic nodes ──
            "switch" => {
                let config = serde_json::json!({
                    "condition": node.pt.as_deref().unwrap_or("msg.payload"),
                    "property": node.property.as_deref().unwrap_or("payload"),
                    "comparison": node.ptt.as_deref().unwrap_or("eq"),
                });
                flow_nodes.push(ConvertedFlowNode {
                    node_type: "switch".to_string(),
                    name,
                    config,
                    source_node_id: node.id.clone(),
                });
            }

            "delay" => {
                let timeout_ms = match node.timeoutUnits.as_deref() {
                    Some("milliseconds") => node.timeout.unwrap_or(1000.0),
                    Some("seconds") => node.timeout.unwrap_or(1.0) * 1000.0,
                    Some("minutes") => node.timeout.unwrap_or(1.0) * 60_000.0,
                    Some("hours") => node.timeout.unwrap_or(1.0) * 3_600_000.0,
                    _ => node.timeout.unwrap_or(1000.0),
                };
                let config = serde_json::json!({
                    "delay_ms": timeout_ms as u64,
                });
                flow_nodes.push(ConvertedFlowNode {
                    node_type: "delay".to_string(),
                    name,
                    config,
                    source_node_id: node.id.clone(),
                });
            }

            "trigger" => {
                let config = serde_json::json!({
                    "conditions": node.conditions,
                });
                flow_nodes.push(ConvertedFlowNode {
                    node_type: "trigger".to_string(),
                    name,
                    config,
                    source_node_id: node.id.clone(),
                });
            }

            "change" => {
                let config = serde_json::json!({
                    "rules": node.properties.get("rules").cloned().unwrap_or(serde_json::Value::Null),
                });
                flow_nodes.push(ConvertedFlowNode {
                    node_type: "change".to_string(),
                    name,
                    config,
                    source_node_id: node.id.clone(),
                });
            }

            "template" => {
                let config = serde_json::json!({
                    "template": node.template,
                    "format": node.properties.get("format").cloned().unwrap_or(serde_json::Value::String("mustache".to_string())),
                });
                flow_nodes.push(ConvertedFlowNode {
                    node_type: "template".to_string(),
                    name,
                    config,
                    source_node_id: node.id.clone(),
                });
            }

            // ── Function nodes: manual review ──
            "function" | "python" | "java" => {
                function_nodes.push(ConvertedFunctionNode {
                    node_id: node.id.clone(),
                    name: name.clone(),
                    source_code: node.func.clone(),
                    warning: format!(
                        "{} node '{}' must be reimplemented as a Voltrus flow script or Lua expression",
                        node.node_type, name
                    ),
                });
            }

            // ── OPC-UA nodes ──
            "opcua-client" | "opcua-item" => {
                warnings.push(format!(
                    "OPC-UA node '{}' detected — configure via Voltrus OPC-UA device manager for full support",
                    name
                ));
                skipped_nodes.push(SkippedNode {
                    node_id: node.id.clone(),
                    node_type: node.node_type.clone(),
                    name: Some(name),
                    reason:
                        "OPC-UA nodes should be configured via the Voltrus OPC-UA device manager"
                            .to_string(),
                });
            }

            // ── S7 nodes ──
            "s7-client" | "s7-in" | "s7-out" => {
                warnings.push(format!(
                    "S7 node '{}' detected — configure via Voltrus S7 device manager for full support",
                    name
                ));
                skipped_nodes.push(SkippedNode {
                    node_id: node.id.clone(),
                    node_type: node.node_type.clone(),
                    name: Some(name),
                    reason: "S7 nodes should be configured via the Voltrus S7 device manager"
                        .to_string(),
                });
            }

            // ── Everything else ──
            _ => {
                if cat == NodeCategory::Unsupported {
                    skipped_nodes.push(SkippedNode {
                        node_id: node.id.clone(),
                        node_type: node.node_type.clone(),
                        name: node.name.clone(),
                        reason: format!("Unsupported node type: {}", node.node_type),
                    });
                }
            }
        }
    }

    _modbus_devices = device_map.into_values().collect();

    ConversionResult {
        modbus_devices: _modbus_devices,
        mqtt_sources,
        widgets,
        flow_nodes,
        function_nodes,
        skipped_nodes,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_nodered::parser::parse_flows;

    #[test]
    fn test_convert_modbus_read() {
        let json = r#"[
            {"id":"n1","type":"modbus-read","name":"Read Voltage","server":"plc1","fc":3,"unitid":1,"address":"40001","quantity":"2","properties":{"host":"192.168.1.10","port":502}}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.modbus_devices.len(), 1);
        let dev = &result.modbus_devices[0];
        assert_eq!(dev.device_id, "plc1");
        assert_eq!(dev.ip, "192.168.1.10");
        assert_eq!(dev.port, 502);
        assert_eq!(dev.unit_id, 1);
        assert_eq!(dev.tags.len(), 1);
        assert_eq!(dev.tags[0].name, "Read Voltage");
        assert_eq!(dev.tags[0].address, 40001);
        assert_eq!(dev.tags[0].register_area, "holding");
    }

    #[test]
    fn test_convert_mqtt_nodes() {
        let json = r#"[
            {"id":"m1","type":"mqtt in","name":"Sensor Sub","topic":"sensor/#","qos":1,"broker":"b1"},
            {"id":"m2","type":"mqtt out","name":"Cmd Pub","topic":"cmd/valve","broker":"b1"}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.mqtt_sources.len(), 2);
        assert_eq!(result.mqtt_sources[0].topic, "sensor/#");
        assert_eq!(result.mqtt_sources[0].direction, "in");
        assert_eq!(result.mqtt_sources[1].direction, "out");
    }

    #[test]
    fn test_convert_ui_widgets() {
        let json = r#"[
            {"id":"u1","type":"ui_gauge","name":"Pressure","label":"PSI","gmin":0,"gmax":100},
            {"id":"u2","type":"ui_chart","name":"Trend"},
            {"id":"u3","type":"ui_text","name":"Status","label":"Running"}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.widgets.len(), 3);
        assert_eq!(result.widgets[0].widget_type, "gauge");
        assert_eq!(result.widgets[0].min_val, Some(0.0));
        assert_eq!(result.widgets[0].max_val, Some(100.0));
        assert_eq!(result.widgets[1].widget_type, "chart");
        assert_eq!(result.widgets[2].widget_type, "text");
    }

    #[test]
    fn test_convert_switch_and_delay() {
        let json = r#"[
            {"id":"s1","type":"switch","name":"Check","property":"payload","pt":"msg","ptt":"number"},
            {"id":"d1","type":"delay","name":"Wait","timeout":"5","timeoutUnits":"seconds"}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.flow_nodes.len(), 2);
        assert_eq!(result.flow_nodes[0].node_type, "switch");
        assert_eq!(result.flow_nodes[1].node_type, "delay");
        // 5 seconds = 5000ms
        assert_eq!(result.flow_nodes[1].config["delay_ms"], 5000);
    }

    #[test]
    fn test_convert_function_node() {
        let json = r#"[
            {"id":"f1","type":"function","name":"Calc","func":"return {payload: msg.payload * 2};"}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.function_nodes.len(), 1);
        assert_eq!(
            result.function_nodes[0].source_code.as_deref(),
            Some("return {payload: msg.payload * 2};")
        );
    }

    #[test]
    fn test_convert_opcua_skipped() {
        let json = r#"[
            {"id":"o1","type":"opcua-item","name":"Temp","endpoint":"opc.tcp://host:4840","item":"ns=2;s=Temp"}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.skipped_nodes.len(), 1);
        assert!(result.skipped_nodes[0].reason.contains("OPC-UA"));
    }

    #[test]
    fn test_convert_merge_modbus_same_server() {
        let json = r#"[
            {"id":"n1","type":"modbus-read","name":"Voltage","server":"plc1","fc":3,"unitid":1,"address":"40001","quantity":"2","properties":{"host":"10.0.0.1"}},
            {"id":"n2","type":"modbus-read","name":"Current","server":"plc1","fc":3,"unitid":1,"address":"40003","quantity":"2","properties":{"host":"10.0.0.1"}}
        ]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.modbus_devices.len(), 1);
        assert_eq!(result.modbus_devices[0].tags.len(), 2);
    }

    #[test]
    fn test_convert_empty_flow() {
        let json = r#"[]"#;
        let parsed = parse_flows(json).unwrap();
        let result = convert(&parsed);

        assert_eq!(result.modbus_devices.len(), 0);
        assert_eq!(result.mqtt_sources.len(), 0);
        assert_eq!(result.widgets.len(), 0);
        assert_eq!(result.flow_nodes.len(), 0);
    }
}
