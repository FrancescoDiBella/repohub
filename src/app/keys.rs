//! Keys: the board editor's shortcuts and the app's (⌘K, ⌘R, ⌘[, Esc, ⌘1…5).

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
    // ⌘/Ctrl+1…5 jump to the main pages, ⌘/Ctrl+, to Settings.
    for (c, page) in [('1', Page::Overview), ('2', Page::Repos), ('3', Page::Pulls), ('4', Page::Issues), ('5', Page::Actions), (',', Page::Settings)] {
        if ev.is_command(c) {
            s.go(page);
            return true;
        }
    }
    false
}
