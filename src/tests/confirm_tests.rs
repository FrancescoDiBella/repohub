//! Asking first (#66): one question for what can't be undone, inline on a computer and a sheet
//! on a phone; Undo for what can be put back.

use rune::{Key, KeyEvent, Mods};

use crate::model::Data;
use crate::state::{Load, RepoTab};
use crate::{App, Page};

/// The sample data, as if it were an account's: writes aren't blocked (none is sent: the test
/// has no handle), so the buttons that ask can be pressed.
fn app(page: Page, compact: bool) -> rune::Runtime<App> {
    let mut s = crate::init();
    s.data = Data::sample();
    s.data.sample = false;
    let mut cfg = (*s.cfg).clone();
    cfg.user = Some("francescodibella".into());
    cfg.token = Some("test-token".into());
    cfg.allow_writes = true;
    s.cfg = std::sync::Arc::new(cfg);
    s.data_changed();
    s.pending_place = None;
    s.page = page;
    s.compact = compact;
    let size = if compact { (390.0, 844.0) } else { (1180.0, 2400.0) };
    let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
    rt.on_key = Some(crate::on_key);
    rt.reduced_motion = true;
    rt.frame(0.0);
    rt.frame(0.1);
    rt
}

fn press(rt: &mut rune::Runtime<App>, key: &str, t: &mut f64) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    *t += 0.1;
    rt.frame(*t);
}

fn esc(rt: &mut rune::Runtime<App>, t: &mut f64) {
    rt.key(KeyEvent { key: Key::Escape, mods: Mods::default() });
    *t += 0.1;
    rt.frame(*t);
}

fn shows(rt: &rune::Runtime<App>, words: &str) -> bool {
    rt.elements().iter().any(|e| e.text.as_deref() == Some(words))
}

#[test]
fn merging_asks_inline_on_a_computer_and_in_a_sheet_on_a_phone() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pr = Data::sample().pulls.into_iter().find(|p| p.number == 14).unwrap();
    let merge = format!("merge-{}", crate::actions::busy_key(&pr.repo, pr.number));
    let ask = crate::actions::merge_ask(&pr.repo, pr.number);
    let row = format!("pr-{}-{}", pr.repo, pr.number);
    let mut t = 0.1;

    // On a computer: under the row, in place of Merge, with how to merge.
    let mut rt = app(Page::Pulls, false);
    press(&mut rt, &merge, &mut t);
    assert_eq!(rt.state.asking.as_deref(), Some(ask.as_str()));
    let (_, row_y) = rt.find(&row).unwrap();
    let (_, ask_y) = rt.find(&ask).expect("the question is on the page");
    assert!(ask_y > row_y, "under its row");
    assert!(rt.find(&merge).is_none() && rt.find("sheet").is_none(), "in place of the button, not a sheet");
    assert!(rt.find("merge-method").is_some() && shows(&rt, "Merge #14?"));
    assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some(ask.as_str()) && e.role == "alertdialog"));
    // The method picked says what the yes button does.
    rt.update(|s, _| s.merge_method = 1);
    t += 0.1;
    rt.frame(t);
    assert!(shows(&rt, "Squash and merge"));
    esc(&mut rt, &mut t);
    assert!(rt.state.asking.is_none() && rt.find(&ask).is_none() && rt.find(&merge).is_some(), "Esc answers no");
    press(&mut rt, &merge, &mut t);
    press(&mut rt, &format!("{ask}-no"), &mut t);
    assert!(rt.state.asking.is_none() && rt.find(&ask).is_none(), "and so does Cancel");

    // On a phone: a sheet at the bottom of the screen; the row keeps its button.
    let mut rt = app(Page::Pulls, true);
    press(&mut rt, &merge, &mut t);
    assert_eq!(rt.state.asking.as_deref(), Some(ask.as_str()));
    let (_, sheet_y) = rt.find("sheet").expect("a sheet");
    assert!(sheet_y > 844.0 / 2.0, "in the bottom half ({sheet_y})");
    let (yes_x, yes_y) = rt.find(&format!("{ask}-yes")).expect("its answers");
    assert!(yes_x < 390.0 && yes_y < 844.0, "on screen");
    assert!(rt.find("merge-method").is_some() && rt.find(&merge).is_some());
    press(&mut rt, &format!("{ask}-no"), &mut t);
    assert!(rt.state.asking.is_none() && rt.find("sheet").is_none());
    press(&mut rt, &merge, &mut t);
    esc(&mut rt, &mut t);
    assert!(rt.state.asking.is_none() && rt.find("sheet").is_none(), "Esc closes the sheet");
    // The back button too.
    press(&mut rt, &merge, &mut t);
    let mut went = false;
    rt.update(|s, tx| went = s.back_step(tx));
    assert!(went && rt.state.asking.is_none() && rt.state.page == Page::Pulls, "back closes the question, not the page");
}

#[test]
fn the_discard_question_is_a_sheet_on_a_phone_too() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app(Page::Repos, true);
    let mut t = 0.1;
    rt.update(|s, _| {
        s.open(0);
        s.open_form(0, crate::state::Compose::Issue);
        s.form.as_mut().unwrap().title = rune::TextEdit::new("Half-written");
        s.cancel_form();
    });
    t += 0.1;
    rt.frame(t);
    assert!(rt.find("sheet").is_some() && rt.find("discard-ask").is_some() && rt.find("discard-yes").is_some());
    press(&mut rt, "discard-no", &mut t);
    assert!(rt.state.discard.is_none() && rt.state.form.is_some(), "Keep editing keeps it");
}

#[test]
fn git_asks_before_aborting_pulling_over_changes_and_pushing_to_the_default_branch() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rt = app(Page::Repos, false);
    let mut t = 0.1;
    let default = rt.state.data.repos[0].branch.clone();
    let changes = move |branch: &str, merging: bool| crate::local::Changes {
        branch: branch.into(), upstream: Some(format!("origin/{branch}")), ahead: 2, behind: 1,
        patch: "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n".into(), untracked: Vec::new(),
        merging, conflicts: Vec::new(), branches: Vec::new(),
    };
    let on = changes(&default, false);
    rt.update(move |s, _| {
        let full = s.data.repos[0].full_name.clone();
        s.page = Page::Repo(full);
        s.tab = RepoTab::Changes;
        s.changes = Some(on);
        s.diff_load = Some(Load::Ready(()));
    });
    t += 0.1;
    rt.frame(t);
    for (button, ask, words) in [
        ("changes-push", "ask-push-default", format!("Push to {default}, the default branch?")),
        ("commit-push", "ask-commit-push-default", format!("Commit and push to {default}, the default branch?")),
        ("changes-pull", "ask-pull-over-changes", "Pull with uncommitted changes?".to_owned()),
    ] {
        press(&mut rt, button, &mut t);
        assert_eq!(rt.state.asking.as_deref(), Some(ask), "{button} asks");
        assert!(rt.find(ask).is_some() && shows(&rt, &words), "{words}");
        assert!(rt.state.git_job.is_none(), "nothing ran yet");
        esc(&mut rt, &mut t);
        assert!(rt.state.asking.is_none());
    }
    // During a merge: Abort asks.
    let merging = changes(&default, true);
    rt.update(move |s, _| s.changes = Some(merging));
    t += 0.1;
    rt.frame(t);
    press(&mut rt, "merge-abort", &mut t);
    assert_eq!(rt.state.asking.as_deref(), Some("ask-abort-merge"));
    assert!(shows(&rt, "Abort the merge?"));
    esc(&mut rt, &mut t);
    // On another branch with nothing to lose, Push and Pull go at once.
    let mut other = changes("feature/x", false);
    other.patch.clear();
    rt.update(move |s, _| s.changes = Some(other));
    t += 0.1;
    rt.frame(t);
    for button in ["changes-push", "changes-pull"] {
        press(&mut rt, button, &mut t);
        assert!(rt.state.asking.is_none(), "{button} doesn't ask");
    }
}

#[test]
fn unpinning_and_removing_an_assistant_offer_undo() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("confirm-undo");
    let mut s = crate::init();
    s.data = Data::sample();
    s.data_changed();
    let before: Vec<usize> = crate::pinned(&s);
    let second = before[1];
    s.toggle_pin(second);
    assert!(!crate::pinned(&s).contains(&second));
    assert!(s.toast.as_ref().is_some_and(|t| t.undo.is_some() && t.text.starts_with("Unpinned")), "Unpin offers Undo");
    s.undo();
    assert_eq!(crate::pinned(&s), before, "pinned again, in the same place");

    let harnesses: Vec<String> = s.assist.harnesses.iter().map(|h| h.name.clone()).collect();
    assert!(!harnesses.is_empty());
    s.remove_harness(0);
    assert!(s.toast.as_ref().is_some_and(|t| t.undo.is_some()), "removing an assistant offers Undo");
    s.undo();
    assert_eq!(s.assist.harnesses.iter().map(|h| h.name.clone()).collect::<Vec<_>>(), harnesses);
}

#[test]
fn reopening_and_starring_offer_undo() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rt = app(Page::Overview, false);
    let mut s = rt.state;
    let full = s.data.repos[0].full_name.clone();
    s.apply_done(&full, crate::github::Done::Reopened(7));
    assert!(matches!(s.toast.as_ref().and_then(|t| t.undo.clone()), Some(crate::state::Undo::Close { number: 7, .. })), "a reopened issue can be closed again");
    s.apply_done(&full, crate::github::Done::PullOpen(9, true));
    assert!(matches!(s.toast.as_ref().and_then(|t| t.undo.clone()), Some(crate::state::Undo::ClosePull { number: 9, .. })), "and a reopened pull request");
    let i = s.data.repos.iter().position(|r| !r.starred).unwrap();
    let starred = s.data.repos[i].full_name.clone();
    s.toggle_star(i);
    let landed = s.landed;
    s.star_sent(s.load_gen, &starred, true, landed, Ok(()));
    assert!(s.toast.as_ref().is_some_and(|t| t.text.starts_with("Starred") && t.undo.is_some()), "a star says so, with Undo");
    s.undo();
    assert!(!s.data.repos[i].starred, "Undo takes it back");
}
