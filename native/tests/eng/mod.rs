//! The Data Manager's Engineering sections against the live plant, one
//! module per section, each leaving the plant as it found it. Called in
//! order from `live_engineering_tools` in `e2e_live.rs`.

pub(super) mod edr;
pub(super) mod flows;
pub(super) mod imports;
pub(super) mod mqtt;
pub(super) mod scripts;
pub(super) mod snapshots;
pub(super) mod system;
