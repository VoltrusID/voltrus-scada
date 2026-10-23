//! Shared simulator data profiles for realistic industrial scenarios.

#![allow(dead_code)]
//!
//! Each profile models a real device category with authentic register maps,
//! realistic initial values, physical ranges, and drift characteristics.
//!
//! Used by all *-sim binaries via `#[path = "sim_profiles.rs"]`.
//!
//! # Scenarios
//!
//! | Scenario            | Devices                                                |
//! |---------------------|--------------------------------------------------------|
//! | `solar-farm`        | SMA inverter, PM5560 grid meter, S7 irradiance, MQTT  |
//! | `water-treatment`   | S7-1200 process PLC, E+H flowmeter, ABB analyzer, BACnet AHU |
//! | `factory-floor`     | CompactLogix L33ER, PM5560 main inlet, S7-1200 CIP, DNP3 genset |
//! | `data-center`       | Raritan PX3 PDU, PM5560 UPS, BACnet CRAC, MQTT env, OPC-UA DCIM |
//! | `oil-gas-well`      | DNP3 SEL-351S, DNP3 ROC800, Modbus flow computer, S7, MQTT |

use rand::Rng;
use std::fmt;

// ── Scenario identifier ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    SolarFarm,
    WaterTreatment,
    FactoryFloor,
    DataCenter,
    OilGasWell,
}

impl Scenario {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "solar-farm" => Some(Self::SolarFarm),
            "water-treatment" => Some(Self::WaterTreatment),
            "factory-floor" => Some(Self::FactoryFloor),
            "data-center" => Some(Self::DataCenter),
            "oil-gas-well" => Some(Self::OilGasWell),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SolarFarm => "solar-farm",
            Self::WaterTreatment => "water-treatment",
            Self::FactoryFloor => "factory-floor",
            Self::DataCenter => "data-center",
            Self::OilGasWell => "oil-gas-well",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::SolarFarm => "100kW Rooftop PV Plant — SMA Tripower + PM5560 + S7-1200",
            Self::WaterTreatment => {
                "Municipal Water Treatment — S7-1200 + E+H Flow + ABB Analyzer + BACnet HVAC"
            }
            Self::FactoryFloor => {
                "Food & Beverage Factory — CompactLogix + PM5560 + S7-1200 CIP + DNP3 Genset"
            }
            Self::DataCenter => "Tier-III Data Center — PDU + UPS + CRAC + Env Sensors + DCIM",
            Self::OilGasWell => "Remote Well Pad — SEL-351S + ROC800 + Daniel Flow + S7 Compressor",
        }
    }

    pub fn all() -> &'static [Scenario] {
        &[
            Scenario::SolarFarm,
            Scenario::WaterTreatment,
            Scenario::FactoryFloor,
            Scenario::DataCenter,
            Scenario::OilGasWell,
        ]
    }
}

impl fmt::Display for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ── Device profile (used by Modbus simulator) ──

#[derive(Debug, Clone)]
pub struct RegisterDef {
    pub name: &'static str,
    pub unit: &'static str,
    pub description: &'static str,
    /// Initial value (Float32). For u64 counters, use 0.0 and set `is_counter: true`.
    pub initial: f32,
    pub min: f32,
    pub max: f32,
    /// Per-tick drift magnitude (random walk step size)
    pub drift: f32,
    /// If true, this is a monotonically-increasing u64 counter (4 registers).
    /// The `initial` field is the starting value, drift is the base rate per tick.
    pub is_counter: bool,
    /// Counter increment per tick (used if is_counter). Applied as base + random variation.
    pub counter_rate: f64,
}

#[derive(Debug, Clone)]
pub struct DeviceProfile {
    pub manufacturer: &'static str,
    pub model: &'static str,
    pub description: &'static str,
    /// (scenario, role_within_scenario) -> profile
    /// Each register occupies 2 u16 (Float32) or 4 u16 (u64 counter)
    pub registers: Vec<RegisterDef>,
}

impl DeviceProfile {
    /// Total number of u16 holding registers needed.
    pub fn num_registers(&self) -> usize {
        self.registers
            .iter()
            .map(|r| if r.is_counter { 4 } else { 2 })
            .sum()
    }

    /// Initialize all register values into a u16 array.
    pub fn to_registers(&self, current_values: &[f64]) -> Vec<u16> {
        let n = self.num_registers();
        let mut regs = vec![0u16; n];
        let mut i = 0;

        for (idx, def) in self.registers.iter().enumerate() {
            let val = current_values[idx];
            if def.is_counter {
                let v = val as u64;
                regs[i] = ((v >> 48) & 0xFFFF) as u16;
                regs[i + 1] = ((v >> 32) & 0xFFFF) as u16;
                regs[i + 2] = ((v >> 16) & 0xFFFF) as u16;
                regs[i + 3] = (v & 0xFFFF) as u16;
                i += 4;
            } else {
                let bits = (val as f32).to_bits();
                regs[i] = (bits >> 16) as u16;
                regs[i + 1] = bits as u16;
                i += 2;
            }
        }

        regs
    }

    /// Initialize current_values from register definitions.
    pub fn initial_values(&self) -> Vec<f64> {
        self.registers.iter().map(|r| r.initial as f64).collect()
    }

    /// Tick all values forward one second.
    pub fn tick_values(&self, values: &mut [f64], load_factor: &mut f32) {
        let mut rng = rand::rng();

        // Slowly-varying load factor (sinusoidal with noise)
        *load_factor =
            (*load_factor + 2.0 * std::f32::consts::PI / 900.0 + rng.random_range(-0.001..0.001))
                % (2.0 * std::f32::consts::PI);
        let load = 1.0 + 0.15 * load_factor.sin();

        for (i, def) in self.registers.iter().enumerate() {
            if def.is_counter {
                // Monotonically increasing counter
                let variation = (rng.random::<f64>() - 0.5) * def.counter_rate * 0.1;
                values[i] += def.counter_rate + variation;
            } else {
                let drift = rng.random_range(-def.drift..def.drift);
                // Apply load factor scaling to power/current-type registers
                let load_effect = if def.unit == "W"
                    || def.unit == "A"
                    || def.unit == "VA"
                    || def.unit == "var"
                {
                    (load - 1.0) * def.initial.abs() * 0.1
                } else if def.unit == "V" || def.unit == "kV" {
                    -(load - 1.0) * 0.5 // voltage sag under load
                } else {
                    0.0
                };
                values[i] = (values[i] + drift as f64 + load_effect as f64)
                    .clamp(def.min as f64, def.max as f64);
            }
        }
    }
}

// ── All device profiles ──

/// SMA Sunny Tripower 60TL-US inverter (Modbus TCP, SunSpec + SMA registers)
/// 60 kW, 3-phase, used in commercial rooftop PV.
pub fn profile_sma_inverter() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "SMA",
        model: "STP 60-US-10",
        description: "Sunny Tripower 60TL-US 60kW String Inverter",
        registers: vec![
            RegisterDef {
                name: "DC_Voltage_A",
                unit: "V",
                description: "DC input voltage string A",
                initial: 580.0,
                min: 0.0,
                max: 800.0,
                drift: 2.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "DC_Voltage_B",
                unit: "V",
                description: "DC input voltage string B",
                initial: 575.0,
                min: 0.0,
                max: 800.0,
                drift: 2.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "DC_Current_A",
                unit: "A",
                description: "DC input current string A",
                initial: 52.0,
                min: 0.0,
                max: 110.0,
                drift: 1.5,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "DC_Current_B",
                unit: "A",
                description: "DC input current string B",
                initial: 49.0,
                min: 0.0,
                max: 110.0,
                drift: 1.5,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "DC_Power",
                unit: "W",
                description: "Total DC input power",
                initial: 58000.0,
                min: 0.0,
                max: 62000.0,
                drift: 500.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Voltage_L1",
                unit: "V",
                description: "AC output voltage L1-N",
                initial: 230.5,
                min: 207.0,
                max: 253.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Voltage_L2",
                unit: "V",
                description: "AC output voltage L2-N",
                initial: 229.8,
                min: 207.0,
                max: 253.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Voltage_L3",
                unit: "V",
                description: "AC output voltage L3-N",
                initial: 230.2,
                min: 207.0,
                max: 253.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Current_L1",
                unit: "A",
                description: "AC output current L1",
                initial: 86.5,
                min: 0.0,
                max: 100.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Current_L2",
                unit: "A",
                description: "AC output current L2",
                initial: 84.2,
                min: 0.0,
                max: 100.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Current_L3",
                unit: "A",
                description: "AC output current L3",
                initial: 85.8,
                min: 0.0,
                max: 100.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Power",
                unit: "W",
                description: "Total AC active power output",
                initial: 56500.0,
                min: 0.0,
                max: 60000.0,
                drift: 400.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "AC_Frequency",
                unit: "Hz",
                description: "Grid frequency",
                initial: 50.01,
                min: 49.8,
                max: 50.2,
                drift: 0.003,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Efficiency",
                unit: "%",
                description: "Current conversion efficiency",
                initial: 97.4,
                min: 90.0,
                max: 98.5,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "PowerFactor",
                unit: "",
                description: "AC power factor (displacement)",
                initial: 0.99,
                min: 0.90,
                max: 1.0,
                drift: 0.002,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature",
                unit: "°C",
                description: "Inverter heatsink temperature",
                initial: 45.2,
                min: 10.0,
                max: 75.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Irradiance",
                unit: "W/m²",
                description: "Plane-of-array irradiance (from sensor)",
                initial: 856.0,
                min: 0.0,
                max: 1100.0,
                drift: 15.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Daily_Yield",
                unit: "kWh",
                description: "Energy produced today",
                initial: 185.0,
                min: 0.0,
                max: 400.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 56.5 / 3600.0,
            },
            RegisterDef {
                name: "Total_Yield",
                unit: "kWh",
                description: "Lifetime energy produced",
                initial: 452380.0,
                min: 0.0,
                max: 9999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 56.5 / 3600.0,
            },
            RegisterDef {
                name: "Operating_Time",
                unit: "h",
                description: "Total operating hours",
                initial: 8920.0,
                min: 0.0,
                max: 999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 1.0 / 3600.0,
            },
        ],
    }
}

/// Schneider Electric PM5560 power quality meter (Modbus TCP).
/// 0.2S accuracy, CT/VT connected, revenue-grade energy metering.
/// Used as grid interconnection meter in solar, main incomer in factories.
pub fn profile_pm5560() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "Schneider Electric",
        model: "PM5560",
        description: "PM5560 3-Phase Power Quality Meter (0.2S, CT/VT)",
        registers: vec![
            RegisterDef {
                name: "Voltage_L1N",
                unit: "V",
                description: "L1-N voltage",
                initial: 230.2,
                min: 0.0,
                max: 347.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Voltage_L2N",
                unit: "V",
                description: "L2-N voltage",
                initial: 229.8,
                min: 0.0,
                max: 347.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Voltage_L3N",
                unit: "V",
                description: "L3-N voltage",
                initial: 230.5,
                min: 0.0,
                max: 347.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L1",
                unit: "A",
                description: "L1 current (via CT)",
                initial: 142.5,
                min: 0.0,
                max: 6000.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L2",
                unit: "A",
                description: "L2 current (via CT)",
                initial: 138.7,
                min: 0.0,
                max: 6000.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L3",
                unit: "A",
                description: "L3 current (via CT)",
                initial: 145.3,
                min: 0.0,
                max: 6000.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Active_Power",
                unit: "W",
                description: "3-phase active power",
                initial: 95000.0,
                min: -999999.0,
                max: 999999.0,
                drift: 500.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Reactive_Power",
                unit: "var",
                description: "3-phase reactive power",
                initial: -12000.0,
                min: -999999.0,
                max: 999999.0,
                drift: 200.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Apparent_Power",
                unit: "VA",
                description: "3-phase apparent power",
                initial: 98000.0,
                min: 0.0,
                max: 999999.0,
                drift: 500.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Power_Factor",
                unit: "",
                description: "Overall power factor",
                initial: 0.96,
                min: -1.0,
                max: 1.0,
                drift: 0.003,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Frequency",
                unit: "Hz",
                description: "Grid frequency",
                initial: 50.0,
                min: 45.0,
                max: 65.0,
                drift: 0.002,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "THD_Voltage",
                unit: "%",
                description: "Avg voltage THD",
                initial: 1.8,
                min: 0.0,
                max: 50.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "THD_Current",
                unit: "%",
                description: "Avg current THD",
                initial: 6.2,
                min: 0.0,
                max: 50.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature",
                unit: "°C",
                description: "Internal temperature",
                initial: 38.5,
                min: -40.0,
                max: 85.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Energy_Import",
                unit: "kWh",
                description: "Cumulative active energy import",
                initial: 1587329.0,
                min: 0.0,
                max: 999999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 95.0 / 3600.0,
            },
            RegisterDef {
                name: "Energy_Export",
                unit: "kWh",
                description: "Cumulative active energy export",
                initial: 2451.0,
                min: 0.0,
                max: 999999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 0.5 / 3600.0,
            },
        ],
    }
}

/// Endress+Hauser Promag 10W electromagnetic flowmeter (Modbus TCP).
/// DN50, 0–50 m³/h, for water treatment applications.
pub fn profile_eh_flowmeter() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "Endress+Hauser",
        model: "Promag 10W DN50",
        description: "Electromagnetic flowmeter, DN50, 0-50 m³/h",
        registers: vec![
            RegisterDef {
                name: "Flow_Rate",
                unit: "m³/h",
                description: "Instantaneous volume flow rate",
                initial: 12.5,
                min: 0.0,
                max: 50.0,
                drift: 0.3,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Flow_Velocity",
                unit: "m/s",
                description: "Flow velocity in pipe",
                initial: 1.77,
                min: 0.0,
                max: 7.0,
                drift: 0.05,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Volume_Total",
                unit: "m³",
                description: "Cumulative volume totalizer",
                initial: 245891.0,
                min: 0.0,
                max: 999999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 12.5 / 3600.0,
            },
            RegisterDef {
                name: "Conductivity",
                unit: "µS/cm",
                description: "Fluid conductivity",
                initial: 520.0,
                min: 0.0,
                max: 5000.0,
                drift: 5.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature",
                unit: "°C",
                description: "Process fluid temperature",
                initial: 15.2,
                min: -20.0,
                max: 60.0,
                drift: 0.05,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Pipe_Volume",
                unit: "%",
                description: "Pipe fill level",
                initial: 100.0,
                min: 0.0,
                max: 100.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Signal_Quality",
                unit: "%",
                description: "Measurement signal quality",
                initial: 98.5,
                min: 0.0,
                max: 100.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
        ],
    }
}

/// ABB AZL402 water quality analyzer (Modbus RTU/TCP).
/// Measures pH, ORP, turbidity, dissolved oxygen, conductivity, temperature.
pub fn fn_profile_abb_analyzer() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "ABB",
        model: "AZL402",
        description: "Water Quality Analyzer (pH, ORP, Turbidity, DO, Cond, Temp)",
        registers: vec![
            RegisterDef {
                name: "pH",
                unit: "pH",
                description: "pH value",
                initial: 7.2,
                min: 0.0,
                max: 14.0,
                drift: 0.02,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "ORP",
                unit: "mV",
                description: "Oxidation-reduction potential",
                initial: 245.0,
                min: -2000.0,
                max: 2000.0,
                drift: 2.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Turbidity",
                unit: "NTU",
                description: "Water turbidity",
                initial: 0.45,
                min: 0.0,
                max: 100.0,
                drift: 0.02,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Dissolved_Oxygen",
                unit: "mg/L",
                description: "Dissolved oxygen concentration",
                initial: 8.2,
                min: 0.0,
                max: 20.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Conductivity",
                unit: "µS/cm",
                description: "Electrical conductivity",
                initial: 485.0,
                min: 0.0,
                max: 5000.0,
                drift: 3.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Chlorine_Free",
                unit: "mg/L",
                description: "Free chlorine residual",
                initial: 0.85,
                min: 0.0,
                max: 5.0,
                drift: 0.02,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature",
                unit: "°C",
                description: "Water temperature",
                initial: 15.8,
                min: 0.0,
                max: 40.0,
                drift: 0.02,
                is_counter: false,
                counter_rate: 0.0,
            },
        ],
    }
}

/// Daniel 1000 Flow Computer (Modbus TCP).
/// Gas orifice metering with pressure, temperature, differential pressure,
/// flow rate, and gas chromatograph composition.
pub fn profile_daniel_flow() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "Emerson",
        model: "Daniel 1000",
        description: "Gas Flow Computer (orifice meter, AGA3/AGA8)",
        registers: vec![
            RegisterDef {
                name: "Diff_Pressure",
                unit: "inH2O",
                description: "Orifice differential pressure",
                initial: 85.0,
                min: 0.0,
                max: 400.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Static_Pressure",
                unit: "psig",
                description: "Line static pressure",
                initial: 745.0,
                min: 0.0,
                max: 1500.0,
                drift: 2.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature",
                unit: "°F",
                description: "Gas temperature",
                initial: 95.0,
                min: -40.0,
                max: 200.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Flow_Rate",
                unit: "MSCFH",
                description: "Instantaneous gas flow rate",
                initial: 1250.0,
                min: 0.0,
                max: 5000.0,
                drift: 20.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Volume_Today",
                unit: "MSCF",
                description: "Accumulated gas volume today",
                initial: 18500.0,
                min: 0.0,
                max: 120000.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 1250.0 / 3600.0,
            },
            RegisterDef {
                name: "Volume_Total",
                unit: "MMSCF",
                description: "Lifetime accumulated volume",
                initial: 8452.3,
                min: 0.0,
                max: 999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 1.25 / 3600.0,
            },
            RegisterDef {
                name: "Gas_Gravity",
                unit: "",
                description: "Specific gravity (air=1)",
                initial: 0.62,
                min: 0.5,
                max: 1.0,
                drift: 0.001,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Heating_Value",
                unit: "BTU/SCF",
                description: "Higher heating value",
                initial: 1025.0,
                min: 900.0,
                max: 1200.0,
                drift: 1.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "CH4",
                unit: "mol%",
                description: "Methane fraction",
                initial: 88.5,
                min: 0.0,
                max: 100.0,
                drift: 0.05,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "CO2",
                unit: "mol%",
                description: "Carbon dioxide fraction",
                initial: 1.8,
                min: 0.0,
                max: 30.0,
                drift: 0.02,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "H2S",
                unit: "ppm",
                description: "Hydrogen sulfide",
                initial: 12.0,
                min: 0.0,
                max: 5000.0,
                drift: 0.5,
                is_counter: false,
                counter_rate: 0.0,
            },
        ],
    }
}

/// Raritan PX3-5902V intelligent PDU (Modbus TCP).
/// 3-phase, 32A, per-outlet metering. Data center rack PDU.
pub fn profile_raritan_pdu() -> DeviceProfile {
    DeviceProfile {
        manufacturer: "Raritan",
        model: "PX3-5902V",
        description: "Intelligent Rack PDU, 3-phase 32A, 24 outlets",
        registers: vec![
            RegisterDef {
                name: "Voltage_L1",
                unit: "V",
                description: "L1-N voltage at PDU inlet",
                initial: 208.5,
                min: 180.0,
                max: 240.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Voltage_L2",
                unit: "V",
                description: "L2-N voltage at PDU inlet",
                initial: 209.1,
                min: 180.0,
                max: 240.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Voltage_L3",
                unit: "V",
                description: "L3-N voltage at PDU inlet",
                initial: 208.8,
                min: 180.0,
                max: 240.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L1",
                unit: "A",
                description: "L1 branch circuit current",
                initial: 18.5,
                min: 0.0,
                max: 32.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L2",
                unit: "A",
                description: "L2 branch circuit current",
                initial: 17.2,
                min: 0.0,
                max: 32.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Current_L3",
                unit: "A",
                description: "L3 branch circuit current",
                initial: 19.1,
                min: 0.0,
                max: 32.0,
                drift: 0.2,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Active_Power",
                unit: "W",
                description: "Total PDU active power",
                initial: 8650.0,
                min: 0.0,
                max: 13300.0,
                drift: 50.0,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Power_Factor",
                unit: "",
                description: "PDU overall power factor",
                initial: 0.98,
                min: 0.0,
                max: 1.0,
                drift: 0.005,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Energy_Total",
                unit: "kWh",
                description: "Cumulative energy",
                initial: 42580.0,
                min: 0.0,
                max: 999999999.0,
                drift: 0.0,
                is_counter: true,
                counter_rate: 8.65 / 3600.0,
            },
            RegisterDef {
                name: "Temperature_Inlet",
                unit: "°C",
                description: "PDU inlet air temperature",
                initial: 24.5,
                min: 10.0,
                max: 50.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Temperature_Outlet",
                unit: "°C",
                description: "PDU outlet air temperature",
                initial: 32.8,
                min: 10.0,
                max: 60.0,
                drift: 0.1,
                is_counter: false,
                counter_rate: 0.0,
            },
            RegisterDef {
                name: "Humidity",
                unit: "%",
                description: "Rack relative humidity",
                initial: 45.0,
                min: 10.0,
                max: 90.0,
                drift: 0.5,
                is_counter: false,
                counter_rate: 0.0,
            },
        ],
    }
}

// ── S7 PLC data block profiles ──

#[derive(Debug, Clone)]
pub struct S7DbProfile {
    pub db_number: u16,
    pub description: &'static str,
    /// (name, offset, initial_value, min, max, drift)
    pub tags: Vec<S7TagDef>,
}

#[derive(Debug, Clone)]
pub struct S7TagDef {
    pub name: &'static str,
    pub offset: usize,
    pub initial: f32,
    pub min: f32,
    pub max: f32,
    pub drift: f32,
    pub unit: &'static str,
}

/// S7-1200 DB1: Solar irradiance & weather station.
/// Models a Kipp & Zonen SMP10 pyranometer + Vaisala WXT536 weather sensor
/// connected to a Siemens S7-1200 via analog inputs.
pub fn s7_profile_solar_weather() -> Vec<S7DbProfile> {
    vec![S7DbProfile {
        db_number: 1,
        description: "Solar irradiance & weather (Kipp&Zonen SMP10 + Vaisala WXT536)",
        tags: vec![
            S7TagDef {
                name: "Irradiance_POA",
                offset: 0,
                initial: 856.0,
                min: 0.0,
                max: 1400.0,
                drift: 15.0,
                unit: "W/m²",
            },
            S7TagDef {
                name: "Irradiance_GHI",
                offset: 4,
                initial: 790.0,
                min: 0.0,
                max: 1200.0,
                drift: 12.0,
                unit: "W/m²",
            },
            S7TagDef {
                name: "Ambient_Temp",
                offset: 8,
                initial: 28.5,
                min: -10.0,
                max: 50.0,
                drift: 0.1,
                unit: "°C",
            },
            S7TagDef {
                name: "Module_Temp",
                offset: 12,
                initial: 42.0,
                min: -10.0,
                max: 70.0,
                drift: 0.2,
                unit: "°C",
            },
            S7TagDef {
                name: "Wind_Speed",
                offset: 16,
                initial: 3.2,
                min: 0.0,
                max: 30.0,
                drift: 0.3,
                unit: "m/s",
            },
            S7TagDef {
                name: "Wind_Direction",
                offset: 20,
                initial: 185.0,
                min: 0.0,
                max: 360.0,
                drift: 2.0,
                unit: "°",
            },
            S7TagDef {
                name: "Humidity",
                offset: 24,
                initial: 62.0,
                min: 10.0,
                max: 100.0,
                drift: 0.5,
                unit: "%",
            },
            S7TagDef {
                name: "Rain_Daily",
                offset: 28,
                initial: 0.0,
                min: 0.0,
                max: 50.0,
                drift: 0.0,
                unit: "mm",
            },
            S7TagDef {
                name: "Tracker_Azimuth",
                offset: 32,
                initial: 185.0,
                min: 0.0,
                max: 360.0,
                drift: 0.5,
                unit: "°",
            },
            S7TagDef {
                name: "Tracker_Elevation",
                offset: 36,
                initial: 52.0,
                min: 0.0,
                max: 90.0,
                drift: 0.2,
                unit: "°",
            },
            S7TagDef {
                name: "Soiling_Index",
                offset: 40,
                initial: 2.1,
                min: 0.0,
                max: 20.0,
                drift: 0.01,
                unit: "%",
            },
        ],
    }]
}

/// S7-1200 DB1-DB3: Water treatment process controller.
/// Models coagulation/flocculation, filtration, and disinfection stages.
pub fn s7_profile_water_treatment() -> Vec<S7DbProfile> {
    vec![
        S7DbProfile {
            db_number: 1,
            description: "Coagulation/Flocculation Stage",
            tags: vec![
                S7TagDef {
                    name: "Raw_Water_pH",
                    offset: 0,
                    initial: 7.8,
                    min: 6.0,
                    max: 9.0,
                    drift: 0.02,
                    unit: "pH",
                },
                S7TagDef {
                    name: "Raw_Water_Turbidity",
                    offset: 4,
                    initial: 15.2,
                    min: 0.0,
                    max: 100.0,
                    drift: 0.5,
                    unit: "NTU",
                },
                S7TagDef {
                    name: "Raw_Water_Flow",
                    offset: 8,
                    initial: 250.0,
                    min: 0.0,
                    max: 500.0,
                    drift: 2.0,
                    unit: "m³/h",
                },
                S7TagDef {
                    name: "Coagulant_Dose",
                    offset: 12,
                    initial: 12.5,
                    min: 0.0,
                    max: 50.0,
                    drift: 0.1,
                    unit: "mg/L",
                },
                S7TagDef {
                    name: "Coagulant_Tank_Level",
                    offset: 16,
                    initial: 72.0,
                    min: 0.0,
                    max: 100.0,
                    drift: 0.05,
                    unit: "%",
                },
                S7TagDef {
                    name: "Flash_Mix_Speed",
                    offset: 20,
                    initial: 180.0,
                    min: 0.0,
                    max: 300.0,
                    drift: 1.0,
                    unit: "RPM",
                },
                S7TagDef {
                    name: "Flocculator_Speed",
                    offset: 24,
                    initial: 45.0,
                    min: 0.0,
                    max: 80.0,
                    drift: 0.5,
                    unit: "RPM",
                },
            ],
        },
        S7DbProfile {
            db_number: 2,
            description: "Rapid Sand Filtration Stage",
            tags: vec![
                S7TagDef {
                    name: "Filter_1_Headloss",
                    offset: 0,
                    initial: 0.85,
                    min: 0.0,
                    max: 3.0,
                    drift: 0.005,
                    unit: "m",
                },
                S7TagDef {
                    name: "Filter_2_Headloss",
                    offset: 4,
                    initial: 1.12,
                    min: 0.0,
                    max: 3.0,
                    drift: 0.005,
                    unit: "m",
                },
                S7TagDef {
                    name: "Filter_3_Headloss",
                    offset: 8,
                    initial: 0.72,
                    min: 0.0,
                    max: 3.0,
                    drift: 0.005,
                    unit: "m",
                },
                S7TagDef {
                    name: "Filter_1_Turbidity",
                    offset: 12,
                    initial: 0.12,
                    min: 0.0,
                    max: 5.0,
                    drift: 0.005,
                    unit: "NTU",
                },
                S7TagDef {
                    name: "Filter_2_Turbidity",
                    offset: 16,
                    initial: 0.08,
                    min: 0.0,
                    max: 5.0,
                    drift: 0.005,
                    unit: "NTU",
                },
                S7TagDef {
                    name: "Filter_3_Turbidity",
                    offset: 20,
                    initial: 0.15,
                    min: 0.0,
                    max: 5.0,
                    drift: 0.005,
                    unit: "NTU",
                },
                S7TagDef {
                    name: "Filtered_Flow",
                    offset: 24,
                    initial: 245.0,
                    min: 0.0,
                    max: 500.0,
                    drift: 1.5,
                    unit: "m³/h",
                },
                S7TagDef {
                    name: "Backwash_Phase",
                    offset: 28,
                    initial: 0.0,
                    min: 0.0,
                    max: 4.0,
                    drift: 0.0,
                    unit: "",
                },
            ],
        },
        S7DbProfile {
            db_number: 3,
            description: "Disinfection & Clearwell",
            tags: vec![
                S7TagDef {
                    name: "Chlorine_Dose",
                    offset: 0,
                    initial: 2.5,
                    min: 0.0,
                    max: 5.0,
                    drift: 0.02,
                    unit: "mg/L",
                },
                S7TagDef {
                    name: "Chlorine_Residual",
                    offset: 4,
                    initial: 0.85,
                    min: 0.0,
                    max: 5.0,
                    drift: 0.01,
                    unit: "mg/L",
                },
                S7TagDef {
                    name: "CT_Value",
                    offset: 8,
                    initial: 45.0,
                    min: 0.0,
                    max: 200.0,
                    drift: 0.5,
                    unit: "mg·min/L",
                },
                S7TagDef {
                    name: "Clearwell_Level",
                    offset: 12,
                    initial: 65.0,
                    min: 0.0,
                    max: 100.0,
                    drift: 0.1,
                    unit: "%",
                },
                S7TagDef {
                    name: "Treated_pH",
                    offset: 16,
                    initial: 7.2,
                    min: 6.5,
                    max: 8.5,
                    drift: 0.01,
                    unit: "pH",
                },
                S7TagDef {
                    name: "Treated_Turbidity",
                    offset: 20,
                    initial: 0.05,
                    min: 0.0,
                    max: 1.0,
                    drift: 0.002,
                    unit: "NTU",
                },
                S7TagDef {
                    name: "Distribution_Pressure",
                    offset: 24,
                    initial: 4.2,
                    min: 0.0,
                    max: 8.0,
                    drift: 0.05,
                    unit: "bar",
                },
                S7TagDef {
                    name: "Output_Flow",
                    offset: 28,
                    initial: 248.0,
                    min: 0.0,
                    max: 500.0,
                    drift: 2.0,
                    unit: "m³/h",
                },
            ],
        },
    ]
}

/// S7-1200 DB1: Food & Beverage CIP (Clean-In-Place) system.
pub fn s7_profile_cip_system() -> Vec<S7DbProfile> {
    vec![S7DbProfile {
        db_number: 1,
        description: "CIP Cleaning System (Tanks + Circuits)",
        tags: vec![
            S7TagDef {
                name: "CIP_Phase",
                offset: 0,
                initial: 3.0,
                min: 0.0,
                max: 6.0,
                drift: 0.0,
                unit: "",
            }, // 0=idle,1=prerinse,2=caustic,3=rinse,4=acid,5=final,6=sanitize
            S7TagDef {
                name: "Caustic_Temp",
                offset: 4,
                initial: 72.0,
                min: 20.0,
                max: 85.0,
                drift: 0.2,
                unit: "°C",
            },
            S7TagDef {
                name: "Caustic_Conc",
                offset: 8,
                initial: 2.5,
                min: 0.0,
                max: 5.0,
                drift: 0.01,
                unit: "%",
            },
            S7TagDef {
                name: "Acid_Temp",
                offset: 12,
                initial: 65.0,
                min: 20.0,
                max: 80.0,
                drift: 0.2,
                unit: "°C",
            },
            S7TagDef {
                name: "Acid_Conc",
                offset: 16,
                initial: 1.2,
                min: 0.0,
                max: 3.0,
                drift: 0.01,
                unit: "%",
            },
            S7TagDef {
                name: "Circuit_Flow",
                offset: 20,
                initial: 15.0,
                min: 0.0,
                max: 30.0,
                drift: 0.3,
                unit: "m³/h",
            },
            S7TagDef {
                name: "Circuit_Pressure",
                offset: 24,
                initial: 3.5,
                min: 0.0,
                max: 6.0,
                drift: 0.05,
                unit: "bar",
            },
            S7TagDef {
                name: "Conductivity",
                offset: 28,
                initial: 12.5,
                min: 0.0,
                max: 50.0,
                drift: 0.1,
                unit: "mS/cm",
            },
            S7TagDef {
                name: "Return_Temp",
                offset: 32,
                initial: 68.0,
                min: 20.0,
                max: 85.0,
                drift: 0.2,
                unit: "°C",
            },
            S7TagDef {
                name: "Timer_Remaining",
                offset: 36,
                initial: 480.0,
                min: 0.0,
                max: 3600.0,
                drift: -1.0,
                unit: "s",
            },
            S7TagDef {
                name: "CIP_Tank_Level",
                offset: 40,
                initial: 75.0,
                min: 0.0,
                max: 100.0,
                drift: 0.1,
                unit: "%",
            },
        ],
    }]
}

/// S7-1200 DB1: Natural gas compressor controller.
pub fn s7_profile_compressor() -> Vec<S7DbProfile> {
    vec![S7DbProfile {
        db_number: 1,
        description: "Gas Compressor Controller (Waukesha/Ariel)",
        tags: vec![
            S7TagDef {
                name: "Suction_Pressure",
                offset: 0,
                initial: 745.0,
                min: 0.0,
                max: 1500.0,
                drift: 2.0,
                unit: "psig",
            },
            S7TagDef {
                name: "Discharge_Pressure",
                offset: 4,
                initial: 1200.0,
                min: 0.0,
                max: 2000.0,
                drift: 5.0,
                unit: "psig",
            },
            S7TagDef {
                name: "Suction_Temp",
                offset: 8,
                initial: 95.0,
                min: 30.0,
                max: 150.0,
                drift: 0.1,
                unit: "°F",
            },
            S7TagDef {
                name: "Discharge_Temp",
                offset: 12,
                initial: 165.0,
                min: 80.0,
                max: 250.0,
                drift: 0.2,
                unit: "°F",
            },
            S7TagDef {
                name: "Compressor_Speed",
                offset: 16,
                initial: 1200.0,
                min: 0.0,
                max: 1500.0,
                drift: 2.0,
                unit: "RPM",
            },
            S7TagDef {
                name: "Load_Step",
                offset: 20,
                initial: 5.0,
                min: 0.0,
                max: 8.0,
                drift: 0.0,
                unit: "",
            },
            S7TagDef {
                name: "Vibration",
                offset: 24,
                initial: 0.12,
                min: 0.0,
                max: 1.0,
                drift: 0.005,
                unit: "in/s",
            },
            S7TagDef {
                name: "Oil_Pressure",
                offset: 28,
                initial: 45.0,
                min: 0.0,
                max: 80.0,
                drift: 0.2,
                unit: "psig",
            },
            S7TagDef {
                name: "Oil_Temp",
                offset: 32,
                initial: 140.0,
                min: 80.0,
                max: 200.0,
                drift: 0.1,
                unit: "°F",
            },
            S7TagDef {
                name: "Coolant_Temp",
                offset: 36,
                initial: 165.0,
                min: 100.0,
                max: 220.0,
                drift: 0.2,
                unit: "°F",
            },
            S7TagDef {
                name: "Runtime_Hours",
                offset: 40,
                initial: 15280.0,
                min: 0.0,
                max: 999999.0,
                drift: 0.0,
                unit: "h",
            },
        ],
    }]
}

// ── ENIP tag profiles ──

#[derive(Debug, Clone)]
pub struct EnipTagProfile {
    pub program_name: &'static str,
    pub description: &'static str,
    pub tags: Vec<EnipTagDef>,
}

#[derive(Debug, Clone)]
pub struct EnipTagDef {
    pub name: &'static str,
    pub initial: f32,
    pub min: f32,
    pub max: f32,
    pub drift: f32,
    pub unit: &'static str,
    pub is_bool: bool,
}

/// CompactLogix L33ER: Food & Beverage filling line.
pub fn enip_profile_filling_line() -> EnipTagProfile {
    EnipTagProfile {
        program_name: "MainProgram",
        description: "Food & Beverage Filling Line (CompactLogix L33ER)",
        tags: vec![
            EnipTagDef {
                name: "Line_Speed",
                initial: 450.0,
                min: 0.0,
                max: 600.0,
                drift: 2.0,
                unit: "bpm",
                is_bool: false,
            },
            EnipTagDef {
                name: "Filler_Head_Temp",
                initial: 4.5,
                min: 1.0,
                max: 25.0,
                drift: 0.1,
                unit: "°C",
                is_bool: false,
            },
            EnipTagDef {
                name: "Filler_Tank_Level",
                initial: 78.0,
                min: 0.0,
                max: 100.0,
                drift: 0.2,
                unit: "%",
                is_bool: false,
            },
            EnipTagDef {
                name: "Filler_Pressure",
                initial: 2.8,
                min: 0.0,
                max: 5.0,
                drift: 0.02,
                unit: "bar",
                is_bool: false,
            },
            EnipTagDef {
                name: "CO2_Pressure",
                initial: 3.2,
                min: 0.0,
                max: 6.0,
                drift: 0.01,
                unit: "bar",
                is_bool: false,
            },
            EnipTagDef {
                name: "Conveyor_Speed",
                initial: 1.2,
                min: 0.0,
                max: 2.0,
                drift: 0.01,
                unit: "m/s",
                is_bool: false,
            },
            EnipTagDef {
                name: "Labeler_Speed",
                initial: 448.0,
                min: 0.0,
                max: 600.0,
                drift: 1.5,
                unit: "bpm",
                is_bool: false,
            },
            EnipTagDef {
                name: "Case_Packer_Count",
                initial: 2847.0,
                min: 0.0,
                max: 999999.0,
                drift: 0.0,
                unit: "",
                is_bool: false,
            },
            EnipTagDef {
                name: "Steam_Pressure",
                initial: 3.5,
                min: 0.0,
                max: 6.0,
                drift: 0.05,
                unit: "bar",
                is_bool: false,
            },
            EnipTagDef {
                name: "Line_Running",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
                is_bool: true,
            },
            EnipTagDef {
                name: "Estop_Active",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
                is_bool: true,
            },
            EnipTagDef {
                name: "Reject_Count",
                initial: 12.0,
                min: 0.0,
                max: 9999.0,
                drift: 0.0,
                unit: "",
                is_bool: false,
            },
        ],
    }
}

// ── BACnet object profiles ──

#[derive(Debug, Clone)]
pub struct BacnetProfile {
    pub device_name: &'static str,
    pub vendor_name: &'static str,
    pub model_name: &'static str,
    pub description: &'static str,
    pub objects: Vec<BacnetObjectDef>,
}

#[derive(Debug, Clone)]
pub struct BacnetObjectDef {
    pub obj_type: u16,
    pub instance: u32,
    pub name: &'static str,
    pub description: &'static str,
    pub initial: f64,
    pub min: f64,
    pub max: f64,
    pub drift: f64,
}

/// Trane Tracer SC+ AHU controller (BACnet/IP).
/// Models AHU-1 serving a data center hall: supply/return air, cooling coil,
/// economizer, VFD fan, filters.
pub fn bacnet_profile_ahu_datacenter() -> BacnetProfile {
    BacnetProfile {
        device_name: "AHU-1",
        vendor_name: "Trane",
        model_name: "Tracer SC+",
        description: "Air Handling Unit for Data Center Hall A",
        objects: vec![
            BacnetObjectDef {
                obj_type: 0,
                instance: 1,
                name: "SupplyAirTemp",
                description: "Supply air temperature (SA)",
                initial: 15.0,
                min: 10.0,
                max: 25.0,
                drift: 0.1,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 2,
                name: "ReturnAirTemp",
                description: "Return air temperature (RA)",
                initial: 28.0,
                min: 20.0,
                max: 40.0,
                drift: 0.1,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 3,
                name: "MixedAirTemp",
                description: "Mixed air temperature",
                initial: 22.0,
                min: 10.0,
                max: 35.0,
                drift: 0.1,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 4,
                name: "OutsideAirTemp",
                description: "Outside air temperature",
                initial: 32.0,
                min: -10.0,
                max: 45.0,
                drift: 0.05,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 5,
                name: "SupplyAirHumidity",
                description: "Supply air RH",
                initial: 45.0,
                min: 20.0,
                max: 80.0,
                drift: 0.5,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 6,
                name: "ReturnAirHumidity",
                description: "Return air RH",
                initial: 42.0,
                min: 20.0,
                max: 80.0,
                drift: 0.5,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 7,
                name: "SupplyAirPressure",
                description: "Supply duct static pressure",
                initial: 250.0,
                min: 0.0,
                max: 500.0,
                drift: 2.0,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 8,
                name: "CoolingCoilTemp",
                description: "Cooling coil leaving temp",
                initial: 7.2,
                min: 2.0,
                max: 15.0,
                drift: 0.05,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 9,
                name: "Fan_Speed",
                description: "VFD fan speed",
                initial: 78.0,
                min: 30.0,
                max: 100.0,
                drift: 0.2,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 10,
                name: "Economizer_Pos",
                description: "Economizer damper position",
                initial: 35.0,
                min: 0.0,
                max: 100.0,
                drift: 0.5,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 11,
                name: "Filter_DP",
                description: "Filter differential pressure",
                initial: 85.0,
                min: 0.0,
                max: 250.0,
                drift: 0.2,
            },
            // Setpoints
            BacnetObjectDef {
                obj_type: 1,
                instance: 1,
                name: "SupplyTemp_SP",
                description: "Supply air temp setpoint",
                initial: 15.0,
                min: 10.0,
                max: 22.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 1,
                instance: 2,
                name: "DuctPressure_SP",
                description: "Duct pressure setpoint",
                initial: 250.0,
                min: 100.0,
                max: 400.0,
                drift: 0.0,
            },
            // Binary
            BacnetObjectDef {
                obj_type: 3,
                instance: 1,
                name: "Fan_Running",
                description: "Supply fan status",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 3,
                instance: 2,
                name: "Filter_Alarm",
                description: "Filter replacement needed",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 3,
                instance: 3,
                name: "FreeCooling_Active",
                description: "Economizer mode active",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            // Multistate
            BacnetObjectDef {
                obj_type: 19,
                instance: 1,
                name: "AHU_Mode",
                description: "AHU mode (1=Off 2=Cool 3=Heat 4=Auto 5=Night)",
                initial: 4.0,
                min: 1.0,
                max: 5.0,
                drift: 0.0,
            },
        ],
    }
}

/// Johnson Controls AHU controller for water treatment building HVAC.
pub fn bacnet_profile_ahu_water_treatment() -> BacnetProfile {
    BacnetProfile {
        device_name: "HVAC-AHU1",
        vendor_name: "Johnson Controls",
        model_name: "Facility Explorer FEC",
        description: "Water Treatment Building HVAC AHU-1",
        objects: vec![
            BacnetObjectDef {
                obj_type: 0,
                instance: 1,
                name: "ZoneTemp_1",
                description: "Chemical room temperature",
                initial: 22.5,
                min: 15.0,
                max: 30.0,
                drift: 0.2,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 2,
                name: "ZoneTemp_2",
                description: "Control room temperature",
                initial: 23.0,
                min: 18.0,
                max: 28.0,
                drift: 0.1,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 3,
                name: "SupplyAirTemp",
                description: "Supply air temperature",
                initial: 18.0,
                min: 12.0,
                max: 28.0,
                drift: 0.1,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 4,
                name: "Humidity",
                description: "Average zone humidity",
                initial: 55.0,
                min: 30.0,
                max: 80.0,
                drift: 0.5,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 5,
                name: "CO2_Level",
                description: "Control room CO2",
                initial: 450.0,
                min: 300.0,
                max: 1500.0,
                drift: 5.0,
            },
            BacnetObjectDef {
                obj_type: 0,
                instance: 6,
                name: "ChwSupplyTemp",
                description: "Chilled water supply",
                initial: 7.0,
                min: 4.0,
                max: 12.0,
                drift: 0.05,
            },
            BacnetObjectDef {
                obj_type: 1,
                instance: 1,
                name: "ZoneSetpoint",
                description: "Zone temp setpoint",
                initial: 23.0,
                min: 18.0,
                max: 28.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 3,
                instance: 1,
                name: "FanStatus",
                description: "AHU fan running",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 3,
                instance: 2,
                name: "FilterAlarm",
                description: "Filter clogged",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
            },
            BacnetObjectDef {
                obj_type: 19,
                instance: 1,
                name: "SystemMode",
                description: "1=Off 2=Cool 3=Heat 4=Auto",
                initial: 4.0,
                min: 1.0,
                max: 4.0,
                drift: 0.0,
            },
        ],
    }
}

// ── MQTT topic profiles ──

#[derive(Debug, Clone)]
pub struct MqttProfile {
    pub gateway_name: &'static str,
    pub description: &'static str,
    pub topics: Vec<MqttTopicDef>,
}

#[derive(Debug, Clone)]
pub struct MqttTopicDef {
    pub topic: &'static str,
    pub description: &'static str,
    pub initial: f64,
    pub min: f64,
    pub max: f64,
    pub drift: f64,
    pub unit: &'static str,
}

/// Data center environmental sensors via MQTT gateway.
pub fn mqtt_profile_datacenter_env() -> MqttProfile {
    MqttProfile {
        gateway_name: "DC-ENV-GW-01",
        description: "Data center environmental sensors (Temp, RH, door contact, leak detect)",
        topics: vec![
            MqttTopicDef {
                topic: "dc/hall-a/row-1/front-temp",
                description: "Cold aisle temp Row 1",
                initial: 21.5,
                min: 15.0,
                max: 35.0,
                drift: 0.05,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "dc/hall-a/row-1/rear-temp",
                description: "Hot aisle temp Row 1",
                initial: 32.0,
                min: 20.0,
                max: 45.0,
                drift: 0.1,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "dc/hall-a/row-2/front-temp",
                description: "Cold aisle temp Row 2",
                initial: 21.8,
                min: 15.0,
                max: 35.0,
                drift: 0.05,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "dc/hall-a/row-2/rear-temp",
                description: "Hot aisle temp Row 2",
                initial: 31.5,
                min: 20.0,
                max: 45.0,
                drift: 0.1,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "dc/hall-a/row-1/humidity",
                description: "Row 1 humidity",
                initial: 45.0,
                min: 20.0,
                max: 80.0,
                drift: 0.3,
                unit: "%",
            },
            MqttTopicDef {
                topic: "dc/hall-a/row-2/humidity",
                description: "Row 2 humidity",
                initial: 44.0,
                min: 20.0,
                max: 80.0,
                drift: 0.3,
                unit: "%",
            },
            MqttTopicDef {
                topic: "dc/hall-a/door-main",
                description: "Main door contact (1=open)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "dc/hall-a/door-emergency",
                description: "Emergency exit (1=open)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "dc/hall-a/leak-row1",
                description: "Leak detect Row 1 (1=wet)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "dc/hall-a/leak-row2",
                description: "Leak detect Row 2 (1=wet)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "dc/hall-a/power- pdu1",
                description: "PDU-1 power consumption",
                initial: 8.65,
                min: 0.0,
                max: 15.0,
                drift: 0.05,
                unit: "kW",
            },
            MqttTopicDef {
                topic: "dc/hall-a/ups-load",
                description: "UPS load percentage",
                initial: 62.0,
                min: 0.0,
                max: 100.0,
                drift: 0.2,
                unit: "%",
            },
        ],
    }
}

/// Oil & gas well site satellite gateway telemetry.
pub fn mqtt_profile_oilgas_gateway() -> MqttProfile {
    MqttProfile {
        gateway_name: "WELL-SAT-GW-01",
        description: "Well pad satellite gateway telemetry",
        topics: vec![
            MqttTopicDef {
                topic: "wellpad/well-1/tubing_pressure",
                description: "Tubing head pressure",
                initial: 1850.0,
                min: 0.0,
                max: 3000.0,
                drift: 5.0,
                unit: "psig",
            },
            MqttTopicDef {
                topic: "wellpad/well-1/casing_pressure",
                description: "Casing head pressure",
                initial: 320.0,
                min: 0.0,
                max: 1000.0,
                drift: 2.0,
                unit: "psig",
            },
            MqttTopicDef {
                topic: "wellpad/well-1/oil_rate",
                description: "Oil production rate",
                initial: 125.0,
                min: 0.0,
                max: 500.0,
                drift: 1.0,
                unit: "bbl/d",
            },
            MqttTopicDef {
                topic: "wellpad/well-1/gas_rate",
                description: "Gas production rate",
                initial: 850.0,
                min: 0.0,
                max: 3000.0,
                drift: 10.0,
                unit: "MSCF/d",
            },
            MqttTopicDef {
                topic: "wellpad/well-1/water_cut",
                description: "Water cut percentage",
                initial: 35.0,
                min: 0.0,
                max: 95.0,
                drift: 0.5,
                unit: "%",
            },
            MqttTopicDef {
                topic: "wellpad/well-1/temperature",
                description: "Wellhead temperature",
                initial: 145.0,
                min: 60.0,
                max: 250.0,
                drift: 0.5,
                unit: "°F",
            },
            MqttTopicDef {
                topic: "wellpad/tank/battery_level",
                description: "Tank battery level",
                initial: 62.0,
                min: 0.0,
                max: 100.0,
                drift: 0.1,
                unit: "%",
            },
            MqttTopicDef {
                topic: "wellpad/satellite/signal",
                description: "Satellite signal strength",
                initial: 85.0,
                min: 0.0,
                max: 100.0,
                drift: 1.0,
                unit: "%",
            },
            MqttTopicDef {
                topic: "wellpad/satellite/latency",
                description: "Satellite link latency",
                initial: 620.0,
                min: 500.0,
                max: 1500.0,
                drift: 10.0,
                unit: "ms",
            },
            MqttTopicDef {
                topic: "wellpad/fence/breach",
                description: "Perimeter breach (1=alarm)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
        ],
    }
}

/// Solar farm weather station via MQTT.
pub fn mqtt_profile_solar_weather() -> MqttProfile {
    MqttProfile {
        gateway_name: "PV-MET-GW-01",
        description: "Solar farm weather station (irradiance, temp, wind, rain)",
        topics: vec![
            MqttTopicDef {
                topic: "solar/weather/irradiance-poa",
                description: "Plane-of-array irradiance",
                initial: 856.0,
                min: 0.0,
                max: 1200.0,
                drift: 15.0,
                unit: "W/m²",
            },
            MqttTopicDef {
                topic: "solar/weather/irradiance-ghi",
                description: "Global horizontal irradiance",
                initial: 790.0,
                min: 0.0,
                max: 1100.0,
                drift: 12.0,
                unit: "W/m²",
            },
            MqttTopicDef {
                topic: "solar/weather/ambient-temp",
                description: "Ambient temperature",
                initial: 28.5,
                min: -5.0,
                max: 45.0,
                drift: 0.1,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "solar/weather/module-temp",
                description: "Module back-surface temp",
                initial: 42.0,
                min: -5.0,
                max: 70.0,
                drift: 0.2,
                unit: "°C",
            },
            MqttTopicDef {
                topic: "solar/weather/wind-speed",
                description: "Wind speed at 10m",
                initial: 3.2,
                min: 0.0,
                max: 25.0,
                drift: 0.3,
                unit: "m/s",
            },
            MqttTopicDef {
                topic: "solar/weather/wind-dir",
                description: "Wind direction",
                initial: 185.0,
                min: 0.0,
                max: 360.0,
                drift: 2.0,
                unit: "°",
            },
            MqttTopicDef {
                topic: "solar/weather/humidity",
                description: "Relative humidity",
                initial: 62.0,
                min: 10.0,
                max: 100.0,
                drift: 0.5,
                unit: "%",
            },
            MqttTopicDef {
                topic: "solar/weather/rain-today",
                description: "Daily rainfall",
                initial: 0.0,
                min: 0.0,
                max: 50.0,
                drift: 0.0,
                unit: "mm",
            },
            MqttTopicDef {
                topic: "solar/inverter/1/status",
                description: "Inverter 1 status (1=running)",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "solar/inverter/1/ac-power",
                description: "Inverter 1 AC output",
                initial: 56.5,
                min: 0.0,
                max: 60.0,
                drift: 0.5,
                unit: "kW",
            },
        ],
    }
}

/// Water treatment plant SCADA telemetry via MQTT.
pub fn mqtt_profile_water_scada() -> MqttProfile {
    MqttProfile {
        gateway_name: "WTP-SCADA-GW-01",
        description: "Water treatment SCADA gateway (alarms, pump status, tank levels)",
        topics: vec![
            MqttTopicDef {
                topic: "wtp/alarm/turbidity-high",
                description: "Treated water turbidity alarm",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "wtp/alarm/chlorine-low",
                description: "Low chlorine residual alarm",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "wtp/pump/1/status",
                description: "Raw water pump 1 (1=running)",
                initial: 1.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "wtp/pump/2/status",
                description: "Raw water pump 2 (1=running)",
                initial: 0.0,
                min: 0.0,
                max: 1.0,
                drift: 0.0,
                unit: "",
            },
            MqttTopicDef {
                topic: "wtp/pump/1/flow",
                description: "Pump 1 discharge flow",
                initial: 125.0,
                min: 0.0,
                max: 250.0,
                drift: 1.0,
                unit: "m³/h",
            },
            MqttTopicDef {
                topic: "wtp/tank/clearwell/level",
                description: "Clearwell tank level",
                initial: 65.0,
                min: 0.0,
                max: 100.0,
                drift: 0.05,
                unit: "%",
            },
            MqttTopicDef {
                topic: "wtp/tank/treated/level",
                description: "Treated water reservoir",
                initial: 78.0,
                min: 0.0,
                max: 100.0,
                drift: 0.05,
                unit: "%",
            },
            MqttTopicDef {
                topic: "wtp/plant/daily-output",
                description: "Today's treated water volume",
                initial: 3200.0,
                min: 0.0,
                max: 12000.0,
                drift: 0.0,
                unit: "m³",
            },
        ],
    }
}
