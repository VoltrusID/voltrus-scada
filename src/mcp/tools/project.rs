//! Project tools: the whole configuration as `voltrus-project/1` YAML.
//!
//! All three expose or replace configuration across every device, tag, screen
//! and flow, so, like the REST routes, they are for a signed-in admin only;
//! API and integration keys are refused before the tool runs. They share
//! [`crate::api::project`] with the HTTP routes: the same parsing, validation,
//! pre-import snapshot, poller restarts and audit entry.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, ToolResult, parse_args};
use crate::api::project;
use crate::project_yaml::Mode;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "project_export_yaml",
            title: "Export project as YAML",
            description: "Export the whole configuration (devices of every protocol, tags with \
                alarm and write policies, screens, flows, scripts, recipes, notifications, \
                device templates, settings, ...) as one `voltrus-project/1` YAML document, \
                returned in `yaml`. Secrets are replaced by ${ENV_VAR} references; users, API \
                keys, telemetry and alarms are not included. The output can be edited and \
                passed to project_plan_import. The document can be large. Limited to a \
                signed-in admin.",
            access: Access::AdminRead,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: export,
        },
        Tool {
            name: "project_plan_import",
            title: "Plan project import",
            description: "Dry run of a project import: check a `voltrus-project/1` YAML document \
                and return what importing it would create, update and delete (per section and \
                per entity, with the names of the changed fields), plus warnings. Writes \
                nothing. A document that fails validation is reported with every problem and \
                its location. `mode` merge (default) leaves entities the document does not \
                list alone; replace deletes them. Always run this before project_apply_import. \
                Limited to a signed-in admin.",
            access: Access::AdminRead,
            destructive: false,
            input_schema: import_schema,
            call: plan,
        },
        Tool {
            name: "project_apply_import",
            title: "Apply project import",
            description: "Import a `voltrus-project/1` YAML document in one transaction: \
                create and update what it lists and, in `replace` mode, delete every entity of \
                the covered kinds it does not list. A configuration snapshot is taken first \
                for rollback, and Modbus pollers are restarted for changed devices. Call \
                project_plan_import first and apply only a plan you have reviewed with the \
                user; `replace` is not reversible except through that snapshot. Limited to a \
                signed-in admin.",
            access: Access::AdminWrite,
            destructive: true,
            input_schema: import_schema,
            call: apply,
        },
    ]
}

fn import_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "yaml": {"type": "string", "minLength": 1, "description": "The project document (YAML, format voltrus-project/1), e.g. from project_export_yaml."},
            "mode": {"type": "string", "enum": ["merge", "replace"], "default": "merge", "description": "merge: create and update only. replace: also delete listed-kind entities the document omits."}
        },
        "required": ["yaml"],
        "additionalProperties": false
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportArgs {
    yaml: String,
    mode: Option<String>,
}

fn export(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {}
        let Args {} = parse_args(args)?;
        let yaml = project::export_yaml(&ctx.state).await?;
        Ok(json!({"format": crate::project_yaml::FORMAT, "bytes": yaml.len(), "yaml": yaml}))
    })
}

fn plan(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move { run(ctx, args, true).await })
}

fn apply(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move { run(ctx, args, false).await })
}

async fn run(ctx: &Ctx, args: Value, dry_run: bool) -> ToolResult {
    let a: ImportArgs = parse_args(args)?;
    let mode: Mode = project::parse_mode(a.mode.as_deref())?;
    let plan = project::run_import(&ctx.state, &ctx.user, &a.yaml, mode, dry_run)
        .await
        .map_err(|e| ToolError::new(e.to_string()))?;
    serde_json::to_value(&plan)
        .map_err(|e| ToolError::new(format!("could not encode the plan: {e}")))
}
