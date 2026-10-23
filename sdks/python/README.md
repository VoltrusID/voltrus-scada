# Voltrus Python SDK

Official Python SDK for the Voltrus SCADA server. Hand-written, typed,
urllib3-only — no pydantic, no codegen, no heavy dependencies.

## Install

```bash
cd sdks/python
pip install -e .
```

Requires Python 3.9+.

## 60-second quickstart

```bash
export VOLTRUS_URL="http://your-server:3000"
export VOLTRUS_API_KEY="vt_..."          # create one in the UI (Admin → API Keys)
python examples/quickstart.py
```

Or in code:

```python
from voltrus import VoltrusClient

client = VoltrusClient("http://your-server:3000", api_key="vt_...")

print(client.get_version())

for v in client.get_live_data(screen_id="overview"):
    print(f"{v.key}: {v.value:.2f} ({v.status})")
```

Runnable scripts live in [`examples/`](examples/) — they share the
`make_client()` helper in `quickstart.py`, so run them from that directory:

- `quickstart.py` — connect, read live data
- `alarms.py` — list alarms, optionally acknowledge (`VOLTRUS_ACK=yes`)
- `devices_24x7.py` — poll devices forever on an interval, survives network blips

## Authentication

Voltrus accepts two credential types; the SDK supports both.

**API key** — the machine-to-machine path. Send it once to the constructor;
every request carries the `X-API-Key` header.

```python
client = VoltrusClient("http://your-server:3000", api_key="vt_...")
```

API keys authenticate as role `api` and can reach the read/protection
endpoints (live data, history, alarms, commands read, OEE, analytics). They
cannot perform writes or device configuration — those need a login session.

**Login session** — full access as a user. The server returns the access and
refresh tokens as httpOnly cookies, not in the response body, so the SDK
captures and replays them for you:

```python
client = VoltrusClient("http://your-server:3000")
client.login("admin", "secret")     # session cookies stored on the client
client.me()                         # {'authenticated': True, 'username': 'admin', ...}
client.refresh()                    # rotate an about-to-expire session
client.logout()                     # revoke server-side and drop cookies
```

For OIDC setups you can also pass `token="..."` and the SDK sends it as
`Authorization: Bearer`.

## Errors

Any non-2xx response raises `VoltrusError` with `.status`, `.message`
(server `error` field) and `.body` (raw response text).

```python
from voltrus import VoltrusError

try:
    client.acknowledge_alarm(42)
except VoltrusError as e:
    print(f"failed: {e.status} {e.message}")
```

## API groups

### Live data & tags

```python
client.get_live_data(screen_id="overview")     # current values for a screen
client.get_live_data(device_id="plc-1")        # ...or one device
client.get_live_snapshots()                    # raw per-device metric snapshots

history = client.get_history(device_id="plc-1", range="1h", agg="avg")
series = history.data["tank1.level"]           # .values, .start, .interval, .count

client.get_tag_tree()                          # hierarchical UNS tree
client.list_data_sources(screen_id="overview") # tag configuration
client.export_csv("tank1.level", "24h")        # CSV as text
client.export_data(from_ts="2026-09-01T00:00:00Z", format="csv")
```

`LiveValue.status` is one of `normal`, `warning`, `critical`, `offline`;
the `quality` property maps it to `good`/`bad` for Kepware-style consumers.

### Alarms

```python
client.get_alarm_summary()                     # counts by state/severity
client.get_active_alarm_count()
alarms = client.list_alarms(status="active", priority="critical")

for a in alarms:
    if a.severity == "critical":
        client.acknowledge_alarm(a.id)

client.acknowledge_all_alarms()
client.shelf_alarm(42, duration_secs=3600, reason="maintenance window")
client.unshelf_alarm(42)
client.list_alarm_audit(alarm_id=42)           # who acked/shelved, when
client.get_alarm_analytics(from_ts=..., to_ts=...)
```

### Devices

Device CRUD and per-protocol tag management. Config bodies differ per
protocol, so create/update take dicts — see the Interactive API docs for
each schema.

```python
# Modbus
client.list_modbus_devices()
client.get_modbus_poller_statuses()            # 24/7 health view
client.restart_modbus_poller("plc-1")
client.scan_modbus_network({"host": "192.168.1.0/24"})

# OPC-UA
client.list_opcua_devices()
client.add_opcua_node("opc-1", {"node_id": "ns=2;s=Tank1.Level"})
client.browse_opcua_device({"id": "opc-1", "node_id": "ns=2"})
client.start_opcua_device("opc-1")

# S7 / EtherNet/IP (same shape per family)
client.list_s7_devices();  client.add_s7_tag("s7-1", {...});  client.start_s7_device("s7-1")
client.list_enip_devices(); client.add_enip_tag("enip-1", {...}); client.stop_enip_device("enip-1")

# DNP3 (substation/RTU) and BACnet (building)
client.list_dnp3_devices(); client.trigger_dnp3_integrity("dnp3-1")
client.list_bacnet_devices(); client.discover_bacnet_network({})

# Built-in MQTT broker
client.get_mqtt_status()
client.get_mqtt_topics()
client.publish_mqtt_message("voltrus/demo", "hello")
```

### Commands (tag writes)

Writes are queued and confirmed by the PLC asynchronously — `write_tag`
returns a command you poll. Writes need a login session with write access
(API keys are read-oriented).

```python
result = client.write_tag(17, 42.5, reason="operator setpoint")
print(result.command_id, result.status)        # 'pending'

status = client.get_command_status(result.command_id)
client.list_commands(status="pending")
client.cancel_command(result.command_id)
client.get_command_audit(result.command_id)
client.update_write_policy(17, writable=True, write_min=0, write_max=100)
```

### OEE

```python
m = client.get_oee_metrics("line-1")           # last 8 hours by default
print(f"OEE {m.oee:.1%} (A {m.availability:.0%} x P {m.performance:.0%} x Q {m.quality:.0%})")

client.list_downtime(device_id="line-1")
event = client.create_downtime("line-1", state="changeover")
client.close_downtime(event["id"], reason="done")
client.list_shifts()
client.create_shift("Day", start_hour=6, start_minute=0, duration_hours=8, days_mask=0b1111111)
```

### Screens

```python
for s in client.list_screens():
    print(s.id, s.name)

client.list_screen_sensors(s.id)               # tags attached to a screen
client.add_screen_sensor(s.id, data_source_id=17)
client.create_screen({"name": "Line 1", "type": "process"})
```

### Reports

```python
client.list_reports()
html = client.generate_report(report_id)       # HTML document as text
with open("report.html", "w") as f:
    f.write(html)
client.create_report({"name": "Daily", "schedule": "daily"})
```

### System & analytics

```python
client.health(); client.get_system_info(); client.get_version()
client.get_features()                          # enabled protocols/features
client.get_license_status()

client.get_aggregate("plc-1", "temperature", from_ts, to_ts, interval=300)
client.get_histogram("plc-1", "temperature", from_ts, to_ts, lsl=10, usl=40)
client.get_spc("plc-1", "temperature", from_ts, to_ts)
client.get_run_chart("plc-1", "temperature", from_ts, to_ts)
client.get_compare("plc-1", "temperature", (t1, t2), (t3, t4))
```

### API keys (admin)

```python
client.create_api_key({"name": "historian"})
client.list_api_keys()
client.revoke_api_key(key_id)
```

### Anything else

The REST surface is bigger than the typed methods above (flows, recipes,
RBAC, hub, video, annotations, shift handover, ...). Use the escape hatch:

```python
client.request("GET", "/api/v1/flows")
```

## Interactive API docs

The server serves a Scalar API reference from its own spec:

- UI: `http://your-server:3000/api/docs`
- OpenAPI JSON: `http://your-server:3000/api/v1/openapi.json`

## 24/7 operation

For unattended production use, see
[`examples/devices_24x7.py`](examples/devices_24x7.py): fixed-interval
polling, transient errors logged and retried instead of crashing, and clean
SIGINT/SIGTERM shutdown. Create a dedicated API key per integration so
access can be audited and revoked independently.

## Troubleshooting

**Version compatibility** — these SDKs target the Voltrus v1 HTTP API and are tested against server 0.47.x.

**401 Invalid credentials** — on first boot the server generates a random admin password and prints it to the server log once (`GENERATED ADMIN PASSWORD`). Change it on first login.

**403 admin access required** — API keys are read-only (`role: api`). Writes (device changes, tag writes, alarm acknowledgment) need a session login as an admin user.

**415 Expected request with Content-Type: application/json after login** — fixed in current versions; upgrade if cookie-authenticated POSTs fail this way.

## License

MIT
