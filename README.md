# Voltrus SCADA

Self-hosted SCADA and industrial monitoring in a single Rust binary, with a
React web client. Poll PLCs and meters, keep the history in SQLite, build
dashboards and run alarms, on your own hardware and with no cloud account.

This repository is the open-source **Core**, licensed under
[AGPL-3.0-or-later](LICENSE). A separate **Enterprise add-on** (single sign-on,
21 CFR Part 11, hot standby and more) is sold per site. Its source is available
to read in
[VoltrusID/voltrus-scada-enterprise](https://github.com/VoltrusID/voltrus-scada-enterprise)
under the Voltrus Enterprise License, which is not an open-source licence.
See [Core vs Enterprise](#core-vs-enterprise).

Built by [JoyoDigitama](https://joyodigitama.com). Website and pricing:
<https://voltrus.id/voltrus-scada>.

## Core features

* **Protocols:** Modbus TCP, OPC UA, Siemens S7, EtherNet/IP (CIP), DNP3,
  BACnet/IP, and an embedded MQTT broker. SNMP network scan for asset discovery.
  Device templates for Modbus and BACnet: built-in, saved from your own
  devices, or imported from a public catalog.
  Each protocol is a Cargo feature, all on by default. The drivers are not
  equally mature; see [Project status](#project-status).
* **Dashboards:** responsive grid, free-form canvas, 3D digital twin and a 2D
  SCADA diagram editor, with per-screen layouts.
* **Alarms:** warning and critical thresholds (zero and negative limits too),
  priorities, shelving, deadband, on and off delay, acknowledgement, alarm
  analytics, email, webhook and push notifications. Alarms are evaluated only
  on values of good quality.
* **Historian:** a sample of every tag each persistence interval
  (`persist_interval_secs`, default 5 s) packed into compressed blocks
  (delta-of-delta timestamps, XOR floats, run-length digitals), per-tag
  deadband, hourly and daily tiers with automatic downsampling and CSV archive,
  trends, SPC charts, annotations, CSV and Excel import and export. See
  [docs/historian.md](docs/historian.md).
* **Automation:** visual flow engine, JavaScript scripts (QuickJS), recipes,
  a command queue with retries, expiry and write read-back, simulation mode.
* **API:** REST with an OpenAPI document and an interactive API reference at
  `/api/docs` (sign in first), live data over WebSocket (MessagePack) with a
  server-sent-events fallback at `/api/v1/stream`, plus TypeScript and Python
  SDKs in [`sdks/`](sdks) (MIT licensed; they cover the main endpoints and
  can lag the server version). Tag writes from REST, WebSocket and MCP go
  through the same command queue, write policy and audit trail
  ([docs/live-stream.md](docs/live-stream.md)), and so do flow `WriteOut`
  nodes and recipes. Every live value carries a quality (good, uncertain,
  stale, bad) and a millisecond timestamp. API keys go in the `X-API-Key`
  header; `?api_key=` is accepted only on `/metrics` (Prometheus) and
  `/api/v1/stream`.
* **AI agents (MCP):** a Model Context Protocol server built into the server at
  `/mcp`, so Claude and other MCP clients can read values, manage devices and
  tags, and acknowledge alarms with an API key. Writes and deletions need a
  second confirming call by default, calls are rate limited per key, and
  `mcp.read_only: true` turns every write tool off. See
  [docs/mcp.md](docs/mcp.md).
* **Operations:** OEE and downtime, shift handover, asset inventory with NIS2
  export, config snapshots, backup and restore, whole-configuration YAML
  projects (`voltrus project export|import|validate`).
* **Security:** local accounts with Argon2; admin, operator (demo) and
  read-only users; API keys whose permissions can narrow, never widen, their
  role; rate limiting, SameSite=Strict session cookies, security headers,
  intrusion detection; a hash-chained audit log (`GET /api/v1/audit/verify`);
  the AI provider key encrypted at rest. Custom roles are labels only for now.
* **No limits and no licence key:** unlimited devices, tags and retention.

## Project status

Voltrus is young and built by a small team. Not every part is at the same
level, and this table says where each one stands so you can plan around it.

| Area | Status |
|---|---|
| Modbus TCP | Production: batched reads, timeouts, reconnect with backoff; queued writes in the register's type, scale, area and word order, coils included |
| OPC UA | Reads through subscriptions; writes through the live session in the node's data type. Secure modes need the server certificate in `pki/trusted` |
| Siemens S7 | Reads split to the negotiated PDU size; writes in the tag's type (BOOL to REAL, INT/DINT). Tested against the bundled simulator, not yet a real PLC |
| EtherNet/IP | Reads and writes in the tag's CIP type (BOOL to LREAL), 5 s timeouts. Unconnected messaging only; not yet tested on a real controller |
| DNP3 | Beta: own IEEE 1815 master (integrity and event polls, unsolicited responses, select-before-operate and direct operate, time sync). Tested only against the bundled outstation simulator |
| BACnet/IP | Experimental: ReadPropertyMultiple polling; WriteProperty in the object's type with priority (default 8). No COV or segmentation yet |
| Device start-up | Every enabled device starts polling at boot |
| Command queue | Retries with backoff; a write not delivered within 30 minutes expires instead of landing late; acknowledged writes are verified by read-back (confirmed, mismatch or timeout) |
| Data quality | Every value is good, uncertain, stale or bad (with a reason) and has a millisecond timestamp; bad and stale values stay out of the historian |
| Alarms | Thresholds (zero and negative too), deadband, on/off delay, acknowledgement and shelving, evaluated on good values only. Not the full ISA-18.2 state model yet |
| Native console ([`native/`](native/README-native.md)) | Experimental, macOS first |

Help with any of these is welcome; see [Contributing](#contributing).

## Core vs Enterprise

Core is complete for one site. The Enterprise add-on is unlocked per site by a
licence key; a lapsed or missing key never limits Core.

| | Core (this repo) | Integrator | Enterprise |
|---|:-:|:-:|:-:|
| Everything listed above, all protocols, no limits | yes | yes | yes |
| White-label branding and theme editor | | yes | yes |
| Scheduled PDF reports and Telegram/WhatsApp alarms | | yes | yes |
| Multi-site hub | | yes | yes |
| Single sign-on (AD, Azure AD, Okta, SAML, OIDC, LDAP) | | | yes |
| 21 CFR Part 11 mode (e-signatures, record checksums) | | | yes |
| IEC 62443 security audit mode | | | yes |
| Hot standby with automatic failover | | | yes |
| RTSP video cameras | | | yes |
| Support | community | standard | priority SLA |

Integrator and Enterprise are sold as a lifetime licence or monthly, per site. A per-site key can also carry an annual support contract; a lapsed contract never limits Core or anything already licensed.
Current prices: <https://voltrus.id/voltrus-scada>. Official Voltrus binaries
are Core plus the add-on. Details of the split and how it is built:
[docs/open-core.md](docs/open-core.md).

## Quick start

You need Rust (stable, edition 2024) and Node.js 22 or newer.

```bash
# 1. Build the web client into ./static
cd client && npm ci && npm run build && cd ..

# 2. Build the server (all protocols and scripts). A release build embeds ./static
#    into the executable, so step 1 has to come first.
cargo build --release

# 3. Edit config.yaml (devices, auth_secret), then run
./target/release/voltrus config.yaml
```

The release binary is one file: the web UI is inside it, compressed, and served
with content types, ETags and long-lived caching for the hashed assets, and with
the page as the answer to client-side routes. It needs only a config file (and
creates its database on first start).

* **Build order.** `cargo build --release` fails with a message when
  `static/index.html` does not exist. To build without the web UI (an API-only
  server, simulators, CI jobs that never open the UI) set
  `VOLTRUS_SKIP_UI_EMBED=1`. Debug builds (`cargo build`, `cargo test`,
  `cargo clippy`) embed nothing and need no client build; `VOLTRUS_EMBED_UI=1`
  embeds the UI in a debug build too.
* **Precedence.** If `static/index.html` exists in the working directory, that
  folder is served instead of the built-in UI, whole. That is for a patched or
  themed UI, or for `npm run build` against a running server. The log says which
  UI is served at startup. A `static/` folder left over from an older release
  therefore shows the old UI: delete it after upgrading to use the built-in one.

If you serve Voltrus over plain HTTP on the LAN (`http://<ip>:3000`) the sign-in
cookie is set without the `Secure` attribute, so login works. Behind an HTTPS
reverse proxy or tunnel on the same machine that sends `X-Forwarded-Proto: https`
(or Cloudflare's `CF-Visitor`) it is set with `Secure`. `cookie_secure: true` or
`false` in the config forces either; the default is `auto`.

Open <http://localhost:3000>. The first start generates a random admin
password and writes it to `initial-admin-password` next to the database
(readable by the owner only); the log shows the path, never the password.
Sign in as `admin`, change the password, and the file is deleted.

Set `auth_secret` in the config. Without it a new secret is generated at every
start, which signs everyone out and makes a stored AI provider key unreadable.

Prebuilt Core binaries for Linux (x86_64, aarch64), Windows (x86_64) and macOS
are attached to each [release](https://github.com/VoltrusID/voltrus-scada/releases).
The web client is inside the executable: unpack, edit `config.yaml.example`
(save it as `config.yaml`), run `./voltrus config.yaml` (`voltrus.exe` on Windows).

To build with fewer protocols:

```bash
cargo build --release --no-default-features --features "modbus,opcua,mqtt-broker"
```

### Configuration

One YAML file. A minimal example:

```yaml
server_port: 3000
db_path: "data.db"
persist_interval_secs: 5
# auth_secret: <random string, at least 16 characters>

devices:
  - id: "meter_1"
    ip: "192.168.1.10"
    port: 502
    unit_id: 1
    interval_ms: 1000
    registers:
      - name: "voltage"
        address: 40001
        type: "float32"
```

[`config.yaml`](config.yaml) lists more options (retention tiers, SMTP, MQTT
broker, networks). Devices can also be managed from the web UI.

Modbus and BACnet devices can also be created from shared device templates,
built in or from a public catalog; see
[docs/device-templates.md](docs/device-templates.md).

### Configuration as code

`voltrus project export ./site` writes the whole configuration (devices of every
protocol, tags with alarm and write policies, screens and layouts, flows,
scripts, recipes, notification rules, ...) as YAML files keyed by names, never
by database ids. `voltrus project import ./site --dry-run` prints what an
import would create, update or delete; without `--dry-run` it applies it in one
transaction (`--mode replace` also deletes what the project does not list).
Secrets are written as `${ENV_NAME}` references. Format and examples:
[`docs/project-format.md`](docs/project-format.md).

### Simulators and scenarios

`cargo run --features modbus --bin modbus-sim` and the other `*-sim` binaries
start fake devices. [`scenarios/`](scenarios) has ready-made plants (solar farm,
data centre, factory floor, oil and gas well, water treatment).

## Network behaviour

Core makes no outbound connections on its own. It does not check for updates,
send telemetry or contact any Voltrus server. It only connects to what you
configure: your devices, your SMTP server, your webhooks and MQTT brokers, and
an AI provider if you enable the optional AI copilot with your own key.
The API documentation page (`/api/docs`, signed-in users only) loads its viewer
script from a public CDN; the OpenAPI document at `/api/v1/openapi.json` does not.

## Development

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test          # debug build: no client build needed

cd client
npm ci
npm run dev        # Vite dev server on :5173
npx tsc --noEmit
npm test
```

The server is a library plus the `voltrus` binary in one crate. The
[extension seam](src/edition.rs) is how an add-on edition plugs in.

```
src/            Rust server (api/, storage/, protocol drivers, flow engine, ...)
client/         React web client (src/edition/ is the add-on seam)
sdks/           TypeScript and Python SDKs (MIT)
native/         experimental native operator console (GPUI), see native/README-native.md
scenarios/      simulated plants for testing
deploy/         example systemd unit
docs/           open-core split, MCP server, live stream, notes
```

## Contributing

Contributions are welcome. Sign off every commit (`git commit -s`) and read
[CONTRIBUTING.md](CONTRIBUTING.md), which explains the DCO and the licence grant
that lets the project keep offering the Enterprise add-on. Report security
problems privately, see [SECURITY.md](SECURITY.md).

## Licence and trademark

Copyright (c) 2026 JoyoDigitama. Licensed under the GNU Affero General Public
License v3.0 or later, see [LICENSE](LICENSE) and [NOTICE](NOTICE). The SDKs in
`sdks/` are MIT licensed. If you run a modified version for others over a
network, AGPL section 13 requires you to offer them the source.

"Voltrus" and the Voltrus logo are trademarks of JoyoDigitama. The AGPL does
not grant trademark rights: forks must use another name, see
[TRADEMARKS.md](TRADEMARKS.md). Product and protocol
names (Modbus, Siemens, Allen-Bradley, BACnet and others) are used only to
describe compatibility and belong to their owners.
