# MCP server

The Voltrus server speaks the [Model Context Protocol](https://modelcontextprotocol.io)
at `POST /mcp`, so an AI client (Claude Desktop, Claude Code, any MCP client) can
inspect and configure the SCADA through tools. It is part of the server process,
not a separate binary, and is on by default (Cargo feature `mcp`; build with
`--no-default-features` and pick the protocols you want to leave it out).

- Transport: MCP Streamable HTTP, stateless. Every JSON-RPC request is answered
  in the HTTP response as `application/json`. There are no sessions and no
  server-initiated stream, so `GET /mcp` and `DELETE /mcp` answer 405.
- Protocol revisions: `2025-06-18` (preferred), `2025-03-26`, `2024-11-05`.
- Methods: `initialize`, `ping`, `tools/list`, `tools/call`. Resources and prompts
  are not offered.
- The tools call the same handlers as the REST routes, so validation (naming
  convention, register parsing), write policy and side effects (poller start,
  asset sync, flow reload, WebSocket pushes) are identical to the web client.

## Connect

### 1. Create an API key

A read-only key is enough for dashboards and Q&A. To let the agent change things
create an **integration** key (as an admin, in the web UI's API keys page or over REST):

```bash
curl -s -X POST http://localhost:3000/api/v1/api-keys \
  -H "Content-Type: application/json" -b "voltrus_token=<admin session>" \
  -d '{"name": "claude", "role": "integration"}'
# {"ok":true,"id":3,"name":"claude","key":"3f9c...","warning":"Save this key now"...}
```

The key is shown once. Roles: `api` is read-only, `integration` may also call
write tools. Admin and demo logins work too (cookie), but an agent should hold a
key of its own so its actions are attributable and revocable.

### 2. Add the server to the client

**Claude Code**

```bash
claude mcp add --transport http voltrus http://localhost:3000/mcp \
  --header "X-API-Key: <key>"
# or, with a bearer token:
claude mcp add --transport http voltrus http://localhost:3000/mcp \
  --header "Authorization: Bearer <key>"
claude mcp list
```

**Claude Desktop** (`claude_desktop_config.json`; Desktop reaches remote HTTP
servers through the `mcp-remote` bridge):

```json
{
  "mcpServers": {
    "voltrus": {
      "command": "npx",
      "args": [
        "-y", "mcp-remote", "http://localhost:3000/mcp",
        "--header", "X-API-Key:${VOLTRUS_API_KEY}"
      ],
      "env": { "VOLTRUS_API_KEY": "<key>" }
    }
  }
}
```

Newer Claude Desktop versions can also add it as a custom connector from
Settings, using the URL and an `Authorization: Bearer <key>` header.

**Other clients**: point them at `http://host:3000/mcp` and send the key as
`X-API-Key: <key>` or `Authorization: Bearer <key>`.

**Try it by hand**

```bash
KEY=<key>
curl -s http://localhost:3000/mcp -H "X-API-Key: $KEY" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
curl -s http://localhost:3000/mcp -H "X-API-Key: $KEY" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_live_values","arguments":{}}}'
```

Behind a reverse proxy use the public `https://` URL, and terminate TLS there:
the key travels in a header.

## Tools

Read tools work with any authenticated principal. Write tools need the `admin`,
`demo` or `integration` role (the rule the REST operational write routes use);
for a read-only key they fail with JSON-RPC error `-32003`.

| Tool | Access | What it does |
| --- | --- | --- |
| `server_info` | read | Version, uptime, licence tier, who you are, headline counts. |
| `list_devices` | read | Devices with live status; `id` returns one Modbus device with its registers. |
| `create_modbus_device` | write | Add a Modbus TCP device with its register map and start polling. |
| `delete_device` | write | Delete a Modbus device and stop its poller. |
| `list_tags` | read | Tags with limits and source config; filter by screen, device, type, text. |
| `create_tag` | write | Create a tag (limits, source type and config, device, path, alarm delays). |
| `update_tag` | write | Change any tag field, for example limits. |
| `delete_tag` | write | Delete a tag. |
| `read_live_values` | read | Current values from the live cache with status against limits. |
| `read_history` | read | Trend data by key and preset range or `from`/`to`, `avg`/`minmax`/`sampling`. |
| `list_alarms` | read | Alarms by status and priority. |
| `acknowledge_alarm` | write | Acknowledge an alarm, attributed to the caller. |
| `alarm_summary` | read | Active counts by severity, shelved, last 24 h, mean acknowledge time. |
| `list_screens` | read | Screens (dashboards). |
| `create_screen` | write | Create an empty screen. |
| `add_tag_to_screen` | write | Place a tag on a screen. |
| `list_flows` | read | Automation flows. |
| `create_flow` | write | Create a flow (trigger plus node pipeline); validated first, disabled by default. |
| `start_flow` / `stop_flow` | write | Enable or disable a flow. |
| `write_tag` | write | Queue a setpoint write through the tag's write policy. |
| `get_command_status` | read | Outcome of a queued write. |

Every tool publishes a JSON Schema (`inputSchema`) with `additionalProperties:
false`, and returns structured JSON (`structuredContent`, mirrored as text).
Mistakes in the arguments or refusals by the server (write policy, naming
convention, unknown ids) come back as a result with `isError: true` and a message
the model can act on; they are not HTTP failures.

A Modbus register becomes a tag only through `create_tag` with
`source_type: "modbus"`, `device_id` = the device id and `key` = the register
name, exactly as in the web UI.

### Example session

> "Add the PLTS meter at 10.0.0.21, register voltage at 40001 (float32), show it
> on a new screen with a warning at 240 V."

The agent calls `create_modbus_device`, `create_tag` (`warn_val: 240`),
`create_screen`, `add_tag_to_screen`, then `read_live_values` to confirm the
value arrives.

## Security

- **Same authentication as REST.** `X-API-Key`, `Authorization: Bearer <api key>`
  or a session cookie. A wrong key is 401 (and counts toward intrusion
  detection); no credentials is 401 with `WWW-Authenticate: Bearer`. The global
  rate limiter, 1 MB body limit and security headers apply to `/mcp`.
- **Least privilege.** Admin-only operations (users, roles, API keys, licence,
  security settings, backups) have no tool. An `integration` key can do what the
  operational REST routes allow: change devices, tags, screens, flows, and write
  setpoints. Treat it like an operator login; give read-only agents an `api` key.
  Revoke it from the API keys page when the agent is done.
- **Writes to the plant are policy-gated.** `write_tag` runs the tag's write
  policy: tags are not writable until an operator enables it, and range,
  deadband, rate limit and reason requirements are enforced. Tool descriptions
  tell the model to write only on explicit instruction, but the policy is the
  control, not the prompt.
- **Audit.** Every write tool call is logged to the audit log with the
  principal (`api_key:<id>` or the username), the tool, the argument names and
  the outcome; refused calls are logged as `mcp:<tool>:denied`. The underlying
  handlers add their own entries as for REST. Argument values are not logged.
- **Prompt injection.** Tag names, alarm messages and device names are data an
  attacker with write access could shape. An agent that can write should be
  supervised: prefer clients that ask for confirmation per tool call, and use a
  read-only key for unattended analysis.
- **Browsers.** A request with an `Origin` header must come from the server's own
  host or a configured `cors_origin`, otherwise 403 (DNS-rebinding protection the
  MCP transport requires), and POST bodies must be `Content-Type:
  application/json`. Non-browser MCP clients send no `Origin` and are unaffected.

## Extending

Tools live in `src/mcp/tools/*.rs`. A module returns a `Vec<Tool>` (name,
description, input schema, access level, async function) and is listed in
`mcp::tools::registry()`. Call the existing axum handler through
`from_handler(...)` so a new tool inherits the REST route's validation and
behaviour instead of copying it.
