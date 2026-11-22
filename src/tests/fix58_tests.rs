//! Loading from a mock GitHub: pages, caps, partial failures, rate limits, and loading more.

use crate::*;
use crate::config::Config;
use crate::model::{Data, Detail};
use crate::state::Load;
use crate::net::Resp;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct MockServer {
    port: u16,
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    fn new(handler: impl Fn(&str, &str) -> (u16, Vec<(&'static str, &'static str)>, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        let handler = Arc::new(handler);
        listener.set_nonblocking(true).unwrap();

        let handle = std::thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 4096];
                        let mut n = 0;
                        while n < buf.len() {
                            match stream.read(&mut buf[n..]) {
                                Ok(0) => break,
                                Ok(bytes) => {
                                    n += bytes;
                                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(std::time::Duration::from_millis(5));
                                }
                                Err(_) => break,
                            }
                        }
                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        let mut lines = req_str.lines();
                        let req_line = lines.next().unwrap_or("");
                        let mut parts = req_line.split_whitespace();
                        let method = parts.next().unwrap_or("GET");
                        let path = parts.next().unwrap_or("/");

                        let (status, headers, body) = handler(method, path);
                        let reason = match status {
                            200 => "OK",
                            403 => "Forbidden",
                            404 => "Not Found",
                            429 => "Too Many Requests",
                            500 => "Internal Server Error",
                            _ => "Status",
                        };
                        let mut resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                        for (k, v) in headers {
                            resp.push_str(&format!("{k}: {v}\r\n"));
                        }
                        resp.push_str("\r\n");
                        resp.push_str(&body);
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        MockServer { port, running, handle: Some(handle) }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn detail_with_error_not_kept_after_refresh_and_readme_404_vs_500() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // 1. Mock HTTP: verify README 404 vs 500 and contents failure
    let server = MockServer::new(|_method, path| {
        if path.contains("/contents") {
            (500, vec![("Content-Type", "application/json")], r#"{"message":"Server error loading files"}"#.into())
        } else if path.contains("/readme") {
            if path.contains("404-repo") {
                (404, vec![("Content-Type", "application/json")], r#"{"message":"Not Found"}"#.into())
            } else {
                (500, vec![("Content-Type", "application/json")], r#"{"message":"Internal Server Error"}"#.into())
            }
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("mock_tok".into());
    let gh = crate::github::Gh::new(&cfg);

    // README 404 returns Ok(None) -> detail has readme: None and readme_error: None
    let d_404 = pollster::block_on(gh.detail("user/404-repo", "main"));
    assert!(d_404.readme.is_none());
    assert!(d_404.readme_error.is_none());

    // README 500 returns Err -> detail has readme_error: Some(...)
    let d_500 = pollster::block_on(gh.detail("user/500-repo", "main"));
    assert!(d_500.readme.is_none());
    assert!(d_500.readme_error.is_some());

    // Contents 500 returns Err -> detail has error: Some(...)
    assert!(d_404.error.is_some());
    assert!(d_500.error.is_some());

    // 2. apply_detail: contents failed and nothing else loaded -> stores Load::Failed
    let mut app = crate::init();
    let key_failed = ("user/failed-repo".into(), "main".into());
    let d_all_failed = Detail {
        error: Some("Failed to load contents".into()),
        ..Detail::default()
    };
    app.apply_detail(app.load_gen, key_failed.clone(), d_all_failed);
    assert!(matches!(app.details.get(&key_failed), Some(Load::Failed(_))));

    // When partial data is loaded, stores Load::Ready with error visible
    let key_partial = ("user/partial-repo".into(), "main".into());
    let d_partial = Detail {
        error: Some("Partial error".into()),
        files: vec![("file.txt".into(), false)],
        ..Detail::default()
    };
    app.apply_detail(app.load_gen, key_partial.clone(), d_partial);
    assert!(matches!(app.details.get(&key_partial), Some(Load::Ready(_))));

    // 3. Cache persistence across refresh:
    // - Error-free detail for an unpushed repo IS kept
    // - Detail with an error (d.error or d.readme_error) is NOT kept
    // - Load::Failed is NOT kept
    let mut s = crate::init();
    let mut s_cfg = (*s.cfg).clone();
    s_cfg.token = Some("tok".into());
    s.cfg = std::sync::Arc::new(s_cfg);
    s.data = model::Data::sample_at(1_700_000_000);
    s.data.sample = false;
    let unpushed = s.data.repos[0].full_name.clone();
    let errored = s.data.repos[1].full_name.clone();
    let readme_errored = s.data.repos[2].full_name.clone();

    s.details.insert((unpushed.clone(), "main".into()), Load::Ready(Detail::default()));
    s.details.insert((errored.clone(), "main".into()), Load::Ready(Detail {
        error: Some("Failed".into()),
        ..Detail::default()
    }));
    s.details.insert((readme_errored.clone(), "main".into()), Load::Ready(Detail {
        readme_error: Some("Failed README".into()),
        ..Detail::default()
    }));
    s.details.insert(("user/load-failed".into(), "main".into()), Load::Failed("network error".into()));

    let mut next = model::Data::sample_at(1_700_000_000);
    next.sample = false;
    let epoch = s.load_gen;
    s.apply(epoch, Ok(next));

    assert!(s.details.contains_key(&(unpushed, "main".into())), "clean detail on unpushed repo is kept");
    assert!(!s.details.contains_key(&(errored, "main".into())), "detail with error is not kept");
    assert!(!s.details.contains_key(&(readme_errored, "main".into())), "detail with readme_error is not kept");
    assert!(!s.details.contains_key(&("user/load-failed".into(), "main".into())), "Load::Failed is not kept");

    // 4. load_detail retries Load::Failed and errored entries, skipping only Loading and clean Ready
    s.details.insert(("user/clean".into(), "main".into()), Load::Ready(Detail::default()));
    s.details.insert(("user/err".into(), "main".into()), Load::Ready(Detail { error: Some("err".into()), ..Detail::default() }));
    s.details.insert(("user/readme-err".into(), "main".into()), Load::Ready(Detail { readme_error: Some("err".into()), ..Detail::default() }));
    s.details.insert(("user/failed".into(), "main".into()), Load::Failed("err".into()));
    s.details.insert(("user/loading".into(), "main".into()), Load::Loading);

    // repo index 0 in s.data has full_name matching repo 0
    let r0 = s.data.repos[0].full_name.clone();
    let key0 = (r0.clone(), s.branch(0));

    // When clean: load_detail skips without resetting to Loading
    s.details.insert(key0.clone(), Load::Ready(Detail::default()));
    s.load_detail(0);
    assert!(matches!(s.details.get(&key0), Some(Load::Ready(_))));

    // When errored: load_detail retries, moving entry to Loading
    s.details.insert(key0.clone(), Load::Ready(Detail { error: Some("failed".into()), ..Detail::default() }));
    s.load_detail(0);
    assert!(matches!(s.details.get(&key0), Some(Load::Loading)));

    // When Load::Failed: load_detail retries, moving entry to Loading
    s.details.insert(key0.clone(), Load::Failed("failed".into()));
    s.load_detail(0);
    assert!(matches!(s.details.get(&key0), Some(Load::Loading)));

    // When Load::Failed, repo page renders retry buttons and does NOT claim "This repository has no README."
    s.details.insert(key0.clone(), Load::Failed("Could not connect to GitHub".into()));
    s.page = Page::Repo(r0.clone());
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("retry-files").is_some(), "retry-files button must be present on Load::Failed");
    assert!(rt.find("readme-retry").is_some(), "readme-retry button must be present on Load::Failed");
    assert!(rt.elements().iter().all(|e| e.text.as_deref() != Some("This repository has no README.")), "must NOT claim no README on Load::Failed");
    assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("Could not connect to GitHub")), "must display the error message");
}

#[test]
fn secondary_rate_limit_and_retry_after() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = "http://127.0.0.1:9".into();
    let gh = crate::github::Gh::new(&cfg);

    // 1. 403 with "You have exceeded a secondary rate limit" and remaining 4999
    let resp_403 = Resp {
        status: 403,
        body: r#"{"message": "You have exceeded a secondary rate limit. Please wait a few minutes before you try again."}"#.into(),
        remaining: Some("4999".into()),
        reset: None,
        retry_after: None,
        link: None,
        etag: None,
    };
    let msg_403 = gh.explain(&resp_403);
    assert!(msg_403.contains("rate limit"), "must explain rate limit: {msg_403}");
    assert!(!msg_403.contains("write access"), "must NOT claim write access: {msg_403}");

    // 2. 429 with retry-after: 40
    let resp_429 = Resp {
        status: 429,
        body: r#"{"message": "rate limited"}"#.into(),
        remaining: None,
        reset: None,
        retry_after: Some(40),
        link: None,
        etag: None,
    };
    let msg_429 = gh.explain(&resp_429);
    assert!(msg_429.contains("40 s") || msg_429.contains("40"), "must mention retry duration: {msg_429}");
    assert!(!msg_429.contains("write access"), "must NOT claim write access: {msg_429}");
    assert_eq!(gh.retry_after(), Some(40), "Gh must record the 40s backoff");

    // 3. Regular 403 write permission failure still says write access
    let resp_generic_403 = Resp {
        status: 403,
        body: r#"{"message": "Resource not accessible by integration"}"#.into(),
        remaining: Some("4999".into()),
        reset: None,
        retry_after: None,
        link: None,
        etag: None,
    };
    let msg_generic_403 = gh.explain(&resp_generic_403);
    assert!(msg_generic_403.contains("The token needs write access"), "generic 403 must explain write access: {msg_generic_403}");

    // 4. Primary rate limit (status 403, remaining 0, no retry-after):
    let resp_primary_403 = Resp {
        status: 403,
        body: r#"{"message": "API rate limit exceeded for user"}"#.into(),
        remaining: Some("0".into()),
        reset: Some(crate::net::now() + 600),
        retry_after: None,
        link: None,
        etag: None,
    };
    let msg_primary_403 = gh.explain(&resp_primary_403);
    assert!(msg_primary_403.contains("API rate limit reached."), "must start with API rate limit reached.: {msg_primary_403}");
    assert!(msg_primary_403.contains("With GITHUB_TOKEN the limit goes up to 5,000 requests an hour."), "must mention GITHUB_TOKEN: {msg_primary_403}");
    assert_eq!(gh.retry_after(), Some(600), "must record reset backoff");

    // 5. Mock HTTP server returning 429 with retry-after: 40
    let server = MockServer::new(|_method, _path| {
        (429, vec![("Content-Type", "application/json"), ("retry-after", "40")], r#"{"message":"rate limited"}"#.into())
    });
    cfg.api = server.url();
    cfg.token = Some("tok".into());
    let gh_live = crate::github::Gh::new(&cfg);
    let err = pollster::block_on(gh_live.profile(&cfg)).unwrap_err();
    assert!(err.contains("40 s") || err.contains("40"));
    assert_eq!(gh_live.retry_after(), Some(40));

    // 6. App::refresh_in honours retry_after
    let mut app = crate::init();
    let mut app_cfg = (*app.cfg).clone();
    app_cfg.token = Some("tok".into());
    app_cfg.refresh_minutes = 5; // 5 minutes = 300 seconds
    app.cfg = std::sync::Arc::new(app_cfg);
    app.status = Status::Ready;
    app.now = 1_700_000_100;
    app.last_load = Some(1_700_000_000); // 100 seconds elapsed
    app.load_clock = Some(0.0);
    app.clock = 100.0;

    // Normal refresh: 300 - 100 = 200s left
    assert_eq!(app.refresh_in(), Some(200));

    // Rate limit backoff: retry_after of 500s > 300s -> 500 - 100 = 400s left
    app.retry_after = Some(500);
    assert_eq!(app.refresh_in(), Some(400));

    // When refresh is initiated, retry_after is cleared
    app.refresh();
    assert_eq!(app.retry_after, None);
}

#[test]
fn partial_failure_warnings_banner_and_tray_menu() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/search/issues") {
            (403, vec![("Content-Type", "application/json")], r#"{"message":"You have exceeded a secondary rate limit"}"#.into())
        } else if path.contains("/actions/runs") {
            (500, vec![("Content-Type", "application/json")], r#"{"message":"Internal Server Error"}"#.into())
        } else if path.contains("/stats/participation") {
            (202, vec![("Content-Type", "application/json")], "null".into())
        } else if path.contains("/user/starred") {
            (403, vec![("Content-Type", "application/json")], r#"{"message":"rate limit"}"#.into())
        } else if path.contains("/user/repos") {
            (200, vec![("Content-Type", "application/json")], r#"[{"name":"repo1","full_name":"alice/repo1","owner":{"login":"alice"},"description":"","language":"Rust","stargazers_count":10,"forks_count":1,"open_issues_count":2,"pushed_at":"2026-11-09T09:35:11Z","private":false,"fork":false,"archived":false,"default_branch":"main","topics":[],"html_url":"https://github.com/alice/repo1"}]"#.into())
        } else if path.contains("/user") {
            (200, vec![("Content-Type", "application/json")], r#"{"login":"alice","name":"Alice"}"#.into())
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("tok_alice".into());
    cfg.activity_repos = 1;

    let res = pollster::block_on(crate::github::load_all(&cfg));
    let data = res.expect("load_all succeeds partially despite search and runs failure");

    // Verify partial failure tracking in Data
    assert!(data.pulls_failed, "pulls_failed must be true");
    assert!(data.issues_failed, "issues_failed must be true");
    assert!(data.runs_failed, "runs_failed must be true");
    assert!(!data.warnings.is_empty(), "warnings must be populated");
    assert!(data.warnings.iter().any(|w| w.contains("Pull requests couldn't load")));
    assert!(data.warnings.iter().any(|w| w.contains("Issues couldn't load")));
    assert!(data.warnings.iter().any(|w| w.contains("Workflow runs couldn't load")));
    assert!(data.warnings.iter().any(|w| w.contains("Starred repositories couldn't load")));

    // Verify tray menu with failure flags and empty lists
    let mut app = crate::init();
    app.data = data.clone();
    let menu = crate::tray::menu(&app);

    let pulls_item = menu.iter().find(|e| e.id == "go:pulls").expect("go:pulls found");
    assert_eq!(pulls_item.label, "Pull requests couldn't load");

    let issues_item = menu.iter().find(|e| e.id == "go:issues").expect("go:issues found");
    assert_eq!(issues_item.label, "Issues couldn't load");

    let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
    assert_eq!(runs_item.label, "Workflow runs couldn't load", "must NOT claim 'Recent runs passed'");

    // When runs is not empty with passing run, but runs_failed is true, label must still be "Workflow runs couldn't load"
    app.data.runs = vec![crate::model::Run {
        id: 1,
        workflow: "CI".into(),
        repo: "alice/repo1".into(),
        branch: "main".into(),
        checks: crate::model::Checks::Passing,
        duration: "1m".into(),
        updated_at: 1_700_000_000,
        url: String::new(),
        pull_numbers: Vec::new(),
    }];
    app.data.runs_failed = true;
    let menu = crate::tray::menu(&app);
    let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
    assert_eq!(runs_item.label, "Workflow runs couldn't load", "must NOT claim 'Recent runs passed' on partial failure even if some runs passed");

    // When runs_failed is false and runs passed, label is "Recent runs passed"
    app.data.runs_failed = false;
    let menu = crate::tray::menu(&app);
    let runs_item = menu.iter().find(|e| e.id == "go:actions").expect("go:actions found");
    assert_eq!(runs_item.label, "Recent runs passed");

    // Verify banner rendering when Ready with warnings
    app.status = Status::Ready;
    let kit = themes::kit(themes::Looks::default().active(false));
    let banner_el = banner(&app, &kit);
    assert!(banner_el.is_some(), "banner must be visible when Ready with warnings");
}

#[test]
fn branches_cleared_on_refresh_retried_and_deleted_fallback() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut s = crate::init();
    let mut s_cfg = (*s.cfg).clone();
    s_cfg.token = Some("tok".into());
    s.cfg = std::sync::Arc::new(s_cfg);
    s.now = 1_700_000_000;
    s.data = model::Data::sample_at(s.now);
    s.data.sample = false;
    let repo_full = s.data.repos[0].full_name.clone();

    // 1. A refresh keeps the branch lists on screen (#126): opening a repository reads its
    // list again behind it (step 3), as does a refresh while it's open.
    s.branches.insert(repo_full.clone(), Load::Ready(vec!["main".into(), "feature".into()]));
    assert!(!s.branches.is_empty());
    let epoch = s.load_gen;
    s.apply(epoch, Ok(s.data.clone()));
    assert!(matches!(s.branches.get(&repo_full), Some(Load::Ready(_))), "kept through a refresh, no skeleton");

    // 2. Retrying Load::Failed in load_branches
    s.branches.insert(repo_full.clone(), Load::Failed("network timeout".into()));
    // calling load_branches(0, false) must NOT return early on Load::Failed
    s.load_branches(0, false);
    // Without handle, in live mode, branches is set to Loading
    assert!(matches!(s.branches.get(&repo_full), Some(Load::Loading)), "Load::Failed must be retried");

    // 3. App::open calls load_branches with force=true
    s.branches.insert(repo_full.clone(), Load::Ready(vec!["main".into()]));
    s.open(0);
    // Calling open(0) forces refresh of branches
    assert!(s.page == Page::Repo(repo_full.clone()));

    // 4. Deleted branch fallback: if branch_of points to a branch no longer in names
    s.page = Page::Repo(repo_full.clone());
    s.branch_of.insert(repo_full.clone(), "deleted-branch".into());
    assert_eq!(s.branch(0), "deleted-branch");

    s.apply_branches(s.load_gen, repo_full.clone(), Ok(vec!["main".into(), "feature-alive".into()]));

    // branch_of removed for that repo, falling back to default branch ("main")
    assert!(!s.branch_of.contains_key(&repo_full), "deleted branch must be removed from branch_of");
    assert_eq!(s.branch(0), "main", "must fall back to default branch");
}

#[test]
fn paged_branches_stars_and_sorted_runs() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/repos/alice/repo1/branches?per_page=100") && !path.contains("page=2") {
            let branches: Vec<serde_json::Value> = (1..=100)
                .map(|i| serde_json::json!({ "name": format!("feature-{i:03}") }))
                .collect();
            (200, vec![
                ("Content-Type", "application/json"),
                ("Link", r#"<https://api.github.com/repos/alice/repo1/branches?per_page=100&page=2>; rel="next""#),
            ], serde_json::to_string(&branches).unwrap())
        } else if path.contains("/repos/alice/repo1/branches?per_page=100&page=2") {
            let branches = serde_json::json!([
                { "name": "main" },
                { "name": "feature-101" }
            ]);
            (200, vec![("Content-Type", "application/json")], branches.to_string())
        } else if path.contains("/user/starred?per_page=100") && !path.contains("page=2") {
            let starred: Vec<serde_json::Value> = (1..=100)
                .map(|i| serde_json::json!({ "full_name": format!("owner/repo-{i:03}") }))
                .collect();
            (200, vec![
                ("Content-Type", "application/json"),
                ("Link", r#"<https://api.github.com/user/starred?per_page=100&page=2>; rel="next""#),
            ], serde_json::to_string(&starred).unwrap())
        } else if path.contains("/user/starred?per_page=100&page=2") {
            let starred = serde_json::json!([
                { "full_name": "owner/repo-101" },
                { "full_name": "alice/repo1" }
            ]);
            (200, vec![("Content-Type", "application/json")], starred.to_string())
        } else if path.contains("/repos/alice/repo1/actions/runs") {
            let runs = serde_json::json!({
                "workflow_runs": [
                    { "name": "CI", "head_branch": "main", "status": "completed", "conclusion": "success", "run_started_at": "2026-11-09T08:00:00Z", "updated_at": "2026-11-09T08:30:00Z", "html_url": "https://github.com/alice/repo1/actions/runs/1" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], runs.to_string())
        } else if path.contains("/repos/alice/repo2/actions/runs") {
            let runs = serde_json::json!({
                "workflow_runs": [
                    { "name": "Release", "head_branch": "main", "status": "completed", "conclusion": "success", "run_started_at": "2026-11-09T09:00:00Z", "updated_at": "2026-11-09T09:30:00Z", "html_url": "https://github.com/alice/repo2/actions/runs/2" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], runs.to_string())
        } else if path.contains("/stats/participation") {
            (200, vec![("Content-Type", "application/json")], "null".into())
        } else if path.contains("/user/repos") {
            let repos = serde_json::json!([
                { "name": "repo1", "full_name": "alice/repo1", "owner": { "login": "alice" }, "description": "", "language": "Rust", "stargazers_count": 10, "forks_count": 1, "open_issues_count": 2, "pushed_at": "2026-11-09T09:35:11Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/alice/repo1" },
                { "name": "repo2", "full_name": "alice/repo2", "owner": { "login": "alice" }, "description": "", "language": "Rust", "stargazers_count": 5, "forks_count": 0, "open_issues_count": 0, "pushed_at": "2026-11-09T09:30:00Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/alice/repo2" }
            ]);
            (200, vec![("Content-Type", "application/json")], repos.to_string())
        } else if path.contains("/search/issues") && path.contains("is%3Aopen") && path.contains("is%3Apr") {
            let pulls = serde_json::json!({
                "total_count": 132,
                "items": [
                    { "number": 1, "title": "Add feature", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T10:00:00Z", "comments": 2, "html_url": "https://github.com/alice/repo1/pull/1" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], pulls.to_string())
        } else if path.contains("/search/issues") && path.contains("is%3Aclosed") && path.contains("is%3Apr") {
            let pulls = serde_json::json!({
                "total_count": 200,
                "items": [
                    { "number": 50, "title": "Old PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "closed", "draft": false, "updated_at": "2026-11-08T10:00:00Z", "comments": 1, "html_url": "https://github.com/alice/repo1/pull/50" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], pulls.to_string())
        } else if path.contains("/search/issues") && path.contains("is%3Aissue") {
            let issues = serde_json::json!({
                "total_count": 75,
                "items": [
                    { "number": 2, "title": "Bug report", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T10:00:00Z", "comments": 1, "html_url": "https://github.com/alice/repo1/issues/2" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], issues.to_string())
        } else if path.contains("/user") {
            (200, vec![("Content-Type", "application/json")], r#"{"login":"alice","name":"Alice"}"#.into())
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("tok_alice".into());
    cfg.activity_repos = 2;

    let gh = crate::github::Gh::new(&cfg);

    // 1. Branches paged beyond 100 with default branch sorted first
    let branch_list = pollster::block_on(gh.branches("alice/repo1", "main")).expect("branches load");
    assert_eq!(branch_list.len(), 102, "paged branches beyond 100 loaded");
    assert_eq!(branch_list[0], "main", "default branch must be first");
    assert!(branch_list.contains(&"feature-001".to_string()));
    assert!(branch_list.contains(&"feature-101".to_string()));

    // 2. Load all: paged stars (beyond 100), sorted runs across repos, totals populated
    let data = pollster::block_on(crate::github::load_all(&cfg)).expect("load_all succeeds");
    assert!(data.repos[0].starred, "alice/repo1 was on page 2 of starred and must be starred=true");
    assert_eq!(data.open_pulls_total, Some(132));
    assert_eq!(data.closed_pulls_total, Some(200));
    assert_eq!(data.pulls_total(), Some(332));
    assert_eq!(data.issues_total, Some(75));
    assert_eq!(data.runs.len(), 2);
    // Runs sorted by updated_at descending: repo2 (09:30) before repo1 (08:30)
    assert_eq!(data.runs[0].workflow, "Release");
    assert_eq!(data.runs[1].workflow, "CI");
    assert!(data.prs_capped());
    assert!(data.issues_capped());

    // Verify UI subtitles and metrics with capped/total counts
    let mut app = crate::init();
    app.data = data;
    app.status = Status::Ready;
    app.page = crate::Page::Pulls;

    let menu = crate::tray::menu(&app);
    let pulls_item = menu.iter().find(|e| e.id == "go:pulls").expect("go:pulls");
    assert_eq!(pulls_item.label, "1+ open pull requests");
    let issues_item = menu.iter().find(|e| e.id == "go:issues").expect("go:issues");
    assert_eq!(issues_item.label, "1+ open issues");

    let mut rt = rune::Runtime::new(app, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains("Showing 1 of 132 open pull requests"))), "PR subtitle shows 'Showing 1 of 132 open pull requests'");
    assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("pulls-load-more")), "Load more button exists on Pulls");

    rt.update(|s, _| s.page = crate::Page::Issues);
    rt.frame(0.0);
    assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains("Showing 1 of 75"))), "Issues subtitle shows 'Showing 1 of 75'");
    assert!(rt.elements().iter().any(|e| e.key.as_deref() == Some("issues-load-more")), "Load more button exists on Issues");
}

#[test]
fn prs_capped_no_zero_plus_regression() {
    // When there are 0 open PRs, but closed_pulls_total has 200, prs_capped() must be false!
    let data = Data {
        pulls: vec![
            model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
                number: 1,
                title: "Closed PR".into(),
                repo: "alice/repo1".into(),
                author: "alice".into(),
                state: model::PullState::Closed,
                checks: None,
                updated_at: 1000,
                comments: 0,
                url: String::new(),
            }
        ],
        open_pulls_total: Some(0),
        closed_pulls_total: Some(200),
        ..Data::default()
    };
    assert!(!data.prs_capped(), "0 open PRs must NOT be capped even if closed_pulls_total is 200");
}

#[test]
fn branches_default_branch_guaranteed_when_past_ten_pages() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/branches") {
            let branches: Vec<serde_json::Value> = (1..=100)
                .map(|i| serde_json::json!({ "name": format!("feature-{i:03}") }))
                .collect();
            (200, vec![
                ("Content-Type", "application/json"),
                ("Link", r#"<https://api.github.com/repos/alice/huge/branches?per_page=100&page=2>; rel="next""#),
            ], serde_json::to_string(&branches).unwrap())
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("tok".into());

    let gh = crate::github::Gh::new(&cfg);
    let branches = pollster::block_on(gh.branches("alice/huge", "main")).expect("branches load");
    assert_eq!(branches[0], "main", "default branch 'main' must be index 0 even if omitted from first 10 pages");
}

#[test]
fn issues_chunking_with_more_than_twenty_orgs() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/search/issues") {
            if path.contains("user%3Aalice") {
                let res = serde_json::json!({
                    "total_count": 10,
                    "items": [
                        { "number": 1, "title": "Chunk 1 issue", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T10:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/issues/1" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], res.to_string())
            } else {
                let res = serde_json::json!({
                    "total_count": 5,
                    "items": [
                        { "number": 2, "title": "Chunk 2 issue", "repository_url": "https://api.github.com/repos/org25/repo2", "created_at": "2026-11-09T11:00:00Z", "comments": 0, "html_url": "https://github.com/org25/repo2/issues/2" }
                    ]
                });
                (200, vec![("Content-Type", "application/json")], res.to_string())
            }
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("tok".into());

    let gh = crate::github::Gh::new(&cfg);
    let orgs: Vec<String> = (1..=25).map(|i| format!("org{i}")).collect();
    let (issues, total) = pollster::block_on(gh.issues("alice", &orgs)).expect("issues load");
    assert_eq!(total, 15, "sum of chunk totals: 10 + 5 = 15");
    assert_eq!(issues.len(), 2, "both chunk issues merged");
    assert_eq!(issues[0].title, "Chunk 2 issue", "sorted newest first");
}

#[test]
fn load_more_pulls_and_issues_pages() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/search/issues") && path.contains("is%3Apr") && path.contains("page=2") {
            let pulls = serde_json::json!({
                "total_count": 2,
                "items": [
                    { "number": 1, "title": "Dupe PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T10:00:00Z", "comments": 2, "html_url": "https://github.com/alice/repo1/pull/1" },
                    { "number": 2, "title": "Page 2 PR", "repository_url": "https://api.github.com/repos/alice/repo1", "user": { "login": "alice" }, "state": "open", "draft": false, "updated_at": "2026-11-09T09:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/pull/2" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], pulls.to_string())
        } else if path.contains("/search/issues") && path.contains("is%3Aissue") && path.contains("page=2") {
            let issues = serde_json::json!({
                "total_count": 2,
                "items": [
                    { "number": 10, "title": "Page 2 Issue", "repository_url": "https://api.github.com/repos/alice/repo1", "created_at": "2026-11-09T12:00:00Z", "comments": 0, "html_url": "https://github.com/alice/repo1/issues/10" }
                ]
            });
            (200, vec![("Content-Type", "application/json")], issues.to_string())
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = Some("tok".into());

    let gh = crate::github::Gh::new(&cfg);

    // Load more pulls page 2 deduplicates against page 1
    let (pulls, total) = pollster::block_on(gh.pulls_page("alice", true, 2)).expect("pulls_page succeeds");
    assert_eq!(total, 2);
    assert_eq!(pulls.len(), 2);

    // Load more issues page 2
    let (issues, total) = pollster::block_on(gh.issues_page("alice", &[], 2)).expect("issues_page succeeds");
    assert_eq!(total, 2);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].title, "Page 2 Issue");
}

#[test]
fn public_orgs_and_repos_pagination_caps() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let server = MockServer::new(|_method, path| {
        if path.contains("/users/alice/orgs") && !path.contains("page=2") {
            let orgs: Vec<serde_json::Value> = (1..=100)
                .map(|i| serde_json::json!({ "login": format!("org{i:03}") }))
                .collect();
            (200, vec![
                ("Content-Type", "application/json"),
                ("Link", r#"<https://api.github.com/users/alice/orgs?per_page=100&page=2>; rel="next""#),
            ], serde_json::to_string(&orgs).unwrap())
        } else if path.contains("/users/alice/orgs?per_page=100&page=2") {
            let orgs = serde_json::json!([
                { "login": "org101" }
            ]);
            (200, vec![("Content-Type", "application/json")], orgs.to_string())
        } else if path.contains("/users/alice/repos") {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        } else if path.contains("/orgs/") && path.contains("/repos") {
            let repos = serde_json::json!([
                { "name": "repo", "full_name": "org/repo", "owner": { "login": "org" }, "description": "", "language": "Rust", "stargazers_count": 0, "forks_count": 0, "open_issues_count": 0, "pushed_at": "2026-11-09T09:30:00Z", "private": false, "fork": false, "archived": false, "default_branch": "main", "topics": [], "html_url": "https://github.com/org/repo" }
            ]);
            (200, vec![("Content-Type", "application/json")], repos.to_string())
        } else {
            (200, vec![("Content-Type", "application/json")], "[]".into())
        }
    });

    let mut cfg = Config::from_json("{}").unwrap();
    cfg.api = server.url();
    cfg.token = None; // unauthenticated: calls public orgs flow
    cfg.max_repos = 500;

    let gh = crate::github::Gh::new(&cfg);
    let mut warnings = Vec::new();
    let (repos, repos_capped) = pollster::block_on(gh.repos(&cfg, "alice", false, &mut warnings)).expect("repos load");
    assert!(repos_capped, "101 orgs > 50 must set repos_capped = true");
    assert_eq!(repos.len(), 50, "only first 50 orgs queried");
}
