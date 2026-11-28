//! voltrus-dissect against tshark: every capture in `tests/fixtures/modbus`
//! is decoded and compared, field by field, with the golden file that
//! `tests/oracle/regen.sh` made from `tshark -T json`. No tshark is needed to
//! run this test.
//!
//! Set `VOLTRUS_ORACLE_DIR` to a directory of other captures with goldens
//! (made with `regen.sh --dir`) to check those too, e.g. downloaded samples
//! that are not committed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use voltrus_dissect::{DecodedFrame, Dir, ErrorKind, modbus, net, pcap};

/// Fields compared for every PDU.
const COMMON: &[&str] = &[
    "mbtcp.trans_id",
    "mbtcp.prot_id",
    "mbtcp.len",
    "mbtcp.unit_id",
    "modbus.func_code",
    "modbus.exception_code",
    "modbus.request_frame",
    "modbus.response_time",
];

/// Further fields compared, for the function codes voltrus-dissect decodes in
/// full. For the others only [`COMMON`] is compared; tshark's extra fields
/// for them (diagnostics, event log, ...) are counted as not compared.
fn supported(function: u8, mei: Option<&str>) -> &'static [&'static str] {
    match function {
        1 | 2 => &[
            "modbus.reference_num",
            "modbus.bit_cnt",
            "modbus.byte_cnt",
            "modbus.bitnum",
            "modbus.bitval",
        ],
        3 | 4 => &[
            "modbus.reference_num",
            "modbus.word_cnt",
            "modbus.byte_cnt",
            "modbus.regnum16",
            "modbus.regval_uint16",
        ],
        5 => &["modbus.reference_num", "modbus.data", "modbus.padding"],
        6 => &[
            "modbus.reference_num",
            "modbus.regnum16",
            "modbus.regval_uint16",
            "modbus.data",
        ],
        15 => &[
            "modbus.reference_num",
            "modbus.bit_cnt",
            "modbus.byte_cnt",
            "modbus.bitnum",
            "modbus.bitval",
        ],
        16 => &[
            "modbus.reference_num",
            "modbus.word_cnt",
            "modbus.byte_cnt",
            "modbus.regnum16",
            "modbus.regval_uint16",
        ],
        22 => &["modbus.reference_num", "modbus.and_mask", "modbus.or_mask"],
        23 => &[
            "modbus.read_reference_num",
            "modbus.read_word_cnt",
            "modbus.write_reference_num",
            "modbus.write_word_cnt",
            "modbus.byte_cnt",
            "modbus.data",
        ],
        43 if mei == Some("14") => &[
            "modbus.mei",
            "modbus.read_device_id",
            "modbus.object_id",
            "modbus.conformity_level",
            "modbus.more_follows",
            "modbus.next_object_id",
            "modbus.num_objects",
            "modbus.objects_len",
            "modbus.object_str_value",
            "modbus.object_value",
        ],
        43 => &["modbus.mei"],
        _ => &[],
    }
}

type Pdu = Vec<(String, String)>;

fn golden_frames(path: &Path) -> BTreeMap<u64, Vec<Pdu>> {
    let text = std::fs::read_to_string(path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut out = BTreeMap::new();
    for f in v["frames"].as_array().unwrap() {
        let pdus = f["pdus"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                p.as_array()
                    .unwrap()
                    .iter()
                    .map(|kv| {
                        (
                            kv[0].as_str().unwrap().to_string(),
                            kv[1].as_str().unwrap().to_string(),
                        )
                    })
                    .collect()
            })
            .collect();
        out.insert(f["frame"].as_u64().unwrap(), pdus);
    }
    out
}

/// Our decode as (field id, tshark-style value) pairs, plus the pairing
/// fields tshark shows on a response. tshark 4.4 leaves them off exception
/// responses (we pair those too), so they are not added there.
fn ours(frame: &DecodedFrame) -> Pdu {
    let mut out: Pdu = Vec::new();
    let exception = frame.field("modbus.exception_code").is_some();
    for f in frame.walk().filter(|f| !f.id.is_empty()) {
        out.push((f.id.to_string(), f.show()));
        if f.id == "modbus.func_code"
            && frame.dir == Dir::Rx
            && !exception
            && let (Some(req), Some(lat)) = (frame.pair_id, frame.latency_us)
        {
            out.push(("modbus.request_frame".into(), req.to_string()));
            out.push((
                "modbus.response_time".into(),
                format!("{}.{:06}000", lat / 1_000_000, lat % 1_000_000),
            ));
        }
    }
    out
}

fn values<'a>(pdu: &'a Pdu, id: &str) -> Vec<&'a str> {
    pdu.iter()
        .filter(|(k, _)| k == id)
        .map(|(_, v)| v.as_str())
        .collect()
}

#[derive(Default, Debug)]
struct Tally {
    frames: usize,
    pdus: usize,
    fields: usize,
    not_compared: BTreeMap<String, usize>,
    /// Bytes on the Modbus port that tshark does not take for Modbus and we
    /// report as a malformed frame.
    malformed_agreed: usize,
    mismatches: Vec<String>,
}

fn check_capture(pcap_path: &Path, golden_path: &Path, t: &mut Tally) {
    let bytes = std::fs::read(pcap_path).unwrap();
    let packets = pcap::read(&bytes).unwrap();
    let frames = net::modbus_frames(&packets, modbus::PORT);
    let mut by_packet: BTreeMap<u64, Vec<&DecodedFrame>> = BTreeMap::new();
    for f in &frames {
        by_packet.entry(f.seq).or_default().push(f);
    }
    let golden = golden_frames(golden_path);
    let name = pcap_path.file_name().unwrap().to_string_lossy();
    let numbers: BTreeSet<u64> = golden.keys().chain(by_packet.keys()).copied().collect();
    for n in numbers {
        let theirs = golden.get(&n).cloned().unwrap_or_default();
        let mine_frames = by_packet.get(&n).cloned().unwrap_or_default();
        t.frames += 1;
        if theirs.is_empty()
            && !mine_frames.is_empty()
            && mine_frames.iter().all(|f| {
                f.error
                    .as_ref()
                    .is_some_and(|e| e.kind == ErrorKind::Malformed)
            })
        {
            t.malformed_agreed += mine_frames.len();
            continue;
        }
        let mine: Vec<Pdu> = mine_frames.iter().map(|f| ours(f)).collect();
        if theirs.len() != mine.len() {
            t.mismatches.push(format!(
                "{name} frame {n}: tshark has {} PDUs, voltrus-dissect {}",
                theirs.len(),
                mine.len()
            ));
            continue;
        }
        for (i, (th, my)) in theirs.iter().zip(&mine).enumerate() {
            t.pdus += 1;
            let fc: u8 = values(th, "modbus.func_code")
                .first()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let mei = values(th, "modbus.mei").first().copied();
            let exception = !values(th, "modbus.exception_code").is_empty();
            let mut ids: Vec<&str> = COMMON.to_vec();
            if !exception {
                ids.extend(supported(fc, mei));
            }
            for (k, _) in th {
                if !ids.contains(&k.as_str()) {
                    *t.not_compared.entry(k.clone()).or_default() += 1;
                }
            }
            for id in ids {
                let (a, b) = (values(th, id), values(my, id));
                t.fields += a.len().max(b.len());
                if a != b {
                    t.mismatches.push(format!(
                        "{name} frame {n} pdu {i} (fc {fc}) {id}: tshark {a:?}, voltrus-dissect {b:?}"
                    ));
                }
            }
            // A field we emit that is outside the compared set would go unchecked.
            for (k, _) in my {
                if !COMMON.contains(&k.as_str()) && !supported(fc, mei).contains(&k.as_str()) {
                    t.mismatches.push(format!(
                        "{name} frame {n} pdu {i}: {k} emitted but not compared"
                    ));
                }
            }
        }
    }
}

fn captures(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "pcap" || ext == "pcapng" {
            let golden = p.with_extension("golden.json");
            assert!(
                golden.exists(),
                "{} has no golden; run tests/oracle/regen.sh",
                p.display()
            );
            out.push((p, golden));
        }
    }
    out.sort();
    out
}

fn run(dir: &Path) -> Tally {
    let mut t = Tally::default();
    for (cap, golden) in captures(dir) {
        check_capture(&cap, &golden, &mut t);
    }
    eprintln!(
        "{}: {} frames, {} PDUs, {} fields compared, {} mismatches, {} non-Modbus payloads flagged malformed; not compared: {:?}",
        dir.display(),
        t.frames,
        t.pdus,
        t.fields,
        t.mismatches.len(),
        t.malformed_agreed,
        t.not_compared
    );
    t
}

#[test]
fn modbus_matches_tshark() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modbus");
    let t = run(&dir);
    assert!(t.pdus >= 50, "too few PDUs compared: {t:?}");
    assert!(t.mismatches.is_empty(), "{}", t.mismatches.join("\n"));
}

#[test]
fn extra_captures_match_tshark() {
    let Ok(dir) = std::env::var("VOLTRUS_ORACLE_DIR") else {
        return;
    };
    let t = run(Path::new(&dir));
    assert!(t.mismatches.is_empty(), "{}", t.mismatches.join("\n"));
}

#[test]
fn generated_capture_covers_every_decoded_function_and_the_errors() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/modbus");
    let bytes = std::fs::read(dir.join("gen-modbus.pcap")).unwrap();
    let packets = pcap::read(&bytes).unwrap();
    let frames = net::modbus_frames(&packets, modbus::PORT);
    let fcs: BTreeSet<u16> = frames.iter().filter_map(|f| f.function).collect();
    assert_eq!(
        fcs,
        BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 15, 16, 22, 23, 43])
    );
    let codes: BTreeSet<u16> = frames
        .iter()
        .filter_map(|f| f.error.as_ref()?.code)
        .collect();
    assert_eq!(codes, BTreeSet::from([1, 2, 3, 4, 6, 11]));
    // The request to the silent unit stays unpaired; every other request is answered.
    let unpaired: Vec<_> = frames
        .iter()
        .filter(|f| f.dir == Dir::Tx && !frames.iter().any(|r| r.pair_id == Some(f.seq)))
        .map(|f| f.raw[6])
        .collect();
    assert_eq!(unpaired, [0x63]);
    // Pipelined requests in one segment, and a request split over two.
    assert!(
        frames
            .windows(2)
            .any(|w| w[0].seq == w[1].seq && w[0].dir == Dir::Tx)
    );
}
