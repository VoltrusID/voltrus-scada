//! The Designer's arrange rules, ported from the web Designer's
//! `client/src/designer/model.ts`: groups, locks, align, distribute, the
//! marquee and what a tag dropped on the empty stage can become
//! (`dropChoices` in `Designer.tsx`). Each operation returns one [`Edit`]
//! (one undo step), or `None` when it would change nothing.
//!
//! Designer-only facts live in each widget's config, under the same keys
//! as the web's, so a screen arranged here opens arranged in the browser:
//!
//! - `config.group`: a group id shared by the members of a group;
//! - `config.locked`: `true` when the widget is not moved, resized,
//!   marquee-selected or deleted.

use serde_json::Value as Json;

use super::expr::js_round;
use crate::mimic::edit::{self, Edit, Placing, Rect};
use crate::mimic::{Layout, STAGE_H, STAGE_W, Widget, symbol_def};

pub use crate::mimic::edit::{GROUP_KEY, LOCKED_KEY, group_of, is_locked};

/// Where align puts the selection (`AlignMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    /// Centres on one vertical line (`hcenter`).
    Center,
    Right,
    Top,
    /// Centres on one horizontal line (`vmiddle`).
    Middle,
    Bottom,
}

impl Align {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Center,
        Self::Right,
        Self::Top,
        Self::Middle,
        Self::Bottom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Align left",
            Self::Center => "Align centres horizontally",
            Self::Right => "Align right",
            Self::Top => "Align top",
            Self::Middle => "Align middles vertically",
            Self::Bottom => "Align bottom",
        }
    }
}

/// Which way distribute spaces the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// The widgets `ids` names, in stacking order.
fn pick<'a>(layout: &'a Layout, ids: &'a [String]) -> impl Iterator<Item = &'a Widget> + 'a {
    layout.widgets.iter().filter(|w| ids.contains(&w.id))
}

/// `ids` with every other member of their groups, in stacking order
/// (`expandToGroups`): picking one member picks its whole group.
pub fn expand_to_groups(layout: &Layout, ids: &[String]) -> Vec<String> {
    let groups: Vec<&str> = pick(layout, ids).filter_map(group_of).collect();
    if groups.is_empty() {
        return ids.to_vec();
    }
    layout
        .widgets
        .iter()
        .filter(|w| ids.contains(&w.id) || group_of(w).is_some_and(|g| groups.contains(&g)))
        .map(|w| w.id.clone())
        .collect()
}

/// Whether two boxes overlap (`intersects`).
fn intersects(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// The widgets a marquee touches, their groups included; locked ones are
/// skipped (`marqueeSelect`).
pub fn marquee(layout: &Layout, band: Rect) -> Vec<String> {
    let touched: Vec<String> = layout
        .widgets
        .iter()
        .filter(|w| !is_locked(w) && intersects(&Rect::of(w), &band))
        .map(|w| w.id.clone())
        .collect();
    expand_to_groups(layout, &touched)
}

/// ⌘A: every unlocked widget, groups whole.
pub fn select_all(layout: &Layout) -> Vec<String> {
    let unlocked: Vec<String> = layout
        .widgets
        .iter()
        .filter(|w| !is_locked(w))
        .map(|w| w.id.clone())
        .collect();
    expand_to_groups(layout, &unlocked)
}

/// What a press on widget `id` selects: with ⇧ its group is added (or
/// taken out when all of it is selected already), else its group, unless
/// it is selected already (a press on the selection keeps it).
pub fn press_selection(
    layout: &Layout,
    selection: &[String],
    id: &str,
    additive: bool,
) -> Vec<String> {
    let group = expand_to_groups(layout, &[id.to_owned()]);
    if additive {
        if group.iter().all(|g| selection.contains(g)) {
            return selection
                .iter()
                .filter(|s| !group.contains(s))
                .cloned()
                .collect();
        }
        let mut next = selection.to_vec();
        next.extend(group.into_iter().filter(|g| !selection.contains(g)));
        return next;
    }
    if selection.iter().any(|s| s == id) {
        selection.to_vec()
    } else {
        group
    }
}

/// The widgets among `ids` that move: the unlocked ones.
pub fn movable(layout: &Layout, ids: &[String]) -> Vec<String> {
    pick(layout, ids)
        .filter(|w| !is_locked(w))
        .map(|w| w.id.clone())
        .collect()
}

fn moves(changes: Vec<(String, f64, f64)>) -> Option<Edit> {
    (!changes.is_empty()).then_some(Edit::Move(changes))
}

/// Align to the selection's bounding box (`alignPatches`); one widget has
/// nothing to align to. Locked widgets stay (they still count for the box).
#[allow(clippy::float_cmp)]
pub fn align(layout: &Layout, ids: &[String], mode: Align) -> Option<Edit> {
    let widgets: Vec<&Widget> = pick(layout, ids).collect();
    if widgets.len() < 2 {
        return None;
    }
    let b = edit::bounds(widgets.iter().copied())?;
    let mut out = Vec::new();
    for w in widgets {
        if is_locked(w) {
            continue;
        }
        let (mut x, mut y) = (w.x, w.y);
        match mode {
            Align::Left => x = b.x,
            Align::Right => x = b.x + b.width - w.width,
            Align::Center => x = js_round(b.x + (b.width - w.width) / 2.0),
            Align::Top => y = b.y,
            Align::Bottom => y = b.y + b.height - w.height,
            Align::Middle => y = js_round(b.y + (b.height - w.height) / 2.0),
        }
        if x != w.x || y != w.y {
            out.push((w.id.clone(), x, y));
        }
    }
    moves(out)
}

/// Equal gaps between neighbours, the first and last staying where they
/// are (`distributePatches`); needs three or more.
#[allow(clippy::float_cmp)]
pub fn distribute(layout: &Layout, ids: &[String], axis: Axis) -> Option<Edit> {
    let mut widgets: Vec<&Widget> = pick(layout, ids).collect();
    if widgets.len() < 3 {
        return None;
    }
    let pos = |w: &Widget| if axis == Axis::Horizontal { w.x } else { w.y };
    let size = |w: &Widget| {
        if axis == Axis::Horizontal {
            w.width
        } else {
            w.height
        }
    };
    // A stable sort, as JavaScript's.
    widgets.sort_by(|a, b| pos(a).total_cmp(&pos(b)));
    let first = widgets[0];
    let last = widgets[widgets.len() - 1];
    let span = pos(last) + size(last) - pos(first);
    let total: f64 = widgets.iter().map(|w| size(w)).sum();
    #[allow(clippy::cast_precision_loss)]
    let gap = (span - total) / (widgets.len() - 1) as f64;
    let mut at = pos(first) + size(first) + gap;
    let mut out = Vec::new();
    for w in &widgets[1..widgets.len() - 1] {
        let v = js_round(at);
        if v != pos(w) && !is_locked(w) {
            let (x, y) = if axis == Axis::Horizontal {
                (v, w.y)
            } else {
                (w.x, v)
            };
            out.push((w.id.clone(), x, y));
        }
        at += size(w) + gap;
    }
    moves(out)
}

fn config_edits(
    layout: &Layout,
    ids: &[String],
    key: &str,
    value: impl Fn(&Widget) -> Option<Json>,
) -> Option<Edit> {
    let edits: Vec<Edit> = pick(layout, ids)
        .filter_map(|w| {
            value(w).map(|value| Edit::Config {
                id: w.id.clone(),
                key: key.to_owned(),
                value,
            })
        })
        .collect();
    (!edits.is_empty()).then_some(Edit::Many(edits))
}

/// Put the selection in one new group `group_id` (two or more widgets);
/// members of other groups move into it (`groupPatches`).
pub fn group(layout: &Layout, ids: &[String], group_id: &str) -> Option<Edit> {
    if ids.len() < 2 || pick(layout, ids).count() < 2 {
        return None;
    }
    config_edits(layout, ids, GROUP_KEY, |w| {
        (group_of(w) != Some(group_id)).then(|| Json::from(group_id))
    })
}

/// Take the selection's widgets out of their groups (`ungroupPatches`).
pub fn ungroup(layout: &Layout, ids: &[String]) -> Option<Edit> {
    config_edits(layout, ids, GROUP_KEY, |w| group_of(w).map(|_| Json::Null))
}

/// Lock or unlock (`lockPatches`); an unlocked widget has no `locked` key.
pub fn lock(layout: &Layout, ids: &[String], locked: bool) -> Option<Edit> {
    config_edits(layout, ids, LOCKED_KEY, |w| {
        (is_locked(w) != locked).then(|| if locked { Json::Bool(true) } else { Json::Null })
    })
}

/// ⌘L: lock the selection, or unlock it when every widget in it is
/// locked already.
pub fn toggle_lock(layout: &Layout, ids: &[String]) -> Option<Edit> {
    let widgets: Vec<&Widget> = pick(layout, ids).collect();
    let all_locked = !widgets.is_empty() && widgets.iter().all(|w| is_locked(w));
    lock(layout, ids, !all_locked)
}

/// Delete: the unlocked widgets among `ids` (locked ones are kept).
pub fn deletable(layout: &Layout, ids: &[String]) -> Vec<String> {
    movable(layout, ids)
}

/// Move the unlocked widgets among `ids` by `(dx, dy)`, kept on the stage
/// together (`movePatches`).
pub fn nudge(layout: &Layout, ids: &[String], dx: f64, dy: f64) -> Option<Edit> {
    let ids = movable(layout, ids);
    let area = edit::bounds(pick(layout, &ids))?;
    let dx = dx.min(STAGE_W - area.right()).max(-area.x);
    let dy = dy.min(STAGE_H - area.bottom()).max(-area.y);
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    moves(
        pick(layout, &ids)
            .map(|w| (w.id.clone(), w.x + dx, w.y + dy))
            .collect(),
    )
}

// ── A tag dropped on the empty stage ─────────────────────────────────────

/// What the placement menu offers for a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub placing: Placing,
    pub label: &'static str,
}

const fn symbol(key: &'static str, label: &'static str) -> Choice {
    Choice {
        placing: Placing::Symbol(key),
        label,
    }
}

const fn widget(kind: &'static str, label: &'static str) -> Choice {
    Choice {
        placing: Placing::Widget(kind),
        label,
    }
}

/// The facts about a tag the menu goes by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TagShape<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub unit: &'a str,
    pub min: f64,
    pub max: f64,
}

impl TagShape<'_> {
    /// A digital tag: a range of at most one from zero up (`isDigital`).
    pub fn digital(&self) -> bool {
        self.max - self.min <= 1.0 && self.min >= 0.0
    }

    fn level(&self) -> bool {
        self.unit == "%"
            || format!("{} {}", self.key, self.name)
                .to_lowercase()
                .contains("level")
    }
}

/// What a dropped tag can become, best guess first (`dropChoices`): for a
/// digital tag equipment and indicators, for an analog one numbers,
/// vessels and charts (a level tag's tank first).
pub fn drop_choices(tag: Option<&TagShape<'_>>) -> Vec<Choice> {
    if tag.is_some_and(TagShape::digital) {
        return vec![
            symbol("isa_pump", "Pump (ISA-101)"),
            symbol("isa_valve", "Valve (ISA-101)"),
            symbol("isa_motor", "Motor (ISA-101)"),
            widget("status_light", "Status light"),
            widget("on_off", "On/off indicator"),
            widget("toggle", "Toggle (writes)"),
            widget("big_number", "Number"),
        ];
    }
    let mut out = vec![
        widget("big_number", "Number"),
        widget("indicator", "Value indicator"),
        symbol("isa_tank", "Tank (ISA-101)"),
        symbol("isa_control_valve", "Control valve (ISA-101)"),
        widget("bar", "Bar"),
        widget("gauge", "Gauge"),
        widget("trend", "Trend"),
        widget("slider", "Slider (writes)"),
    ];
    if tag.is_some_and(TagShape::level) {
        let tank = out.remove(2);
        out.insert(0, tank);
    }
    out
}

/// The component a pick from the placement menu adds (`pickDrop`):
/// centred on the drop point (on the 10-pixel grid, kept on the stage),
/// bound to the tag; a symbol is labelled with the tag's name, and one
/// that shows a state shows its status chip.
pub fn place_tag(
    choice: Choice,
    tag: i64,
    tag_name: Option<&str>,
    at: (f64, f64),
    id: String,
) -> Widget {
    let (width, height) = choice.placing.size();
    let x = js_round((at.0 - width / 2.0) / 10.0) * 10.0;
    let y = js_round((at.1 - height / 2.0) / 10.0) * 10.0;
    let mut widget = choice.placing.widget(id, x, y);
    widget.tags = vec![tag];
    if let Placing::Symbol(key) = choice.placing
        && let Some(name) = tag_name
    {
        widget.config.insert("label".into(), Json::from(name));
        if symbol_def(key).is_some_and(|def| def.binding == crate::mimic::Binding::State) {
            widget.config.insert("showStatus".into(), Json::Bool(true));
        }
    }
    widget
}
