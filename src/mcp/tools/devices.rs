use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, ToolResult, from_handler, parse_args};
use crate::models::DeviceStatus;

pub fn tools() -> Vec<Tool> {
    #[allow(unused_mut)]
    let mut tools = vec![Tool {
        name: "list_devices",
        title: "List devices",
        description: "List acquisition devices (PLCs, meters) with live connection status. \
            Configured Modbus devices come with their endpoint and register count; devices of \
            other protocols appear while they report data. Pass `id` to get one Modbus device \
            with its full register map. A device's metrics become tag values through tags with \
            source_type \"modbus\", device_id = the device id and key = the register name.",
        access: Access::Read,
        destructive: false,
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Return only this Modbus device, including its registers."}
                },
                "additionalProperties": false
            })
        },
        call: list_devices,
    }];
    #[cfg(feature = "modbus")]
    {
        tools.push(Tool {
            name: "create_modbus_device",
            title: "Create Modbus TCP device",
            description: "Add a Modbus TCP device with its register map and start polling it at \
                once. Each register needs a unique `name` and either `address` (legacy: 0-based \
                offset below 30001, or a 5-digit reference such as 40001) or `offset` (explicit \
                0-based, 0-65535). Registers are not tags: to see a register's value in \
                read_live_values / screens, follow up with create_tag (source_type \"modbus\", \
                device_id = this id, key = register name). Fails if the id already exists.",
            access: Access::Write,
            destructive: false,
            input_schema: modbus::create_schema,
            call: modbus::create,
        });
        tools.push(Tool {
            name: "delete_device",
            title: "Delete Modbus device",
            description: "Delete a Modbus device and its register map, and stop polling it. \
                Tags that referenced the device are kept but stop receiving values. Not \
                reversible; confirm the id with list_devices first.",
            access: Access::Write,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {"id": {"type": "string", "description": "Device id, as listed by list_devices."}},
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
            call: modbus::delete,
        });
    }
    tools
}

fn status_str(status: &DeviceStatus) -> &'static str {
    match status {
        DeviceStatus::Online => "online",
        DeviceStatus::Offline => "offline",
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    id: Option<String>,
}

fn list_devices(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let args: ListArgs = parse_args(args)?;
        if let Some(id) = args.id {
            return one_device(ctx, &id).await;
        }
        all_devices(ctx).await
    })
}

#[cfg(feature = "modbus")]
async fn one_device(ctx: &Ctx, id: &str) -> ToolResult {
    use crate::api::modbus_devices;
    use axum::extract::{Path, State};
    if ctx.state.storage.get_modbus_device(id).await?.is_none() {
        return Err(ToolError::new(format!("device '{id}' not found")));
    }
    let mut out = from_handler(
        modbus_devices::get_modbus_device(State(ctx.state.clone()), Path(id.to_string())).await,
    )
    .await?;
    if let Some(snapshot) = ctx
        .state
        .cache
        .get_all()
        .await
        .into_iter()
        .find(|s| s.device_id == id)
    {
        out["status"] = json!(status_str(&snapshot.status));
        out["last_update"] = json!(snapshot.timestamp);
        let mut metrics: Vec<&String> = snapshot.metrics.keys().collect();
        metrics.sort();
        out["metrics"] = json!(metrics);
    } else {
        out["status"] = json!("unknown");
    }
    Ok(out)
}

#[cfg(not(feature = "modbus"))]
async fn one_device(_ctx: &Ctx, id: &str) -> ToolResult {
    Err(ToolError::new(format!(
        "device '{id}' not found (this build has no Modbus support)"
    )))
}

async fn all_devices(ctx: &Ctx) -> ToolResult {
    let snapshots = ctx.state.cache.get_all().await;
    let mut devices: Vec<Value> = Vec::new();
    let mut configured_ids: Vec<String> = Vec::new();

    #[cfg(feature = "modbus")]
    {
        use crate::api::modbus_devices;
        use axum::extract::State;
        let configured =
            from_handler(modbus_devices::list_modbus_devices(State(ctx.state.clone())).await)
                .await?;
        for mut device in configured.as_array().cloned().unwrap_or_default() {
            let id = device["id"].as_str().unwrap_or_default().to_string();
            device["protocol"] = json!("modbus");
            match snapshots.iter().find(|s| s.device_id == id) {
                Some(s) => {
                    device["status"] = json!(status_str(&s.status));
                    device["last_update"] = json!(s.timestamp);
                    device["metric_count"] = json!(s.metrics.len());
                }
                None => device["status"] = json!("unknown"),
            }
            configured_ids.push(id);
            devices.push(device);
        }
    }

    for s in snapshots
        .iter()
        .filter(|s| !configured_ids.contains(&s.device_id))
    {
        devices.push(json!({
            "id": s.device_id,
            "protocol": s.protocol.as_deref().unwrap_or("modbus"),
            "status": status_str(&s.status),
            "simulated": s.simulated,
            "last_update": s.timestamp,
            "metric_count": s.metrics.len(),
        }));
    }
    Ok(json!({"devices": devices, "count": devices.len()}))
}

#[cfg(feature = "modbus")]
mod modbus {
    use axum::extract::{Extension, Path, State};
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    use super::super::{Ctx, ToolError, ToolFuture, from_handler, parse_args};
    use crate::api::modbus_devices;

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum RegisterType {
        Int16,
        Uint16,
        Int32,
        Float32,
        Float64,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum RegisterArea {
        Holding,
        Input,
        Coil,
        Discrete,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum Endianness {
        Big,
        Little,
        Swapped,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Register {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        address: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        offset: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        zero_based: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        register_type: Option<RegisterType>,
        #[serde(skip_serializing_if = "Option::is_none")]
        register_area: Option<RegisterArea>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scale: Option<f64>,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct CreateArgs {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        ip: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit_id: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        interval_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        endianness: Option<Endianness>,
        #[serde(default)]
        registers: Vec<Register>,
    }

    pub fn create_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "minLength": 1, "description": "Unique device id, e.g. \"plts_1\". Used as the device_id of its tags."},
                "name": {"type": "string", "description": "Display name. Defaults to the id."},
                "ip": {"type": "string", "minLength": 1, "description": "Host or IP address of the Modbus TCP server."},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535, "default": 502},
                "unit_id": {"type": "integer", "minimum": 0, "maximum": 255, "default": 1, "description": "Modbus unit / slave id."},
                "interval_ms": {"type": "integer", "minimum": 1, "default": 1000, "description": "Polling interval in milliseconds."},
                "endianness": {"type": "string", "enum": ["big", "little", "swapped"], "default": "big", "description": "Word/byte order for 32- and 64-bit registers."},
                "registers": {
                    "type": "array",
                    "description": "Register map, polled every interval.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string", "minLength": 1, "description": "Register name; becomes the metric name and the key of a modbus tag."},
                            "address": {"type": "integer", "minimum": 0, "maximum": 65535, "description": "Legacy address: 0-based below 30001, or a 5-digit reference (40001 = holding offset 0). Give address or offset."},
                            "offset": {"type": "integer", "minimum": 0, "maximum": 65535, "description": "Explicit 0-based offset inside register_area. Give address or offset."},
                            "zero_based": {"type": "boolean", "default": false, "description": "With address: treat the value as an already 0-based offset."},
                            "register_type": {"type": "string", "enum": ["int16", "uint16", "int32", "float32", "float64"], "default": "float32"},
                            "register_area": {"type": "string", "enum": ["holding", "input", "coil", "discrete"], "default": "holding"},
                            "scale": {"type": "number", "default": 1.0, "description": "Multiplier applied to the raw value."}
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["id", "ip"],
            "additionalProperties": false
        })
    }

    pub fn create(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            let mut args: CreateArgs = parse_args(args)?;
            let id = args.id.trim().to_string();
            if id.is_empty() {
                return Err(ToolError::new("id must not be empty"));
            }
            if args.ip.trim().is_empty() {
                return Err(ToolError::new("ip must not be empty"));
            }
            if ctx.state.storage.get_modbus_device(&id).await?.is_some() {
                return Err(ToolError::new(format!("device '{id}' already exists")));
            }
            args.name.get_or_insert_with(|| id.clone());
            args.id = id.clone();
            let body = serde_json::to_value(&args)
                .map_err(|e| ToolError::new(format!("invalid arguments: {e}")))?;
            let mut out = from_handler(
                modbus_devices::create_modbus_device(
                    State(ctx.state.clone()),
                    Extension(ctx.user.clone()),
                    axum::Json(body),
                )
                .await,
            )
            .await?;
            out["registers"] = json!(args.registers.len());
            Ok(out)
        })
    }

    pub fn delete(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                id: String,
            }
            let Args { id } = parse_args(args)?;
            if ctx.state.storage.get_modbus_device(&id).await?.is_none() {
                return Err(ToolError::new(format!("device '{id}' not found")));
            }
            from_handler(
                modbus_devices::delete_modbus_device(
                    State(ctx.state.clone()),
                    Extension(ctx.user.clone()),
                    Path(id),
                )
                .await,
            )
            .await
        })
    }
}
