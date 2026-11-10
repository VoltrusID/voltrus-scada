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
        match values.into_iter().next() {
            Some(Ok(v)) => Ok(v),
            Some(Err(code)) => anyhow::bail!("S7 read item error: 0x{code:02X}"),
            None => anyhow::bail!("No value returned for tag"),
        }
    }

    /// The negotiated PDU size (bytes of S7 header, parameters and data).
    pub fn pdu_size(&self) -> u16 {
        self.pdu_size
    }

    /// Read tags in as few ReadVar requests as the negotiated PDU size and
    /// the item limit allow. One value per tag, in order; an item the PLC
    /// refused is `Err(return code)`.
    pub async fn read_tags(&mut self, tags: &[S7Tag]) -> Result<Vec<Result<S7Value, u8>>> {
        let mut values = Vec::with_capacity(tags.len());
        for range in plan_read_requests(tags, self.pdu_size) {
            let batch = &tags[range];
            let request = self.build_read_var_request(batch)?;
            self.send_s7(&request).await?;
            let response = self.read_s7_response().await?;
            values.extend(Self::parse_read_var_response(&response, batch)?);
        }
        Ok(values)
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
        params.extend_from_slice(&1u16.to_be_bytes()); // parallel jobs, calling
        params.extend_from_slice(&1u16.to_be_bytes()); // parallel jobs, called
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

        self.pdu_size = Self::negotiated_pdu_size(data)?;
        Ok(())
    }

    /// The PDU size in a Setup Communication ack: the 12-byte ack header, then
    /// function, reserved, parallel jobs calling, parallel jobs called and the
    /// PDU size at bytes 18-19.
    pub fn negotiated_pdu_size(data: &[u8]) -> Result<u16> {
        if data.len() < 20 || data[12] != S7_SETUP_COMM {
            anyhow::bail!("S7 Setup Communication ack without a PDU size");
        }
        let pdu = u16::from_be_bytes([data[18], data[19]]);
        if pdu < MIN_PDU_SIZE {
            anyhow::bail!("S7 PLC negotiated an unusable PDU size of {pdu}");
        }
        Ok(pdu.min(MAX_PDU_SIZE))
    }

    /// A ReadVar job: the 10-byte header and the parameters; a read request
    /// has no data part.
    pub fn build_read_var_request(&mut self, tags: &[S7Tag]) -> Result<Vec<u8>> {
        if tags.len() > MAX_READ_ITEMS {
            anyhow::bail!(
                "{} items in one S7 ReadVar (max {MAX_READ_ITEMS})",
                tags.len()
            );
        }
        let pdu_ref = self.next_pdu_ref();

        let mut params = Vec::with_capacity(2 + 12 * tags.len());
        params.push(S7_READ_VAR);
        params.push(tags.len() as u8);

        for tag in tags {
            params.push(0x12); // variable specification
            params.push(0x0A); // length of the address specification
            params.push(0x10); // syntax ID: S7ANY
            params.push(tag.data_type.transport_size());
            let num_elements = if tag.data_type == S7DataType::Bool {
                1u16
            } else {
                tag.length
            };
            params.extend_from_slice(&num_elements.to_be_bytes());
            params.extend_from_slice(&tag.db_number.to_be_bytes());
            params.push(tag.area.code());
            let offset_bits = (tag.offset as u32) * 8 + (tag.bit_offset as u32);
            params.extend_from_slice(&offset_bits.to_be_bytes()[1..]);
        }

        let mut s7 = Vec::with_capacity(10 + params.len());
        s7.push(S7_PROTOCOL_ID);
        s7.push(S7_MSG_TYPE_JOB);
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&pdu_ref.to_be_bytes());
        s7.extend_from_slice(&(params.len() as u16).to_be_bytes());
        s7.extend_from_slice(&0x0000u16.to_be_bytes());
        s7.extend_from_slice(&params);

        Ok(s7)
    }

    /// Decode a ReadVar ack (12-byte header, function and item count, then one
    /// data item per tag: return code, transport size, length, data, and a fill
    /// byte after odd-length data except the last).
    pub fn parse_read_var_response(
        data: &[u8],
        tags: &[S7Tag],
    ) -> Result<Vec<Result<S7Value, u8>>> {
        if data.len() < 14 {
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
        let data_len = u16::from_be_bytes([data[8], data[9]]) as usize;
        let data_start = 12 + param_len;
        if param_len < 2 || data.len() < data_start + data_len || data[12] != S7_READ_VAR {
            anyhow::bail!("Malformed S7 Read Var response");
        }
        let item_count = data[13] as usize;
        if item_count != tags.len() {
            anyhow::bail!(
                "S7 Read Var answered {item_count} items for {} requested",
                tags.len()
            );
        }

        let area = &data[data_start..data_start + data_len];
        let mut values = Vec::with_capacity(tags.len());
        let mut pos = 0;
        for (i, tag) in tags.iter().enumerate() {
            let header = area
                .get(pos..pos + 4)
                .context("S7 Read Var response ends inside an item header")?;
            let (return_code, transport, length) = (
                header[0],
                header[1],
                u16::from_be_bytes([header[2], header[3]]),
            );
            pos += 4;
            if return_code != ITEM_OK {
                values.push(Err(return_code));
                continue;
            }
            let byte_len = match transport {
                DATA_TS_BIT | DATA_TS_BYTE | DATA_TS_INT => (length as usize).div_ceil(8),
                _ => length as usize,
            };
            let item = area
                .get(pos..pos + byte_len)
                .context("S7 Read Var response ends inside an item")?;
            values.push(Ok(Self::decode_value(item, tag)));
            pos += byte_len;
            if byte_len % 2 == 1 && i + 1 < tags.len() {
                pos += 1;
            }
        }

        Ok(values)
    }

    pub fn decode_value(data: &[u8], tag: &S7Tag) -> S7Value {
        match tag.data_type {
            // The item addresses the bit itself; the PLC answers its state in
            // bit 0 of the only data byte.
            S7DataType::Bool => S7Value::Bool(data.first().is_some_and(|b| b & 1 == 1)),
            S7DataType::Byte => S7Value::Byte(data.first().copied().unwrap_or(0)),
            S7DataType::Word => {
                if data.len() >= 2 {
                    S7Value::Word(u16::from_be_bytes([data[0], data[1]]))
                } else {
                    S7Value::Word(0)
                }
            }
            S7DataType::Int => {
                if data.len() >= 2 {
                    S7Value::Int(i16::from_be_bytes([data[0], data[1]]))
                } else {
                    S7Value::Int(0)
                }
            }
            S7DataType::DWord => {
                if data.len() >= 4 {
                    S7Value::DWord(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
                } else {
                    S7Value::DWord(0)
                }
            }
            S7DataType::DInt => {
                if data.len() >= 4 {
                    S7Value::DInt(i32::from_be_bytes([data[0], data[1], data[2], data[3]]))
                } else {
                    S7Value::DInt(0)
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

        let value_bytes = Self::encode_value(value, tag);
        let (data_transport, data_length) = Self::data_item_header(tag, value_bytes.len());
        let mut data = Vec::new();
        data.push(0x00); // reserved (return code in responses)
        data.push(data_transport);
        data.extend_from_slice(&data_length.to_be_bytes());
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

    /// The transport size and length of a written data item. A PLC rejects a
    /// write whose header does not match the item (Snap7 `WriteArea` layout):
    /// bits and integers give their length in bits, REAL and strings in bytes.
    pub fn data_item_header(tag: &S7Tag, byte_len: usize) -> (u8, u16) {
        match tag.data_type {
            S7DataType::Bool => (DATA_TS_BIT, 1),
            S7DataType::Real => (DATA_TS_REAL, byte_len as u16),
            S7DataType::String => (DATA_TS_OCTET, byte_len as u16),
            _ => (DATA_TS_BYTE, (byte_len * 8) as u16),
        }
    }

    pub fn encode_value(value: &S7Value, tag: &S7Tag) -> Vec<u8> {
        match value {
            // The bit is addressed by the item's bit offset; the data byte
            // carries only its new state.
            S7Value::Bool(v) => vec![u8::from(*v)],
            S7Value::Byte(v) => vec![*v],
            S7Value::Word(v) => v.to_be_bytes().to_vec(),
            S7Value::Int(v) => v.to_be_bytes().to_vec(),
            S7Value::DWord(v) => v.to_be_bytes().to_vec(),
            S7Value::DInt(v) => v.to_be_bytes().to_vec(),
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

        // A job can succeed while the item fails (wrong address, type or
        // length); the first data byte after the parameters is its return code.
        let param_len = u16::from_be_bytes([data[6], data[7]]) as usize;
        if let Some(&code) = data.get(12 + param_len)
            && code != ITEM_OK
        {
            anyhow::bail!("S7 Write Var item error: 0x{:02X}", code);
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

/// Most items one ReadVar may carry (Snap7's limit, which every S7 CPU accepts).
pub const MAX_READ_ITEMS: usize = 20;

/// Bytes a tag's item takes in a ReadVar request and in its ack (data header,
/// value and fill byte).
fn item_sizes(tag: &S7Tag) -> (usize, usize) {
    let bytes = if tag.data_type == S7DataType::Bool {
        1
    } else {
        tag.data_type.byte_size() as usize * tag.length as usize
    };
    (12, 4 + bytes + bytes % 2)
}

/// Split `tags` into consecutive runs, each fitting one ReadVar request and
/// its ack into `pdu_size` bytes with at most [`MAX_READ_ITEMS`] items. A tag
/// too large for any PDU goes alone, and the PLC answers it with an error.
pub fn plan_read_requests(tags: &[S7Tag], pdu_size: u16) -> Vec<std::ops::Range<usize>> {
    let pdu = pdu_size as usize;
    let mut plans = Vec::new();
    let mut start = 0;
    let (mut request, mut response) = (12usize, 14usize);
    for (i, tag) in tags.iter().enumerate() {
        let (req, resp) = item_sizes(tag);
        let full = i - start == MAX_READ_ITEMS || request + req > pdu || response + resp > pdu;
        if i > start && full {
            plans.push(start..i);
            start = i;
            (request, response) = (12, 14);
        }
        request += req;
        response += resp;
    }
    if start < tags.len() {
        plans.push(start..tags.len());
    }
    plans
}
