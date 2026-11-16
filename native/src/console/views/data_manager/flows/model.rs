//! The flow canvas's geometry, free of GPUI: world and screen points, the
//! view's pan and zoom (zooming about the pointer), where a node's ports
//! sit (one input on the left, the outputs spread down the right), the
//! wires' Bézier curves and the hit tests.

use voltrus_scada_native::api::eng_flows::{Catalogue, End, FlowGraph};

/// A node's card width, in world units; its height grows with its ports.
pub(crate) const NODE_W: f32 = 190.0;
/// The least card height (three lines of text).
pub(crate) const NODE_H: f32 = 66.0;
/// Room per output port down a card's right edge.
const PORT_ROW: f32 = 22.0;
/// A port's radius on screen, and how far from it a press still takes it.
pub(crate) const PORT_R: f32 = 5.0;
pub(crate) const PORT_REACH: f32 = 10.0;
/// How near a wire a press selects it, on screen.
pub(crate) const WIRE_REACH: f32 = 6.0;
pub(crate) const MIN_ZOOM: f32 = 0.25;
pub(crate) const MAX_ZOOM: f32 = 2.5;
/// One zoom step (+ and −).
pub(crate) const ZOOM_STEP: f32 = 1.2;
/// Room around the nodes when the view fits them.
const FIT_MARGIN: f32 = 48.0;
/// The least zoom a fit chooses: nodes stay legible in a narrow canvas.
const FIT_LEAST: f32 = 0.7;
/// Moves land on this grid (world units).
pub(crate) const SNAP: f32 = 10.0;

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
    /// 100 %, but never smaller than [`FIT_LEAST`] so the steps stay
    /// readable: a flow wider than that starts at its left edge (the
    /// trigger), the rest a pan away.
    pub(crate) fn fit(content: Rect, size: Vec2) -> Self {
        let room = Vec2::new(
            (size.x - 2.0 * FIT_MARGIN).max(1.0),
            (size.y - 2.0 * FIT_MARGIN).max(1.0),
        );
        // FIT_LEAST and 1.0 both lie within MIN_ZOOM..=MAX_ZOOM.
        let zoom = (room.x / content.size.x.max(1.0))
            .min(room.y / content.size.y.max(1.0))
            .clamp(FIT_LEAST, 1.0);
        let mut pan = size.scale(0.5) - content.center().scale(zoom);
        if content.size.x * zoom > room.x {
            pan.x = FIT_MARGIN - content.origin.x * zoom;
        }
        if content.size.y * zoom > room.y {
            pan.y = FIT_MARGIN - content.origin.y * zoom;
        }
        Self { pan, zoom }
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

/// `at` on the snapping grid.
pub(crate) fn snap(at: Vec2) -> Vec2 {
    Vec2::new((at.x / SNAP).round() * SNAP, (at.y / SNAP).round() * SNAP)
}

/// A card's height with `outputs` ports.
pub(crate) fn node_height(outputs: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let ports = outputs as f32 * PORT_ROW + 12.0;
    ports.max(NODE_H)
}

/// One node's card as laid out: the trigger or a node, where, how tall,
/// with how many output ports and whether it takes input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Card {
    pub end: End,
    pub at: Vec2,
    pub height: f32,
    pub outputs: usize,
    pub input: bool,
}

impl Card {
    pub(crate) fn rect(&self) -> Rect {
        Rect::new(self.at, Vec2::new(NODE_W, self.height))
    }

    /// Where wires enter: the middle of the left edge.
    pub(crate) fn input_port(&self) -> Vec2 {
        Vec2::new(self.at.x, self.at.y + self.height / 2.0)
    }

    /// Where wires leave by output `port`: spread evenly down the right
    /// edge.
    pub(crate) fn output_port(&self, port: usize) -> Vec2 {
        #[allow(clippy::cast_precision_loss)]
        let share = (port as f32 + 1.0) / (self.outputs.max(1) as f32 + 1.0);
        Vec2::new(self.at.x + NODE_W, self.at.y + self.height * share)
    }
}

/// The canvas laid out: the trigger's card first, then one per node in
/// the graph's order, and per edge (in the graph's order) the cards and
/// port it joins, `None` for an edge naming a node that is not there.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Geometry {
    pub cards: Vec<Card>,
    pub edges: Vec<Option<(usize, usize, usize)>>,
}

/// The card of `end` in [`Geometry::cards`].
pub(crate) fn card_index(end: End) -> usize {
    match end {
        End::Trigger => 0,
        End::Node(index) => index + 1,
    }
}

impl Geometry {
    pub(crate) fn build(graph: &FlowGraph, trigger_at: Vec2, catalogue: &Catalogue) -> Self {
        let card = |end: End, at: Vec2| {
            let outputs = graph.outputs(end, catalogue).len();
            Card {
                end,
                at,
                height: node_height(outputs),
                outputs,
                input: end != End::Trigger,
            }
        };
        let mut cards = Vec::with_capacity(graph.nodes.len() + 1);
        cards.push(card(End::Trigger, trigger_at));
        for index in 0..graph.nodes.len() {
            let (x, y) = graph.position(index);
            #[allow(clippy::cast_possible_truncation)]
            cards.push(card(End::Node(index), Vec2::new(x as f32, y as f32)));
        }
        let edges = graph
            .edges
            .iter()
            .map(|edge| {
                let from = graph.end_of(&edge.from)?;
                let to = graph.index_of(&edge.to)?;
                Some((card_index(from), edge.port, card_index(End::Node(to))))
            })
            .collect();
        Self { cards, edges }
    }

    pub(crate) fn card(&self, end: End) -> Option<&Card> {
        self.cards.get(card_index(end))
    }

    /// The curve of edge `edge`, in world units.
    pub(crate) fn curve(&self, edge: usize) -> Option<Bezier> {
        let (from, port, to) = (*self.edges.get(edge)?)?;
        let (from, to) = (self.cards.get(from)?, self.cards.get(to)?);
        Some(wire_curve(from.output_port(port), to.input_port()))
    }

    /// Everything, for fitting the view.
    pub(crate) fn bounds(&self) -> Rect {
        self.cards
            .iter()
            .skip(1)
            .fold(self.cards[0].rect(), |all, card| all.union(&card.rect()))
    }

    /// `near`, or the first place below it no card already starts at.
    pub(crate) fn free_spot(&self, near: Vec2) -> Vec2 {
        let taken = |spot: Vec2| {
            self.cards
                .iter()
                .any(|card| (card.at - spot).length() < 24.0)
        };
        let mut spot = near;
        for _ in 0..64 {
            if !taken(spot) {
                break;
            }
            spot.y += NODE_H + 24.0;
        }
        spot
    }
}

/// What a press on the canvas hits, topmost first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    /// An output port: a wire starts here.
    Output(End, usize),
    /// A card (its input port included).
    Node(End),
    /// Edge `n` of the graph.
    Wire(usize),
    Empty,
}

/// What is at `world` with the view at `zoom` (reaches are on-screen
/// distances).
pub(crate) fn hit(geometry: &Geometry, world: Vec2, zoom: f32) -> Hit {
    let port_reach = PORT_REACH / zoom;
    for card in geometry.cards.iter().rev() {
        let nearest = (0..card.outputs)
            .map(|port| (port, (card.output_port(port) - world).length()))
            .filter(|(_, distance)| *distance <= port_reach)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((port, _)) = nearest {
            return Hit::Output(card.end, port);
        }
        if card
            .rect()
            .inflate(port_reach.min(PORT_R / zoom))
            .contains(world)
        {
            return Hit::Node(card.end);
        }
    }
    let wire_reach = WIRE_REACH / zoom;
    (0..geometry.edges.len())
        .filter_map(|edge| Some((edge, geometry.curve(edge)?)))
        .filter(|(_, curve)| curve.bounds().inflate(wire_reach).contains(world))
        .map(|(edge, curve)| (edge, curve.distance(world)))
        .filter(|(_, distance)| *distance <= wire_reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(Hit::Empty, |(edge, _)| Hit::Wire(edge))
}

/// Where a wire dragged to `world` lands: a node (its card or its input
/// port), the topmost; never the trigger.
pub(crate) fn drop_target(geometry: &Geometry, world: Vec2, zoom: f32) -> Option<usize> {
    let reach = PORT_REACH / zoom;
    geometry
        .cards
        .iter()
        .rev()
        .filter(|card| card.input)
        .find(|card| card.rect().contains(world) || (card.input_port() - world).length() <= reach)
        .and_then(|card| match card.end {
            End::Node(index) => Some(index),
            End::Trigger => None,
        })
}

/// The next (or previous) end after `from` for Tab: the trigger, then the
/// nodes in order, round.
pub(crate) fn cycle(nodes: usize, from: Option<End>, forward: bool) -> End {
    let all: Vec<End> = std::iter::once(End::Trigger)
        .chain((0..nodes).map(End::Node))
        .collect();
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
    use voltrus_scada_native::api::eng_flows::Catalogue;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    const CATALOGUE: &str = r#"{
        "triggers": [ { "type": "timer", "fields": [] } ],
        "nodes": [
            { "type": "switch", "category": "logic", "outputs": ["otherwise"] },
            { "type": "delay", "category": "timing", "outputs": ["out"] },
            { "type": "alarm_out", "category": "output", "outputs": [] }
        ]
    }"#;

    /// trigger → switch (rules a, b; otherwise); a → delay, otherwise →
    /// alarm.
    const GRAPH: &str = r#"{"version": 2,
        "nodes": [
            {"id": "s", "type": "switch", "x": 300, "y": 0,
             "rules": [{"name": "a", "expression": "1"}, {"name": "b", "expression": "2"}]},
            {"id": "d", "type": "delay", "x": 600, "y": -100},
            {"id": "al", "type": "alarm_out", "x": 600, "y": 200}
        ],
        "edges": [
            {"from": "trigger", "port": 0, "to": "s"},
            {"from": "s", "port": 0, "to": "d"},
            {"from": "s", "port": 2, "to": "al"},
            {"from": "s", "port": 1, "to": "ghost"}
        ]}"#;

    fn geometry() -> Geometry {
        let catalogue: Catalogue = serde_json::from_str(CATALOGUE).unwrap();
        let (graph, _) = FlowGraph::parse(r#"{"type": "timer"}"#, GRAPH).unwrap();
        Geometry::build(&graph, Vec2::new(0.0, 0.0), &catalogue)
    }

    #[test]
    fn cards_grow_with_their_ports_spread_down_the_right() {
        let geometry = geometry();
        assert_eq!(geometry.cards.len(), 4);
        let trigger = geometry.cards[0];
        assert_eq!((trigger.outputs, trigger.input), (1, false));
        assert!((trigger.height - NODE_H).abs() < 1e-6);
        let switch = geometry.cards[1];
        assert_eq!(switch.outputs, 3, "two rules and otherwise");
        assert!((switch.height - 78.0).abs() < 1e-6);
        assert!(close(switch.input_port(), Vec2::new(300.0, 39.0)));
        assert!(close(switch.output_port(0), Vec2::new(490.0, 19.5)));
        assert!(close(switch.output_port(1), Vec2::new(490.0, 39.0)));
        assert!(close(switch.output_port(2), Vec2::new(490.0, 58.5)));
        let alarm = geometry.cards[3];
        assert_eq!(alarm.outputs, 0);
        // A dangling edge has no curve; the others leave by their port.
        assert_eq!(geometry.edges[3], None);
        assert!(geometry.curve(3).is_none());
        let curve = geometry.curve(2).unwrap();
        assert!(close(curve.from, switch.output_port(2)));
        assert!(close(curve.to, alarm.input_port()));
        let bounds = geometry.bounds();
        assert_eq!(bounds.origin, Vec2::new(0.0, -100.0));
        assert_eq!(bounds.size, Vec2::new(790.0, 366.0));
    }

    #[test]
    fn hits_prefer_ports_then_cards_then_wires() {
        let geometry = geometry();
        let switch = geometry.cards[1];
        assert_eq!(
            hit(&geometry, switch.output_port(1), 1.0),
            Hit::Output(End::Node(0), 1)
        );
        assert_eq!(
            hit(&geometry, switch.output_port(2) + Vec2::new(2.0, 3.0), 1.0),
            Hit::Output(End::Node(0), 2),
            "the nearest port"
        );
        assert_eq!(
            hit(&geometry, switch.at + Vec2::new(40.0, 30.0), 1.0),
            Hit::Node(End::Node(0))
        );
        assert_eq!(
            hit(&geometry, switch.input_port(), 1.0),
            Hit::Node(End::Node(0)),
            "the input port belongs to the card"
        );
        let alarm = geometry.cards[3];
        assert_eq!(
            hit(&geometry, alarm.at + Vec2::new(NODE_W, NODE_H / 2.0), 1.0),
            Hit::Node(End::Node(2)),
            "no output port on an alarm"
        );
        let middle = geometry.curve(1).unwrap().at(0.5);
        assert_eq!(hit(&geometry, middle, 1.0), Hit::Wire(1));
        assert_eq!(hit(&geometry, Vec2::new(0.0, 900.0), 1.0), Hit::Empty);
        // Zoomed out, reaches grow in world units.
        let near = geometry.cards[2].output_port(0) + Vec2::new(0.0, 15.0);
        assert_ne!(hit(&geometry, near, 1.0), Hit::Output(End::Node(1), 0));
        assert_eq!(hit(&geometry, near, 0.5), Hit::Output(End::Node(1), 0));
        // Drops land on inputs, never on the trigger.
        assert_eq!(drop_target(&geometry, alarm.input_port(), 1.0), Some(2));
        assert_eq!(
            drop_target(&geometry, Vec2::new(10.0, 30.0), 1.0),
            None,
            "the trigger takes no input"
        );
    }

    #[test]
    fn snapping_tab_and_free_spots() {
        assert_eq!(snap(Vec2::new(14.0, -26.0)), Vec2::new(10.0, -30.0));
        assert_eq!(cycle(3, None, true), End::Trigger);
        assert_eq!(cycle(3, None, false), End::Node(2));
        assert_eq!(cycle(3, Some(End::Node(2)), true), End::Trigger);
        assert_eq!(cycle(3, Some(End::Trigger), false), End::Node(2));
        assert_eq!(cycle(0, Some(End::Trigger), true), End::Trigger);
        let geometry = geometry();
        let switch = geometry.cards[1].at;
        assert_eq!(
            geometry.free_spot(switch),
            switch + Vec2::new(0.0, NODE_H + 24.0)
        );
        let empty = Vec2::new(5000.0, 5000.0);
        assert_eq!(geometry.free_spot(empty), empty);
    }

    #[test]
    fn wires_leave_right_and_arrive_left() {
        let curve = wire_curve(Vec2::new(0.0, 0.0), Vec2::new(-100.0, 50.0));
        assert!(curve.control_a.x > 0.0, "leaves to the right");
        assert!(curve.control_b.x < -100.0, "arrives from the left");
        assert!(close(curve.at(0.0), curve.from));
        assert!(close(curve.at(1.0), curve.to));
        assert!(curve.distance(curve.at(0.37)) < 0.5);
        assert!(curve.distance(Vec2::new(500.0, 500.0)) > 100.0);
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
        assert!(
            (view.zoom - FIT_LEAST).abs() < 1e-6,
            "a wide flow stays readable"
        );
        assert!(
            close(
                view.to_screen(wide.origin),
                Vec2::new(FIT_MARGIN, view.to_screen(wide.origin).y)
            ),
            "and starts at its left edge"
        );
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
    #[test]
    fn revealing_pans_only_as_far_as_needed() {
        let view = View::default();
        let size = Vec2::new(800.0, 600.0);
        let inside = Rect::new(Vec2::new(100.0, 100.0), Vec2::new(180.0, 72.0));
        assert_eq!(view.reveal(inside, size), view);
        let right = Rect::new(Vec2::new(900.0, 100.0), Vec2::new(180.0, 72.0));
        let moved = view.reveal(right, size);
        assert!(close(moved.pan, Vec2::new(800.0 - 24.0 - 1080.0, 0.0)));
        let left = Rect::new(Vec2::new(-300.0, 100.0), Vec2::new(180.0, 72.0));
        assert!(close(view.reveal(left, size).pan, Vec2::new(324.0, 0.0)));
    }
}
