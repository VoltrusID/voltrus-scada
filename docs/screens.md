# Values on screens

How the web client shows live values on dashboards (grid and canvas) and on 2D
SCADA screens: state colours, the trend popup and tag-swap faceplates.

## State colours

Every value bound to a tag shows its alarm state and its data quality, the same
way on every surface and with no setting per widget. One style map in the client
(`client/src/valueStyle.ts`) decides it:

| State | When | Colour (theme variable) | Marker | Label |
|---|---|---|:-:|---|
| Critical | the value is past a critical limit (high or low) | `--crit` | `!` | CRITICAL |
| Warning | the value is past a warning limit | `--warn` | `!` | WARNING |
| Bad | quality `bad`, `bad_comm_failure`, `bad_device_error`, `bad_config_error` | `--hmi-stale` | `✕` | BAD |
| Uncertain | quality `uncertain`, `uncertain_stale` | `--hmi-stale` | `?` | UNCERTAIN |
| Offline | the device is unreachable and there is no value | `--text-muted` | | OFFLINE |
| Normal | none of the above | the widget's own colour | | |

Quality wins over the alarm level, because the server evaluates alarms on good
values only (a stale value past a limit is shown as uncertain, not as an alarm).
The live stream spells the status `crit`/`warn` and the REST snapshot
`critical`/`warning`; both map to the same state.

Where it shows:

* **Dashboard cards:** the value text, the status dot and badge, and the card
  border take the state colour; a bad or uncertain value gets its marker.
* **2D SCADA screens:** at runtime every bound widget except areas, labels and
  pipes gets a frame in the state colour (dashed for uncertain and offline) and a
  round marker badge at its top-right corner, with the state in the tooltip. The
  big number, indicator and data table widgets also colour the value and show the
  label. Equipment symbols keep their own run/stop/fault colours inside the frame.
* **Faceplates** (below) colour analog values the same way.

Colour is never the only cue: each state has a marker and a label for screen
readers. Both themes define every colour, and a client test checks that each
reaches 4.5:1 contrast on the background and on cards (`tests/contrast.test.ts`).

Limits: add-on widgets drawn by an edition (not the core widgets) decide their own
look; the 3D twin does not use the map.

## Tag-swap faceplates

A faceplate is the popup an operator opens by clicking equipment on a 2D SCADA
screen. Without further setup a symbol opens the built-in faceplate for its one
bound tag (state, value against its limits, trend, start/stop). A **faceplate
definition** describes the popup for an equipment kind once, with named roles; each
symbol only stores which of its tags fills which role. Three pumps are one
definition and three short mappings.

A definition (`GET`/`POST /api/v1/faceplates`, `GET`/`PUT`/`DELETE /api/v1/faceplates/{id}`):

```json
{"id": "pump", "name": "Pump", "kind": "pump",
 "roles": [
   {"role": "run",   "label": "Running",      "kind": "state",   "required": true},
   {"role": "fault", "label": "Fault",        "kind": "fault"},
   {"role": "cmd",   "label": "Start / stop", "kind": "command"},
   {"role": "speed", "label": "Speed",        "kind": "analog"}]}
```

| Field | Values |
|---|---|
| `id` | 1-64 lowercase letters, digits, `_`, `-` |
| `kind` | `pump`, `motor`, `valve` (OPEN/CLOSE instead of START/STOP), `analog` |
| `roles[].kind` | `state` (on/off feedback), `fault` (on = tripped, blinks), `analog` (value in the state colours; click for the trend), `command` (writes 1 or 0 after an arm-and-confirm step), `setpoint` (the operator types a value) |
| `roles[].required` | the editor warns while an instance leaves it unmapped |

Unknown fields and values outside these lists are refused with `400` naming the
field. A definition that symbols still use cannot be deleted (`409` lists them);
`GET /api/v1/faceplates/{id}` returns them as `used_by`. Changing a definition
(new role, other label) changes the popup of every symbol that uses it.

An instance is a symbol whose config carries the definition id and a role map of
tag ids:

```json
{"symbol": "pump", "faceplate": "pump", "faceplateMap": {"run": 41, "fault": 42, "cmd": 43}}
```

**In the editor:** select a symbol, then in the inspector's *Faceplate* section
pick a definition and a tag for each role. If none exists yet, *+ Pump*, *+ Valve*
and *+ Analog measurement* save a starting definition. An unbound symbol takes
the first state or analog tag mapped as the tag it draws. **At runtime** clicking
the symbol opens the definition with that symbol's tags. Commands and setpoints go
through the command queue and write policy like every other write, and need the
operate permission.

**In a project file** definitions are the `faceplates` section, and a symbol's
`config.faceplateMap` names tags by `ref` (`device/key`), like its `sensors`
([project-format.md](project-format.md)).

Limits: one definition has no graphics of its own (the symbol draws the equipment);
roles are single tags (no arrays or nested equipment); MCP has no faceplate tools
yet (an agent writes definitions and mappings through a project import).
