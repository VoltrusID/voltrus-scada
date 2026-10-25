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

- **Sign in** with the server address, username and password. The address and
  username are remembered in `<config dir>/voltrus-native/config.toml`
  (`~/Library/Application Support/voltrus-native/` on macOS); the password
  never is. When the server flags a generated or reset password, the console
  asks for a new one before anything else.
- **Overview**: tag count, tags that need attention, alarms to acknowledge and
  live updates per second; the worst tags and the open alarms (with
  Acknowledge); a tile per tag with its value, status and a 15-minute
  sparkline.
- **Tags**: every tag in a virtualised table (name, key, value and unit
  coloured by the server's status, status, age, 15-minute sparkline), filtered
  by name or key, ↑/↓ to move. The detail panel shows the value, the warning
  and critical limits, the last hour's lowest/highest/average and trend, a
  field to write a value (admin role, and the server's write policy must
  allow it), and Add to Trends.
- **Alarms**: open, to-acknowledge and shelved alarms, worst first, with
  Acknowledge, Acknowledge all (⇧⌘A), and for the admin, demo and integration
  roles (the ones the server lets shelve) Shelve for one hour and Unshelve.
  Alarm times show on the computer's clock.
- **Trends**: the chosen tags on one time axis over 5 min, 30 min, 1 h, 6 h or 24 h,
  filled from the server's history and extended by live values. While the
  link is live the chart moves on once a second. Each tag keeps a day of
  history: live values of the last 15 minutes at full rate, older ones
  thinned to one per 20 seconds.
- **Status bar**: link state (live, reconnecting with the reason, offline),
  server, updates per second, tag and alarm counts, and a warning when the
  alarm list could not be fetched.
- Light or dark (⇧⌘L), ⌘1–⌘4 for the pages, standard Edit menu in text fields.

The network side runs on its own tokio thread: it signs in, keeps the
WebSocket open, reconnects with backoff (2 s, 5 s, then every 10 s) signing in
again each time, and renews the 15-minute session token every 10 minutes
(30 seconds after a renewal the server did not answer) so REST calls keep
working. Only the operator's own sign-in waits for the server; every other
REST call (tags, alarms, history, alarm actions, password changes, renewals,
the sign-in before a reconnect) runs on its own and never holds up the tag
stream. Answers that arrive after the session ended are dropped. When the
server refuses the stored password (changed elsewhere, account locked) the
session ends with "Not authorised" instead of trying again, since every
wrong password counts towards locking the account.

## Build and run

```bash
cargo build --release -p voltrus-scada-native
./target/release/voltrus-native
```

Options, mainly for kiosks and screenshots:

```
voltrus-native [--sign-in] [--page overview|tags|alarms|trends] [--tag <key>] [--range 5m|30m|1h|6h|24h]
               [--appearance dark|light]
```

`--sign-in` signs in at once with `VOLTRUS_NATIVE_URL` (default
`http://localhost:3000`), `VOLTRUS_NATIVE_USER` (default `admin`) and
`VOLTRUS_NATIVE_PASSWORD`. `--tag` opens one tag's detail; `--range` picks the Trends range.

Requires a recent stable Rust (edition 2024). CI builds and tests on macOS;
GPUI also supports Linux (X11 or Wayland), which is not tested here.

## Layout

```
native/src/
  lib.rs            api, runtime and ws_types (no UI; used by the tests)
  api.rs            REST client: login (token from Set-Cookie), data sources,
                    alarms (acknowledge, shelve, unshelve), history, password change
  ws_types.rs       MessagePack WebSocket messages (mirror of the server's)
  runtime.rs        the network actor: Command in, Event out
  main.rs           command line, window, theme, menus
  console/
    mod.rs          the Console model: phases, pages, actions, key bindings
    store.rs        tags, values, statuses and history; update rate
    config.rs       remembered address and username
    views/          one function per area: sign_in, sidebar, status_bar,
                    overview, tags, alarms, trends
native/scripts/     CI: the solar-farm plant, app bundle, screenshots
```

## Tests

```bash
cargo test -p voltrus-scada-native     # unit tests and wire round trips

# End to end against a running server:
VOLTRUS_E2E_URL=http://localhost:3000 \
VOLTRUS_E2E_USERNAME=admin \
VOLTRUS_E2E_PASSWORD=... \
cargo test -p voltrus-scada-native --test e2e_live -- --nocapture
```

Set `VOLTRUS_E2E_EXPECT_UPDATES=1` to fail when no live values arrive.

The `Native console` workflow (`.github/workflows/native.yml`) runs on macOS:
formatting, clippy, the tests, then a release build of the console, the
server and the Modbus simulator. `native/scripts/ci-server.sh` starts the
solar-farm scenario, resets the admin password with
`voltrus --reset-admin-password`, replaces it through the API, and sets
plant-like alarm limits. The end-to-end test runs against that server
(signing in, tags, history, live values, acknowledging an open alarm, and
changing the admin password and back), and
`native/scripts/screenshots-macos.sh` captures every page of the signed-in
console. The screenshots and a zipped, ad-hoc signed `Voltrus SCADA.app` are
the workflow's artifacts; tagged releases attach the zip as well.

Manual WebSocket debugging:

```bash
cargo run -p voltrus-scada-native --example ws_probe -- http://localhost:3000 admin <password>
```

## Server behaviour to know

- The login token comes only in the `Set-Cookie` header, with the JWT's
  base64 padding percent-encoded (`%3D`). The console decodes it
  (`api.rs:percent_decode`); the WebSocket refuses the encoded form.
- Writes address the data source's numeric id. The server answers only
  writes it accepts (admin role, a writable tag); otherwise it stays silent,
  so the console reports no answer after 10 seconds, and corrects that if
  the answer comes later. The server sends every answer to every client, so
  write ids carry a random per-session nonce.
- The server's `active` alarm list holds unacknowledged alarms only, so the
  console also fetches the acknowledged and shelved lists and drops resolved
  ones. Alarms resolve without a WebSocket message; the list is fetched again
  every 10 seconds.
- Tag keys are register names, so two devices may share one (solar-farm has
  `frequency` twice). The console shows the first; keying by device is future
  work.
