//! Equipment type tools: list and define types (Pump, Valve, Tank, ...),
//! change one with a preview of every instance it updates, and instantiate
//! equipment in one line. They call the `/api/v1/equipment-types` and
//! `/api/v1/equipment-instances` handlers (docs/equipment-types.md).

use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::equipment_types::{self as api, InstanceQuery};
use crate::edition::ActionOrigin;

/// A handler's response as a tool result.
async fn answer(r: axum::response::Response) -> super::ToolResult {
    from_handler(Ok::<_, crate::api::AppError>(r)).await
}

const TYPE_SCHEMA: &str = "An equipment type: {id (slug), name, description, category, symbol \
(symbol key, e.g. isa_pump_centrifugal), faceplate (faceplate id or null), tags: [{role, label, \
kind: state|fault|analog|command|setpoint, key (pattern, default \"{prefix}_{role}\"), name \
(pattern), offset (register offset from the instance base), register_type, unit, min, max, \
decimals, alarm {warn, crit, low_warn, low_crit, deadband, on_delay_secs}, write {writable, min, \
max, max_rate, requires_reason}, required, fallback (role)}], bindings {primary, fault, run, \
manual, interlock} (roles), symbol_config {...}, flows [flow templates]}. Patterns use {prefix}, \
{name}, {device}, {role}, {label}; flow strings also {ref:<role>} and {key:<role>}.";

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_equipment_types",
            title: "List equipment types",
            description: "Equipment types defined on this server (Pump, Valve, Tank, ...): the \
                tags each piece of equipment has (by role and key pattern), their limits and \
                alarms, the faceplate, the mimic symbol and how it binds, with instance_count. \
                `library: true` lists the starter types that ship with the server instead; \
                install them with create_equipment_type from_library.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "library": {"type": "boolean", "description": "List the starter library instead of the stored types."}
                    },
                    "additionalProperties": false
                })
            },
            call: list_types,
        },
        Tool {
            name: "create_equipment_type",
            title: "Create equipment type",
            description: "Define an equipment type once, so each piece of equipment becomes one \
                instantiate_equipment call. Pass `type` (the full definition) or `from_library` \
                (a starter type id from list_equipment_types library: true, e.g. pump; its \
                faceplate is created too when missing). Fails when the id exists.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "type": {"type": "object", "description": TYPE_SCHEMA},
                        "from_library": {"type": "string", "description": "Starter type id to install."}
                    },
                    "additionalProperties": false
                })
            },
            call: create_type,
        },
        Tool {
            name: "update_equipment_type",
            title: "Update equipment type",
            description: "Change an equipment type; every instance follows (its tags get the new \
                limits, alarms and write policy, missing tags and registers are created, its \
                symbols are rebound). By default this is a dry run that lists the affected \
                instances (tags created or updated with the fields, registers, flows, widgets); \
                call again with dry_run: false to save.",
            access: Access::Write,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "type": {"type": "object", "description": TYPE_SCHEMA},
                        "dry_run": {"type": "boolean", "description": "Default true: preview only."}
                    },
                    "required": ["type"],
                    "additionalProperties": false
                })
            },
            call: update_type,
        },
        Tool {
            name: "instantiate_equipment",
            title: "Instantiate equipment",
            description: "Create one piece of equipment from a type in one line, e.g. {name: \
                \"P-101A\", type: \"pump\", device: \"wtp\"}. Without `base` its roles bind to \
                existing tags by key pattern ({prefix} defaults to the name in lower case without \
                punctuation: p101a). With `base` on a Modbus device, missing roles get a \
                register at base + offset and a tag. `overrides` change one role: {fault: {skip: \
                true}}, {speed: {key: \"p301a_speed\"}}, limits or alarms. Answers the tag of each \
                role (resolved.roles), what it creates or changes (changes: every tag with \
                its attributes before and after, registers added or moved, flows created or \
                overwritten) and the symbol binding. It can change existing tags (limits, \
                alarms, write policy) and flows, so by default this is a dry run; call again \
                with dry_run: false to save. A change of a write policy needs an electronic \
                signature where the edition asks for one, which an MCP client cannot give. A \
                SCADA widget with config.equipment = the name is bound to it on save.",
            access: Access::Write,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "minLength": 1, "maxLength": 128},
                        "type": {"type": "string", "description": "Equipment type id."},
                        "device": {"type": "string", "description": "Device id the tags belong to."},
                        "base": {"type": "integer", "minimum": 0, "maximum": 65535, "description": "Register base address (Modbus)."},
                        "prefix": {"type": "string", "description": "Tag key prefix. Default: from the name."},
                        "description": {"type": "string"},
                        "symbol": {"type": "string", "description": "Symbol instead of the type's."},
                        "overrides": {"type": "object", "description": "Per role: {key, name, unit, min, max, decimals, alarm, write, skip}."},
                        "dry_run": {"type": "boolean", "description": "Default true: preview only."}
                    },
                    "required": ["name", "type", "device"],
                    "additionalProperties": false
                })
            },
            call: instantiate,
        },
        Tool {
            name: "list_equipment_instances",
            title: "List equipment instances",
            description: "Equipment instances with their type, device and, per role, the tag it \
                is bound to (resolved.roles: role, key, ref, tag_id, status bound/fallback/\
                missing/skipped). `type` filters by type id.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "type": {"type": "string", "description": "Only instances of this type."}
                    },
                    "additionalProperties": false
                })
            },
            call: list_instances,
        },
    ]
}

fn list_types(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            #[serde(default)]
            library: bool,
        }
        let a: Args = parse_args(args)?;
        let v = if a.library {
            from_handler(Ok::<_, crate::api::AppError>(api::get_library().await)).await?
        } else {
            from_handler(api::list_types(State(ctx.state.clone())).await).await?
        };
        Ok(json!({ if a.library { "library" } else { "equipment_types" }: v }))
    })
}

fn create_type(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            #[serde(rename = "type")]
            body: Option<Value>,
            from_library: Option<String>,
        }
        let a: Args = parse_args(args)?;
        match (a.body, a.from_library) {
            (Some(body), None) => {
                answer(api::create_type_as(&ctx.state, &ctx.user, ActionOrigin::Mcp, body).await)
                    .await
            }
            (None, Some(id)) => {
                match api::install(&ctx.state, &ctx.user, ActionOrigin::Mcp, Some(vec![id])).await {
                    Ok(v) => Ok(v),
                    Err(r) => answer(*r).await,
                }
            }
            _ => Err(ToolError::new(
                "give exactly one of `type` and `from_library`",
            )),
        }
    })
}

fn update_type(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            #[serde(rename = "type")]
            body: Value,
            dry_run: Option<bool>,
        }
        let a: Args = parse_args(args)?;
        let id = a
            .body
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::new("type.id is required"))?
            .to_string();
        answer(
            api::update_type_as(
                &ctx.state,
                &ctx.user,
                ActionOrigin::Mcp,
                &id,
                a.dry_run.unwrap_or(true),
                a.body,
            )
            .await,
        )
        .await
    })
}

fn instantiate(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let mut body = args;
        let dry_run = body
            .as_object_mut()
            .and_then(|o| o.remove("dry_run"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        answer(
            api::create_instance_as(&ctx.state, &ctx.user, ActionOrigin::Mcp, dry_run, body).await,
        )
        .await
    })
}

fn list_instances(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            #[serde(rename = "type")]
            type_id: Option<String>,
        }
        let a: Args = parse_args(args)?;
        let v = from_handler(
            api::list_instances(
                State(ctx.state.clone()),
                Query(InstanceQuery { type_id: a.type_id }),
            )
            .await,
        )
        .await?;
        Ok(json!({"equipment_instances": v}))
    })
}
