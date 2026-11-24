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
        // What each data source last went out as, and when to send them all again.
        let mut sent = SentTags::default();
        let mut sent_generation: Option<u64> = None;

        loop {
            tokio::select! {
                r = rx.changed() => {
                    if r.is_err() { break; }
                }
                _ = ds_timer.tick() => {
                    if let Ok(ds) = storage.list_data_sources(None).await {
                        data_sources = ds;
                        ds_fresh = true;
                        // Also resends every tag every 5 s, a backstop for any client that missed one.
                        sent.clear();
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
                let generation = ws.generation();
                if sent_generation != Some(generation) {
                    sent_generation = Some(generation);
                    sent.clear();
                }
                let changed = sent.changed(&sensors);
                if changed.is_empty() {
                    continue;
                }
                updates_buf.clear();
                for s in changed.iter().map(|&i| &sensors[i]) {
                    updates_buf.push(crate::ws_types::TagUpdateEntry {
                        tag_id: s.key.clone(),
                        value: s.value,
                        quality: s.status.clone(),
                        ts: now,
                        simulated: !crate::alarm_eval::is_device_source_type(&s.source_type)
                            && s.source_type != "api",
                        data_quality: Some(s.quality.as_str().to_string()),
                        ts_ms: s.ts_ms,
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

/// One data source's broadcast state: value bits, sample time, status, quality.
type TagSignature = (u64, Option<i64>, String, crate::quality::Quality);

fn signature(s: &crate::models::DataSourceData) -> TagSignature {
    (s.value.to_bits(), s.ts_ms, s.status.clone(), s.quality)
}

/// Which tags changed since they last went out over the WebSocket.
///
/// The broadcaster runs whenever any device updates the cache and used to send
/// every tag each time, so N devices polling once a second sent every tag N
/// times a second, mostly unchanged. Now only changed tags go out; after
/// `clear` (a new connection or subscription, a data-source reload) all of
/// them do.
#[derive(Default)]
struct SentTags {
    last: Vec<Option<TagSignature>>,
}

impl SentTags {
    fn clear(&mut self) {
        self.last.clear();
    }

    /// Indexes into `sensors` to send, in order; records them as sent.
    fn changed(&mut self, sensors: &[crate::models::DataSourceData]) -> Vec<usize> {
        if self.last.len() != sensors.len() {
            self.last = vec![None; sensors.len()];
        }
        let mut changed: Vec<bool> = sensors
            .iter()
            .zip(&self.last)
            .map(|(s, last)| last.as_ref() != Some(&signature(s)))
            .collect();
        // Clients match tags that share a key by their position among that
        // key's entries in the batch, so a changed shared key sends all of them.
        let mut counts: std::collections::HashMap<&str, (usize, bool)> =
            std::collections::HashMap::new();
        for (s, &c) in sensors.iter().zip(&changed) {
            let entry = counts.entry(s.key.as_str()).or_default();
            entry.0 += 1;
            entry.1 |= c;
        }
        for (s, c) in sensors.iter().zip(changed.iter_mut()) {
            let (count, any) = counts[s.key.as_str()];
            if count > 1 && any {
                *c = true;
            }
        }
        let mut out = Vec::new();
        for (i, s) in sensors.iter().enumerate() {
            if changed[i] {
                self.last[i] = Some(signature(s));
                out.push(i);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DataSourceData;
    use crate::quality::Quality;

    fn tag(key: &str, value: f64, ts_ms: i64) -> DataSourceData {
        DataSourceData {
            id: 0,
            key: key.into(),
            value,
            warn: 80.0,
            crit: 90.0,
            source_type: "modbus".into(),
            status: "normal".into(),
            quality: Quality::Good,
            ts_ms: Some(ts_ms),
        }
    }

    #[test]
    fn sends_everything_first_then_only_changes() {
        let mut sent = SentTags::default();
        let mut tags = vec![tag("a", 1.0, 10), tag("b", 2.0, 10), tag("c", 3.0, 10)];
        assert_eq!(sent.changed(&tags), vec![0, 1, 2]);
        assert!(sent.changed(&tags).is_empty());
        tags[1] = tag("b", 2.0, 20); // a new sample with the same value is still news
        assert_eq!(sent.changed(&tags), vec![1]);
        tags[2].quality = Quality::UncertainStale;
        assert_eq!(sent.changed(&tags), vec![2]);
        tags[0].status = "warning".into();
        assert_eq!(sent.changed(&tags), vec![0]);
    }

    #[test]
    fn clear_and_a_new_tag_list_send_everything_again() {
        let mut sent = SentTags::default();
        let tags = vec![tag("a", 1.0, 10), tag("b", 2.0, 10)];
        sent.changed(&tags);
        sent.clear();
        assert_eq!(sent.changed(&tags), vec![0, 1]);
        let more = vec![tag("a", 1.0, 10), tag("b", 2.0, 10), tag("c", 3.0, 10)];
        assert_eq!(sent.changed(&more), vec![0, 1, 2]);
    }

    #[test]
    fn a_changed_shared_key_sends_every_tag_with_that_key_in_order() {
        let mut sent = SentTags::default();
        let mut tags = vec![tag("v", 1.0, 10), tag("x", 5.0, 10), tag("v", 2.0, 10)];
        sent.changed(&tags);
        tags[2] = tag("v", 3.0, 20);
        assert_eq!(sent.changed(&tags), vec![0, 2]);
    }

    #[test]
    fn nan_readings_do_not_count_as_changes() {
        let mut sent = SentTags::default();
        let tags = vec![tag("a", f64::NAN, 0)];
        sent.changed(&tags);
        assert!(sent.changed(&tags).is_empty());
    }
}
