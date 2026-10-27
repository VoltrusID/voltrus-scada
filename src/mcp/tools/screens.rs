use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::screens;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_screens",
            title: "List screens",
            description: "List SCADA screens (dashboards) with their id and name, in display \
                order. Use a screen id with list_tags(screen) or add_tag_to_screen.",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: list_screens,
        },
        Tool {
            name: "create_screen",
            title: "Create screen",
            description: "Create an empty screen. The id is derived from the name (lowercase, \
                underscores, made unique). Returns the screen; then place tags on it with \
                add_tag_to_screen.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {"name": {"type": "string", "minLength": 1, "description": "Screen title, e.g. \"Boiler house\"."}},
                    "required": ["name"],
                    "additionalProperties": false
                })
            },
            call: create_screen,
        },
        Tool {
            name: "add_tag_to_screen",
            title: "Add tag to screen",
            description: "Show an existing tag on a screen (appended after the tags already \
                there). Both ids must exist: screen id from list_screens, tag id from list_tags.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string", "description": "Screen id from list_screens."},
                        "tag_id": {"type": "integer", "description": "Tag id from list_tags."}
                    },
                    "required": ["screen_id", "tag_id"],
                    "additionalProperties": false
                })
            },
            call: add_tag_to_screen,
        },
    ]
}

fn list_screens(ctx: &Ctx, _args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let out = from_handler(screens::list_screens(State(ctx.state.clone())).await).await?;
        let list = out.as_array().cloned().unwrap_or_default();
        Ok(json!({"screens": list, "count": list.len()}))
    })
}

fn create_screen(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            name: String,
        }
        let Args { name } = parse_args(args)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(ToolError::new("name must not be empty"));
        }
        let screen = from_handler(
            screens::create_screen(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                axum::Json(json!({"name": name})),
            )
            .await,
        )
        .await?;
        Ok(json!({"screen": screen}))
    })
}

fn add_tag_to_screen(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
            tag_id: i64,
        }
        let Args { screen_id, tag_id } = parse_args(args)?;
        if !ctx.state.storage.device_exists(&screen_id).await? {
            return Err(ToolError::new(format!("screen '{screen_id}' not found")));
        }
        if ctx
            .state
            .storage
            .get_data_source_by_id(tag_id)
            .await?
            .is_none()
        {
            return Err(ToolError::new(format!("tag {tag_id} not found")));
        }
        let already = ctx
            .state
            .storage
            .list_data_sources(Some(&screen_id))
            .await?
            .iter()
            .any(|t| t.id == tag_id);
        if already {
            return Ok(
                json!({"ok": true, "screen_id": screen_id, "tag_id": tag_id, "already_present": true}),
            );
        }
        from_handler(
            screens::add_screen_sensor(
                State(ctx.state.clone()),
                Path(screen_id.clone()),
                axum::Json(json!({"data_source_id": tag_id})),
            )
            .await,
        )
        .await?;
        Ok(json!({"ok": true, "screen_id": screen_id, "tag_id": tag_id, "already_present": false}))
    })
}
