//! DNP3 transport function (IEEE 1815-2012 clause 8): one header octet per
//! link frame, FIN (bit 7) | FIR (bit 6) | SEQ (6 bits).

use crate::dnp3::datalink::MAX_USER_DATA;

/// Application octets per segment: one link frame minus the transport header.
pub const MAX_SEGMENT_PAYLOAD: usize = MAX_USER_DATA - 1;

/// Upper bound on a reassembled fragment. The spec lets devices choose; 2048
/// is the common default and this leaves room for larger outstations.
pub const MAX_FRAGMENT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportHeader {
    pub fin: bool,
    pub fir: bool,
    pub seq: u8,
}

impl TransportHeader {
    pub fn to_byte(self) -> u8 {
        (if self.fin { 0x80 } else { 0 }) | (if self.fir { 0x40 } else { 0 }) | (self.seq & 0x3F)
    }

    pub fn from_byte(b: u8) -> Self {
        Self {
            fin: b & 0x80 != 0,
            fir: b & 0x40 != 0,
            seq: b & 0x3F,
        }
    }
}

/// Split an APDU into transport segments (header octet + payload), numbering
/// them from `seq`. Returns the segments and the next sequence to use.
pub fn segment(apdu: &[u8], seq: u8) -> (Vec<Vec<u8>>, u8) {
    let mut seq = seq & 0x3F;
    let chunks: Vec<&[u8]> = if apdu.is_empty() {
        vec![&[][..]]
    } else {
        apdu.chunks(MAX_SEGMENT_PAYLOAD).collect()
    };
    let n = chunks.len();
    let mut out = Vec::with_capacity(n);
    for (i, chunk) in chunks.into_iter().enumerate() {
        let h = TransportHeader {
            fin: i + 1 == n,
            fir: i == 0,
            seq,
        };
        let mut s = Vec::with_capacity(chunk.len() + 1);
        s.push(h.to_byte());
        s.extend_from_slice(chunk);
        out.push(s);
        seq = (seq + 1) & 0x3F;
    }
    (out, seq)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentResult {
    /// A whole APDU is ready.
    Complete(Vec<u8>),
    /// More segments are expected.
    Pending,
    /// The segment was dropped (spec 8.2.1.2: discard non-FIR segments when
    /// idle, and abandon the fragment on a sequence gap).
    Discarded(&'static str),
}

/// Rebuilds APDUs from the transport segments of consecutive link frames.
#[derive(Debug, Default)]
pub struct TransportReassembler {
    buffer: Vec<u8>,
    expected_seq: u8,
    active: bool,
}

impl TransportReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, segment: &[u8]) -> SegmentResult {
        let Some((&hdr, payload)) = segment.split_first() else {
            return SegmentResult::Discarded("empty segment");
        };
        let h = TransportHeader::from_byte(hdr);
        if h.fir {
            // A new FIR always starts over, even mid-fragment: the sender has
            // given up on the old one.
            self.buffer.clear();
            self.active = true;
        } else if !self.active {
            return SegmentResult::Discarded("segment without FIR while idle");
        } else if h.seq != self.expected_seq {
            self.reset();
            return SegmentResult::Discarded("transport sequence gap");
        }
        if self.buffer.len() + payload.len() > MAX_FRAGMENT {
            self.reset();
            return SegmentResult::Discarded("fragment too large");
        }
        self.buffer.extend_from_slice(payload);
        self.expected_seq = (h.seq + 1) & 0x3F;
        if h.fin {
            self.active = false;
            SegmentResult::Complete(std::mem::take(&mut self.buffer))
        } else {
            SegmentResult::Pending
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_bits_follow_the_spec() {
        let h = TransportHeader {
            fin: true,
            fir: true,
            seq: 3,
        };
        assert_eq!(h.to_byte(), 0xC3);
        assert_eq!(
            TransportHeader::from_byte(0x40),
            TransportHeader {
                fin: false,
                fir: true,
                seq: 0
            }
        );
        assert_eq!(
            TransportHeader::from_byte(0x85),
            TransportHeader {
                fin: true,
                fir: false,
                seq: 5
            }
        );
    }

    #[test]
    fn single_segment_round_trip() {
        let (segs, next) = segment(&[0xC0, 0x01], 63);
        assert_eq!(segs, vec![vec![0xFF, 0xC0, 0x01]]);
        assert_eq!(next, 0);
        let mut r = TransportReassembler::new();
        assert_eq!(r.push(&segs[0]), SegmentResult::Complete(vec![0xC0, 0x01]));
    }

    #[test]
    fn multi_frame_reassembly() {
        let apdu: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
        let (segs, next) = segment(&apdu, 62);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0][0], 0x40 | 62);
        assert_eq!(segs[1][0], 63);
        assert_eq!(segs[2][0], 0x80);
        assert_eq!(next, 1);
        let mut r = TransportReassembler::new();
        assert_eq!(r.push(&segs[0]), SegmentResult::Pending);
        assert_eq!(r.push(&segs[1]), SegmentResult::Pending);
        assert_eq!(r.push(&segs[2]), SegmentResult::Complete(apdu));
    }

    #[test]
    fn out_of_sequence_segment_abandons_the_fragment() {
        let apdu = vec![7u8; 600];
        let (segs, _) = segment(&apdu, 0);
        let mut r = TransportReassembler::new();
        assert_eq!(r.push(&segs[0]), SegmentResult::Pending);
        assert!(matches!(r.push(&segs[2]), SegmentResult::Discarded(_)));
        // The rest of the old fragment is now noise.
        assert!(matches!(r.push(&segs[1]), SegmentResult::Discarded(_)));
        // A fresh FIR recovers.
        let (again, _) = segment(&[1, 2], 9);
        assert_eq!(r.push(&again[0]), SegmentResult::Complete(vec![1, 2]));
    }

    #[test]
    fn repeated_segment_is_rejected() {
        let (segs, _) = segment(&[3u8; 300], 4);
        let mut r = TransportReassembler::new();
        assert_eq!(r.push(&segs[0]), SegmentResult::Pending);
        assert!(matches!(r.push(&segs[0][..]), SegmentResult::Pending));
        // A duplicated FIR restarts; a duplicated middle segment is a gap.
        let mut r = TransportReassembler::new();
        let (three, _) = segment(&[3u8; 600], 4);
        r.push(&three[0]);
        r.push(&three[1]);
        assert!(matches!(r.push(&three[1]), SegmentResult::Discarded(_)));
    }

    #[test]
    fn non_fir_while_idle_is_discarded() {
        let mut r = TransportReassembler::new();
        assert!(matches!(r.push(&[0x81, 1]), SegmentResult::Discarded(_)));
    }

    #[test]
    fn new_fir_restarts_mid_fragment() {
        let mut r = TransportReassembler::new();
        assert_eq!(r.push(&[0x40, 1]), SegmentResult::Pending);
        assert_eq!(r.push(&[0xC5, 9]), SegmentResult::Complete(vec![9]));
    }
}
