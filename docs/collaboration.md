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
  rows and writes them with the same code as `PUT …/scada-layout`: equipment
  instances are bound, the change log gets the changes, the revision and
  `seq` move. A write is one batch per author (who touched which object since
  the last write), squashed with that author's earlier batches by the change
  log's 60-second rule, so a session of small moves is one entry per object
  and person in History, blame names the right person and revert works as for
  any save.
* **Other writers.** REST and MCP layout `PUT`s, revert, publish and the
  project import take the same per-screen lock as the session's writes. While
  a session is open, a REST write first waits for the session's pending edits
  to be written (so its `base_seq`/`base_revision` is checked against what the
  editors see), then writes, then its difference is applied to the document
  as a server change: everyone sees it at once and nobody's work is
  overwritten. Other writers (equipment propagation, tag renames, a snapshot
  rollback, the FEED wizard) write the rows directly; the session notices the
  change log moved within half a second and merges their difference the same
  way.
* **Publishing** publishes the written draft: the session's pending edits are
  written first, under the lock.

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
| sync messages | 50 a second per connection | refused: `rate_limited` (the web client batches its updates every 40 ms) |
| document | 5 MB | refused: `doc_too_large` |
| awareness state | 4 KB | dropped |

At every write the server checks each object that changed since the last
write: known shape (widget type, numbers finite and within ±100,000, size
positive), `config` an object under 64 KB, binding shapes (`tag`, `expr`,
`map`, as project import checks them), tag refs and tag ids that exist (only
new ones: a binding to a tag deleted earlier stays as it was), equipment
instances and faceplates that exist, connection ends that exist. An object
that fails is not written: the server puts it back as it was last written (or
removes it if it is new) as a server change, so every client converges on the
valid state; the author's client is told which objects and why
(`DesignRejected`, a toast) and the rejection goes to the audit log
(`design_collab_rejected`). A connection left dangling by someone else's
delete is removed the same way.

### Permissions

Joining needs a role that may configure (admin, demo, configure API keys);
viewers and operators are refused (`DesignError forbidden`) and never receive
the document or anyone's presence. A session whose user loses the role or
whose sign-in ends is closed by the WebSocket's 30-second re-check, which
removes it from the session.

### Presence

Presence uses y-protocols awareness. The server forwards each client's state
to the others and keeps the latest per connection for newcomers; it is never
stored. The server overwrites the name and colour in it with the signed-in
user's (nobody can appear as someone else) and drops states over 4 KB. When a
connection leaves or drops, the others get its removal at once.

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
| `DesignError {screen, code, message, reload}` | `forbidden`, `disabled` (collaboration is off: save whole layouts), `protocol` (version mismatch: reload the page), `too_large`, `rate_limited`, `doc_too_large`, `bad_message`, `resync` (the server could not deliver a message: join again), `not_joined` |
| `DesignRejected {screen, objects: [{object_type, object_id, reason}]}` | your edits that failed validation were put back |
| `DesignFlushed {screen, seq, at_ms, users}` | written to the draft |
| `DesignSessions {sessions: [{screen, users: [{user, color}]}]}` | who edits which screen |
| `DesignComment {screen, event, comment}` | a comment was `created`, `updated` or `deleted` |

**Epochs and reconnects.** Each session has an epoch (a random id). A client
that reconnects within the session's life sends its epoch and runs the normal
y-sync exchange: edits made while offline merge. When the epoch differs (the
server restarted, or the session closed), `DesignJoined` says `resync`: the
client keeps the objects it changed while offline, takes the server's
document, and writes those objects into it again as one edit.

## Comments

Comments are pinned to a component or connection (or a point of the stage),
in threads: a comment and its replies. `design_comments` (id, screen,
parent, object, x, y, body, author, mentions, resolved, resolved by/at,
created and updated). `@name` mentions a user, who gets a notification. A
thread is resolved or reopened by anyone who may configure; a comment is edited
by its author and deleted by its author or an administrator. They show as pins
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
  while edits wait to be written), Ctrl+S writes at once. No conflict dialog,
  no leave prompt; closing the tab while offline with edits not yet sent asks
  first.
* **Undo** (Ctrl+Z / Ctrl+Y) takes back only your own edits, one edit at a
  time as in phase 1 (typing in one field and nudges coalesce).
* **Offline:** when the socket drops, editing goes on; the edits are sent when
  it comes back (merged with everyone else's). If the server restarted in
  between, the objects you changed offline are written into its document
  again.
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
| `src/collab/mod.rs` | sessions, the per-screen lock, sync, presence, writes per author, limits, idle close |
| `src/collab/validate.rs` | what a write checks |
| `src/api/collab.rs`, `src/storage/design_comments.rs` | sessions list and comments over REST |
| `client/src/designer/collab/ydoc.ts` | the same document in Yjs, `writeDiff` from the Designer's model |
| `client/src/designer/collab/provider.ts` | the provider over the app's WebSocket (no y-websocket server) |
| `client/src/designer/collab/undo.ts`, `useCollab.ts`, `ui.tsx` | per-user undo, the React hook, presence and comments UI |
| `native/src/designer/collab.rs`, `native/src/console/views/mimic/editor/live.rs` | the console's session |

## Tests

`collab` crate tests; server `api::tests::collab` (convergence, merges,
stacking order, quarantine and audit, REST/revert/publish during a session,
writes outside the lock, permissions, limits, presence, last leave, restart and
reconnect, comments, the setting, idle close) and `collab::validate`; client
`tests/collabFractional`, `collabDoc`, `collabProvider`, `collabUndo`,
`tests/flow/designerCollab`; native `designer::collab` tests and
`tests/ws_roundtrip.rs`. `scripts/smoke-collab.sh` (in CI) runs
`client/e2e/collab.smoke.mjs`: two headless browsers as two users on one
screen.

## Settings

`collaboration` in the design-history settings (`PUT
/api/v1/settings/design-history {"collaboration": false}`), on by default.
Off, joining answers `disabled` and the Designer saves whole layouts as in
phases 1 and 3 (Save button, conflict dialog). `GET …/design-status` says
`collaboration: {enabled, protocol, editors}`; a server without the field is
an older one, and the Designer falls back the same way.
