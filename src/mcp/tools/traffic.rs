//! `get_device_traffic`: a device's recent protocol frames for an agent
//! troubleshooting communication (docs/traffic.md).

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, parse_args};
use crate::api::traffic::{self, TrafficQuery};

pub fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "get_device_traffic",
        title: "Device protocol traffic",
        description: "Recent Modbus TCP requests and responses between Voltrus and one device, \
            decoded like Wireshark (field ids are Wireshark filter names, e.g. modbus.func_code, \
            modbus.exception_code), each response paired with its request (pair_id, latency_us). \
            Errors carry a plain-language explanation and likely cause: Modbus exceptions (02 = \
            address outside the device map, often a base-0/base-1 offset), malformed frames and \
            unanswered requests. tag_refs names the tags a frame read or wrote. Use it when a tag \
            reads bad or zero, a device goes offline or a write is refused. The first call starts \
            a 10-minute capture on that device (nothing is recorded before), so call again after a \
            few poll intervals. `errors: true` lists only problems; `detail: true` adds the field \
            tree and raw hex. For roles that configure devices (admin, integration keys).",
        access: Access::Read,
        destructive: false,
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "device_id": {"type": "string", "description": "Modbus device id (from list_devices)."},
                    "function": {"type": "integer", "minimum": 1, "maximum": 127, "description": "Only this function code, e.g. 3 read holding registers."},
                    "errors": {"type": "boolean", "default": false, "description": "Only frames with an error."},
                    "addr_from": {"type": "integer", "minimum": 0, "maximum": 65535, "description": "Only frames touching addresses from here (0-based wire address)."},
                    "addr_to": {"type": "integer", "minimum": 0, "maximum": 65535},
                    "space": {"type": "string", "enum": ["coil", "discrete", "input", "holding"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50, "description": "Newest frames to return."},
                    "detail": {"type": "boolean", "default": false, "description": "Include each frame's field tree and raw bytes."}
                },
                "required": ["device_id"],
                "additionalProperties": false
            })
        },
        call: get_device_traffic,
    }]
}

fn get_device_traffic(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            device_id: String,
            function: Option<u16>,
            #[serde(default)]
            errors: bool,
            addr_from: Option<u32>,
            addr_to: Option<u32>,
            space: Option<String>,
            limit: Option<usize>,
            #[serde(default)]
            detail: bool,
        }
        // Traffic carries plant data such as setpoints: the configuring
        // roles may read it, as on the REST route.
        if !crate::api::role_allows_configuration(&ctx.user.role) {
            return Err(ToolError::new(
                "device traffic is for roles that configure devices (admin, integration keys)",
            ));
        }
        let a: Args = parse_args(args)?;
        let q = TrafficQuery {
            function: a.function,
            errors: a.errors,
            addr_from: a.addr_from,
            addr_to: a.addr_to,
            space: a.space,
            after: None,
            limit: Some(a.limit.unwrap_or(50).clamp(1, 500)),
            detail: Some(a.detail),
            watch: Some(true),
        };
        Ok(traffic::device_traffic(&ctx.state, &a.device_id, q).await?)
    })
}
