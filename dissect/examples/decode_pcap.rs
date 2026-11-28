//! Print the Modbus/TCP frames of a capture as voltrus-dissect decodes them.
//! `cargo run -p voltrus-dissect --example decode_pcap -- file.pcap [port]`
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: decode_pcap <file.pcap> [port]");
    let port = args
        .next()
        .map_or(voltrus_dissect::modbus::PORT, |p| p.parse().expect("port"));
    let bytes = std::fs::read(&path).expect("read capture");
    let packets = voltrus_dissect::pcap::read(&bytes).expect("parse capture");
    for f in voltrus_dissect::net::modbus_frames(&packets, port) {
        let err = f
            .error
            .as_ref()
            .map(|e| format!("  [{}]", e.title))
            .unwrap_or_default();
        let lat = f
            .latency_us
            .map(|l| format!("  {l} us"))
            .unwrap_or_default();
        println!("{:>5} {:?} {}{lat}{err}", f.seq, f.dir, f.summary);
    }
}
