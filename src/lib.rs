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
pub mod alarm_delay;
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
pub mod config_export;
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
pub mod flow_engine;
pub mod hardware_cert;
pub mod health;
pub mod import_nodered;
pub mod integration;
pub mod license;
#[cfg(feature = "modbus")]
pub mod modbus;
pub mod models;
#[cfg(feature = "mqtt-broker")]
pub mod mqtt_broker;
pub mod network;
pub mod notification;
pub mod oee;
#[cfg(feature = "opcua")]
pub mod opcua;
pub mod poller_manager;
pub mod project_export;
pub mod recipe;
pub mod retention;
#[cfg(feature = "s7")]
pub mod s7;
pub mod script_engine;
pub mod security;
pub mod sim_engine;
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
