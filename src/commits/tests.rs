use super::*;
use serde_json::json;

fn row(sha: &str, title: &str, author: &str, at: i64) -> Row {
    Row { sha: sha.into(), title: title.into(), body: String::new(), author: author.into(), account: true, at, verified: false }
}

#[test]
fn a_list_of_commits_reads_who_when_and_whether_it_is_signed() {
    let v = json!([
        {
            "sha": "0123456789abcdef0123456789abcdef01234567",
            "commit": {
                "message": "Fix the bunny\r\n\r\nThe poncho went through the arms.\nNow it doesn't.\n",
                "author": { "name": "Ada", "date": "2026-11-20T10:00:00Z" },
                "committer": { "name": "GitHub", "date": "2026-11-21T09:00:00Z" },
                "verification": { "verified": true },
            },
            "author": { "login": "ada" },
            "committer": { "login": "web-flow" },
        },
        {
            "sha": "89abcdef0123456789abcdef0123456789abcdef",
            "commit": { "message": "Initial", "author": { "name": "Laura Bianchi", "date": "2026-11-01T08:00:00Z" }, "committer": null, "verification": { "verified": false } },
            "author": null,
        },
        { "sha": "not-a-commit", "commit": { "message": "skipped" } },
    ]);
    let rows = parse_rows(&v);
    assert_eq!(rows.len(), 2, "an entry without a valid id is left out");
    assert_eq!((rows[0].title.as_str(), rows[0].body.as_str()), ("Fix the bunny", "The poncho went through the arms.\nNow it doesn't."));
    assert_eq!((rows[0].author.as_str(), rows[0].account, rows[0].verified), ("ada", true, true));
    assert_eq!(rows[0].at, parse_iso("2026-11-21T09:00:00Z").unwrap(), "the committer's date orders the history");
    assert_eq!(rows[0].short(), "0123456");
    assert_eq!((rows[1].author.as_str(), rows[1].account, rows[1].verified), ("Laura Bianchi", false, false), "no account: the git name");
    assert_eq!(rows[1].at, parse_iso("2026-11-01T08:00:00Z").unwrap(), "no committer date: the author's");
    assert_eq!(split_message(""), ("(no message)".into(), String::new()));
}

#[test]
fn one_commit_reads_its_people_parents_totals_and_files() {
    let v = json!({
        "sha": "0123456789abcdef0123456789abcdef01234567",
        "html_url": "https://github.com/o/r/commit/0123456789abcdef0123456789abcdef01234567",
        "commit": {
            "message": "Merge branch 'feature'\n\nTwo lines\nof body",
            "author": { "name": "Ada Lovelace", "date": "2026-11-20T10:00:00Z" },
            "committer": { "name": "Bob", "date": "2026-11-20T11:00:00Z" },
            "verification": { "verified": false },
        },
        "author": { "login": "ada" },
        "committer": null,
        "parents": [{ "sha": "1111111111111111111111111111111111111111" }, { "sha": "2222222222222222222222222222222222222222" }],
        "stats": { "additions": 120, "deletions": 7, "total": 127 },
        "files": [
            { "filename": "src/a.rs", "status": "modified", "additions": 3, "deletions": 1, "patch": "@@ -1,2 +1,4 @@\n a\n-b\n+c\n+d\n+e" },
            { "filename": "logo.png", "status": "added", "additions": 0, "deletions": 0 },
            { "filename": "new.rs", "previous_filename": "old.rs", "status": "renamed", "additions": 0, "deletions": 0 },
        ],
    });
    let d = parse_detail(&v, false);
    assert_eq!((d.title.as_str(), d.body.as_str()), ("Merge branch 'feature'", "Two lines\nof body"));
    assert_eq!((d.author.shown(), d.author.name.as_str()), ("ada", "Ada Lovelace"));
    assert_eq!((d.committer.shown(), d.committer.login.as_ref()), ("Bob", None), "no account: the git name");
    assert_eq!(d.parents.len(), 2);
    assert_eq!((d.additions, d.deletions), (120, 7), "GitHub's totals, which count every file");
    assert_eq!(d.files.len(), 3);
    assert!(d.files[1].patch.is_none(), "a binary file has no patch");
    assert_eq!(d.files[2].previous.as_deref(), Some("old.rs"));
    assert!(!d.more_files);
    assert!(parse_detail(&v, true).more_files, "another page of files means more than 300");
    let shown = diffs::pull_files(&d.files);
    assert_eq!(shown.iter().map(|f| f.path()).collect::<Vec<_>>(), ["src/a.rs", "logo.png", "new.rs"]);
}

#[test]
fn a_comparison_reads_how_far_apart_the_branches_are() {
    let v = json!({
        "status": "diverged", "ahead_by": 3, "behind_by": 1, "total_commits": 3,
        "html_url": "https://github.com/o/r/compare/main...feature",
        "commits": [
            { "sha": "1111111111111111111111111111111111111111", "commit": { "message": "First", "committer": { "date": "2026-11-01T00:00:00Z" } } },
            { "sha": "2222222222222222222222222222222222222222", "commit": { "message": "Second", "committer": { "date": "2026-11-02T00:00:00Z" } } },
            { "sha": "3333333333333333333333333333333333333333", "commit": { "message": "Third", "committer": { "date": "2026-11-03T00:00:00Z" } } },
        ],
        "files": [{ "filename": "a.txt", "status": "modified", "additions": 4, "deletions": 2, "patch": "@@ -1 +1 @@\n-a\n+b" }],
    });
    let c = parse_comparison(&v, "main", "feature");
    assert_eq!((c.standing, c.ahead, c.behind), (Standing::Diverged, 3, 1));
    assert_eq!(c.commits.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Third", "Second", "First"], "newest first");
    assert_eq!((c.additions, c.deletions, c.more_commits, c.more_files), (4, 2, false, false));
    assert_eq!(c.summary(), "3 commits ahead, 1 behind main");
    assert!(!c.nothing());

    let more = json!({ "status": "ahead", "ahead_by": 300, "behind_by": 0, "total_commits": 300, "commits": [], "files": [] });
    let m = parse_comparison(&more, "main", "big");
    assert!(m.more_commits && m.summary() == "300 commits ahead of main");

    let same = parse_comparison(&json!({ "status": "identical", "ahead_by": 0, "behind_by": 0, "commits": [], "files": [] }), "main", "copy");
    assert_eq!(same.standing, Standing::Identical);
    assert!(same.nothing() && same.summary().contains("identical"), "{}", same.summary());
    let behind = parse_comparison(&json!({ "status": "behind", "ahead_by": 0, "behind_by": 1 }), "main", "old");
    assert!(behind.nothing() && behind.summary() == "old has nothing that main doesn't have: it's 1 commit behind.");
    assert!(Comparison::unrelated("main", "orphan").summary().contains("no history in common"));
    let one = parse_comparison(&json!({ "ahead_by": 1, "behind_by": 0 }), "main", "fix");
    assert_eq!((one.standing, one.summary().as_str()), (Standing::Ahead, "1 commit ahead of main"), "no status: from the counts");
}

#[test]
fn commits_are_grouped_by_day_on_the_users_clock() {
    // Sunday 22 November 2026, 10:00 UTC; the user is in UTC+1.
    let now = parse_iso("2026-11-22T10:00:00Z").unwrap();
    let rome = |t: i64| LocalTime::at_offset(t * 1000, 3600);
    let rows = [
        row("a000000", "late tonight", "x", parse_iso("2026-11-21T23:30:00Z").unwrap()),
        row("b000000", "this morning", "x", parse_iso("2026-11-22T08:00:00Z").unwrap()),
        row("c000000", "yesterday", "x", parse_iso("2026-11-21T12:00:00Z").unwrap()),
        row("d000000", "last week", "x", parse_iso("2026-11-12T12:00:00Z").unwrap()),
        row("e000000", "same day", "x", parse_iso("2026-11-12T09:00:00Z").unwrap()),
        row("f000000", "last year", "x", parse_iso("2025-12-31T22:00:00Z").unwrap()),
    ];
    let refs: Vec<&Row> = rows.iter().collect();
    let groups = days(&refs, now, rome);
    let seen: Vec<(&str, usize)> = groups.iter().map(|(l, r)| (l.as_str(), r.len())).collect();
    // 23:30 UTC on the 21st is already the 22nd in UTC+1.
    assert_eq!(seen, [("Today", 2), ("Yesterday", 1), ("12 November", 2), ("31 December 2025", 1)]);
    let utc = |t: i64| LocalTime::at_offset(t * 1000, 0);
    assert_eq!(day_label(rows[0].at, now, &utc), "Yesterday", "in UTC it's still the 21st");
    assert_eq!(day_label(now + 600, now, &utc), "Today", "a clock ahead of ours is still today");
    // A row's age agrees with its heading.
    assert_eq!(ago(parse_iso("2026-11-22T08:00:00Z").unwrap(), now, &rome), "2 hours ago");
    assert_eq!(ago(parse_iso("2026-11-20T23:30:00Z").unwrap(), now, &utc), "2 days ago", "34 hours, but two days on the calendar");
    assert_eq!(ago(parse_iso("2026-11-21T00:30:00Z").unwrap(), now, &utc), "yesterday");
    assert_eq!(ago(parse_iso("2026-11-12T12:00:00Z").unwrap(), now, &utc), "1 week ago");
}

#[test]
fn the_filter_narrows_by_message_author_or_commit_id() {
    let mut r = row("0123abcd99", "Fix the bunny's poncho", "ada", 0);
    r.body = "Closes #31".into();
    assert!(matches(&r, ""));
    assert!(matches(&r, "PONCHO") && matches(&r, "#31") && matches(&r, "ada") && matches(&r, "0123ab"));
    assert!(matches(&r, "ada poncho"), "every word must match somewhere");
    assert!(!matches(&r, "ada theme") && !matches(&r, "abcd"), "an id matches from its start");
    assert!(valid_sha("0123abc") && !valid_sha("0123ab") && !valid_sha("main") && !valid_sha(&"a".repeat(65)));
}

/// Answers GitHub's paths on 127.0.0.1 from `answer` (status, extra headers, body), and
/// records the paths asked for.
fn mock(answer: impl Fn(&str) -> (u16, String, String) + Send + Sync + 'static) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 2048];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let req = String::from_utf8_lossy(&buf).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
            log.lock().unwrap().push(path.clone());
            let (status, headers, body) = answer(&path);
            let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len());
        }
    });
    (url, seen)
}

fn gh(url: &str) -> crate::github::Gh {
    let mut cfg = crate::config::Config::from_json("{}").unwrap();
    cfg.api = url.to_owned();
    cfg.token = Some("test".into());
    crate::github::Gh::new(&cfg)
}

#[test]
fn requests_say_when_github_has_more_and_encode_branch_names() {
    let commit = |sha: &str| json!({ "sha": sha, "commit": { "message": "m", "committer": { "date": "2026-11-01T00:00:00Z" } } });
    let page: Vec<Value> = (0..PAGE).map(|n| commit(&format!("{n:040x}"))).collect();
    let full = serde_json::to_string(&page).unwrap();
    let (url, seen) = mock(move |path| {
        if path.starts_with("/repos/o/r/commits?sha=feature%2Fx") && path.ends_with("page=1") {
            (200, "Link: <https://api.github.com/repos/o/r/commits?sha=feature%2Fx&per_page=30&page=2>; rel=\"next\"\r\n".into(), full.clone())
        } else if path.starts_with("/repos/o/r/commits?") {
            (200, String::new(), "[]".into())
        } else if path == "/repos/o/r/commits/0123abc" {
            (200, "Link: <https://api.github.com/repositories/1/commits/0123abc?page=2>; rel=\"next\"\r\n".into(), json!({ "sha": "0123abc", "files": [] }).to_string())
        } else if path == "/repos/o/r/compare/main...feature/x" {
            (200, String::new(), json!({ "status": "ahead", "ahead_by": 1, "behind_by": 0, "commits": [commit(&"a".repeat(40))], "files": [] }).to_string())
        } else if path.starts_with("/repos/o/r/compare/main...orphan") {
            (404, String::new(), json!({ "message": "No common ancestor between main and orphan." }).to_string())
        } else if path.starts_with("/search/issues") {
            (200, String::new(), json!({ "total_count": 1, "items": [{ "number": 12, "html_url": "https://github.com/o/r/pull/12" }] }).to_string())
        } else {
            (404, String::new(), json!({ "message": "Not Found" }).to_string())
        }
    });
    let g = gh(&url);
    let (rows, more) = pollster::block_on(g.commits("o/r", "feature/x", 1)).unwrap();
    assert_eq!((rows.len(), more), (PAGE, true), "a Link to the next page");
    let (rows, more) = pollster::block_on(g.commits("o/r", "feature/x", 2)).unwrap();
    assert_eq!((rows.len(), more), (0, false));
    let d = pollster::block_on(g.commit("o/r", "0123abc")).unwrap();
    assert!(d.more_files, "another page of files");
    assert!(pollster::block_on(g.commit("o/r", "main")).is_err(), "not a commit id: nothing is asked");
    let c = pollster::block_on(g.compare("o/r", "main", "feature/x")).unwrap();
    assert_eq!((c.ahead, c.open_pull.clone()), (1, Some((12, "https://github.com/o/r/pull/12".into()))), "the open pull request is found");
    let u = pollster::block_on(g.compare("o/r", "main", "orphan")).unwrap();
    assert_eq!(u.standing, Standing::Unrelated, "no common history is a sentence, not an error");
    assert!(pollster::block_on(g.compare("o/r", "main", "-x")).is_err());
    let seen = seen.lock().unwrap().clone();
    assert!(seen.iter().any(|p| p == "/repos/o/r/commits?sha=feature%2Fx&per_page=30&page=2"), "{seen:?}");
    let search = seen.iter().find(|p| p.starts_with("/search/issues")).unwrap();
    assert!(search.contains("head%3Afeature%2Fx") && search.contains("base%3Amain") && search.contains("is%3Aopen"), "{search}");
    assert!(!seen.iter().any(|p| p.contains("/commits/main") || p.contains("-x")), "invalid ids and names never leave");
    assert_eq!(seen.iter().filter(|p| p.starts_with("/search")).count(), 1, "an unrelated branch isn't searched for");
}

#[test]
fn a_late_answer_for_another_branch_commit_or_comparison_is_dropped() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    // Live data, with nothing behind it: every request is answered by hand below.
    s.data.sample = false;
    s.pending_place = None;
    s.page = crate::Page::Repo(s.data.repos[0].full_name.clone());
    s.set_tab(0, crate::state::RepoTab::Commits);
    let account = s.load_gen;
    let main = s.commits.history.as_ref().map(|h| (h.branch.clone(), h.epoch)).unwrap();
    assert_eq!(main.0, "main");
    // Another branch is picked before main's answer arrives.
    s.set_branch(0, "feature/readme-selection".into());
    let feature = s.commits.history.as_ref().map(|h| (h.branch.clone(), h.epoch)).unwrap();
    assert_eq!(feature.0, "feature/readme-selection");
    assert_ne!(main.1, feature.1);
    let late = vec![row("1111111", "main's", "x", 1)];
    s.apply_history(account, main.1, 1, Ok((late.clone(), false)));
    assert!(!s.commits.history.as_ref().unwrap().load.ready().is_some_and(|r| r == &late), "main's answer is dropped");
    s.apply_history(account + 1, feature.1, 1, Ok((late.clone(), false)));
    assert!(s.commits.history.as_ref().unwrap().load.ready().is_none(), "another account's answer is dropped");
    let mine = vec![row("2222222", "feature's", "x", 1)];
    s.apply_history(account, feature.1, 1, Ok((mine.clone(), true)));
    let h = s.commits.history.as_ref().unwrap();
    assert!(h.load.ready() == Some(&mine) && h.more);
    // A failed reload keeps what's on screen.
    s.load_history(0, true);
    assert_eq!(s.commits.history.as_ref().unwrap().load.ready(), Some(&mine));
    // A second page, then a refresh with a commit pushed since: it goes on top, the pages stay.
    let epoch = s.commits.history.as_ref().unwrap().epoch;
    let older = vec![row("3333333", "older", "x", 0)];
    s.apply_history(account, epoch, 2, Ok((older, false)));
    s.load_history(0, true);
    let epoch = s.commits.history.as_ref().unwrap().epoch;
    let pushed = vec![row("4444444", "pushed since", "x", 2), row("2222222", "feature's", "x", 1)];
    s.apply_history(account, epoch, 1, Ok((pushed, true)));
    let h = s.commits.history.as_ref().unwrap();
    let titles: Vec<&str> = h.load.ready().unwrap().iter().map(|r| r.title.as_str()).collect();
    assert_eq!((titles, h.page, h.more), (vec!["pushed since", "feature's", "older"], 2, false));
    // A history rewritten under the list (a force push) starts again from its first page.
    s.load_history(0, true);
    let epoch = s.commits.history.as_ref().unwrap().epoch;
    s.apply_history(account, epoch, 1, Ok((vec![row("5555555", "rewritten", "x", 3)], true)));
    let h = s.commits.history.as_ref().unwrap();
    assert_eq!((h.load.ready().unwrap().len(), h.page, h.more), (1, 1, true));

    // A commit: the answer for the one opened before is dropped.
    let repo = s.data.repos[0].full_name.clone();
    s.open_commit(&repo, "aaaaaaa", None);
    let first = s.commits.open.as_ref().unwrap().epoch;
    s.open_commit(&repo, "bbbbbbb", None);
    let detail = |sha: &str| parse_detail(&json!({ "sha": sha, "commit": { "message": sha } }), false);
    s.apply_commit(account, first, Ok(detail("aaaaaaa")));
    assert!(!matches!(&s.commits.open.as_ref().unwrap().load, Load::Ready(d) if d.sha == "aaaaaaa"));
    let second = s.commits.open.as_ref().unwrap().epoch;
    s.apply_commit(account, second, Ok(detail("bbbbbbb")));
    assert!(matches!(&s.commits.open.as_ref().unwrap().load, Load::Ready(d) if d.sha == "bbbbbbb"));
    s.close_commit(None);

    // A comparison: closed, or asked again, before its answer.
    s.open_compare(0);
    if let Some(c) = s.commits.compare.as_mut() {
        c.head = "fix/theme-switch".into();
    }
    s.run_compare();
    let asked = s.commits.compare.as_ref().unwrap().epoch;
    assert!(matches!(s.commits.compare.as_ref().unwrap().result, Some(Load::Failed(_))), "no window yet: it says so, with Retry");
    s.run_compare();
    let cmp = |head: &str| parse_comparison(&json!({ "status": "ahead", "ahead_by": 1, "behind_by": 0 }), "main", head);
    s.apply_compare(account, asked, Ok(cmp("old")));
    assert!(!matches!(&s.commits.compare.as_ref().unwrap().result, Some(Load::Ready(c)) if c.head == "old"));
    s.close_compare();
    s.apply_compare(account, s.commits.epoch, Ok(cmp("late")));
    assert!(s.commits.compare.is_none(), "a closed comparison stays closed");
}

#[test]
fn the_sample_data_has_a_history_a_commit_and_a_comparison() {
    let data = crate::model::Data::sample_at(1_794_216_911);
    for r in &data.repos {
        let rows = sample_history(r, &r.branch, "me");
        assert!(rows.len() >= 3, "{}", r.name);
        assert!(rows.windows(2).all(|w| w[0].at > w[1].at), "newest first");
        assert_eq!(crate::model::sample_detail(r).commit.unwrap().sha, rows[0].short(), "the Code tab's last commit is the newest");
        let d = sample_detail(r, rows[0].short(), "me").expect("the newest commit opens");
        assert_eq!(d.parents, vec![rows[1].sha.clone()]);
        assert!(!d.files.is_empty());
    }
    let site = &data.repos[0];
    assert!(sample_history(site, "main", "me").len() > PAGE, "the first repository shows Load more");
    let c = sample_comparison(site, "main", "feature/readme-selection", "me", 1_794_216_911);
    assert_eq!((c.ahead, c.behind, c.summary().as_str()), (3, 1, "3 commits ahead, 1 behind main"));
    assert_eq!(sample_comparison(site, "main", "main", "me", 0).standing, Standing::Identical);
}

#[test]
fn immutable_commits_are_cached_in_memory() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data.sample = false;
    let repo = s.data.repos[0].full_name.clone();
    let full_sha = "a".repeat(40);
    let parent_sha = "b".repeat(40);
    let mut detail = sample_detail(&s.data.repos[0], &sample_history(&s.data.repos[0], "main", "me")[0].sha, "me").unwrap();
    detail.sha = full_sha.clone();
    detail.parents = vec![parent_sha.clone()];
    s.open_commit(&repo, &full_sha, None);
    let epoch = s.commits.open.as_ref().unwrap().epoch;
    s.apply_commit(s.load_gen, epoch, Ok(detail.clone()));
    // Navigating to a parent and pressing Back serves the cached commit synchronously (even without a handle).
    s.open_parent(&parent_sha);
    s.close_commit(None);
    assert_eq!(s.commits.open.as_ref().and_then(|o| o.load.ready()), Some(&detail));
    // Short SHAs are not cached to avoid prefix ambiguity.
    let mut short_detail = detail;
    short_detail.sha = "1234567".into();
    s.commits.remember_commit(&repo, &short_detail);
    assert!(s.commits.cached_commit(&repo, "1234567").is_none());
}
