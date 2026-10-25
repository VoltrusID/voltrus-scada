# Native console architecture

How the GPUI console (`native/`) is put together, and how to add a page, a
REST call, a key binding, an overlay or an animation. Later parity slices
follow these recipes; keep them true when you change the shell.

## The pieces

| Piece | Where | What it does |
|---|---|---|
| REST client | `src/api.rs` | `VoltrusApi`: sign-in cookies, typed mirrors of server rows, and three generic helpers: `get_json`, `send_json`, `send_empty`. |
| Runtime | `src/runtime.rs` | One actor on its own tokio thread: the session, the WebSocket, renewals, every REST call. The UI sends `Command`s, hears `Event`s. Answers from an ended session are dropped by session id. |
| Stress feed | `src/stress.rs` | A deterministic synthetic plant, fed through the runtime's event channel like the real stream. |
| Frame statistics | `src/perf.rs` | Percentiles, drop counting, the stress report. |
| Model | `src/console/mod.rs` | One entity, `Console`: session state, the store, pages, overlay slot, HUD, stress run. |
| Store | `src/console/store.rs` | Tags by data source id; WebSocket updates by key (shared keys routed by list order). |
| Page registry | `src/console/pages.rs` | Every page in one table, `PAGES`. |
| Commands and keys | `src/console/keys.rs` | Every command in one table, `COMMANDS`; key bindings, the View menu, the palette and the help overlay come from it. |
| Keyboard lists | `src/console/nav.rs`, `views::keyboard_list` | A focusable list with a cursor row. |
| Overlay slot | `src/console/overlay.rs`, `views/overlays.rs` | One modal, sheet or popover at a time, with a focus trap. |
| Animation | `src/console/anim.rs` | Easing, progress, `request_frame`. |
| Frame meter | `src/console/meter.rs` | Feeds the HUD (⇧⌘P), the Performance page and the stress report. |
| Views | `src/console/views/` | Functions over `&mut Console`, each part drawn by its own cached view. |

## Rules

1. **No frame when nothing moves.** A view calls
   `anim::request_frame(window, &console.meter)` only while something it
   draws is animating (a flash, a tween, a live trend). Nothing loops on
   timers to redraw; the one-second ticker is the only periodic redraw
   (ages, rates). An idle console draws nothing between data batches.
2. **Cached views.** The navbar, the page, the status bar, the toasts, the
   overlay and the HUD are each a `CachedView` drawn with `AnyView::cached`.
   A view that asks for a frame redraws itself and its ancestors (the thin
   root); its siblings replay their last frame. Draw new window parts the
   same way; never put an animation in the root.
3. **Pages stay alive.** A page's view is made on first show and kept; its
   state (cursor, scroll, filters, its `Console::state::<S>()`) survives a
   switch, and switching does not refetch. Fetch in `on_show` only when the
   data is not there yet.
4. **Tokens only.** Colours from `vt_ui::theme::theme(cx)`, sizes from the
   metrics constants (`SPACE_*`, `FONT_*`, `RADIUS_*`, `CONTROL_*`), values
   in `MONO`. No hex colours or magic pixel sizes beyond layout widths.
5. **Lists leave room for the toasts**: `.pb(LIST_TAIL)` on every scrolling
   list, so its last rows scroll clear of the bottom-right toast stack.
6. **Roles**: gate on `console.is_operator()` (admin, demo, integration:
   the server's `demo_protected` routes) or `console.is_admin()`; never a
   list of role names.

## Add a page

1. Add a variant to `pages::Page`.
2. Write `views/<page>.rs` with
   `pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement`
   (and optionally `on_show`, `on_reply`, `search`, `side_panel`). Declare
   it `pub(crate) mod <page>;` in `views/mod.rs`.
3. Add one row to `pages::PAGES`:

   ```rust
   PageSpec {
       page: Page::Commands,
       id: "commands",            // --page commands; CallKey scope
       title: "Commands",
       icon: Icon::Send,
       digit: Some(5),            // ⌘5; each digit once, or None
       role: Role::Any,           // or Operator / Admin
       placement: Placement::More, // or Tab
       render: views::commands::render,
       badge: None,               // Some(fn(&Console) -> usize)
       on_show: Some(views::commands::on_show),
       on_reply: Some(views::commands::on_reply),
       search: None,              // Some(fn(&Console) -> Option<Entity<TextInput>>), focused by `/`
       side_panel: no_panel,      // width the toasts keep clear of
   }
   ```

That is all: the navbar tab or More item, the ⌘ digit, the View menu, the
palette's "Go to …", the help overlay, `--page` and role gating follow from
the row. Keep page state in your module: `#[derive(Default)] struct
State { … }` and `console.state::<State>()`.

## Add a REST call

1. In `api.rs`, mirror the answer (`#[derive(Deserialize)]`, `#[serde(default)]`
   where the server may omit fields).
2. From the page, send it with a key scoped by the page id:

   ```rust
   console.call(
       CallKey::new("commands.list").with(page_number),
       Call::get::<Vec<api::Command>>(format!("/api/v1/commands?limit=20&offset={offset}")),
   );
   // POST with a body:  Call::send::<IgnoredAny>(Method::POST, path, serde_json::json!({...}))
   // POST, no body:     Call::send_empty::<IgnoredAny>(Method::POST, path)
   // anything else:     Call::new(|api| async move { api.get_json::<T>(path).await })
   ```

3. Handle the answer in the page's `on_reply`:

   ```rust
   pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
       match key.name {
           "commands.list" => match reply.take::<Vec<api::Command>>() {
               Ok(list) => console.state::<State>().list = list,
               Err(error) => console.show_toast(format!("Commands: {error}"), true, cx),
           },
           _ => {}
       }
       cx.notify();
   }
   ```

The runtime decodes off the UI thread and drops answers for an ended
session. A failure for a key whose page has no `on_reply` becomes a toast.
After a mutation that changes alarms, also send `Command::RefreshAlarms`.
Add an e2e step in `tests/e2e_live.rs` for every new path.

## Add a key binding or command

Add a row to `keys::COMMANDS`:

```rust
command!("Cancel the command", Alarms, X, ["secondary-shift-x"], CONSOLE, CancelCommand, true),
//        label                 group   icon  keys                context  action        in palette
```

declare the action in the `actions!` list, and handle it on the root in
`views/mod.rs` (`.on_action(cx.listener(|console, _: &CancelCommand, window, cx| …))`).
The binding, the help overlay line and the palette entry follow. Contexts:
`CONSOLE` (anywhere), `NOT_TYPING` (not in a text field), `LIST` (a keyboard
list has focus), `MENU` (a popup list, e.g. the palette). `secondary` is ⌘
on macOS and Ctrl elsewhere. Unit tests check every row binds and no two
rows share keys in a context.

## Make a list keyboard-driven

```rust
let list = keyboard_list(
    ListKeys { id: LIST, on_move: None, on_activate: open_row },
    rows.len(),
    rows_on_screen(window, ROW_HEIGHT),
    console,
    cx,
);
let nav = console.list(LIST, cx);
let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
// rows: .relative() and .when(focused && cursor == Some(row), |row| row.child(focus_ring(&theme)))
// uniform_list(...).track_scroll(scroll).pb(LIST_TAIL)
list.flex_1().min_h_0().child(the_list)
```

j/k and ↑/↓ move, PgUp/PgDn a screenful, Home/End to the ends, Enter calls
`on_activate(.., toggle = false)`, Space `toggle = true`. Focus the list in
the page's `on_show` with `console.focus_list(LIST, window, cx)`.

## Open an overlay

```rust
let overlay = Overlay::modal("shelve", px(420.0), views::alarms::shelve_dialog, cx);
// or Overlay::sheet(id, width, render, cx), Overlay::popover(id, width, at, render, cx)
console.open_overlay(overlay, window, cx);
```

The render function draws the content only; the frame (backdrop, card,
focus trap, click-outside-closes) is the shell's. Tab cycles inside the
overlay, Esc closes it (a text field's Esc closes it too if you subscribe to
`InputEvent::Cancelled`), and focus returns where it was. Keep the dialog's
state in your page's state; close with `console.close_overlay(window, cx)`.

## Animate

- One-shot transitions: GPUI's `with_animation` with an id that changes per
  occurrence (see the page entrance in `views::page` and the toasts), and
  `console.meter.borrow_mut().expect_frame(now)` while it runs so the meter
  counts its frames.
- Value-driven motion: compute progress from a timestamp
  (`anim::progress(since, now, anim::FLASH)`), draw with `anim::mix` /
  `anim::lerp`, and call `anim::request_frame` while any is running (the
  Overview tiles).
- Continuous motion: only while visible and live (the Trends chart).

## Measure

⇧⌘P shows the HUD: frames per second, frame interval and build time p50,
p95 and max, drops against the measured display refresh, the last frame's
build time, updates per second, tags and peak memory. The Performance page
charts the last ten seconds. `--stress` feeds 10 000 synthetic tags at
20 000 updates/s through the runtime, switching pages every 2 s;
`--stress-seconds N --stress-report FILE` ends the run, prints the report
and writes it. CI runs a 20 s pass (`native/scripts/stress-macos.sh`) and
prints the report in the job log and summary.

## Add a Data Manager section

The Data Manager (`views/data_manager/`, `--page data`) is one page with a
sidebar of sections grouped Data, Protocols, Insights and Admin, as the web
client's `DataManager.tsx`. A section is one module plus one row:

1. Write `views/data_manager/<section>.rs` with
   `pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement`
   (it draws everything right of the sidebar, its `page_header` included)
   and the optional hooks below. Declare it `pub(crate) mod <section>;` in
   `views/data_manager/mod.rs`.
2. Add one row to `data_manager::SECTIONS`:

   ```rust
   SectionSpec {
       id: "modbus",                  // calls are named data.modbus.<what>
       title: "Modbus",
       icon: Icon::Cable,
       group: SectionGroup::Protocols, // Data, Protocols, Insights or Admin
       role: Role::Operator,          // web `admin` items; `strictAdmin` is Role::Admin
       render: modbus::render,
       on_open: Some(modbus::on_open), // shown (page opened, sidebar moved): fetch once
       focus: Some(modbus::focus),     // Enter in the sidebar: focus the section's list
       on_reply: Some(modbus::on_reply),
       search: Some(modbus::search),   // `/`
   }
   ```

That is all: the sidebar lists the row for users whose role allows it, a
group appears once it has a row, and nothing is listed for a section that is
not built. The page routes the answer of every `CallKey` named
`data.<id>.<what>` to that section's `on_reply`, so a section sends
`console.call(CallKey::new("data.modbus.list"), Call::get::<…>(…))` and
matches on `key.name` there. Keep the section's state in its module
(`console.state::<State>()`, and `console.state_ref::<State>()` in hooks
that get `&Console`).

Keys: j/k or ↑↓ in the sidebar switch sections (`on_open` runs), Enter goes
in (`focus` runs), Esc anywhere in the section comes back to the sidebar
(the page handles `Dismiss` around the section; overlays and text fields
take their own Esc first). Commands that act on a section's list get rows in
`keys::COMMANDS` with a key context the section sets around its list (the
Sources table uses `keys::DATA_SOURCES`: N, D, ⌫, ⌥↑ ⌥↓, S) and handle them
on that element; a command that must work from anywhere (New data source in
the palette) is handled on the root.

After a change to data sources, `console.refresh_tags()` fetches the tag
list again (`Command::RefreshTags`, answered by `Event::Tags` as at sign-in),
so every page shows what the server now has. Replies come without a window:
to close a dialog after the server answered, keep the window handle in the
section's state and close it in `cx.defer` (see
`data_manager::source_form::close_later`).
## Poll while a page is on screen

A list the server changes without a WebSocket message (the Commands
page's Active list, every 2 s; the shift report, every 30 s) is fetched
again only while its page is on screen:

```rust
// in on_show
ops_kit::poll_while_shown(console, Page::Commands, Duration::from_secs(2), tick, cx);
```

The loop ends by itself when the page goes off screen, the session ends or
a stress test runs; the next `on_show` starts it again. It only fetches; the
answer redraws through `on_reply` like any other. Number each request
(`CallKey::new("commands.active").with(seq)`) and drop answers whose
argument is not the newest, so a slow answer never overwrites a fresh one.

## Page keys

Keys that only make sense on one page bind to a key context its root
carries: the Commands page's root has `key_context("OpsCommands OpsPaged
…")`, and `keys::COMMANDS` binds `x` in `"OpsCommands && !TextInput"`, so
the key works wherever focus is inside the page except in a text field.
Handle the action on the page's root (`.on_action(...)`); handle it on the
window's root (`views/mod.rs`) only when the palette should offer it from
any page.

## Text fields

`vt_ui::text_input::TextInput` is one line. `console::text_area::TextArea`
is the multi-line field (Enter is a new line, ↑/↓ move between lines, ⌘↵
submits); it answers the same `text_input` actions and emits the same
`InputEvent`s, and its key context includes `TextInput`, so `!TextInput`
keys stay off while typing in it. A page reads its own state from
`&Console` hooks (such as `search`) with `console.state_ref::<State>()`.

## Save a file

`ops_kit::save_download(key, path, suggested_name, format, cancelled, cx)`
asks where to save (GPUI's `prompt_for_new_path`, in Downloads), then
fetches `path` as text off the UI thread (`VoltrusApi::download`) and
answers the page of `key` with an `api::ops::Saved` (file, rows, the
server's total).
