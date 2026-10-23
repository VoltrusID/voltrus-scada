# Voltrus TypeScript SDK

Official TypeScript client for the [Voltrus SCADA](https://voltrus.id/voltrus-scada) server. Drive your plant's live data, alarms, tag writes, devices, OEE and reports from Node.js or the browser — the same API the bundled web UI uses.

## Install

```bash
npm install voltrus
```

Requires Node.js 18 or newer (browser usage works out of the box too).

## 60-second quickstart

```typescript
import { VoltrusClient } from "voltrus";

const client = new VoltrusClient({
  basePath: "http://192.168.1.100:3000", // default server port is 3000
  apiKey: process.env.VOLTRUS_API_KEY,   // or use auth.login() — see below
});

const health = await client.system.getHealth();
console.log("online devices:", Object.values(health.devices).filter((s) => s === "online").length);

const live = await client.system.getLive();
for (const device of live.devices) {
  console.log(device.device_id, device.status, device.metrics);
}

const readings = await client.data.getData({ screen: "overview" });
for (const reading of readings) {
  console.log(`${reading.key} = ${reading.value}`);
}
```

Runnable versions of this and more live in [`examples/`](./examples):

| Example | What it shows |
| --- | --- |
| `quickstart.ts` | Connect, verify the server, read live data |
| `alarms.ts` | List alarms, read the summary, acknowledge and shelve |
| `devices-24x7.ts` | Interval polling with backoff — the always-on pattern |

## Authentication

The server accepts three credential types; configure **one** per client:

1. **API key** (recommended for scripts and services) — create one under *Settings → API Keys* in the web UI, then pass it via `apiKey`. It is sent as the `X-API-Key` header and takes precedence on the server.

   ```typescript
   const client = new VoltrusClient({ basePath, apiKey: "vt_sk_..." });
   ```

2. **Username/password login** — the server delivers the session as `HttpOnly` cookies (there is deliberately **no token in the login response body**). The SDK captures and replays those cookies for you; just call `auth.login()` before any protected request.

   ```typescript
   const client = new VoltrusClient({ basePath });
   const session = await client.auth.login("admin", "secret");
   console.log(session.role); // "admin"
   ```

   In browsers the cookie jar is handled by the browser itself (`withCredentials` is already on). Use `auth.refreshToken()` to rotate an expiring session and `auth.logout()` to end it.

3. **OIDC bearer token** — pass an `accessToken` (sent as `Authorization: Bearer …`). The server only honors this when OIDC is enabled.

## API groups

Everything below hangs off `VoltrusClient`; each group is also exported as a standalone class (e.g. `AlarmsApi`) if you prefer explicit construction.

### System

`client.system` — health, live device snapshots, host stats, version.

- `getHealth()` — unauthenticated liveness + per-device online map
- `getLive()` — latest snapshot per device: `metrics`, `status`, `protocol`, `timestamp`
- `getSystemInfo()`, `getVersion()`

### Live data & tags

`client.data` — evaluated data-source values, history, exports.

- `getData({ screen?, device?, sensor? })` — current values with warn/crit limits
- `getHistory({ screen?, range: "30m" | "1h" | … , agg? })` — compact series (`values[]`, `start`, `interval`)
- `exportCsv({ sensor, range })` and `exportData({ from, to, device, metric, aggregation, interval, limit, format })` — telemetry export

### Alarms

`client.alarms` — list, acknowledge, shelving, audit.

- `list({ status, priority, state, shelved, limit })`
- `getSummary()` — active critical/warning/info counts, shelved, 24 h volume, avg ack time
- `getActiveCount()`, `acknowledge(id)`, `acknowledgeAll()`
- `shelf(id, durationSecs, reason)`, `unshelf(id)`
- `getAudit(...)`, `getAnalytics(...)`

### Devices

`client.devices` — per-protocol management. Reading device lists works for any authenticated user; create/update/delete, start/stop and browse need the **admin** role.

- **Modbus** — `listModbus()`, `getModbus(id)`, `createModbus(...)`, `updateModbus(...)`, `deleteModbus(id)`, `getModbusPollerStatuses()`, `restartModbusPoller(id)`
- **OPC-UA** — `listOpcUa()`, `createOpcUa(...)`, `updateOpcUa(...)`, `deleteOpcUa(id)`, `listOpcUaNodes(id)`, `addOpcUaNode(...)`, `removeOpcUaNode(...)`, `startOpcUa(id)`, `stopOpcUa(id)`
- **S7** — `listS7()`, `createS7(...)`, `updateS7(...)`, `deleteS7(id)`, `startS7(id)`, `stopS7(id)`
- **EtherNet/IP** — `listEnip()`, `createEnip(...)`, `updateEnip(...)`, `deleteEnip(id)`, `listEnipTags(id)`, `addEnipTag(...)`, `removeEnipTag(...)`, `startEnip(id)`, `stopEnip(id)`
- **DNP3** — `listDnp3()`, `getDnp3(id)`, `createDnp3(...)`, `updateDnp3(...)`, `deleteDnp3(id)`, `addDnp3Point(...)`, `removeDnp3Point(...)`, `scanDnp3Network()`, `startDnp3(id)`, `stopDnp3(id)`, `triggerDnp3Integrity(id)`
- **BACnet** — `listBacnet()`, `createBacnet(...)`, `updateBacnet(...)`, `deleteBacnet(id)`, `listBacnetObjects(id)`, `addBacnetObject(...)`, `removeBacnetObject(...)`, `startBacnet(id)`, `stopBacnet(id)`
- **MQTT broker (embedded)** — `getMqttBrokerStatus()`, `getMqttTopics()`, `getMqttMessages(...)`

### Commands (tag writes)

`client.commands` — audited, policy-enforced writes to tags.

```typescript
const queued = await client.commands.writeTag(42, { value: 73.5, reason: "operator setpoint" });
const done = await client.commands.waitForCompletion(queued.command_id);
console.log(done.status); // "acked" | "failed" | "expired" | "cancelled"
```

- `list({ tag, status, device })`, `getStatus(id)`, `cancel(id)`, `getAudit(id)`
- `getWritePolicy(dataSourceId)`, `updateWritePolicy(dataSourceId, { writable, write_min, write_max, ... })`

Writes go through the server's write policy: value range, deadband, rate limit, and mandatory reason are all enforced before the command is queued (HTTP 202).

### OEE

`client.oee` — availability/performance/quality and shift management.

- `getMetrics({ device_id, from?, to? })`
- `listDowntime(...)`, `createDowntime({ device_id, state })`, `closeDowntime(id, reason?)`
- `listShifts()`, `createShift({ name, start_hour, start_minute, duration_hours, days_mask })`, `deleteShift(id)`

### Screens

`client.screens` — dashboard screens and their data-source bindings.

- `list()`, `create(name)`, `rename(id, name)`, `delete(id)`
- `listSensors(id)`, `addSensor(id, dataSourceId)`, `updateSensorOrder(id, dataSourceId, sortOrder)`, `removeSensor(id, dataSourceId)`

### Reports

`client.reports` — scheduled PDF reports (an Enterprise add-on feature: the endpoints return 404 on a Core-only server).

- `list()`, `create({ name, schedule, email_recipients, device_ids? })`, `update(id, ...)`, `delete(id)`
- `generate(id)` — run a report now instead of waiting for its schedule

## Interactive API docs

The server ships a live, interactive reference of every endpoint (Scalar UI backed by its OpenAPI spec). Point a browser at your server:

```
http://<server>:3000/api/docs
```

The raw spec is at `/api/v1/openapi.json`. The SDK's request and response types are modeled directly on it.

## Error handling

Every non-2xx response throws a `VoltrusApiError` with the HTTP `status`, the `method`/`path`, and the server's `error` message:

```typescript
import { VoltrusApiError } from "voltrus";

try {
  await client.commands.writeTag(42, { value: 9999 });
} catch (error) {
  if (error instanceof VoltrusApiError && error.status === 400) {
    console.log("rejected by write policy:", error.message);
  }
}
```

## Development

```bash
npm install
npm run build              # compile src/ → dist/
npm run typecheck          # strict typecheck of src/
npm run typecheck:examples # typecheck examples/ against src/
```

## Troubleshooting

**Version compatibility** — these SDKs target the Voltrus v1 HTTP API and are tested against server 0.47.x.

**401 Invalid credentials** — on first boot the server generates a random admin password and prints it to the server log once (`GENERATED ADMIN PASSWORD`). Change it on first login.

**403 admin access required** — API keys are read-only (`role: api`). Writes (device changes, tag writes, alarm acknowledgment) need a session login as an admin user.

## License

MIT
