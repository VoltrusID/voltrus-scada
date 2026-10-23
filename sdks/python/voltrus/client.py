"""Voltrus SCADA Python SDK client.

Hand-written, typed, urllib3-only. Method names follow the REST surface:
``GET /api/v1/alarms`` -> ``list_alarms()``, ``POST /api/v1/alarms/{id}/acknowledge``
-> ``acknowledge_alarm()``. Protocol-specific config bodies are plain dicts
(they differ per protocol); read-side results are typed models where the
server shape is stable.
"""

import json
from dataclasses import fields as dataclass_fields
from typing import Any, Dict, List, Optional, Union

import urllib3

from .models import (
    Alarm,
    AlarmSummary,
    DeviceSnapshot,
    HistoryResponse,
    HistorySeries,
    LiveValue,
    OeeMetrics,
    Screen,
    WriteResult,
)


class VoltrusError(Exception):
    """Raised for any non-2xx Voltrus API response."""

    def __init__(self, status: int, message: str, body: str = "") -> None:
        super().__init__(f"Voltrus API error {status}: {message}")
        self.status = status
        self.message = message
        self.body = body


def _model(cls: Any, data: Dict[str, Any]) -> Any:
    """Build a dataclass from a server row, ignoring unknown keys."""
    names = {f.name for f in dataclass_fields(cls)}
    return cls(**{k: v for k, v in data.items() if k in names})


class VoltrusClient:
    """Official Python SDK for Voltrus SCADA.

    Two auth modes:

    - API key (machine-to-machine, read-oriented): ``VoltrusClient(url, api_key=...)``
    - Login session (full read/write as the user): ``client.login(user, password)``

    Usage:
        client = VoltrusClient("http://localhost:3000", api_key="vt_...")
        values = client.get_live_data(screen_id="overview")
        client.login("admin", "secret")
        client.acknowledge_alarm(42)
    """

    def __init__(
        self,
        base_url: str,
        api_key: Optional[str] = None,
        token: Optional[str] = None,
        timeout: int = 30,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout
        self._cookies: Dict[str, str] = {}

        headers: Dict[str, str] = {"Content-Type": "application/json"}
        if api_key:
            headers["X-API-Key"] = api_key
        if token:
            headers["Authorization"] = f"Bearer {token}"

        self._http = urllib3.PoolManager(
            headers=headers, timeout=urllib3.Timeout(total=timeout)
        )

    # ── Plumbing ────────────────────────────────────────────

    def _url(self, path: str) -> str:
        return f"{self._base_url}{path}"

    def _store_cookies(self, resp: urllib3.HTTPResponse) -> None:
        for header in resp.headers.getlist("Set-Cookie"):
            first = header.split(";", 1)[0]
            name, _, value = first.partition("=")
            if name and value:
                self._cookies[name.strip()] = value.strip()

    def _request(
        self,
        method: str,
        path: str,
        body: Optional[Dict[str, Any]] = None,
        params: Optional[Dict[str, Any]] = None,
        raw: bool = False,
    ) -> Any:
        kwargs: Dict[str, Any] = {}
        if params:
            kwargs["fields"] = {k: v for k, v in params.items() if v is not None}
        if body is not None:
            kwargs["body"] = json.dumps(body)
        headers: Dict[str, str] = {}
        if body is not None:
            headers["Content-Type"] = "application/json"
        if self._cookies:
            headers["Cookie"] = "; ".join(f"{k}={v}" for k, v in self._cookies.items())
        if headers:
            kwargs["headers"] = headers

        resp = self._http.request(method, self._url(path), **kwargs)
        if resp.status >= 400:
            text = resp.data.decode("utf-8", "replace")
            try:
                message = json.loads(text).get("error", text)
            except (ValueError, AttributeError):
                message = text
            raise VoltrusError(resp.status, message, text)

        self._store_cookies(resp)

        if raw:
            return resp.data.decode("utf-8")
        if resp.status == 204 or not resp.data:
            return None

        return json.loads(resp.data.decode("utf-8"))

    def request(
        self,
        method: str,
        path: str,
        body: Optional[Dict[str, Any]] = None,
        params: Optional[Dict[str, Any]] = None,
    ) -> Any:
        """Call any Voltrus endpoint directly. Escape hatch for endpoints
        without a dedicated method yet."""
        return self._request(method.upper(), path, body=body, params=params)

    # ── Auth ────────────────────────────────────────────────

    def login(self, username: str, password: str) -> Dict[str, Any]:
        """Login as a user and keep the session cookies for later calls.

        The server returns the access/refresh tokens as httpOnly cookies,
        not in the body — this stores them transparently.
        """
        data = self._request(
            "POST", "/api/v1/auth/login", body={"username": username, "password": password}
        )
        return data

    def refresh(self) -> Dict[str, Any]:
        """Rotate the session using the stored refresh cookie."""
        return self._request("POST", "/api/v1/auth/refresh")

    def logout(self) -> None:
        """Revoke the server-side session and drop stored cookies."""
        self._request("POST", "/api/v1/auth/logout")
        self._cookies.clear()

    def me(self) -> Dict[str, Any]:
        """Who am I? Returns {'authenticated': bool, 'username', 'role'}."""
        return self._request("GET", "/api/v1/auth/me")

    def change_password(self, old_password: str, new_password: str) -> Dict[str, Any]:
        """Change the logged-in user's password. Required on first login
        when :meth:`login` returns ``must_change_password: True``."""
        return self._request(
            "POST",
            "/api/v1/auth/change-password",
            body={"old_password": old_password, "new_password": new_password},
        )

    # ── System ──────────────────────────────────────────────

    def health(self) -> Dict[str, Any]:
        """Health check."""
        return self._request("GET", "/api/v1/health")

    def get_system_info(self) -> Dict[str, Any]:
        """System info (CPU, memory, uptime)."""
        return self._request("GET", "/api/v1/system")

    def get_version(self) -> str:
        """Voltrus server version."""
        data = self._request("GET", "/api/v1/version")
        return data.get("version", "unknown")

    def get_features(self) -> Dict[str, Any]:
        """Enabled compile-time features (protocols etc.)."""
        return self._request("GET", "/api/v1/system/features")

    def get_license_status(self) -> Dict[str, Any]:
        """Edition tier ("community", "integrator", "enterprise") and licence status."""
        return self._request("GET", "/api/license/status")

    # ── Live data & tags ────────────────────────────────────

    def get_live_data(
        self, screen_id: Optional[str] = None, device_id: Optional[str] = None
    ) -> List[LiveValue]:
        """Current values for all data sources, optionally scoped to one
        screen or device."""
        params: Dict[str, Any] = {}
        if screen_id:
            params["screen"] = screen_id
        if device_id:
            params["device"] = device_id
        data = self._request("GET", "/api/v1/data", params=params)
        return [_model(LiveValue, v) for v in data]

    def get_live_snapshots(self) -> List[DeviceSnapshot]:
        """Latest cached metrics for every device."""
        data = self._request("GET", "/api/v1/live")
        return [_model(DeviceSnapshot, d) for d in data.get("devices", [])]

    def get_history(
        self,
        screen_id: Optional[str] = None,
        device_id: Optional[str] = None,
        sensor: Optional[str] = None,
        range: str = "30m",
        detail: Optional[str] = None,
        agg: Optional[str] = None,
    ) -> HistoryResponse:
        """Historical series for a screen, device, or single sensor.

        ``range`` examples: '15m', '30m', '1h', '24h'. ``agg``: 'avg',
        'minmax' or 'sampling'.
        """
        params: Dict[str, Any] = {"range": range}
        if screen_id:
            params["screen"] = screen_id
        if device_id:
            params["device"] = device_id
        if sensor:
            params["sensor"] = sensor
        if detail:
            params["detail"] = detail
        if agg:
            params["agg"] = agg
        data = self._request("GET", "/api/v1/history", params=params)
        return HistoryResponse(
            sensors=data.get("sensors", []),
            data={
                key: _model(HistorySeries, series)
                for key, series in data.get("data", {}).items()
            },
            range=data.get("range", range),
            interval=data.get("interval", 0),
        )

    def export_data(
        self,
        from_ts: Optional[str] = None,
        to_ts: Optional[str] = None,
        device: Optional[str] = None,
        metric: Optional[str] = None,
        aggregation: Optional[str] = None,
        interval: Optional[str] = None,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        format: Optional[str] = None,
    ) -> Union[Dict[str, Any], str]:
        """Export telemetry rows. ``from_ts``/``to_ts`` take ISO 8601 or unix
        seconds. Returns a dict, or CSV text when ``format='csv'``."""
        params: Dict[str, Any] = {
            "from": from_ts,
            "to": to_ts,
            "device": device,
            "metric": metric,
            "aggregation": aggregation,
            "interval": interval,
            "limit": limit,
            "offset": offset,
            "format": format,
        }
        if format == "csv":
            return self._request("GET", "/api/v1/data/export", params=params, raw=True)
        return self._request("GET", "/api/v1/data/export", params=params)

    def export_csv(self, sensor: str, range: str = "30m") -> str:
        """CSV export for one sensor, e.g. export_csv('tank1.level', '1h')."""
        return self._request(
            "GET", "/api/v1/export/csv", params={"sensor": sensor, "range": range}, raw=True
        )

    def get_tag_tree(self, search: Optional[str] = None) -> List[Dict[str, Any]]:
        """Hierarchical ISA-95 style tag tree."""
        data = self._request("GET", "/api/v1/tags/tree", params={"search": search})
        return data.get("tree", [])

    def list_data_sources(self, screen_id: Optional[str] = None) -> List[Dict[str, Any]]:
        """All configured data sources (tags), optionally for one screen."""
        return self._request(
            "GET", "/api/v1/data-sources", params={"screen": screen_id}
        )

    def update_tag_path(self, tag_id: int, path: Optional[str]) -> None:
        """Set the UNS tree path of a tag ('Site/Area/Unit'), None to clear."""
        self._request("PUT", f"/api/v1/tags/{tag_id}/path", body={"path": path})

    # ── Alarms ──────────────────────────────────────────────

    def list_alarms(
        self,
        status: Optional[str] = None,
        priority: Optional[str] = None,
        state: Optional[str] = None,
        shelved: Optional[bool] = None,
        limit: Optional[int] = None,
    ) -> List[Alarm]:
        """List alarms. ``status``: 'active'|'acknowledged'|'all'."""
        params: Dict[str, Any] = {
            "status": status,
            "priority": priority,
            "state": state,
            "shelved": shelved,
            "limit": limit,
        }
        data = self._request("GET", "/api/v1/alarms", params=params)
        return [_model(Alarm, a) for a in data]

    def get_active_alarm_count(self) -> int:
        """Number of currently active (unacknowledged) alarms."""
        data = self._request("GET", "/api/v1/alarms/active/count")
        return data.get("count", 0)

    def get_alarm_summary(self) -> AlarmSummary:
        """Alarm counts by state and severity."""
        return _model(AlarmSummary, self._request("GET", "/api/v1/alarms/summary"))

    def acknowledge_alarm(self, alarm_id: int) -> None:
        """Acknowledge one alarm."""
        self._request("POST", f"/api/v1/alarms/{alarm_id}/acknowledge")

    def acknowledge_all_alarms(self) -> None:
        """Acknowledge every active alarm."""
        self._request("POST", "/api/v1/alarms")

    def shelf_alarm(self, alarm_id: int, duration_secs: int, reason: str) -> None:
        """Shelve an alarm (suppress notifications) for a duration."""
        self._request(
            "POST",
            f"/api/v1/alarms/{alarm_id}/shelf",
            body={"duration_secs": duration_secs, "reason": reason},
        )

    def unshelf_alarm(self, alarm_id: int) -> None:
        """Remove shelving from an alarm."""
        self._request("POST", f"/api/v1/alarms/{alarm_id}/unshelf")

    def list_alarm_audit(
        self,
        alarm_id: Optional[int] = None,
        from_ts: Optional[int] = None,
        to_ts: Optional[int] = None,
        limit: Optional[int] = None,
    ) -> List[Dict[str, Any]]:
        """Alarm audit trail (ack/shelve/unshelf events)."""
        params: Dict[str, Any] = {
            "alarm_id": alarm_id,
            "from": from_ts,
            "to": to_ts,
            "limit": limit,
        }
        return self._request("GET", "/api/v1/alarms/audit", params=params)

    def get_alarm_analytics(
        self, from_ts: Optional[int] = None, to_ts: Optional[int] = None
    ) -> Dict[str, Any]:
        """Alarm analytics over a time window."""
        return self._request(
            "GET", "/api/v1/alarms/analytics", params={"from": from_ts, "to": to_ts}
        )

    # ── Commands (writes) ───────────────────────────────────

    def write_tag(
        self,
        tag_id: int,
        value: Union[float, str, bool],
        reason: Optional[str] = None,
        scheduled_at: Optional[int] = None,
    ) -> WriteResult:
        """Write a value to a tag. Returns a command to poll with
        get_command_status(). Writes require a login session with write
        access (API keys are read-oriented)."""
        body: Dict[str, Any] = {"value": value}
        if reason is not None:
            body["reason"] = reason
        if scheduled_at is not None:
            body["scheduled_at"] = scheduled_at
        data = self._request("POST", f"/api/v1/tags/{tag_id}/write", body=body)
        return _model(WriteResult, data)

    def get_command_status(self, command_id: str) -> Dict[str, Any]:
        """Poll a write command by id."""
        return self._request("GET", f"/api/v1/commands/{command_id}")

    def list_commands(
        self,
        tag_id: Optional[str] = None,
        status: Optional[str] = None,
        device_id: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Write command history, filterable by tag, status, or device."""
        return self._request(
            "GET",
            "/api/v1/commands",
            params={"tag": tag_id, "status": status, "device": device_id},
        )

    def cancel_command(self, command_id: str) -> None:
        """Cancel a pending write command."""
        self._request("POST", f"/api/v1/commands/{command_id}/cancel")

    def get_command_audit(self, command_id: str) -> List[Dict[str, Any]]:
        """Audit entries for a write command."""
        return self._request("GET", f"/api/v1/commands/{command_id}/audit")

    def update_write_policy(
        self,
        tag_id: int,
        writable: Optional[bool] = None,
        write_min: Optional[Optional[float]] = None,
        write_max: Optional[Optional[float]] = None,
        write_max_rate: Optional[Optional[int]] = None,
        write_requires_reason: Optional[bool] = None,
        write_deadband: Optional[Optional[float]] = None,
    ) -> None:
        """Update the write policy of a data source. Pass None for fields
        you do not want to change; a nested None clears a limit."""
        body: Dict[str, Any] = {}
        if writable is not None:
            body["writable"] = writable
        if write_min is not None:
            body["write_min"] = write_min
        if write_max is not None:
            body["write_max"] = write_max
        if write_max_rate is not None:
            body["write_max_rate"] = write_max_rate
        if write_requires_reason is not None:
            body["write_requires_reason"] = write_requires_reason
        if write_deadband is not None:
            body["write_deadband"] = write_deadband
        self._request("PATCH", f"/api/v1/data-sources/{tag_id}/write-policy", body=body)

    # ── Modbus devices ──────────────────────────────────────

    def list_modbus_devices(self) -> List[Dict[str, Any]]:
        """List Modbus devices."""
        return self._request("GET", "/api/v1/modbus-devices")

    def get_modbus_device(self, device_id: str) -> Dict[str, Any]:
        """Get one Modbus device."""
        return self._request("GET", f"/api/v1/modbus-devices/{device_id}")

    def create_modbus_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create a Modbus device. See the docs for the config schema."""
        return self._request("POST", "/api/v1/modbus-devices", body=config)

    def update_modbus_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update a Modbus device."""
        return self._request("PUT", f"/api/v1/modbus-devices/{device_id}", body=config)

    def delete_modbus_device(self, device_id: str) -> None:
        """Delete a Modbus device."""
        self._request("DELETE", f"/api/v1/modbus-devices/{device_id}")

    def get_modbus_poller_statuses(self) -> Dict[str, Any]:
        """Per-device poller status (online, last scan, errors)."""
        return self._request("GET", "/api/v1/modbus-devices/status")

    def scan_modbus_registers(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Scan registers of one Modbus device."""
        return self._request("POST", "/api/v1/modbus-devices/scan", body=config)

    def scan_modbus_network(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Scan a network range for Modbus units."""
        return self._request("POST", "/api/v1/modbus-devices/scan-network", body=config)

    def replace_modbus_registers(self, device_id: str, registers: List[Dict[str, Any]]) -> None:
        """Replace the whole register map of a Modbus device."""
        self._request(
            "PUT", f"/api/v1/modbus-devices/{device_id}/registers", body={"registers": registers}
        )

    def restart_modbus_poller(self, device_id: str) -> None:
        """Restart the poller of a Modbus device."""
        self._request("POST", f"/api/v1/modbus-devices/{device_id}/restart")

    # ── OPC-UA devices ──────────────────────────────────────

    def list_opcua_devices(self) -> List[Dict[str, Any]]:
        """List OPC-UA devices."""
        return self._request("GET", "/api/v1/opcua/devices")

    def create_opcua_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create an OPC-UA device."""
        return self._request("POST", "/api/v1/opcua/devices", body=config)

    def update_opcua_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update an OPC-UA device."""
        return self._request("PUT", f"/api/v1/opcua/devices/{device_id}", body=config)

    def delete_opcua_device(self, device_id: str) -> None:
        """Delete an OPC-UA device."""
        self._request("DELETE", f"/api/v1/opcua/devices/{device_id}")

    def list_opcua_nodes(self, device_id: str) -> List[Dict[str, Any]]:
        """Node subscriptions of an OPC-UA device."""
        return self._request("GET", f"/api/v1/opcua/devices/{device_id}/nodes")

    def add_opcua_node(self, device_id: str, node: Dict[str, Any]) -> None:
        """Subscribe an OPC-UA node (NodeId)."""
        self._request("POST", f"/api/v1/opcua/devices/{device_id}/nodes", body=node)

    def remove_opcua_node(self, device_id: str, node_id: str) -> None:
        """Unsubscribe an OPC-UA node."""
        self._request("DELETE", f"/api/v1/opcua/devices/{device_id}/nodes/{node_id}")

    def browse_opcua_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Browse the address space of an OPC-UA server."""
        return self._request("POST", f"/api/v1/opcua/devices/{config.get('id')}/browse", body=config)

    def start_opcua_device(self, device_id: str) -> None:
        """Start polling an OPC-UA device."""
        self._request("POST", f"/api/v1/opcua/devices/{device_id}/start")

    def stop_opcua_device(self, device_id: str) -> None:
        """Stop polling an OPC-UA device."""
        self._request("POST", f"/api/v1/opcua/devices/{device_id}/stop")

    # ── S7 devices ──────────────────────────────────────────

    def list_s7_devices(self) -> List[Dict[str, Any]]:
        """List S7 devices."""
        return self._request("GET", "/api/v1/s7/devices")

    def create_s7_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create an S7 device."""
        return self._request("POST", "/api/v1/s7/devices", body=config)

    def update_s7_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update an S7 device."""
        return self._request("PUT", f"/api/v1/s7/devices/{device_id}", body=config)

    def delete_s7_device(self, device_id: str) -> None:
        """Delete an S7 device."""
        self._request("DELETE", f"/api/v1/s7/devices/{device_id}")

    def list_s7_tags(self, device_id: str) -> List[Dict[str, Any]]:
        """Tag subscriptions of an S7 device."""
        return self._request("GET", f"/api/v1/s7/devices/{device_id}/tags")

    def add_s7_tag(self, device_id: str, tag: Dict[str, Any]) -> None:
        """Subscribe an S7 tag (DB, offset, type)."""
        self._request("POST", f"/api/v1/s7/devices/{device_id}/tags", body=tag)

    def remove_s7_tag(self, device_id: str, tag_id: str) -> None:
        """Unsubscribe an S7 tag."""
        self._request("DELETE", f"/api/v1/s7/devices/{device_id}/tags/{tag_id}")

    def browse_s7_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Browse PLC blocks/data of an S7 device."""
        return self._request("POST", f"/api/v1/s7/devices/{device_id}/browse", body=config)

    def start_s7_device(self, device_id: str) -> None:
        """Start polling an S7 device."""
        self._request("POST", f"/api/v1/s7/devices/{device_id}/start")

    def stop_s7_device(self, device_id: str) -> None:
        """Stop polling an S7 device."""
        self._request("POST", f"/api/v1/s7/devices/{device_id}/stop")

    # ── EtherNet/IP devices ─────────────────────────────────

    def list_enip_devices(self) -> List[Dict[str, Any]]:
        """List EtherNet/IP devices."""
        return self._request("GET", "/api/v1/enip/devices")

    def create_enip_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create an EtherNet/IP device."""
        return self._request("POST", "/api/v1/enip/devices", body=config)

    def update_enip_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update an EtherNet/IP device."""
        return self._request("PUT", f"/api/v1/enip/devices/{device_id}", body=config)

    def delete_enip_device(self, device_id: str) -> None:
        """Delete an EtherNet/IP device."""
        self._request("DELETE", f"/api/v1/enip/devices/{device_id}")

    def list_enip_tags(self, device_id: str) -> List[Dict[str, Any]]:
        """Tag subscriptions of an EtherNet/IP device."""
        return self._request("GET", f"/api/v1/enip/devices/{device_id}/tags")

    def add_enip_tag(self, device_id: str, tag: Dict[str, Any]) -> None:
        """Subscribe an EtherNet/IP tag."""
        self._request("POST", f"/api/v1/enip/devices/{device_id}/tags", body=tag)

    def remove_enip_tag(self, device_id: str, tag_id: str) -> None:
        """Unsubscribe an EtherNet/IP tag."""
        self._request("DELETE", f"/api/v1/enip/devices/{device_id}/tags/{tag_id}")

    def browse_enip_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Browse tags of an EtherNet/IP device."""
        return self._request("POST", f"/api/v1/enip/devices/{device_id}/browse", body=config)

    def start_enip_device(self, device_id: str) -> None:
        """Start polling an EtherNet/IP device."""
        self._request("POST", f"/api/v1/enip/devices/{device_id}/start")

    def stop_enip_device(self, device_id: str) -> None:
        """Stop polling an EtherNet/IP device."""
        self._request("POST", f"/api/v1/enip/devices/{device_id}/stop")

    # ── DNP3 devices ────────────────────────────────────────

    def list_dnp3_devices(self) -> List[Dict[str, Any]]:
        """List DNP3 devices."""
        return self._request("GET", "/api/v1/dnp3/devices")

    def get_dnp3_device(self, device_id: str) -> Dict[str, Any]:
        """Get one DNP3 device."""
        return self._request("GET", f"/api/v1/dnp3/devices/{device_id}")

    def create_dnp3_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create a DNP3 device."""
        return self._request("POST", "/api/v1/dnp3/devices", body=config)

    def update_dnp3_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update a DNP3 device."""
        return self._request("PUT", f"/api/v1/dnp3/devices/{device_id}", body=config)

    def delete_dnp3_device(self, device_id: str) -> None:
        """Delete a DNP3 device."""
        self._request("DELETE", f"/api/v1/dnp3/devices/{device_id}")

    def add_dnp3_point(self, device_id: str, point: Dict[str, Any]) -> None:
        """Subscribe a DNP3 point (group/variation/index)."""
        self._request("POST", f"/api/v1/dnp3/devices/{device_id}/points", body=point)

    def remove_dnp3_point(self, device_id: str, point_id: str) -> None:
        """Unsubscribe a DNP3 point."""
        self._request("DELETE", f"/api/v1/dnp3/devices/{device_id}/points/{point_id}")

    def scan_dnp3_network(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Scan a network range for DNP3 outstations."""
        return self._request("POST", "/api/v1/dnp3/scan", body=config)

    def start_dnp3_device(self, device_id: str) -> None:
        """Start polling a DNP3 device."""
        self._request("POST", f"/api/v1/dnp3/devices/{device_id}/start")

    def stop_dnp3_device(self, device_id: str) -> None:
        """Stop polling a DNP3 device."""
        self._request("POST", f"/api/v1/dnp3/devices/{device_id}/stop")

    def trigger_dnp3_integrity(self, device_id: str) -> None:
        """Trigger an integrity poll on a DNP3 device."""
        self._request("POST", f"/api/v1/dnp3/devices/{device_id}/integrity")

    def trigger_dnp3_freeze(self, device_id: str) -> None:
        """Trigger a counter freeze on a DNP3 device."""
        self._request("POST", f"/api/v1/dnp3/devices/{device_id}/freeze")

    def list_dnp3_group_variations(self) -> Dict[str, Any]:
        """Supported DNP3 object group/variation catalog."""
        return self._request("GET", "/api/v1/dnp3/group-variations")

    # ── BACnet devices ──────────────────────────────────────

    def list_bacnet_devices(self) -> List[Dict[str, Any]]:
        """List BACnet devices."""
        return self._request("GET", "/api/v1/bacnet/devices")

    def create_bacnet_device(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create a BACnet device."""
        return self._request("POST", "/api/v1/bacnet/devices", body=config)

    def update_bacnet_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Update a BACnet device."""
        return self._request("PUT", f"/api/v1/bacnet/devices/{device_id}", body=config)

    def delete_bacnet_device(self, device_id: str) -> None:
        """Delete a BACnet device."""
        self._request("DELETE", f"/api/v1/bacnet/devices/{device_id}")

    def list_bacnet_objects(self, device_id: str) -> List[Dict[str, Any]]:
        """Object subscriptions of a BACnet device."""
        return self._request("GET", f"/api/v1/bacnet/devices/{device_id}/objects")

    def add_bacnet_object(self, device_id: str, obj: Dict[str, Any]) -> None:
        """Subscribe a BACnet object."""
        self._request("POST", f"/api/v1/bacnet/devices/{device_id}/objects", body=obj)

    def remove_bacnet_object(self, device_id: str, object_id: str) -> None:
        """Unsubscribe a BACnet object."""
        self._request("DELETE", f"/api/v1/bacnet/devices/{device_id}/objects/{object_id}")

    def browse_bacnet_device(self, device_id: str, config: Dict[str, Any]) -> Dict[str, Any]:
        """Browse objects of a BACnet device."""
        return self._request("POST", f"/api/v1/bacnet/devices/{device_id}/browse", body=config)

    def start_bacnet_device(self, device_id: str) -> None:
        """Start polling a BACnet device."""
        self._request("POST", f"/api/v1/bacnet/devices/{device_id}/start")

    def stop_bacnet_device(self, device_id: str) -> None:
        """Stop polling a BACnet device."""
        self._request("POST", f"/api/v1/bacnet/devices/{device_id}/stop")

    def discover_bacnet_network(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Who-is discovery on the BACnet network."""
        return self._request("POST", "/api/v1/bacnet/discover", body=config)

    # ── MQTT broker ─────────────────────────────────────────

    def get_mqtt_status(self) -> Dict[str, Any]:
        """Built-in MQTT broker status."""
        return self._request("GET", "/api/v1/mqtt-broker/status")

    def update_mqtt_config(self, config: Dict[str, Any]) -> None:
        """Update broker settings (port, max_connections, auth users)."""
        self._request("PUT", "/api/v1/mqtt-broker/config", body=config)

    def start_mqtt_broker(self) -> None:
        """Start the built-in MQTT broker."""
        self._request("POST", "/api/v1/mqtt-broker/start")

    def stop_mqtt_broker(self) -> None:
        """Stop the built-in MQTT broker."""
        self._request("POST", "/api/v1/mqtt-broker/stop")

    def get_mqtt_topics(self) -> Dict[str, Any]:
        """Topics seen by the broker."""
        return self._request("GET", "/api/v1/mqtt-broker/topics")

    def get_mqtt_topic_messages(self, topic: str, limit: Optional[int] = None) -> List[Dict[str, Any]]:
        """Recent messages on one topic."""
        return self._request(
            "GET", f"/api/v1/mqtt-broker/topics/{topic}/messages", params={"limit": limit}
        )

    def get_mqtt_messages(self, limit: Optional[int] = None) -> List[Dict[str, Any]]:
        """Recent messages across all topics."""
        return self._request("GET", "/api/v1/mqtt-broker/messages", params={"limit": limit})

    def publish_mqtt_message(self, topic: str, payload: str) -> None:
        """Publish a message through the built-in broker."""
        self._request(
            "POST", "/api/v1/mqtt-broker/publish", body={"topic": topic, "payload": payload}
        )

    # ── Screens ─────────────────────────────────────────────

    def list_screens(self) -> List[Screen]:
        """List dashboard screens."""
        data = self._request("GET", "/api/v1/screens")
        return [_model(Screen, s) for s in data]

    def create_screen(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create a screen."""
        return self._request("POST", "/api/v1/screens", body=config)

    def update_screen(self, screen_id: str, config: Dict[str, Any]) -> None:
        """Update a screen."""
        self._request("PUT", f"/api/v1/screens/{screen_id}", body=config)

    def delete_screen(self, screen_id: str) -> None:
        """Delete a screen."""
        self._request("DELETE", f"/api/v1/screens/{screen_id}")

    def list_screen_sensors(self, screen_id: str) -> List[Dict[str, Any]]:
        """Data sources attached to a screen."""
        return self._request("GET", f"/api/v1/screens/{screen_id}/data-sources")

    def add_screen_sensor(self, screen_id: str, data_source_id: int) -> None:
        """Attach a data source to a screen."""
        self._request(
            "POST", f"/api/v1/screens/{screen_id}/data-sources", body={"data_source_id": data_source_id}
        )

    def update_screen_sensor(self, screen_id: str, data_source_id: int, config: Dict[str, Any]) -> None:
        """Update a data source on a screen (e.g. sort order)."""
        self._request("PUT", f"/api/v1/screens/{screen_id}/data-sources/{data_source_id}", body=config)

    def remove_screen_sensor(self, screen_id: str, data_source_id: int) -> None:
        """Detach a data source from a screen."""
        self._request("DELETE", f"/api/v1/screens/{screen_id}/data-sources/{data_source_id}")

    # ── OEE ─────────────────────────────────────────────────

    def get_oee_metrics(
        self, device_id: str, from_ts: Optional[int] = None, to_ts: Optional[int] = None
    ) -> OeeMetrics:
        """OEE (availability x performance x quality) for one device.
        ``from_ts``/``to_ts`` are unix seconds; defaults to the last 8 hours."""
        params: Dict[str, Any] = {"device_id": device_id, "from": from_ts, "to": to_ts}
        return _model(OeeMetrics, self._request("GET", "/api/v1/oee/metrics", params=params))

    def list_downtime(
        self, device_id: Optional[str] = None, from_ts: Optional[int] = None, to_ts: Optional[int] = None
    ) -> List[Dict[str, Any]]:
        """Downtime events, optionally filtered by device and window."""
        return self._request(
            "GET",
            "/api/v1/oee/downtime",
            params={"device_id": device_id, "from": from_ts, "to": to_ts},
        )

    def create_downtime(self, device_id: str, state: str) -> Dict[str, Any]:
        """Manually open a downtime event for a device."""
        return self._request(
            "POST", "/api/v1/oee/downtime", body={"device_id": device_id, "state": state}
        )

    def close_downtime(self, downtime_id: int, reason: Optional[str] = None) -> None:
        """Close a downtime event."""
        self._request(
            "PUT", f"/api/v1/oee/downtime/{downtime_id}/close", body={"reason": reason}
        )

    def list_shifts(self) -> List[Dict[str, Any]]:
        """Configured production shifts."""
        return self._request("GET", "/api/v1/oee/shifts")

    def create_shift(
        self, name: str, start_hour: int, start_minute: int, duration_hours: int, days_mask: int
    ) -> Dict[str, Any]:
        """Create a shift. ``days_mask`` bit 0 = Monday."""
        return self._request(
            "POST",
            "/api/v1/oee/shifts",
            body={
                "name": name,
                "start_hour": start_hour,
                "start_minute": start_minute,
                "duration_hours": duration_hours,
                "days_mask": days_mask,
            },
        )

    def delete_shift(self, shift_id: int) -> None:
        """Delete a shift."""
        self._request("DELETE", f"/api/v1/oee/shifts/{shift_id}")

    # ── Analytics ───────────────────────────────────────────

    def get_aggregate(
        self, device_id: str, metric: str, from_ts: int, to_ts: int, interval: int = 3600
    ) -> List[Dict[str, Any]]:
        """Aggregate a metric over time. ``interval`` in seconds."""
        return self._request(
            "GET",
            "/api/v1/analytics/aggregate",
            params={
                "device_id": device_id,
                "metric": metric,
                "from": from_ts,
                "to": to_ts,
                "interval": interval,
            },
        )

    def get_compare(
        self, device_id: str, metric: str, period1: tuple, period2: tuple
    ) -> Dict[str, Any]:
        """Compare a metric between two (from, to) unix-second windows."""
        return self._request(
            "GET",
            "/api/v1/analytics/compare",
            params={
                "device_id": device_id,
                "metric": metric,
                "period1_from": period1[0],
                "period1_to": period1[1],
                "period2_from": period2[0],
                "period2_to": period2[1],
            },
        )

    def get_spc(
        self,
        device_id: str,
        metric: str,
        from_ts: int,
        to_ts: int,
        subgroup_size: int = 5,
        chart_type: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Statistical process control chart data."""
        return self._request(
            "GET",
            "/api/v1/analytics/spc",
            params={
                "device_id": device_id,
                "metric": metric,
                "from": from_ts,
                "to": to_ts,
                "subgroup_size": subgroup_size,
                "chart_type": chart_type,
            },
        )

    def get_histogram(
        self,
        device_id: str,
        metric: str,
        from_ts: int,
        to_ts: int,
        bins: int = 20,
        lsl: Optional[float] = None,
        usl: Optional[float] = None,
    ) -> Dict[str, Any]:
        """Histogram (and process capability when lsl/usl given)."""
        return self._request(
            "GET",
            "/api/v1/analytics/histogram",
            params={
                "device_id": device_id,
                "metric": metric,
                "from": from_ts,
                "to": to_ts,
                "bins": bins,
                "lsl": lsl,
                "usl": usl,
            },
        )

    def get_run_chart(self, device_id: str, metric: str, from_ts: int, to_ts: int) -> Dict[str, Any]:
        """Run chart for one metric."""
        return self._request(
            "GET",
            "/api/v1/analytics/run-chart",
            params={"device_id": device_id, "metric": metric, "from": from_ts, "to": to_ts},
        )

    # ── API keys (admin) ────────────────────────────────────

    def create_api_key(self, config: Dict[str, Any]) -> Dict[str, Any]:
        """Create an API key. The key value is only shown once."""
        return self._request("POST", "/api/v1/api-keys", body=config)

    def list_api_keys(self) -> List[Dict[str, Any]]:
        """List API keys (id, name, never the secret)."""
        return self._request("GET", "/api/v1/api-keys")

    def revoke_api_key(self, key_id: str) -> None:
        """Revoke an API key."""
        self._request("DELETE", f"/api/v1/api-keys/{key_id}")
