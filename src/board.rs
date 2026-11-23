//! A small Excalidraw-like whiteboard drawn natively with Rune.
//!
//! Shapes have the slightly wobbly, hand-drawn stroke Excalidraw is known for (each edge is drawn
//! twice with a seeded jitter), and boards are saved in Excalidraw's `.excalidraw` JSON format,
//! so a drawing also opens on excalidraw.com. Supported: rectangles, ellipses, diamonds,
//! arrows, lines, freehand strokes and text; select and move, delete, undo and redo.
//!
//! Elements live in a fixed world of `WORLD` points; the canvas scales it to the space it has.
use std::collections::{HashMap, HashSet};

use rune::{Color, Drag, El, LineCap, Phase, Shape, TextEdit, TextStyle, canvas, col, el, row, text, text_input};
use serde_json::{Value, json};

mod drawing;
mod file;

pub use drawing::canvas_view;
pub use file::import;

pub const WORLD: [f32; 2] = [1200.0, 720.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Pen,
    Text,
    /// Drags the view around a zoomed-in drawing.
    Pan,
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Select, Tool::Rectangle, Tool::Ellipse, Tool::Diamond, Tool::Arrow, Tool::Line, Tool::Pen, Tool::Text];
    /// The tools in the order a phone shows them: drawing by hand first.
    pub const TOUCH: [Tool; 9] = [Tool::Pen, Tool::Pan, Tool::Select, Tool::Arrow, Tool::Rectangle, Tool::Ellipse, Tool::Diamond, Tool::Line, Tool::Text];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Diamond => "Diamond",
            Tool::Arrow => "Arrow",
            Tool::Line => "Line",
            Tool::Pen => "Draw",
            Tool::Text => "Text",
            Tool::Pan => "Move the view",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Rectangle,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Freedraw,
    Text,
}

impl Kind {
    fn excalidraw(self) -> &'static str {
        match self {
            Kind::Rectangle => "rectangle",
            Kind::Ellipse => "ellipse",
            Kind::Diamond => "diamond",
            Kind::Arrow => "arrow",
            Kind::Line => "line",
            Kind::Freedraw => "freedraw",
            Kind::Text => "text",
        }
    }
    fn from_excalidraw(t: &str) -> Option<Kind> {
        Some(match t {
            "rectangle" => Kind::Rectangle,
            "ellipse" => Kind::Ellipse,
            "diamond" => Kind::Diamond,
            "arrow" => Kind::Arrow,
            "line" => Kind::Line,
            "freedraw" => Kind::Freedraw,
            "text" => Kind::Text,
            _ => return None,
        })
    }
    fn uses_points(self) -> bool {
        matches!(self, Kind::Arrow | Kind::Line | Kind::Freedraw)
    }
}

/// Excalidraw's stroke palette, and the matching light fills.
pub const STROKES: [&str; 6] = ["#1e1e1e", "#e03131", "#2f9e44", "#1971c2", "#f08c00", "#9c36b5"];
pub const FILLS: [&str; 6] = ["#e9ecef", "#ffc9c9", "#b2f2bb", "#a5d8ff", "#ffec99", "#eebefa"];

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub id: String,
    pub kind: Kind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Arrow, line and freehand points, relative to `x`, `y`.
    pub points: Vec<[f32; 2]>,
    pub stroke: String,
    pub fill: Option<String>,
    pub width: f32,
    pub text: String,
    pub font: f32,
    pub seed: u32,
}

impl Element {
    /// Bounding box `[x0, y0, x1, y1]` in world points.
    pub fn bbox(&self) -> [f32; 4] {
        if self.kind.uses_points() && !self.points.is_empty() {
            let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in &self.points {
                a = [a[0].min(p[0]), a[1].min(p[1])];
                b = [b[0].max(p[0]), b[1].max(p[1])];
            }
            [self.x + a[0], self.y + a[1], self.x + b[0], self.y + b[1]]
        } else {
            [self.x, self.y, self.x + self.w, self.y + self.h]
        }
    }

    fn hit(&self, p: [f32; 2]) -> bool {
        let tol = 7.0 + self.width;
        let [x0, y0, x1, y1] = self.bbox();
        if p[0] < x0 - tol || p[0] > x1 + tol || p[1] < y0 - tol || p[1] > y1 + tol {
            return false;
        }
        match self.kind {
            Kind::Arrow | Kind::Line | Kind::Freedraw => self.points.windows(2).any(|s| {
                let a = [self.x + s[0][0], self.y + s[0][1]];
                let b = [self.x + s[1][0], self.y + s[1][1]];
                seg_dist(p, a, b) <= tol
            }) || self.points.len() == 1,
            Kind::Rectangle | Kind::Ellipse | Kind::Diamond if self.fill.is_none() => {
                // Unfilled shapes are picked by their outline, like Excalidraw.
                let inner = p[0] > x0 + tol && p[0] < x1 - tol && p[1] > y0 + tol && p[1] < y1 - tol;
                !inner
            }
            _ => true,
        }
    }
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = vx * vx + vy * vy;
    let t = if len2 < 1e-6 { 0.0 } else { (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (a[0] + t * vx, a[1] + t * vy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

/// What the pointer and keyboard are doing on the canvas.
///
/// ```text
/// Idle ──press with a shape tool──▶ Creating ──release──▶ Idle (shape kept, or dropped if tiny)
/// Idle ──press on a shape (Select)──▶ Moving ──release──▶ Idle
/// Idle ──press (Text)──▶ Typing ──Enter / another press / another tool──▶ Idle
/// ```
#[derive(Clone, Debug, PartialEq)]
enum Mode {
    Idle,
    /// Dragging out the last element of `elements`.
    Creating,
    /// Dragging the selected element; `original` is how it was, `moved` once it really moved
    /// (and the undo step was recorded).
    Moving { original: Element, moved: bool },
    /// Typing a text that will be placed at this point.
    Typing { at: [f32; 2] },
    /// Dragging the view; `from` is its top-left when the drag began.
    Panning { from: [f32; 2] },
}

/// A pointer gesture on the canvas, in the canvas' local points (see [`Drag`]).
#[derive(Clone, Copy, Debug)]
pub struct Gesture {
    pub phase: Phase,
    pub x: f32,
    pub y: f32,
    pub start_x: f32,
    pub start_y: f32,
    pub w: f32,
    pub h: f32,
    pub cancelled: bool,
}

impl From<&Drag> for Gesture {
    fn from(d: &Drag) -> Gesture {
        Gesture { phase: d.phase, x: d.x, y: d.y, start_x: d.start_x, start_y: d.start_y, w: d.w, h: d.h, cancelled: d.phase == Phase::Up && d.cancelled() }
    }
}

/// The part of the drawing on screen: `zoom` times the size that fits the drawing's width, with
/// world point (`x`, `y`) at the canvas' top-left. A zoom of 0 means "not chosen yet": the
/// canvas then shows the whole width, or fills its height if it's taller than the drawing, and
/// (`x`, `y`) is the point to show in the middle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub x: f32,
    pub y: f32,
}

pub const MAX_ZOOM: f32 = 8.0;

impl View {
    /// The zoom in use on a canvas of `size` points.
    pub fn zoom_on(&self, size: [f32; 2]) -> f32 {
        if self.zoom > 0.0 {
            return self.zoom;
        }
        // A canvas taller than the drawing (a phone held upright) starts filled top to bottom.
        ((size[1] / size[0].max(1.0)) / (WORLD[1] / WORLD[0])).clamp(1.0, MAX_ZOOM)
    }

    /// Canvas points per world point.
    fn scale_on(&self, size: [f32; 2]) -> f32 {
        size[0].max(1.0) / WORLD[0] * self.zoom_on(size)
    }

    /// The world size the canvas shows.
    pub fn visible(&self, size: [f32; 2]) -> [f32; 2] {
        let k = self.scale_on(size);
        [size[0] / k, size[1] / k]
    }

    /// The view's top-left, kept inside the drawing (centred when the canvas shows more).
    pub fn origin(&self, size: [f32; 2]) -> [f32; 2] {
        let vis = self.visible(size);
        let fit = |at: f32, vis: f32, world: f32| if vis >= world { (world - vis) / 2.0 } else { at.clamp(0.0, world - vis) };
        let (x, y) = if self.zoom > 0.0 { (self.x, self.y) } else { (self.x - vis[0] / 2.0, self.y - vis[1] / 2.0) };
        [fit(x, vis[0], WORLD[0]), fit(y, vis[1], WORLD[1])]
    }

    /// The world point under canvas point `p`.
    pub fn world(&self, size: [f32; 2], p: [f32; 2]) -> [f32; 2] {
        let (k, o) = (self.scale_on(size), self.origin(size));
        [o[0] + p[0] / k, o[1] + p[1] / k]
    }

    /// Zooms by `factor`, keeping the middle of the canvas where it is.
    pub fn zoom_by(&mut self, size: [f32; 2], factor: f32) {
        let mid = self.world(size, [size[0] / 2.0, size[1] / 2.0]);
        self.zoom = (self.zoom_on(size) * factor).clamp(1.0, MAX_ZOOM);
        let vis = self.visible(size);
        (self.x, self.y) = (mid[0] - vis[0] / 2.0, mid[1] - vis[1] / 2.0);
        [self.x, self.y] = self.origin(size);
    }
}

/// One open drawing: its elements, the editing state and where it's stored.
pub struct Board {
    pub repo: String,
    pub branch: String,
    pub name: String,
    pub elements: Vec<Element>,
    undo: Vec<Vec<Element>>,
    redo: Vec<Vec<Element>>,
    pub selected: Option<usize>,
    pub tool: Tool,
    pub color: usize,
    pub filled: bool,
    pub bold: bool,
    pub save: crate::state::Save,
    pub text_edit: TextEdit,
    pub view: View,
    mode: Mode,
    next: u32,
    /// Excalidraw's `version`, `versionNonce` and `updated` of each element, drawn or deleted,
    /// by id: what tells another device's copy which of two edits is newer.
    meta: HashMap<String, Meta>,
    /// Deleted elements, written with `isDeleted` so another device's copy learns of it.
    deleted: Vec<Element>,
    /// The elements as last written, to tell which changed since ([`Board::save_json`]).
    stamped: HashMap<String, Element>,
}

/// An element's Excalidraw bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Meta {
    version: u64,
    nonce: u32,
    /// Milliseconds since 1970.
    updated: i64,
}

/// A deterministic value in [-1, 1] for (seed, i).
fn jit(seed: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ i.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 32767.5 - 1.0
}

fn hex(c: &str) -> Color {
    rune::color::hex(c).unwrap_or(Color::BLACK)
}

impl Board {
    pub fn new(repo: &str, branch: &str, name: &str) -> Board {
        Board {
            repo: repo.into(),
            branch: branch.into(),
            name: name.into(),
            elements: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            tool: Tool::Rectangle,
            color: 0,
            filled: false,
            bold: false,
            save: crate::state::Save::Saved,
            text_edit: TextEdit::new(""),
            view: View { zoom: 0.0, x: WORLD[0] / 2.0, y: WORLD[1] / 2.0 },
            mode: Mode::Idle,
            next: 1,
            meta: HashMap::new(),
            deleted: Vec::new(),
            stamped: HashMap::new(),
        }
    }

    /// True while a gesture or a text is in progress: nothing replaces the drawing then.
    pub fn busy(&self) -> bool {
        self.mode != Mode::Idle
    }

    fn snapshot(&mut self) {
        self.undo.push(self.elements.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Returns true when the drawing changed (and should be saved).
    pub fn undo(&mut self) -> bool {
        if self.save == crate::state::Save::Loading {
            return false;
        }
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.elements, prev));
        self.selected = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.save == crate::state::Save::Loading {
            return false;
        }
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.elements, next));
        self.selected = None;
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        if self.save == crate::state::Save::Loading {
            return false;
        }
        let Some(i) = self.selected.filter(|i| *i < self.elements.len()) else { return false };
        self.snapshot();
        self.elements.remove(i);
        self.selected = None;
        true
    }

    /// A new element's id and seed. Ids are random, as Excalidraw's are, so two devices drawing
    /// on the same board never make the same one.
    fn new_id(&mut self) -> (String, u32) {
        self.next += 1;
        let (a, b) = (crate::sync::random(), crate::sync::random());
        const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let id: String = (0..20).map(|i| {
            let v = if i < 10 { a >> (i * 6) } else { b >> ((i - 10) * 6) };
            DIGITS[(v % 62) as usize] as char
        }).collect();
        (id, (b >> 32) as u32 ^ self.next)
    }

    fn hit(&self, p: [f32; 2]) -> Option<usize> {
        (0..self.elements.len()).rev().find(|&i| self.elements[i].hit(p))
    }

    /// Where the text being typed will go, while the text tool is typing.
    pub fn typing_at(&self) -> Option<[f32; 2]> {
        match self.mode {
            Mode::Typing { at } => Some(at),
            _ => None,
        }
    }

    pub fn is_typing(&self) -> bool {
        self.typing_at().is_some()
    }

    /// Starts typing a text at `at` (a point in the board's world).
    pub fn start_typing(&mut self, at: [f32; 2]) {
        if self.save == crate::state::Save::Loading {
            return;
        }
        self.selected = None;
        self.mode = Mode::Typing { at };
        self.text_edit = TextEdit::new("");
        self.text_edit.focused = true;
    }

    /// Picks a tool. Whatever was in progress (typing, a drag) is dropped.
    pub fn set_tool(&mut self, tool: Tool) {
        if self.save == crate::state::Save::Loading {
            return;
        }
        self.tool = tool;
        self.mode = Mode::Idle;
        if tool != Tool::Select {
            self.selected = None;
        }
    }

    pub fn deselect(&mut self) {
        self.selected = None;
    }

    /// Applies a pointer gesture on the canvas (`d` in the canvas' local points). Returns true
    /// when the drawing changed and should be saved.
    pub fn on_drag(&mut self, d: Gesture) -> bool {
        if self.save == crate::state::Save::Loading {
            return false;
        }
        let size = [d.w, d.h];
        if self.tool == Tool::Pan {
            return self.pan(d, size);
        }
        let p = self.view.world(size, [d.x, d.y]);
        let start = self.view.world(size, [d.start_x, d.start_y]);
        match d.phase {
            Phase::Down => {
                self.mode = Mode::Idle;
                match self.tool {
                    Tool::Select => {
                        self.selected = self.hit(p);
                        if let Some(i) = self.selected {
                            self.mode = Mode::Moving { original: self.elements[i].clone(), moved: false };
                        }
                    }
                    Tool::Text => self.start_typing(p),
                    tool => {
                        self.snapshot();
                        let (id, seed) = self.new_id();
                        let kind = match tool {
                            Tool::Rectangle => Kind::Rectangle,
                            Tool::Ellipse => Kind::Ellipse,
                            Tool::Diamond => Kind::Diamond,
                            Tool::Arrow => Kind::Arrow,
                            Tool::Line => Kind::Line,
                            _ => Kind::Freedraw,
                        };
                        let points = match kind {
                            Kind::Arrow | Kind::Line => vec![[0.0, 0.0], [0.0, 0.0]],
                            Kind::Freedraw => vec![[0.0, 0.0]],
                            _ => Vec::new(),
                        };
                        let fill = (self.filled && matches!(kind, Kind::Rectangle | Kind::Ellipse | Kind::Diamond)).then(|| FILLS[self.color].to_owned());
                        self.elements.push(Element {
                            id,
                            kind,
                            x: p[0],
                            y: p[1],
                            w: 0.0,
                            h: 0.0,
                            points,
                            stroke: STROKES[self.color].to_owned(),
                            fill,
                            width: if self.bold { 4.0 } else { 2.0 },
                            text: String::new(),
                            font: 20.0,
                            seed,
                        });
                        self.selected = None;
                        self.mode = Mode::Creating;
                    }
                }
                false
            }
            Phase::Move => {
                if self.mode == Mode::Creating {
                    if let Some(e) = self.elements.last_mut() {
                        match e.kind {
                            Kind::Arrow | Kind::Line => {
                                let pt = [p[0] - e.x, p[1] - e.y];
                                if e.points.len() >= 2 {
                                    e.points[1] = pt;
                                } else {
                                    if e.points.is_empty() {
                                        e.points.push([0.0, 0.0]);
                                    }
                                    e.points.push(pt);
                                }
                            }
                            Kind::Freedraw => {
                                let q = [p[0] - e.x, p[1] - e.y];
                                let last = *e.points.last().unwrap_or(&[0.0, 0.0]);
                                if (q[0] - last[0]).hypot(q[1] - last[1]) > 1.5 {
                                    e.points.push(q);
                                }
                            }
                            _ => {
                                e.x = start[0].min(p[0]);
                                e.y = start[1].min(p[1]);
                                e.w = (p[0] - start[0]).abs();
                                e.h = (p[1] - start[1]).abs();
                            }
                        }
                    }
                } else if let (Some(i), Mode::Moving { original, moved }) = (self.selected, &mut self.mode) {
                    let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
                    if !*moved && dx.hypot(dy) > 1.0 {
                        *moved = true;
                        let before = {
                            let mut v = self.elements.clone();
                            v[i] = original.clone();
                            v
                        };
                        self.undo.push(before);
                        self.redo.clear();
                    }
                    let (ox, oy) = (original.x, original.y);
                    if let Some(e) = self.elements.get_mut(i) {
                        e.x = ox + dx;
                        e.y = oy + dy;
                    }
                }
                false
            }
            Phase::Up => {
                // Releasing ends a drag; typing goes on until the text is committed.
                if self.is_typing() {
                    return false;
                }
                let mode = std::mem::replace(&mut self.mode, Mode::Idle);
                if mode == Mode::Creating {
                    let Some(e) = self.elements.last_mut() else { return false };
                    let [x0, y0, x1, y1] = e.bbox();
                    let tiny = match e.kind {
                        Kind::Freedraw | Kind::Arrow | Kind::Line if e.points.len() < 2 => true,
                        Kind::Freedraw => false,
                        _ => x1 - x0 < 4.0 && y1 - y0 < 4.0,
                    };
                    if tiny || d.cancelled {
                        self.elements.pop();
                        self.undo.pop();
                        return false;
                    }
                    if e.kind.uses_points() {
                        e.w = x1 - x0;
                        e.h = y1 - y0;
                    }
                    self.selected = Some(self.elements.len() - 1);
                    return true;
                }
                matches!(mode, Mode::Moving { moved: true, .. })
            }
        }
    }

    /// The Move tool: the drawing follows the finger. Never changes the drawing.
    fn pan(&mut self, d: Gesture, size: [f32; 2]) -> bool {
        match d.phase {
            Phase::Down => self.mode = Mode::Panning { from: self.view.origin(size) },
            Phase::Move => {
                if let Mode::Panning { from } = self.mode {
                    let k = self.view.scale_on(size);
                    // Keep the zoom the canvas was showing; from here (`x`, `y`) is the top-left.
                    self.view.zoom = self.view.zoom_on(size);
                    (self.view.x, self.view.y) = (from[0] - (d.x - d.start_x) / k, from[1] - (d.y - d.start_y) / k);
                    [self.view.x, self.view.y] = self.view.origin(size);
                }
            }
            Phase::Up => self.mode = Mode::Idle,
        }
        false
    }

    /// Adds the text being typed. Returns true when something was added.
    pub fn commit_text(&mut self) -> bool {
        if self.save == crate::state::Save::Loading {
            return false;
        }
        let Some(at) = self.typing_at() else { return false };
        self.mode = Mode::Idle;
        let t = self.text_edit.text.trim().to_owned();
        self.text_edit = TextEdit::new("");
        if t.is_empty() {
            return false;
        }
        self.snapshot();
        let (id, seed) = self.new_id();
        let font = if self.bold { 28.0 } else { 20.0 };
        let lines = t.lines().count().max(1) as f32;
        let widest = t.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
        self.elements.push(Element {
            id,
            kind: Kind::Text,
            x: at[0],
            y: at[1] - font * 0.6,
            w: widest * font * 0.55,
            h: lines * font * 1.25,
            points: Vec::new(),
            stroke: STROKES[self.color].to_owned(),
            fill: None,
            width: 1.0,
            text: t,
            font,
            seed,
        });
        self.selected = Some(self.elements.len() - 1);
        true
    }
}

#[cfg(test)]
mod tests;
