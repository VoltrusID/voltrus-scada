use std::time::Duration;

use anyhow::Result;
use opcua::crypto::SecurityPolicy;
use opcua::server::ANONYMOUS_USER_TOKEN_ID;
use opcua::server::ServerBuilder;
use opcua::server::address_space::Variable;
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{SimpleNodeManager, simple_node_manager};
use opcua::types::{BuildInfo, DataValue, DateTime, MessageSecurityMode, NodeId, NumericRange};
use rand::Rng;

// --- Data Store ---
//
// Same 3-phase meter data model as modbus-sim.
// Values tick every second with random drift to simulate a real meter.

struct MeterData {
    voltage: [f64; 3],
    current: [f64; 3],
    power_factor: [f64; 3],
    frequency: f64,
    energy_active: f64,
    thd_voltage: [f64; 3],
    thd_current: [f64; 3],
    load_imbalance: f64,
}

impl MeterData {
    fn new() -> Self {
        Self {
            voltage: [220.0, 219.0, 221.0],
            current: [45.0, 42.0, 47.0],
            power_factor: [0.92, 0.91, 0.93],
            frequency: 50.0,
            energy_active: 158732.0,
            thd_voltage: [1.8, 2.1, 1.6],
            thd_current: [4.5, 5.2, 3.9],
            load_imbalance: 0.1,
        }
    }

    fn tick(&mut self) {
        let mut rng = rand::rng();

        // Voltage: slight per-phase drift + common mode variation
        let v_common = rng.random_range(-1.5..1.5);
        for v in &mut self.voltage {
            let drift = rng.random_range(-0.8..0.8);
            *v = (*v + v_common + drift).clamp(210.0, 230.0);
        }

        // Current: drift with load imbalance
        let i_common = rng.random_range(-2.0..2.0);
        for i in 0..3 {
            let imbalance = self.load_imbalance * rng.random_range(-5.0..5.0);
            self.current[i] = (self.current[i] + i_common + imbalance).clamp(5.0, 80.0);
        }

        // Power factor: slow drift per phase
        for pf in &mut self.power_factor {
            *pf = (*pf + rng.random_range(-0.005..0.005)).clamp(0.80, 0.99);
        }

        // Frequency: tight around 50Hz
        self.frequency = (self.frequency + rng.random_range(-0.005..0.005)).clamp(49.9, 50.1);

        // THD: slight variations
        for i in 0..3 {
            self.thd_voltage[i] =
                (self.thd_voltage[i] + rng.random_range(-0.2..0.2)).clamp(0.5, 5.0);
            self.thd_current[i] =
                (self.thd_current[i] + rng.random_range(-0.3..0.3)).clamp(1.0, 8.0);
        }

        // Energy accumulation (W -> kWh per second tick)
        let total_power: f64 = (0..3)
            .map(|p| self.voltage[p] * self.current[p] * self.power_factor[p])
            .sum();
        self.energy_active += total_power / 3600.0 / 1000.0;
    }
}

// --- CLI ---

/// Benchmark mode (`--tags N`): N Double tags instead of the meter, a share
/// of them changed every tick, and a `SendTime` tag carrying the tick's send
/// time in Unix milliseconds so a client can measure end-to-end latency.
#[derive(Clone, Copy)]
struct Bench {
    tags: usize,
    change_pct: f64,
    /// MaxMonitoredItemsPerCall the server announces and enforces.
    max_items_per_call: Option<usize>,
}

struct Args {
    host: String,
    port: u16,
    delay_ms: u64,
    bench: Option<Bench>,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut host: String = "127.0.0.1".to_string();
    let mut port: u16 = 4840;
    let mut delay_ms: u64 = 0;
    let mut tags: Option<usize> = None;
    let mut change_pct = 100.0;
    let mut max_items_per_call = None;
    let value = |i: usize| args.get(i + 1).and_then(|v| v.parse::<f64>().ok());

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                host = args.get(i + 1).cloned().unwrap_or_else(|| {
                    eprintln!("Missing host value, using default 127.0.0.1");
                    "127.0.0.1".to_string()
                });
                i += 2;
            }
            "--port" => {
                port = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("Invalid port, using default 4840");
                        4840
                    });
                i += 2;
            }
            // `--rate-ms` is the benchmark's name for the tick delay.
            "--delay" | "--rate-ms" => {
                delay_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 2;
            }
            "--tags" => {
                tags = value(i).map(|n| n as usize).filter(|n| *n > 0);
                i += 2;
            }
            "--change-pct" => {
                change_pct = value(i).unwrap_or(100.0).clamp(0.0, 100.0);
                i += 2;
            }
            "--max-items-per-call" => {
                max_items_per_call = value(i).map(|n| n as usize);
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }
    Args {
        host,
        port,
        delay_ms,
        bench: tags.map(|tags| Bench {
            tags,
            change_pct,
            max_items_per_call,
        }),
    }
}

/// Node id of benchmark tag `k`.
fn bench_node(ns: u16, k: usize) -> NodeId {
    NodeId::new(ns, format!("Bench.T{k:06}"))
}

/// Add the benchmark tags and `SendTime` under a `Bench` folder.
fn add_bench_nodes(manager: &SimpleNodeManager, ns: u16, bench: Bench) {
    let address_space = manager.address_space();
    let mut address_space = address_space.write();
    let folder_id = NodeId::new(ns, "Bench");
    address_space.add_folder(
        &folder_id,
        "Bench",
        "Benchmark tags",
        &NodeId::objects_folder_id(),
    );
    let mut vars: Vec<Variable> = (0..bench.tags)
        .map(|k| {
            let name = format!("T{k:06}");
            Variable::new(&bench_node(ns, k), name.as_str(), name.as_str(), 0.0f64)
        })
        .collect();
    vars.push(Variable::new(
        &NodeId::new(ns, "Bench.SendTime"),
        "SendTime",
        "Send time (Unix ms)",
        0.0f64,
    ));
    address_space.add_variables(vars, &folder_id);
}

/// Every tick, change `change_pct` % of the tags (a window that moves on, so
/// each tag changes in turn) and then `SendTime`, in one notification batch.
async fn run_bench_ticker(
    nm: std::sync::Arc<SimpleNodeManager>,
    subs: std::sync::Arc<opcua::server::SubscriptionCache>,
    ns: u16,
    bench: Bench,
    tick: Duration,
) {
    let ids: Vec<NodeId> = (0..bench.tags).map(|k| bench_node(ns, k)).collect();
    let send_time = NodeId::new(ns, "Bench.SendTime");
    let per_tick = ((bench.tags as f64 * bench.change_pct / 100.0).ceil() as usize).min(bench.tags);
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut start = 0usize;
    let mut ticks: u64 = 0;
    let mut produced: u64 = 0;
    let mut last_log = std::time::Instant::now();
    loop {
        interval.tick().await;
        ticks += 1;
        let now = DateTime::now();
        let mut values: Vec<(&NodeId, Option<&NumericRange>, DataValue)> =
            Vec::with_capacity(per_tick + 1);
        for j in 0..per_tick {
            let k = (start + j) % bench.tags;
            let v = ticks as f64 + k as f64 * 1e-6;
            values.push((&ids[k], None, DataValue::new_at(v, now)));
        }
        start = (start + per_tick) % bench.tags;
        let sent_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or_default();
        values.push((&send_time, None, DataValue::new_at(sent_ms, now)));
        produced += values.len() as u64;
        if let Err(e) = nm.set_values(&subs, values.into_iter()) {
            tracing::warn!("bench tick failed: {e:?}");
        }
        if last_log.elapsed() >= Duration::from_secs(10) {
            last_log = std::time::Instant::now();
            tracing::info!("bench ticks={ticks} produced={produced}");
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let Args {
        host,
        port,
        delay_ms,
        bench,
    } = parse_args();

    let endpoint_path = format!("opc.tcp://{host}:{port}");
    let discovery_url = format!("opc.tcp://{host}:{port}");
    let endpoint_rel_path = "/";

    let mut builder = ServerBuilder::new()
        .application_name("Voltrus OPC-UA Simulator")
        .application_uri("urn:voltrus-lite-opcua-sim")
        .product_uri("urn:voltrus-lite-opcua-sim")
        .host(host.clone())
        .port(port)
        .discovery_urls(vec![discovery_url])
        .create_sample_keypair(true)
        .pki_dir("./pki-opcua-sim")
        .add_endpoint(
            "none",
            (
                endpoint_rel_path,
                SecurityPolicy::None,
                MessageSecurityMode::None,
                &[ANONYMOUS_USER_TOKEN_ID] as &[&str],
            ),
        )
        .trust_client_certs(true)
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: "urn:VoltrusLiteSim".to_owned(),
                ..Default::default()
            },
            "voltrus-sim",
        ))
        .build_info(BuildInfo {
            product_uri: "urn:voltrus-lite-opcua-sim".into(),
            manufacturer_name: "Voltrus".into(),
            product_name: "Voltrus OPC-UA Simulator".into(),
            software_version: "0.20.1".into(),
            build_number: "1".into(),
            build_date: DateTime::now(),
        });
    if let Some(bench) = bench {
        let limits = builder.limits_mut();
        // Tens of thousands of items on one subscription, and responses that
        // carry all of them.
        limits.subscriptions.max_monitored_items_per_sub = 0;
        limits.max_message_size = 0;
        limits.max_chunk_count = 0;
        if let Some(n) = bench.max_items_per_call {
            limits.operational.max_monitored_items_per_call = n;
        }
    }
    let (server, handle) = builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build server: {e}"))?;

    // Get the node manager and namespace index
    let node_manager = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .ok_or_else(|| anyhow::anyhow!("SimpleNodeManager not found"))?;

    let ns = handle
        .get_namespace_index("urn:VoltrusLiteSim")
        .ok_or_else(|| anyhow::anyhow!("Namespace urn:VoltrusLiteSim not found"))?;

    let subscriptions = handle.subscriptions();
    tracing::info!("namespace urn:VoltrusLiteSim has index {ns}");

    if let Some(bench) = bench {
        add_bench_nodes(&node_manager, ns, bench);
        let tick = Duration::from_millis(if delay_ms > 0 { delay_ms } else { 1000 });
        tokio::spawn(run_bench_ticker(
            node_manager.clone(),
            subscriptions.clone(),
            ns,
            bench,
            tick,
        ));
        tracing::info!(
            "OPC-UA benchmark simulator on {host}:{port}: {} tags, {} % changed every {} ms",
            bench.tags,
            bench.change_pct,
            tick.as_millis()
        );
        return run_server(server, handle).await;
    }

    // Add nodes to the address space
    {
        let address_space = node_manager.address_space();
        let mut address_space = address_space.write();

        // Create a folder under the Objects folder
        let folder_id = NodeId::new(ns, "Meter");
        address_space.add_folder(
            &folder_id,
            "Meter",
            "3-Phase Energy Meter",
            &NodeId::objects_folder_id(),
        );

        // Add meter variables as f64 (OPC UA Double)
        let vars = vec![
            Variable::new(
                &NodeId::new(ns, "Voltage_L1"),
                "Voltage_L1",
                "L1-N Voltage",
                220.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Voltage_L2"),
                "Voltage_L2",
                "L2-N Voltage",
                219.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Voltage_L3"),
                "Voltage_L3",
                "L3-N Voltage",
                221.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Current_L1"),
                "Current_L1",
                "L1 Current",
                45.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Current_L2"),
                "Current_L2",
                "L2 Current",
                42.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Current_L3"),
                "Current_L3",
                "L3 Current",
                47.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "PowerFactor"),
                "PowerFactor",
                "Power Factor",
                0.92f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Frequency"),
                "Frequency",
                "Grid Frequency",
                50.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "Energy_kWh"),
                "Energy_kWh",
                "Active Energy",
                158732.0f64,
            ),
            Variable::new(
                &NodeId::new(ns, "THD_Voltage"),
                "THD_Voltage",
                "Voltage THD",
                1.8f64,
            ),
            Variable::new(
                &NodeId::new(ns, "THD_Current"),
                "THD_Current",
                "Current THD",
                4.5f64,
            ),
        ];
        address_space.add_variables(vars, &folder_id);
    }
    // Address space write lock is released here, before running the server.

    // Spawn background ticking task
    let nm = node_manager.clone();
    let subs = subscriptions.clone();
    let tick_duration = if delay_ms > 0 {
        Duration::from_millis(delay_ms)
    } else {
        Duration::from_secs(1)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tick_duration);
        let mut data = MeterData::new();

        let voltage_names = ["Voltage_L1", "Voltage_L2", "Voltage_L3"];
        let current_names = ["Current_L1", "Current_L2", "Current_L3"];

        loop {
            interval.tick().await;
            data.tick();

            // Update 3-phase voltage nodes
            for (i, name) in voltage_names.iter().enumerate() {
                let node_id = NodeId::new(ns, *name);
                if let Err(e) =
                    nm.set_value(&subs, &node_id, None, DataValue::new_now(data.voltage[i]))
                {
                    tracing::warn!("Failed to update {name}: {e:?}");
                }
            }

            // Update 3-phase current nodes
            for (i, name) in current_names.iter().enumerate() {
                let node_id = NodeId::new(ns, *name);
                if let Err(e) =
                    nm.set_value(&subs, &node_id, None, DataValue::new_now(data.current[i]))
                {
                    tracing::warn!("Failed to update {name}: {e:?}");
                }
            }

            // Update overall power factor (average of 3 phases)
            {
                let avg_pf: f64 = data.power_factor.iter().sum::<f64>() / 3.0;
                let node_id = NodeId::new(ns, "PowerFactor");
                if let Err(e) = nm.set_value(&subs, &node_id, None, DataValue::new_now(avg_pf)) {
                    tracing::warn!("Failed to update PowerFactor: {e:?}");
                }
            }

            // Update frequency
            {
                let node_id = NodeId::new(ns, "Frequency");
                if let Err(e) =
                    nm.set_value(&subs, &node_id, None, DataValue::new_now(data.frequency))
                {
                    tracing::warn!("Failed to update Frequency: {e:?}");
                }
            }

            // Update energy
            {
                let node_id = NodeId::new(ns, "Energy_kWh");
                if let Err(e) = nm.set_value(
                    &subs,
                    &node_id,
                    None,
                    DataValue::new_now(data.energy_active),
                ) {
                    tracing::warn!("Failed to update Energy_kWh: {e:?}");
                }
            }

            // Update THD voltage (average)
            {
                let avg: f64 = data.thd_voltage.iter().sum::<f64>() / 3.0;
                let node_id = NodeId::new(ns, "THD_Voltage");
                if let Err(e) = nm.set_value(&subs, &node_id, None, DataValue::new_now(avg)) {
                    tracing::warn!("Failed to update THD_Voltage: {e:?}");
                }
            }

            // Update THD current (average)
            {
                let avg: f64 = data.thd_current.iter().sum::<f64>() / 3.0;
                let node_id = NodeId::new(ns, "THD_Current");
                if let Err(e) = nm.set_value(&subs, &node_id, None, DataValue::new_now(avg)) {
                    tracing::warn!("Failed to update THD_Current: {e:?}");
                }
            }

            tracing::debug!(
                "V=[{:.1},{:.1},{:.1}] I=[{:.1},{:.1},{:.1}] PF={:.3} F={:.2}Hz E={:.1}kWh",
                data.voltage[0],
                data.voltage[1],
                data.voltage[2],
                data.current[0],
                data.current[1],
                data.current[2],
                data.power_factor.iter().sum::<f64>() / 3.0,
                data.frequency,
                data.energy_active,
            );
        }
    });

    tracing::info!("OPC-UA 3-phase energy meter simulator listening on {host}:{port}");
    tracing::info!("Endpoint: {endpoint_path}");
    if delay_ms > 0 {
        tracing::info!("PLC delay simulation: {delay_ms}ms between updates");
    }

    run_server(server, handle).await
}

async fn run_server(
    server: opcua::server::Server,
    handle: opcua::server::ServerHandle,
) -> Result<()> {
    // Register Ctrl-C handler
    let handle_c = handle.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Shutting down OPC-UA simulator");
        handle_c.cancel();
    });

    server
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {e}"))?;

    Ok(())
}
