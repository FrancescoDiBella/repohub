//! Conversations: an issue's or pull request's thread, read, written to, and late answers dropped.

use crate::state::Load;
use crate::thread::{Standing, Verdict};
use crate::{App, Page, github};

fn app() -> App {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = Page::Pulls;
    s
}

fn press(rt: &mut rune::Runtime<App>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
}

#[test]
fn githubs_answers_become_one_conversation_in_order() {
    let issue = serde_json::json!({
        "title": "Speed up loading", "state": "open", "html_url": "https://github.com/o/r/pull/7", "comments": 2,
        "created_at": "2026-11-01T10:00:00Z", "body": "The description.", "user": { "login": "alice" },
        "labels": [{ "name": "perf", "color": "ff0000" }], "pull_request": { "url": "x" },
    });
    let comments = serde_json::json!([
        { "user": { "login": "bob" }, "created_at": "2026-11-03T10:00:00Z", "body": "Second." },
        { "user": null, "created_at": "2026-11-02T10:00:00Z", "body": "First." },
    ]);
    let detail = serde_json::json!({ "draft": true, "merged": false, "head": { "ref": "fast" }, "base": { "ref": "main" } });
    let reviews = serde_json::json!([
        { "user": { "login": "carol" }, "state": "APPROVED", "body": "", "submitted_at": "2026-11-04T10:00:00Z" },
        { "user": { "login": "dave" }, "state": "CHANGES_REQUESTED", "body": "Fix the flag.", "submitted_at": "2026-11-02T12:00:00Z" },
        { "user": { "login": "erin" }, "state": "COMMENTED", "body": "", "submitted_at": "2026-11-02T13:00:00Z" },
        { "user": { "login": "me" }, "state": "PENDING", "body": "not sent yet" },
        { "user": { "login": "frank" }, "state": "DISMISSED", "body": "old", "submitted_at": "2026-11-02T14:00:00Z" },
    ]);
    let t = github::parse_thread(&issue, &comments, Some((&detail, &reviews)));
    assert_eq!((t.title.as_str(), t.author.as_str(), t.standing), ("Speed up loading", "alice", Standing::Draft));
    assert_eq!(t.branches, Some(("fast".into(), "main".into())));
    assert_eq!(t.labels.len(), 1);
    // The description first, then everything by time; a deleted account is "ghost".
    let seen: Vec<(&str, &str, Option<Verdict>)> = t.entries.iter().map(|e| (e.author.as_str(), e.body.as_str(), e.verdict)).collect();
    assert_eq!(seen, [
        ("alice", "The description.", None),
        ("ghost", "First.", None),
        ("dave", "Fix the flag.", Some(Verdict::ChangesRequested)),
        ("bob", "Second.", None),
        ("carol", "", Some(Verdict::Approved)),
    ], "an empty comment-only review, a pending one and a dismissed one are left out");
    assert!(!t.more);

    // An issue: no pull request part; closed stays closed; more comments than were read.
    let issue = serde_json::json!({ "title": "Bug", "state": "closed", "comments": 150, "created_at": "2026-11-01T10:00:00Z", "body": null, "user": { "login": "alice" } });
    let t = github::parse_thread(&issue, &serde_json::json!([]), None);
    assert_eq!((t.standing, t.branches.clone(), t.entries.len(), t.more), (Standing::Closed, None, 1, true));
    assert_eq!(t.entries[0].body, "");
    // A merged pull request is merged, not just closed.
    let merged = serde_json::json!({ "merged": true, "draft": false });
    assert_eq!(github::parse_thread(&issue, &serde_json::json!([]), Some((&merged, &serde_json::json!([])))).standing, Standing::Merged);
}

#[test]
fn a_row_opens_its_conversation_and_back_returns_to_the_list() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = app();
    let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    press(&mut rt, &format!("pr-{}-{}", pr.repo, pr.number));
    let open = rt.state.thread.as_ref().expect("the conversation opened");
    assert!(open.pull && open.number == 14);
    assert!(matches!(&open.load, Load::Ready(t) if t.title == pr.title), "the sample data shows a conversation without GitHub");
    assert!(rt.find("thread-entry-0").is_some() && rt.find("thread-reply-text").is_some());
    assert!(rt.find("pr-filter").is_none(), "it takes the list's place");
    // A pull request can be approved; with the sample data nothing can be sent, and it says so.
    assert!(rt.find("thread-approve").is_some() && rt.find("thread-blocked").is_some());
    press(&mut rt, "thread-back");
    assert!(rt.state.thread.is_none());
    assert!(rt.find("pr-filter").is_some() && rt.state.page == Page::Pulls);

    // An issue has no review buttons, and going to another page closes the conversation.
    rt.update(|s, _| s.go(Page::Issues));
    rt.frame(0.3);
    let issue = rt.state.data.issues[0].clone();
    press(&mut rt, &format!("issue-{}-{}", issue.repo, issue.number));
    assert!(rt.state.thread.as_ref().is_some_and(|t| !t.pull && t.number == issue.number));
    assert!(rt.find("thread-approve").is_none() && rt.find("thread-close-issue").is_some());
    rt.update(|s, _| s.go(Page::Issues));
    assert!(rt.state.thread.is_none(), "choosing the page again goes back to its list");
}

#[test]
fn a_pull_request_shows_its_checks_and_can_be_closed_and_reopened() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    let open_before = s.data.open_pulls_total;
    s.open_thread(&pr.repo, 14, true);
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("thread-checks").is_some() && rt.find("thread-check-2").is_some(), "one row per check");
    assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("1 running, 2 passed")));
    assert!(rt.find("thread-close-pull").is_some() && rt.find("thread-reopen-pull").is_none());

    // Closing it without merging: the list and its counts follow, and the button turns to Reopen.
    let mut s = rt.state;
    s.apply_done(&pr.repo, github::Done::PullOpen(14, false));
    let p = s.data.pulls.iter().find(|p| p.number == 14).unwrap();
    assert_eq!(p.state, crate::model::PullState::Closed);
    if let Some(n) = open_before {
        assert_eq!(s.data.open_pulls_total, Some(n.saturating_sub(1)));
    }
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("thread-reopen-pull").is_some(), "a closed pull request can be reopened");
    let mut s = rt.state;
    s.apply_done(&pr.repo, github::Done::PullOpen(14, true));
    assert_eq!(s.data.pulls.iter().find(|p| p.number == 14).unwrap().state, crate::model::PullState::Open);
    assert_eq!(s.data.open_pulls_total, open_before);
    // An issue has no checks.
    s.open_thread(&s.data.issues[0].repo.clone(), s.data.issues[0].number, false);
    let rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
    assert!(rt.find("thread-checks").is_none() && rt.find("thread-close-pull").is_none());
}

#[test]
fn what_is_sent_and_what_a_finished_write_changes() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    s.open_thread(&pr.repo, 14, true);
    // Nothing typed: a comment and "request changes" need words; nothing is marked as sending.
    s.send_reply(None);
    s.send_reply(Some(Verdict::ChangesRequested));
    assert!(!s.thread.as_ref().unwrap().sending);
    assert_eq!(github::Write::Review { number: 14, verdict: Verdict::Approved, body: String::new() }.number(), Some(14));
    assert_eq!((Verdict::Approved.event(), Verdict::ChangesRequested.event(), Verdict::Commented.event()), ("APPROVE", "REQUEST_CHANGES", "COMMENT"));

    // A comment that was sent: the box empties, the row's count goes up, a toast says so.
    s.thread.as_mut().unwrap().reply = rune::TextEdit::new("Thanks!");
    let before = pr.comments;
    s.apply_done(&pr.repo, github::Done::Commented(14));
    assert!(s.thread.as_ref().unwrap().reply.text.is_empty());
    assert_eq!(s.data.pulls.iter().find(|p| p.number == 14).unwrap().comments, before + 1);
    assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("Comment added to #14"));
    // A write about another pull request leaves what's being typed here alone.
    s.thread.as_mut().unwrap().reply = rune::TextEdit::new("still typing");
    s.apply_done(&pr.repo, github::Done::Commented(15));
    assert_eq!(s.thread.as_ref().unwrap().reply.text, "still typing");
    s.apply_done(&pr.repo, github::Done::Reviewed(14, Verdict::Approved));
    assert_eq!(s.toast.as_ref().map(|t| t.text.as_str()), Some("Approved pull request #14"));
}

#[test]
fn a_late_answer_for_another_conversation_is_dropped() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let pr = s.data.pulls.iter().find(|p| p.number == 14).unwrap().clone();
    s.open_thread(&pr.repo, 14, true);
    let first = s.thread.as_ref().unwrap().epoch;
    s.thread = None;
    s.open_thread(&pr.repo, 14, true);
    assert_ne!(s.thread.as_ref().unwrap().epoch, first, "opening again is a new load: the old answer isn't for it");
    // Changing section or account closes the conversation.
    s.leave_tab();
    assert!(s.thread.is_none());
}
