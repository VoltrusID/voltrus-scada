use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use opcua::client::{ClientBuilder, DataChangeCallback, IdentityToken, MonitoredItem};
use opcua::types::{
    AttributeId, DataValue, MessageSecurityMode, MonitoringMode, MonitoringParameters, NodeId,
    NumericRange, ReadValueId, TimestampsToReturn, UserTokenPolicy, Variant,
};
use tokio::sync::mpsc;

use crate::cache::DeviceCache;
use crate::models::*;

/// Main entry point for OPC-UA device polling.
///
/// Spawned as `tokio::spawn(opcua::poll_opcua_device(config, cache))`.
/// Runs an infinite loop with exponential backoff on connection failure.
pub async fn poll_opcua_device(config: OpcUaDeviceConfig, cache: DeviceCache) {
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        match try_connect_and_subscribe(&config, &cache).await {
            Ok(()) => {
                backoff.reset();
                // Connection lost or session ended -- retry
                tracing::warn!("OPC-UA device {} connection lost, reconnecting", device_id);
                cache.set_status(&device_id, DeviceStatus::Offline).await;
            }
            Err(e) => {
                tracing::warn!("OPC-UA device {} error: {e}", device_id);
                cache.set_status(&device_id, DeviceStatus::Offline).await;
                let delay = backoff.next();
                tracing::info!("OPC-UA device {} retrying in {delay}ms", device_id);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}

/// Attempts to connect to the OPC-UA server, create a subscription,
/// and monitor nodes. Returns Ok(()) when the session/event-loop ends
/// (e.g. connection drop), or Err on setup failure.
async fn try_connect_and_subscribe(config: &OpcUaDeviceConfig, cache: &DeviceCache) -> Result<()> {
    let device_id = config.id.clone();

    // --- Build client ---
    let mut client = ClientBuilder::new()
        .application_name("Voltrus")
        .application_uri("urn:voltrus-lite")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .client()
        .map_err(|e| anyhow::anyhow!("Client build errors: {:?}", e))?;

    // --- Configure endpoint ---
    let security_policy_str = security_policy_to_str(&config.security_policy);
    let message_security_mode = security_mode_to_opcua(&config.security_mode);

    let user_token_policy = match (&config.username, &config.password) {
        (Some(_), Some(_)) => {
            // UserTokenPolicy doesn't have a user_name() constructor in 0.18.0,
            // so we build it directly for the endpoint description.
            UserTokenPolicy {
                policy_id: opcua::types::UAString::from("voltrus-credential"),
                token_type: opcua::types::UserTokenType::UserName,
                issued_token_type: opcua::types::UAString::null(),
                issuer_endpoint_url: opcua::types::UAString::null(),
                security_policy_uri: opcua::types::UAString::null(),
            }
        }
        _ => UserTokenPolicy::anonymous(),
    };

    let endpoint = opcua::types::EndpointDescription::from((
        config.url.as_str(),
        security_policy_str,
        message_security_mode,
        user_token_policy,
    ));

    // --- Identity token ---
    let identity_token = match (&config.username, &config.password) {
        (Some(user), Some(pass)) => IdentityToken::new_user_name(user.clone(), pass.clone()),
        _ => IdentityToken::Anonymous,
    };

    // --- Connect ---
    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, identity_token)
        .await
        .context("OPC-UA connection failed")?;

    let mut event_handle = event_loop.spawn();

    // wait_for_connection() returns bool -- false means it failed to connect
    tokio::select! {
        connected = session.wait_for_connection() => {
            if !connected {
                anyhow::bail!("OPC-UA session failed to connect");
            }
        }
        event_result = &mut event_handle => {
            let code = event_result
                .context("OPC-UA event loop task panicked")?;
            anyhow::bail!("OPC-UA event loop ended with status {code}");
        }
    }

    tracing::info!("OPC-UA device {} connected to {}", device_id, config.url);

    // --- Create subscription ---
    let interval_ms = config.interval_ms.max(100);
    let subscription_id = session
        .create_subscription(
            Duration::from_millis(interval_ms),
            90,   // lifetime_count (3x max_keep_alive_count per OPC-UA spec)
            30,   // max_keep_alive_count
            0,    // max_notifications_per_publish
            0,    // priority
            true, // publishing_enabled
            DataChangeCallback::new(create_data_change_callback(
                device_id.clone(),
                cache.clone(),
                &config.nodes,
            )),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create subscription: {:?}", e))?;

    tracing::info!(
        "OPC-UA device {} subscription created (id={})",
        device_id,
        subscription_id
    );

    // --- Create monitored items ---
    let items_to_create: Vec<opcua::types::MonitoredItemCreateRequest> = config
        .nodes
        .iter()
        .filter_map(|node| {
            NodeId::from_str(&node.node_id)
                .ok()
                .map(|node_id| (node_id, node.clone()))
        })
        .map(|(node_id, _node)| {
            opcua::types::MonitoredItemCreateRequest {
                item_to_monitor: ReadValueId {
                    node_id,
                    attribute_id: AttributeId::Value as u32,
                    index_range: NumericRange::None,
                    data_encoding: opcua::types::QualifiedName::null(),
                },
                monitoring_mode: MonitoringMode::Reporting,
                requested_parameters: MonitoringParameters {
                    client_handle: 0, // will be assigned by the session
                    sampling_interval: interval_ms as f64,
                    queue_size: 10,
                    discard_oldest: true,
                    filter: opcua::types::ExtensionObject::null(),
                },
            }
        })
        .collect();

    if items_to_create.is_empty() {
        tracing::warn!("OPC-UA device {} has no valid nodes to monitor", device_id);
        return Ok(());
    }

    let results = session
        .create_monitored_items(subscription_id, TimestampsToReturn::Source, items_to_create)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create monitored items: {:?}", e))?;

    let mut success_count = 0;
    for (i, result) in results.iter().enumerate() {
        if result.result.status_code.is_good() {
            success_count += 1;
        } else {
            tracing::warn!(
                "OPC-UA device {} monitored item {} creation failed: {:?}",
                device_id,
                i,
                result.result.status_code
            );
        }
    }

    tracing::info!(
        "OPC-UA device {} created {}/{} monitored items",
        device_id,
        success_count,
        results.len()
    );

    cache.set_status(&device_id, DeviceStatus::Online).await;

    // Wait for the event loop to end (connection lost, error, etc.)
    let _ = event_handle.await;

    Ok(())
}

/// Creates a `DataChangeCallback` closure that extracts values from
/// OPC-UA `DataValue` updates and pushes them into the `DeviceCache`.
///
/// Uses an mpsc channel so that high-frequency data changes (100+ tags at
/// 100 ms) are drained by a single consumer task instead of spawning one
/// tokio task per value change.
fn create_data_change_callback(
    device_id: String,
    cache: DeviceCache,
    nodes: &[OpcUaNodeConfig],
) -> impl FnMut(DataValue, &MonitoredItem) + Send + Sync + 'static {
    // Build a mapping from node_id string -> node name for lookup
    let node_id_to_name: Arc<HashMap<String, String>> = Arc::new(
        nodes
            .iter()
            .filter_map(|n| {
                NodeId::from_str(&n.node_id)
                    .ok()
                    .map(|id: NodeId| (id.to_string(), n.name.clone()))
            })
            .collect(),
    );

    // Bounded channel: 1024 slots is generous for a single device.
    // If the consumer falls behind, the oldest updates are simply dropped
    // (the next update will overwrite the same metric anyway).
    let (tx, mut rx) = mpsc::channel::<(String, f64)>(1024);

    // Single consumer task batches per-value updates into cache writes.
    {
        let cache = cache.clone();
        let device_id = device_id.clone();
        tokio::spawn(async move {
            // Accumulate metrics between drain cycles.
            let mut batch: HashMap<String, f64> = HashMap::new();

            loop {
                // Block until at least one update arrives.
                match rx.recv().await {
                    Some((name, value)) => {
                        batch.insert(name, value);
                    }
                    None => break, // channel closed — subscription gone
                }

                // Drain everything that is already buffered so we can
                // collapse multiple updates to the same tag into one
                // cache write.
                while let Ok((name, value)) = rx.try_recv() {
                    batch.insert(name, value);
                }

                let metrics = std::mem::take(&mut batch);
                cache.update(&device_id, metrics, false).await;
            }
        });
    }

    move |data_value: DataValue, item: &MonitoredItem| {
        let node_id_str = item.item_to_monitor().node_id.to_string();
        let node_name = node_id_to_name
            .get(&node_id_str)
            .cloned()
            .unwrap_or_else(|| node_id_str.clone());

        let Some(ref variant) = data_value.value else {
            return;
        };

        let value = match variant_to_f64(variant) {
            Some(v) => v,
            None => return,
        };

        // Non-blocking send; if the channel is full the update is dropped
        // (the next publish cycle will deliver a fresher value anyway).
        let _ = tx.try_send((node_name, value));
    }
}

/// Extract an f64 from an OPC-UA Variant.
///
/// - Boolean -> 0.0 or 1.0
/// - Integer types (SByte, Int16, Int32, Int64, Byte, UInt16, UInt32, UInt64) -> f64
/// - Float / Double -> f64
/// - String and other types -> None (skip)
fn variant_to_f64(variant: &Variant) -> Option<f64> {
    match variant {
        Variant::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        Variant::SByte(v) => Some(*v as f64),
        Variant::Byte(v) => Some(*v as f64),
        Variant::Int16(v) => Some(*v as f64),
        Variant::UInt16(v) => Some(*v as f64),
        Variant::Int32(v) => Some(*v as f64),
        Variant::UInt32(v) => Some(*v as f64),
        Variant::Int64(v) => Some(*v as f64),
        Variant::UInt64(v) => Some(*v as f64),
        Variant::Float(v) => Some(*v as f64),
        Variant::Double(v) => Some(*v),
        // Skip non-numeric types -- use a catch-all since Variant is non-exhaustive
        _ => None,
    }
}

/// Convert our `OpcUaSecurityPolicy` to the string expected by
/// `EndpointDescription`'s From impl (security policy URI).
pub(crate) fn security_policy_to_str(policy: &OpcUaSecurityPolicy) -> &'static str {
    match policy {
        OpcUaSecurityPolicy::None => "http://opcfoundation.org/UA/SecurityPolicy#None",
        OpcUaSecurityPolicy::Basic256Sha256 => {
            "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256"
        }
    }
}

/// Convert our `OpcUaSecurityMode` to the OPC-UA `MessageSecurityMode`.
pub(crate) fn security_mode_to_opcua(mode: &OpcUaSecurityMode) -> MessageSecurityMode {
    match mode {
        OpcUaSecurityMode::None => MessageSecurityMode::None,
        OpcUaSecurityMode::Sign => MessageSecurityMode::Sign,
        OpcUaSecurityMode::SignAndEncrypt => MessageSecurityMode::SignAndEncrypt,
    }
}

// ── One-shot write function ──

/// Write a single f64 value to an OPC UA node.
///
/// This is currently a stub because the `async-opcua` 0.18.0 client requires a
/// running session event-loop for write operations. The subscription-based
/// polling model in `try_connect_and_subscribe` holds a long-lived session;
/// performing an ad-hoc write requires building a separate client + event loop
/// which adds significant complexity.
///
/// Returns `WriteResult` with `success: false` and an explanatory error.
pub async fn write_tag(_endpoint: &str, _node_id: &str, _value: f64) -> WriteResult {
    WriteResult {
        success: false,
        device_response: None,
        error: Some("OPC UA write not yet supported: requires session-based write service (async-opcua 0.18.0)".into()),
        latency_ms: 0,
    }
}

/// Exponential backoff for connection retries.
struct Backoff {
    attempt: u32,
    max_delay_ms: u64,
}

impl Backoff {
    fn new() -> Self {
        Self {
            attempt: 0,
            max_delay_ms: 30_000,
        }
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn next(&mut self) -> u64 {
        let attempt = self.attempt.min(30);
        let delay = 1000 * 2u64.pow(attempt);
        self.attempt = self.attempt.saturating_add(1);
        delay.min(self.max_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variant_to_f64_boolean() {
        assert_eq!(variant_to_f64(&Variant::Boolean(true)), Some(1.0));
        assert_eq!(variant_to_f64(&Variant::Boolean(false)), Some(0.0));
    }

    #[test]
    fn test_variant_to_f64_integers() {
        assert_eq!(variant_to_f64(&Variant::Int32(42)), Some(42.0));
        assert_eq!(variant_to_f64(&Variant::Int32(-5)), Some(-5.0));
        assert_eq!(variant_to_f64(&Variant::Int16(-100)), Some(-100.0));
        assert_eq!(variant_to_f64(&Variant::UInt32(300)), Some(300.0));
        assert_eq!(variant_to_f64(&Variant::Int64(999999)), Some(999999.0));
        assert_eq!(variant_to_f64(&Variant::SByte(-1)), Some(-1.0));
        assert_eq!(variant_to_f64(&Variant::Byte(255)), Some(255.0));
    }

    #[test]
    fn test_variant_to_f64_floats() {
        let val = variant_to_f64(&Variant::Double(3.14159));
        assert!((val.unwrap() - 3.14159).abs() < 1e-10);

        let val = variant_to_f64(&Variant::Float(2.5));
        assert!((val.unwrap() - 2.5_f32 as f64).abs() < 1e-5);
    }

    #[test]
    fn test_variant_to_f64_string_skipped() {
        assert_eq!(
            variant_to_f64(&Variant::String(opcua::types::UAString::from("hello"))),
            None
        );
    }

    #[test]
    fn test_variant_to_f64_empty_skipped() {
        assert_eq!(variant_to_f64(&Variant::Empty), None);
    }

    #[test]
    fn test_security_policy_to_str() {
        assert_eq!(
            security_policy_to_str(&OpcUaSecurityPolicy::None),
            "http://opcfoundation.org/UA/SecurityPolicy#None"
        );
        assert_eq!(
            security_policy_to_str(&OpcUaSecurityPolicy::Basic256Sha256),
            "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256"
        );
    }

    #[test]
    fn test_security_mode_to_opcua() {
        assert_eq!(
            security_mode_to_opcua(&OpcUaSecurityMode::None),
            MessageSecurityMode::None
        );
        assert_eq!(
            security_mode_to_opcua(&OpcUaSecurityMode::Sign),
            MessageSecurityMode::Sign
        );
        assert_eq!(
            security_mode_to_opcua(&OpcUaSecurityMode::SignAndEncrypt),
            MessageSecurityMode::SignAndEncrypt
        );
    }

    #[test]
    fn test_backoff_sequence() {
        let mut b = Backoff::new();
        assert_eq!(b.next(), 1000);
        assert_eq!(b.next(), 2000);
        assert_eq!(b.next(), 4000);
    }

    #[test]
    fn test_backoff_resets() {
        let mut b = Backoff::new();
        b.next();
        b.next();
        b.reset();
        assert_eq!(b.next(), 1000);
    }

    #[test]
    fn test_backoff_max() {
        let mut b = Backoff::new();
        for _ in 0..20 {
            b.next();
        }
        assert_eq!(b.next(), 30_000);
    }
}
