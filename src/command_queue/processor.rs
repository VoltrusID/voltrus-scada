use super::*;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::cache::DeviceCache;
#[cfg(feature = "modbus")]
use crate::modbus::ModbusPool;
use crate::storage::Storage;
use crate::ws_manager::WsManager;

#[cfg(feature = "modbus")]
impl CommandQueue {
    pub fn new(storage: Storage, cache: DeviceCache, modbus_pool: Arc<ModbusPool>) -> Self {
        Self {
            storage,
            cache,
            in_flight: Arc::new(RwLock::new(HashSet::new())),
            ws_manager: None,
            readback_trackers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            modbus_pool,
            #[cfg(feature = "mqtt-broker")]
            mqtt_broker: Arc::new(RwLock::new(None)),
        }
    }
}

#[cfg(not(feature = "modbus"))]
impl CommandQueue {
    pub fn new(storage: Storage, cache: DeviceCache) -> Self {
        Self {
            storage,
            cache,
            in_flight: Arc::new(RwLock::new(HashSet::new())),
            ws_manager: None,
            readback_trackers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            #[cfg(feature = "mqtt-broker")]
            mqtt_broker: Arc::new(RwLock::new(None)),
        }
    }
}

/// True once a command due at `due_at` (Unix seconds) has waited longer than
/// the expiry window.
fn is_expired(due_at: i64, now: i64) -> bool {
    due_at + COMMAND_EXPIRY_SECS < now
}

impl CommandQueue {
    /// Attach a WsManager so that command status changes are broadcast over WebSocket.
    pub fn with_ws_manager(mut self, ws_manager: Arc<WsManager>) -> Self {
        self.ws_manager = Some(ws_manager);
        self
    }

    /// Returns a shared handle to the embedded MQTT broker slot so main.rs can
    /// populate it after the broker starts (the queue task is already spawned).
    #[cfg(feature = "mqtt-broker")]
    pub fn mqtt_broker_handle(&self) -> Arc<RwLock<Option<crate::mqtt_broker::MqttBrokerState>>> {
        self.mqtt_broker.clone()
    }

    /// Main loop: process the queue every 500 ms.
    pub async fn run(&self) {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(POLL_INTERVAL_MS));
        let mut recovered = false;
        loop {
            interval.tick().await;
            // Queued commands wait for the node that owns the devices.
            if !crate::node_role::is_active() {
                continue;
            }
            // Retried every tick until it succeeds.
            if !recovered {
                recovered = self.fail_interrupted().await;
            }
            self.check_readbacks().await;
            if let Err(e) = self.process_queue().await {
                tracing::warn!("Command queue processor error: {}", e);
            }
        }
    }

    /// Commands a previous run left mid-delivery are failed, never resent.
    /// False when the sweep could not run (it is tried again).
    async fn fail_interrupted(&self) -> bool {
        match self.storage.fail_interrupted_commands().await {
            Ok(0) => true,
            Ok(n) => {
                tracing::warn!(
                    "{n} command(s) were being sent when the server stopped; marked failed"
                );
                true
            }
            Err(e) => {
                tracing::warn!("Could not resolve interrupted commands: {e}");
                false
            }
        }
    }

    async fn process_queue(&self) -> anyhow::Result<()> {
        let commands = self.storage.get_queued_commands().await?;
        // Devices whose oldest due command has been looked at: a device takes
        // its commands in order, so a newer one never overtakes an older one
        // that is waiting out a retry backoff (and later overwrites it).
        let mut device_seen = std::collections::HashSet::new();

        for cmd in &commands {
            let now = crate::timeutil::now_secs();
            tracing::info!(
                "Processing command: id={}, ds={}, device={}, protocol={}, status={}, value={}",
                cmd.id,
                cmd.data_source_id,
                cmd.device_id,
                cmd.protocol,
                cmd.status,
                cmd.value
            );

            // -- Expiry check --
            // A command that has not reached the device within the expiry window
            // is dropped, whether it is still pending (the device was offline)
            // or queued for a retry: a setpoint must never land hours late. A
            // scheduled command's window starts at its scheduled time.
            let due_at = cmd
                .scheduled_at
                .unwrap_or(cmd.created_at)
                .max(cmd.created_at);
            if matches!(cmd.status.as_str(), "pending" | "queued") && is_expired(due_at, now) {
                self.publish_status_event(&cmd.id, "expired", None);
                // Logged, not returned: devices already claimed in this pass
                // must still be delivered and released.
                if let Err(e) = self
                    .storage
                    .update_command_status(
                        &cmd.id,
                        "expired",
                        Some("Command expired: not delivered within 30 minutes"),
                        "system",
                        Some("auto-expired"),
                    )
                    .await
                {
                    tracing::warn!("Could not expire command {}: {e}", cmd.id);
                }
                continue;
            }

            // -- Scheduled check --
            if cmd.policy == "scheduled"
                && let Some(scheduled_at) = cmd.scheduled_at
                && scheduled_at > now
            {
                continue; // Not yet time
            }

            // -- Order: only the oldest due command of a device is considered --
            if !device_seen.insert(cmd.device_id) {
                continue;
            }

            // -- Retry backoff: don't hammer a failing device. --
            // Exponential backoff off retry_count (2^n s, capped at 60s). sent_at
            // records the last send and survives the queued→sent→queued retry
            // cycle, so skip until the backoff window elapses.
            if cmd.retry_count > 0
                && let Some(last_sent) = cmd.sent_at
            {
                let backoff_secs = 2_i64.pow(cmd.retry_count.min(6) as u32).min(60);
                if last_sent + backoff_secs > now {
                    continue;
                }
            }

            // -- Concurrency check: only one command per device in-flight --
            if self.in_flight.read().await.contains(&cmd.device_id) {
                continue;
            }

            // -- Resolve the text device ID from the data source for cache lookups --
            let ds = self
                .storage
                .get_data_source_by_id(cmd.data_source_id)
                .await
                .ok()
                .flatten();
            let cache_device_id = ds
                .as_ref()
                .map(|d| crate::alarm_eval::cache_ref(d).0)
                .unwrap_or_else(|| {
                    if cmd.device_ref.is_empty() {
                        cmd.device_id.to_string()
                    } else {
                        cmd.device_ref.clone()
                    }
                });

            // -- Device online check --
            if !self.cache.is_device_online(&cache_device_id).await {
                tracing::warn!(
                    "Skipping command {}: device '{}' (cache key '{}') is offline or not in cache",
                    cmd.id,
                    cmd.device_id,
                    cache_device_id
                );
                continue;
            }

            // Claim the device and deliver in a task of its own, released
            // when it finishes: devices do not wait for each other, and the
            // queue keeps serving the others while one is slow.
            if !self.in_flight.write().await.insert(cmd.device_id) {
                continue;
            }
            let queue = self.clone();
            let cmd = cmd.clone();
            tokio::spawn(async move {
                queue
                    .with_in_flight(cmd.device_id, queue.deliver(&cmd, ds, &cache_device_id))
                    .await;
            });
        }

        Ok(())
    }

    /// Run `work` for a device already marked in flight, then release the
    /// device whatever the outcome, so an error never leaves it wedged.
    pub(super) async fn with_in_flight<F>(&self, device_id: i64, work: F)
    where
        F: std::future::Future<Output = anyhow::Result<()>>,
    {
        let result = work.await;
        self.in_flight.write().await.remove(&device_id);
        if let Err(e) = result {
            tracing::warn!("Command delivery to device {device_id} failed: {e:#}");
        }
    }

    /// Send one command to its device and record the outcome.
    async fn deliver(
        &self,
        cmd: &crate::models::Command,
        ds: Option<crate::models::DataSource>,
        cache_device_id: &str,
    ) -> anyhow::Result<()> {
        // -- Execute --
        self.publish_status_event(&cmd.id, "sent", None);
        self.storage
            .update_command_status(&cmd.id, "sent", None, "system", None)
            .await?;

        let started = std::time::Instant::now();
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(WRITE_TIMEOUT_SECS),
            self.execute_write(cmd),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => crate::models::WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Write timed out after {WRITE_TIMEOUT_SECS}s")),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        };

        // -- Handle result --
        if result.success {
            tracing::info!(
                "Command acked: id={}, response={}, latency={}ms",
                cmd.id,
                result.device_response.as_deref().unwrap_or("OK"),
                result.latency_ms
            );
            let response = result
                .device_response
                .clone()
                .unwrap_or_else(|| "OK".to_string());
            self.publish_ack_event(&cmd.id, &response, result.latency_ms);
            // The device took the write: losing that to a passing storage
            // error would leave the command "sent" for good, so try again.
            let mut attempt = 1;
            while let Err(e) = self
                .storage
                .update_command_status(&cmd.id, "acked", None, "system", None)
                .await
            {
                if attempt >= ACK_RECORD_ATTEMPTS {
                    return Err(e.context(format!(
                        "command {} was acknowledged by the device but could not be recorded",
                        cmd.id
                    )));
                }
                tracing::warn!(
                    "Recording the ack of command {} failed, retrying: {e}",
                    cmd.id
                );
                tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)))
                    .await;
                attempt += 1;
            }

            // The cache keeps showing what the device last reported: an ack
            // says the device took the write, not that the value is in
            // effect. The next poll confirms it.
            if let Some(ds) = ds
                && let Ok(value) = cmd.value.parse::<f64>()
            {
                self.start_readback(cmd, &ds, cache_device_id, value).await;
            }
        } else {
            let error = result.error.as_deref().unwrap_or("Unknown error");
            tracing::warn!(
                "Command failed: id={}, error={}, retry={}/{}",
                cmd.id,
                error,
                cmd.retry_count,
                cmd.max_retries
            );
            if cmd.retry_count < cmd.max_retries {
                self.publish_status_event(&cmd.id, "queued", Some(error));
                self.storage
                    .update_command_status(&cmd.id, "queued", Some(error), "system", Some("retry"))
                    .await?;
            } else {
                self.publish_status_event(&cmd.id, "failed", Some(error));
                self.storage
                    .update_command_status(&cmd.id, "failed", Some(error), "system", None)
                    .await?;
            }
        }
        Ok(())
    }
}

impl CommandQueue {
    /// Wait for a poll to read back the value an acknowledged write set.
    async fn start_readback(
        &self,
        cmd: &crate::models::Command,
        ds: &crate::models::DataSource,
        cache_device_id: &str,
        value: f64,
    ) {
        let deadband = self
            .storage
            .get_write_policy(ds.id)
            .await
            .ok()
            .flatten()
            .and_then(|p| p.write_deadband);
        let interval = self.cache.poll_interval(cache_device_id).await;
        let tracker = readback::PendingReadback::new(
            cmd.id.clone(),
            cache_device_id.to_string(),
            crate::alarm_eval::cache_ref(ds).1,
            value,
            readback::tolerance(value, deadband),
            crate::timeutil::now_millis(),
            readback::timeout_ms(interval),
        );
        self.readback_trackers
            .write()
            .await
            .insert(cmd.id.clone(), tracker);
    }

    /// Compare every pending read-back with the cache; record and announce
    /// the ones that are decided. Reads the cache only, so the queue loop
    /// never waits on a device here.
    async fn check_readbacks(&self) {
        let now_ms = crate::timeutil::now_millis();
        let mut decided = Vec::new();
        {
            let mut trackers = self.readback_trackers.write().await;
            if trackers.is_empty() {
                return;
            }
            for tracker in trackers.values_mut() {
                let reading = self
                    .cache
                    .get_reading(&tracker.device_id, &tracker.metric)
                    .await;
                if let Some(outcome) = tracker.check(reading, now_ms) {
                    decided.push((tracker.command_id.clone(), tracker.attempts, outcome));
                }
            }
            for (id, _, _) in &decided {
                trackers.remove(id);
            }
        }
        for (command_id, attempts, outcome) in decided {
            let (value, ts) = match outcome {
                readback::Outcome::Confirmed { value, ts_ms }
                | readback::Outcome::Mismatch { value, ts_ms } => {
                    (Some(value), Some(iso_millis(ts_ms)))
                }
                readback::Outcome::Timeout => (None, None),
            };
            if !matches!(outcome, readback::Outcome::Confirmed { .. }) {
                tracing::warn!(
                    "Command {command_id}: write read-back {} (read {value:?})",
                    outcome.status()
                );
            }
            if let Err(e) = self
                .storage
                .update_command_readback(&command_id, outcome.status(), value, attempts, ts)
                .await
            {
                tracing::warn!("Failed to record read-back of command {command_id}: {e}");
            }
            self.publish_readback_event(&command_id, &outcome);
        }
    }
}

/// Unix milliseconds as ISO 8601.
fn iso_millis(ts_ms: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ts_ms) * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Iso8601::DEFAULT)
                .ok()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_expire_after_the_window_from_when_they_were_due() {
        let now = 1_000_000;
        assert!(!is_expired(now - COMMAND_EXPIRY_SECS, now));
        assert!(is_expired(now - COMMAND_EXPIRY_SECS - 1, now));
        // A command scheduled in the future is not expired.
        assert!(!is_expired(now + 60, now));
    }

    fn queue(storage: Storage, cache: DeviceCache) -> CommandQueue {
        #[cfg(feature = "modbus")]
        return CommandQueue::new(storage, cache, Arc::new(ModbusPool::new()));
        #[cfg(not(feature = "modbus"))]
        return CommandQueue::new(storage, cache);
    }

    async fn acked_command(storage: &Storage, id: &str) {
        let cmd = crate::models::Command {
            id: id.into(),
            data_source_id: 1,
            device_id: 0,
            protocol: "modbus".into(),
            value: "42".into(),
            status: "acked".into(),
            policy: "immediate".into(),
            scheduled_at: None,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at: 1,
            sent_at: None,
            acked_at: None,
            resolved_at: None,
            created_by: "admin".into(),
            device_ref: "plc".into(),
            readback_status: None,
            readback_value: None,
            readback_attempts: 0,
            readback_ts: None,
            origin: None,
            source: None,
        };
        storage.insert_command(&cmd).await.unwrap();
    }

    async fn track(q: &CommandQueue, id: &str, acked_at_ms: i64) {
        let tracker = readback::PendingReadback::new(
            id.into(),
            "plc".into(),
            "sp".into(),
            42.0,
            readback::tolerance(42.0, None),
            acked_at_ms,
            readback::timeout_ms(None),
        );
        q.readback_trackers.write().await.insert(id.into(), tracker);
    }

    #[tokio::test]
    async fn a_fresh_poll_confirms_the_write_and_is_recorded() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        let q = queue(storage.clone(), cache.clone());
        acked_command(&storage, "c1").await;
        let now = crate::timeutil::now_millis();
        track(&q, "c1", now - 1_000).await;

        // Nothing read yet: still waiting.
        q.check_readbacks().await;
        assert_eq!(q.readback_trackers.read().await.len(), 1);

        cache
            .update(
                "plc",
                std::collections::HashMap::from([("sp".into(), 42.0)]),
                false,
            )
            .await;
        q.check_readbacks().await;
        assert!(q.readback_trackers.read().await.is_empty());
        let cmd = storage.get_command("c1").await.unwrap().unwrap();
        assert_eq!(cmd.readback_status.as_deref(), Some("confirmed"));
        assert_eq!(cmd.readback_value, Some(42.0));
        assert_eq!(cmd.readback_attempts, 1);
        assert!(cmd.readback_ts.unwrap().starts_with("20"));
    }

    #[tokio::test]
    async fn no_poll_before_the_deadline_is_a_timeout() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        let q = queue(storage.clone(), cache.clone());
        acked_command(&storage, "c2").await;
        // Acked long enough ago that the 10 s window is over.
        track(&q, "c2", crate::timeutil::now_millis() - 60_000).await;
        q.check_readbacks().await;
        let cmd = storage.get_command("c2").await.unwrap().unwrap();
        assert_eq!(cmd.readback_status.as_deref(), Some("timeout"));
        assert_eq!(cmd.readback_value, None);
    }

    #[tokio::test]
    async fn a_failed_delivery_releases_the_device() {
        let q = queue(Storage::new_in_memory().unwrap(), DeviceCache::new());
        q.in_flight.write().await.insert(7);
        q.with_in_flight(7, async { anyhow::bail!("storage unavailable") })
            .await;
        assert!(q.in_flight.read().await.is_empty());
    }

    #[tokio::test]
    async fn commands_cut_off_by_a_restart_are_failed_not_resent() {
        let storage = Storage::new_in_memory().unwrap();
        acked_command(&storage, "c3").await;
        storage
            .update_command_status("c3", "sent", None, "system", None)
            .await
            .unwrap();
        let q = queue(storage.clone(), DeviceCache::new());
        q.fail_interrupted().await;
        let cmd = storage.get_command("c3").await.unwrap().unwrap();
        assert_eq!(cmd.status, "failed");
        assert!(cmd.error_message.unwrap().contains("restart"));
        assert!(storage.get_queued_commands().await.unwrap().is_empty());
        let audit = storage.get_command_audit("c3").await.unwrap();
        assert!(audit.iter().any(|a| a.new_status == "failed"));
    }

    fn queued_command(id: &str, created_at: i64) -> crate::models::Command {
        crate::models::Command {
            id: id.into(),
            data_source_id: 999,
            device_id: 5,
            protocol: "modbus".into(),
            value: "1".into(),
            status: "pending".into(),
            policy: "immediate".into(),
            scheduled_at: None,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at,
            sent_at: None,
            acked_at: None,
            resolved_at: None,
            created_by: "admin".into(),
            device_ref: "plc".into(),
            readback_status: None,
            readback_value: None,
            readback_attempts: 0,
            readback_ts: None,
            origin: None,
            source: None,
        }
    }

    #[tokio::test]
    async fn a_newer_command_waits_for_an_older_one_in_backoff() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        cache
            .update(
                "plc",
                std::collections::HashMap::from([("sp".into(), 1.0)]),
                false,
            )
            .await;
        cache
            .set_status("plc", crate::models::DeviceStatus::Online)
            .await;
        let now = crate::timeutil::now_secs();
        // The older command failed once just now and is backing off.
        let mut older = queued_command("old", now - 10);
        older.status = "queued".into();
        older.retry_count = 1;
        older.sent_at = Some(now);
        storage.insert_command(&older).await.unwrap();
        storage
            .insert_command(&queued_command("new", now - 5))
            .await
            .unwrap();

        let q = queue(storage.clone(), cache);
        q.process_queue().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let newer = storage.get_command("new").await.unwrap().unwrap();
        assert_eq!(
            newer.status, "pending",
            "the newer command must not overtake"
        );
        assert_eq!(newer.sent_at, None);
        assert!(q.in_flight.read().await.is_empty());
    }

    #[tokio::test]
    async fn the_oldest_command_of_a_device_goes_first() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        cache
            .update(
                "plc",
                std::collections::HashMap::from([("sp".into(), 1.0)]),
                false,
            )
            .await;
        cache
            .set_status("plc", crate::models::DeviceStatus::Online)
            .await;
        let now = crate::timeutil::now_secs();
        storage
            .insert_command(&queued_command("a", now - 10))
            .await
            .unwrap();
        storage
            .insert_command(&queued_command("b", now - 5))
            .await
            .unwrap();
        let q = queue(storage.clone(), cache);
        q.process_queue().await.unwrap();
        // The write itself fails (no such tag); what matters is which was tried.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !q.in_flight.read().await.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let a = storage.get_command("a").await.unwrap().unwrap();
        let b = storage.get_command("b").await.unwrap().unwrap();
        assert!(a.sent_at.is_some(), "the oldest was sent");
        assert_eq!(b.sent_at, None, "the newer one waits");
    }

    #[tokio::test]
    async fn the_restart_sweep_reports_success() {
        let q = queue(Storage::new_in_memory().unwrap(), DeviceCache::new());
        assert!(q.fail_interrupted().await);
    }
}
