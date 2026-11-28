use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use opcua::client::{
    ClientBuilder, IdentityToken, MonitoredItemMap, OnSubscriptionNotificationCore, Session,
};
use opcua::types::{
    AttributeId, DataChangeNotification, DataValue, MessageSecurityMode, MonitoringMode,
    MonitoringParameters, NodeId, NotificationMessage, NumericRange, ReadValueId, StatusCode,
    TimestampsToReturn, UserTokenPolicy, VariableId, Variant, WriteValue,
};

use crate::cache::DeviceCache;
use crate::models::*;
use crate::quality::Quality;

/// Main entry point for OPC-UA device polling.
///
/// Spawned as `tokio::spawn(opcua::poll_opcua_device(config, cache))`.
/// Runs an infinite loop with exponential backoff on connection failure.
pub async fn poll_opcua_device(config: OpcUaDeviceConfig, cache: DeviceCache) {
    bench::start_stats_log();
    let device_id = config.id.clone();
    let mut backoff = Backoff::new();

    loop {
        let started = std::time::Instant::now();
        match try_connect_and_subscribe(&config, &cache).await {
            Ok(()) => {
                // Only a session that stayed up a while resets the backoff: a
                // server that accepts and drops at once must not be hammered.
                if started.elapsed() >= STABLE_SESSION {
                    backoff.reset();
                }
                tracing::warn!("OPC-UA device {} connection lost, reconnecting", device_id);
            }
            Err(e) => {
                tracing::warn!("OPC-UA device {} error: {e}", device_id);
            }
        }
        cache.set_status(&device_id, DeviceStatus::Offline).await;
        let delay = backoff.next();
        tracing::info!("OPC-UA device {} retrying in {delay}ms", device_id);
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

/// Data changes received from servers, all OPC UA devices since start.
pub static VALUES_DELIVERED: AtomicU64 = AtomicU64::new(0);
/// Data changes replaced by a newer value of the same tag before the cache
/// took them (the cache writer was behind). Nothing else is ever dropped.
pub static VALUES_SUPERSEDED: AtomicU64 = AtomicU64::new(0);

/// Monitored items per CreateMonitoredItems call, unless the server allows
/// fewer: a common server default, and requests that stay far below the
/// default message size limit (about 60 bytes per item).
const ITEMS_PER_CALL: usize = 1000;

/// Values per publish response asked of the server; it sends the rest in the
/// next responses, one round trip each. At 1000, a device of 50,000 tags all
/// changing every second got 38 % of the changes from async-opcua's server
/// (which queues 20 notification messages per subscription by default). 5000 Double values with
/// their source time are ~125 kB, well inside the default 320 kB message
/// limit. The client must decode arrays that long: see [`MAX_ARRAY_LENGTH`].
const MAX_NOTIFICATIONS_PER_PUBLISH: u32 = 5000;

/// Longest array the client decodes. async-opcua's default, 1000, made the
/// session drop on the first publish response with more values (a device
/// with more than 1000 tags never stayed connected).
const MAX_ARRAY_LENGTH: usize = MAX_NOTIFICATIONS_PER_PUBLISH as usize;

/// Hooks for `scenarios/opcua-bench`, off unless their environment variables
/// are set.
mod bench {
    use super::*;

    /// `VOLTRUS_BENCH_LATENCY_TAG`: the tag (its name in Voltrus) whose value
    /// is the simulator's send time in Unix ms. Each time it reaches the cache
    /// the delay is logged as `opcua_bench latency_ms=...`.
    pub(super) static LATENCY_TAG: LazyLock<Option<String>> =
        LazyLock::new(|| std::env::var("VOLTRUS_BENCH_LATENCY_TAG").ok());

    pub(super) fn record_latency(sent_ms: f64) {
        let now = crate::timeutil::now_millis() as f64;
        tracing::info!("opcua_bench latency_ms={:.1}", now - sent_ms);
    }

    /// `VOLTRUS_BENCH_STATS=1`: log the counters once a second, as
    /// `opcua_bench delivered=... superseded=... allocs=...`.
    pub(super) fn start_stats_log() {
        static STARTED: std::sync::Once = std::sync::Once::new();
        if std::env::var_os("VOLTRUS_BENCH_STATS").is_none() {
            return;
        }
        STARTED.call_once(|| {
            tokio::spawn(async {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tick.tick().await;
                    tracing::info!(
                        "opcua_bench delivered={} superseded={} allocs={}",
                        VALUES_DELIVERED.load(Ordering::Relaxed),
                        VALUES_SUPERSEDED.load(Ordering::Relaxed),
                        crate::alloc_count::allocations().unwrap_or(0)
                    );
                }
            });
        });
    }
}

/// A session that lasted this long counts as a working connection.
const STABLE_SESSION: Duration = Duration::from_secs(60);

/// Where the OPC UA client keeps its own certificate and the server
/// certificates it trusts (`trusted/`) or refused (`rejected/`), relative to
/// the working directory. The poller and browse share it.
pub(crate) const PKI_DIR: &str = "pki";

/// One OPC UA server and how to log on to it: what the poller and browse both
/// connect with, so both apply the same certificate trust.
pub(crate) struct ServerTarget<'a> {
    pub url: &'a str,
    pub security_policy: &'a OpcUaSecurityPolicy,
    pub security_mode: &'a OpcUaSecurityMode,
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
}

impl<'a> From<&'a OpcUaDeviceConfig> for ServerTarget<'a> {
    fn from(config: &'a OpcUaDeviceConfig) -> Self {
        Self {
            url: &config.url,
            security_policy: &config.security_policy,
            security_mode: &config.security_mode,
            username: config.username.as_deref(),
            password: config.password.as_deref(),
        }
    }
}

impl<'a> From<&'a OpcUaDevice> for ServerTarget<'a> {
    fn from(device: &'a OpcUaDevice) -> Self {
        Self {
            url: &device.url,
            security_policy: &device.security_policy,
            security_mode: &device.security_mode,
            username: device.username.as_deref(),
            password: device.password.as_deref(),
        }
    }
}

/// Owns a connected session and its event loop. However the attempt ends,
/// an error, the loop ending, or the poller task being aborted, dropping
/// this closes the session and stops the event loop, so neither outlives
/// the poller. [`SessionGuard::close`] does the same and waits for it.
pub(crate) struct SessionGuard {
    session: Arc<Session>,
    event_loop: tokio::task::JoinHandle<StatusCode>,
    closed: bool,
}

/// How long closing a session may take before the event loop is stopped anyway.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);

impl SessionGuard {
    pub(crate) fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Close the session now and wait (bounded) until the server has been told.
    pub(crate) async fn close(mut self) {
        self.closed = true;
        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, self.session.disconnect()).await;
        self.event_loop.abort();
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let session = self.session.clone();
        let event_loop = self.event_loop.abort_handle();
        match tokio::runtime::Handle::try_current() {
            // Disconnecting needs the event loop running, so it is stopped
            // after the (bounded) disconnect.
            Ok(rt) => {
                rt.spawn(async move {
                    let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, session.disconnect()).await;
                    event_loop.abort();
                });
            }
            Err(_) => event_loop.abort(),
        }
    }
}

/// Whether a status code says the server's certificate was refused.
fn is_certificate_error(code: StatusCode) -> bool {
    let sub = code.sub_code();
    [
        StatusCode::BadCertificateUntrusted,
        StatusCode::BadCertificateInvalid,
        StatusCode::BadCertificateTimeInvalid,
        StatusCode::BadCertificateHostNameInvalid,
        StatusCode::BadCertificateUriInvalid,
        StatusCode::BadCertificateUseNotAllowed,
        StatusCode::BadCertificateRevoked,
        StatusCode::BadCertificateIssuerRevoked,
        StatusCode::BadCertificateRevocationUnknown,
        StatusCode::BadCertificatePolicyCheckFailed,
        StatusCode::BadSecurityChecksFailed,
    ]
    .iter()
    .any(|c| c.sub_code() == sub)
}

/// The error for a connection that failed with `code` (`detail` says more),
/// telling an operator what to do when it is the server certificate that was
/// refused.
fn connection_error(
    code: StatusCode,
    detail: impl std::fmt::Display,
    secure: bool,
    pki_dir: &std::path::Path,
) -> anyhow::Error {
    let trusted = pki_dir.join("trusted");
    let rejected = pki_dir.join("rejected");
    if code.sub_code() == StatusCode::BadCertificateUntrusted.sub_code() {
        anyhow::anyhow!(
            "OPC-UA connection refused: the server certificate is not trusted ({detail}). \
             It was saved to {}; check that it is this server's certificate, move it \
             to {} and try again",
            rejected.display(),
            trusted.display()
        )
    } else if is_certificate_error(code) {
        anyhow::anyhow!(
            "OPC-UA connection refused: the server certificate failed validation ({detail}). \
             Check its validity dates, host name and application URI"
        )
    } else if secure {
        anyhow::anyhow!(
            "OPC-UA connection failed: {detail} (a secure endpoint needs the server \
             certificate in {}; a refused one is in {})",
            trusted.display(),
            rejected.display()
        )
    } else {
        anyhow::anyhow!("OPC-UA connection failed: {detail}")
    }
}

/// Connect to an OPC UA server and wait until the session is up.
///
/// Without message security the server certificate protects nothing, so any
/// is accepted. With Sign or SignAndEncrypt the certificate is what proves
/// the server is the right one: it must already be in `<pki_dir>/trusted`. An
/// unknown one is refused and copied to `<pki_dir>/rejected`, from where an
/// administrator moves it to `trusted`.
///
/// `single_attempt` gives up after the first failed attempt instead of
/// retrying with backoff (for a request someone is waiting on).
pub(crate) async fn connect(
    target: &ServerTarget<'_>,
    application_name: &str,
    pki_dir: &std::path::Path,
    single_attempt: bool,
) -> Result<SessionGuard> {
    let secure = !matches!(target.security_mode, OpcUaSecurityMode::None);
    let mut builder = ClientBuilder::new()
        .application_name(application_name)
        .application_uri("urn:voltrus-lite")
        .pki_dir(pki_dir)
        .trust_server_certs(!secure)
        .create_sample_keypair(true)
        .max_array_length(MAX_ARRAY_LENGTH);
    if single_attempt {
        builder = builder.session_retry_limit(0);
    }
    let mut client = builder
        .client()
        .map_err(|e| anyhow::anyhow!("Client build errors: {:?}", e))?;

    let user_token_policy = match (target.username, target.password) {
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
        target.url,
        security_policy_to_str(target.security_policy),
        security_mode_to_opcua(target.security_mode),
        user_token_policy,
    ));

    let identity_token = match (target.username, target.password) {
        (Some(user), Some(pass)) => IdentityToken::new_user_name(user, pass),
        _ => IdentityToken::Anonymous,
    };

    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, identity_token)
        .await
        .map_err(|e| connection_error(e.status(), &e, secure, pki_dir))?;

    let mut guard = SessionGuard {
        session: session.clone(),
        event_loop: event_loop.spawn(),
        closed: false,
    };

    // The certificate check happens here, when the event loop creates the
    // session; a refusal ends the loop with the status code.
    tokio::select! {
        connected = session.wait_for_connection() => {
            if !connected {
                anyhow::bail!("OPC-UA session failed to connect");
            }
        }
        event_result = &mut guard.event_loop => {
            let code = event_result.context("OPC-UA event loop task panicked")?;
            // The loop has ended: nothing to disconnect.
            guard.closed = true;
            return Err(connection_error(code, code, secure, pki_dir));
        }
    }
    Ok(guard)
}

/// A node found by [`browse_objects`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowsedNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub node_class: &'static str,
}

/// Connect once (same certificate trust as the poller), list the objects and
/// variables directly under the Objects folder, and close the session again,
/// whether the browse worked or not.
pub(crate) async fn browse_objects(
    target: &ServerTarget<'_>,
    pki_dir: &std::path::Path,
) -> Result<Vec<BrowsedNode>> {
    let guard = connect(target, "Voltrus Browse", pki_dir, true).await?;
    let result = browse_objects_folder(guard.session()).await;
    guard.close().await;
    result
}

async fn browse_objects_folder(session: &Session) -> Result<Vec<BrowsedNode>> {
    use opcua::client::{UARequest, services::Browse};
    use opcua::types::{
        BrowseDescription, BrowseDescriptionResultMask, BrowseDirection, NodeClass, NodeClassMask,
    };

    let browse_desc = BrowseDescription {
        node_id: NodeId::objects_folder_id(),
        browse_direction: BrowseDirection::Forward,
        reference_type_id: NodeId::null(),
        include_subtypes: true,
        node_class_mask: NodeClassMask::OBJECT.bits() | NodeClassMask::VARIABLE.bits(),
        result_mask: BrowseDescriptionResultMask::RESULT_MASK_NODE_CLASS.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_BROWSE_NAME.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_DISPLAY_NAME.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_TYPE_DEFINITION.bits(),
    };

    let response = Browse::new(session)
        .browse_node(browse_desc)
        .send(session.channel())
        .await
        .map_err(|e| anyhow::anyhow!("Browse request failed: {e}"))?;

    let mut nodes = Vec::new();
    for result in response.results.iter().flatten() {
        if !result.status_code.is_good() {
            continue;
        }
        for reference in result.references.iter().flatten() {
            // Objects may contain variables further down.
            let node_class = match reference.node_class {
                NodeClass::Variable => "Variable",
                NodeClass::Object => "Object",
                _ => continue,
            };
            // A node on another server cannot be read through this session.
            if reference.node_id.server_index != 0 {
                continue;
            }
            nodes.push(BrowsedNode {
                // The plain node id (`ns=2;s=Temp`), as the poller parses it;
                // the expanded id would carry a `svr=0;` prefix it refuses.
                node_id: reference.node_id.node_id.to_string(),
                browse_name: reference.browse_name.to_string(),
                display_name: reference.display_name.to_string(),
                node_class,
            });
        }
    }
    Ok(nodes)
}

/// Attempts to connect to the OPC-UA server, create a subscription,
/// and monitor nodes. Returns Ok(()) when the session/event-loop ends
/// (e.g. connection drop), or Err on setup failure.
async fn try_connect_and_subscribe(config: &OpcUaDeviceConfig, cache: &DeviceCache) -> Result<()> {
    let device_id = config.id.clone();
    let mut guard = connect(
        &ServerTarget::from(config),
        "Voltrus",
        std::path::Path::new(PKI_DIR),
        false,
    )
    .await?;
    let session = guard.session().clone();

    tracing::info!("OPC-UA device {} connected to {}", device_id, config.url);

    // Queued writes go through this session while it lives.
    let _registration = SessionRegistration::register(&device_id, session.clone());

    // --- Tags: tag i is monitored with client handle i + 1 ---
    let tags: Vec<(NodeId, &OpcUaNodeConfig)> = config
        .nodes
        .iter()
        .filter_map(|node| NodeId::from_str(&node.node_id).ok().map(|id| (id, node)))
        .collect();
    if tags.is_empty() {
        anyhow::bail!("no valid node ids to monitor");
    }
    let names: Arc<[String]> = tags.iter().map(|(_, node)| node.name.clone()).collect();
    let feed = Arc::new(Feed::new(names.len()));
    tokio::spawn(write_to_cache(
        device_id.clone(),
        cache.clone(),
        feed.clone(),
        names,
    ));

    // --- Create subscription ---
    let interval_ms = config.interval_ms.max(100);
    let subscription_id = session
        .create_subscription(
            Duration::from_millis(interval_ms),
            90, // lifetime_count (3x max_keep_alive_count per OPC-UA spec)
            30, // max_keep_alive_count
            MAX_NOTIFICATIONS_PER_PUBLISH,
            0,    // priority
            true, // publishing_enabled
            Notifier { feed },
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create subscription: {:?}", e))?;

    tracing::info!(
        "OPC-UA device {} subscription created (id={})",
        device_id,
        subscription_id
    );

    // --- Create monitored items, in calls the server accepts ---
    let per_call = items_per_call(&session).await;
    let mut success_count = 0;
    for (chunk_index, chunk) in tags.chunks(per_call).enumerate() {
        let first = chunk_index * per_call;
        let items_to_create: Vec<opcua::types::MonitoredItemCreateRequest> = chunk
            .iter()
            .enumerate()
            .map(
                |(j, (node_id, _))| opcua::types::MonitoredItemCreateRequest {
                    item_to_monitor: ReadValueId {
                        node_id: node_id.clone(),
                        attribute_id: AttributeId::Value as u32,
                        index_range: NumericRange::None,
                        data_encoding: opcua::types::QualifiedName::null(),
                    },
                    monitoring_mode: MonitoringMode::Reporting,
                    requested_parameters: MonitoringParameters {
                        client_handle: (first + j + 1) as u32,
                        sampling_interval: interval_ms as f64,
                        // As before. Queue size 1 let async-opcua's server
                        // send a third fewer values at 100 ms with no clear
                        // latency cost (docs/footprint.md), but has not been
                        // tried against other servers.
                        queue_size: 10,
                        discard_oldest: true,
                        filter: opcua::types::ExtensionObject::null(),
                    },
                },
            )
            .collect();

        let results = session
            .create_monitored_items(subscription_id, TimestampsToReturn::Source, items_to_create)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create monitored items: {:?}", e))?;

        for (j, result) in results.iter().enumerate() {
            if result.result.status_code.is_good() {
                success_count += 1;
            } else {
                tracing::warn!(
                    "OPC-UA device {} monitored item {} creation failed: {:?}",
                    device_id,
                    first + j,
                    result.result.status_code
                );
            }
        }
    }

    tracing::info!(
        "OPC-UA device {} created {}/{} monitored items",
        device_id,
        success_count,
        tags.len()
    );

    cache.set_status(&device_id, DeviceStatus::Online).await;

    // Wait for the event loop to end (connection lost, error, etc.)
    let _ = (&mut guard.event_loop).await;

    Ok(())
}

/// How many monitored items to create per call: the server's
/// MaxMonitoredItemsPerCall when it announces one, at most [`ITEMS_PER_CALL`].
async fn items_per_call(session: &Session) -> usize {
    let limit = ReadValueId {
        node_id: VariableId::Server_ServerCapabilities_OperationLimits_MaxMonitoredItemsPerCall
            .into(),
        attribute_id: AttributeId::Value as u32,
        index_range: NumericRange::None,
        data_encoding: opcua::types::QualifiedName::null(),
    };
    let announced = session
        .read(&[limit], TimestampsToReturn::Neither, 0.0)
        .await
        .ok()
        .and_then(|values| values.into_iter().next())
        .and_then(|value| match value.value {
            Some(Variant::UInt32(n)) if n > 0 => Some(n as usize),
            _ => None,
        });
    announced.map_or(ITEMS_PER_CALL, |n| n.min(ITEMS_PER_CALL))
}

/// One tag's latest change not yet in the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Change {
    value: f64,
    quality: Quality,
    /// The server's source timestamp in Unix ms.
    source_ms: Option<i64>,
}

/// The changes of one device's tags between the subscription callback and the
/// task that writes them to the cache: at most one per tag, so it is bounded
/// by the tag count and never drops a tag's latest value. A tag that changes
/// again before the cache took its last change keeps the newer one (counted in
/// [`VALUES_SUPERSEDED`]).
struct Feed {
    pending: Mutex<Pending>,
    wake: tokio::sync::Notify,
    /// The subscription is gone: the writer stops once it has written the rest.
    closed: AtomicBool,
}

#[derive(Default)]
struct Pending {
    /// By tag index; `Some` while waiting for the cache.
    slots: Vec<Option<Change>>,
    /// Tags with a change waiting, in the order they first changed.
    waiting: Vec<u32>,
}

impl Pending {
    /// Store the change of the tag with `client_handle` (index + 1). Returns
    /// whether it replaced one the cache had not taken yet, `None` for a
    /// handle that is not ours.
    fn set(&mut self, client_handle: u32, change: Change) -> Option<bool> {
        let index = client_handle.checked_sub(1)?;
        let slot = self.slots.get_mut(index as usize)?;
        let superseded = slot.replace(change).is_some();
        if !superseded {
            self.waiting.push(index);
        }
        Some(superseded)
    }

    /// Move every waiting change into `out` as `(tag index, change)`.
    fn take(&mut self, out: &mut Vec<(u32, Change)>) {
        for index in self.waiting.drain(..) {
            if let Some(change) = self.slots.get_mut(index as usize).and_then(Option::take) {
                out.push((index, change));
            }
        }
    }
}

impl Feed {
    fn new(tags: usize) -> Self {
        Self {
            pending: Mutex::new(Pending {
                slots: vec![None; tags],
                waiting: Vec::new(),
            }),
            wake: tokio::sync::Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The subscription callback: takes each publish response whole and files its
/// data changes into the device's [`Feed`] by client handle, with no lookup by
/// node id and no allocation per value.
struct Notifier {
    feed: Arc<Feed>,
}

impl OnSubscriptionNotificationCore for Notifier {
    fn on_subscription_notification(
        &mut self,
        notification: NotificationMessage,
        _items: MonitoredItemMap<'_>,
    ) {
        self.file(notification);
    }
}

impl Notifier {
    fn file(&mut self, notification: NotificationMessage) {
        let Some(data) = notification.notification_data else {
            return;
        };
        let (mut received, mut superseded) = (0u64, 0u64);
        {
            let mut pending = self.feed.pending();
            for object in data {
                let Some(changes) = object.into_inner_as::<DataChangeNotification>() else {
                    continue;
                };
                for item in changes.monitored_items.into_iter().flatten() {
                    let Some((value, quality)) = reading_of(&item.value) else {
                        continue;
                    };
                    let change = Change {
                        value,
                        quality,
                        source_ms: item
                            .value
                            .source_timestamp
                            .map(|t| t.as_chrono().timestamp_millis()),
                    };
                    match pending.set(item.client_handle, change) {
                        Some(replaced) => {
                            received += 1;
                            superseded += u64::from(replaced);
                        }
                        None => tracing::debug!(
                            "OPC-UA data change for unknown client handle {}",
                            item.client_handle
                        ),
                    }
                }
            }
        }
        if received > 0 {
            VALUES_DELIVERED.fetch_add(received, Ordering::Relaxed);
            if superseded > 0 {
                VALUES_SUPERSEDED.fetch_add(superseded, Ordering::Relaxed);
            }
            self.feed.wake.notify_one();
        }
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.feed.closed.store(true, Ordering::Release);
        self.feed.wake.notify_one();
    }
}

/// Writes a device's changes to the cache as they come, one merge for all the
/// changes waiting, until its subscription is gone.
async fn write_to_cache(
    device_id: String,
    cache: DeviceCache,
    feed: Arc<Feed>,
    names: Arc<[String]>,
) {
    let latency_tag = bench::LATENCY_TAG
        .as_deref()
        .and_then(|tag| names.iter().position(|name| name == tag));
    let mut batch: Vec<(u32, Change)> = Vec::new();
    loop {
        feed.wake.notified().await;
        // Read before taking: a change filed after this is still taken now or
        // on the wake-up it sent.
        let closed = feed.closed.load(Ordering::Acquire);
        feed.pending().take(&mut batch);
        if !batch.is_empty() {
            // A subscription only reports changes: merge, so the tags that
            // did not change keep their value.
            cache
                .merge_readings(
                    &device_id,
                    batch.iter().filter_map(|(index, change)| {
                        let name = names.get(*index as usize)?;
                        Some((
                            name.as_str(),
                            change.value,
                            change.quality,
                            change.source_ms,
                        ))
                    }),
                )
                .await;
            if let Some(tag) = latency_tag
                && let Some((_, change)) = batch.iter().find(|(index, _)| *index as usize == tag)
            {
                bench::record_latency(change.value);
            }
            batch.clear();
        }
        if closed {
            break;
        }
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

        // The in-process server and the client handshake share the test runner's
        // threads with ~1000 other tests, so allow a slow first connect.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
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

        // A device whose node ids all fail to parse is an error (retried with
        // backoff), and its session is closed rather than left behind.
        let bad = OpcUaDeviceConfig {
            id: format!("{device_id}_bad"),
            url,
            security_policy: OpcUaSecurityPolicy::None,
            security_mode: OpcUaSecurityMode::None,
            username: None,
            password: None,
            interval_ms: 200,
            nodes: vec![OpcUaNodeConfig {
                node_id: "not a node id".into(),
                name: "x".into(),
                display_name: String::new(),
                data_type: String::new(),
            }],
        };
        let err = tokio::time::timeout(
            Duration::from_secs(60),
            try_connect_and_subscribe(&bad, &DeviceCache::new()),
        )
        .await
        .expect("returns instead of waiting on the session")
        .unwrap_err();
        assert!(err.to_string().contains("no valid node ids"), "{err}");
        assert!(!SESSIONS.lock().unwrap().contains_key(&bad.id));
    }

    /// Browse trusts server certificates as the poller does: any on an
    /// unsecured endpoint, only those in `trusted` on a secure one (a refused
    /// one lands in `rejected` and the error says so). Every browse closes its
    /// session on the server, whether it worked or not.
    #[tokio::test]
    async fn test_browse_trusts_like_the_poller_and_closes_its_session() {
        use opcua::crypto::SecurityPolicy;
        use opcua::server::address_space::Variable;
        use opcua::server::diagnostics::NamespaceMetadata;
        use opcua::server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
        use opcua::server::{ANONYMOUS_USER_TOKEN_ID, ServerBuilder};
        use opcua::types::VariableId;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("opc.tcp://127.0.0.1:{port}");
        let server_pki = tempfile::tempdir().unwrap();
        let anonymous = &[ANONYMOUS_USER_TOKEN_ID] as &[&str];
        let (server, handle) = ServerBuilder::new()
            .application_name("browse test")
            .application_uri("urn:voltrus-browse-test")
            .product_uri("urn:voltrus-browse-test")
            .host("127.0.0.1")
            .port(port)
            .discovery_urls(vec![url.clone()])
            .create_sample_keypair(true)
            .pki_dir(server_pki.path())
            .diagnostics_enabled(true)
            .add_endpoint(
                "none",
                (
                    "/",
                    SecurityPolicy::None,
                    MessageSecurityMode::None,
                    anonymous,
                ),
            )
            .add_endpoint(
                "secure",
                (
                    "/",
                    SecurityPolicy::Basic256Sha256,
                    MessageSecurityMode::SignAndEncrypt,
                    anonymous,
                ),
            )
            .trust_client_certs(true)
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: "urn:BrowseTest".to_owned(),
                    ..Default::default()
                },
                "browse-test",
            ))
            .build()
            .unwrap();
        let manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();
        let ns = handle.get_namespace_index("urn:BrowseTest").unwrap();
        manager.address_space().write().add_variables(
            vec![Variable::new(
                &NodeId::new(ns, "Temp"),
                "Temp",
                "Temp",
                21.5f64,
            )],
            &NodeId::objects_folder_id(),
        );
        tokio::spawn(server.run_with(listener));

        let sessions = || {
            let value = handle
                .info()
                .diagnostics
                .get(VariableId::Server_ServerDiagnostics_ServerDiagnosticsSummary_CurrentSessionCount)
                .and_then(|v| v.value);
            match value {
                Some(Variant::UInt32(n)) => n,
                other => panic!("no session count: {other:?}"),
            }
        };
        let no_sessions_left = |stage: &'static str, allowed: u32| async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while sessions() > allowed {
                assert!(
                    std::time::Instant::now() < deadline,
                    "{stage}: browse left {} session(s) open on the server",
                    sessions()
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };

        let client_pki = tempfile::tempdir().unwrap();
        let target = |mode: &'static OpcUaSecurityMode| ServerTarget {
            url: &url,
            security_policy: match mode {
                OpcUaSecurityMode::None => &OpcUaSecurityPolicy::None,
                _ => &OpcUaSecurityPolicy::Basic256Sha256,
            },
            security_mode: mode,
            username: None,
            password: None,
        };
        // The connect and handshake share the runner's threads with ~1000
        // other tests, so allow them time.
        let browse = |mode: &'static OpcUaSecurityMode| {
            let target = target(mode);
            let pki = client_pki.path().to_path_buf();
            async move {
                tokio::time::timeout(Duration::from_secs(60), browse_objects(&target, &pki))
                    .await
                    .expect("browse returns instead of hanging")
            }
        };
        let temp = BrowsedNode {
            node_id: format!("ns={ns};s=Temp"),
            browse_name: "Temp".into(),
            display_name: "Temp".into(),
            node_class: "Variable",
        };

        // Unsecured: any certificate, nothing to trust.
        let nodes = browse(&OpcUaSecurityMode::None).await.unwrap();
        assert!(nodes.contains(&temp), "{nodes:?}");
        for n in &nodes {
            NodeId::from_str(&n.node_id).expect("a browsed id is one the poller can monitor");
        }
        no_sessions_left("unsecured", 0).await;

        // Secure, server certificate unknown: refused, saved to rejected.
        let err = browse(&OpcUaSecurityMode::SignAndEncrypt)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not trusted"), "{err}");
        assert!(err.contains("rejected"), "{err}");
        let rejected: Vec<_> = std::fs::read_dir(client_pki.path().join("rejected"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        // The client checks the server certificate in the CreateSession
        // response, so the server has a session the client then abandons
        // without activating it (no credentials were sent) and cannot close
        // over a channel it does not trust: the server expires it. Nothing
        // else may be left.
        let abandoned = sessions();
        assert!(
            abandoned <= 1,
            "{abandoned} sessions after a refused browse"
        );

        // Once an administrator moves it to trusted, browse works.
        let cert = &rejected[0];
        std::fs::rename(
            cert,
            client_pki
                .path()
                .join("trusted")
                .join(cert.file_name().unwrap()),
        )
        .unwrap();
        let nodes = browse(&OpcUaSecurityMode::SignAndEncrypt).await.unwrap();
        assert!(nodes.contains(&temp), "{nodes:?}");
        no_sessions_left("trusted", abandoned).await;

        handle.cancel();
    }

    #[test]
    fn test_untrusted_certificate_error_says_what_to_do() {
        let pki = std::path::Path::new("pki");
        let msg = connection_error(
            StatusCode::BadCertificateUntrusted,
            StatusCode::BadCertificateUntrusted,
            true,
            pki,
        )
        .to_string();
        assert!(msg.contains("not trusted"), "{msg}");
        assert!(
            msg.contains("pki/rejected") && msg.contains("pki/trusted"),
            "{msg}"
        );

        let msg = connection_error(
            StatusCode::BadCertificateHostNameInvalid,
            StatusCode::BadCertificateHostNameInvalid,
            true,
            pki,
        )
        .to_string();
        assert!(msg.contains("failed validation"), "{msg}");

        let msg = connection_error(StatusCode::BadTimeout, "timed out", false, pki).to_string();
        assert_eq!(msg, "OPC-UA connection failed: timed out");
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

    fn change(value: f64) -> Change {
        Change {
            value,
            quality: Quality::Good,
            source_ms: None,
        }
    }

    #[test]
    fn pending_keeps_the_latest_change_per_tag_in_first_change_order() {
        let mut pending = Pending {
            slots: vec![None; 3],
            waiting: Vec::new(),
        };
        assert_eq!(pending.set(3, change(1.0)), Some(false));
        assert_eq!(pending.set(1, change(2.0)), Some(false));
        assert_eq!(pending.set(3, change(3.0)), Some(true), "superseded");
        assert_eq!(pending.set(0, change(9.0)), None, "handles start at 1");
        assert_eq!(pending.set(4, change(9.0)), None, "not one of ours");
        let mut out = Vec::new();
        pending.take(&mut out);
        assert_eq!(out, vec![(2, change(3.0)), (0, change(2.0))]);
        out.clear();
        pending.take(&mut out);
        assert!(out.is_empty(), "taken changes are gone");
        assert_eq!(pending.set(3, change(4.0)), Some(false), "a new change");
    }

    fn notification(items: Vec<(u32, DataValue)>) -> NotificationMessage {
        let changes = DataChangeNotification {
            monitored_items: Some(
                items
                    .into_iter()
                    .map(
                        |(client_handle, value)| opcua::types::MonitoredItemNotification {
                            client_handle,
                            value,
                        },
                    )
                    .collect(),
            ),
            diagnostic_infos: None,
        };
        NotificationMessage {
            sequence_number: 1,
            publish_time: opcua::types::DateTime::now(),
            notification_data: Some(vec![opcua::types::ExtensionObject::from_message(changes)]),
        }
    }

    /// Publish responses go through the feed to the cache by client handle,
    /// with the server's source time; the writer stops with the subscription.
    #[tokio::test]
    async fn notifications_reach_the_cache_by_client_handle() {
        let cache = DeviceCache::new();
        let names: Arc<[String]> = ["a".to_string(), "b".to_string()].into();
        let feed = Arc::new(Feed::new(names.len()));
        let writer = tokio::spawn(write_to_cache(
            "opcua_t".into(),
            cache.clone(),
            feed.clone(),
            names,
        ));
        let mut notifier = Notifier { feed };
        let source = opcua::types::DateTime::now();
        let source_ms = source.as_chrono().timestamp_millis() - 60_000;
        let earlier = opcua::types::DateTime::from(
            opcua::types::DateTimeUtc::from_timestamp_millis(source_ms).unwrap(),
        );
        let items = vec![
            (2, DataValue::new_at(7.5f64, earlier)),
            (
                1,
                DataValue {
                    value: Some(Variant::Int32(3)),
                    status: Some(StatusCode::BadSensorFailure),
                    ..Default::default()
                },
            ),
            (9, DataValue::new_now(1.0f64)),
        ];
        notifier.file(notification(items));
        drop(notifier);
        tokio::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("the writer stops once the subscription is gone")
            .unwrap();

        let b = cache.get_reading("opcua_t", "b").await.unwrap();
        assert_eq!(b.value, 7.5);
        assert_eq!(b.ts_ms, source_ms, "the server's source time is kept");
        let a = cache.get_reading("opcua_t", "a").await.unwrap();
        assert!(a.value.is_nan());
        assert_eq!(a.quality, Quality::BadDeviceError);
    }

    /// More tags than the server takes in one CreateMonitoredItems call
    /// (async-opcua's default MaxMonitoredItemsPerCall is 1000): created in
    /// several calls, and every initial value reaches the cache, although
    /// there are more than one publish response may carry.
    #[tokio::test]
    async fn monitored_items_are_created_in_calls_the_server_accepts() {
        use opcua::crypto::SecurityPolicy;
        use opcua::server::address_space::Variable;
        use opcua::server::diagnostics::NamespaceMetadata;
        use opcua::server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
        use opcua::server::{ANONYMOUS_USER_TOKEN_ID, ServerBuilder};

        const TAGS: usize = 2500;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("opc.tcp://127.0.0.1:{port}");
        let pki = tempfile::tempdir().unwrap();
        let mut builder = ServerBuilder::new()
            .application_name("chunk test")
            .application_uri("urn:voltrus-chunk-test")
            .product_uri("urn:voltrus-chunk-test")
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
                    namespace_uri: "urn:ChunkTest".to_owned(),
                    ..Default::default()
                },
                "chunk-test",
            ));
        builder
            .limits_mut()
            .subscriptions
            .max_monitored_items_per_sub = 0;
        let (server, handle) = builder.build().unwrap();
        let manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();
        let ns = handle.get_namespace_index("urn:ChunkTest").unwrap();
        {
            let address_space = manager.address_space();
            let mut address_space = address_space.write();
            let vars = (0..TAGS)
                .map(|k| {
                    let name = format!("T{k}");
                    Variable::new(&NodeId::new(ns, name.as_str()), name.as_str(), "", k as f64)
                })
                .collect();
            address_space.add_variables(vars, &NodeId::objects_folder_id());
        }
        tokio::spawn(server.run_with(listener));

        let device_id = format!("opcua_chunk_{port}");
        let config = OpcUaDeviceConfig {
            id: device_id.clone(),
            url,
            security_policy: OpcUaSecurityPolicy::None,
            security_mode: OpcUaSecurityMode::None,
            username: None,
            password: None,
            interval_ms: 200,
            nodes: (0..TAGS)
                .map(|k| OpcUaNodeConfig {
                    node_id: format!("ns={ns};s=T{k}"),
                    name: format!("t{k}"),
                    display_name: String::new(),
                    data_type: String::new(),
                })
                .collect(),
        };
        let cache = DeviceCache::new();
        let poller = tokio::spawn(poll_opcua_device(config, cache.clone()));
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let count = cache
                .get_all()
                .await
                .iter()
                .find(|s| s.device_id == device_id)
                .map_or(0, |s| s.metrics.len());
            if count == TAGS {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{count} of {TAGS} tags reached the cache"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(
            cache.get_device_metric(&device_id, "t2499").await,
            Some(2499.0)
        );
        poller.abort();
    }
}
