//! Keys: the board editor's shortcuts and the app's (⌘K, ⌘R, ⌘[, Esc, ⌘1…5, and the ones of one key:
//! `/`, `j` `k` and the arrows, Enter, `i`, `p`, `?`; `keyboard::SHORTCUTS` lists them all).

use super::*;

pub(crate) fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // The board editor's shortcuts, unless the text tool's field is taking the keys.
    // They apply only while the editor is on screen: an open drawing left behind on another
    // page must not swallow Delete or ⌘Z there.
    if s.editing_board()
        && let Some(b) = s.board.as_mut()
        && !b.is_typing()
    {
        let changed = match &ev.key {
            rune::Key::Delete | rune::Key::Backspace => b.delete_selected(),
            rune::Key::Escape => {
                b.deselect();
                return true;
            }
            _ if ev.is_command('z') && ev.mods.shift => b.redo(),
            _ if ev.is_command('z') => b.undo(),
            _ if ev.is_command('y') => b.redo(),
            _ => return shortcuts(s, ev, tx),
        };
        if changed {
            s.save_board();
        }
        return true;
    }
    shortcuts(s, ev, tx)
}

pub(crate) fn shortcuts(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('r') {
        s.refresh();
        return true;
    }
    if ev.is_command('k') {
        s.palette = if s.palette.is_some() { None } else { Some(palette::Palette::new()) };
        return true;
    }
    // ⌘/Ctrl+[ goes back: out of a conversation, a workflow run, a commit or a comparison, or
    // from a repository to the list.
    if ev.is_command('[') {
        if s.thread.is_some() {
            s.close_thread();
        } else if s.open_run.is_some() {
            s.open_run = None;
        } else if s.commits.open.is_some() {
            s.close_commit(Some(tx));
        } else if s.commits.compare.is_some() {
            s.close_compare();
        } else if matches!(s.page, Page::Repo(_)) {
            s.go(Page::Repos);
        } else {
            return false;
        }
        return true;
    }
    // Esc closes what's open on top of the page, the most recent first.
    if ev.key == rune::Key::Escape && ev.mods == rune::Mods::default() {
        return s.escape(tx);
    }
    if one_key(s, ev, tx) {
        return true;
    }
    // ⌘/Ctrl+1…5 jump to the main pages, ⌘/Ctrl+, to Settings.
    for (c, page) in [('1', Page::Overview), ('2', Page::Repos), ('3', Page::Pulls), ('4', Page::Issues), ('5', Page::Actions), (',', Page::Settings)] {
        if ev.is_command(c) {
            s.go(page);
            return true;
        }
    }
    false
}

/// The shortcuts of one key. A text field takes typed keys before they get here; these also
/// leave the terminal's and the board editor's keys alone, and the arrows and Enter act only
/// while no control has the keyboard (or a row of the list does).
fn one_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || s.palette.is_some() || s.discard.is_some() {
        return false;
    }
    if s.editing_board() || tx.focused() == Some("terminal") {
        return false;
    }
    match &ev.key {
        rune::Key::Text(t) => match t.as_str() {
            "/" => s.focus_search(),
            "?" => {
                s.settings_section = state::SettingsSection::Keyboard;
                s.go(Page::Settings);
                true
            }
            "j" if !ev.mods.shift => s.move_cursor(1, tx),
            "k" if !ev.mods.shift => s.move_cursor(-1, tx),
            "i" | "p" if !ev.mods.shift && s.thread.is_none() => match s.repo_index() {
                Some(i) => {
                    s.open_form(i, if t == "i" { Compose::Issue } else { Compose::Pull });
                    true
                }
                None => false,
            },
            _ => false,
        },
        rune::Key::Down | rune::Key::Up | rune::Key::Enter if !ev.mods.shift => {
            let free = match tx.focused() {
                None => true,
                Some(f) => keyboard::rows(s).iter().any(|(key, _)| key == f),
            };
            if !free {
                return false;
            }
            match ev.key {
                rune::Key::Down => s.move_cursor(1, tx),
                rune::Key::Up => s.move_cursor(-1, tx),
                _ => s.open_cursor(tx),
            }
        }
        _ => false,
    }
}

impl App {
    /// `/`: the search field of the page on screen gets the keyboard (Repositories, Pull
    /// requests, Issues, a repository's Commits); where there's none, the palette opens, which
    /// searches everything.
    pub(crate) fn focus_search(&mut self) -> bool {
        let over = self.thread.is_some() || self.open_run.is_some();
        match &self.page {
            Page::Repos if !over => self.search.focused = true,
            Page::Pulls if !over => self.pull_filter.search.focused = true,
            Page::Issues if !over => self.issue_filter.search.focused = true,
            Page::Repo(_) if !over && self.plugin_tab.is_none() && self.tab == RepoTab::Commits && self.commits.open.is_none() && self.commits.compare.is_none() => {
                self.commits.filter.focused = true;
            }
            _ => self.palette = Some(palette::Palette::new()),
        }
        true
    }
}
