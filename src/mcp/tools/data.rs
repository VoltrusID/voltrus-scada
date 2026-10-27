//! Process values: the live cache and stored history.

use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::data;
use crate::models::DataQuery;

/// History answers larger than this are refused so one call cannot flood the model's context.
const MAX_HISTORY_BYTES: usize = 400 * 1024;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_live_values",
            title: "Read live values",
            description: "Current value of tags, taken from the live cache (what the SCADA \
                screens show right now). Each entry has the tag id, key, name, unit, value \
                (null = no data / device offline) and status: normal, warning, critical or \
                offline, judged against the tag's limits. Pass `keys` (tag keys) to read only \
                some tags, or `screen` for the tags of one screen; with neither, all tags.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "keys": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 500, "description": "Tag keys to read (see list_tags)."},
                        "screen": {"type": "string", "description": "Only tags of this screen id."}
                    },
                    "additionalProperties": false
                })
            },
            call: read_live_values,
        },
        Tool {
            name: "read_history",
            title: "Read tag history",
            description: "Stored trend data for one or more tags. Pick a `range` preset (30m, 1h, \
                6h, 24h, 7d, 30d) or an absolute window with `from` and `to` (Unix seconds). \
                `agg`: avg (default, bucketed averages), minmax (averages plus min and max per \
                bucket), sampling (raw samples). Result: `data[key] = {values, start (ms), \
                interval (ms), count}`; value i is at start + i * interval. Buckets are coarse \
                for long ranges, so narrow the window to see detail.",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "keys": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 50, "description": "Tag keys to read (see list_tags)."},
                        "range": {"type": "string", "enum": ["30m", "1h", "6h", "24h", "7d", "30d"], "default": "1h", "description": "Window ending now. Ignored when from/to are given."},
                        "from": {"type": "integer", "minimum": 1, "description": "Window start, Unix seconds. Give together with `to`."},
                        "to": {"type": "integer", "minimum": 1, "description": "Window end, Unix seconds."},
                        "agg": {"type": "string", "enum": ["avg", "minmax", "sampling"], "default": "avg"}
                    },
                    "required": ["keys"],
                    "additionalProperties": false
                })
            },
            call: read_history,
        },
    ]
}

fn read_live_values(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            keys: Option<Vec<String>>,
            screen: Option<String>,
        }
        let args: Args = parse_args(args)?;
        let state = &ctx.state;
        let mut sources = state
            .storage
            .list_data_sources(args.screen.as_deref())
            .await?;
        if let Some(keys) = &args.keys {
            sources.retain(|s| keys.contains(&s.key));
        }
        let snapshots = state.cache.get_all().await;
        let now = crate::timeutil::now_secs() as f64;
        let values = data::evaluate_data_sources(&sources, &snapshots, now);
        let entries: Vec<Value> = values
            .iter()
            .zip(&sources)
            .map(|(v, s)| {
                json!({
                    "id": v.id,
                    "key": v.key,
                    "name": s.name,
                    "unit": s.unit,
                    "device_id": s.device_id,
                    "value": if v.value.is_finite() { json!(v.value) } else { Value::Null },
                    "status": v.status,
                    "warn": v.warn,
                    "crit": v.crit,
                })
            })
            .collect();
        let missing: Vec<&String> = args
            .keys
            .iter()
            .flatten()
            .filter(|k| !sources.iter().any(|s| &s.key == *k))
            .collect();
        let mut out = json!({
            "values": entries,
            "count": entries.len(),
            "timestamp": crate::timeutil::now_secs(),
        });
        if !missing.is_empty() {
            out["unknown_keys"] = json!(missing);
        }
        Ok(out)
    })
}

fn read_history(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            keys: Vec<String>,
            range: Option<String>,
            from: Option<i64>,
            to: Option<i64>,
            agg: Option<String>,
        }
        let args: Args = parse_args(args)?;
        if args.keys.is_empty()
            || args
                .keys
                .iter()
                .any(|k| k.trim().is_empty() || k.contains(','))
        {
            return Err(ToolError::new(
                "keys must be a non-empty list of tag keys (no empty entries, no commas)",
            ));
        }
        let agg = args.agg.as_deref().unwrap_or("avg");
        if !matches!(agg, "avg" | "minmax" | "sampling") {
            return Err(ToolError::new("agg must be avg, minmax or sampling"));
        }
        let range = match (args.from, args.to) {
            (Some(from), Some(to)) => {
                if from <= 0 || to <= from {
                    return Err(ToolError::new(
                        "from and to are Unix seconds with 0 < from < to",
                    ));
                }
                // The history handler's custom window is "startMs-endMs".
                format!("{}-{}", from.saturating_mul(1000), to.saturating_mul(1000))
            }
            (None, None) => {
                let preset = args.range.as_deref().unwrap_or("1h");
                if !matches!(preset, "30m" | "1h" | "6h" | "24h" | "7d" | "30d") {
                    return Err(ToolError::new(
                        "range must be one of 30m, 1h, 6h, 24h, 7d, 30d (or use from/to)",
                    ));
                }
                preset.to_string()
            }
            _ => return Err(ToolError::new("give both from and to, or neither")),
        };
        let query: DataQuery = serde_json::from_value(json!({
            "sensor": args.keys.join(","),
            "range": range,
            "agg": agg,
        }))
        .map_err(|e| ToolError::new(e.to_string()))?;
        let out =
            from_handler(data::get_history(State(ctx.state.clone()), Query(query)).await).await?;
        let size = out.to_string().len();
        if size > MAX_HISTORY_BYTES {
            return Err(ToolError::new(format!(
                "history answer is {} KB, over the {} KB limit: request fewer keys or a shorter window",
                size / 1024,
                MAX_HISTORY_BYTES / 1024
            )));
        }
        let found: Vec<&str> = out["sensors"]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s["key"].as_str()).collect())
            .unwrap_or_default();
        let unknown: Vec<&String> = args
            .keys
            .iter()
            .filter(|k| !found.contains(&k.as_str()))
            .collect();
        let mut out = out;
        if !unknown.is_empty() {
            out["unknown_keys"] = json!(unknown);
        }
        Ok(out)
    })
}
