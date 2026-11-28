//! From captured packets to protocol frames: link layer, IPv4/IPv6, TCP, and
//! a TCP stream reassembly that is enough for request/response protocols
//! (in-order data, retransmissions and overlaps dropped, a gap restarts the
//! stream).

use std::collections::HashMap;
use std::net::IpAddr;

use crate::pcap::Packet;
use crate::{DecodedFrame, Dir, modbus};

/// A TCP segment and its endpoints.
#[derive(Debug, Clone, Copy)]
pub struct Segment<'a> {
    pub src: (IpAddr, u16),
    pub dst: (IpAddr, u16),
    pub seq: u32,
    pub syn: bool,
    pub fin_or_rst: bool,
    pub payload: &'a [u8],
}

/// The TCP segment in a captured packet, if it holds one.
pub fn tcp_segment(linktype: u32, data: &[u8]) -> Option<Segment<'_>> {
    let (ethertype, ip) = match linktype {
        // Ethernet, with 802.1Q tags skipped.
        1 => {
            let mut off = 12;
            let mut et = u16::from_be_bytes([*data.get(off)?, *data.get(off + 1)?]);
            while et == 0x8100 || et == 0x88A8 {
                off += 4;
                et = u16::from_be_bytes([*data.get(off)?, *data.get(off + 1)?]);
            }
            (et, data.get(off + 2..)?)
        }
        // BSD loopback: address family in host byte order.
        0 => {
            let fam = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
            let fam = if fam > 0xFFFF { fam.swap_bytes() } else { fam };
            (if fam == 2 { 0x0800 } else { 0x86DD }, data.get(4..)?)
        }
        // Linux cooked v1 and v2.
        113 => (
            u16::from_be_bytes([*data.get(14)?, *data.get(15)?]),
            data.get(16..)?,
        ),
        276 => (
            u16::from_be_bytes([*data.first()?, *data.get(1)?]),
            data.get(20..)?,
        ),
        // Raw IP.
        12 | 14 | 101 | 228 | 229 => {
            let v = data.first()? >> 4;
            (if v == 4 { 0x0800 } else { 0x86DD }, data)
        }
        _ => return None,
    };
    let (src, dst, tcp) = match ethertype {
        0x0800 => {
            if ip.first()? >> 4 != 4 || *ip.get(9)? != 6 {
                return None;
            }
            let ihl = usize::from(ip[0] & 0x0F) * 4;
            let total = usize::from(u16::from_be_bytes([*ip.get(2)?, *ip.get(3)?]));
            // Fragments are not reassembled.
            let frag = u16::from_be_bytes([*ip.get(6)?, *ip.get(7)?]);
            if frag & 0x3FFF != 0 {
                return None;
            }
            let s: [u8; 4] = ip.get(12..16)?.try_into().ok()?;
            let d: [u8; 4] = ip.get(16..20)?.try_into().ok()?;
            let end = total.min(ip.len());
            (IpAddr::from(s), IpAddr::from(d), ip.get(ihl..end)?)
        }
        0x86DD => {
            if ip.first()? >> 4 != 6 || *ip.get(6)? != 6 {
                return None;
            }
            let plen = usize::from(u16::from_be_bytes([*ip.get(4)?, *ip.get(5)?]));
            let s: [u8; 16] = ip.get(8..24)?.try_into().ok()?;
            let d: [u8; 16] = ip.get(24..40)?.try_into().ok()?;
            let end = (40 + plen).min(ip.len());
            (IpAddr::from(s), IpAddr::from(d), ip.get(40..end)?)
        }
        _ => return None,
    };
    let sport = u16::from_be_bytes([*tcp.first()?, *tcp.get(1)?]);
    let dport = u16::from_be_bytes([*tcp.get(2)?, *tcp.get(3)?]);
    let seq = u32::from_be_bytes(tcp.get(4..8)?.try_into().ok()?);
    let doff = usize::from(tcp.get(12)? >> 4) * 4;
    let flags = *tcp.get(13)?;
    Some(Segment {
        src: (src, sport),
        dst: (dst, dport),
        seq,
        syn: flags & 0x02 != 0,
        fin_or_rst: flags & 0x05 != 0,
        payload: tcp.get(doff..)?,
    })
}

/// One direction of a TCP connection.
#[derive(Debug, Default)]
struct HalfStream {
    next: Option<u32>,
    splitter: modbus::Splitter,
}

impl HalfStream {
    /// The new bytes of a segment, after dropping what was already seen.
    fn accept<'a>(&mut self, seg: &Segment<'a>) -> &'a [u8] {
        if seg.syn {
            self.next = Some(seg.seq.wrapping_add(1));
            self.splitter.reset();
            return &[];
        }
        let len = seg.payload.len() as u32;
        let data = match self.next {
            Some(next) => {
                let ahead = seg.seq.wrapping_sub(next) as i32;
                if ahead > 0 {
                    // Bytes missing from the capture: start again here.
                    self.splitter.reset();
                    seg.payload
                } else {
                    let skip = ahead.unsigned_abs();
                    if skip >= len {
                        return &[];
                    }
                    &seg.payload[skip as usize..]
                }
            }
            None => seg.payload,
        };
        self.next = Some(seg.seq.wrapping_add(len));
        data
    }
}

type Endpoint = (IpAddr, u16);

#[derive(Default)]
struct Conn {
    to_server: HalfStream,
    to_client: HalfStream,
    session: modbus::Session,
}

/// Decode the Modbus/TCP traffic of a capture: frames to or from `port`
/// (502 normally), with each frame's `seq` set to its packet number. Several
/// ADUs in one packet give several frames; an ADU split over segments is
/// decoded at the packet that completes it.
pub fn modbus_frames(packets: &[Packet<'_>], port: u16) -> Vec<DecodedFrame> {
    let mut conns: HashMap<(Endpoint, Endpoint), Conn> = HashMap::new();
    let mut out = Vec::new();
    for p in packets {
        let Some(seg) = tcp_segment(p.linktype, p.data) else {
            continue;
        };
        let (dir, key) = if seg.dst.1 == port {
            (Dir::Tx, (seg.src, seg.dst))
        } else if seg.src.1 == port {
            (Dir::Rx, (seg.dst, seg.src))
        } else {
            continue;
        };
        let conn = conns.entry(key).or_default();
        let half = match dir {
            Dir::Tx => &mut conn.to_server,
            Dir::Rx => &mut conn.to_client,
        };
        let data = half.accept(&seg);
        let session = &mut conn.session;
        half.splitter.push(data, |adu| {
            out.push(session.decode(adu, dir, p.ts_us, p.number));
        });
        if seg.fin_or_rst && seg.payload.is_empty() {
            half.splitter.reset();
        }
    }
    out
}
