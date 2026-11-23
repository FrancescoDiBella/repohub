//! The keyboard (#69): `/` searches the page, `j` / `k` and the arrows move through its list and
//! Enter opens a row, `i` and `p` start a New issue or pull request in a repository, and none of
//! the one-key shortcuts fires while a text field or the board editor has the keyboard. Focus
//! goes back where it was when the palette closes.

use rune::{Key, KeyEvent, Mods};

use crate::model::Data;
use crate::state::{Compose, RepoTab};
use crate::{App, Page};

fn app(page: Page) -> rune::Runtime<App> {
    let mut s = crate::init();
    s.data = Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.page = page;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.on_key = Some(crate::on_key);
    rt.reduced_motion = true;
    rt.frame(0.0);
    rt
}

fn key(rt: &mut rune::Runtime<App>, key: Key, mods: Mods, t: &mut f64) {
    rt.key(KeyEvent { key, mods });
    *t += 0.05;
    rt.frame(*t);
}

fn letter(rt: &mut rune::Runtime<App>, c: &str, t: &mut f64) {
    key(rt, Key::Text(c.into()), Mods::default(), t);
}

/// Whether element `k` (or the field inside it) has the keyboard.
fn focused(rt: &rune::Runtime<App>, k: &str) -> bool {
    rt.elements().iter().any(|e| e.focused && (e.key.as_deref() == Some(k) || e.path.contains(&format!("/{k}/"))))
}

#[test]
fn slash_searches_the_page_and_letters_then_type_into_it() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    for (page, field) in [(Page::Issues, "issues-search"), (Page::Pulls, "pulls-search"), (Page::Repos, "search")] {
        let mut rt = app(page.clone());
        letter(&mut rt, "/", &mut t);
        t += 0.05;
        rt.frame(t);
        assert!(focused(&rt, field), "/ on {page:?} focuses {field}");
        // Now the letters are typed: no shortcut fires.
        letter(&mut rt, "j", &mut t);
        assert!(rt.state.cursor.is_none(), "j typed into the field doesn't move through the list");
        let typed = match page {
            Page::Issues => rt.state.issue_filter.search.text.clone(),
            Page::Pulls => rt.state.pull_filter.search.text.clone(),
            _ => rt.state.search.text.clone(),
        };
        assert_eq!(typed, "j");
    }
    // Where there's no search field, the palette opens.
    let mut rt = app(Page::Overview);
    letter(&mut rt, "/", &mut t);
    assert!(rt.state.palette.is_some());
}

#[test]
fn j_and_k_move_through_the_list_and_enter_opens_the_row() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    let mut rt = app(Page::Issues);
    let rows: Vec<String> = crate::keyboard::rows(&rt.state).into_iter().map(|(k, _)| k).collect();
    assert!(rows.len() >= 3);
    letter(&mut rt, "j", &mut t);
    assert_eq!(rt.state.cursor.as_deref(), Some(rows[0].as_str()), "j marks the first row");
    letter(&mut rt, "j", &mut t);
    key(&mut rt, Key::Down, Mods::default(), &mut t);
    assert_eq!(rt.state.cursor.as_deref(), Some(rows[2].as_str()), "j and ↓ move down");
    letter(&mut rt, "k", &mut t);
    assert_eq!(rt.state.cursor.as_deref(), Some(rows[1].as_str()), "k moves up");
    // The marked row says so, to the eye and to assistive technology.
    let tinted = |rt: &rune::Runtime<App>, key: &str| {
        let els = rt.elements();
        let row = els.iter().find(|e| e.key.as_deref() == Some(key)).unwrap();
        row.parent.is_some_and(|p| els[p].fill.a > 0.0)
    };
    assert!(tinted(&rt, &rows[1]) && !tinted(&rt, &rows[0]), "the marked row is tinted, the others aren't");
    key(&mut rt, Key::Enter, Mods::default(), &mut t);
    let opened = rt.state.thread.as_ref().map(|o| format!("issue-{}-{}", o.repo, o.number));
    assert_eq!(opened.as_deref(), Some(rows[1].as_str()), "Enter opens it");
    // In the conversation the list isn't on screen: j does nothing; Esc goes back, still marked.
    letter(&mut rt, "j", &mut t);
    assert_eq!(rt.state.cursor.as_deref(), Some(rows[1].as_str()));
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.thread.is_none());
    // Repositories: Enter opens the repository.
    let mut rt = app(Page::Repos);
    letter(&mut rt, "j", &mut t);
    key(&mut rt, Key::Enter, Mods::default(), &mut t);
    assert_eq!(rt.state.page, Page::Repo(rt.state.data.repos[rt.state.filtered_repos()[0]].full_name.clone()));
    assert!(rt.state.cursor.is_none(), "a new page starts unmarked");
}

#[test]
fn i_and_p_start_a_new_issue_or_pull_request_in_a_repository() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    let mut rt = app(Page::Repos);
    letter(&mut rt, "i", &mut t);
    assert!(rt.state.form.is_none(), "not on the list of repositories");
    rt.update(|s, _| s.open(0));
    t += 0.05;
    rt.frame(t);
    letter(&mut rt, "i", &mut t);
    assert_eq!(rt.state.form.as_ref().map(|f| f.kind), Some(Compose::Issue));
    assert_eq!(rt.state.tab, RepoTab::Issues);
    // The form's title has the keyboard: "p" is typed there, not a shortcut.
    t += 0.05;
    rt.frame(t);
    letter(&mut rt, "p", &mut t);
    assert_eq!(rt.state.form.as_ref().map(|f| (f.kind, f.title.text.as_str())), Some((Compose::Issue, "p")));
    // Without a field focused, p opens the New pull request form (the empty issue closes at once).
    rt.update(|s, _| {
        s.close_form();
    });
    t += 0.05;
    rt.frame(t);
    letter(&mut rt, "p", &mut t);
    assert_eq!(rt.state.form.as_ref().map(|f| f.kind), Some(Compose::Pull));
}

#[test]
fn one_key_shortcuts_leave_the_board_editor_alone() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    let mut rt = app(Page::Repos);
    rt.update(|s, _| {
        s.open(0);
        s.tab = RepoTab::Notes;
        let r = &s.data.repos[0];
        s.board = Some(crate::board::Board::new(&r.full_name, &r.branch, "Sketch"));
    });
    t += 0.05;
    rt.frame(t);
    for c in ["i", "p", "/", "?", "j"] {
        letter(&mut rt, c, &mut t);
    }
    assert!(rt.state.form.is_none() && rt.state.palette.is_none() && rt.state.page != Page::Settings && rt.state.cursor.is_none());
}

#[test]
fn focus_goes_back_where_it_was_when_the_palette_closes() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    let mut rt = app(Page::Issues);
    letter(&mut rt, "/", &mut t);
    t += 0.05;
    rt.frame(t);
    assert!(focused(&rt, "issues-search"));
    let cmd = Mods { cmd: true, ..Mods::default() };
    key(&mut rt, Key::Command('k'), cmd, &mut t);
    assert!(rt.state.palette.is_some() && focused(&rt, "palette-input"), "⌘K from a field opens the palette, focused");
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    t += 0.05;
    rt.frame(t);
    assert!(rt.state.palette.is_none());
    assert!(focused(&rt, "issues-search"), "Esc gives the keyboard back to the search field");
    // After running a command it stays where the command put it (a new page).
    key(&mut rt, Key::Command('k'), cmd, &mut t);
    rt.update(|s, _| if let Some(p) = s.palette.as_mut() { p.query = rune::TextEdit::new("Repositories") });
    t += 0.05;
    rt.frame(t);
    key(&mut rt, Key::Enter, Mods::default(), &mut t);
    assert!(rt.state.palette.is_none() && rt.state.page == Page::Repos);
}


/// Esc closes each thing opened over a page (the questions, pickers, editors, forms and the
/// conversation are in `fix69_tests`, `confirm_tests` and `followups_discard`).
#[test]
fn esc_closes_a_diff_a_run_and_a_file_too() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0.0;
    let mut rt = app(Page::Pulls);
    let pr = rt.state.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    let repo = crate::changes_page::pull_repo(&pr.url).unwrap();
    rt.update(move |s, _| s.open_pull_diff(repo, 14));
    t += 0.05;
    rt.frame(t);
    assert!(rt.state.pull_diff_of.is_some());
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.pull_diff_of.is_none(), "the diff");
    let run = rt.state.data.runs.iter().find(|r| r.id > 0).unwrap().clone();
    rt.update(move |s, _| { s.go(Page::Actions); s.open_run(&run); });
    t += 0.05;
    rt.frame(t);
    assert!(rt.state.open_run.is_some());
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.open_run.is_none() && rt.state.page == Page::Actions, "the run");
    rt.update(|s, _| { s.open(0); s.local_file = Some(("README.md".into(), crate::state::Load::Loading)); });
    t += 0.05;
    rt.frame(t);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.local_file.is_none(), "the file");
    rt.update(|s, _| s.palette = Some(crate::palette::Palette::new()));
    t += 0.05;
    rt.frame(t);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.palette.is_none(), "the palette");
}
