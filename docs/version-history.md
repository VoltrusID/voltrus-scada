# Version history

Designer phase 3 ([designer.md](designer.md#phase-3-version-history-built)):
every change to a screen's components and to the project's equipment types,
equipment instances and faceplates is logged with who, when and what; screens
get named versions, a diff between any two states, revert of one object or a
whole screen, and a draft/published split with an optional approval step. The
published project can be pushed to a Git remote.

Nothing changes for a site that never publishes: a screen that has never been
published shows its draft to everybody, exactly as before.

## The change log

Every write of a design object appends one row per added, changed or removed
object to `design_changes`, all rows of one write under one **batch** id:

| Column | |
|---|---|
| `seq` | increasing number, the log's order |
| `batch` | one save, import, revert, rollback… (`design_batches` has its note) |
| `screen_id` | the widget's or connection's screen; `""` for project objects |
| `object_type` | `widget`, `connection`, `equipment_type`, `equipment_instance`, `faceplate` |
| `object_id` | `equipment_id`, `connection_id`, type id, instance name, faceplate id |
| `kind` | `add`, `update`, `remove` |
| `before`, `after` | the stored row as JSON (`null` on add / remove) |
| `username` | user name, `api_key:<id>`, `cli:<os user>` or `system` |
| `source` | `user` (a session), `designer` / `native_designer` / `editor` (a session that names its editor with the `X-Voltrus-Client` header), `api_key`, `mcp`, `copilot`, `flow`, `cli`, `system` |
| `operation` | `save`, `import`, `equipment`, `faceplate`, `revert`, `rollback`, `tag_rename`, `wizard` |
| `at_ms`, `squashed` | when; how many earlier rows were merged into this one |

The rows are written by the server inside the transaction of the write, by
diffing the objects the write may touch before and after it. So every path is
logged without client changes:

| Path | Logged as |
|---|---|
| `PUT …/scada-layout`, `…/scada-equipment`, `…/scada-connections` (Designer, old editor, native console, scripts) | `save`, source from the credential and `X-Voltrus-Client` |
| Project import (REST, MCP `project_apply_import`, `voltrus project import`) | `import`, one batch for all screens and project objects |
| Equipment types and instances (REST, MCP, the Designer's instance editor), starter library install, instance/type delete | `equipment`, including the re-bound symbols on every screen |
| Faceplates (`/api/v1/faceplates`) | `faceplate` |
| Config snapshot rollback, go-live rollback | `rollback` |
| A tag's key or device renamed (binding refs rewritten) | `tag_rename`, `system` |
| FEED wizard placing equipment | `wizard`, `system` |
| Revert | `revert` |

A write that changes nothing appends nothing (no batch). The audit log keeps
its one line per write and names the batch (`batch=<id>` in its details).

The AI copilot's apply writes grid dashboard layouts, not 2D screen rows, so
it does not appear in this log; a future copilot write of 2D rows goes through
the same save path with source `copilot`. Flows do not write design objects.

### Optimistic locking

A screen's **head** is the highest `seq` touching it. `GET …/scada-layout`
returns it as `seq` next to phase 1's `revision` (the hash of the rows). A save
may send `base_seq`, `base_revision`, or both; when the screen moved on, the
answer is 409 with the current `revision` and `seq` and `changes_since`: the
batches after `base_seq` with who, when and which objects, so the Designer can
show who changed what before the user reloads or overwrites. Writers that send
no base keep overwriting, now logged.

### Retention

Site setting `design_history` (`PUT /api/v1/settings/design-history`, admin):

| Field | Default | |
|---|---|---|
| `squash_window_secs` | 60 | Consecutive `save`s of one object by one author through one source within this window become one row (first `before`, last `after`). A row that a version points at is never merged; an add followed by a remove in the window leaves nothing. 0 turns it off. |
| `squash_after_days` | 30 | A daily job squashes rows older than this to one per object per hour, never across a version's `upto_seq`. 0: never. |
| `keep_days` | 0 | Rows older than this are deleted, except the rows versions point at. 0: keep forever. |
| `publishing` | `true` | The draft/published flow (below). |
| `approval` | `none` | `required`: publishing needs a second admin. |

Versions keep full snapshots, so squashing or pruning never breaks restoring
one. A state by `seq` (`seq:<n>`) is reconstructed from the log (the draft
with every later change undone) and is exact while the rows after `n` are not
squashed across it.

## Versions

`POST /api/v1/screens/{id}/versions {name?, message?}` records the current
draft as a version: name (default `v1`, `v2`, … unique per screen), message,
author, the head `seq` it is as of, and the rows (snapshot). Versions are
listed newest first with a `status`: `saved`, `pending` (a publish waiting for
approval), `published` (published at least once) or `rejected`, and
`current: true` on the one operators see.

## Diff

`GET /api/v1/screens/{id}/diff?from=&to=` compares two **states**: `draft`,
`published`, `seq:<n>` (after the batch that change `n` belongs to), or a
version id or name (defaults: from `published` to
`draft`, i.e. what publishing would change). Per widget and connection:
`added`, `removed`, and `changed` with the changed fields (`x`,
`config.bindings`, …), a `summary`, both states as project YAML (tags by ref,
like `screens[].scada` in a project file) and the rows for a side-by-side
render. `GET …/state?at=` returns the rows of one state (the History pane's
read-only preview).

## Revert

`POST /api/v1/screens/{id}/revert` writes the old state as new changes (a
`revert` batch); history is never deleted.

* Whole screen: `{to: "<version>" | "published" | "seq:<n>"}` or `{to_seq}`. A
  snapshot restore, consistent by construction.
* One object: `{object, object_type?, to_seq | to}`; several as one batch:
  `{objects: [{object_id, object_type?, to_seq | to}]}`. The rules:
  * Reverting a widget to a state where it did not exist removes it and, in
    the same batch, every connection ending on it (`removed_connections`).
  * Restoring a connection whose end widget does not exist is refused: 409
    `dangling_connection` with `missing` widgets, unless the request restores
    them too (the Designer offers *Also restore …*).
  * A restored widget whose `config.group` has no other member is restored
    ungrouped (`ungrouped`); `config.z` is kept.
* Optional `base_seq`/`base_revision` (409 as for saves) and `message` (in the
  batch's note).

Revert changes the draft; publish to show it to operators.

Project objects: `POST /api/v1/design/revert {object_type, object_id, to_seq}`
puts an equipment type, instance or faceplate back as it was after `to_seq`,
through its own write path (a type re-applies to every instance, with the
write-policy signature where the edition wants one). Reverting a type or
instance to before it existed is refused: delete it instead.

## Draft and published

`scada_equipment`/`scada_connections` stay the **draft**. Publishing copies a
version's rows into `scada_equipment_published`/`scada_connections_published`
and records it in `design_screen_state`.

| Path | Writes | Reads |
|---|---|---|
| Designer, old editor, native console editor | draft | draft |
| REST layout `PUT`s, MCP, scripts, the copilot | draft; publishing is a separate, explicit step | draft, or published with `?published=1` |
| Roles that cannot configure (operator, viewer, read-only API keys) | — | always published (`?published` ignored) |
| 2D runtime (web) | — | `?published=1` |
| Project import | draft for every screen it touches; option *publish* (`?publish=1`, MCP `publish: true`, CLI `--publish`) publishes those screens in the same transaction | — |
| Project export | — | draft (the project file is the engineering source); `?published=1` / `--published` / MCP `published: true` exports what operators see. Each published screen carries `published_version: <name>`, which import ignores |
| Tag rename | rewrites binding refs in draft (logged) and published rows | — |

A screen never published reads as its draft everywhere (`view: "draft"` in
the GET answers). With `publishing: false` every reader sees the draft and
publishing is refused (409 `publishing_off`); the published copies are kept
for when it is turned back on. Deleting a screen drops its published copy;
its log and versions stay.

Equipment instance and type changes re-bind the draft's symbols; the published
copy keeps its bindings until the screen is published again.

### Publish

`POST /api/v1/screens/{id}/publish {version?, name?, message?, base_seq?,
signature?}` (configure roles: admin, demo, integration keys). Without
`version` the current draft is recorded as a version first (`name`,
`message`); `base_seq` refuses when the draft moved on since the user looked
at it. The answer is `{status: "published", version}`.

`GET …/design-status` gives the title bar its line: the head `seq`, the
published version, `changes_since_published`, `draft_differs`, the last
version and the pending requests.

### Approval

With `approval: required`, a publish files a **request** instead (`status:
"pending"`, `requested_by`). An administrator other than the requester, signed
in (not an API key or MCP), approves it (`POST …/versions/{v}/approve {note?,
signature?}`), which publishes exactly the rows that were requested, or rejects
it (`…/reject {note?}`). The reviewer, time and note are kept on the version.

### Electronic signatures (21 CFR Part 11)

Publishing is the signed action `design_publish` of the edition hooks
([edition-hooks.md](edition-hooks.md)). The core asks the edition
(`Extension::signature_required`) with resource `screen:<id>:publish` and
`details.stage`:

| Stage | When | Who signs |
|---|---|---|
| `publish` | direct publish (no approval) | the publisher |
| `request` | a publish request (approval required) | the author |
| `approve` | approving a request | the reviewer |

`details` also has `screen_id`, `version`, `message` and the diff `changes`
summary (or `requested_by` on approval). Core asks for nothing; an edition that
answers with a requirement gets the usual 428 → signed retry, the signature id
on the version (`signature`) and in the audit entry. API keys and MCP cannot
sign, so such an edition makes publishing a person's act.

## Git sync (optional)

Admin settings `GET`/`PUT /api/v1/design/git`:

| Field | |
|---|---|
| `enabled` | on/off |
| `remote` | `https://…`, `ssh://…`, `git@host:org/repo.git`, or the path of a bare repository |
| `branch`, `path` | default `main`, `project.yaml` (a relative path inside the repository) |
| `auth` | `none`, `token` (HTTPS; `username` e.g. `x-access-token` for GitHub, your user for Gitea), `ssh_key` (a deploy key) |
| `secret` | the token or private key; write-only, encrypted at rest (`enc:v1:` like the copilot's API key), never returned (`has_secret`) |
| `push_on_publish` | push after every publish (default on) |
| `author_email_domain` | commits are authored `<user>@<domain>` (default `voltrus.local`) |
| `work_dir` | the working copy (default `design-git/` beside the database) |

On publish (in the background) or `POST /api/v1/design/git/sync {message?}`
(manual, waits for the result) the server exports the project with each
screen's **published** rows as one `voltrus-project/1` file, commits it with
the version's message authored by the publishing user, and pushes. A file
that differs only in `generated_at` is not committed. The last outcome is
shown in `GET /api/v1/design/git` (`status`).

It uses the `git` command line, not a library: nothing is added to the single
binary, and the server needs `git` (and `ssh` for SSH remotes) on its path.
Without git the export is still written to the working folder and the sync
reports that nothing was pushed. Credentials never appear on a command line:
a token goes in an `http.extraHeader` through git's environment, a deploy key
in a 0600 file next to the working copy for the length of the sync. Only the
`file`, `git`, `http(s)` and `ssh` transports are allowed, hooks and credential
helpers are off, and remotes or branches starting with `-` are refused.

Pull stays manual: take the file from the repository and import it through
the ordinary project import plan (`/api/v1/project/import?dry_run=true`, then
apply), so a merge shows as a plan, never as a half-written screen. Git is
never on the operator path.

## MCP

`list_changes`, `list_versions`, `create_version`, `diff_versions`,
`revert_screen`, `publish_screen` (propose, then `apply: true`; admin only).
See [mcp.md](mcp.md#tools).

## REST summary

| Route | Tier |
|---|---|
| `GET /api/v1/screens/{id}/history?object=&object_type=&user=&batch=&before_seq=&limit=` | configure |
| `GET /api/v1/design/history?screen=&…` (`screen=` empty: project objects) | configure |
| `GET /api/v1/screens/{id}/blame` | configure |
| `GET /api/v1/screens/{id}/design-status` | configure |
| `GET`/`POST /api/v1/screens/{id}/versions`, `GET …/versions/{v}` | configure |
| `GET /api/v1/screens/{id}/diff?from=&to=&yaml=`, `GET …/state?at=` | configure |
| `POST /api/v1/screens/{id}/revert`, `POST /api/v1/design/revert` | configure |
| `POST /api/v1/screens/{id}/publish` | configure |
| `POST /api/v1/screens/{id}/versions/{v}/approve`, `…/reject` | admin session, not the requester |
| `GET /api/v1/settings/design-history` | configure |
| `PUT /api/v1/settings/design-history`, `GET`/`PUT /api/v1/design/git`, `POST /api/v1/design/git/sync` | admin |
| `GET …/scada-equipment`, `…/scada-connections`, `…/scada-layout` `?published=1` | any (non-configure roles always get published) |

## Native console

The native Designer (`native/src/designer.rs`) and the console's mimic editor
save through the same routes with `base_revision`, so their saves are logged
(source `user`) and their conflicts still answer 409. Operators on the console
get the published rows from the server by their role; an admin's console view
shows the draft. Follow-ups for the console: send `X-Voltrus-Client:
native-designer` and `base_seq`, ask for `?published=1` in run mode, and a
History pane over the routes above.
