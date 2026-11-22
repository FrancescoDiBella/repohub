//! The catch-all: every page and every section of a repository's page draws from the sample data,
//! on a computer and on a phone.

use crate::state::{Load, RepoTab};
use crate::{App, Page};

/// The sample data on `page` (after `then`), drawn once in a window of a computer's size or a
/// phone's.
fn draw(phone: bool, then: impl FnOnce(&mut App)) -> rune::Runtime<App> {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.compact = phone;
    then(&mut s);
    let size = if phone { (390.0, 844.0) } else { (1180.0, 900.0) };
    let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn has(rt: &rune::Runtime<App>, key: &str) -> bool {
    match key.strip_suffix('*') {
        Some(prefix) => rt.elements().iter().any(|e| e.key.as_deref().is_some_and(|k| k.starts_with(prefix))),
        None => rt.find(key).is_some(),
    }
}

/// The page around it: the sidebar on a computer, the tab bar on a phone, and the page itself.
fn shell_is_there(rt: &rune::Runtime<App>, phone: bool, what: &str) {
    assert!(has(rt, "page"), "{what}: the page");
    if phone {
        assert!(has(rt, "tab-Overview") && !has(rt, "sidebar"), "{what}: the phone's tab bar, no sidebar");
    } else {
        assert!(has(rt, "sidebar") && !has(rt, "tab-Overview"), "{what}: the sidebar");
    }
}

#[test]
fn every_page_and_section_draws_on_a_computer_and_a_phone() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = crate::load_gen_tests::TempDataDir::new("every-page");
    // Each page, and something only it shows. A plugin page whose plugin isn't there shows the
    // overview.
    let pages = [
        (Page::Overview, "metric-Repositories"),
        (Page::Repos, "owner-All"),
        (Page::Pulls, "pulls-filters"),
        (Page::Issues, "issues-filters"),
        (Page::Actions, "runs-filters"),
        (Page::Notifications, "inbox-sample"),
        (Page::Settings, "settings-section"),
        (Page::Plugin("gone".into()), "metric-Repositories"),
    ];
    // Each section of a repository's page, and something only it shows (`*`: a key starting so).
    let sections = [
        (RepoTab::Code, "readme-copy"),
        (RepoTab::Commits, "commits-history"),
        (RepoTab::Changes, "changes-status"),
        (RepoTab::Issues, "new-issue"),
        (RepoTab::Pulls, "merge-*"),
        (RepoTab::Actions, "run-*"),
        (RepoTab::Releases, "releases-list"),
        (RepoTab::Notes, "new-note"),
        (RepoTab::Claude, "clone-path"),
        (RepoTab::Terminal, "clone-path"),
        (RepoTab::Settings, "gh-settings"),
    ];
    assert_eq!(sections.len(), RepoTab::ALL.len(), "a new section is added here too");
    for phone in [false, true] {
        let size = if phone { "phone" } else { "computer" };
        for (page, key) in pages.clone() {
            let what = format!("{page:?} on a {size}");
            let rt = draw(phone, |s| s.page = page.clone());
            assert_eq!(rt.state.page, page, "{what}");
            shell_is_there(&rt, phone, &what);
            assert!(has(&rt, key), "{what}: {key}");
        }
        for (tab, key) in sections {
            let what = format!("the {tab:?} section on a {size}");
            let rt = draw(phone, |s| {
                // Claude's section shows while it's on; Changes, once the clone's files are listed.
                s.assist.claude = true;
                if tab == RepoTab::Changes {
                    let full = s.data.repos[0].full_name.clone();
                    s.local.insert(full, Load::Ready(crate::local::Tree::default()));
                }
                s.open(0);
                s.set_tab(0, tab);
            });
            assert_eq!(rt.state.tab, tab, "{what}: opened");
            assert!(matches!(&rt.state.page, Page::Repo(full) if *full == rt.state.data.repos[0].full_name), "{what}");
            shell_is_there(&rt, phone, &what);
            assert!(has(&rt, "tabs"), "{what}: the section bar");
            assert!(has(&rt, key), "{what}: {key}");
        }
    }
}
