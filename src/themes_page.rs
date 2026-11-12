//! Settings → Appearance: System / Light / Dark, the theme gallery and the theme editor.
use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, Switch, TextInput};

use crate::ui::{panel, panel_head, rule};
use crate::state::Theme;
use crate::themes::{self, Backdrop, CORNERS, Look, Part, SIZES, STRENGTHS, step};
use crate::{App, icons, ui};

pub fn appearance(s: &App, k: &Kit) -> El {
    let shown_dark = k.dark;
    let mode = row().w_full().px(16.0).py(12.0).gap(16.0).center_y().justify_between().flex_wrap()
        .child(col().grow(1.0).shrink().min_w(200.0).gap(2.0)
            .child(text("Mode").font(k.ty.ui_medium).color(k.c.text_base))
            .child(text("System follows your computer. The sun and moon in the toolbar switch between your light and dark theme.")
                .font(k.ty.text_small).color(k.c.text_muted).wrap()))
        .child(SegmentedControl::new(&Theme::LABELS, Some(s.theme.index())).label("Mode").fit()
            .on_pick(|s: &mut App, i, _| s.set_theme(Theme::ALL[i.min(2)])).build(k).key("theme-pick"));
    let cards = s.looks.all.iter().map(|t| card(s, k, t, shown_dark));
    let gallery = col().w_full().px(16.0).py(12.0).gap(10.0)
        .child(text("Themes").font(k.ty.ui_medium).color(k.c.text_base))
        .child(row().w_full().gap(10.0).flex_wrap().children(cards));
    panel(k).key("appearance").child(panel_head(k, "Appearance", None)).child(rule(k))
        .child(mode).child(rule(k)).child(gallery).child(rule(k))
        .child(editor(s, k, shown_dark))
}

/// A theme in the gallery: a small picture of it, its name and the appearance it's used for.
fn card(s: &App, k: &Kit, t: &Look, shown_dark: bool) -> El {
    let used = if s.looks.light == t.id { Some("Light") } else if s.looks.dark == t.id { Some("Dark") } else { None };
    let on_screen = s.looks.active(shown_dark).id == t.id;
    let c = |p| t.color(p);
    let accent = c(Part::Accent);
    let mut pill = el().w(36.0).h(10.0).radius(5.0).fill(accent);
    if t.glow > 0.0 {
        pill = pill.shadow(0.0, 4.0 + 10.0 * t.glow, accent.alpha(0.35 + 0.4 * t.glow));
    }
    let pattern = if matches!(t.backdrop, Backdrop::Plain | Backdrop::Image) { el() } else {
        themes::backdrop(t, [96.0, 72.0], None)
    };
    let picture = row().w_full().h(72.0).radius(8.0).clip_rounded(8.0).fill(c(Part::Page)).border(1.0, k.c.border_weak)
        .child(col().w(30.0).h_full().fill(c(Part::Sidebar)).padding(6.0).gap(4.0)
            .child(el().w_full().h(4.0).radius(2.0).fill(c(Part::Muted).alpha(0.6)))
            .child(el().w_full().h(4.0).radius(2.0).fill(accent.alpha(0.8)))
            .child(el().w_full().h(4.0).radius(2.0).fill(c(Part::Muted).alpha(0.6))))
        .child(col().grow(1.0).h_full().child(pattern).padding(8.0).gap(6.0)
            .child(col().w_full().padding(6.0).gap(4.0).radius(t.radius.min(8.0) * 0.6).fill(c(Part::Panel)).border(1.0, c(Part::Border))
                .child(el().w(44.0).h(4.0).radius(2.0).fill(c(Part::Text)))
                .child(el().w(30.0).h(4.0).radius(2.0).fill(c(Part::Muted))))
            .child(pill));
    let mut name = row().w_full().gap(6.0).center_y()
        .child(text(t.name.clone()).font(k.ty.small_medium).color(k.c.text_base).truncate().grow(1.0).shrink());
    if let Some(u) = used {
        name = name.child(ui::pill(k, u, if on_screen { k.c.text_accent } else { k.c.text_muted }));
    }
    let id = t.id.clone();
    let box_ = col().w(150.0).padding(6.0).gap(6.0).radius(10.0)
        .border(if on_screen { 2.0 } else { 1.0 }, if on_screen { k.c.bg_accent } else { k.c.border_weak })
        .role("button").label(format!("Use the theme {}", t.name))
        .on_press(move |s: &mut App| s.choose_look(&id, shown_dark))
        .child(picture).child(name);
    k.interactive(box_).key(format!("look-{}", t.id))
}

/// The editor for one of your themes; for a ready-made one, a way to copy it.
fn editor(s: &App, k: &Kit, shown_dark: bool) -> El {
    let body = col().w_full().px(16.0).py(12.0).gap(12.0);
    let Some(t) = s.looks.get(&s.look_edit.id).filter(|t| !t.builtin) else {
        let active = s.looks.active(shown_dark);
        let id = active.id.clone();
        let mut r = row().w_full().gap(12.0).center_y().justify_between().flex_wrap()
            .child(col().grow(1.0).shrink().min_w(200.0).gap(2.0)
                .child(text(format!("Customize {}", active.name)).font(k.ty.ui_medium).color(k.c.text_base))
                .child(text(if active.builtin {
                    "Ready-made themes stay as they are. Duplicate one to choose its colours, background, glow and corners."
                } else {
                    "Open your theme to change it."
                }).font(k.ty.text_small).color(k.c.text_muted).wrap()));
        r = if active.builtin {
            r.child(Button::new("Duplicate to customize").icon(&icons::PLUS).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.duplicate_look(&id, shown_dark)).build(k).key("look-duplicate"))
        } else {
            r.child(Button::new("Edit").control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.edit_look(&id)).build(k).key("look-edit"))
        };
        return body.child(r);
    };
    let id = t.id.clone();
    let (dup, del) = (id.clone(), id.clone());
    let head = row().w_full().gap(8.0).center_y().justify_between().flex_wrap()
        .child(text(format!("Editing {}", t.name)).font(k.ty.ui_medium).color(k.c.text_base).grow(1.0).shrink())
        .child(row().gap(6.0)
            .child(Button::new("Duplicate").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.duplicate_look(&dup, shown_dark)).build(k).key("look-copy"))
            .child(Button::new("Delete").variant(ButtonVariant::Danger).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.delete_look(&del)).build(k).key("look-delete"))
            .child(Button::new("Done").control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.look_edit = crate::LookEditor::default()).build(k).key("look-done")));
    let label = |t: &str| text(t.to_owned()).font(k.ty.small).color(k.c.text_muted).w(190.0).shrink();
    let name = row().w_full().gap(12.0).center_y().flex_wrap().child(label("Name"))
        .child(TextInput::new(&s.look_edit.name).label("Theme name").on_edit(|s: &mut App| &mut s.look_edit.name)
            .on_change(|s: &mut App, _| s.look_typed()).build(k).key("look-name").grow(1.0).min_w(180.0));
    let colors = Part::ALL.iter().map(|p| {
        let i = p.index();
        let edit = &s.look_edit.colors[i];
        let valid = themes::parse_hex(&edit.text).is_some();
        row().w_full().gap(12.0).center_y()
            .child(label(p.label()))
            .child(el().size(24.0, 24.0).radius(6.0).fill(t.color(*p)).border(1.0, k.c.border_base))
            .child(TextInput::new(edit).label(p.label()).placeholder("#rrggbb").invalid(!valid)
                .on_edit(move |s: &mut App| &mut s.look_edit.colors[i])
                .on_change(|s: &mut App, _| s.look_typed()).build(k).key(format!("look-color-{i}")).w(120.0))
            .key(format!("look-color-row-{i}"))
    });
    let seg = |labels: &[&str], at: usize, key: &str, f: fn(&mut Look, usize)| {
        SegmentedControl::new(labels, Some(at)).label(key.to_owned()).fit()
            .on_pick(move |s: &mut App, i, _| s.change_look(|t| f(t, i))).build(k).key(key.to_owned())
    };
    let line = |name: &str, control: El| row().w_full().gap(12.0).center_y().flex_wrap().child(label(name)).child(control);
    let names = |steps: &[(&'static str, f32)]| steps.iter().map(|s| s.0).collect::<Vec<_>>();
    let mut background = col().w_full().gap(10.0)
        .child(line("Background", seg(&Backdrop::LABELS, t.backdrop.index(), "look-backdrop", |t, i| t.backdrop = Backdrop::ALL[i])));
    if t.backdrop == Backdrop::Image {
        background = background.child(line("Picture", TextInput::new(&s.look_edit.image).label("Picture").placeholder("A file on this computer, or https://…")
            .on_edit(|s: &mut App| &mut s.look_edit.image).on_change(|s: &mut App, _| s.look_typed()).build(k).key("look-image").grow(1.0).min_w(220.0)));
    }
    if t.backdrop != Backdrop::Plain {
        background = background.child(line("Strength", seg(&names(&STRENGTHS[1..]), step(&STRENGTHS[1..], t.backdrop_strength), "look-strength", |t, i| t.backdrop_strength = STRENGTHS[i + 1].1)));
    }
    if !matches!(t.backdrop, Backdrop::Plain | Backdrop::Image) {
        background = background.child(line("Pattern size", seg(&names(&SIZES), step(&SIZES, t.backdrop_size), "look-size", |t, i| t.backdrop_size = SIZES[i].1)));
    }
    let effects = col().w_full().gap(10.0)
        .child(line("Glow on buttons", seg(&names(&STRENGTHS), step(&STRENGTHS, t.glow), "look-glow", |t, i| t.glow = STRENGTHS[i].1)))
        .child(line("Corners", seg(&names(&CORNERS), step(&CORNERS, t.radius), "look-corners", |t, i| t.radius = CORNERS[i].1)))
        .child(line("Accent buttons", Switch::new(t.accent_buttons).label("Main buttons in the accent colour")
            .on_toggle(|s: &mut App| s.change_look(|t| t.accent_buttons = !t.accent_buttons)).build(k).key("look-accent-buttons")))
        .child(line("Preview", row().gap(10.0).py(6.0)
            .child(Button::new("Main button").variant(ButtonVariant::Submit).control_size(ControlSize::Small).on_press(|_: &mut App| {}).build(k).key("look-preview-submit"))
            .child(Button::new("Button").control_size(ControlSize::Small).on_press(|_: &mut App| {}).build(k).key("look-preview-neutral"))));
    let section = |title: &str| text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_strong).pt(6.0);
    let note = if t.id == s.looks.active(shown_dark).id { "" } else { "This theme isn't on screen now: pick it in the gallery to see your changes." };
    let mut body = body.child(head);
    if !note.is_empty() {
        body = body.child(text(note).font(k.ty.text_small).color(k.c.warning.icon).wrap());
    }
    body.child(name)
        .child(section("Colours"))
        .children(colors)
        .child(section("Background"))
        .child(background)
        .child(section("Effects"))
        .child(effects)
}
