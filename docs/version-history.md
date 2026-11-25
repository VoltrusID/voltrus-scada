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

A screen's **head** is the highest `seq` any write of it was given. It never
goes down: when squashing or pruning delete a screen's newest rows the head
stays, and a write whose rows cancel out (moved and moved back) still gets a
new head, so a `base_seq` never matches a later state by accident. `GET …/scada-layout`
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
| `squash_window_secs` | 60 (0-3600) | A `save` absorbs the previous batch of the same author, source and operation within this window when that batch is the latest of its screens, no version points at it, and the new save changes (and does not remove) every object it changed: each row keeps the first `before` and takes the last `after`. A batch is absorbed whole or not at all, so every state the log rebuilds is one the screen had (a connection never comes back without its widgets). A removal is never squashed into the add before it. 0 turns it off. |
| `squash_after_days` | 30 (0-36500) | A daily job squashes rows older than this to one per object, author, source and operation per hour, never across a version's `upto_seq`. 0: never. |
| `keep_days` | 0 (0-36500) | Rows older than this are deleted, except the rows versions point at. 0: keep forever. |
| `publishing` | `true` | The draft/published flow (below). |
| `approval` | `none` | `required`: publishing needs a second admin. |

Changing `publishing` or `approval` is the signed action `design_publish`
(stage `settings`). While approval is required, a change that drops it or
turns publishing off is not applied: it is filed (answer 202 `{status:
"pending", request}`, shown as `pending_request` in the GET) and another
admin, signed in, applies it with `POST …/settings/design-history/approve
{note?, signature?}` (stage `settings_approve`) or drops it with `…/reject`.
Both are audited. An API key or a demo account cannot file one.

Versions keep full snapshots, so squashing or pruning never breaks restoring
one. A state by `seq` (`seq:<n>`) is reconstructed from the log (the draft
with every later change undone). The daily squash merges rows across batches,
so every seq it deleted or merged raises the history's **floor**: a `seq:`
state at or below it, or a seq a live squash removed, is refused with 410
`state_gone` (the History answer carries `floor`). Pick a later change or a
version instead.

## Versions

`POST /api/v1/screens/{id}/versions {name?, message?}` records the current
draft as a version: name (default `v1`, `v2`, … unique per screen), message,
author, the head `seq` it is as of, and the rows (snapshot). Versions are
listed newest first with a `status`: `saved`, `pending` (a publish waiting for
approval), `published` (published at least once) or `rejected`, and
`current: true` on the one operators see. Each keeps its widget and connection
counts, so a list parses no snapshot.

A version is referred to as `id:<n>`, `name:<name>`, or bare: a bare reference
is a name, or an id when it is all digits. Names of digits only, `draft`,
`published` and names starting with `seq:`, `id:` or `name:` are refused, so
the two never meet.

## Diff

`GET /api/v1/screens/{id}/diff?from=&to=` compares two **states**: `draft`,
`published` (what operators see: the published rows, or the draft when the
screen was never published or publishing is off; the state label says which
with `shows` and `publishing`), `seq:<n>` (after the batch that change `n`
belongs to), or a version (defaults: from `published` to
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
  snapshot restore with the same rules as below: a connection whose widget is
  not in the state is left out (`removed_connections`) and a lone group member
  comes back ungrouped (`ungrouped`).
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
* `missing_tags` in the answer lists tag ids the restored widgets are bound to
  that no longer exist: they come back with those bindings unresolved.

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
| REST layout `PUT`s, MCP, scripts, the copilot | draft; publishing is a separate, explicit step | draft (`?draft=1` says so explicitly), or published with `?published=1` |
| Roles that cannot configure (operator, viewer, read-only API keys) | — | always published (`?published`/`?draft` ignored) |
| 2D runtime (web) | — | `?published=1`; its Edit mode reads the draft, and after *Done* a configurer sees "the draft differs" with a link to publish in the Designer |
| Native console | its editor: draft | run mode `scada-layout?published=1`, with a note to configurers when the draft differs (`draft_differs` in the answer) |
| Project import | draft for every screen it touches; option *publish* (`?publish=1`, MCP `publish: true`, CLI `--publish`) publishes those screens in the same transaction, with the rules of a publish (below) | — |
| Project export | — | draft (the project file is the engineering source); `?published=1` / `--published` / MCP `published: true` exports what operators see. Each published screen carries `published_version: <name>`, which import ignores |
| Tag rename | rewrites binding refs in the draft (logged), the published rows and every saved version, so an older version published later binds the renamed tag | — |
| Config snapshot and go-live rollback | draft and published rows, as they were at the snapshot (the log and versions stay) | — |

A screen never published reads as its draft everywhere (`view: "draft"` in
the GET answers). With `publishing: false` every reader sees the draft and
publishing is refused (409 `publishing_off`); the published copies are kept
for when it is turned back on. Deleting a screen (REST, or an import in
`replace` mode) drops its published copy in the same transaction; its log and
versions stay.

Equipment instance and type changes re-bind the draft's symbols; the published
copy keeps its bindings until the screen is published again, except that
deleting an instance unbinds it in the published copy too. A faceplate that a
published screen still opens cannot be deleted (409), even if the draft no
longer uses it.

Integration keys configure, so like admins they read the draft by default
(unchanged from before phase 3). A script that shows rows to operators should
ask for `?published=1`; one that edits should say `?draft=1`.

### Publish

`POST /api/v1/screens/{id}/publish {version?, name?, message?, base_seq?,
signature?}` (configure roles: admin, demo, integration keys). Without
`version` the current draft is recorded as a version first (`name`,
`message`); `base_seq` refuses when the draft moved on since the user looked
at it. The base is checked before any signature is asked for. Without
`base_seq` the server pins the head it read the diff at: the signature covers
that diff (from what operators see to the actual target, the named version or
the draft), and the publish is refused (409) when the draft or the published
version moved in between. The answer is `{status: "published", version}`.

**Publish after import** follows the same rules: refused (409
`publishing_off`) while publishing is off, before anything is written; with
approval required it files a publish request per screen (`publish_requested`
in the plan) instead of publishing; it is the signed action `design_publish`
(resource `project:import:publish`, `details: {stage, source: "import", mode,
document_sha256}`), the REST signature in the `X-Voltrus-Signature` header
(JSON) since the body is YAML. The command line cannot sign: `--publish` is
refused where the edition wants a signature. A screen whose draft is already
what operators see gets no new version, and the newest version is reused when
it is the draft.

`GET …/design-status` gives the title bar its line: the head `seq`, the
published version, `changes_since_published`, `draft_differs`, the last
version and the pending requests.

### Approval

With `approval: required`, a publish files a **request** instead (`status:
"pending"`, `requested_by`). Requests come from people: an API key or a demo
account is refused (403 `approval_needs_person`). An administrator signed in
(not an API key or MCP) who is none of the requester, the version's author and
the authors of the changes it holds beyond the published version (an API key
counts as the person who created it) approves it (`POST
…/versions/{v}/approve {note?, signature?, confirm_older?}`), which publishes
exactly the rows that were requested, or rejects it (`…/reject {note?}`);
otherwise 403 `self_review`. The check and the status change are one
statement in the publishing transaction, so of two approvals at once one
publishes and the other gets 409 `not_pending`. Approving a version older than
the one operators see now (it would roll the screen back) needs
`confirm_older: true` (else 409 `older_than_published`). The reviewer, time
and note are kept on the version.

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
| `remote` | `https://…`, `ssh://…`, `git@host:org/repo.git`, or the path of a bare repository inside the data folder (the database's folder). No credentials in it: a remote with a password (or any user for http(s)) is refused, so nothing secret is stored in plain text or shown |
| `branch`, `path` | default `main`, `project.yaml` (a relative path inside the repository) |
| `auth` | `none`, `token` (`https://` only, never plain `http://`; `username` e.g. `x-access-token` for GitHub, your user for Gitea), `ssh_key` (a deploy key, ssh remotes) |
| `secret` | the token or private key; write-only, encrypted at rest (`enc:v1:` like the copilot's API key), never returned (`has_secret`) |
| `push_on_publish` | push after every publish (default on) |
| `author_email_domain` | commits are authored `<user>@<domain>` (default `voltrus.local`) |
| `work_dir` | the working copy, a folder relative to the data folder (default `design-git`) |

On publish (in the background) or `POST /api/v1/design/git/sync {message?}`
(manual, waits for the result) the server exports the project with each
screen's **published** rows as one `voltrus-project/1` file, commits it with
the version's message authored by the publishing user, and pushes.

**The file is the whole project, not only the screens**: devices with their
addresses (IP, ports, unit ids), tags, alarms, flows, recipes, equipment and
settings. Secrets are written as `${ENV}` references, but everything else
about the plant goes to the remote host. Point it at a repository you would
trust with the plant's configuration. A file
that differs only in `generated_at` is not committed. The last outcome is
shown in `GET /api/v1/design/git` (`status`).

It uses the `git` command line, not a library: nothing is added to the single
binary, and the server needs `git` (and `ssh` for SSH remotes) on its path.
Without git the export is still written to the working folder and the sync
reports that nothing was pushed. The working copy is a bare repository: the
commit is built from the export with `hash-object`, `update-index` (on an
index file of its own), `write-tree` and `commit-tree`, and nothing is ever
checked out, so a symbolic link in the remote repository cannot make the
server read or write outside it; a link on the project file's path in the
remote tree is refused. Credentials never appear on a command line: a token
goes in an `http.extraHeader` through git's environment, a deploy key in a
0600 file in the data folder for the length of the sync (removed after it, and
any left by a crash at the next start or sync), passed to `ssh` as a quoted
word. Only the remote's own transport is allowed (`GIT_ALLOW_PROTOCOL`),
redirects are not followed, the system and user git configuration are not
read, hooks and credential helpers are off, and remotes or branches starting
with `-` are refused.

Pull stays manual: take the file from the repository and import it through
the ordinary project import plan (`/api/v1/project/import?dry_run=true`, then
apply), so a merge shows as a plan, never as a half-written screen. Git is
never on the operator path.

## MCP

`list_changes`, `list_versions`, `create_version`, `diff_versions`,
`revert_screen`, `publish_screen` (propose, then `apply: true` with the
proposal's `base_seq`, so a draft that changed after the user saw the diff is
not published; admin only).
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
| `POST /api/v1/screens/{id}/versions/{v}/approve`, `…/reject` | admin session, none of the requester, author or change authors |
| `GET /api/v1/settings/design-history` | configure |
| `PUT /api/v1/settings/design-history`, `GET`/`PUT /api/v1/design/git`, `POST /api/v1/design/git/sync` | admin |
| `POST /api/v1/settings/design-history/approve`, `…/reject` | admin session, not the requester |
| `GET …/scada-equipment`, `…/scada-connections`, `…/scada-layout` `?published=1` / `?draft=1` | any (non-configure roles always get published) |

## Native console

The native Designer (`native/src/designer.rs`) and the console's mimic editor
save through the same routes with `base_revision`, so their saves are logged
(source `user`) and their conflicts still answer 409. Operators on the console
get the published rows from the server by their role. The console's run mode
asks for `?published=1` for every role, so an admin sees what operators see,
with a note when the draft differs; its editor reads the draft (entering it
reads the draft first, leaving it reads the published copy again).
Follow-ups for the console: send `X-Voltrus-Client: native-designer` and
`base_seq`, and a History pane over the routes above.
