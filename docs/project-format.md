# Project format `voltrus-project/1`

A *project* is the whole configuration of a Voltrus server written as YAML: the
devices of every protocol, the tags with their alarm and write policies, screens
with their layouts and mimics, flows, scripts, recipes, notification rules and
the rest. It is meant for version control, review, templating and moving a
configuration between servers.

```
voltrus project export ./site            # directory, one file per entity
voltrus project export site.yaml         # one file
voltrus project validate ./site          # schema, duplicate keys, dangling references
voltrus project import ./site --dry-run  # print what would change, write nothing
voltrus project import ./site            # merge (default)
voltrus project import ./site --mode replace
```

`voltrus config export|import|validate` still work and run the same code.
Every command reads `db_path` from `config.yaml` (`--config <PATH>` to pick another).

The same operations are available over HTTP for admins:

| Route | What |
|---|---|
| `GET /api/v1/project/export` | the single-file YAML as a download |
| `POST /api/v1/project/import?mode=merge\|replace&dry_run=true\|false` | YAML body; returns the plan (dry run) or the result |

AI agents reach the same operations through the MCP tools `project_export_yaml`,
`project_plan_import` (dry run) and `project_apply_import` (`mode` merge or
replace). Like the routes they need a signed-in admin: API and integration keys are
refused. See `docs/mcp.md`.

An invalid project answers `400` with `{"error", "issues": [{severity, at, message}]}`.
A real import first takes a configuration snapshot (`trigger: import`) and restarts
the Modbus pollers of devices it touched. Other protocols pick up changes on the
next server restart.

## What is covered, and what is not

| Section | Entity | Key |
|---|---|---|
| `modbus_devices` | Modbus device with `registers` | `id` |
| `opcua_devices` | OPC UA device with `nodes` | `name` |
| `s7_devices` | S7 device with `tags` | `name` |
| `enip_devices` | EtherNet/IP device with `tags` | `name` |
| `dnp3_devices` | DNP3 device with `points` | `name` |
| `bacnet_devices` | BACnet device with `objects` | `name` |
| `voltrus_plc_devices` | Voltrus PLC device with `tags` | `name` |
| `device_templates` | device template (Modbus registers or BACnet objects) of source `local` or `catalog:<url>` | `template_id` + `version` + `source` |
| `sim_profiles` | simulation profile | `name` |
| `scripts` | script | `name` |
| `tags` | data source: limits, alarm thresholds, ISA-18.2 delays, write policy, `source_config` | `ref` = `device/key` |
| `screens` | screen: simulation settings, tag membership, grid layouts (all breakpoints), canvas, 3D twin layout and camera, 2D SCADA mimic (equipment, connections) | `id` |
| `flows` | flow: trigger and nodes | `id` |
| `recipes` | recipe | `name` |
| `notifications` | alarm notification rule | `source_key` + `channel_type` |
| `webhooks` | outbound webhook | `name` |
| `egress_jobs` | scheduled data egress job | `name` |
| `reports` | scheduled report | `name` |
| `cameras` | camera (with its snapshot retention) | `name` |
| `hub_sites` | hub site | `name` |
| `equipment_templates` | equipment template | `name` |
| `shifts` | shift definition | `name` |
| `custom_metrics` | custom analytics metric | `name` |
| `assets` | asset inventory entry | `id` |
| `settings` | whitelisted settings (below) | singleton |

`settings` holds exactly: `alarm_cooldown_secs`, `naming_convention`
(`template`, `regex`), `oee_thresholds` (per device), `security_thresholds` and
`compliance`. Nothing else in the settings table is exported.

**Never part of a project:** telemetry and its rollups, alarm rows and their
journals, audit and security logs, command history, flow and script executions,
users, roles, password hashes, API keys, refresh tokens, push subscriptions,
user preferences and shortcuts, licence activation, config snapshots, maintenance
logs, annotations and shift handovers.

**Not in the database, so not in a project:** the server's own `config.yaml`
(listen address, MQTT broker, SMTP, the `retention:` tiers, HA, TLS). Per-camera
snapshot retention is part of `cameras`.

### Device templates

`device_templates` carries the library of [device templates](device-templates.md)
that belong to the installation: those saved from a device or imported from a file
(`source: local`) and those imported from a catalog (`source: catalog:<url>`). The
built-in templates ship with the server and are never exported; a project that lists
one (`source: builtin`) is invalid. A template is identified by `template_id`,
`version` and `source` together, so two versions of one template, or the same id
from two catalogs, are separate entities.

The entity is the template file with its `format` line left out and `source` added:
the same fields, the same validation (semantic version, register types and areas,
unique names, ...) and the same defaults the template routes apply. A template a
hand-written project leaves short (no `port`, `unit_id`, `endianness`,
`interval_ms`, `license`) is stored and exported with those filled in, so applying
the exported file again changes nothing.

```yaml
device_templates:
- template_id: acme-pm100
  version: 1.0.0
  source: local
  manufacturer: Acme
  model: PM100
  protocol: modbus
  description: Power meter
  verified: true
  verified_notes: Firmware 2.1 at site A
  quirks: [Word order is swapped]
  port: 1502
  interval_ms: 2000
  unit_id: 3
  endianness: swapped
  registers:
  - {name: voltage, offset: 0, register_type: float32, register_area: holding, scale: 1.0, unit: V}
  - {name: energy, offset: 2, register_type: int32, register_area: input, scale: 0.1, writable: true}
```

Devices created from a template do not remember it: the device carries its own
registers (`modbus_devices`), so a project stays complete without the template.
In **replace** mode templates the project does not list are deleted like any other
entity, so an old project without a `device_templates` section removes the stored
local and catalog templates; use merge for partial files.

Derived columns are recomputed on import the way the API does when saving: a
script's syntax error, and a custom metric's `tag_refs` and validation error.
Creation and update timestamps are not exported; imported rows get the import time.

## Natural keys and references

Nothing in a project refers to an AUTOINCREMENT id.

* **Tags.** A tag's identity is its `ref`, `<device>/<key>`. `key` alone is not
  unique in the database, so when two tags share the same `device` and `key` the
  later one (by database id) is written with an explicit `ref: device/key#2`
  (`#3`, ...). Everything that points at a tag (`screens[].tags[].ref`, layout
  items, canvas entries, SCADA widget `sensors`, flow nodes) uses the `ref`.
  A hand-written tag may also carry `id_hint: <n>`: when the tag is created, the
  database id `n` is used if it is free. It is accepted on input only and never
  exported.
* **Layouts.** The grid layout items the web client stores as `{i: "<tag id>", x, y, w, h}`
  are written as `{tag: <ref>, x, y, w, h}`. An `i` that matches no tag is kept as is.
* **Flows.** `write_out` nodes use `tag: <ref>` instead of `data_source_id`;
  `script` nodes use `script: <script name>` instead of `script_id`.
* **Screens.** `sim_profile` is a simulation profile name. A screen without a
  `name` has no row of its own (the overview screen `__overview` only carries
  layouts).
* Entities of sections without a unique key (for example two notification rules
  with the same source and channel) are matched by key and then by position.

Entities are listed sorted by key, fields in a fixed order, and JSON-valued
fields (`source_config`, flow `nodes`, layouts, ...) have their keys sorted, so
`export -> import -> export` reproduces the same bytes. The manifest's
`generated_at` is the export time; set `SOURCE_DATE_EPOCH` to make it
reproducible too.

## Layouts: directory and single file

Single file (`site.yaml`): the manifest keys `format`, `server_version`,
`generated_at`, then one key per section.

Directory:

```
project.yaml                  manifest (and, if you like, any section inline)
devices/modbus/<id>.yaml      one entity per file; opcua, s7, enip, dnp3, bacnet, voltrus_plc alike
tags/<device>.yaml            a list: all tags of one device
screens/<id>.yaml             flows/<id>.yaml   scripts/<name>.yaml   recipes/<name>.yaml
sim_profiles.yaml  notifications.yaml  webhooks.yaml  egress_jobs.yaml  reports.yaml
cameras.yaml  hub_sites.yaml  equipment_templates.yaml  shifts.yaml
device_templates.yaml  custom_metrics.yaml  assets.yaml   (each a list)
settings.yaml                 a mapping
```

On import every `*.yaml` below a section's directory is read, a file may hold
one mapping or a list, and the file layout is free; entities are identified by
their keys, not by file names. Files outside the layout are ignored with a
warning. Exporting over an existing directory replaces the files of this layout
and leaves everything else.

## Import

```
voltrus project import <PATH> [--mode merge|replace] [--dry-run]
```

The whole import runs in one SQLite transaction. The document is validated
first; any error aborts before anything is written.

* **merge** (default): entities the project lists are created or updated;
  everything else is left alone. A listed entity is written as listed, so
  nested lists (registers, layouts, ...) are replaced by the project's version.
  Fields the project leaves out take their defaults. Entities are updated in
  place, so database ids of tags survive.
* **replace**: after the above, entities of every covered kind that the project
  does *not* list are deleted (with their registers, memberships, layouts, ...),
  and whitelisted settings it does not mention are reset to their defaults.
  A project for replace must therefore be complete. Use merge for partial files.
* **`--dry-run`**: the same code path, then roll back. It prints one line per
  change (`+ create`, `~ update` with the fields that differ, `- delete`) and the
  totals. Values are never printed.

Validation covers: the format version, unknown fields and top-level keys, types,
required fields, duplicate keys (and duplicate children inside one entity),
and dangling references (tags, scripts, simulation profiles). In merge mode a
reference may be satisfied by an entity already in the database; in replace mode
only by the project. References to devices and notification sources that cannot
be resolved are warnings (a tag may legitimately use a device name such as
`default`). `voltrus project validate` checks a project on its own, without a
database.

## Secrets

A project never contains a secret. On export, every secret-bearing string is
replaced by a reference to an environment variable:

```yaml
opcua_devices:
- name: Inverter OPC
  url: opc.tcp://svc:${VOLTRUS_OPCUA_DEVICES_INVERTER_OPC_URL_PASSWORD}@10.0.0.5:4840
  username: svc
  password: ${VOLTRUS_OPCUA_DEVICES_INVERTER_OPC_PASSWORD}
```

Names are `VOLTRUS_<SECTION>_<KEY>_<FIELD PATH>` in upper case. A string counts
as secret when

* its key contains `password`, `passwd`, `passphrase`, `secret`, `token`,
  `apikey`/`api_key`/`api-key`, `authorization`, `bearer`, `cookie`,
  `credential`, `private_key` or `auth_key`, or is `auth`/`pwd`
  (anywhere in nested JSON, so HTTP headers like `Authorization` are covered);
* it is a URL (`url`, `uri`, `endpoint`, `destination`, ...) with `user:password@`
  or a credential query parameter (`token=`, `api_key=`, `key=`, `sig=`, ...);
* the section forces it: the `url` of a webhook and every target of a `webhook`
  notification rule (Slack/Teams style URLs carry the secret in the path).

On import the references are resolved from the process environment. If a
variable is unset, the value already stored for the *same entity* is kept, so a
project exported from a database can be imported back into it without any
environment. When a variable is unset and the entity is new, the import is
rejected and lists the missing variables, before anything is written.
A literal value written by hand is accepted too, and replaced by a reference on
the next export. Plans and error messages never show secret values.

## Example

```yaml
format: voltrus-project/1
server_version: 0.48.0
generated_at: 2026-01-01T00:00:00Z
modbus_devices:
- id: plts_1
  name: PLTS 1
  ip: 192.168.1.10
  port: 502
  unit_id: 1
  interval_ms: 1000
  endianness: big
  enabled: true
  registers:
  - name: voltage
    address: 40001
    register_type: float32
    register_area: holding
    scale: 1.0
  - name: power
    address: 40005
    register_type: float32
    register_area: holding
    scale: 1.0
device_templates:
- template_id: acme-pm100
  version: 1.0.0
  source: local
  manufacturer: Acme
  model: PM100
  protocol: modbus
  port: 502
  interval_ms: 1000
  unit_id: 1
  endianness: big
  registers:
  - name: voltage
    offset: 0
    register_type: float32
    register_area: holding
    scale: 1.0
scripts:
- name: boost
  enabled: true
  cpu_limit_ms: 1000
  memory_limit_bytes: 1048576
  source: |
    // double the setpoint
    return inputs[0] * 2;
tags:
- key: voltage
  device: plts_1
  name: Voltage
  unit: V
  min: 0.0
  max: 450.0
  base: 380.0
  source_type: modbus
  path: site/plts1/voltage
  alarm:
    warn: 400.0
    crit: 420.0
    low_warn: 350.0        # lives in source_config in the database
    deadband: 2.0
    on_delay_secs: 5       # ISA-18.2 delays
    off_delay_secs: 10
- key: power
  device: plts_1
  name: Power
  unit: kW
  min: 0.0
  max: 500.0
  base: 100.0
  source_type: modbus
  alarm: {warn: 400.0, crit: 450.0}
  write:
    writable: true
    min: 0.0
    max: 450.0
    max_rate: 5
    requires_reason: true
screens:
- id: plts_1
  name: PLTS 1
  tags:
  - {ref: plts_1/voltage}
  - {ref: plts_1/power, sort_order: 1}
  layouts:
    lg:
    - {tag: plts_1/voltage, x: 0, y: 0, w: 4, h: 3}
    - {tag: plts_1/power, x: 4, y: 0, w: 4, h: 3}
  scada:
    equipment:
    - equipment_id: gauge1
      equipment_type: gauge
      x: 10.0
      y: 20.0
      width: 100.0
      height: 60.0
      sensors: [plts_1/voltage]
flows:
- id: boost-on-high
  name: Boost on high voltage
  enabled: true
  trigger: {type: data_change, device_id: plts_1, metric: voltage}
  nodes:
  - {type: script, id: n1, script: boost, inputs: [voltage]}
  - {type: write_out, id: n2, tag: plts_1/power, policy: immediate, reason: 'flow:boost'}
notifications:
- source_key: voltage
  channel_type: webhook
  enabled: true
  config:
  - ${VOLTRUS_NOTIFICATIONS_VOLTAGE_WEBHOOK_CONFIG_0}
settings:
  alarm_cooldown_secs: 300
```

Fields that are omitted take the defaults of the database; fields equal to their
default may be left out of hand-written files, and are left out of exports where
that keeps files short (empty lists, zero delays, default colours, ...).
