use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use opcua::client::{ClientBuilder, DataChangeCallback, IdentityToken, MonitoredItem, Session};
use opcua::types::{
    AttributeId, DataValue, MessageSecurityMode, MonitoringMode, MonitoringParameters, NodeId,
    NumericRange, ReadValueId, StatusCode, TimestampsToReturn, UserTokenPolicy, Variant,
    WriteValue,
};
use tokio::sync::mpsc;

use crate::cache::DeviceCache;
use crate::models::*;
use crate::quality::Quality;

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
    // Without message security the server certificate protects nothing, so any
    // is accepted. With Sign or SignAndEncrypt the certificate is what proves
    // the server is the right one: it must already be in `pki/trusted` (under
    // the working directory). An unknown one is refused and copied to
    // `pki/rejected`, from where an administrator moves it to `pki/trusted`.
    let accept_any_server_cert = matches!(config.security_mode, OpcUaSecurityMode::None);
    let mut client = ClientBuilder::new()
        .application_name("Voltrus")
        .application_uri("urn:voltrus-lite")
        .trust_server_certs(accept_any_server_cert)
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
        .map_err(|e| {
            if accept_any_server_cert {
                anyhow::anyhow!("OPC-UA connection failed: {e}")
            } else {
                anyhow::anyhow!(
                    "OPC-UA connection failed: {e} (a secure endpoint needs the server \
                     certificate in pki/trusted; a refused one is in pki/rejected)"
                )
            }
        })?;

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

    // Queued writes go through this session while it lives.
    let _registration = SessionRegistration::register(&device_id, session.clone());

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
    let (tx, mut rx) = mpsc::channel::<(String, f64, Quality)>(1024);

    // Single consumer task batches per-value updates into cache writes.
    {
        let cache = cache.clone();
        let device_id = device_id.clone();
        tokio::spawn(async move {
            // Accumulate metrics between drain cycles.
            let mut batch: HashMap<String, f64> = HashMap::new();
            let mut quality: HashMap<String, Quality> = HashMap::new();

            loop {
                // Block until at least one update arrives.
                match rx.recv().await {
                    Some((name, value, q)) => {
                        quality.insert(name.clone(), q);
                        batch.insert(name, value);
                    }
                    None => break, // channel closed — subscription gone
                }

                // Drain everything that is already buffered so we can
                // collapse multiple updates to the same tag into one
                // cache write.
                while let Ok((name, value, q)) = rx.try_recv() {
                    quality.insert(name.clone(), q);
                    batch.insert(name, value);
                }

                let metrics = std::mem::take(&mut batch);
                let quality = std::mem::take(&mut quality);
                // A subscription only reports changes: merge, so the tags
                // that did not change keep their value.
                cache.merge_with_quality(&device_id, metrics, quality).await;
            }
        });
    }

    move |data_value: DataValue, item: &MonitoredItem| {
        let node_id_str = item.item_to_monitor().node_id.to_string();
        let node_name = node_id_to_name
            .get(&node_id_str)
            .cloned()
            .unwrap_or_else(|| node_id_str.clone());

        let Some((value, quality)) = reading_of(&data_value) else {
            return;
        };

        // Non-blocking send; if the channel is full the update is dropped
        // (the next publish cycle will deliver a fresher value anyway).
        let _ = tx.try_send((node_name, value, quality));
    }
}

/// The value and quality a data change carries. A bad status yields NaN
/// whatever value came with it (servers often repeat the last one); a value
/// we cannot turn into a number is skipped unless the status already says bad.
fn reading_of(data_value: &DataValue) -> Option<(f64, Quality)> {
    let quality = data_value
        .status
        .map(status_quality)
        .unwrap_or(Quality::Good);
    if quality.is_bad() {
        return Some((f64::NAN, quality));
    }
    let value = data_value.value.as_ref().and_then(variant_to_f64)?;
    Some((value, quality))
}

/// Map an OPC UA status code to our quality, keeping the reasons we act on.
fn status_quality(status: StatusCode) -> Quality {
    if status.is_good() {
        return Quality::Good;
    }
    if status.is_uncertain() {
        return if status.sub_code() == StatusCode::UncertainLastUsableValue.sub_code() {
            Quality::UncertainStale
        } else {
            Quality::Uncertain
        };
    }
    let sub = status.sub_code();
    let is = |code: StatusCode| sub == code.sub_code();
    if is(StatusCode::BadCommunicationError)
        || is(StatusCode::BadNoCommunication)
        || is(StatusCode::BadTimeout)
        || is(StatusCode::BadServerNotConnected)
        || is(StatusCode::BadWaitingForInitialData)
    {
        Quality::BadCommFailure
    } else if is(StatusCode::BadConfigurationError)
        || is(StatusCode::BadNodeIdUnknown)
        || is(StatusCode::BadNotConnected)
    {
        Quality::BadConfigError
    } else if is(StatusCode::BadDeviceFailure)
        || is(StatusCode::BadSensorFailure)
        || is(StatusCode::BadOutOfService)
    {
        Quality::BadDeviceError
    } else {
        Quality::Bad
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

// ── Writes through the poller's session ──

/// A connected device's session, shared by its poller (subscriptions) and
/// queued writes, with the data type of each node written so far.
struct LiveSession {
    session: Arc<Session>,
    data_types: tokio::sync::Mutex<HashMap<NodeId, ScalarType>>,
}

/// Live sessions by poller device id (`opcua_<id>`).
static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<LiveSession>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Keeps a session in [`SESSIONS`] for as long as its poller holds it. Dropped
/// when the connection ends or the poller task is stopped, it removes the entry
/// unless a newer session has replaced it.
struct SessionRegistration {
    device_id: String,
    live: Arc<LiveSession>,
}

impl SessionRegistration {
    fn register(device_id: &str, session: Arc<Session>) -> Self {
        let live = Arc::new(LiveSession {
            session,
            data_types: tokio::sync::Mutex::new(HashMap::new()),
        });
        SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.to_string(), live.clone());
        Self {
            device_id: device_id.to_string(),
            live,
        }
    }
}

impl Drop for SessionRegistration {
    fn drop(&mut self) {
        let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        if sessions
            .get(&self.device_id)
            .is_some_and(|live| Arc::ptr_eq(live, &self.live))
        {
            sessions.remove(&self.device_id);
        }
    }
}

/// The scalar types a numeric value can be written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarType {
    Boolean,
    SByte,
    Byte,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float,
    Double,
}

impl ScalarType {
    /// The type for a node's DataType attribute, for the built-in numeric
    /// types (namespace 0, i=1 to i=11) and enumerations (written as Int32).
    fn from_data_type(data_type: &NodeId) -> Option<Self> {
        if data_type.namespace != 0 {
            return None;
        }
        Some(match data_type.as_u32()? {
            1 => Self::Boolean,
            2 => Self::SByte,
            3 => Self::Byte,
            4 => Self::Int16,
            5 => Self::UInt16,
            6 => Self::Int32,
            7 => Self::UInt32,
            8 => Self::Int64,
            9 => Self::UInt64,
            10 => Self::Float,
            11 => Self::Double,
            29 => Self::Int32,
            _ => return None,
        })
    }

    /// The type of a value a node holds now, for nodes whose DataType is not a
    /// built-in one (a vendor subtype of Double, say).
    fn from_variant(value: &Variant) -> Option<Self> {
        Some(match value {
            Variant::Boolean(_) => Self::Boolean,
            Variant::SByte(_) => Self::SByte,
            Variant::Byte(_) => Self::Byte,
            Variant::Int16(_) => Self::Int16,
            Variant::UInt16(_) => Self::UInt16,
            Variant::Int32(_) => Self::Int32,
            Variant::UInt32(_) => Self::UInt32,
            Variant::Int64(_) => Self::Int64,
            Variant::UInt64(_) => Self::UInt64,
            Variant::Float(_) => Self::Float,
            Variant::Double(_) => Self::Double,
            _ => return None,
        })
    }

    /// `value` as this type. A server refuses a Value whose type differs from
    /// the node's (BadTypeMismatch), and an integer out of range must fail here
    /// rather than wrap.
    pub(crate) fn variant_for(self, value: f64) -> Result<Variant, String> {
        if !value.is_finite() {
            return Err(format!("Value {value} is not a finite number"));
        }
        let integer = |min: f64, max: f64| -> Result<f64, String> {
            let rounded = value.round();
            if rounded < min || rounded > max {
                Err(format!(
                    "Value {value} is outside the {self:?} range {min}..={max}"
                ))
            } else {
                Ok(rounded)
            }
        };
        Ok(match self {
            Self::Boolean => Variant::Boolean(value != 0.0),
            Self::SByte => Variant::SByte(integer(i8::MIN as f64, i8::MAX as f64)? as i8),
            Self::Byte => Variant::Byte(integer(0.0, u8::MAX as f64)? as u8),
            Self::Int16 => Variant::Int16(integer(i16::MIN as f64, i16::MAX as f64)? as i16),
            Self::UInt16 => Variant::UInt16(integer(0.0, u16::MAX as f64)? as u16),
            Self::Int32 => Variant::Int32(integer(i32::MIN as f64, i32::MAX as f64)? as i32),
            Self::UInt32 => Variant::UInt32(integer(0.0, u32::MAX as f64)? as u32),
            // i64::MAX is not exact in f64; the bound is the largest f64 below 2^63.
            Self::Int64 => {
                Variant::Int64(integer(-(2f64.powi(63)), 2f64.powi(63) - 1024.0)? as i64)
            }
            Self::UInt64 => Variant::UInt64(integer(0.0, 2f64.powi(64) - 2048.0)? as u64),
            Self::Float => {
                if value.abs() > f32::MAX as f64 {
                    return Err(format!("Value {value} is outside the Float range"));
                }
                Variant::Float(value as f32)
            }
            Self::Double => Variant::Double(value),
        })
    }
}

impl LiveSession {
    /// The node's type, read once (DataType attribute, else the current Value)
    /// and remembered for the session's life.
    async fn scalar_type(&self, node_id: &NodeId) -> Result<ScalarType, String> {
        if let Some(t) = self.data_types.lock().await.get(node_id) {
            return Ok(*t);
        }
        let read = |attribute: AttributeId| ReadValueId {
            node_id: node_id.clone(),
            attribute_id: attribute as u32,
            index_range: NumericRange::None,
            data_encoding: opcua::types::QualifiedName::null(),
        };
        let results = self
            .session
            .read(
                &[read(AttributeId::DataType), read(AttributeId::Value)],
                TimestampsToReturn::Neither,
                0.0,
            )
            .await
            .map_err(|status| format!("OPC UA read of {node_id} failed: {status}"))?;
        let attribute_status = |dv: &DataValue| dv.status.unwrap_or(StatusCode::Good);
        if let Some(dv) = results.first()
            && attribute_status(dv).is_bad()
        {
            return Err(format!("OPC UA node {node_id}: {}", attribute_status(dv)));
        }
        let from_data_type = results.first().and_then(|dv| match &dv.value {
            Some(Variant::NodeId(id)) => ScalarType::from_data_type(id),
            _ => None,
        });
        let from_value = || {
            results
                .get(1)
                .and_then(|dv| dv.value.as_ref())
                .and_then(ScalarType::from_variant)
        };
        let scalar = from_data_type.or_else(from_value).ok_or_else(|| {
            format!("OPC UA node {node_id} does not hold a numeric or boolean value")
        })?;
        self.data_types.lock().await.insert(node_id.clone(), scalar);
        Ok(scalar)
    }
}

/// Write `value` to the Value attribute of `node_id` on the device whose
/// poller id is `device_id` (`opcua_<id>`), through the poller's session, in
/// the node's own data type.
pub async fn write_tag(device_id: &str, node_id: &str, value: f64) -> WriteResult {
    if !crate::node_role::is_active() {
        return WriteResult::refused_passive();
    }
    let start = std::time::Instant::now();
    let fail = |error: String| WriteResult {
        success: false,
        device_response: None,
        error: Some(error),
        latency_ms: start.elapsed().as_millis() as u64,
    };

    let node = match NodeId::from_str(node_id) {
        Ok(n) => n,
        Err(e) => return fail(format!("Invalid OPC UA node id '{node_id}': {e:?}")),
    };
    let live = SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(device_id)
        .cloned();
    let Some(live) = live else {
        return fail("device not connected".into());
    };

    let scalar = match live.scalar_type(&node).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let variant = match scalar.variant_for(value) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };

    let write = WriteValue {
        node_id: node.clone(),
        attribute_id: AttributeId::Value as u32,
        index_range: NumericRange::None,
        value: DataValue::value_only(variant),
    };
    match live.session.write(&[write]).await {
        Ok(results) => match results.first() {
            Some(status) if status.is_good() => WriteResult {
                success: true,
                device_response: Some(format!("{status} ({scalar:?})")),
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            },
            Some(status) => {
                if *status == StatusCode::BadTypeMismatch {
                    // The server's type may have changed; read it again next time.
                    live.data_types.lock().await.remove(&node);
                }
                fail(format!("OPC UA write of {node_id} refused: {status}"))
            }
            None => fail("OPC UA write returned no result".into()),
        },
        Err(status) => fail(format!("OPC UA write of {node_id} failed: {status}")),
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

    fn data_value(value: Option<Variant>, status: Option<StatusCode>) -> DataValue {
        DataValue {
            value,
            status,
            ..Default::default()
        }
    }

    #[test]
    fn status_codes_map_to_quality() {
        assert_eq!(status_quality(StatusCode::Good), Quality::Good);
        assert_eq!(status_quality(StatusCode::Uncertain), Quality::Uncertain);
        assert_eq!(
            status_quality(StatusCode::UncertainLastUsableValue),
            Quality::UncertainStale
        );
        assert_eq!(
            status_quality(StatusCode::BadCommunicationError),
            Quality::BadCommFailure
        );
        assert_eq!(
            status_quality(StatusCode::BadNodeIdUnknown),
            Quality::BadConfigError
        );
        assert_eq!(
            status_quality(StatusCode::BadSensorFailure),
            Quality::BadDeviceError
        );
        assert_eq!(status_quality(StatusCode::BadUnexpectedError), Quality::Bad);
    }

    #[test]
    fn a_bad_data_change_is_nan_whatever_value_it_carries() {
        let (v, q) = reading_of(&data_value(
            Some(Variant::Double(12.0)),
            Some(StatusCode::BadSensorFailure),
        ))
        .unwrap();
        assert!(v.is_nan());
        assert_eq!(q, Quality::BadDeviceError);
        // Bad with no value at all still reaches the cache.
        assert!(reading_of(&data_value(None, Some(StatusCode::BadTimeout))).is_some());
    }

    #[test]
    fn good_and_uncertain_data_changes_keep_their_value() {
        assert_eq!(
            reading_of(&data_value(Some(Variant::Int32(7)), None)),
            Some((7.0, Quality::Good))
        );
        assert_eq!(
            reading_of(&data_value(
                Some(Variant::Float(1.5)),
                Some(StatusCode::Uncertain)
            )),
            Some((1.5, Quality::Uncertain))
        );
        assert_eq!(reading_of(&data_value(None, None)), None);
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
    fn test_scalar_type_from_data_type() {
        let t = |i: u32| ScalarType::from_data_type(&NodeId::new(0, i));
        assert_eq!(t(1), Some(ScalarType::Boolean));
        assert_eq!(t(4), Some(ScalarType::Int16));
        assert_eq!(t(7), Some(ScalarType::UInt32));
        assert_eq!(t(10), Some(ScalarType::Float));
        assert_eq!(t(11), Some(ScalarType::Double));
        assert_eq!(t(29), Some(ScalarType::Int32), "enumerations");
        assert_eq!(t(12), None, "String");
        assert_eq!(ScalarType::from_data_type(&NodeId::new(2, 11u32)), None);
    }

    #[test]
    fn test_scalar_type_from_variant() {
        assert_eq!(
            ScalarType::from_variant(&Variant::Float(1.0)),
            Some(ScalarType::Float)
        );
        assert_eq!(
            ScalarType::from_variant(&Variant::UInt64(1)),
            Some(ScalarType::UInt64)
        );
        assert_eq!(ScalarType::from_variant(&Variant::Empty), None);
    }

    #[test]
    fn test_variant_for_each_type() {
        let v = |t: ScalarType, x: f64| t.variant_for(x).unwrap();
        assert_eq!(v(ScalarType::Boolean, 3.0), Variant::Boolean(true));
        assert_eq!(v(ScalarType::Boolean, 0.0), Variant::Boolean(false));
        assert_eq!(v(ScalarType::SByte, -7.0), Variant::SByte(-7));
        assert_eq!(v(ScalarType::Byte, 254.6), Variant::Byte(255));
        assert_eq!(v(ScalarType::Int16, -300.0), Variant::Int16(-300));
        assert_eq!(v(ScalarType::UInt16, 65_535.0), Variant::UInt16(65_535));
        assert_eq!(v(ScalarType::Int32, -100_000.0), Variant::Int32(-100_000));
        assert_eq!(v(ScalarType::UInt32, 4e9), Variant::UInt32(4_000_000_000));
        assert_eq!(
            v(ScalarType::Int64, -5e12),
            Variant::Int64(-5_000_000_000_000)
        );
        assert_eq!(
            v(ScalarType::UInt64, 5e12),
            Variant::UInt64(5_000_000_000_000)
        );
        assert_eq!(v(ScalarType::Float, 12.5), Variant::Float(12.5));
        assert_eq!(v(ScalarType::Double, 0.1), Variant::Double(0.1));
    }

    #[test]
    fn test_variant_for_refuses_out_of_range() {
        assert!(ScalarType::Byte.variant_for(-1.0).is_err());
        assert!(ScalarType::SByte.variant_for(128.0).is_err());
        assert!(ScalarType::UInt16.variant_for(65_536.0).is_err());
        assert!(ScalarType::Int32.variant_for(3e9).is_err());
        assert!(ScalarType::UInt64.variant_for(-1.0).is_err());
        assert!(ScalarType::Int64.variant_for(1e19).is_err());
        assert!(ScalarType::Float.variant_for(1e39).is_err());
        assert!(ScalarType::Double.variant_for(f64::NAN).is_err());
    }

    #[tokio::test]
    async fn test_write_without_session_is_not_connected() {
        let r = write_tag("opcua_999999", "ns=2;s=X", 1.0).await;
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("device not connected"));
    }

    /// Writes through a poller's live session to an in-process server, read
    /// back from the server's address space.
    #[tokio::test]
    async fn test_writes_through_poller_session() {
        use opcua::crypto::SecurityPolicy;
        use opcua::nodes::AccessLevel;
        use opcua::server::address_space::{NodeType, Variable};
        use opcua::server::diagnostics::NamespaceMetadata;
        use opcua::server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
        use opcua::server::{ANONYMOUS_USER_TOKEN_ID, ServerBuilder};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("opc.tcp://127.0.0.1:{port}");
        let pki = tempfile::tempdir().unwrap();
        let (server, handle) = ServerBuilder::new()
            .application_name("write test")
            .application_uri("urn:voltrus-write-test")
            .product_uri("urn:voltrus-write-test")
            .host("127.0.0.1")
            .port(port)
            .discovery_urls(vec![url.clone()])
            .create_sample_keypair(true)
            .pki_dir(pki.path())
            .add_endpoint(
                "none",
                (
                    "/",
                    SecurityPolicy::None,
                    MessageSecurityMode::None,
                    &[ANONYMOUS_USER_TOKEN_ID] as &[&str],
                ),
            )
            .trust_client_certs(true)
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: "urn:WriteTest".to_owned(),
                    ..Default::default()
                },
                "write-test",
            ))
            .build()
            .unwrap();
        let manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();
        let ns = handle.get_namespace_index("urn:WriteTest").unwrap();
        {
            let address_space = manager.address_space();
            let mut address_space = address_space.write();
            let writable = |mut v: Variable| {
                let rw = AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE;
                v.set_access_level(rw);
                v.set_user_access_level(rw);
                v
            };
            address_space.add_variables(
                vec![
                    writable(Variable::new(&NodeId::new(ns, "Sp"), "Sp", "Sp", 1.0f32)),
                    writable(Variable::new(
                        &NodeId::new(ns, "Count"),
                        "Count",
                        "Count",
                        0i16,
                    )),
                    writable(Variable::new(&NodeId::new(ns, "Run"), "Run", "Run", false)),
                    Variable::new(&NodeId::new(ns, "Fixed"), "Fixed", "Fixed", 5.0f64),
                ],
                &NodeId::objects_folder_id(),
            );
        }
        tokio::spawn(server.run_with(listener));

        let device_id = format!("opcua_test_{port}");
        let config = OpcUaDeviceConfig {
            id: device_id.clone(),
            url: url.clone(),
            security_policy: OpcUaSecurityPolicy::None,
            security_mode: OpcUaSecurityMode::None,
            username: None,
            password: None,
            interval_ms: 200,
            nodes: vec![OpcUaNodeConfig {
                node_id: format!("ns={ns};s=Sp"),
                name: "sp".into(),
                display_name: String::new(),
                data_type: String::new(),
            }],
        };
        let poller = tokio::spawn(poll_opcua_device(config, DeviceCache::new()));

        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !SESSIONS.lock().unwrap().contains_key(&device_id) {
            assert!(
                std::time::Instant::now() < deadline,
                "poller never connected"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let node = |name: &str| format!("ns={ns};s={name}");
        let value_of = |name: &str| {
            let address_space = manager.address_space();
            let address_space = address_space.read();
            match address_space.find_node(&NodeId::new(ns, name)) {
                Some(NodeType::Variable(v)) => v
                    .value(
                        TimestampsToReturn::Neither,
                        &NumericRange::None,
                        &opcua::types::DataEncoding::Binary,
                        0.0,
                    )
                    .value
                    .unwrap(),
                _ => panic!("no variable {name}"),
            }
        };

        let r = write_tag(&device_id, &node("Sp"), 12.5).await;
        assert!(r.success, "{r:?}");
        assert_eq!(value_of("Sp"), Variant::Float(12.5));

        let r = write_tag(&device_id, &node("Count"), -300.0).await;
        assert!(r.success, "{r:?}");
        assert_eq!(value_of("Count"), Variant::Int16(-300));

        let r = write_tag(&device_id, &node("Run"), 1.0).await;
        assert!(r.success, "{r:?}");
        assert_eq!(value_of("Run"), Variant::Boolean(true));

        let r = write_tag(&device_id, &node("Count"), 40_000.0).await;
        assert!(!r.success);
        assert_eq!(value_of("Count"), Variant::Int16(-300));

        let r = write_tag(&device_id, &node("Fixed"), 1.0).await;
        assert!(!r.success, "a read-only node refuses: {r:?}");
        assert!(r.error.unwrap().contains("Bad"));

        poller.abort();
        let _ = poller.await;
        assert!(
            !SESSIONS.lock().unwrap().contains_key(&device_id),
            "stopping the poller unregisters its session"
        );
        let r = write_tag(&device_id, &node("Sp"), 1.0).await;
        assert_eq!(r.error.as_deref(), Some("device not connected"));
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
