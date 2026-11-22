//! Loading, failed and empty: skeletons, the error with Retry, and the empty messages.

use std::sync::Arc;

use rune::TextEdit;

use crate::*;
use crate::model::{Data, Detail};
use crate::state::Load;

struct IsolatedEnv {
    prev_data_dir: Option<String>,
    dir: std::path::PathBuf,
}

impl IsolatedEnv {
    fn new(tag: &str) -> Self {
        let prev_data_dir = std::env::var("REPOHUB_DATA_DIR").ok();
        let dir = std::env::temp_dir().join(format!(
            "repohub-fix62-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        let _ = std::fs::create_dir_all(&dir);
        // SAFETY: callers hold crate::ENV_LOCK.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        Self { prev_data_dir, dir }
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        unsafe {
            match &self.prev_data_dir {
                Some(v) => std::env::set_var("REPOHUB_DATA_DIR", v),
                None => std::env::remove_var("REPOHUB_DATA_DIR"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn live_app() -> App {
    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.demo = false;
    cfg.user = Some("alice".into());
    cfg.token = None;
    cfg.api = "http://127.0.0.1:9".into();
    s.cfg = Arc::new(cfg);
    s.data = Data::default();
    s.pending_place = None;
    s.status = Status::Starting;
    s
}

fn sample_repo(full: &str) -> model::Repo {
    let (owner, name) = full.split_once('/').unwrap_or(("alice", full));
    model::Repo {
        name: name.into(),
        full_name: full.into(),
        owner: owner.into(),
        description: "Example repo".into(),
        language: "Rust".into(),
        hue: 40.0,
        stars: 3,
        forks: 1,
        issues: 0,
        pushed: 1_700_000_000,
        private: false,
        fork: false,
        archived: false,
        starred: false,
        branch: "main".into(),
        topics: vec![],
        url: format!("https://github.com/{full}"),
        ..model::Repo::default()
    }
}

fn has_text(rt: &rune::Runtime<App>, needle: &str) -> bool {
    rt.elements().iter().any(|e| e.text.as_deref() == Some(needle))
}

fn count_text(rt: &rune::Runtime<App>, needle: &str) -> usize {
    rt.elements().iter().filter(|e| e.text.as_deref() == Some(needle)).count()
}

fn contains_text(rt: &rune::Runtime<App>, needle: &str) -> bool {
    rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains(needle)))
}

fn new_rt(app: App) -> rune::Runtime<App> {
    let mut rt = rune::Runtime::new(app, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    rt
}

fn click(rt: &mut rune::Runtime<App>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing element key `{key}`"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0);
}

fn label_of(rt: &rune::Runtime<App>, key: &str) -> Option<String> {
    rt.elements().iter().find(|e| e.key.as_deref() == Some(key)).and_then(|e| e.label.clone())
}

#[test]
fn first_load_shows_skeletons_and_ready_empty_data_keeps_empty_messages() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _iso = IsolatedEnv::new("skeletons");

    let mut rt = new_rt(live_app());

    for status in [Status::Starting, Status::Loading] {
        rt.update(|s, _| {
            s.status = status.clone();
            s.page = Page::Overview;
        });
        rt.frame(1.0);
        assert!(rt.find("skeleton-activity").is_some(), "Overview shows activity skeleton in {status:?}");
        assert!(rt.find("skeleton-recent").is_some(), "Overview shows recent repos skeleton in {status:?}");
        assert!(rt.find("skeleton-pulls").is_some(), "Overview shows open PRs skeleton in {status:?}");
        assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("Loading Repositories"));
        assert_eq!(label_of(&rt, "metric-Stars received").as_deref(), Some("Loading Stars received"));
        assert_eq!(label_of(&rt, "metric-Open pull requests").as_deref(), Some("Loading Open pull requests"));
        assert_eq!(label_of(&rt, "metric-Open issues").as_deref(), Some("Loading Open issues"));
        assert!(!has_text(&rt, "0 commits"), "Overview hides 0 commits while loading ({status:?})");
        assert!(!has_text(&rt, "No repositories to show"));
        assert!(!has_text(&rt, "No open pull requests"));

        rt.update(|s, _| s.page = Page::Repos);
        rt.frame(1.0);
        assert!(rt.find("skeleton-repos").is_some(), "Repos shows skeleton list in {status:?}");
        assert!(rt.find("skeleton-card-0").is_some(), "Repos shows skeleton cards in {status:?}");
        assert!(!has_text(&rt, "No repositories to show"));
        assert!(!has_text(&rt, "0 of 0 repositories"));

        rt.update(|s, _| s.page = Page::Pulls);
        rt.frame(1.0);
        assert!(rt.find("skeleton-list").is_some(), "Pulls shows skeleton rows in {status:?}");
        assert!(!has_text(&rt, "No pull requests in this state"));

        rt.update(|s, _| s.page = Page::Issues);
        rt.frame(1.0);
        assert!(rt.find("skeleton-list").is_some(), "Issues shows skeleton rows in {status:?}");
        assert!(!has_text(&rt, "No open issues"));

        rt.update(|s, _| s.page = Page::Actions);
        rt.frame(1.0);
        assert!(rt.find("skeleton-list").is_some(), "Actions shows skeleton rows in {status:?}");
        assert!(!has_text(&rt, "No recent runs"));
        assert!(!contains_text(&rt, "0 of 0 recent runs"));
    }

    // Once the load finishes with genuinely empty results, the real "No …" messages return.
    rt.update(|s, _| {
        s.status = Status::Ready;
        s.page = Page::Overview;
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-activity").is_none());
    assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("0 Repositories"));
    assert!(has_text(&rt, "0 commits"));
    assert!(has_text(&rt, "No repositories to show"));
    assert!(has_text(&rt, "No open pull requests"));

    rt.update(|s, _| s.page = Page::Repos);
    rt.frame(1.0);
    assert!(rt.find("skeleton-repos").is_none());
    assert!(has_text(&rt, "No repositories to show"));

    rt.update(|s, _| {
        s.data.repos.push(sample_repo("alice/app"));
        s.search = TextEdit::new("zzzz-no-match");
    });
    rt.frame(1.0);
    assert!(has_text(&rt, "No repositories match your search"));

    rt.update(|s, _| {
        s.data = Data::default();
        s.page = Page::Pulls;
    });
    rt.frame(1.0);
    assert!(has_text(&rt, "No pull requests in this state"));

    rt.update(|s, _| s.page = Page::Issues);
    rt.frame(1.0);
    assert!(has_text(&rt, "No open issues"));

    rt.update(|s, _| s.page = Page::Actions);
    rt.frame(1.0);
    assert!(has_text(&rt, "No recent runs"));
}

#[test]
fn failed_first_load_shows_error_and_retry_once_and_retries() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _iso = IsolatedEnv::new("failed");

    let mut rt = new_rt(live_app());
    let err = "Could not reach GitHub (offline).";

    for (page, retry_key, forbidden_empty) in [
        (Page::Overview, "retry-overview", "No repositories to show"),
        (Page::Repos, "retry-repos", "No repositories to show"),
        (Page::Pulls, "retry-pulls", "No pull requests in this state"),
        (Page::Issues, "retry-issues", "No open issues"),
        (Page::Actions, "retry-actions", "No recent runs"),
    ] {
        rt.update(|s, _| {
            s.status = Status::Failed(err.into());
            s.page = page.clone();
        });
        rt.frame(1.0);
        let panel_key = format!("{retry_key}-panel");
        assert!(rt.find(&panel_key).is_some(), "{page:?} shows error panel in place of list");
        assert!(rt.find(retry_key).is_some(), "{page:?} shows Retry button in list area");
        assert!(rt.find("banner").is_none(), "{page:?} omits duplicate top banner when list shows error");
        assert_eq!(count_text(&rt, err), 1, "{page:?} shows the error message exactly once");
        assert_eq!(count_text(&rt, "Retry"), 1, "{page:?} shows Retry exactly once");
        assert!(!has_text(&rt, forbidden_empty), "{page:?} must not claim `{forbidden_empty}` on failed load");
        if page == Page::Overview {
            assert_eq!(label_of(&rt, "metric-Repositories").as_deref(), Some("Repositories, not loaded"));
            assert!(has_text(&rt, "—"), "Overview shows dash instead of 0 when load failed");
        }

        // Clicking Retry starts a fresh load and swaps the error panel for skeletons.
        click(&mut rt, retry_key);
        assert_eq!(rt.state.status, Status::Loading, "{page:?} Retry must start loading");
        assert!(rt.find(&panel_key).is_none());
    }

    // When data is already on screen, the banner remains the single place showing the error and Retry.
    rt.update(|s, _| {
        s.data.repos.push(sample_repo("alice/app"));
        s.status = Status::Failed(err.into());
        s.page = Page::Repos;
    });
    rt.frame(1.0);
    assert!(rt.find("banner").is_some(), "banner is kept when data is on screen");
    assert!(rt.find("retry").is_some());
    assert!(rt.find("retry-repos-panel").is_none());
    assert_eq!(count_text(&rt, err), 1);
    assert_eq!(count_text(&rt, "Retry"), 1);

    // Settings has no empty-list error panel, so the top banner still carries Retry there.
    rt.update(|s, _| {
        s.data = Data::default();
        s.status = Status::Failed(err.into());
        s.page = Page::Settings;
    });
    rt.frame(1.0);
    assert!(rt.find("banner").is_some(), "Settings keeps top banner on failed first load");
    assert!(rt.find("retry").is_some());
}

#[test]
fn code_tab_and_local_clone_show_skeletons_while_loading_and_retry_on_error() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _iso = IsolatedEnv::new("code-tab");

    let mut s = live_app();
    s.status = Status::Ready;
    s.data.repos.push(sample_repo("alice/app"));
    s.page = Page::Repo("alice/app".into());
    s.tab = RepoTab::Code;

    let mut rt = new_rt(s);
    rt.frame(1.0);

    // With no load started (`None`), show static prompts rather than an endless skeleton.
    assert!(rt.find("skeleton-files").is_none());
    assert!(rt.find("skeleton-readme").is_none());
    assert!(rt.find("skeleton-languages").is_none());
    assert!(has_text(&rt, "Open the repository to load its files"));
    assert!(has_text(&rt, "Open the repository to load its README"));
    assert!(!has_text(&rt, "This repository has no README."));

    // Once loading starts (`Some(Load::Loading)`), show skeletons for files, README, and languages.
    rt.update(|s, _| {
        s.details.insert(("alice/app".into(), "main".into()), Load::Loading);
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-files").is_some());
    assert!(rt.find("skeleton-readme").is_some());
    assert!(rt.find("skeleton-languages").is_some());
    assert!(!has_text(&rt, "Open the repository to load its files"));
    assert!(!has_text(&rt, "Open the repository to load its README"));
    assert!(!has_text(&rt, "Loading files…"));
    assert!(!has_text(&rt, "This repository has no README."));

    // Ready with an empty branch and no README -> real empty messages appear.
    rt.update(|s, _| {
        s.details.insert(("alice/app".into(), "main".into()), Load::Ready(Detail::default()));
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-files").is_none());
    assert!(rt.find("skeleton-readme").is_none());
    assert!(has_text(&rt, "No files in this branch"));
    assert!(has_text(&rt, "This repository has no README."));

    // Local clone file listing and viewer states.
    rt.update(|s, _| {
        s.files_from_github = false;
        s.local.insert("alice/app".into(), Load::Loading);
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-local-files").is_some());
    assert!(!has_text(&rt, "Reading the local clone…"));

    rt.update(|s, _| {
        s.local.insert("alice/app".into(), Load::Failed("Clone folder is unreadable.".into()));
    });
    rt.frame(1.0);
    assert!(has_text(&rt, "Clone folder is unreadable."));
    assert!(rt.find("retry-local-files").is_some());

    rt.update(|s, _| {
        s.local.insert("alice/app".into(), Load::Ready(local::Tree {
            branch: "main".into(),
            files: vec!["src/main.rs".into()],
            ..local::Tree::default()
        }));
        s.local_file = Some(("src/main.rs".into(), Load::Loading));
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-local-file").is_some());
    assert!(!has_text(&rt, "Reading…"));

    rt.update(|s, _| {
        s.local_file = Some(("src/main.rs".into(), Load::Failed("Permission denied.".into())));
    });
    rt.frame(1.0);
    assert!(has_text(&rt, "Permission denied."));
    assert!(rt.find("retry-local-file").is_some());
}

#[test]
fn changes_tab_and_pull_diff_show_skeletons_while_git_runs_and_retry_on_error() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _iso = IsolatedEnv::new("changes-tab");

    let mut s = live_app();
    s.status = Status::Ready;
    s.data.repos.push(sample_repo("alice/app"));
    s.page = Page::Repo("alice/app".into());
    s.tab = RepoTab::Changes;
    s.diff_load = Some(Load::Loading);

    let mut rt = new_rt(s);
    rt.frame(1.0);

    // Initial git status/diff loading -> skeleton diff, no "Reading the changes…" or "0 files".
    assert!(rt.find("skeleton-diff").is_some());
    assert!(!has_text(&rt, "Reading the changes…"));
    assert!(!has_text(&rt, "Nothing changed since the last commit."));
    assert!(!has_text(&rt, "0 files"));

    // While a background git command runs on an otherwise clean diff, show skeleton rather than "Nothing changed".
    rt.update(|s, _| {
        s.diff_load = Some(Load::Ready(()));
        s.git_job = Some("Pulling…");
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-diff").is_some());
    assert!(!has_text(&rt, "Nothing changed since the last commit."));

    // Once git finishes with no changed files -> real empty message.
    rt.update(|s, _| s.git_job = None);
    rt.frame(1.0);
    assert!(rt.find("skeleton-diff").is_none());
    assert!(has_text(&rt, "Nothing changed since the last commit."));

    // Failed git diff -> error + Retry button.
    rt.update(|s, _| s.diff_load = Some(Load::Failed("git diff failed.".into())));
    rt.frame(1.0);
    assert!(has_text(&rt, "git diff failed."));
    assert!(rt.find("retry-changes").is_some());

    // Pull request diff panel loading and failed states.
    rt.update(|s, _| {
        s.page = Page::Pulls;
        s.pull_diff_of = Some(("alice/app".into(), 12));
        s.pull_diff_load = Some(Load::Loading);
    });
    rt.frame(1.0);
    assert!(rt.find("skeleton-pull-diff").is_some());
    assert!(!has_text(&rt, "Reading the changes…"));
    assert!(!has_text(&rt, "0 files"));

    rt.update(|s, _| s.pull_diff_load = Some(Load::Failed("Could not load pull request files.".into())));
    rt.frame(1.0);
    assert!(has_text(&rt, "Could not load pull request files."));
    assert!(rt.find("retry-pull-diff").is_some());
}

#[test]
fn slow_mock_server_at_200ms_shows_skeletons_and_failed_load_shows_error_in_list() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _iso = IsolatedEnv::new("slow-mock");

    // 1. Slow mock server (holds request for 450 ms): at 0.2 s after start, skeletons are shown.
    let slow_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let slow_port = slow_listener.local_addr().unwrap().port();
    slow_listener.set_nonblocking(true).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let r2 = running.clone();
    let slow_srv = std::thread::spawn(move || {
        while r2.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = slow_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                for _ in 0..45 {
                    if !r2.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let body = "[]";
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(resp.as_bytes());
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    });

    let mut s = live_app();
    let mut cfg = (*s.cfg).clone();
    cfg.api = format!("http://127.0.0.1:{slow_port}");
    s.cfg = Arc::new(cfg);
    s.page = Page::Repos;
    s.refresh();
    let cfg_bg = s.cfg.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let load_thread = std::thread::spawn(move || {
        let _ = tx.send(pollster::block_on(github::load_all(&cfg_bg)));
    });

    let mut rt = new_rt(s);
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(17));
        if let Ok(res) = rx.try_recv() {
            let epoch = rt.state.load_gen;
            rt.update(move |app, _| app.apply(epoch, res));
        }
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.status, Status::Loading, "still loading at ~0.2 s against slow server");
    assert!(rt.find("skeleton-repos").is_some(), "skeletons shown at 0.2 s on Repos");
    assert!(!has_text(&rt, "No repositories to show"));

    running.store(false, Ordering::Relaxed);
    let _ = slow_srv.join();
    let _ = load_thread.join();

    // 2. Failing mock server (503): failed load shows the error and Retry once in the list.
    let fail_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let fail_port = fail_listener.local_addr().unwrap().port();
    let fail_srv = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = fail_listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = r#"{"message":"Service offline."}"#;
            let resp = format!("HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let mut fail_cfg = (*rt.state.cfg).clone();
    fail_cfg.api = format!("http://127.0.0.1:{fail_port}");
    let fail_res = pollster::block_on(github::load_all(&fail_cfg));
    let _ = fail_srv.join();
    assert!(fail_res.is_err());

    let epoch = rt.state.load_gen;
    rt.update(move |app, _| app.apply(epoch, fail_res));
    rt.frame(1.0);
    assert!(rt.find("retry-repos-panel").is_some(), "error panel shown in list after failed load");
    assert!(rt.find("retry-repos").is_some(), "Retry button shown in list after failed load");
    assert!(rt.find("banner").is_none());
    assert_eq!(count_text(&rt, "Retry"), 1);
    assert!(contains_text(&rt, "Service offline."));
    assert!(!has_text(&rt, "No repositories to show"));
}
