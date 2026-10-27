use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::commands;
use crate::models::WriteRequest;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "write_tag",
            title: "Write value to a tag",
            description: "Send a setpoint / command value to the device behind a tag. This \
                changes a real process: write only on an explicit instruction, and state the \
                tag and value back. The request goes through the same write policy as the REST \
                API and is refused when the tag is not writable (the default), the value is \
                outside write_min/write_max, inside the deadband of the current value, over the \
                write rate limit, or the policy demands a `reason` and none is given. On \
                success the command is queued (status \"pending\") and a command_id returned; \
                poll get_command_status for the outcome.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "tag_id": {"type": "integer", "description": "Tag id from list_tags."},
                        "value": {"type": "number", "description": "Numeric value to write."},
                        "reason": {"type": "string", "description": "Why the write is made; recorded in the command audit and required by some tags."},
                        "policy": {"type": "string", "enum": ["immediate", "scheduled"], "default": "immediate"},
                        "scheduled_at": {"type": "integer", "description": "Unix seconds; with policy \"scheduled\", the command is held until then."}
                    },
                    "required": ["tag_id", "value"],
                    "additionalProperties": false
                })
            },
            call: write_tag,
        },
        Tool {
            name: "get_command_status",
            title: "Get command status",
            description: "Status of a command queued by write_tag: pending, queued, sent, \
                acknowledged, failed, expired or cancelled, with retry count, error message and \
                read-back result when the device was read back.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {"command_id": {"type": "string", "description": "command_id returned by write_tag."}},
                    "required": ["command_id"],
                    "additionalProperties": false
                })
            },
            call: get_command_status,
        },
    ]
}

fn write_tag(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            tag_id: i64,
            value: f64,
            reason: Option<String>,
            policy: Option<String>,
            scheduled_at: Option<i64>,
        }
        let args: Args = parse_args(args)?;
        if !args.value.is_finite() {
            return Err(ToolError::new("value must be a finite number"));
        }
        let policy = args.policy.unwrap_or_else(|| "immediate".to_string());
        if !matches!(policy.as_str(), "immediate" | "scheduled") {
            return Err(ToolError::new("policy must be immediate or scheduled"));
        }
        if policy == "scheduled" && args.scheduled_at.is_none() {
            return Err(ToolError::new("policy \"scheduled\" needs scheduled_at"));
        }
        if ctx
            .state
            .storage
            .get_data_source_by_id(args.tag_id)
            .await?
            .is_none()
        {
            return Err(ToolError::new(format!("tag {} not found", args.tag_id)));
        }
        let request = WriteRequest {
            value: json!(args.value),
            policy,
            scheduled_at: args.scheduled_at,
            reason: args.reason.filter(|r| !r.trim().is_empty()),
        };
        from_handler(
            commands::write_tag(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                Path(args.tag_id),
                axum::Json(request),
            )
            .await,
        )
        .await
    })
}

fn get_command_status(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            command_id: String,
        }
        let Args { command_id } = parse_args(args)?;
        let command = from_handler(
            commands::get_command_status(State(ctx.state.clone()), Path(command_id)).await,
        )
        .await?;
        Ok(json!({"command": command}))
    })
}
