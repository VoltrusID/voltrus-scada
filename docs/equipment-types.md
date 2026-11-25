# Equipment types and instances

An **equipment type** (Pump, Valve, Tank, Analog instrument, ...) says once what
one piece of equipment of that kind is: which tags it has, named by role (`run`,
`fault`, `cmd`, `speed`) with a key pattern and optionally a register offset;
their limits, alarm thresholds and write policy; the faceplate it opens; the mimic
symbol that draws it and which role drives which part of the symbol; and,
optionally, flows (interlocks) copied per instance.

An **equipment instance** is one line:

```yaml
- {name: P-101A, type: pump, device: wtp}
```

Instantiating binds every role to its tag (creating registers and tags when the
instance gives a register base), puts the type's limits and alarms on those tags,
and binds every SCADA symbol that names the instance. Changing the type changes
every instance. This is the datapoint-type idea of WinCC OA, expressed with
Voltrus' tags, faceplates and symbols. Code: `src/equipment_types/` (model,
resolution, starter library), `src/project_yaml/equipment.rs` (expansion),
`src/api/equipment_types.rs` (routes), `src/mcp/tools/equipment.rs`.

## Type

```json
{
  "id": "pump",
  "name": "Pump",
  "description": "Centrifugal pump with run feedback, trip, start/stop and VFD speed.",
  "category": "Pumps",
  "symbol": "isa_pump_centrifugal",
  "faceplate": "pump",
  "tags": [
    {"role": "run",   "label": "Running", "kind": "state",   "key": "{prefix}_run",   "offset": 0, "min": 0, "max": 1, "decimals": 0, "required": true},
    {"role": "fault", "label": "Fault",   "kind": "fault",   "key": "{prefix}_fault", "offset": 2, "alarm": {"crit": 1}},
    {"role": "cmd",   "label": "Start / stop", "kind": "command", "key": "{prefix}_cmd", "offset": 4, "write": {"writable": true, "min": 0, "max": 1}, "fallback": "run"},
    {"role": "speed", "label": "Speed",   "kind": "analog",  "key": "{prefix}_speed", "offset": 6, "unit": "Hz", "min": 0, "max": 50, "decimals": 1}
  ],
  "bindings": {"primary": "run", "fault": "fault"},
  "symbol_config": {"showStatus": true, "writable": true},
  "flows": []
}
```

| Field | Meaning |
|---|---|
| `id` | 1-64 lowercase letters, digits, `_`, `-`; not `library` (a route) |
| `symbol` | a symbol key: a file of the shared library `symbols/<key>.json` or a symbol the web client draws in code (`pump`, `tank`, `isa_pump`, ...); the server knows the list (build.rs) and refuses others |
| `faceplate` | id of a faceplate definition ([screens.md](screens.md)), or `null`. Role names of the type and the faceplate should match: the symbol's `faceplateMap` is keyed by role. A faceplate a type opens cannot be deleted (409) |
| `tags[].role` | unique in the type |
| `tags[].kind` | `state`, `fault`, `analog`, `command`, `setpoint` (the faceplate role kinds) |
| `tags[].key` | tag key pattern, default `{prefix}_{role}` |
| `tags[].name` | tag name pattern; when set, it is applied to the tag. A tag made by instantiating without one is named `{name} {label}` |
| `tags[].offset` | register offset from the instance's `base` (Modbus); with `register_type` (default `float32`), `register_area` (default `holding`), `scale` (default 1) |
| `tags[].unit`, `min`, `max`, `decimals` | applied to the tag when set; left alone when not |
| `tags[].alarm` | `warn`, `crit`, `low_warn`, `low_crit`, `deadband`, `on_delay_secs`, `off_delay_secs`: each one given is applied |
| `tags[].write` | `writable`, `min`, `max`, `max_rate`, `requires_reason`: each one given is applied. A tag made writable without write limits (by the type, the override or the tag itself) gets its own `min`/`max` as write limits: a type never opens a tag to any value |
| `tags[].required` | an instance without a tag for this role cannot be made |
| `tags[].fallback` | another role whose tag this role uses when it has none of its own (a start command written to the run tag). Fallbacks do not chain |
| `bindings` | roles that drive the symbol: `primary` (the symbol's tag: state, level or value), `fault`, `run` (flow, agitator, backwash), `manual`, `interlock` |
| `symbol_config` | config defaults of every symbol of the type's instances (`showStatus`, `writable`, `showValue`, ...): set where a symbol has no value of its own, see below. The keys the binding owns (`equipment`, `symbol`, `faceplate`, `faceplateMap`, `label`, `*SensorId`) cannot be set here |
| `flows` | flow templates (`{id, name, enabled, trigger, nodes}`, as in [flows.md](flows.md)); each instance gets a copy with id `<prefix>-<id>` |

**Patterns** use `{prefix}`, `{name}`, `{device}`, `{role}` and `{label}`; flow
strings also `{ref:<role>}` (the tag's `device/key`) and `{key:<role>}`. An unknown
placeholder or an unclosed `{` is an error naming the field.

An interlock is a flow: "trip stops the pump" is

```json
{"id": "trip-stop", "name": "{name}: stop on trip", "enabled": true,
 "trigger": {"type": "tag_change", "tags": ["{ref:fault}"]},
 "nodes": [{"type": "write_out", "id": "n1", "tag": "{ref:cmd}", "policy": "immediate", "reason": "interlock"}]}
```

A flow naming a role the instance has no tag for is left out for that instance,
with a note.

## Instance

```json
{"name": "P-501A", "type": "pump", "device": "wtp", "base": null, "prefix": "p301a",
 "description": "", "symbol": null,
 "overrides": {"fault": {"skip": true}, "speed": {"unit": "rpm", "max": 1500}}}
```

| Field | Meaning |
|---|---|
| `name` | natural key; no `/`; unique without case (`p-101a` next to `P-101A` is refused) |
| `device` | the device its tags belong to (any protocol) |
| `base` | register base address; only for Modbus devices |
| `prefix` | `{prefix}` of the patterns; default: the name in lower case with everything but letters and digits dropped (`P-101A` → `p101a`). Unique without case, as the flow ids made from it are lower case: `P101A` (prefix `p101a`) next to `P-101A` is refused, naming `P-101A` |
| `symbol` | another symbol than the type's |
| `overrides.<role>` | `key` (pattern), `name`, `unit`, `min`, `max`, `decimals`, `alarm`, `write` over the type's; `skip: true` leaves the role out (not for required roles) |

## Resolution

For every role of the type, in order:

1. `skip` → **skipped**.
2. The key is the override's pattern or the type's, expanded.
3. If the instance has a `base`, the role an `offset` and the device is a Modbus
   device without a register of that key, a register is added at
   `base + offset` (a device that follows a device template refuses). It may not
   overlap any register of the device in the same area, counting the width of
   each type (`int16`/`uint16` one register, `int32`/`float32` two, `float64`
   four; coils and discrete inputs one bit): `P-202` at base 102 next to `P-201`
   at base 100 is refused, naming `p201_fault`. When the register exists at
   another address, it is moved to `base + offset` only if it is the instance's
   own (where its previous base and offset put it: the base or an offset
   changed); the dry run lists the move in `registers_moved`. A register of that
   name somewhere else is not taken over: an error says where it is.
4. A tag `device/key` exists → **bound**. Otherwise, when its register exists or
   is being added → **created** (a tag with `source_type: modbus`).
5. Otherwise a role with a `fallback` whose role has a tag → **fallback** (the
   fallback's tag, nothing applied to it). Otherwise → **missing**; a required
   role missing is an error that names the key looked for.

The type's attributes and the override are applied to bound and created tags,
only the fields they set. Two instances giving the same tag is an error, checked
against every stored instance inside the write's transaction.

A role of kind `command` that falls back to a `state` role (a pump's start/stop
written to its run feedback, as the WTP showcase does) is allowed, with a note in
`resolved.notes` and the change's `notes`: commands then go to the feedback tag.

Each instance gets a copy of the type's flows, `<prefix>-<id>`. A flow of that id
the instance did not make (a user's flow) is never overwritten: the write is
refused. When the prefix changes, or the type drops a flow, the instance's old
copies are deleted (`flows_removed`).

## Symbols bound to an instance

A SCADA widget with `config.equipment: "<instance>"` is bound to it whenever it
is saved (`PUT .../scada-layout`, `PUT .../scada-equipment`, project import) and
whenever its instance or type changes:

* `equipment_type: "symbol"`, `config.symbol` = the instance's symbol or the
  type's;
* `sensor_bindings` = the `primary` role's tag;
* `config.faultSensorId`, `runSensorId`, `manualSensorId`, `interlockSensorId`
  from `bindings` (removed when the role has no tag);
* `config.faceplate` = the type's faceplate, `config.faceplateMap` = role → tag
  for every role with a tag;
* the type's `symbol_config` as defaults: a key is set where the widget has no
  value of its own; a widget's own value (say `writable: false` on one pump) is
  kept on every save and type change. When the type changes a default, widgets
  still holding the old default follow it; a default the type drops is removed
  from widgets that still hold it;
* `config.label` = the instance name when the widget has no label.

`sensor_bindings[0]` is the primary role's tag; the widget's other tags stay
after it (those that are not the instance's own: its role map and indicator
tags). Everything else in the row (position, size, z-order, a label) is kept.
Binding happens in the transaction that saves the screen.
Binding is idempotent: saving a bound screen again stores the same rows and the
same revision. A widget naming an instance that does not exist is saved as it is.

## REST

Reads need a session or key; writes the roles that configure screens (as
faceplates: admin and demo; operators and viewers get 403). Every write is
audit-logged with every tag attribute it changed, before and after.

Every write that is not a dry run takes a configuration snapshot first (label
`pre-equipment`), which covers equipment types and instances, tags, Modbus
devices and registers, flows and SCADA rows, so
`POST /api/v1/config/snapshots/{id}/rollback` undoes it. A write whose expansion
changes the write policy (`write.*`) of any tag needs the electronic signature
`PATCH /data-sources/{id}/write-policy` needs, when the edition asks for one: 428
with `signature_required` until the body carries `signature`; an API key or MCP
client cannot sign. The write is applied only if the policy changes found in its
transaction are the ones signed (409 otherwise).

| Route | What |
|---|---|
| `GET /api/v1/equipment-types` | types, each with `instance_count` |
| `GET /api/v1/equipment-types/{id}` | one type |
| `POST /api/v1/equipment-types` | create; 201, 409 when the id exists (checked in the transaction), 400 naming every problem (`tags[2].kind 'lamp' is not one of ...`, `unknown faceplate 'x'`) |
| `PUT /api/v1/equipment-types/{id}?dry_run=` | change; answers `{type, dry_run, affected, warnings}` |
| `DELETE /api/v1/equipment-types/{id}` | 409 while instances exist |
| `GET /api/v1/equipment-types/library` | starter types: `[{type, faceplate}]` |
| `POST /api/v1/equipment-types/library/install` | `{ids?}` → `{installed, skipped}`; creates the faceplates they open when missing |
| `GET /api/v1/equipment-instances?type=` | instances, each with `resolved` |
| `GET /api/v1/equipment-instances/{name}` | one instance with `resolved` |
| `POST /api/v1/equipment-instances?dry_run=` | instantiate; 201 (200 for a dry run) with `resolved` and `changes`; 409 when the name exists (checked in the transaction: of two concurrent creates the second gets 409); 400 for a tag, prefix, register or flow another instance or a user has |
| `PUT /api/v1/equipment-instances/{name}?dry_run=` | change type, device, base, prefix, symbol, overrides |
| `DELETE /api/v1/equipment-instances/{name}?dry_run=` | deletes the instance and its generated flows, and unbinds its widgets (`config.equipment` cleared, `config.unboundEquipment` = the old name; the symbol keeps its tags and stays drawn), so a later instance of the same name does not take them over. Its tags and registers stay. Answers `{flows_deleted, widgets_unbound}`; the dry run lists them and writes nothing |

`resolved`:

```json
{"prefix": "p101b", "symbol": "isa_pump_centrifugal", "faceplate": "pump",
 "roles": [
   {"role": "run",   "kind": "state",   "label": "Running", "key": "p101b_run",   "ref": "wtp/p101b_run",   "tag_id": 12, "status": "bound"},
   {"role": "fault", "kind": "fault",   "label": "Fault",   "key": "p101b_fault", "ref": "wtp/p101b_fault", "tag_id": 13, "status": "bound"},
   {"role": "cmd",   "kind": "command", "label": "Start / stop", "key": "p101b_run", "ref": "wtp/p101b_run", "tag_id": 12, "status": "fallback"},
   {"role": "speed", "kind": "analog",  "label": "Speed",   "key": "p101b_speed", "ref": "wtp/p101b_speed", "tag_id": null, "status": "missing"}],
 "widget": {"equipment_type": "symbol", "sensor_id": 12,
            "config": {"equipment": "P-101B", "symbol": "isa_pump_centrifugal", "faceplate": "pump",
                       "faceplateMap": {"run": 12, "fault": 13, "cmd": 12}, "faultSensorId": 13,
                       "showStatus": true, "writable": true, "label": "P-101B"},
            "remove": ["runSensorId", "manualSensorId", "interlockSensorId"]},
 "problems": [], "notes": []}
```

`widget` is what the Designer applies to a symbol it places for the instance:
set `config`'s keys (`label` only when the symbol has none), remove `remove`, bind
`sensor_id`. The server binds the same way on save.

`changes` (one instance) and each entry of `affected` (a type change):

```json
{"instance": "P-101B", "type": "pump",
 "tags": [{"key": "p101b_fault", "ref": "wtp/p101b_fault", "action": "update", "fields": ["alarm"],
           "diff": [{"field": "alarm.crit", "before": 0.0, "after": 1.0}]},
          {"key": "p101b_run", "ref": "wtp/p101b_run", "action": "unchanged"}],
 "registers_added": ["skid/p201_run@100"],
 "registers_moved": ["skid/p201_fault@102->202"],
 "flows": ["p101b-trip-stop"],
 "flows_updated": ["p101b-trip-stop"],
 "flows_removed": ["oldprefix-trip-stop"],
 "widgets": [{"screen_id": "overview", "equipment_id": "e004"}]}
```

`action` is `create`, `update` (with the fields the type changes), `unchanged`, or
`bind` (a fallback, or a tag a project file lists as written). `diff` lists every
attribute set, before and after (`write.*` are the write policy; `before` is null
for a new tag). `flows_updated` are the flows of `flows` that existed and are
overwritten. A stored instance that cannot be expanded any more when its type
changes (its device was deleted, ...) is skipped with a warning naming it; the
others are updated.

**A changed type reaches every instance.** `PUT /equipment-types/{id}` with
`dry_run=true` lists every instance and what the change does to it, writing
nothing (the same preview as linked device templates). Without it, the type is
saved and every instance is expanded again in the same transaction: tags get the
new attributes, missing tags and registers are created, flows are updated and
every bound symbol is rebound. Modbus pollers of changed devices restart and
changed flows are loaded into the running engine.

Under the hood every write is a small merge import of the project format (the
type or instance as an entity), so the routes, the MCP tools and project files
share one expansion and one validation.

## MCP

`list_equipment_types` (`library: true` for the starter set),
`create_equipment_type` (`type`, or `from_library`), `update_equipment_type`
(dry run by default; returns `affected`), `instantiate_equipment` (dry run by
default, marked destructive: it can change existing tags and flows; `changes`
lists them), `list_equipment_instances` (`type`). The write tools need a
configuring principal (admin, demo or an integration key). A change of a write
policy needs a signature where the edition asks for one, which an MCP client
cannot give. See [mcp.md](mcp.md).

## Project format

Sections `equipment_types` (key `id`) and `equipment_instances` (key `name`),
expanded at plan time; see [project-format.md](project-format.md#equipment-types-and-instances).

## Starter library

Shipped with the server (`src/equipment_types/library.json`), installed on
request; installed types are ordinary types.

| Type | Symbol | Faceplate | Roles (key) |
|---|---|---|---|
| `pump` Pump | `isa_pump_centrifugal` | `pump` | run `{prefix}_run`*, fault `{prefix}_fault` (alarm crit 1), cmd `{prefix}_cmd` (falls back to run), speed `{prefix}_speed` (Hz) |
| `valve` Valve | `isa_valve_gate` | `valve` | open `{prefix}_open`*, fault, cmd (falls back to open), position `{prefix}_position` (%) |
| `control_valve` Control valve | `isa_valve_control` | `control_valve` | position `{prefix}_pos`* (%), position_sp `{prefix}_pos_sp`, fault |
| `tank` Tank | `isa_tank_vertical` | `tank` | level `{prefix}_level`* (%), high `{prefix}_lsh`, low `{prefix}_lsl` |
| `analog` Analog instrument | `isa_instrument` | `analog` | pv `{prefix}`*, sp `{prefix}_sp` |
| `dosing_pump` Dosing pump | `isa_pump_dosing` | `dosing_pump` | run `{prefix}_pump_run`*, fault `{prefix}_pump_fault`, cmd (falls back to run), dose `{prefix}_dose` (mg/L), dose_sp `{prefix}_dose_sp` |
| `filter` Filter | `isa_filter` | `filter` | headloss `{prefix}_headloss`* (m), turbidity `{prefix}_turbidity` (NTU), backwash `{prefix}_backwash` (drives the symbol's run state) |

`*` required. The command roles of `pump`, `valve` and `dosing_pump` are
writable 0/1 (start/stop, open/close; without a tag of their own they fall back
to the feedback, with a note). The setpoints (`analog.sp`, `dosing_pump.dose_sp`,
`control_valve.position_sp`) are writable within their tag's range;
`analog.sp` and `dosing_pump.dose_sp` need a reason, and the dose setpoint takes
at most 10 writes a minute. Narrow them per instance (`overrides.<role>.write`).

Every role has a register offset (float32, two registers apart), so
on a new Modbus device `base` lays them out; the key patterns follow the WTP
showcase, whose register map is not grouped per equipment, so there instances bind
by key without a base: `{name: P-101A, type: pump, device: wtp}` binds
`p101a_run`; `{name: P-201, type: dosing_pump, device: wtp, prefix: coag}` binds
`coag_pump_run`, `coag_dose`; `{name: AT-102, type: analog, device: wtp, prefix:
raw_turbidity}` binds `raw_turbidity`.
