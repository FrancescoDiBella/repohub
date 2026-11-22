//! Esc, ⌘[ and the palette's selection.

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

/// Esc closes what's on top first (a picker, an editor, a question), and never a form that
/// isn't on screen: an issue being written on a repository's page stays when Esc closes a
/// conversation on another page.
#[test]
fn esc_closes_pickers_and_editors_first_and_never_a_form_out_of_sight() {
    use crate::state::{BranchSlot, Compose, Facet};
    use crate::triage::Host;
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app(Page::Pulls);
    let mut t = 0.0;
    let pr = rt.state.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    rt.update(|s, _| s.open_thread(&pr.repo, 14, true));
    rt.frame(0.1);
    rt.update(|s, _| s.open_picker(Host::Thread, Facet::Labels));
    rt.frame(0.2);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.picker(Host::Thread).is_none(), "the labels picker closes");
    assert!(rt.state.thread.is_some(), "the conversation under it stays");
    for edit in ["title", "body"] {
        rt.update(|s, _| if edit == "title" { s.edit_title() } else { s.edit_body() });
        rt.frame(t + 0.01);
        key(&mut rt, Key::Escape, Mods::default(), &mut t);
        let e = &rt.state.thread.as_ref().expect("the conversation stays").edit;
        assert!(e.title.is_none() && e.body.is_none(), "the {edit} editor closes");
    }
    assert!(rt.state.choices.sent.is_empty(), "nothing was sent");
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.thread.is_none(), "then the conversation");

    // An issue being written on a repository's page, left for another page.
    rt.update(|s, _| {
        s.open(0);
        s.open_form(0, Compose::Issue);
        s.form.as_mut().unwrap().title = rune::TextEdit::new("Half-written issue");
        s.go(Page::Pulls);
        s.open_thread(&pr.repo, 14, true);
    });
    rt.frame(t + 0.01);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.thread.is_none(), "Esc closes the conversation on screen");
    assert_eq!(rt.state.form.as_ref().map(|f| f.title.text.as_str()), Some("Half-written issue"), "not the issue on the repository's page");
    // Back there: the branch picker, then the form's labels picker, then the form.
    rt.update(|s, _| {
        let full = s.data.repos[0].full_name.clone();
        s.go(Page::Repo(full));
        s.picker = Some(BranchSlot::View);
    });
    rt.frame(t + 0.01);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.picker.is_none() && rt.state.form.is_some(), "the branch picker first");
    rt.update(|s, _| s.open_picker(Host::Form, Facet::Labels));
    rt.frame(t + 0.01);
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.picker(Host::Form).is_none() && rt.state.form.is_some(), "the form's picker next");
    key(&mut rt, Key::Escape, Mods::default(), &mut t);
    assert!(rt.state.form.is_none(), "then the form");
}
