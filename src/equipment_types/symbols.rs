//! Symbol keys the server accepts for an equipment type's or instance's
//! `symbol`: every file of the shared library `symbols/<key>.json` (listed at
//! build time by build.rs) and the symbols the web client draws in code
//! (`client/src/scada2/hmi/symbols.tsx`, the shaded library).

include!(concat!(env!("OUT_DIR"), "/symbol_keys.rs"));

/// Keys drawn by client code rather than a data file.
pub const CODE_SYMBOLS: &[&str] = &[
    "pump",
    "fan",
    "motor",
    "compressor",
    "conveyor",
    "valve",
    "control_valve",
    "check_valve",
    "tank",
    "tank_horizontal",
    "silo",
    "mixer",
    "heat_exchanger",
    "filter",
    "clarifier",
    "dosing_pump",
    "gravity_filter",
    "flow_meter",
    "breaker",
    "transformer",
    "generator",
    "solar_panel",
    "battery",
    "inverter",
    "meter",
    "busbar",
    "instrument",
    "pipe",
    "pipe_vertical",
    "pipe_elbow",
    "pipe_tee",
];

pub fn is_known(key: &str) -> bool {
    SYMBOL_FILES.contains(&key) || CODE_SYMBOLS.contains(&key)
}

/// Every known key, sorted.
pub fn all() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = SYMBOL_FILES.iter().chain(CODE_SYMBOLS).copied().collect();
    v.sort_unstable();
    v.dedup();
    v
}
