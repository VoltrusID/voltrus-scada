//! Tags (data sources): the named values the SCADA shows, trends and alarms on.

use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, ToolResult, from_handler, parse_args};
use crate::api::data_sources;
use crate::models::DataQuery;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_tags",
            title: "List tags",
            description: "List tags (data sources) with their limits, unit, source type and \
                source_config. Filter by `screen`, `device_id`, `source_type` or a case-insensitive \
                `query` matched against key and name. Results are capped by `limit` (default 200); \
                `total` is the number of matches before the cap.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen": {"type": "string", "description": "Only tags assigned to this screen id."},
                        "device_id": {"type": "string", "description": "Only tags of this device."},
                        "source_type": {"type": "string", "description": "Only tags of this source type, e.g. \"modbus\"."},
                        "query": {"type": "string", "description": "Substring of the key or name."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 2000, "default": 200}
                    },
                    "additionalProperties": false
                })
            },
            call: list_tags,
        },
        Tool {
            name: "create_tag",
            title: "Create tag",
            description: "Create a tag. `key` identifies it in live values and history (for a \
                register of a Modbus device: source_type \"modbus\", device_id = the device id, key = \
                the register name). `name` must pass the server's naming convention. min_val/max_val \
                set the display range, warn_val/crit_val the high alarm limits (0 = disabled). Low \
                limits and expressions go in source_config, e.g. {\"low_warn\": 10, \"low_crit\": 5} \
                or {\"expression\": \"a * 2\"}. Without a source_type the tag is a simulated \
                random_walk around base_val. Returns the new tag id.",
            access: Access::Write,
            destructive: false,
            input_schema: || tag_schema(true),
            call: create_tag,
        },
        Tool {
            name: "update_tag",
            title: "Update tag",
            description: "Change fields of an existing tag, identified by its numeric `id` from \
                list_tags. Only the fields you pass change. Use it to set limits (warn_val, \
                crit_val, min_val, max_val), unit, name or source_config. Returns the updated tag.",
            access: Access::Write,
            destructive: false,
            input_schema: || tag_schema(false),
            call: update_tag,
        },
        Tool {
            name: "delete_tag",
            title: "Delete tag",
            description: "Delete a tag by numeric id. Its history, screen placements and alarm \
                rules go with it as far as the server cascades them. Not reversible; confirm the \
                id with list_tags first.",
            access: Access::Write,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {"id": {"type": "integer", "description": "Tag id from list_tags."}},
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
            call: delete_tag,
        },
    ]
}

/// JSON Schema shared by create_tag and update_tag (`create` adds `key` as required, `update` adds `id`).
fn tag_schema(create: bool) -> Value {
    let mut props = Map::new();
    if !create {
        props.insert(
            "id".into(),
            json!({"type": "integer", "description": "Tag id from list_tags."}),
        );
    }
    let fields = json!({
        "key": {"type": "string", "minLength": 1, "description": "Machine key used in live values and history. For modbus tags: the register name."},
        "name": {"type": "string", "minLength": 1, "description": "Display name; checked against the naming convention. Defaults to the key on create."},
        "unit": {"type": "string", "description": "Engineering unit, e.g. \"degC\", \"kW\"."},
        "color": {"type": "string", "description": "CSS hex colour for charts, e.g. \"#f97316\"."},
        "min_val": {"type": "number", "description": "Lower end of the display range (default 0)."},
        "max_val": {"type": "number", "description": "Upper end of the display range (default 100)."},
        "warn_val": {"type": "number", "description": "High warning limit; 0 disables."},
        "crit_val": {"type": "number", "description": "High critical limit; 0 disables."},
        "base_val": {"type": "number", "description": "Nominal value for simulated source types (default 50)."},
        "decimals": {"type": "integer", "minimum": 0, "maximum": 10, "description": "Display decimals (default 1)."},
        "source_type": {"type": "string", "description": "Where the value comes from: \"modbus\" (device register), or a simulated type: random_walk (default), constant, noise, sine, step, sawtooth."},
        "source_config": {"type": ["object", "string"], "description": "Source settings as an object (or JSON text): low_warn/low_crit limits, expression, sine period, ..."},
        "device_id": {"type": "string", "description": "Device that produces the value (default \"default\")."},
        "equipment_id": {"type": "string", "description": "Equipment asset this tag belongs to."},
        "chart_type": {"type": "string", "description": "Default chart: \"sparkline\" (default), or another widget chart type."},
        "path": {"type": "string", "description": "Hierarchical tag path, e.g. \"plant/line1/tank\"."},
        "on_delay_secs": {"type": "integer", "minimum": 0, "description": "Seconds a limit must stay exceeded before an alarm raises (0 = immediately)."},
        "off_delay_secs": {"type": "integer", "minimum": 0, "description": "Seconds the value must stay back in range before the alarm clears."}
    });
    if let Value::Object(map) = fields {
        props.extend(map);
    }
    json!({
        "type": "object",
        "properties": props,
        "required": if create { json!(["key"]) } else { json!(["id"]) },
        "additionalProperties": false
    })
}

#[derive(Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
struct TagFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warn_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crit_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_val: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decimals: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    equipment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chart_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    on_delay_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    off_delay_secs: Option<i64>,
}

impl TagFields {
    /// Reject what the server would store but could never evaluate.
    fn validate(&self) -> Result<(), ToolError> {
        if let Some(true) = self.key.as_deref().map(|k| k.trim().is_empty()) {
            return Err(ToolError::new("key must not be empty"));
        }
        if let Some(true) = self.name.as_deref().map(|n| n.trim().is_empty()) {
            return Err(ToolError::new("name must not be empty"));
        }
        if let (Some(min), Some(max)) = (self.min_val, self.max_val)
            && min > max
        {
            return Err(ToolError::new("min_val must not exceed max_val"));
        }
        for (field, value) in [
            ("decimals", self.decimals),
            ("on_delay_secs", self.on_delay_secs),
            ("off_delay_secs", self.off_delay_secs),
        ] {
            if value.is_some_and(|v| v < 0) {
                return Err(ToolError::new(format!("{field} must not be negative")));
            }
        }
        match &self.source_config {
            None | Some(Value::Null) => {}
            Some(Value::Object(_)) => {}
            Some(Value::String(text)) => {
                if !matches!(serde_json::from_str::<Value>(text), Ok(Value::Object(_))) {
                    return Err(ToolError::new(
                        "source_config text must be a JSON object, e.g. {\"low_warn\": 10}",
                    ));
                }
            }
            Some(_) => {
                return Err(ToolError::new(
                    "source_config must be an object or JSON text",
                ));
            }
        }
        Ok(())
    }
}

fn list_tags(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen: Option<String>,
            device_id: Option<String>,
            source_type: Option<String>,
            query: Option<String>,
            limit: Option<usize>,
        }
        let args: Args = parse_args(args)?;
        let query: DataQuery = serde_json::from_value(json!({"screen": args.screen}))
            .map_err(|e| ToolError::new(e.to_string()))?;
        let all = from_handler(
            data_sources::list_data_sources(State(ctx.state.clone()), Query(query)).await,
        )
        .await?;
        let needle = args.query.as_deref().map(str::to_lowercase);
        let matching: Vec<Value> = all
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| {
                args.device_id
                    .as_deref()
                    .is_none_or(|d| t["device_id"] == d)
                    && args
                        .source_type
                        .as_deref()
                        .is_none_or(|s| t["source_type"] == s)
                    && needle.as_deref().is_none_or(|n| {
                        ["key", "name"]
                            .iter()
                            .any(|f| t[f].as_str().is_some_and(|v| v.to_lowercase().contains(n)))
                    })
            })
            .collect();
        let total = matching.len();
        let limit = args.limit.unwrap_or(200).clamp(1, 2000);
        let tags: Vec<Value> = matching.into_iter().take(limit).collect();
        Ok(
            json!({"tags": tags, "count": tags.len(), "total": total, "truncated": total > tags.len()}),
        )
    })
}

fn create_tag(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let mut fields: TagFields = parse_args(args)?;
        fields.validate()?;
        let Some(key) = fields.key.clone() else {
            return Err(ToolError::new("key is required"));
        };
        let key = key.trim().to_string();
        fields.key = Some(key.clone());
        fields.name.get_or_insert_with(|| key.clone());

        // The create handler takes the base fields; delays and path are set by a
        // follow-up update, as the web client does.
        let (path, on_delay, off_delay) = (
            fields.path.take(),
            fields.on_delay_secs.take(),
            fields.off_delay_secs.take(),
        );
        let body = serde_json::to_value(&fields).map_err(|e| ToolError::new(e.to_string()))?;
        let created = from_handler(
            data_sources::create_data_source(State(ctx.state.clone()), axum::Json(body)).await,
        )
        .await?;
        let id = created["id"]
            .as_i64()
            .ok_or_else(|| ToolError::new("the server did not return a tag id"))?;

        let mut extra = Map::new();
        if let Some(path) = path {
            extra.insert("path".into(), json!(path));
        }
        if let Some(v) = on_delay {
            extra.insert("on_delay_secs".into(), json!(v));
        }
        if let Some(v) = off_delay {
            extra.insert("off_delay_secs".into(), json!(v));
        }
        if !extra.is_empty() {
            from_handler(
                data_sources::update_data_source(
                    State(ctx.state.clone()),
                    Path(id),
                    axum::Json(Value::Object(extra)),
                )
                .await,
            )
            .await?;
        }
        tag_by_id(ctx, id).await
    })
}

fn update_tag(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let mut object = match args {
            Value::Object(map) => map,
            _ => return Err(ToolError::new("invalid arguments: expected an object")),
        };
        let id = object
            .remove("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ToolError::new("invalid arguments: id (integer) is required"))?;
        let fields: TagFields = parse_args(Value::Object(object))?;
        fields.validate()?;
        let body = serde_json::to_value(&fields).map_err(|e| ToolError::new(e.to_string()))?;
        if body.as_object().is_none_or(|o| o.is_empty()) {
            return Err(ToolError::new("nothing to update: pass at least one field"));
        }
        if ctx.state.storage.get_data_source_by_id(id).await?.is_none() {
            return Err(ToolError::new(format!("tag {id} not found")));
        }
        from_handler(
            data_sources::update_data_source(State(ctx.state.clone()), Path(id), axum::Json(body))
                .await,
        )
        .await?;
        tag_by_id(ctx, id).await
    })
}

fn delete_tag(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            id: i64,
        }
        let Args { id } = parse_args(args)?;
        let Some(tag) = ctx.state.storage.get_data_source_by_id(id).await? else {
            return Err(ToolError::new(format!("tag {id} not found")));
        };
        from_handler(data_sources::delete_data_source(State(ctx.state.clone()), Path(id)).await)
            .await?;
        Ok(json!({"ok": true, "deleted": {"id": id, "key": tag.key, "name": tag.name}}))
    })
}

async fn tag_by_id(ctx: &Ctx, id: i64) -> ToolResult {
    let tag = ctx
        .state
        .storage
        .get_data_source_by_id(id)
        .await?
        .ok_or_else(|| ToolError::new(format!("tag {id} not found")))?;
    Ok(json!({"tag": tag}))
}
