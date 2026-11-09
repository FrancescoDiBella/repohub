//! A small Excalidraw-like whiteboard drawn natively with Rune.
//!
//! Shapes have the slightly wobbly, hand-drawn stroke Excalidraw is known for (each edge is drawn
//! twice with a seeded jitter), and boards are saved in Excalidraw's `.excalidraw` JSON format,
//! so a drawing also opens on excalidraw.com. Supported: rectangles, ellipses, diamonds,
//! arrows, lines, freehand strokes and text; select and move, delete, undo and redo.
//!
//! Elements live in a fixed world of `WORLD` points; the canvas scales it to the space it has.
use rune::{Color, Drag, El, LineCap, Phase, Shape, TextEdit, TextStyle, canvas, col, el, row, text, text_input};
use serde_json::{Value, json};

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
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Select, Tool::Rectangle, Tool::Ellipse, Tool::Diamond, Tool::Arrow, Tool::Line, Tool::Pen, Tool::Text];
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

#[derive(Clone, Debug, PartialEq)]
pub enum SaveState {
    Loading,
    Saved,
    Saving,
    Error(String),
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
    pub save: SaveState,
    pub text_at: Option<[f32; 2]>,
    pub text_edit: TextEdit,
    grab: Option<(Element, bool)>,
    creating: bool,
    next: u32,
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
            save: SaveState::Saved,
            text_at: None,
            text_edit: TextEdit::new(""),
            grab: None,
            creating: false,
            next: 1,
        }
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
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.elements, prev));
        self.selected = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.elements, next));
        self.selected = None;
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        let Some(i) = self.selected.filter(|i| *i < self.elements.len()) else { return false };
        self.snapshot();
        self.elements.remove(i);
        self.selected = None;
        true
    }

    fn new_id(&mut self) -> (String, u32) {
        self.next += 1;
        let seed = self.next.wrapping_mul(2_654_435_761) ^ self.elements.len() as u32;
        (format!("rh{}-{}", self.next, seed % 100_000), seed)
    }

    fn hit(&self, p: [f32; 2]) -> Option<usize> {
        (0..self.elements.len()).rev().find(|&i| self.elements[i].hit(p))
    }

    /// Applies a pointer gesture on the canvas (`d` in the canvas' local points). Returns true
    /// when the drawing changed and should be saved.
    pub fn on_drag(&mut self, d: Gesture) -> bool {
        let (sx, sy) = (WORLD[0] / d.w.max(1.0), WORLD[1] / d.h.max(1.0));
        let p = [d.x * sx, d.y * sy];
        let start = [d.start_x * sx, d.start_y * sy];
        match d.phase {
            Phase::Down => {
                self.text_at = None;
                match self.tool {
                    Tool::Select => {
                        self.selected = self.hit(p);
                        self.grab = self.selected.map(|i| (self.elements[i].clone(), false));
                    }
                    Tool::Text => {
                        self.selected = None;
                        self.text_at = Some(p);
                        self.text_edit = TextEdit::new("");
                        self.text_edit.focused = true;
                    }
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
                        self.creating = true;
                    }
                }
                false
            }
            Phase::Move => {
                if self.creating {
                    if let Some(e) = self.elements.last_mut() {
                        match e.kind {
                            Kind::Arrow | Kind::Line => e.points[1] = [p[0] - e.x, p[1] - e.y],
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
                } else if let (Some(i), Some((orig, moved))) = (self.selected, self.grab.as_mut()) {
                    let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
                    if !*moved && dx.hypot(dy) > 1.0 {
                        *moved = true;
                        let before = {
                            let mut v = self.elements.clone();
                            v[i] = orig.clone();
                            v
                        };
                        self.undo.push(before);
                        self.redo.clear();
                    }
                    let (ox, oy) = (orig.x, orig.y);
                    if let Some(e) = self.elements.get_mut(i) {
                        e.x = ox + dx;
                        e.y = oy + dy;
                    }
                }
                false
            }
            Phase::Up => {
                if std::mem::take(&mut self.creating) {
                    let Some(e) = self.elements.last_mut() else { return false };
                    let [x0, y0, x1, y1] = e.bbox();
                    let tiny = match e.kind {
                        Kind::Freedraw => e.points.len() < 2,
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
                matches!(self.grab.take(), Some((_, true)))
            }
        }
    }

    /// Adds the text being typed at `text_at`. Returns true when something was added.
    pub fn commit_text(&mut self) -> bool {
        let Some(at) = self.text_at.take() else { return false };
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

    // ---------------------------------------------------------------- file format

    /// The drawing as an Excalidraw document.
    pub fn to_json(&self) -> String {
        let els: Vec<Value> = self.elements.iter().enumerate().map(|(i, e)| {
            let mut v = json!({
                "id": e.id, "type": e.kind.excalidraw(), "x": e.x, "y": e.y, "width": e.w, "height": e.h,
                "angle": 0, "strokeColor": e.stroke, "backgroundColor": e.fill.clone().unwrap_or_else(|| "transparent".into()),
                "fillStyle": "solid", "strokeWidth": e.width, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
                "groupIds": [], "frameId": null, "roundness": null, "seed": e.seed, "version": 1, "versionNonce": e.seed ^ 0x5bd1,
                "isDeleted": false, "boundElements": null, "updated": 0, "link": null, "locked": false, "index": format!("a{i}"),
            });
            if e.kind.uses_points() {
                v["points"] = json!(e.points);
                v["lastCommittedPoint"] = Value::Null;
            }
            match e.kind {
                Kind::Arrow | Kind::Line => {
                    v["startBinding"] = Value::Null;
                    v["endBinding"] = Value::Null;
                    v["startArrowhead"] = Value::Null;
                    v["endArrowhead"] = if e.kind == Kind::Arrow { json!("arrow") } else { Value::Null };
                }
                Kind::Freedraw => {
                    v["pressures"] = json!([]);
                    v["simulatePressure"] = json!(true);
                }
                Kind::Text => {
                    v["text"] = json!(e.text);
                    v["originalText"] = json!(e.text);
                    v["fontSize"] = json!(e.font);
                    v["fontFamily"] = json!(1);
                    v["textAlign"] = json!("left");
                    v["verticalAlign"] = json!("top");
                    v["containerId"] = Value::Null;
                    v["autoResize"] = json!(true);
                    v["lineHeight"] = json!(1.25);
                }
                _ => {}
            }
            v
        }).collect();
        let doc = json!({
            "type": "excalidraw", "version": 2, "source": "https://github.com/FrancescoDiBella/repohub",
            "elements": els, "appState": { "viewBackgroundColor": "#ffffff", "gridSize": null }, "files": {},
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    /// Reads an Excalidraw document; element types RepoHub doesn't draw are skipped.
    pub fn load_json(&mut self, src: &str) -> Result<(), String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("Not an Excalidraw file: {e}"))?;
        let f = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let st = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        self.elements = v.get("elements").and_then(Value::as_array).cloned().unwrap_or_default().iter().filter_map(|e| {
            if e.get("isDeleted").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let kind = Kind::from_excalidraw(&st(e, "type"))?;
            let fill = Some(st(e, "backgroundColor")).filter(|c| !c.is_empty() && c != "transparent");
            Some(Element {
                id: st(e, "id"),
                kind,
                x: f(e, "x"),
                y: f(e, "y"),
                w: f(e, "width"),
                h: f(e, "height"),
                points: e.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(|p| {
                    let p = p.as_array()?;
                    Some([p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])
                }).collect()).unwrap_or_default(),
                stroke: Some(st(e, "strokeColor")).filter(|c| !c.is_empty()).unwrap_or_else(|| STROKES[0].into()),
                fill,
                width: f(e, "strokeWidth").max(1.0),
                text: st(e, "text"),
                font: Some(f(e, "fontSize")).filter(|s| *s > 0.0).unwrap_or(20.0),
                seed: e.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32,
            })
        }).collect();
        self.next = self.elements.len() as u32 + 1;
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        Ok(())
    }
}

// -------------------------------------------------------------------- drawing

/// A hand-drawn segment: a slight bow and jittered ends.
fn rough_seg(a: [f32; 2], b: [f32; 2], seed: u32, k: u32) -> Vec<[f32; 2]> {
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    let r = (len * 0.012).clamp(0.4, 2.2);
    let (nx, ny) = if len > 0.0 { (-(b[1] - a[1]) / len, (b[0] - a[0]) / len) } else { (0.0, 0.0) };
    let bow = jit(seed, k * 7 + 1) * (len * 0.01).min(3.0);
    let j = |i: u32| [jit(seed, k * 7 + i) * r, jit(seed, k * 7 + i + 3) * r];
    let (j0, j1, j2) = (j(2), j(3), j(4));
    let m = [(a[0] + b[0]) / 2.0 + nx * bow + j1[0], (a[1] + b[1]) / 2.0 + ny * bow + j1[1]];
    vec![[a[0] + j0[0], a[1] + j0[1]], m, [b[0] + j2[0], b[1] + j2[1]]]
}

fn rough_poly(mut s: Shape, pts: &[[f32; 2]], closed: bool, seed: u32, width: f32, c: Color) -> Shape {
    let n = pts.len();
    if n < 2 {
        return s;
    }
    let segs = if closed { n } else { n - 1 };
    for pass in 0..2u32 {
        for i in 0..segs {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            let seg = rough_seg(a, b, seed.wrapping_add(pass * 977), i as u32);
            s = s.stroke_with(seg, false, if pass == 0 { width } else { width * 0.7 }, LineCap::Round, c);
        }
    }
    s
}

fn ellipse_pts(e: &Element, wobble: bool) -> Vec<[f32; 2]> {
    let (cx, cy, rx, ry) = (e.x + e.w / 2.0, e.y + e.h / 2.0, e.w / 2.0, e.h / 2.0);
    let n = 44;
    (0..n).map(|i| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let w = if wobble { 1.0 + jit(e.seed, i as u32 + 50) * 0.012 } else { 1.0 };
        [cx + rx * w * a.cos(), cy + ry * w * a.sin()]
    }).collect()
}

fn ink(e: &Element, dark: bool) -> Color {
    // Excalidraw draws black as near-white on a dark canvas.
    if dark && (e.stroke == STROKES[0] || e.stroke == "#000000") { hex("#e6edf3") } else { hex(&e.stroke) }
}

fn draw_element(mut s: Shape, e: &Element, dark: bool) -> Shape {
    let c = ink(e, dark);
    let fill = e.fill.as_deref().map(|f| if dark { hex(f).alpha(0.35) } else { hex(f) });
    match e.kind {
        Kind::Rectangle => {
            let p = [[e.x, e.y], [e.x + e.w, e.y], [e.x + e.w, e.y + e.h], [e.x, e.y + e.h]];
            if let Some(f) = fill {
                s = s.fill(p.to_vec(), f);
            }
            rough_poly(s, &p, true, e.seed, e.width, c)
        }
        Kind::Diamond => {
            let p = [[e.x + e.w / 2.0, e.y], [e.x + e.w, e.y + e.h / 2.0], [e.x + e.w / 2.0, e.y + e.h], [e.x, e.y + e.h / 2.0]];
            if let Some(f) = fill {
                s = s.fill(p.to_vec(), f);
            }
            rough_poly(s, &p, true, e.seed, e.width, c)
        }
        Kind::Ellipse => {
            if let Some(f) = fill {
                s = s.fill(ellipse_pts(e, false), f);
            }
            let p = ellipse_pts(e, true);
            s = s.stroke_with(p.clone(), true, e.width, LineCap::Round, c);
            let shifted: Vec<[f32; 2]> = p.iter().enumerate().map(|(i, q)| [q[0] + jit(e.seed, i as u32 + 200) * 0.9, q[1] + jit(e.seed, i as u32 + 300) * 0.9]).collect();
            s.stroke_with(shifted, true, e.width * 0.6, LineCap::Round, c)
        }
        Kind::Line | Kind::Arrow => {
            let pts: Vec<[f32; 2]> = e.points.iter().map(|p| [e.x + p[0], e.y + p[1]]).collect();
            s = rough_poly(s, &pts, false, e.seed, e.width, c);
            if e.kind == Kind::Arrow && pts.len() >= 2 {
                let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
                let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                let head = (12.0 + e.width * 2.0).min(len * 0.5);
                for side in [-1.0f32, 1.0] {
                    let t = ang + std::f32::consts::PI + side * 0.45;
                    let tip = [b[0] + head * t.cos(), b[1] + head * t.sin()];
                    s = rough_poly(s, &[tip, b], false, e.seed.wrapping_add(side as u32 + 9), e.width, c);
                }
            }
            s
        }
        Kind::Freedraw => {
            let pts: Vec<[f32; 2]> = e.points.iter().map(|p| [e.x + p[0], e.y + p[1]]).collect();
            s.stroke_with(pts, false, e.width * 1.4, LineCap::Round, c)
        }
        Kind::Text => s,
    }
}

/// The canvas: dot grid, elements, selection, text labels and the text being typed.
pub fn canvas_view<S: 'static>(
    b: &Board,
    dark: bool,
    accent: Color,
    scale: f32,
    on_drag: impl Fn(&mut S, &mut Drag, &mut rune::Tx) + 'static,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
    commit: impl Fn(&mut S, &mut rune::Tx) + 'static,
) -> El {
    let bg = if dark { hex("#121212") } else { hex("#ffffff") };
    let dot = if dark { Color::WHITE.alpha(0.08) } else { Color::BLACK.alpha(0.09) };
    let mut s = Shape::new().view_box(WORLD[0], WORLD[1]);
    let step = 40.0;
    let (mut y, mut gy) = (step, 0);
    while y < WORLD[1] && gy < 40 {
        let mut x = step;
        while x < WORLD[0] {
            s = s.fill(vec![[x - 0.9, y - 0.9], [x + 0.9, y - 0.9], [x + 0.9, y + 0.9], [x - 0.9, y + 0.9]], dot);
            x += step;
        }
        y += step;
        gy += 1;
    }
    for e in &b.elements {
        s = draw_element(s, e, dark);
    }
    if let Some(e) = b.selected.and_then(|i| b.elements.get(i)) {
        let [x0, y0, x1, y1] = e.bbox();
        let pad = 6.0;
        let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
        s = s.stroke_with(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true, 1.0, LineCap::Butt, accent);
        for c in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            let h = 4.5;
            s = s.fill(vec![[c[0] - h, c[1] - h], [c[0] + h, c[1] - h], [c[0] + h, c[1] + h], [c[0] - h, c[1] + h]], accent);
        }
    }
    let mut board = el().key("board").w_full().aspect_ratio(WORLD[0] / WORLD[1]).fill(bg).radius(10.0).clip_rounded(10.0)
        .role("img").label("Drawing canvas")
        .child(canvas(s).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)))
        .on_drag(on_drag);
    for e in b.elements.iter().filter(|e| e.kind == Kind::Text) {
        let size = (e.font * scale).max(6.0);
        let mut lines = col().absolute(e.x * scale, e.y * scale).gap(0.0);
        for l in e.text.lines() {
            lines = lines.child(text(l.to_owned()).font(TextStyle::new(size, 500.0)).color(ink(e, dark)));
        }
        board = board.child(lines);
    }
    if let Some(at) = b.text_at {
        let field = text_input(&b.text_edit, TextStyle::new((20.0 * scale).max(10.0), 500.0))
            .single_line().placeholder("Type, then Enter", if dark { Color::WHITE.alpha(0.4) } else { Color::BLACK.alpha(0.35) })
            .color(if dark { hex("#e6edf3") } else { hex(&b_stroke(b)) })
            .w(320.0 * scale.max(0.6)).h((30.0 * scale).max(22.0)).px(4.0)
            .border(1.0, accent).radius(4.0).fill(bg)
            .on_edit(access).on_submit(commit).key("board-text");
        board = board.child(row().absolute(at[0] * scale, (at[1] - 14.0) * scale).child(field));
    }
    board
}

fn b_stroke(b: &Board) -> String {
    STROKES[b.color].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(phase: Phase, x: f32, y: f32, sx: f32, sy: f32) -> Gesture {
        Gesture { phase, x, y, start_x: sx, start_y: sy, w: WORLD[0], h: WORLD[1], cancelled: false }
    }

    #[test]
    fn draw_move_undo_and_round_trip() {
        let mut b = Board::new("o/r", "main", "t");
        b.tool = Tool::Rectangle;
        assert!(!b.on_drag(drag(Phase::Down, 100.0, 100.0, 100.0, 100.0)));
        b.on_drag(drag(Phase::Move, 300.0, 200.0, 100.0, 100.0));
        assert!(b.on_drag(drag(Phase::Up, 300.0, 200.0, 100.0, 100.0)));
        assert_eq!((b.elements[0].w, b.elements[0].h), (200.0, 100.0));
        // Move it by its outline.
        b.tool = Tool::Select;
        b.on_drag(drag(Phase::Down, 100.0, 150.0, 100.0, 150.0));
        assert_eq!(b.selected, Some(0));
        b.on_drag(drag(Phase::Move, 150.0, 150.0, 100.0, 150.0));
        assert!(b.on_drag(drag(Phase::Up, 150.0, 150.0, 100.0, 150.0)));
        assert_eq!(b.elements[0].x, 150.0);
        assert!(b.undo());
        assert_eq!(b.elements[0].x, 100.0);
        assert!(b.redo());
        // A tiny click with a shape tool adds nothing.
        b.tool = Tool::Ellipse;
        b.on_drag(drag(Phase::Down, 500.0, 500.0, 500.0, 500.0));
        assert!(!b.on_drag(drag(Phase::Up, 501.0, 501.0, 500.0, 500.0)));
        assert_eq!(b.elements.len(), 1);
        // Text and arrow, then the Excalidraw round trip.
        b.text_at = Some([400.0, 400.0]);
        b.text_edit = TextEdit::new("Hello");
        assert!(b.commit_text());
        b.tool = Tool::Arrow;
        b.on_drag(drag(Phase::Down, 10.0, 10.0, 10.0, 10.0));
        b.on_drag(drag(Phase::Move, 90.0, 60.0, 10.0, 10.0));
        assert!(b.on_drag(drag(Phase::Up, 90.0, 60.0, 10.0, 10.0)));
        let json = b.to_json();
        let mut c = Board::new("o/r", "main", "t");
        c.load_json(&json).unwrap();
        assert_eq!(c.elements.len(), 3);
        assert_eq!(c.elements[1].text, "Hello");
        assert_eq!(c.elements[2].kind, Kind::Arrow);
        assert_eq!(c.elements[2].points[1], [80.0, 50.0]);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "excalidraw");
        assert_eq!(v["elements"][2]["endArrowhead"], "arrow");
    }
}
