//! The app's state: `App`, `Page`, the small types `App`'s fields use, and `init`. The `impl App`
//! blocks are in the files below, grouped by what they are about; each starts with
//! `use super::*`, so the imports here are theirs too.

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppHandle, Color, AppSpec, Cx, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::shell::{self, PaneState, Sidebar, SidebarRow, SidebarSection, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, PaletteSeed};

use crate::{actions, board, cache, cache_page, claude, clones_shim, commits, confirm, diffs, discard, filters, github, harness, icons, inbox, keyboard, lists, local, mobile, model, net, notes, notes_page, pages, palette, plugins, prefs, releases, replay, review, runs_page, state, sync_io, terminal, themes, thread, thread_page, tray, triage, ui, update};
#[cfg(not(target_arch = "wasm32"))]
use crate::config;
use crate::config::Config;
use crate::model::{Data, Detail, PullState, same_repo};
use crate::state::{BranchSlot, Compose, Load, OwnerFilter, Refresh, RepoFilter, RepoSort, RepoTab, Save, Status, Submit, Theme, Toast, ToastKind};

mod chrome;
mod conversations;
mod drawings;
mod frame;
mod git;
mod keys;
mod loading;
mod nav;
mod repos;
mod settings;
mod updates;
mod writes;

pub use frame::start;
#[cfg(target_arch = "wasm32")]
pub use frame::repohub_back;
pub(crate) use chrome::view;
pub(crate) use keys::on_key;
pub(crate) use repos::{orgs, pinned};
#[cfg(test)]
pub(crate) use chrome::banner;
#[cfg(test)]
pub(crate) use frame::poll;
#[cfg(test)]
pub(crate) use keys::shortcuts;
#[cfg(test)]
pub(crate) use loading::More;

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

pub struct App {
    sidebar: PaneState,
    pub(crate) page: Page,
    pub(crate) filter: RepoFilter,
    /// Whose repositories the Repositories page shows.
    pub(crate) owner: OwnerFilter,
    /// The repositories pinned in the app (full names); `None`: never changed, so
    /// `REPOHUB_PINNED` or the most recently pushed decide (see `pinned`).
    pub(crate) pins: Option<Vec<String>>,
    /// Where you were last time, waiting for the repositories to load (see `restore_place`).
    /// Navigating before then drops it: what you do now wins.
    pub(crate) pending_place: Option<state::Place>,
    /// Whether the current place may be saved: not before the saved one was restored.
    save_places: bool,
    /// The arrangement last saved, so it's written only when it changes.
    pub(crate) saved_layout: Option<state::Layout>,
    /// The Organizations section of the sidebar is collapsed.
    orgs_closed: bool,
    /// The section of the repository page. Change it with `set_tab`, which runs what a section
    /// needs on entry and tidies up the one it leaves.
    pub(crate) tab: RepoTab,
    /// The Pull requests page shows the pull requests in this state.
    pub(crate) pr_filter: PullState,
    pub(crate) search: TextEdit,
    pub(crate) theme: Theme,
    /// Installed plugins (desktop app).
    pub(crate) plugins: plugins::Plugins,
    /// The plugin whose section is open on the repository page (instead of `tab`).
    pub(crate) plugin_tab: Option<String>,
    /// Settings → Plugins: the folder or Git address to install from, and what went wrong.
    pub(crate) plugin_src: TextEdit,
    pub(crate) plugin_error: Option<String>,
    /// A plugin is being installed in the background.
    pub(crate) plugin_installing: bool,
    /// What a plugin asks to do outside the app, waiting for the user's answer.
    pub(crate) plugin_ask: Option<plugins::Request>,
    /// What the plugins' context was last built from (`plugins::context_key`).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    plugin_ctx_key: u64,
    /// Whether the dark look is on screen (set while drawing; plugins' context).
    pub(crate) shown_dark: std::cell::Cell<bool>,
    /// The coding assistants (harnesses) and whether the Claude section is on.
    pub(crate) assist: harness::Assistants,
    /// The Settings form that adds a harness.
    pub(crate) harness_name: TextEdit,
    pub(crate) harness_cmd: TextEdit,
    pub(crate) harness_error: Option<String>,
    /// The looks (themes) and which one each appearance uses; edited in Settings.
    pub(crate) looks: themes::Looks,
    /// The theme editor's fields, for one of your looks.
    pub(crate) look_edit: LookEditor,
    /// The page area's size, for the background pattern.
    pub(crate) backdrop_size: [f32; 2],
    /// Decodes the background picture of a look (set up with the window).
    images: Option<rune::ImageCache>,
    pub(crate) sort: RepoSort,
    /// Counts toasts, so a toast's timer only hides its own message.
    toast_id: u64,
    pinned_open: bool,
    pub(crate) toast: Option<Toast>,
    pub(crate) cfg: Arc<Config>,
    pub(crate) data: Data,
    /// Local clones' file lists, by full name (desktop; read from disk).
    pub(crate) local: HashMap<String, Load<local::Tree>>,
    /// The Code tab's folder in the local clone (`""`: its root), and the file open in it.
    pub(crate) local_dir: String,
    pub(crate) local_file: Option<(String, Load<local::Content>)>,
    /// The diff on screen (local changes or a pull request's files) and what it is of.
    pub(crate) diff: rune_kit::diff::DiffState,
    pub(crate) diff_of: Option<diffs::DiffOf>,
    pub(crate) diff_load: Option<Load<()>>,
    /// The pull request diff on screen and what it is of: (repo full_name, pull number).
    pub(crate) pull_diff: rune_kit::diff::DiffState,
    pub(crate) pull_diff_of: Option<(String, u32)>,
    pub(crate) pull_diff_load: Option<Load<()>>,
    /// The local clone's branch, upstream and changes, for the Changes tab.
    pub(crate) changes: Option<local::Changes>,
    /// The Changes tab's commit message.
    pub(crate) commit_msg: TextEdit,
    /// A git command running in the background ("Committing…"), and the last one's error.
    pub(crate) git_job: Option<&'static str>,
    pub(crate) git_error: Option<String>,
    /// The branch being previewed for a local merge, and its commits.
    pub(crate) merge_preview: Option<local::MergePreview>,
    /// How a pull request is merged from its diff: merge, squash or rebase.
    pub(crate) merge_method: usize,
    /// The Code tab shows GitHub's files although a local clone is known.
    pub(crate) files_from_github: bool,
    /// Repository details by (full name, branch); a missing entry hasn't been asked for.
    pub(crate) details: HashMap<(String, String), Load<Detail>>,
    /// Where the data comes from. Change it with `set_status`.
    pub(crate) status: Status,
    /// Generation of the active account/session; incremented on account change or sign-out so
    /// in-flight background loads for an older account are dropped.
    pub(crate) load_gen: u64,
    pub(crate) handle: Option<AppHandle<App>>,
    // Notes (Excalidraw drawings), keyed by repository full name.
    pub(crate) branches: HashMap<String, Load<Vec<String>>>,
    /// The branch picked for each repository (Code and Notes tabs); absent: the default branch.
    pub(crate) branch_of: HashMap<String, String>,
    /// The open branch picker, and what it fills in.
    pub(crate) picker: Option<BranchSlot>,
    pub(crate) branch_filter: TextEdit,
    /// The open form (new issue, branch or pull request).
    pub(crate) form: Option<actions::Form>,
    /// The question waiting for a yes (see `confirm`): its key, e.g. `ask-merge-owner/repo#12`.
    pub(crate) asking: Option<String>,
    /// On a phone, the open question's sheet, left by the page that asks it for `view` to draw
    /// over the page (see `confirm::Confirm::show`).
    pub(crate) sheet: std::cell::RefCell<Option<El>>,
    /// Your unread notifications; `None` until they're first asked for.
    pub(crate) inbox: Option<Load<Vec<inbox::Notification>>>,
    /// The notifications being marked as read.
    pub(crate) inbox_busy: std::collections::HashSet<String>,
    /// The Settings section on screen.
    pub(crate) settings_section: state::SettingsSection,
    /// The connection's details (environment variables, files) are shown in Settings.
    pub(crate) connection_details: bool,
    /// Settings → Account says which token to make ("Which token?").
    pub(crate) token_help: bool,
    /// The line comments of the pull request whose diff is open, and the one being written.
    pub(crate) pull_review: Option<review::Review>,
    /// Issues being reopened with Undo, to put back in the list when GitHub confirms.
    pub(crate) reopening: Vec<model::Issue>,
    /// Pull requests being reopened with Undo (`owner/repo#12`): their message offers no Undo
    /// in turn.
    pub(crate) reopening_pulls: Vec<String>,
    /// What the Pull requests and the Issues pages are narrowed to.
    pub(crate) pull_filter: filters::ListFilter,
    pub(crate) issue_filter: filters::ListFilter,
    /// What the Actions page is narrowed to.
    pub(crate) runs_filter: runs_page::RunsFilter,
    /// The Commits section: the branch's history, the commit open and the comparison.
    pub(crate) commits: commits::Commits,
    /// The Releases section (its list, the form, questions waiting for a yes) and each
    /// repository's latest release for the header.
    pub(crate) releases: releases::Releases,
    /// The row of the list on screen marked with the keyboard (`j` / `k`), by its key (see
    /// `keyboard`).
    pub(crate) cursor: Option<String>,
    /// The section bar scrolls the section on screen into view on the next frame (`poll`).
    pub(crate) reveal_tab: bool,
    /// Where the section bar is scrolled, for its arrows (see `pages::SectionBar`).
    pub(crate) section_bar: pages::SectionBar,
    /// The issue or pull request whose conversation is on screen, in place of the page.
    pub(crate) thread: Option<thread::Open>,
    /// Counts conversation loads (see [`thread::Open::epoch`]).
    thread_epoch: u64,
    /// The repositories' labels, assignable people and milestones, kept once loaded (triage).
    pub(crate) choices: triage::Cache,
    /// The labels and assignees picked in the New issue form.
    pub(crate) issue_picks: triage::Picks,
    /// The list filters last saved in the preferences (`filters::Saved`), so they're written only
    /// when they change.
    pub(crate) saved_filters: Option<String>,
    /// The workflow run whose jobs and log are on screen, in place of the page.
    pub(crate) open_run: Option<runs_page::OpenRun>,
    /// Counts workflow run detail loads (see [`runs_page::OpenRun::epoch`]).
    run_epoch: u64,
    /// Where the lists on screen come from (the saved copy, part of a load) and what's kept to
    /// show at once when opened again (see `cache`).
    pub(crate) fresh: cache::Fresh,
    /// Rows with a write running (`owner/repo#12`), so their buttons wait.
    pub(crate) busy: std::collections::HashSet<String>,
    /// What the writes that finished during the running load changed in the lists: applied
    /// again when the load lands (see `replay`).
    pub(crate) replays: Vec<replay::Change>,
    /// Counts the loads that landed: a write that sees it move while it runs lands on lists
    /// that may already show it.
    pub(crate) landed: u64,
    /// "Discard what you typed?": the question on screen, and what happens on **Discard**.
    pub(crate) discard: Option<discard::Ask>,
    /// Counts reads of a diff's line comments (see `review::Review::asked`).
    line_epoch: u64,
    pub(crate) notes: HashMap<(String, String), notes::NotesView>,
    pub(crate) note_name: TextEdit,
    /// Generation of the open board load request; incremented on board open so late load results
    /// from an earlier board or earlier load cannot overwrite the open board.
    pub(crate) board_load_gen: u64,
    /// Drawings currently having a `notes::save` operation in-flight: `(repo, branch, name)`.
    pub(crate) saving_notes: HashMap<(String, String, String), NoteOp>,
    /// The note operations started, in order (tests check the write chain with it).
    #[cfg(test)]
    pub(crate) note_log: Vec<String>,
    /// Pending drawing save payloads waiting for an in-flight write to complete.
    pub(crate) pending_note_saves: HashMap<(String, String, String), NoteOp>,
    /// The drawing open in the Notes tab, if any.
    pub(crate) board: Option<board::Board>,
    /// Boards on every device: the notes repository and how syncing with it goes.
    pub(crate) sync: sync_io::SyncState,
    // Claude tab (desktop): one conversation per repository, run in its local clone.
    pub(crate) chats: HashMap<String, claude::Chat>,
    pub(crate) claude_prompt: TextEdit,
    pub(crate) claude_mode: claude::Mode,
    // Terminal tab (desktop): one shell per repository, opened in its local clone.
    pub(crate) terms: HashMap<String, terminal::Term>,
    /// Width of one character and height of one line of the terminal font, measured on screen.
    pub(crate) term_cell: [f32; 2],
    /// Rows and columns the terminal area fits.
    pub(crate) term_grid: (u16, u16),
    pub(crate) clone_path: TextEdit,
    /// Cached local clone path per repository (`full_name`), avoiding filesystem/git probes in `view()`.
    pub(crate) clone_dirs: std::cell::RefCell<HashMap<String, Option<std::path::PathBuf>>>,
    /// Cached lookalike folder per repository (`full_name`), avoiding `git remote -v` probes in `view()`.
    lookalike_dirs: std::cell::RefCell<HashMap<String, Option<std::path::PathBuf>>>,
    pub(crate) default_clone_parent: std::path::PathBuf,
    pub(crate) plugins_folder: std::path::PathBuf,
    /// How many cards `Page::Repos` renders before **Show more**.
    pub(crate) repos_limit: usize,
    /// Counts changes to `data`. The cached repository lists, the pinned set and the palette's
    /// entries are rebuilt when it moves, so **every** change to `data` in place must go through
    /// [`App::data_changed`]; replacing `data` bumps it where it's replaced.
    pub(crate) data_gen: u64,
    /// Cached filtered/sorted repository indices, pinned indices, and organization counts.
    repo_cache: std::cell::RefCell<RepoCache>,
    // The account form in Settings (desktop).
    pub(crate) acct_user: TextEdit,
    pub(crate) acct_token: TextEdit,
    pub(crate) acct_error: Option<String>,
    pub(crate) term_error: Option<String>,
    pub(crate) compact: bool,
    pub open_pulls_page: usize,
    pub closed_pulls_page: usize,
    pub issues_page: usize,
    pub loading_more_pulls: bool,
    pub loading_more_issues: bool,
    /// When GitHub's data on screen was loaded, and when the last load started (Unix seconds).
    pub(crate) loaded_at: Option<i64>,
    pub(crate) last_load: Option<i64>,
    /// The clock the "updated … ago" labels read; it ticks every 30 seconds.
    pub(crate) now: i64,
    /// Rune's frame clock (seconds since start; virtual in headless runs), when the last tick
    /// ran, and its value at the last load and the last data. It measures intervals with the
    /// wall clock above, which also counts time the computer slept.
    pub(crate) clock: f64,
    tick_at: f64,
    pub(crate) load_clock: Option<f64>,
    pub(crate) loaded_clock: Option<f64>,
    /// Updating the app itself (see `update`), whether its banner was put off, and Rune's clock
    /// at the last check.
    pub(crate) update: update::Update,
    /// Where updates come from: numbered releases or the Latest build.
    pub(crate) update_channel: update::Channel,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) update_cancel: Option<UpdateJob>,
    update_later: bool,
    /// The update banner shows the new version's notes (What's new).
    show_notes: bool,
    update_checked: Option<f64>,
    /// The command palette, while it's open (⌘K / Ctrl+K).
    pub(crate) palette: Option<palette::Palette>,
    /// Seconds to back off until the next automatic refresh if rate limited.
    pub(crate) retry_after: Option<u64>,
    /// The status-bar widget, and the menu it shows now.
    #[cfg(not(target_arch = "wasm32"))]
    tray: Option<tray::Native>,
    tray_menu: Vec<tray::Entry>,
}

#[derive(Default)]
struct RepoCache {
    orgs_stamp: Option<u64>,
    orgs: Vec<(String, usize)>,
    mine_count: usize,
    pins_stamp: Option<u64>,
    pinned: Vec<usize>,
    pinned_set: std::collections::HashSet<usize>,
    shown_stamp: Option<u64>,
    shown: Vec<usize>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub(crate) struct UpdateJob {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    phase: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl UpdateJob {
    const DOWNLOADING: u8 = 0;
    const CANCELLED: u8 = 1;
    const INSTALLING: u8 = 2;

    pub(crate) fn new() -> Self {
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

    pub(crate) fn begin_install(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.phase.compare_exchange(Self::DOWNLOADING, Self::INSTALLING, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }

    pub(crate) fn was_cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub(crate) fn init() -> App {
    cache::trace_time("");
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
        asking: None,
        sheet: std::cell::RefCell::new(None),
        settings_section: state::SettingsSection::Account,
        connection_details: false,
        token_help: false,
        pull_review: None,
        reopening: Vec::new(),
        reopening_pulls: Vec::new(),
        inbox: None,
        inbox_busy: std::collections::HashSet::new(),
        pull_filter: filters::ListFilter::default(),
        issue_filter: filters::ListFilter::default(),
        runs_filter: runs_page::RunsFilter::default(),
        commits: commits::Commits::default(),
        releases: releases::Releases::default(),
        cursor: None,
        reveal_tab: false,
        section_bar: pages::SectionBar::default(),
        thread: None,
        thread_epoch: 0,
        choices: triage::Cache::default(),
        issue_picks: triage::Picks::default(),
        saved_filters: None,
        open_run: None,
        run_epoch: 0,
        fresh: cache::Fresh::default(),
        busy: std::collections::HashSet::new(),
        replays: Vec::new(),
        landed: 0,
        discard: None,
        line_epoch: 0,
        notes: HashMap::new(),
        note_name: TextEdit::new(""),
        board_load_gen: 0,
        saving_notes: HashMap::new(),
        #[cfg(test)]
        note_log: Vec::new(),
        pending_note_saves: HashMap::new(),
        board: None,
        sync: Default::default(),
        chats: HashMap::new(),
        claude_prompt: TextEdit::new(""),
        claude_mode: claude::Mode::Plan,
        terms: HashMap::new(),
        term_cell: [7.8, 17.0],
        term_grid: (24, 100),
        clone_path: TextEdit::new(""),
        clone_dirs: std::cell::RefCell::new(HashMap::new()),
        lookalike_dirs: std::cell::RefCell::new(HashMap::new()),
        default_clone_parent: clones_shim::default_parent(),
        plugins_folder: plugins_folder_init(),
        repos_limit: pages::REPOS_PAGE,
        data_gen: 0,
        repo_cache: std::cell::RefCell::new(RepoCache::default()),
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
        update_channel: update::Channel::load(),
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
    let blocking = live && std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1");
    // Otherwise the lists of last time show at once, and the first load starts now, before
    // the window opens (what it finds waits for the window in a mailbox).
    #[cfg(not(target_arch = "wasm32"))]
    if live && !blocking {
        s.show_saved_copy();
        s.fresh.early = Some(net::Mailbox::default());
        s.refresh();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if blocking {
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
        s.reveal_tab = true;
    }
    // For headless stills the clone's changes are read before the first frame (the background
    // read would land after the picture is taken).
    #[cfg(not(target_arch = "wasm32"))]
    if s.tab == RepoTab::Changes
        && let Page::Repo(full) = &s.page
        && let Some(root) = clones_shim::find(full)
        && let Ok(c) = local::changes(&root)
    {
        s.diff_of = Some(diffs::DiffOf::Local(full.clone()));
        s.diff.set_files(diffs::local_files(&c));
        s.changes = Some(c);
        s.diff_load = Some(Load::Ready(()));
        // The clone's files too: the section bar offers Changes only with them.
        s.local.insert(full.clone(), local::scan(&root).into());
    }
    // `-- thread 14` opens the conversation of that pull request or issue (for headless stills).
    if let Some(number) = arg("thread").and_then(|n| n.parse::<u32>().ok()) {
        if let Some(repo) = s.data.pulls.iter().find(|p| p.number == number).map(|p| p.repo.clone()) {
            s.open_thread(&repo, number, true);
        } else if let Some(repo) = s.data.issues.iter().find(|x| x.number == number).map(|x| x.repo.clone()) {
            s.open_thread(&repo, number, false);
        }
    }
    // `-- picker labels` (assignees, milestone, reviewers) opens that picker in the conversation
    // (on a repository's page without one: the New issue form's), `-- edit title` (or `body`) the
    // title's or the description's editor (headless stills).
    s.triage_still(arg("picker"), arg("edit"));
    // `-- section appearance` picks the Settings page's section.
    if let Some(sec) = arg("section")
        && let Some(n) = state::SettingsSection::LABELS.iter().position(|l| l.eq_ignore_ascii_case(&sec))
    {
        s.settings_section = state::SettingsSection::ALL[n];
    }
    // `-- diff 14` also opens that pull request's changes.
    if let Some(number) = arg("diff").and_then(|n| n.parse::<u32>().ok())
        && let Some(repo) = s.data.pulls.iter().find(|p| p.number == number).map(|p| p.repo.clone())
    {
        s.open_thread(&repo, number, true);
        s.open_pull_diff(repo, number);
    }
    // With an account, the conversation and the diff are read before the first frame (there's no
    // handle to read them in the background yet); without `REPOHUB_BLOCKING`, once the window is
    // there (`start`).
    #[cfg(not(target_arch = "wasm32"))]
    if blocking {
        s.read_opened_now();
    }
    // `-- tab commits` reads the history before the first frame; `-- commit 0123abc` (or `commit 2`,
    // the second in the list) opens a commit, `-- compare main...feature/x` compares two branches.
    s.commits_still(arg("commit"), arg("compare"));
    // With `tab notes` the drawings are listed before the first frame (as the Changes above).
    #[cfg(not(target_arch = "wasm32"))]
    if s.tab == RepoTab::Notes
        && matches!(s.page, Page::Repo(_))
        && let Some((full, branch)) = s.notes_branch(0)
        && let Ok(items) = pollster::block_on(notes::list(&full, &branch))
    {
        s.notes.insert((full, branch), notes::NotesView { items, state: state::Refresh::Fresh });
    }
    // `-- sync waiting` shows board sync in that state (synced, syncing, waiting, failed, public,
    // setup, confirm), with `-- notes-repo owner/name`; for headless stills, it never runs.
    if let Some(state) = arg("sync") {
        s.sync_still(&state, arg("notes-repo"));
    }
    // `-- tab releases` reads the releases before the first frame; `-- release new` (or `notes`,
    // `edit`, `publish`) opens the form. Any repository page reads its latest release.
    s.releases_still(arg("release"));
    // `-- discard form` (or `reply`) types into the New issue form (or the conversation's
    // comment box) and asks "Discard what you typed?" (headless stills).
    s.discard_still(arg("discard"));
    // `-- ask merge` (or `sign-out`, `push-default`, …) asks that question before something that
    // can't be undone (headless stills).
    confirm::still(&mut s, arg("ask"));
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
    // The list filters of last time (an account's own: the sample data starts with none).
    if !s.data.sample {
        s.restore_filters();
    }
    // The sample data has no places of its own to go back to.
    if arg("page").is_none() && !s.data.sample {
        s.pending_place = prefs::get("place").as_ref().and_then(state::Place::from_json);
        // Over the saved copy it's restored once the window is there (`start`): what the page
        // reads needs the app's handle.
        if !s.data.repos.is_empty() && !s.fresh.from_copy {
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
pub(crate) enum NoteOp {
    Save(String),
    Delete,
}

/// The theme editor's text fields, for one of your looks (`id` empty: none open).
#[derive(Default)]
pub struct LookEditor {
    pub(crate) id: String,
    pub(crate) name: TextEdit,
    pub(crate) colors: Vec<TextEdit>,
    pub(crate) image: TextEdit,
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

fn plugins_folder_init() -> std::path::PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    {
        plugins::folder()
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::path::PathBuf::new()
    }
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
