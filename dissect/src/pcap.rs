//! Reading capture files from memory: classic pcap (microsecond and
//! nanosecond, either byte order) and pcapng (section header, interface
//! description, enhanced and simple packet blocks). Written from the file
//! format descriptions (pcap-savefile(5), draft-ietf-opsawg-pcapng).

/// One captured packet.
#[derive(Debug, Clone, Copy)]
pub struct Packet<'a> {
    /// 1-based, as Wireshark numbers frames.
    pub number: u64,
    pub ts_us: i64,
    /// LINKTYPE_* of the interface.
    pub linktype: u32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NotACapture,
    Truncated,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::NotACapture => "not a pcap or pcapng file",
            Error::Truncated => "the capture file is truncated",
        })
    }
}

impl std::error::Error for Error {}

/// Every packet of a pcap or pcapng file. A truncated last packet ends the
/// list with [`Error::Truncated`] after the packets before it.
pub fn read(file: &[u8]) -> Result<Vec<Packet<'_>>, Error> {
    let magic = file.get(..4).ok_or(Error::NotACapture)?;
    match magic {
        [0x0A, 0x0D, 0x0D, 0x0A] => read_pcapng(file),
        _ => read_pcap(file),
    }
}

fn read_pcap(file: &[u8]) -> Result<Vec<Packet<'_>>, Error> {
    let magic_le = u32::from_le_bytes(file[..4].try_into().unwrap_or_default());
    let (le, nanos) = match magic_le {
        0xA1B2_C3D4 => (true, false),
        0xA1B2_3C4D => (true, true),
        0xD4C3_B2A1 => (false, false),
        0x4D3C_B2A1 => (false, true),
        _ => return Err(Error::NotACapture),
    };
    let r = Reader { b: file, le };
    let linktype = r.u32(20).ok_or(Error::Truncated)? & 0x0FFF_FFFF;
    let mut out = Vec::new();
    let mut pos = 24;
    while pos < file.len() {
        let (Some(sec), Some(frac), Some(caplen)) = (r.u32(pos), r.u32(pos + 4), r.u32(pos + 8))
        else {
            return Err(Error::Truncated);
        };
        let start = pos + 16;
        let data = file
            .get(start..start + caplen as usize)
            .ok_or(Error::Truncated)?;
        let frac_us = if nanos { frac / 1000 } else { frac };
        out.push(Packet {
            number: out.len() as u64 + 1,
            ts_us: i64::from(sec) * 1_000_000 + i64::from(frac_us),
            linktype,
            data,
        });
        pos = start + caplen as usize;
    }
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    le: bool,
}

impl Reader<'_> {
    fn u16(&self, off: usize) -> Option<u16> {
        let b: [u8; 2] = self.b.get(off..off + 2)?.try_into().ok()?;
        Some(if self.le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }

    fn u32(&self, off: usize) -> Option<u32> {
        let b: [u8; 4] = self.b.get(off..off + 4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
}

struct Interface {
    linktype: u32,
    /// Timestamp units per second.
    units: u64,
}

fn read_pcapng(file: &[u8]) -> Result<Vec<Packet<'_>>, Error> {
    let mut out = Vec::new();
    let mut ifaces: Vec<Interface> = Vec::new();
    let mut le = true;
    let mut pos = 0;
    while pos < file.len() {
        let block_type_raw = file.get(pos..pos + 4).ok_or(Error::Truncated)?;
        if block_type_raw == [0x0A, 0x0D, 0x0D, 0x0A] {
            // Section header: its byte-order magic decides the order of the section.
            le = match file.get(pos + 8..pos + 12) {
                Some([0x4D, 0x3C, 0x2B, 0x1A]) => true,
                Some([0x1A, 0x2B, 0x3C, 0x4D]) => false,
                _ => return Err(Error::NotACapture),
            };
            ifaces.clear();
        }
        let r = Reader { b: file, le };
        let block_type = r.u32(pos).ok_or(Error::Truncated)?;
        let len = r.u32(pos + 4).ok_or(Error::Truncated)? as usize;
        if len < 12 || !len.is_multiple_of(4) || pos + len > file.len() {
            return Err(Error::Truncated);
        }
        let body = pos + 8;
        match block_type {
            1 => {
                let linktype = u32::from(r.u16(body).ok_or(Error::Truncated)?);
                let mut units = 1_000_000;
                // Options: code, length, value padded to 4 bytes.
                let mut o = body + 8;
                while o + 4 <= pos + len - 4 {
                    let (Some(code), Some(olen)) = (r.u16(o), r.u16(o + 2)) else {
                        break;
                    };
                    if code == 0 {
                        break;
                    }
                    if code == 9 && olen == 1 {
                        let v = file[o + 4];
                        units = if v & 0x80 != 0 {
                            1u64.checked_shl(u32::from(v & 0x7F)).unwrap_or(1_000_000)
                        } else {
                            10u64.checked_pow(u32::from(v)).unwrap_or(1_000_000)
                        };
                    }
                    o += 4 + usize::from(olen).div_ceil(4) * 4;
                }
                ifaces.push(Interface { linktype, units });
            }
            6 => {
                let iface = r.u32(body).ok_or(Error::Truncated)? as usize;
                let ts = (u64::from(r.u32(body + 4).ok_or(Error::Truncated)?) << 32)
                    | u64::from(r.u32(body + 8).ok_or(Error::Truncated)?);
                let caplen = r.u32(body + 12).ok_or(Error::Truncated)? as usize;
                let data = file
                    .get(body + 20..body + 20 + caplen)
                    .ok_or(Error::Truncated)?;
                let Some(i) = ifaces.get(iface) else {
                    return Err(Error::NotACapture);
                };
                out.push(Packet {
                    number: out.len() as u64 + 1,
                    ts_us: to_us(ts, i.units),
                    linktype: i.linktype,
                    data,
                });
            }
            3 => {
                let caplen = (len - 16).min(r.u32(body).ok_or(Error::Truncated)? as usize);
                let data = file
                    .get(body + 4..body + 4 + caplen)
                    .ok_or(Error::Truncated)?;
                let linktype = ifaces.first().map_or(1, |i| i.linktype);
                out.push(Packet {
                    number: out.len() as u64 + 1,
                    ts_us: 0,
                    linktype,
                    data,
                });
            }
            _ => {}
        }
        pos += len;
    }
    Ok(out)
}

fn to_us(ts: u64, units: u64) -> i64 {
    if units == 1_000_000 {
        ts as i64
    } else {
        (u128::from(ts) * 1_000_000 / u128::from(units.max(1))) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage_and_truncation() {
        assert_eq!(read(b"hi").unwrap_err(), Error::NotACapture);
        assert_eq!(read(&[0u8; 30]).unwrap_err(), Error::NotACapture);
        let mut pcap = vec![0xD4, 0xC3, 0xB2, 0xA1, 2, 0, 4, 0];
        pcap.extend([0; 12]);
        pcap.extend(1u32.to_le_bytes());
        assert!(read(&pcap).unwrap().is_empty());
        pcap.extend([0; 8]);
        pcap.extend(100u32.to_le_bytes());
        pcap.extend(100u32.to_le_bytes());
        assert_eq!(read(&pcap).unwrap_err(), Error::Truncated);
    }

    #[test]
    fn reads_pcapng_with_nanosecond_resolution() {
        let mut f = Vec::new();
        let block = |f: &mut Vec<u8>, t: u32, body: &[u8]| {
            let len = (12 + body.len()) as u32;
            f.extend(t.to_le_bytes());
            f.extend(len.to_le_bytes());
            f.extend(body);
            f.extend(len.to_le_bytes());
        };
        block(
            &mut f,
            0x0A0D0D0A,
            &[
                0x4D, 0x3C, 0x2B, 0x1A, 1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            ],
        );
        // Interface: Ethernet, snaplen, if_tsresol = 9, end of options.
        block(
            &mut f,
            1,
            &[1, 0, 0, 0, 0, 0, 4, 0, 9, 0, 1, 0, 9, 0, 0, 0, 0, 0, 0, 0],
        );
        let ts: u64 = 1_700_000_000_123_456_789;
        let mut epb = vec![0, 0, 0, 0];
        epb.extend(((ts >> 32) as u32).to_le_bytes());
        epb.extend((ts as u32).to_le_bytes());
        epb.extend(4u32.to_le_bytes());
        epb.extend(4u32.to_le_bytes());
        epb.extend([1, 2, 3, 4]);
        block(&mut f, 6, &epb);
        let p = read(&f).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].ts_us, 1_700_000_000_123_456);
        assert_eq!(p[0].data, [1, 2, 3, 4]);
        assert_eq!(p[0].linktype, 1);
    }
}
