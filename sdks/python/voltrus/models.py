"""Typed models for Voltrus SCADA SDK.

Plain dataclasses — the only dependency is urllib3. Fields mirror the JSON
the server actually returns (see src/models/ and src/api/ in the voltrus
repository). Endpoints whose bodies are protocol-specific (device configs,
flows, recipes, ...) return plain dicts instead of models.
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Union


@dataclass
class LiveValue:
    """One evaluated data source — a row of GET /api/v1/data."""

    id: int
    key: str
    value: float
    warn: float
    crit: float
    source_type: str  # 'modbus' | 'simulated' | ...
    status: str  # 'normal' | 'warning' | 'critical' | 'offline'

    @property
    def quality(self) -> str:
        """Map status to good/bad, Kepware-style."""
        return "bad" if self.status == "offline" else "good"


@dataclass
class DeviceSnapshot:
    """Latest cached metrics for one device — a row of GET /api/v1/live."""

    device_id: str
    status: str  # 'online' | 'offline' | ...
    timestamp: int  # unix seconds
    metrics: Dict[str, float]
    simulated: bool = False
    protocol: str = ""


@dataclass
class HistorySeries:
    """Compact time series for one sensor."""

    values: List[float] = field(default_factory=list)
    start: int = 0  # unix ms
    interval: int = 0  # ms between points
    count: int = 0


@dataclass
class HistoryResponse:
    """Response of GET /api/v1/history."""

    sensors: List[Dict[str, Any]]
    data: Dict[str, HistorySeries]
    range: str
    interval: int  # ms between points


@dataclass
class Alarm:
    """One alarm row from GET /api/v1/alarms."""

    id: int
    source_key: str
    source_name: str
    severity: str  # 'critical' | 'warning' | 'info'
    priority: str
    value: float
    threshold: float
    message: str
    triggered_at: int  # unix seconds
    state: str  # 'active' | 'acknowledged' | 'shelved' | 'cleared'
    alarm_type: str = "high"  # 'high' fires at >= threshold, 'low' at <=
    deadband: float = 0.0
    resolved_at: Union[int, None] = None
    acknowledged_at: Union[int, None] = None
    acknowledged_by: Union[str, None] = None
    cooldown_until: Union[int, None] = None
    shelved_until: Union[int, None] = None
    shelved_by: Union[str, None] = None
    shelved_reason: Union[str, None] = None


@dataclass
class AlarmSummary:
    """Counts from GET /api/v1/alarms/summary."""

    active: int = 0
    acknowledged: int = 0
    shelved: int = 0
    critical: int = 0
    warning: int = 0
    info: int = 0


@dataclass
class WriteResult:
    """Response of POST /api/v1/tags/{id}/write (HTTP 202)."""

    command_id: str
    status: str  # 'pending' until the device acks
    data_source_id: int
    value: Union[float, str, bool, None] = None
    created_at: Union[int, None] = None


@dataclass
class Screen:
    """One screen (dashboard page)."""

    id: str
    name: str
    type: str = ""
    sort_order: int = 0


@dataclass
class OeeMetrics:
    """OEE breakdown for one device."""

    availability: float
    performance: float
    quality: float
    oee: float
    total_running_secs: int
    total_planned_secs: int
    downtime_count: int
