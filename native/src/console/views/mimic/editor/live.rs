//! The editor in a live session ([`collab`]): joins when the Designer
//! opens, pushes the editor's changes into the shared document every
//! [`collab::BATCH`], shows remote edits, undo of the console's own edits,
//! ⌘S as a write of the session's edits, the people in the bar and their
//! cursors and selections on the stage. An older server, or collaboration
//! off, leaves the editor saving whole layouts as before.

use std::collections::HashMap;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Hsla, Task, Window, div, px, rgb};
use voltrus_scada_native::designer::collab::{self, Phase, Session, Viewport};
use voltrus_scada_native::mimic::edit::Rect;
use voltrus_scada_native::mimic::{Camera, Layout};
use voltrus_scada_native::runtime::{Command, Link};
use voltrus_scada_native::ws_types::ServerMessage;
use vt_ui::theme::*;

use super::{Editor, State, edits, edits_ref, inspector};
use crate::console::Console;

/// The session of an open editor, and the task that drives it.
pub(super) struct Live {
    pub(super) session: Session,
    _task: Task<()>,
}

/// The names of the tags a layout shows (a widget's saved name may be its
/// tag's).
fn tag_names(console: &Console, layout: &Layout) -> HashMap<i64, String> {
    layout
        .tags()
        .into_iter()
        .filter_map(|id| console.store.get(id).map(|t| (id, t.info.name.clone())))
        .collect()
}

/// Join the session of the screen just opened (not in a stress run).
pub(super) fn start(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    let Some(editor) = edits(console) else {
        return;
    };
    let (session, join) = Session::join(&editor.screen, editor.history.present(), Instant::now());
    let task = cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(collab::BATCH).await;
            match this.update_in(cx, |console, _, cx| tick(console, cx)) {
                Ok(true) => {}
                _ => break,
            }
        }
    });
    editor.live = Some(Live {
        session,
        _task: task,
    });
    console.send(Command::Design(join));
}

/// The editor's changes into the document; what is due goes out. Whether
/// the session is still on.
pub(super) fn tick(console: &mut Console, cx: &mut Context<Console>) -> bool {
    let now = Instant::now();
    let names = edits_ref(console)
        .map(|editor| tag_names(console, editor.history.present()))
        .unwrap_or_default();
    let viewport = viewport(console, now);
    let Some(editor) = edits(console) else {
        return false;
    };
    let Some(live) = editor.live.as_mut() else {
        return false;
    };
    if live.session.expire(now) {
        cx.notify();
        return false;
    }
    if live.session.off() {
        return false;
    }
    let changed = push(editor, &names);
    let Some(live) = editor.live.as_mut() else {
        return false;
    };
    let cursor = editor.pointer;
    let selection = editor.selection.clone();
    live.session.set_presence(cursor, &selection, viewport);
    let frames = live.session.outgoing(now);
    for frame in frames {
        console.send(Command::Design(frame));
    }
    if changed {
        cx.notify();
    }
    true
}

/// The stage centre and zoom in view.
fn viewport(console: &mut Console, now: Instant) -> Option<Viewport> {
    let state = console.state::<State>();
    let (camera, _) = state.camera_at(now);
    let drawn = state.drawn.get()?;
    let camera: Camera = camera.unwrap_or_else(|| super::super::fit_of(&drawn));
    let (x, y) = camera.to_stage(
        f64::from(f32::from(drawn.bounds.size.width)) / 2.0,
        f64::from(f32::from(drawn.bounds.size.height)) / 2.0,
    );
    Some(Viewport {
        x: x.round(),
        y: y.round(),
        scale: camera.zoom,
    })
}

/// The editor's edits since the last push, into the document. Whether
/// the editor's layout was replaced.
fn push(editor: &mut Editor, names: &HashMap<i64, String>) -> bool {
    let Some(live) = editor.live.as_mut() else {
        return false;
    };
    if !live.session.active() || editor.history.present() == live.session.layout() {
        return false;
    }
    let new_step = editor.history.stepped();
    let tag_name = |id: i64| names.get(&id).cloned();
    live.session
        .local(editor.history.present(), new_step, &tag_name);
    let layout = live.session.layout().clone();
    editor.history.rebase(layout);
    editor.prune();
    true
}

/// A `Design*` frame from the server.
pub(crate) fn on_server(
    console: &mut Console,
    message: &ServerMessage,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let names = edits_ref(console)
        .map(|editor| tag_names(console, editor.history.present()))
        .unwrap_or_default();
    let Some(editor) = edits(console) else {
        return;
    };
    // Edits not pushed yet go in first: the frame is applied on top.
    push(editor, &names);
    let Some(live) = editor.live.as_mut() else {
        return;
    };
    let tag_name = |id: i64| names.get(&id).cloned();
    let was_active = live.session.active();
    let reaction = live
        .session
        .on_server(message, editor.history.present(), &tag_name);
    let joined = !was_active && live.session.active();
    if let Some(layout) = reaction.layout {
        editor.history.rebase(layout);
        editor.prune();
    }
    for frame in reaction.send {
        console.send(Command::Design(frame));
    }
    if joined {
        console.show_toast("Live session: edits are shared as you make them", false, cx);
    }
    if let Some((text, error)) = reaction.toast {
        console.show_toast(text, error, cx);
    }
    inspector::sync(console, window, cx);
    cx.notify();
}

/// The link went down or came back.
pub(crate) fn on_link(console: &mut Console, link: &Link) {
    let Some(live) = edits(console).and_then(|editor| editor.live.as_mut()) else {
        return;
    };
    let frame = match link {
        Link::Live => live.session.reconnect(Instant::now()),
        _ => {
            live.session.offline();
            None
        }
    };
    if let Some(frame) = frame {
        console.send(Command::Design(frame));
    }
}

/// The running session, when edits go through it.
pub(super) fn active(editor: &Editor) -> bool {
    editor
        .live
        .as_ref()
        .is_some_and(|live| live.session.active())
}

/// Undo or redo in a session: the console's own edits only. Whether it
/// was the session's to do.
pub(super) fn undo(console: &mut Console, redo: bool) -> bool {
    let names = edits_ref(console)
        .map(|editor| tag_names(console, editor.history.present()))
        .unwrap_or_default();
    let Some(editor) = edits(console) else {
        return false;
    };
    if !active(editor) {
        return false;
    }
    push(editor, &names);
    let Some(live) = editor.live.as_mut() else {
        return false;
    };
    let layout = if redo {
        live.session.redo()
    } else {
        live.session.undo()
    };
    if let Some(layout) = layout {
        editor.history.rebase(layout);
        editor.prune();
    }
    true
}

/// ⌘S in a session: the server writes the session's edits to the draft
/// now. Whether it was the session's to do.
pub(super) fn save(console: &mut Console, cx: &mut Context<Console>) -> bool {
    let names = edits_ref(console)
        .map(|editor| tag_names(console, editor.history.present()))
        .unwrap_or_default();
    let Some(editor) = edits(console) else {
        return false;
    };
    if !active(editor) {
        return false;
    }
    push(editor, &names);
    let Some(live) = editor.live.as_mut() else {
        return false;
    };
    let mut frames = live.session.outgoing(Instant::now() + collab::BATCH);
    frames.push(live.session.flush_frame());
    for frame in frames {
        console.send(Command::Design(frame));
    }
    cx.notify();
    true
}

/// Leaving the Designer: out of the session (pending edits go first).
pub(super) fn leave(console: &mut Console) {
    let names = edits_ref(console)
        .map(|editor| tag_names(console, editor.history.present()))
        .unwrap_or_default();
    let Some(editor) = edits(console) else {
        return;
    };
    push(editor, &names);
    let Some(live) = editor.live.as_mut() else {
        return;
    };
    if live.session.off() {
        return;
    }
    let mut frames = live.session.outgoing(Instant::now() + collab::BATCH);
    frames.push(live.session.leave_frame());
    for frame in frames {
        console.send(Command::Design(frame));
    }
}

/// What the bar shows of the session.
pub(super) struct Bar {
    pub(super) status: String,
    /// Initials, name, colour of each other person.
    pub(super) peers: Vec<(String, String, Hsla)>,
    pub(super) can_undo: bool,
    pub(super) can_redo: bool,
}

pub(super) fn color(color: &str) -> Hsla {
    collab::color_rgb(color).map_or_else(|| rgb(0x0088_8888).into(), |c| rgb(c).into())
}

pub(super) fn bar(editor: &Editor) -> Option<Bar> {
    let live = editor.live.as_ref()?;
    let session = &live.session;
    let status = match &session.phase {
        Phase::Off(_) => return None,
        Phase::Joining { .. } => "Joining the live session…".to_owned(),
        Phase::Offline => "Offline · edits wait for the link".to_owned(),
        Phase::Live if !session.active() => "Joining the live session…".to_owned(),
        Phase::Live => {
            let others = session.peers.len();
            let pending = if session.unsent() { " · sending" } else { "" };
            if others == 0 {
                format!("Live · only you{pending}")
            } else {
                let mut names: Vec<&str> =
                    session.peers.values().map(|p| p.name.as_str()).collect();
                names.dedup();
                format!("Live · with {}{pending}", names.join(", "))
            }
        }
    };
    let mut peers: Vec<(String, String, Hsla)> = session
        .peers
        .values()
        .map(|p| (collab::initials(&p.name), p.name.clone(), color(&p.color)))
        .collect();
    peers.dedup_by(|a, b| a.1 == b.1);
    Some(Bar {
        status,
        peers,
        can_undo: session.can_undo(),
        can_redo: session.can_redo(),
    })
}

/// The people in the bar: coloured initials (the status names them).
pub(super) fn avatars(peers: &[(String, String, Hsla)], theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.0))
        .children(peers.iter().enumerate().map(|(i, (initials, _, color))| {
            div()
                .id(("live-peer", i))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded_full()
                .bg(*color)
                .border_1()
                .border_color(theme.chrome)
                .text_size(FONT_CAPTION)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .child(initials.clone())
        }))
        .into_any_element()
}

/// One other person on the stage: colour, cursor, selected boxes.
pub(super) struct PeerMark {
    pub(super) color: Hsla,
    pub(super) name: String,
    pub(super) cursor: Option<(f64, f64)>,
    pub(super) boxes: Vec<Rect>,
}

pub(super) fn peer_marks(editor: &Editor) -> Vec<PeerMark> {
    let Some(live) = editor.live.as_ref().filter(|l| l.session.active()) else {
        return Vec::new();
    };
    let layout = editor.history.present();
    live.session
        .peers
        .values()
        .map(|p| PeerMark {
            color: color(&p.color),
            name: p.name.clone(),
            cursor: p.cursor,
            boxes: p
                .selection
                .iter()
                .filter_map(|id| layout.widget(id))
                .map(Rect::of)
                .collect(),
        })
        .collect()
}

/// The names beside the other people's cursors, over the stage.
pub(super) fn cursor_labels(marks: &[PeerMark], camera: Option<Camera>) -> Vec<AnyElement> {
    let Some(camera) = camera else {
        return Vec::new();
    };
    marks
        .iter()
        .filter_map(|m| {
            let (x, y) = m.cursor?;
            let (vx, vy) = camera.to_view(x, y);
            #[allow(clippy::cast_possible_truncation)]
            let at = (vx as f32 + 12.0, vy as f32 + 14.0);
            Some(
                div()
                    .absolute()
                    .left(px(at.0))
                    .top(px(at.1))
                    .px(SPACE_2)
                    .rounded(RADIUS_SM)
                    .bg(m.color)
                    .text_size(FONT_CAPTION)
                    .text_color(gpui::white())
                    .whitespace_nowrap()
                    .child(m.name.clone())
                    .into_any_element(),
            )
        })
        .collect()
}
