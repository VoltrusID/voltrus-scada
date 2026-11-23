# Edition hooks

An add-on edition (the Voltrus Enterprise add-on, or your own) links this
crate as a library and calls `voltrus::run` with its own implementation of
`edition::Extension` (`src/edition.rs`). Every method has a default that keeps
the open core's behaviour, so an add-on implements only what it needs. This
page is the contract for the hooks that regulated sites (21 CFR Part 11,
IEC 62443) need: password policy, session limits, electronic signatures, the
audit trail of setpoint changes, roles, and add-on widgets on screens. The
older hooks (licence, node role, routes, bearer tokens, sign-in providers,
chat delivery, hosted AI) are described in [open-core.md](open-core.md).

All hooks are async (`BoxFuture`) unless noted. They are called on request
paths, so keep them fast: read your settings from memory or one indexed query.

## Summary

| Hook | Called | Core default |
|---|---|---|
| `validate_new_password(&PasswordCheck) -> Result<(), String>` | before a password is stored | `Ok(())` |
| `password_changed(username, password_hash, PasswordChangeKind)` | after a password is stored | nothing |
| `password_expired(&User) -> bool` | at sign-in, after the password checked out, at every refresh (and by `/auth/me` while `must_change_password` is set) | `false` |
| `session_policy(username, role) -> SessionPolicy` | at sign-in, `/auth/me`, every refresh and activity ping | no limits |
| `signature_required(&ActionContext) -> Option<SignatureRequirement>` | before every signable action | `None` |
| `record_signature(&SignatureRecord) -> anyhow::Result<Option<String>>` | after the core verified a signature, before the action runs | `Ok(None)` |
| `ExtensionRoutes::operate` | route group for the `operator` tier | empty |
| `edition::install` / `edition::installed()` | `run` installs the extension so code outside a request (flows) can ask it | `Community` |

Client side (web): `EnterpriseClientModule.screenWidgets` and the widget
registry in `client/src/edition/widgets.ts`.

## Password policy

```rust
pub enum PasswordChangeKind { SelfService, AdminCreate, AdminReset }
pub struct PasswordCheck { username, role, new_password, kind }   // Debug redacts the password
fn validate_new_password<'a>(&'a self, check: &'a PasswordCheck) -> BoxFuture<'a, Result<(), String>>;
fn password_changed<'a>(&'a self, username: &'a str, password_hash: &'a str, kind: PasswordChangeKind) -> BoxFuture<'a, ()>;
fn password_expired<'a>(&'a self, user: &'a models::User) -> BoxFuture<'a, bool>;
```

Where it is called:

| Request | `kind` | Order |
|---|---|---|
| `POST /api/v1/auth/change-password` | `SelfService` | core checks: shared account refused, 8-character minimum, current password; then the hook |
| `POST /api/v1/users` | `AdminCreate` | core checks: fields, known role, username free; then the hook |
| `PUT /api/v1/users/{id}` with `password` | `AdminReset` | core checks: user exists, role known, password not empty; then the hook, before any field changes. `role` in the check is the new role when the request changes it |

* `Err(message)` is answered `400 {"error": message}` and nothing is stored.
  Write the message for the user ("Password must contain a digit").
* `password_changed` receives the stored Argon2 hash (never the password).
  Use it for password history (keep the last N hashes; in
  `validate_new_password`, refuse a new password for which
  `voltrus::api::auth::verify_password(new, old_hash)` is true) and for the
  change date that expiry counts from. It is not called for the CLI
  `--reset-admin-password` recovery path (it sets `must_change_password`).
* `password_expired` is asked at sign-in once the password is verified and
  the account is enabled; never for a shared demo account. `true` makes the
  sign-in answer `must_change_password: true` and `password_expired: true`
  (the session is still opened, exactly as for an admin-forced change), and
  core sets `must_change_password` on the account until the password is
  changed, so every session of that user is held, not only this one. The
  web client then shows the change-password screen with "Your password has
  expired". It is asked again at every `POST /api/v1/auth/refresh`, so a
  password that expires during a session holds that session (and the
  others) from the next refresh on.
* A self-service password change revokes the user's other refresh tokens:
  every other session ends at its next refresh; the one that changed the
  password stays.
* A session whose user has `must_change_password` set (generated first admin
  password, CLI `--reset-admin-password`, or an expired password) may only
  change the password and leave: `POST /api/v1/auth/change-password`,
  `GET /api/v1/version` and the public routes (`/auth/me`, `/auth/refresh`,
  `/auth/activity`, `/auth/logout`, login, health). Every other request,
  including the SSE stream, MCP and add-on routes behind the core's
  authentication, answers
  `403 {"code": "password_change_required", "error": "Change your password before you continue"}`.
  The WebSocket refuses the upgrade with the same 403 when the session comes
  in the cookie, answers `AuthError` with a message starting
  `password_change_required` when it comes in the `Auth` message, and closes
  an open socket at its next check (30 s). `/auth/me` reports
  `must_change_password` and `password_expired` so a reloaded page returns
  to the change-password screen. API keys and sign-in-provider bearer tokens
  are not held; neither is a shared demo account (`shared_accounts`).
  Changing the password clears the flag and the same session goes on.

Implementation notes for the add-on: store the policy (minimum length,
character classes, history depth, maximum age) in your own settings table;
store `(username, hash, changed_at)` rows from `password_changed`; compute
expiry as `now - last changed_at > max_age`. Users created before the add-on
was installed have no row: treat them as changed at install time, or as
expired, per the site's policy.

## Session policy

```rust
pub struct SessionPolicy {
    pub idle_timeout_secs: Option<u64>,      // sign out after this long without user activity
    pub absolute_timeout_secs: Option<u64>,  // sign out this long after sign-in, active or not
}
fn session_policy<'a>(&'a self, username: &'a str, role: &'a str) -> BoxFuture<'a, SessionPolicy>;
```

How Core enforces it:

* A session is the chain of rotated refresh tokens that starts at sign-in.
  Each `refresh_tokens` row carries `session_started_at` and
  `last_activity_at`, copied forward on every rotation. Rows from before this
  change fall back to `created_at`.
* User activity is recorded by `POST /api/v1/auth/activity` (no body; the
  session is found by its `voltrus_refresh` cookie). The web client sends it,
  throttled to once per `min(60 s, idle/3)`, on pointer, key, wheel and touch
  input only, and only while `idle_timeout_secs` is set. Background polling,
  the WebSocket and automatic refreshes are not activity.
* `POST /api/v1/auth/refresh` ends the session (clears both cookies) when
  `now - session_started_at >= absolute_timeout_secs` (`401 {"code":
  "session_expired"}`) or `now - last_activity_at >= idle_timeout_secs`
  (`401 {"code": "session_idle"}`). `/auth/activity` answers the same way
  once a session is over, so activity cannot revive it.
* Access tokens are issued for `min(15 min, idle_timeout, time left of the
  absolute lifetime)`, at sign-in (local login) and on every refresh. A
  session that stops refreshing is therefore unusable at most one idle timeout
  after the last activity.
* Sign-in and `GET /api/v1/auth/me` report the policy as `"session":
  {"idle_timeout_secs": n|null, "absolute_timeout_secs": n|null}`. The web
  client signs out on `session_idle`/`session_expired` and says why on the
  sign-in screen.

Limits worth knowing: an open WebSocket keeps its connection until the client
closes it (the web client does when it signs out); a sign-in provider (SSO)
that opens sessions with `api::auth::issue_session` gets a 15-minute first
access token; use `api::auth::issue_session_with_policy(..., &policy)` to cap
it as well (refreshes apply the policy either way). Default: no limits, the
core's 15-minute access token and 7-day sliding refresh token.

## Electronic signatures

```rust
pub enum SignedAction { TagWrite, AlarmAck, AlarmShelve, RecipeLoad, ModeSwitch }
pub enum ActionOrigin { Session, ApiKey, Mcp, Flow, Recipe }   // can_sign(): Session, Recipe
pub struct ActionContext { action, origin, username, role, resource: String, details: serde_json::Value }
pub struct SignatureRequirement { pub meaning: String }
pub struct SignatureRecord { username, role, action, origin, resource, details, meaning, reason, signed_at }

fn signature_required<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, Option<SignatureRequirement>>;
fn record_signature<'a>(&'a self, record: &'a SignatureRecord) -> BoxFuture<'a, anyhow::Result<Option<String>>>;
```

### Flow of a signed action

1. The request passes authentication, the route's role check and the action's
   own validation (for a tag write: the whole write policy, so nobody signs a
   write that would be refused).
2. Core builds an `ActionContext` and calls `signature_required`. `None`: the
   action runs exactly as in Core.
3. `Some(req)` and the origin cannot sign (`ApiKey`, `Mcp`, `Flow`):
   **403** `{"error", "code": "signature_not_possible"}`. The action does not
   run.
4. No `signature` in the request, or one whose `meaning` differs from
   `req.meaning`: **428 Precondition Required**
   `{"error": "This action needs an electronic signature", "code": "signature_required", "signature_required": {"action": "tag_write", "meaning": "<req.meaning>"}}`.
5. `signature.reason` empty: **400** `{"code": "signature_incomplete"}`.
6. Core checks `signature.password` against the **signed-in user's own**
   Argon2 hash:
   * no local password (SSO/bearer identity without a users row, or empty
     hash) or a disabled account: **403** `signature_not_possible`;
   * account locked (`locked_until` in the future): **403**
     `{"code": "account_locked"}`;
   * wrong password: `failed_attempts += 1` (the counter sign-in uses), a
     `signature_failed` security event, and **403**
     `{"code": "signature_invalid", "signature_required": {...}}` so the
     client prompts again. The 10th consecutive failure (sign-in and signing
     together, `api::signature::MAX_FAILED_ATTEMPTS`) locks the account for
     15 minutes (`LOCKOUT_SECS`), revokes all its refresh tokens and answers
     `account_locked`. Shared demo accounts count no failures, as at sign-in;
   * right password: `failed_attempts` is reset.
7. Core calls `record_signature` with the verified record. `Err` answers
   **500** `{"code": "signature_not_recorded"}` and the action does not run:
   a signed action never happens unrecorded. `Ok(Some(id))` is your record id.
8. Core writes a `signature` audit entry
   (`action=<action> signature=<id> meaning="<meaning>" reason=<reason>`, resource
   as in the context), runs the action, and appends
   ` signature=<id> meaning="<meaning>"` to the action's own audit entry
   (`tag_write`, `update_write_policy`, `start_flow`/`stop_flow`; for actions that
   had no audit entry of their own a new one is written: `alarm_acknowledge`,
   `alarm_acknowledge_all`, `alarm_shelve`, `alarm_unshelve`, `recipe_load`,
   `toggle_simulation`, `go_live`, `go_live_rollback`). Whitespace in the id
   is replaced by `_`.

The password is never logged, serialized or passed to the edition
(`SignatureInput` has no `Serialize` and its `Debug` is redacted). The request
format is the same everywhere:

```json
"signature": {"password": "…", "meaning": "<meaning from the 428>", "reason": "…"}
```

### Signable actions

| Action | Requests | `resource` | `details` |
|---|---|---|---|
| `tag_write` | `POST /api/v1/tags/{id}/write` (body field `signature`); WebSocket `TagWrite`; MCP `write_tag`; flow write nodes | `data_source:<id>` | `{tag_id, key, device_id, value, previous}` (`tag_id` is the `<id>` of `resource`; `previous` is the live value or `null`) |
| `tag_write` (origin `recipe`) | asked once per tag before a recipe load | `data_source:<id>` | `{tag_id, key, device_id, value, recipe_id}` |
| `recipe_load` | `POST /api/v1/recipes/{id}/load` (optional body `{signature}`) | `recipe:<id>` | `{recipe_id, name, version, device_id, values}` |
| `alarm_ack` | `POST /api/v1/alarms/{id}/acknowledge`, `POST /api/v1/alarms` (all), optional body `{signature}`; MCP `acknowledge_alarm` | `alarm:<id>` or `alarm:*` | `{alarm_id}` or `{all: true}` |
| `alarm_shelve` | `POST /api/v1/alarms/{id}/shelf` (body `{duration_secs, reason, signature?}`), `POST /api/v1/alarms/{id}/unshelf` (optional body) | `alarm:<id>` | `{alarm_id, shelve: true, duration_secs, reason}` / `{alarm_id, shelve: false}` |
| `mode_switch` | `POST /api/v1/devices/{id}/simulate` (body `{simulation_mode, …, signature?}`) | `device:<id>:simulation` | `{mode: "simulation"\|"live", device_id}` |
| `mode_switch` | `POST /api/v1/devices/{id}/go-live`, `…/go-live/rollback` (optional body) | `device:<id>:go_live`, `device:<id>:go_live_rollback` | `{mode: "go_live"\|"go_live_rollback", device_id}` |
| `mode_switch` | `POST /api/v1/flows/{id}/start`, `…/stop` (optional body); MCP `start_flow`/`stop_flow` | `flow:<id>` | `{mode: "start"\|"stop", flow_id, name}` |
| `mode_switch` | `POST /api/v1/flows/{id}/run` (manual run; body `{input?, signature?}`) | `flow:<id>` | `{mode: "run", flow_id, name}` |
| `mode_switch` | `PATCH /api/v1/data-sources/{id}/write-policy` (body fields plus `signature`) | `data_source:<id>:write_policy` | `{mode: "write_policy", tag_id, policy}` |

"Optional body" means the route accepts no body (as before) or a JSON body
`{"signature": {...}}` with `Content-Type: application/json`.

A recipe load needs one signature: Core asks `recipe_load`, and if that needs
none, asks `tag_write` (origin `recipe`) for each tag the recipe writes; the
first requirement found is the one the user signs, and it covers every write
of the load. The individual writes are then queued without asking again.

### Who can sign

| Origin | Who | When the edition requires a signature |
|---|---|---|
| `session` | a person with a session (web, native console, WebSocket, add-on bearer identity with a local users row) | prompted (428) |
| `recipe` | a recipe load's per-tag question | the load is prompted |
| `api_key` | `X-API-Key` / `?api_key=` / WebSocket key auth | refused, 403 `signature_not_possible` |
| `mcp` | any MCP tool call, key or session | refused, 403 (tool error) |
| `flow` | a flow's write node (`username` `flow:<id>`, `role` `flow`), whatever started the run: its trigger (tag change, schedule, MQTT, webhook), a manual run (`POST /flows/{id}/run`) or start | the write is refused and the node fails with "needs an electronic signature … which a flow cannot give" |

A manual run is signed as a `mode_switch` (`mode: "run"`) by the person who
starts it, but that signature does not cover the writes the flow then makes:
they are still asked as `flow` writes, because the run's content (payload,
branches, values) is decided by the flow, not shown to the signer. Dry runs
(`POST /flows/test`, `/flows/{id}/test`) write nothing and ask nothing; their
preview of a write node does not report a signature refusal.

The edition decides per origin: to let flows or MCP act unsigned, return
`None` for those origins (for example for tags a validated flow owns). Returning
`Some` for them is a refusal, never a prompt.

### WebSocket

`ClientMessage::TagWrite` has an optional `signature: {password, meaning,
reason}`. A write that needs one is answered on the requesting connection with
a `CommandAck` whose `status` is `signature_required` (none or wrong meaning)
or `rejected` (wrong password; `device_response` says so), carrying
`signature_required: {action, meaning}`. The client resends the write with
`signature` (the native console does), or repeats it over REST (the web
client does). Other refusals (`signature_not_possible`, `account_locked`) are
plain `rejected` acks.

### Implementing the Enterprise side

* Keep the 21 CFR Part 11 configuration in your settings: which actions need a
  signature, per tag or tag group, per role, the meaning text per action
  ("Performed by", "Reviewed by", "Approved by"), and whether flows/MCP may act
  unsigned. Answer `signature_required` from memory.
* `record_signature`: insert the record (append-only table, hash-chained or
  checksummed like the core audit log), return its id. Include `username`,
  the full name if you keep one, `signed_at` (UTC), `meaning`, `reason`,
  `resource`, `details` (it holds the old and new value of a tag write). Fail
  (`Err`) when the record cannot be stored; the action is then refused.
* Two-person rules (a second signer) are not in Core; build them on your own
  routes (for example approve first, then call the action with the approver's
  signature id in `reason`).

## Audit trail of tag writes

Every tag write records the value before and after:

* The `tag_write` audit entry (REST, WebSocket, MCP) reads
  `command=<id> previous=<live value> value=<new> [signature=<id> meaning="…"] [reason=<text>]`;
  `previous=` is left out when the tag had no live value yet. The reason is
  always last.
* The command's first `command_audit_log` row has a `previous_value` column
  (`GET /api/v1/commands/{id}/audit` → `previous_value`), for writes from
  every source including flows and recipes.
* `GET /api/v1/audit/export` (CSV) has three more columns:
  `previous_value`, `new_value` (for `tag_write` rows) and `signature` (any
  row with a signature), parsed from the fields before the quoted meaning and
  the free-text reason, so a reason cannot fake them.

The previous value is read from the live cache when the write is queued, the
same value the deadband check uses.

## Roles

| Role | Can |
|---|---|
| `admin` | everything, including users, roles, API keys, security, compliance |
| `demo` | operate and configure (the public demo account) |
| `operator` | operate: write tags within their write policy, cancel commands, acknowledge, shelve and unshelve alarms and read the alarm journal, summary, audit and analytics, list, view and load recipes, start, stop and run flows (any signed-in user may read flows and their runs), shift handover, log and close downtime. Cannot change devices, tags, write policies, screens, flows, recipes or settings |
| `viewer`, `user` (legacy) | read only; cannot acknowledge alarms |
| API key `integration` | like `demo` (scopes can narrow it to read-only) |
| API key `api` | read only |

Server: `api::role_allows_operation(role)` (operator tier) and
`api::role_allows_configuration(role)` (configuration tier;
`role_allows_operational_writes` is its old name). Routes:
`public → protected → operator_protected → demo_protected → admin_protected`
in `src/api/mod.rs`. Add-on routes go into `ExtensionRoutes::operate` for the
operator tier and `ExtensionRoutes::operator` for the configuration tier (the
field name predates the operator role). The WebSocket grants `write` and MCP
grants `write_tag`, `acknowledge_alarm`, `start_flow`, `stop_flow` to the
operator tier. Web client: `client/src/roles.ts` (`canOperate`,
`canConfigure`, `isAdmin`). Native console: `Role::Operate` below
`Role::Configure`.

Custom roles created on the Roles page are labels; the built-in role of the
user decides.

## Screen widget slot (web client)

Dashboards and SCADA screens accept widget types registered at run time, for
example a live camera. The server stores them opaquely: a SCADA element keeps
`equipment_type` and its `config` JSON string; a dashboard keeps the widget as
an extra grid item `{i: "widget:<id>", x, y, w, h, widget: {type, config}}`
in each breakpoint of `PUT /api/v1/screens/{id}/layouts`.

`client/src/edition/widgets.ts` (re-exported from `client/src/edition`):

```ts
type ScreenSurface = 'dashboard' | 'scada'
interface ScreenWidgetDef {
  type: string                         // namespaced, e.g. 'enterprise.camera'
  label: string
  icon?: ReactNode
  surfaces: readonly ScreenSurface[]
  defaultSize?: { w: number; h: number }               // dashboard grid cells, default 6x5
  defaultPixelSize?: { width: number; height: number } // SCADA px, default 320x240
  defaultConfig?: Record<string, unknown>
  render: ComponentType<ScreenWidgetRenderProps>       // {config, width, height, editMode, surface, sensors, values}
  configEditor?: ComponentType<ScreenWidgetConfigEditorProps> // {config, onChange, surface, sensors}
}
registerScreenWidget(def) / registerScreenWidgets(defs) / unregisterScreenWidget(type)
screenWidgets(surface?) / getScreenWidget(type, surface?) / resolveScreenWidget(type, surface)
```

The enterprise client module registers them declaratively:

```ts
export default {
  screenWidgets: [{
    type: 'enterprise.camera', label: 'Camera', surfaces: ['dashboard', 'scada'],
    defaultSize: { w: 8, h: 6 }, defaultConfig: { camera_id: null },
    render: CameraView, configEditor: CameraSettings,
  }],
}
```

* Types must contain a dot (a namespace); registering an existing type
  replaces it. An invalid definition is logged and skipped.
* The widget pickers show add-on widgets only when at least one is
  registered, so Community looks unchanged.
* A saved widget whose type is not registered (the add-on was removed) shows
  "Widget not available in this edition" and is saved back unchanged.

## Hosted AI provider

The copilot's `voltrus_ai` provider exists only when the add-on registers a
`HostedAi` (`edition::register_hosted_ai`). `GET /api/v1/ai/settings` reports
`hosted_ai_available`; the web client offers "Voltrus AI" only then (or when
it is already the stored provider), and `PUT /api/v1/ai/settings` refuses to
switch to it otherwise. A stored `voltrus_ai` setting still parses.
