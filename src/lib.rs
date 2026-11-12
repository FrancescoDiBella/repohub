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

use rune::{AppHandle, Color, AppSpec, Cx, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::shell::{self, PaneState, Sidebar, SidebarRow, SidebarSection, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, PaletteSeed};

mod actions;
mod board;
mod claude;
mod claude_page;
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
mod palette;
mod prefs;
mod state;
mod terminal;
mod terminal_page;
mod tray;
mod ui;
mod update;

use config::Config;

/// Tests that change process environment variables hold this, so they don't race.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use model::{Data, Detail, PullState};

/// `a` and `b` name the same repository (`owner/name`, or only the name).
fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || b.rsplit('/').next().is_some_and(|n| a.eq_ignore_ascii_case(n))
}

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
use state::{BranchSlot, Compose, Load, OwnerFilter, Refresh, RepoFilter, RepoSort, RepoTab, Save, Submit, Theme, Toast, ToastKind};

pub struct App {
    sidebar: PaneState,
    page: Page,
    filter: RepoFilter,
    /// Whose repositories the Repositories page shows.
    owner: OwnerFilter,
    /// The repositories pinned in the app (full names); `None`: never changed, so
    /// `REPOHUB_PINNED` or the most recently pushed decide (see `pinned`).
    pins: Option<Vec<String>>,
    /// Where you were last time, waiting for the repositories to load (see `restore_place`).
    /// Navigating before then drops it: what you do now wins.
    pending_place: Option<state::Place>,
    /// Whether the current place may be saved: not before the saved one was restored.
    save_places: bool,
    /// The arrangement last saved, so it's written only when it changes.
    saved_layout: Option<state::Layout>,
    /// The Organizations section of the sidebar is collapsed.
    orgs_closed: bool,
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
    /// Repository details by (full name, branch); a missing entry hasn't been asked for.
    details: HashMap<(String, String), Load<Detail>>,
    /// Where the data comes from. Change it with `set_status`.
    status: Status,
    handle: Option<AppHandle<App>>,
    // Notes (Excalidraw drawings), keyed by repository full name.
    branches: HashMap<String, Load<Vec<String>>>,
    /// The branch picked for each repository (Code and Notes tabs); absent: the default branch.
    branch_of: HashMap<String, String>,
    /// The open branch picker, and what it fills in.
    picker: Option<BranchSlot>,
    branch_filter: TextEdit,
    /// The open form (new issue, branch or pull request).
    form: Option<actions::Form>,
    /// The pull request whose Merge is waiting for a yes (`owner/repo#12`).
    confirm_merge: Option<String>,
    /// Rows with a write running (`owner/repo#12`), so their buttons wait.
    busy: std::collections::HashSet<String>,
    /// The title (and draft flag) of the item a form is creating, for showing it once created.
    form_title: Option<String>,
    pull_meta: Option<(String, bool)>,
    notes: HashMap<(String, String), notes::NotesView>,
    note_name: TextEdit,
    confirm_delete: Option<String>,
    /// The drawing open in the Notes tab, if any.
    board: Option<board::Board>,
    // Claude tab (desktop): one conversation per repository, run in its local clone.
    chats: HashMap<String, claude::Chat>,
    claude_prompt: TextEdit,
    claude_mode: claude::Mode,
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
    /// When GitHub's data on screen was loaded, and when the last load started (Unix seconds).
    loaded_at: Option<i64>,
    last_load: Option<i64>,
    /// The clock the "updated … ago" labels read; it ticks every 30 seconds.
    now: i64,
    /// Rune's frame clock (seconds since start; virtual in headless runs), when the last tick
    /// ran, and its value at the last load and the last data. It measures intervals with the
    /// wall clock above, which also counts time the computer slept.
    clock: f64,
    tick_at: f64,
    load_clock: Option<f64>,
    loaded_clock: Option<f64>,
    /// Updating the app itself (see `update`), whether its banner was put off, and Rune's clock
    /// at the last check.
    update: update::Update,
    update_later: bool,
    /// The update banner shows the new version's notes (What's new).
    show_notes: bool,
    update_checked: Option<f64>,
    /// The command palette, while it's open (⌘K / Ctrl+K).
    palette: Option<palette::Palette>,
    /// The status-bar widget, and the menu it shows now.
    #[cfg(not(target_arch = "wasm32"))]
    tray: Option<tray::Native>,
    tray_menu: Vec<tray::Entry>,
}

impl App {
    /// Opens a page. Leaving a repository's section closes what belonged to it.
    fn go(&mut self, p: Page) {
        // Moving before the saved place was restored: you chose where to be.
        if self.pending_place.take().is_some() {
            self.save_places = true;
        }
        if self.page != p {
            state::trace("page", &self.page, &p);
            self.leave_tab();
        }
        self.page = p;
        self.toast = None;
        self.sync_tray();
        self.save_place();
    }

    /// The place on screen, for the next start.
    fn place(&self) -> state::Place {
        let (page, repo) = match self.page {
            Page::Overview => ("overview", None),
            Page::Repos => ("repos", None),
            Page::Repo(i) => ("repo", self.data.repos.get(i).map(|r| r.full_name.clone())),
            Page::Pulls => ("pulls", None),
            Page::Issues => ("issues", None),
            Page::Actions => ("actions", None),
            Page::Settings => ("settings", None),
        };
        state::Place { page: page.into(), tab: repo.as_ref().map(|_| self.tab), repo }
    }

    fn save_place(&mut self) {
        if self.save_places && !self.data.sample_or_empty() {
            prefs::set("place", self.place().to_json());
        }
        self.save_layout();
    }

    /// The sidebar and the list's sort and owner, saved when they changed.
    fn save_layout(&mut self) {
        let now = state::Layout { sidebar_width: self.sidebar.width(), sidebar_open: self.sidebar.is_open(), sort: self.sort, owner: self.owner.clone() };
        if self.saved_layout.as_ref() != Some(&now) {
            prefs::set("layout", now.to_json());
            self.saved_layout = Some(now);
        }
    }

    /// Opens the place saved last time, once the repositories are here. A repository that's
    /// gone falls back to the Repositories page.
    fn restore_place(&mut self) {
        let Some(p) = self.pending_place.take() else { return };
        self.save_places = true;
        state::trace("restore", &"place", &p);
        let page = match p.page.as_str() {
            "repos" => Page::Repos,
            "pulls" => Page::Pulls,
            "issues" => Page::Issues,
            "actions" => Page::Actions,
            "settings" => Page::Settings,
            "repo" => match p.repo.as_ref().and_then(|full| self.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full))) {
                Some(i) => {
                    self.open(i);
                    if let Some(t) = p.tab {
                        self.set_tab(i, t);
                    }
                    return;
                }
                None => Page::Repos,
            },
            _ => Page::Overview,
        };
        self.go(page);
    }

    /// Tidies up the repository section on screen before another one (or another page) shows:
    /// the open drawing is closed (its last edits are written first), and pending confirmations
    /// and error messages don't follow you to the next place.
    fn leave_tab(&mut self) {
        self.flush_board();
        self.board = None;
        self.confirm_delete = None;
        self.term_error = None;
        self.picker = None;
        self.confirm_merge = None;
    }

    /// Switches the repository page of repository `i` to `tab`.
    fn set_tab(&mut self, i: usize, tab: RepoTab) {
        if self.tab != tab {
            state::trace("tab", &self.tab, &tab);
            self.leave_tab();
        }
        self.tab = tab;
        self.save_place();
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
        self.sync_tray();
    }

    /// Seconds since `wall` (Unix seconds) and `mono` (frame clock), whichever measured more.
    fn since(&self, wall: Option<i64>, mono: Option<f64>) -> Option<i64> {
        let w = wall.map(|t| self.now - t);
        let m = mono.map(|c| (self.clock - c) as i64);
        w.max(m).map(|d| d.max(0))
    }

    /// "Updated 3 minutes ago", once GitHub's data has loaded.
    fn freshness(&self) -> Option<String> {
        let age = self.since(self.loaded_at, self.loaded_clock)?;
        Some(format!("Updated {}", model::ago(self.now - age, self.now)))
    }

    /// Seconds until the next automatic refresh (negative: overdue), if they're on. The
    /// interval counts from when the last load started, so a failed load waits a whole one.
    fn refresh_in(&self) -> Option<i64> {
        let every = self.cfg.refresh_every()? as i64 * 60;
        let since = self.since(self.last_load.or(self.loaded_at), self.load_clock.or(self.loaded_clock))?;
        Some(every - since)
    }

    /// Every 30 seconds: move the labels' clock, and refresh when it's due. A load already
    /// running, the sample data, or a refresh turned off skip it.
    fn tick(&mut self) {
        self.now = net::now();
        if self.refresh_in().is_some_and(|left| left <= 0) && self.status.can_load() {
            state::trace("refresh", &"due", &self.cfg.refresh_every());
            self.refresh();
        }
        self.save_layout();
        // A new Latest build is looked for every 6 hours.
        if self.update_checked.is_some_and(|c| self.clock - c >= 6.0 * 3600.0) {
            self.check_update(false);
        }
        self.sync_tray();
    }

    /// Asks GitHub for the Latest build. `asked`: the user pressed Check, so say the outcome.
    fn check_update(&mut self, asked: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !self.update.can_check() {
                return;
            }
            let Some(h) = self.handle.clone() else { return };
            self.update = update::Update::Checking;
            self.update_checked = Some(self.clock);
            net::spawn(h, || async { update::check() }, move |s: &mut App, r| {
                s.update = match r {
                    Ok(rel) if update::build().is_some_and(|b| update::is_newer(&rel, b, update::built_at())) => {
                        if asked {
                            s.notify(format!("A new version is ready ({})", rel.short()));
                        }
                        update::Update::Available(rel)
                    }
                    Ok(_) => {
                        if asked {
                            s.notify("RepoHub is up to date");
                        }
                        update::Update::Current
                    }
                    Err(e) => {
                        if asked {
                            s.notify_problem(e.clone());
                        }
                        update::Update::Failed(e)
                    }
                };
                s.sync_tray();
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = asked;
    }

    /// Downloads the new version, checks it, puts it in place and restarts into it.
    fn install_update(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(rel) = self.update.available().cloned() else { return };
            let Some(h) = self.handle.clone() else { return };
            self.update = update::Update::Downloading { release: rel.clone(), percent: 0 };
            let progress = h.clone();
            net::spawn(h, move || async move {
                let bytes = update::download(&rel.url, rel.size, |pct| {
                    progress.update(move |s: &mut App, _| {
                        if let update::Update::Downloading { percent, .. } = &mut s.update {
                            *percent = pct;
                        }
                    });
                })?;
                let sums = rel.sums.as_deref().ok_or("This build has no checksums (SHA256SUMS), so it isn't installed. Download it from the Releases page.")?;
                let sums = String::from_utf8_lossy(&update::download(sums, 0, |_| {})?).into_owned();
                update::verify(&bytes, &rel.asset, &sums)?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let target = update::install(&bytes, &rel.asset, &exe)?;
                update::relaunch(&target)
            }, |s: &mut App, r: Result<(), String>| match r {
                Ok(()) => {
                    s.update = update::Update::Installing;
                    if let Some(h) = &s.handle {
                        h.quit();
                    }
                }
                Err(e) => {
                    s.notify_problem(e.clone());
                    s.update = update::Update::Failed(e);
                }
            });
        }
    }

    /// Shows the status-bar widget (desktop windows; see `tray`).
    fn start_tray(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !tray::wanted() || self.tray.is_some() {
                return;
            }
            let Some(h) = self.handle.clone() else { return };
            let entries = tray::menu(self);
            match tray::create(h, &entries, &tray::tooltip(self)) {
                Ok(t) => {
                    self.tray = Some(t);
                    self.tray_menu = entries;
                }
                Err(e) => eprintln!("repohub: no status-bar icon: {e}"),
            }
        }
    }

    /// Pushes the widget's menu again when what it shows changed.
    fn sync_tray(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.tray.is_some() {
            let entries = tray::menu(self);
            if entries != self.tray_menu {
                if let Some(t) = &self.tray {
                    t.update(&entries, &tray::tooltip(self));
                }
                self.tray_menu = entries;
            }
        }
    }

    /// A pick from the widget's menu (or a click on its icon: `open`).
    fn tray_action(&mut self, id: &str) {
        let Some(action) = tray::parse(id) else { return };
        let repo = |s: &App, full: &str| s.data.repos.iter().position(|r| r.full_name == full);
        let show = !matches!(action, tray::Action::Refresh | tray::Action::Quit);
        match action {
            tray::Action::Open => {}
            tray::Action::Go(p) => self.go(p),
            tray::Action::OpenRepo(full) => if let Some(i) = repo(self, &full) { self.open(i) },
            tray::Action::NewIssue(full) => if let Some(i) = repo(self, &full) {
                if self.page != Page::Repo(i) {
                    self.open(i);
                }
                self.open_form(i, Compose::Issue);
            },
            tray::Action::Refresh => self.refresh(),
            tray::Action::Update => self.install_update(),
            tray::Action::Quit => if let Some(h) = &self.handle { h.quit() },
        }
        if show && let Some(h) = &self.handle {
            h.open();
        }
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
        self.last_load = Some(net::now());
        self.load_clock = Some(self.clock);
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
                self.loaded_at = Some(net::now());
                self.loaded_clock = Some(self.clock);
                self.now = net::now();
                self.set_status(Status::Ready);
                if let Page::Repo(i) = self.page {
                    if i < self.data.repos.len() { self.load_detail(i) } else { self.page = Page::Repos }
                }
                self.restore_place();
            }
            Err(e) => self.set_status(Status::Failed(e)),
        }
    }

    /// The branch shown for repository `i`: the one picked, else the default branch.
    fn branch(&self, i: usize) -> String {
        self.data.repos.get(i).map(|r| self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone())).unwrap_or_default()
    }

    /// Loads files, README and last commit of repository `i` at its shown branch (once).
    fn load_detail(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let key = (full.clone(), self.branch(i));
        if self.data.sample {
            let d = model::sample_detail(r);
            self.details.insert(key, Load::Ready(d));
            return;
        }
        self.load_branches(i);
        if self.details.contains_key(&key) {
            return;
        }
        self.details.insert(key.clone(), Load::Loading);
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let at = key.1.clone();
            net::spawn(h, move || async move { github::load_detail(&cfg, &full, &at).await }, move |s: &mut App, d| {
                s.details.insert(key, Load::Ready(d));
            });
        }
    }

    /// Loads repository `i`'s branch list, once. The sample data has its default branch only.
    fn load_branches(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if self.branches.contains_key(&full) {
            return;
        }
        if self.data.sample || !self.cfg.live() {
            let mut names = vec![r.branch.clone()];
            names.extend(["feature/readme-selection", "fix/theme-switch"].map(String::from));
            self.branches.insert(full, Load::Ready(names));
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        self.branches.insert(full.clone(), Load::Loading);
        let (cfg, default, key) = (self.cfg.clone(), r.branch.clone(), full.clone());
        net::spawn(h, move || async move { github::branches(&cfg, &full, &default).await }, move |s: &mut App, r: Result<Vec<String>, String>| {
            // A pull request form opened before the list arrived starts from the first other branch.
            if let (Ok(names), Some(f)) = (&r, s.form.as_mut())
                && f.repo == key && f.kind == Compose::Pull && f.from == f.into
                && let Some(other) = names.iter().find(|n| **n != f.into)
            {
                f.from = other.clone();
            }
            s.branches.insert(key, r.into());
        });
    }

    /// Shows branch `branch` of repository `i` in the Code and Notes tabs.
    fn set_branch(&mut self, i: usize, branch: String) {
        let Some(r) = self.data.repos.get(i) else { return };
        state::trace("branch", &self.branch(i), &branch);
        if branch == r.branch {
            self.branch_of.remove(&r.full_name);
        } else {
            self.branch_of.insert(r.full_name.clone(), branch);
        }
        self.confirm_delete = None;
        self.load_detail(i);
        if self.tab == RepoTab::Notes {
            self.load_notes(i);
        }
    }

    /// The branch picker chose `branch` for `slot`.
    fn pick_branch_for(&mut self, i: usize, slot: BranchSlot, branch: String) {
        self.picker = None;
        match slot {
            BranchSlot::View => self.set_branch(i, branch),
            BranchSlot::From => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Head => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Base => if let Some(f) = self.form.as_mut() { f.into = branch },
        }
    }

    /// Opens form `kind` for repository `i` on the section it belongs to.
    fn open_form(&mut self, i: usize, kind: Compose) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, default, shown) = (r.full_name.clone(), r.branch.clone(), self.branch(i));
        // A pull request starts from the branch on screen, or the first other branch.
        let head = if shown != default {
            shown.clone()
        } else {
            self.branches.get(&full).and_then(Load::ready).and_then(|b| b.iter().find(|n| **n != default).cloned()).unwrap_or(shown.clone())
        };
        self.form = Some(actions::Form {
            repo: full,
            kind,
            title: TextEdit::new(""),
            body: TextEdit::new(""),
            from: if kind == Compose::Pull { head } else { shown },
            into: default,
            draft: false,
            submit: Submit::Editing,
        });
        self.picker = None;
        if self.tab != kind.tab() {
            self.set_tab(i, kind.tab());
        }
        self.load_branches(i);
        // `set_tab` tidies up the section it leaves; the form opened for the new one stays.
        if let Some(f) = self.form.as_mut() {
            f.title.focused = true;
        }
    }

    /// Sends the open form.
    fn submit_form(&mut self, i: usize) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some(f) = self.form.as_mut() else { return };
        let Ok(w) = f.write() else { return };
        if !f.submit.start() {
            return;
        }
        let repo = f.repo.clone();
        // What the screen shows for the new item until the lists reload.
        self.form_title = Some(f.title.text.trim().to_owned());
        self.pull_meta = Some((f.title.text.trim().to_owned(), f.draft));
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        net::spawn(h, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| match r {
            Ok(done) => {
                s.form = None;
                s.picker = None;
                s.apply_done(i, done);
            }
            Err(e) => {
                if let Some(f) = s.form.as_mut() {
                    f.submit = Submit::Failed(e);
                }
            }
        });
    }

    /// Runs a row's write (close an issue, merge a pull request) on repository `repo`.
    fn write(&mut self, repo: &str, w: github::Write) {
        if actions::blocked(self).is_some() {
            return;
        }
        let number = match &w {
            github::Write::CloseIssue(n) | github::Write::Merge { number: n, .. } => *n,
            _ => 0,
        };
        let key = actions::busy_key(repo, number);
        if !self.busy.insert(key.clone()) {
            return;
        }
        self.confirm_merge = None;
        let i = self.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(repo) || r.name.eq_ignore_ascii_case(repo)).unwrap_or(usize::MAX);
        let full = if i != usize::MAX { self.data.repos[i].full_name.clone() } else { repo.to_owned() };
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        net::spawn(h, move || async move { github::run(&cfg, &full, w).await }, move |s: &mut App, r| {
            s.busy.remove(&key);
            match r {
                Ok(done) => s.apply_done(i, done),
                Err(e) => s.notify_problem(e),
            }
        });
    }

    /// Shows what a write changed at once, without waiting for GitHub's lists to catch up.
    fn apply_done(&mut self, i: usize, done: github::Done) {
        self.apply_done_inner(i, done);
        self.sync_tray();
    }

    fn apply_done_inner(&mut self, i: usize, done: github::Done) {
        let repo = self.data.repos.get(i).map(|r| r.full_name.clone()).unwrap_or_default();
        match done {
            github::Done::Issue(c) => {
                let title = self.form_title.take().unwrap_or_default();
                self.data.issues.insert(0, model::Issue { number: c.number, title, repo: repo.clone(), label: None, age: "just now".into(), comments: 0, url: c.url });
                if let Some(r) = self.data.repos.get_mut(i) {
                    r.issues += 1;
                }
                self.notify(format!("Opened issue #{} in {repo}", c.number));
            }
            github::Done::Branch(name) => {
                if let Some(Load::Ready(list)) = self.branches.get_mut(&repo) {
                    list.push(name.clone());
                }
                self.notify(format!("Created branch {name}"));
                self.set_branch(i, name);
            }
            github::Done::Pull(c) => {
                let (title, draft) = self.pull_meta.take().unwrap_or_default();
                let author = self.data.profile.login.clone();
                self.data.pulls.insert(0, model::Pull {
                    number: c.number, title, repo: repo.clone(), author,
                    state: if draft { PullState::Draft } else { PullState::Open },
                    checks: None, age: "just now".into(), comments: 0, url: c.url,
                });
                self.notify(format!("Opened pull request #{}", c.number));
            }
            github::Done::Closed(n) => {
                self.data.issues.retain(|x| !(x.number == n && same_repo(&x.repo, &repo)));
                if let Some(r) = self.data.repos.get_mut(i) {
                    r.issues = r.issues.saturating_sub(1);
                }
                self.notify(format!("Closed issue #{n}"));
            }
            github::Done::Merged(n) => {
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &repo)) {
                    p.state = PullState::Merged;
                }
                self.notify(format!("Merged pull request #{n}"));
            }
        }
    }

    /// Stars or unstars repository    /// Stars or unstars repository `i`: on GitHub when a token is configured, locally otherwise.
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
        let b = self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone());
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

    /// The Notes tab's branch chips: the same branch as the Code tab.
    fn pick_branch(&mut self, i: usize, branch: String) {
        self.set_branch(i, branch);
        self.load_notes(i);
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

    /// Sends `prompt` to Claude Code in repository `i`'s local clone, continuing its conversation.
    #[cfg(not(target_arch = "wasm32"))]
    fn ask_claude(&mut self, i: usize, prompt: String) {
        let prompt = prompt.trim().to_owned();
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = clones_shim::find(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        let chat = self.chats.entry(full.clone()).or_default();
        if prompt.is_empty() || chat.run.is_busy() {
            return;
        }
        chat.items.push(claude::Item::User(prompt.clone()));
        chat.run = claude::Run::Starting;
        chat.run_id += 1;
        let run = chat.run_id;
        let (h2, f2) = (h.clone(), full.clone());
        let started = claude::start(
            &dir,
            &prompt,
            self.claude_mode,
            chat.session.as_deref(),
            move |e| {
                let full = full.clone();
                h.update(move |s: &mut App, _| {
                    if let Some(c) = s.chats.get_mut(&full).filter(|c| c.run_id == run) {
                        c.apply(e);
                    }
                });
            },
            move |problem| {
                h2.update(move |s: &mut App, _| {
                    if let Some(c) = s.chats.get_mut(&f2).filter(|c| c.run_id == run) {
                        c.finished(problem);
                    }
                });
            },
        );
        match started {
            Ok(p) => {
                chat.proc = Some(p);
                self.claude_prompt.clear();
            }
            Err(e) => chat.finished(Some(e)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ask_claude(&mut self, _: usize, _: String) {}

    /// Stops the answer Claude is writing for repository `i`.
    fn stop_claude(&mut self, i: usize) {
        let Some(c) = self.data.repos.get(i).and_then(|r| self.chats.get_mut(&r.full_name)) else { return };
        if c.run.is_busy() {
            c.run = claude::Run::Stopping;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = &c.proc {
                p.stop();
            }
        }
    }

    /// Forgets repository `i`'s conversation; the next message starts a new Claude Code session.
    fn new_chat(&mut self, i: usize) {
        if let Some(r) = self.data.repos.get(i) {
            self.chats.remove(&r.full_name);
        }
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
        // From the Claude tab the folder is all that's needed; the Terminal tab opens a shell there.
        if self.tab == RepoTab::Terminal {
            self.start_terminal(i, dir, None);
        }
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
        // The clone runs in the terminal, so show it.
        if self.tab != RepoTab::Terminal {
            self.set_tab(i, RepoTab::Terminal);
        }
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
        owner: OwnerFilter::All,
        orgs_closed: false,
        pins: prefs::load_pins(),
        pending_place: None,
        save_places: false,
        saved_layout: None,
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
        branch_of: HashMap::new(),
        picker: None,
        branch_filter: TextEdit::new(""),
        form: None,
        confirm_merge: None,
        busy: std::collections::HashSet::new(),
        form_title: None,
        pull_meta: None,
        notes: HashMap::new(),
        note_name: TextEdit::new(""),
        confirm_delete: None,
        board: None,
        chats: HashMap::new(),
        claude_prompt: TextEdit::new(""),
        claude_mode: claude::Mode::Plan,
        terms: HashMap::new(),
        term_cell: [7.8, 17.0],
        term_grid: (24, 100),
        clone_path: TextEdit::new(""),
        acct_user: TextEdit::new(cfg.user.clone().unwrap_or_default()),
        acct_token: TextEdit::new(""),
        acct_error: None,
        term_error: None,
        compact: starts_compact(),
        #[cfg(not(target_arch = "wasm32"))]
        tray: None,
        tray_menu: Vec::new(),
        palette: None,
        update: update::Update::initial(),
        update_later: false,
        show_notes: false,
        update_checked: None,
        loaded_at: None,
        last_load: None,
        now: net::now(),
        clock: 0.0,
        tick_at: 0.0,
        load_clock: None,
        loaded_clock: None,
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
            let at = s.data.repos[i].branch.clone();
            s.details.insert((full.clone(), at.clone()), Load::Ready(pollster::block_on(github::load_detail(&s.cfg, &full, &at))));
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
    // The arrangement and the place of last time; a page on the command line (scripts) wins.
    if let Some(l) = prefs::get("layout").as_ref().and_then(state::Layout::from_json) {
        s.sidebar.set_width(l.sidebar_width);
        s.sidebar.set_open(l.sidebar_open);
        s.sort = l.sort;
        s.owner = l.owner.clone();
        s.saved_layout = Some(l);
    }
    // The sample data has no places of its own to go back to.
    if arg("page").is_none() && !s.data.sample {
        s.pending_place = prefs::get("place").as_ref().and_then(state::Place::from_json);
        if !s.data.repos.is_empty() {
            s.restore_place();
        }
    }
    // Nothing waiting to be restored: where you go from now on is saved.
    if s.pending_place.is_none() {
        s.save_places = true;
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
    // The Repositories row shows every owner again.
    let mut r = SidebarRow::new(label).icon(icon).height(32.0).selected(selected).on_select(move |s: &mut App, _| {
        if page == Page::Repos {
            s.owner = OwnerFilter::All;
        }
        s.go(page)
    });
    if let Some(n) = badge.filter(|n| *n > 0) {
        r = r.badge(n.to_string());
    }
    r.build(k).key(format!("nav-{label}"))
}

/// The organizations whose repositories are loaded, with how many each, most first.
fn orgs(s: &App) -> Vec<(String, usize)> {
    let me = &s.data.profile.login;
    let mut out: Vec<(String, usize)> = Vec::new();
    for r in s.data.repos.iter().filter(|r| !r.owner.eq_ignore_ascii_case(me)) {
        match out.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(&r.owner)) {
            Some((_, n)) => *n += 1,
            None => out.push((r.owner.clone(), 1)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    out
}

impl App {
    /// Whether repository `i` is in the sidebar's Pinned section.
    fn is_pinned(&self, i: usize) -> bool {
        pinned(self).contains(&i)
    }

    /// Pins or unpins repository `i`. The first change starts from what the sidebar shows, so
    /// the pins it had by default stay.
    fn toggle_pin(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let mut list: Vec<String> = match &self.pins {
            Some(p) => p.clone(),
            None => pinned(self).into_iter().map(|j| self.data.repos[j].full_name.clone()).collect(),
        };
        let name = self.data.repos[i].name.clone();
        if let Some(at) = list.iter().position(|p| p.eq_ignore_ascii_case(&full)) {
            list.remove(at);
            self.notify(format!("Unpinned {name}"));
        } else {
            list.push(full);
            self.notify(format!("Pinned {name}"));
        }
        prefs::save_pins(&list);
        self.pins = Some(list);
        self.sync_tray();
    }

    /// Shows the Repositories page for `owner`.
    fn show_owner(&mut self, owner: OwnerFilter) {
        state::trace("owner", &self.owner, &owner);
        self.owner = owner;
        self.go(Page::Repos);
    }
}

/// The pinned repositories: the names in REPOHUB_PINNED, else the three most recently pushed.
fn pinned(s: &App) -> Vec<usize> {
    let repos = &s.data.repos;
    // Pins chosen in the app win; a repository that's no longer loaded is skipped.
    if let Some(pins) = &s.pins {
        return pins.iter().filter_map(|p| repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(p))).collect();
    }
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
    let org_rows: Vec<El> = orgs(s).into_iter().map(|(o, n)| {
        let on = s.page == Page::Repos && s.owner == OwnerFilter::Org(o.clone());
        let pick = o.clone();
        SidebarRow::new(o.clone()).avatar(ui::avatar(k, &o, 210.0, 16.0)).height(30.0).badge(n.to_string())
            .selected(on).on_select(move |s: &mut App, _| s.show_owner(OwnerFilter::Org(pick.clone()))).build(k).key(format!("org-{o}"))
    }).collect();
    let mut sb = Sidebar::new()
        .label("Navigation")
        .header(brand)
        .header(nav(s, k, "Overview", &icons::HOME, Page::Overview, None))
        .header(nav(s, k, "Repositories", &icons::REPO, Page::Repos, Some(s.data.repos.len())))
        .header(nav(s, k, "Pull requests", &icons::PULL, Page::Pulls, Some(open_prs)))
        .header(nav(s, k, "Issues", &icons::ISSUE, Page::Issues, Some(s.data.issues.len())))
        .header(nav(s, k, "Actions", &icons::PLAY, Page::Actions, None))
        .section(SidebarSection::new("Pinned").count(pinned.len()).collapsed(!s.pinned_open)
            .on_toggle(|s: &mut App, _| s.pinned_open = !s.pinned_open).rows(pinned));
    if !org_rows.is_empty() {
        sb = sb.section(SidebarSection::new("Organizations").count(org_rows.len()).collapsed(s.orgs_closed)
            .on_toggle(|s: &mut App, _| s.orgs_closed = !s.orgs_closed).rows(org_rows));
    }
    sb.footer(nav(s, k, "Settings", &icons::GEAR, Page::Settings, None)).build(k)
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

/// A banner when a new version is ready, or while it downloads.
fn update_banner(s: &App, k: &Kit) -> Option<El> {
    use rune_kit::{Button, ButtonVariant};
    let (msg, actions) = match &s.update {
        update::Update::Available(r) if !s.update_later => (
            format!("A new version of RepoHub is ready (built {} from {}).", model::ago(r.published, s.now.max(r.published)), r.short()),
            true,
        ),
        update::Update::Downloading { percent, .. } => (format!("Downloading the new version… {percent}%"), false),
        update::Update::Installing => ("Installing the new version; RepoHub restarts in a moment.".into(), false),
        _ => return None,
    };
    let tint = k.c.success.icon;
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg).key("update-banner")
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(&icons::REFRESH, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    let notes = s.update.available().map(|r| r.notes.clone()).filter(|n| actions && !n.is_empty());
    if notes.is_some() {
        r = r.child(Button::new(if s.show_notes { "Hide what's new" } else { "What's new" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.show_notes = !s.show_notes).build(k).key("update-notes"));
    }
    if actions {
        r = r.child(Button::new("Later").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.update_later = true).build(k).key("update-later"))
            .child(Button::new("Install and restart").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.install_update()).build(k).key("update-install"));
    }
    match notes {
        Some(md) if s.show_notes => Some(col().w_full().gap(0.0).radius(k.radius.lg).fill(tint.alpha(0.06)).border(1.0, tint.alpha(0.45)).key("update-banner-box")
            .child(r.radius(0.0).fill(Color::TRANSPARENT).border(0.0, Color::TRANSPARENT))
            .child(col().w_full().px(16.0).pb(12.0).key("update-notes-body").child(pages::readme_view(k, &md)))),
        _ => Some(r),
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
    if let Some(b) = update_banner(s, &k) {
        inner = inner.child(b);
    }
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
            .help(match s.freshness() { Some(f) => format!("Refresh from GitHub · {}", f.to_lowercase()), None => "Refresh from GitHub".into() })
            .disabled(s.status.is_busy()).on_press(|s: &mut App| s.refresh()).build(&k).key("refresh"));
    }
    let search = IconButton::new(k.glyph.search).variant(IconButtonVariant::GhostMuted).label("Search and commands")
        .help(format!("Search and commands ({}+K)", palette::MOD)).on_press(|s: &mut App| s.palette = Some(palette::Palette::new()))
        .build(&k).key("open-palette");
    let trailing = row().gap(4.0).center_y().child(search).child(trailing);
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
    let overlays = palette::layer(s, &k);
    let root = root.on_paste(|s: &mut App, text: &str, _| {
            s.paste(text);
        })
        // The window's width picks the layout: phone-sized windows get the one in `mobile`.
        .on_layout_tx(|s: &mut App, [_, _, w, _], _| {
            let compact = w < mobile::BREAKPOINT;
            if s.compact != compact {
                s.compact = compact;
            }
        });
    // Overlays (the command palette) go above the whole window.
    rune_kit::root(root, overlays)
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

/// Runs at the start of every frame with Rune's clock: ticks every 30 seconds (`App::tick`) and
/// asks Rune to wake for the next one, also while the window is closed. No timer thread.
fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    const EVERY: f64 = 30.0;
    s.clock = clock;
    let due = clock - s.tick_at >= EVERY;
    if due {
        s.tick_at = clock;
        s.tick();
    }
    tx.wake_after(std::time::Duration::from_secs_f64((EVERY - (clock - s.tick_at)).max(1.0)));
    due
}

/// The window closed: RepoHub stays in the status bar, unless its icon couldn't be shown.
fn on_close(s: &mut App, tx: &mut Tx) {
    #[cfg(not(target_arch = "wasm32"))]
    if s.tray.is_none() {
        tx.quit();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (s, tx);
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
    if ev.is_command('k') {
        s.palette = if s.palette.is_some() { None } else { Some(palette::Palette::new()) };
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
    let mut app = rune::App::new(AppSpec { on_key: Some(on_key), poll: Some(poll), fonts: FONTS, families: FAMILIES, ..AppSpec::new("RepoHub", (1180.0, 760.0), init, view) });
    // With the status-bar widget, closing the window keeps RepoHub there (macOS, Windows).
    if tray::keeps_running() {
        app = app.keep_running().on_close(on_close);
    }
    let app = app.on_quit(|s: &mut App, _| s.save_layout());
    let handle = app.handle();
    let h2 = handle.clone();
    handle.update(move |s: &mut App, _| {
        s.handle = Some(h2.clone());
        s.start_tray();
        // Clear what an earlier update left, then look for a newer build.
        #[cfg(not(target_arch = "wasm32"))]
        update::cleanup();
        s.check_update(false);
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

#[cfg(test)]
mod pin_tests {
    #[test]
    fn the_first_pin_keeps_the_default_ones() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pins = None;
        let mut cfg = (*s.cfg).clone();
        cfg.pinned.clear();
        s.cfg = std::sync::Arc::new(cfg);
        assert_eq!(crate::pinned(&s), vec![0, 1, 2], "by default the three most recently pushed");
        s.toggle_pin(4);
        assert_eq!(crate::pinned(&s), vec![0, 1, 2, 4], "pinning one more keeps them");
        s.toggle_pin(1);
        assert_eq!(crate::pinned(&s), vec![0, 2, 4]);
        assert!(s.is_pinned(4) && !s.is_pinned(1));
        let saved = crate::prefs::load_pins().unwrap();
        assert_eq!(saved.len(), 3);
        assert!(saved[2].eq_ignore_ascii_case(&s.data.repos[4].full_name), "remembered by full name");
        let _ = std::fs::remove_dir_all(dir);
    }
}
