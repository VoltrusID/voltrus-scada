# Flows

A flow is one trigger plus a graph of nodes. When the trigger fires, it sends a
message (`payload` plus string `metadata`) along its edges; each node works on
the message and sends the result out of one of its output ports, and the edges
from that port take it to the next nodes.

The flow editors in the web UI and the native console draw their palette and
configuration forms from the node catalogue (`GET /api/v1/flows/node-types`),
so a node type added on the server shows up in both without UI changes.

## Storage and the `nodes` document

A flow is stored as `{id, name, enabled, trigger_config, nodes}`.
`trigger_config` and `nodes` are JSON text. `nodes` is either the legacy
pipeline (v1: an array of nodes run in order) or a graph (v2):

```json
{ "version": 2,
  "nodes": [
    { "id": "n1", "type": "filter", "x": 120, "y": 80, "name": "Hot?", "condition": "value > 50" },
    { "id": "n2", "type": "notify", "x": 120, "y": 200, "channel": "email", "to": "ops@example.com",
      "message": "Temperature {{value}}" }
  ],
  "edges": [
    { "from": "trigger", "port": 0, "to": "n1" },
    { "from": "n1", "port": 0, "to": "n2" }
  ] }
```

* A node object holds `id`, `type`, its layout (`x`, `y`, optional `name`)
  and the fields of its type. Anything else an editor adds is kept.
* `"trigger"` is the source id of the flow's trigger. It has one output, port 0.
* `port` is the index of an output of the `from` node (default 0). Every
  node except the trigger has one input; any number of edges may enter it.
* Nodes without edges are allowed; they never run.

`GET /api/v1/flows` and `GET /api/v1/flows/{id}` always return `nodes` as v2
JSON text. `POST` and `PUT` take v1 or v2, as JSON text or as a JSON value
(the same goes for `trigger_config`), and store v2.

### Reading a v1 pipeline

A v1 array is converted to a chain in the order the v1 engine ran it:
processing nodes first, then the output nodes (`data_source_out`, `http_out`,
`alarm_out`, `write_out`), each group in array order. Nodes are placed in a
row, left to right (y 80, x 120, 400, 680, ...). Edges keep the old behaviour:

* `filter`: only `pass` (port 0) continues; `reject` has no edge, as v1
  stopped there.
* `threshold`: both `above` and `below` lead to the next node, as v1 always
  continued.
* `script`: `out` and `error` both lead to the next node (a failing script did
  not stop a v1 flow).
* `http_out` / `write_out`: only `out`; an error stops that branch and marks
  the run failed, as in v1.

When stored v1 data has nodes without an id or with a repeated id, reading
gives them unique ids (`n3`, `pump_2`). Submitting such a pipeline is refused.

### Validation

`POST`/`PUT /api/v1/flows`, `POST /api/v1/flows/test` and the MCP tools refuse
a flow with HTTP 400 and `{"error": "...", "node_id": "n1"}` when:

* the trigger type or its fields are invalid (`node_id` is null),
* a node has an unknown type, a missing or invalid field, or an expression that
  does not parse,
* two nodes share an id, or a node uses the reserved id `trigger`,
* an edge comes from or goes to an unknown node, enters the trigger, or uses a
  port the node does not have,
* the edges form a cycle.

`node_id` names the node the problem is about (for edge problems, the node the
edge leaves). Flow ids `test` and `node-types` are reserved.

## Running

When the trigger fires, the message is sent along every edge from `trigger`.
Nodes run breadth first. A node sends its result out of the port it chooses;
a port with several edges sends a copy down each. A node with several incoming
edges runs once per message that reaches it. One trigger message may cause at
most 1000 node runs; past that the run stops and is marked failed.

* A node that fails sends the message (unchanged, with `metadata.error` and
  `metadata.error_node`) out of its `error` port when it has one and that port
  is connected; the run still counts as successful. Otherwise that branch
  stops and the run is marked failed with the error. Other branches go on.
* Two runs of the same flow never overlap: a trigger that fires while a run
  is in progress is skipped (a `delay` node keeps the run in progress).
* Only the active node of a redundant pair runs flows.

Every device write from a flow (`write_out`) goes through the command queue
with the tag's write policy (writable, range, deadband, rate limit, reason),
attributed to `flow:<flow id>`, like a write from the API.

## Triggers

`trigger_config` is `{"type": ..., fields}`.

| Type | Fields | Fires | Payload |
|---|---|---|---|
| `manual` | none | Only from the run button or `POST /api/v1/flows/{id}/run`. | The run's `input.payload`. |
| `tag_change` | `data_source_ids` (one id or a list), `deadband` (default 0) | A tag of any protocol moved by more than the deadband since the value it last fired on. The first value after start or an edit sets the baseline and does not fire. Several tags of one poll give one run each, in turn. | `data_source_id`, `key`, `device_id`, `metric`, `value`, `previous`, `quality`, `ts` (ms) |
| `modbus` | `device_id` (`*` = any), `metric`, `debounce_ms`, `on_change_only` | Every poll of the device (any protocol, despite the name). | `device_id`, `status`, `timestamp`, `metrics`, `simulated` |
| `mqtt` | `topic` (`+`/`#` wildcards), `debounce_ms` | A message to the built-in broker. | The JSON body, or `{value}` |
| `alarm` | `on` (`raised`, `cleared`, `acknowledged`, `any`; default `raised`), `severity`, `source_key` | An alarm event broadcast to operators. | `event`, `alarm_id`, `source_key`, `severity`, `message`, `value`, `state`, `actor`, `ts` |
| `timer` | `interval_ms` | Every interval (checked once a second). | `timestamp` |
| `schedule` | `cron` | A five-field cron expression in the server's local time. | `timestamp` |
| `webhook` | none | `POST /api/v1/flows/{id}/webhook` with the flow's token. | The JSON body |

`metadata.source` names the trigger (`tag_change`, `alarm`, `webhook`,
`manual`, ...; `modbus` messages carry the device's protocol).

### Cron

`minute hour day-of-month month day-of-week`. Each field is `*`, a number, a
range `a-b`, a step `*/n`, `a-b/n` or `a/n`, or a comma list. Months and
weekdays take names (`jan`, `mon-fri`); weekday 0 and 7 are Sunday. When both
day fields are restricted, a day matching either fires (classic cron). A field
that starts with `*` counts as unrestricted, as in Vixie cron: `0 0 */2 * mon`
fires on Mondays that fall on an odd day, `0 0 1-31/2 * mon` on odd days and on
Mondays.
Up to v0.51 the first one fired on odd days and on Mondays; see
[upgrade notes](upgrade-notes.md) to keep that.
Shorthands: `@hourly`, `@daily` (`@midnight`), `@weekly`, `@monthly`,
`@yearly` (`@annually`). A schedule fires at most once per minute.

Daylight saving time: a schedule with a fixed minute and hour (`30 2 * * *`)
fires once when the clock goes back and repeats that time, and fires at the
change when the clock goes forward over it. A schedule with `*` in the minute
or hour field (`*/15 * * * *`, `30 * * * *`) follows the clock as it is.

### Webhooks

1. An admin creates the flow's token: `POST /api/v1/flows/{id}/webhook-token`
   returns `{"token": "..."}`. It is shown once; only its SHA-256 is stored.
   Calling it again replaces the token.
2. The caller posts JSON to `POST /api/v1/flows/{id}/webhook` with the token in
   the `X-Flow-Token` header (no session or API key). The body becomes the
   payload; an empty body gives `{}`.

Answers: 202 `{accepted: true}` (the run happens in the background), 400 when
the body is not JSON, 401 for a missing or wrong token, 409 when the flow is
stopped, its trigger is not `webhook`, or a run is in progress, 429 past 30
accepted calls per flow per minute, and 429 instead of 401 for a client
address's missing or wrong tokens past 30 in a minute. Wrong tokens do not use
up the flow's share, and a caller with the right token is never turned away
because of them.

## Nodes

`GET /api/v1/flows/node-types` is the authoritative list (fields, defaults,
help text). Ports are listed in port order.

Each type has `type`, `label`, `description`, `fields` and, for nodes,
`category`, `outputs` and `side_effect`; `switch` also has
`dynamic_outputs: "rules"` (one output per rule before `outputs`). A field has
`key`, `label`, `kind` (`string`, `number`, `bool`, `select`, `expression`,
`code`, `tag`, `device`, `json`, `duration_ms`, `cron`, `severity`),
`required`, `help`, and when they apply `default`, `options` (`select`),
`multiple: true` (a `tag` field that takes a list of ids, such as the
`tag_change` trigger's `data_source_ids`) and `item_fields` (a `json` list of
objects, such as switch and threshold rules: the fields of one item).

| Type | Category | Outputs | Side effect | What it does |
|---|---|---|---|---|
| `filter` | logic | pass, reject | no | `condition` (expression) true: pass, otherwise reject. |
| `switch` | logic | one per rule, then otherwise | no | `rules: [{name, expression}]` checked in order. `mode: first` (default) sends the message out of the first matching rule's port, `all` out of every matching rule's port. No match: `otherwise`, the last port. |
| `threshold` | logic | above, below | no | Numeric `field` against `rules: [{gt, gte, lt, lte, eq, set}]`; the first rule whose bounds all hold sets its `set` fields and the message leaves on `above`. No match: `below`. |
| `on_change` | logic | changed, unchanged | no | `field` moved by more than `deadband` (default 0) since the last message that passed. The first message passes. Non-numbers pass on any change. |
| `transform` | transform | out | no | New payload from `map: {output path: expression}`. |
| `expression` | transform | out | no | Sets `output_field` to `expr`. |
| `lookup` | transform | out | no | Looks `field` up in `table` and stores the result (or `default`) in `output_field`. |
| `dsl` | advanced | out | no | Runs a DSL program (`source`); a program that returns nothing drops the message. |
| `script` | advanced | out, error | no | Runs saved script `script_id` with the payload fields in `inputs`; merges its result into the payload. A failing script goes to `error`. |
| `delay` | timing | out | no | Waits `duration_ms` (at most one hour). |
| `rate_limit` | timing | out, dropped | no | At most `max` messages per `duration_ms`. `mode: drop` (default) sends extra messages out of `dropped`; `keep_latest` holds the newest extra one and sends it out of `out` when the window allows (as a new run). |
| `write_out` | output | out, error | yes | Writes to tag `data_source_id` through the command queue and its write policy. Value: `value_expr`, or the first number in the payload. Refusals go to `error`. |
| `alarm_out` | output | out | yes | Raises an alarm (`severity`, `source_key`, `message`). |
| `notify` | output | out, error | yes | Sends `message` (and `subject`) through `channel`: `settings` (every notification setting of `source_key`: email, webhook, chat), `email` or `webhook` (to the comma-separated `to`). Failures go to `error`. |
| `http_out` | output | out, error | yes | POSTs the payload as JSON to `url` (`headers`, `timeout_ms` default 5000, `retry_count`). No redirects; the address rules of `outbound_http` (README, Network behaviour) apply. Header values and the URL password read as `********` to roles that may not configure; saved back masked, they keep the stored values only while the URL keeps its scheme, host and port. |
| `data_source_out` | output | out | yes | Publishes numeric `value_field` as metric `source_key` of device `flow_<flow id>`. |
| `debug` | advanced | out | no | Records the message (or payload path `field`) in the run trace; passes it on unchanged. |

Expressions read payload fields by name or path (`metrics.temp > 50 &&
status == "online"`). In `notify` messages, `{{path}}` inserts a payload value,
`{{payload}}` the whole payload, `{{flow_id}}`, `{{error}}` (the error that
sent the message to an error port) and `{{meta.<key>}}` a metadata value.

## Test runs

`POST /api/v1/flows/test` runs a definition (saved or not) once, as a dry run:

```json
{ "flow": { "trigger_config": "{\"type\":\"manual\"}", "nodes": "{\"version\":2,...}" },
  "input": { "payload": { "value": 72 }, "metadata": {} } }
```

`trigger_config` and `nodes` may be JSON text or values. The answer:

```json
{ "ok": true, "error": null,
  "trace": [ { "node_id": "n1", "type": "filter", "input": {"value": 72}, "output": {"value": 72},
               "port": 0, "ports": [0], "error": null, "duration_us": 41, "side_effect_skipped": false } ],
  "outputs_reached": ["n2"] }
```

* Side-effect nodes (`write_out`, `http_out`, `notify`, `alarm_out`,
  `data_source_out`) do nothing; their `output` says what they would have done
  (`would_write`, `would_post` with header names only, `would_notify`,
  `would_raise_alarm`, `would_set`) and `side_effect_skipped` is true.
  `write_out` still checks the tag is writable and the value is in its write
  range, and fails as the real write would.
* `delay` does not wait. `on_change` and `rate_limit` start from empty memory
  and do not change what live runs remember. Scripts run, but are not logged.
* `outputs_reached` lists the nodes where a message ended: it left on a port
  with no edges, or the node sent nothing on.

`POST /api/v1/flows/{id}/test` with `{payload, metadata}` does the same for a
saved flow (its answer also has the older `success`, `output` and
`node_results` fields).

## Manual runs

`POST /api/v1/flows/{id}/run` with an optional `{"input": {"payload", "metadata"}}`
runs a saved flow for real now, whatever its trigger and whether it is started,
and answers with the same shape as a test run. The run is in the run log. 409
when a run of the flow is in progress, 503 on the passive node of a pair.

## Run log

`GET /api/v1/flows/{id}/runs` lists the 20 newest runs: `started_at` (seconds),
`duration_ms`, `success`, `error`, `output` (the payload that ended the run)
and, for the 50 newest runs of each flow, `trace`: the per-node entries above,
with inputs and outputs over 2 KB replaced by `{"truncated": true, "bytes": n}`
and at most 200 entries.

## Access

| Route | Who |
|---|---|
| `GET /api/v1/flows`, `/flows/{id}`, `/flows/{id}/runs`, `/flows/node-types` | Any signed-in user or API key |
| `POST /api/v1/flows`, `PUT`/`DELETE /flows/{id}`, `/start`, `/stop`, `/run`, `/test`, `/flows/test` | Admin, demo (operator) and integration keys |
| `POST /api/v1/flows/{id}/webhook-token` | Admin |
| `POST /api/v1/flows/{id}/webhook` | Anyone with the flow's token |

## Example: alarm escalation

Raise a page only for critical alarms on the boiler, and at most once every 10
minutes:

```json
{ "id": "boiler_page", "name": "Page on boiler alarms", "enabled": true,
  "trigger_config": {"type": "alarm", "on": "raised", "source_key": "boiler.temp"},
  "nodes": { "version": 2,
    "nodes": [
      {"id": "sev", "type": "switch", "rules": [{"name": "critical", "expression": "severity == \"critical\""}]},
      {"id": "limit", "type": "rate_limit", "max": 1, "duration_ms": 600000},
      {"id": "page", "type": "notify", "channel": "settings", "source_key": "boiler.temp",
       "severity": "critical", "message": "Boiler: {{message}} ({{value}})"},
      {"id": "log", "type": "debug"}
    ],
    "edges": [
      {"from": "trigger", "to": "sev"},
      {"from": "sev", "port": 0, "to": "limit"},
      {"from": "sev", "port": 1, "to": "log"},
      {"from": "limit", "port": 0, "to": "page"}
    ] } }
```

## Example: setpoint from a webhook

```json
{ "id": "mes_setpoint", "name": "Setpoint from MES", "enabled": true,
  "trigger_config": {"type": "webhook"},
  "nodes": { "version": 2,
    "nodes": [
      {"id": "w", "type": "write_out", "data_source_id": 12, "value_expr": "setpoint", "reason": "MES order"},
      {"id": "fail", "type": "notify", "channel": "email", "to": "shift@example.com",
       "subject": "Setpoint refused", "message": "{{error}}"}
    ],
    "edges": [
      {"from": "trigger", "to": "w"},
      {"from": "w", "port": 1, "to": "fail"}
    ] } }
```

Then `curl -X POST -H "X-Flow-Token: $TOKEN" -H 'content-type: application/json'
-d '{"setpoint": 42}' https://scada.local/api/v1/flows/mes_setpoint/webhook`.

## MCP

`list_flows`, `create_flow` (v1 or v2 nodes), `test_flow` (dry run),
`flow_node_types`, `start_flow`, `stop_flow`. See [mcp.md](mcp.md).

## Not in this version

* `notify` has no browser push channel: the server stores push subscriptions
  but has no sender for them. Use `settings`, `email` or `webhook`.
* Project files (`voltrus.yaml`) carry flows with v2 `nodes` as they are;
  `write_out` and `script` node ids and `tag_change` trigger ids are rewritten
  to tag and script names on export and back on import.
