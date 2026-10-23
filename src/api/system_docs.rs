use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::api::{AppError, AppState, AuthUser};
use crate::system_docs::*;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>, // "pdf" or "html" (default)
}

// ── Handlers ──

#[utoipa::path(
    get,
    path = "/api/v1/docs/topology",
    responses((status = 200, description = "System topology diagram data")),
    tag = "System Docs",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_topology(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let devices = state.storage.list_devices().await.map_err(AppError)?;
    let data_sources = state
        .storage
        .list_data_sources(None)
        .await
        .map_err(AppError)?;

    // Gather protocol devices
    #[cfg(feature = "modbus")]
    let modbus_devs = state
        .storage
        .list_modbus_devices()
        .await
        .unwrap_or_default();
    #[cfg(feature = "opcua")]
    let opcua_devs = state.storage.list_opcua_devices().await.unwrap_or_default();
    #[cfg(feature = "s7")]
    let s7_devs = state.storage.list_s7_devices().await.unwrap_or_default();
    #[cfg(feature = "ethernet-ip")]
    let enip_devs = state.storage.list_enip_devices().await.unwrap_or_default();
    #[cfg(feature = "dnp3")]
    let dnp3_devs = state.storage.list_dnp3_devices().await.unwrap_or_default();
    #[cfg(feature = "bacnet")]
    let bacnet_devs = state
        .storage
        .list_bacnet_devices()
        .await
        .unwrap_or_default();

    // Map device_id → (protocol, address, enabled)
    let mut proto_map: std::collections::HashMap<String, (String, String, u16, bool)> =
        std::collections::HashMap::new();

    #[cfg(feature = "modbus")]
    for d in &modbus_devs {
        proto_map.insert(
            d.id.clone(),
            (
                "Modbus TCP".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }
    #[cfg(feature = "opcua")]
    for d in &opcua_devs {
        proto_map.insert(
            format!("opcua_{}", d.id),
            ("OPC-UA".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "s7")]
    for d in &s7_devs {
        proto_map.insert(
            format!("s7_{}", d.id),
            ("Siemens S7".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "ethernet-ip")]
    for d in &enip_devs {
        proto_map.insert(
            format!("enip_{}", d.id),
            ("EtherNet/IP".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "dnp3")]
    for d in &dnp3_devs {
        proto_map.insert(
            format!("dnp3_{}", d.id),
            (
                "DNP3".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }
    #[cfg(feature = "bacnet")]
    for d in &bacnet_devs {
        proto_map.insert(
            format!("bacnet_{}", d.id),
            (
                "BACnet/IP".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }

    // Count tags per device
    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ds in &data_sources {
        *tag_counts.entry(ds.device_id.clone()).or_insert(0) += 1;
    }

    // Build topology devices
    let mut topology_devices = Vec::new();
    let mut connections = Vec::new();

    for dev in &devices {
        let (protocol, address, port, enabled) =
            proto_map.get(&dev.id).cloned().unwrap_or_else(|| {
                // Generic device — infer protocol from data source source_type
                let ds_for_dev: Vec<_> = data_sources
                    .iter()
                    .filter(|ds| ds.device_id == dev.id)
                    .collect();
                let st = ds_for_dev
                    .first()
                    .map(|ds| ds.source_type.as_str())
                    .unwrap_or("unknown");
                let proto_name = match st {
                    "modbus" => "Modbus TCP",
                    "opcua" => "OPC-UA",
                    "s7" => "Siemens S7",
                    "enip" | "ethernet_ip" => "EtherNet/IP",
                    "dnp3" => "DNP3",
                    "bacnet" => "BACnet/IP",
                    "mqtt" => "MQTT",
                    "random_walk" | "simulated" => "Simulation",
                    _ => st,
                };
                (proto_name.to_string(), String::new(), 0u16, true)
            });

        let online = state.cache.is_device_online(&dev.id).await;

        topology_devices.push(TopologyDevice {
            id: dev.id.clone(),
            name: dev.name.clone(),
            protocol: protocol.clone(),
            address: address.clone(),
            enabled,
            tag_count: *tag_counts.get(&dev.id).unwrap_or(&0),
            online,
        });

        if !address.is_empty() {
            connections.push(TopologyConnection {
                from: "voltrus".to_string(),
                to: dev.id.clone(),
                protocol: protocol.clone(),
                address: address.clone(),
                port,
            });
        }
    }

    // Build summary
    let mut protocol_counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for td in &topology_devices {
        let entry = protocol_counts.entry(td.protocol.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += td.tag_count;
    }

    let online_count = topology_devices.iter().filter(|d| d.online).count();
    let offline_count = topology_devices.len() - online_count;

    let total_tags: usize = topology_devices.iter().map(|d| d.tag_count).sum();

    Ok(Json(SystemTopology {
        devices: topology_devices,
        connections,
        summary: TopologySummary {
            total_devices: devices.len(),
            total_tags,
            protocols: protocol_counts
                .into_iter()
                .map(|(proto, (devs, tags))| ProtocolCount {
                    protocol: proto,
                    device_count: devs,
                    tag_count: tags,
                })
                .collect(),
            online_count,
            offline_count,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/tag-xref",
    responses((status = 200, description = "Tag cross-reference report")),
    tag = "System Docs",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_tag_xref(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let devices = state.storage.list_devices().await.map_err(AppError)?;
    let data_sources = state
        .storage
        .list_data_sources(None)
        .await
        .map_err(AppError)?;
    let alarms = state
        .storage
        .list_alarms(None, 10000)
        .await
        .unwrap_or_default();

    // Device lookup
    let device_map: std::collections::HashMap<String, String> = devices
        .iter()
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect();

    // Protocol lookup per data source
    #[cfg(feature = "modbus")]
    let modbus_devs = state
        .storage
        .list_modbus_devices()
        .await
        .unwrap_or_default();
    #[cfg(feature = "opcua")]
    let opcua_devs = state.storage.list_opcua_devices().await.unwrap_or_default();
    #[cfg(feature = "s7")]
    let s7_devs = state.storage.list_s7_devices().await.unwrap_or_default();
    #[cfg(feature = "ethernet-ip")]
    let enip_devs = state.storage.list_enip_devices().await.unwrap_or_default();
    #[cfg(feature = "dnp3")]
    let dnp3_devs = state.storage.list_dnp3_devices().await.unwrap_or_default();
    #[cfg(feature = "bacnet")]
    let bacnet_devs = state
        .storage
        .list_bacnet_devices()
        .await
        .unwrap_or_default();

    let mut proto_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    #[cfg(feature = "modbus")]
    for d in &modbus_devs {
        proto_map.insert(d.id.clone(), "Modbus TCP".into());
    }
    #[cfg(feature = "opcua")]
    for d in &opcua_devs {
        proto_map.insert(format!("opcua_{}", d.id), "OPC-UA".into());
    }
    #[cfg(feature = "s7")]
    for d in &s7_devs {
        proto_map.insert(format!("s7_{}", d.id), "Siemens S7".into());
    }
    #[cfg(feature = "ethernet-ip")]
    for d in &enip_devs {
        proto_map.insert(format!("enip_{}", d.id), "EtherNet/IP".into());
    }
    #[cfg(feature = "dnp3")]
    for d in &dnp3_devs {
        proto_map.insert(format!("dnp3_{}", d.id), "DNP3".into());
    }
    #[cfg(feature = "bacnet")]
    for d in &bacnet_devs {
        proto_map.insert(format!("bacnet_{}", d.id), "BACnet/IP".into());
    }

    // Alarm lookup by source_key
    let alarm_map: std::collections::HashMap<String, &crate::models::Alarm> =
        alarms.iter().map(|a| (a.source_key.clone(), a)).collect();

    // Build tag entries
    let mut tags = Vec::new();
    for ds in &data_sources {
        let protocol = proto_map.get(&ds.device_id).cloned().unwrap_or_else(|| {
            match ds.source_type.as_str() {
                "modbus" => "Modbus TCP".into(),
                "opcua" => "OPC-UA".into(),
                "s7" => "Siemens S7".into(),
                "enip" | "ethernet_ip" => "EtherNet/IP".into(),
                "dnp3" => "DNP3".into(),
                "bacnet" => "BACnet/IP".into(),
                "mqtt" => "MQTT".into(),
                "random_walk" | "simulated" => "Simulation".into(),
                o => o.to_string(),
            }
        });

        let alarm_info = alarm_map.get(&ds.key);

        // Parse source_config for data type / address
        let config: serde_json::Value = serde_json::from_str(&ds.source_config).unwrap_or_default();
        let data_type = config["data_type"]
            .as_str()
            .or_else(|| config["register_type"].as_str())
            .map(String::from);
        let address = config["address"]
            .as_i64()
            .map(|a| a.to_string())
            .or_else(|| config["node_id"].as_str().map(String::from))
            .or_else(|| config["tag_address"].as_str().map(String::from))
            .or_else(|| config["tag_name"].as_str().map(String::from));

        tags.push(TagXrefEntry {
            key: ds.key.clone(),
            name: ds.name.clone(),
            device_id: ds.device_id.clone(),
            device_name: device_map.get(&ds.device_id).cloned().unwrap_or_default(),
            protocol,
            unit: ds.unit.clone(),
            source_type: ds.source_type.clone(),
            data_type,
            address,
            has_alarm: alarm_info.is_some(),
            alarm_severity: alarm_info.map(|a| a.severity.clone()),
            alarm_threshold: alarm_info.map(|a| format!("{} {}", a.message, a.threshold)),
            warn_val: ds.warn_val,
            crit_val: ds.crit_val,
            path: ds.path.clone(),
        });
    }

    // Device summaries
    let mut device_summaries: std::collections::HashMap<String, (String, String, usize, usize)> =
        std::collections::HashMap::new();
    for t in &tags {
        let entry = device_summaries.entry(t.device_id.clone()).or_insert((
            t.device_name.clone(),
            t.protocol.clone(),
            0,
            0,
        ));
        entry.2 += 1;
        if t.has_alarm {
            entry.3 += 1;
        }
    }

    // Protocol counts
    let mut proto_counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for t in &tags {
        let entry = proto_counts.entry(t.protocol.clone()).or_insert((0, 0));
        entry.0 += 1;
    }

    Ok(Json(TagXrefReport {
        total_tags: tags.len(),
        tags,
        by_device: device_summaries
            .into_iter()
            .map(|(id, (name, proto, count, alarmed))| DeviceTagSummary {
                device_id: id,
                device_name: name,
                protocol: proto,
                tag_count: count,
                alarmed_count: alarmed,
            })
            .collect(),
        by_protocol: proto_counts
            .into_iter()
            .map(|(proto, (tags, _))| ProtocolCount {
                protocol: proto,
                device_count: 0,
                tag_count: tags,
            })
            .collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/automation",
    responses((status = 200, description = "Script and flow documentation")),
    tag = "System Docs",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_automation_docs(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let scripts = state.storage.list_scripts().await.map_err(AppError)?;
    let flows = state.storage.list_flows().await.map_err(AppError)?;

    // Build script docs
    let mut script_docs = Vec::new();
    for s in &scripts {
        let source_preview = if s.source.len() > 500 {
            format!("{}...", &s.source[..500])
        } else {
            s.source.clone()
        };

        let description = describe_script_source(&s.source);

        // Get last execution (we'd need storage access — use None for now)
        script_docs.push(ScriptDoc {
            id: s.id,
            name: s.name.clone(),
            source_preview,
            enabled: s.enabled,
            syntax_error: s.syntax_error.clone(),
            cpu_limit_ms: s.cpu_limit_ms,
            memory_limit_bytes: s.memory_limit_bytes,
            description,
            last_execution: None,
        });
    }

    // Build flow docs
    let mut flow_docs = Vec::new();
    for f in &flows {
        let trigger_config: serde_json::Value =
            serde_json::from_str(&f.trigger_config).unwrap_or_default();
        let nodes: serde_json::Value = serde_json::from_str(&f.nodes).unwrap_or_default();

        let trigger_type = trigger_config["type"].as_str().unwrap_or("tag_threshold");
        let trigger_description = match trigger_type {
            "tag_threshold" => describe_trigger(&trigger_config),
            "schedule" => {
                let cron = trigger_config["cron"].as_str().unwrap_or("unknown");
                format!("Scheduled: {}", cron)
            }
            "alarm" => {
                let severity = trigger_config["severity"].as_str().unwrap_or("any");
                format!("When {} alarm fires", severity)
            }
            _ => format!("Custom trigger: {}", trigger_type),
        };

        let node_descriptions = describe_nodes(&nodes);
        let node_count = nodes.as_array().map(|a| a.len()).unwrap_or(0);

        let description = format!(
            "Flow '{}' — {}. Actions: {}",
            f.name,
            trigger_description,
            if node_descriptions.is_empty() {
                "none defined".to_string()
            } else {
                node_descriptions.join(", ")
            }
        );

        flow_docs.push(FlowDoc {
            id: f.id.clone(),
            name: f.name.clone(),
            enabled: f.enabled,
            trigger_description,
            node_count,
            node_descriptions,
            description,
            last_run: None,
        });
    }

    let total_scripts = script_docs.len();
    let enabled_scripts = script_docs.iter().filter(|s| s.enabled).count();
    let total_flows = flow_docs.len();
    let enabled_flows = flow_docs.iter().filter(|f| f.enabled).count();

    Ok(Json(AutomationDocs {
        scripts: script_docs,
        flows: flow_docs,
        summary: AutomationSummary {
            total_scripts,
            enabled_scripts,
            total_flows,
            enabled_flows,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/export",
    responses((status = 200, description = "Export system documentation as PDF or HTML")),
    tag = "System Docs",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn export_docs(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Query(_query): Query<ExportQuery>,
) -> Result<impl IntoResponse, AppError> {
    // Gather all data by calling internal logic
    let topology = build_topology(&state).await?;
    let tag_xref = build_tag_xref(&state).await?;
    let automation = build_automation_docs(&state).await?;

    let hostname = System::get_hostname();
    let generated_at = chrono_now_string();

    let html = generate_docs_html(&topology, &tag_xref, &automation, &hostname, &generated_at);

    {
        let print_html = format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Voltrus System Docs</title>\
            <style>body{{font-family:system-ui,sans-serif;margin:40px;max-width:1000px;color:#111}}\
            table{{border-collapse:collapse;width:100%;margin:12px 0}}th,td{{border:1px solid #ccc;padding:6px 10px;text-align:left;font-size:13px}}\
            th{{background:#f5f5f5;font-size:11px;text-transform:uppercase}}\
            h1{{font-size:20px}}h2{{font-size:16px;border-bottom:1px solid #ddd;padding-bottom:4px}}h3{{font-size:14px}}\
            .meta{{color:#666;font-size:12px}}@media print{{body{{margin:0}}}}</style>\
            </head><body>{}</body></html>",
            html
        );
        Ok(axum::response::Response::builder()
            .status(200)
            .header("content-type", "text/html")
            .header(
                "content-disposition",
                "attachment; filename=\"voltrus-system-docs.html\"",
            )
            .body(axum::body::Body::from(print_html))
            .unwrap())
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/config-diff",
    responses((status = 200, description = "Config diff between two snapshots")),
    tag = "System Docs",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_config_diff(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Query(query): Query<ConfigDiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let from = state
        .storage
        .get_config_snapshot(query.from)
        .await
        .map_err(AppError)?
        .ok_or_else(|| AppError(anyhow::anyhow!("Snapshot {} not found", query.from)))?;
    let to = state
        .storage
        .get_config_snapshot(query.to)
        .await
        .map_err(AppError)?
        .ok_or_else(|| AppError(anyhow::anyhow!("Snapshot {} not found", query.to)))?;

    let from_data: serde_json::Value =
        serde_json::from_str(&from.snapshot_data).unwrap_or_default();
    let to_data: serde_json::Value = serde_json::from_str(&to.snapshot_data).unwrap_or_default();

    // Use the existing diff logic from config_snapshots
    let diff = crate::api::config_snapshots::compute_diff_public(&from_data, &to_data);

    Ok(Json(serde_json::json!({
        "from": { "id": from.id, "label": from.label, "created_at": from.created_at },
        "to": { "id": to.id, "label": to.label, "created_at": to.created_at },
        "diff": diff,
    })))
}

#[derive(Deserialize)]
pub struct ConfigDiffQuery {
    pub from: i64,
    pub to: i64,
}

// ── Helpers ──

struct System;
impl System {
    fn get_hostname() -> String {
        std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "voltrus-server".to_string())
    }
}

fn chrono_now_string() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

// ── Internal data builders (shared with export) ──

async fn build_topology(state: &AppState) -> Result<SystemTopology, AppError> {
    let devices = state.storage.list_devices().await.map_err(AppError)?;
    let data_sources = state
        .storage
        .list_data_sources(None)
        .await
        .map_err(AppError)?;

    #[cfg(feature = "modbus")]
    let modbus_devs = state
        .storage
        .list_modbus_devices()
        .await
        .unwrap_or_default();
    #[cfg(feature = "opcua")]
    let opcua_devs = state.storage.list_opcua_devices().await.unwrap_or_default();
    #[cfg(feature = "s7")]
    let s7_devs = state.storage.list_s7_devices().await.unwrap_or_default();
    #[cfg(feature = "ethernet-ip")]
    let enip_devs = state.storage.list_enip_devices().await.unwrap_or_default();
    #[cfg(feature = "dnp3")]
    let dnp3_devs = state.storage.list_dnp3_devices().await.unwrap_or_default();
    #[cfg(feature = "bacnet")]
    let bacnet_devs = state
        .storage
        .list_bacnet_devices()
        .await
        .unwrap_or_default();

    let mut proto_map: std::collections::HashMap<String, (String, String, u16, bool)> =
        std::collections::HashMap::new();
    #[cfg(feature = "modbus")]
    for d in &modbus_devs {
        proto_map.insert(
            d.id.clone(),
            (
                "Modbus TCP".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }
    #[cfg(feature = "opcua")]
    for d in &opcua_devs {
        proto_map.insert(
            format!("opcua_{}", d.id),
            ("OPC-UA".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "s7")]
    for d in &s7_devs {
        proto_map.insert(
            format!("s7_{}", d.id),
            ("Siemens S7".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "ethernet-ip")]
    for d in &enip_devs {
        proto_map.insert(
            format!("enip_{}", d.id),
            ("EtherNet/IP".into(), d.url.clone(), 0, d.enabled),
        );
    }
    #[cfg(feature = "dnp3")]
    for d in &dnp3_devs {
        proto_map.insert(
            format!("dnp3_{}", d.id),
            (
                "DNP3".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }
    #[cfg(feature = "bacnet")]
    for d in &bacnet_devs {
        proto_map.insert(
            format!("bacnet_{}", d.id),
            (
                "BACnet/IP".into(),
                format!("{}:{}", d.ip, d.port),
                d.port,
                d.enabled,
            ),
        );
    }

    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for ds in &data_sources {
        *tag_counts.entry(ds.device_id.clone()).or_insert(0) += 1;
    }

    let mut topology_devices = Vec::new();
    let mut connections = Vec::new();

    for dev in &devices {
        let (protocol, address, port, enabled) = proto_map.remove(&dev.id).unwrap_or_else(|| {
            let ds_for_dev: Vec<_> = data_sources
                .iter()
                .filter(|ds| ds.device_id == dev.id)
                .collect();
            let st = ds_for_dev
                .first()
                .map(|ds| ds.source_type.as_str())
                .unwrap_or("unknown");
            let proto_name = match st {
                "modbus" => "Modbus TCP",
                "opcua" => "OPC-UA",
                "s7" => "Siemens S7",
                "enip" | "ethernet_ip" => "EtherNet/IP",
                "dnp3" => "DNP3",
                "bacnet" => "BACnet/IP",
                "mqtt" => "MQTT",
                "random_walk" | "simulated" => "Simulation",
                o => o,
            };
            (proto_name.to_string(), String::new(), 0u16, true)
        });

        let online = state.cache.is_device_online(&dev.id).await;

        topology_devices.push(TopologyDevice {
            id: dev.id.clone(),
            name: dev.name.clone(),
            protocol: protocol.clone(),
            address: address.clone(),
            enabled,
            tag_count: *tag_counts.get(&dev.id).unwrap_or(&0),
            online,
        });

        if !address.is_empty() {
            connections.push(TopologyConnection {
                from: "voltrus".to_string(),
                to: dev.id.clone(),
                protocol,
                address,
                port,
            });
        }
    }

    let mut protocol_counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for td in &topology_devices {
        let entry = protocol_counts.entry(td.protocol.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += td.tag_count;
    }

    let online_count = topology_devices.iter().filter(|d| d.online).count();
    let total_tags: usize = topology_devices.iter().map(|d| d.tag_count).sum();

    Ok(SystemTopology {
        devices: topology_devices,
        connections,
        summary: TopologySummary {
            total_devices: devices.len(),
            total_tags,
            protocols: protocol_counts
                .into_iter()
                .map(|(p, (d, t))| ProtocolCount {
                    protocol: p,
                    device_count: d,
                    tag_count: t,
                })
                .collect(),
            online_count,
            offline_count: devices.len() - online_count,
        },
    })
}

async fn build_tag_xref(state: &AppState) -> Result<TagXrefReport, AppError> {
    // Delegate to simplified version for export
    let devices = state.storage.list_devices().await.map_err(AppError)?;
    let data_sources = state
        .storage
        .list_data_sources(None)
        .await
        .map_err(AppError)?;
    let device_map: std::collections::HashMap<String, String> = devices
        .iter()
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect();

    let tags: Vec<TagXrefEntry> = data_sources
        .iter()
        .map(|ds| TagXrefEntry {
            key: ds.key.clone(),
            name: ds.name.clone(),
            device_id: ds.device_id.clone(),
            device_name: device_map.get(&ds.device_id).cloned().unwrap_or_default(),
            protocol: ds.source_type.clone(),
            unit: ds.unit.clone(),
            source_type: ds.source_type.clone(),
            data_type: None,
            address: None,
            has_alarm: false,
            alarm_severity: None,
            alarm_threshold: None,
            warn_val: ds.warn_val,
            crit_val: ds.crit_val,
            path: ds.path.clone(),
        })
        .collect();

    Ok(TagXrefReport {
        total_tags: tags.len(),
        tags,
        by_device: vec![],
        by_protocol: vec![],
    })
}

async fn build_automation_docs(state: &AppState) -> Result<AutomationDocs, AppError> {
    let scripts = state.storage.list_scripts().await.map_err(AppError)?;
    let flows = state.storage.list_flows().await.map_err(AppError)?;

    let script_docs: Vec<ScriptDoc> = scripts
        .iter()
        .map(|s| ScriptDoc {
            id: s.id,
            name: s.name.clone(),
            source_preview: if s.source.len() > 500 {
                format!("{}...", &s.source[..500])
            } else {
                s.source.clone()
            },
            enabled: s.enabled,
            syntax_error: s.syntax_error.clone(),
            cpu_limit_ms: s.cpu_limit_ms,
            memory_limit_bytes: s.memory_limit_bytes,
            description: describe_script_source(&s.source),
            last_execution: None,
        })
        .collect();

    let flow_docs: Vec<FlowDoc> = flows
        .iter()
        .map(|f| {
            let tc: serde_json::Value = serde_json::from_str(&f.trigger_config).unwrap_or_default();
            let nodes: serde_json::Value = serde_json::from_str(&f.nodes).unwrap_or_default();
            let trigger_description = describe_trigger(&tc);
            let node_descriptions = describe_nodes(&nodes);
            FlowDoc {
                id: f.id.clone(),
                name: f.name.clone(),
                enabled: f.enabled,
                trigger_description: trigger_description.clone(),
                node_count: nodes.as_array().map(|a| a.len()).unwrap_or(0),
                node_descriptions: node_descriptions.clone(),
                description: format!(
                    "{} — Actions: {}",
                    trigger_description,
                    node_descriptions.join(", ")
                ),
                last_run: None,
            }
        })
        .collect();

    let total_scripts = script_docs.len();
    let enabled_scripts = script_docs.iter().filter(|s| s.enabled).count();
    let total_flows = flow_docs.len();
    let enabled_flows = flow_docs.iter().filter(|f| f.enabled).count();

    Ok(AutomationDocs {
        scripts: script_docs,
        flows: flow_docs,
        summary: AutomationSummary {
            total_scripts,
            enabled_scripts,
            total_flows,
            enabled_flows,
        },
    })
}
