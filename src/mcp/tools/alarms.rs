use axum::extract::{Extension, Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, into_object, parse_args};
use crate::api::alarms::{self, AlarmListQuery};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_alarms",
            title: "List alarms",
            description: "List alarms, newest first. `status` narrows by lifecycle: \"active\" = \
                raised and still waiting for an operator; \"acknowledged\" = acknowledged by an \
                operator (it may since have cleared; resolved_at is set then); \"shelved\" = \
                temporarily suppressed; \"all\" (default) = everything. Optional `priority` \
                (critical, warning, info) narrows further. Each alarm has id, source_key, \
                source_name, severity, value, threshold, message, triggered_at.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "status": {"type": "string", "enum": ["active", "acknowledged", "shelved", "all"], "default": "all"},
                        "priority": {"type": "string", "enum": ["critical", "warning", "info"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50}
                    },
                    "additionalProperties": false
                })
            },
            call: list_alarms,
        },
        Tool {
            name: "acknowledge_alarm",
            title: "Acknowledge alarm",
            description: "Acknowledge one alarm by id (from list_alarms), recording who did it \
                and starting the alarm cooldown. Operators watching the SCADA see the change at \
                once. Only acknowledge an alarm you have reviewed.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {"id": {"type": "integer", "description": "Alarm id from list_alarms."}},
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
            call: acknowledge_alarm,
        },
        Tool {
            name: "alarm_summary",
            title: "Alarm summary",
            description: "Counts of active critical, warning and info alarms, shelved alarms, \
                alarms raised in the last 24 hours and the average time operators take to \
                acknowledge.",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: alarm_summary,
        },
    ]
}

fn list_alarms(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            status: Option<String>,
            priority: Option<String>,
            limit: Option<i64>,
        }
        let args: Args = parse_args(args)?;
        if let Some(status) = args.status.as_deref()
            && !matches!(status, "active" | "acknowledged" | "shelved" | "all")
        {
            return Err(ToolError::new(
                "status must be active, acknowledged, shelved or all",
            ));
        }
        let query = AlarmListQuery {
            status: args.status,
            priority: args.priority,
            state: None,
            shelved: None,
            limit: Some(args.limit.unwrap_or(50).clamp(1, 500)),
        };
        let out =
            from_handler(alarms::list_alarms(State(ctx.state.clone()), Query(query)).await).await?;
        let list = out.as_array().cloned().unwrap_or_default();
        Ok(json!({"alarms": list, "count": list.len()}))
    })
}

fn acknowledge_alarm(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            id: i64,
        }
        let Args { id } = parse_args(args)?;
        if !ctx.state.storage.alarm_exists(id).await? {
            return Err(ToolError::new(format!("alarm {id} not found")));
        }
        let out = from_handler(
            alarms::acknowledge_alarm(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                Path(id),
            )
            .await,
        )
        .await?;
        Ok(into_object("result", out))
    })
}

fn alarm_summary(ctx: &Ctx, _args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let out = from_handler(alarms::get_alarm_summary(State(ctx.state.clone())).await).await?;
        Ok(json!({"summary": out}))
    })
}
