//! RepoHub: a personal GitHub repository manager built with Rune and rune-kit.
//!
//! It reads your repositories, pull requests, issues and workflow runs from the GitHub REST API.
//! The desktop app is configured by a `.env` file (see `.env.example`); the web build gets its
//! configuration and a token-adding proxy from `repohub-server` (the Docker image). Without either
//! it shows built-in sample data.
// Desktop-only helpers (local files, the terminal) aren't called from the web build.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppHandle, AppSpec, Cx, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::shell::{self, PaneState, Sidebar, SidebarRow, SidebarSection, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, PaletteSeed};

mod board;
#[cfg(not(target_arch = "wasm32"))]
mod clones;
mod clones_shim;
mod config;
mod github;
mod icons;
mod lists;
mod markdown;
mod mobile;
mod model;
mod net;
mod notes;
mod notes_page;
mod pages;
mod prefs;
mod state;
mod terminal;
mod terminal_page;
mod ui;

use config::Config;

/// Tests that change process environment variables hold this, so they don't race.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use model::{Data, Detail, PullState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Repos,
    Repo(usize),
    Pulls,
    Issues,
    Actions,
    Settings,
}

pub use state::Status;
use state::{Load, Refresh, RepoFilter, RepoSort, RepoTab, Save, Theme, Toast, ToastKind};

pub struct App {
    sidebar: PaneState,
    page: Page,
    filter: RepoFilter,
    /// The section of the repository page. Change it with `set_tab`, which runs what a section
    /// needs on entry and tidies up the one it leaves.
    tab: RepoTab,
    /// The Pull requests page shows the pull requests in this state.
    pr_filter: PullState,
    search: TextEdit,
    theme: Theme,
    sort: RepoSort,
    /// Counts toasts, so a toast's timer only hides its own message.
    toast_id: u64,
    pinned_open: bool,
    toast: Option<Toast>,
    cfg: Arc<Config>,
    data: Data,
    /// Repository details by full name; a missing entry hasn't been asked for.
    details: HashMap<String, Load<Detail>>,
    /// Where the data comes from. Change it with `set_status`.
    status: Status,
    handle: Option<AppHandle<App>>,
    // Notes (Excalidraw drawings), keyed by repository full name.
    branches: HashMap<String, Load<Vec<String>>>,
    note_branch: HashMap<String, String>,
    notes: HashMap<(String, String), notes::NotesView>,
    note_name: TextEdit,
    confirm_delete: Option<String>,
    /// The drawing open in the Notes tab, if any.
    board: Option<board::Board>,
    // Terminal tab (desktop): one shell per repository, opened in its local clone.
    terms: HashMap<String, terminal::Term>,
    /// Width of one character and height of one line of the terminal font, measured on screen.
    term_cell: [f32; 2],
    /// Rows and columns the terminal area fits.
    term_grid: (u16, u16),
    clone_path: TextEdit,
    // The account form in Settings (desktop).
    acct_user: TextEdit,
    acct_token: TextEdit,
    acct_error: Option<String>,
    term_error: Option<String>,
    /// The window is phone-sized: the pages use the layout in `mobile`.
    compact: bool,
}

impl App {
    /// Opens a page. Leaving a repository's section closes what belonged to it.
    fn go(&mut self, p: Page) {
        if self.page != p {
            state::trace("page", &self.page, &p);
            self.leave_tab();
        }
        self.page = p;
        self.toast = None;
    }

    /// Tidies up the repository section on screen before another one (or another page) shows:
    /// the open drawing is closed (its last edits are written first), and pending confirmations
    /// and error messages don't follow you to the next place.
    fn leave_tab(&mut self) {
        self.flush_board();
        self.board = None;
        self.confirm_delete = None;
        self.term_error = None;
    }

    /// Switches the repository page of repository `i` to `tab`.
    fn set_tab(&mut self, i: usize, tab: RepoTab) {
        if self.tab != tab {
            state::trace("tab", &self.tab, &tab);
            self.leave_tab();
        }
        self.tab = tab;
        match tab {
            RepoTab::Notes => self.load_notes(i),
            RepoTab::Terminal => self.open_terminal(i),
            _ => {}
        }
    }

    fn set_status(&mut self, status: Status) {
        if self.status != status {
            state::trace("status", &self.status.name(), &status.name());
        }
        self.status = status;
    }

    /// True when the board editor is on screen (and so takes its keyboard shortcuts).
    fn editing_board(&self) -> bool {
        self.board.is_some() && self.tab == RepoTab::Notes && matches!(self.page, Page::Repo(_))
    }

    /// Who owns the account setting: the environment, nobody yet, or the form in Settings.
    fn account(&self) -> state::Account {
        if self.cfg.account_locked {
            state::Account::Locked
        } else if self.cfg.live() {
            state::Account::SignedIn
        } else {
            state::Account::SignedOut
        }
    }

    /// Saves the account typed in Settings and loads its data. An empty token field keeps the
    /// saved token; `sign_out` removes both.
    #[cfg(not(target_arch = "wasm32"))]
    fn save_account(&mut self, sign_out: bool) {
        let user = self.acct_user.text.trim().trim_start_matches('@').to_owned();
        let typed = self.acct_token.text.trim().to_owned();
        let (user, token) = if sign_out {
            (None, None)
        } else {
            (Some(user).filter(|u| !u.is_empty()), Some(typed).filter(|t| !t.is_empty()).or_else(|| self.cfg.token.clone()))
        };
        if !sign_out && user.is_none() && token.is_none() {
            self.acct_error = Some("Type your GitHub user name, a token, or both.".into());
            return;
        }
        if let Err(e) = config::save_account(user.as_deref(), token.as_deref()) {
            self.acct_error = Some(e);
            return;
        }
        let mut cfg = (*self.cfg).clone();
        cfg.user = user;
        cfg.token = token;
        cfg.env_file = Some(config::user_env_file().display().to_string());
        self.cfg = Arc::new(cfg);
        self.acct_error = None;
        self.acct_token.clear();
        self.details.clear();
        if sign_out {
            self.acct_user.clear();
        }
        if self.cfg.live() {
            self.data = Data::default();
            self.set_status(Status::Starting);
            self.refresh();
            self.notify("Account saved. Loading your repositories…");
        } else {
            self.data = Data::sample();
            self.set_status(Status::Sample);
            self.notify("Signed out. Showing the sample data.");
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn save_account(&mut self, _: bool) {}

    /// Picks the theme and remembers it.
    fn set_theme(&mut self, theme: Theme) {
        state::trace("theme", &self.theme, &theme);
        self.theme = theme;
        prefs::save_theme(theme);
    }

    /// Shows a short message at the bottom of the window; it goes away after a few seconds.
    fn notify(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Done, 4500);
    }

    /// Like `notify`, for something that went wrong: marked as a problem and shown longer.
    fn notify_problem(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Problem, 8000);
    }

    fn show_toast(&mut self, text: String, kind: ToastKind, ms: u32) {
        self.toast_id += 1;
        let id = self.toast_id;
        self.toast = Some(Toast { id, text, kind });
        if let Some(h) = self.handle.clone() {
            net::spawn(h, move || net::sleep(ms), move |s: &mut App, _| {
                if s.toast.as_ref().is_some_and(|t| t.id == id) {
                    s.toast = None;
                }
            });
        }
    }

    fn open(&mut self, i: usize) {
        self.go(Page::Repo(i));
        self.set_tab(i, RepoTab::Code);
        self.load_detail(i);
    }

    /// Reloads everything from GitHub in the background.
    fn refresh(&mut self) {
        if !self.cfg.live() || !self.status.can_load() {
            return;
        }
        self.set_status(Status::Loading);
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            net::spawn(h, move || async move { github::load_all(&cfg).await }, |s: &mut App, r| s.apply(r));
        }
    }

    fn apply(&mut self, result: Result<Data, String>) {
        match result {
            Ok(d) => {
                self.data = d;
                self.details.clear();
                self.set_status(Status::Ready);
                if let Page::Repo(i) = self.page {
                    if i < self.data.repos.len() { self.load_detail(i) } else { self.page = Page::Repos }
                }
            }
            Err(e) => self.set_status(Status::Failed(e)),
        }
    }

    fn load_detail(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if self.data.sample {
            let d = model::sample_detail(r);
            self.details.insert(full, Load::Ready(d));
            return;
        }
        if self.details.contains_key(&full) {
            return;
        }
        self.details.insert(full.clone(), Load::Loading);
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let key = full.clone();
            net::spawn(h, move || async move { github::load_detail(&cfg, &full).await }, move |s: &mut App, d| {
                s.details.insert(key, Load::Ready(d));
            });
        }
    }

    /// Stars or unstars repository `i`: on GitHub when a token is configured, locally otherwise.
    fn toggle_star(&mut self, i: usize) {
        let Some(r) = self.data.repos.get_mut(i) else { return };
        let on = !r.starred;
        r.starred = on;
        r.stars = if on { r.stars + 1 } else { r.stars.saturating_sub(1) };
        if !self.cfg.live() || self.cfg.token.is_none() {
            if self.cfg.live() {
                self.notify("Star changed here only: GITHUB_TOKEN is needed to save it on GitHub.");
            }
            return;
        }
        if let Some(h) = self.handle.clone() {
            let (cfg, full) = (self.cfg.clone(), r.full_name.clone());
            let key = full.clone();
            net::spawn(h, move || async move { github::set_star(&cfg, &full, on).await }, move |s: &mut App, res| {
                if let Err(e) = res {
                    if let Some(r) = s.data.repos.iter_mut().find(|r| r.full_name == key) {
                        r.starred = !on;
                        r.stars = if on { r.stars.saturating_sub(1) } else { r.stars + 1 };
                    }
                    s.notify_problem(format!("Couldn't change the star: {e}"));
                }
            });
        }
    }

    /// The branch the Notes tab shows for repository `i`: the one picked, else the default branch.
    fn notes_branch(&self, i: usize) -> Option<(String, String)> {
        let r = self.data.repos.get(i)?;
        let b = self.note_branch.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone());
        Some((r.full_name.clone(), b))
    }

    /// Loads the branch list (once) and the drawings of the selected branch.
    fn load_notes(&mut self, i: usize) {
        if !self.cfg.notes {
            return;
        }
        let Some((full, branch)) = self.notes_branch(i) else { return };
        let default = self.data.repos[i].branch.clone();
        let Some(h) = self.handle.clone() else { return };
        if !self.branches.contains_key(&full) {
            self.branches.insert(full.clone(), Load::Loading);
            let (cfg, f) = (self.cfg.clone(), full.clone());
            let key = full.clone();
            net::spawn(h.clone(), move || async move { github::branches(&cfg, &f, &default).await }, move |s: &mut App, r| {
                s.branches.insert(key, r.into());
            });
        }
        let k = (full.clone(), branch.clone());
        self.notes.entry(k.clone()).or_default().state = Refresh::Loading;
        net::spawn(h, move || async move { notes::list(&full, &branch).await }, move |s: &mut App, r| {
            let v = s.notes.entry(k).or_default();
            match r {
                Ok(items) => {
                    v.items = items;
                    v.state = Refresh::Fresh;
                }
                Err(e) => v.state = Refresh::Failed(e),
            }
        });
    }

    fn pick_branch(&mut self, i: usize, branch: String) {
        if let Some(r) = self.data.repos.get(i) {
            self.note_branch.insert(r.full_name.clone(), branch);
            self.confirm_delete = None;
            self.load_notes(i);
        }
    }

    fn delete_note(&mut self, i: usize, name: String) {
        let Some((full, branch)) = self.notes_branch(i) else { return };
        self.confirm_delete = None;
        let Some(h) = self.handle.clone() else { return };
        net::spawn(h, move || async move { notes::delete(&full, &branch, &name).await }, move |s: &mut App, r| {
            match r {
                Ok(()) => s.notify("Drawing deleted"),
                Err(e) => s.notify_problem(e),
            }
            s.load_notes(i);
        });
    }

    /// Opens drawing `name` of repository `i` (on the selected branch) in the board editor.
    fn open_board(&mut self, i: usize, name: String, new: bool) {
        let Some((full, branch)) = self.notes_branch(i) else { return };
        let mut b = board::Board::new(&full, &branch, &name);
        if new {
            self.board = Some(b);
            return;
        }
        b.save = Save::Loading;
        self.board = Some(b);
        let Some(h) = self.handle.clone() else { return };
        net::spawn(h, move || async move { notes::load(&full, &branch, &name).await }, |s: &mut App, r| {
            let Some(b) = s.board.as_mut() else { return };
            let loaded = match r {
                Ok(Some(json)) => b.load_json(&json),
                Ok(None) => Ok(()),
                Err(e) => Err(e),
            };
            b.save.loaded(loaded);
        });
    }

    /// The open drawing changed: write it, or queue the write behind the one that is running
    /// (see [`Save`]). One write runs at a time, so an older one can't land after a newer one.
    fn save_board(&mut self) {
        if self.board.as_mut().is_some_and(|b| b.save.edit()) {
            self.write_board();
        }
    }

    fn write_board(&mut self) {
        let Some(b) = self.board.as_ref() else { return };
        let (repo, branch, name, json) = (b.repo.clone(), b.branch.clone(), b.name.clone(), b.to_json());
        let Some(h) = self.handle.clone() else { return };
        let id = (repo.clone(), branch.clone(), name.clone());
        net::spawn(h, move || async move { notes::save(&repo, &branch, &name, json).await }, move |s: &mut App, r| {
            // Only the drawing this write belongs to hears about it.
            let same = s.board.as_ref().is_some_and(|b| (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) == (id.0.as_str(), id.1.as_str(), id.2.as_str()));
            if same && s.board.as_mut().is_some_and(|b| b.save.written(r)) {
                s.write_board();
            }
        });
    }

    /// Before the editor goes away: edits still waiting behind a running write (or after a failed
    /// one) are written now.
    fn flush_board(&mut self) {
        let Some(b) = self.board.as_ref() else { return };
        if !b.save.pending() {
            return;
        }
        let (repo, branch, name, json) = (b.repo.clone(), b.branch.clone(), b.name.clone(), b.to_json());
        if let Some(h) = self.handle.clone() {
            net::spawn(h, move || async move { notes::save(&repo, &branch, &name, json).await }, |s: &mut App, r| {
                if let Err(e) = r {
                    s.notify_problem(format!("The last change to the drawing wasn't saved: {e}"));
                }
            });
        }
    }

    /// Closes the editor and refreshes the list it came from.
    fn close_board(&mut self, i: usize) {
        self.flush_board();
        self.board = None;
        self.load_notes(i);
    }

    /// Starts a shell for repository `i` in `dir` (its clone, or where a clone will go).
    fn start_terminal(&mut self, i: usize, dir: std::path::PathBuf, first: Option<String>) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(h) = self.handle.clone() else { return };
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on_output = move || {
            if !pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let p = pending.clone();
                h.update(move |_: &mut App, _| p.store(false, std::sync::atomic::Ordering::SeqCst));
            }
        };
        let (rows, cols) = self.term_grid;
        match terminal::Term::spawn(&dir, rows, cols, on_output) {
            Ok(mut t) => {
                if let Some(cmd) = first {
                    t.write(cmd.as_bytes());
                }
                self.terms.insert(full, t);
                self.term_error = None;
            }
            Err(e) => self.term_error = Some(e),
        }
    }

    /// Opens the terminal in the repository's known clone, if any.
    fn open_terminal(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        if self.terms.get(&r.full_name).is_some_and(|t| t.is_running()) {
            return;
        }
        if let Some(dir) = clones_shim::find(&r.full_name) {
            self.start_terminal(i, dir, None);
        }
    }

    /// Uses the folder typed in the field as repository `i`'s clone.
    fn use_folder(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let dir = clones_shim::expand(self.clone_path.text.trim());
        if !dir.is_dir() {
            self.term_error = Some(format!("{} isn't a folder on this computer.", dir.display()));
            return;
        }
        if let Err(e) = clones_shim::remember(&full, &dir) {
            self.term_error = Some(e);
            return;
        }
        self.clone_path.clear();
        self.terms.remove(&full);
        self.start_terminal(i, dir, None);
    }

    /// Clones repository `i` into the default folder, in its terminal, and remembers where.
    fn clone_repo(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, name, url) = (r.full_name.clone(), r.name.clone(), r.url.clone());
        let parent = clones_shim::default_parent();
        if let Err(e) = std::fs::create_dir_all(&parent) {
            self.term_error = Some(format!("Couldn't create {}: {e}", parent.display()));
            return;
        }
        let _ = clones_shim::remember(&full, &parent.join(&name));
        self.terms.remove(&full);
        self.start_terminal(i, parent, Some(format!("git clone {url}.git && cd {name}\r")));
    }

    /// Types `text` into repository `i`'s terminal.
    fn term_type(&mut self, i: usize, text: &str) {
        if let Some(t) = self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            t.write(text.as_bytes());
        }
    }

    fn term_type_bytes(&mut self, i: usize, bytes: &[u8]) {
        if let Some(t) = self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            t.write(bytes);
        }
    }

    /// Pastes into the terminal of the repository on screen, when its Terminal tab is open.
    fn paste(&mut self, text: &str) -> bool {
        let Page::Repo(i) = self.page else { return false };
        if self.tab != RepoTab::Terminal {
            return false;
        }
        match self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            Some(t) => {
                t.paste(text);
                true
            }
            None => false,
        }
    }

    fn starred_count(&self) -> u32 {
        self.data.repos.iter().map(|r| r.stars).sum()
    }
}

fn init() -> App {
    let args = rune::extra_args();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let cfg = Arc::new(Config::load());
    let live = cfg.live();
    let mut s = App {
        sidebar: PaneState::new(248.0).range(180.0, 360.0),
        page: Page::Overview,
        filter: RepoFilter::All,
        tab: RepoTab::Code,
        pr_filter: PullState::Open,
        search: TextEdit::new(""),
        theme: prefs::load_theme().unwrap_or(cfg.theme),
        sort: RepoSort::Recent,
        toast_id: 0,
        pinned_open: true,
        toast: None,
        // The web build waits for repohub-server's config.json before choosing (see `start`).
        data: if live || cfg!(target_arch = "wasm32") { Data::default() } else { Data::sample() },
        details: HashMap::new(),
        status: if live || cfg!(target_arch = "wasm32") { Status::Starting } else { Status::Sample },
        handle: None,
        branches: HashMap::new(),
        note_branch: HashMap::new(),
        notes: HashMap::new(),
        note_name: TextEdit::new(""),
        confirm_delete: None,
        board: None,
        terms: HashMap::new(),
        term_cell: [7.8, 17.0],
        term_grid: (24, 100),
        clone_path: TextEdit::new(""),
        acct_user: TextEdit::new(cfg.user.clone().unwrap_or_default()),
        acct_token: TextEdit::new(""),
        acct_error: None,
        term_error: None,
        compact: starts_compact(),
        cfg,
    };
    // `REPOHUB_BLOCKING=1` loads synchronously before the first frame (headless snapshots).
    #[cfg(not(target_arch = "wasm32"))]
    if live && std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1") {
        s.set_status(Status::Loading);
        let r = pollster::block_on(github::load_all(&s.cfg));
        s.apply(r);
        if let Some(i) = arg("page").filter(|p| p == "repo").map(|_| 0)
            && let Some(r) = s.data.repos.get(i)
        {
            let full = r.full_name.clone();
            s.details.insert(full.clone(), Load::Ready(pollster::block_on(github::load_detail(&s.cfg, &full))));
        }
    }
    // `-- page repos|repo|pulls|issues|actions|settings` opens a page (for headless stills).
    s.page = match arg("page").as_deref() {
        Some("repos") => Page::Repos,
        Some("repo") if !s.data.repos.is_empty() => Page::Repo(0),
        Some("pulls") => Page::Pulls,
        Some("issues") => Page::Issues,
        Some("actions") => Page::Actions,
        Some("settings") => Page::Settings,
        _ => Page::Overview,
    };
    if s.data.sample && matches!(s.page, Page::Repo(_)) {
        s.load_detail(0);
    }
    // `-- tab notes` (a name or a number) picks the repository page's section.
    if let Some(t) = arg("tab").and_then(|t| RepoTab::parse(&t)) {
        s.tab = t;
    }
    if let Some(q) = arg("search") {
        s.search = TextEdit::new(q);
    }
    s
}

/// GitHub-like palette seeds: canvas, ink and the blue accent.
fn kit(dark: bool) -> Kit {
    let c = |h: &str| rune::color::hex(h).expect("valid colour");
    let mut k = if dark {
        Kit::from_palette(PaletteSeed { bg: c("#0d1117"), fg: c("#e6edf3"), accent: c("#2f81f7") })
    } else {
        Kit::from_palette(PaletteSeed { bg: c("#f6f8fa"), fg: c("#1f2328"), accent: c("#0969da") })
    };
    k.radius.lg = 10.0;
    k
}

fn nav(s: &App, k: &Kit, label: &str, icon: &'static rune::Icon, page: Page, badge: Option<usize>) -> El {
    let selected = s.page == page || (page == Page::Repos && matches!(s.page, Page::Repo(_)));
    let mut r = SidebarRow::new(label).icon(icon).height(32.0).selected(selected).on_select(move |s: &mut App, _| s.go(page));
    if let Some(n) = badge.filter(|n| *n > 0) {
        r = r.badge(n.to_string());
    }
    r.build(k).key(format!("nav-{label}"))
}

/// The pinned repositories: the names in REPOHUB_PINNED, else the three most recently pushed.
fn pinned(s: &App) -> Vec<usize> {
    let repos = &s.data.repos;
    if s.cfg.pinned.is_empty() {
        return (0..repos.len().min(3)).collect();
    }
    s.cfg.pinned.iter().filter_map(|p| repos.iter().position(|r| r.name.eq_ignore_ascii_case(p) || r.full_name.eq_ignore_ascii_case(p))).collect()
}

fn sidebar(s: &App, k: &Kit) -> El {
    let open_prs = s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let brand = row().w_full().h(40.0).px(10.0).gap(10.0).center_y()
        .child(row().size(26.0, 26.0).radius(7.0).center().fill(k.c.text_strong)
            .child(ui::ico(&icons::REPO, 15.0, k.c.page)))
        .child(text(s.cfg.title.clone()).font(k.ty.body_medium).color(k.c.text_strong));
    let pinned: Vec<El> = pinned(s).into_iter().map(|i| {
        let r = &s.data.repos[i];
        SidebarRow::new(r.name.clone()).avatar(ui::avatar(k, &r.name, r.hue, 16.0)).height(30.0)
            .selected(s.page == Page::Repo(i)).on_select(move |s: &mut App, _| s.open(i)).build(k).key(format!("pin-{}", r.full_name))
    }).collect();
    Sidebar::new()
        .label("Navigation")
        .header(brand)
        .header(nav(s, k, "Overview", &icons::HOME, Page::Overview, None))
        .header(nav(s, k, "Repositories", &icons::REPO, Page::Repos, Some(s.data.repos.len())))
        .header(nav(s, k, "Pull requests", &icons::PULL, Page::Pulls, Some(open_prs)))
        .header(nav(s, k, "Issues", &icons::ISSUE, Page::Issues, Some(s.data.issues.len())))
        .header(nav(s, k, "Actions", &icons::PLAY, Page::Actions, None))
        .section(SidebarSection::new("Pinned").count(pinned.len()).collapsed(!s.pinned_open)
            .on_toggle(|s: &mut App, _| s.pinned_open = !s.pinned_open).rows(pinned))
        .footer(nav(s, k, "Settings", &icons::GEAR, Page::Settings, None))
        .build(k)
}

fn title(s: &App) -> String {
    match s.page {
        Page::Overview => "Overview".into(),
        Page::Repos => "Repositories".into(),
        Page::Repo(i) => s.data.repos.get(i).map(|r| r.full_name.replace('/', " / ")).unwrap_or_default(),
        Page::Pulls => "Pull requests".into(),
        Page::Issues => "Issues".into(),
        Page::Actions => "Actions".into(),
        Page::Settings => "Settings".into(),
    }
}

/// A banner above the page for loading, errors and sample data.
fn banner(s: &App, k: &Kit) -> Option<El> {
    let (msg, tint, action) = match &s.status {
        Status::Starting | Status::Loading => ("Loading data from GitHub…".to_owned(), k.c.info.icon, false),
        Status::Failed(e) => (e.clone(), k.c.danger.icon, true),
        Status::Sample if !s.cfg.demo && cfg!(not(target_arch = "wasm32")) => ("This is sample data. Connect your GitHub account to see your repositories.".to_owned(), k.c.warning.icon, false),
        Status::Sample if !s.cfg.demo => ("Sample data. To see your repositories, create a .env file with GITHUB_USER or GITHUB_TOKEN (see .env.example) and run RepoHub with Docker or as the desktop app.".to_owned(), k.c.warning.icon, false),
        _ => return None,
    };
    let connect = s.status == Status::Sample && cfg!(not(target_arch = "wasm32")) && s.page != Page::Settings;
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg)
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(if action { &icons::CROSS } else { &icons::BELL }, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    if action {
        r = r.child(rune_kit::Button::new("Retry").control_size(ControlSize::Small).on_press(|s: &mut App| s.refresh()).build(k).key("retry"));
    }
    if connect {
        r = r.child(rune_kit::Button::new("Connect").variant(rune_kit::ButtonVariant::Submit).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.go(Page::Settings)).build(k).key("connect"));
    }
    Some(r.key("banner"))
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let system_dark = cx.system_dark();
    let dark = s.theme.is_dark(system_dark);
    let k = kit(dark);
    let content = match s.page {
        Page::Overview => pages::overview(s, &k),
        Page::Repos => pages::repos(s, &k),
        Page::Repo(i) if i < s.data.repos.len() => pages::repo_detail(s, &k, i),
        Page::Repo(_) => pages::repos(s, &k),
        Page::Pulls => lists::pulls(s, &k),
        Page::Issues => lists::issues(s, &k),
        Page::Actions => lists::actions(s, &k),
        Page::Settings => lists::settings(s, &k),
    };
    let mut inner = col().w_full().max_w(1080.0).gap(16.0);
    if let Some(b) = banner(s, &k) {
        inner = inner.child(b);
    }
    let pad = if s.compact { 12.0 } else { 24.0 };
    let page = col().w_full().padding(pad).gap(16.0).items_center().child(inner.child(content));
    let mut main = col().w_full().h_full().shrink().fill(k.c.page)
        .child(col().key("page").w_full().h_full().shrink().scroll(ScrollSpec { stick_bottom: false }).child(page));
    // The toast floats over the page, at the bottom; it leaves by itself or with its button.
    if let Some(toast) = &s.toast {
        let (icon, tint) = match toast.kind {
            ToastKind::Done => (&icons::CHECK, k.c.success.icon),
            ToastKind::Problem => (&icons::CROSS, k.c.danger.icon),
        };
        let close = IconButton::new(&icons::CROSS).variant(IconButtonVariant::GhostMuted).label("Dismiss")
            .on_press(|s: &mut App| s.toast = None).build(&k).key("toast-close");
        main = main.child(row().pin(Some(16.0), None, Some(16.0), Some(20.0)).center()
            .child(row().key("toast").gap(10.0).pl(14.0).pr(6.0).py(6.0).center_y().max_w(560.0).radius(k.radius.lg)
                .fill(k.c.bg_base).border(1.0, if toast.kind == ToastKind::Problem { tint.alpha(0.6) } else { k.c.border_base })
                .role(if toast.kind == ToastKind::Problem { "alert" } else { "status" })
                .child(ui::ico(icon, 14.0, tint))
                .child(text(toast.text.clone()).font(k.ty.small).color(k.c.text_base).wrap().shrink())
                .child(close)));
    }
    let (theme_icon, theme_help) = if dark { (&icons::SUN, "Switch to the light theme") } else { (&icons::MOON, "Switch to the dark theme") };
    let theme = IconButton::new(theme_icon).variant(IconButtonVariant::GhostMuted).label(theme_help)
        .help(theme_help).on_press(move |s: &mut App| s.set_theme(s.theme.toggled(system_dark))).build(&k).key("theme");
    let mut trailing = row().gap(4.0).center_y();
    if s.cfg.live() {
        trailing = trailing.child(IconButton::new(&icons::REFRESH).variant(IconButtonVariant::GhostMuted).label("Refresh")
            .help("Refresh from GitHub").disabled(s.status.is_busy()).on_press(|s: &mut App| s.refresh()).build(&k).key("refresh"));
    }
    let who = if s.data.profile.login.is_empty() { "?".to_owned() } else { s.data.profile.login.clone() };
    let profile_url = format!("https://github.com/{who}");
    let profile = ui::user(&k, &who, 26.0).ml(6.0).role("button").label("Your profile on GitHub").key("profile")
        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&profile_url); });
    let root = if s.compact {
        let settings = IconButton::new(&icons::GEAR).variant(IconButtonVariant::GhostMuted).label("Settings")
            .expanded(s.page == Page::Settings).on_press(|s: &mut App| s.go(Page::Settings)).build(&k).key("open-settings");
        let name = match s.page {
            Page::Repo(i) => s.data.repos.get(i).map(|r| r.name.clone()).unwrap_or_default(),
            _ => title(s),
        };
        mobile::shell(s, &k, cx.safe_area(), name, trailing.child(theme).child(settings), main.grow(1.0))
    } else {
        let toolbar = Toolbar::new().title(title(s)).trailing(trailing
            .child(theme)
            .child(profile));
        SplitView::new(main)
            .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar(s, &k))
            .toolbar(toolbar)
            .titlebar_inset(cx.titlebar_inset())
            .sidebar_toggle(SidebarToggle::new().size(ControlSize::Large))
            .reduced_motion(cx.is_reduced_motion())
            .build(&k)
    };
    root.on_paste(|s: &mut App, text: &str, _| {
            s.paste(text);
        })
        // The window's width picks the layout: phone-sized windows get the one in `mobile`.
        .on_layout_tx(|s: &mut App, [_, _, w, _], _| {
            let compact = w < mobile::BREAKPOINT;
            if s.compact != compact {
                s.compact = compact;
            }
        })
}

/// The layout to start with, before the first frame is measured (avoids a flash on phones).
fn starts_compact() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::get(&js_sys::global(), &"innerWidth".into()).ok().and_then(|v| v.as_f64()).is_some_and(|w| (w as f32) < mobile::BREAKPOINT)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("REPOHUB_COMPACT").is_ok_and(|v| v == "1")
    }
}

fn on_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
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
            _ => return shortcuts(s, ev),
        };
        if changed {
            s.save_board();
        }
        return true;
    }
    shortcuts(s, ev)
}

fn shortcuts(s: &mut App, ev: &KeyEvent) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('r') {
        s.refresh();
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

/// Inter for the interface and IBM Plex Mono for code and the terminal (both SIL OFL, see
/// `assets/fonts`). The mono slot prefers the system's monospaced font (SF Mono on macOS).
const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
];
const FAMILIES: (&str, &str) = ("Inter", "ui-monospace, IBM Plex Mono");

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(start))]
pub fn start() {
    let app = rune::App::new(AppSpec { on_key: Some(on_key), fonts: FONTS, families: FAMILIES, ..AppSpec::new("RepoHub", (1180.0, 760.0), init, view) });
    let handle = app.handle();
    let h2 = handle.clone();
    handle.update(move |s: &mut App, _| {
        s.handle = Some(h2.clone());
        #[cfg(not(target_arch = "wasm32"))]
        if s.status == Status::Starting {
            s.refresh(); // the first load, when the app starts with an account
        }
        // The web build asks repohub-server for its configuration; a static host (GitHub Pages)
        // has none, and then shows the sample data.
        #[cfg(target_arch = "wasm32")]
        net::spawn(h2, || net::fetch_config(), |s: &mut App, cfg| match cfg.filter(|c| c.live()) {
            Some(cfg) => {
                if prefs::load_theme().is_none() {
                    s.theme = cfg.theme;
                }
                s.cfg = Arc::new(cfg);
                s.refresh();
            }
            None => {
                s.data = Data::sample();
                s.set_status(Status::Sample);
            }
        });
    });
    app.run();
}
