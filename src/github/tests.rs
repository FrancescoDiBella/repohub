#[test]
fn check_runs_read_as_passed_failed_running_or_skipped() {
    use crate::thread::CheckState::*;
    let v = serde_json::json!({"check_runs": [
        {"name": "test", "status": "completed", "conclusion": "success", "html_url": "u1"},
        {"name": "lint", "status": "completed", "conclusion": "failure"},
        {"name": "build", "status": "in_progress", "conclusion": null},
        {"name": "deploy", "status": "completed", "conclusion": "skipped"},
    ]});
    let list = super::parse_check_runs(&v);
    let states: Vec<_> = list.iter().map(|c| c.state).collect();
    assert_eq!(states, vec![Passed, Failed, Running, Other]);
    assert_eq!(list[0].url, "u1");
    assert_eq!(crate::thread::checks_summary(&list), "1 failed, 1 running, 1 passed, 1 skipped");
    assert_eq!(crate::thread::checks_summary(&[]), "No checks on the last commit.");
}

#[test]
fn line_comments_read_their_side_and_skip_outdated_ones() {
    let v = serde_json::json!({"path": "src/a.rs", "line": 7, "side": "LEFT", "user": {"login": "bob"}, "body": "hm", "created_at": "2026-01-02T03:04:05Z"});
    let c = super::parse_line_comment(&v).expect("on a line");
    assert_eq!((c.path.as_str(), c.line, c.side, c.author.as_str()), ("src/a.rs", 7, rune_kit::diff::Side::Old, "bob"));
    let gone = serde_json::json!({"path": "src/a.rs", "line": null, "side": "RIGHT", "user": null, "body": ""});
    assert!(super::parse_line_comment(&gone).is_none(), "an outdated comment isn't on a line any more");
}

/// The `Link` header is read as links, not split on commas: an address or a quoted value
/// may hold one, and `rel` may name several relations.
#[test]
fn the_link_header_is_read_as_links() {
    use super::{next_page_path, parse_link_header};
    let api = "https://api.github.com";
    let github = r#"<https://api.github.com/search/issues?q=a&page=2>; rel="next", <https://api.github.com/search/issues?q=a&page=5>; rel="last""#;
    assert_eq!(next_page_path(Some(github), api).as_deref(), Some("/search/issues?q=a&page=2"));
    // A comma (and a semicolon) in the address, unencoded.
    let commas = r#"<https://api.github.com/search/issues?q=label:a,b;c&page=1>; rel="prev", <https://api.github.com/search/issues?q=label:a,b;c&page=3>; rel="next""#;
    assert_eq!(next_page_path(Some(commas), api).as_deref(), Some("/search/issues?q=label:a,b;c&page=3"));
    // A quoted parameter with a comma, `rel` with two relations, other spacing and case.
    let odd = r#"<https://api.github.com/x?page=1>;title="a, b";rel="first",<https://api.github.com/x?page=2> ; REL = "prev NEXT""#;
    assert_eq!(next_page_path(Some(odd), api).as_deref(), Some("/x?page=2"));
    assert_eq!(parse_link_header(odd)[0].1, vec![("title".to_owned(), "a, b".to_owned()), ("rel".to_owned(), "first".to_owned())]);
    // Unquoted, single-quoted, a relative address, GitHub Enterprise's /api/v3.
    assert_eq!(next_page_path(Some("</repos/o/r/branches?page=2>; rel=next"), api).as_deref(), Some("/repos/o/r/branches?page=2"));
    assert_eq!(next_page_path(Some("<https://ghe.example.com/api/v3/user/repos?page=2>; rel='next'"), "https://ghe.example.com/api/v3").as_deref(), Some("/user/repos?page=2"));
    // No next page, nothing, or something that isn't a link.
    assert_eq!(next_page_path(Some(r#"<https://api.github.com/x?page=1>; rel="prev""#), api), None);
    assert_eq!(next_page_path(None, api), None);
    assert_eq!(next_page_path(Some("garbage, more; rel=next"), api), None);
    assert_eq!(next_page_path(Some(r#"junk, <https://api.github.com/x?page=4>; rel="next""#), api).as_deref(), Some("/x?page=4"));
}

#[test]
fn check_runs_and_line_comments_say_when_github_has_more() {
    let v = serde_json::json!({"total_count": 130, "check_runs": [{"name": "a", "status": "completed", "conclusion": "success"}]});
    assert_eq!((super::check_runs_total(&v), super::parse_check_runs(&v).len()), (130, 1));
    assert_eq!(super::check_runs_total(&serde_json::json!({})), 0);
    let one = serde_json::json!([{"path": "a", "line": 1, "side": "RIGHT", "user": {"login": "x"}, "body": "b"}]);
    let full: Vec<serde_json::Value> = (0..100).map(|_| one[0].clone()).collect();
    let api = "https://api.github.com";
    let next = r#"<https://api.github.com/repos/o/r/pulls/1/comments?per_page=100&page=2>; rel="next""#;
    assert!(super::parse_line_comments("h".into(), &one, Some(next), api).more, "a next page");
    assert!(!super::parse_line_comments("h".into(), &one, None, api).more);
    assert!(super::parse_line_comments("h".into(), &serde_json::Value::Array(full.clone()), None, api).more, "a full page and no Link header");
    let last = r#"<https://api.github.com/repos/o/r/pulls/1/comments?per_page=100&page=1>; rel="prev""#;
    assert!(!super::parse_line_comments("h".into(), &serde_json::Value::Array(full), Some(last), api).more, "the last page");
}

#[test]
fn repo_name_from_url() {
    assert_eq!(super::repo_from_url("https://api.github.com/repos/octo/hello"), "octo/hello");
}

#[test]
fn branch_names_and_encoding() {
    use super::{enc, enc_path, valid_branch};
    for ok in ["main", "feature/login", "fix-12", "v1.2"] {
        assert!(valid_branch(ok), "{ok}");
    }
    for bad in ["", "has space", "-x", "a..b", "a/", "x.lock", "a:b", "a~1", "/a", "a//b"] {
        assert!(!valid_branch(bad), "{bad}");
    }
    assert_eq!(enc("feature/x y"), "feature%2Fx%20y");
    assert_eq!(enc_path("feature/x"), "feature/x");
}

#[test]
fn workflow_runs_and_jobs_parse_with_steps_and_pr_numbers() {
    use crate::model::Checks;
    let now = crate::model::parse_iso("2026-11-18T12:02:00Z").unwrap();
    let runs_json = serde_json::json!({"workflow_runs": [
        {
            "id": 7001,
            "name": "CI",
            "head_branch": "fix-bug",
            "status": "completed",
            "conclusion": "failure",
            "run_started_at": "2026-11-18T12:00:00Z",
            "updated_at": "2026-11-18T12:00:45Z",
            "html_url": "https://github.com/acme/app/actions/runs/7001",
            "pull_requests": [{"number": 19}]
        },
        {
            "name": "Fallback ID",
            "head_branch": "main",
            "status": "in_progress",
            "conclusion": null,
            "run_started_at": "2026-11-18T12:01:00Z",
            "updated_at": "2026-11-18T12:01:30Z",
            "html_url": "https://github.com/acme/app/actions/runs/7002"
        }
    ]});
    let runs = super::parse_workflow_runs(&runs_json, "acme/app", now);
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].id, runs[0].checks, runs[0].duration.as_str()), (7001, Checks::Failing, "45s"));
    assert_eq!(runs[0].pull_numbers, vec![19]);
    assert_eq!((runs[1].id, runs[1].checks, runs[1].duration.as_str()), (7002, Checks::Running, "1m 00s"));

    let jobs_json = serde_json::json!({"jobs": [
        {
            "id": 910,
            "name": "test (ubuntu-latest)",
            "status": "completed",
            "conclusion": "failure",
            "started_at": "2026-11-18T12:00:02Z",
            "completed_at": "2026-11-18T12:00:44Z",
            "html_url": "https://github.com/acme/app/actions/runs/7001/job/910",
            "steps": [
                {"number": 1, "name": "Checkout", "status": "completed", "conclusion": "success", "started_at": "2026-11-18T12:00:02Z", "completed_at": "2026-11-18T12:00:05Z"},
                {"number": 2, "name": "cargo test", "status": "completed", "conclusion": "failure", "started_at": "2026-11-18T12:00:05Z", "completed_at": "2026-11-18T12:00:44Z"}
            ]
        }
    ]});
    let jobs = super::parse_run_jobs(&jobs_json, now);
    assert_eq!(jobs.len(), 1);
    assert_eq!((jobs[0].id, jobs[0].checks, jobs[0].status_label.as_str(), jobs[0].duration.as_str()), (910, Checks::Failing, "failure", "42s"));
    assert_eq!(jobs[0].steps.len(), 2);
    assert_eq!((jobs[0].steps[1].number, jobs[0].steps[1].checks, jobs[0].steps[1].duration.as_str()), (2, Checks::Failing, "39s"));
}

#[test]
fn strip_ansi_and_tail_log_keep_last_200_lines() {
    let raw = "\x1b[32mPASS\x1b[0m step 1\r\n\x1b]8;;https://example.com\x07link\x1b]8;;\x07\n\x1b[1;31mFAIL\x1b[0m exit 1";
    assert_eq!(super::strip_ansi(raw), "PASS step 1\nlink\nFAIL exit 1");

    let many: String = (1..=250).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let tailed = super::tail_log(&many, super::LOG_TAIL_LINES);
    assert!(tailed.truncated);
    assert_eq!(tailed.total_lines, 250);
    assert_eq!(tailed.text.lines().count(), 200);
    assert!(tailed.text.starts_with("line 51\n"));
    assert!(tailed.text.ends_with("\nline 250"));
}

#[test]
fn explain_actions_write_mentions_actions_permission_on_403() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let gh = super::Gh::new(&crate::config::Config::load());
    let resp = crate::net::Resp {
        status: 403,
        body: r#"{"message":"Resource not accessible by personal access token"}"#.into(),
        remaining: Some("4990".into()),
        reset: None,
        retry_after: None,
        link: None,
        etag: None,
    };
    let msg = gh.explain_actions_write(&resp);
    assert!(msg.contains("Actions write permission"), "{msg}");
    assert!(msg.contains("Actions: Read and write"), "{msg}");
}
