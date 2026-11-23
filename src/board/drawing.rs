//! Drawing: the hand-drawn strokes and shapes, and the canvas with its gestures.

use super::*;

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

pub(super) fn draw_element(mut s: Shape, e: &Element, dark: bool) -> Shape {
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
///
/// `size` is the canvas' size in points (as last laid out). With `fill` the canvas takes the
/// space it's given, for the phone's full-screen editor; otherwise it keeps the drawing's shape.
#[allow(clippy::too_many_arguments)]
pub fn canvas_view<S: 'static>(
    b: &Board,
    dark: bool,
    accent: Color,
    size: [f32; 2],
    fill: bool,
    on_drag: impl Fn(&mut S, &mut Drag, &mut rune::Tx) + 'static,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
    commit: impl Fn(&mut S, &mut rune::Tx) + 'static,
) -> El {
    let bg = if dark { hex("#121212") } else { hex("#ffffff") };
    let outside = if dark { hex("#1c1c1c") } else { hex("#eceef1") };
    let dot = if dark { Color::WHITE.alpha(0.08) } else { Color::BLACK.alpha(0.09) };
    let (vis, o, scale) = (b.view.visible(size), b.view.origin(size), b.view.scale_on(size));
    let mut s = Shape::new().view_box(vis[0], vis[1]);
    // The drawing's own area; around it (a zoomed-out or taller canvas) nothing can be drawn.
    s = s.fill(vec![[-o[0], -o[1]], [WORLD[0] - o[0], -o[1]], [WORLD[0] - o[0], WORLD[1] - o[1]], [-o[0], WORLD[1] - o[1]]], bg);
    let step = 40.0;
    let mut y = step;
    while y < WORLD[1] {
        let mut x = step;
        while x < WORLD[0] {
            let (px, py) = (x - o[0], y - o[1]);
            if px > -1.0 && py > -1.0 && px < vis[0] + 1.0 && py < vis[1] + 1.0 {
                s = s.fill(vec![[px - 0.9, py - 0.9], [px + 0.9, py - 0.9], [px + 0.9, py + 0.9], [px - 0.9, py + 0.9]], dot);
            }
            x += step;
        }
        y += step;
    }
    let shown = |e: &Element| {
        let mut e = e.clone();
        (e.x, e.y) = (e.x - o[0], e.y - o[1]);
        e
    };
    for e in &b.elements {
        s = draw_element(s, &shown(e), dark);
    }
    if let Some(e) = b.selected.and_then(|i| b.elements.get(i)) {
        let [x0, y0, x1, y1] = shown(e).bbox();
        let pad = 6.0;
        let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
        s = s.stroke_with(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true, 1.0, LineCap::Butt, accent);
        for c in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            let h = 4.5;
            s = s.fill(vec![[c[0] - h, c[1] - h], [c[0] + h, c[1] - h], [c[0] + h, c[1] + h], [c[0] - h, c[1] + h]], accent);
        }
    }
    let frame = el().key("board").w_full().fill(outside).radius(10.0).clip_rounded(10.0);
    let frame = if fill { frame.grow(1.0).min_h(0.0) } else { frame.aspect_ratio(WORLD[0] / WORLD[1]) };
    let mut board = frame
        .role("img").label("Drawing canvas")
        .child(canvas(s).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)))
        .on_drag(on_drag);
    for e in b.elements.iter().filter(|e| e.kind == Kind::Text) {
        let font = (e.font * scale).max(6.0);
        let mut lines = col().absolute((e.x - o[0]) * scale, (e.y - o[1]) * scale).gap(0.0);
        for l in e.text.lines() {
            lines = lines.child(text(l.to_owned()).font(TextStyle::new(font, 500.0)).color(ink(e, dark)));
        }
        board = board.child(lines);
    }
    if let Some(at) = b.typing_at() {
        let field = text_input(&b.text_edit, TextStyle::new((20.0 * scale).max(10.0), 500.0))
            .single_line().placeholder("Type, then Enter", if dark { Color::WHITE.alpha(0.4) } else { Color::BLACK.alpha(0.35) })
            .color(if dark { hex("#e6edf3") } else { hex(&b_stroke(b)) })
            .w((320.0 * scale.max(0.6)).min(size[0] - 16.0)).h((30.0 * scale).max(22.0)).px(4.0)
            .border(1.0, accent).radius(4.0).fill(bg)
            .on_edit(access).on_submit(commit).key("board-text");
        board = board.child(row().absolute((at[0] - o[0]) * scale, (at[1] - 14.0 - o[1]) * scale).child(field));
    }
    board
}

fn b_stroke(b: &Board) -> String {
    STROKES[b.color].to_owned()
}
