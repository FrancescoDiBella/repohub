use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use super::app::explain;
use crate::github::{Done, Write as GhWrite};

type Log = Arc<Mutex<Vec<(String, String, String)>>>;

/// Answers each request with `answer(method, path, body)`, and keeps what it was asked.
struct Mock {
    url: String,
    log: Log,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Mock {
    fn new(answer: impl Fn(&str, &str, &str) -> (u16, String, Option<String>) + Send + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (log, stop): (Log, _) = (Arc::default(), Arc::new(AtomicBool::new(false)));
        let (log2, stop2) = (log.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // The head, then as much body as it announces.
                let head_end = loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(at + 4);
                    }
                };
                let Some(head_end) = head_end else { continue };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let (method, path) = (first.next().unwrap_or("").to_owned(), first.next().unwrap_or("").to_owned());
                let (status, reply, link) = answer(&method, &path, &body);
                log2.lock().unwrap().push((method, path, body));
                let link = link.map(|l| format!("Link: {l}\r\n")).unwrap_or_default();
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{link}Content-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
            }
        });
        Mock { url, log, stop, thread: Some(thread) }
    }

    fn cfg(&self) -> crate::config::Config {
        let mut cfg = crate::config::Config::from_json("{}").unwrap();
        cfg.api = self.url.clone();
        cfg.token = Some("test-token".into());
        cfg
    }

    fn asked(&self) -> Vec<(String, String, String)> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[test]
fn every_change_reaches_github_as_its_api_expects() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let server = Mock::new(|method, path, body| {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (method, path) {
            ("GET", p) if p.starts_with("/repos/o/r/labels") => (200, r#"[{"name":"bug","color":"d73a4a","description":"Broken"},{"name":"docs","color":"0075ca"}]"#.into(),
                Some(r#"</repos/o/r/labels?per_page=100&page=2>; rel="next""#.into())),
            ("GET", p) if p.starts_with("/repos/o/r/assignees") => (200, r#"[{"login":"alice"},{"login":"bob"}]"#.into(), None),
            ("GET", p) if p.starts_with("/repos/o/r/milestones") => (200, r#"[{"number":3,"title":"v1.0","due_on":null}]"#.into(), None),
            // GitHub answers a PATCH with the issue as it is now.
            ("PATCH", "/repos/o/r/issues/7") => {
                let labels: Vec<Value> = v["labels"].as_array().into_iter().flatten().map(|l| json!({ "name": l, "color": "d73a4a" })).collect();
                let assignees: Vec<Value> = v["assignees"].as_array().into_iter().flatten().map(|l| json!({ "login": l })).collect();
                let milestone = v["milestone"].as_u64().map(|n| json!({ "number": n, "title": "v1.0" })).unwrap_or(Value::Null);
                (200, json!({ "number": 7, "title": v["title"].as_str().unwrap_or("Old"), "body": v["body"].as_str().unwrap_or(""),
                    "labels": labels, "assignees": assignees, "milestone": milestone }).to_string(), None)
            }
            ("PATCH", "/repos/o/r/issues/9") => (403, r#"{"message":"Must have admin rights to Repository."}"#.into(), None),
            ("DELETE", "/repos/o/r/pulls/8/requested_reviewers") => (200, r#"{"number":8,"requested_reviewers":[{"login":"dave"}]}"#.into(), None),
            ("POST", "/repos/o/r/pulls/8/requested_reviewers") => (201, r#"{"number":8,"requested_reviewers":[{"login":"dave"},{"login":"carol"}]}"#.into(), None),
            ("POST", "/repos/o/r/issues") => (201, r#"{"number":50,"html_url":"https://github.com/o/r/issues/50"}"#.into(), None),
            _ => (404, r#"{"message":"Not Found"}"#.into(), None),
        }
    });
    let cfg = server.cfg();
    let run = |w: GhWrite| pollster::block_on(github::run(&cfg, "o/r", w));

    // The choices: the first 100, and whether GitHub has more.
    let labels = pollster::block_on(github::load_choices(&cfg, "o/r", Source::Labels)).unwrap();
    assert_eq!((labels.items.len(), labels.more, labels.items[0].note.as_str()), (2, true, "Broken"));
    let people = pollster::block_on(github::load_choices(&cfg, "o/r", Source::People)).unwrap();
    assert_eq!((people.items.len(), people.more), (2, false));
    let ms = pollster::block_on(github::load_choices(&cfg, "o/r", Source::Milestones)).unwrap();
    assert_eq!(ms.items[0].id, "3");

    // Each change, and what GitHub says it is now.
    let triaged = |d: Done| match d { Done::Triaged { applied, .. } => applied, other => panic!("{other:?}") };
    assert!(matches!(triaged(run(GhWrite::Triage { number: 7, change: Change::Labels(vec!["bug".into(), "docs".into()]) }).unwrap()), Applied::Labels(l) if l.len() == 2));
    assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Assignees(vec!["alice".into()]) }).unwrap()), Applied::Assignees(vec!["alice".into()]));
    assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Milestone(Some((3, "v1.0".into()))) }).unwrap()), Applied::Milestone(Some((3, "v1.0".into()))));
    assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Milestone(None) }).unwrap()), Applied::Milestone(None));
    assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Title("New title".into()) }).unwrap()), Applied::Title("New title".into()));
    assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Body("New body".into()) }).unwrap()), Applied::Body("New body".into()));
    assert_eq!(triaged(run(GhWrite::Triage { number: 8, change: Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] } }).unwrap()),
        Applied::Reviewers(vec!["dave".into(), "carol".into()]));
    // A new issue with its labels and assignees.
    assert!(matches!(run(GhWrite::Issue { title: "T".into(), body: String::new(), labels: vec!["bug".into()], assignees: vec!["alice".into()] }).unwrap(), Done::Issue(c) if c.number == 50));
    // A refusal says why, with GitHub's words.
    let err = run(GhWrite::Triage { number: 9, change: Change::Labels(vec!["bug".into()]) }).unwrap_err();
    assert!(err.contains("(403)") && err.contains("admin rights"), "{err}");
    assert!(explain(&err).ends_with("Nothing was changed."));

    let asked: Vec<(String, String, Value)> = server.asked().into_iter().map(|(m, p, b)| (m, p, serde_json::from_str(&b).unwrap_or(Value::Null))).collect();
    let writes: Vec<_> = asked.iter().filter(|(m, ..)| m != "GET").cloned().collect();
    assert_eq!(writes, [
        ("PATCH".to_owned(), "/repos/o/r/issues/7".to_owned(), json!({ "labels": ["bug", "docs"] })),
        ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "assignees": ["alice"] })),
        ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "milestone": 3 })),
        ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "milestone": null })),
        ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "title": "New title" })),
        ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "body": "New body" })),
        ("DELETE".into(), "/repos/o/r/pulls/8/requested_reviewers".into(), json!({ "reviewers": ["bob"] })),
        ("POST".into(), "/repos/o/r/pulls/8/requested_reviewers".into(), json!({ "reviewers": ["carol"] })),
        ("POST".into(), "/repos/o/r/issues".into(), json!({ "title": "T", "body": "", "labels": ["bug"], "assignees": ["alice"] })),
        ("PATCH".into(), "/repos/o/r/issues/9".into(), json!({ "labels": ["bug"] })),
    ]);
    assert!(asked.iter().any(|(m, p, _)| m == "GET" && p == "/repos/o/r/milestones?state=open&per_page=100"));
    // A new issue without labels or assignees doesn't send them.
    assert_eq!(github::issue_body("T", "B", &[], &[]), json!({ "title": "T", "body": "B" }));
}

#[test]
fn the_conversation_reads_assignees_milestone_and_reviewers_from_its_answers() {
    let issue = json!({ "title": "T", "state": "open", "created_at": "2026-11-01T10:00:00Z", "body": "", "user": { "login": "alice" },
        "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "bob" }, { "login": "carol" }],
        "milestone": { "number": 3, "title": "v1.0" }, "pull_request": { "url": "x" } });
    let detail = json!({ "draft": false, "merged": false, "head": { "ref": "f" }, "base": { "ref": "main" }, "requested_reviewers": [{ "login": "dave" }] });
    let reviews = json!([{ "user": { "login": "erin" }, "state": "APPROVED", "body": "", "submitted_at": "2026-11-02T10:00:00Z" }]);
    let t = github::parse_thread(&issue, &json!([]), Some((&detail, &reviews)));
    assert_eq!(t.assignees, ["bob", "carol"]);
    assert_eq!(t.milestone, Some((3, "v1.0".into())));
    assert_eq!(t.requested, ["dave"]);
    assert_eq!(t.reviewed, [("erin".to_owned(), Verdict::Approved)]);
    // An issue: no reviewers; no milestone is none.
    let issue = json!({ "title": "T", "state": "open", "created_at": "2026-11-01T10:00:00Z", "user": { "login": "alice" }, "milestone": null });
    let t = github::parse_thread(&issue, &json!([]), None);
    assert!(t.requested.is_empty() && t.reviewed.is_empty() && t.milestone.is_none() && t.assignees.is_empty());
}

#[test]
fn pull_requests_waiting_for_review_are_flagged_once() {
    use crate::model::{Pull, PullState};
    let pull = |n: u32, repo: &str| Pull { number: n, title: format!("#{n}"), repo: repo.into(), author: "x".into(), state: PullState::Open, checks: None,
        updated_at: 0, comments: 0, url: String::new(), labels: Vec::new(), assignees: Vec::new(), review_requested: false };
    let mut list = vec![pull(1, "o/a"), pull(2, "o/a")];
    let mut total = Some(2);
    let waiting = vec![Pull { review_requested: true, ..pull(2, "O/A") }, Pull { review_requested: true, ..pull(3, "o/b") }];
    github::merge_review_requested(&mut list, waiting, &mut total);
    assert_eq!(list.iter().map(|p| (p.number, p.review_requested)).collect::<Vec<_>>(), [(1, false), (2, true), (3, true)], "#2 isn't added twice");
    assert_eq!(total, Some(3), "the one added counts");
    assert!(github::parse_pull(&json!({ "assignees": [{ "login": "a" }] })).assignees == ["a"]);
}
