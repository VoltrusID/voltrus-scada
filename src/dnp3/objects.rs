//! DNP3 object catalogue and response object parsing.
//!
//! Object headers are walked generically from the qualifier (prefix code and
//! range specifier) and a per group/variation size table, so a response can
//! carry objects this master does not turn into points without derailing the
//! parse of the headers that follow.

use anyhow::Result;

/// Point flag bits shared by the input and output types.
pub mod flags {
    pub const ONLINE: u8 = 0x01;
    pub const RESTART: u8 = 0x02;
    pub const COMM_LOST: u8 = 0x04;
    pub const REMOTE_FORCED: u8 = 0x08;
    pub const LOCAL_FORCED: u8 = 0x10;
    /// CHATTER_FILTER on binaries, OVER_RANGE on analogs, ROLLOVER on counters.
    pub const BIT5: u8 = 0x20;
    /// REFERENCE_ERR on analogs, DISCONTINUITY on counters.
    pub const BIT6: u8 = 0x40;
    /// The state of a binary point.
    pub const STATE: u8 = 0x80;
}

/// Whether a value carrying these DNP3 flags can be trusted as good.
///
/// Forced values (bits 3, 4) are still real readings, so they stay good.
/// RESTART and COMM_LOST mean the value is stale; bits 5 and 6 mean the
/// value is out of range, filtered or discontinuous depending on the type,
/// none of which should count as good. Bit 7 is a binary state, not quality.
pub fn dnp3_flags_to_good(point_flags: u8) -> bool {
    point_flags & flags::ONLINE != 0
        && point_flags & (flags::RESTART | flags::COMM_LOST | flags::BIT5 | flags::BIT6) == 0
}

/// A point value parsed from a response.
#[derive(Debug, Clone, PartialEq)]
pub struct Dnp3PointValue {
    pub group: u8,
    pub variation: u8,
    pub index: u16,
    pub value: f64,
    /// DNP3 quality flags. Variations without flags report ONLINE, which is
    /// what the spec says their absence implies.
    pub flags: u8,
    /// Event timestamp, ms since 1970 UTC, when the variation carries one.
    pub time_ms: Option<u64>,
}

impl Dnp3PointValue {
    pub fn is_good(&self) -> bool {
        dnp3_flags_to_good(self.flags)
    }

    pub fn is_event(&self) -> bool {
        static_group(self.group) != self.group
    }
}

/// The static group an event or output group reports on, so a tag configured
/// as g30 also picks up g32 events for the same index.
pub fn static_group(group: u8) -> u8 {
    match group {
        2 => 1,
        4 => 3,
        11 => 10,
        22 => 20,
        23 => 21,
        32 => 30,
        42 => 40,
        g => g,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Val {
    /// One bit per point, packed LSB first.
    Packed1,
    /// Two bits per point (double-bit binary).
    Packed2,
    /// Binary state in bit 7 of the flags octet.
    State,
    /// Double-bit state in bits 7..6 of the flags octet.
    DoubleState,
    I16,
    I32,
    U16,
    U32,
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Time {
    None,
    /// 48-bit absolute ms since epoch.
    Abs,
    /// 16-bit ms relative to the preceding g51 common time of occurrence.
    Rel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    flags: bool,
    val: Val,
    time: Time,
}

const fn l(flags: bool, val: Val, time: Time) -> Option<Layout> {
    Some(Layout { flags, val, time })
}

/// Layout of point-carrying objects.
fn point_layout(group: u8, variation: u8) -> Option<Layout> {
    use Time::*;
    use Val::*;
    match (group, variation) {
        (1, 1) | (10, 1) => l(false, Packed1, None),
        (1, 2) | (10, 2) | (2, 1) | (11, 1) => l(true, State, None),
        (2, 2) | (11, 2) => l(true, State, Abs),
        (2, 3) => l(true, State, Rel),
        (3, 1) => l(false, Packed2, None),
        (3, 2) | (4, 1) => l(true, DoubleState, None),
        (4, 2) => l(true, DoubleState, Abs),
        (4, 3) => l(true, DoubleState, Rel),
        (20, 1) | (21, 1) | (22, 1) | (23, 1) => l(true, U32, None),
        (20, 2) | (21, 2) | (22, 2) | (23, 2) => l(true, U16, None),
        (20, 5) | (21, 9) => l(false, U32, None),
        (20, 6) | (21, 10) => l(false, U16, None),
        (21, 5) | (22, 5) | (23, 5) => l(true, U32, Abs),
        (21, 6) | (22, 6) | (23, 6) => l(true, U16, Abs),
        (30, 1) | (32, 1) | (40, 1) | (42, 1) => l(true, I32, None),
        (30, 2) | (32, 2) | (40, 2) | (42, 2) => l(true, I16, None),
        (30, 3) => l(false, I32, None),
        (30, 4) => l(false, I16, None),
        (30, 5) | (32, 5) | (40, 3) | (42, 5) => l(true, F32, None),
        (30, 6) | (32, 6) | (40, 4) | (42, 6) => l(true, F64, None),
        (32, 3) | (42, 3) => l(true, I32, Abs),
        (32, 4) | (42, 4) => l(true, I16, Abs),
        (32, 7) | (42, 7) => l(true, F32, Abs),
        (32, 8) | (42, 8) => l(true, F64, Abs),
        _ => Option::None,
    }
}

fn layout_size(lay: Layout) -> usize {
    let val = match lay.val {
        Val::Packed1 | Val::Packed2 | Val::State | Val::DoubleState => 0,
        Val::I16 | Val::U16 => 2,
        Val::I32 | Val::U32 | Val::F32 => 4,
        Val::F64 => 8,
    };
    let time = match lay.time {
        Time::None => 0,
        Time::Abs => 6,
        Time::Rel => 2,
    };
    usize::from(lay.flags) + val + time
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    Fixed(usize),
    /// Bits per point, packed into whole octets across the whole range.
    Bits(usize),
}

fn object_size(group: u8, variation: u8) -> Option<Size> {
    if let Some(lay) = point_layout(group, variation) {
        return Some(match lay.val {
            Val::Packed1 => Size::Bits(1),
            Val::Packed2 => Size::Bits(2),
            _ => Size::Fixed(layout_size(lay)),
        });
    }
    Some(match (group, variation) {
        (12, 1) => Size::Fixed(11),
        (41, 1) => Size::Fixed(5),
        (41, 2) => Size::Fixed(3),
        (41, 3) => Size::Fixed(5),
        (41, 4) => Size::Fixed(9),
        (50, 1) | (50, 3) | (51, 1) | (51, 2) => Size::Fixed(6),
        (52, 1) | (52, 2) => Size::Fixed(2),
        (80, 1) => Size::Bits(1),
        _ => return None,
    })
}

/// One object of a header, with its index (from the range or prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectItem {
    pub index: u32,
    /// For bit-packed objects, a single octet holding the point's bits.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectBlock {
    pub group: u8,
    pub variation: u8,
    pub qualifier: u8,
    pub items: Vec<ObjectItem>,
}

fn read_uint(buf: &[u8], pos: &mut usize, width: usize) -> Result<u32> {
    let Some(b) = buf.get(*pos..*pos + width) else {
        anyhow::bail!("object header truncated at offset {}", *pos);
    };
    *pos += width;
    Ok(match width {
        1 => b[0] as u32,
        2 => u16::from_le_bytes([b[0], b[1]]) as u32,
        _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    })
}

/// Walk the object headers of a response or of a request that carries
/// objects (WRITE, SELECT, OPERATE).
pub fn parse_object_headers(buf: &[u8]) -> Result<Vec<ObjectBlock>> {
    let mut blocks = Vec::new();
    walk_headers(buf, true, &mut blocks)?;
    Ok(blocks)
}

/// Headers of a READ request, which name ranges but carry no object data.
pub fn parse_read_headers(buf: &[u8]) -> Result<Vec<ObjectBlock>> {
    let mut blocks = Vec::new();
    walk_headers(buf, false, &mut blocks)?;
    Ok(blocks)
}

/// Like `parse_object_headers`, but keeps the blocks parsed before an error.
pub fn parse_object_headers_lenient(buf: &[u8]) -> (Vec<ObjectBlock>, Option<anyhow::Error>) {
    let mut blocks = Vec::new();
    let err = walk_headers(buf, true, &mut blocks).err();
    (blocks, err)
}

fn walk_headers(buf: &[u8], with_data: bool, blocks: &mut Vec<ObjectBlock>) -> Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        if buf.len() - pos < 3 {
            anyhow::bail!("object header truncated at offset {pos}");
        }
        let (group, variation, q) = (buf[pos], buf[pos + 1], buf[pos + 2]);
        pos += 3;
        let prefix_width = match (q >> 4) & 0x07 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 4,
            p => anyhow::bail!("g{group}v{variation}: prefix code {p} is not supported"),
        };
        // (first index if a range, count)
        let (start, count) = match q & 0x0F {
            r @ 0..=2 => {
                let w = [1, 2, 4][r as usize];
                let start = read_uint(buf, &mut pos, w)?;
                let stop = read_uint(buf, &mut pos, w)?;
                if stop < start {
                    anyhow::bail!("g{group}v{variation}: range {start}..{stop} is reversed");
                }
                (Some(start), (stop - start) as usize + 1)
            }
            6 => (None, 0),
            r @ (7..=9) => {
                let w = [1, 2, 4][(r - 7) as usize];
                (None, read_uint(buf, &mut pos, w)? as usize)
            }
            r => anyhow::bail!("g{group}v{variation}: range specifier {r} is not supported"),
        };
        if !with_data || q & 0x0F == 6 || group == 60 {
            // "All objects" and class headers carry no object data, nor do
            // READ requests.
            let first = start.unwrap_or(0);
            let items = if with_data || prefix_width != 0 {
                Vec::new()
            } else {
                (0..count as u32)
                    .map(|i| ObjectItem {
                        index: first + i,
                        data: Vec::new(),
                    })
                    .collect()
            };
            blocks.push(ObjectBlock {
                group,
                variation,
                qualifier: q,
                items,
            });
            continue;
        }
        let Some(size) = object_size(group, variation) else {
            anyhow::bail!("g{group}v{variation} is not in the object catalogue");
        };
        let mut items = Vec::with_capacity(count.min(4096));
        match size {
            Size::Bits(bits) => {
                if prefix_width != 0 {
                    anyhow::bail!("g{group}v{variation}: packed objects cannot be prefixed");
                }
                let nbytes = (count * bits).div_ceil(8);
                let Some(data) = buf.get(pos..pos + nbytes) else {
                    anyhow::bail!("g{group}v{variation}: packed data truncated");
                };
                let first = start.unwrap_or(0);
                for i in 0..count {
                    let bit = i * bits;
                    let v = (data[bit / 8] >> (bit % 8)) & ((1u8 << bits) - 1);
                    items.push(ObjectItem {
                        index: first + i as u32,
                        data: vec![v],
                    });
                }
                pos += nbytes;
            }
            Size::Fixed(n) => {
                for i in 0..count {
                    let index = if prefix_width > 0 {
                        read_uint(buf, &mut pos, prefix_width)?
                    } else {
                        start.unwrap_or(0) + i as u32
                    };
                    let Some(data) = buf.get(pos..pos + n) else {
                        anyhow::bail!("g{group}v{variation}: object {i} truncated");
                    };
                    items.push(ObjectItem {
                        index,
                        data: data.to_vec(),
                    });
                    pos += n;
                }
            }
        }
        blocks.push(ObjectBlock {
            group,
            variation,
            qualifier: q,
            items,
        });
    }
    Ok(())
}

fn le_u48(b: &[u8]) -> u64 {
    let mut x = [0u8; 8];
    x[..6].copy_from_slice(&b[..6]);
    u64::from_le_bytes(x)
}

/// Decode one object of a point-carrying variation. `cto` is the last g51
/// common time of occurrence, needed by relative-time event variations.
fn decode_point(
    group: u8,
    variation: u8,
    item: &ObjectItem,
    cto: Option<u64>,
) -> Option<Dnp3PointValue> {
    let lay = point_layout(group, variation)?;
    let d = &item.data;
    let mut p = 0;
    let mut flags = flags::ONLINE;
    if lay.flags {
        flags = d[0];
        p = 1;
    }
    let value = match lay.val {
        Val::Packed1 | Val::Packed2 => d[0] as f64,
        Val::State => f64::from(flags >> 7),
        Val::DoubleState => f64::from(flags >> 6),
        Val::I16 => {
            p += 2;
            i16::from_le_bytes([d[p - 2], d[p - 1]]) as f64
        }
        Val::U16 => {
            p += 2;
            u16::from_le_bytes([d[p - 2], d[p - 1]]) as f64
        }
        Val::I32 => {
            p += 4;
            i32::from_le_bytes(d[p - 4..p].try_into().ok()?) as f64
        }
        Val::U32 => {
            p += 4;
            u32::from_le_bytes(d[p - 4..p].try_into().ok()?) as f64
        }
        Val::F32 => {
            p += 4;
            f32::from_le_bytes(d[p - 4..p].try_into().ok()?) as f64
        }
        Val::F64 => {
            p += 8;
            f64::from_le_bytes(d[p - 8..p].try_into().ok()?)
        }
    };
    let time_ms = match lay.time {
        Time::None => None,
        Time::Abs => Some(le_u48(&d[p..])),
        Time::Rel => cto.map(|base| base + u16::from_le_bytes([d[p], d[p + 1]]) as u64),
    };
    Some(Dnp3PointValue {
        group,
        variation,
        index: u16::try_from(item.index).ok()?,
        value,
        flags,
        time_ms,
    })
}

/// Turn parsed object blocks into point values, in the order received (so a
/// later event for the same point overrides an earlier one).
pub fn points_from_blocks(blocks: &[ObjectBlock]) -> Vec<Dnp3PointValue> {
    let mut out = Vec::new();
    let mut cto = None;
    for b in blocks {
        if b.group == 51 {
            cto = b.items.first().map(|i| le_u48(&i.data));
            continue;
        }
        for item in &b.items {
            if let Some(p) = decode_point(b.group, b.variation, item, cto) {
                out.push(p);
            }
        }
    }
    out
}

/// Parse the object part of a response straight into point values.
pub fn parse_points(objects: &[u8]) -> Result<Vec<Dnp3PointValue>> {
    Ok(points_from_blocks(&parse_object_headers(objects)?))
}

/// Human-readable name for a group/variation pair.
pub fn group_variation_name(group: u8, variation: u8) -> &'static str {
    match (group, variation) {
        (1, 1) => "Binary Input (packed)",
        (1, 2) => "Binary Input (with flags)",
        (2, 1) => "Binary Input Event (no time)",
        (2, 2) => "Binary Input Event (absolute time)",
        (2, 3) => "Binary Input Event (relative time)",
        (10, 1) => "Binary Output Status (packed)",
        (10, 2) => "Binary Output Status (with flags)",
        (11, 1) => "Binary Output Event (no time)",
        (11, 2) => "Binary Output Event (with time)",
        (12, 1) => "Control Relay Output Block",
        (20, 1) => "Counter (32-bit with flags)",
        (20, 2) => "Counter (16-bit with flags)",
        (20, 5) => "Counter (32-bit no flags)",
        (20, 6) => "Counter (16-bit no flags)",
        (22, 1) => "Counter Event (32-bit)",
        (22, 2) => "Counter Event (16-bit)",
        (22, 5) => "Counter Event (32-bit with time)",
        (22, 6) => "Counter Event (16-bit with time)",
        (30, 1) => "Analog Input (32-bit with flags)",
        (30, 2) => "Analog Input (16-bit with flags)",
        (30, 3) => "Analog Input (32-bit no flags)",
        (30, 4) => "Analog Input (16-bit no flags)",
        (30, 5) => "Analog Input (float32 with flags)",
        (30, 6) => "Analog Input (float64 with flags)",
        (32, 1) => "Analog Input Event (32-bit)",
        (32, 2) => "Analog Input Event (16-bit)",
        (32, 3) => "Analog Input Event (32-bit with time)",
        (32, 4) => "Analog Input Event (16-bit with time)",
        (32, 5) => "Analog Input Event (float32)",
        (32, 6) => "Analog Input Event (float64)",
        (32, 7) => "Analog Input Event (float32 with time)",
        (32, 8) => "Analog Input Event (float64 with time)",
        (40, 1) => "Analog Output Status (32-bit with flags)",
        (40, 2) => "Analog Output Status (16-bit with flags)",
        (40, 3) => "Analog Output Status (float32 with flags)",
        (40, 4) => "Analog Output Status (float64 with flags)",
        (41, 1) => "Analog Output Command (32-bit)",
        (41, 2) => "Analog Output Command (16-bit)",
        (41, 3) => "Analog Output Command (float32)",
        (41, 4) => "Analog Output Command (float64)",
        (50, 1) => "Time and Date (absolute)",
        (50, 3) => "Time and Date (last recorded)",
        (51, 1) => "Common Time of Occurrence (synchronized)",
        (51, 2) => "Common Time of Occurrence (unsynchronized)",
        (52, 1) => "Time Delay (coarse)",
        (52, 2) => "Time Delay (fine)",
        (60, 1) => "Class 0 data",
        (60, 2) => "Class 1 data",
        (60, 3) => "Class 2 data",
        (60, 4) => "Class 3 data",
        (80, 1) => "Internal Indications",
        _ => "Unknown",
    }
}

/// Group/variation pairs offered when configuring points. Writes go to the
/// output status points (g10, g40); the master sends g12/g41 commands.
pub fn common_group_variations() -> Vec<(u8, u8, &'static str)> {
    vec![
        (1, 2, "Binary Input"),
        (10, 2, "Binary Output Status"),
        (20, 1, "Counter"),
        (30, 1, "Analog Input (32-bit)"),
        (30, 5, "Analog Input (float32)"),
        (40, 1, "Analog Output Status (32-bit)"),
        (40, 3, "Analog Output Status (float32)"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    fn one(objects: &str) -> Dnp3PointValue {
        let pts = parse_points(&hex(objects)).unwrap();
        assert_eq!(pts.len(), 1, "{pts:?}");
        pts[0].clone()
    }

    #[test]
    fn g1v1_packed_range() {
        // 10 points from index 0: bits 0, 2 and 9 set.
        let pts = parse_points(&hex("01 01 00 00 09 05 02")).unwrap();
        assert_eq!(pts.len(), 10);
        assert_eq!(pts[0].value, 1.0);
        assert_eq!(pts[1].value, 0.0);
        assert_eq!(pts[2].value, 1.0);
        assert_eq!(pts[9].value, 1.0);
        assert_eq!(pts[9].index, 9);
        assert_eq!(pts[0].flags, flags::ONLINE);
    }

    #[test]
    fn g1v2_state_is_bit_7() {
        let pts = parse_points(&hex("01 02 00 03 04 81 01")).unwrap();
        assert_eq!((pts[0].index, pts[0].value), (3, 1.0));
        assert_eq!((pts[1].index, pts[1].value), (4, 0.0));
        assert!(pts[0].is_good());
    }

    #[test]
    fn g2_events() {
        let p = one("02 01 17 01 05 81");
        assert_eq!((p.index, p.value, p.time_ms), (5, 1.0, None));
        let p = one("02 02 28 01 00 0A 00 01 E8 03 00 00 00 00");
        assert_eq!((p.index, p.value, p.time_ms), (10, 0.0, Some(1000)));
        // g51v1 CTO then g2v3 relative time.
        let pts = parse_points(&hex(
            "33 01 07 01 10 27 00 00 00 00 02 03 17 01 02 81 05 00",
        ))
        .unwrap();
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].time_ms, Some(10_005));
        assert!(pts[0].is_event());
    }

    #[test]
    fn g10_binary_output_status() {
        let pts = parse_points(&hex("0A 01 00 00 03 0A")).unwrap();
        assert_eq!(
            pts.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![0.0, 1.0, 0.0, 1.0]
        );
        assert_eq!(one("0A 02 00 00 00 81").value, 1.0);
        assert_eq!(one("0B 01 17 01 00 01").value, 0.0);
    }

    #[test]
    fn g20_and_g22_counters() {
        assert_eq!(one("14 01 00 00 00 01 40 E2 01 00").value, 123_456.0);
        assert_eq!(one("14 02 00 00 00 01 39 30").value, 12_345.0);
        assert_eq!(one("14 05 00 00 00 40 E2 01 00").value, 123_456.0);
        assert_eq!(one("14 06 00 00 00 39 30").value, 12_345.0);
        let p = one("16 05 17 01 02 01 07 00 00 00 E8 03 00 00 00 00");
        assert_eq!((p.index, p.value, p.time_ms), (2, 7.0, Some(1000)));
        assert_eq!(one("16 02 17 01 00 01 07 00").value, 7.0);
    }

    #[test]
    fn g30_all_variations() {
        assert_eq!(one("1E 01 00 00 00 01 18 FC FF FF").value, -1000.0);
        assert_eq!(one("1E 02 00 00 00 01 18 FC").value, -1000.0);
        assert_eq!(one("1E 03 00 00 00 E8 03 00 00").value, 1000.0);
        assert_eq!(one("1E 04 00 00 00 E8 03").value, 1000.0);
        assert_eq!(one("1E 05 00 00 00 01 00 00 80 3F").value, 1.0);
        assert_eq!(one("1E 06 00 00 00 01 00 00 00 00 00 00 F0 3F").value, 1.0);
        let p = one("1E 01 00 00 00 05 00 00 00 00");
        assert_eq!(p.flags, 0x05);
        assert!(!p.is_good());
    }

    #[test]
    fn g32_all_variations() {
        let t = "E8 03 00 00 00 00";
        assert_eq!(one("20 01 17 01 01 01 0A 00 00 00").value, 10.0);
        assert_eq!(one("20 02 17 01 01 01 0A 00").value, 10.0);
        let p = one(&format!("20 03 17 01 01 01 0A 00 00 00 {t}"));
        assert_eq!((p.value, p.time_ms), (10.0, Some(1000)));
        let p = one(&format!("20 04 17 01 01 01 0A 00 {t}"));
        assert_eq!((p.value, p.time_ms), (10.0, Some(1000)));
        assert_eq!(one("20 05 17 01 01 01 00 00 20 41").value, 10.0);
        assert_eq!(one("20 06 17 01 01 01 00 00 00 00 00 00 24 40").value, 10.0);
        let p = one(&format!("20 07 17 01 01 01 00 00 20 41 {t}"));
        assert_eq!((p.value, p.time_ms), (10.0, Some(1000)));
        let p = one(&format!(
            "20 08 28 01 00 01 00 01 00 00 00 00 00 00 24 40 {t}"
        ));
        assert_eq!((p.index, p.value, p.time_ms), (1, 10.0, Some(1000)));
    }

    #[test]
    fn g40_analog_output_status() {
        assert_eq!(one("28 01 00 00 00 01 64 00 00 00").value, 100.0);
        assert_eq!(one("28 02 00 00 00 01 64 00").value, 100.0);
        assert_eq!(one("28 03 00 00 00 01 00 00 C8 42").value, 100.0);
        assert_eq!(
            one("28 04 00 00 00 01 00 00 00 00 00 00 59 40").value,
            100.0
        );
    }

    #[test]
    fn command_echoes_parse_but_are_not_points() {
        let blocks =
            parse_object_headers(&hex("0C 01 28 01 00 03 00 03 01 00 00 00 00 00 00 00 00 00 29 01 28 01 00 00 00 05 00 00 00 00"))
                .unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].items[0].index, 3);
        assert_eq!(blocks[1].items[0].data, hex("05 00 00 00 00"));
        assert!(points_from_blocks(&blocks).is_empty());
    }

    #[test]
    fn g50_g52_g80() {
        let b = parse_object_headers(&hex("32 01 07 01 01 02 03 04 05 06")).unwrap();
        assert_eq!(le_u48(&b[0].items[0].data), 0x0605_0403_0201);
        let b = parse_object_headers(&hex("34 02 07 01 2C 01")).unwrap();
        assert_eq!(b[0].items[0].data, hex("2C 01"));
        let b = parse_object_headers(&hex("50 01 00 07 07 00")).unwrap();
        assert_eq!(
            b[0].items[0],
            ObjectItem {
                index: 7,
                data: vec![0]
            }
        );
    }

    #[test]
    fn qualifiers_00_01_06_07_08_17_28() {
        assert_eq!(one("1E 03 00 05 05 01 00 00 00").index, 5);
        assert_eq!(one("1E 03 01 2C 01 2C 01 01 00 00 00").index, 300);
        let b = parse_object_headers(&hex("3C 01 06 1E 00 06")).unwrap();
        assert_eq!(b.len(), 2);
        assert!(b[1].items.is_empty());
        assert_eq!(one("1E 03 07 01 01 00 00 00").index, 0);
        assert_eq!(one("1E 03 08 01 00 01 00 00 00").index, 0);
        assert_eq!(one("1E 03 17 01 09 01 00 00 00").index, 9);
        assert_eq!(one("1E 03 28 01 00 2C 01 01 00 00 00").index, 300);
    }

    #[test]
    fn unknown_object_and_truncation_are_errors() {
        assert!(parse_object_headers(&hex("63 01 00 00 00 00")).is_err());
        assert!(parse_object_headers(&hex("1E 01 00 00 01 01 00 00 00 00")).is_err());
        assert!(parse_object_headers(&hex("1E 01")).is_err());
    }

    #[test]
    fn several_headers_in_one_response() {
        let pts = parse_points(&hex(
            "01 02 00 00 01 81 01 1E 01 00 00 00 01 2A 00 00 00 28 02 00 00 00 01 07 00",
        ))
        .unwrap();
        assert_eq!(pts.len(), 4);
        assert_eq!((pts[2].group, pts[2].value), (30, 42.0));
        assert_eq!((pts[3].group, pts[3].value), (40, 7.0));
    }

    #[test]
    fn flags_to_good() {
        assert!(dnp3_flags_to_good(0x01));
        assert!(dnp3_flags_to_good(0x81));
        assert!(dnp3_flags_to_good(0x09));
        assert!(!dnp3_flags_to_good(0x00));
        assert!(!dnp3_flags_to_good(0x03));
        assert!(!dnp3_flags_to_good(0x05));
        assert!(!dnp3_flags_to_good(0x21));
    }

    #[test]
    fn catalogue_names() {
        assert_eq!(group_variation_name(32, 1), "Analog Input Event (32-bit)");
        assert_eq!(
            group_variation_name(40, 2),
            "Analog Output Status (16-bit with flags)"
        );
        assert_eq!(
            group_variation_name(30, 5),
            "Analog Input (float32 with flags)"
        );
        assert_eq!(static_group(32), 30);
        assert_eq!(static_group(2), 1);
        assert_eq!(static_group(40), 40);
    }
}
