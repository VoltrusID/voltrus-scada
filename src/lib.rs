//! Voltrus SCADA open core.
//!
//! Library and binary are one crate: `voltrus` (the binary) calls [`run`] with
//! the [`edition::Community`] extension. A downstream build can call [`run`]
//! with its own [`edition::Extension`].

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::new_without_default)]
#![allow(clippy::should_implement_trait)]

pub mod ai_copilot;
pub mod alarm_chatter;
pub mod alarm_delay;
pub mod alarm_eval;
pub mod analytics;
pub mod api;
pub mod asset;
pub mod backup;
#[cfg(feature = "bacnet")]
pub mod bacnet;
pub mod cache;
pub mod cli;
pub mod command_queue;
pub mod compliance;
pub mod compression;
pub mod config;
pub mod deadband;
pub mod device_templates;
#[cfg(any(
    feature = "modbus",
    feature = "opcua",
    feature = "bacnet",
    feature = "dnp3",
    feature = "ethernet-ip",
    feature = "s7"
))]
pub mod discovery;
#[cfg(feature = "dnp3")]
pub mod dnp3;
pub mod edition;
pub mod edr_compat;
#[cfg(feature = "ethernet-ip")]
pub mod ethernet_ip;
pub mod expression;
pub mod faceplate;
pub mod flow_engine;
pub mod hardware_cert;
pub mod health;
pub mod import_nodered;
pub mod integration;
pub mod license;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod mobile;
#[cfg(feature = "modbus")]
pub mod modbus;
pub mod models;
#[cfg(feature = "mqtt-broker")]
pub mod mqtt_broker;
pub mod network;
pub mod node_role;
pub mod notification;
pub mod oee;
#[cfg(feature = "opcua")]
pub mod opcua;
pub mod poller_manager;
pub mod project_export;
pub mod project_yaml;
pub mod quality;
pub mod recipe;
pub mod retention;
#[cfg(feature = "s7")]
pub mod s7;
pub mod script_engine;
pub mod security;
pub mod service;
pub mod sim_engine;
pub mod station;
pub mod storage;
pub mod system_docs;
pub mod timeutil;
pub mod vertical_templates;
pub mod voltrus_plc;
pub mod widget_sdk;
pub mod wizard;
pub mod ws_manager;
pub mod ws_types;

mod app;

pub use app::run;

// The simulator device profiles are shared with the `*-sim` binaries by path;
// the built-in device templates are generated from them.
#[path = "sim_profiles.rs"]
mod sim_profiles;

#[cfg(test)]
mod drivers_in_the_binary {
    /// Every protocol driver is a Cargo feature that the default build (the
    /// one the release workflow ships) turns on, so the release binary carries
    /// them all and never fetches a driver at run time (docs: README,
    /// "Drivers ship in the binary").
    const PROTOCOLS: [&str; 7] = [
        "modbus",
        "opcua",
        "s7",
        "ethernet-ip",
        "dnp3",
        "bacnet",
        "mqtt-broker",
    ];

    #[test]
    fn the_default_build_enables_every_protocol_driver() {
        let manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        let default = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("default = ["))
            .expect("a default feature list");
        for p in PROTOCOLS {
            assert!(
                default.contains(&format!("\"{p}\"")),
                "{p} is not a default feature: {default}"
            );
        }
    }

    #[test]
    fn the_release_workflow_builds_with_default_features() {
        let wf = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.github/workflows/release.yml"
        ))
        .unwrap();
        let build = wf
            .lines()
            .find(|l| l.contains("--release --target") && l.contains("--bin voltrus"))
            .expect("the release build line");
        assert!(
            !build.contains("--no-default-features") && !build.contains("--features"),
            "{build}"
        );
    }
}
