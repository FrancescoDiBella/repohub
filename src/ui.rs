//! Small building blocks shared by the pages.
use rune::{Color, El, Icon, TextStyle, col, el, icon, oklch, row, text};

pub const H1: TextStyle = TextStyle::new(24.0, 650.0);
pub const H2: TextStyle = TextStyle::new(21.0, 620.0);
use rune_kit::Kit;

use crate::model::{Checks, PullState};
use crate::icons;

/// A rounded panel on the page background.
pub fn panel(k: &Kit) -> El {
    col().w_full().radius(k.radius.lg).fill(k.c.bg_base).border(1.0, k.c.border_weak)
}

/// A panel's title row: a label on the left, anything on the right.
pub fn panel_head(k: &Kit, title: &str, trailing: Option<El>) -> El {
    let mut r = row().w_full().h(44.0).px(16.0).center_y().justify_between()
        .child(text(title.to_owned()).font(k.ty.ui_medium).color(k.c.text_base));
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

pub fn rule(k: &Kit) -> El {
    el().w_full().h(1.0).fill(k.c.border_weak)
}

pub fn ico(i: &'static Icon, size: f32, c: Color) -> El {
    icon(i, size).color(c)
}

/// The language swatch and name of a repository.
pub fn language(k: &Kit, name: &str, hue: f32) -> El {
    row().gap(6.0).center_y()
        .child(el().size(10.0, 10.0).radius(5.0).fill(oklch(0.68, 0.15, hue)))
        .child(text(name.to_owned()).font(k.ty.text_small).color(k.c.text_muted))
}

/// An icon followed by a number or short label, in muted ink.
pub fn stat(k: &Kit, i: &'static Icon, label: impl Into<String>) -> El {
    row().gap(4.0).center_y()
        .child(ico(i, 14.0, k.c.icon_muted))
        .child(text(label.into()).font(k.ty.text_small).color(k.c.text_muted))
}

/// A rounded tag in the accent tint, as GitHub shows topics.
pub fn topic(k: &Kit, name: &str) -> El {
    text(name.to_owned()).font(k.ty.text_xsmall_medium).color(k.c.text_accent)
        .px(9.0).py(3.0).radius(10.0).fill(k.c.bg_accent.alpha(0.14))
}

/// A coloured label chip (issue labels).
pub fn label_chip(k: &Kit, name: &str, hue: f32) -> El {
    text(name.to_owned()).font(k.ty.text_xsmall_medium).color(oklch(0.72, 0.13, hue))
        .px(8.0).py(2.0).radius(10.0).fill(oklch(0.62, 0.12, hue).alpha(0.16)).border(1.0, oklch(0.62, 0.12, hue).alpha(0.45))
}

/// A square avatar with the first letter of `name`.
pub fn avatar(k: &Kit, name: &str, hue: f32, size: f32) -> El {
    let initial = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_ascii_uppercase().to_string();
    row().size(size, size).radius(size * 0.28).center().fill(oklch(0.6, 0.13, hue))
        .child(text(initial).font(if size > 24.0 { k.ty.body_medium } else { k.ty.small_medium }).color(Color::WHITE))
}

/// A round user avatar.
pub fn user(k: &Kit, name: &str, size: f32) -> El {
    let hue = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360;
    avatar(k, name, hue as f32, size).radius(size / 2.0)
}

/// The CI state as a distinct, labelled icon.
pub fn checks(k: &Kit, c: Checks) -> El {
    match c {
        Checks::Passing => ico(&icons::CHECK, 14.0, k.c.success.icon).label("CI passing"),
        Checks::Failing => ico(&icons::CROSS, 14.0, k.c.danger.icon).label("CI failing"),
        Checks::Running => ico(&icons::DOT, 14.0, k.c.warning.icon).label("CI running"),
    }
}

/// The pull request state icon, with a distinct shape and accessible label for each state.
pub fn pull_icon(k: &Kit, s: PullState) -> El {
    match s {
        PullState::Open => ico(&icons::PULL, 16.0, k.c.success.icon).label("Pull request open"),
        PullState::Draft => ico(&icons::PULL_DRAFT, 16.0, k.c.icon_faint).label("Draft pull request"),
        PullState::Merged => ico(&icons::MERGE, 16.0, oklch(0.62, 0.17, 300.0)).label("Pull request merged"),
        PullState::Closed => ico(&icons::PULL_CLOSED, 16.0, k.c.danger.icon).label("Pull request closed"),
    }
}

/// A rounded pill with text, for states such as "Public" or "Draft".
pub fn pill(k: &Kit, label: &str, ink: Color) -> El {
    text(label.to_owned()).font(k.ty.text_xsmall_medium).color(ink)
        .px(8.0).py(2.0).radius(10.0).border(1.0, ink.alpha(0.5))
}

/// Weekly commit bars, newest on the right.
pub fn sparkline(k: &Kit, values: &[u32], height: f32) -> El {
    let max = values.iter().copied().max().unwrap_or(1).max(1) as f32;
    let n = values.len();
    let bars = values.iter().enumerate().map(|(i, v)| {
        let h = (*v as f32 / max * height).max(2.0);
        let a = 0.35 + 0.65 * (i as f32 / (n.max(2) - 1) as f32);
        col().grow(1.0).h(height).justify_end()
            .child(el().w_full().h(h).radius(2.0).fill(k.c.success.icon.alpha(a)))
    });
    row().w_full().h(height).gap(3.0).children(bars)
}

/// A big number with a caption under it.
pub fn metric(k: &Kit, i: &'static Icon, value: &str, caption: &str, tint: Color) -> El {
    panel(k).grow(1.0).min_w(150.0).padding(16.0).gap(10.0)
        .child(row().size(30.0, 30.0).radius(8.0).center().fill(tint.alpha(0.15)).child(ico(i, 16.0, tint)))
        .child(text(value.to_owned()).font(H2).color(k.c.text_strong))
        .child(text(caption.to_owned()).font(k.ty.text_small).color(k.c.text_muted))
}

/// A metric card whose count is still loading.
pub fn metric_skeleton(k: &Kit, i: &'static Icon, caption: &str, tint: Color) -> El {
    panel(k).grow(1.0).min_w(150.0).padding(16.0).gap(10.0)
        .child(row().size(30.0, 30.0).radius(8.0).center().fill(tint.alpha(0.15)).child(ico(i, 16.0, tint)))
        .child(row().w_full().h(26.0).center_y().child(skeleton_bar(k, 52.0, 22.0)))
        .child(text(caption.to_owned()).font(k.ty.text_small).color(k.c.text_muted))
}

/// A primary grey placeholder bar that shrinks on narrow screens.
pub fn skeleton_bar(k: &Kit, w: f32, h: f32) -> El {
    el().w(w).h(h).shrink().radius((h * 0.38).clamp(3.0, 6.0)).fill(k.c.text_muted.alpha(0.20))
}

/// A fainter grey placeholder bar for secondary metadata.
pub fn skeleton_faint(k: &Kit, w: f32, h: f32) -> El {
    el().w(w).h(h).shrink().radius((h * 0.38).clamp(3.0, 6.0)).fill(k.c.text_muted.alpha(0.12))
}

/// Placeholder bars for the weekly commit chart while data loads.
pub fn sparkline_skeleton(k: &Kit, height: f32) -> El {
    const HEIGHTS: [f32; 26] = [
        0.22, 0.35, 0.28, 0.48, 0.30, 0.18, 0.42, 0.58, 0.36, 0.26, 0.44, 0.65, 0.38,
        0.24, 0.52, 0.40, 0.30, 0.62, 0.46, 0.32, 0.54, 0.72, 0.48, 0.36, 0.60, 0.44,
    ];
    let bars = HEIGHTS.into_iter().map(|f| {
        col().grow(1.0).h(height).justify_end()
            .child(el().w_full().h((f * height).max(4.0)).radius(2.0).fill(k.c.text_muted.alpha(0.14)))
    });
    row().w_full().h(height).gap(3.0).children(bars)
}

/// One placeholder list row (pull request, issue, workflow run, or compact repository).
pub fn skeleton_row(k: &Kit, title_w: f32, meta_w: f32, trailing: bool) -> El {
    let mut r = row().w_full().px(16.0).py(12.0).gap(12.0).center_y()
        .child(el().size(16.0, 16.0).radius(8.0).fill(k.c.text_muted.alpha(0.16)))
        .child(col().grow(1.0).shrink().gap(6.0)
            .child(row().w_full().child(skeleton_bar(k, title_w, 13.0)))
            .child(row().w_full().gap(8.0).child(skeleton_faint(k, meta_w, 10.0)).child(skeleton_faint(k, 52.0, 10.0))));
    if trailing {
        r = r.child(skeleton_faint(k, 56.0, 18.0).radius(9.0));
    }
    r
}

/// Appends `count` placeholder rows (each preceded by a rule) under a panel header.
pub fn skeleton_rows_in_panel(k: &Kit, mut p: El, count: usize, trailing: bool) -> El {
    const WIDTHS: [(f32, f32); 5] = [(230.0, 125.0), (185.0, 105.0), (265.0, 140.0), (205.0, 115.0), (245.0, 130.0)];
    for i in 0..count {
        let (tw, mw) = WIDTHS[i % WIDTHS.len()];
        p = p.child(rule(k)).child(skeleton_row(k, tw, mw, trailing));
    }
    p
}

/// A standalone panel of placeholder rows while a list loads.
pub fn skeleton_list(k: &Kit, count: usize, key: &str) -> El {
    const WIDTHS: [(f32, f32); 5] = [(260.0, 140.0), (200.0, 115.0), (290.0, 155.0), (225.0, 125.0), (250.0, 135.0)];
    let mut p = panel(k).key(key.to_owned()).role("status").label("Loading…");
    for i in 0..count {
        if i > 0 {
            p = p.child(rule(k));
        }
        let (tw, mw) = WIDTHS[i % WIDTHS.len()];
        p = p.child(skeleton_row(k, tw, mw, true));
    }
    p
}

/// A placeholder repository card while the Repositories list loads.
pub fn skeleton_repo_card(k: &Kit, n: usize) -> El {
    const WIDTHS: [(f32, f32); 4] = [(170.0, 320.0), (210.0, 270.0), (150.0, 350.0), (195.0, 290.0)];
    let (tw, dw) = WIDTHS[n % WIDTHS.len()];
    panel(k).padding(16.0).gap(10.0).key(format!("skeleton-card-{n}"))
        .child(row().w_full().justify_between().gap(12.0)
            .child(col().grow(1.0).shrink().gap(8.0)
                .child(row().w_full().gap(8.0).center_y()
                    .child(skeleton_bar(k, tw, 15.0))
                    .child(skeleton_faint(k, 48.0, 18.0).radius(9.0)))
                .child(row().w_full().child(skeleton_faint(k, dw, 12.0))))
            .child(row().gap(6.0).center_y()
                .child(skeleton_faint(k, 26.0, 26.0).radius(6.0))
                .child(skeleton_faint(k, 58.0, 26.0).radius(6.0))))
        .child(row().gap(6.0)
            .child(skeleton_faint(k, 54.0, 18.0).radius(9.0))
            .child(skeleton_faint(k, 68.0, 18.0).radius(9.0)))
        .child(row().gap(16.0).center_y()
            .child(skeleton_faint(k, 64.0, 11.0))
            .child(skeleton_faint(k, 38.0, 11.0))
            .child(skeleton_faint(k, 92.0, 11.0)))
}

/// Placeholder file rows for the Code tab while repository details or a local clone load.
pub fn skeleton_files(k: &Kit, with_commit: bool, key: &str) -> El {
    const NAMES: [f32; 5] = [96.0, 72.0, 128.0, 110.0, 84.0];
    let mut c = col().w_full().key(key.to_owned()).role("status").label("Loading files…");
    if with_commit {
        c = c.child(row().w_full().px(16.0).py(10.0).gap(10.0).center_y().fill(k.c.layer_01).radius(k.radius.lg)
            .child(el().size(22.0, 22.0).radius(11.0).fill(k.c.text_muted.alpha(0.18)))
            .child(skeleton_bar(k, 76.0, 12.0))
            .child(row().grow(1.0).shrink().child(skeleton_faint(k, 210.0, 12.0)))
            .child(skeleton_faint(k, 64.0, 11.0)));
    }
    for (idx, w) in NAMES.into_iter().enumerate() {
        if with_commit || idx > 0 {
            c = c.child(rule(k));
        }
        c = c.child(row().w_full().px(16.0).py(9.0).gap(10.0).center_y()
            .child(el().size(16.0, 16.0).radius(4.0).fill(k.c.text_muted.alpha(0.16)))
            .child(row().grow(1.0).shrink().child(skeleton_bar(k, w, 12.0))));
    }
    c
}

/// Placeholder paragraphs and code block while a README or local file loads.
pub fn skeleton_readme(k: &Kit, key: &str) -> El {
    col().w_full().gap(10.0).key(key.to_owned()).role("status").label("Loading…")
        .child(row().w_full().child(skeleton_bar(k, 210.0, 20.0)))
        .child(row().w_full().pt(4.0).child(skeleton_faint(k, 380.0, 12.0)))
        .child(row().w_full().child(skeleton_faint(k, 340.0, 12.0)))
        .child(row().w_full().child(skeleton_faint(k, 260.0, 12.0)))
        .child(el().w_full().h(54.0).radius(k.radius.md).fill(k.c.text_muted.alpha(0.10)))
        .child(row().w_full().child(skeleton_faint(k, 300.0, 12.0)))
        .child(row().w_full().child(skeleton_faint(k, 220.0, 12.0)))
}

/// Placeholder file headers and diff lines while `git` or pull request changes load.
pub fn skeleton_diff(k: &Kit, key: &str) -> El {
    let hunk = |file_w: f32, lines: &[f32]| {
        let mut s = col().w_full()
            .child(row().w_full().px(16.0).py(9.0).gap(10.0).center_y().fill(k.c.layer_01)
                .child(el().size(14.0, 14.0).radius(3.0).fill(k.c.text_muted.alpha(0.16)))
                .child(row().grow(1.0).shrink().child(skeleton_bar(k, file_w, 12.0)))
                .child(skeleton_faint(k, 44.0, 11.0)));
        for &w in lines {
            s = s.child(row().w_full().px(16.0).py(6.0).gap(12.0).center_y()
                .child(skeleton_faint(k, 24.0, 10.0))
                .child(row().grow(1.0).shrink().child(skeleton_faint(k, w, 11.0))));
        }
        s
    };
    col().w_full().pb(8.0).key(key.to_owned()).role("status").label("Loading changes…")
        .child(hunk(160.0, &[260.0, 190.0, 310.0, 220.0]))
        .child(rule(k))
        .child(hunk(130.0, &[240.0, 280.0, 175.0]))
}

/// A panel showing why a load failed, with **Retry**, in place of an empty list.
pub fn error_panel(k: &Kit, message: &str, retry_key: &str, on_retry: impl Fn(&mut crate::App) + 'static) -> El {
    panel(k).padding(28.0).items_center().gap(10.0).role("alert").key(format!("{retry_key}-panel"))
        .child(ico(&icons::CROSS, 20.0, k.c.danger.icon))
        .child(text(message.to_owned()).font(k.ty.small).color(k.c.text_base).wrap())
        .child(rune_kit::Button::new("Retry").icon(&icons::REFRESH)
            .control_size(rune_kit::ControlSize::Small)
            .on_press(on_retry).build(k).key(retry_key.to_owned()))
}

/// An inline error row with **Retry**, for panels that already have their own header.
pub fn error_inline(k: &Kit, message: &str, retry_key: &str, on_retry: impl Fn(&mut crate::App) + 'static) -> El {
    row().w_full().padding(16.0).center_y().gap(12.0).role("alert")
        .child(text(message.to_owned()).font(k.ty.small).color(k.c.danger.icon).wrap().grow(1.0).shrink())
        .child(rune_kit::Button::new("Retry").control_size(rune_kit::ControlSize::Small)
            .on_press(on_retry).build(k).key(retry_key.to_owned()))
}

