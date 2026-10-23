use anyhow::{Context, Result};
use std::collections::HashMap;
use std::time::Duration;

use super::types::PlcRuntimeState;

/// Lightweight client for a Voltrus PLC Runtime.
///
/// Supports two transport modes:
/// - "grpc" — gRPC over HTTP/2 (Linux runtime)
/// - "websocket" — HTTP REST + WebSocket (ESP32 runtime)
///
/// For now, implements the HTTP transport (ESP32) since it requires fewer deps.
/// gRPC support will be added when the mt-grpc crate is integrated.
pub struct PlcRuntimeClient {
    url: String,
    transport: String,
    http: reqwest::Client,
}

impl PlcRuntimeClient {
    pub fn new(url: &str, transport: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            url: url.to_string(),
            transport: transport.to_string(),
            http,
        }
    }

    /// Connect and verify the runtime is reachable.
    pub async fn connect(&self) -> Result<()> {
        match self.transport.as_str() {
            "websocket" => self.ws_health_check().await,
            "grpc" => self.tcp_health_check().await,
            _ => anyhow::bail!("Unknown transport: {}", self.transport),
        }
    }

    /// Read all watched variable values from the runtime.
    pub async fn read_variables(&self) -> Result<HashMap<String, f64>> {
        match self.transport.as_str() {
            "websocket" => self.ws_read_variables().await,
            "grpc" => {
                // TODO: gRPC WatchVariables streaming via mt-grpc
                Err(anyhow::anyhow!("gRPC transport not yet implemented"))
            }
            _ => anyhow::bail!("Unknown transport: {}", self.transport),
        }
    }

    /// Write a value to a variable on the runtime.
    pub async fn write_variable(
        &self,
        name: &str,
        value: f64,
    ) -> Result<crate::models::WriteResult> {
        match self.transport.as_str() {
            "websocket" => self.ws_write_variable(name, value).await,
            "grpc" => Err(anyhow::anyhow!("gRPC transport not yet implemented")),
            _ => anyhow::bail!("Unknown transport: {}", self.transport),
        }
    }

    /// Get the current runtime state.
    pub async fn get_state(&self) -> Result<PlcRuntimeState> {
        match self.transport.as_str() {
            "websocket" => self.ws_get_state().await,
            "grpc" => Err(anyhow::anyhow!("gRPC transport not yet implemented")),
            _ => anyhow::bail!("Unknown transport: {}", self.transport),
        }
    }

    // ── HTTP transport (ESP32) ──

    async fn ws_health_check(&self) -> Result<()> {
        let health_url = format!("http://{}/api/health", self.url);
        let resp = self
            .http
            .get(&health_url)
            .send()
            .await
            .context("PLC runtime health check failed")?;
        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("PLC runtime returned status {}", resp.status())
        }
    }

    async fn ws_read_variables(&self) -> Result<HashMap<String, f64>> {
        let vars_url = format!("http://{}/api/variables", self.url);
        let resp = self
            .http
            .get(&vars_url)
            .send()
            .await
            .context("Failed to read PLC variables")?;

        if !resp.status().is_success() {
            anyhow::bail!("PLC returned status {}", resp.status());
        }

        let variables: HashMap<String, f64> =
            resp.json().await.context("Failed to parse PLC variables")?;
        Ok(variables)
    }

    async fn ws_write_variable(
        &self,
        name: &str,
        value: f64,
    ) -> Result<crate::models::WriteResult> {
        let start = std::time::Instant::now();

        let write_url = format!("http://{}/api/variables/{}", self.url, name);
        let resp = self
            .http
            .post(&write_url)
            .json(&serde_json::json!({ "value": value }))
            .send()
            .await
            .context("Failed to write PLC variable")?;

        let latency_ms = start.elapsed().as_millis() as u64;

        if resp.status().is_success() {
            Ok(crate::models::WriteResult {
                success: true,
                device_response: Some(format!("{} = {}", name, value)),
                error: None,
                latency_ms,
            })
        } else {
            Ok(crate::models::WriteResult {
                success: false,
                device_response: None,
                error: Some(format!("PLC returned status {}", resp.status())),
                latency_ms,
            })
        }
    }

    async fn ws_get_state(&self) -> Result<PlcRuntimeState> {
        let state_url = format!("http://{}/api/state", self.url);
        let resp = self
            .http
            .get(&state_url)
            .send()
            .await
            .context("Failed to get PLC state")?;

        if !resp.status().is_success() {
            return Ok(PlcRuntimeState::Error);
        }

        let state: serde_json::Value = resp.json().await.unwrap_or_default();
        match state
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("stopped")
        {
            "running" => Ok(PlcRuntimeState::Running),
            "paused" => Ok(PlcRuntimeState::Paused),
            "error" => Ok(PlcRuntimeState::Error),
            _ => Ok(PlcRuntimeState::Stopped),
        }
    }

    // ── gRPC transport ──

    async fn tcp_health_check(&self) -> Result<()> {
        tokio::net::TcpStream::connect(&self.url)
            .await
            .context(format!("Cannot connect to PLC runtime at {}", self.url))?;
        Ok(())
    }
}
