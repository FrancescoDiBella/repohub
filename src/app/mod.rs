//! The app: its state (`App`, `Page`), `init`, and the `impl App` blocks.

use std::collections::HashMap;
use std::sync::Arc;

use rune::{AppHandle, Color, AppSpec, Cx, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::shell::{self, PaneState, Sidebar, SidebarRow, SidebarSection, SidebarToggle, SplitView, Toolbar};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, PaletteSeed};

use crate::{actions, board, cache, cache_page, claude, clones_shim, commits, diffs, filters, github, harness, icons, inbox, lists, local, mobile, model, net, notes, notes_page, pages, palette, plugins, prefs, releases, review, runs_page, state, sync_io, terminal, themes, thread, thread_page, tray, triage, ui, update};
#[cfg(not(target_arch = "wasm32"))]
use crate::config;
use crate::config::Config;
use crate::model::{Data, Detail, PullState, same_repo};
use crate::state::{BranchSlot, Compose, Load, OwnerFilter, Refresh, RepoFilter, RepoSort, RepoTab, Save, Status, Submit, Theme, Toast, ToastKind};

mod chrome;
mod conversations;
mod frame;
mod keys;
mod loading;
mod nav;
mod repos;
mod writes;

pub(crate) use chrome::view;
#[cfg(test)]
pub(crate) use chrome::banner;
pub(crate) use keys::on_key;
#[cfg(test)]
pub(crate) use keys::shortcuts;
pub use frame::start;
#[cfg(target_arch = "wasm32")]
pub use frame::repohub_back;
#[cfg(test)]
pub(crate) use frame::poll;
pub(crate) use repos::{orgs, pinned};

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
    /// The plugin whose Remove was pressed once (press again to remove it).
    pub(crate) plugin_confirm: Option<String>,
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
    /// The pull request whose Merge is waiting for a yes (`owner/repo#12`).
    pub(crate) confirm_merge: Option<String>,
    /// Your unread notifications; `None` until they're first asked for.
    pub(crate) inbox: Option<Load<Vec<inbox::Notification>>>,
    /// The notifications being marked as read.
    pub(crate) inbox_busy: std::collections::HashSet<String>,
    /// The Settings section on screen.
    pub(crate) settings_section: state::SettingsSection,
    /// The connection's details (environment variables, files) are shown in Settings.
    pub(crate) connection_details: bool,
    /// The line comments of the pull request whose diff is open, and the one being written.
    pub(crate) pull_review: Option<review::Review>,
    /// Issues being reopened with Undo, to put back in the list when GitHub confirms.
    pub(crate) reopening: Vec<model::Issue>,
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
    /// The section bar scrolls the section on screen into view on the next frame (`poll`).
    pub(crate) reveal_tab: bool,
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
    /// The title (and draft flag) of the item a form is creating, for showing it once created.
    pub(crate) form_title: Option<String>,
    pub(crate) pull_meta: Option<(String, bool)>,
    pub(crate) notes: HashMap<(String, String), notes::NotesView>,
    pub(crate) note_name: TextEdit,
    pub(crate) confirm_delete: Option<String>,
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

impl App {
    /// Asks GitHub for the Latest build. `asked`: the user pressed Check, so say the outcome.
    pub(crate) fn check_update(&mut self, asked: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(h) = self.handle.clone() else { return };
            let Some(prior) = self.update.start_check(asked) else { return };
            self.update_checked = Some(self.clock);
            let channel = self.update_channel;
            net::spawn(h, move || async move { update::check(channel) }, move |s: &mut App, r| {
                s.apply_update_check(prior, asked, r);
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = asked;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn apply_update_check(&mut self, prior: Option<update::Release>, asked: bool, r: Result<update::Release, String>) {
        let outcome = match r {
            Ok(rel) if update::is_newer(&rel, update::build().unwrap_or(""), update::built_at()) => {
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
    pub(crate) fn install_update(&mut self) {
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
    /// Picks where updates come from, and checks there.
    pub(crate) fn set_update_channel(&mut self, channel: update::Channel) {
        if self.update_channel == channel {
            return;
        }
        self.update_channel = channel;
        channel.save();
        // A download from the other channel stops (unless it's already being installed).
        if matches!(self.update, update::Update::Downloading { .. }) {
            self.cancel_update();
        }
        // What was found on the other channel doesn't count here.
        if !matches!(self.update, update::Update::Unavailable(_) | update::Update::Downloading { .. } | update::Update::Installing) {
            self.update = update::Update::Idle;
            self.check_update(false);
        }
    }

    pub(crate) fn cancel_update(&mut self) {
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
    pub(crate) fn tray_action(&mut self, id: &str) {
        let Some(action) = tray::parse(id) else { return };
        let repo = |s: &App, full: &str| s.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full));
        let show = !matches!(action, tray::Action::Refresh | tray::Action::Quit);
        match action {
            tray::Action::Open => {}
            tray::Action::Go(p) => self.go(p),
            tray::Action::MyFailingPulls => {
                let me = self.data.profile.login.trim();
                let is_failing = |p: &model::Pull| {
                    p.checks == Some(model::Checks::Failing)
                        || self.data.runs.iter().any(|r| r.checks == model::Checks::Failing && same_repo(&r.repo, &p.repo) && r.pull_numbers.contains(&p.number))
                };
                let draft_only = !me.is_empty()
                    && !self.data.pulls.iter().any(|p| p.state == PullState::Open && p.author.eq_ignore_ascii_case(me) && is_failing(p))
                    && self.data.pulls.iter().any(|p| p.state == PullState::Draft && p.author.eq_ignore_ascii_case(me) && is_failing(p));
                self.go(Page::Pulls);
                self.pr_filter = if draft_only { PullState::Draft } else { PullState::Open };
                self.pull_filter.clear();
                self.pull_filter.mine = true;
                self.pull_filter.failing = true;
            }
            tray::Action::ReviewRequested => self.show_review_requested(),
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
    pub(crate) fn pick_board_color(&mut self, ci: usize) {
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
    pub(crate) fn account(&self) -> state::Account {
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
    pub(crate) fn reset_account_state(&mut self) {
        self.load_gen = self.load_gen.wrapping_add(1);
        self.board_load_gen = self.board_load_gen.wrapping_add(1);
        // The saved copy and the tagged answers were the other account's.
        self.forget_copy();
        // Drawings are files on this computer (or the server's notes folder), not account data:
        // writes in flight and queued ones still finish, so no edit is lost on an account switch.
        self.leave_tab();
        self.inbox = None;
        self.inbox_busy.clear();
        // Another account has other repositories and labels.
        self.pull_filter.clear();
        self.issue_filter.clear();
        self.choices.clear();
        self.issue_picks = triage::Picks::default();
        self.runs_filter.clear();
        self.data = Data::default();
        self.data_gen = self.data_gen.wrapping_add(1);
        self.repos_limit = pages::REPOS_PAGE;
        self.refresh_clone_cache();
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
        self.commits = commits::Commits::default();
        self.releases = releases::Releases::default();
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
    pub(crate) fn save_account(&mut self, sign_out: bool) {
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
            self.data_gen = self.data_gen.wrapping_add(1);
            self.set_status(Status::Sample);
            self.notify("Signed out. Showing the sample data.");
        }
    }

    /// The phone app: the account is kept by the app's own side (`PUT api/account`), which
    /// answers with the new configuration. The token never comes back to the page.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save_account(&mut self, sign_out: bool) {
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
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.set_status(Status::Sample);
                    s.notify("Signed out. Showing the sample data.");
                }
            }
        });
    }

    /// Picks the theme and remembers it.
    pub(crate) fn set_theme(&mut self, theme: Theme) {
        state::trace("theme", &self.theme, &theme);
        self.theme = theme;
        prefs::save_theme(theme);
    }

    /// Uses look `id` for its appearance and shows that appearance (`shown_dark` is what's on
    /// screen now). One of your looks opens in the editor.
    pub(crate) fn choose_look(&mut self, id: &str, shown_dark: bool) {
        let Some(dark) = self.looks.choose(id) else { return };
        state::trace("look", &shown_dark, &id);
        if dark != shown_dark {
            self.set_theme(if dark { Theme::Dark } else { Theme::Light });
        }
        self.looks.save();
        self.edit_look(id);
    }

    /// Opens look `id` in the editor (nothing for a ready-made one).
    pub(crate) fn edit_look(&mut self, id: &str) {
        self.look_edit = match self.looks.get(id).filter(|t| !t.builtin) {
            Some(t) => LookEditor::of(t),
            None => LookEditor::default(),
        };
    }

    /// Makes an editable copy of look `id` and opens it.
    pub(crate) fn duplicate_look(&mut self, id: &str, shown_dark: bool) {
        if let Some(new) = self.looks.duplicate(id) {
            self.choose_look(&new, shown_dark);
            self.notify("Copied: change it below");
        }
    }

    pub(crate) fn delete_look(&mut self, id: &str) {
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
    pub(crate) fn change_look(&mut self, f: impl FnOnce(&mut themes::Look)) {
        let id = self.look_edit.id.clone();
        if let Some(t) = self.looks.get_mut(&id) {
            f(t);
            self.looks.save();
        }
    }

    /// Takes what was typed in the editor: valid colours apply at once.
    pub(crate) fn look_typed(&mut self) {
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

    /// Lists repository `i`'s local clone, if one is known (`again`: even if listed already).
    pub(crate) fn load_local(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = self.known_clone(&full) else { return };
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
    pub(crate) fn load_changes(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else { return };
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
    pub(crate) fn git_run(&mut self, i: usize, job: &'static str, done: &'static str, f: impl FnOnce(&std::path::Path) -> Result<String, String> + Send + 'static) {
        if self.git_job.is_some() {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else {
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

    pub(crate) fn commit(&mut self, i: usize, then_push: bool) {
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
    pub(crate) fn preview_merge(&mut self, i: usize, branch: String) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else { return };
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
    pub(crate) fn merge_local(&mut self, i: usize, branch: String) {
        let b = branch.clone();
        self.git_run(i, "Merging…", "Merged:", move |root| match local::merge(root, &b)? {
            local::Merged::Done(summary) => Ok(summary),
            local::Merged::UpToDate => Ok("already up to date".into()),
            local::Merged::Conflicts(files) => Err(format!("{} file{} in conflict: resolve them, then Commit here to finish the merge (or Abort).", files.len(), if files.len() == 1 { "" } else { "s" })),
        });
    }

    /// Opens `path` in repository `i`'s local clone: a folder lists it, a file shows it.
    pub(crate) fn open_local(&mut self, i: usize, path: String, dir: bool) {
        if dir {
            self.local_dir = path;
            self.local_file = None;
            return;
        }
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let Some(root) = self.known_clone(&full) else { return };
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

    /// The branch the Notes tab shows for repository `i`: the one picked, else the default branch.
    pub(crate) fn notes_branch(&self, i: usize) -> Option<(String, String)> {
        let r = self.data.repos.get(i)?;
        let b = self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone());
        Some((r.full_name.clone(), b))
    }

    pub(crate) fn apply_notes(&mut self, epoch: u64, key: (String, String), r: Result<Vec<notes::Note>, String>) {
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
    pub(crate) fn load_notes(&mut self, i: usize) {
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
    pub(crate) fn pick_branch(&mut self, i: usize, branch: String) {
        self.set_branch(i, branch);
        self.load_notes(i);
    }

    pub(crate) fn delete_note(&mut self, i: usize, name: String) {
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
            if r.is_ok() {
                s.sync_saved(&id);
            }
            s.apply_note_op(epoch, id, r);
        });
    }

    /// Opens drawing `name` of repository `i` (on the selected branch) in the board editor.
    pub(crate) fn apply_board_load(&mut self, epoch: u64, board_epoch: u64, id: (String, String, String), r: Result<Option<String>, String>) {
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
    pub(crate) fn open_board(&mut self, i: usize, name: String, new: bool) {
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
    pub(crate) fn save_board(&mut self) {
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
        let Some(b) = self.board.as_mut() else { return };
        let id = (b.repo.clone(), b.branch.clone(), b.name.clone());
        let json = b.save_json();
        self.write_note(id, json);
    }

    fn write_note(&mut self, id: (String, String, String), json: String) {
        #[cfg(test)]
        self.note_log.push(format!("save {}: {}", id.2, json.len()));
        let Some(h) = self.handle.clone() else { return };
        self.saving_notes.insert(id.clone(), NoteOp::Save(json.clone()));
        let (repo, branch, name) = (id.0.clone(), id.1.clone(), id.2.clone());
        let epoch = self.load_gen;
        net::spawn(h, move || async move { notes::save(&repo, &branch, &name, json).await }, move |s: &mut App, r: Result<Option<String>, String>| {
            if let Ok(merged) = &r {
                if let Some(m) = merged {
                    s.board_merged(&id, m);
                }
                s.sync_saved(&id);
            }
            s.apply_note_op(epoch, id, r.map(|_| ()));
        });
    }

    /// A drawing's running operation (a write or a delete) finished.
    pub(crate) fn apply_note_op(&mut self, epoch: u64, id: (String, String, String), r: Result<(), String>) {
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
        let Some(b) = self.board.as_mut() else { return };
        if !b.save.pending() {
            return;
        }
        let id = (b.repo.clone(), b.branch.clone(), b.name.clone());
        let json = b.save_json();
        if self.saving_notes.contains_key(&id) {
            self.pending_note_saves.insert(id, NoteOp::Save(json));
        } else {
            self.write_note(id, json);
        }
    }

    /// Closes the editor and refreshes the list it came from.
    pub(crate) fn close_board(&mut self, i: usize) {
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
    pub(crate) fn ask_claude(&mut self, i: usize, prompt: String) {
        let prompt = prompt.trim().to_owned();
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = self.known_clone(&full) else { return };
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
    pub(crate) fn ask_claude(&mut self, _: usize, _: String) {}

    /// Stops the answer Claude is writing for repository `i`.
    pub(crate) fn stop_claude(&mut self, i: usize) {
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
    pub(crate) fn new_chat(&mut self, i: usize) {
        if let Some(r) = self.data.repos.get(i) {
            self.chats.remove(&r.full_name);
        }
    }

    /// Starts a shell for repository `i` in `dir` (its clone, or where a clone will go).
    pub(crate) fn start_terminal(&mut self, i: usize, dir: std::path::PathBuf, first: Option<String>) {
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
    pub(crate) fn install_plugin(&mut self, example: bool) {
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
    pub(crate) fn remove_plugin(&mut self, id: &str) {
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
    pub(crate) fn set_claude(&mut self, on: bool) {
        state::trace("claude", &self.assist.claude, &on);
        self.assist.claude = on;
        self.assist.save();
        if !on && self.tab == RepoTab::Claude {
            self.tab = RepoTab::Code;
            self.save_place();
        }
    }

    /// Adds the harness typed in Settings (or `suggested`, a name and command).
    pub(crate) fn add_harness(&mut self, suggested: Option<(&str, &str)>) {
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

    pub(crate) fn remove_harness(&mut self, n: usize) {
        if let Some(h) = self.assist.remove(n) {
            self.assist.save();
            self.notify(format!("Removed {}", h.name));
        }
    }

    /// Starts harness `n` in repository `i`'s Terminal: typed into the running shell, or as the
    /// first command of a new one in the known clone.
    pub(crate) fn run_harness(&mut self, i: usize, n: usize) {
        let Some(h) = self.assist.harnesses.get(n).cloned() else { return };
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        self.go(Page::Repo(full));
        self.set_tab(i, RepoTab::Terminal);
        let line = format!("{}\r", h.command);
        let running = self.data.repos.get(i).and_then(|r| self.terms.get(&r.full_name)).is_some_and(|t| t.is_running());
        if running {
            self.term_type(i, &line);
        } else if let Some(dir) = self.data.repos.get(i).map(|r| r.full_name.clone()).and_then(|full| self.known_clone(&full)) {
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
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        if self.terms.get(&full).is_some_and(|t| t.is_running()) {
            return;
        }
        if let Some(dir) = self.known_clone(&full) {
            self.start_terminal(i, dir, None);
        }
    }

    /// Uses the folder typed in the field as repository `i`'s clone.
    /// Uses the folder that has the repository's name but isn't known to be its clone: the
    /// user says it is.
    pub(crate) fn use_lookalike(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let Some(dir) = self.lookalike_clone(&full) else { return };
        match clones_shim::remember(&full, &dir) {
            Ok(()) => {
                self.clone_dirs.borrow_mut().insert(full.clone(), Some(dir.clone()));
                self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
                self.term_error = None;
                self.notify(format!("Using {} for {full}", dir.display()));
                self.load_local(i, true);
            }
            Err(e) => self.term_error = Some(e),
        }
    }

    pub(crate) fn use_folder(&mut self, i: usize) {
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
        self.clone_dirs.borrow_mut().insert(full.clone(), Some(dir.clone()));
        self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
        self.clone_path.clear();
        self.terms.remove(&full);
        self.load_local(i, true);
        // From the Claude tab the folder is all that's needed; the Terminal tab opens a shell there.
        if self.tab == RepoTab::Terminal {
            self.start_terminal(i, dir, None);
        }
    }

    /// Clones repository `i` into the default folder, in its terminal, and remembers where.
    pub(crate) fn clone_repo(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, name, url) = (r.full_name.clone(), r.name.clone(), r.url.clone());
        let parent = self.default_clone_parent.clone();
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
        let target = parent.join(&name);
        let _ = clones_shim::remember(&full, &target);
        self.clone_dirs.borrow_mut().insert(full.clone(), Some(target));
        self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
        self.terms.remove(&full);
        // The clone runs in the terminal, so show it.
        if self.tab != RepoTab::Terminal {
            self.set_tab(i, RepoTab::Terminal);
        }
        self.start_terminal(i, parent, Some(command));
    }

    /// Types `text` into repository `i`'s terminal.
    pub(crate) fn term_type(&mut self, i: usize, text: &str) {
        if let Some(t) = self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            t.write(text.as_bytes());
        }
    }

    pub(crate) fn term_type_bytes(&mut self, i: usize, bytes: &[u8]) {
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
        pull_review: None,
        reopening: Vec::new(),
        inbox: None,
        inbox_busy: std::collections::HashSet::new(),
        pull_filter: filters::ListFilter::default(),
        issue_filter: filters::ListFilter::default(),
        runs_filter: runs_page::RunsFilter::default(),
        commits: commits::Commits::default(),
        releases: releases::Releases::default(),
        reveal_tab: false,
        thread: None,
        thread_epoch: 0,
        choices: triage::Cache::default(),
        issue_picks: triage::Picks::default(),
        saved_filters: None,
        open_run: None,
        run_epoch: 0,
        fresh: cache::Fresh::default(),
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

/// Whether `b` is the drawing `id` (repository, branch, name).
fn board_is(b: &board::Board, id: &(String, String, String)) -> bool {
    (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) == (id.0.as_str(), id.1.as_str(), id.2.as_str())
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

impl App {
    /// Cached local clone path for `full`, probing disk/remotes at most once until invalidated.
    pub(crate) fn known_clone(&self, full: &str) -> Option<std::path::PathBuf> {
        if let Some(cached) = self.clone_dirs.borrow().get(full) {
            return cached.clone();
        }
        let found = clones_shim::find(full);
        self.clone_dirs.borrow_mut().insert(full.to_owned(), found.clone());
        found
    }

    /// Cached lookalike checkout path for `full`, probing disk/remotes at most once until invalidated.
    pub(crate) fn lookalike_clone(&self, full: &str) -> Option<std::path::PathBuf> {
        if let Some(cached) = self.lookalike_dirs.borrow().get(full) {
            return cached.clone();
        }
        let found = clones_shim::lookalike(full);
        self.lookalike_dirs.borrow_mut().insert(full.to_owned(), found.clone());
        found
    }

    /// Forgets that repository `i` had no local clone, so the next look checks the disk again:
    /// a clone made outside RepoHub (in a terminal) shows when its page or a section is opened.
    fn recheck_missing_clone(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        if self.clone_dirs.borrow().get(&full) == Some(&None) {
            self.clone_dirs.borrow_mut().remove(&full);
            self.lookalike_dirs.borrow_mut().remove(&full);
        }
    }

    pub(crate) fn refresh_clone_cache(&mut self) {
        self.clone_dirs.borrow_mut().clear();
        self.lookalike_dirs.borrow_mut().clear();
        self.default_clone_parent = clones_shim::default_parent();
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
