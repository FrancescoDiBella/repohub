//! Settings → About → Saved data: where the copy of the last load is, how large and from when,
//! and **Clear saved data**.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::ui::{panel, panel_head, rule};
use crate::{App, cache, icons};

fn info(s: &App, k: &Kit, label: &str, value: String) -> El {
    row().w_full().px(16.0).py(10.0).gap(16.0).center_y()
        .child(text(label.to_owned()).font(k.ty.small).color(k.c.text_muted).w(if s.compact { 84.0 } else { 170.0 }))
        .child(text(value).font(k.ty.small_medium).color(k.c.text_base).grow(1.0).shrink().wrap())
}

/// "The lists show what was saved 3 hours ago.": after a failed load over the saved copy.
pub fn shown_copy(s: &App) -> String {
    match s.loaded_at {
        Some(t) => format!("The lists show what was saved {}.", crate::model::ago(t, s.now.max(t))),
        None => "The lists show what was saved last time.".into(),
    }
}

pub fn panel_of(s: &App, k: &Kit) -> El {
    let state = if !cache::enabled() {
        "Off (REPOHUB_CACHE=off): every start waits for GitHub.".to_owned()
    } else if !s.cfg.live() {
        "Nothing: the sample data isn't saved.".to_owned()
    } else {
        match &s.fresh.info {
            Some(i) => format!("{}, from the load {}. The next start shows it at once, then updates it.", crate::notes::size(i.bytes as u64), crate::model::ago(i.saved, s.now.max(i.saved))),
            None => "Nothing yet: it's saved after the next load.".to_owned(),
        }
    };
    let clear = Button::new("Clear saved data").icon(&icons::TRASH).control_size(ControlSize::Small).variant(ButtonVariant::Ghost)
        .disabled(s.fresh.info.is_none())
        .on_press(|s: &mut App| s.ask("ask-clear-saved-data")).build(k).key("clear-saved-data");
    let question = crate::confirm::Confirm::new("ask-clear-saved-data", &icons::TRASH, "Clear the saved data?")
        .detail("The copy of your last load is deleted from this computer: the next start waits for GitHub.")
        .yes("Clear saved data", crate::confirm::Tone::Danger, |s: &mut App| s.clear_saved_data())
        .show(s, k);
    let mut p = panel(k).key("saved-data").child(panel_head(k, "Saved data", Some(clear)));
    if let Some(q) = question {
        p = p.child(col().w_full().px(16.0).pb(12.0).child(q));
    }
    let mut p = p.child(rule(k))
        .child(info(s, k, "Last load", state));
    if s.cfg.live() && cache::enabled() {
        p = p.child(rule(k)).child(info(s, k, "Where", cache::place(&s.cfg)));
    }
    p.child(rule(k))
        .child(info(s, k, "What", "Your lists and the repository pages you opened, to open at once next time. Never your token. Removed when you sign out or switch accounts.".into()))
}
