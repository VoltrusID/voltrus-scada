use super::*;
#[cfg(feature = "dnp3")]
use crate::dnp3::master::Dnp3MasterSession;
use crate::models::WriteResult;

impl CommandQueue {
    pub(super) async fn execute_write(&self, cmd: &crate::models::Command) -> WriteResult {
        let value: f64 = match cmd.value.parse() {
            Ok(v) => v,
            Err(_) => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some(format!("Invalid value: '{}'", cmd.value)),
                    latency_ms: 0,
                };
            }
        };

        match cmd.protocol.as_str() {
            #[cfg(feature = "modbus")]
            "modbus" => self.execute_modbus_write(cmd, value).await,
            #[cfg(feature = "s7")]
            "s7" => self.execute_s7_write(cmd, value).await,
            #[cfg(feature = "opcua")]
            "opcua" => self.execute_opcua_write(cmd, value).await,
            #[cfg(feature = "ethernet-ip")]
            // Tags created on an EtherNet/IP device have source type "enip".
            "ethernet_ip" | "enip" => self.execute_enip_write(cmd, value).await,
            // MQTT is fire-and-forget publish, not request/response. Publish to
            // the topic in source_config via the embedded broker (see
            // execute_mqtt_write); without the broker feature it fails explicitly.
            "mqtt" => self.execute_mqtt_write(cmd, value).await,
            #[cfg(feature = "dnp3")]
            "dnp3" => self.execute_dnp3_write(cmd, value).await,
            #[cfg(feature = "bacnet")]
            "bacnet" => self.execute_bacnet_write(cmd, value).await,
            _ => WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("Unsupported protocol: '{}'", cmd.protocol)),
                latency_ms: 0,
            },
        }
    }
}

fn failed(error: impl Into<String>) -> WriteResult {
    WriteResult {
        success: false,
        device_response: None,
        error: Some(error.into()),
        latency_ms: 0,
    }
}

impl CommandQueue {
    async fn command_data_source(
        &self,
        cmd: &crate::models::Command,
    ) -> Result<crate::models::DataSource, WriteResult> {
        match self.storage.get_data_source_by_id(cmd.data_source_id).await {
            Ok(Some(ds)) => Ok(ds),
            Ok(None) => Err(failed("Data source not found")),
            Err(e) => Err(failed(format!("Data source lookup failed: {e}"))),
        }
    }
}

#[cfg(feature = "modbus")]
impl CommandQueue {
    pub(super) async fn execute_modbus_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.command_data_source(cmd).await {
            Ok(ds) => ds,
            Err(r) => return r,
        };

        let device = match self.storage.get_modbus_device(&ds.device_id).await {
            Ok(Some(d)) => d,
            _ => return failed(format!("Modbus device '{}' not found", ds.device_id)),
        };
        let registers = match self.storage.list_modbus_registers(&device.id).await {
            Ok(r) => r,
            Err(e) => return failed(format!("Modbus registers of '{}': {e}", device.id)),
        };
        // The same register config the poller reads with.
        let config = device.to_device_config(&registers);

        let register = match super::resolve::modbus_register(
            &ds.key,
            &ds.device_id,
            &ds.source_config,
            &config.registers,
        ) {
            Ok(r) => r,
            Err(e) => return failed(e),
        };
        crate::modbus::write_tag(&self.modbus_pool, &config, &register, value).await
    }
}

#[cfg(feature = "s7")]
impl CommandQueue {
    pub(super) async fn execute_s7_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.command_data_source(cmd).await {
            Ok(ds) => ds,
            Err(r) => return r,
        };
        let device = match self.storage.get_s7_device(cmd.device_id).await {
            Ok(Some(d)) => d,
            _ => return failed(format!("S7 device {} not found", cmd.device_id)),
        };
        let tags = match self.storage.list_s7_tags(device.id).await {
            Ok(t) => t,
            Err(e) => return failed(format!("S7 tags of device {}: {e}", device.id)),
        };
        let tag = match super::resolve::s7_tag(&ds.key, &ds.device_id, &ds.source_config, &tags) {
            Ok(t) => t,
            Err(e) => return failed(e),
        };

        let (ip, port) = Self::parse_ip_port(&device.url, 102);
        let url = format!("{ip}:{port}");
        crate::s7::write_tag(&url, device.rack, device.slot, &tag, value).await
    }
}

#[cfg(feature = "opcua")]
impl CommandQueue {
    pub(super) async fn execute_opcua_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.command_data_source(cmd).await {
            Ok(ds) => ds,
            Err(r) => return r,
        };
        let device = match self.storage.get_opcua_device(cmd.device_id).await {
            Ok(Some(d)) => d,
            _ => return failed(format!("OPC UA device {} not found", cmd.device_id)),
        };
        let nodes = match self.storage.list_opcua_nodes(device.id).await {
            Ok(n) => n,
            Err(e) => return failed(format!("OPC UA nodes of device {}: {e}", device.id)),
        };
        let node_id = match super::resolve::opcua_node_id(
            &ds.key,
            &ds.device_id,
            &ds.source_config,
            &nodes,
        ) {
            Ok(id) => id,
            Err(e) => return failed(e),
        };

        // The poller's session, keyed like its cache entries.
        crate::opcua::write_tag(&format!("opcua_{}", device.id), &node_id, value).await
    }
}

#[cfg(feature = "ethernet-ip")]
impl CommandQueue {
    pub(super) async fn execute_enip_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.command_data_source(cmd).await {
            Ok(ds) => ds,
            Err(r) => return r,
        };
        let device = match self.storage.get_enip_device(cmd.device_id).await {
            Ok(Some(d)) => d,
            _ => return failed(format!("EtherNet/IP device {} not found", cmd.device_id)),
        };
        let tags = match self.storage.list_enip_tags(device.id).await {
            Ok(t) => t,
            Err(e) => return failed(format!("EtherNet/IP tags of device {}: {e}", device.id)),
        };
        let (tag_name, data_type) =
            match super::resolve::enip_tag(&ds.key, &ds.device_id, &ds.source_config, &tags) {
                Ok(t) => t,
                Err(e) => return failed(e),
            };

        let (ip, port) = Self::parse_ip_port(&device.url, 44818);
        crate::ethernet_ip::write_tag(&format!("{ip}:{port}"), &tag_name, &data_type, value).await
    }
}

#[cfg(feature = "dnp3")]
impl CommandQueue {
    pub(super) async fn execute_dnp3_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.storage.get_data_source_by_id(cmd.data_source_id).await {
            Ok(Some(ds)) => ds,
            _ => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some("Data source not found".to_string()),
                    latency_ms: 0,
                };
            }
        };

        let point_index = match Self::parse_dnp3_point_index(&ds.source_config) {
            Some(idx) => idx,
            None => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some("Could not parse DNP3 point index from source_config".to_string()),
                    latency_ms: 0,
                };
            }
        };

        let device = match self.storage.get_dnp3_device(cmd.device_id).await {
            Ok(Some(d)) => d,
            _ => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some(format!("DNP3 device {} not found", cmd.device_id)),
                    latency_ms: 0,
                };
            }
        };

        let on = value != 0.0;
        match Dnp3MasterSession::connect(
            &device.ip,
            device.port,
            device.master_address,
            device.outstation_address,
        )
        .await
        {
            Ok(mut session) => session
                .direct_operate_binary(point_index, on)
                .await
                .unwrap_or_else(|e| WriteResult {
                    success: false,
                    device_response: None,
                    error: Some(format!("DNP3 write failed: {e}")),
                    latency_ms: 0,
                }),
            Err(e) => WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("DNP3 connect failed: {e}")),
                latency_ms: 0,
            },
        }
    }
}

#[cfg(feature = "bacnet")]
impl CommandQueue {
    pub(super) async fn execute_bacnet_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.command_data_source(cmd).await {
            Ok(ds) => ds,
            Err(r) => return r,
        };
        let device = match self.storage.get_bacnet_device(cmd.device_id).await {
            Ok(Some(d)) => d,
            _ => return failed(format!("BACnet device {} not found", cmd.device_id)),
        };
        let objects = match self.storage.list_bacnet_objects(device.id).await {
            Ok(o) => o,
            Err(e) => return failed(format!("BACnet objects of device {}: {e}", device.id)),
        };
        let point =
            match super::resolve::bacnet_point(&ds.key, &ds.device_id, &ds.source_config, &objects)
            {
                Ok(p) => p,
                Err(e) => return failed(e),
            };

        crate::bacnet::write_property(
            &device.ip,
            device.port,
            &point.object_type,
            point.object_instance,
            &point.property,
            value,
            point.priority,
        )
        .await
    }
}

#[cfg(feature = "mqtt-broker")]
impl CommandQueue {
    /// Publish the command value to the MQTT topic in the data source's
    /// `source_config` (`{"topic": "..."}`). MQTT has no device ACK, so success
    /// means "the broker accepted the publish", not "the device acted on it" —
    /// the closest faithful analogue of the request/response protocols' ACK.
    pub(super) async fn execute_mqtt_write(
        &self,
        cmd: &crate::models::Command,
        value: f64,
    ) -> WriteResult {
        let ds = match self.storage.get_data_source_by_id(cmd.data_source_id).await {
            Ok(Some(ds)) => ds,
            _ => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some("Data source not found".to_string()),
                    latency_ms: 0,
                };
            }
        };

        let topic = match Self::parse_mqtt_topic(&ds.source_config) {
            Some(t) => t,
            None => {
                return WriteResult {
                    success: false,
                    device_response: None,
                    error: Some("Could not parse MQTT topic from source_config".to_string()),
                    latency_ms: 0,
                };
            }
        };

        let payload = Self::mqtt_payload(&ds.source_config, value);
        let started = std::time::Instant::now();
        let broker = self.mqtt_broker.read().await;
        match broker.as_ref() {
            Some(b) => match b.publish_sync(&topic, payload.as_bytes()) {
                Ok(_) => WriteResult {
                    success: true,
                    device_response: Some(format!("published {} bytes to {topic}", payload.len())),
                    error: None,
                    latency_ms: started.elapsed().as_millis() as u64,
                },
                Err(e) => WriteResult {
                    success: false,
                    device_response: None,
                    error: Some(format!("MQTT publish failed: {e}")),
                    latency_ms: started.elapsed().as_millis() as u64,
                },
            },
            None => WriteResult {
                success: false,
                device_response: None,
                error: Some("MQTT broker is not running; cannot publish command".to_string()),
                latency_ms: 0,
            },
        }
    }
}

#[cfg(not(feature = "mqtt-broker"))]
impl CommandQueue {
    pub(super) async fn execute_mqtt_write(
        &self,
        _cmd: &crate::models::Command,
        _value: f64,
    ) -> WriteResult {
        WriteResult {
            success: false,
            device_response: None,
            error: Some("MQTT broker feature is not enabled in this build".to_string()),
            latency_ms: 0,
        }
    }
}
