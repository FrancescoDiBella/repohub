//! The command palette: ⌘K (Ctrl+K) opens a search over pages, repositories, the repository on
//! screen's sections and actions, open pull requests and issues, and app actions.
//!
//! - [`entries`] lists every command for the current state; [`search`] ranks them for a query
//!   ([`score`]: every word must match, a match at the start of a word ranks higher). Both are
//!   pure and tested.
//! - The panel is a modal layer near the top of the window: type to filter, ↑ ↓ to move, Enter
//!   to run, Esc or a click outside to close.
use rune::{El, Icon, TextEdit, Tx, col, el, row, text};
use rune_kit::{Keybind, KeybindVariant, Kit, TextInput};

use crate::state::{Compose, RepoTab, Theme};
use crate::{App, Page, icons, ui};

/// The open palette.
pub struct Palette {
    pub query: TextEdit,
    /// The highlighted row among the results.
    pub selected: usize,
}

impl Palette {
    pub fn new() -> Palette {
        let mut query = TextEdit::new("");
        query.focused = true;
        Palette { query, selected: 0 }
    }
}

/// What an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Go(Page),
    OpenRepo(usize),
    Tab(usize, RepoTab),
    Compose(usize, Compose),
    Refresh,
    Theme(Theme),
    OpenUrl(String),
    CopyClone(String),
    TogglePin(usize),
    CheckUpdate,
    InstallUpdate,
    /// The Repositories page for one owner.
    Owner(crate::state::OwnerFilter),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub title: String,
    /// Shown on the right in muted ink: "Repository", "Action"…
    pub kind: &'static str,
    /// Extra words the search matches (an owner, a repository of a pull request).
    pub detail: String,
    pub keys: &'static [&'static str],
    pub icon: &'static Icon,
    pub cmd: Command,
}

fn entry(title: impl Into<String>, kind: &'static str, icon: &'static Icon, cmd: Command) -> Entry {
    Entry { title: title.into(), kind, detail: String::new(), keys: &[], icon, cmd }
}

/// The modifier the platform's shortcuts use.
pub const MOD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };

/// Every command, in the order shown before anything is typed.
pub fn entries(s: &App) -> Vec<Entry> {
    let mut out = Vec::new();
    // The repository on screen first: what you most likely want next.
    if let Page::Repo(i) = s.page
        && let Some(r) = s.data.repos.get(i)
    {
        let n = &r.name;
        out.push(entry(format!("New issue in {n}"), "Action", &icons::ISSUE, Command::Compose(i, Compose::Issue)));
        out.push(entry(format!("New pull request in {n}"), "Action", &icons::PULL, Command::Compose(i, Compose::Pull)));
        out.push(entry(format!("New branch in {n}"), "Action", &icons::BRANCH, Command::Compose(i, Compose::Branch)));
        out.push(entry(format!("Ask Claude about {n}"), "Action", &icons::SPARK, Command::Tab(i, RepoTab::Claude)));
        out.push(if s.is_pinned(i) {
            entry(format!("Unpin {n}"), "Action", &icons::PIN, Command::TogglePin(i))
        } else {
            entry(format!("Pin {n} to the sidebar"), "Action", &icons::PIN_ON, Command::TogglePin(i))
        });
        out.push(entry(format!("Open a terminal in {n}"), "Action", &icons::TERMINAL, Command::Tab(i, RepoTab::Terminal)));
        out.push(entry(format!("Open {n} on GitHub"), "Action", &icons::EXTERNAL, Command::OpenUrl(r.url.clone())));
        out.push(entry(format!("Copy the clone URL of {n}"), "Action", &icons::COPY, Command::CopyClone(format!("{}.git", r.url))));
        for (t, label) in RepoTab::ALL.iter().zip(RepoTab::LABELS) {
            if *t != s.tab {
                out.push(entry(format!("{label} · {n}"), "Section", &icons::REPO, Command::Tab(i, *t)));
            }
        }
    }
    const PAGES: [(&str, Page, &[&str]); 6] = [
        ("Overview", Page::Overview, &[MOD, "1"]),
        ("Repositories", Page::Repos, &[MOD, "2"]),
        ("Pull requests", Page::Pulls, &[MOD, "3"]),
        ("Issues", Page::Issues, &[MOD, "4"]),
        ("Actions", Page::Actions, &[MOD, "5"]),
        ("Settings", Page::Settings, &[MOD, ","]),
    ];
    for (title, page, keys) in PAGES {
        let icon = match page {
            Page::Overview => &icons::HOME,
            Page::Pulls => &icons::PULL,
            Page::Issues => &icons::ISSUE,
            Page::Actions => &icons::PLAY,
            Page::Settings => &icons::GEAR,
            _ => &icons::REPO,
        };
        out.push(Entry { keys, ..entry(title, "Page", icon, Command::Go(page)) });
    }
    if s.cfg.live() {
        out.push(Entry { keys: &[MOD, "R"], ..entry("Refresh from GitHub", "Action", &icons::REFRESH, Command::Refresh) });
    }
    match &s.update {
        crate::update::Update::Available(r) => out.push(entry(format!("Install the new version and restart ({})", r.short()), "Update", &icons::REFRESH, Command::InstallUpdate)),
        u if u.can_check() => out.push(entry("Check for updates", "Update", &icons::REFRESH, Command::CheckUpdate)),
        _ => {}
    }
    for (t, label) in Theme::ALL.iter().zip(Theme::LABELS) {
        if *t != s.theme {
            out.push(entry(format!("Theme: {label}"), "Appearance", if *t == Theme::Dark { &icons::MOON } else { &icons::SUN }, Command::Theme(*t)));
        }
    }
    for (o, n) in crate::orgs(s) {
        out.push(Entry { detail: format!("{n} repositories"), ..entry(format!("Repositories in {o}"), "Organization", &icons::GLOBE, Command::Owner(crate::state::OwnerFilter::Org(o))) });
    }
    for (i, r) in s.data.repos.iter().enumerate() {
        out.push(Entry { detail: r.owner.clone(), ..entry(r.name.clone(), "Repository", &icons::REPO, Command::OpenRepo(i)) });
    }
    use crate::model::PullState;
    for p in s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)) {
        out.push(Entry { detail: p.repo.clone(), ..entry(format!("#{} {}", p.number, p.title), "Pull request", &icons::PULL, Command::OpenUrl(p.url.clone())) });
    }
    for x in &s.data.issues {
        out.push(Entry { detail: x.repo.clone(), ..entry(format!("#{} {}", x.number, x.title), "Issue", &icons::ISSUE, Command::OpenUrl(x.url.clone())) });
    }
    out
}

/// How well `query` matches `text` (higher is better), or `None` when some word of the query
/// doesn't appear. Each word scores most at the start of the text, then at the start of a word,
/// then anywhere, then as letters in order ("rhb" finds "repohub").
pub fn score(query: &str, text: &str) -> Option<i32> {
    let hay = text.to_lowercase();
    let mut total = 0;
    for word in query.split_whitespace().map(str::to_lowercase) {
        let at_word = hay.match_indices(&word).find(|(i, _)| *i == 0 || !hay.as_bytes()[i - 1].is_ascii_alphanumeric()).map(|(i, _)| i);
        total += if hay.starts_with(&word) {
            300
        } else if let Some(i) = at_word {
            200 - i.min(100) as i32
        } else if let Some(i) = hay.find(&word) {
            100 - i.min(90) as i32
        } else {
            // Letters in order, fewer gaps first.
            let mut gaps = 0;
            let mut chars = hay.chars();
            for c in word.chars() {
                let mut found = false;
                for h in chars.by_ref() {
                    if h == c {
                        found = true;
                        break;
                    }
                    gaps += 1;
                }
                if !found {
                    return None;
                }
            }
            (40 - gaps.min(39)) as i32
        };
    }
    // The whole name typed out wins over a longer name that starts the same.
    if hay == query.trim().to_lowercase() {
        total += 100;
    }
    Some(total)
}

/// The entries matching `query`, best first (all of them, in order, for an empty query).
pub fn search(all: Vec<Entry>, query: &str) -> Vec<Entry> {
    if query.trim().is_empty() {
        return all;
    }
    let mut ranked: Vec<(i32, usize, Entry)> = all.into_iter().enumerate().filter_map(|(n, e)| {
        let title = score(query, &e.title);
        let detail = score(query, &format!("{} {}", e.title, e.detail)).map(|v| v - 50);
        title.max(detail).map(|v| (v, n, e))
    }).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|x| x.2).collect()
}

/// The results for the open palette.
pub fn results(s: &App) -> Vec<Entry> {
    let q = s.palette.as_ref().map(|p| p.query.text.clone()).unwrap_or_default();
    search(entries(s), &q).into_iter().take(60).collect()
}

/// Runs a command (the palette is already closed).
pub fn run(s: &mut App, cmd: Command, tx: &mut Tx) {
    let show = |s: &mut App, i: usize| {
        if s.page != Page::Repo(i) {
            s.open(i);
        }
    };
    match cmd {
        Command::Go(p) => s.go(p),
        Command::OpenRepo(i) => s.open(i),
        Command::Tab(i, t) => {
            show(s, i);
            s.set_tab(i, t);
        }
        Command::Compose(i, k) => {
            show(s, i);
            s.open_form(i, k);
        }
        Command::Refresh => s.refresh(),
        Command::Theme(t) => s.set_theme(t),
        Command::OpenUrl(u) => {
            let _ = tx.open_url(&u);
        }
        Command::Owner(o) => s.show_owner(o),
        Command::TogglePin(i) => s.toggle_pin(i),
        Command::CheckUpdate => s.check_update(true),
        Command::InstallUpdate => s.install_update(),
        Command::CopyClone(u) => {
            tx.copy(u);
            s.notify("Clone URL copied");
        }
    }
}

/// Runs the highlighted result.
fn run_selected(s: &mut App, tx: &mut Tx) {
    let list = results(s);
    let n = s.palette.as_ref().map(|p| p.selected).unwrap_or(0);
    if let Some(e) = list.into_iter().nth(n) {
        s.palette = None;
        run(s, e.cmd, tx);
    }
}

/// The palette's layer (scrim and panel), when it's open.
pub fn layer(s: &App, k: &Kit) -> Vec<El> {
    let Some(p) = &s.palette else { return Vec::new() };
    let list = results(s);
    let count = list.len();
    let selected = p.selected.min(count.saturating_sub(1));
    let field = TextInput::new(&p.query).placeholder("Search repositories, pages and actions").leading_icon(k.glyph.search).large()
        .label("Command palette")
        .on_edit(|s: &mut App| &mut s.palette.as_mut().expect("palette open").query)
        .on_change(|s: &mut App, _| if let Some(p) = s.palette.as_mut() { p.selected = 0 })
        .on_submit(|s: &mut App, tx| run_selected(s, tx))
        .on_key(move |s: &mut App, ev, _| {
            let Some(p) = s.palette.as_mut() else { return false };
            match ev.key {
                rune::Key::Down => p.selected = (p.selected + 1).min(count.saturating_sub(1)),
                rune::Key::Up => p.selected = p.selected.saturating_sub(1),
                rune::Key::Escape => s.palette = None,
                _ => return false,
            }
            true
        })
        .build(k).key("palette-input").w_full();
    let mut rows = col().w_full().gap(2.0);
    if list.is_empty() {
        rows = rows.child(text("Nothing matches. Try a repository's name or a page.").font(k.ty.small).color(k.c.text_muted).padding(14.0));
    }
    for (n, e) in list.into_iter().enumerate() {
        let on = n == selected;
        let cmd = e.cmd.clone();
        let mut line = row().w_full().h(36.0).px(10.0).gap(10.0).center_y().radius(k.radius.md)
            .fill(if on { k.c.bg_accent.alpha(0.14) } else { rune::Color::TRANSPARENT }).hover(k.c.hover)
            .role("option").label(e.title.clone()).key(format!("palette-{n}"))
            .on_press_tx(move |s: &mut App, tx| {
                s.palette = None;
                run(s, cmd.clone(), tx);
            })
            .child(ui::ico(e.icon, 15.0, if on { k.c.text_accent } else { k.c.icon_muted }))
            .child(text(e.title.clone()).font(k.ty.ui).color(k.c.text_base).grow(1.0).shrink().max_lines(1));
        if !e.detail.is_empty() && e.kind != "Repository" {
            line = line.child(text(e.detail.clone()).font(k.ty.text_small).color(k.c.text_faint).max_lines(1).max_w(180.0));
        }
        line = line.child(text(e.kind).font(k.ty.text_small).color(k.c.text_muted));
        if !e.keys.is_empty() {
            line = line.child(Keybind::new(e.keys).variant(KeybindVariant::Ghost).build(k));
        }
        rows = rows.child(line);
    }
    let close = |s: &mut App| s.palette = None;
    let panel = col().key("palette").role("dialog").label("Command palette").w_full().max_w(640.0)
        .radius(k.radius.lg).fill(k.c.layer_01).border(1.0, k.c.border_base).padding(8.0).gap(6.0)
        .child(field)
        .child(col().w_full().max_h(420.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(rows))
        .child(row().w_full().px(6.0).gap(14.0)
            .child(text("↑ ↓ to move · Enter to run · Esc to close").font(k.ty.text_small).color(k.c.text_faint)))
        .modal(true, close)
        .modal_click_outside(true);
    vec![
        el().key("palette-scrim").absolute(0.0, 0.0).w_full().h_full().fill(k.c.scrim).a11y_hidden(),
        // A column, so the panel centres across the window and sits near the top.
        col().key("palette-layer").absolute(0.0, 0.0).w_full().h_full().padding(16.0).pt(72.0).items_center().child(panel),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring() {
        assert!(score("repo", "repohub").unwrap() > score("repo", "my-repohub").unwrap(), "the start of the text first");
        assert!(score("hub", "repo-hub").unwrap() > score("hub", "github").unwrap(), "then the start of a word");
        assert!(score("rhb", "repohub").is_some(), "letters in order");
        assert!(score("xyz", "repohub").is_none());
        assert!(score("new issue", "New issue in repohub").is_some());
        assert!(score("issue zzz", "New issue in repohub").is_none(), "every word must match");
    }

    #[test]
    fn entries_follow_the_state() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        let all = entries(&s);
        assert!(all.iter().any(|e| e.cmd == Command::Go(Page::Pulls)));
        assert!(all.iter().any(|e| e.cmd == Command::OpenRepo(0)));
        assert!(!all.iter().any(|e| matches!(e.cmd, Command::Compose(..))), "no repository on screen, no New issue");
        s.page = Page::Repo(1);
        let name = s.data.repos[1].name.clone();
        let found = search(entries(&s), "new issue");
        assert_eq!(found[0].cmd, Command::Compose(1, Compose::Issue), "{:?}", found.iter().map(|e| &e.title).collect::<Vec<_>>());
        let found = search(entries(&s), &name);
        assert!(matches!(found[0].cmd, Command::OpenRepo(1) | Command::Compose(1, _) | Command::Tab(1, _) | Command::OpenUrl(_) | Command::CopyClone(_)));
        assert_eq!(search(entries(&s), "settings")[0].cmd, Command::Go(Page::Settings));
    }
}
