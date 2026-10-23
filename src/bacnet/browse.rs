#![allow(dead_code)]
use std::net::SocketAddr;

use anyhow::{Context, Result};

use bacnet_rs::client::BacnetClient;
use bacnet_rs::object::ObjectType;

use super::helpers::object_type_to_string;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BacnetObjectInfo {
    pub object_type: String,
    pub object_instance: u32,
    pub object_name: Option<String>,
}

pub async fn browse_objects(
    ip: &str,
    port: u16,
    device_instance: u32,
) -> Result<Vec<BacnetObjectInfo>> {
    let addr = format!("{}:{}", ip, port)
        .parse::<SocketAddr>()
        .context("Invalid BACnet address")?;

    tokio::task::spawn_blocking(move || {
        let client =
            BacnetClient::new().map_err(|e| anyhow::anyhow!("BACnet client failed: {e}"))?;

        let obj_ids = client
            .read_object_list(addr, device_instance)
            .map_err(|e| anyhow::anyhow!("Read object list failed: {e}"))?;

        let objects = obj_ids
            .iter()
            .filter(|id| id.object_type != ObjectType::Device)
            .map(|id| BacnetObjectInfo {
                object_type: object_type_to_string(&id.object_type),
                object_instance: id.instance,
                object_name: None,
            })
            .collect();

        Ok(objects)
    })
    .await
    .context("Task join failed")?
}
