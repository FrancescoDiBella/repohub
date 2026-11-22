//! Loading against a mock of GitHub on localhost: the parallel load gives the same lists and the
//! same warnings, in the same order, as reading everything one after the other; a second load
//! asks with the tags and GitHub's 304s reuse what was kept; at most `AT_ONCE` requests run at
//! once. Then the saved copy in the app: shown at start, never with the token, gone on sign-out
//! and account switch, ignored when broken, not written by a late load of another account.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::config::Config;
use crate::github::{self, Gh, LoadError, merge_review_requested};
use crate::model::{Data, Run, same_repo};
use crate::{cache, etag};

/// One request the mock saw.
#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub if_none_match: Option<String>,
    pub status: u16,
}

/// A small GitHub on 127.0.0.1: answers with `route`, tags every 200 with an ETag, answers 304
/// to a matching `If-None-Match`, and waits a few milliseconds (different for each path, so
/// answers come back in another order than they were asked).
pub struct Mock {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub most_at_once: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl Mock {
    pub fn start(route: impl Fn(&str) -> (u16, Value) + Send + Sync + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the mock");
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let most = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let route = Arc::new(route);
        let now = Arc::new(AtomicUsize::new(0));
        let (seen2, most2, stop2) = (seen.clone(), most.clone(), stop.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let (route, seen, most, now) = (route.clone(), seen2.clone(), most2.clone(), now.clone());
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let inm = req.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("if-none-match")).map(|(_, v)| v.trim().to_owned()));
                    let at_once = now.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(at_once, Ordering::SeqCst);
                    let wait = path.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64)) % 30 + 5;
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                    let (status, body) = route(&path);
                    let body = body.to_string();
                    let tag = format!("\"{:x}\"", body.bytes().fold(1469598103934665603u64, |a, b| (a ^ b as u64).wrapping_mul(1099511628211)));
                    let fresh = status == 200 && inm.as_deref() == Some(tag.as_str());
                    let resp = if fresh {
                        format!("HTTP/1.1 304 Not Modified\r\nETag: {tag}\r\nx-ratelimit-remaining: 4999\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    } else {
                        let reason = if status == 200 { "OK" } else { "Error" };
                        format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nETag: {tag}\r\nx-ratelimit-remaining: 4998\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                    };
                    now.fetch_sub(1, Ordering::SeqCst);
                    seen.lock().unwrap().push(Seen { path, if_none_match: inm, status: if fresh { 304 } else { status } });
                    let _ = stream.write_all(resp.as_bytes());
                });
            }
        });
        Mock { url, seen, most_at_once: most, stop }
    }

    pub fn take(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

fn repo(owner: &str, i: usize) -> Value {
    json!({
        "full_name": format!("{owner}/r{i}"), "name": format!("r{i}"), "owner": {"login": owner},
        "description": format!("Repository {i}"), "language": "Rust", "stargazers_count": i * 3, "forks_count": i,
        "open_issues_count": i % 4, "pushed_at": format!("2026-11-{:02}T10:00:00Z", 20 - i), "private": i.is_multiple_of(3),
        "fork": false, "archived": i == 7, "default_branch": "main", "topics": ["x"], "html_url": format!("https://github.com/{owner}/r{i}"),
    })
}

fn item(n: u32, repo: &str, pull: bool, state: &str) -> Value {
    let mut v = json!({
        "number": n, "title": format!("Item {n}"), "repository_url": format!("https://api.github.com/repos/{repo}"),
        "user": {"login": if n.is_multiple_of(2) { "octo" } else { "alice" }}, "state": state, "draft": n.is_multiple_of(5),
        "labels": [{"name": "bug", "color": "d73a4a"}], "assignees": [{"login": "octo"}],
        "created_at": "2026-11-10T10:00:00Z", "updated_at": format!("2026-11-{:02}T10:00:00Z", 1 + n % 20), "comments": n % 3,
        "html_url": format!("https://github.com/{repo}/issues/{n}"),
    });
    if pull {
        v["pull_request"] = json!({ "merged_at": if state == "closed" && n.is_multiple_of(2) { json!("2026-11-11T10:00:00Z") } else { Value::Null } });
    }
    v
}

/// What the mock knows: 10 repositories of `octo` and 2 of `acme`, pull requests, issues, runs
/// and activity; `broken` paths answer 500 with a message naming them.
fn github(broken: &'static [&'static str]) -> impl Fn(&str) -> (u16, Value) + Send + Sync + 'static {
    move |path: &str| {
        if let Some(b) = broken.iter().find(|b| path.starts_with(**b)) {
            return (500, json!({ "message": format!("broken {b}") }));
        }
        let p = path.split('?').next().unwrap_or("");
        let q = path.split_once('?').map(|(_, q)| q).unwrap_or("");
        let repos: Vec<Value> = (0..10).map(|i| repo("octo", i)).chain((10..12).map(|i| repo("acme", i))).collect();
        match p {
            "/user" | "/users/octo" => (200, json!({"login": "octo", "name": "Octo Cat", "bio": "hi"})),
            "/user/starred" => (200, json!([repos[2], repos[5]])),
            "/user/repos" | "/users/octo/repos" => (200, Value::Array(repos.clone())),
            "/users/octo/orgs" => (200, json!([{"login": "acme"}])),
            "/orgs/acme/repos" => (200, Value::Array(repos[10..].to_vec())),
            "/search/issues" if q.contains("review-requested") => (200, json!({"total_count": 2, "items": [item(3, "octo/r1", true, "open"), item(40, "acme/r10", true, "open")]})),
            "/search/issues" if q.contains("is%3Apr") && q.contains("is%3Aopen") => (200, json!({"total_count": 9, "items": (1..8).map(|n| item(n, &format!("octo/r{}", n % 5), true, "open")).collect::<Vec<_>>()})),
            "/search/issues" if q.contains("is%3Apr") => (200, json!({"total_count": 30, "items": (20..26).map(|n| item(n, "octo/r2", true, "closed")).collect::<Vec<_>>()})),
            "/search/issues" => (200, json!({"total_count": 50, "items": (100..112).map(|n| item(n, if n % 2 == 0 { "octo/r3" } else { "acme/r11" }, false, "open")).collect::<Vec<_>>()})),
            "/notifications" => (200, json!([])),
            _ if p.ends_with("/actions/runs") => {
                let full = p.trim_start_matches("/repos/").trim_end_matches("/actions/runs");
                (200, json!({"workflow_runs": (0..3).map(|k| json!({
                    "id": 1000 + k + full.len() as u64 * 10, "name": format!("CI {k}"), "head_branch": "main",
                    "status": "completed", "conclusion": if k == 1 { "failure" } else { "success" },
                    "run_started_at": "2026-11-18T12:00:00Z",
                    // Some runs of different repositories end at the same second: their order must not change.
                    "updated_at": format!("2026-11-18T12:0{}:00Z", k), "html_url": format!("https://github.com/{full}/actions/runs/{k}"),
                    "pull_requests": [{"number": k + 1}],
                })).collect::<Vec<_>>()}))
            }
            _ if p.ends_with("/stats/participation") => {
                let seed = p.len() as u32;
                (200, json!({"owner": (0..52).map(|w| (w + seed) % 5).collect::<Vec<_>>()}))
            }
            _ => (404, json!({"message": "Not Found"})),
        }
    }
}

fn config(api: &str, user: Option<&str>, token: Option<&str>) -> Config {
    let mut cfg = Config::load();
    cfg.api = api.to_owned();
    cfg.user = user.map(str::to_owned);
    cfg.token = token.map(str::to_owned);
    cfg.demo = false;
    cfg.max_repos = 100;
    cfg.activity_repos = 6;
    cfg.orgs = crate::state::Orgs::All;
    cfg.include_archived = true;
    cfg
}

/// `load_all` as it was before the parts ran side by side: everything one after the other.
async fn sequential(cfg: &Config) -> Result<Data, LoadError> {
    let gh = Gh::new(cfg);
    let profile = match gh.profile(cfg).await {
        Ok(p) => p,
        Err(e) => return Err(LoadError::new(e, gh.retry_after())),
    };
    let own = gh.has_token() && match &cfg.user {
        None => true,
        Some(u) => gh.viewer_login().await.is_some_and(|v| v.eq_ignore_ascii_case(u)),
    };
    let mut warnings = Vec::new();
    let (repos, repos_capped) = match gh.repos(cfg, &profile.login, own, &mut warnings).await {
        Ok(r) => r,
        Err(e) => return Err(LoadError::new(e, gh.retry_after())),
    };
    let mut pulls_failed = false;
    let mut open_pulls_total = None;
    let mut closed_pulls_total = None;
    let mut pulls = match gh.pulls(&profile.login).await {
        Ok((p, o, c)) => {
            open_pulls_total = Some(o);
            closed_pulls_total = Some(c);
            p
        }
        Err(e) => {
            pulls_failed = true;
            warnings.push(format!("Pull requests couldn't load: {e}"));
            Vec::new()
        }
    };
    let mut review_total = None;
    if own && !pulls_failed {
        match gh.review_requested(&profile.login).await {
            Ok((waiting, total)) => {
                review_total = Some(total);
                merge_review_requested(&mut pulls, waiting, &mut open_pulls_total);
            }
            Err(e) => warnings.push(format!("Pull requests waiting for your review couldn't load: {e}")),
        }
    }
    let mut orgs: Vec<String> = repos.iter().map(|r| r.owner.clone()).filter(|o| !o.eq_ignore_ascii_case(&profile.login)).collect();
    orgs.sort_by_key(|o| o.to_lowercase());
    orgs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let mut issues_failed = false;
    let mut issues_total = None;
    let issues = match gh.issues(&profile.login, &orgs).await {
        Ok((i, t)) => {
            issues_total = Some(t);
            i
        }
        Err(e) => {
            issues_failed = true;
            warnings.push(format!("Issues couldn't load: {e}"));
            Vec::new()
        }
    };
    let active: Vec<String> = repos.iter().filter(|r| !r.archived).take(cfg.activity_repos).map(|r| r.full_name.clone()).collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut runs_failed = false;
    let mut activity = vec![0u32; 26];
    let (mut run_error, mut participation_error) = (None, None);
    for full in &active {
        match gh.runs(full).await {
            Ok(r) => runs.extend(r),
            Err(e) => {
                runs_failed = true;
                run_error.get_or_insert(e);
            }
        }
        match gh.participation(full).await {
            Ok(part) => part.iter().enumerate().for_each(|(i, w)| activity[i] += w),
            Err(e) => {
                participation_error.get_or_insert(e);
            }
        }
    }
    if let Some(e) = run_error {
        warnings.push(format!("Workflow runs couldn't load: {e}"));
    }
    if let Some(e) = participation_error {
        warnings.push(format!("Commit activity couldn't load: {e}"));
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    for p in &mut pulls {
        if p.checks.is_none()
            && let Some(r) = runs.iter().find(|r| same_repo(&r.repo, &p.repo) && r.pull_numbers.contains(&p.number))
        {
            p.checks = Some(r.checks);
        }
    }
    let runs_total = runs.len();
    runs.truncate(20);
    Ok(Data {
        profile, repos, pulls, issues, runs, activity, warnings, pulls_failed, issues_failed, runs_failed,
        retry_after: gh.retry_after(), sample: false, open_pulls_total, closed_pulls_total, issues_total, runs_total,
        repos_capped, review_total,
    })
}

/// Everything a load gives that the screen shows, to compare two loads.
fn shown(d: &Result<Data, LoadError>) -> Value {
    match d {
        Ok(d) => json!({ "lists": cache::data_json(d), "warnings": d.warnings, "retryAfter": d.retry_after }),
        Err(e) => json!({ "error": e.message }),
    }
}

#[test]
fn the_parallel_load_equals_the_sequential_one_with_its_warnings_in_order() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    etag::clear();
    // Healthy; then with failures everywhere a warning can come from (two runs failing, so
    // "the first in the repositories' order" is checked too).
    let cases: [&'static [&'static str]; 4] = [
        &[],
        &["/user/starred", "/repos/octo/r1/actions/runs", "/repos/octo/r4/actions/runs", "/repos/octo/r2/stats/participation", "/search/issues?q=is%3Apr+is%3Aopen+review-requested"],
        &["/search/issues?q=is%3Apr+is%3Aopen+involves", "/search/issues?q=is%3Aissue"],
        &["/user/repos"],
    ];
    for broken in cases {
        let mock = Mock::start(github(broken));
        for (user, token) in [(None, Some("tok")), (Some("octo"), None), (Some("octo"), Some("tok"))] {
            let cfg = config(&mock.url, user, token);
            etag::clear();
            let one_by_one = pollster::block_on(sequential(&cfg));
            etag::clear();
            let side_by_side = pollster::block_on(github::load_all(&cfg));
            assert_eq!(shown(&side_by_side), shown(&one_by_one), "broken {broken:?}, user {user:?}, token {token:?}");
            if let (Ok(a), true) = (&side_by_side, broken.len() > 1) {
                assert!(!a.warnings.is_empty(), "the failures show as warnings");
            }
        }
        if broken == ["/user/repos"] {
            continue;
        }
        assert!(mock.most_at_once.load(Ordering::SeqCst) > 1, "the parts ran side by side");
    }
    etag::clear();
}

#[test]
fn a_second_load_asks_with_the_tags_and_reuses_the_kept_answers() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    etag::clear();
    let mock = Mock::start(github(&[]));
    let cfg = config(&mock.url, None, Some("tok"));
    let first = pollster::block_on(github::load_all(&cfg));
    let asked = mock.take();
    assert!(asked.iter().all(|s| s.if_none_match.is_none() && s.status == 200), "the first load reads everything");
    let reused_before = etag::reused_count();
    let second = pollster::block_on(github::load_all(&cfg));
    let again = mock.take();
    assert_eq!(shown(&second), shown(&first), "304s give the same lists");
    assert_eq!(again.len(), asked.len(), "the same requests");
    for s in &again {
        if s.path.starts_with("/search/") {
            assert!(s.if_none_match.is_none(), "searches aren't asked conditionally: {}", s.path);
            assert_eq!(s.status, 200);
        } else {
            assert!(s.if_none_match.is_some(), "asked with its tag: {}", s.path);
            assert_eq!(s.status, 304, "nothing changed: {}", s.path);
        }
    }
    let not_modified = again.iter().filter(|s| s.status == 304).count() as u64;
    assert!(not_modified >= 10, "most of a load is answered with 304");
    assert_eq!(etag::reused_count() - reused_before, not_modified);
    assert!(mock.most_at_once.load(Ordering::SeqCst) <= github::AT_ONCE, "never more than {} at once", github::AT_ONCE);
    // Another account forgets the tags: nothing is asked conditionally.
    etag::clear();
    let _ = pollster::block_on(github::load_all(&cfg));
    assert!(mock.take().iter().all(|s| s.if_none_match.is_none()));
    etag::clear();
}

#[test]
fn the_profile_and_repositories_arrive_before_the_rest() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    etag::clear();
    let mock = Mock::start(github(&[]));
    let cfg = config(&mock.url, None, Some("tok"));
    let early: Mutex<Vec<Data>> = Mutex::new(Vec::new());
    let full = pollster::block_on(github::load_all_with(&cfg, |d| early.lock().unwrap().push(d))).expect("loads");
    let early = early.into_inner().unwrap();
    assert_eq!(early.len(), 1, "once");
    assert_eq!(early[0].profile.login, "octo");
    assert_eq!(early[0].repos.len(), full.repos.len());
    assert!(early[0].pulls.is_empty() && early[0].issues.is_empty() && early[0].runs.is_empty(), "the rest comes with the whole load");
    etag::clear();
}

// ------------------------------------------------------------------ the saved copy in the app

const TOKEN: &str = "ghp_saved_copy_must_never_hold_this_0123456789";

fn live_app(user: &str) -> crate::App {
    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.user = Some(user.into());
    cfg.token = Some(TOKEN.into());
    cfg.api = "http://127.0.0.1:9".into();
    cfg.account_locked = false;
    cfg.demo = false;
    s.cfg = Arc::new(cfg);
    s.pending_place = None;
    s.data = Data::default();
    s.data_changed();
    s.set_status(crate::Status::Starting);
    s
}

fn loaded(login: &str) -> Data {
    let mut d = Data::sample_at(1_790_000_000);
    d.sample = false;
    d.profile.login = login.into();
    for r in &mut d.repos {
        r.owner = login.into();
        r.full_name = format!("{login}/{}", r.name);
    }
    d.warnings = vec!["Issues couldn't load: x".into()];
    d
}

fn copies() -> Vec<std::path::PathBuf> {
    std::fs::read_dir(cache::folder()).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

#[test]
fn a_load_is_saved_without_the_token_and_shown_at_the_next_start() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("saved-copy");
    etag::clear();
    let mut s = live_app("octo");
    etag::put(etag::generation(), "k".into(), etag::Entry { etag: "W/\"1\"".into(), body: "[]".into(), link: None });
    s.refresh();
    let epoch = s.load_gen;
    s.details.insert(("octo/dotfiles".into(), "main".into()), crate::state::Load::Ready(crate::model::sample_detail(&s.data.repos.first().cloned().unwrap_or_default())));
    s.apply(epoch, Ok(loaded("octo")));
    let files = copies();
    assert_eq!(files.len(), 1, "one copy: {files:?}");
    let path = cache::file(&s.cfg.api, "octo");
    assert_eq!(files[0], path);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(TOKEN), "the copy never holds the token");
    assert!(!text.contains("Issues couldn't load"), "nor the load's warnings");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "readable by you only");
    }
    assert!(s.fresh.info.is_some());

    // The next start: the lists at once, saying they're the saved copy, then the load.
    let mut next = live_app("octo");
    etag::clear();
    next.show_saved_copy();
    assert!(next.fresh.from_copy);
    assert_eq!(next.data.repos.len(), loaded("octo").repos.len());
    assert!(next.data.warnings.is_empty());
    assert!(next.details.contains_key(&("octo/dotfiles".to_owned(), "main".to_owned())), "the repository pages read before");
    assert!(etag::get("k").is_some(), "the tags come back with it");
    next.now = next.loaded_at.expect("when the copy was loaded") + 7200;
    assert_eq!(next.freshness().as_deref(), Some("Updating…"), "the first load starts at once");
    next.refresh();
    assert_eq!(next.freshness().as_deref(), Some("Updating…"));
    // A failed refresh keeps the copy and says how old it is.
    let epoch = next.load_gen;
    next.apply(epoch, Err("GitHub is down".into()));
    assert!(!next.data.repos.is_empty() && next.fresh.from_copy);
    assert_eq!(next.freshness().as_deref(), Some("Data from 2 hours ago"));
    assert!(crate::cache_page::shown_copy(&next).contains("2 hours ago"));
    // Another account's start doesn't see it.
    let mut other = live_app("alice");
    other.show_saved_copy();
    assert!(other.data.is_empty() && !other.fresh.from_copy);
    etag::clear();
}

#[test]
fn sign_out_and_account_switch_remove_the_copy_and_a_late_load_writes_none() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("saved-copy-gone");
    for sign_out in [true, false] {
        let mut s = live_app("octo");
        s.refresh();
        let old = s.load_gen;
        s.apply(old, Ok(loaded("octo")));
        assert_eq!(copies().len(), 1);
        s.refresh();
        let late = s.load_gen;
        if sign_out {
            s.save_account(true);
        } else {
            s.acct_user = rune::TextEdit::new("alice");
            s.acct_token = rune::TextEdit::new("tok_alice");
            s.save_account(false);
        }
        assert!(copies().is_empty(), "removed (sign out: {sign_out})");
        // A load of the previous account answering late writes nothing.
        s.apply(late, Ok(loaded("octo")));
        assert!(copies().is_empty(), "a late load of the previous account isn't saved");
        // A write that started before the removal doesn't happen either.
        let before = cache::generation();
        cache::forget_all();
        let copy = cache::Copy { api: "http://127.0.0.1:9".into(), account: "octo".into(), data: loaded("octo"), ..cache::Copy::default() };
        assert_eq!(cache::write(&copy.api, &copy.account, &copy, before, u64::MAX), Ok(0));
        assert!(copies().is_empty());
    }
}

#[test]
fn a_broken_or_older_copy_is_ignored_and_removed_and_off_means_off() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("saved-copy-broken");
    let s = live_app("octo");
    let path = cache::file(&s.cfg.api, "octo");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for broken in ["{\"format\":1,\"data\":", "{\"format\":0,\"api\":\"http://127.0.0.1:9\",\"account\":\"octo\",\"data\":{}}", "\u{0}\u{1}"] {
        std::fs::write(&path, broken).unwrap();
        let mut s = live_app("octo");
        s.show_saved_copy();
        assert!(s.data.is_empty() && !s.fresh.from_copy, "{broken:?} isn't shown");
        assert!(!path.exists(), "and removed");
    }
    // REPOHUB_CACHE=off: nothing written, nothing read.
    // SAFETY: the environment lock is held.
    unsafe { std::env::set_var("REPOHUB_CACHE", "off") };
    let mut s = live_app("octo");
    s.refresh();
    let epoch = s.load_gen;
    s.apply(epoch, Ok(loaded("octo")));
    assert!(copies().is_empty());
    unsafe { std::env::remove_var("REPOHUB_CACHE") };
}

#[test]
fn clear_saved_data_in_settings_removes_the_copy() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("saved-copy-clear");
    let mut s = live_app("octo");
    s.refresh();
    let epoch = s.load_gen;
    s.apply(epoch, Ok(loaded("octo")));
    assert_eq!(copies().len(), 1);
    s.page = crate::Page::Settings;
    s.settings_section = crate::state::SettingsSection::About;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains("The next start shows it at once"))));
    let (x, y) = rt.find("clear-saved-data").expect("Clear saved data");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
    assert!(copies().is_empty(), "cleared");
    assert!(rt.state.toast.as_ref().is_some_and(|t| t.text.contains("Saved data cleared")));
    assert!(!rt.state.data.repos.is_empty(), "the lists on screen stay");
}

#[test]
fn the_first_part_of_a_load_shows_with_the_other_lists_loading() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("early-part");
    let mut s = live_app("octo");
    s.refresh();
    let epoch = s.load_gen;
    let full = loaded("octo");
    s.apply_early(epoch - 1, Data { profile: full.profile.clone(), repos: full.repos.clone(), ..Data::default() });
    assert!(s.data.is_empty(), "another load's part is dropped");
    s.apply_early(epoch, Data { profile: full.profile.clone(), repos: full.repos.clone(), ..Data::default() });
    assert!(s.lists_loading() && !s.data.repos.is_empty());
    s.page = crate::Page::Pulls;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 1600.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("skeleton-list").is_some(), "pull requests are still loading, not empty");
    rt.update(|s, _| s.go(crate::Page::Overview));
    rt.frame(0.1);
    assert!(rt.find("skeleton-pulls").is_some() && rt.find("skeleton-recent").is_none(), "repositories shown, pull requests loading");
    // A repository's Issues, Pull requests and Actions sections too.
    for tab in [crate::state::RepoTab::Issues, crate::state::RepoTab::Pulls, crate::state::RepoTab::Actions] {
        rt.update(|s, _| {
            s.open(0);
            s.set_tab(0, tab);
        });
        rt.frame(0.15);
        assert!(rt.find("repo-list-loading").is_some(), "{tab:?}: loading, not \"Nothing to show here\"");
    }
    rt.update(|s, _| s.apply(epoch, Ok(full)));
    rt.frame(0.2);
    assert!(!rt.state.lists_loading() && rt.find("skeleton-pulls").is_none());
    // Over lists already on screen a part changes nothing.
    rt.update(|s, _| {
        s.refresh();
        let e = s.load_gen;
        s.apply_early(e, Data::default());
    });
    assert!(!rt.state.lists_loading() && !rt.state.data.repos.is_empty());
}

#[test]
fn conversations_and_repository_pages_read_before_show_at_once() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("seen-before");
    let mut s = live_app("octo");
    s.refresh();
    let epoch = s.load_gen;
    s.apply(epoch, Ok(loaded("octo")));
    let t = crate::thread::sample("Fix it", "octo", false, crate::thread::Standing::Open, 1_790_000_000);
    s.remember_thread("octo/dotfiles", 7, t.clone());
    s.open_thread("octo/dotfiles", 7, false);
    assert_eq!(s.thread.as_ref().map(|o| o.load.clone()), Some(crate::state::Load::Ready(t)), "shown at once (read again behind it)");
    s.open_thread("octo/dotfiles", 8, false);
    assert!(matches!(s.thread.as_ref().map(|o| &o.load), Some(crate::state::Load::Loading) | Some(crate::state::Load::Failed(_))), "one never read loads");

    // A page of a repository pushed to since the last load stays, and is read again when opened.
    let i = 1;
    let key = (s.data.repos[i].full_name.clone(), s.data.repos[i].branch.clone());
    s.details.insert(key.clone(), crate::state::Load::Ready(crate::model::Detail { readme: Some("old".into()), ..Default::default() }));
    let mut next = s.data.clone();
    next.repos[i].pushed += 60;
    s.refresh();
    let epoch = s.load_gen;
    s.apply(epoch, Ok(next));
    assert!(s.fresh.stale.contains(&key), "marked to be read again");
    s.load_detail(i);
    assert!(matches!(s.details.get(&key), Some(crate::state::Load::Ready(_))), "it stays on screen meanwhile, no skeleton");
    assert!(!s.fresh.stale.contains(&key));
    // A failed read behind it keeps the page.
    s.apply_detail(s.load_gen, key.clone(), crate::model::Detail { error: Some("offline".into()), ..Default::default() });
    assert!(matches!(s.details.get(&key), Some(crate::state::Load::Ready(d)) if d.readme.as_deref() == Some("old")));
}

/// Only a token is set, and it was changed outside the app to another account's: the copy
/// (named after the host, not an account) is the previous account's. It goes as soon as the
/// new account's profile and repositories are in, not after the whole load (nor never, when
/// the rest of the load fails).
#[test]
fn another_accounts_copy_goes_as_soon_as_its_profile_is_in() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("saved-copy-other");
    etag::clear();
    let token_only = |s: &mut crate::App| {
        let mut cfg = (*s.cfg).clone();
        cfg.user = None;
        s.cfg = Arc::new(cfg);
    };
    let mut first = live_app("octo");
    token_only(&mut first);
    first.refresh();
    let epoch = first.load_gen;
    first.apply(epoch, Ok(loaded("octo")));
    assert_eq!(copies(), vec![cache::file(&first.cfg.api, "")], "the token's copy");

    // The next start, with another account's token in the same place.
    let mut next = live_app("octo");
    token_only(&mut next);
    next.show_saved_copy();
    assert_eq!(next.data.profile.login, "octo");
    next.refresh();
    let epoch = next.load_gen;
    let mut alice = loaded("alice");
    (alice.pulls, alice.issues, alice.runs) = (Vec::new(), Vec::new(), Vec::new());
    next.apply_early(epoch, alice);
    assert_eq!(next.data.profile.login, "alice", "the other account's lists are gone");
    assert!(next.data.repos.iter().all(|r| r.owner == "alice"));
    assert!(!next.fresh.from_copy && next.lists_loading(), "the rest of the lists are loading");
    assert!(copies().is_empty(), "and its copy");
    next.apply(epoch, Err("GitHub is down".into()));
    assert_eq!(next.data.profile.login, "alice", "a failure doesn't bring it back");

    // The same account's copy stays on screen until the whole load is in.
    let mut same = live_app("octo");
    token_only(&mut same);
    let mut octo = loaded("octo");
    octo.repos.truncate(1);
    same.data = loaded("octo");
    same.data_changed();
    same.fresh.from_copy = true;
    same.refresh();
    let epoch = same.load_gen;
    let shown = same.data.repos.len();
    same.apply_early(epoch, octo);
    assert_eq!(same.data.repos.len(), shown);
    assert!(same.fresh.from_copy);
    etag::clear();
}
