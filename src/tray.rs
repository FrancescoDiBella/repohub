//! The status-bar widget: an icon in the macOS menu bar and the Windows notification area (and a
//! Linux tray, where one runs) with quick actions.
//!
//! - The menu is computed by [`menu`] from the app's state, so it's testable without a desktop,
//!   and pushed again whenever it changes ([`App::sync_tray`]).
//! - Windows and Linux use Rune's [`rune::Tray`], which runs on its own thread.
//! - macOS uses the `tray-icon` crate (Rune has no menu bar item yet); it lives on the main
//!   thread, where `AppHandle::update` runs, in a thread-local.
//! - On macOS and Windows, closing the window keeps RepoHub in the status bar; **Quit RepoHub**
//!   ends it. If the icon couldn't be created, closing quits as before.
use crate::{App, Page, actions};

/// One menu line. An empty `id` is a separator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

impl Entry {
    fn item(id: impl Into<String>, label: impl Into<String>) -> Entry {
        Entry { id: id.into(), label: label.into(), enabled: true }
    }
    fn off(mut self, disabled: bool) -> Entry {
        self.enabled = !disabled;
        self
    }
    fn sep() -> Entry {
        Entry { id: String::new(), label: String::new(), enabled: false }
    }
}

/// What a menu line does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Go(Page),
    OpenRepo(String),
    NewIssue(String),
    Refresh,
    Update,
    Quit,
}

/// The action behind a menu line's id.
pub fn parse(id: &str) -> Option<Action> {
    let (kind, arg) = id.split_once(':').unwrap_or((id, ""));
    Some(match kind {
        "open" => Action::Open,
        "go" => Action::Go(match arg {
            "pulls" => Page::Pulls,
            "issues" => Page::Issues,
            "actions" => Page::Actions,
            "repos" => Page::Repos,
            _ => Page::Overview,
        }),
        "repo" if !arg.is_empty() => Action::OpenRepo(arg.to_owned()),
        "new-issue" if !arg.is_empty() => Action::NewIssue(arg.to_owned()),
        "refresh" => Action::Refresh,
        "update" => Action::Update,
        "quit" => Action::Quit,
        _ => return None,
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn plural_capped(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", crate::model::capped(n), if n == 1 { one } else { many })
}

/// The menu for the app's current state.
pub fn menu(s: &App) -> Vec<Entry> {
    use crate::model::{Checks, PullState};
    let d = &s.data;
    let prs = d.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let failed = d.runs.iter().filter(|r| r.checks == Checks::Failing).count();
    let mut m = vec![
        Entry::item("open", format!("Open {}", s.cfg.title)),
        Entry::sep(),
    ];
    if let Some(r) = s.update.available() {
        m.push(Entry::item("update", format!("Install the new version and restart ({})", r.short())));
        m.push(Entry::sep());
    }
    m.extend([
        Entry::item("go:pulls", if prs == 0 { "No open pull requests".into() } else { plural_capped(prs, "open pull request", "open pull requests") }),
        Entry::item("go:issues", if d.issues.is_empty() { "No open issues".into() } else { plural_capped(d.issues.len(), "open issue", "open issues") }),
        Entry::item("go:actions", if failed == 0 { "Recent runs passed".into() } else { plural(failed, "failed run", "failed runs") }),
    ]);
    let pinned = crate::pinned(s);
    if !pinned.is_empty() {
        m.push(Entry::sep());
        for i in pinned {
            let r = &d.repos[i];
            m.push(Entry::item(format!("repo:{}", r.full_name), r.name.clone()));
        }
    }
    // A new issue goes to the repository on screen, else the first pinned one.
    let target = match s.page {
        Page::Repo(i) => d.repos.get(i),
        _ => crate::pinned(s).first().and_then(|i| d.repos.get(*i)),
    };
    m.push(Entry::sep());
    if let Some(r) = target {
        let blocked = actions::blocked(s).is_some();
        let label = if blocked { format!("New issue in {}… (connect your account)", r.name) } else { format!("New issue in {}…", r.name) };
        m.push(Entry::item(format!("new-issue:{}", r.full_name), label).off(blocked));
    }
    let refresh = match (s.status.is_busy(), s.freshness()) {
        (true, _) => "Refreshing…".to_owned(),
        (false, Some(f)) => format!("Refresh ({})", f.to_lowercase()),
        (false, None) => "Refresh".to_owned(),
    };
    m.push(Entry::item("refresh", refresh).off(!s.cfg.live() || !s.status.can_load()));
    m.push(Entry::sep());
    m.push(Entry::item("quit", format!("Quit {}", s.cfg.title)));
    m
}

/// The tooltip: the app's name and what waits for you.
pub fn tooltip(s: &App) -> String {
    let prs = s.data.pulls.iter().filter(|p| matches!(p.state, crate::model::PullState::Open | crate::model::PullState::Draft)).count();
    format!("{} · {}", s.cfg.title, plural_capped(prs, "open pull request", "open pull requests"))
}

/// Whether to show the widget: a desktop window (not a headless run), unless `REPOHUB_TRAY=0`.
pub fn wanted() -> bool {
    if cfg!(target_arch = "wasm32") || std::env::var("REPOHUB_TRAY").is_ok_and(|v| v.trim() == "0") {
        return false;
    }
    const HEADLESS: [&str; 6] = ["--snap", "--tree", "--script", "--html", "--video", "--strip"];
    !std::env::args().any(|a| HEADLESS.iter().any(|h| a == *h || a.starts_with(&format!("{h}="))))
}

/// Whether closing the window should keep RepoHub in the status bar: where a status-bar icon is
/// always visible (macOS, Windows). A Linux tray may have no host to show it.
pub fn keeps_running() -> bool {
    wanted() && (cfg!(target_os = "macos") || cfg!(windows))
}

const COLOR: &[u8] = include_bytes!("../assets/tray-32.rgba");
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const TEMPLATE: &[u8] = include_bytes!("../assets/tray-template-32.rgba");

// ------------------------------------------------------------------ Windows and Linux: Rune

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
pub struct Native(rune::Tray);

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
fn rune_menu(entries: &[Entry]) -> Vec<rune::TrayItem> {
    entries.iter().map(|e| if e.id.is_empty() { rune::TrayItem::Separator } else { rune::TrayItem::action(e.id.clone(), e.label.clone()).enabled(e.enabled) }).collect()
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
pub fn create(h: rune::AppHandle<App>, entries: &[Entry], tip: &str) -> Result<Native, String> {
    let spec = rune::TraySpec::new("io.github.francescodibella.repohub", "RepoHub", rune::TrayIcon::rgba(32, 32, COLOR.to_vec()))
        .tooltip(tip)
        .menu(rune_menu(entries));
    rune::Tray::new(spec, move |e| {
        let id = match e {
            rune::TrayEvent::Menu(id) => id,
            _ => "open".to_owned(),
        };
        h.update(move |s: &mut App, _| s.tray_action(&id));
    })
    .map(Native)
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
impl Native {
    pub fn update(&self, entries: &[Entry], tip: &str) {
        self.0.set_menu(rune_menu(entries));
        self.0.set_tooltip(tip);
    }
}

// ------------------------------------------------------------------ macOS: tray-icon

#[cfg(target_os = "macos")]
pub struct Native;

#[cfg(target_os = "macos")]
thread_local! {
    /// The menu bar item; it isn't `Send`, so it stays on the main thread.
    static ITEM: std::cell::RefCell<Option<tray_icon::TrayIcon>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "macos")]
fn mac_menu(entries: &[Entry]) -> tray_icon::menu::Menu {
    use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
    let menu = Menu::new();
    for e in entries {
        let _ = if e.id.is_empty() {
            menu.append(&PredefinedMenuItem::separator())
        } else {
            menu.append(&MenuItem::with_id(e.id.clone(), &e.label, e.enabled, None))
        };
    }
    menu
}

/// Runs on the main thread (from `AppHandle::update`).
#[cfg(target_os = "macos")]
pub fn create(h: rune::AppHandle<App>, entries: &[Entry], tip: &str) -> Result<Native, String> {
    use tray_icon::menu::MenuEvent;
    let icon = tray_icon::Icon::from_rgba(TEMPLATE.to_vec(), 32, 32).map_err(|e| e.to_string())?;
    let item = tray_icon::TrayIconBuilder::new()
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_tooltip(tip)
        .with_menu(Box::new(mac_menu(entries)))
        .build()
        .map_err(|e| e.to_string())?;
    // Menu picks arrive on the main thread; the handle goes in a Mutex so the handler is Sync.
    let h = std::sync::Mutex::new(h);
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let id = e.id.as_ref().to_owned();
        if let Ok(h) = h.lock() {
            h.update(move |s: &mut App, _| s.tray_action(&id));
        }
    }));
    ITEM.with(|t| *t.borrow_mut() = Some(item));
    Ok(Native)
}

#[cfg(target_os = "macos")]
impl Native {
    pub fn update(&self, entries: &[Entry], tip: &str) {
        let menu = mac_menu(entries);
        ITEM.with(|t| {
            if let Some(item) = t.borrow().as_ref() {
                item.set_menu(Some(Box::new(menu)));
                let _ = item.set_tooltip(Some(tip));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_follows_the_state() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.status = crate::Status::Sample;
        let m = menu(&s);
        let ids: Vec<&str> = m.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"open"));
        assert_eq!(ids.last(), Some(&"quit"));
        assert!(m.iter().any(|e| e.id == "go:pulls" && e.label.ends_with("open pull requests")), "{m:#?}");
        let first = &s.data.repos[0].full_name;
        assert!(ids.contains(&format!("repo:{first}").as_str()));
        // Sample data: nothing can be created, and there is nothing to refresh.
        let new_issue = m.iter().find(|e| e.id.starts_with("new-issue:")).expect("a new-issue line");
        assert!(!new_issue.enabled && new_issue.label.contains("connect"));
        assert!(!m.iter().find(|e| e.id == "refresh").expect("refresh").enabled);
        // On a repository's page, a new issue goes there.
        s.page = Page::Repo(1);
        let target = format!("new-issue:{}", s.data.repos[1].full_name);
        assert!(menu(&s).iter().any(|e| e.id == target));
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(parse("go:pulls"), Some(Action::Go(Page::Pulls)));
        assert_eq!(parse("repo:octo/app"), Some(Action::OpenRepo("octo/app".into())));
        assert_eq!(parse("new-issue:octo/app"), Some(Action::NewIssue("octo/app".into())));
        assert_eq!(parse("repo:"), None);
        assert_eq!(parse("nope"), None);
        assert_eq!(COLOR.len(), 32 * 32 * 4);
        assert_eq!(TEMPLATE.len(), 32 * 32 * 4);
    }
}
