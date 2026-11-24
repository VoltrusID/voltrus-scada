# Lessons from competitor labs: what was implemented

Status: branch `feat/lessons-from-competitors`, 2026-11-25. Source for the
voltrus.id/compare update.

Ten improvements taken from hands-on tests of FUXA 1.3.6, Rapid SCADA 6.5.1 and
Siemens WinCC OA 3.21 against the same water-treatment simulator. For each: the
competitor behaviour that motivated it and where the evidence is, what Voltrus
does now, the commit, the tests and what is not done.

Evidence paths:

* `~/scada-compare/COMPARISON.md` (the cross-product table and the ranked list
  "What Voltrus should learn"), `~/scada-compare/products/fuxa/NOTES.md`,
  `~/scada-compare/products/rapidscada/NOTES.md` and their `evidence/` folders.
  **L** in those files means verified in the lab.
* `docs/research/winccoa-comparison.md` in the voltrus-scada main checkout (WinCC
  OA lab, `~/winccoa-eval/lab`), cited by section.

Commit SHAs are on `feat/lessons-from-competitors` (based on `main` at 5822670).
Rust test names are module paths in the `voltrus` library (`cargo test --lib`);
client tests are files in `client/tests/` (`npm test`).

---

## 1. Default state colours on every bound value

**Motivation.** Rapid SCADA colours any text bound to a channel with the channel
status (raw turbidity turned orange on High) with no screen work
(`rapidscada/NOTES.md`, "Process screen and command [lab]";
`COMPARISON.md` item 1).

**Before.** Each SCADA widget decided on its own; most showed neither alarm state
nor quality. Dashboard cards compared the status with
`critical`/`warning`, while the live stream (and the client's own status from tag
limits) uses `crit`/`warn`, so a card fed by live updates never showed an alarm
colour; only the first REST snapshot did.

**Now.** One style map, `client/src/valueStyle.ts`: critical, warning, bad and
uncertain quality, offline and no data, each with a theme colour, a marker
(`!`, `✕`, `?`) and a label; quality wins over the alarm level (alarms are only
evaluated on good values). Used with no per-widget setting by dashboard cards
(value, dot, badge, border), by a runtime frame plus corner marker around every
bound 2D SCADA widget (areas, labels and pipes excepted), by the big number,
indicator and data table widgets, and by faceplates. Both status spellings map to
the same state. Docs: `docs/screens.md` "State colours".

**Commit.** 82c978c.

**Tests.** `client/tests/valueStyle.test.ts` (both spellings map to one state;
quality wins over alarm level, offline and no data over both; markers and labels
tell states apart without colour; every state colour is a variable both themes
define); `client/tests/contrast.test.ts` "value state colours (valueStyle.ts) read
at 4.5:1 on background and cards".

**Not done.** Add-on (edition) widgets and the 3D twin do not use the map; the
frame is drawn by the SCADA widget shell, not inside each widget's own graphics.

## 2. Tag-swap popup faceplate

**Motivation.** FUXA opened one faceplate view from three pumps, each with a
`from → to` tag mapping; STOP in the P-301A popup wrote `p301a_run=0` while
`p101a_run` stayed 1 (`fuxa/NOTES.md` "Reusable objects [lab]",
`fuxa/evidence/faceplate-mapping.log`). Rapid SCADA's faceplates could not take
channel parameters, so commands stayed outside the faceplate
(`rapidscada/NOTES.md` "Reusable objects [lab]"). WinCC OA reference panels take
`$`-parameters (`winccoa-comparison.md` §2.7, top-10 item 10).

**Now.** A faceplate definition (`pump`, `motor`, `valve` or `analog`) lists named
roles of kind `state`, `fault`, `analog`, `command` or `setpoint`, stored once
(`faceplates` and `faceplate_roles` tables; REST `GET/POST /api/v1/faceplates`,
`GET/PUT/DELETE /api/v1/faceplates/{id}`). A SCADA symbol stores only
`{faceplate: <id>, faceplateMap: {role: tag id}}`. The inspector picks a definition
(or saves a preset) and a tag per role; at runtime the symbol opens the definition
with its own tags; commands are armed, confirmed and sent through the command
queue and write policy. Deleting a definition that symbols use is refused (409,
lists them). The project format has a `faceplates` section; `faceplateMap` is
exported by tag ref and validated. Docs: `docs/screens.md` "Tag-swap faceplates",
`docs/project-format.md` "Faceplates".

**Commit.** 723ab88.

**Tests.** `faceplate::tests::a_valid_definition_has_no_problems`,
`faceplate::tests::problems_name_the_field`,
`faceplate::tests::unknown_fields_are_refused`,
`api::tests::faceplates::faceplates_are_defined_once_and_opened_by_many_symbols`,
`project_yaml::tests::import_recreates_everything_with_cross_references_intact`
(faceplate mapping follows the tags to new ids),
`project_yaml::tests::export_covers_every_entity_kind`;
`client/tests/faceplateMap.test.ts`.

**Not done.** A definition has no graphics of its own (the symbol draws the
equipment); roles are single tags (no arrays, no nested equipment); no MCP tools
for faceplates (agents use a project import); not tied to equipment types (WinCC
item 6, not started).

## 3. Linked device templates

**Motivation.** Rapid SCADA shares one Modbus device template file by reference
across every device that uses it, so a register-map fix reaches all of them
(`rapidscada/NOTES.md` "Reusable objects [lab]"; `COMPARISON.md` item 3). Voltrus
copied templates at creation (`docs/device-templates.md`; `winccoa-comparison.md`
§2.1 and top-10 item 6).

**Now.** A Modbus device can be linked to a stored template (`template_id`,
`version`, `source` columns on `modbus_devices`): at creation (`"linked": true`
on instantiate, REST and MCP) or later (`PUT /api/v1/modbus-devices/{id}/template`).
`PUT /api/v1/device-templates/{id}` saves a changed local or catalog template and
rewrites the registers of every linked device (poller restarted);
`?dry_run=true` lists the linked devices with registers added, removed and
changed. While linked, register edits answer 409; `DELETE .../template` unlinks
and keeps the registers as a copy. A template with linked devices cannot be
deleted. Copied devices keep working unchanged. The project format carries the
link next to the registers and validates it. UI: a link option in **Use**, an
**Update** button on library rows with the preview, and the link with **Unlink**
in the Modbus device list. Docs: `docs/device-templates.md` "Linked devices",
`docs/project-format.md`.

**Commit.** 59c4ce0.

**Tests.** `device_templates::link::tests::diff_names_added_removed_and_changed_registers`,
`api::tests::device_templates::linked_devices_follow_their_template_until_unlinked`
(preview, apply, copies untouched, 409 on edit and delete, unlink, relink, project
export and validation); `client/tests/deviceTemplates.test.ts` ("a linked device is
asked for on Modbus templates only", "the template save preview lists each linked
device and its register changes").

**Not done.** BACnet devices are always copies. Templates are edited by uploading a
changed file (no register editor for templates in the UI). A project import writes
the registers it lists and does not re-sync them from the template. Linking covers
register maps only, not alarm limits or tags (that is WinCC item 6, equipment
types).

## 4. Alarm chatter: on-delay suggestion and chatter count

**Motivation.** Rapid SCADA has no on-delay on limits; on the sim's ±0.3 NTU noise
with a 0.5 deadband the turbidity alarm chattered 5 times in 9 s
(`rapidscada/NOTES.md` "Alarms [lab]", `rapidscada/evidence/events-raw-turbidity.txt`,
`evidence/alarm-drill.log`; `COMPARISON.md` item 4). WinCC OA needed hysteresis to
stop a CAME/WENT chatter (`winccoa-comparison.md` §2.4).

**Now.** `GET /api/v1/alarms/analytics` returns per tag `max_per_hour` (most alarms
raised in one clock hour of the range) and `chattering` (5 or more).
`GET /api/v1/data-sources/{id}/alarm-suggestion` proposes an on-delay from the last
30 minutes of history and the last day of alarms: 0 s for a digital tag, 10 s when
chattering, 5 s when the noise (standard deviation of successive differences / √2)
exceeds a third of the alarm deadband, otherwise 2 s; plus a 3-sigma deadband. The
tag editor shows it with **Use** buttons and never changes the form by itself
(after review: an earlier version pre-filled the delay, which a save for any other
reason then stored); the analytics tab marks
`CHATTER n/h` with a hint. Docs: `docs/alarms.md`.

**Commit.** e47a391.

**Tests.** `alarm_chatter::tests::noise_ignores_a_slow_ramp`,
`alarm_chatter::tests::noisy_analog_gets_five_seconds_and_a_three_sigma_deadband`,
`alarm_chatter::tests::chattering_wins_and_quiet_or_unknown_analogs_get_the_default`,
`alarm_chatter::tests::digital_tags_get_no_delay`,
`alarm_chatter::tests::max_per_hour_counts_clock_hours`,
`api::tests::alarms::chattering_alarms_are_counted_and_get_a_longer_on_delay`;
`client/tests/alarmSuggestion.test.ts`.

**Not done.** Chatter is counted per clock hour, not in a sliding minute (ISA-18.2
uses alarms per minute); alarms are still evaluated in the 5 s persistence loop, so
delays round up to loop periods; no state (digital) alarms; the server never
applies a suggestion by itself.

## 5. Trend from any value, multi-pen, independent axes

**Motivation.** Rapid SCADA opens a chart from any value on a mimic, but the free
chart is one channel per chart (`rapidscada/NOTES.md` "History and trend [lab]").
FUXA's history chart left a pen off-scale because the y-axis did not autoscale
(`fuxa/NOTES.md` "What was easy / hard", `fuxa/evidence/history-daq.txt`;
`COMPARISON.md` item 5).

**Now.** A trend popup opens from the value on a dashboard card, from any bound
2D SCADA widget without a faceplate (operator controls excepted), from analog
values in a faceplate, and from a **trend** button in the tag list. **+ Add pen**
adds tags up to six; each pen has its own y axis, autoscaled to its data with an
8 % margin and drawn in its colour; flat and digital pens get fixed sensible axes;
ranges 15 min to 24 h, refreshed every 10 s. Docs: `docs/screens.md` "Trend popup".

**Commit.** 42db963.

**Tests.** `client/tests/trendModel.test.ts` (each pen is autoscaled to its own
data; flat, digital and empty pens; pens added once up to the limit; history in s
or ms becomes ms and spans all pens).

**Not done.** No zoom or pan in the popup (the tag detail page keeps those); not
wired into the 3D twin; no saved pen sets.

## 6. Strict validation and a JSON Schema

**Motivation.** FUXA stores enum *keys* and silently failed when given enum
*values*: an empty chart and a button that did nothing, with no error anywhere
(`fuxa/NOTES.md` "History and trend [lab]" and "Process screen and command [lab]";
`COMPARISON.md` item 6).

**Before.** The project format already rejected unknown fields, but enumerated text
was not checked: a register type such as `uint32` was stored and the poller read
it as `float32`. The REST create/update routes for tags, screens and Modbus devices
ignored unknown fields.

**Now.** Enumerated fields of the project format (Modbus `register_type`,
`register_area`, `endianness` on devices and templates, template `protocol` and
BACnet `object_type`, SCADA `connection_type`, faceplate kinds) are checked while
parsing; the error names the field and lists the allowed values.
`docs/project-schema.json` (JSON Schema 2020-12) is generated from the parser's
field lists (`voltrus project schema [OUT]`) and a test fails when it is stale.
`POST/PUT /api/v1/data-sources`, `/api/v1/screens`, `/api/v1/modbus-devices` and
`PUT .../registers` answer 400 naming an unknown field or a bad enum value; the
fields `GET` returns are accepted so read-modify-write works. MCP tools already
refused unknown arguments and go through these routes. Docs:
`docs/project-format.md` (enum table, "JSON Schema", "The REST routes").

**Commit.** 51b80fc (faceplate and template-link fields added to the schema in
723ab88 and 59c4ce0).

**Tests.** `project_yaml::schema::tests::committed_schema_matches_the_spec`,
`project_yaml::schema::tests::schema_lists_every_section_and_closes_every_object`,
`project_yaml::schema::tests::enum_fields_reject_unknown_values_naming_the_field_and_the_choices`,
`project_yaml::schema::tests::unknown_fields_are_rejected_at_every_level`,
`project_yaml::tests::invalid_device_templates_are_reported_with_their_key`,
`api::strict::tests::unknown_fields_and_bad_enum_values_are_named`,
`api::strict::tests::modbus_registers_are_checked_one_by_one`,
`api::tests::strict_bodies::data_source_routes_reject_unknown_fields`,
`api::tests::strict_bodies::screen_routes_reject_unknown_fields`,
`api::tests::strict_bodies::modbus_routes_reject_unknown_register_types`.

**Not done.** `chart_type` and `source_type` stay open text (older versions and the
setup wizard store values such as `led`; protocol modules add source types). OPC UA
security names are not enumerated (stored spellings differ). Other untyped routes
(SCADA layout save, the other protocols' device routes, flows) are not yet strict.
A database holding an invalid value from before now fails to re-import until the
value is fixed; the error says which.

## 7. API- and MCP-made screens stay editable (round trip)

**Motivation.** In both FUXA and Rapid SCADA, screens built by script opened
cleanly in the web editors (`fuxa/NOTES.md` build table, `fuxa/build/fuxa_view.py`;
`rapidscada/NOTES.md`, `rapidscada/build/build_mimic.py`; `COMPARISON.md` item 7).
This is what makes generating screens with the MCP server credible.

**Now.** Server tests build a 2D screen over REST and through MCP
(`project_apply_import`), read it back, save the result again as the editor does,
and re-import the project export; rows and project text stay identical. The
editor's save moved to `client/src/scada2/engine/layout.ts` (`saveLayoutToAPI`) next
to the loader and has its own load-then-save test. The tests found two gaps, now
fixed: the editor renamed equipment that the API or a project file had named
(the loaded name is now kept until the tag changes), and a rotation set only in the
row's column was not shown.

**Commit.** 4c4174d.

**Tests.** `api::tests::screen_round_trip::scada_screen_made_over_rest_saves_back_identically`,
`api::tests::mcp::mcp_screen_from_a_project_import_round_trips`;
`client/tests/scadaRoundTrip.test.ts` (rows the editor wrote load and save back
identically; rows made by the API or a project import keep name, rotation and
order, adding only the stacking order `z` and a rotation the column already had).

**Not done.** The client test covers the editor model's load and save functions,
not a browser session. MCP has no tool that edits a SCADA layout directly (agents
use `project_apply_import`).

## 8. Drivers ship in the binary

**Motivation.** Since 1.3.x FUXA installs its drivers (Modbus, OPC UA, S7, ...) as
npm packages at run time from *Setup → Plugins*, which needs internet; the install
lives outside the volumes and disappeared when the container was recreated
(`fuxa/NOTES.md` install section; `COMPARISON.md` "Install / footprint" row and
item 8).

**Now (already true, now tested and documented).** All protocol drivers are Cargo
features on by default and the release workflow builds with the defaults, so the
release binary and the container carry every driver and nothing is installed at
run time. A unit test fails if a protocol leaves the default features or the
release build starts passing feature flags. README "Drivers ship in the binary"
also lists what the server does fetch, and only when used: catalog device templates
(JSON), the configured AI provider, notification and webhook targets.

**Commit.** cdd3bb9.

**Tests.** `drivers_in_the_binary::the_default_build_enables_every_protocol_driver`,
`drivers_in_the_binary::the_release_workflow_builds_with_default_features`.

**Not done.** Nothing.

## 9. Official Docker image with measured footprint

**Motivation.** Measured in the lab: FUXA image 936 MB, 81 MiB RAM idle; Rapid
SCADA 616 MB, 211 MiB; Ignition 2.1 GB compressed (`COMPARISON.md` "Install /
footprint" row and item 9). WinCC OA has official Docker support
(`winccoa-comparison.md` §2.13); Voltrus had none.

**Now.** `Dockerfile`: multi-stage (Node 22 client, `rust:1-alpine` static musl
server, Alpine 3.21 runtime as uid 10001), `/data` volume, health check on
`/api/v1/health`, an entrypoint that writes `/data/config.yaml` with a random
`auth_secret` on first start; `--target prebuilt` packages a release binary
instead. CI job `docker` builds the image, starts it and checks health; it pushes
nothing. Measured (2026-11-25, Ryzen 5 PRO 4650U, image from the v0.52.0 release
binary via `--target prebuilt`): 103 MB image, 26.9 MB compressed; idle with the
factory-floor scenario (one Modbus meter, 7 registers at 1 s, embedded MQTT broker,
history every 5 s) 29.3–29.5 MiB (`VmRSS` 28.3 MiB, peak 33.4 MiB) and 0.11–0.52 %
CPU, mean 0.22 % of one core. Docs: `docs/docker.md`, README "Run with Docker".

**Commit.** da4e0ab.

**Tests.** CI job `docker` in `.github/workflows/ci.yml`. Locally: `docker build
--check` passes; the prebuilt image started, wrote its config and answered
`/api/v1/health`.

**Not done.** No image in a registry (no push workflow existed; no secrets added).
Single architecture per build, no multi-arch manifest. The from-source target was
not built locally (disk space); it runs in CI. The scenario measured has one
polled device; the FUXA and Rapid SCADA memory figures are for a 47-tag project, so
only the image sizes compare like for like.

## 10. Command attribution in the UI

**Motivation.** Rapid SCADA's Server log records `Command to channel 1035 from user
with ID 11` (`rapidscada/NOTES.md` "Process screen and command [lab]",
`rapidscada/evidence/write-back.log`; `COMPARISON.md` item 10). Voltrus had the data
(`created_by` on every command, a hash-chained audit log) but showed the raw actor
string (`api_key:3`) and could not tell an MCP write from a REST one.

**Now.** Commands store the front end that queued them (`origin`: `session`,
`api_key`, `mcp`, `flow`, `recipe`); reads return `source: {kind, label}` with the
API key's or flow's name looked up. The command panel and each step of a command's
audit trail, the alarm audit tab and acknowledged alarms show a badge (USER, API
KEY, MCP, FLOW, RECIPE) and the name. Docs: `docs/live-stream.md` "Who sent a
command".

**Commit.** 47233f3.

**Tests.** `storage::tests::command_tests::commands_name_their_sender`,
`api::tests::mcp::mcp_write_tag_respects_write_policy` (an MCP write is stored with
origin `mcp` and labelled with the key's name); `client/tests/commandSource.test.ts`.

**Not done.** Commands written before this change have no origin and are classified
from `created_by` (an old MCP write shows as its user or key). Alarm actions record
the user or key but not whether MCP was the front end; their badge is derived from
the actor string in the client, so an API key shows as `API KEY #<id>` without its
name.
