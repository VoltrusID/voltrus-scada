//! The flow canvas's geometry, free of GPUI: world and screen points, the
//! view's pan and zoom (zooming about the pointer), where a step's ports
//! sit, the wires' Bézier curves, hit tests and the web client's layout
//! (trigger, the process chain left to right, the outputs stacked after
//! it). Positions are the editor's own: the stored flow has none.

use voltrus_scada_native::api::eng_flows::{Category, End, FlowGraph, Wire};

/// A step's card, in world units.
pub(crate) const NODE_W: f32 = 180.0;
pub(crate) const NODE_H: f32 = 72.0;
/// The web client's gaps: between chain steps, and between stacked outputs.
const NODE_GAP: f32 = 48.0;
const OUTPUT_GAP: f32 = 16.0;
/// A port's radius on screen, and how far from it a press still takes it.
pub(crate) const PORT_R: f32 = 5.0;
pub(crate) const PORT_REACH: f32 = 10.0;
/// How near a wire a press selects it, on screen.
pub(crate) const WIRE_REACH: f32 = 6.0;
pub(crate) const MIN_ZOOM: f32 = 0.25;
pub(crate) const MAX_ZOOM: f32 = 2.5;
/// One zoom step (+ and −).
pub(crate) const ZOOM_STEP: f32 = 1.2;
/// Room around the steps when the view fits them.
const FIT_MARGIN: f32 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub(crate) fn scale(self, factor: f32) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }

    pub(crate) fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub origin: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub(crate) const fn new(origin: Vec2, size: Vec2) -> Self {
        Self { origin, size }
    }

    pub(crate) fn right(&self) -> f32 {
        self.origin.x + self.size.x
    }

    pub(crate) fn bottom(&self) -> f32 {
        self.origin.y + self.size.y
    }

    pub(crate) fn center(&self) -> Vec2 {
        self.origin + self.size.scale(0.5)
    }

    pub(crate) fn contains(&self, point: Vec2) -> bool {
        point.x >= self.origin.x
            && point.x <= self.right()
            && point.y >= self.origin.y
            && point.y <= self.bottom()
    }

    pub(crate) fn intersects(&self, other: &Self) -> bool {
        self.origin.x <= other.right()
            && other.origin.x <= self.right()
            && self.origin.y <= other.bottom()
            && other.origin.y <= self.bottom()
    }

    /// The smallest rectangle holding both.
    pub(crate) fn union(&self, other: &Self) -> Self {
        let left = self.origin.x.min(other.origin.x);
        let top = self.origin.y.min(other.origin.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(Vec2::new(left, top), Vec2::new(right - left, bottom - top))
    }

    /// The rectangle grown by `by` on every side.
    pub(crate) fn inflate(&self, by: f32) -> Self {
        Self::new(
            self.origin - Vec2::new(by, by),
            self.size + Vec2::new(2.0 * by, 2.0 * by),
        )
    }
}

/// A step's card at `at` (its top-left corner).
pub(crate) fn node_rect(at: Vec2) -> Rect {
    Rect::new(at, Vec2::new(NODE_W, NODE_H))
}

/// Where a wire enters a step: the middle of its left edge.
pub(crate) fn input_port(at: Vec2) -> Vec2 {
    Vec2::new(at.x, at.y + NODE_H / 2.0)
}

/// Where a wire leaves a step: the middle of its right edge.
pub(crate) fn output_port(at: Vec2) -> Vec2 {
    Vec2::new(at.x + NODE_W, at.y + NODE_H / 2.0)
}

/// Pan and zoom: `screen = world × zoom + pan`, screen points relative to
/// the canvas's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan: Vec2::default(),
            zoom: 1.0,
        }
    }
}

impl View {
    pub(crate) fn to_screen(self, world: Vec2) -> Vec2 {
        world.scale(self.zoom) + self.pan
    }

    pub(crate) fn to_world(self, screen: Vec2) -> Vec2 {
        (screen - self.pan).scale(1.0 / self.zoom)
    }

    /// Zoom by `factor` keeping the world point under `anchor` (a screen
    /// point: the pointer, or the canvas's centre) where it is.
    pub(crate) fn zoom_about(&self, anchor: Vec2, factor: f32) -> Self {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let world = self.to_world(anchor);
        Self {
            pan: anchor - world.scale(zoom),
            zoom,
        }
    }

    pub(crate) fn panned(&self, by: Vec2) -> Self {
        Self {
            pan: self.pan + by,
            zoom: self.zoom,
        }
    }

    /// The world area a canvas of `size` shows.
    pub(crate) fn visible(&self, size: Vec2) -> Rect {
        Rect::new(self.to_world(Vec2::default()), size.scale(1.0 / self.zoom))
    }

    /// `content` centred in a canvas of `size`, as large as fits up to
    /// 100 %.
    pub(crate) fn fit(content: Rect, size: Vec2) -> Self {
        let room = Vec2::new(
            (size.x - 2.0 * FIT_MARGIN).max(1.0),
            (size.y - 2.0 * FIT_MARGIN).max(1.0),
        );
        let zoom = (room.x / content.size.x.max(1.0))
            .min(room.y / content.size.y.max(1.0))
            .min(1.0)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Self {
            pan: size.scale(0.5) - content.center().scale(zoom),
            zoom,
        }
    }

    /// The least pan that brings `rect` (world) into a canvas of `size`.
    pub(crate) fn reveal(&self, rect: Rect, size: Vec2) -> Self {
        let margin = 24.0;
        let top_left = self.to_screen(rect.origin);
        let bottom_right = self.to_screen(Vec2::new(rect.right(), rect.bottom()));
        let shift = |low: f32, high: f32, extent: f32| {
            if low < margin {
                margin - low
            } else if high > extent - margin {
                (extent - margin - high).max(margin - low)
            } else {
                0.0
            }
        };
        self.panned(Vec2::new(
            shift(top_left.x, bottom_right.x, size.x),
            shift(top_left.y, bottom_right.y, size.y),
        ))
    }
}

/// A cubic Bézier curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bezier {
    pub from: Vec2,
    pub control_a: Vec2,
    pub control_b: Vec2,
    pub to: Vec2,
}

impl Bezier {
    pub(crate) fn at(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        self.from.scale(u * u * u)
            + self.control_a.scale(3.0 * u * u * t)
            + self.control_b.scale(3.0 * u * t * t)
            + self.to.scale(t * t * t)
    }

    /// The box holding the curve (its control points' hull).
    pub(crate) fn bounds(&self) -> Rect {
        let points = [self.from, self.control_a, self.control_b, self.to];
        let left = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let top = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let right = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        let bottom = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
        Rect::new(Vec2::new(left, top), Vec2::new(right - left, bottom - top))
    }

    /// How far `point` is from the curve (24 straight pieces).
    pub(crate) fn distance(&self, point: Vec2) -> f32 {
        const PIECES: u8 = 24;
        let mut best = f32::INFINITY;
        let mut previous = self.from;
        for step in 1..=PIECES {
            let next = self.at(f32::from(step) / f32::from(PIECES));
            best = best.min(segment_distance(point, previous, next));
            previous = next;
        }
        best
    }
}

fn segment_distance(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let length = along.x * along.x + along.y * along.y;
    if length <= f32::EPSILON {
        return (point - a).length();
    }
    let t = (((point.x - a.x) * along.x + (point.y - a.y) * along.y) / length).clamp(0.0, 1.0);
    (point - (a + along.scale(t))).length()
}

/// A wire from an output port to an input port: it leaves to the right
/// and arrives from the left, also when the target sits behind the source.
pub(crate) fn wire_curve(from: Vec2, to: Vec2) -> Bezier {
    let reach = ((to.x - from.x).abs() / 2.0).max(40.0);
    Bezier {
        from,
        control_a: Vec2::new(from.x + reach, from.y),
        control_b: Vec2::new(to.x - reach, to.y),
        to,
    }
}

/// The steps' positions: the trigger's, then one per step.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Layout {
    pub trigger: Vec2,
    pub nodes: Vec<Vec2>,
}

impl Layout {
    /// The web client's arrangement: the trigger, the process steps to its
    /// right in order, the outputs stacked after the chain, centred on it.
    pub(crate) fn arrange(graph: &FlowGraph) -> Self {
        let step = NODE_W + NODE_GAP;
        let mut nodes = vec![Vec2::default(); graph.nodes.len()];
        let mut x = step;
        for (index, at) in nodes.iter_mut().enumerate() {
            if graph.category(index) != Category::Output {
                *at = Vec2::new(x, 0.0);
                x += step;
            }
        }
        let outputs: Vec<usize> = (0..graph.nodes.len())
            .filter(|index| graph.category(*index) == Category::Output)
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let stack = outputs.len() as f32 * (NODE_H + OUTPUT_GAP) - OUTPUT_GAP;
        let top = -(stack - NODE_H) / 2.0;
        for (row, index) in outputs.into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let y = top + row as f32 * (NODE_H + OUTPUT_GAP);
            nodes[index] = Vec2::new(x, y);
        }
        Self {
            trigger: Vec2::default(),
            nodes,
        }
    }

    pub(crate) fn at(&self, end: End) -> Vec2 {
        match end {
            End::Trigger => self.trigger,
            End::Node(index) => self.nodes.get(index).copied().unwrap_or_default(),
        }
    }

    pub(crate) fn set(&mut self, end: End, at: Vec2) {
        match end {
            End::Trigger => self.trigger = at,
            End::Node(index) => {
                if let Some(slot) = self.nodes.get_mut(index) {
                    *slot = at;
                }
            }
        }
    }

    /// Follow a structural edit (`order`: each new position's old one);
    /// a new step goes to `new_at`.
    pub(crate) fn reorder(&mut self, order: &[Option<usize>], new_at: Vec2) {
        self.nodes = order
            .iter()
            .map(|before| {
                before
                    .and_then(|old| self.nodes.get(old).copied())
                    .unwrap_or(new_at)
            })
            .collect();
    }

    /// `near`, or the first place below it no card already starts at.
    pub(crate) fn free_spot(&self, near: Vec2) -> Vec2 {
        let taken = |spot: Vec2| {
            std::iter::once(&self.trigger)
                .chain(&self.nodes)
                .any(|at| (*at - spot).length() < 24.0)
        };
        let mut spot = near;
        for _ in 0..64 {
            if !taken(spot) {
                break;
            }
            spot.y += NODE_H + OUTPUT_GAP;
        }
        spot
    }

    /// Everything, for fitting the view.
    pub(crate) fn bounds(&self) -> Rect {
        self.nodes.iter().fold(node_rect(self.trigger), |all, at| {
            all.union(&node_rect(*at))
        })
    }

    /// The curve of `wire`, in world units.
    pub(crate) fn curve(&self, wire: Wire) -> Bezier {
        wire_curve(
            output_port(self.at(wire.from)),
            input_port(self.at(End::Node(wire.to))),
        )
    }
}

/// What a press on the canvas hits, topmost first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    /// A step's output port: a wire starts here.
    Output(End),
    /// A step's card (its input port included).
    Node(End),
    /// The wire into step `to`.
    Wire(usize),
    Empty,
}

/// Whether a step has an input and an output port: the trigger only
/// sends, outputs only receive.
pub(crate) fn ports(graph: &FlowGraph, end: End) -> (bool, bool) {
    match end {
        End::Trigger => (false, true),
        End::Node(index) => (true, graph.category(index) != Category::Output),
    }
}

/// The steps in drawing order (later ones on top).
pub(crate) fn ends(graph: &FlowGraph) -> impl DoubleEndedIterator<Item = End> {
    std::iter::once(End::Trigger).chain((0..graph.nodes.len()).map(End::Node))
}

/// What is at `world` with the view at `zoom` (reaches are on-screen
/// distances).
pub(crate) fn hit(graph: &FlowGraph, layout: &Layout, world: Vec2, zoom: f32) -> Hit {
    let port_reach = PORT_REACH / zoom;
    for end in ends(graph).rev() {
        let at = layout.at(end);
        if ports(graph, end).1 && (output_port(at) - world).length() <= port_reach {
            return Hit::Output(end);
        }
        if node_rect(at)
            .inflate(port_reach.min(PORT_R / zoom))
            .contains(world)
        {
            return Hit::Node(end);
        }
    }
    let wire_reach = WIRE_REACH / zoom;
    graph
        .wires()
        .into_iter()
        .filter(|wire| {
            layout
                .curve(*wire)
                .bounds()
                .inflate(wire_reach)
                .contains(world)
        })
        .map(|wire| (wire.to, layout.curve(wire).distance(world)))
        .filter(|(_, distance)| *distance <= wire_reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Hit::Empty, |(to, _)| Hit::Wire(to))
}

/// Where a wire dragged to `world` lands: a step that takes input (its
/// card or its input port), the topmost.
pub(crate) fn drop_target(
    graph: &FlowGraph,
    layout: &Layout,
    world: Vec2,
    zoom: f32,
) -> Option<usize> {
    let reach = PORT_REACH / zoom;
    (0..graph.nodes.len()).rev().find(|index| {
        let at = layout.nodes[*index];
        node_rect(at).contains(world) || (input_port(at) - world).length() <= reach
    })
}

/// The next (or previous) end after `from` for Tab: the trigger, then the
/// steps in order, round.
pub(crate) fn cycle(graph: &FlowGraph, from: Option<End>, forward: bool) -> End {
    let all: Vec<End> = ends(graph).collect();
    let at = from.and_then(|end| all.iter().position(|other| *other == end));
    let index = match (at, forward) {
        (None, true) => 0,
        (None, false) => all.len() - 1,
        (Some(at), true) => (at + 1) % all.len(),
        (Some(at), false) => (at + all.len() - 1) % all.len(),
    };
    all[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    use voltrus_scada_native::api::eng_flows::node_kind;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    fn graph() -> FlowGraph {
        let mut graph = FlowGraph::parse(r#"{"type":"timer","interval_ms":5000}"#, "[]").unwrap();
        graph.add(node_kind("filter").unwrap(), None, "f".into());
        graph.add(node_kind("expression").unwrap(), None, "e".into());
        graph.add(node_kind("alarm_out").unwrap(), None, "a".into());
        graph.add(node_kind("http_out").unwrap(), None, "h".into());
        graph
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let view = View {
            pan: Vec2::new(30.0, -20.0),
            zoom: 1.0,
        };
        let pointer = Vec2::new(400.0, 250.0);
        let under = view.to_world(pointer);
        let zoomed = view.zoom_about(pointer, 2.0);
        assert!((zoomed.zoom - 2.0).abs() < 1e-6);
        assert!(close(zoomed.to_screen(under), pointer));
        assert!(close(zoomed.to_world(pointer), under));
        // Clamped at both ends, still about the pointer.
        let far = view.zoom_about(pointer, 100.0);
        assert!((far.zoom - MAX_ZOOM).abs() < 1e-6);
        assert!(close(far.to_screen(under), pointer));
        assert!((view.zoom_about(pointer, 0.001).zoom - MIN_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn fitting_centres_the_steps_and_never_enlarges() {
        let content = Rect::new(Vec2::new(0.0, -50.0), Vec2::new(600.0, 172.0));
        let size = Vec2::new(1000.0, 600.0);
        let view = View::fit(content, size);
        assert!((view.zoom - 1.0).abs() < 1e-6, "fits at 100 %");
        assert!(close(view.to_screen(content.center()), size.scale(0.5)));
        let wide = Rect::new(Vec2::default(), Vec2::new(3500.0, 100.0));
        let view = View::fit(wide, size);
        assert!(view.zoom < 0.3);
        assert!(view.to_screen(wide.origin).x >= 47.0);
    }

    #[test]
    fn revealing_pans_only_as_far_as_needed() {
        let view = View::default();
        let size = Vec2::new(800.0, 600.0);
        let inside = node_rect(Vec2::new(100.0, 100.0));
        assert_eq!(view.reveal(inside, size), view);
        let right = node_rect(Vec2::new(900.0, 100.0));
        let moved = view.reveal(right, size);
        assert!(close(moved.pan, Vec2::new(800.0 - 24.0 - 1080.0, 0.0)));
        let left = node_rect(Vec2::new(-300.0, 100.0));
        assert!(close(view.reveal(left, size).pan, Vec2::new(324.0, 0.0)));
    }

    #[test]
    fn the_layout_is_the_web_clients() {
        let graph = graph();
        let layout = Layout::arrange(&graph);
        assert_eq!(layout.trigger, Vec2::default());
        assert_eq!(layout.nodes[0], Vec2::new(228.0, 0.0));
        assert_eq!(layout.nodes[1], Vec2::new(456.0, 0.0));
        // Two outputs stacked, centred on the chain.
        assert_eq!(layout.nodes[2], Vec2::new(684.0, -44.0));
        assert_eq!(layout.nodes[3], Vec2::new(684.0, 44.0));
        let bounds = layout.bounds();
        assert_eq!(bounds.origin, Vec2::new(0.0, -44.0));
        assert_eq!(bounds.size, Vec2::new(864.0, 160.0));
    }

    #[test]
    fn ports_sit_mid_edge_and_wires_leave_right_and_arrive_left() {
        let at = Vec2::new(10.0, 20.0);
        assert_eq!(input_port(at), Vec2::new(10.0, 56.0));
        assert_eq!(output_port(at), Vec2::new(190.0, 56.0));
        let curve = wire_curve(Vec2::new(0.0, 0.0), Vec2::new(-100.0, 50.0));
        assert!(curve.control_a.x > 0.0, "leaves to the right");
        assert!(curve.control_b.x < -100.0, "arrives from the left");
        assert!(close(curve.at(0.0), curve.from));
        assert!(close(curve.at(1.0), curve.to));
        assert!(curve.distance(curve.at(0.37)) < 0.5);
        assert!(curve.distance(Vec2::new(500.0, 500.0)) > 100.0);
    }

    #[test]
    fn hits_prefer_ports_then_cards_then_wires() {
        let graph = graph();
        let layout = Layout::arrange(&graph);
        let filter = layout.nodes[0];
        assert_eq!(
            hit(&graph, &layout, output_port(filter), 1.0),
            Hit::Output(End::Node(0))
        );
        assert_eq!(
            hit(&graph, &layout, filter + Vec2::new(40.0, 30.0), 1.0),
            Hit::Node(End::Node(0))
        );
        // The input port belongs to the card.
        assert_eq!(
            hit(&graph, &layout, input_port(filter), 1.0),
            Hit::Node(End::Node(0))
        );
        // Outputs have no output port.
        let alarm = layout.nodes[2];
        assert_eq!(
            hit(&graph, &layout, output_port(alarm), 1.0),
            Hit::Node(End::Node(2))
        );
        // Mid-gap between the trigger and the filter: the wire.
        let middle = layout
            .curve(Wire {
                from: End::Trigger,
                to: 0,
            })
            .at(0.5);
        assert_eq!(hit(&graph, &layout, middle, 1.0), Hit::Wire(0));
        assert_eq!(hit(&graph, &layout, Vec2::new(0.0, 900.0), 1.0), Hit::Empty);
        // Zoomed out, reaches grow in world units.
        let near = output_port(filter) + Vec2::new(0.0, 15.0);
        assert_ne!(hit(&graph, &layout, near, 1.0), Hit::Output(End::Node(0)));
        assert_eq!(hit(&graph, &layout, near, 0.5), Hit::Output(End::Node(0)));
        assert_eq!(
            drop_target(&graph, &layout, input_port(layout.nodes[1]), 1.0),
            Some(1)
        );
        assert_eq!(
            drop_target(&graph, &layout, Vec2::new(0.0, 30.0), 1.0),
            None,
            "the trigger takes no input"
        );
    }

    #[test]
    fn layouts_follow_edits_and_tab_goes_round() {
        let mut graph = graph();
        let mut layout = Layout::arrange(&graph);
        let moved = Vec2::new(5.0, 5.0);
        layout.set(End::Node(1), moved);
        let order = graph.connect(End::Trigger, 1).unwrap().unwrap();
        layout.reorder(&order, Vec2::default());
        assert_eq!(layout.nodes[0], moved, "the expression keeps its place");
        let (_, order) = graph.add(node_kind("lookup").unwrap(), None, "l".into());
        layout.reorder(&order, Vec2::new(1.0, 2.0));
        assert_eq!(layout.nodes.len(), 5);
        assert_eq!(layout.nodes[2], Vec2::new(1.0, 2.0));
        assert_eq!(cycle(&graph, None, true), End::Trigger);
        assert_eq!(cycle(&graph, None, false), End::Node(4));
        assert_eq!(cycle(&graph, Some(End::Node(4)), true), End::Trigger);
        assert_eq!(cycle(&graph, Some(End::Trigger), false), End::Node(4));
        assert_eq!(cycle(&graph, Some(End::Node(0)), true), End::Node(1));
    }

    #[test]
    fn new_steps_find_a_free_place() {
        let layout = Layout::arrange(&graph());
        let filter = layout.nodes[0];
        assert_eq!(
            layout.free_spot(filter),
            filter + Vec2::new(0.0, NODE_H + OUTPUT_GAP)
        );
        let empty = Vec2::new(5000.0, 5000.0);
        assert_eq!(layout.free_spot(empty), empty);
    }

    #[test]
    fn rectangles_meet_and_hold() {
        let a = Rect::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0));
        let b = Rect::new(Vec2::new(5.0, 5.0), Vec2::new(10.0, 10.0));
        let c = Rect::new(Vec2::new(20.0, 0.0), Vec2::new(1.0, 1.0));
        assert!(a.intersects(&b) && !a.intersects(&c));
        assert_eq!(a.union(&c).size, Vec2::new(21.0, 10.0));
        let view = View {
            pan: Vec2::new(-100.0, 0.0),
            zoom: 2.0,
        };
        let seen = view.visible(Vec2::new(400.0, 300.0));
        assert_eq!(seen.origin, Vec2::new(50.0, 0.0));
        assert_eq!(seen.size, Vec2::new(200.0, 150.0));
    }
}
