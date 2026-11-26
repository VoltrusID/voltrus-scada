# Live collaboration in the Designer

Phase 4 of the Designer ([designer.md](designer.md#phase-4-live-collaboration-built)).
Several engineers edit one screen at the same time, in the web Designer or
the native console, and see each other's cursors, selections and edits as
they happen. Everything runs on the plant network: the Voltrus server is the
relay and the authority, nothing leaves the site, there is no cloud service.

## How it works

```
 browser (Yjs)        browser (Yjs)        native console (yrs)
      │  y-sync over /api/v1/ws (MessagePack frames)   │
      └──────────────┬───────────────┬─────────────────┘
                     ▼
        server: one yrs document per open screen draft
                     │ every ~2 s and when the last person leaves,
                     ▼ through the same save path as REST
        scada_equipment / scada_connections (the draft) + design_changes
```

* **The document.** While anyone has a screen open in the Designer, the server
  holds its draft as a Yjs document (`yrs`, crate `collab/`, shared with the
  native console). It is built from the draft rows when the first person joins
  and dropped a minute after the last one leaves; the rows stay the truth
  (restart safety: a restarted server rebuilds the document from the rows).
* **Sync.** Clients exchange y-sync v1 messages (`y-protocols/sync`) with the
  server over the app's existing WebSocket, so sign-in, the 30-second access
  re-check and reconnects are the ones the live data already has. The server
  applies every update to its document and forwards it to the others.
* **Writing the draft.** About every 2 seconds, and at once when the last
  person leaves or someone presses Ctrl+S, the server turns the document into
  rows and writes the rows that changed (only those): equipment instances are
  bound, the change log gets the changes, the revision and `seq` move. Each
  object's change is logged under its author (whoever last changed it since
  the last write; the `config.z` renumbering a new stacking order causes
  belongs to whoever changed the order), one batch per author in the same
  transaction, squashed with that author's earlier batches by the change
  log's 60-second rule, so a session of small moves is one entry per object
  and person in History, blame names the right person and revert works as for
  any save. Each write also spreads out stacking indexes that grew long (over
  32 characters) and drops keys a screen document never holds. The
  background pass skips a session whose write is still running and writes
  the others concurrently.
* **Other writers.** REST and MCP layout `PUT`s, revert, publish and the
  project import take the same per-screen lock as the session's writes. While
  a session is open, a REST write first waits for the session's pending edits
  to be written (so its `base_seq`/`base_revision` is checked against what the
  editors see), then writes, then its difference is applied to the document
  as a server change: everyone sees it at once and nobody's work is
  overwritten. Other writers (equipment propagation, tag renames, a snapshot
  rollback) write the rows directly; the session notices the screen's change
  log moved (a per-screen write count) within half a second and merges their
  difference the same way. The project import and the FEED wizard write every
  session first and merge their rows in after.
* **Reads and versions** (`GET …/state`, `…/diff`, `…/blame`, `…/history`,
  creating a version over REST or MCP) write the session's pending edits
  first, so they show what the editors see.
* **Publishing** publishes the written draft: the session's pending edits are
  written first, under the lock. The Designer sends the `seq` of the summary
  the Publish dialog showed as `base_seq`; a draft that moved since gets a 409
  and the dialog shows the new summary.
* **Shutdown.** A server shutting down stops taking edits (`DesignError
  shutdown`: the client keeps them), then writes what the sessions hold. The
  clients send what was refused to the next session.

### The document

```
widgets:      Y.Map  id -> Y.Map { type, name, x, y, width, height,
                                    sensors: [tag ids], zi: "V",
                                    config: Y.Map { key -> value,
                                                    bindings: Y.Map { prop -> binding } } }
connections:  Y.Map  id -> Y.Map { from, to, type, waypoints: [[x, y], ...] }
```

* `config` is a nested map, so two people changing different properties of one
  component both keep their change; `config.bindings` is nested again, so two
  people binding different properties of one component merge too. One binding
  is replaced whole (the last writer wins).
* **Stacking order is defined once**: each widget's `zi` is a fractional index
  (`collab/src/fractional.rs`, `client/src/designer/collab/fractional.ts`), and
  the order is the widgets sorted by `(zi, id)`. There is no separate order
  list, so a concurrent reorder and delete cannot disagree. When the server
  writes rows it turns the order into the integer `config.z` (0, 1, 2, …) the
  rows have always had (only when the order changed), so REST, MCP, project
  files and the old editor never see fractional indexes. Loading rows derives
  `zi` from `config.z`.
* `config.rotation` and the rotation column are one fact (as the web's
  `loadLayoutFromAPI` treats them).
* A row whose meaning did not change is written back byte for byte, so the
  change log only shows real changes.

### Validation

A client is trusted with nothing it could not do over REST:

| Limit | Value | What happens |
|---|---|---|
| sync message | 256 KB | refused: `DesignError too_large`, the client reloads its document |
| sync messages | 50 a second per connection | refused: `rate_limited`; the client waits and joins again (clients pace themselves at 40 a second, merging and splitting updates under 256 KB) |
| document | 5 MB (measured again every half second) | refused: `doc_too_large` |
| nesting of a value | 32 levels | refused before decoding: `refused`, the client reloads |
| awareness state | 4 KB | dropped |
| awareness updates | 30 a second per connection | dropped |
| sessions | 8 per connection, 200 per server | refused: `too_many_sessions`, `unavailable` |
| silence | 120 s without any message (clients renew their presence every 15 s) | removed: `idle`; the client joins again |
| edits that fail validation | 10 a minute per connection | sent away: `invalid_edits`, the client reloads |

**Before an update is applied** its encoding is walked without recursion
(`collab/src/guard.rs`): values nested deeper than 32 levels (`yrs` and Yjs
decode them recursively; a few hundred kilobytes of `[[[[…]]]]` overflow a
stack), Yjs types other than maps and arrays, content other than plain
values (text, XML, embeds, formats, binary, subdocuments), top-level types
other than `widgets` and `connections`, counts larger than the message and
clocks that overflow are refused whole: nothing is applied or forwarded, and
the connection leaves the session (`refused`, reload). The native console
checks the server's updates the same way.

**As an update arrives** the server removes keys a widget or connection map
never holds (and root entries that are not maps), then checks the shape of
each object it touched (the checks below without the references). What fails
is put back at once as a server change, before the update is forwarded, so
the others see the valid state right after; the author gets `DesignRejected`.

**At every write** the server checks each object that changed since the last
write: known shape (widget type, numbers finite and within ±100,000, size
positive), `config` an object under 64 KB, binding shapes (`tag`, `expr`,
`map`, as project import checks them), tag refs and tag ids that exist (only
new ones: a binding to a tag deleted earlier stays as it was), equipment
instances and faceplates that exist, connection ends that exist. The view of
what exists is reused for 10 seconds and read again before anything is
refused. An object that fails is put back field by field as a server change:
of an object written before, only the fields whose change fails (all its
changed fields when only their combination fails); a new object, or a
connection whose end is gone, is removed. Every client converges on the valid
state. The rest of the write goes ahead; the object's valid fields are
checked again and written at the next write. Everyone who changed the object
since the last write is told which objects and fields and why
(`DesignRejected`, a toast), the rejection goes to the audit log
(`design_collab_rejected`) under each of them, and it counts against the
connections of whoever changed it last.

### Permissions

Joining needs a role that may configure (admin, demo, configure API keys);
viewers and operators are refused (`DesignError forbidden`) and never receive
the document or anyone's presence. Changing a role or a password, disabling
or deleting a user, signing out and revoking a key bump an access generation
(`storage::ACCESS`): every open WebSocket checks its grant at once and
closes when it no longer holds, and a session member's next `DesignSync`,
`DesignAwareness` or `DesignFlush` is checked again before it is applied. A
member who may no longer configure leaves every session (`forbidden`); one
whose access is gone also loses the socket. The 30-second re-check stays as
the backstop.

### Presence

Presence uses y-protocols awareness. The server forwards each client's state
to the others and keeps the latest per connection for newcomers; it is never
stored. The server overwrites the name and colour in it with the signed-in
user's (nobody can appear as someone else) and drops states over 4 KB and
updates over 30 a second. An awareness client id belongs to the connection
that first used it (at most 4 per connection): nobody else can set or clear
it. When a connection leaves or drops, the others get its removal at once.

## Wire protocol

All frames are MessagePack maps with a `type`, on `/api/v1/ws`
(`src/ws_types.rs`, mirrored in `native/src/ws_types.rs`). Binary payloads are
MessagePack `bin`.

| Client → server | |
|---|---|
| `DesignJoin {screen, protocol, epoch?}` | join; `protocol` is 1; `epoch` the session the client was in before a reconnect |
| `DesignSync {screen, data}` | one y-sync message: step 1, step 2 or update (`varuint type`, `varUint8Array payload`, no y-websocket outer tag) |
| `DesignAwareness {screen, data}` | y-protocols awareness update of this client |
| `DesignLeave {screen}` | leave |
| `DesignFlush {screen}` | write pending edits now |

| Server → client | |
|---|---|
| `DesignJoined {screen, epoch, resync, user, color, protocol, limits}` | joined; a `DesignSync` step 1 of the server document and a `DesignAwareness` with the others follow. The client answers the step 1 and sends its own |
| `DesignSync {screen, data}` | sync message (another client's update, or a server change) |
| `DesignAwareness {screen, data}` | other clients' presence |
| `DesignError {screen, code, message, reload}` | see below |
| `DesignRejected {screen, objects: [{object_type, object_id, reason}]}` | edits of objects you changed failed validation and were put back (`reason` names the fields) |
| `DesignFlushed {screen, seq, at_ms, users, acked?}` | written to the draft. `acked`: how many of this connection's `DesignSync` messages since its last `DesignJoined` the written state holds (step 1, step 2 and updates all count). Also sent with `users: []` when nothing needed writing, to acknowledge |
| `DesignSessions {sessions: [{screen, users: [{user, color}]}]}` | who edits which screen |
| `DesignComment {screen, event, comment}` | a comment was `created`, `updated` or `deleted` |

`limits` in `DesignJoined` are `{max_update, max_rate, max_doc,
max_awareness, flush_ms, idle_ms}`.

| `DesignError` code | reload | The client |
|---|---|---|
| `disabled`, `forbidden`, `protocol` | no | stops live editing and saves whole layouts (`protocol`: reload the page) |
| `resync` (a message could not be delivered, or the session closed), `not_joined`, `idle`, `rate_limited` | no | joins again with its document and epoch (after `rate_limited` it waits 1 s, longer when it repeats) |
| `shutdown` | no | keeps the refused edit and sends it to the next session |
| `too_large`, `doc_too_large`, `bad_message`, `refused`, `invalid_edits` | yes | drops its document and joins again |
| `not_found`, `too_many_sessions`, `unavailable`, `deleted`, `write_failed` | no | shows the message |

**Epochs, acknowledgements and reconnects.** Each session has an epoch (a
random id). Clients number the `DesignSync` messages they send from their
latest `DesignJoined` and keep, per message, which objects and fields its
edits changed until a `DesignFlushed` whose `acked` covers its number (a
write that crosses newer messages leaves those pending). A client that joins
again within the session's life sends its epoch and runs the normal y-sync
exchange: edits made while offline or dropped merge, carried by its step 2.
When the epoch differs (the server restarted, or the session closed),
`DesignJoined` says `resync`: the client takes the server's document and
writes every edit not yet acknowledged (sent or not, online or offline) into
it again, field by field: objects it created come back, objects it removed
go, only the fields it changed are written, and an object someone else
removed is not brought back. It tells the user how many changes were applied
again.

## Comments

Comments are pinned to a component or connection (or a point of the stage),
in threads: a comment and its replies. `design_comments` (id, screen,
parent, object, x, y, body, author, mentions, resolved, resolved by/at,
created and updated). `@name` mentions a user, who gets a notification. A
thread is resolved or reopened by anyone who may configure; a comment is edited
by its author and deleted by its author or an administrator. A mention
notifies a user once per comment (editing the text never notifies the same
person again; `design_comment_mentions` records who was told), only users
who may open comments (roles that configure, not disabled), the first 10
names of a comment, at most 30 notifications a minute per author. A user
writes at most 30 comments a minute (429 beyond); commenting on a screen that
does not exist is a 404; deleting a screen deletes its comments and ends its
live session (`deleted`). They show as pins
on the canvas (resolved ones hidden unless asked), in the Comments tab, and
in the History pane next to the changes.

| Route | |
|---|---|
| `GET /api/v1/screens/{id}/comments?resolved=1` | threads of the screen, newest first |
| `POST /api/v1/screens/{id}/comments` | `{body, object_id?, object_type?, x, y, parent_id?}` |
| `PATCH /api/v1/screens/{id}/comments/{cid}` | `{body?, resolved?}` |
| `DELETE /api/v1/screens/{id}/comments/{cid}` | |
| `GET /api/v1/design/sessions` | who edits which screen |

## In the Designer

* **Title bar:** *Live · 2 editing* (or *Offline · reconnecting*, or
  *Single-user* when collaboration is off), the avatars of the others (their
  initials in their colour). Click an avatar to **follow** that person: your
  view pans and zooms with theirs until you pan, zoom or press Esc.
* **Stage:** the others' cursors with their names, their selections outlined in
  their colour, comment pins.
* **Project pane:** who edits which screen (their initials next to it).
* **Saving:** there is nothing to save: the button says *Saved* (or *Saving…*
  while edits wait to be written, until the server acknowledges them), Ctrl+S
  writes at once. No conflict dialog; closing the tab while any edit is not
  yet written to the draft (queued, sent but not acknowledged, or offline)
  asks first.
* **Undo** (Ctrl+Z / Ctrl+Y) takes back only your own edits, one edit at a
  time as in phase 1 (typing in one field and nudges coalesce).
* **Offline:** when the socket drops, editing goes on; the edits are sent when
  it comes back (merged with everyone else's). If the server restarted in
  between, every change not yet written is applied to its document again,
  field by field, and a toast says how many. When the server drops the
  Designer from the session (a slow link, a burst over the rate, a long
  silence), it joins again on its own and says so once.
* **Rejected edits:** a toast names the objects the server put back and why.
* **Comments:** the Comment tool (M, or its toolbar button) drops a pin
  on a component or a point; the Comments tab lists threads with replies,
  *Resolve* / *Reopen*, delete, `@` completion of user names.

The native console joins the same sessions with `yrs`: live edits, presence
(avatars, cursors, selections) and per-user undo; comments, follow mode and
who-edits-which-screen are web-only for now
([native-designer.md](native-designer.md#live-collaboration-phase-4-yrs-natively-built)).

## Code

| Where | What |
|---|---|
| `collab/src/document.rs` | rows ↔ Yjs document, field-level writes, merging outside writes (`apply_rows`), rows out with `config.z` |
| `collab/src/fractional.rs`, `client/src/designer/collab/fractional.ts` | fractional indexes (the same keys on both sides) |
| `collab/src/wire.rs` | y-sync messages and awareness updates |
| `collab/src/guard.rs` | the structural check of an update before it is decoded |
| `src/collab/mod.rs` | sessions, the per-screen lock, sync, presence, writes per author, limits, idle close |
| `src/collab/validate.rs` | what a write checks |
| `src/api/collab.rs`, `src/storage/design_comments.rs` | sessions list and comments over REST |
| `client/src/designer/collab/ydoc.ts` | the same document in Yjs, `writeDiff` from the Designer's model |
| `client/src/designer/collab/provider.ts` | the provider over the app's WebSocket (no y-websocket server): pacing, acknowledgements, rejoining |
| `client/src/designer/collab/replay.ts`, `updates.ts` | field-level record and replay of a client's edits; splitting an update under the size limit |
| `client/src/designer/collab/undo.ts`, `useCollab.ts`, `ui.tsx` | per-user undo, the React hook, presence and comments UI |
| `native/src/designer/collab.rs`, `native/src/console/views/mimic/editor/live.rs` | the console's session |

## Tests

`collab` crate tests (including the guard against the review's stack bomb,
field-level replay, minimal restacking); server `api::tests::collab`
(convergence, merges, stacking order, quarantine and audit, field-level
put-back telling every author, malicious updates, bad shapes on arrival and
the strike limit, a downgraded or revoked editor, rejoining after a drop,
acknowledgements, shutdown, attribution of a reorder, versions/reads/publish
while live, mention rules, presence ownership, session limits, deleting a
screen, REST/revert/publish during a session, writes outside the lock,
permissions, limits, presence, last leave, restart and reconnect, comments,
the setting, idle close) and `collab::validate`; client
`tests/collabFractional`, `collabDoc`, `collabProvider`, `collabUpdates`,
`collabUndo`, `tests/flow/designerCollab`; native `designer::collab` tests
and `tests/ws_roundtrip.rs`. CI also builds the server without its default
features. `scripts/smoke-collab.sh` (in CI) runs
`client/e2e/collab.smoke.mjs`: two headless browsers as two users on one
screen.

## Settings

`collaboration` in the design-history settings (`PUT
/api/v1/settings/design-history {"collaboration": false}`), on by default.
Off, joining answers `disabled` and the Designer saves whole layouts as in
phases 1 and 3 (Save button, conflict dialog). `GET …/design-status` says
`collaboration: {enabled, protocol, editors}`; a server without the field is
an older one, and the Designer falls back the same way.
