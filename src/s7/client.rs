#![allow(dead_code)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::cloned_ref_to_slice_refs)]
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::types::*;

// ── S7 Client ──

pub struct S7Client {
    stream: Option<TcpStream>,
    url: String,
    rack: u8,
    slot: u8,
    pdu_ref: u16,
    pdu_size: u16,
}

impl S7Client {
    pub fn new(url: &str, rack: u8, slot: u8) -> Self {
        Self {
            stream: None,
            url: url.to_string(),
            rack,
            slot,
            pdu_ref: 0,
            pdu_size: DEFAULT_PDU_SIZE,
        }
    }

    /// Connect to the S7 PLC via ISO-on-TCP (TPKT + COTP + S7 Setup Communication).
    pub async fn connect(&mut self) -> Result<()> {
        let stream = TcpStream::connect(&self.url)
            .await
            .with_context(|| format!("Failed to connect to S7 PLC at {}", self.url))?;

        self.stream = Some(stream);

        // Send COTP Connection Request
        let cotp_cr = self.build_cotp_cr()?;
        self.send_raw(&cotp_cr).await?;

        // Read COTP Connection Confirm
        let cc = self.read_cotp_cc().await?;

        // Verify it's a CC
        if cc.len() < 6 || cc[1] != COTP_CC {
            anyhow::bail!(
                "Expected COTP Connection Confirm, got: {:02X?}",
                &cc[..cc.len().min(6)]
            );
        }

        // Send S7 Setup Communication
        let setup = self.build_s7_setup_comm();
        self.send_s7(&setup).await?;

        // Read S7 Setup Communication response
        let resp = self.read_s7_response().await?;
        self.parse_setup_comm_response(&resp)?;

        tracing::info!(
            "Connected to S7 PLC at {} (rack={}, slot={}), PDU size={}",
            self.url,
            self.rack,
            self.slot,
            self.pdu_size
        );

        Ok(())
    }

    /// Read a single tag from the PLC.
    pub async fn read_tag(&mut self, tag: &S7Tag) -> Result<S7Value> {
        let values = self.read_tags(&[tag.clone()]).await?;
        values
            .into_iter()
            .next()
            .context("No value returned for tag")
    }

    /// Read multiple tags in one S7 request.
    pub async fn read_tags(&mut self, tags: &[S7Tag]) -> Result<Vec<S7Value>> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        let request = self.build_read_var_request(tags)?;
        self.send_s7(&request).await?;

        let response = self.read_s7_response().await?;
        self.parse_read_var_response(&response, tags)
    }

    /// Write a single tag to the PLC.
    pub async fn write_tag(&mut self, tag: &S7Tag, value: &S7Value) -> Result<()> {
        let request = self.build_write_var_request(tag, value)?;
        self.send_s7(&request).await?;

        let response = self.read_s7_response().await?;
        self.parse_write_var_response(&response)
    }

    /// Disconnect from the PLC.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.shutdown().await;
        }
        Ok(())
    }

    /// Parse an S7 tag address string into an S7Tag struct.
    ///
    /// Supported formats:
    /// - "DB1.DBD0"     -> DB1, DWord at offset 0
    /// - "DB1.DBX0.1"   -> DB1, Bool at offset 0, bit 1
    /// - "DB1.DBW0"     -> DB1, Word at offset 0
    /// - "DB1.DBB0"     -> DB1, Byte at offset 0
    /// - "DB1.DBD100"   -> DB1, DWord at offset 100
    /// - "DB10.DBX10.3" -> DB10, Bool at offset 10, bit 3
    /// - "I0.0"         -> Input, Bool at offset 0, bit 0
    /// - "IB0"          -> Input, Byte at offset 0
    /// - "IW100"        -> Input, Word at offset 100
    /// - "ID100"        -> Input, DWord at offset 100
    /// - "Q0.0"         -> Output, Bool at offset 0, bit 0
    /// - "QB0"          -> Output, Byte at offset 0
    /// - "QW200"        -> Output, Word at offset 200
    /// - "MW100"        -> Marker, Word at offset 100
    /// - "MD100"        -> Marker, DWord at offset 100
    /// - "MB100"        -> Marker, Byte at offset 100
    pub fn parse_tag_address(addr: &str) -> Result<S7Tag> {
        let addr = addr.trim().to_uppercase();

        // DB addresses: DB<n>.DB<type><offset>[.bit]
        if let Some(rest) = addr.strip_prefix("DB") {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 2 {
                let db_number: u16 = parts[0]
                    .parse()
                    .with_context(|| format!("Invalid DB number in '{}'", addr))?;

                let type_and_offset = parts[1];
                if type_and_offset.starts_with("DBX")
                    || type_and_offset.starts_with("DBB")
                    || type_and_offset.starts_with("DBW")
                    || type_and_offset.starts_with("DBD")
                {
                    let (_, rest) = type_and_offset.split_at(3);
                    let data_type = if type_and_offset.starts_with("DBX") {
                        S7DataType::Bool
                    } else if type_and_offset.starts_with("DBB") {
                        S7DataType::Byte
                    } else if type_and_offset.starts_with("DBW") {
                        S7DataType::Word
                    } else {
                        S7DataType::DWord
                    };
                    let offset_str = rest;

                    let offset: u16 = offset_str
                        .parse()
                        .with_context(|| format!("Invalid offset in '{}'", addr))?;

                    let bit_offset = if parts.len() >= 3 && data_type == S7DataType::Bool {
                        parts[2]
                            .parse()
                            .with_context(|| format!("Invalid bit offset in '{}'", addr))?
                    } else {
                        0
                    };

                    return Ok(S7Tag {
                        area: S7Area::DB,
                        db_number,
                        offset,
                        bit_offset,
                        data_type,
                        length: 1,
                        name: None,
                    });
                }
            }
        }

        // I, Q, M addresses
        let (area, rest): (S7Area, &str) = if let Some(r) = addr.strip_prefix('I') {
            (S7Area::Input, r)
        } else if let Some(r) = addr.strip_prefix('Q') {
            (S7Area::Output, r)
        } else if let Some(r) = addr.strip_prefix('M') {
            (S7Area::Marker, r)
        } else {
            anyhow::bail!(
                "Unsupported tag address format: '{}'. Use DB<n>.DBx<m>, Ix, Qx, or Mx format.",
                addr
            );
        };

        // Check if it's a bit address (e.g., "I0.0", "Q0.1", "M10.3")
        if rest.contains('.') {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() == 2 {
                let offset: u16 = parts[0]
                    .parse()
                    .with_context(|| format!("Invalid offset in '{}'", addr))?;
                let bit_offset: u8 = parts[1]
                    .parse()
                    .with_context(|| format!("Invalid bit offset in '{}'", addr))?;
                return Ok(S7Tag {
                    area,
                    db_number: 0,
                    offset,
                    bit_offset,
                    data_type: S7DataType::Bool,
                    length: 1,
                    name: None,
                });
            }
        }

        // Byte/Word/DWord addresses (e.g., "IB0", "IW100", "ID100", "MB10", "MW100", "QW200")
        let (data_type, offset_str): (S7DataType, &str) = if let Some(s) = rest.strip_prefix('B') {
            (S7DataType::Byte, s)
        } else if let Some(s) = rest.strip_prefix('W') {
            (S7DataType::Word, s)
        } else if let Some(s) = rest.strip_prefix('D') {
            (S7DataType::DWord, s)
        } else {
            // Default to byte if just a number (e.g., "I0" -> IB0)
            (S7DataType::Byte, rest)
        };

        let offset: u16 = offset_str
            .parse()
            .with_context(|| format!("Invalid offset in '{}'", addr))?;

        Ok(S7Tag {
            area,
            db_number: 0,
            offset,
            bit_offset: 0,
            data_type,
            length: 1,
            name: None,
        })
    }

    // ── Private helpers ──

    fn next_pdu_ref(&mut self) -> u16 {
        self.pdu_ref = self.pdu_ref.wrapping_add(1);
        self.pdu_ref
    }

    pub fn build_cotp_cr(&self) -> Result<Vec<u8>> {
        // COTP CR parameters: TSAP (Transport Service Access Point)
        // Source TSAP: 0x0100 (local)
        // Destination TSAP: 0x0100 + (rack * 0x20) + slot
        let src_tsap: u16 = 0x0100;
        let dst_tsap: u16 = 0x0100 + ((self.rack as u16) << 5) + self.slot as u16;

        let mut cotp_params = Vec::new();
        // Source TSAP reference (parameter code 0xC1)
        cotp_params.push(0xC1); // parameter code: source TSAP
        cotp_params.push(0x02); // parameter length
        cotp_params.extend_from_slice(&src_tsap.to_be_bytes());
        // Destination TSAP reference (parameter code 0xC2)
        cotp_params.push(0xC2); // parameter code: destination TSAP
        cotp_params.push(0x02); // parameter length
        cotp_params.extend_from_slice(&dst_tsap.to_be_bytes());
        // TPDU size (parameter code 0xC0)
        cotp_params.push(0xC0);
        cotp_params.push(0x01);
        cotp_params.push(0x0A); // TPDU size = 0x0A (1024)

        let mut cotp = Vec::new();
        let cotp_len = 1 + 2 + 2 + 1 + cotp_params.len(); // length byte + PDU type + dst-ref + src-ref + class + params
        cotp.push(cotp_len as u8);
        cotp.push(COTP_CR);
        cotp.extend_from_slice(&0x0001u16.to_be_bytes()); // dst-ref
        cotp.extend_from_slice(&0x0001u16.to_be_bytes()); // src-ref
        cotp.push(0x00); // class
        cotp.extend_from_slice(&cotp_params);

        // Wrap in TPKT
        let mut tpkt = Vec::new();
        let total_len = 4 + cotp.len(); // TPKT header (4 bytes) + COTP
        tpkt.push(TPKT_VERSION);
        tpkt.push(0x00); // reserved
        tpkt.extend_from_slice(&(total_len as u16).to_be_bytes());
        tpkt.extend_from_slice(&cotp);

        Ok(tpkt)
    }

    async fn read_cotp_cc(&mut self) -> Result<Vec<u8>> {
        let stream = self.stream.as_mut().context("Not connected")?;

        // Read TPKT header (4 bytes)
        let mut tpkt_header = [0u8; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_exact(&mut tpkt_header),
        )
        .await
        .context("Timeout waiting for COTP TPKT header (5s)")?
        .context("Failed to read TPKT header")?;

        if tpkt_header[0] != TPKT_VERSION {
            anyhow::bail!("Invalid TPKT version: {}", tpkt_header[0]);
        }

        let packet_len = u16::from_be_bytes([tpkt_header[2], tpkt_header[3]]) as usize;
        if packet_len < 4 {
            anyhow::bail!("Invalid TPKT packet length: {}", packet_len);
        }

        let mut payload = vec![0u8; packet_len - 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_exact(&mut payload),
        )
        .await
        .context("Timeout waiting for COTP payload (5s)")?
        .context("Failed to read COTP payload")?;

        Ok(payload)
    }

    pub fn build_s7_setup_comm(&mut self) -> Vec<u8> {
        let pdu_ref = self.next_pdu_ref();

        // S7 Setup Communication parameters
        let mut params = Vec::new();
        params.push(S7_SETUP_COMM); // function code
        params.push(0x00); // reserved
        params.extend_from_slice(&MAX_PDU_SIZE.to_be_bytes()); // max AMQ calling
        params.extend_from_slice(&MAX_PDU_SIZE.to_be_bytes()); // max AMQ called
        params.extend_from_slice(&DEFAULT_PDU_SIZE.to_be_bytes()); // PDU size

        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes()); // reserved
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes()); // data length = 0
        s7.extend_from_slice(&params);

        s7
    }

    fn parse_setup_comm_response(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 17 {
            anyhow::bail!(
                "S7 Setup Communication response too short: {} bytes",
                data.len()
            );
        }

        if data[0] != S7_PROTOCOL_ID {
            anyhow::bail!("Invalid S7 protocol ID: 0x{:02X}", data[0]);
        }

        let error_code = u16::from_be_bytes([data[10], data[11]]);
        if error_code != S7_ERR_NONE {
            anyhow::bail!("S7 Setup Communication error: 0x{:04X}", error_code);
        }

        // Parse negotiated PDU size from data area
        if data.len() >= 27 {
            let negotiated_pdu = u16::from_be_bytes([data[25], data[26]]);
            if negotiated_pdu > 0 {
                self.pdu_size = negotiated_pdu.min(MAX_PDU_SIZE);
            }
        }

        Ok(())
    }

    pub fn build_read_var_request(&mut self, tags: &[S7Tag]) -> Result<Vec<u8>> {
        let pdu_ref = self.next_pdu_ref();

        // Build parameter area: function code + item count + items
        let mut params = Vec::new();
        params.push(S7_READ_VAR);
        params.push(tags.len() as u8); // item count

        // Calculate total data size for the data header
        let mut total_data_size: u16 = 0;
        let mut items = Vec::new();

        for tag in tags {
            let transport_size = tag.data_type.transport_size();
            let byte_count = if tag.data_type == S7DataType::Bool {
                1
            } else {
                tag.data_type.byte_size() * tag.length
            };
            total_data_size += byte_count;

            // S7 Item specification (12 bytes)
            let mut item = Vec::with_capacity(12);
            item.push(0x12); // variable specification: address
            item.push(0x0A); // length of following address specification
            item.push(0x10); // syntax ID: S7 any
            item.push(transport_size);
            let num_elements = if tag.data_type == S7DataType::Bool {
                1u16
            } else {
                tag.length
            };
            item.extend_from_slice(&num_elements.to_be_bytes());
            item.push(tag.db_number.to_be_bytes()[0]);
            item.push(tag.db_number.to_be_bytes()[1]);
            item.push(tag.area.code());
            // Offset: 3 bytes (offset * 8 + bit_offset for bits, offset * 8 for others)
            let offset_bits = (tag.offset as u32) * 8 + (tag.bit_offset as u32);
            item.push(((offset_bits >> 16) & 0xFF) as u8);
            item.push(((offset_bits >> 8) & 0xFF) as u8);
            item.push((offset_bits & 0xFF) as u8);

            items.extend_from_slice(&item);
        }

        params.extend_from_slice(&items);

        // Build data area: data header + data
        let mut data = Vec::new();
        data.push(0x00); // return code
        data.push(0xFF); // transport size: byte/word/dword
        let data_values = vec![0x00u8; total_data_size as usize];
        data.extend_from_slice(&total_data_size.to_be_bytes());
        data.extend_from_slice(&data_values);

        // Build S7 header + params + data
        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
        s7.extend_from_slice(&params);
        s7.extend_from_slice(&data);

        Ok(s7)
    }

    fn parse_read_var_response(&self, data: &[u8], tags: &[S7Tag]) -> Result<Vec<S7Value>> {
        if data.len() < 17 {
            anyhow::bail!("S7 Read Var response too short");
        }

        if data[0] != S7_PROTOCOL_ID {
            anyhow::bail!("Invalid S7 protocol ID: 0x{:02X}", data[0]);
        }

        let error_code = u16::from_be_bytes([data[10], data[11]]);
        if error_code != S7_ERR_NONE {
            anyhow::bail!("S7 Read Var error: 0x{:04X}", error_code);
        }

        let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
        let data_start = 10 + param_len; // after header(10) + params

        if data.len() < data_start + 1 {
            anyhow::bail!("S7 Read Var response data area too short");
        }

        let mut values = Vec::new();
        let mut offset = data_start;

        for tag in tags {
            if offset >= data.len() {
                values.push(S7Value::Byte(0));
                continue;
            }

            // Read return code
            let return_code = data[offset];
            offset += 1;

            if return_code != 0xFF {
                // Error reading this item
                values.push(S7Value::Byte(0));
                // Skip transport size and data length
                if offset + 3 <= data.len() {
                    offset += 1; // transport size
                    let item_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                    offset += 2;
                    offset += item_len;
                }
                continue;
            }

            // Transport size
            if offset + 2 > data.len() {
                values.push(S7Value::Byte(0));
                break;
            }
            offset += 1; // transport size

            let item_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            if offset + item_len > data.len() {
                values.push(S7Value::Byte(0));
                break;
            }

            let item_data = &data[offset..offset + item_len];
            let value = Self::decode_value(item_data, tag);
            values.push(value);

            offset += item_len;
        }

        Ok(values)
    }

    pub fn decode_value(data: &[u8], tag: &S7Tag) -> S7Value {
        match tag.data_type {
            S7DataType::Bool => {
                if data.is_empty() {
                    S7Value::Bool(false)
                } else {
                    S7Value::Bool((data[0] & (1 << tag.bit_offset)) != 0)
                }
            }
            S7DataType::Byte => S7Value::Byte(data.first().copied().unwrap_or(0)),
            S7DataType::Word => {
                if data.len() >= 2 {
                    S7Value::Word(u16::from_be_bytes([data[0], data[1]]))
                } else {
                    S7Value::Word(0)
                }
            }
            S7DataType::DWord => {
                if data.len() >= 4 {
                    S7Value::DWord(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
                } else {
                    S7Value::DWord(0)
                }
            }
            S7DataType::Real => {
                if data.len() >= 4 {
                    let bits = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                    S7Value::Real(f32::from_bits(bits))
                } else {
                    S7Value::Real(0.0)
                }
            }
            S7DataType::String => {
                // S7 strings: first byte is max length, second byte is actual length
                if data.len() >= 2 {
                    let actual_len = data[1] as usize;
                    if data.len() >= 2 + actual_len {
                        let s = String::from_utf8_lossy(&data[2..2 + actual_len]);
                        S7Value::String(s.to_string())
                    } else {
                        S7Value::String(String::new())
                    }
                } else {
                    S7Value::String(String::new())
                }
            }
        }
    }

    pub fn build_write_var_request(&mut self, tag: &S7Tag, value: &S7Value) -> Result<Vec<u8>> {
        let pdu_ref = self.next_pdu_ref();

        // Build parameter: function code + item count + item
        let mut params = Vec::new();
        params.push(S7_WRITE_VAR);
        params.push(0x01); // 1 item

        let transport_size = tag.data_type.transport_size();

        // Item specification (12 bytes)
        let mut item = Vec::with_capacity(12);
        item.push(0x12); // variable specification
        item.push(0x0A); // length
        item.push(0x10); // syntax ID
        item.push(transport_size);
        let num_elements = if tag.data_type == S7DataType::Bool {
            1u16
        } else {
            tag.length
        };
        item.extend_from_slice(&num_elements.to_be_bytes());
        item.push(tag.db_number.to_be_bytes()[0]);
        item.push(tag.db_number.to_be_bytes()[1]);
        item.push(tag.area.code());
        let offset_bits = (tag.offset as u32) * 8 + (tag.bit_offset as u32);
        item.push(((offset_bits >> 16) & 0xFF) as u8);
        item.push(((offset_bits >> 8) & 0xFF) as u8);
        item.push((offset_bits & 0xFF) as u8);

        params.extend_from_slice(&item);

        // Build data: data header + value
        let value_bytes = Self::encode_value(value, tag);
        let mut data = Vec::new();
        data.push(0x00); // return code (success)
        data.push(0x00); // transport size
        data.extend_from_slice(&(value_bytes.len() as u16).to_be_bytes());
        data.extend_from_slice(&value_bytes);

        // Build S7 header + params + data
        let mut s7 = Vec::new();
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&(data.len() as u16).to_be_bytes());
        s7.extend_from_slice(&params);
        s7.extend_from_slice(&data);

        Ok(s7)
    }

    pub fn encode_value(value: &S7Value, tag: &S7Tag) -> Vec<u8> {
        match value {
            S7Value::Bool(v) => {
                let byte = if *v { 1u8 << tag.bit_offset } else { 0u8 };
                vec![byte]
            }
            S7Value::Byte(v) => vec![*v],
            S7Value::Word(v) => v.to_be_bytes().to_vec(),
            S7Value::DWord(v) => v.to_be_bytes().to_vec(),
            S7Value::Real(v) => v.to_be_bytes().to_vec(),
            S7Value::String(s) => {
                let mut bytes = Vec::new();
                bytes.push(tag.length as u8); // max length
                bytes.push(s.len() as u8); // actual length
                bytes.extend_from_slice(s.as_bytes());
                bytes
            }
        }
    }

    fn parse_write_var_response(&self, data: &[u8]) -> Result<()> {
        if data.len() < 17 {
            anyhow::bail!("S7 Write Var response too short");
        }

        if data[0] != S7_PROTOCOL_ID {
            anyhow::bail!("Invalid S7 protocol ID: 0x{:02X}", data[0]);
        }

        let error_code = u16::from_be_bytes([data[10], data[11]]);
        if error_code != S7_ERR_NONE {
            anyhow::bail!("S7 Write Var error: 0x{:04X}", error_code);
        }

        Ok(())
    }

    async fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(data)
            .await
            .context("Failed to send data to S7 PLC")?;
        Ok(())
    }

    async fn send_s7(&mut self, s7_data: &[u8]) -> Result<()> {
        // Wrap S7 data in COTP Data Transfer + TPKT
        let mut cotp = Vec::new();
        cotp.push(0x02); // length (header length = 2 for DT)
        cotp.push(COTP_DT);
        cotp.push(0x80); // EOT flag + TPDU number

        // TPKT wrapper
        let total_len = 4 + cotp.len() + s7_data.len();
        let mut packet = Vec::with_capacity(total_len);
        packet.push(TPKT_VERSION);
        packet.push(0x00);
        packet.extend_from_slice(&(total_len as u16).to_be_bytes());
        packet.extend_from_slice(&cotp);
        packet.extend_from_slice(s7_data);

        self.send_raw(&packet).await
    }

    async fn read_s7_response(&mut self) -> Result<Vec<u8>> {
        let stream = self.stream.as_mut().context("Not connected")?;

        // Read TPKT header
        let mut tpkt_header = [0u8; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_exact(&mut tpkt_header),
        )
        .await
        .context("Timeout waiting for S7 TPKT header (5s)")?
        .context("Failed to read S7 TPKT header")?;

        if tpkt_header[0] != TPKT_VERSION {
            anyhow::bail!("Invalid TPKT version in S7 response: {}", tpkt_header[0]);
        }

        let packet_len = u16::from_be_bytes([tpkt_header[2], tpkt_header[3]]) as usize;
        if packet_len < 4 {
            anyhow::bail!("Invalid TPKT packet length in S7 response: {}", packet_len);
        }

        let mut payload = vec![0u8; packet_len - 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_exact(&mut payload),
        )
        .await
        .context("Timeout waiting for S7 COTP/S7 payload (5s)")?
        .context("Failed to read S7 COTP/S7 payload")?;

        // Skip COTP DT header (3 bytes: length, type=0xF0, EOT=0x80)
        if payload.len() < 3 {
            anyhow::bail!("S7 COTP payload too short");
        }

        Ok(payload[3..].to_vec())
    }
}
