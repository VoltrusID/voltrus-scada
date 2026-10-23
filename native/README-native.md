# Voltrus SCADA — Native Console (`voltrus-native`)

The GPU-accelerated operator console for the Voltrus SCADA server.

**Powered by [GPUI](https://gpui.rs) — Zed's GPU-accelerated Rust UI framework —
with components from Longbridge's [gpui-component](https://github.com/longbridge/gpui-component).
Every frame is rendered on the GPU (Metal on macOS, Vulkan/blade on Linux). No DOM, no
Electron, no browser: a 12 MB native binary that ships live tag streams at display
refresh rate.**

## Features (v0.1 — operator console)

- **Login** — server URL + credentials; URL/username remembered in `~/.config/voltrus-native/config.toml` (password never persisted)
- **Live tag dashboard** — sortable table of all data sources with warn/crit threshold coloring, quality state, per-tag decimals
- **Trend chart** — click any row to chart it; 300-point live ring per tag (same window as the web client), rendered as an area chart
- **Alarms** — active alarm list with severity coloring, per-row and bulk acknowledge; live WebSocket alarm feed
- **Status bar** — connection state, tag count, live **updates/s counter** (watch it scream)
- Auto-reconnect with backoff (2s → 5s → 10s) incl. automatic re-login on token expiry

## Build & run

```bash
cargo build -p voltrus-scada-native
./target/debug/voltrus-native          # or target/release
```

Start the server first (`cargo run -p voltrus`), then sign in with your
Voltrus credentials.

> Requires a recent Rust (edition 2024). macOS uses Metal; Linux needs X11 or
> Wayland dev libraries.

## Architecture

```
native/src/
  main.rs           GPUI bootstrap, window, login ↔ console switch, event pump
  ws_types.rs       MessagePack WS wire types (mirror of server src/ws_types.rs)
  api.rs            REST client: login (Set-Cookie parse + %3D decode), data-sources, alarms, ack
  runtime.rs        Dedicated tokio thread: WS client + reconnect backoff;
                    UiCmd in, RtEvent out, bridged into GPUI's executor
  views/
    login.rs        Login form
    console.rs      Tab shell (Dashboard / Alarms) + status bar
    dashboard.rs    Live table + AreaChart trend
    alarms.rs       Alarm table + acknowledge
```

The tokio I/O runtime and the GPUI render loop each own their thread; they
talk over lock-free unbounded channels, so UI frames never block on network.

Wire protocol: binary MessagePack frames over `/api/v1/ws` — identical to the
web client. `tests/ws_roundtrip.rs` mirrors the server's own wire tests.

## Tests

```bash
cargo test -p voltrus-scada-native           # unit + wire roundtrips

# Live end-to-end against a running server:
VOLTRUS_E2E_URL=http://localhost:3100 \
VOLTRUS_E2E_USERNAME=admin \
VOLTRUS_E2E_PASSWORD=... \
cargo test -p voltrus-scada-native --test e2e_live -- --nocapture

# Manual WS debugging:
cargo run -p voltrus-scada-native --example ws_probe -- http://localhost:3100 admin <password>
```

## Notes

- The server's `Set-Cookie` percent-encodes the JWT's base64 padding
  (`==` → `%3D%3D`). Browser cookie jars decode this transparently; this
  client does the same in `api.rs:percent_decode` — without it, WS auth
  rejects the raw token while REST accepts it.
- Duplicate data-source keys (same key on multiple devices) collapse to one
  live row; keying by device is future work.
