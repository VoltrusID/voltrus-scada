# Upgrade notes

Changes in behaviour that can need action when upgrading. The repository has
no changelog file: release notes are generated from pull requests, so this
page is where such changes are written down. Newest release first.

## Unreleased

### Designer live collaboration

Phase 4 of the Designer ([collaboration.md](collaboration.md)). Several
people edit one screen at the same time, in the browser or the native
console; the server relays and checks every change over the existing
WebSocket and writes the draft rows itself. The upgrade adds the table
`design_comments`; nothing existing is migrated.

What changes for people and API clients:

* **On by default.** The Designer no longer has unsaved changes or a
  conflict dialog: edits are written to the draft about every 2 seconds (the
  Save button and Ctrl+S write at once). Turn it off with `PUT
  /api/v1/settings/design-history {"collaboration": false}`; the Designer then
  saves whole layouts as before, and open sessions are written and closed.
* **REST and MCP writes during a session** take the screen's lock: the
  session's pending edits are written first, so a `PUT …/scada-layout` with a
  `base_seq` or `base_revision` loaded before those edits gets a 409 (as if
  someone had saved), and a write without a base lands on top of them. The
  editors see the write at once. Revert and publish behave the same way.
* **Change log:** a session's writes are batches by each author with source
  `designer` and the note *Live session*. A screen edited by two people in one
  write gets two batches.
* **Edits that fail validation are not written** (unknown tag in a new binding,
  an equipment instance or faceplate that does not exist, a connection whose
  widget is gone, values out of range): they are put back for everyone, the
  author is told, and the audit log has `design_collab_rejected`. The layout
  `PUT`s are unchanged and still accept what they accepted before.
* **WebSocket:** new message types (`Design*`) on `/api/v1/ws`. Clients that
  ignore unknown `type`s are unaffected. The server-sent events fallback does
  not carry them: collaboration needs the WebSocket (the Designer saves whole
  layouts when only the fallback works).
* **Reverse proxies** must pass WebSocket upgrades for `/api/v1/ws` (as the
  live data already needs).
* Dependencies: `yrs` 0.28.2 (MIT) on the server and console, `yjs` 13.6.33,
  `y-protocols` 1.0.7 and `lib0` 0.2.120 (MIT) in the web client; `dashmap` moves
  from 6.1.0 to 6.2.1 (required by `yrs`).

### Designer version history: change log, versions, draft and published

Phase 3 of the Designer ([version-history.md](version-history.md)). The
upgrade adds the tables `design_changes`, `design_batches`, `design_versions`,
`design_screen_state`, `scada_equipment_published` and
`scada_connections_published`; existing rows are not touched and the log
starts empty (history begins with the first write after the upgrade).

Nothing changes for screens nobody publishes: a screen that was never
published shows its draft to everybody, as before. Once a screen is
published:

* **Operators see the published screen.** `GET …/scada-equipment`,
  `…/scada-connections` and `…/scada-layout` answer roles that cannot
  configure (operator, viewer, read-only API keys) with the published rows.
  Configure roles (admin, demo, integration keys) get the draft, or the
  published rows with `?published=1`; the web 2D view and 3D twin ask for
  them. Scripts with an integration key that read rows to show them to
  operators should add `?published=1`. Every GET answer has `view: "draft"` or
  `"published"`.
* **Every write changes the draft.** REST layout `PUT`s, MCP, project import
  and equipment propagation are unchanged for clients; their changes appear to
  operators when the screen is published. A project import publishes only
  with `?publish=1` / `--publish` / MCP `publish: true`.
* Turn the two-stage flow off with `PUT /api/v1/settings/design-history
  {"publishing": false}`: everybody sees the draft again.

Other changes visible to API clients:

* Layout `PUT`s answer `{ok, revision, seq, batch}` and accept `base_seq` next
  to `base_revision`; a 409 also has `seq` and `changes_since` (who changed
  what). The shapes of the old fields are unchanged.
* `voltrus project export` writes `published_version: <name>` on published
  screens; import ignores it, older servers refuse it as an unknown field
  (export with an older version, or remove the line, to go back).
* A new signed action `design_publish` for editions with electronic
  signatures ([edition-hooks.md](edition-hooks.md)).
* The change log grows with every save. Consecutive saves of one widget by one
  person within 60 s are one row, and rows older than 30 days are squashed to
  one per object and author per hour; set `keep_days` to prune (default:
  keep). States by `seq:` at or below what retention squashed or pruned are
  refused (410 `state_gone`).
* Git sync needs the `git` command line on the server only if you turn it on.
  It pushes the **whole project** (devices and their addresses, tags, flows,
  settings), not only screens. Remotes must carry no credentials, token auth
  needs `https://`, and a local remote and `work_dir` must be inside the data
  folder.
* A config snapshot (and so a rollback, including a go-live rollback) now
  covers the screens' published copies: rolling back changes what operators
  see, not only the draft. Snapshots taken before this release have no
  published rows and leave them as they are.
* Publish after import (`?publish=1`, `--publish`, MCP `publish`) follows the
  site's switches: refused while publishing is off, publish requests when
  approval is required, and the signed action `design_publish` where the
  edition wants one (REST: `X-Voltrus-Signature` header; the CLI cannot sign).
* With approval required, dropping it or turning publishing off waits for a
  second admin (`POST /api/v1/settings/design-history/approve`).
* Version names of digits only, `draft`, `published`, and names starting with
  `seq:`, `id:` or `name:` are refused; `id:<n>` and `name:<name>` pick one.

## v0.53.0

### New tags have no alarm limits; alarms clear when their limit goes

On a fresh database the first start used to create the configured devices'
tags with alarm limits nobody set: high warning 80 and high critical 90 for
any register (and fixed limits for registers named `voltage`, `current`,
`power`, `frequency`, `power_factor` and `energy_total`, including a high
power-factor alarm at 0.98). Tags from vertical templates got the range's
minimum and maximum as high limits, and a template's "below" alarm became a
high limit. A flow reading 1178 m³/h was then in critical alarm on its first
poll. Now:

* Tags created at first start, from vertical templates and from FEED
  templates have no high or low limits until someone sets them. A vertical
  template's own alarms still become limits on its tags: "above" a high
  limit, "below" a low limit (it used to be stored as a high limit), and
  "equal" a low limit when it is the bottom of the tag's range (safety
  contacts such as `SAFE_DOOR`, `SAFE_ROPE` and `NAV_<light>` alarm at 0;
  they used to get no alarm at all) or a high limit otherwise. A tag with
  several template alarms keeps them all, and the template's deadband is
  written to the tag. An "above 0" alarm (the lift template's `OPS_FAULT`)
  still sets no limit, since a high limit of 0 is "not set".
* An active limit alarm clears on the next evaluation (persistence interval)
  when its condition no longer exists because the tag changed: its limit was
  removed or disabled, its limit moved so the current value is inside it
  (with the tag's deadband), or the tag was deleted or no longer reads a
  device. Before, such alarms stayed active forever, since evaluation only
  cleared an alarm once the value came back inside the limit it was raised
  on. The alarm's audit trail gets an `auto_resolved` entry whose details
  say why: `limit_removed`, `limit_changed`, `tag_deleted` or `not_alarmed`,
  with a message such as "Raw Water Intake Flow FT-101: high critical limit
  removed (was 90 m³/h)". A removed limit clears its alarm even while the
  tag has no good value; a changed one waits for a good value inside the new
  limit (with the deadband stored on the alarm) and, on a tag with an off
  delay, for that value to stay inside for the off delay.
* Renaming a tag's key moves its active limit alarms to the new key, so they
  are not cleared and raised again.
* Alarms raised by a flow's alarm node are stored with `alarm_type`
  `event`, not `high`, so limit evaluation never clears them (it used to
  clear them when the tag of the same key read below 0). An event alarm has
  no condition to return to normal, so **acknowledging it resolves it** (the
  audit trail gets a `resolved` entry by the operator, reason
  `event_acknowledged`). It no longer holds back limit alarms on the same
  key, and its acknowledgement cooldown applies to it only. The upgrade
  retypes flow alarms stored before (`high`, value, limit and deadband 0,
  and a message not written by limit evaluation) as `event`. API clients
  and the SDKs see `alarm_type: "event"`; value and threshold are 0.

**Existing tags keep their limits.** A limit seeded by an earlier version
cannot be told apart from the same limit set on purpose, so the upgrade
changes no stored limit. At start the server logs a warning for each kind of
unconfigured limit it still finds on tags never edited since (no
`*_enabled` flag in their settings):

* first-start tags with the seeded limits exactly as seeded (name, colour,
  range, base and limits unchanged);
* vertical and FEED template tags still as the template wrote them (orange,
  template name, base in the middle of the range) whose high warning limit is
  still the range minimum or whose high critical limit is still the range
  maximum. Two-state tags (range 0–1, bool, coil, discrete input, FEED DI/DO)
  are not listed for a critical limit at the maximum: that is how a fault bit
  is alarmed, so a seeded one looks the same as one set on purpose;
* vertical template tags whose "below" alarm was stored as a high limit
  (the warning names them; move the value to the low limit).

Review those tags and set real limits or clear them; once cleared, any alarm
raised on them clears on the next evaluation. Template "equal 0" safety
alarms are not added to existing tags: set a low limit of 0 on them, or
create the template again.

### S7, OPC UA and EtherNet/IP tags read their device's values

Tags on S7, OPC UA and EtherNet/IP devices were never linked to what their
pollers read: the live data API and dashboards showed a generated value with
quality `good`, history was generated, limits never raised alarms and queued
writes found the device offline. They now read the poller's values, so after
the upgrade these tags show real readings, **their limits start raising
alarms**, and an offline device shows `bad_comm_failure`. Check the limits
on such tags before upgrading a running plant. Each tag records its tag name
(`source_config.tag`); existing tags get it at the first start.

* Earlier S7 samples stay in history under `<area>_<offset>` (`db_0`); new
  samples are stored under the tag's name.
* A device name already used by an S7 device, or by an OPC UA or EtherNet/IP
  device, is refused on create and rename (their tag keys would clash and
  one tag could clear the other's alarms). Existing duplicates keep working
  but should be renamed.
* `POST /api/v1/s7/devices/{id}/start` and a device update now apply each
  tag's data type (a REAL at `DB1.DBD4` was read as a DWORD until the server
  restarted).

### Stricter project files and REST bodies

Enumerated fields in the project format (Modbus `register_type`,
`register_area` and `endianness` on devices and templates, template
`protocol`, BACnet `object_type`, SCADA `connection_type`, faceplate kinds)
are checked when the file is loaded; an unknown value is an error naming the
field and the allowed values. Before, `uint32` was accepted and read as
`float32`. `POST`/`PUT` on `/api/v1/data-sources`, `/api/v1/screens`,
`/api/v1/modbus-devices` and `PUT .../registers` answer 400 for an unknown
field or a bad enum value; the fields `GET` returns are accepted, so
read-modify-write keeps working. Fix scripts that send extra fields.
`docs/project-schema.json` describes the project format.

### Viewers can read alarm details; alarm messages are rounded

* Any signed-in user, viewers included, can now read
  `GET /api/v1/alarms/summary`, `/alarms/analytics`, `/alarms/audit`,
  `/alarms/{id}/journal`, `/annotations` and `/annotations/{id}`. They were
  operator or configuration tier, so read-only users got 403 errors in the
  alarm panel. Acknowledging, shelving and editing annotations keep their
  tiers. See the roles table in `docs/edition-hooks.md`.
* New alarm messages use the tag's decimals (0 to 6) and unit: `reached
  2.20 m (warning: 2.20 m)` instead of `reached 2.3550000190734863m
  (warning: 2.2m)`. Email and chat notifications round the same way;
  outbound webhook JSON keeps the raw numbers. Anything that parses alarm
  message text should be checked.
* Edit controls are hidden for roles that cannot use them (the server
  already refused the actions).

### AI copilot: proposals are validated and applied all or nothing

Applying a proposal now writes through the same REST handlers as the UI, so
it gets their validation, and undoes its earlier writes if one fails. Some
inputs that used to be accepted are refused: Modbus addresses outside their
area or range (`400001`), register types the driver does not have, an edited
configuration that is not valid, PDF or binary uploads, and approving a
proposal that is not pending. A provider's 401/403 is returned as 400
`ai_invalid_api_key`, a provider 429 as 429 with `Retry-After`, and a reply
cut off at the token limit as 502 `ai_truncated`.

## v0.52.0

### Sign-in lockout is per address, not per account

Ten wrong passwords for an account used to lock the account for everyone for
15 minutes, so anyone who could reach the sign-in page could keep a user
(an admin, the operators of a plant) locked out by sending a few wrong
passwords every 15 minutes. Failed sign-ins are now counted in a new table,
`login_failures`, against the client address the server resolves (the
`trusted_proxy` rules: a proxy header is read only from the proxy's own
address; IPv6 addresses count per /64 network):

* 10 wrong passwords for one account from one address within 15 minutes
  lock that account **for that address** for 15 minutes (403, as before).
  The user still signs in from any other address.
* 30 wrong passwords (or unknown usernames) from one address across all
  accounts within 15 minutes lock that address out of sign-in for 15 minutes
  (429), against password spraying. A user who signed in from that address
  in the last 30 days is not locked out by it (someone else behind the same
  NAT can't shut the plant out). Failures from this machine (127.0.0.1,
  ::1) don't count here, and the in-memory limit of 10 failures a minute
  per address doesn't apply to it: the native console and a local reverse
  proxy connect from it.
* 50 wrong passwords for one account from all addresses within 15 minutes
  make sign-ins to it from addresses the user has not signed in from in the
  last 30 days wait a minute (429), renewed by every further failure in
  those 15 minutes. Addresses the user did sign in from are not slowed down.
  The trade-off: during a sustained attack from many addresses, the user
  cannot sign in from a new device until the attack stops or an admin
  resets the password.
* A successful sign-in clears the count of that account and address (not
  the address's count across accounts). Changing the password, or an admin
  resetting it (Users, or `voltrus --reset-admin-password` for admin),
  clears every count and lock of the account.
* The in-memory limit of 5 tries a minute is now per account and address,
  no longer per account.
* Wrong passwords typed inside a session (electronic signatures, and now
  also the current password when changing it, which was not limited
  before) are counted apart from sign-in: 10 within 15 minutes lock these
  checks for 15 minutes and end the user's sessions, but no longer lock
  sign-in. They do count towards the account's 50.
* Attempts are counted before the password is checked, so requests sent in
  parallel cannot get past a limit. Windows are fixed, starting at the first
  failure, so up to twice a limit can get through around the end of one.

Behind a reverse proxy on another machine, list it in `trusted_proxy`: if
every client shows up as the proxy's address, all users share one address,
and 30 failures from anyone lock out of sign-in every user who has not
signed in through it in the last 30 days.

The upgrade creates the table and lifts any lock of the old kind; if it
cannot create the table the server does not start. The `failed_attempts`
and `locked_until` columns of `users` are left in place but no longer used.

### OEE: devices with several metrics need a state metric

The OEE monitor used to decide whether a machine runs from whichever metric
the cache happened to list first when no state metric was configured. It now
tracks such a device only if the device has a single metric. A device with
several metrics and no `metric` in its `oee_threshold_<device id>` setting
gets no downtime events until one is chosen:

```json
{ "metric": "spindle_speed", "running_above": 0 }
```

The server logs a warning once per device, and `GET /api/v1/oee/metrics`
answers with `"tracking": "not tracked: choose a state metric ..."` for it.
Readings that are not good (bad quality, stale, NaN) no longer change the
state either.

### Modbus: `float64` in `little` and `swapped` order

`little` and `swapped` used to apply only inside each half of a 64-bit
value: the high 32 bits always came first. They now reverse the whole value,
as the 32-bit orders do (`little`: `GH EF CD AB`, `swapped`: `HG FE DC BA`;
see docs/device-templates.md). 32-bit values and `big` are unchanged.

A device with `float64` registers whose readings were correct with `little`
or `swapped` before the upgrade keeps the old layout under the new names
`little_dword` and `swapped_dword`: change the device's `endianness` (device
API, project YAML or template). Readings of such a device that suddenly look
like tiny or huge numbers after the upgrade are the sign it needs this.

### Flow schedules: day fields written with `*`

Schedules now read the two day fields as Vixie cron does: a field that
starts with `*` counts as unrestricted, and then a day must match both
fields. Up to v0.51 a starred step such as `*/2` counted as restricted, so a
day matching either field fired.

Only schedules with a `*`-prefixed step in one day field and a restricted
other day field change, for example `0 0 */2 * mon`: it used to fire on
every odd day and every Monday, and now fires on Mondays that fall on an odd
day. To keep the old behaviour, write the step without the star:
`0 0 1-31/2 * mon`. Plain `*` and schedules with only one day field
restricted behave as before.

Schedules with a fixed minute and hour also handle daylight saving time
differently: they fire once when the clock goes back, and at the change
when the clock skips their time (docs/flows.md).
