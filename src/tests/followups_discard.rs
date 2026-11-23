//! #138: "Discard what you typed?" before Esc, a Cancel, opening another form, or moving to
//! another place throws away typed text (the New issue / branch / pull request form, the
//! release form, an edited title or description, a comment in a conversation or on a diff's
//! lines); an empty one closes at once. And the Android back button closes things in the order
//! Esc does.

use std::cell::Cell;

use rune::{Key, KeyEvent, Mods, TextEdit};

use crate::state::{BranchSlot, Compose, Facet, RepoTab};
use crate::triage::Host;
use crate::{App, Page};

thread_local! {
    static CLOCK: Cell<f64> = const { Cell::new(0.0) };
}

fn rt_of(s: App, w: f32) -> rune::Runtime<App> {
    let mut rt = rune::Runtime::new(s, crate::view, (w, 2400.0), rune::Env::new(true, 1.0));
    rt.on_key = Some(crate::on_key);
    rt.reduced_motion = true;
    rt.frame(0.0);
    rt
}

fn app() -> rune::Runtime<App> {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.open(0);
    rt_of(s, 1180.0)
}

fn settle(rt: &mut rune::Runtime<App>) {
    for _ in 0..3 {
        let t = CLOCK.with(|c| {
            c.set(c.get() + 0.4);
            c.get()
        });
        rt.frame(t);
    }
}

fn press(rt: &mut rune::Runtime<App>, key: &str) {
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn esc(rt: &mut rune::Runtime<App>) {
    rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
    settle(rt);
}

fn asking(rt: &mut rune::Runtime<App>) -> bool {
    settle(rt);
    rt.find("discard-ask").is_some()
}

fn typed_issue(rt: &mut rune::Runtime<App>, i: usize, title: &str) {
    rt.update(|s, _| s.open_form(i, Compose::Issue));
    let title = title.to_owned();
    rt.update(move |s, _| s.form.as_mut().unwrap().title = TextEdit::new(title));
    settle(rt);
}

#[test]
fn esc_and_cancel_ask_before_a_typed_form_closes_and_an_empty_one_closes_at_once() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    // Empty: Esc closes it, nothing asked (the first Esc in the focused title only leaves the field).
    rt.update(|s, _| s.open_form(0, Compose::Issue));
    rt.update(|s, _| s.form.as_mut().unwrap().title.focused = false);
    esc(&mut rt);
    assert!(rt.state.form.is_none() && !asking(&mut rt), "an empty form closes at once");
    // Typed: Esc asks; Keep editing leaves it as it was.
    typed_issue(&mut rt, 0, "Half-written");
    esc(&mut rt);
    assert!(asking(&mut rt) && rt.state.form.is_some(), "Esc asks first");
    let what = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("discard-ask")).and_then(|e| e.label).unwrap_or_default();
    assert!(what.starts_with("Discard what you typed?") && what.contains("new issue in"), "{what}");
    press(&mut rt, "discard-no");
    assert!(!asking(&mut rt));
    assert_eq!(rt.state.form.as_ref().map(|f| f.title.text.as_str()), Some("Half-written"), "kept");
    // Esc while asking is Keep editing too.
    esc(&mut rt);
    esc(&mut rt);
    assert!(!asking(&mut rt) && rt.state.form.is_some());
    // Discard closes it.
    esc(&mut rt);
    press(&mut rt, "discard-yes");
    assert!(rt.state.form.is_none() && !asking(&mut rt));
    // The form's own Cancel: at once when empty, asks when typed.
    rt.update(|s, _| s.open_form(0, Compose::Issue));
    press(&mut rt, "form-cancel");
    assert!(rt.state.form.is_none() && !asking(&mut rt));
    typed_issue(&mut rt, 0, "Again");
    press(&mut rt, "form-cancel");
    assert!(asking(&mut rt) && rt.state.form.is_some());
    press(&mut rt, "discard-yes");
    assert!(rt.state.form.is_none());
}

#[test]
fn another_form_or_another_repositorys_form_asks_before_replacing_a_typed_one() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    let (a, b) = (rt.state.data.repos[0].full_name.clone(), rt.state.data.repos[1].full_name.clone());
    // A new branch with no name yet is replaced at once.
    rt.update(|s, _| s.open_form(0, Compose::Branch));
    rt.update(|s, _| s.open_form(0, Compose::Pull));
    assert!(!asking(&mut rt) && rt.state.form.as_ref().is_some_and(|f| f.kind == Compose::Pull));
    // An issue being written for one repository, and a form opened for another.
    typed_issue(&mut rt, 0, "For the first");
    rt.update(|s, _| {
        s.open(1);
        s.open_form(1, Compose::Pull);
    });
    assert!(asking(&mut rt), "not silently replaced");
    let what = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("discard-ask-what")).and_then(|e| e.text).unwrap_or_default();
    assert!(what.contains(&a), "it says which: {what}");
    press(&mut rt, "discard-no");
    assert_eq!(rt.state.form.as_ref().map(|f| (f.repo.clone(), f.title.text.clone())), Some((a.clone(), "For the first".into())));
    rt.update(|s, _| s.open_form(1, Compose::Pull));
    press(&mut rt, "discard-yes");
    let f = rt.state.form.as_ref().expect("the new form");
    assert_eq!((f.repo.as_str(), f.kind, f.title.text.as_str()), (b.as_str(), Compose::Pull, ""));
    // A new branch with a name asks too.
    rt.update(|s, _| {
        s.form = None;
        s.open_form(1, Compose::Branch);
        s.form.as_mut().unwrap().title = TextEdit::new("feature/x");
        s.open_form(1, Compose::Issue);
    });
    assert!(asking(&mut rt) && rt.state.form.as_ref().is_some_and(|f| f.kind == Compose::Branch));
}

#[test]
fn the_release_form_asks_before_esc_or_cancel_throw_away_what_was_typed() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    rt.update(|s, _| s.set_tab(0, RepoTab::Releases));
    settle(&mut rt);
    rt.update(|s, _| s.open_release_form(0, None));
    rt.update(|s, _| s.releases.form.as_mut().unwrap().tag.focused = false);
    esc(&mut rt);
    assert!(rt.state.releases.form.is_none() && !asking(&mut rt), "empty: closed at once");
    rt.update(|s, _| s.open_release_form(0, None));
    rt.update(|s, _| {
        let f = s.releases.form.as_mut().unwrap();
        f.tag.focused = false;
        f.notes = TextEdit::new("What changed");
    });
    esc(&mut rt);
    assert!(asking(&mut rt) && rt.state.releases.form.is_some(), "typed notes: asks");
    press(&mut rt, "discard-no");
    press(&mut rt, "release-cancel");
    assert!(asking(&mut rt) && rt.state.releases.form.is_some(), "Cancel asks too");
    press(&mut rt, "discard-yes");
    assert!(rt.state.releases.form.is_none());
}

#[test]
fn an_edited_title_or_a_comment_being_written_isnt_lost_to_esc_or_moving_away() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    let p = rt.state.data.pulls.iter().find(|p| p.number == 14).cloned().unwrap();
    rt.update(move |s, _| {
        s.go(Page::Pulls);
        s.open_thread(&p.repo, 14, true);
    });
    settle(&mut rt);
    // The title opened for editing and left unchanged closes at once.
    rt.update(|s, _| s.edit_title());
    esc(&mut rt);
    assert!(rt.state.thread.as_ref().unwrap().edit.title.is_none() && !asking(&mut rt));
    // Changed: Esc asks, Discard closes the editor (the conversation stays).
    rt.update(|s, _| s.edit_title());
    rt.update(|s, _| s.thread.as_mut().unwrap().edit.title = Some(TextEdit::new("A better title")));
    esc(&mut rt);
    assert!(asking(&mut rt) && rt.state.thread.as_ref().unwrap().edit.title.is_some());
    press(&mut rt, "discard-yes");
    assert!(rt.state.thread.as_ref().is_some_and(|t| t.edit.title.is_none()));
    // Its Cancel asks the same.
    rt.update(|s, _| s.edit_body());
    rt.update(|s, _| s.thread.as_mut().unwrap().edit.body = Some(TextEdit::new("New words")));
    press(&mut rt, "triage-body-cancel");
    assert!(asking(&mut rt));
    press(&mut rt, "discard-yes");
    assert!(rt.state.thread.as_ref().is_some_and(|t| t.edit.body.is_none()));
    // A comment being written: another page asks first; Keep editing stays.
    rt.update(|s, _| s.thread.as_mut().unwrap().reply = TextEdit::new("Looks good, one question"));
    press(&mut rt, "nav-Issues");
    assert!(asking(&mut rt) && rt.state.page == Page::Pulls && rt.state.thread.is_some(), "still there");
    press(&mut rt, "discard-no");
    assert_eq!(rt.state.thread.as_ref().map(|t| t.reply.text.as_str()), Some("Looks good, one question"));
    // The conversation's Back and Esc ask too; Discard goes where it was asked to.
    press(&mut rt, "thread-back");
    assert!(asking(&mut rt) && rt.state.thread.is_some());
    press(&mut rt, "discard-no");
    esc(&mut rt);
    assert!(asking(&mut rt) && rt.state.thread.is_some());
    press(&mut rt, "discard-no");
    press(&mut rt, "nav-Issues");
    press(&mut rt, "discard-yes");
    assert!(rt.state.page == Page::Issues && rt.state.thread.is_none(), "gone where it was asked");
    // An empty reply: no question.
    let x = rt.state.data.issues[0].clone();
    rt.update(move |s, _| s.open_thread(&x.repo, x.number, false));
    press(&mut rt, "nav-Pull requests");
    assert!(!asking(&mut rt) && rt.state.page == Page::Pulls);
}

#[test]
fn a_comment_on_a_diffs_lines_isnt_lost_when_the_diff_closes() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    let p = rt.state.data.pulls.iter().find(|p| p.number == 14).cloned().unwrap();
    rt.update(move |s, _| {
        s.go(Page::Pulls);
        s.open_thread(&p.repo, 14, true);
        s.open_pull_diff(crate::changes_page::pull_repo(&p.url).unwrap(), 14);
    });
    settle(&mut rt);
    let range = rune_kit::diff::LineRange::line(0, rune_kit::diff::LineRef::new(rune_kit::diff::Side::New, 2));
    rt.update(move |s, _| s.start_line_comment(range));
    rt.update(|s, _| s.pull_review.as_mut().unwrap().draft.as_mut().unwrap().text = TextEdit::new("Why this?"));
    settle(&mut rt);
    press(&mut rt, "pull-close");
    assert!(asking(&mut rt) && rt.state.pull_diff_of.is_some(), "the diff stays while it asks");
    press(&mut rt, "discard-no");
    // Esc asks about the comment first, and Discard drops only the comment.
    esc(&mut rt);
    assert!(asking(&mut rt));
    press(&mut rt, "discard-yes");
    assert!(rt.state.pull_diff_of.is_some() && rt.state.pull_review.as_ref().is_some_and(|r| r.draft.is_none()));
    esc(&mut rt);
    assert!(rt.state.pull_diff_of.is_none() && rt.state.thread.is_some(), "then the diff, at once");
}

/// The back button closes what's open in Esc's order (a confirmation, the branch picker, a
/// triage picker, an editor, a form, a conversation), then goes up a level; from the
/// Overview it leaves the app.
#[test]
fn the_back_button_follows_escs_order() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app();
    let back = |rt: &mut rune::Runtime<App>| {
        let mut went = true;
        rt.update(|s, tx| went = s.back_step(tx));
        settle(rt);
        went
    };
    // On a repository's page: the merge question, the branch picker, the form's picker, the form.
    let pull = rt.state.data.pulls.iter().find(|p| p.state == crate::model::PullState::Open).cloned().unwrap();
    rt.update(move |s, _| {
        s.open_form(0, Compose::Issue);
        s.open_picker(Host::Form, Facet::Labels);
        s.picker = Some(BranchSlot::View);
        s.ask(crate::actions::merge_ask(&pull.repo, pull.number));
    });
    assert!(back(&mut rt) && rt.state.asking.is_none() && rt.state.picker.is_some(), "the question first");
    assert!(back(&mut rt) && rt.state.picker.is_none() && rt.state.picker(Host::Form).is_some(), "the branch picker");
    assert!(back(&mut rt) && rt.state.picker(Host::Form).is_none() && rt.state.form.is_some(), "the form's picker");
    rt.update(|s, _| s.form.as_mut().unwrap().title = TextEdit::new("Typed"));
    assert!(back(&mut rt) && rt.state.form.is_some() && rt.state.discard.is_some(), "a typed form asks");
    assert!(back(&mut rt) && rt.state.discard.is_none() && rt.state.form.is_some(), "back again: Keep editing");
    rt.update(|s, _| s.answer_discard(false));
    rt.update(|s, _| s.form.as_mut().unwrap().title = TextEdit::new(""));
    assert!(back(&mut rt) && rt.state.form.is_none(), "an empty form closes");
    // A conversation: its picker, its editor, then the conversation.
    let p = rt.state.data.pulls.iter().find(|p| p.number == 14).cloned().unwrap();
    rt.update(move |s, _| {
        s.go(Page::Pulls);
        s.open_thread(&p.repo, 14, true);
    });
    settle(&mut rt);
    rt.update(|s, _| {
        s.edit_title();
        s.open_picker(Host::Thread, Facet::Labels);
    });
    assert!(back(&mut rt) && rt.state.picker(Host::Thread).is_none() && rt.state.thread.as_ref().unwrap().edit.title.is_some(), "the picker");
    assert!(back(&mut rt) && rt.state.thread.as_ref().unwrap().edit.title.is_none(), "the editor");
    assert!(back(&mut rt) && rt.state.thread.is_none() && rt.state.page == Page::Pulls, "the conversation");
    assert!(back(&mut rt) && rt.state.page == Page::Overview, "up a level");
    assert!(!back(&mut rt), "the app leaves from the Overview");
    // Esc does the same, and closes a toast last.
    rt.update(|s, _| s.notify("Done"));
    esc(&mut rt);
    assert!(rt.state.toast.is_none());
}
