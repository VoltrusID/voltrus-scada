//! Version history tools (docs/version-history.md): the change log, versions,
//! diff, revert and publish of a Designer screen. They call the functions the
//! REST routes use, so checks, audit entries and git sync are the same.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, ToolResult, parse_args};
use crate::api::design_history as api;
use crate::edition::ActionOrigin;
use crate::storage::design_history::{ChangeFilter, RevertObject};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_changes",
            title: "List design changes",
            description: "The change log of a Designer screen (or of the project objects: equipment \
                types, instances, faceplates when screen_id is \"\"), newest first. Each change is \
                one object (widget, connection, ...) added, updated or removed, with who \
                (user, API key, MCP, import...), when, the batch (one save) and the row before \
                and after. Filter by object, object_type or user; page with before_seq. Use \
                seq values with diff_versions (from: \"seq:<n>\") or revert_screen (to_seq).",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string", "description": "Screen id from list_screens; \"\" for project objects."},
                        "object": {"type": "string", "description": "Only this widget/connection id (or type id, instance name, faceplate id)."},
                        "object_type": {"type": "string", "enum": ["widget", "connection", "equipment_type", "equipment_instance", "faceplate"]},
                        "user": {"type": "string"},
                        "before_seq": {"type": "integer", "description": "Paging: changes older than this seq."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                        "include_rows": {"type": "boolean", "default": false, "description": "Include the before/after rows (large)."}
                    },
                    "required": ["screen_id"],
                    "additionalProperties": false
                })
            },
            call: list_changes,
        },
        Tool {
            name: "list_versions",
            title: "List screen versions",
            description: "Named versions of a Designer screen, newest first: name, message, who, \
                when, status (saved, pending approval, published, rejected), and which one \
                operators see now (current). Also returns the screen's publish status: head \
                seq, the published version and how many changes the draft has since.",
            access: Access::Read,
            destructive: false,
            input_schema: screen_only,
            call: list_versions,
        },
        Tool {
            name: "create_version",
            title: "Create screen version",
            description: "Record the screen's current draft as a named version (a commit) with a \
                message. The name defaults to v<n>. It changes nothing operators see; publish \
                it with publish_screen.",
            access: Access::Write,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string"},
                        "name": {"type": "string", "maxLength": 100},
                        "message": {"type": "string", "maxLength": 2000}
                    },
                    "required": ["screen_id"],
                    "additionalProperties": false
                })
            },
            call: create_version,
        },
        Tool {
            name: "diff_versions",
            title: "Diff two screen states",
            description: "Compare two states of a screen: per widget and connection added, removed \
                or changed (with the changed fields), and both states as project YAML (tags by \
                ref). A state is \"draft\", \"published\", \"seq:<n>\" (right after change n) or \
                a version id or name. Defaults: from published to draft (what publishing would \
                change).",
            access: Access::Read,
            destructive: false,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string"},
                        "from": {"type": "string", "default": "published"},
                        "to": {"type": "string", "default": "draft"},
                        "yaml": {"type": "boolean", "default": true}
                    },
                    "required": ["screen_id"],
                    "additionalProperties": false
                })
            },
            call: diff_versions,
        },
        Tool {
            name: "revert_screen",
            title: "Revert screen or objects",
            description: "Put a screen's draft back to an earlier state, written as new changes \
                (history is never deleted). Whole screen: to (version id/name, \"published\", \
                \"seq:<n>\") or to_seq. One object: object (+ object_type widget|connection) \
                with to_seq or to. Several as one batch: objects. Removing a widget removes the \
                connections ending on it; restoring a connection whose widget is gone is refused \
                (dangling_connection) unless that widget is in objects too. Only the draft \
                changes; publish to show it to operators.",
            access: Access::Write,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string"},
                        "to": {"type": "string"},
                        "to_seq": {"type": "integer"},
                        "object": {"type": "string"},
                        "object_type": {"type": "string", "enum": ["widget", "connection"]},
                        "objects": {"type": "array", "items": {
                            "type": "object",
                            "properties": {
                                "object_id": {"type": "string"},
                                "object_type": {"type": "string", "enum": ["widget", "connection"]},
                                "to_seq": {"type": "integer"},
                                "to": {"type": "string"}
                            },
                            "required": ["object_id"],
                            "additionalProperties": false
                        }},
                        "base_seq": {"type": "integer", "description": "Refuse when the screen changed since this seq."},
                        "message": {"type": "string", "maxLength": 2000}
                    },
                    "required": ["screen_id"],
                    "additionalProperties": false
                })
            },
            call: revert_screen,
        },
        Tool {
            name: "publish_screen",
            title: "Publish screen",
            description: "Make a screen's draft (or a version) what operators see. Propose, then \
                apply: without apply:true it only returns the diff from published to what would \
                be published and whether the site needs an approval. With apply:true it records \
                a version (name/message) and publishes it; when the site requires approval it \
                files a request an administrator approves in the Designer instead. Show the \
                diff to the user and apply only after they agree. A site whose edition wants an \
                electronic signature for publishing refuses MCP (sign in the Designer). Limited \
                to a signed-in admin.",
            access: Access::AdminWrite,
            destructive: true,
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "screen_id": {"type": "string"},
                        "version": {"type": "string", "description": "Publish this version (id or name) instead of the draft."},
                        "name": {"type": "string", "maxLength": 100},
                        "message": {"type": "string", "maxLength": 2000},
                        "apply": {"type": "boolean", "default": false}
                    },
                    "required": ["screen_id"],
                    "additionalProperties": false
                })
            },
            call: publish_screen,
        },
    ]
}

fn screen_only() -> Value {
    json!({
        "type": "object",
        "properties": {"screen_id": {"type": "string"}},
        "required": ["screen_id"],
        "additionalProperties": false
    })
}

/// History shows other people's configuration work: the roles that configure.
fn configure_only(ctx: &Ctx) -> Result<(), ToolError> {
    if crate::api::role_allows_configuration(&ctx.user.role) {
        Ok(())
    } else {
        Err(ToolError::new(
            "version history is for roles that configure screens (admin, integration keys)",
        ))
    }
}

fn list_changes(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
            object: Option<String>,
            object_type: Option<String>,
            user: Option<String>,
            before_seq: Option<i64>,
            limit: Option<i64>,
            #[serde(default)]
            include_rows: bool,
        }
        configure_only(ctx)?;
        let a: Args = parse_args(args)?;
        let mut out = api::list_changes(
            &ctx.state,
            ChangeFilter {
                screen_id: Some(a.screen_id),
                object_type: a.object_type,
                object_id: a.object,
                username: a.user,
                batch: None,
                before_seq: a.before_seq,
                limit: Some(a.limit.unwrap_or(50)),
            },
        )
        .await?;
        if !a.include_rows
            && let Some(Value::Array(changes)) = out.get_mut("changes")
        {
            for c in changes {
                if let Some(m) = c.as_object_mut() {
                    m.remove("before");
                    m.remove("after");
                }
            }
        }
        Ok(out)
    })
}

fn list_versions(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
        }
        configure_only(ctx)?;
        let Args { screen_id } = parse_args(args)?;
        let id = screen_id.clone();
        let (versions, status) = ctx
            .state
            .storage
            .design_tx(move |c| {
                Ok((
                    crate::storage::design_history::list_versions(c, &id)?,
                    crate::storage::design_history::screen_status(c, &id)?,
                ))
            })
            .await?;
        Ok(json!({"screen_id": screen_id, "versions": versions, "status": status}))
    })
}

fn create_version(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
            name: Option<String>,
            #[serde(default)]
            message: String,
        }
        let a: Args = parse_args(args)?;
        let v = api::create_version(
            &ctx.state,
            &ctx.user,
            &a.screen_id,
            api::CreateVersion {
                name: a.name,
                message: a.message,
            },
        )
        .await?;
        Ok(json!({"version": v}))
    })
}

fn diff_versions(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
            from: Option<String>,
            to: Option<String>,
            yaml: Option<bool>,
        }
        configure_only(ctx)?;
        let a: Args = parse_args(args)?;
        let mut out = api::diff(
            &ctx.state,
            &a.screen_id,
            a.from.as_deref().unwrap_or("published"),
            a.to.as_deref().unwrap_or("draft"),
            a.yaml.unwrap_or(true),
        )
        .await?;
        // The YAML says the same as the rows, shorter.
        if let Some(m) = out.as_object_mut() {
            m.remove("rows");
        }
        Ok(out)
    })
}

fn revert_screen(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            screen_id: String,
            to: Option<String>,
            to_seq: Option<i64>,
            object: Option<String>,
            object_type: Option<String>,
            #[serde(default)]
            objects: Vec<RevertObject>,
            base_seq: Option<i64>,
            #[serde(default)]
            message: String,
        }
        let a: Args = parse_args(args)?;
        let actor =
            crate::api::design_history::actor_for_origin(&ctx.user, ActionOrigin::Mcp, "revert");
        Ok(api::revert(
            &ctx.state,
            actor,
            &a.screen_id,
            api::RevertBody {
                to: a.to,
                to_seq: a.to_seq,
                object: a.object,
                object_type: a.object_type,
                objects: a.objects,
                base_seq: a.base_seq,
                base_revision: None,
                message: a.message,
            },
        )
        .await?)
    })
}

fn publish_screen(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move { publish(ctx, args).await })
}

async fn publish(ctx: &Ctx, args: Value) -> ToolResult {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        screen_id: String,
        version: Option<String>,
        name: Option<String>,
        #[serde(default)]
        message: String,
        #[serde(default)]
        apply: bool,
    }
    let a: Args = parse_args(args)?;
    if !a.apply {
        let settings = ctx.state.storage.design_settings().await?;
        let target = a.version.clone().unwrap_or_else(|| "draft".into());
        let mut diff = api::diff(&ctx.state, &a.screen_id, "published", &target, true).await?;
        if let Some(m) = diff.as_object_mut() {
            m.remove("rows");
        }
        return Ok(json!({
            "proposal": true,
            "publishing": settings.publishing,
            "needs_approval": settings.approval == crate::storage::design_history::Approval::Required,
            "would_publish": target,
            "diff": diff,
            "next": "Show this diff to the user; call again with apply: true to publish.",
        }));
    }
    Ok(api::publish(
        &ctx.state,
        &ctx.user,
        ActionOrigin::Mcp,
        &a.screen_id,
        api::PublishBody {
            version: a.version,
            name: a.name,
            message: a.message,
            base_seq: None,
            signature: None,
        },
    )
    .await?)
}
