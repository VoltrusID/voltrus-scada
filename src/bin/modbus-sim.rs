use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rand::Rng;
use tokio::net::{TcpListener, TcpStream};
use tokio_modbus::prelude::*;
use tokio_modbus::server::Service;
use tokio_modbus::server::tcp::{Server, accept_tcp_connection};

#[path = "../sim_profiles.rs"]
mod sim_profiles;
use sim_profiles::{DeviceProfile, Scenario};

// ── Fault injection ──

struct FaultConfig {
    delay: f64,
    disconnect: f64,
    garbage: f64,
}

// ── Device runtime state ──

struct DeviceState {
    profile: DeviceProfile,
    values: Vec<f64>, // current values (one per register definition)
    regs: Vec<u16>,   // raw register image (source of truth for reads)
    load_phase: f32,  // slow sinusoidal load variation
}

impl DeviceState {
    fn new(profile: DeviceProfile) -> Self {
        let values = profile.initial_values();
        let regs = profile.to_registers(&values);
        Self {
            profile,
            values,
            regs,
            load_phase: 0.0,
        }
    }

    fn tick(&mut self) {
        self.profile
            .tick_values(&mut self.values, &mut self.load_phase);
        self.regs = self.profile.to_registers(&self.values);
    }

    fn write_register(&mut self, addr: usize, word: u16) -> bool {
        let mut reg_offset = 0usize;
        for (idx, def) in self.profile.registers.iter().enumerate() {
            let width = if def.is_counter { 4 } else { 2 };
            if addr >= reg_offset && addr < reg_offset + width {
                self.regs[addr] = word;
                if def.is_counter {
                    let v = ((self.regs[reg_offset] as u64) << 48)
                        | ((self.regs[reg_offset + 1] as u64) << 32)
                        | ((self.regs[reg_offset + 2] as u64) << 16)
                        | (self.regs[reg_offset + 3] as u64);
                    self.values[idx] = v as f64;
                } else {
                    let bits =
                        ((self.regs[reg_offset] as u32) << 16) | (self.regs[reg_offset + 1] as u32);
                    self.values[idx] = f32::from_bits(bits) as f64;
                }
                return true;
            }
            reg_offset += width;
        }
        // Fallback: just write raw register
        self.regs[addr] = word;
        false
    }
}

// ── Modbus service ──

struct MeterService {
    data: Arc<Mutex<DeviceState>>,
    faults: Arc<FaultConfig>,
}

impl Service for MeterService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = std::future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let should_delay = {
            let mut rng = rand::rng();
            rng.random::<f64>() < self.faults.delay
        };
        if should_delay {
            tracing::warn!("Fault injection: delay on {req:?}");
        }

        let should_disconnect = {
            let mut rng = rand::rng();
            rng.random::<f64>() < self.faults.disconnect
        };
        if should_disconnect {
            tracing::warn!("Fault injection: disconnect on {req:?}");
            return std::future::ready(Err(ExceptionCode::ServerDeviceFailure));
        }

        let should_garbage = {
            let mut rng = rand::rng();
            rng.random::<f64>() < self.faults.garbage
        };

        let response = match &req {
            Request::ReadHoldingRegisters(addr, qty) => {
                let data = self.data.lock().unwrap();
                let addr = *addr as usize;
                let qty = *qty as usize;

                let regs: Vec<u16> = if should_garbage {
                    tracing::warn!("Fault injection: garbage addr={addr} qty={qty}");
                    let mut rng = rand::rng();
                    (0..qty).map(|_| rng.random::<u16>()).collect()
                } else {
                    let end = std::cmp::min(addr.saturating_add(qty), data.regs.len());
                    data.regs.get(addr..end).unwrap_or(&[]).to_vec()
                };

                Response::ReadHoldingRegisters(regs)
            }
            Request::WriteSingleRegister(req_addr, word) => {
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                if addr < data.regs.len() {
                    let mapped = data.write_register(addr, *word);
                    if mapped {
                        tracing::info!("Write FC06: addr={addr} value={word} → value updated");
                    } else {
                        tracing::info!("Write FC06: addr={addr} value={word} (raw)");
                    }
                    Response::WriteSingleRegister(*req_addr, *word)
                } else {
                    tracing::warn!("Write FC06: addr={addr} out of range");
                    return std::future::ready(Err(ExceptionCode::IllegalDataAddress));
                }
            }
            Request::WriteMultipleRegisters(req_addr, words) => {
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                let end = addr + words.len();
                if end <= data.regs.len() {
                    for (i, &word) in words.iter().enumerate() {
                        data.write_register(addr + i, word);
                    }
                    tracing::info!("Write FC16: addr={addr} qty={}", words.len());
                    Response::WriteMultipleRegisters(*req_addr, words.len() as u16)
                } else {
                    tracing::warn!(
                        "Write FC16: addr={addr}..{end} out of range (max={})",
                        data.regs.len()
                    );
                    return std::future::ready(Err(ExceptionCode::IllegalDataAddress));
                }
            }
            _ => {
                tracing::warn!("Unsupported function code: {req:?}");
                return std::future::ready(Err(ExceptionCode::IllegalFunction));
            }
        };

        std::future::ready(Ok(response))
    }
}

// ── Profile selection ──

fn select_profile(scenario: Scenario, role: &str) -> Option<DeviceProfile> {
    match (scenario, role) {
        // Solar Farm
        (Scenario::SolarFarm, "inverter" | "sma") => Some(sim_profiles::profile_sma_inverter()),
        (Scenario::SolarFarm, "grid-meter" | "pm5560" | "meter") => {
            Some(sim_profiles::profile_pm5560())
        }
        (Scenario::SolarFarm, "flow" | "flowmeter") => Some(sim_profiles::profile_eh_flowmeter()),
        (Scenario::SolarFarm, "pdu" | "raritan") => Some(sim_profiles::profile_raritan_pdu()),
        (Scenario::SolarFarm, "daniel" | "gas-flow") => Some(sim_profiles::profile_daniel_flow()),
        (Scenario::SolarFarm, "analyzer" | "abb") => Some(sim_profiles::fn_profile_abb_analyzer()),

        // Water Treatment
        (Scenario::WaterTreatment, "grid-meter" | "pm5560" | "meter") => {
            Some(sim_profiles::profile_pm5560())
        }
        (Scenario::WaterTreatment, "flow" | "flowmeter") => {
            Some(sim_profiles::profile_eh_flowmeter())
        }
        (Scenario::WaterTreatment, "analyzer" | "abb") => {
            Some(sim_profiles::fn_profile_abb_analyzer())
        }
        (Scenario::WaterTreatment, "inverter" | "sma") => {
            Some(sim_profiles::profile_sma_inverter())
        }
        (Scenario::WaterTreatment, "pdu" | "raritan") => Some(sim_profiles::profile_raritan_pdu()),
        (Scenario::WaterTreatment, "daniel" | "gas-flow") => {
            Some(sim_profiles::profile_daniel_flow())
        }

        // Factory Floor
        (Scenario::FactoryFloor, "grid-meter" | "pm5560" | "meter") => {
            Some(sim_profiles::profile_pm5560())
        }
        (Scenario::FactoryFloor, "flow" | "flowmeter") => {
            Some(sim_profiles::profile_eh_flowmeter())
        }
        (Scenario::FactoryFloor, "analyzer" | "abb") => {
            Some(sim_profiles::fn_profile_abb_analyzer())
        }
        (Scenario::FactoryFloor, "inverter" | "sma") => Some(sim_profiles::profile_sma_inverter()),
        (Scenario::FactoryFloor, "pdu" | "raritan") => Some(sim_profiles::profile_raritan_pdu()),
        (Scenario::FactoryFloor, "daniel" | "gas-flow") => {
            Some(sim_profiles::profile_daniel_flow())
        }

        // Data Center
        (Scenario::DataCenter, "grid-meter" | "pm5560" | "meter" | "ups") => {
            Some(sim_profiles::profile_pm5560())
        }
        (Scenario::DataCenter, "pdu" | "raritan") => Some(sim_profiles::profile_raritan_pdu()),
        (Scenario::DataCenter, "flow" | "flowmeter") => Some(sim_profiles::profile_eh_flowmeter()),
        (Scenario::DataCenter, "analyzer" | "abb") => Some(sim_profiles::fn_profile_abb_analyzer()),
        (Scenario::DataCenter, "inverter" | "sma") => Some(sim_profiles::profile_sma_inverter()),
        (Scenario::DataCenter, "daniel" | "gas-flow") => Some(sim_profiles::profile_daniel_flow()),

        // Oil & Gas
        (Scenario::OilGasWell, "daniel" | "gas-flow" | "flow-computer") => {
            Some(sim_profiles::profile_daniel_flow())
        }
        (Scenario::OilGasWell, "grid-meter" | "pm5560" | "meter") => {
            Some(sim_profiles::profile_pm5560())
        }
        (Scenario::OilGasWell, "flow" | "flowmeter") => Some(sim_profiles::profile_eh_flowmeter()),
        (Scenario::OilGasWell, "pdu" | "raritan") => Some(sim_profiles::profile_raritan_pdu()),
        (Scenario::OilGasWell, "analyzer" | "abb") => Some(sim_profiles::fn_profile_abb_analyzer()),
        (Scenario::OilGasWell, "inverter" | "sma") => Some(sim_profiles::profile_sma_inverter()),

        _ => None,
    }
}

// ── CLI ──

fn print_usage() {
    eprintln!("Usage: modbus-sim [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --port PORT           TCP port (default: 5020)");
    eprintln!("  --scenario SCENARIO   Study case (default: none → PM5560)");
    eprintln!("  --role ROLE           Device role within scenario (default: grid-meter)");
    eprintln!("  --delay PROB          Delay fault probability (0.0-1.0)");
    eprintln!("  --disconnect PROB     Disconnect fault probability (0.0-1.0)");
    eprintln!("  --garbage PROB        Garbage response probability (0.0-1.0)");
    eprintln!("  --list                List available scenarios and device profiles");
    eprintln!();
    eprintln!("Scenarios:");
    for s in Scenario::all() {
        eprintln!("  {:<20} {}", s.name(), s.description());
    }
    eprintln!();
    eprintln!("Device roles per scenario:");
    eprintln!("  inverter   — SMA Sunny Tripower 60TL-US (solar inverter)");
    eprintln!("  grid-meter — Schneider PM5560 power quality meter");
    eprintln!("  flowmeter  — Endress+Hauser Promag 10W flowmeter");
    eprintln!("  analyzer   — ABB AZL402 water quality analyzer");
    eprintln!("  gas-flow   — Daniel 1000 gas flow computer");
    eprintln!("  pdu        — Raritan PX3-5902V rack PDU");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  modbus-sim --scenario solar-farm --role inverter --port 5020");
    eprintln!("  modbus-sim --scenario water-treatment --role flowmeter --port 5021");
    eprintln!("  modbus-sim --scenario data-center --role pdu --port 5022");
    eprintln!("  modbus-sim --scenario oil-gas-well --role gas-flow --port 5023");
}

fn parse_args() -> Option<(u16, FaultConfig, DeviceProfile)> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage();
        std::process::exit(0);
    }

    if args.iter().any(|a| a == "--list") {
        println!("Available scenarios and device profiles:\n");
        for s in Scenario::all() {
            println!("  {} — {}", s.name(), s.description());
        }
        println!();
        println!("Device profiles:");
        let profiles = [
            ("inverter", sim_profiles::profile_sma_inverter()),
            ("grid-meter", sim_profiles::profile_pm5560()),
            ("flowmeter", sim_profiles::profile_eh_flowmeter()),
            ("analyzer", sim_profiles::fn_profile_abb_analyzer()),
            ("gas-flow", sim_profiles::profile_daniel_flow()),
            ("pdu", sim_profiles::profile_raritan_pdu()),
        ];
        for (role, p) in &profiles {
            println!(
                "  {:<12} {:<20} — {} ({} regs)",
                role,
                p.model,
                p.description,
                p.num_registers()
            );
            for r in &p.registers {
                let counter_note = if r.is_counter { " [u64 counter]" } else { "" };
                println!(
                    "              addr={:3} {} ({}){}",
                    r.name, r.unit, r.description, counter_note
                );
            }
        }
        std::process::exit(0);
    }

    let mut port: u16 = 5020;
    let mut faults = FaultConfig {
        delay: 0.0,
        disconnect: 0.0,
        garbage: 0.0,
    };
    let mut scenario_name: Option<String> = None;
    let mut role = "grid-meter".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                port = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5020);
                i += 2;
            }
            "--delay" => {
                faults.delay = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--disconnect" => {
                faults.disconnect = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--garbage" => {
                faults.garbage = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                i += 2;
            }
            "--scenario" => {
                scenario_name = args.get(i + 1).cloned();
                i += 2;
            }
            "--role" => {
                role = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let profile = if let Some(name) = &scenario_name {
        let scenario = Scenario::from_str(name).unwrap_or_else(|| {
            eprintln!("Unknown scenario: {name}");
            eprintln!("Run with --list to see available scenarios.");
            std::process::exit(1);
        });
        select_profile(scenario, &role).unwrap_or_else(|| {
            eprintln!("Role '{role}' not available for scenario {name}");
            eprintln!("Run with --list to see available profiles.");
            std::process::exit(1);
        })
    } else {
        // Default: Schneider PM5560
        sim_profiles::profile_pm5560()
    };

    Some((port, faults, profile))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let Some((port, faults, profile)) = parse_args() else {
        std::process::exit(0);
    };

    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("invalid address");

    let data = Arc::new(Mutex::new(DeviceState::new(profile.clone())));
    let faults = Arc::new(faults);

    // Background tick
    {
        let data = data.clone();
        let model = profile.model.to_string();
        let mfr = profile.manufacturer.to_string();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut d = data.lock().unwrap();
                d.tick();
                // Log first 5 register values as summary
                let vals: Vec<String> = d
                    .values
                    .iter()
                    .take(5)
                    .map(|v| format!("{:.1}", v))
                    .collect();
                tracing::debug!(
                    "{mfr} {model}: [{}] ({}/{} regs)",
                    vals.join(", "),
                    d.values.len(),
                    d.profile.num_registers(),
                );
            }
        });
    }

    tracing::info!(
        "{} {} simulator on {}",
        profile.manufacturer,
        profile.model,
        addr
    );
    tracing::info!(
        "  {} ({}, {} u16 registers)",
        profile.description,
        profile.manufacturer,
        profile.num_registers()
    );
    tracing::info!(
        "Fault injection: delay={:.0}% disconnect={:.0}% garbage={:.0}%",
        faults.delay * 100.0,
        faults.disconnect * 100.0,
        faults.garbage * 100.0
    );

    // Print register map
    tracing::info!("Register map:");
    let mut reg_addr = 0u16;
    for r in &profile.registers {
        let counter_note = if r.is_counter { " [u64]" } else { "" };
        tracing::info!(
            "  {:4}-{:<4} {:<20} ({}) [{}..{}]{}",
            reg_addr,
            reg_addr + if r.is_counter { 3 } else { 1 },
            r.name,
            r.unit,
            r.min,
            r.max,
            counter_note
        );
        reg_addr += if r.is_counter { 4 } else { 2 };
    }

    let listener = TcpListener::bind(addr).await?;
    let server = Server::new(listener);

    let service_data = data.clone();
    let service_faults = faults.clone();

    let new_service = move |_socket_addr: SocketAddr| -> io::Result<Option<MeterService>> {
        Ok(Some(MeterService {
            data: service_data.clone(),
            faults: service_faults.clone(),
        }))
    };

    let on_connected = |stream: TcpStream, socket_addr: SocketAddr| {
        let new_service = &new_service;
        async move { accept_tcp_connection(stream, socket_addr, new_service) }
    };

    let on_process_error = |err: io::Error| {
        tracing::error!("Connection error: {err}");
    };

    server.serve(&on_connected, on_process_error).await?;

    Ok(())
}
