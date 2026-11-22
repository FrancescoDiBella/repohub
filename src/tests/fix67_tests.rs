//! The Actions page: filters, a run's jobs, steps and logs, and re-running.

use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::model::Checks;
use crate::state::RunStatusFilter;
use crate::{App, Page, Status, github, state};

fn click(rt: &mut rune::Runtime<App>, key: &str, t: &mut f64) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing element `{key}`"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    *t += 0.1;
    rt.frame(*t);
}

#[test]
fn actions_filters_and_run_detail_view_jobs_steps_and_logs() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    // The sample runs, whatever account the machine has.
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.toast = None;
    s.page = Page::Actions;

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 1800.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    let mut t = 0.0;
    rt.frame(t);

    // Filter bar and sample runs are rendered; the failing run has a quick Re-run failed button.
    assert!(rt.find("runs-status").is_some(), "status segmented control shown");
    assert!(rt.find("runs-repo-ngsi-ld-xr-contexts").is_some(), "repository chip shown");
    assert!(rt.find("runs-workflow-validate-jsonld").is_some(), "workflow chip shown");
    assert!(rt.find("run-ngsi-ld-xr-contexts-1003").is_some(), "failing run row shown");
    assert!(rt.find("rerun-row-ngsi-ld-xr-contexts-1003").is_some(), "quick Re-run failed button shown on failing row");
    assert!(rt.find("run-francescodibella.github.io-1001").is_some(), "passing run row shown");

    // Filter by Failed status -> keeps only the failing run and exposes Clear.
    rt.update(|app, _| app.runs_filter.status = RunStatusFilter::Failed);
    t += 0.1;
    rt.frame(t);
    assert!(rt.find("run-ngsi-ld-xr-contexts-1003").is_some());
    assert!(rt.find("run-francescodibella.github.io-1001").is_none());
    assert!(rt.find("runs-clear").is_some());

    // Clear restores all runs.
    click(&mut rt, "runs-clear", &mut t);
    assert!(rt.find("run-francescodibella.github.io-1001").is_some());

    // Filter by repository chip and workflow chip.
    click(&mut rt, "runs-repo-francescodibella.github.io", &mut t);
    assert!(rt.find("run-francescodibella.github.io-1001").is_some());
    assert!(rt.find("run-ngsi-ld-xr-contexts-1003").is_none());
    click(&mut rt, "runs-clear", &mut t);

    // Clicking a failing run opens the in-app run detail view with auto-selected failed job & log.
    click(&mut rt, "run-ngsi-ld-xr-contexts-1003", &mut t);
    assert!(rt.find("run-detail").is_some(), "run detail view opened");
    assert!(rt.find("run-jobs").is_some(), "jobs panel shown");
    assert!(rt.find("run-log").is_some(), "log panel shown");
    assert!(rt.find("run-log-text").is_some(), "selected job log text rendered");
    assert!(rt.find("run-copy-log").is_some(), "Copy log button rendered");
    assert!(rt.find("run-rerun-failed").is_some(), "Re-run failed jobs button rendered");
    assert!(rt.find("run-rerun-all").is_some(), "Re-run all button rendered");
    assert!(
        rt.elements().iter().any(|e| e.key.as_deref() == Some("run-log-text") && e.text.as_deref().is_some_and(|txt| txt.contains("Process completed with exit code 1."))),
        "failing job log automatically loaded and displayed",
    );

    // Selecting the passing lint job switches the displayed log.
    let lint_job_id = 1003 * 10 + 1;
    click(&mut rt, &format!("run-job-{lint_job_id}"), &mut t);
    assert_eq!(rt.state.open_run.as_ref().and_then(|o| o.selected_job), Some(lint_job_id));

    // Back button closes the run detail view back to Actions.
    click(&mut rt, "run-back", &mut t);
    assert!(rt.find("run-detail").is_none());
    assert!(rt.find("runs-status").is_some());

    // Re-open and verify Escape also closes the run detail view.
    click(&mut rt, "run-ngsi-ld-xr-contexts-1003", &mut t);
    assert!(rt.find("run-detail").is_some());
    rt.update(|app, tx| {
        let handled = crate::shortcuts(app, &rune::KeyEvent { key: rune::Key::Escape, mods: rune::Mods::default() }, tx);
        assert!(handled);
    });
    t += 0.1;
    rt.frame(t);
    assert!(rt.find("run-detail").is_none());
    assert!(rt.find("runs-status").is_some());
}

#[test]
fn live_job_log_follows_redirect_without_auth_and_rerun_handles_success_and_403() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Blob storage server: serves 250 ANSI-formatted log lines and checks Authorization is NOT forwarded.
    let blob_listener = TcpListener::bind("127.0.0.1:0").expect("bind blob mock");
    let blob_port = blob_listener.local_addr().unwrap().port();
    let blob_saw_auth = Arc::new(AtomicBool::new(false));
    let blob_saw_auth_bg = blob_saw_auth.clone();
    let blob_thread = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = blob_listener.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            if req.contains("\nauthorization:") || req.contains("\r\nauthorization:") {
                blob_saw_auth_bg.store(true, Ordering::SeqCst);
            }
            let mut body = String::new();
            for i in 1..=250 {
                body.push_str(&format!("\x1b[32mstep\x1b[0m line {i}\n"));
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    // GitHub API mock server: serves jobs, 302 redirect for logs, 403 then 201 for rerun-failed-jobs, and 201 for rerun.
    let api_listener = TcpListener::bind("127.0.0.1:0").expect("bind api mock");
    let api_port = api_listener.local_addr().unwrap().port();
    let api_saw_auth = Arc::new(AtomicBool::new(false));
    let api_saw_auth_bg = api_saw_auth.clone();
    let rerun_calls = Arc::new(AtomicUsize::new(0));
    let rerun_calls_bg = rerun_calls.clone();
    let api_thread = std::thread::spawn(move || {
        for _ in 0..5 {
            let Ok((mut stream, _)) = api_listener.accept() else { break };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first_line = req.lines().next().unwrap_or("");
            if req.to_ascii_lowercase().contains("authorization: bearer secret-pat") {
                api_saw_auth_bg.store(true, Ordering::SeqCst);
            }
            let resp = if first_line.starts_with("GET /repos/alice/app/actions/runs/77/jobs") {
                let body = r#"{"jobs":[
                        {"id":501,"name":"lint","status":"completed","conclusion":"success","started_at":"2026-11-18T10:00:00Z","completed_at":"2026-11-18T10:00:20Z","html_url":"https://github.com/alice/app/actions/runs/77/job/501","steps":[{"number":1,"name":"cargo clippy","status":"completed","conclusion":"success"}]},
                        {"id":502,"name":"test","status":"completed","conclusion":"failure","started_at":"2026-11-18T10:00:00Z","completed_at":"2026-11-18T10:01:05Z","html_url":"https://github.com/alice/app/actions/runs/77/job/502","steps":[{"number":1,"name":"cargo test","status":"completed","conclusion":"failure"}]}
                    ]}"#;
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
            } else if first_line.starts_with("GET /repos/alice/app/actions/jobs/502/logs") {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{blob_port}/raw-log\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            } else if first_line.starts_with("POST /repos/alice/app/actions/runs/77/rerun-failed-jobs") {
                let call = rerun_calls_bg.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    let body = r#"{"message":"Resource not accessible by personal access token"}"#;
                    format!("HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                } else {
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_owned()
                }
            } else if first_line.starts_with("POST /repos/alice/app/actions/runs/77/rerun") {
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_owned()
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
            };
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let mut cfg = crate::config::Config::load();
    cfg.api = format!("http://127.0.0.1:{api_port}");
    cfg.user = Some("alice".into());
    cfg.token = Some("secret-pat".into());
    cfg.allow_writes = true;

    // 1. Load jobs and verify parsing.
    let jobs = pollster::block_on(github::load_run_jobs(&cfg, "alice/app", 77)).expect("jobs load");
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[1].id, 502);
    assert_eq!(jobs[1].checks, Checks::Failing);

    // 2. Load job log via 302 redirect -> ANSI stripped, tailed to 200 of 250 lines, Authorization NOT leaked!
    let log = pollster::block_on(github::load_job_log(&cfg, "alice/app", 502)).expect("job log load");
    assert!(api_saw_auth.load(Ordering::SeqCst), "GitHub API request carried Authorization");
    assert!(!blob_saw_auth.load(Ordering::SeqCst), "redirected blob host MUST NOT receive Authorization header");
    assert!(log.truncated);
    assert_eq!(log.total_lines, 250);
    assert!(log.text.starts_with("step line 51\n"), "tailed to last 200 lines");
    assert!(log.text.ends_with("step line 250"));
    assert!(!log.text.contains("\x1b["), "ANSI escape sequences stripped");

    // 3. First re-run attempt returns 403 -> clear Actions write permission explanation, rendered in run-write-error.
    let err = pollster::block_on(github::run(&cfg, "alice/app", github::Write::RerunRun { run_id: 77, failed_only: true }))
        .expect_err("403 must surface error");
    assert!(err.contains("Actions: Read and write"), "403 message names fine-grained permission: {err}");
    assert!(err.contains("workflow"), "403 message names classic scope: {err}");

    let mut s = crate::init();
    s.pending_place = None;
    s.toast = None;
    s.cfg = Arc::new(cfg.clone());
    s.data.sample = false;
    s.status = Status::Ready;
    s.data.profile.login = "alice".into();
    s.data.runs = vec![crate::model::Run {
        id: 77,
        workflow: "CI".into(),
        repo: "alice/app".into(),
        branch: "feat/ci".into(),
        checks: Checks::Failing,
        duration: "1m 5s".into(),
        updated_at: s.now,
        url: "https://github.com/alice/app/actions/runs/77".into(),
        pull_numbers: vec![12],
    }];
    s.data.pulls = vec![crate::model::Pull {
        number: 12,
        title: "Fix CI".into(),
        repo: "alice/app".into(),
        author: "alice".into(),
        state: crate::model::PullState::Open,
        checks: Some(Checks::Failing),
        updated_at: s.now,
        comments: 0,
        labels: Vec::new(),
        assignees: Vec::new(),
        review_requested: false,
        url: "https://github.com/alice/app/pull/12".into(),
    }];
    s.open_run = Some(crate::runs_page::OpenRun {
        repo: "alice/app".into(),
        run_id: 77,
        workflow: "CI".into(),
        branch: "feat/ci".into(),
        checks: Checks::Failing,
        duration: "1m 5s".into(),
        updated_at: s.now,
        url: "https://github.com/alice/app/actions/runs/77".into(),
        epoch: 1,
        jobs: state::Load::Ready(jobs),
        selected_job: Some(502),
        logs: std::collections::HashMap::from([(502, state::Load::Ready(log))]),
        write_error: Some(err),
    });
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 1800.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    rt.frame(0.0);
    assert!(rt.find("run-write-error").is_some(), "permission error banner rendered in run detail");

    // 4. Second re-run attempt succeeds (201) -> transitions run to Running; full re-run also succeeds (201).
    let done = pollster::block_on(github::run(&cfg, "alice/app", github::Write::RerunRun { run_id: 77, failed_only: true }))
        .expect("201 rerun-failed-jobs succeeds");
    rt.update(|app, _| app.apply_done_inner("alice/app", done));
    assert_eq!(rt.state.data.runs[0].checks, Checks::Running);
    assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Re-running failed jobs"));

    let done_all = pollster::block_on(github::run(&cfg, "alice/app", github::Write::RerunRun { run_id: 77, failed_only: false }))
        .expect("201 rerun succeeds");
    assert!(matches!(done_all, github::Done::Reran { run_id: 77, failed_only: false }));

    // 5. Status-bar tray action opens Pull requests narrowed to Mine + Failing checks.
    rt.update(|app, _| app.tray_action("go:pulls:mine-failing"));
    assert_eq!(rt.state.page, Page::Pulls);
    assert!(rt.state.pull_filter.mine);
    assert!(rt.state.pull_filter.failing);

    let _ = blob_thread.join();
    let _ = api_thread.join();
}
