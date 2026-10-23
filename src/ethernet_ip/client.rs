#![allow(dead_code)]
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::packets::*;
use super::types::*;

#[derive(Debug)]
pub struct EthernetIpClient {
    stream: Option<TcpStream>,
    url: String,
    session_handle: u32,
    connection_id: u32,
    sequence_counter: u16,
}

impl EthernetIpClient {
    pub fn new(url: &str) -> Self {
        Self {
            stream: None,
            url: url.to_string(),
            session_handle: 0,
            connection_id: 0,
            sequence_counter: 0,
        }
    }

    /// Connect to the PLC and register an EtherNet/IP session.
    pub async fn connect(&mut self) -> Result<()> {
        let stream = TcpStream::connect(&self.url)
            .await
            .with_context(|| format!("Failed to connect to {} for EtherNet/IP", self.url))?;
        stream.set_nodelay(true)?;
        self.stream = Some(stream);

        self.register_session().await?;
        Ok(())
    }

    /// Read a single CIP tag by name.
    pub async fn read_tag(&mut self, tag_name: &str) -> Result<CipValue> {
        let request = build_read_tag_request(tag_name);
        let response = self.send_rr_data(request).await?;
        parse_read_tag_response(&response)
    }

    /// Read multiple CIP tags.
    pub async fn read_tags(&mut self, tag_names: &[&str]) -> Result<Vec<CipValue>> {
        let mut values = Vec::with_capacity(tag_names.len());
        for name in tag_names {
            match self.read_tag(name).await {
                Ok(v) => values.push(v),
                Err(e) => {
                    tracing::warn!("Failed to read tag '{}': {}", name, e);
                    values.push(CipValue::Bytes(Vec::new()));
                }
            }
        }
        Ok(values)
    }

    /// Write a value to a CIP tag.
    pub async fn write_tag(&mut self, tag_name: &str, value: &CipValue) -> Result<()> {
        let request = build_write_tag_request(tag_name, value);
        let response = self.send_rr_data(request).await?;
        parse_write_tag_response(&response)
    }

    /// Browse available tags using GetAttributeAll on the Symbol class (0x6B).
    pub async fn browse_tags(&mut self) -> Result<Vec<CipTag>> {
        let mut tags = Vec::new();
        let mut instance_id: u32 = 1;

        // Iterate symbol class instances to discover tags
        loop {
            let request = build_get_attribute_all_request(SYMBOL_CLASS_ID, instance_id);
            let response = match self.send_rr_data(request).await {
                Ok(r) => r,
                Err(_) => break, // No more instances
            };

            if response.len() < 4 {
                break;
            }

            // Check CIP status
            let status = response[2];
            if status != CIP_STATUS_SUCCESS {
                break;
            }

            match parse_symbol_instance(&response[4..]) {
                Some(tag) => tags.push(tag),
                None => break,
            }

            instance_id += 1;

            // Safety limit: prevent infinite loop on malformed data
            if instance_id > 10000 {
                break;
            }
        }

        Ok(tags)
    }

    /// Disconnect and unregister the session.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            // Send UnregisterSession
            let packet = build_enip_header(CMD_UNREGISTER_SESSION, self.session_handle, 0, &[]);
            let _ = stream.write_all(&packet).await;
            let _ = stream.shutdown().await;
        }
        self.session_handle = 0;
        self.connection_id = 0;
        Ok(())
    }

    // ── Internal methods ──

    async fn register_session(&mut self) -> Result<()> {
        // Protocol version 1, options 0
        let mut body = [0u8; 4];
        body[0..2].copy_from_slice(&1u16.to_le_bytes()); // protocol version
        body[2..4].copy_from_slice(&0u16.to_le_bytes()); // options

        let packet = build_enip_header(CMD_REGISTER_SESSION, 0, 0, &body);

        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(&packet)
            .await
            .context("Write RegisterSession failed")?;
        stream.flush().await?;

        let mut header_buf = [0u8; ENIP_HEADER_SIZE];
        stream
            .read_exact(&mut header_buf)
            .await
            .context("Read RegisterSession header failed")?;

        let status = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
        if status != 0 {
            anyhow::bail!("RegisterSession failed with status 0x{:08X}", status);
        }

        self.session_handle = u32::from_le_bytes(header_buf[4..8].try_into().unwrap());

        let length = u16::from_le_bytes(header_buf[2..4].try_into().unwrap()) as usize;
        if length > 0 {
            let mut body_buf = vec![0u8; length];
            stream
                .read_exact(&mut body_buf)
                .await
                .context("Read RegisterSession body failed")?;
        }

        tracing::debug!(
            "EtherNet/IP session registered: handle=0x{:08X}",
            self.session_handle
        );
        Ok(())
    }

    pub(crate) async fn send_rr_data(&mut self, cip_payload: Vec<u8>) -> Result<Vec<u8>> {
        // Build SendRRData wrapper
        let mut rr_data = Vec::with_capacity(cip_payload.len() + 6);
        rr_data.extend_from_slice(&0u32.to_le_bytes()); // interface handle (CIP = 0)
        rr_data.extend_from_slice(&0u16.to_le_bytes()); // timeout = 0
        let item_count: u16 = 2; // connected address + data
        rr_data.extend_from_slice(&item_count.to_le_bytes());

        // Null address item (unconnected)
        rr_data.extend_from_slice(&0x0000u16.to_le_bytes()); // type ID: null
        rr_data.extend_from_slice(&0u16.to_le_bytes()); // length: 0

        // Unconnected data item
        rr_data.extend_from_slice(&0x00B2u16.to_le_bytes()); // type ID: unconnected data
        rr_data.extend_from_slice(&(cip_payload.len() as u16).to_le_bytes());
        rr_data.extend_from_slice(&cip_payload);

        let seq = self.next_sequence();
        let packet = build_enip_header(CMD_SEND_RR_DATA, self.session_handle, seq as u64, &rr_data);

        let stream = self.stream.as_mut().context("Not connected")?;
        stream
            .write_all(&packet)
            .await
            .context("Write SendRRData failed")?;
        stream.flush().await?;

        // Read response header
        let mut header_buf = [0u8; ENIP_HEADER_SIZE];
        stream
            .read_exact(&mut header_buf)
            .await
            .context("Read SendRRData header failed")?;

        let enip_status = u32::from_le_bytes(header_buf[8..12].try_into().unwrap());
        if enip_status != 0 {
            anyhow::bail!("SendRRData ENIP status 0x{:08X}", enip_status);
        }

        let length = u16::from_le_bytes(header_buf[2..4].try_into().unwrap()) as usize;
        let mut body_buf = vec![0u8; length];
        if length > 0 {
            stream
                .read_exact(&mut body_buf)
                .await
                .context("Read SendRRData body failed")?;
        }

        // Parse SendRRData response to extract CIP reply
        // Skip: interface handle (4) + timeout (2) + item count (2) + address item (4 or 6) + data item header (4)
        if body_buf.len() < 12 {
            anyhow::bail!("SendRRData response too short");
        }

        // Skip to data item
        let offset = 6; // past interface handle + timeout
        let _item_count = u16::from_le_bytes(body_buf[offset..offset + 2].try_into().unwrap());

        let mut pos = offset + 2;
        // Skip address item
        if pos + 4 <= body_buf.len() {
            let _addr_type = u16::from_le_bytes(body_buf[pos..pos + 2].try_into().unwrap());
            let addr_len =
                u16::from_le_bytes(body_buf[pos + 2..pos + 4].try_into().unwrap()) as usize;
            pos += 4 + addr_len;
        }

        // Data item
        if pos + 4 > body_buf.len() {
            anyhow::bail!("No data item in SendRRData response");
        }

        let _data_type = u16::from_le_bytes(body_buf[pos..pos + 2].try_into().unwrap());
        let data_len = u16::from_le_bytes(body_buf[pos + 2..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if pos + data_len > body_buf.len() {
            anyhow::bail!("Data item truncated in SendRRData response");
        }

        Ok(body_buf[pos..pos + data_len].to_vec())
    }

    fn next_sequence(&mut self) -> u16 {
        self.sequence_counter = self.sequence_counter.wrapping_add(1);
        self.sequence_counter
    }
}
