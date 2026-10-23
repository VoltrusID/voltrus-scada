use std::sync::Arc;

use crate::storage::Storage;
use crate::ws_manager::WsManager;

pub fn spawn_data_broadcaster(
    cache: crate::cache::DeviceCache,
    storage: Storage,
    ws_manager: Option<Arc<WsManager>>,
) {
    tokio::spawn(async move {
        let mut rx = cache.trigger_receiver();

        // Cache data sources between ticks; only re-fetch periodically.
        let mut data_sources: Vec<crate::models::DataSource> = Vec::new();
        let mut ds_fresh = false;
        let mut ds_timer = tokio::time::interval(std::time::Duration::from_secs(5));
        ds_timer.tick().await; // consume the immediate first tick

        // Pre-allocated reusable buffers — cleared each tick, never reallocated after warm-up
        let mut updates_buf: Vec<crate::ws_types::TagUpdateEntry> = Vec::with_capacity(64);
        let mut encode_buf: Vec<u8> = Vec::with_capacity(2048);

        loop {
            tokio::select! {
                r = rx.changed() => {
                    if r.is_err() { break; }
                }
                _ = ds_timer.tick() => {
                    if let Ok(ds) = storage.list_data_sources(None).await {
                        data_sources = ds;
                        ds_fresh = true;
                    }
                    continue;
                }
            }

            // Lazily load data sources on first trigger
            if !ds_fresh {
                if let Ok(ds) = storage.list_data_sources(None).await {
                    data_sources = ds;
                    ds_fresh = true;
                } else {
                    continue;
                }
            }

            // Skip all work when there are no WS consumers
            let has_ws = ws_manager
                .as_ref()
                .is_some_and(|ws| ws.connection_count() > 0);
            if !has_ws {
                continue;
            }

            let snapshots = cache.get_all().await;
            let now = crate::timeutil::now_secs();
            let sensors = super::data::evaluate_data_sources(&data_sources, &snapshots, now as f64);

            // Broadcast WS batch — reuse encode buffer across ticks
            if let Some(ref ws) = ws_manager {
                updates_buf.clear();
                for s in &sensors {
                    updates_buf.push(crate::ws_types::TagUpdateEntry {
                        tag_id: s.key.clone(),
                        value: s.value,
                        quality: s.status.clone(),
                        ts: now,
                        simulated: s.source_type != "modbus" && s.source_type != "api",
                    });
                }
                let msg = crate::ws_types::ServerMessage::TagUpdateBatch {
                    updates: std::mem::take(&mut updates_buf),
                };
                ws.broadcast_preencoded_with_buf(&msg, &mut encode_buf);
                // Recover the updates vec from the message (it was moved into msg then dropped)
                if let crate::ws_types::ServerMessage::TagUpdateBatch { updates } = msg {
                    updates_buf = updates;
                }
            }
        }
    });
}
