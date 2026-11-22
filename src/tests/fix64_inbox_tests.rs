//! Notifications: GitHub's list read, the page, and forgetting it on an account change.

use crate::inbox::{Notification, Subject};
use crate::state::Load;
use crate::{App, Page, github, inbox};

fn live() -> App {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data.sample = false;
    s.pending_place = None;
    let mut cfg = (*s.cfg).clone();
    cfg.demo = false;
    cfg.user = Some("alice".into());
    cfg.token = Some("tok".into());
    cfg.api = "http://127.0.0.1:9".into();
    s.cfg = std::sync::Arc::new(cfg);
    s.status = crate::Status::Ready;
    s.page = Page::Notifications;
    s
}

fn note(id: &str, repo: &str, title: &str, subject: Subject, number: Option<u32>) -> Notification {
    Notification { id: id.into(), repo: repo.into(), title: title.into(), subject, number, reason: "review_requested".into(), updated_at: 0, url: format!("https://github.com/{repo}") }
}

#[test]
fn githubs_notifications_are_read_into_what_the_page_needs() {
    let v = serde_json::json!({
        "id": "123", "unread": true, "reason": "review_requested", "updated_at": "2026-11-10T10:00:00Z",
        "repository": { "full_name": "octo/app" },
        "subject": { "title": "Speed up loading", "type": "PullRequest", "url": "https://api.github.com/repos/octo/app/pulls/7" },
    });
    let n = github::parse_notification(&v).unwrap();
    assert_eq!((n.id.as_str(), n.repo.as_str(), n.subject, n.number), ("123", "octo/app", Subject::Pull, Some(7)));
    assert_eq!(n.why(), "Your review was requested");
    assert_eq!(n.url, "https://github.com/octo/app/pull/7");
    // A release has no conversation in the app: it opens the repository on GitHub.
    let mut release = v.clone();
    release["subject"] = serde_json::json!({ "title": "v1.0", "type": "Release", "url": "https://api.github.com/repos/octo/app/releases/55" });
    let n = github::parse_notification(&release).unwrap();
    assert_eq!((n.subject, n.number, n.url.as_str()), (Subject::Other, None, "https://github.com/octo/app"));
    // Read ones, and ids that aren't numbers (they go into a request's address), are left out.
    let mut read = v.clone();
    read["unread"] = false.into();
    assert_eq!(github::parse_notification(&read), None);
    let mut odd = v.clone();
    odd["id"] = "12/../../user".into();
    assert_eq!(github::parse_notification(&odd), None);
    assert!(inbox::explain("GitHub refused (403): Resource not accessible by personal access token").contains("classic token"));
    assert_eq!(inbox::explain("Can't reach GitHub"), "Can't reach GitHub");
}

/// GitHub gives the 50 newest: with that many, the page and the phone's bell don't say "50
/// unread" as if that were all.
#[test]
fn fifty_notifications_are_said_to_be_the_newest_not_all() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    s.inbox = Some(Load::Ready((0..50).map(|i| note(&i.to_string(), "octo/app", "Review this", Subject::Pull, Some(7))).collect()));
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let texts: Vec<String> = rt.elements().iter().filter_map(|e| e.text.clone()).collect();
    assert!(texts.iter().any(|t| t.starts_with("The 50 newest unread")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("50 unread")));
    let mut s = rt.state;
    s.compact = true;
    let mut rt = rune::Runtime::new(s, crate::view, (390.0, 844.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let bell = rt.elements().iter().find(|e| e.key.as_deref() == Some("open-notifications")).and_then(|e| e.label.clone());
    assert_eq!(bell.as_deref(), Some("Notifications, 50 or more unread"));
}

#[test]
fn the_page_groups_by_repository_and_says_why_it_is_empty() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    s.inbox = Some(Load::Ready(vec![
        note("1", "octo/app", "Review this", Subject::Pull, Some(7)),
        note("2", "octo/site", "A bug", Subject::Issue, Some(3)),
        note("3", "octo/app", "v1.0", Subject::Other, None),
    ]));
    assert_eq!(s.unread(), 3);
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    for key in ["notif-1", "notif-2", "notif-3", "notif-read-1", "nav-Notifications"] {
        assert!(rt.find(key).is_some(), "{key}");
    }
    let texts = |rt: &rune::Runtime<App>| rt.elements().iter().filter_map(|e| e.text.clone()).collect::<Vec<_>>();
    let t = texts(&rt);
    assert_eq!(t.iter().filter(|x| x.as_str() == "octo/app").count(), 1, "one group per repository");
    assert!(t.iter().any(|x| x.starts_with("3 unread")));
    // Pressing a pull request's notification opens its conversation; the page stays to go back to.
    let (x, y) = rt.find("notif-1").unwrap();
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
    assert!(rt.state.thread.as_ref().is_some_and(|t| t.pull && t.number == 7 && t.repo == "octo/app"));
    assert_eq!(rt.state.page, Page::Notifications);

    rt.update(|s, _| { s.thread = None; s.inbox = Some(Load::Ready(Vec::new())); });
    rt.frame(0.2);
    assert!(rt.find("inbox-empty").is_some());
    rt.update(|s, _| s.inbox = Some(Load::Failed(inbox::explain("GitHub refused (403)"))));
    rt.frame(0.3);
    assert!(rt.find("inbox-failed").is_some() && rt.find("inbox-retry").is_some());
    assert!(texts(&rt).iter().any(|x| x.contains("classic token")));
    // Without a token there is nothing to ask GitHub, and the page says what's needed.
    rt.update(|s, _| { let mut c = (*s.cfg).clone(); c.token = None; s.cfg = std::sync::Arc::new(c); s.inbox = None; });
    rt.frame(0.4);
    assert!(rt.find("inbox-no-token").is_some());
}

#[test]
fn changing_account_forgets_the_notifications() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    s.inbox = Some(Load::Ready(vec![note("1", "octo/app", "x", Subject::Issue, Some(1))]));
    s.inbox_busy.insert("1".into());
    s.reset_account_state();
    assert!(s.inbox.is_none() && s.inbox_busy.is_empty() && s.unread() == 0);
}
