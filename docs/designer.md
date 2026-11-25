# Voltrus Designer

Design for the browser screen editor that replaces the 2D view's Edit mode:
panes, bindings, a flat ISA-101 library, version history, live collaboration,
mobile layouts and a widget SDK. Phase 1 is built (branch
`feat/designer-phase1`); the later phases are designs, grounded in the code as it
is on `main` after #29.

Order: **1** editor, **2** symbol library and equipment types, **2b** widget SDK,
**3** version history, **4** collaboration, **5** mobile and PWA. Version history
comes before collaboration because collaboration needs its change log.

## What exists today

A 2D screen is two sets of rows per screen, read and written whole:

| Route | Rows | Storage |
|---|---|---|
| `GET`/`PUT /api/v1/screens/{id}/scada-equipment` | `{equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config}` | `scada_equipment`, replaced per screen in one transaction (`storage/settings_scada.rs`) |
| `GET`/`PUT /api/v1/screens/{id}/scada-connections` | `{connection_id, from_equipment, to_equipment, connection_type, waypoints}` | `scada_connections` |
| `GET`/`POST /api/v1/faceplates`, `.../{id}` | faceplate definitions with roles | `faceplates` |

`config` and `sensor_bindings` are JSON text. The client turns rows into a
`ScadaLayoutState` (`client/src/scada2/engine/layout.ts`); stacking order travels
in `config.z`. Load-then-save gives identical rows (`client/tests/scadaRoundTrip.test.ts`,
`api::tests::screen_round_trip`, `api::tests::mcp`). Projects carry screens as
`screens[].scada` with tags named by ref (`device/key`); `config.faceplateMap` is
translated id↔ref on export and import (`project_yaml/export.rs`, `import.rs`).

The editor before the Designer: `client/src/scada2/ScadaView.tsx` (run mode and
an Edit mode in one component), a palette, an inspector, react-rnd per widget,
undo/redo over whole layouts. It stays reachable (the 2D view's **Edit** button)
until the Designer is accepted.

## Phase 1: the editor (built)

Open it from the 2D view (**Designer**, admins and engineers) or at
`/designer?screen=<id>`. It is a full-window overlay of the app, so it shares the
session, live data store, theme, toasts, trend popup and signature dialog.

```
┌ title: screen · unsaved dot · Design | Preview · theme · close ───────────────┐
├ toolbar: save undo redo · select connect · align ×6 distribute ×2 same size ×2 │
│          group ungroup lock · front forward backward back · dup del ·        │
│          grid snap rulers · zoom                                             │
├──────────────┬──────────────────────────────────────────┬───────────────────┤
│ Project      │ rulers                                    │ Components        │
│  screens     │ ┌──────────────────────────────────────┐ │  ISA-101 equipment│
│   components │ │ stage 1920×1080, grid, guides,       │ │  symbols, widgets │
│  faceplates  │ │ live values, selection, handles      │ │                   │
│  scripts …   │ └──────────────────────────────────────┘ ├───────────────────┤
├──────────────┤                                          │ Properties        │
│ Tag browser  │                                          │  value tag, layout│
│ (live, drag) │                                          │  every property:  │
│              │                                          │  fixed or bound   │
├──────────────┴──────────────────────────────────────────┴───────────────────┤
│ status: mode · cursor · selection box · guides · tags live · undo depth · saved│
└────────────────────────────────────────────────────────────────────────────┘
```

Code, all under `client/src/designer/`:

| File | What |
|---|---|
| `model.ts` | the document (`ScadaLayoutState`), every edit as one undoable `Edit` (`add`, `remove`, `patch`, `order`, connection edits), history with coalescing (typing, nudging), align, distribute, same size, move, group, ungroup, lock, duplicate. No React. |
| `snap.ts` | snapping to guides, other components' edges and centres, the stage, then the grid; ruler steps |
| `expr.ts` | binding expression parser and evaluator (no `eval`) |
| `bindings.ts` | property bindings: types, evaluation, coercion, refs |
| `props.ts` | the property list of a widget (its schema + label, visible, opacity) |
| `DesignerCanvas.tsx` | stage, rulers, guides, marquee, move and resize with snap lines, drop targets, zoom and pan |
| `panes.tsx` | project browser (with the screen's component tree, visibility and lock toggles), tag browser, component palette |
| `PropertyEditor.tsx` | property grid, binding editor (tag, expression, map), faceplate section |
| `Designer.tsx` | the shell: data loading and saving, keyboard, toolbar, drop menu, preview |

**Saving** goes through the same `saveLayoutToAPI` and the same two `PUT`s as the
old editor, so a Designer screen is an ordinary screen for REST, MCP, the 3D twin
and project files. Designer-only facts live in each row's `config`:

| Key | Meaning |
|---|---|
| `group` | group id; members of one group share it (flat groups, no nesting yet) |
| `locked` | `true`: not moved, resized, marquee-selected or deleted |
| `bindings` | property bindings, below |
| `visible`, `opacity` | common properties; at runtime a hidden widget is not drawn (the Designer shows it faded) |

**Drag-to-bind.** A tag dropped on empty stage opens a menu of what to place
(digital tags: ISA pump, valve, motor, status light, …; analog: number, value,
tank, control valve, bar, gauge, trend; a `%` or "level" tag puts the tank
first); Enter takes the first. A tag dropped on a component binds it (a table or
multi-pen trend gets one more tag). A tag dropped on a binding field inserts its
ref.

**Bindings.** Any property can be fixed or bound:

```json
"bindings": {
  "label":   { "kind": "expr", "expr": "{wtp/clearwell_level} > 90 ? \"HIGH\" : \"Clearwell \" + text({wtp/clearwell_level}, 1) + \" %\"" },
  "visible": { "kind": "tag",  "tag": "wtp/p101b_fault" },
  "fillColor": { "kind": "map", "expr": "{wtp/raw_tank_level}",
                 "cases": [{ "op": ">=", "v": 95, "out": "#f04646" }, { "op": ">=", "v": 85, "out": "#f5b21b" }],
                 "else": "#8b939e" }
}
```

Tags are named by project ref, so bindings survive export and import with no
translation. Expressions: numbers, strings, `true`/`false`/`null`, `{ref}`,
`+ - * / %`, comparisons, `&& || !`, `a ? b : c`, and `abs round floor ceil min
max clamp if bit scale text`. A missing tag or value gives "no value" (the
property keeps its fixed value), never 0. Bindings are evaluated in the browser,
in the Designer (design and preview) and at runtime in the 2D view
(`WidgetShell.useBoundInstance`); the server stores them and does not evaluate
them.

**Undo.** Every edit is one step (a drag, an align, a paste of 20 widgets, a
binding); typing in one field and arrow nudges coalesce. 200 steps, per screen,
cleared on load. Selection is not part of history.

**Keyboard.** Ctrl+S, Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z), Ctrl+C/X/V, Ctrl+D, Ctrl+A,
Ctrl+G / Ctrl+Shift+G, Ctrl+L, Ctrl+] / Ctrl+[ (Shift: to front / back), arrows
(Shift ×10), Del, Esc, V / C (select / connect), F5 (preview), Ctrl+wheel and
Ctrl+0 (zoom, fit), Space+drag or middle button (pan), Alt while dragging (no
snap), Shift on a corner handle (keep ratio), double click (one member of a
group). A drag on a process area draws a marquee (areas are backdrops).

**Preview** runs the unsaved screen in the real runtime (`ScadaView` in run mode):
live values, alarm frames, faceplates, trend popups, operator writes with the
usual permissions and confirmations.

**Guides** are dragged out of the rulers and kept per screen in the browser
(`localStorage`), not in the screen.

Not in phase 1: nested groups, rotation handles (rotation is a property), multi
resize, copy between screens through the system clipboard, connection waypoint
editing (the old editor does not have it either), screen size other than
1920×1080, templates (UDT-like parameterised symbols).

## Phase 2: ISA-101 library and equipment types

Phase 1 ships the first flat set in `client/src/scada2/hmi/isa101.tsx` (category
`ISA-101`): pump, motor, valve, control valve, tank, heat exchanger, process
line (horizontal, vertical). Running/open is filled, stopped is hollow, fault is
red and blinks, no data is magenta and dashed; colour never carries state alone.
They use the `SymbolDef` contract, so ports, faceplates, the 3D twin and saved
rows work unchanged. Colours are theme variables (`--isa-line`, `--isa-on`,
`--isa-off`, `--isa-level`, `--isa-text`), defined for dark and light.

**Equipment types** (pump + faceplate in one drag) exist in a first form: the
palette's *Equipment* tiles place the ISA symbol with a tag, a numbered label
(`P-101`) and `config.faceplate` set to the definition of that kind, saving the
starting definition first if the project has none.

To do in phase 2:

* More symbols: compressor, fan, agitator, conveyor, filter, clarifier, column,
  three-way valve, check valve, damper, instrument bubble, reducer, flanges,
  arrows, off-page connectors. Rotation and mirror for all.
* Lines as real polylines: a `line` widget with `config.points`, snapping to
  symbol ports, drawn under symbols; flow animation from a tag. Replaces the
  stretched process-line symbol.
* **Equipment types as data**: `equipment_types` (id, name, symbol, faceplate id,
  default role→tag pattern such as `{prefix}_run`, `{prefix}_fault`). Dropping
  one asks for the prefix (`p101a`) and maps every role by pattern; unmatched roles
  are listed for the engineer. Project section `equipment_types`, keyed by `id`.
* Analog indicator set from the HP-HMI guidance: moving analog indicator (value
  on its normal range with alarm limits), sparkline, deviation bar.

## Phase 2b: widget SDK

Custom widgets are Web Components with a JSON manifest:

```json
{
  "tag": "acme-ph-dial",
  "name": "pH dial",
  "version": "1.0.0",
  "size": [160, 160],
  "properties": {
    "title":  { "type": "string", "default": "pH" },
    "accent": { "type": "color",  "default": "#5aa9e6" }
  },
  "bindable": ["value", "quality", "unit", "limits", "alarm"],
  "writes": false
}
```

* The manifest drives the property grid (the same `PropRow`/binding editor) and
  says which live attributes the widget accepts. Voltrus' binding layer sets them
  (`el.value = 7.2; el.quality = 'good'; el.limits = {lo, hi, warn, crit}`); a
  widget never fetches, holds no token and cannot see other tags.
* Sandbox: each custom widget renders in a shadow root inside an `<iframe
  sandbox="allow-scripts">` with a CSP that blocks network (`connect-src 'none'`)
  and a `postMessage` bridge for attributes and the one allowed event
  (`write-request`, which goes through the normal command path, write policy and
  signature rules when the manifest declares `writes: true`). Same-origin script
  is not allowed.
* Storage: `widgets` table (id, version, manifest JSON, JS source, sha256,
  created_by), `GET/POST/DELETE /api/v1/widgets`, project section `widgets`. The
  JS is served from `/api/v1/widgets/{id}/{version}.js` with the stored hash as
  ETag; a widget row on a screen names `type: "custom:<id>@<version>"`.
* AI generation in the Assistant: the model proposes a manifest and source, the
  Designer renders it live in the sandbox with recorded values, the engineer
  approves, then it is saved like any other widget (propose, approve, apply, as
  the existing copilot tools do). Generated code goes through the same sandbox;
  approval is required, never automatic.

The existing add-on widget registry (`client/src/edition/widgets.ts`) stays for
first-party edition widgets; the SDK is for project widgets.

## Phase 3: version history

### Data model

An **object** is a screen or a widget on a screen (later: faceplates, equipment
types, custom widgets). New tables:

```sql
-- one row per change of one object; never updated or deleted
CREATE TABLE design_changes (
  seq        INTEGER PRIMARY KEY AUTOINCREMENT,
  screen_id  TEXT NOT NULL,
  object_id  TEXT NOT NULL,        -- equipment_id, connection_id, or '' for the screen
  kind       TEXT NOT NULL,        -- 'add' | 'update' | 'remove'
  before     TEXT,                 -- row JSON before (null on add)
  after      TEXT,                 -- row JSON after (null on remove)
  user_id    INTEGER, username TEXT NOT NULL,
  at_ms      INTEGER NOT NULL,
  batch      TEXT NOT NULL         -- one save or one collaborative transaction
);
CREATE INDEX design_changes_obj ON design_changes(screen_id, object_id, seq);

CREATE TABLE design_versions (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  screen_id  TEXT NOT NULL,
  name       TEXT NOT NULL, message TEXT NOT NULL DEFAULT '',
  upto_seq   INTEGER NOT NULL,     -- the version is the screen as of this change
  snapshot   TEXT NOT NULL,        -- rows JSON, so restore does not replay
  username   TEXT NOT NULL, at_ms INTEGER NOT NULL,
  published  INTEGER NOT NULL DEFAULT 0
);
```

The change log is written on the server: `PUT .../scada-equipment` and
`.../scada-connections` already receive whole sets, so the handler diffs the new
set against the stored one by id inside the same transaction and appends one row
per added, changed or removed object, all with one `batch` id. REST, MCP,
project import and the Designer are logged alike, with no client change. The
audit log keeps its one line per save, pointing at the batch.

### Draft and published

`scada_equipment`/`scada_connections` stay the **draft** (what editors see and
save). Publishing copies the draft into `scada_equipment_published` /
`scada_connections_published` (same columns) and records a `design_versions` row
with `published = 1`. Operators read the published rows; a screen never published
falls back to the draft, so existing installations behave as today. A setting
per site can turn the two-stage flow off.

### API

| Route | What |
|---|---|
| `GET /api/v1/screens/{id}/history?object=&before_seq=&limit=` | change log, newest first |
| `GET /api/v1/screens/{id}/blame` | per object: last change (who, when, batch) |
| `GET/POST /api/v1/screens/{id}/versions` | list; create `{name, message}` from the current draft |
| `GET /api/v1/screens/{id}/versions/{v}` | snapshot |
| `GET /api/v1/screens/{id}/diff?from=v1&to=v2\|draft` | per object: added, removed, changed fields |
| `POST /api/v1/screens/{id}/revert` | `{to: v}` whole screen, or `{object, to_seq}` one object; written as new changes, never by deleting history |
| `POST /api/v1/screens/{id}/publish` | `{name, message}`; needs the configure permission and, when the site requires it, an electronic signature (existing `signature/` flow) |
| `GET /api/v1/screens/{id}/scada-equipment?published=1` | what operators see |

MCP gets `screen_history`, `screen_diff`, `screen_publish` (admin, propose then
apply, like `project_apply_import`).

### Designer UI

A *History* pane (per screen and per selected object): timeline grouped by batch,
avatar and time, click to preview that state on the canvas (read-only overlay),
*Revert this object*, *Revert screen to here*. *Versions*: name + message,
*Compare* opens a split view: the two canvases side by side with added (green),
removed (red) and changed (amber) outlines, and a YAML diff of the rows in project
format (refs, not ids) underneath. The title bar shows *Draft · 3 changes since
v12* and a *Publish* button.

### Git sync (optional)

A site setting points at a local bare repo or a remote with a deploy key. On
publish, the server runs the project exporter for that screen (and anything it
references) into `screens/<id>.yaml` and commits with the version message, author
from the user. Pull is manual (*Import from git*) through the existing project
import plan, so a merge conflict shows as an import plan, never as a half-written
screen. No git on the operator path.

## Phase 4: live collaboration

LAN only: the Voltrus server is the relay and the authority; nothing leaves the
site.

* **Client:** one `Y.Doc` per open screen. `Y.Map widgets` (id → `Y.Map` of the
  row's fields, with `config` as a nested `Y.Map` so two people editing different
  properties of one widget merge), `Y.Map connections`, `Y.Array order` for z-order
  (fractional index strings in `config.z` on save, so concurrent reorders do not
  conflict). The Designer's `Edit` operations become Yjs transactions; undo uses
  `Y.UndoManager` scoped to the local user's origin, which keeps phase 1's
  one-edit-one-step behaviour.
* **Server:** `yrs` holds the authoritative doc per screen while anyone has it
  open. Sync messages ride the existing `/api/v1/ws` socket as new MessagePack
  types (`DesignJoin {screen}`, `DesignSync {screen, update}`, `DesignAwareness
  {screen, state}`, `DesignLeave`), so auth, the 30-second re-check and
  reconnect come for free. Joining needs the configure permission; awareness
  (presence, cursor, selection, colour) is broadcast but never stored.
* **Persistence:** the server applies updates to the doc and, debounced (2 s) and
  on last leave, writes the rows through the same save path as REST. Each flush is
  one `design_changes` batch with the authors of the updates in it, which is why
  phase 3 comes first: blame and revert work the same for collaborative edits.
  REST/MCP writes while a doc is open are applied to the doc as a server-origin
  transaction, so nobody overwrites anybody.
* **Comments** pinned to objects: `design_comments` (id, screen, object_id, x, y,
  body, author, resolved, at); shown as pins on the canvas and in a pane; events
  over the same socket.
* **Presence UI:** avatars in the title bar, coloured cursors with names on the
  canvas, other people's selections outlined in their colour, *Follow* a user.

## Phase 5: mobile layouts and PWA

* An operator layout is generated per screen, not drawn: components are ordered
  by area (process areas become sections), then top-to-bottom, left-to-right
  inside an area; each equipment symbol becomes a row (symbol thumbnail, label,
  state, primary value, faceplate on tap); trends and tables become full-width
  cards; decoration (lines, labels without bindings) is dropped. The generated
  layout is stored as an override list (`config.mobile = {hide, order, span}`) only
  when an engineer adjusts it in a *Mobile* preview pane, so regeneration keeps
  their changes.
* The client already registers a service worker (`sw-register.ts`); phase 5 adds a
  manifest per site, offline shell, last-known values with their age, and push for
  alarms (existing push subscriptions).

## Migration from the scada2 editor

1. Phase 1 (this branch): Designer beside the old editor. Both read and write the
   same rows. Screens edited in the Designer open in the old editor; groups, locks
   and bindings are kept by it (they are config keys it passes through) but it
   cannot edit them.
2. When the Designer is accepted: the 2D view's **Edit** opens the Designer;
   `ScadaView` keeps run mode only (about 300 lines go: palette, inspector,
   edit toolbar, react-rnd shell). `ScadaPalette.tsx`, `ScadaInspector.tsx` (its
   field components move to `designer/`) and the edit half of `WidgetShell` are
   removed.
3. Server: project export/import translates refs inside `config.bindings`
   expressions only if we ever change the ref syntax; today refs are already
   portable. Strict validation of the layout `PUT`s (lessons-implemented §6 left
   them open) should learn `group`, `locked`, `visible`, `opacity` and the binding
   shapes, and reject a binding naming an unknown tag on import (warning on save).

## Risks

| Risk | Mitigation |
|---|---|
| Bindings evaluated only in the browser: a bound `visible` hides equipment from an operator if the expression is wrong | the binding editor shows the live result and errors before Apply; unknown tags are named; a failed evaluation keeps the fixed value; phase 3 blame shows who bound it |
| Expression cost on big screens (hundreds of bindings × 1 Hz) | parse once per text (cache), evaluation is a tree walk; widgets re-render only when their own tags change (tracked store). Measure at 1,000 bindings before phase 4 |
| Whole-set `PUT` loses a concurrent editor's work (today and in phase 1–3) | phase 3 logs it (recoverable); phase 4 removes it. Until then the Designer reloads only when it has no unsaved edits |
| Tag refs `device/key#2` for duplicate keys | `refIndex` maps the first tag of a ref; duplicate keys are rare and the project exporter already flags them |
| Yjs/yrs version skew | pin both; protocol version in `DesignJoin`; server refuses mismatches with a reload prompt |
| Custom widget code (SDK, AI-generated) | iframe sandbox, no network, attribute-only bridge, approval step, hash-pinned versions |
| Overlay route bypasses the app layout | it is inside the app's providers and error boundary; closing returns to the 2D view of the same screen; unsaved changes prompt before close and before leaving the page |
| Old and new editor drift | both use `layout.ts`; round-trip tests cover Designer config keys (`client/tests/designerModel.test.ts`) |

## Tests

* `client/tests/designerModel.test.ts`: undo/redo, coalescing, align, distribute,
  same size, move clamping, group/ungroup (and through save/load rows), lock,
  z-order (and through `config.z`), duplicate, primary and property binding in the
  saved row, snapping.
* `client/tests/designerBindings.test.ts`: expression grammar, missing values,
  errors, tag/expr/map bindings, `applyBindings`, refs.
* `client/e2e/designer.smoke.mjs`: browser run against the WTP showcase: open,
  drag a tag in, bind an expression, undo/redo, marquee, align, group, save,
  reload and check the rows over REST, preview, light theme, ISA-101 equipment
  with drag-to-bind. Needs a server, `plant-sim.py` and `playwright-core`; not
  part of `npm test`.
