//! What the app asks GitHub for, against `server/allowlist.txt`: the list of requests that
//! repohub-server's proxy (`allowed()`, its test `allowlist_cases`) and the Android app's
//! `GitHubPaths` (`BackendTest`) must both allow, and the ones both must refuse. Every request
//! the app makes is sent to a recording stand-in for GitHub here, and each must be one of the
//! file's `allow` lines: a new or changed request that isn't added there (and so tested on
//! both allowlists) fails this test.
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::config::Config;
use crate::github::{self, Gh, NewLineComment, NewPull, NewRelease, Write};
use crate::sync::Remote;
use crate::sync_io::GhRemote;
use crate::thread::Verdict;
use crate::triage::{Change, Source};

/// The shared list: `allow` and `deny` lines, `METHOD /path` each.
pub(crate) const CASES: &str = include_str!("../server/allowlist.txt");

pub(crate) fn cases(kind: &str) -> BTreeSet<(String, String)> {
    CASES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| {
        let mut parts = l.split_whitespace();
        let (k, m, p) = (parts.next()?, parts.next()?, parts.next()?);
        (k == kind).then(|| (m.to_owned(), p.split('?').next().unwrap_or(p).to_owned()))
    }).collect()
}

/// `METHOD` and `/path?query` of each request heard.
type Heard = Arc<Mutex<Vec<(String, String)>>>;

/// A stand-in for GitHub on localhost that answers enough for each call to go on to its next
/// request, and records `METHOD /path` of each.
fn recorder() -> (String, Heard) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, target) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let _ = reader.read_exact(&mut vec![0u8; len]);
            let path = target.split('?').next().unwrap_or("").to_owned();
            log.lock().unwrap().push((method.clone(), target.clone()));
            let repo = json!({ "full_name": "octo/app", "name": "app", "owner": { "login": "octo" }, "default_branch": "main", "pushed_at": "2026-11-01T00:00:00Z", "permissions": { "push": true }, "private": true });
            let body = match (method.as_str(), path.as_str()) {
                ("GET", "/user") | ("GET", "/users/octo") => json!({ "login": "octo", "name": "Octo" }),
                ("GET", "/user/repos") | ("GET", "/users/octo/repos") => json!([repo]),
                ("GET", "/users/octo/orgs") => json!([{ "login": "org" }]),
                ("GET", "/repos/octo/notes") => json!({ "full_name": "octo/notes", "private": true, "default_branch": "main", "permissions": { "push": true } }),
                ("GET", "/repos/octo/notes/git/trees/main") => json!({ "tree": [], "truncated": false }),
                ("GET", "/repos/octo/app/git/ref/heads/main") => json!({ "object": { "sha": "0123abcd0123abcd0123abcd0123abcd0123abcd" } }),
                ("GET", "/repos/octo/app/issues/14") => json!({ "number": 14, "title": "x", "pull_request": {} }),
                ("GET", "/repos/octo/app/pulls/14") => json!({ "number": 14, "head": { "sha": "0123abcd0123abcd0123abcd0123abcd0123abcd" } }),
                ("GET", "/repos/octo/app/compare/main...feature/x") => json!({ "status": "ahead", "ahead_by": 1, "behind_by": 0, "commits": [], "files": [] }),
                ("GET", p) if p.starts_with("/search/") => json!({ "total_count": 0, "items": [] }),
                ("GET", _) => json!([]),
                _ => json!({}),
            }.to_string();
            let mut out = stream;
            let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    (url, seen)
}

/// Every kind of write, once. A new kind doesn't compile here until it's added (and to the
/// allowlists).
fn every_write() -> Vec<Write> {
    let release = || NewRelease { tag: "v1.0.0".into(), target: "main".into(), name: "One".into(), body: String::new(), draft: true, prerelease: false };
    let all = vec![
        Write::Issue { title: "t".into(), body: String::new(), labels: vec!["bug".into()], assignees: vec![] },
        Write::Branch { name: "feature/new".into(), from: "main".into() },
        Write::Pull(NewPull { title: "t".into(), body: String::new(), head: "feature/x".into(), base: "main".into(), draft: false }),
        Write::CloseIssue(14),
        Write::ReopenIssue(14),
        Write::SetPullOpen { number: 14, open: false },
        Write::Merge { number: 14, method: "squash" },
        Write::Comment { number: 14, body: "b".into() },
        Write::Review { number: 14, verdict: Verdict::Approved, body: String::new() },
        Write::LineComment { number: 14, comment: NewLineComment { body: "b".into(), commit: "0123abcd".into(), path: "src/a.rs".into(), line: 3, side: "RIGHT", start: None } },
        Write::RerunRun { run_id: 9, failed_only: false },
        Write::RerunRun { run_id: 9, failed_only: true },
        Write::Release(release()),
        Write::EditRelease { id: 200, release: release() },
        Write::PublishRelease { id: 200 },
        Write::Triage { number: 14, change: Change::Labels(vec!["bug".into()]) },
        Write::Triage { number: 14, change: Change::Reviewers { add: vec!["ada".into()], remove: vec!["bob".into()] } },
    ];
    for w in &all {
        match w {
            Write::Issue { .. } | Write::Branch { .. } | Write::Pull(_) | Write::CloseIssue(_) | Write::ReopenIssue(_) | Write::SetPullOpen { .. }
            | Write::Merge { .. } | Write::Comment { .. } | Write::Review { .. } | Write::LineComment { .. } | Write::RerunRun { .. }
            | Write::Release(_) | Write::EditRelease { .. } | Write::PublishRelease { .. } | Write::Triage { .. } => {}
        }
    }
    all
}

/// Every request the app makes (the lists, a repository's page, a conversation, its diff and
/// checks, triage, commits, releases, Actions, notifications, stars, every write, board sync)
/// is an `allow` line of `server/allowlist.txt`, and none is a `deny` line.
#[test]
fn every_request_the_app_makes_is_on_the_shared_allowlist() {
    let (url, seen) = recorder();
    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = url;
    cfg.token = Some("test".into());
    let gh = Gh::new(&cfg);
    let full = "octo/app";
    pollster::block_on(async {
        let _ = github::load_all(&cfg).await;
        let mut by_user = cfg.clone();
        by_user.user = Some("octo".into());
        let _ = github::load_all(&by_user).await;
        // A user name and no token: public repositories and organizations.
        by_user.token = None;
        let _ = github::load_all(&by_user).await;
        let _ = github::load_more_pulls(&cfg, "octo", true, 2).await;
        let _ = github::load_more_issues(&cfg, "octo", &["org".into()], 2).await;
        let _ = github::load_detail(&cfg, full, "main").await;
        let _ = github::branches(&cfg, full, "main").await;
        let _ = github::load_thread(&cfg, full, 14).await;
        let _ = github::load_pull_files(&cfg, full, 14).await;
        let _ = github::load_line_comments(&cfg, full, 14).await;
        for source in [Source::Labels, Source::People, Source::Milestones] {
            let _ = github::load_choices(&cfg, full, source).await;
        }
        let _ = gh.review_requested("octo").await;
        let _ = github::load_commits(&cfg, full, "feature/x", 1).await;
        let _ = github::load_commit(&cfg, full, "0123abcd").await;
        let _ = github::load_compare(&cfg, full, "main", "feature/x").await;
        // A branch name with `%`, which the proxies take as `%25`.
        let _ = github::load_compare(&cfg, full, "main", "50%-off").await;
        let _ = github::run(&cfg, full, Write::Branch { name: "x".into(), from: "50%-off".into() }).await;
        let _ = github::load_release_listing(&cfg, full).await;
        let _ = github::load_releases(&cfg, full, 2).await;
        let _ = github::generate_release_notes(&cfg, full, "v1.0.0", "main", Some("v0.9.0")).await;
        let _ = github::load_run_jobs(&cfg, full, 9).await;
        let _ = github::load_job_log(&cfg, full, 7).await;
        let _ = github::load_notifications(&cfg).await;
        let _ = github::mark_read(&cfg, "123").await;
        let _ = github::set_star(&cfg, full, true).await;
        let _ = github::set_star(&cfg, full, false).await;
        for w in every_write() {
            let _ = github::run(&cfg, full, w).await;
        }
        // Board sync, with a board whose names need encoding.
        let _ = crate::sync_io::check(&cfg, "octo/notes").await;
        let remote = GhRemote::new(&cfg, "octo/notes");
        let path = crate::sync::Key::new("octo/app", "feature/x y", "Plan (v2)").path();
        let _ = remote.list().await;
        let _ = remote.read(&path).await;
        let _ = remote.write(&path, "{}", None, "Update").await;
        let _ = remote.delete(&path, "0123", "Delete").await;
    });
    let allow = cases("allow");
    let deny = cases("deny");
    let asked: BTreeSet<(String, String)> = seen.lock().unwrap().iter().map(|(m, t)| (m.clone(), t.split('?').next().unwrap_or(t).to_owned())).collect();
    assert!(asked.len() > 40, "the stand-in heard the app: {asked:?}");
    let missing: Vec<String> = asked.iter().filter(|r| !allow.contains(*r)).map(|(m, p)| format!("allow {m} {p}")).collect();
    assert!(missing.is_empty(), "requests the app makes that server/allowlist.txt doesn't list (add them there, and to both allowlists if they fail):\n{}", missing.join("\n"));
    let refused: Vec<&(String, String)> = asked.iter().filter(|r| deny.contains(*r)).collect();
    assert!(refused.is_empty(), "requests the app makes that the list says must be refused: {refused:?}");
}

#[test]
fn the_shared_list_reads() {
    let (allow, deny) = (cases("allow"), cases("deny"));
    assert!(allow.len() > 40 && deny.len() > 20);
    assert!(allow.is_disjoint(&deny), "a request is both allowed and refused");
    for l in CASES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let parts: Vec<&str> = l.split_whitespace().collect();
        assert!(parts.len() == 3 && matches!(parts[0], "allow" | "deny") && matches!(parts[1], "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD") && parts[2].starts_with('/'), "{l}");
    }
}
