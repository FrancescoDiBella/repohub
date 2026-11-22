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
mod drawings;
mod frame;
mod git;
mod keys;
mod loading;
mod nav;
mod repos;
mod updates;
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
