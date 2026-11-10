//! The phone layout: a top bar, the page, and a tab bar at the bottom instead of the sidebar.
//!
//! The app switches to it when the window is narrower than [`BREAKPOINT`] points (a phone, or a
//! narrow desktop window). Pages read `App::compact` to stack their columns and trim their rows.
use rune::{El, Icon, TextStyle, col, el, row, text};
use rune_kit::Kit;

use crate::{App, Page, icons, ui};

/// Windows narrower than this get the phone layout.
pub const BREAKPOINT: f32 = 720.0;

const LABEL: TextStyle = TextStyle::new(10.5, 550.0);

fn tab(s: &App, k: &Kit, label: &str, icon: &'static Icon, page: Page) -> El {
    let on = s.page == page || (page == Page::Repos && matches!(s.page, Page::Repo(_)));
    let ink = if on { k.c.text_accent } else { k.c.text_muted };
    col().grow(1.0).min_w(0.0).h(52.0).gap(3.0).center().radius(k.radius.md).press(k.c.pressed)
        .role("button").label(label.to_owned()).key(format!("tab-{label}"))
        .on_press(move |s: &mut App| s.go(page))
        .child(ui::ico(icon, 20.0, ink))
        .child(text(label.to_owned()).font(LABEL).color(ink).max_lines(1))
}

/// The whole window: `bar_trailing` are the top bar's buttons, `page` the scrolling content.
/// `safe` is the browser's safe area (top, right, bottom, left): the notch and the home bar.
pub fn shell(s: &App, k: &Kit, safe: [f32; 4], title: String, bar_trailing: El, page: El) -> El {
    let bar = row().w_full().h(48.0).px(12.0).gap(10.0).center_y()
        .child(row().size(26.0, 26.0).radius(7.0).center().fill(k.c.text_strong).child(ui::ico(&icons::REPO, 15.0, k.c.page)))
        .child(text(title).font(k.ty.body_medium).color(k.c.text_strong).grow(1.0).shrink().max_lines(1))
        .child(bar_trailing);
    let tabs = row().w_full().px(6.0).pt(4.0).gap(2.0)
        .child(tab(s, k, "Overview", &icons::HOME, Page::Overview))
        .child(tab(s, k, "Repos", &icons::REPO, Page::Repos))
        .child(tab(s, k, "Pulls", &icons::PULL, Page::Pulls))
        .child(tab(s, k, "Issues", &icons::ISSUE, Page::Issues))
        .child(tab(s, k, "Actions", &icons::PLAY, Page::Actions));
    col().w_full().h_full().fill(k.c.bg_base)
        .child(el().w_full().h(safe[0]))
        .child(bar)
        .child(ui::rule(k))
        .child(page)
        .child(ui::rule(k))
        .child(tabs)
        .child(el().w_full().h(safe[2].max(4.0)))
}
