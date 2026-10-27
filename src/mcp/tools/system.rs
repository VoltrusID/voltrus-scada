use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolFuture, ToolResult};
use crate::api::role_allows_operational_writes;
use crate::models::DeviceStatus;

pub fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "server_info",
        title: "Server info",
        description: "Start here. Returns the Voltrus server version, uptime and licence tier, \
            who you are authenticated as (username, role, whether you may call write tools) and \
            headline counts: tags, screens, flows, configured/online devices and active alarms.",
        access: Access::Read,
        destructive: false,
        input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
        call: server_info,
    }]
}

fn server_info(ctx: &Ctx, _args: Value) -> ToolFuture<'_> {
    Box::pin(async move { run(ctx).await })
}

async fn run(ctx: &Ctx) -> ToolResult {
    let state = &ctx.state;
    let tags = state.storage.list_data_sources(None).await?.len();
    let screens = state.storage.list_devices().await?.len();
    let flows = state.storage.list_flows().await?.len();
    let active_alarms = state.storage.count_active_alarms().await?;
    let snapshots = state.cache.get_all().await;
    let online = snapshots
        .iter()
        .filter(|s| s.status == DeviceStatus::Online)
        .count();
    Ok(json!({
        "name": "voltrus-scada",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "license_tier": state.tier().as_str(),
        "caller": {
            "username": ctx.user.username,
            "role": ctx.user.role,
            "can_write": role_allows_operational_writes(&ctx.user.role),
        },
        "counts": {
            "tags": tags,
            "screens": screens,
            "flows": flows,
            "devices_reporting": snapshots.len(),
            "devices_online": online,
            "active_alarms": active_alarms,
        },
    }))
}
