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
mod terminal;
mod terminal_page;
mod ui;

use config::Config;

/// Tests that change process environment variables hold this, so they don't race.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use model::{Data, Detail, PullState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Repos,
    Repo(usize),
    Pulls,
    Issues,
    Actions,
    Settings,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Status {
    /// Showing the sample data (no configuration, demo mode or web build).
    Sample,
    Loading,
    Ready,
    Error(String),
}

pub struct App {
    sidebar: PaneState,
    page: Page,
    filter: usize,
    tab: usize,
    pr_filter: usize,
    default_vis: usize,
    search: TextEdit,
    prefs: [bool; 5],
    pinned_open: bool,
    toast: Option<String>,
    cfg: Arc<Config>,
    data: Data,
    details: HashMap<String, Detail>,
    status: Status,
    handle: Option<AppHandle<App>>,
    // Notes (Excalidraw drawings), keyed by repository full name.
    branches: HashMap<String, Result<Vec<String>, String>>,
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
    term_error: Option<String>,
    /// The window is phone-sized: the pages use the layout in `mobile`.
    compact: bool,
}

impl App {
    fn go(&mut self, p: Page) {
        self.page = p;
        self.toast = None;
    }

    fn open(&mut self, i: usize) {
        self.tab = 0;
        self.go(Page::Repo(i));
        self.load_detail(i);
    }

    /// Reloads everything from GitHub in the background.
    fn refresh(&mut self) {
        if !self.cfg.live() || self.status == Status::Loading {
            return;
        }
        self.status = Status::Loading;
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
                self.status = Status::Ready;
                if let Page::Repo(i) = self.page {
                    if i < self.data.repos.len() { self.load_detail(i) } else { self.page = Page::Repos }
                }
            }
            Err(e) => self.status = Status::Error(e),
        }
    }

    fn load_detail(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if self.data.sample {
            let d = model::sample_detail(r);
            self.details.insert(full, d);
            return;
        }
        if self.details.contains_key(&full) {
            return;
        }
        self.details.insert(full.clone(), Detail { loading: true, ..Default::default() });
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let key = full.clone();
            net::spawn(h, move || async move { github::load_detail(&cfg, &full).await }, move |s: &mut App, d| {
                s.details.insert(key, d);
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
                self.toast = Some("Star changed here only: GITHUB_TOKEN is needed to save it on GitHub.".into());
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
                    s.toast = Some(format!("Couldn't change the star: {e}"));
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
            self.branches.insert(full.clone(), Ok(Vec::new()));
            let (cfg, f) = (self.cfg.clone(), full.clone());
            let key = full.clone();
            net::spawn(h.clone(), move || async move { github::branches(&cfg, &f, &default).await }, move |s: &mut App, r| {
                s.branches.insert(key, r);
            });
        }
        let k = (full.clone(), branch.clone());
        self.notes.entry(k.clone()).or_default().loading = true;
        net::spawn(h, move || async move { notes::list(&full, &branch).await }, move |s: &mut App, r| {
            let v = s.notes.entry(k).or_default();
            v.loading = false;
            match r {
                Ok(items) => {
                    v.items = items;
                    v.error = None;
                }
                Err(e) => v.error = Some(e),
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
                Ok(()) => s.toast = Some("Drawing deleted".into()),
                Err(e) => s.toast = Some(e),
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
        b.save = board::SaveState::Loading;
        self.board = Some(b);
        let Some(h) = self.handle.clone() else { return };
        net::spawn(h, move || async move { notes::load(&full, &branch, &name).await }, |s: &mut App, r| {
            let Some(b) = s.board.as_mut() else { return };
            match r {
                Ok(Some(json)) => match b.load_json(&json) {
                    Ok(()) => b.save = board::SaveState::Saved,
                    Err(e) => b.save = board::SaveState::Error(e),
                },
                Ok(None) => b.save = board::SaveState::Saved,
                Err(e) => b.save = board::SaveState::Error(e),
            }
        });
    }

    /// Saves the open drawing in the background.
    fn save_board(&mut self) {
        let Some(b) = self.board.as_mut() else { return };
        if b.save == board::SaveState::Loading {
            return;
        }
        b.save = board::SaveState::Saving;
        let (repo, branch, name, json) = (b.repo.clone(), b.branch.clone(), b.name.clone(), b.to_json());
        let Some(h) = self.handle.clone() else { return };
        net::spawn(h, move || async move { notes::save(&repo, &branch, &name, json).await }, |s: &mut App, r| {
            if let Some(b) = s.board.as_mut() {
                b.save = match r {
                    Ok(()) if b.save == board::SaveState::Saving => board::SaveState::Saved,
                    Ok(()) => b.save.clone(),
                    Err(e) => board::SaveState::Error(e),
                };
            }
        });
    }

    /// Closes the editor and refreshes the list it came from.
    fn close_board(&mut self, i: usize) {
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
        if self.tab != pages::TERMINAL_TAB {
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
        filter: 0,
        tab: 0,
        pr_filter: 0,
        default_vis: 0,
        search: TextEdit::new(""),
        prefs: [cfg.theme == Some(true), true, false, true, true],
        pinned_open: true,
        toast: None,
        // The web build waits for repohub-server's config.json before choosing (see `start`).
        data: if live || cfg!(target_arch = "wasm32") { Data::default() } else { Data::sample() },
        details: HashMap::new(),
        status: if live || cfg!(target_arch = "wasm32") { Status::Loading } else { Status::Sample },
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
        term_error: None,
        compact: starts_compact(),
        cfg,
    };
    // `REPOHUB_BLOCKING=1` loads synchronously before the first frame (headless snapshots).
    #[cfg(not(target_arch = "wasm32"))]
    if live && std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1") {
        let r = pollster::block_on(github::load_all(&s.cfg));
        s.apply(r);
        if let Some(i) = arg("page").filter(|p| p == "repo").map(|_| 0)
            && let Some(r) = s.data.repos.get(i)
        {
            let full = r.full_name.clone();
            s.details.insert(full.clone(), pollster::block_on(github::load_detail(&s.cfg, &full)));
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
    if let Some(t) = arg("tab").and_then(|t| t.parse().ok()) {
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
        Status::Loading => ("Loading data from GitHub…".to_owned(), k.c.info.icon, false),
        Status::Error(e) => (e.clone(), k.c.danger.icon, true),
        Status::Sample if !s.cfg.demo => ("Sample data. To see your repositories, create a .env file with GITHUB_USER or GITHUB_TOKEN (see .env.example) and run RepoHub with Docker or as the desktop app.".to_owned(), k.c.warning.icon, false),
        _ => return None,
    };
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg)
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(if action { &icons::CROSS } else { &icons::BELL }, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    if action {
        r = r.child(rune_kit::Button::new("Retry").control_size(ControlSize::Small).on_press(|s: &mut App| {
            s.status = Status::Ready;
            s.refresh();
        }).build(k).key("retry"));
    }
    Some(r.key("banner"))
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let dark = s.prefs[0] || (s.cfg.theme.is_none() && cx.system_dark());
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
    let mut page = col().w_full().padding(pad).gap(16.0).items_center().child(inner.child(content));
    if let Some(msg) = &s.toast {
        page = page.child(row().gap(10.0).px(14.0).py(10.0).radius(k.radius.lg).fill(k.c.bg_inverse)
            .child(text(msg.clone()).font(k.ty.small).color(k.c.text_inverse)));
    }
    let main = col().key("page").w_full().h_full().shrink().fill(k.c.page)
        .scroll(ScrollSpec { stick_bottom: false }).child(page);
    let theme = IconButton::new(&icons::MOON).variant(IconButtonVariant::GhostMuted).label("Theme")
        .help("Dark theme").expanded(s.prefs[0]).on_press(|s: &mut App| s.prefs[0] = !s.prefs[0]).build(&k).key("theme");
    let mut trailing = row().gap(4.0).center_y();
    if s.cfg.live() {
        trailing = trailing.child(IconButton::new(&icons::REFRESH).variant(IconButtonVariant::GhostMuted).label("Refresh")
            .help("Refresh from GitHub").disabled(s.status == Status::Loading).on_press(|s: &mut App| s.refresh()).build(&k).key("refresh"));
    }
    let who = if s.data.profile.login.is_empty() { "?".to_owned() } else { s.data.profile.login.clone() };
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
            .child(ui::user(&k, &who, 26.0).ml(6.0)));
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
    if let Some(b) = s.board.as_mut()
        && b.text_at.is_none()
    {
        let changed = match &ev.key {
            rune::Key::Delete | rune::Key::Backspace => b.delete_selected(),
            rune::Key::Escape => {
                b.selected = None;
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
        if s.status == Status::Loading && s.data.repos.is_empty() {
            s.status = Status::Ready; // let refresh() start the first load
            s.refresh();
        }
        // The web build asks repohub-server for its configuration; a static host (GitHub Pages)
        // has none, and then shows the sample data.
        #[cfg(target_arch = "wasm32")]
        net::spawn(h2, || net::fetch_config(), |s: &mut App, cfg| match cfg.filter(|c| c.live()) {
            Some(cfg) => {
                if cfg.theme == Some(true) {
                    s.prefs[0] = true;
                }
                s.cfg = Arc::new(cfg);
                s.status = Status::Ready;
                s.refresh();
            }
            None => {
                s.data = Data::sample();
                s.status = Status::Sample;
            }
        });
    });
    app.run();
}
