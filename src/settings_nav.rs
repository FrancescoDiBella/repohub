//! Settings on a phone: its sections as a list (each with what it holds), and a way back to the
//! list from the section opened. On a wider window a section bar picks them instead (`lists`).
//!
//! ```text
//! the list ──a section──▶ the section ──Settings / Esc / the back button──▶ the list
//! ```

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::state::SettingsSection;
use crate::ui::{self, panel, rule};
use crate::{App, Page, icons};

/// The phone's Settings page: the sections, one row each.
pub fn phone_list(k: &Kit) -> El {
    let mut list = panel(k).key("settings-section").role("list").label("Settings sections");
    for (n, section) in SettingsSection::ALL.into_iter().enumerate() {
        let name = SettingsSection::LABELS[n];
        if n > 0 {
            list = list.child(rule(k));
        }
        list = list.child(row().w_full().px(16.0).py(12.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
            .role("button").label(format!("{name}: {}", section.about())).key(format!("settings-open-{name}"))
            .on_press(move |s: &mut App| s.open_settings(section))
            .child(col().grow(1.0).shrink().gap(2.0)
                .child(text(name).font(k.ty.ui_medium).color(k.c.text_base))
                .child(text(section.about()).font(k.ty.small).color(k.c.text_muted).wrap()))
            .child(ui::ico(&icons::NEXT, 14.0, k.c.icon_muted)));
    }
    col().w_full().gap(16.0)
        .child(crate::pages::heading(k, "Settings", "Your account, how RepoHub looks, its assistants, plugins and shortcuts.", None))
        .child(list)
}

/// Above a section on a phone: back to the list.
pub fn back(k: &Kit) -> El {
    row().w_full().child(Button::new("Settings").icon(&icons::BACK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.settings_open = false).build(k).key("settings-back"))
}

impl App {
    /// Opens Settings on `section` (on a phone, the section itself rather than the list).
    pub(crate) fn open_settings(&mut self, section: SettingsSection) {
        if self.page != Page::Settings {
            self.go(Page::Settings);
        }
        if self.page == Page::Settings {
            self.settings_section = section;
            self.settings_open = true;
        }
    }
}
