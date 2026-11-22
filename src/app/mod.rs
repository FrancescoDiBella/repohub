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
mod keys;

pub(crate) use chrome::view;
#[cfg(test)]
pub(crate) use chrome::banner;
pub(crate) use keys::on_key;
#[cfg(test)]
pub(crate) use keys::shortcuts;

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
    pub(crate) fn go(&mut self, p: Page) {
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
        self.open_run = None;
        if self.page == Page::Notifications {
            self.load_inbox();
        }
        self.toast = None;
        self.sync_tray();
        self.save_place();
    }

    /// The place on screen, for the next start.
    pub(crate) fn place(&self) -> state::Place {
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
        self.save_filters();
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
    pub(crate) fn restore_place(&mut self) {
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
    pub(crate) fn leave_tab(&mut self) {
        self.flush_board();
        self.board = None;
        self.confirm_delete = None;
        self.term_error = None;
        self.picker = None;
        self.confirm_merge = None;
        self.thread = None;
        self.open_run = None;
        self.close_pull_diff();
        self.commits.leave();
        self.releases.leave();
    }

    /// How many unread notifications are loaded.
    pub(crate) fn unread(&self) -> usize {
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
        self.open_run = None;
        self.thread_epoch = self.thread_epoch.wrapping_add(1);
        // The one being left is kept as it is now (with what was written in it).
        if let Some(t) = self.thread.take()
            && let Load::Ready(read) = t.load
            && !self.data.sample
        {
            self.remember_thread(&t.repo, t.number, read);
        }
        // One read before in this session shows at once, and is read again behind it.
        let load = match self.remembered_thread(repo, number) {
            Some(t) if !self.data.sample => Load::Ready(t),
            _ => Load::Loading,
        };
        self.thread = Some(thread::Open { repo: repo.to_owned(), number, pull, load, reply: TextEdit::new(""), sending: false, epoch: self.thread_epoch, edit: Default::default() });
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
            let mut sample = thread::sample(&title, &author, pull, standing, self.now);
            triage::sample_thread(&mut sample, &self.data, &repo, number, pull);
            t.load = Load::Ready(sample);
            return;
        }
        let Some(h) = self.handle.clone() else {
            // One read before stays on screen.
            if !matches!(t.load, Load::Ready(_)) {
                t.load = Load::Failed("The app isn't ready to load this yet: try again in a moment.".into());
            }
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
            if let Ok(read) = &r {
                s.remember_thread(&repo, number, read.clone());
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

    /// Opens workflow run `r` in the in-app detail view (jobs, steps, and job logs).
    pub(crate) fn open_run(&mut self, r: &model::Run) {
        self.close_pull_diff();
        self.confirm_merge = None;
        self.thread = None;
        self.run_epoch = self.run_epoch.wrapping_add(1);
        let repo = runs_page::full_repo_of(self, &r.repo);
        self.open_run = Some(runs_page::OpenRun {
            repo,
            run_id: r.id,
            workflow: r.workflow.clone(),
            branch: r.branch.clone(),
            checks: r.checks,
            duration: r.duration.clone(),
            updated_at: r.updated_at,
            url: r.url.clone(),
            epoch: self.run_epoch,
            jobs: Load::Loading,
            selected_job: None,
            logs: HashMap::new(),
            write_error: None,
        });
        self.load_run_jobs();
    }

    /// Reloads jobs and cached logs for the open workflow run.
    pub(crate) fn reload_open_run(&mut self) {
        if let Some(o) = self.open_run.as_mut() {
            o.logs.clear();
            o.write_error = None;
        }
        self.load_run_jobs();
    }

    /// Reads the jobs and steps of the open workflow run from GitHub (or sample data).
    pub(crate) fn load_run_jobs(&mut self) {
        let Some(o) = self.open_run.as_mut() else { return };
        let (repo, run_id, epoch) = (o.repo.clone(), o.run_id, o.epoch);
        if self.data.sample {
            let run = o.as_run();
            let jobs = model::sample_run_jobs(&run);
            let selected = jobs.iter().find(|j| j.checks == model::Checks::Failing)
                .or_else(|| jobs.first())
                .map(|j| j.id);
            o.selected_job = selected;
            o.logs.clear();
            if let Some(jid) = selected
                && let Some(job) = jobs.iter().find(|j| j.id == jid)
            {
                o.logs.insert(jid, Load::Ready(model::sample_job_log(&run, job)));
            }
            o.jobs = Load::Ready(jobs);
            return;
        }
        let Some(h) = self.handle.clone() else {
            o.jobs = Load::Failed("The app isn't ready to load this yet: try again in a moment.".into());
            return;
        };
        if !matches!(o.jobs, Load::Ready(_)) {
            o.jobs = Load::Loading;
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_run_jobs(&cfg, &repo, run_id).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            let Some(o) = s.open_run.as_mut().filter(|o| o.epoch == epoch) else { return };
            match r {
                Ok(jobs) => {
                    let keep = o.selected_job.filter(|id| jobs.iter().any(|j| j.id == *id));
                    let pick = keep
                        .or_else(|| jobs.iter().find(|j| j.checks == model::Checks::Failing).map(|j| j.id))
                        .or_else(|| jobs.first().map(|j| j.id));
                    o.selected_job = pick;
                    o.jobs = Load::Ready(jobs);
                    if let Some(jid) = pick {
                        s.load_job_log(jid);
                    }
                }
                Err(e) if matches!(o.jobs, Load::Ready(_)) => {
                    s.notify_problem(e);
                }
                Err(e) => {
                    o.jobs = Load::Failed(e);
                }
            }
        });
    }

    /// Selects job `job_id` in the open workflow run and loads its log if not yet fetched.
    pub(crate) fn select_run_job(&mut self, job_id: u64) {
        let need_load = match self.open_run.as_mut() {
            Some(o) => {
                o.selected_job = Some(job_id);
                !matches!(o.logs.get(&job_id), Some(Load::Ready(_) | Load::Loading))
            }
            None => false,
        };
        if need_load {
            self.load_job_log(job_id);
        }
    }

    /// Fetches the plain-text log tail (last 200 lines) for `job_id` in the open workflow run.
    pub(crate) fn load_job_log(&mut self, job_id: u64) {
        let Some(o) = self.open_run.as_mut() else { return };
        if self.data.sample {
            let run = o.as_run();
            if let Some(job) = o.jobs.ready().and_then(|jobs| jobs.iter().find(|j| j.id == job_id)) {
                let log = model::sample_job_log(&run, job);
                o.logs.insert(job_id, Load::Ready(log));
            }
            return;
        }
        let Some(h) = self.handle.clone() else {
            o.logs.insert(job_id, Load::Failed("The app isn't ready to load this yet: try again in a moment.".into()));
            return;
        };
        let (repo, epoch) = (o.repo.clone(), o.epoch);
        if !matches!(o.logs.get(&job_id), Some(Load::Ready(_))) {
            o.logs.insert(job_id, Load::Loading);
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_job_log(&cfg, &repo, job_id).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            let Some(o) = s.open_run.as_mut().filter(|o| o.epoch == epoch) else { return };
            match r {
                Ok(log) => {
                    o.logs.insert(job_id, Load::Ready(log));
                }
                Err(e) if matches!(o.logs.get(&job_id), Some(Load::Ready(_))) => {
                    s.notify_problem(e);
                }
                Err(e) => {
                    o.logs.insert(job_id, Load::Failed(e));
                }
            }
        });
    }

    /// Triggers a workflow re-run (`failed_only` = failed jobs only, otherwise the whole workflow).
    pub(crate) fn rerun_open_run(&mut self, failed_only: bool) {
        let Some((repo, run_id)) = self.open_run.as_mut().map(|o| {
            o.write_error = None;
            (o.repo.clone(), o.run_id)
        }) else {
            return;
        };
        self.write(&repo, github::Write::RerunRun { run_id, failed_only });
    }

    /// Switches the repository page of repository `i` to `tab`.
    /// The sections a repository's page shows, in order: Claude only while it's on.
    pub(crate) fn tabs(&self) -> Vec<RepoTab> {
        let local = match &self.page {
            Page::Repo(full) => self.local.contains_key(full),
            _ => false,
        };
        RepoTab::ALL.into_iter().filter(|t| (*t != RepoTab::Claude || self.assist.claude) && (*t != RepoTab::Changes || local)).collect()
    }

    pub(crate) fn set_tab(&mut self, i: usize, tab: RepoTab) {
        self.plugin_tab = None;
        self.recheck_missing_clone(i);
        let tab = if self.tabs().contains(&tab) { tab } else { RepoTab::Code };
        if self.tab != tab {
            state::trace("tab", &self.tab, &tab);
            self.leave_tab();
        }
        self.tab = tab;
        self.reveal_tab = true;
        self.save_place();
        match tab {
            RepoTab::Notes => {
                self.load_notes(i);
                self.sync_now();
            }
            RepoTab::Terminal => self.open_terminal(i),
            RepoTab::Changes => self.load_changes(i),
            RepoTab::Commits => self.load_history(i, false),
            RepoTab::Releases => self.load_releases(i, false),
            _ => {}
        }
    }

    pub(crate) fn set_status(&mut self, status: Status) {
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

    /// "Updated 3 minutes ago", once GitHub's data has loaded ("Updating…" while it loads
    /// again, "Data from 2 hours ago" for the saved copy).
    pub(crate) fn freshness(&self) -> Option<String> {
        self.freshness_words(self.since(self.loaded_at, self.loaded_clock))
    }

    /// Seconds until the next automatic refresh (negative: overdue), if they're on. The
    /// interval counts from when the last load started, so a failed load waits a whole one.
    pub(crate) fn refresh_in(&self) -> Option<i64> {
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
        self.save_filters();
        // A new Latest build is looked for every 6 hours, unless one is already waiting.
        if self.update.available().is_none() && self.update_checked.is_some_and(|c| self.clock - c >= 6.0 * 3600.0) {
            self.check_update(false);
        }
        self.sync_tray();
    }

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
        } else if self.open_run.is_some() {
            self.open_run = None;
        } else if self.pull_diff_of.is_some() {
            self.close_pull_diff();
        } else if self.commits.open.is_some() {
            self.close_commit(None);
        } else if self.commits.compare.is_some() {
            self.close_compare();
        } else if !self.release_escape() {
            match self.page {
                Page::Overview => net::leave_app(),
                Page::Repo(_) => self.go(Page::Repos),
                _ => self.go(Page::Overview),
            }
        }
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

    /// Shows a short message at the bottom of the window; it goes away after a few seconds.
    pub(crate) fn notify(&mut self, msg: impl Into<String>) {
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
    pub(crate) fn undo(&mut self) {
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
            state::Undo::ReopenPull { repo, number } => self.write(&repo, github::Write::SetPullOpen { number, open: true }),
        }
    }

    /// Like `notify`, for something that went wrong: marked as a problem and shown longer.
    pub(crate) fn notify_problem(&mut self, msg: impl Into<String>) {
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

    pub(crate) fn open(&mut self, i: usize) {
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
        self.load_latest_release(i, false);
    }

    /// Reloads everything from GitHub in the background.
    pub(crate) fn refresh(&mut self) {
        if !self.cfg.live() || !self.status.can_load() {
            return;
        }
        self.retry_after = None;
        self.last_load = Some(net::now());
        self.load_clock = Some(self.clock);
        self.set_status(Status::Loading);
        // Before the window exists, the first load reports to a mailbox (see `init`).
        let to = match (self.handle.clone(), &self.fresh.early) {
            (Some(h), _) => net::To::Now(h),
            (None, Some(m)) => net::To::Later(m.clone()),
            (None, None) => return,
        };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        // The profile and the repositories show as soon as they're in (on a first start).
        let early = to.clone();
        let work = move || async move { github::load_all_with(&cfg, move |d| early.post(move |s: &mut App| s.apply_early(epoch, d))).await };
        let done = move |s: &mut App, r| s.apply(epoch, r);
        #[cfg(not(target_arch = "wasm32"))]
        net::spawn_to(to, work, done);
        #[cfg(target_arch = "wasm32")]
        if let net::To::Now(h) = to {
            net::spawn(h, work, done);
        }
    }

    pub(crate) fn apply(&mut self, epoch: u64, result: Result<Data, github::LoadError>) {
        if epoch != self.load_gen {
            return;
        }
        match result {
            Ok(d) => {
                self.retry_after = d.retry_after;
                // Details of repositories nobody pushed to since stay as they are; those of
                // repositories pushed to stay on screen and are read again when opened. Branch
                // lists stay too: opening a repository reads its list again behind it.
                let kept = cache::keep_details(&self.data, &d, std::mem::take(&mut self.details), &mut self.fresh.stale);
                self.fresh.from_copy = false;
                self.fresh.partial = false;
                self.data = d;
                self.data_gen = self.data_gen.wrapping_add(1);
                self.refresh_clone_cache();
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
                        self.load_branches(i, true);
                        if self.tab == RepoTab::Commits {
                            self.load_history(i, true);
                        }
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
                self.releases_refreshed(self.repo_index());
                self.restore_place();
                // With the lists, what's waiting for you.
                self.load_inbox();
                self.sync_after_load();
                cache::trace_time("loaded");
                // For the next start.
                self.save_copy();
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
    pub(crate) fn branch(&self, i: usize) -> String {
        self.data.repos.get(i).map(|r| self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone())).unwrap_or_default()
    }

    pub(crate) fn apply_detail(&mut self, epoch: u64, key: (String, String), d: Detail) {
        // Read again behind a page on screen: a failure keeps the page.
        let shown = matches!(self.details.get(&key), Some(Load::Ready(_)));
        if epoch == self.load_gen && !(shown && (d.error.is_some() || d.readme_error.is_some())) {
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
    pub(crate) fn load_detail(&mut self, i: usize) {
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
        let mut behind = false;
        if let Some(load) = self.details.get(&key) {
            match load {
                Load::Loading => return,
                // Read before a refresh, or from the saved copy: it stays on screen while it's read
                // again (cheaply: GitHub answers "not modified" when nothing changed).
                Load::Ready(d) if d.error.is_none() && d.readme_error.is_none() => {
                    if !self.fresh.stale.remove(&key) {
                        return;
                    }
                    behind = true;
                }
                _ => {}
            }
        }
        if !behind {
            self.details.insert(key.clone(), Load::Loading);
        }
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

    pub(crate) fn apply_pull_diff(&mut self, epoch: u64, key: (String, u32), r: Result<Vec<github::PullFile>, String>) {
        if epoch != self.load_gen || self.pull_diff_of != Some(key) {
            return;
        }
        match r {
            Ok(files) => {
                self.pull_diff.set_files(diffs::pull_files(&files));
                self.pull_diff_load = Some(Load::Ready(()));
                self.place_line_comments();
            }
            Err(e) => self.pull_diff_load = Some(Load::Failed(e)),
        }
    }

    /// Shows pull request `number` of `full`'s files as a diff.
    pub(crate) fn open_pull_diff(&mut self, full: String, number: u32) {
        let key = (full.clone(), number);
        // A merge confirmation belongs to the diff it was asked from.
        self.confirm_merge = None;
        self.pull_diff_of = Some(key.clone());
        self.pull_diff_load = Some(Load::Loading);
        self.pull_diff.set_files(Vec::new());
        self.pull_review = Some(review::Review::default());
        if self.data.sample {
            self.pull_diff.set_files(diffs::pull_files(&diffs::sample_pull_files()));
            self.pull_diff_load = Some(Load::Ready(()));
            return;
        }
        self.load_line_comments();
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

    pub(crate) fn close_pull_diff(&mut self) {
        if self.pull_diff_of.is_some() {
            self.confirm_merge = None;
        }
        self.pull_diff_of = None;
        self.pull_diff_load = None;
        self.pull_review = None;
        self.pull_diff.set_files(Vec::new());
    }

    /// Reads the comments on the open diff's lines (again), with the commit it shows.
    fn load_line_comments(&mut self) {
        let Some((full, number)) = self.pull_diff_of.clone() else { return };
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        let key = (full.clone(), number);
        net::spawn(h, move || async move { github::load_line_comments(&cfg, &full, number).await }, move |s: &mut App, r| {
            // Another account or another diff since: not for this one.
            if s.load_gen != epoch || s.pull_diff_of.as_ref() != Some(&key) {
                return;
            }
            match r {
                Ok((head, comments)) => {
                    if let Some(rv) = s.pull_review.as_mut() {
                        rv.head = head;
                        rv.comments = comments;
                    }
                    s.place_line_comments();
                }
                // The diff still shows; only its comments are missing.
                Err(e) => s.notify_problem(format!("The comments on this pull request's lines couldn't load: {e}")),
            }
        });
    }

    /// Puts the line comments and the one being written under their lines.
    fn place_line_comments(&mut self) {
        let list = self.pull_review.as_ref().map(|r| r.annotations(self.pull_diff.files())).unwrap_or_default();
        self.pull_diff.set_annotations(list);
    }

    /// The "+" by a line of the pull request's diff: a box to write a comment there.
    pub(crate) fn start_line_comment(&mut self, range: rune_kit::diff::LineRange) {
        let files = self.pull_diff.files().to_vec();
        if self.pull_review.get_or_insert_with(review::Review::default).start(&files, range) {
            self.place_line_comments();
        }
    }

    pub(crate) fn cancel_line_comment(&mut self) {
        if let Some(r) = self.pull_review.as_mut() {
            r.draft = None;
        }
        self.place_line_comments();
    }

    /// Sends the comment being written on the diff's lines.
    pub(crate) fn send_line_comment(&mut self) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some((full, number)) = self.pull_diff_of.clone() else { return };
        let Some(r) = self.pull_review.as_mut() else { return };
        let head = r.head.clone();
        let Some(d) = r.draft.as_mut() else { return };
        let body = d.text.text.trim().to_owned();
        if body.is_empty() || d.sending {
            return;
        }
        if head.is_empty() {
            self.notify_problem("The pull request's commit isn't known yet: try again in a moment.");
            return;
        }
        d.sending = true;
        let comment = github::NewLineComment {
            body,
            commit: head,
            path: d.path.clone(),
            line: d.end.line,
            side: review::side_word(d.end.side),
            start: (d.start != d.end).then(|| (d.start.line, review::side_word(d.start.side))),
        };
        self.write(&full, github::Write::LineComment { number, comment });
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

    pub(crate) fn apply_branches(&mut self, epoch: u64, key: String, r: Result<Vec<String>, String>) {
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
        if let Ok(names) = &r {
            self.commits.branches_arrived(&key, names);
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
    pub(crate) fn load_branches(&mut self, i: usize, force: bool) {
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
    pub(crate) fn set_branch(&mut self, i: usize, branch: String) {
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
        // The history follows the branch; the answer for the previous one is dropped.
        if self.tab == RepoTab::Commits {
            self.commits.open = None;
            self.load_history(i, false);
        }
    }

    /// The branch picker chose `branch` for `slot`.
    pub(crate) fn pick_branch_for(&mut self, i: usize, slot: BranchSlot, branch: String) {
        self.picker = None;
        match slot {
            BranchSlot::View => self.set_branch(i, branch),
            BranchSlot::From => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Head => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Base => if let Some(f) = self.form.as_mut() { f.into = branch },
            BranchSlot::CompareBase | BranchSlot::CompareHead => self.pick_compare(slot, branch),
            BranchSlot::ReleaseTarget => self.pick_release_target(branch),
        }
    }

    /// Opens form `kind` for repository `i` on the section it belongs to.
    pub(crate) fn open_form(&mut self, i: usize, kind: Compose) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, default, shown) = (r.full_name.clone(), r.branch.clone(), self.branch(i));
        // A pull request starts from the branch on screen, or the first other branch.
        let head = if shown != default {
            shown.clone()
        } else {
            self.branches.get(&full).and_then(Load::ready).and_then(|b| b.iter().find(|n| **n != default).cloned()).unwrap_or(shown.clone())
        };
        self.issue_picks = triage::Picks::for_repo(&full);
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
    pub(crate) fn submit_form(&mut self) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some(f) = self.form.as_mut() else { return };
        let Ok(w) = f.write() else { return };
        let w = triage::with_picks(w, &self.issue_picks);
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
        net::spawn(h, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| s.form_sent(epoch, &repo_cb, r));
    }

    /// The form's write finished: the form closes, or stays with what was typed and GitHub's
    /// reason. A form opened since (or none, after Cancel) is left as it is, and a failure is
    /// then said in a toast.
    pub(crate) fn form_sent(&mut self, epoch: u64, repo: &str, r: Result<github::Done, String>) {
        if self.load_gen != epoch {
            return;
        }
        let ours = self.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(repo) && f.submit.is_sending());
        match r {
            Ok(done) => {
                if ours {
                    self.form = None;
                    self.picker = None;
                }
                self.apply_done(repo, done);
            }
            Err(e) => match self.form.as_mut().filter(|_| ours) {
                Some(f) => f.submit = Submit::Failed(e),
                None => self.notify_problem(e),
            },
        }
    }

    /// Runs a row's write (close an issue, merge a pull request, re-run a workflow) on repository `repo`.
    pub(crate) fn write(&mut self, repo: &str, w: github::Write) {
        if actions::blocked(self).is_some() {
            return;
        }
        let full = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        let number = w.number().unwrap_or(0);
        let rerun_id = match &w {
            github::Write::RerunRun { run_id, .. } => Some(*run_id),
            _ => None,
        };
        // A triage change: what it's about, and whether it takes several requests.
        let triage = match &w {
            github::Write::Triage { change, .. } => Some((change.part(), change.requests(&full, number).len() > 1)),
            _ => None,
        };
        let key = match (rerun_id, triage) {
            (Some(run_id), _) => runs_page::run_busy_key(&full, run_id),
            (None, Some(_)) => triage::busy_key(repo, number),
            (None, None) => actions::busy_key(repo, number),
        };
        if !self.busy.insert(key.clone()) {
            return;
        }
        self.confirm_merge = None;
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
                if triage.is_some() {
                    t.edit.saving = None;
                } else {
                    t.sending = false;
                }
            }
            if s.pull_diff_of.as_ref().is_some_and(|(f, n)| *n == number && same_repo(f, &full_cb))
                && let Some(d) = s.pull_review.as_mut().and_then(|r| r.draft.as_mut())
            {
                d.sending = false;
            }
            match r {
                Ok(done) => s.apply_done(&full_cb, done),
                // A refused triage change is said where it was made, once.
                Err(e) if triage.is_some_and(|(part, partial)| s.triage_failed(&full_cb, number, part, partial, &e)) => {}
                Err(e) => {
                    if let Some(run_id) = rerun_id
                        && let Some(o) = s.open_run.as_mut().filter(|o| o.run_id == run_id && same_repo(&o.repo, &full_cb))
                    {
                        o.write_error = Some(e.clone());
                    }
                    s.notify_problem(e);
                }
            }
        });
    }

    /// Shows what a write changed at once, without waiting for GitHub's lists to catch up.
    pub(crate) fn apply_done(&mut self, repo: &str, done: github::Done) {
        let about = match &done {
            github::Done::Closed(n) | github::Done::Reopened(n) | github::Done::Merged(n) | github::Done::PullOpen(n, _) | github::Done::Commented(n) | github::Done::Reviewed(n, _) => Some(*n),
            _ => None,
        };
        let sent = matches!(done, github::Done::Commented(_) | github::Done::Reviewed(..));
        self.apply_done_inner(repo, done);
        // Counts, states and lists changed in place: the cached lists follow.
        self.data_changed();
        self.data_gen = self.data_gen.wrapping_add(1);
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

    pub(crate) fn apply_done_inner(&mut self, repo: &str, done: github::Done) {
        let full_name = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        match done {
            github::Done::Issue(c) => {
                let title = self.form_title.take().unwrap_or_default();
                let (labels, assignees) = self.picked_for_new_issue(&full_name);
                self.data.issues.insert(0, model::Issue { label: labels.first().cloned(), labels, assignees, number: c.number, title, repo: full_name.clone(), created_at: self.now, comments: 0, url: c.url });
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
                self.commits.pull_opened(&full_name, c.number, &c.url);
                let (title, draft) = self.pull_meta.take().unwrap_or_default();
                let author = self.data.profile.login.clone();
                self.data.pulls.insert(0, model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
                    number: c.number, title, repo: full_name.clone(), author,
                    state: if draft { PullState::Draft } else { PullState::Open },
                    checks: None, updated_at: self.now, comments: 0, url: c.url,
                });
                if let Some(t) = self.data.open_pulls_total.as_mut() {
                    *t += 1;
                }
                // GitHub's count of a repository's open issues includes its pull requests.
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues += 1;
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
            github::Done::LineCommented(n) => {
                if self.pull_diff_of.as_ref().is_some_and(|(f, num)| *num == n && same_repo(f, &full_name)) {
                    if let Some(r) = self.pull_review.as_mut() {
                        r.draft = None;
                    }
                    self.place_line_comments();
                    self.load_line_comments();
                }
                self.notify(format!("Comment added to pull request #{n}"));
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
                    if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                        r.issues = r.issues.saturating_sub(1);
                    }
                }
                self.notify(format!("Merged pull request #{n}"));
            }
            github::Done::PullOpen(n, open) => {
                let mut changed = false;
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    let was_open = matches!(p.state, PullState::Open | PullState::Draft);
                    if was_open != open {
                        changed = true;
                        p.state = if open { PullState::Open } else { PullState::Closed };
                    }
                }
                if changed {
                    let (from, to) = if open { (&mut self.data.closed_pulls_total, &mut self.data.open_pulls_total) } else { (&mut self.data.open_pulls_total, &mut self.data.closed_pulls_total) };
                    if let Some(t) = from.as_mut() {
                        *t = t.saturating_sub(1);
                    }
                    if let Some(t) = to.as_mut() {
                        *t += 1;
                    }
                    if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                        r.issues = if open { r.issues + 1 } else { r.issues.saturating_sub(1) };
                    }
                }
                if open {
                    self.notify(format!("Reopened pull request #{n}"));
                } else {
                    self.notify_undo(format!("Closed pull request #{n}"), state::Undo::ReopenPull { repo: full_name.clone(), number: n });
                }
            }
            github::Done::Release(r) => self.release_done(&full_name, *r),
            github::Done::Triaged { number, change, applied } => self.apply_triaged(&full_name, number, change, applied),
            github::Done::Reran { run_id, failed_only } => {
                for r in self.data.runs.iter_mut().filter(|r| r.id == run_id && same_repo(&r.repo, &full_name)) {
                    r.checks = model::Checks::Running;
                    r.updated_at = self.now;
                }
                let reload = if let Some(o) = self.open_run.as_mut().filter(|o| o.run_id == run_id && same_repo(&o.repo, &full_name)) {
                    o.checks = model::Checks::Running;
                    o.updated_at = self.now;
                    o.write_error = None;
                    true
                } else {
                    false
                };
                if reload {
                    self.reload_open_run();
                }
                self.notify(if failed_only { "Re-running failed jobs" } else { "Re-running workflow" });
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
                    s.data_gen = s.data_gen.wrapping_add(1);
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
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.sync_tray();
                }
                Err(e) => {
                    s.notify_problem(format!("Could not load more issues: {e}"));
                }
            }
        });
    }

    /// Stars or unstars repository `i`: on GitHub when a token is configured, locally otherwise.
    pub(crate) fn toggle_star(&mut self, i: usize) {
        if !self.cfg.allow_writes {
            self.notify_problem("Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true).");
            return;
        }
        let Some((on, full)) = self.data.repos.get_mut(i).map(|r| {
            let on = !r.starred;
            r.starred = on;
            r.stars = if on { r.stars + 1 } else { r.stars.saturating_sub(1) };
            (on, r.full_name.clone())
        }) else { return };
        self.data_gen = self.data_gen.wrapping_add(1);
        if !self.cfg.live() || self.cfg.token.is_none() {
            if self.cfg.live() {
                self.notify("Star changed here only: GITHUB_TOKEN is needed to save it on GitHub.");
            }
            return;
        }
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
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
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.notify_problem(format!("Couldn't change the star: {e}"));
                }
            });
        }
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

    /// Opens plugin `p`'s section on repository `i`'s page.
    pub(crate) fn open_plugin_tab(&mut self, i: usize, p: Option<String>) {
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

    /// Esc: closes what's open on top of the page, the most recent first (a question, a picker,
    /// an editor, then a form, a conversation…). Only what's on screen: a form left on another
    /// repository's page stays. True when it closed something.
    pub(crate) fn escape(&mut self, tx: &mut Tx) -> bool {
        let form_shown = matches!(&self.page, Page::Repo(full) if self.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(full)));
        let thread_edit = self.thread.as_mut().map(|t| &mut t.edit);
        if self.confirm_merge.is_some() {
            self.confirm_merge = None;
        } else if self.confirm_delete.is_some() {
            self.confirm_delete = None;
        } else if self.picker.is_some() {
            self.picker = None;
        } else if let Some(e) = thread_edit.filter(|e| e.picker.is_some() || e.title.is_some() || e.body.is_some()) {
            // A picker, or the title or description being edited: closed without saving.
            if e.picker.is_some() {
                e.picker = None;
            } else if e.title.is_some() {
                e.title = None;
            } else {
                e.body = None;
            }
        } else if form_shown && self.issue_picks.picker.is_some() {
            self.issue_picks.picker = None;
        } else if form_shown {
            self.form = None;
            self.issue_picks = Default::default();
        } else if self.release_escape() {
            // The publish question, GitHub's notes on offer, or the release form.
        } else if self.pull_diff_of.is_some() {
            self.close_pull_diff();
        } else if self.thread.is_some() {
            self.thread = None;
        } else if self.open_run.is_some() {
            self.open_run = None;
        } else if self.commits.open.is_some() {
            self.close_commit(Some(tx));
        } else if self.commits.compare.is_some() {
            self.close_compare();
        } else if self.local_file.is_some() && matches!(self.page, Page::Repo(_)) {
            self.local_file = None;
        } else if self.toast.is_some() {
            self.toast = None;
        } else {
            return false;
        }
        true
    }

    pub(crate) fn starred_count(&self) -> u32 {
        self.data.repos.iter().map(|r| r.stars).sum()
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

fn hash_stamp(f: impl FnOnce(&mut std::collections::hash_map::DefaultHasher)) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f(&mut h);
    h.finish()
}

/// The organizations whose repositories are loaded, with how many each, most first.
pub(crate) fn orgs(s: &App) -> Vec<(String, usize)> {
    s.ensure_orgs_cache();
    s.repo_cache.borrow().orgs.clone()
}

impl App {
    fn repos_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.data_gen.hash(h);
            self.data.repos.len().hash(h);
            self.data.profile.login.hash(h);
            if let Some(r) = self.data.repos.first() {
                r.full_name.hash(h);
                r.stars.hash(h);
                r.pushed.hash(h);
                r.private.hash(h);
                r.fork.hash(h);
            }
            if let Some(r) = self.data.repos.last() {
                r.full_name.hash(h);
                r.stars.hash(h);
                r.pushed.hash(h);
                r.private.hash(h);
                r.fork.hash(h);
            }
        })
    }

    fn pins_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.repos_stamp().hash(h);
            self.pins.hash(h);
            self.cfg.pinned.hash(h);
        })
    }

    fn ensure_orgs_cache(&self) {
        let stamp = self.repos_stamp();
        if self.repo_cache.borrow().orgs_stamp == Some(stamp) {
            return;
        }
        let me = &self.data.profile.login;
        let mut out: Vec<(String, usize)> = Vec::new();
        let mut mine = 0usize;
        for r in &self.data.repos {
            if r.owner.eq_ignore_ascii_case(me) {
                mine += 1;
            } else {
                match out.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(&r.owner)) {
                    Some((_, n)) => *n += 1,
                    None => out.push((r.owner.clone(), 1)),
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        let mut c = self.repo_cache.borrow_mut();
        c.orgs = out;
        c.mine_count = mine;
        c.orgs_stamp = Some(stamp);
    }

    pub(crate) fn mine_repo_count(&self) -> usize {
        self.ensure_orgs_cache();
        self.repo_cache.borrow().mine_count
    }

    fn ensure_pins_cache(&self) {
        let stamp = self.pins_stamp();
        if self.repo_cache.borrow().pins_stamp == Some(stamp) {
            return;
        }
        let repos = &self.data.repos;
        let list: Vec<usize> = if let Some(pins) = &self.pins {
            pins.iter().filter_map(|p| repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(p))).collect()
        } else if self.cfg.pinned.is_empty() {
            (0..repos.len().min(3)).collect()
        } else {
            self.cfg.pinned.iter().filter_map(|p| repos.iter().position(|r| r.name.eq_ignore_ascii_case(p) || r.full_name.eq_ignore_ascii_case(p))).collect()
        };
        let set: std::collections::HashSet<usize> = list.iter().copied().collect();
        let mut c = self.repo_cache.borrow_mut();
        c.pinned = list;
        c.pinned_set = set;
        c.pins_stamp = Some(stamp);
    }

    /// Indices into `data.repos` matching the current owner, visibility/fork filter, search, and sort.
    pub(crate) fn filtered_repos(&self) -> Vec<usize> {
        use std::hash::Hash;
        let stamp = hash_stamp(|h| {
            self.repos_stamp().hash(h);
            std::mem::discriminant(&self.owner).hash(h);
            if let OwnerFilter::Org(o) = &self.owner {
                o.hash(h);
            }
            self.filter.index().hash(h);
            self.sort.index().hash(h);
            self.search.text.hash(h);
        });
        if self.repo_cache.borrow().shown_stamp == Some(stamp) {
            return self.repo_cache.borrow().shown.clone();
        }
        let q = self.search.text.trim().to_lowercase();
        let all = &self.data.repos;
        let login = &self.data.profile.login;
        let mut shown: Vec<usize> = (0..all.len()).filter(|&i| {
            let r = &all[i];
            if !self.owner.matches(&r.owner, login) || !self.filter.matches(r.private, r.fork) {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            let keys = r.keys();
            keys.name_lower.contains(&q) || keys.desc_lower.contains(&q) || keys.topics_lower.iter().any(|t| t.contains(&q))
        }).collect();
        match self.sort {
            RepoSort::Recent => {} // GitHub's order: most recently pushed first
            RepoSort::Stars => shown.sort_by(|a, b| all[*b].stars.cmp(&all[*a].stars)),
            RepoSort::Name => shown.sort_by(|a, b| all[*a].keys().name_lower.cmp(&all[*b].keys().name_lower)),
        }
        let mut c = self.repo_cache.borrow_mut();
        c.shown = shown.clone();
        c.shown_stamp = Some(stamp);
        shown
    }

    /// Cheap fingerprint of everything [`palette::entries`] reads, so typing in the palette
    /// reuses pre-indexed entries without rebuilding or re-lowercasing them.
    pub(crate) fn palette_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.pins_stamp().hash(h);
            self.data.pulls.len().hash(h);
            if let Some(p) = self.data.pulls.first() {
                p.number.hash(h);
                std::mem::discriminant(&p.state).hash(h);
            }
            self.data.issues.len().hash(h);
            if let Some(x) = self.data.issues.first() {
                x.number.hash(h);
            }
            std::mem::discriminant(&self.page).hash(h);
            match &self.page {
                Page::Repo(full) => {
                    full.hash(h);
                    self.local.contains_key(full).hash(h);
                }
                Page::Plugin(id) => id.hash(h),
                _ => {}
            }
            self.tab.index().hash(h);
            self.plugin_tab.hash(h);
            std::mem::discriminant(&self.theme).hash(h);
            self.shown_dark.get().hash(h);
            self.looks.light.hash(h);
            self.looks.dark.hash(h);
            self.looks.all.len().hash(h);
            self.assist.claude.hash(h);
            self.assist.harnesses.len().hash(h);
            self.plugins.list.len().hash(h);
            std::mem::discriminant(&self.update).hash(h);
            self.cfg.live().hash(h);
            self.status.can_load().hash(h);
        })
    }

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

    /// Says `data` changed in place (a count, a star, a state), so what's cached from it is
    /// rebuilt on the next frame.
    pub(crate) fn data_changed(&mut self) {
        self.data_gen = self.data_gen.wrapping_add(1);
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

    /// Whether repository `i` is in the sidebar's Pinned section.
    pub(crate) fn is_pinned(&self, i: usize) -> bool {
        self.ensure_pins_cache();
        self.repo_cache.borrow().pinned_set.contains(&i)
    }

    /// Pins or unpins repository `i`. The first change starts from what the sidebar shows, so
    /// the pins it had by default stay.
    pub(crate) fn toggle_pin(&mut self, i: usize) {
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
    pub(crate) fn show_owner(&mut self, owner: OwnerFilter) {
        state::trace("owner", &self.owner, &owner);
        self.owner = owner;
        self.repos_limit = pages::REPOS_PAGE;
        self.go(Page::Repos);
    }
}

/// The pinned repositories: the names in REPOHUB_PINNED, else the three most recently pushed.
pub(crate) fn pinned(s: &App) -> Vec<usize> {
    s.ensure_pins_cache();
    s.repo_cache.borrow().pinned.clone()
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
pub(crate) fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    const EVERY: f64 = 30.0;
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    if FIRST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        cache::trace_time("first frame");
    }
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
    if std::mem::take(&mut s.reveal_tab) {
        pages::reveal_tab(s, tx);
    }
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

/// Inter for the interface and IBM Plex Mono for code and the terminal (both SIL OFL, see
/// `assets/fonts`). The mono slot prefers the system's monospaced font (SF Mono on macOS).
const FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf"),
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
    let app = rune::App::new(AppSpec { on_key: Some(on_key), poll: Some(poll), setup: Some(setup), fonts: FONTS, families: FAMILIES, ..AppSpec::new("RepoHub", (1180.0, 760.0), init, view) });
    // macOS: draw with wgpu's Metal backend rather than Rune's own, which makes coloured parts of
    // the window flicker on every key press and pointer move (#108). `RUNE_GPU_BACKEND=metal`
    // still picks Rune's, to check whether a newer Rune fixed it.
    let mut app = app.gpu_backend(rune::GpuBackend::Wgpu);
    // With the status-bar widget, closing the window keeps RepoHub there (macOS, Windows).
    if tray::keeps_running() {
        app = app.keep_running().on_close(on_close);
    }
    let app = app.on_quit(|s: &mut App, _| { s.save_layout(); s.save_filters(); });
    let handle = app.handle();
    let h2 = handle.clone();
    handle.update(move |s: &mut App, _| {
        s.handle = Some(h2.clone());
        // What the first load found before the window opened.
        if let Some(m) = s.fresh.early.take() {
            m.attach(h2.clone());
        }
        // The place of last time, over the saved copy (before the first frame).
        if !s.data.repos.is_empty() {
            s.restore_place();
        }
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
        net::spawn(h2.clone(), net::fetch_config, |s: &mut App, cfg| {
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
                s.show_saved_copy();
                s.refresh();
            } else {
                s.data = Data::sample();
                s.data_gen = s.data_gen.wrapping_add(1);
                s.set_status(Status::Sample);
            }
        });
        #[cfg(target_arch = "wasm32")]
        BACK.with(|b| *b.borrow_mut() = Some(h2.clone()));
    });
    app.run();
}
