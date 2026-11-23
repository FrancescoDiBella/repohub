//! Feedback: Undo after closing or deleting, a form's answer, a change with no local copy, open counts.

use crate::{App, Page, github, state};

fn app() -> App {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = Page::Issues;
    s
}

#[test]
fn a_deleted_theme_comes_back_with_undo() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("repohub-undo-{}", std::process::id()));
    // SAFETY: the lock keeps other environment-changing tests out.
    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
    let mut s = app();
    let first = s.looks.all.iter().find(|t| t.builtin).unwrap().id.clone();
    let id = s.looks.duplicate(&first).unwrap();
    s.looks.dark = id.clone();
    let at = s.looks.all.iter().position(|t| t.id == id).unwrap();
    let before = s.looks.all.len();
    s.delete_look(&id);
    assert_eq!(s.looks.all.len(), before - 1);
    assert_ne!(s.looks.dark, id, "the dark theme fell back to a ready-made one");
    assert!(s.toast.as_ref().is_some_and(|t| t.undo.is_some()), "the toast offers Undo");
    s.undo();
    assert_eq!(s.looks.all.len(), before);
    assert_eq!(s.looks.all[at].id, id, "back where it was");
    assert_eq!(s.looks.dark, id, "and the dark theme again");
    assert!(s.toast.as_ref().is_some_and(|t| t.text.starts_with("Restored") && t.undo.is_none()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_closed_issue_offers_undo_and_comes_back_when_reopened() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let issue = s.data.issues[0].clone();
    let count = s.data.issues.len();
    s.apply_done(&issue.repo, github::Done::Closed(issue.number));
    assert_eq!(s.data.issues.len(), count - 1);
    let undo = s.toast.as_ref().and_then(|t| t.undo.clone());
    assert!(matches!(&undo, Some(state::Undo::Reopen { issue: i, .. }) if i.number == issue.number));
    // The toast's button is on screen.
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("toast-undo").is_some());
    let mut s = rt.state;
    // Pressing Undo asks GitHub to reopen it (the sample data can't, so it waits); once
    // GitHub says it's reopened, it's back in the list, once.
    s.reopening.push(issue.clone());
    s.apply_done(&issue.repo, github::Done::Reopened(issue.number));
    s.apply_done(&issue.repo, github::Done::Reopened(issue.number));
    assert_eq!(s.data.issues.len(), count);
    assert_eq!(s.data.issues[0].number, issue.number);
    assert!(s.reopening.is_empty());
}

#[test]
fn a_change_with_no_local_copy_says_so() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("repohub-noclone-{}", std::process::id()));
    // SAFETY: the lock keeps other environment-changing tests out.
    unsafe {
        std::env::set_var("REPOHUB_DATA_DIR", &dir);
        std::env::set_var("REPOHUB_REPOS_DIR", &dir);
    }
    let mut s = app();
    s.git_run(0, "Pushing", "Pushed", |_| Ok(String::new()));
    assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("no local copy")), "it doesn't return silently");
    assert!(s.git_job.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// A repository's "open issues and pull requests" count (GitHub's `open_issues_count`)
/// follows pull requests too, as it follows issues: opened, merged, closed, reopened.
#[test]
fn the_repositorys_open_count_follows_pull_requests_too() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let p = s.data.pulls.iter().find(|p| p.state == crate::model::PullState::Open).unwrap().clone();
    let count = |s: &App| s.data.repos.iter().find(|r| crate::model::same_repo(&r.full_name, &p.repo)).unwrap().issues;
    let before = count(&s);
    s.apply_done(&p.repo, github::Done::Pull(github::Created { number: 900, title: "New".into(), ..Default::default() }));
    assert_eq!(count(&s), before + 1, "a new pull request");
    s.apply_done(&p.repo, github::Done::Merged(900));
    assert_eq!(count(&s), before, "merged");
    s.apply_done(&p.repo, github::Done::PullOpen(p.number, false));
    assert_eq!(count(&s), before - 1, "closed");
    s.apply_done(&p.repo, github::Done::PullOpen(p.number, true));
    assert_eq!(count(&s), before, "reopened");
    s.apply_done(&p.repo, github::Done::Merged(900));
    assert_eq!(count(&s), before, "merging again changes nothing");
}

/// Closing a pull request changes GitHub as closing an issue does: its toast offers Undo,
/// which reopens it.
#[test]
fn a_closed_pull_request_offers_undo() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let p = s.data.pulls.iter().find(|p| p.state == crate::model::PullState::Open).unwrap().clone();
    s.apply_done(&p.repo, github::Done::PullOpen(p.number, false));
    let undo = s.toast.as_ref().and_then(|t| t.undo.clone());
    assert!(matches!(&undo, Some(state::Undo::ReopenPull { number, .. }) if *number == p.number), "{undo:?}");
    // Reopening it with that Undo isn't undone in turn (reopening it from its conversation is,
    // `confirm_tests`).
    s.undo();
    s.apply_done(&p.repo, github::Done::PullOpen(p.number, true));
    assert!(s.toast.as_ref().is_some_and(|t| t.undo.is_none() && t.text.starts_with("Reopened")));
}

/// GitHub's answer to a New issue form belongs to that form: a form opened since keeps
/// what's typed in it, and a failure after the form was closed is still said.
#[test]
fn a_forms_answer_reaches_that_form_or_a_toast() {
    use crate::state::{Compose, Submit};
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let (a, b) = (s.data.repos[0].full_name.clone(), s.data.repos[1].full_name.clone());
    let epoch = s.load_gen;
    let typed = |s: &mut App, i: usize, title: &str| {
        s.open_form(i, Compose::Issue);
        s.form.as_mut().unwrap().title = rune::TextEdit::new(title);
    };
    // Sent, then Cancel: GitHub refuses. Nothing on screen would say it otherwise.
    typed(&mut s, 0, "First");
    s.form.as_mut().unwrap().submit = Submit::Sending;
    s.form = None;
    s.toast = None;
    s.form_sent(epoch, &a, Err("GitHub refused: Issues are disabled".into()));
    assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("Issues are disabled")), "the failure is said");
    // Sent, then another form opened and typed in: the first's answer leaves it alone.
    typed(&mut s, 0, "Second");
    s.form.as_mut().unwrap().submit = Submit::Sending;
    typed(&mut s, 1, "Half-written");
    s.form_sent(epoch, &a, Err("GitHub refused".into()));
    let f = s.form.as_ref().expect("the open form stays");
    assert_eq!((f.repo.as_str(), f.title.text.as_str(), f.submit.is_sending() || matches!(f.submit, Submit::Failed(_))), (b.as_str(), "Half-written", false));
    s.form_sent(epoch, &a, Ok(github::Done::Issue(github::Created { number: 99, ..Default::default() })));
    assert_eq!(s.form.as_ref().map(|f| f.title.text.as_str()), Some("Half-written"), "still open after the other one's success");
    assert!(s.data.issues.iter().any(|x| x.number == 99), "and the new issue is listed");
    // Its own answer closes it.
    s.form.as_mut().unwrap().submit = Submit::Sending;
    s.form_sent(epoch, &b, Ok(github::Done::Issue(github::Created { number: 100, ..Default::default() })));
    assert!(s.form.is_none());
}
