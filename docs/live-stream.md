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
data: {"type":"TagUpdateBatch","updates":[{"tag_id":"inverter-1.power","value":412.5,"quality":"good","ts":1761700000,"simulated":false}]}
```

The first event of a stream is `AuthOk` (the signed-in user and permissions), as on
the WebSocket. A comment line is sent every 15 seconds to keep proxies from closing an
idle stream. A value that is NaN (a failed read) is `null` in JSON.

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
Only admins may write over the
WebSocket. Clients that are not on the WebSocket (including the event stream) write
with the REST endpoint.

## The web client

The client uses the WebSocket. When three connection attempts in a row end before the
server says anything (a proxy that blocks upgrades, for instance) it switches to the event
stream, shows `LIVE (SSE)` in the footer, and tries the WebSocket again every minute,
switching back as soon as one works.
