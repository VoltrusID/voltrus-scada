# Voltrus SCADA

Self-hosted SCADA and industrial monitoring in a single Rust binary, with a
React web client. Poll PLCs and meters, keep the history in SQLite, build
dashboards and run alarms, on your own hardware and with no cloud account.

This repository is the open-source **Core**, licensed under
[AGPL-3.0-or-later](LICENSE). A separate closed-source **Enterprise add-on**
(single sign-on, 21 CFR Part 11, hot standby and more) is sold per site.
See [Core vs Enterprise](#core-vs-enterprise).

Built by [JoyoDigitama](https://joyodigitama.com). Website and pricing:
<https://voltrus.id/voltrus-scada>.

## Core features

* **Protocols:** Modbus TCP, OPC UA, Siemens S7, EtherNet/IP (CIP), DNP3,
  BACnet/IP, and an embedded MQTT broker. SNMP network scan for asset discovery.
  Device templates for Modbus and BACnet: built-in, saved from your own
  devices, or imported from a public catalog.
  Each protocol is a Cargo feature, all on by default.
* **Dashboards:** responsive grid, free-form canvas, 3D digital twin and a 2D
  SCADA diagram editor, with per-screen layouts.
* **Alarms:** warning and critical thresholds, priorities, shelving, deadband,
  on and off delay, acknowledgement, alarm analytics, email, webhook and push
  notifications.
* **Historian:** raw, hourly and daily tiers with automatic downsampling and
  CSV archive, trends, SPC charts, annotations, CSV and Excel import and export.
* **Automation:** visual flow engine, JavaScript scripts (QuickJS), recipes,
  a command queue with write read-back, simulation mode.
* **API:** REST with an OpenAPI document, plus TypeScript and Python SDKs in
  [`sdks/`](sdks) (MIT licensed).
* **AI agents (MCP):** a Model Context Protocol server built into the server at
  `/mcp`, so Claude and other MCP clients can read values, manage devices and
  tags, and acknowledge alarms with an API key. See [docs/mcp.md](docs/mcp.md).
* **Operations:** OEE and downtime, shift handover, asset inventory with NIS2
  export, config snapshots, backup and restore, whole-configuration YAML
  projects (`voltrus project export|import|validate`).
* **Security:** local accounts with Argon2, role-based access control, API
  keys, rate limiting, CSRF protection, intrusion detection, audit log.
* **No limits and no licence key:** unlimited devices, tags and retention.

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

Integrator and Enterprise are sold as a lifetime licence or monthly, per site. A per-site key can also carry an annual support contract; a lapsed contract never limits Core or anything already licensed (plan: [docs/licensing-per-site-plan.md](docs/licensing-per-site-plan.md)).
Current prices: <https://voltrus.id/voltrus-scada>. Official Voltrus binaries
are Core plus the add-on. Details of the split and how it is built:
[docs/open-core.md](docs/open-core.md).

## Quick start

You need Rust (stable, edition 2024) and Node.js 22 or newer.

```bash
# 1. Build the web client into ./static
cd client && npm ci && npm run build && cd ..

# 2. Build the server (all protocols and scripts)
cargo build --release

# 3. Edit config.yaml (devices, auth_secret), then run
./target/release/voltrus config.yaml
```

Open <http://localhost:3000>. The first start generates a random admin
password and prints it once to the server log (look for
`GENERATED ADMIN PASSWORD`). Change it after the first sign-in.

Prebuilt Core binaries for Linux (x86_64, aarch64) and macOS are attached to
each [release](https://github.com/VoltrusID/voltrus-scada/releases). They
include the web client. Unpack, edit `config.yaml.example`, run `./voltrus`.

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
The API documentation page loads its viewer script from a public CDN.

## Development

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test

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
native/         experimental native operator console (GPUI)
scenarios/      simulated plants for testing
deploy/         example systemd unit
docs/           open-core split, MCP server, notes
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
