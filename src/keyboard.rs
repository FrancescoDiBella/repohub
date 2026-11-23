//! The keyboard: every shortcut ([`SHORTCUTS`], listed in Settings → Keyboard), and moving through
//! a list with it.
//!
//! `j` / `k` (or ↓ / ↑ while no field has focus) move a
//! marked row through the list on screen (pull requests, issues, workflow runs, repositories,
//! notifications, and a repository's Issues, Pull requests and Actions sections); Enter opens
//! it, as a click does. The mark is a row's key (`App::cursor`), so it stays on the same row when
//! the list changes, and moving starts again from the top when that row is gone.
//!
//! [`rows`] lists the page's rows in the order they're drawn; [`mark`] draws the marked one.

use rune::{El, Tx};
use rune_kit::Kit;

use crate::model::Run;
use crate::state::RepoTab;
use crate::{App, Page, inbox};

/// What Enter does with a row.
#[derive(Clone, Debug)]
pub enum Open {
    Thread { repo: String, number: u32, pull: bool },
    Repo(usize),
    Run(Box<Run>),
    Url(String),
    Notification(Box<inbox::Notification>),
}

/// The rows of the list on screen, in order: their keys (as the page draws them) and what
/// opening one does. Empty where the page has no list (or a conversation is open over it).
pub fn rows(s: &App) -> Vec<(String, Open)> {
    if s.thread.is_some() || s.open_run.is_some() {
        return Vec::new();
    }
    let me = s.data.profile.login.as_str();
    let pull = |p: &crate::model::Pull| {
        let repo = crate::changes_page::pull_repo(&p.url).unwrap_or_else(|| p.repo.clone());
        (format!("pr-{}-{}", p.repo, p.number), Open::Thread { repo, number: p.number, pull: true })
    };
    let issue = |x: &crate::model::Issue| (format!("issue-{}-{}", x.repo, x.number), Open::Thread { repo: x.repo.clone(), number: x.number, pull: false });
    let run = |r: &Run| if r.id > 0 {
        (format!("run-{}-{}", r.repo, r.id), Open::Run(Box::new(r.clone())))
    } else {
        (format!("run-{}-{}", r.repo, r.workflow), Open::Url(r.url.clone()))
    };
    match &s.page {
        Page::Pulls => s.data.pulls.iter().filter(|p| p.state == s.pr_filter && s.pull_filter.pull(p, me)).map(pull).collect(),
        Page::Issues => s.data.issues.iter().filter(|x| s.issue_filter.issue(x, me)).map(issue).collect(),
        Page::Actions => s.data.runs.iter().filter(|r| s.runs_filter.matches(r)).map(run).collect(),
        Page::Repos => {
            let shown = s.filtered_repos();
            let n = shown.len().min(s.repos_limit.max(crate::pages::REPOS_PAGE));
            shown[..n].iter().map(|&i| (format!("repo-{}", s.data.repos[i].full_name), Open::Repo(i))).collect()
        }
        Page::Notifications => match &s.inbox {
            // Grouped by repository, as the page draws them.
            Some(crate::state::Load::Ready(list)) => {
                let mut repos: Vec<&str> = Vec::new();
                for n in list {
                    if !repos.iter().any(|r| r.eq_ignore_ascii_case(&n.repo)) {
                        repos.push(&n.repo);
                    }
                }
                repos.into_iter()
                    .flat_map(|repo| list.iter().filter(move |n| n.repo.eq_ignore_ascii_case(repo)))
                    .map(|n| (format!("notif-{}", n.id), Open::Notification(Box::new(n.clone()))))
                    .collect()
            }
            _ => Vec::new(),
        },
        Page::Repo(full) if s.plugin_tab.is_none() => {
            let Some(r) = s.repo_index_of(full).and_then(|i| s.data.repos.get(i)) else { return Vec::new() };
            let in_repo = |name: &str| name.eq_ignore_ascii_case(&r.full_name) || name.eq_ignore_ascii_case(&r.name);
            match s.tab {
                RepoTab::Issues => s.data.issues.iter().filter(|x| in_repo(&x.repo)).map(issue).collect(),
                RepoTab::Pulls => s.data.pulls.iter().filter(|p| in_repo(&p.repo)).map(pull).collect(),
                RepoTab::Actions => s.data.runs.iter().filter(|x| in_repo(&x.repo)).map(run).collect(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

impl App {
    /// `j` / ↓ (`by` 1) or `k` / ↑ (-1): the next or previous row of the list on screen is
    /// marked and scrolled into view. True when the page has a list.
    pub(crate) fn move_cursor(&mut self, by: i32, tx: &mut Tx) -> bool {
        let list = rows(self);
        if list.is_empty() {
            return false;
        }
        let at = self.cursor.as_ref().and_then(|c| list.iter().position(|(k, _)| k == c));
        let next = match at {
            Some(n) => (n as i32 + by).clamp(0, list.len() as i32 - 1) as usize,
            None if by < 0 => list.len() - 1,
            None => 0,
        };
        let key = list[next].0.clone();
        crate::state::trace("cursor", &self.cursor, &key);
        tx.scroll_into_view(key.clone());
        self.cursor = Some(key);
        true
    }

    /// Enter: opens the marked row, as a click does. False when no row is marked.
    pub(crate) fn open_cursor(&mut self, tx: &mut Tx) -> bool {
        let Some(c) = self.cursor.clone() else { return false };
        let Some((_, open)) = rows(self).into_iter().find(|(k, _)| *k == c) else { return false };
        match open {
            Open::Thread { repo, number, pull } => self.open_thread(&repo, number, pull),
            Open::Repo(i) => self.open(i),
            Open::Run(r) => self.open_run(&r),
            Open::Url(url) => {
                let _ = tx.open_url(&url);
            }
            Open::Notification(n) => self.open_notification(&n, tx),
        }
        true
    }

    /// Whether row `key` is the one marked with the keyboard.
    pub(crate) fn marked(&self, key: &str) -> bool {
        self.cursor.as_deref() == Some(key)
    }
}

/// A row as the keyboard marks it: tinted, with a ring, and said to assistive technology.
pub fn mark(k: &Kit, row: El, on: bool) -> El {
    if !on {
        return row;
    }
    row.fill(k.c.bg_accent.alpha(0.12)).border(1.0, k.c.border_focus).description("Marked: Enter opens it")
}

/// Every shortcut, as Settings → Keyboard lists them. The ones of one key (`/`, `j`, `i`…) work
/// while no text field, the terminal or the board editor has the keyboard.
pub const SHORTCUTS: [(&str, &str); 14] = [
    ("⌘/Ctrl + K", "Search and commands: jump to any repository or page, run any action (in the terminal too)"),
    ("⌘/Ctrl + 1 … 5", "Overview, Repositories, Pull requests, Issues, Actions"),
    ("⌘/Ctrl + ,", "Settings"),
    ("⌘/Ctrl + R", "Refresh from GitHub"),
    ("⌘/Ctrl + [", "Back: out of a conversation, a run, a commit or a comparison, or from a repository to the list"),
    ("Esc", "Close what's open, the newest first: a question, a picker, an editor, a form, a diff, a conversation, a run, a commit, a comparison, a file, a message"),
    ("/", "Search this page (Repositories, Pull requests, Issues, a repository's Commits); elsewhere the palette"),
    ("j  k  or  ↓ ↑", "Move through the list on screen: pull requests, issues, runs, repositories, notifications"),
    ("Enter", "Open the row marked with j / k"),
    ("i", "New issue, on a repository's page"),
    ("p", "New pull request, on a repository's page"),
    ("?", "This list"),
    ("Ctrl + Shift + Tab", "Leave the terminal, so the keyboard reaches the rest of the app"),
    ("↑ ↓ in ⌘K", "Move through the results; Enter runs the one highlighted"),
];

/// Settings → Keyboard: every shortcut, a line each.
pub fn panel(s: &App, k: &Kit) -> El {
    use rune::{col, row, text};
    let line = |key: &str, what: &str| row().w_full().px(16.0).py(7.0).gap(12.0).center_y()
        .child(text(key.to_owned()).font(k.ty.small_medium).color(k.c.text_base).w(if s.compact { 96.0 } else { 140.0 }).shrink())
        .child(text(what.to_owned()).font(k.ty.small).color(k.c.text_muted).grow(1.0).shrink().wrap());
    let mut p = crate::ui::panel(k).key("shortcuts").child(crate::ui::panel_head(k, "Keyboard shortcuts", None));
    for (key, what) in SHORTCUTS {
        p = p.child(crate::ui::rule(k)).child(line(key, what));
    }
    col().w_full().child(p)
}
