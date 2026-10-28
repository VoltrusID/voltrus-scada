use serde::{Deserialize, Serialize};

// ── Topology ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTopology {
    pub devices: Vec<TopologyDevice>,
    pub connections: Vec<TopologyConnection>,
    pub summary: TopologySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyDevice {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub address: String,
    pub enabled: bool,
    pub tag_count: usize,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyConnection {
    pub from: String, // Voltrus server
    pub to: String,   // Device ID
    pub protocol: String,
    pub address: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologySummary {
    pub total_devices: usize,
    pub total_tags: usize,
    pub protocols: Vec<ProtocolCount>,
    pub online_count: usize,
    pub offline_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolCount {
    pub protocol: String,
    pub device_count: usize,
    pub tag_count: usize,
}

// ── Tag Cross-Reference ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagXrefReport {
    pub total_tags: usize,
    pub tags: Vec<TagXrefEntry>,
    pub by_device: Vec<DeviceTagSummary>,
    pub by_protocol: Vec<ProtocolCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagXrefEntry {
    pub key: String,
    pub name: String,
    pub device_id: String,
    pub device_name: String,
    pub protocol: String,
    pub unit: String,
    pub source_type: String,
    pub data_type: Option<String>,
    pub address: Option<String>,
    pub has_alarm: bool,
    pub alarm_severity: Option<String>,
    pub alarm_threshold: Option<String>,
    pub warn_val: f64,
    pub crit_val: f64,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceTagSummary {
    pub device_id: String,
    pub device_name: String,
    pub protocol: String,
    pub tag_count: usize,
    pub alarmed_count: usize,
}

// ── Script & Flow Documentation ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationDocs {
    pub scripts: Vec<ScriptDoc>,
    pub flows: Vec<FlowDoc>,
    pub summary: AutomationSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptDoc {
    pub id: i64,
    pub name: String,
    pub source_preview: String, // First 500 chars of source
    pub enabled: bool,
    pub syntax_error: Option<String>,
    pub cpu_limit_ms: i64,
    pub memory_limit_bytes: i64,
    pub description: String, // Auto-generated plain-English description
    pub last_execution: Option<ScriptExecutionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptExecutionSummary {
    pub status: String,
    pub duration_ms: i64,
    pub error_message: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDoc {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger_description: String, // Plain-English trigger description
    pub node_count: usize,
    pub node_descriptions: Vec<String>, // Plain-English node descriptions
    pub description: String,            // Overall description
    pub last_run: Option<FlowRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRunSummary {
    pub success: bool,
    pub duration_ms: i64,
    pub error: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationSummary {
    pub total_scripts: usize,
    pub enabled_scripts: usize,
    pub total_flows: usize,
    pub enabled_flows: usize,
}

// ── Plain-English description generators ──

pub fn describe_trigger(trigger_config: &serde_json::Value) -> String {
    let tag = trigger_config["tag"].as_str().unwrap_or("unknown tag");
    let comparison = trigger_config["comparison"].as_str().unwrap_or("changes");
    let threshold = trigger_config["threshold"].as_f64().unwrap_or(0.0);
    let holdoff = trigger_config["holdoff_secs"].as_f64().unwrap_or(0.0);

    let comp_str = match comparison {
        "above" => format!("rises above {}", threshold),
        "below" => format!("drops below {}", threshold),
        "equal" => format!("equals {}", threshold),
        "not_equal" => format!("does not equal {}", threshold),
        "change" => "changes value".to_string(),
        _ => format!("{} {}", comparison, threshold),
    };

    let holdoff_str = if holdoff > 0.0 {
        format!(", held for {} seconds", holdoff as i64)
    } else {
        String::new()
    };

    format!("When '{}' {}{}", tag, comp_str, holdoff_str)
}

pub fn describe_nodes(nodes_json: &serde_json::Value) -> Vec<String> {
    let nodes = match nodes_json.as_array() {
        Some(arr) => arr,
        None => return vec![],
    };

    nodes
        .iter()
        .map(|node| {
            let action = node["action"].as_str().unwrap_or("unknown");
            let msg = node["message"].as_str().unwrap_or("");

            match action {
                "log" => format!("Log: {}", msg),
                "email" => {
                    let subject = node["subject"].as_str().unwrap_or("Alert");
                    format!("Send email: \"{}\" — {}", subject, msg)
                }
                "webhook" => format!("Send webhook: {}", msg),
                "write" => {
                    let tag = node["tag"].as_str().unwrap_or("tag");
                    let val = node["value"].as_f64().unwrap_or(0.0);
                    format!("Write {} to tag '{}'", val, tag)
                }
                _ => format!("{}: {}", action, msg),
            }
        })
        .collect()
}

pub fn describe_script_source(source: &str) -> String {
    if source.is_empty() {
        return "Empty script — no source code.".to_string();
    }

    let lines: Vec<&str> = source.lines().collect();
    let first_line = lines.first().unwrap_or(&"");

    // Check for common patterns
    if source.contains("on_tag_change") {
        return "Event-driven script triggered when a tag value changes. Processes incoming data and may trigger actions based on threshold logic.".to_string();
    } else if source.contains("on_alarm") {
        return "Alarm handler script — executes custom logic when an alarm fires. May perform additional checks or send notifications.".to_string();
    } else if source.contains("on_schedule") || source.contains("setInterval") {
        return "Scheduled script — runs periodically at a defined interval. Typically performs batch calculations or periodic checks.".to_string();
    } else if source.contains("read_tag") && source.contains("write_tag") {
        return "Bidirectional script — reads tag values and writes to other tags based on calculated logic. Implements control or translation logic.".to_string();
    } else if source.contains("read_tag") {
        return "Read-only script — reads tag values and performs calculations or generates derived metrics.".to_string();
    }

    if first_line.starts_with("//") || first_line.starts_with('#') {
        let desc = first_line
            .trim_start_matches('/')
            .trim_start_matches('#')
            .trim();
        if desc.len() > 200 {
            return format!("{}...", &desc[..200]);
        } else {
            return desc.to_string();
        }
    }

    "Custom automation script. Review source code for specific behavior.".to_string()
}

/// Generate full system documentation as HTML for PDF export
pub fn generate_docs_html(
    topology: &SystemTopology,
    tag_xref: &TagXrefReport,
    automation: &AutomationDocs,
    hostname: &str,
    generated_at: &str,
) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 40px; color: #1a1a1a; font-size: 13px; }}
  h1 {{ font-size: 24px; border-bottom: 2px solid #f97316; padding-bottom: 8px; }}
  h2 {{ font-size: 18px; color: #f97316; margin-top: 32px; border-bottom: 1px solid #e5e7eb; padding-bottom: 4px; }}
  h3 {{ font-size: 14px; color: #374151; margin-top: 20px; }}
  table {{ width: 100%; border-collapse: collapse; margin: 12px 0; font-size: 12px; }}
  th {{ background: #f9fafb; text-align: left; padding: 6px 8px; border: 1px solid #e5e7eb; font-weight: 600; }}
  td {{ padding: 5px 8px; border: 1px solid #e5e7eb; }}
  .cover {{ text-align: center; padding: 60px 0; }}
  .cover h1 {{ font-size: 32px; border: none; }}
  .cover .meta {{ color: #6b7280; margin-top: 16px; }}
  .summary-grid {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin: 16px 0; }}
  .summary-card {{ background: #f9fafb; border: 1px solid #e5e7eb; border-radius: 6px; padding: 12px; text-align: center; }}
  .summary-card .num {{ font-size: 24px; font-weight: 700; color: #f97316; }}
  .summary-card .label {{ font-size: 11px; color: #6b7280; text-transform: uppercase; }}
  .badge {{ display: inline-block; padding: 1px 6px; border-radius: 3px; font-size: 10px; font-weight: 600; }}
  .badge-ok {{ background: #dcfce7; color: #16a34a; }}
  .badge-off {{ background: #fef2f2; color: #ef4444; }}
  .page-break {{ page-break-before: always; }}
  code {{ background: #f3f4f6; padding: 1px 4px; border-radius: 3px; font-size: 11px; }}
  .footer {{ margin-top: 40px; padding-top: 12px; border-top: 1px solid #e5e7eb; color: #9ca3af; font-size: 10px; }}
</style>
</head>
<body>
  <div class="cover">
    <h1>Voltrus System Documentation</h1>
    <div class="meta">
      <p><strong>Host:</strong> {hostname}</p>
      <p><strong>Generated:</strong> {generated_at}</p>
      <p><strong>Voltrus SCADA — v0.35.0</strong></p>
    </div>
  </div>

  <h2>System Topology</h2>
  <div class="summary-grid">
    <div class="summary-card"><div class="num">{total_devices}</div><div class="label">Devices</div></div>
    <div class="summary-card"><div class="num">{total_tags}</div><div class="label">Tags</div></div>
    <div class="summary-card"><div class="num">{online_count}</div><div class="label">Online</div></div>
  </div>

  <h3>Protocols</h3>
  <table>
    <tr><th>Protocol</th><th>Devices</th><th>Tags</th></tr>
    {protocol_rows}
  </table>

  <h3>Device Inventory</h3>
  <table>
    <tr><th>Name</th><th>Protocol</th><th>Address</th><th>Tags</th><th>Status</th></tr>
    {device_rows}
  </table>

  <div class="page-break"></div>
  <h2>Tag Cross-Reference</h2>
  <p>Total tags: <strong>{total_tags}</strong></p>
  <table>
    <tr><th>Tag Key</th><th>Name</th><th>Device</th><th>Protocol</th><th>Unit</th><th>Alarm</th><th>Warn</th><th>Crit</th></tr>
    {tag_rows}
  </table>

  <div class="page-break"></div>
  <h2>Automation &amp; Scripts</h2>
  <div class="summary-grid">
    <div class="summary-card"><div class="num">{total_scripts}</div><div class="label">Scripts</div></div>
    <div class="summary-card"><div class="num">{total_flows}</div><div class="label">Flow Rules</div></div>
    <div class="summary-card"><div class="num">{enabled_count}</div><div class="label">Enabled</div></div>
  </div>

  {automation_section}

  <div class="footer">
    <p>Generated by Voltrus SCADA — {generated_at}</p>
    <p>This document is auto-generated and reflects the system configuration at the time of export.</p>
  </div>
</body>
</html>"#,
        hostname = hostname,
        generated_at = generated_at,
        total_devices = topology.summary.total_devices,
        total_tags = tag_xref.total_tags,
        online_count = topology.summary.online_count,
        protocol_rows = topology.summary.protocols.iter().map(|p| format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
            p.protocol, p.device_count, p.tag_count
        )).collect::<Vec<_>>().join(""),
        device_rows = topology.devices.iter().map(|d| format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><span class='badge {}'>{}</span></td></tr>",
            d.name, d.protocol, d.address, d.tag_count,
            if d.online { "badge-ok" } else { "badge-off" },
            if d.online { "Online" } else { "Offline" }
        )).collect::<Vec<_>>().join(""),
        tag_rows = tag_xref.tags.iter().take(500).map(|t| format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            t.key, t.name, t.device_name, t.protocol, t.unit,
            if t.has_alarm { "Yes" } else { "—" },
            t.warn_val, t.crit_val
        )).collect::<Vec<_>>().join(""),
        total_scripts = automation.summary.total_scripts,
        total_flows = automation.summary.total_flows,
        enabled_count = automation.summary.enabled_scripts + automation.summary.enabled_flows,
        automation_section = generate_automation_html(automation),
    )
}

fn generate_automation_html(automation: &AutomationDocs) -> String {
    let mut html = String::new();

    if !automation.flows.is_empty() {
        html.push_str("<h3>Flow Rules</h3><table>");
        html.push_str("<tr><th>Name</th><th>Trigger</th><th>Actions</th><th>Enabled</th></tr>");
        for f in &automation.flows {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                f.name,
                f.trigger_description,
                f.node_descriptions.join("; "),
                if f.enabled { "✓" } else { "—" }
            ));
        }
        html.push_str("</table>");
    }

    if !automation.scripts.is_empty() {
        html.push_str("<h3>Scripts</h3><table>");
        html.push_str("<tr><th>Name</th><th>Description</th><th>Enabled</th><th>Timeout</th></tr>");
        for s in &automation.scripts {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}ms</td></tr>",
                s.name,
                s.description,
                if s.enabled { "✓" } else { "—" },
                s.cpu_limit_ms
            ));
        }
        html.push_str("</table>");
    }

    html
}
