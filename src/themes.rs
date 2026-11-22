//! Themes: ready-made looks and the ones you make.
//!
//! - A [`Look`] has a colour for each part of the window ([`Part`]), a background (a pattern or
//!   an image, [`Backdrop`]), a glow on buttons, corner roundness and whether the main buttons
//!   take the accent colour.
//! - [`kit`] turns a look into the `rune_kit::Kit` every component reads; [`backdrop`] paints
//!   the background behind the pages.
//! - You pick one look for the light appearance and one for the dark ([`Looks::light`],
//!   [`Looks::dark`]); System / Light / Dark (`state::Theme`) decides which shows, and the
//!   toolbar's sun and moon switch between them.
//! - The ready-made looks ([`builtin`]) can't be changed; *Duplicate* makes an editable copy.
//!   Your looks are saved in the preferences (`themes`, `theme.light`, `theme.dark`).
use rune::{Color, El, LineCap, Shape, canvas, el};
use rune_kit::{Kit, PaletteSeed};
use serde_json::{Value, json};

/// The parts of the window a look colours, in the editor's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Page,
    Sidebar,
    Panel,
    Text,
    Muted,
    Accent,
    Border,
    Pattern,
    Success,
    Warning,
    Danger,
}

impl Part {
    pub const ALL: [Part; 11] = [Part::Page, Part::Sidebar, Part::Panel, Part::Text, Part::Muted, Part::Accent, Part::Border, Part::Pattern, Part::Success, Part::Warning, Part::Danger];

    pub fn index(self) -> usize {
        Part::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::Page => "Page",
            Part::Sidebar => "Sidebar and toolbar",
            Part::Panel => "Panels and cards",
            Part::Text => "Text",
            Part::Muted => "Secondary text and icons",
            Part::Accent => "Accent (links, selection, focus)",
            Part::Border => "Borders",
            Part::Pattern => "Background pattern",
            Part::Success => "Success",
            Part::Warning => "Warning",
            Part::Danger => "Danger",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Part::Page => "page",
            Part::Sidebar => "sidebar",
            Part::Panel => "panel",
            Part::Text => "text",
            Part::Muted => "muted",
            Part::Accent => "accent",
            Part::Border => "border",
            Part::Pattern => "pattern",
            Part::Success => "success",
            Part::Warning => "warning",
            Part::Danger => "danger",
        }
    }
}

/// What's painted behind the pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    #[default]
    Plain,
    Dots,
    Grid,
    Lines,
    Diagonal,
    Waves,
    /// A picture: [`Look::image`], a file on this computer or a web address.
    Image,
}

impl Backdrop {
    pub const ALL: [Backdrop; 7] = [Backdrop::Plain, Backdrop::Dots, Backdrop::Grid, Backdrop::Lines, Backdrop::Diagonal, Backdrop::Waves, Backdrop::Image];
    pub const LABELS: [&'static str; 7] = ["Plain", "Dots", "Grid", "Lines", "Diagonal", "Waves", "Image"];

    pub fn index(self) -> usize {
        Backdrop::ALL.iter().position(|b| *b == self).unwrap_or(0)
    }

    fn parse(s: &str) -> Backdrop {
        Backdrop::ALL.into_iter().find(|b| Backdrop::LABELS[b.index()].eq_ignore_ascii_case(s)).unwrap_or_default()
    }
}

/// Steps the editor offers for the strengths (opacity, glow), the pattern's size and corners.
pub const STRENGTHS: [(&str, f32); 4] = [("Off", 0.0), ("Soft", 0.35), ("Medium", 0.65), ("Strong", 1.0)];
pub const SIZES: [(&str, f32); 3] = [("Small", 16.0), ("Medium", 24.0), ("Large", 40.0)];
pub const CORNERS: [(&str, f32); 3] = [("Square", 4.0), ("Rounded", 10.0), ("Round", 16.0)];

/// The step nearest to `v`.
pub fn step(steps: &[(&str, f32)], v: f32) -> usize {
    steps.iter().enumerate().min_by(|a, b| (a.1.1 - v).abs().total_cmp(&(b.1.1 - v).abs())).map_or(0, |(i, _)| i)
}

/// One theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// Stable id: `classic-dark` for the ready-made ones, `custom-…` for yours.
    pub id: String,
    pub name: String,
    /// A dark look (light text on a dark page); decides which slot it goes to when picked.
    pub dark: bool,
    /// `#rrggbb` for each [`Part`], in `Part::ALL`'s order.
    pub colors: [String; 11],
    pub backdrop: Backdrop,
    /// How strongly the pattern or image shows, 0–1.
    pub backdrop_strength: f32,
    /// The pattern's spacing in points.
    pub backdrop_size: f32,
    /// The picture for [`Backdrop::Image`]: a path or an `https://` address.
    pub image: String,
    /// The glow around buttons, 0 (none) to 1.
    pub glow: f32,
    /// Corner radius of panels, in points.
    pub radius: f32,
    /// Main buttons (Connect, Create, Install…) take the accent colour.
    pub accent_buttons: bool,
    /// A ready-made look, which can't be changed.
    pub builtin: bool,
}

impl Look {
    pub fn color(&self, p: Part) -> Color {
        rune::color::hex(&self.colors[p.index()]).unwrap_or(if self.dark { Color::WHITE } else { Color::BLACK })
    }

    pub fn to_json(&self) -> Value {
        let colors: serde_json::Map<String, Value> = Part::ALL.iter().map(|p| (p.key().to_owned(), Value::from(self.colors[p.index()].clone()))).collect();
        json!({
            "id": self.id,
            "name": self.name,
            "dark": self.dark,
            "colors": colors,
            "backdrop": Backdrop::LABELS[self.backdrop.index()].to_lowercase(),
            "backdrop_strength": self.backdrop_strength,
            "backdrop_size": self.backdrop_size,
            "image": self.image,
            "glow": self.glow,
            "radius": self.radius,
            "accent_buttons": self.accent_buttons,
        })
    }

    /// A saved look; missing colours come from the classic look of the same kind.
    pub fn from_json(v: &Value) -> Option<Look> {
        let id = v.get("id")?.as_str()?.to_owned();
        let dark = v.get("dark").and_then(Value::as_bool).unwrap_or(true);
        let base = classic(dark);
        let num = |k: &str, d: f32| v.get(k).and_then(Value::as_f64).map_or(d, |n| n as f32);
        let mut colors = base.colors.clone();
        for p in Part::ALL {
            if let Some(c) = v.get("colors").and_then(|c| c.get(p.key())).and_then(Value::as_str).filter(|c| rune::color::hex(c).is_some()) {
                colors[p.index()] = c.to_owned();
            }
        }
        Some(Look {
            id,
            name: v.get("name").and_then(Value::as_str).unwrap_or("Untitled").to_owned(),
            dark,
            colors,
            backdrop: Backdrop::parse(v.get("backdrop").and_then(Value::as_str).unwrap_or("")),
            backdrop_strength: num("backdrop_strength", 0.35).clamp(0.0, 1.0),
            backdrop_size: num("backdrop_size", 24.0).clamp(8.0, 96.0),
            image: v.get("image").and_then(Value::as_str).unwrap_or("").to_owned(),
            glow: num("glow", 0.0).clamp(0.0, 1.0),
            radius: num("radius", 10.0).clamp(0.0, 24.0),
            accent_buttons: v.get("accent_buttons").and_then(Value::as_bool).unwrap_or(false),
            builtin: false,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn look(id: &str, name: &str, dark: bool, colors: [&str; 11], backdrop: Backdrop, strength: f32, size: f32, glow: f32, radius: f32, accent_buttons: bool) -> Look {
    Look {
        id: id.into(),
        name: name.into(),
        dark,
        colors: colors.map(String::from),
        backdrop,
        backdrop_strength: strength,
        backdrop_size: size,
        image: String::new(),
        glow,
        radius,
        accent_buttons,
        builtin: true,
    }
}

/// RepoHub's original look, light or dark.
fn classic(dark: bool) -> Look {
    if dark {
        //        page       sidebar    panel      text       muted      accent     border     pattern    success    warning    danger
        look("classic-dark", "Classic Dark", true, ["#0d1117", "#0d1117", "#0d1117", "#e6edf3", "#a2a7b0", "#2f81f7", "#292d34", "#a2a7b0", "#5dc879", "#dfa11a", "#ff847d"], Backdrop::Plain, 0.35, 24.0, 0.0, 10.0, false)
    } else {
        look("classic-light", "Classic Light", false, ["#f6f8fa", "#f6f8fa", "#f6f8fa", "#1f2328", "#6e7379", "#0969da", "#cbd1d8", "#6e7379", "#00792f", "#8e5500", "#a83634"], Backdrop::Plain, 0.35, 24.0, 0.0, 10.0, false)
    }
}

/// The ready-made looks.
pub fn builtin() -> Vec<Look> {
    vec![
        classic(false),
        classic(true),
        look("midnight", "Midnight", true, ["#0a1020", "#070b17", "#0f1729", "#e2e8f5", "#8d9ab5", "#38bdf8", "#1e2a44", "#38bdf8", "#4ade80", "#fbbf24", "#f87171"], Backdrop::Dots, 0.35, 24.0, 0.65, 12.0, true),
        look("neon", "Neon", true, ["#0e0716", "#0a0510", "#150c22", "#f3e8ff", "#a78bba", "#f0abfc", "#2e1a45", "#c026d3", "#5eead4", "#fde047", "#fb7185"], Backdrop::Grid, 0.35, 40.0, 1.0, 12.0, true),
        look("forest", "Forest", true, ["#0f1612", "#0b110e", "#141e18", "#e3efe6", "#93a89a", "#4ade80", "#24332a", "#4ade80", "#86efac", "#facc15", "#f87171"], Backdrop::Diagonal, 0.35, 24.0, 0.35, 10.0, false),
        look("ember", "Ember", true, ["#151110", "#100d0c", "#1c1715", "#f4ebe6", "#ab9d95", "#fb923c", "#33291f", "#fb923c", "#a3e635", "#fbbf24", "#f87171"], Backdrop::Waves, 0.35, 24.0, 0.35, 10.0, true),
        look("paper", "Paper", false, ["#f6f1e7", "#efe8da", "#fbf8f1", "#2b2620", "#7a6f62", "#b4532a", "#ddd3c2", "#7a6f62", "#3f7d3a", "#9a6b00", "#b3261e"], Backdrop::Dots, 0.35, 24.0, 0.0, 6.0, true),
        look("ocean", "Ocean", false, ["#eef6fb", "#e3eef6", "#f8fbfd", "#10242f", "#5a7180", "#0e7490", "#c7d9e4", "#0e7490", "#047857", "#a16207", "#be123c"], Backdrop::Waves, 0.35, 40.0, 0.35, 12.0, true),
        look("mono", "Mono", false, ["#f5f5f5", "#ededed", "#fcfcfc", "#171717", "#6b6b6b", "#171717", "#d4d4d4", "#a3a3a3", "#15803d", "#a16207", "#b91c1c"], Backdrop::Grid, 0.35, 24.0, 0.0, 4.0, false),
    ]
}

/// All looks and which one each appearance uses.
#[derive(Clone, Debug)]
pub struct Looks {
    /// The ready-made ones, then yours.
    pub all: Vec<Look>,
    pub light: String,
    pub dark: String,
}

impl Default for Looks {
    fn default() -> Self {
        Looks { all: builtin(), light: "classic-light".into(), dark: "classic-dark".into() }
    }
}

impl Looks {
    /// The saved looks and choices.
    pub fn load() -> Looks {
        let mut l = Looks::default();
        if let Some(list) = crate::prefs::get("themes").and_then(|v| v.as_array().cloned()) {
            l.all.extend(list.iter().filter_map(Look::from_json).filter(|t| !t.id.is_empty() && !t.id.starts_with("classic-")));
        }
        let pick = |key: &str| crate::prefs::get(key).and_then(|v| v.as_str().map(str::to_owned));
        if let Some(id) = pick("theme.light").filter(|id| l.get(id).is_some()) {
            l.light = id;
        }
        if let Some(id) = pick("theme.dark").filter(|id| l.get(id).is_some()) {
            l.dark = id;
        }
        l
    }

    /// Saves your looks and the choices.
    pub fn save(&self) {
        crate::prefs::set("themes", Value::from(self.all.iter().filter(|t| !t.builtin).map(Look::to_json).collect::<Vec<_>>()));
        crate::prefs::set("theme.light", Value::from(self.light.clone()));
        crate::prefs::set("theme.dark", Value::from(self.dark.clone()));
    }

    pub fn get(&self, id: &str) -> Option<&Look> {
        self.all.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Look> {
        self.all.iter_mut().find(|t| t.id == id && !t.builtin)
    }

    /// The look for the appearance on screen.
    pub fn active(&self, dark: bool) -> &Look {
        self.get(if dark { &self.dark } else { &self.light }).unwrap_or_else(|| &self.all[if dark { 1 } else { 0 }])
    }

    /// Uses `id` for the appearance it is made for; returns whether it's dark.
    pub fn choose(&mut self, id: &str) -> Option<bool> {
        let dark = self.get(id)?.dark;
        if dark { self.dark = id.to_owned() } else { self.light = id.to_owned() }
        Some(dark)
    }

    /// An editable copy of `id`, chosen in its place; returns the new id.
    pub fn duplicate(&mut self, id: &str) -> Option<String> {
        let mut copy = self.get(id)?.clone();
        let n = (1..).find(|n| self.get(&format!("custom-{n}")).is_none()).unwrap_or(1);
        copy.id = format!("custom-{n}");
        copy.name = format!("{} (copy)", copy.name.trim_end_matches(" (copy)"));
        copy.builtin = false;
        self.all.push(copy.clone());
        self.choose(&copy.id);
        Some(copy.id)
    }

    /// Removes one of your looks; an appearance that used it goes back to Classic.
    pub fn delete(&mut self, id: &str) -> bool {
        let Some(i) = self.all.iter().position(|t| t.id == id && !t.builtin) else { return false };
        self.all.remove(i);
        if self.light == id {
            self.light = "classic-light".into();
        }
        if self.dark == id {
            self.dark = "classic-dark".into();
        }
        true
    }
}

/// The kit every component reads, from `look`.
pub fn kit(look: &Look) -> Kit {
    let col = |p| look.color(p);
    let accent = col(Part::Accent);
    let mut k = Kit::from_palette(PaletteSeed { bg: col(Part::Page), fg: col(Part::Text), accent });
    let c = &mut k.c;
    c.page = col(Part::Page);
    c.bg_deep = col(Part::Sidebar);
    c.bg_base = col(Part::Panel);
    c.text_base = col(Part::Text);
    c.text_strong = col(Part::Text);
    c.text_muted = col(Part::Muted);
    c.icon_muted = col(Part::Muted);
    c.bg_accent = accent;
    c.text_accent = accent;
    c.icon_accent = accent;
    c.border_weak = col(Part::Border);
    c.success.icon = col(Part::Success);
    c.warning.icon = col(Part::Warning);
    c.danger.icon = col(Part::Danger);
    if look.accent_buttons {
        c.bg_icon_button_contrast = accent;
    }
    let ink = contrast_ink(c.bg_icon_button_contrast);
    c.submit_fg = ink;
    c.submit_icon = ink;
    let r = look.radius;
    k.radius.lg = r;
    k.radius.xl = r;
    k.radius.md = (r * 0.6).max(2.0);
    k.radius.sm = (r * 0.4).max(2.0);
    if look.glow > 0.0 {
        let g = look.glow;
        let glow = |a: f32| rune::Shadow { x: 0.0, y: 0.0, blur: 6.0 + 18.0 * g, spread: 0.0, color: accent.alpha(a * (0.25 + 0.5 * g)), inset: false };
        k.elevation.button_contrast.push(glow(1.0));
        k.elevation.button_neutral.push(glow(0.45));
    }
    k
}

/// WCAG 2.1 minimum contrast for normal text (`4.5:1`) and large text or graphical marks (`3:1`).
pub const MIN_CONTRAST: f32 = 4.5;
pub const MIN_LARGE_CONTRAST: f32 = 3.0;

/// `fg` (possibly translucent) composited over opaque `bg` in linear sRGB.
pub fn blend_over(fg: Color, bg: Color) -> Color {
    let [r, g, b, _] = rune::color::over(fg.premul(1.0), bg.alpha(1.0).premul(1.0));
    Color { r, g, b, a: 1.0 }
}

/// WCAG 2.1 contrast ratio between two colours (`1.0` to `21.0`).
pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Whether `contrast(a, b)` reaches `min` at the one-decimal precision shown in the editor.
pub fn meets_contrast(a: Color, b: Color, min: f32) -> bool {
    (contrast(a, b) * 10.0).round() >= (min * 10.0).round()
}

/// Black or white, whichever gives higher WCAG contrast against `bg`.
pub fn contrast_ink(bg: Color) -> Color {
    if contrast(bg, Color::BLACK) >= contrast(bg, Color::WHITE) { Color::BLACK } else { Color::WHITE }
}

/// A colour pair in a [`Look`] whose contrast falls below its WCAG target.
#[derive(Clone, Debug, PartialEq)]
pub struct ContrastWarning {
    pub pair: &'static str,
    pub ratio: f32,
    pub min: f32,
}

impl ContrastWarning {
    pub fn message(&self) -> String {
        format!("Low contrast {:.1}:1, aim for {:.1}:1", self.ratio, self.min)
    }
}

fn check_pairs(fg: Color, checks: &[(&'static str, Color, f32)]) -> Option<ContrastWarning> {
    checks.iter()
        .filter(|(_, bg, min)| !meets_contrast(fg, *bg, *min))
        .map(|(pair, bg, min)| ContrastWarning { pair, ratio: contrast(fg, *bg), min: *min })
        .min_by(|a, b| a.ratio.total_cmp(&b.ratio))
}

/// The backgrounds and minimum contrast a colour [`Part`] is checked against in the editor.
fn part_targets(look: &Look, part: Part) -> Vec<(&'static str, Color, f32)> {
    match part {
        Part::Text => vec![
            ("Text/Page", look.color(Part::Page), MIN_CONTRAST),
            ("Text/Panel", look.color(Part::Panel), MIN_CONTRAST),
        ],
        Part::Muted => vec![("Secondary/Panel", look.color(Part::Panel), MIN_CONTRAST)],
        Part::Accent => vec![("Accent/Panel", look.color(Part::Panel), MIN_CONTRAST)],
        Part::Success => vec![("Success/Panel", look.color(Part::Panel), MIN_LARGE_CONTRAST)],
        Part::Warning => vec![("Warning/Panel", look.color(Part::Panel), MIN_LARGE_CONTRAST)],
        Part::Danger => vec![("Danger/Panel", look.color(Part::Panel), MIN_LARGE_CONTRAST)],
        _ => Vec::new(),
    }
}

/// The worst contrast failure for `part` in `look`, if any.
pub fn part_contrast_warning(look: &Look, part: Part) -> Option<ContrastWarning> {
    let targets = part_targets(look, part);
    if targets.is_empty() {
        return None;
    }
    check_pairs(look.color(part), &targets)
}

/// Contrast warning for the main buttons (`bg_icon_button_contrast` vs `submit_fg`, and when
/// **Accent buttons** is on, the button fill against the panel at 3:1).
pub fn button_contrast_warning(look: &Look) -> Option<ContrastWarning> {
    let k = kit(look);
    let mut checks = vec![("Accent buttons/ink", k.c.submit_fg, MIN_CONTRAST)];
    if look.accent_buttons {
        checks.push(("Accent buttons/Panel", look.color(Part::Panel), MIN_LARGE_CONTRAST));
    }
    check_pairs(k.c.bg_icon_button_contrast, &checks)
}

/// Every failing contrast check in `look` (empty when the theme passes).
#[cfg(test)]
pub fn contrast_warnings(look: &Look) -> Vec<ContrastWarning> {
    let mut out = Vec::new();
    for p in Part::ALL {
        for (pair, bg, min) in part_targets(look, p) {
            let fg = look.color(p);
            if !meets_contrast(fg, bg, min) {
                out.push(ContrastWarning { pair, ratio: contrast(fg, bg), min });
            }
        }
    }
    if let Some(w) = button_contrast_warning(look) {
        out.push(w);
    }
    out
}

fn hex6(c: Color) -> String {
    let h = c.hex();
    h[..7].to_owned()
}

/// Moves `part`'s lightness in OKLab (keeping hue and as much chroma as sRGB allows) until it
/// passes all of its contrast targets in `look`, returning the new `#rrggbb` colour.
pub fn fix_part_contrast(look: &mut Look, part: Part) -> Option<String> {
    let targets = part_targets(look, part);
    if targets.is_empty() {
        return None;
    }
    let fg = look.color(part);
    if targets.iter().all(|(_, bg, min)| meets_contrast(fg, *bg, *min)) {
        return None;
    }
    let [l0, a, b, _] = fg.to_oklab();
    // Aim slightly above `min` first so `#rrggbb` rounding stays clear of the boundary; fall back
    // to exact `meets_contrast` when two extreme backgrounds (black + white) leave a narrow band.
    for margin in [0.06, 0.0] {
        for scale in [1.0, 0.75, 0.5, 0.25, 0.0] {
            let mut best: Option<(f32, String)> = None;
            for step in 0..=512 {
                let l = step as f32 / 512.0;
                let cand_hex = hex6(Color::from_oklab([l, a * scale, b * scale, 1.0]));
                let Some(cand) = rune::color::hex(&cand_hex) else { continue };
                let ok = if margin > 0.0 {
                    targets.iter().all(|(_, bg, min)| contrast(cand, *bg) >= *min + margin)
                } else {
                    targets.iter().all(|(_, bg, min)| meets_contrast(cand, *bg, *min))
                };
                if !ok {
                    continue;
                }
                // Prefer the smaller lightness jump; break ties toward light ink on dark themes.
                let dir_bias = if (look.dark && l >= l0) || (!look.dark && l <= l0) { 0.0 } else { 0.001 };
                let dist = (l - l0).abs() + dir_bias;
                if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                    best = Some((dist, cand_hex));
                }
            }
            if let Some((_, chosen)) = best {
                look.colors[part.index()] = chosen.clone();
                return Some(chosen);
            }
        }
    }
    None
}

/// `#rgb` or `#rrggbb` (the `#` optional) as `#rrggbb`; `None` when it isn't a colour.
pub fn parse_hex(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#').to_ascii_lowercase();
    let h = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h,
        _ => return None,
    };
    h.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{h}"))
}

/// The background behind the pages, `size` points (the page area). `image` is the picture, when
/// the look has one and it has loaded.
pub fn backdrop(look: &Look, size: [f32; 2], image: Option<&rune::Pixels>) -> El {
    let strength = look.backdrop_strength;
    let layer = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if strength <= 0.0 {
        return layer;
    }
    match look.backdrop {
        Backdrop::Plain => layer,
        Backdrop::Image => match image {
            Some(px) => layer.pixels_fit(px, rune::SurfaceFit::Fill).opacity(0.12 + 0.6 * strength),
            None => layer,
        },
        pattern => {
            let key = format!("{:?}{}{}{}{}{}", pattern, look.colors[Part::Pattern.index()], strength, look.backdrop_size, size[0] as i32 / 64, size[1] as i32 / 64);
            let look = look.clone();
            layer.child(rune::memo("backdrop", key, move || {
                let w = (size[0] / 64.0).ceil() * 64.0 + 64.0;
                let h = (size[1] / 64.0).ceil() * 64.0 + 64.0;
                canvas(pattern_shape(&look, pattern, w, h)).size(w, h)
            }))
        }
    }
}

/// The strokes of a pattern over `w` × `h` points.
fn pattern_shape(look: &Look, pattern: Backdrop, w: f32, h: f32) -> Shape {
    let gap = look.backdrop_size.max(8.0);
    let ink = look.color(Part::Pattern).alpha(0.05 + 0.25 * look.backdrop_strength);
    let n = |len: f32| (len / gap).ceil() as usize + 1;
    let mut s = Shape::new();
    match pattern {
        Backdrop::Dots => {
            let d = (gap / 10.0).clamp(1.5, 3.0);
            for y in 0..n(h) {
                for x in 0..n(w) {
                    let p = [x as f32 * gap + gap / 2.0, y as f32 * gap + gap / 2.0];
                    s = s.line(p, [p[0] + 0.01, p[1]], d, LineCap::Round, ink);
                }
            }
        }
        Backdrop::Grid | Backdrop::Lines => {
            for y in 0..n(h) {
                s = s.line([0.0, y as f32 * gap], [w, y as f32 * gap], 1.0, LineCap::Butt, ink);
            }
            if pattern == Backdrop::Grid {
                for x in 0..n(w) {
                    s = s.line([x as f32 * gap, 0.0], [x as f32 * gap, h], 1.0, LineCap::Butt, ink);
                }
            }
        }
        Backdrop::Diagonal => {
            for i in 0..n(w + h) {
                let o = i as f32 * gap;
                s = s.line([o, 0.0], [o - h, h], 1.0, LineCap::Butt, ink);
            }
        }
        Backdrop::Waves => {
            let amp = gap / 4.0;
            for y in 0..n(h) {
                let base = y as f32 * gap;
                let pts: Vec<[f32; 2]> = (0..=(w / 4.0) as usize).map(|i| {
                    let x = i as f32 * 4.0;
                    [x, base + (x / gap * std::f32::consts::PI).sin() * amp]
                }).collect();
                s = s.stroke(pts, 1.2, ink);
            }
        }
        Backdrop::Plain | Backdrop::Image => {}
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_round_trip_and_choose() {
        let mut l = Looks::default();
        assert_eq!(l.active(true).id, "classic-dark");
        assert_eq!(l.choose("neon"), Some(true));
        assert_eq!(l.active(true).id, "neon");
        assert_eq!(l.active(false).id, "classic-light", "a dark look leaves the light slot");
        // Ready-made looks can't be edited; a copy can.
        assert!(l.get_mut("neon").is_none());
        let id = l.duplicate("neon").unwrap();
        assert_eq!(l.active(true).id, id);
        let t = l.get_mut(&id).unwrap();
        t.colors[Part::Accent.index()] = "#ff0000".into();
        t.backdrop = Backdrop::Image;
        t.image = "/tmp/x.png".into();
        let back = Look::from_json(&l.get(&id).unwrap().to_json()).unwrap();
        assert_eq!(back.colors, l.get(&id).unwrap().colors);
        assert_eq!((back.backdrop, back.image.as_str(), back.builtin), (Backdrop::Image, "/tmp/x.png", false));
        assert_eq!(back.name, "Neon (copy)");
        // Deleting it puts Classic back.
        assert!(l.delete(&id));
        assert_eq!(l.active(true).id, "classic-dark");
        assert!(!l.delete("neon"));
    }

    #[test]
    fn bad_saved_values_fall_back() {
        let v = json!({"id": "custom-9", "dark": false, "colors": {"accent": "nope", "page": "#ffffff"}, "glow": 7, "backdrop": "stripes"});
        let t = Look::from_json(&v).unwrap();
        assert_eq!(t.colors[Part::Accent.index()], classic(false).colors[Part::Accent.index()]);
        assert_eq!(t.colors[Part::Page.index()], "#ffffff");
        assert_eq!((t.glow, t.backdrop), (1.0, Backdrop::Plain));
    }

    #[test]
    fn classic_matches_the_original_kit() {
        for dark in [false, true] {
            let k = kit(&classic(dark));
            let seed = if dark { ("#0d1117", "#e6edf3", "#2f81f7") } else { ("#f6f8fa", "#1f2328", "#0969da") };
            let c = |h: &str| rune::color::hex(h).unwrap();
            let original = Kit::from_palette(PaletteSeed { bg: c(seed.0), fg: c(seed.1), accent: c(seed.2) });
            assert_eq!(k.c.text_muted.hex(), original.c.text_muted.hex());
            assert_eq!(k.c.border_weak.hex(), original.c.border_weak.hex());
            assert_eq!(k.c.danger.icon.hex(), original.c.danger.icon.hex());
            assert_eq!(k.elevation.button_contrast.len(), original.elevation.button_contrast.len(), "no glow");
        }
        assert_eq!(step(&STRENGTHS, 0.4), 1);
        assert_eq!(step(&SIZES, 40.0), 2);
    }

    #[test]
    fn every_builtin_theme_and_label_chip_passes_contrast_checks() {
        for look in builtin() {
            let warnings = contrast_warnings(&look);
            assert!(warnings.is_empty(), "{} failed contrast: {warnings:?}", look.id);
            let k = kit(&look);
            assert!(
                contrast(k.c.bg_icon_button_contrast, k.c.submit_fg) >= MIN_CONTRAST,
                "{} main button ink contrast too low",
                look.id
            );
            for step in 0..24 {
                let hue = step as f32 * 15.0;
                let (ink, fill, _) = crate::ui::label_chip_colors(&k, hue);
                let bg = blend_over(fill, k.c.bg_base);
                let ratio = contrast(ink, bg);
                assert!(
                    ratio >= MIN_CONTRAST,
                    "{} label chip at hue {hue} has {ratio:.2}:1 (< {MIN_CONTRAST})",
                    look.id
                );
            }
        }
    }

    #[test]
    fn button_ink_uses_contrast_even_without_accent_buttons() {
        let mut custom = classic(false);
        custom.builtin = false;
        // Text and Page have almost identical mid-grey luminance, and accent_buttons is off.
        custom.colors[Part::Page.index()] = "#707070".into();
        custom.colors[Part::Text.index()] = "#767676".into();
        custom.accent_buttons = false;
        let k = kit(&custom);
        assert!(contrast(k.c.bg_icon_button_contrast, k.c.submit_fg) >= MIN_CONTRAST);
    }

    #[test]
    fn fix_part_contrast_resolves_low_contrast_colours() {
        for dark in [false, true] {
            let mut t = classic(dark);
            t.builtin = false;
            t.colors[Part::Text.index()] = t.colors[Part::Page.index()].clone();
            t.colors[Part::Muted.index()] = t.colors[Part::Panel.index()].clone();
            t.colors[Part::Accent.index()] = t.colors[Part::Panel.index()].clone();
            t.colors[Part::Warning.index()] = t.colors[Part::Panel.index()].clone();

            let w = part_contrast_warning(&t, Part::Text).expect("Text == Page fails");
            assert_eq!(w.message(), "Low contrast 1.0:1, aim for 4.5:1");
            let w_warn = part_contrast_warning(&t, Part::Warning).expect("Warning == Panel fails");
            assert_eq!(w_warn.message(), "Low contrast 1.0:1, aim for 3.0:1");

            for part in [Part::Text, Part::Muted, Part::Accent, Part::Warning] {
                assert!(fix_part_contrast(&mut t, part).is_some(), "{part:?} fixed");
                assert_eq!(part_contrast_warning(&t, part), None, "{part:?} passes after fix");
            }
            assert!(contrast_warnings(&t).is_empty());
        }

        // Split Page/Panel extremes (black page, white panel) still find a readable Text colour.
        let mut split = classic(true);
        split.builtin = false;
        split.colors[Part::Page.index()] = "#000000".into();
        split.colors[Part::Panel.index()] = "#ffffff".into();
        split.colors[Part::Text.index()] = "#ffffff".into();
        assert!(part_contrast_warning(&split, Part::Text).is_some());
        assert!(fix_part_contrast(&mut split, Part::Text).is_some());
        assert_eq!(part_contrast_warning(&split, Part::Text), None);
    }
}
