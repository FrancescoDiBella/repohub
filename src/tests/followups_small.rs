//! #138's small defects, each as it showed: a late answer for a diff's line comments, **Load
//! more** across a refresh, two forms sent at once, a star in the sample data, the checks and
//! line comments that GitHub has more of, and the headless stills (`-- tab notes`, `-- thread`
//! with an account).

use std::sync::Arc;

use serde_json::json;

use crate::github::{self, Created, Done, LineComments};
use crate::model::{Data, same_repo};
use crate::state::{Compose, Load, RepoTab, Submit};
use crate::{App, Page, Status};

fn live() -> App {
    let mut s = crate::init();
    s.pending_place = None;
    let mut cfg = crate::config::Config::from_json("{}").unwrap();
    cfg.api = "http://127.0.0.1:9".into();
    cfg.token = Some("test-token".into());
    s.cfg = Arc::new(cfg);
    s.data = Data::sample_at(1_790_000_000);
    s.data.sample = false;
    s.data.profile.login = "octocat".into();
    s.data_changed();
    s.status = Status::Ready;
    s
}

fn sample() -> App {
    let mut s = crate::init();
    s.data = Data::sample();
    s.data_changed();
    s.pending_place = None;
    s
}

fn texts(s: App, w: f32) -> (App, Vec<String>, Vec<String>) {
    let mut rt = rune::Runtime::new(s, crate::view, (w, 2400.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    rt.frame(0.0);
    rt.frame(0.5);
    let els = rt.elements();
    let texts = els.iter().filter_map(|e| e.text.clone()).collect();
    let keys = els.iter().filter_map(|e| e.key.clone()).collect();
    (rt.state, texts, keys)
}

fn comment(body: &str) -> crate::review::LineComment {
    crate::review::LineComment { path: "src/a.rs".into(), line: 2, side: rune_kit::diff::Side::New, author: "a".into(), body: body.into(), at: 0 }
}

/// Two reads of the same diff's line comments: the older answer arriving last is dropped.
#[test]
fn an_older_answer_for_the_same_diffs_line_comments_is_dropped() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    let p = s.data.pulls[0].clone();
    let full = s.data.repos.iter().find(|r| same_repo(&r.full_name, &p.repo)).unwrap().full_name.clone();
    s.open_pull_diff(full.clone(), p.number);
    let first = s.pull_review.as_ref().unwrap().asked;
    // A comment was sent: the comments are read again.
    s.load_line_comments();
    let second = s.pull_review.as_ref().unwrap().asked;
    assert_ne!(first, second, "each read has its own number");
    let key = (full, p.number);
    let read = |body: &str| Ok(LineComments { head: "abc".into(), comments: vec![comment(body)], more: false });
    s.apply_line_comments(s.load_gen, key.clone(), second, read("newer"));
    s.apply_line_comments(s.load_gen, key, first, read("older"));
    assert_eq!(s.pull_review.as_ref().unwrap().comments.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(), ["newer"], "the late, older answer is dropped");
}

/// **Load more** asked before a refresh landed: its page belongs to the list before the
/// refresh, and isn't added to the one the refresh brought.
#[test]
fn load_more_across_a_refresh_adds_nothing_to_the_new_list() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    let ask = crate::app::More { epoch: s.load_gen, landed: s.landed, page: 2 };
    s.loading_more_pulls = true;
    s.loading_more_issues = true;
    s.refresh();
    let epoch = s.load_gen;
    let fresh = s.data.clone();
    s.apply(epoch, Ok(fresh));
    let (pulls, issues) = (s.data.pulls.len(), s.data.issues.len());
    let mut page2 = s.data.pulls[0].clone();
    page2.number = 9_999;
    let mut issue2 = s.data.issues[0].clone();
    issue2.number = 9_998;
    // The user asked for the next page of the new list meanwhile.
    s.loading_more_pulls = true;
    s.more_pulls_arrived(ask, true, Ok((vec![page2.clone()], 500)));
    s.more_issues_arrived(ask, Ok((vec![issue2.clone()], 500)));
    assert_eq!((s.data.pulls.len(), s.data.issues.len(), s.open_pulls_page, s.issues_page), (pulls, issues, 1, 1), "nothing added, still the first page");
    assert!(s.loading_more_pulls, "the request for the new list still runs");
    // An answer for the list on screen is added.
    let now = crate::app::More { epoch: s.load_gen, landed: s.landed, page: 2 };
    s.more_pulls_arrived(now, true, Ok((vec![page2], 500)));
    s.more_issues_arrived(now, Ok((vec![issue2], 500)));
    assert_eq!((s.data.pulls.len(), s.data.issues.len(), s.open_pulls_page, s.issues_page), (pulls + 1, issues + 1, 2, 2));
    assert!(!s.loading_more_pulls);
}

/// Two forms sent one after the other, answered in the other order: each new row has its own
/// title (and a pull request its own draft flag).
#[test]
fn two_forms_in_flight_each_get_their_own_title() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = live();
    let repo = s.data.repos[0].full_name.clone();
    s.open(0);
    let epoch = s.load_gen;
    s.open_form(0, Compose::Issue);
    s.form.as_mut().unwrap().title = rune::TextEdit::new("First");
    s.form.as_mut().unwrap().submit = Submit::Sending;
    // A form being sent isn't at risk: another opens without asking.
    s.open_form(0, Compose::Pull);
    assert!(s.discard.is_none());
    s.form.as_mut().unwrap().title = rune::TextEdit::new("Second");
    s.form.as_mut().unwrap().draft = true;
    s.form.as_mut().unwrap().submit = Submit::Sending;
    s.form_sent(epoch, &repo, Ok(Done::Pull(Created { number: 2, title: "Second".into(), draft: true, ..Default::default() })));
    s.form_sent(epoch, &repo, Ok(Done::Issue(Created { number: 1, title: "First".into(), ..Default::default() })));
    let issue = s.data.issues.iter().find(|x| x.number == 1 && x.repo == repo).unwrap();
    let pull = s.data.pulls.iter().find(|x| x.number == 2 && x.repo == repo).unwrap();
    assert_eq!((issue.title.as_str(), pull.title.as_str(), pull.state), ("First", "Second", crate::model::PullState::Draft));
}

/// A star in the sample data changes on screen only, and the toast says why nothing was sent.
#[test]
fn starring_the_sample_data_says_nothing_was_sent() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = sample();
    let was = (s.data.repos[0].starred, s.data.repos[0].stars);
    s.toggle_star(0);
    assert_eq!(s.data.repos[0].starred, !was.0);
    let toast = s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default();
    assert!(toast.contains("nothing was sent to GitHub") && toast.contains("sample data"), "{toast}");
}

/// GitHub has more checks, or more line comments, than were read: the page says so and links
/// to the rest.
#[test]
fn checks_and_line_comments_say_when_github_has_more() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = sample();
    let p = s.data.pulls.iter().find(|p| p.number == 14).cloned().unwrap();
    s.page = Page::Pulls;
    s.open_thread(&p.repo, 14, true);
    if let Some(Load::Ready(t)) = s.thread.as_mut().map(|o| &mut o.load) {
        t.url = "https://github.com/o/r/pull/14".into();
        t.checks_total = 130;
    }
    let (mut s, texts_on, keys) = texts(s, 1180.0);
    assert!(texts_on.iter().any(|t| t.starts_with("Showing the first 3 of 130 checks")), "{texts_on:?}");
    assert!(keys.iter().any(|k| k == "thread-checks-all"));
    // All of them read: nothing more to say.
    if let Some(Load::Ready(t)) = s.thread.as_mut().map(|o| &mut o.load) {
        t.checks_total = 3;
    }
    let (mut s, _, keys) = texts(s, 1180.0);
    assert!(!keys.iter().any(|k| k == "thread-checks-more"));
    // The diff's line comments.
    s.open_pull_diff(crate::changes_page::pull_repo(&p.url).unwrap(), 14);
    if let Some(r) = s.pull_review.as_mut() {
        r.comments = vec![comment("x"); 100];
        r.more = true;
    }
    let (_, texts_on, keys) = texts(s, 1180.0);
    assert!(texts_on.iter().any(|t| t.starts_with("Showing the first 100 comments on lines")), "{texts_on:?}");
    assert!(keys.iter().any(|k| k == "pull-comments-all"));
}

/// `-- tab notes` on the sample data: the branch picker shows the branches, not "Loading
/// branches…" (they were never asked for).
#[test]
fn the_notes_tab_from_the_command_line_has_its_branches() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("still-notes");
    let mut s = sample();
    // What `init` does for `-- page repo -- tab notes`.
    s.page = Page::Repo(s.data.repos[0].full_name.clone());
    s.load_detail(0);
    s.tab = RepoTab::Notes;
    let (_, texts_on, _) = texts(s, 1180.0);
    assert!(!texts_on.iter().any(|t| t == "Loading branches…"), "{texts_on:?}");
}

/// `-- thread 14` with an account (`REPOHUB_BLOCKING=1`): the conversation is read before the
/// first frame, not left saying the app isn't ready.
#[test]
fn a_conversation_from_the_command_line_is_read_with_an_account() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::etag::clear();
    let mock = crate::load_tests::Mock::start(|path: &str| {
        let p = path.split('?').next().unwrap_or("");
        match p {
            "/repos/octo/app/issues/14" => (200, json!({ "number": 14, "title": "Read before the first frame", "state": "open", "user": { "login": "octo" }, "body": "Hi", "comments": 0 })),
            "/repos/octo/app/issues/14/comments" => (200, json!([])),
            _ => (404, json!({ "message": "Not Found" })),
        }
    });
    let mut s = live();
    let mut cfg = (*s.cfg).clone();
    cfg.api = mock.url.clone();
    s.cfg = Arc::new(cfg);
    s.data.repos[0].full_name = "octo/app".into();
    s.open_thread("octo/app", 14, false);
    assert!(matches!(s.thread.as_ref().map(|t| &t.load), Some(Load::Failed(_))), "no handle yet");
    s.read_opened_now();
    match s.thread.as_ref().map(|t| &t.load) {
        Some(Load::Ready(t)) => assert_eq!(t.title, "Read before the first frame"),
        other => panic!("{other:?}"),
    }
    crate::etag::clear();
}

#[test]
fn github_answers_are_read_for_the_new_row() {
    let c = github::parse_created(&json!({ "number": 7, "html_url": "u", "title": "As GitHub has it", "draft": true, "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "ada" }] }), "Sent", false);
    assert_eq!((c.number, c.title.as_str(), c.draft), (7, "As GitHub has it", true));
    assert_eq!(c.labels.as_ref().map(|l| l.iter().map(|x| x.0.as_str()).collect::<Vec<_>>()), Some(vec!["bug"]));
    assert_eq!(c.assignees, Some(vec!["ada".to_owned()]));
    let c = github::parse_created(&json!({ "number": 8 }), "Sent", true);
    assert_eq!((c.title.as_str(), c.draft, c.labels.is_none()), ("Sent", true, true), "what was sent where GitHub doesn't say");
}
