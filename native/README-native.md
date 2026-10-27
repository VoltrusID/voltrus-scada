# Voltrus SCADA: native operator console (`voltrus-native`)

A desktop console for the Voltrus SCADA server, drawn on the GPU with
[GPUI](https://gpui.rs) (Metal on macOS). It speaks the same REST API and
MessagePack WebSocket as the web client, so it needs nothing on the server
beyond a user account.

The look and controls come from `vt-ui`, the Voltrus design system shared
with Voltrus Tools (in the `voltrus-explorers` repository): colour tokens
from the bundled Voltrus dark and light themes, one spacing and type scale,
JetBrains Mono for values.

## What it does

- **Sign in** under the Voltrus logo, with the server address, username and
  password. The address and username are remembered in
  `<config dir>/voltrus-native/config.toml`
  (`~/Library/Application Support/voltrus-native/` on macOS); the password
  never is. When the server flags a generated or reset password, the console
  asks for a new one before anything else.
- **Top navbar**, in the window's title bar row (right of the macOS traffic
  lights, and the window's drag area): the Voltrus logo, "Voltrus SCADA" and
  the server's address; the Overview, Tags, Alarms and Trends tabs (⌘1–⌘4,
  shown in each tab's tooltip; Alarms carries the count still to
  acknowledge); then the light/dark switch, who is signed in with their role,
  and Sign out. Pages span the full width below it.
- **Overview**: tag count, tags that need attention, alarms to acknowledge and
  live updates per second; the worst tags and the open alarms (with
  Acknowledge); a tile per tag with its value, status and a 15-minute
  sparkline.
- **Tags**: every tag in a virtualised table (name, key, value and unit
  coloured by the server's status, status, age, 15-minute sparkline), filtered
  by name or key, ↑/↓ to move. The detail panel (widened across the page
  from its header or the palette) shows the value and its
  facts, and a chart with its own span: 5 min to 7 days running live at the
  display's rate, or a custom stretch in the past; Auto or Full detail and
  sample, average or min/max aggregation; overlays of up to five other tags
  (a very different size gets its own scale); warning and critical lines; a
  crosshair that reads every line's value; points, lowest, highest and
  average over the span. ⇧⌘S saves what the chart shows as CSV to a file you
  pick. Operators (admin, demo, integration) see the tag's notes on the chart
  and in a list (Enter shows a note's moment), add one with ⇧⌘N or a
  right-click on the chart, and delete them. Writing (⇧⌘W, Enter on a row):
  operators write through the server's command queue, immediate or queued,
  with a reason, checked against the write policy and answered with the
  command's id (copyable); an admin can also write directly over the
  WebSocket. Operators edit the write policy (writable, minimum, maximum,
  writes per minute, reason required, deadband). Add to Trends.
- **Alarms**: a summary strip (critical, warning and info alarms waiting,
  shelved, raised in the last 24 h, mean time to acknowledge: the server's
  summary for the admin, demo and integration roles, counted from the open
  alarms for others); the list filtered to open, to acknowledge,
  acknowledged, shelved or the whole history (resolved alarms too), searched
  with `/` (source, key, message, priority, `#id`) and sorted by priority,
  newest or source; a priority pill and HI/LO per row. Keys on the list:
  Enter opens the detail sheet (every fact, the actions and the ISA-18.2
  lifecycle journal, j/k to step through the list), `a` acknowledges, `s`
  or Space shelves (a dialog: 15 min to 8 h and a reason) or unshelves,
  ⇧A or ⇧⌘A acknowledges all after a confirmation, ←/→ switch the filter
  and `o` the order. The same roles get the Analytics view (the ten most
  frequent sources of the last 24 h, an alarm-flood warning) and the Audit
  trail (every acknowledge, shelve and unshelve, with the shelve's reason).
  Alarm times show on the computer's clock.
- **Alarm sound**: while an open alarm waits for acknowledgement a tone
  loops, set by the worst one's priority (a fast two-tone warble for
  critical, a double beep every 2.5 s for warning, a chime every 5 s for
  info). The tones are synthesized, so no sound file ships. The navbar's
  speaker, the palette and ⇧⌘M mute it; the choice is remembered per user
  and server in `<config dir>/voltrus-native/alarm-sound.toml`.
- **Trends**: the chosen tags on one time axis over 5 min, 30 min, 1 h, 6 h or 24 h,
  filled from the server's history and extended by live values. While the
  link is live the chart moves on once a second. Each tag keeps a day of
  history: live values of the last 15 minutes at full rate, older ones
  thinned to one per 20 seconds.
- **Data Manager** (More menu, palette; offered to a role that sees at least
  one section): a sidebar of sections grouped Data, Protocols, Insights,
  Admin and Engineering as in the web client (only built sections are
  listed; ↑↓ switch, Enter goes in, Esc comes back; `--section <id>` opens
  one). **Sources**: every data source in a virtualised table
  (order, name, key, unit, range, limits, source type, card, per-tag
  shortcut), `/` to search, sorting by clicking a header or S (⇧S reverses);
  N or New source and Enter or a double click open the form (General: name,
  key, unit, decimals, range, limits, base, card kind, colour, low limits and
  deadband, alarm delays, recorded per-tag shortcut; Source: the type and
  its settings, a Modbus device with one register or an expression); D
  duplicates, ⌫ deletes after a confirmation, ⌥↑/⌥↓ or dragging a row
  reorders (shown at once, saved to the server).
  **OEE** (every user; writes need an operator): a device's OEE,
  availability, performance and quality over 8 h, 24 h or 7 d as figure
  cards with bars (←→ move, Enter shows what the figure is made of:
  A × P × Q, the running and planned time, downtime by machine state);
  the downtime events (N opens one with a machine state, Enter closes an
  open one with a reason) and the shift schedule (T switches; N adds a
  shift with its start, length and days, ⌫ deletes); `,`/`.` step the
  devices, `[`/`]` the ranges, R fetches again (also every 15 s on
  screen). **Analytics** (every user): a device metric (the device's tags,
  ↑↓) as buckets over 1 h to 30 d, each bucket's low-to-high span a bar
  with its average marked, the range's figures, a crosshair and readout
  following the pointer; ←→ move between bars, Enter (or a double click)
  drills into a bar with finer buckets and ⌫ or Esc comes back; M switches
  to the comparison of the range's recent half with the half before; E
  saves the buckets as CSV. **Recipes** (operators): saved device values by
  device and category (`/` searches); N saves a device's current values,
  Enter opens a recipe (name, category, description; its values beside the
  live ones), L loads it after a confirmation and follows the commands the
  server queued (every 2 s until each is done; Retry writes a failed one
  again; Open in Commands shows them on the Commands page), ⌫ deletes.
  **Protocols** (Modbus, OPC UA, Siemens S7, EtherNet/IP, DNP3, BACnet), each
  a device table like Sources (`/` search, header or S sort, the poller's
  or the WebSocket's online state per device, columns dropping as the
  window narrows) whose rows expand (Space) into their registers, nodes,
  tags, points or objects. N adds a device, Enter edits, A adds a child, E
  enables or disables, ⌫ removes a device or a child after a confirmation,
  `.` lists the row's actions (start and stop polling, restart, browse,
  integrity poll, freeze, simulation), T runs the section's scan or
  discovery, whose results are picked to add devices, registers, nodes,
  tags or objects. Modbus also has duplicate (D, registers included),
  simulation profiles and the simulation switch; Modbus and BACnet import
  Modbus Poll and BACnet Explorer device files from Import… (I) or dropped
  on the section, with a report of what was added. Modbus reads are open to
  every user and its changes to operators; the server's other protocol
  handlers answer only admins, so only admins see those sections.
  **Admin** (as the web client's group): **Users** (admin only): every
  account with its role; N adds one (name, the password typed twice in
  masked fields, role), Enter changes the role and optionally the password,
  P resets a password, ⌫ deletes after a confirmation; the console refuses
  demoting or deleting yourself or the last admin. **Roles** (admin only):
  the server's RBAC roles with their permissions picked from the 24 the
  server accepts. **Notifications** (operators): who hears of a tag's (or
  every tag's) alarms by email, webhook or push; N, Enter, Space turns a
  rule on or off, T sends a test through the server and shows each
  target's outcome, ⌫ deletes. **API keys** (admin only): name, prefix,
  role, maker, last use and expiry; N makes a read-only or integration key
  with a lifetime and shows its secret once (masked until revealed, C or
  Copy), ⌫ revokes. The palette offers New user, New role, New API key and
  New notification rule.
- **Dashboard** (⌘6): the server's screens as card dashboards, as the web
  client's card view lays them out (its 24-column grid and saved layouts).
  Screen tabs sit in a bar under the navbar (1–9 jump to them, [ and ] step;
  1 is the overview, which has every tag). Each card draws its tag's chart
  type: a sparkline with the warning and critical limits, a gauge, a big
  number with its level bar, a picture or a Markdown note; a click or Enter
  opens the tag, the arrows move between cards. Operators edit (E): drag a
  card to move it and its corner to resize it, the grid reflowing under it
  at the display's rate; arrows move the focused card and Shift-arrows
  resize it; A arranges the cards by device; T adds tags, Backspace removes
  one; screens are created (N), renamed and deleted. E, Esc or Done saves
  the layout. Only the cards in view are drawn.
- **Mimic** (⌘7): a screen's HMI mimic, as the web client's SCADA view
  draws it (`scada-equipment` and `scada-connections`): all 31 widget
  types and 31 library symbols (pumps, valves, vessels, the electrical
  kit, instruments, pipes) with their live values and states, process
  areas, and the connections between them, pipes flowing while their
  equipment runs. Drag or scroll to pan; pinch, ⌘-scroll, ⌘+/⌘− zoom
  about the pointer and ⌘0 fits, all at the display's rate. Tab and the
  arrows walk the equipment, Enter or a click opens its faceplate (state,
  value against its limits, a 30-minute trend, related run and fault
  tags), Esc closes it. Operators command from it: 1 arms START/OPEN/ON
  (or the set point, stepped with ⇧↑/⇧↓), 0 arms STOP/CLOSE/OFF, Enter
  confirms within 6 s; an admin's command goes over the WebSocket, other
  operators' through the policy-checked REST write. The alarm banner
  under the stage lists the newest active alarms, acknowledged in place.
  Screen tabs and [ / ] as on the Dashboard.
- **Mimic editor** (E on the mimic, or Edit; operators), as the web
  client's SCADA editor: a palette of the 31 library symbols by category
  (drawn by the stage's painter) and the widget types, searchable (/),
  click to add at the centre or drag onto the stage; click, ⇧-click, a
  rubber band (⌥ anywhere) or ⌘A select; drags move the selection and the
  eight handles resize, snapping to the 10-pixel grid (dots every 20) and
  lining up with other widgets' edges and centres (guide lines; ⌘ held:
  free), on the 1920×1080 stage; the knob (R, ⇧R) turns a symbol a
  quarter, H mirrors it. The connect tool (C; V back) joins a source and a
  target as a process pipe, power line or signal. The inspector edits
  position and size, the type's settings, tag bindings (searched in the
  store), z-order ([ ]), duplicate and delete; a connection's type; or
  shows the screen's summary. ⌘Z / ⇧⌘Z (100 steps, typing in one field
  is one), ⌘C / ⌘V / ⌘D, arrows nudge 1 (⇧ 10), ⌫ deletes. ⌘S saves the
  rows (`PUT scada-equipment` and `scada-connections`); a failure keeps
  the edits. Done, another screen, another page or signing out with
  unsaved edits asks Save / Discard / Cancel (the web client saves on a
  screen switch without asking).
- **Tag tree** (⌘5): the tags filed by their path (or their device when
  they have none), folded and unfolded with Space or Enter, searched on the
  server as you type (`/`), each tag with its live value; Enter on a tag
  opens it on the Tags page. Operators move a branch or one tag to another
  path (each tag keeps its place below the branch; empty clears it) and set
  the naming convention (ISA-5.1, IEC 61346, KKS, a custom pattern, or any
  name).
- **Status bar**: link state (live, reconnecting with the reason, offline),
  server, updates per second, tag and alarm counts, and a warning when the
  alarm list could not be fetched.
- **Toasts** for new alarms, link losses and failed actions: bottom right,
  above the status bar and left of any side panel, up to three stacked
  upward, each gone after 6 seconds.
- Light or dark (⇧⌘L), standard Edit menu in text fields.
- **Keyboard first**: ⌘K opens the command palette (every page and
  command, fuzzy search); `?` lists every shortcut; ⌘1–⌘9 go to pages,
  ⌘[ and ⌘] go back and forward; in every list j/k or ↑/↓ move a visible
  cursor, PgUp/PgDn/Home/End jump, Enter opens and Space toggles; `/`
  focuses the page's filter; Tab walks the controls (trapped inside a
  dialog) and Esc closes or goes back. Pages beyond the tabs sit in the
  navbar's More menu; narrow windows show the tabs as icons.
- **Smooth**: each window part is its own cached view, pages stay alive so
  switching is instant, and animations (tile flashes and level tweens, the
  Trends chart scrolling at the display rate while live, page entrances,
  toast slides) ask for frames only while they move.
- **Performance HUD** (⇧⌘P) and the **Performance** page: frame rate, frame
  time and build time percentiles, dropped frames against the display's
  refresh, events per second, tags and peak memory. `--stress` feeds a
  deterministic 10 000-tag synthetic plant through the same store path as
  the WebSocket, without a server, and prints a report when it ends.
- **Settings** (⌘,, or the gear in the navbar): the appearance (light,
  dark or system), kept on the server per user (`settings/ui-theme`) and
  applied at sign-in unless `--appearance` pins it; any of vt-ui's bundled
  themes for dark and for light, picked from a searchable list that shows
  each theme as the cursor moves (Enter keeps it, Esc goes back); the
  language (`settings/ui-lang`, kept for the web client; this console's
  text stays English); the alarm cooldown (operators save it, others read
  it); the history aggregation default; system notifications for new
  critical alarms (macOS; during a flood at most one every 2 s, counting
  the alarms raised meanwhile); the kiosk lock (in the console: no Quit,
  Sign out, window close or stress test; the sign-in card can be quit); and
  the edition, versions and add-on features. ⇧⌘L and the
  navbar's sun and moon also keep the choice on the server. Local choices
  sit in `<config dir>/voltrus-native/preferences.toml`.
- **Keyboard shortcuts from the server**: the user's `nav-shortcuts`
  (Overview, Alarms) and per-tag `shortcuts`, shared with the web client,
  become key bindings at sign-in. The shortcut editor (in Settings or the
  palette) lists every target and tag: Enter records new keys, Space
  clears; shortcuts that only mean something in the web client, that take
  a built-in key, or that are a key without ⌘, ⌥ or ⇧ (other than a
  function key; Enter, Space and the arrows belong to lists and dialogs)
  are kept but say why they are not bound here.
- **Notifications**: the navbar's bell counts unread user notifications
  (mentions in shift handovers and annotations, fetched every 30 s); its
  popover (⌥⌘N) shows the alarms to acknowledge and the notifications,
  Enter marks one read, Space dismisses it.
- **Edition**: the licence tier as a badge in the navbar and on the
  sign-in card (`/api/license/status`), and the server's sign-in methods
  (`/api/v1/auth/providers`) under the sign-in form. Single sign-on is
  listed but not usable here: the server has no hand-off that gives a
  desktop client the session, so the card says so and offers the web
  sign-in.
- **Status bar clock** on the computer's time, beside the server's version
  and the console's.
- **Operations pages** in the More menu (and the palette): **Commands**
  (the write command queue: a History of every command, filtered by status
  with `[` `]` and paged with ← →, an Active list fetched every 2 s while it
  is on screen, a side panel with every field and the audit trail, and
  Cancel for operators with `x`); **Shift handover** (operators: the log by
  date and shift, new entries written in a dialog with multi-line notes, `n`,
  where @mentions are highlighted and named; the server's automatic report
  of the current shift, fetched every 30 s while shown); **Audit log**
  (operators: search with `/`, action, user and time filters, sort with `s`
  or the column heads, pages of 50, `e` saves the filtered log as CSV).
  **Export data…** (palette or View menu) saves telemetry from
  `/api/v1/data/export`, raw or aggregated, as JSON or CSV, to a file you
  pick.

The network side runs on its own tokio thread: it signs in, keeps the
WebSocket open, reconnects with backoff (2 s, 5 s, then every 10 s), and
renews the 15-minute session token with the session's refresh token
(`POST /auth/refresh`) every 10 minutes and before each reconnect (30
seconds after a renewal the server did not answer), so REST calls keep
working. It does not keep the password (only while the server wants it
changed), and a renewal or a sign-in elsewhere leaves the user's other
sessions alone (a sign-in replaces only the refresh token it presents). Only the operator's own
sign-in waits for the server; every other REST call (tags, alarms,
history, alarm actions, password changes, renewals) runs on its own and
never holds up the tag stream. Answers that arrive after the session ended
are dropped. When the server refuses a renewal (the refresh token revoked
or expired, for instance after a password or role change, or a deletion) the session
ends with "Not authorised". Signing out, quitting and starting a stress
test all sign the session out of the server, which revokes its refresh
token. When a session ends (sign-out, a refused renewal, or before the next
sign-in) every page's state goes with it; only the operator's own
preferences stay.

## Build and run

```bash
cargo build --release -p voltrus-scada-native
./target/release/voltrus-native
```

Options, mainly for kiosks and screenshots:

```
voltrus-native [--sign-in] [--page overview|tags|alarms|trends|performance|tagtree|commands|shift|audit] [--tag <key>]
               [--range 5m|30m|1h|6h|24h] [--appearance dark|light] [--hud]
               [--overlay palette|help|<action>|tag-write|tag-policy|tag-note|tag-compare|tag-range|tag-expand]
               [--alarms history|analytics|audit|detail|shelve|confirm]
               [--window WxH] [--keys <keystrokes>]
               [--stress [--stress-tags N] [--stress-rate N] [--stress-switch-ms N]
                         [--stress-seconds N] [--stress-report FILE]]
               [--stress-profile]
```

`--sign-in` signs in at once with `VOLTRUS_NATIVE_URL` (default
`http://localhost:3000`), `VOLTRUS_NATIVE_USER` (default `admin`) and
`VOLTRUS_NATIVE_PASSWORD`. `--tag` opens one tag's detail; `--range` picks the Trends range.
`--overlay tag-…` opens one of that tag's dialogs (`tag-expand` widens its detail).
`--overlay` also runs a palette command by its action name once signed in,
e.g. `--overlay OpsExportData` (the data export dialog),
`--overlay OpsNewHandover` (a new shift handover entry) or
`--overlay ShellOpenSettings` (the settings sheet).
`--alarms` opens the Alarms page on a view (history, analytics, audit) or
with a dialog open on its worst alarm (detail, shelve) or the
acknowledge-all confirmation.
`--window 1280x800` opens the window at that size in points (the minimum,
1024×640, gives way to a smaller size, to show the navbar's narrow steps),
and `--keys down,down,down` types those keys once the console and its
`--overlay` are up (a menu scrolled by the keyboard). In a narrow window the
navbar shows its tabs as icons, a search button and one menu for
notifications, settings, the alarm sound, the HUD and the appearance
(`--overlay ShowNavMenu` opens it).
`--stress-seconds 20 --stress-report report.txt` runs the stress test for
20 seconds, prints the report (frames, frame time p50/p95/max, drops,
page-switch time, memory, the share of frames built within a 60 Hz and a
120 Hz frame, event-batch apply time, each page's frame and render times
and its first frame, the slowest frames, and the post-sign-in warm-up kept
apart), writes it to the file and quits. `--stress-profile` is the
animation showcase: the Overview stays put, its tiles flash in ripples and
a trend card redraws every frame, at 120 fps on a ProMotion display (the
meter takes the refresh rate from the display).

How the console is built, and how to add a page, a REST call, a key, an
overlay or an animation: [`docs/architecture.md`](docs/architecture.md).

Requires a recent stable Rust (edition 2024). CI builds and tests on macOS;
GPUI also supports Linux (X11 or Wayland), which is not tested here.

## Layout

```
native/src/
  lib.rs            api, runtime, ws_types, stress, perf and tag_api (no UI; used by the tests)
  tag_api.rs        tag writes and policies, tag tree, naming convention, annotations,
                    the detail chart's history query, CSV
  api.rs            REST client: login and logout cookies, generic JSON helpers,
                    data sources, alarms, history, password change
  api/ops.rs        commands, shift handover, audit log and data export: mirrors,
                    paths, text downloads
  api/admin.rs      users, RBAC roles, API keys and notification rules: mirrors,
                    request bodies checked as the server checks them
  ws_types.rs       MessagePack WebSocket messages (mirror of the server's)
  runtime.rs        the network actor: Command in, Event out, generic calls
  stress.rs         the deterministic synthetic plant
  perf.rs           frame statistics and the stress report
  screens.rs        screens, card layouts and the card grid's rules (compaction,
                    drag, resize, auto-arrange; shared with the mimic viewer)
  main.rs           command line, window, theme, menus
  console/
    mod.rs          the Console model: phases, navigation, overlays, stress run
    alarms.rs       alarm filters, sort, search, summary counts, shelve durations
    audio.rs        the alarm annunciator: synthesized tones, mute per user
    pages.rs        the page registry
    keys.rs         actions, key bindings, menus, the command table
    nav.rs          keyboard lists
    overlay.rs      the overlay slot and the palette's search
    anim.rs, meter.rs  animation helpers, the frame meter
    store.rs        tags, values, statuses and history; update rate
    screens.rs      the session's screens: list, active screen, tags, layouts
    config.rs       remembered address and username
    text_area.rs    the multi-line text field (handover notes)
    views/          one cached view per window part, one module per page
    shell.rs        settings, notifications, edition, kiosk lock (state and server calls)
    shortcuts.rs    server-kept shortcuts as live key bindings
    prefs.rs        workstation preferences (themes per brightness, kiosk, …)
native/scripts/     CI: the solar-farm plant, app bundle, screenshots
```

## Tests

```bash
cargo test -p voltrus-scada-native     # unit tests and wire round trips

# End to end against a running server:
VOLTRUS_E2E_URL=http://localhost:3000 \
VOLTRUS_E2E_USERNAME=admin \
VOLTRUS_E2E_PASSWORD=... \
cargo test -p voltrus-scada-native --test e2e_live -- --nocapture --test-threads=1
```

Set `VOLTRUS_E2E_EXPECT_UPDATES=1` to fail when no live values arrive. Run
the end-to-end tests one at a time: they share the server and its admin
account, and the first changes the admin password and back. The Tags
test moves a tag in the tree and back, saves the naming convention it
found, sets a write policy and writes through the command queue (refused
without the reason, then accepted), adds and deletes a note, and reads a
custom span of full-detail, averaged history.

The `Native console` workflow (`.github/workflows/native.yml`) runs on macOS:
formatting, clippy, the tests, then a release build of the console, the
server and the Modbus simulator. `native/scripts/ci-server.sh` starts the
solar-farm scenario, resets the admin password with
`voltrus --reset-admin-password`, replaces it through the API, and sets
plant-like alarm limits. The end-to-end test runs against that server
(signing in, tags, history, live values, acknowledging an open alarm, the
alarm summary, history and analytics, shelving an alarm with a reason and
finding it on the shelf, in its journal and in the audit trail, unshelving
it, and changing the admin password and back), and
`native/scripts/screenshots-macos.sh` captures the signed-in console:
`overview`, `tags` (with a tag's detail open), `alarms`, `trends` (5 min) and
`overview-light`, the Alarms page's views and dialogs (`alarms-history`,
`-analytics`, `-audit`, `-detail`, `-shelve`, `-confirm`, `alarms-light`),
the operations pages (`ops-commands`, `ops-shift`, `ops-shift-new`,
`ops-audit`, `ops-audit-light`, `ops-export`), plus the `sign-in` screen.
The end-to-end test also writes a scheduled command and cancels it, writes
a handover entry and reads it back by date and shift, pages, filters, sorts
and exports the audit log, and saves a telemetry export. The screenshots, a zipped,
ad-hoc signed `Voltrus SCADA.app`, and the demo plant
(`voltrus-demo-plant.tar.gz`: the server, the Modbus simulator,
`ci-server.sh` and the scenario) are the workflow's artifacts; tagged
releases attach the app's zip as well.

To run the demo plant on a Mac (Apple silicon; `curl`, `jq` and `openssl`
ship with macOS 15):

```bash
gh run download <run id> -n voltrus-native-demo-plant-macos-arm64 -D plant
cd plant && tar -xzf voltrus-demo-plant.tar.gz
VOLTRUS_PLANT_PASSWORD=demo-plant-1 native/scripts/ci-server.sh work
```

It serves `http://127.0.0.1:3000` (Modbus simulators on 5020 and 5021) and
leaves everything running. Sign in to the console as `admin` with
`VOLTRUS_PLANT_PASSWORD` (at least 8 characters; when unset the script picks
a random one and prints it). Stop it with `kill $(cat work/*.pid)`.

Manual WebSocket debugging:

```bash
cargo run -p voltrus-scada-native --example ws_probe -- http://localhost:3000 admin <password>
```

## Server behaviour to know

- The alarm-flood banner counts the alarms raised in the last 5 minutes from
  the alarm list (`triggered_at`), in the web client and in the console
  alike. The analytics' `trigger_count` is a 24-hour figure per source and
  is not a 5-minute count.
- The login token comes only in the `Set-Cookie` header, with the JWT's
  base64 padding percent-encoded (`%3D`). The console decodes it
  (`api.rs:percent_decode`); the WebSocket refuses the encoded form.
- Writes address the data source's numeric id. The server answers every
  write on the connection that sent it: `acked` or `failed` for a write it
  tried (the cache takes the value only on `acked`), `rejected` with the
  reason for a refusal (not the admin role, a non-numeric id, an unknown or
  non-writable tag). The console still reports no answer after 10 seconds,
  for an answer lost with the connection, and corrects that if it comes
  later.
- The server's `active` alarm list holds unacknowledged alarms only, so the
  console also fetches the acknowledged and shelved lists and drops resolved
  ones. Alarms resolve without a WebSocket message; the list is fetched again
  every 10 seconds.
- The alarm summary counts waiting alarms whose priority is exactly
  `critical`, `warning` or `info`; the console's local count for other roles
  ranks any priority (high and medium count as warning). Acknowledging takes
  no comment (`POST /alarms/{id}/acknowledge` has no body); a shelve's reason
  is kept on the alarm and in the audit trail. Alarm sound needs an audio
  output device; without one the speaker's tooltip says why.
- Tag keys are register names, so two devices may share one (solar-farm has
  `frequency` twice). The console shows the first; keying by device is future
  work.
- `POST /api/v1/data-sources` stores `source_config` as the text of
  whatever JSON it gets, so the console sends an object (the web client's
  string is stored quoted), and ignores the alarm delays, which follow in a
  `PUT`. `PUT /api/v1/data-sources/{id}` stores a `source_config` string
  as-is and an object as its JSON text.
- `PUT /api/v1/shortcuts` replaces the user's whole map and stores a `null`
  as an empty shortcut instead of removing it: the console sends the full
  map without the cleared entry. The default "permissive" name rule refuses
  parentheses, so a duplicate is named `<name> copy` (the web's
  `<name> (Copy)` is refused).
- `settings/ui-theme` accepts only `light`, `dark` or `system`, so the
  bundled theme chosen for each brightness is kept on the workstation.
- `PUT /nav-shortcuts` and `PUT /shortcuts` replace the user's whole map,
  and `GET /nav-shortcuts` answers the defaults merged with the user's
  choices; the editor sends every entry each time, as the web client does.
- Single sign-on providers (`auth/providers`, from an add-on edition)
  finish in a browser; there is no loopback redirect or device code that
  would hand the session to a desktop client.
- `GET /api/v1/data-sources/{id}/write-policy` reads a tag's write policy
  (the same fields `PATCH` takes). The write dialog and the policy editor
  read it each time they open, so the console checks limits and the reason
  against what the server has, not only against a policy it saved itself.
  When the read fails the editor says so and saving sends only what changed
  (the choices made and the limits typed), leaving the rest as stored.
- `GET /api/v1/history` takes a `sensor` filter (a tag key or a
  comma-separated list; `device` too), and the detail chart asks only for
  the open tag and its overlays. `agg=avg` and `agg=minmax` bucket the
  series for Modbus tags as for the others, on multiples of the bucket
  length; `agg=minmax` adds `min` and `max` arrays next to `values`, which
  the web client draws as a band and the console, drawing the mean line
  only, ignores.
- A REST write records a command for the device (`pending`); the command
  queue sends it. The command carries the device's id as `device_ref`
  (a Modbus device's text id, also when the device exists only in the
  configuration file) next to the numeric `device_id`, which is 0 for a
  Modbus device; the console shows `device_ref`.
- Shift handover routes (all operator-only) list entries with the author's
  user id only; the console shows `user #id`. The automatic report places
  the shift in progress on the server's UTC clock (a night shift belongs to
  the date it started on, so between midnight and 06:00 UTC it is
  yesterday's) and sends its window as `period_start`/`period_end`
  (Unix seconds); the console shows that window and the alarms raised in it.
- The audit log's `created_at`, `from` and `to` are Unix seconds (the web
  client sends and reads seconds too). An unknown command
  answers 500 "Command not found", not 404; cancelling a command already
  sent answers 400.
- Engineering routes: a config snapshot covers devices, tags, flows,
  screens and recipes (not alarms, which a rollback leaves alone) and can be
  deleted; the MQTT broker's configuration cannot
  be cleared once set, and a message published through
  `POST /mqtt-broker/publish` does not reach the broker's own message log
  on the CI plant. Flows store no wires or positions (process steps run in
  stored order, then the outputs), so the editor's wires reorder steps and
  positions last while it is open; `start`/`stop` change only the running
  engine, not `enabled`. Saving a script with fixed source keeps its old
  `syntax_error`. CSV/Excel imports write readings only (no route removes
  them) and replace mode answers 501; the Node-RED import only converts and
  reports.
- Users, roles and API keys are admin-only routes. A user's role is one of
  `admin`, `demo`, `operator`, `viewer` or `user`; only admin and demo pass
  the operator routes. The server reads the role from the user's row on
  every request, so a role change applies at once and a deleted user is
  signed out; a role or password change or a deletion also revokes the
  user's refresh tokens. There is no way
  to disable an account (only delete it), and deleting a user whose
  handovers or mentions remain fails on a foreign key.
- RBAC roles (`/api/v1/roles`) store a JSON list of permissions, refused
  unless every one is in `models::ALL_PERMISSIONS` (the web's own list
  offers names the server refuses); nothing checks them yet, and
  `PUT /users/{id}/role` assigns one without any route to read it back.
- API keys expire in Unix seconds (the web client sends seconds too) and are shown once; the list keeps only
  the first eight characters. Their stored permissions are not checked;
  `api` keys read, `integration` keys also pass operator routes.
- `POST /notifications/test` answers 200 with a per-target status (sent or
  the error) for webhooks and email; email needs SMTP configured (else
  500), push is not testable. `PUT /notifications/settings/{id}` checks the
  config but not email or URL shapes, so the console checks them as
  `POST` does.
