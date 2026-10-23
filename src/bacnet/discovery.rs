#![allow(dead_code)]
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::models::BacnetDiscoveredDevice;

use bacnet_rs::datalink::{DataLink, DataLinkAddress, bip::BacnetIpDataLink};
use bacnet_rs::network::Npdu;
use bacnet_rs::service::{IAmRequest, UnconfirmedServiceChoice, WhoIsRequest};
use bacnet_rs::vendor::get_vendor_name;

pub async fn discover_devices(timeout_secs: u64) -> Result<Vec<BacnetDiscoveredDevice>> {
    tokio::task::spawn_blocking(move || {
        let mut datalink =
            BacnetIpDataLink::new("0.0.0.0:0").context("BACnet datalink create failed")?;

        let whois = WhoIsRequest::new();
        let mut service_data = Vec::new();
        whois.encode(&mut service_data)?;

        let mut apdu = vec![0x10, UnconfirmedServiceChoice::WhoIs as u8];
        apdu.extend_from_slice(&service_data);

        let npdu = Npdu::global_broadcast();
        let mut message = npdu.encode();
        message.extend_from_slice(&apdu);

        datalink
            .send_frame(&message, &DataLinkAddress::Broadcast)
            .context("BACnet Who-Is broadcast failed")?;

        let mut discovered = Vec::new();
        let start = std::time::Instant::now();

        while start.elapsed() < Duration::from_secs(timeout_secs) {
            match datalink.receive_frame() {
                Ok((data, source)) => {
                    let source_addr = match source {
                        DataLinkAddress::Ip(addr) => addr,
                        _ => continue,
                    };

                    if let Some(device) = parse_iam_response(&data, source_addr) {
                        discovered.push(device);
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(discovered)
    })
    .await
    .context("Task join failed")?
}

fn parse_iam_response(data: &[u8], source: SocketAddr) -> Option<BacnetDiscoveredDevice> {
    if data.len() < 4 || data[0] != 0x81 {
        return None;
    }

    let bvlc_len = ((data[2] as usize) << 8) | data[3] as usize;
    if data.len() != bvlc_len {
        return None;
    }

    let npdu_start = 4;
    let (_npdu, npdu_len) = Npdu::decode(&data[npdu_start..]).ok()?;

    let apdu_start = npdu_start + npdu_len;
    let apdu = &data[apdu_start..];

    if apdu.len() < 2 || apdu[0] != 0x10 || apdu[1] != UnconfirmedServiceChoice::IAm as u8 {
        return None;
    }

    let iam = IAmRequest::decode(&apdu[2..]).ok()?;

    Some(BacnetDiscoveredDevice {
        device_instance: iam.device_identifier.instance,
        ip: source.ip().to_string(),
        port: source.port(),
        vendor_name: Some(
            get_vendor_name(iam.vendor_identifier)
                .unwrap_or("Unknown")
                .to_string(),
        ),
        model_name: None,
        firmware_version: None,
        object_list_count: None,
    })
}
