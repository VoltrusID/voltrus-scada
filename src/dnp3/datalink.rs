//! DNP3 data link layer (IEEE 1815-2012 clause 9): FT3 frames with a CRC
//! after the 8-byte header and after every 16 bytes of user data.

use anyhow::Result;

/// Most user data octets one link frame can carry (LENGTH max 255 minus the
/// 5 octets of control and addresses it also counts).
pub const MAX_USER_DATA: usize = 250;

/// CRC-16/DNP: polynomial 0x3D65, processed LSB first (so the table uses the
/// bit-reversed 0xA6BC), initial value 0, result inverted.
const CRC_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA6BC
            } else {
                crc >> 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

pub fn crc_dnp(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ b as u16) & 0xFF) as usize];
    }
    !crc
}

/// Link function codes. Primary (PRM=1) and secondary (PRM=0) codes overlap
/// numerically, so the meaning depends on the PRM bit of the control octet.
pub mod func {
    pub const PRI_RESET_LINK_STATES: u8 = 0;
    pub const PRI_TEST_LINK_STATES: u8 = 2;
    pub const PRI_CONFIRMED_USER_DATA: u8 = 3;
    pub const PRI_UNCONFIRMED_USER_DATA: u8 = 4;
    pub const PRI_REQUEST_LINK_STATUS: u8 = 9;
    pub const SEC_ACK: u8 = 0;
    pub const SEC_NACK: u8 = 1;
    pub const SEC_LINK_STATUS: u8 = 11;
    pub const SEC_NOT_SUPPORTED: u8 = 15;
}

/// Link control octet: DIR | PRM | FCB | FCV(DFC) | FUNC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlByte {
    /// Set on every frame sent by a master, primary or secondary.
    pub dir: bool,
    pub prm: bool,
    pub fcb: bool,
    /// FCV on primary frames, DFC on secondary frames.
    pub fcv: bool,
    pub func: u8,
}

impl ControlByte {
    pub fn to_byte(self) -> u8 {
        (if self.dir { 0x80 } else { 0 })
            | (if self.prm { 0x40 } else { 0 })
            | (if self.fcb { 0x20 } else { 0 })
            | (if self.fcv { 0x10 } else { 0 })
            | (self.func & 0x0F)
    }

    pub fn from_byte(b: u8) -> Self {
        Self {
            dir: b & 0x80 != 0,
            prm: b & 0x40 != 0,
            fcb: b & 0x20 != 0,
            fcv: b & 0x10 != 0,
            func: b & 0x0F,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFrame {
    pub control: ControlByte,
    pub destination: u16,
    pub source: u16,
    pub data: Vec<u8>,
}

impl LinkFrame {
    pub fn new(control: ControlByte, destination: u16, source: u16, data: Vec<u8>) -> Self {
        Self {
            control,
            destination,
            source,
            data,
        }
    }

    /// Unconfirmed user data is what masters and outstations use over TCP;
    /// confirmed link service only adds round trips on a reliable transport.
    pub fn user_data(from_master: bool, destination: u16, source: u16, data: Vec<u8>) -> Self {
        let control = ControlByte {
            dir: from_master,
            prm: true,
            fcb: false,
            fcv: false,
            func: func::PRI_UNCONFIRMED_USER_DATA,
        };
        Self::new(control, destination, source, data)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.data.len() > MAX_USER_DATA {
            anyhow::bail!(
                "DNP3 link frame user data is {} bytes, the maximum is {MAX_USER_DATA}",
                self.data.len()
            );
        }
        let mut out = Vec::with_capacity(encoded_len(self.data.len()));
        out.extend_from_slice(&[
            0x05,
            0x64,
            (5 + self.data.len()) as u8,
            self.control.to_byte(),
        ]);
        out.extend_from_slice(&self.destination.to_le_bytes());
        out.extend_from_slice(&self.source.to_le_bytes());
        let crc = crc_dnp(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        for block in self.data.chunks(16) {
            out.extend_from_slice(block);
            out.extend_from_slice(&crc_dnp(block).to_le_bytes());
        }
        Ok(out)
    }

    /// Decode exactly one complete frame.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        match parse_frame(bytes)? {
            ParseResult::Frame(f, used) if used == bytes.len() => Ok(f),
            ParseResult::Frame(_, used) => {
                anyhow::bail!("{} trailing bytes after frame", bytes.len() - used)
            }
            ParseResult::Incomplete => anyhow::bail!("DNP3 frame truncated"),
        }
    }
}

/// Total wire size of a frame carrying `data_len` user data octets.
pub fn encoded_len(data_len: usize) -> usize {
    10 + data_len + 2 * data_len.div_ceil(16)
}

pub enum ParseResult {
    Frame(LinkFrame, usize),
    Incomplete,
}

/// Parse a frame at the start of `buf`. Errors mean the bytes at the start
/// cannot be a valid frame; the caller should skip a byte and resynchronise.
pub fn parse_frame(buf: &[u8]) -> Result<ParseResult> {
    if buf.len() < 10 {
        if buf.first().is_some_and(|&b| b != 0x05) || buf.get(1).is_some_and(|&b| b != 0x64) {
            anyhow::bail!("bad DNP3 start octets");
        }
        return Ok(ParseResult::Incomplete);
    }
    if buf[0] != 0x05 || buf[1] != 0x64 {
        anyhow::bail!("bad DNP3 start octets {:02X} {:02X}", buf[0], buf[1]);
    }
    let len = buf[2] as usize;
    if len < 5 {
        anyhow::bail!("DNP3 LENGTH {len} is below the minimum of 5");
    }
    let header_crc = u16::from_le_bytes([buf[8], buf[9]]);
    if crc_dnp(&buf[0..8]) != header_crc {
        anyhow::bail!("DNP3 header CRC mismatch");
    }
    let data_len = len - 5;
    let total = encoded_len(data_len);
    if buf.len() < total {
        return Ok(ParseResult::Incomplete);
    }
    let mut data = Vec::with_capacity(data_len);
    let mut pos = 10;
    let mut remaining = data_len;
    while remaining > 0 {
        let n = remaining.min(16);
        let block = &buf[pos..pos + n];
        let crc = u16::from_le_bytes([buf[pos + n], buf[pos + n + 1]]);
        if crc_dnp(block) != crc {
            anyhow::bail!("DNP3 data block CRC mismatch");
        }
        data.extend_from_slice(block);
        pos += n + 2;
        remaining -= n;
    }
    let frame = LinkFrame {
        control: ControlByte::from_byte(buf[3]),
        destination: u16::from_le_bytes([buf[4], buf[5]]),
        source: u16::from_le_bytes([buf[6], buf[7]]),
        data,
    };
    Ok(ParseResult::Frame(frame, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn crc_matches_the_catalogue_check_value() {
        // CRC-16/DNP check value from the CRC catalogue (reveng).
        assert_eq!(crc_dnp(b"123456789"), 0xEA82);
    }

    #[test]
    fn reset_link_states_matches_known_frame() {
        // Reset Link States, master 1024 -> outstation 1 (opendnp3 test vector).
        let frame = LinkFrame::new(
            ControlByte {
                dir: true,
                prm: true,
                fcb: false,
                fcv: false,
                func: func::PRI_RESET_LINK_STATES,
            },
            1,
            1024,
            vec![],
        );
        let expected = hex("05 64 05 C0 01 00 00 04 E9 21");
        assert_eq!(frame.encode().unwrap(), expected);
        assert_eq!(LinkFrame::decode(&expected).unwrap(), frame);
    }

    #[test]
    fn ack_matches_known_frame() {
        // Secondary ACK, outstation 1 -> master 1024.
        let frame = LinkFrame::new(ControlByte::from_byte(0x00), 1024, 1, vec![]);
        assert_eq!(
            frame.encode().unwrap(),
            hex("05 64 05 00 00 04 01 00 19 A6")
        );
    }

    #[test]
    fn user_data_is_split_into_crc_blocks() {
        let data: Vec<u8> = (0..20).collect();
        let frame = LinkFrame::user_data(true, 10, 1, data.clone());
        let bytes = frame.encode().unwrap();
        assert_eq!(bytes.len(), 10 + 16 + 2 + 4 + 2);
        assert_eq!(bytes[2], 25);
        assert_eq!(bytes[3], 0xC4);
        assert_eq!(&bytes[10..26], &data[..16]);
        assert_eq!(
            u16::from_le_bytes([bytes[26], bytes[27]]),
            crc_dnp(&data[..16])
        );
        assert_eq!(&bytes[28..32], &data[16..]);
        assert_eq!(LinkFrame::decode(&bytes).unwrap(), frame);
    }

    #[test]
    fn full_frame_with_integrity_poll() {
        // Transport C0 + integrity poll APDU, master 1 -> outstation 10.
        let apdu = hex("C0 C0 01 3C 02 06 3C 03 06 3C 04 06 3C 01 06");
        let bytes = LinkFrame::user_data(true, 10, 1, apdu.clone())
            .encode()
            .unwrap();
        assert_eq!(&bytes[..8], &hex("05 64 14 C4 0A 00 01 00")[..]);
        assert_eq!(bytes.len(), encoded_len(apdu.len()));
        assert_eq!(LinkFrame::decode(&bytes).unwrap().data, apdu);
    }

    #[test]
    fn corrupt_block_crc_is_rejected() {
        let mut bytes = LinkFrame::user_data(false, 1, 10, vec![1, 2, 3])
            .encode()
            .unwrap();
        bytes[11] ^= 0xFF;
        assert!(LinkFrame::decode(&bytes).is_err());
        let mut bytes = LinkFrame::user_data(false, 1, 10, vec![1, 2, 3])
            .encode()
            .unwrap();
        bytes[4] ^= 0x01;
        assert!(LinkFrame::decode(&bytes).is_err());
    }

    #[test]
    fn partial_frame_is_incomplete() {
        let bytes = LinkFrame::user_data(false, 1, 10, vec![9; 40])
            .encode()
            .unwrap();
        assert!(matches!(
            parse_frame(&bytes[..bytes.len() - 1]).unwrap(),
            ParseResult::Incomplete
        ));
        assert!(matches!(
            parse_frame(&bytes[..4]).unwrap(),
            ParseResult::Incomplete
        ));
    }

    #[test]
    fn oversized_user_data_is_refused() {
        assert!(
            LinkFrame::user_data(true, 10, 1, vec![0; 251])
                .encode()
                .is_err()
        );
        assert!(
            LinkFrame::user_data(true, 10, 1, vec![0; 250])
                .encode()
                .is_ok()
        );
    }
}
