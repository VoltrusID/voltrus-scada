# Live data stream

The web client and API clients get live values, command acknowledgements, device
status and alarm events over one of two endpoints. Both sit behind the same
authentication as the REST API and carry the same messages.

| | WebSocket | Server-sent events |
|---|---|---|
| Endpoint | `GET /api/v1/ws` | `GET /api/v1/stream` |
| Encoding | MessagePack (binary frames) | JSON (`text/event-stream`) |
| Direction | both: subscribe, tag writes | server to client only |
| Authentication | session cookie sent with the upgrade, or an `Auth` message with a token as the first frame | session cookie or `X-API-Key`, like any REST route |

## Messages

Each message has a `type`: `AuthOk`, `TagUpdateBatch`, `TagUpdate`, `CommandAck`,
`DeviceStatus`, `Alarm`, `AlarmUpdate`, `HaStatus`. On the event stream the event
name is the `type` and the data is the message as JSON, for example:

```
event: TagUpdateBatch
data: {"type":"TagUpdateBatch","updates":[{"tag_id":"inverter-1.power","value":412.5,"quality":"normal","ts":1761700000,"simulated":false,"data_quality":"good","ts_ms":1761700000123}]}
```

The first event of a stream is `AuthOk` (the signed-in user and permissions), as on
the WebSocket. A comment line is sent every 15 seconds to keep proxies from closing an
idle stream. A value that is NaN (a failed read) is `null` in JSON.

The user or key behind a stream is checked again every 30 seconds, as on the
WebSocket: when the user is disabled or deleted, must change their password,
has another role than when the stream opened, or the sign-in session has ended
(signed out, password changed elsewhere, revoked; or, for a stream opened with
an access token only, that token expired), or the key is revoked or expired,
the stream sends an `AuthError` event and ends. The client reconnects with
what it has now.

Each tag update may also carry `data_quality` and `ts_ms`. `ts_ms` is when the value
was read, in Unix milliseconds. `data_quality` is one of `good`, `uncertain`,
`uncertain_stale` (not refreshed for three poll intervals), `bad`,
`bad_comm_failure` (the device is unreachable; the value is the last one read),
`bad_device_error` or `bad_config_error`. A missing `data_quality` means `good`.
The older `quality` field is the tag's status (`normal`, `warning`, `critical`,
`offline`) and is kept as
it was.

## Writing a tag

A write sent over the WebSocket (`TagWrite`: `tag_id` as the numeric data source id,
`value`, optional `command_id` and `reason`) takes the same road as
`POST /api/v1/tags/{id}/write`: the tag's write policy is checked (writable, mandatory
reason, range, deadband, rate limit), the command is queued, and the audit trail
records the user and the reason. A refused write is answered at once with a `CommandAck`
of status `rejected` and the reason. An accepted one is answered when the command is
final: the first `CommandAck` the requester receives for its `command_id` is `acked`
(the device took it), `failed` (retries used up) or `expired` (the device stayed
unreachable for 30 minutes), so a client may take that first acknowledgement as the
outcome, as it could when writes went straight to the device. The intermediate `sent`
and `queued` (a retry) acknowledgements are broadcast to every other connection but not
to the requester. A device that is offline leaves the write queued, so a client should
stop waiting after a timeout and show it as queued (the web client does after 15 s).

After `acked`, the server checks the device really holds the value: the next fresh poll
of good quality is compared with the written value. One more `CommandAck` follows, with
status `readback_confirmed`, `readback_mismatch` (two fresh polls disagreed) or
`readback_timeout` (no fresh good value within three poll intervals or 10 s). An acked
value is never written into the live cache: the screen shows it once a poll confirms it.

The same roles may write over the WebSocket as over REST and MCP: admins, operators,
demo users and integration API keys (viewers and read-only keys may not). The user or
key is checked again on every write and
every 30 seconds; a socket whose user is disabled or deleted, whose role or
sign-in session changed as above, or whose key is revoked, is closed with code
1008. An application-level `Ping` message is answered with `Pong`. Clients that are not on the WebSocket (including the event stream) write
with the REST endpoint.

### Who sent a command

Every queued command records who sent it (`created_by`: the user name,
`api_key:<id>` or `flow:<id>`) and, from v0.53, through which front end
(`origin`: `session`, `api_key`, `mcp`, `flow` or `recipe`).
`GET /api/v1/commands`, `GET /api/v1/commands/{id}` and the MCP
`get_command_status` tool return both plus a display form,
`source: {kind, label}`, with the name of the API key or flow looked up:

```json
{"created_by": "api_key:4", "origin": "mcp",
 "source": {"kind": "mcp", "label": "MCP (API key line-2 gateway)"}}
```

The command panel shows it as a badge (`USER`, `API KEY`, `MCP`, `FLOW`, `RECIPE`)
and the name on every command and on each step of its audit trail; the alarm
audit tab and acknowledged alarms show who acted the same way. Commands written
before v0.53 have no `origin` and are classified from `created_by` (so an MCP
write from then shows as its user or key). The hash-chained security audit log
(`GET /api/v1/audit`) keeps its own `tag_write` entries with the user and value.

## The web client

The client uses the WebSocket. When three connection attempts in a row end before the
server says anything (a proxy that blocks upgrades, for instance) it switches to the event
stream, shows `LIVE (SSE)` in the footer, and tries the WebSocket again every minute,
switching back as soon as one works.
