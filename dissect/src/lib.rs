//! Protocol traffic decoding for Voltrus SCADA.
//!
//! Every decoder turns one protocol data unit into a [`DecodedFrame`]: a
//! summary line, a tree of [`Field`]s with byte offsets (for highlighting in a
//! hex pane), request/response pairing, latency and, for errors, an
//! explanation in plain words. Field ids are Wireshark display-filter names
//! (`mbtcp.trans_id`, `modbus.func_code`, ...) so a decode can be checked
//! field by field against `tshark -T json`, which the tests do with committed
//! golden files. The decoders are written from the protocol specifications;
//! no Wireshark code is used.
//!
//! The crate does no I/O and has no runtime: the server feeds it bytes from
//! its driver taps, the tests feed it packets from [`pcap`] files, and it
//! builds for `wasm32-unknown-unknown`.

#![forbid(unsafe_code)]

pub mod modbus;
mod model;
pub mod net;
pub mod pcap;

pub use model::*;
