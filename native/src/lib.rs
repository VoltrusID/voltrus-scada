//! voltrus-scada-native: the network side of the GPUI operator console, with
//! no UI, so it can be tested on its own.
//!
//! - [`api`]: REST client (sign-in cookies, generic JSON helpers, tags,
//!   alarms, history).
//! - [`ws_types`]: the WebSocket wire format, mirrored from the server.
//! - [`runtime`]: the actor that owns the session and streams events,
//!   including generic REST calls tied to the session.
//! - [`stress`]: a deterministic synthetic plant for stress tests.
//! - [`perf`]: frame statistics for the HUD and the stress report.
//! - [`screens`]: screens, their card layouts and the 24-column card grid
//!   (compaction, drag, resize, auto-arrange), shared with the mimic viewer.
//! - [`signature`]: electronic signatures an Enterprise server may ask for.
//! - [`tag_api`]: tag writes and write policies, the tag tree, naming
//!   convention, annotations and the tag detail's history queries.
//! - [`device_file`]: Modbus Poll and BACnet Explorer device files, parsed
//!   for import (a port of the web client's `deviceFileImport.ts`).
//! - [`tls`]: the certificates trusted for `https://` and `wss://` (the
//!   system's store, webpki roots, an optional `ca_file`).
//! - [`zone`]: the operator's time zone with its daylight-saving rules, for
//!   showing and reading local times.
//! - [`mimic`]: HMI mimic screens: the server's SCADA rows, the editable
//!   screen model, the symbol library's geometry, live states, connection
//!   routes and the faceplate's command rules.
//! - [`station`]: the local station (the server installed on this computer
//!   as a service): its directory, its credential and its setup commands.
pub mod api;
pub mod device_file;
pub mod mimic;
pub mod perf;
pub mod runtime;
pub mod screens;
pub mod signature;
pub mod station;
pub mod stress;
pub mod tag_api;
pub mod tls;
pub mod ws_types;
pub mod zone;
