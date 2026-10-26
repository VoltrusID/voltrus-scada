//! Data Manager › Recipes, the web client's `RecipeManager.tsx` and
//! `RecipeLoadStatus.tsx`: a device's saved values by name and category.
//! Save a device's current values as a recipe, rename it or change its
//! category and description, delete it, and load it back: the server
//! queues one command per value, and the section follows those commands
//! (every 2 s while on screen, until each is done), retries a failed one,
//! and opens them on the Commands page.
//!
//! Every recipe route is an operator route (`demo_protected`, the list
//! too), so the section is listed for operators only.
//!
//! Keys: `/` searches by name, Enter opens a recipe (its values beside the
//! live ones, and its fields), `n` saves a new one, `l` loads the recipe
//! under the cursor, ⌫ deletes it, `,`/`.` step the device filter, `c` the
//! category filter, `r` fetches again.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    SharedString, Subscription, Window, actions, div, px, uniform_list,
};
use serde::de::IgnoredAny;
use serde_json::json;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::insights::{
    self, LoadTally, NewRecipe, RECIPES_PATH, Recipe, RecipeLoad, RecipeUpdate, categories, tally,
};
use voltrus_scada_native::api::ops::QueuedCommand;
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use voltrus_scada_native::tag_api::WriteAccepted;
use vt_ui::Icon;
use vt_ui::chart::format_value;
use vt_ui::components::{Button, IconButton, field, pill};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::insights_kit::{self, chips, devices, every_while_shown};
use super::sidebar_width;
use crate::console::overlay::Overlay;
use crate::console::views::commands;
use crate::console::views::ops_kit::{close_overlay_in, command_status_color, notice};
use crate::console::views::tag_detail::{dialog, note_line, on_field, text_field};
use crate::console::views::{
    ListKeys, card, focus_ring, keyboard_list, local_time, page_header, rows_on_screen, tail_row,
    tail_rows,
};
use crate::console::{Console, Page};

/// The section's id: its calls are `data.recipes.*`.
pub(crate) const ID: &str = "recipes";
/// The section root's key context; its keys never fire while typing.
pub const RECIPES: &str = "Recipes && !TextInput";
const LIST: &str = "data.recipes";
const NEW: &str = "recipe-new";
const SHEET: &str = "recipe-sheet";
const LOAD: &str = "recipe-load";
const DELETE: &str = "recipe-delete";
const ROW_HEIGHT_PX: f32 = 34.0;
const STATUS_EVERY: Duration = Duration::from_secs(2);

/// A tag's unit and live value.
type Now = (String, f64);

actions!(
    voltrus_console,
    [
        RecipesNew,
        RecipesLoad,
        RecipesDelete,
        RecipesPreviousDevice,
        RecipesNextDevice,
        RecipesNextCategory,
        RecipesRefresh,
    ]
);

/// The form for a new recipe.
struct NewForm {
    name: Entity<TextInput>,
    category: Entity<TextInput>,
    description: Entity<TextInput>,
    device: Option<SharedString>,
    error: Option<String>,
    sending: bool,
    _subscriptions: Vec<Subscription>,
}

/// The open recipe: its fields while edited.
struct Sheet {
    recipe: Recipe,
    name: Entity<TextInput>,
    category: Entity<TextInput>,
    description: Entity<TextInput>,
    /// The saved values with the store position of the tag each writes.
    values: Vec<(String, f64, Option<usize>)>,
    error: Option<String>,
    sending: bool,
    _subscriptions: Vec<Subscription>,
}

/// A recipe load being followed.
pub(crate) struct LoadRun {
    recipe: String,
    device: String,
    message: String,
    /// The commands the load queued, a retried one replaced by its retry.
    pub(crate) ids: Vec<String>,
    commands: HashMap<String, QueuedCommand>,
    seq: i64,
    retrying: Vec<String>,
    errors: HashMap<String, String>,
}

impl LoadRun {
    /// Command `old` was written again as command `new`: follow the new
    /// one in its place.
    fn retried(&mut self, old: &str, new: String) {
        self.retrying.retain(|id| id != old);
        self.errors.remove(old);
        if let Some(slot) = self.ids.iter_mut().find(|id| *id == old) {
            *slot = new;
        }
    }

    /// Writing command `old` again was refused: say why beside it.
    fn failed_retry(&mut self, old: &str, error: String) {
        self.retrying.retain(|id| id != old);
        self.errors.insert(old.to_owned(), error);
    }
}

#[derive(Default)]
pub(crate) struct State {
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    /// The device the server lists recipes of; `None` for every device.
    device: Option<SharedString>,
    /// The category shown; `None` for every category.
    category: Option<String>,
    list: Option<Result<Rc<Vec<Recipe>>, String>>,
    seq: i64,
    asked: bool,
    new: Option<NewForm>,
    sheet: Option<Sheet>,
    confirm_load: Option<(Recipe, FocusHandle, bool)>,
    confirm_delete: Option<(Recipe, FocusHandle)>,
    pub(crate) load: Option<LoadRun>,
    window: Option<AnyWindowHandle>,
}

pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() || console.state::<State>().asked {
        return;
    }
    console.state::<State>().asked = true;
    fetch(console);
    cx.notify();
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search recipes  /"));
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(LIST, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(LIST, cx);
                nav.select(0);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.search = Some(field);
    state._search = Some(subscription);
}

fn fetch(console: &mut Console) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let path = insights::recipes_path(insights_kit::text(&state.device));
    console.call(
        CallKey::new("data.recipes.list").with(seq),
        Call::get::<Vec<Recipe>>(path),
    );
}

/// The commands of the load being followed, each fetched again.
fn fetch_commands(console: &mut Console) {
    let Some(run) = console.state::<State>().load.as_mut() else {
        return;
    };
    run.seq += 1;
    let (seq, ids) = (run.seq, run.ids.clone());
    console.call(
        CallKey::new("data.recipes.commands").with(seq),
        Call::new(move |api| async move {
            let answers = futures_util::future::join_all(ids.iter().map(|id| {
                let api = &api;
                let path = insights::command_path(id);
                async move { api.get_json::<QueuedCommand>(&path).await }
            }))
            .await;
            anyhow::Ok(
                ids.into_iter()
                    .zip(answers)
                    .filter_map(|(id, answer)| Some((id, answer.ok()?)))
                    .collect::<Vec<(String, QueuedCommand)>>(),
            )
        }),
    );
}

/// Every 2 s on screen while a load runs: its commands again.
fn tick(console: &mut Console, cx: &mut Context<Console>) -> bool {
    let running = console
        .state::<State>()
        .load
        .as_ref()
        .is_some_and(|run| !tally(&run.ids, &run.commands).done);
    if running {
        fetch_commands(console);
        cx.notify();
    }
    running
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "data.recipes.list" if key.arg == state.seq => {
            state.list = Some(reply.take::<Vec<Recipe>>().map(Rc::new));
        }
        "data.recipes.create" => match reply.take::<Recipe>() {
            Ok(recipe) => {
                state.new = None;
                close_dialog(console, NEW, cx);
                console.show_toast(
                    format!(
                        "Saved {} with {} values of {}",
                        recipe.name,
                        recipe.values().len(),
                        recipe.device_id
                    ),
                    false,
                    cx,
                );
                fetch(console);
            }
            Err(error) => {
                if let Some(form) = state.new.as_mut() {
                    form.sending = false;
                    form.error = Some(error);
                }
            }
        },
        "data.recipes.update" => match reply.take::<Recipe>() {
            Ok(recipe) => {
                state.sheet = None;
                close_dialog(console, SHEET, cx);
                console.show_toast(
                    format!("Saved {} (v{})", recipe.name, recipe.version),
                    false,
                    cx,
                );
                fetch(console);
            }
            Err(error) => {
                if let Some(sheet) = state.sheet.as_mut() {
                    sheet.sending = false;
                    sheet.error = Some(error);
                }
            }
        },
        "data.recipes.delete" => {
            match reply.error() {
                None => console.show_toast("Recipe deleted", false, cx),
                Some(error) => console.show_toast(format!("Delete failed: {error}"), true, cx),
            }
            fetch(console);
        }
        "data.recipes.load" => match reply.take::<(String, String, RecipeLoad)>() {
            Ok((recipe, device, load)) => {
                let queued = load.command_ids.len();
                state.confirm_load = None;
                state.load = Some(LoadRun {
                    recipe,
                    device,
                    message: load.message,
                    ids: load.command_ids,
                    commands: HashMap::new(),
                    seq: 0,
                    retrying: Vec::new(),
                    errors: HashMap::new(),
                });
                close_dialog(console, LOAD, cx);
                if queued > 0 {
                    fetch_commands(console);
                    every_while_shown(console, ID, STATUS_EVERY, tick, cx);
                }
                // The load changes nothing in the recipe, but the list may
                // have changed meanwhile.
                fetch(console);
            }
            Err(error) => {
                if let Some((_, _, sending)) = state.confirm_load.as_mut() {
                    *sending = false;
                }
                console.show_toast(format!("Load failed: {error}"), true, cx);
            }
        },
        "data.recipes.commands" => {
            let Some(run) = state.load.as_mut() else {
                return;
            };
            if key.arg != run.seq {
                return;
            }
            if let Ok(found) = reply.take::<Vec<(String, QueuedCommand)>>() {
                run.commands.extend(found);
            }
        }
        "data.recipes.retry" => {
            let Some(run) = state.load.as_mut() else {
                return;
            };
            match reply.take::<(String, Result<WriteAccepted, String>)>() {
                Ok((old, Ok(accepted))) => {
                    run.retried(&old, accepted.command_id);
                    fetch_commands(console);
                    every_while_shown(console, ID, STATUS_EVERY, tick, cx);
                }
                Ok((old, Err(error))) => run.failed_retry(&old, error),
                Err(error) => console.show_toast(format!("Retry failed: {error}"), true, cx),
            }
        }
        _ => return,
    }
    cx.notify();
}

fn close_dialog(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    if let Some(window) = console.state::<State>().window {
        close_overlay_in(window, id, cx);
    }
}

// ── Rows ─────────────────────────────────────────────────────────────────

/// The recipes shown: the category chosen, the search typed.
fn rows(console: &mut Console, cx: &gpui::App) -> Vec<Recipe> {
    let query = console
        .state_ref::<State>()
        .and_then(|state| state.search.as_ref())
        .map(|field| field.read(cx).value().trim().to_lowercase())
        .unwrap_or_default();
    let state = console.state::<State>();
    let Some(Ok(list)) = state.list.as_ref() else {
        return Vec::new();
    };
    let category = state.category.as_deref();
    list.iter()
        .filter(|recipe| category.is_none() || recipe.category() == category)
        .filter(|recipe| query.is_empty() || recipe.name.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

fn cursor_recipe(console: &mut Console, cx: &mut Context<Console>) -> Option<Recipe> {
    let row = console.list(LIST, cx).cursor?;
    rows(console, cx).into_iter().nth(row)
}

fn pick_device(console: &mut Console, device: Option<SharedString>, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.device = device;
    state.category = None;
    fetch(console);
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

fn step_device(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let options = devices(console);
    let current = console.state::<State>().device.clone();
    if let Some(next) = insights_kit::step(&options, insights_kit::text(&current), forward).cloned()
    {
        pick_device(console, Some(next), cx);
    }
}

fn pick_category(console: &mut Console, category: Option<SharedString>, cx: &mut Context<Console>) {
    console.state::<State>().category = category.map(|category| category.to_string());
    console.list(LIST, cx).cursor = None;
    cx.notify();
}

fn next_category(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(Ok(list)) = state.list.as_ref() else {
        return;
    };
    let options: Vec<SharedString> = categories(list).into_iter().map(Into::into).collect();
    // Every category, then all of them again.
    let next = match state.category.as_deref() {
        None => options.first().cloned(),
        Some(current) => options
            .iter()
            .position(|option| option == current)
            .and_then(|at| options.get(at + 1).cloned()),
    };
    pick_category(console, next, cx);
}

fn needs_operator(console: &mut Console, cx: &mut Context<Console>) -> bool {
    if console.is_operator() {
        return false;
    }
    console.show_toast(
        "Recipes need the operator role (admin, demo or integration)",
        true,
        cx,
    );
    true
}

// ── New recipe ───────────────────────────────────────────────────────────

fn open_new(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if needs_operator(console, cx) {
        return;
    }
    let name = text_field("Summer setpoints", InputKind::Text, false, cx);
    let category = text_field("Optional", InputKind::Text, false, cx);
    let description = text_field("Optional", InputKind::Text, false, cx);
    let subscriptions = [&name, &category, &description]
        .into_iter()
        .map(|input| on_field(input, submit_new, window, cx))
        .collect();
    let device = console
        .state::<State>()
        .device
        .clone()
        .or_else(|| devices(console).first().cloned());
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.new = Some(NewForm {
        name: name.clone(),
        category,
        description,
        device,
        error: None,
        sending: false,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::modal(NEW, px(500.0), new_dialog, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn submit_new(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<State>().new.as_mut() else {
        return;
    };
    if form.sending {
        return;
    }
    let body = match NewRecipe::new(
        form.name.read(cx).value(),
        insights_kit::text(&form.device).unwrap_or(""),
        form.category.read(cx).value(),
        form.description.read(cx).value(),
    ) {
        Ok(body) => body,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.sending = true;
    form.error = None;
    console.call(
        CallKey::new("data.recipes.create"),
        Call::new(move |api| async move {
            api.send_json::<_, Recipe>(Method::POST, RECIPES_PATH, &body)
                .await
        }),
    );
    cx.notify();
}

fn new_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let options = devices(console);
    let Some(form) = console.state::<State>().new.as_ref() else {
        return div().into_any_element();
    };
    let device = form.device.clone();
    let device_row = if options.is_empty() {
        note_line(
            Icon::Info,
            "The plant's tags name no device.".into(),
            theme.fg_muted,
        )
        .into_any_element()
    } else {
        chips(
            "recipe-new-device",
            &options,
            insights_kit::text(&device),
            None,
            |console, device, cx| {
                if let Some(form) = console.state::<State>().new.as_mut() {
                    form.device = device;
                }
                cx.notify();
            },
            cx,
        )
    };
    let mut body = vec![
        field("Name", form.name.clone(), cx).into_any_element(),
        field("Device", device_row, cx).into_any_element(),
        div()
            .grid()
            .grid_cols(2)
            .gap(SPACE_6)
            .child(field("Category", form.category.clone(), cx))
            .child(field("Description", form.description.clone(), cx))
            .into_any_element(),
        note_line(
            Icon::Info,
            "The server keeps the device's values as they are now.".into(),
            theme.fg_muted,
        )
        .into_any_element(),
    ];
    if let Some(error) = form.error.clone() {
        body.push(note_line(Icon::CircleAlert, error, theme.danger).into_any_element());
    }
    let sending = form.sending;
    dialog(
        "Save current as recipe",
        "A named set of the device's values, to load back later (↵ saves).",
        body,
        vec![
            Button::new("recipe-new-cancel", "Cancel")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| {
                    console.state::<State>().new = None;
                    console.close_overlay(window, cx);
                }))
                .into_any_element(),
            Button::new("recipe-new-save", if sending { "Saving…" } else { "Save" })
                .primary()
                .icon(Icon::Check)
                .disabled(sending)
                .on_click(cx.listener(|console, _, window, cx| submit_new(console, window, cx)))
                .into_any_element(),
        ],
        cx,
    )
}

// ── The open recipe ──────────────────────────────────────────────────────

fn open_sheet(
    console: &mut Console,
    recipe: Recipe,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let prefilled = |text: &str, placeholder: &'static str, cx: &mut Context<Console>| {
        let input = text_field(placeholder, InputKind::Text, false, cx);
        let text = text.to_owned();
        input.update(cx, |field, cx| field.set_text(text, cx));
        input
    };
    let name = prefilled(&recipe.name, "Name", cx);
    let category = prefilled(recipe.category().unwrap_or(""), "None", cx);
    let description = prefilled(recipe.description().unwrap_or(""), "None", cx);
    let subscriptions = [&name, &category, &description]
        .into_iter()
        .map(|input| on_field(input, submit_sheet, window, cx))
        .collect();
    // Where each value goes: the device's tag with the register's key,
    // looked up once here.
    let values = recipe
        .values()
        .into_iter()
        .map(|(key, value)| {
            let at = console
                .store
                .tags
                .iter()
                .position(|tag| tag.info.device_id == recipe.device_id && tag.info.key == key);
            (key, value, at)
        })
        .collect();
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.sheet = Some(Sheet {
        recipe,
        name: name.clone(),
        category,
        description,
        values,
        error: None,
        sending: false,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::sheet(SHEET, px(500.0), sheet_view, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn submit_sheet(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(sheet) = console.state::<State>().sheet.as_mut() else {
        return;
    };
    if sheet.sending {
        return;
    }
    let body = match RecipeUpdate::new(
        sheet.name.read(cx).value(),
        sheet.category.read(cx).value(),
        sheet.description.read(cx).value(),
    ) {
        Ok(body) => body,
        Err(error) => {
            sheet.error = Some(error);
            cx.notify();
            return;
        }
    };
    sheet.sending = true;
    sheet.error = None;
    let path = insights::recipe_path(sheet.recipe.id);
    console.call(
        CallKey::new("data.recipes.update"),
        Call::new(
            move |api| async move { api.send_json::<_, Recipe>(Method::PUT, &path, &body).await },
        ),
    );
    cx.notify();
}

#[allow(clippy::too_many_lines)]
fn sheet_view(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let Some(sheet) = console
        .state_ref::<State>()
        .and_then(|state| state.sheet.as_ref())
    else {
        return div().into_any_element();
    };
    let recipe = sheet.recipe.clone();
    // Per value: its key, the saved value, and the tag's unit and value now.
    let live: Vec<(String, f64, Option<Now>)> = sheet
        .values
        .iter()
        .map(|(key, value, at)| {
            let now = at.map(|at| {
                let tag = &console.store.tags[at];
                (tag.info.unit.clone(), tag.value)
            });
            (key.clone(), *value, now)
        })
        .collect();
    let (name, category, description, error, sending) = (
        sheet.name.clone(),
        sheet.category.clone(),
        sheet.description.clone(),
        sheet.error.clone(),
        sheet.sending,
    );
    let value_rows = live
        .into_iter()
        .enumerate()
        .map(|(row, (key, saved, now))| {
            let (live, differs) = match &now {
                Some((unit, value)) => (
                    if unit.is_empty() {
                        format_value(*value)
                    } else {
                        format!("{} {unit}", format_value(*value))
                    },
                    (value - saved).abs() > f64::EPSILON.max(saved.abs() * 1e-9),
                ),
                None => ("no tag".to_owned(), false),
            };
            div()
                .id(("recipe-value", row))
                .flex()
                .items_center()
                .gap(SPACE_5)
                .h(px(28.0))
                .border_b_1()
                .border_color(theme.border.alpha(0.6))
                .text_size(FONT_SMALL)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .child(key),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .text_right()
                        .font_family(MONO)
                        .text_color(theme.accent)
                        .child(format_value(saved)),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .text_right()
                        .font_family(MONO)
                        .text_color(match now {
                            None => theme.fg_dim,
                            Some(_) if differs => theme.warning,
                            Some(_) => theme.fg_muted,
                        })
                        .child(live),
                )
        });
    let count = recipe.values().len();
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_5)
                .h(px(56.0))
                .px(SPACE_8)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Layers, ICON_LG, theme.accent))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(recipe.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(format!(
                                    "{} · updated {}",
                                    recipe.device_id,
                                    local_time(recipe.updated_at, offset)
                                )),
                        ),
                )
                .child(pill(format!("v{}", recipe.version), theme.accent)),
        )
        .child(
            div()
                .id("recipe-sheet-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .overflow_y_scroll()
                .child(field("Name", name, cx))
                .child(
                    div()
                        .grid()
                        .grid_cols(2)
                        .gap(SPACE_6)
                        .child(field("Category", category, cx))
                        .child(field("Description", description, cx)),
                )
                .children(error.map(|error| note_line(Icon::CircleAlert, error, theme.danger)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .pt(SPACE_4)
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child(div().flex_1().child(format!("VALUES · {count}")))
                        .child(div().w(px(110.0)).text_right().child("SAVED"))
                        .child(div().w(px(130.0)).text_right().child("NOW")),
                )
                .children(value_rows)
                .when(count == 0, |this| {
                    this.child(note_line(
                        Icon::Info,
                        "No values: the device had none cached when it was saved.".into(),
                        theme.fg_muted,
                    ))
                }),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(56.0))
                .px(SPACE_8)
                .border_t_1()
                .border_color(theme.border)
                .child({
                    let target = recipe.clone();
                    Button::new("recipe-sheet-delete", "Delete")
                        .danger()
                        .compact()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_delete(console, target.clone(), window, cx);
                        }))
                })
                .child({
                    let target = recipe.clone();
                    Button::new("recipe-sheet-load", "Load…")
                        .compact()
                        .icon(Icon::Upload)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_load(console, target.clone(), window, cx);
                        }))
                })
                .child(div().flex_1())
                .child(
                    Button::new("recipe-sheet-cancel", "Close")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().sheet = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new(
                        "recipe-sheet-save",
                        if sending { "Saving…" } else { "Save" },
                    )
                    .primary()
                    .icon(Icon::Check)
                    .disabled(sending)
                    .on_click(
                        cx.listener(|console, _, window, cx| submit_sheet(console, window, cx)),
                    ),
                ),
        )
        .into_any_element()
}

// ── Load ─────────────────────────────────────────────────────────────────

fn ask_load(console: &mut Console, recipe: Recipe, window: &mut Window, cx: &mut Context<Console>) {
    if needs_operator(console, cx) {
        return;
    }
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.sheet = None;
    state.confirm_load = Some((recipe, focus.clone(), false));
    let overlay = Overlay::modal(LOAD, px(440.0), load_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn load_confirmed(console: &mut Console, cx: &mut Context<Console>) {
    let Some((recipe, _, sending)) = console.state::<State>().confirm_load.as_mut() else {
        return;
    };
    if *sending {
        return;
    }
    *sending = true;
    let (id, name, device) = (recipe.id, recipe.name.clone(), recipe.device_id.clone());
    console.call(
        CallKey::new("data.recipes.load").with(id),
        Call::new(move |api| async move {
            let load: RecipeLoad = api
                .send_empty(Method::POST, &insights::load_path(id))
                .await?;
            anyhow::Ok((name, device, load))
        }),
    );
    cx.notify();
}

fn load_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some((recipe, focus, sending)) = console.state::<State>().confirm_load.clone() else {
        return div().into_any_element();
    };
    let count = recipe.values().len();
    let own = focus.clone();
    div()
        .track_focus(&focus)
        .on_key_down(cx.listener(move |console, event: &KeyDownEvent, window, cx| {
            // Enter on the Cancel button is that button's, not a load.
            if event.keystroke.key == "enter" && own.is_focused(window) {
                load_confirmed(console, cx);
                cx.stop_propagation();
            }
        }))
        .child(dialog(
            format!("Load {} to {}?", recipe.name, recipe.device_id),
            format!(
                "Writes its {count} values through the command queue, one command each (↵ loads, Esc keeps)."
            ),
            vec![
                note_line(
                    Icon::Info,
                    "Values without a data source of the same key on the device are skipped by the server."
                        .into(),
                    theme.fg_muted,
                )
                .into_any_element(),
            ],
            vec![
                Button::new("recipe-load-cancel", "Cancel")
                    .ghost()
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.state::<State>().confirm_load = None;
                        console.close_overlay(window, cx);
                    }))
                    .into_any_element(),
                Button::new("recipe-load-confirm", if sending { "Loading…" } else { "Load" })
                    .primary()
                    .icon(Icon::Upload)
                    .disabled(sending)
                    .on_click(cx.listener(|console, _, _, cx| load_confirmed(console, cx)))
                    .into_any_element(),
            ],
            cx,
        ))
        .into_any_element()
}

/// Retry a failed command of the load: write its value again (`POST
/// /api/v1/tags/{id}/write`, as the web client does).
fn retry(console: &mut Console, id: String, cx: &mut Context<Console>) {
    if needs_operator(console, cx) {
        return;
    }
    let Some(run) = console.state::<State>().load.as_mut() else {
        return;
    };
    let Some(command) = run.commands.get(&id).cloned() else {
        return;
    };
    if run.retrying.contains(&id) {
        return;
    }
    let Ok(value) = command.value.trim().parse::<f64>() else {
        run.errors
            .insert(id, format!("“{}” is not a number", command.value));
        cx.notify();
        return;
    };
    run.retrying.push(id.clone());
    run.errors.remove(&id);
    let path = format!("/api/v1/tags/{}/write", command.data_source_id);
    let policy = if command.policy.is_empty() {
        "immediate".to_owned()
    } else {
        command.policy.clone()
    };
    console.call(
        CallKey::new("data.recipes.retry"),
        Call::new(move |api| async move {
            let accepted = api
                .send_json::<_, WriteAccepted>(
                    Method::POST,
                    &path,
                    &json!({ "value": value, "policy": policy }),
                )
                .await
                .map_err(|error| error.to_string());
            anyhow::Ok((id, accepted))
        }),
    );
    cx.notify();
}

/// The load's commands on the Commands page (its Active list).
fn open_commands(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<commands::State>().tab = commands::Tab::Active;
    console.show(Page::Commands, window, cx);
}

// ── Delete ───────────────────────────────────────────────────────────────

fn ask_delete(
    console: &mut Console,
    recipe: Recipe,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if needs_operator(console, cx) {
        return;
    }
    let focus = cx.focus_handle();
    let state = console.state::<State>();
    state.sheet = None;
    state.confirm_delete = Some((recipe, focus.clone()));
    let overlay = Overlay::modal(DELETE, px(420.0), delete_dialog, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some((recipe, _)) = console.state::<State>().confirm_delete.take() else {
        return;
    };
    console.close_overlay(window, cx);
    console.call(
        CallKey::new("data.recipes.delete").with(recipe.id),
        Call::send_empty::<IgnoredAny>(Method::DELETE, insights::recipe_path(recipe.id)),
    );
}

fn delete_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let Some((recipe, focus)) = console.state::<State>().confirm_delete.clone() else {
        return div().into_any_element();
    };
    let own = focus.clone();
    div()
        .track_focus(&focus)
        .on_key_down(cx.listener(move |console, event: &KeyDownEvent, window, cx| {
            // Enter on the Keep button is that button's, not a delete.
            if event.keystroke.key == "enter" && own.is_focused(window) {
                delete_confirmed(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(dialog(
            format!("Delete {}?", recipe.name),
            format!(
                "Its {} saved values of {} go with it. This cannot be undone (↵ deletes, Esc keeps).",
                recipe.values().len(),
                recipe.device_id
            ),
            Vec::new(),
            vec![
                Button::new("recipe-delete-keep", "Keep")
                    .ghost()
                    .on_click(cx.listener(|console, _, window, cx| {
                        console.state::<State>().confirm_delete = None;
                        console.close_overlay(window, cx);
                    }))
                    .into_any_element(),
                Button::new("recipe-delete-confirm", "Delete")
                    .danger()
                    .icon(Icon::Trash)
                    .on_click(cx.listener(|console, _, window, cx| {
                        delete_confirmed(console, window, cx);
                    }))
                    .into_any_element(),
            ],
            cx,
        ))
        .into_any_element()
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn activate_row(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(recipe) = rows(console, cx).into_iter().nth(row) {
        open_sheet(console, recipe, window, cx);
    }
}

const DEVICE_W: f32 = 120.0;
const CATEGORY_W: f32 = 110.0;
const VERSION_W: f32 = 48.0;
const VALUES_W: f32 = 56.0;
const UPDATED_W: f32 = 96.0;
const ACTIONS_W: f32 = 96.0;

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_search(console, window, cx);
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let options = devices(console);
    let shown_rows = Rc::new(rows(console, cx));
    let state = console.state::<State>();
    let (device, category) = (state.device.clone(), state.category.clone());
    let search_field = state.search.clone();
    let list = state.list.clone();
    let category_options: Vec<SharedString> = match &list {
        Some(Ok(list)) => categories(list).into_iter().map(Into::into).collect(),
        _ => Vec::new(),
    };
    let total = match &list {
        Some(Ok(list)) => list.len(),
        _ => 0,
    };
    let width = f32::from(window.viewport_size().width - sidebar_width());
    let wide = width > 980.0;

    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(220.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        Button::new("recipe-new", "Save current as recipe")
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(|console, _, window, cx| open_new(console, window, cx)))
            .into_any_element(),
        IconButton::new("recipe-refresh", Icon::RefreshCw)
            .tooltip("Fetch again  R")
            .on_click(cx.listener(|console, _, _, cx| {
                fetch(console);
                cx.notify();
            }))
            .into_any_element(),
    ];
    let subtitle = format!(
        "{} of {total} recipes · ↵ open · N new · L load · ⌫ delete · , . device · C category",
        shown_rows.len()
    );

    let filters = div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_4)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child("DEVICE"),
        )
        .child(chips(
            "recipe-device",
            &options,
            insights_kit::text(&device),
            Some("All devices"),
            pick_device,
            cx,
        ))
        .when(!category_options.is_empty(), |this| {
            this.child(
                div()
                    .pl(SPACE_6)
                    .text_size(FONT_CAPTION)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_dim)
                    .child("CATEGORY"),
            )
            .child(chips(
                "recipe-category",
                &category_options,
                category.as_deref(),
                Some("All"),
                pick_category,
                cx,
            ))
        });

    let load = load_card(console, cx);

    let shown = shown_rows.len();
    let keys = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate_row,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_8)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(div().flex_1().min_w_0().child("NAME"))
        .child(div().w(px(DEVICE_W)).flex_none().child("DEVICE"))
        .child(div().w(px(CATEGORY_W)).flex_none().child("CATEGORY"))
        .child(div().w(px(VERSION_W)).flex_none().child("VER"))
        .child(
            div()
                .w(px(VALUES_W))
                .flex_none()
                .text_right()
                .child("VALUES"),
        )
        .when(wide, |this| {
            this.child(div().flex_1().min_w_0().child("DESCRIPTION"))
        })
        .child(div().w(px(UPDATED_W)).flex_none().child("UPDATED"))
        .child(div().w(px(ACTIONS_W)).flex_none());
    let body = match &list {
        None => notice("Fetching the recipes…", false, &theme),
        Some(Err(error)) => notice(format!("Recipes: {error}"), true, &theme),
        Some(Ok(_)) if shown == 0 => notice(
            if total == 0 {
                "No recipes yet: save a device's current values as one (N)."
            } else {
                "No recipe matches the filters."
            },
            false,
            &theme,
        ),
        Some(Ok(_)) => uniform_list(
            "recipes",
            shown + tail_rows(shown, ROW_HEIGHT_PX),
            cx.processor(
                move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|row| match shown_rows.get(row) {
                            None => tail_row(ROW_HEIGHT_PX),
                            Some(recipe) => {
                                recipe_row(recipe, row, cursor, focused, wide, offset, &theme, cx)
                            }
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element(),
    };

    div()
        .key_context("Recipes")
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &RecipesNew, window, cx| {
            open_new(console, window, cx);
        }))
        .on_action(cx.listener(|console, _: &RecipesLoad, window, cx| {
            if let Some(recipe) = cursor_recipe(console, cx) {
                ask_load(console, recipe, window, cx);
            }
        }))
        .on_action(cx.listener(|console, _: &RecipesDelete, window, cx| {
            if let Some(recipe) = cursor_recipe(console, cx) {
                ask_delete(console, recipe, window, cx);
            }
        }))
        .on_action(cx.listener(|console, _: &RecipesPreviousDevice, _, cx| {
            step_device(console, false, cx);
        }))
        .on_action(cx.listener(|console, _: &RecipesNextDevice, _, cx| {
            step_device(console, true, cx);
        }))
        .on_action(cx.listener(|console, _: &RecipesNextCategory, _, cx| {
            next_category(console, cx);
        }))
        .on_action(cx.listener(|console, _: &RecipesRefresh, _, cx| {
            fetch(console);
            cx.notify();
        }))
        .child(page_header("Recipes", subtitle, controls, cx))
        .child(filters)
        .children(load)
        .child(header)
        .child(keys.flex_1().min_h_0().child(body))
        .into_any_element()
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn recipe_row(
    recipe: &Recipe,
    row: usize,
    cursor: Option<usize>,
    focused: bool,
    wide: bool,
    offset: time::UtcOffset,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let current = cursor == Some(row);
    let hover = theme.hover;
    let dim = |text: Option<&str>| text.map_or_else(|| "—".to_owned(), str::to_owned);
    let (load_target, edit_target, delete_target) =
        (recipe.clone(), recipe.clone(), recipe.clone());
    div()
        .id(("recipe-row", row))
        .group("recipe-row")
        .relative()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT_PX))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .text_size(FONT_SMALL)
        .cursor_pointer()
        .when(current, |this| this.bg(theme.selected))
        .when(!current, |this| this.hover(move |style| style.bg(hover)))
        .on_click(
            cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                console.list(LIST, cx).select(row);
                if event.click_count() >= 2 {
                    open_sheet(console, edit_target.clone(), window, cx);
                } else {
                    let focus = console.list(LIST, cx).focus.clone();
                    window.focus(&focus);
                }
                cx.notify();
            }),
        )
        .when(focused && current, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_BODY)
                .font_weight(FontWeight::MEDIUM)
                .child(recipe.name.clone()),
        )
        .child(
            div()
                .w(px(DEVICE_W))
                .flex_none()
                .truncate()
                .font_family(MONO)
                .text_color(theme.fg_muted)
                .child(recipe.device_id.clone()),
        )
        .child(
            div()
                .w(px(CATEGORY_W))
                .flex_none()
                .truncate()
                .text_color(if recipe.category().is_some() {
                    theme.fg
                } else {
                    theme.fg_dim
                })
                .child(dim(recipe.category())),
        )
        .child(
            div()
                .w(px(VERSION_W))
                .flex_none()
                .flex()
                .child(pill(format!("v{}", recipe.version), theme.accent)),
        )
        .child(
            div()
                .w(px(VALUES_W))
                .flex_none()
                .text_right()
                .font_family(MONO)
                .text_color(theme.fg_muted)
                .child(recipe.values().len().to_string()),
        )
        .when(wide, |this| {
            this.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.fg_muted)
                    .child(dim(recipe.description())),
            )
        })
        .child(
            div()
                .w(px(UPDATED_W))
                .flex_none()
                .font_family(MONO)
                .text_color(theme.fg_muted)
                .child(local_time(recipe.updated_at, offset)),
        )
        .child(
            div()
                .w(px(ACTIONS_W))
                .flex_none()
                .flex()
                .justify_end()
                .gap(SPACE_1)
                .opacity(if current { 1.0 } else { 0.0 })
                .group_hover("recipe-row", |style| style.opacity(1.0))
                .child(
                    IconButton::new(("recipe-load", row), Icon::Upload)
                        .tooltip("Load to the device…  L")
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_load(console, load_target.clone(), window, cx);
                        })),
                )
                .child(
                    IconButton::new(("recipe-open", row), Icon::Pencil)
                        .tooltip("Open  ↵")
                        .on_click(cx.listener({
                            let target = recipe.clone();
                            move |console, _, window, cx| {
                                open_sheet(console, target.clone(), window, cx);
                            }
                        })),
                )
                .child(
                    IconButton::new(("recipe-delete", row), Icon::Trash)
                        .tooltip("Delete  ⌫")
                        .color(theme.danger)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            ask_delete(console, delete_target.clone(), window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// The load being followed: its commands, how far they got, the way to
/// the Commands page.
#[allow(clippy::too_many_lines)]
fn load_card(console: &mut Console, cx: &mut Context<Console>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let run = console.state_ref::<State>()?.load.as_ref()?;
    let LoadTally {
        total,
        acked,
        failed,
        done,
    } = tally(&run.ids, &run.commands);
    let summary_color = if total > 0 && acked == total {
        theme.success
    } else if failed > 0 {
        theme.danger
    } else {
        theme.fg
    };
    let rows: Vec<AnyElement> = run
        .ids
        .iter()
        .enumerate()
        .map(|(row, id)| {
            let command = run.commands.get(id);
            let error = run
                .errors
                .get(id)
                .cloned()
                .or_else(|| command.and_then(|command| command.error_message.clone()));
            let retrying = run.retrying.contains(id);
            let tag = command.and_then(|command| {
                console
                    .store
                    .position(command.data_source_id)
                    .map(|at| console.store.tags[at].label.clone())
            });
            let status = command.map_or("…", |command| command.status.as_str());
            let failed = status == "failed";
            let target = id.clone();
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .h(px(28.0))
                .px(SPACE_7)
                .border_t_1()
                .border_color(theme.border.alpha(0.6))
                .text_size(FONT_SMALL)
                .child(div().w(px(80.0)).flex_none().flex().child(pill(
                    status.to_owned(),
                    command_status_color(status, &theme),
                )))
                .child(
                    div()
                        .w(px(150.0))
                        .flex_none()
                        .truncate()
                        .child(match (&tag, command) {
                            (Some(tag), _) => tag.to_string(),
                            (None, Some(command)) => format!("Tag #{}", command.data_source_id),
                            (None, None) => "…".to_owned(),
                        }),
                )
                .child(
                    div()
                        .w(px(90.0))
                        .flex_none()
                        .font_family(MONO)
                        .text_color(theme.accent)
                        .child(command.map_or(String::new(), |command| command.value.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_color(if error.is_some() {
                            theme.danger
                        } else {
                            theme.fg_dim
                        })
                        .child(error.unwrap_or_else(|| id.clone())),
                )
                .when(failed, |this| {
                    this.child(
                        Button::new(
                            ("recipe-retry", row),
                            if retrying { "Retrying…" } else { "Retry" },
                        )
                        .compact()
                        .disabled(retrying || !operator)
                        .on_click(cx.listener(move |console, _, _, cx| {
                            retry(console, target.clone(), cx);
                        })),
                    )
                })
                .into_any_element()
        })
        .collect();
    let message = run.message.clone();
    let title = format!("Loading {} to {}", run.recipe, run.device);
    Some(
        div()
            .flex_none()
            .px(SPACE_9)
            .pt(SPACE_6)
            .pb(SPACE_4)
            .child(
                card(&theme)
                    .max_h(px(260.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(SPACE_4)
                            .h(px(40.0))
                            .px(SPACE_7)
                            .child(vt_ui::icon(
                                if done { Icon::CircleCheck } else { Icon::Send },
                                ICON_MD,
                                summary_color,
                            ))
                            .child(
                                div()
                                    .text_size(FONT_EMPHASIS)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(summary_color)
                                    .child(if total == 0 {
                                        message
                                    } else {
                                        format!(
                                            "{acked}/{total} written{}",
                                            if failed > 0 {
                                                format!(" · {failed} failed")
                                            } else {
                                                String::new()
                                            }
                                        )
                                    }),
                            )
                            .when(!done && total > 0, |this| {
                                this.child(
                                    div()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_dim)
                                        .child("following every 2 s"),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                Button::new("recipe-load-commands", "Open in Commands")
                                    .compact()
                                    .icon(Icon::Send)
                                    .disabled(total == 0)
                                    .on_click(cx.listener(|console, _, window, cx| {
                                        open_commands(console, window, cx);
                                    })),
                            )
                            .child(
                                IconButton::new("recipe-load-close", Icon::X)
                                    .tooltip(if done || total == 0 {
                                        "Close"
                                    } else {
                                        "Stop following (the commands carry on)"
                                    })
                                    .on_click(cx.listener(|console, _, _, cx| {
                                        console.state::<State>().load = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(total == 0, |this| {
                        this.child(
                            div().px(SPACE_7).pb(SPACE_5).child(note_line(
                                Icon::Info,
                                "No value matched a data source of this device's keys, so nothing was queued."
                                    .into(),
                                theme.fg_muted,
                            )),
                        )
                    })
                    .child(
                        div()
                            .id("recipe-load-rows")
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .children(rows),
                    ),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::insights_kit::testing::{open, sent, signed_in};
    use super::super::{SECTIONS, visible};
    use crate::console::pages::Role;

    fn summer() -> Recipe {
        Recipe {
            id: 4,
            name: "Summer".into(),
            device_id: "plts_1".into(),
            values: r#"{"p": 50.0, "q": 2.5}"#.into(),
            version: 1,
            ..Recipe::default()
        }
    }

    #[gpui::test]
    fn a_recipe_opens_loads_and_its_commands_are_followed(cx: &mut gpui::TestAppContext) {
        let mut h = signed_in(cx, "demo");
        open(&mut h, ID);
        let list = sent(&h.calls(), "data.recipes.list").expect("the recipes");
        h.reply(list, Reply::ok(vec![summer()]));
        h.draw();
        // Enter opens it beside the live values; Esc closes.
        h.keys("down enter");
        assert!(h.update(|console, _, _| console.overlay_is(SHEET)));
        h.update(|console, _, _| {
            let values = &console.state::<State>().sheet.as_ref().expect("open").values;
            assert_eq!(values.len(), 2);
            assert!(values.iter().all(|(_, _, at)| at.is_some()), "each finds its tag");
        });
        h.keys("escape");
        assert!(!h.update(|console, _, _| console.overlay_is(SHEET)));
        // L asks, Enter loads.
        h.draw();
        h.keys("l");
        assert!(h.update(|console, _, _| console.overlay_is(LOAD)));
        h.draw();
        h.keys("enter");
        let load = sent(&h.calls(), "data.recipes.load").expect("the load is sent");
        assert_eq!(load.arg, 4);
        h.reply(
            load,
            Reply::ok((
                "Summer".to_owned(),
                "plts_1".to_owned(),
                RecipeLoad {
                    ok: true,
                    command_ids: vec!["cmd_a".into(), "cmd_b".into()],
                    message: "Recipe load queued: 2 commands".into(),
                },
            )),
        );
        let follow = sent(&h.calls(), "data.recipes.commands").expect("its commands");
        let command = |id: &str, status: &str| {
            (
                id.to_owned(),
                QueuedCommand {
                    id: id.into(),
                    status: status.into(),
                    ..QueuedCommand::default()
                },
            )
        };
        h.reply(
            follow,
            Reply::ok(vec![command("cmd_a", "acked"), command("cmd_b", "failed")]),
        );
        h.draw();
        let tallied = h.update(|console, _, _| {
            let run = console.state::<State>().load.as_ref().expect("followed");
            tally(&run.ids, &run.commands)
        });
        assert_eq!((tallied.acked, tallied.failed, tallied.done), (1, 1, true));
        // The ids lead to the Commands page.
        h.update(open_commands);
        assert_eq!(h.update(|console, _, _| console.page), Page::Commands);
    }

    #[gpui::test]
    fn recipes_are_for_operators_only(cx: &mut gpui::TestAppContext) {
        assert!(
            !visible(SECTIONS, Some(Role::Any))
                .iter()
                .any(|spec| spec.id == ID),
            "the server's recipe routes are all operator routes"
        );
        let mut h = signed_in(cx, "viewer");
        h.update(|console, window, cx| {
            open_new(console, window, cx);
            ask_load(console, summer(), window, cx);
            ask_delete(console, summer(), window, cx);
        });
        assert!(h.update(|console, _, _| console.overlay.is_none()));
        assert!(h.calls().is_empty(), "nothing is asked of the server");
    }


    #[test]
    fn a_retry_replaces_the_failed_command_it_retries() {
        let mut run = LoadRun {
            recipe: "Summer".into(),
            device: "sma_inv_1".into(),
            message: String::new(),
            ids: vec!["cmd_a".into(), "cmd_b".into()],
            commands: HashMap::new(),
            seq: 0,
            retrying: vec!["cmd_b".into()],
            errors: HashMap::new(),
        };
        run.retried("cmd_b", "cmd_c".into());
        assert_eq!(run.ids, ["cmd_a", "cmd_c"]);
        assert!(run.retrying.is_empty());
        run.retrying.push("cmd_a".into());
        run.failed_retry("cmd_a", "refused".into());
        assert!(run.retrying.is_empty());
        assert_eq!(run.errors["cmd_a"], "refused");
        run.retried("gone", "cmd_d".into());
        assert_eq!(run.ids, ["cmd_a", "cmd_c"], "an unknown id changes nothing");
    }
}
