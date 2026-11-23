//! GitHub's answers read into the app's model: conversations, pull requests, issues, notifications, checks,
//! workflow runs and job logs, a pull request's files and line comments.

use super::*;

/// One of GitHub's notification threads; `None` for one with no id or repository.
pub fn parse_notification(v: &Value) -> Option<Notification> {
    let id = s(v, "id");
    let repo = v.get("repository").map(|r| s(r, "full_name")).unwrap_or_default();
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
    if !id_ok || repo.is_empty() || !b(v, "unread") {
        return None;
    }
    let subject = v.get("subject")?;
    let kind = match s(subject, "type").as_str() {
        "Issue" => Subject::Issue,
        "PullRequest" => Subject::Pull,
        _ => Subject::Other,
    };
    // The subject's API address ends with the number: …/repos/o/r/pulls/12.
    let number = s(subject, "url").rsplit('/').next().and_then(|n| n.parse::<u32>().ok()).filter(|_| kind != Subject::Other);
    let page = match (kind, number) {
        (Subject::Issue, Some(n)) => format!("https://github.com/{repo}/issues/{n}"),
        (Subject::Pull, Some(n)) => format!("https://github.com/{repo}/pull/{n}"),
        _ => format!("https://github.com/{repo}"),
    };
    Some(Notification { id, repo, title: s(subject, "title"), subject: kind, number, reason: s(v, "reason"), updated_at: parse_iso(&s(v, "updated_at")).unwrap_or(0), url: page })
}

/// One of GitHub's comments on a diff line; `None` for one that isn't on a line any more.
pub fn parse_line_comment(v: &Value) -> Option<LineComment> {
    let line = v.get("line").and_then(Value::as_u64)? as u32;
    let side = if s(v, "side") == "LEFT" { rune_kit::diff::Side::Old } else { rune_kit::diff::Side::New };
    Some(LineComment {
        path: s(v, "path"),
        line,
        side,
        author: v.get("user").map(|u| s(u, "login")).filter(|l| !l.is_empty()).unwrap_or_else(|| "ghost".into()),
        body: s(v, "body"),
        at: parse_iso(&s(v, "created_at")).unwrap_or(0),
    })
}

pub(super) fn labels_of(v: &Value) -> Vec<(String, f32)> {
    v.get("labels").and_then(Value::as_array).into_iter().flatten().map(|l| (s(l, "name"), hex_hue(&s(l, "color")))).filter(|(n, _)| !n.is_empty()).collect()
}

pub(super) fn logins_of(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().map(|u| s(u, "login")).filter(|l| !l.is_empty()).collect()
}

pub fn parse_pull(v: &Value) -> Pull {
    let merged = v.get("pull_request").and_then(|p| p.get("merged_at")).is_some_and(|m| !m.is_null());
    let state = if merged { PullState::Merged } else if s(v, "state") == "closed" { PullState::Closed } else if b(v, "draft") { PullState::Draft } else { PullState::Open };
    let updated_at = s(v, "updated_at");
    Pull { labels: labels_of(v), assignees: logins_of(v, "assignees"), review_requested: false,
        number: n(v, "number"),
        title: s(v, "title"),
        repo: repo_from_url(&s(v, "repository_url")),
        author: v.get("user").map(|u| s(u, "login")).unwrap_or_default(),
        state,
        checks: None,
        updated_at: parse_iso(&updated_at).unwrap_or(0),
        comments: n(v, "comments"),
        url: s(v, "html_url"),
    }
}

/// Builds a conversation from GitHub's answers: the issue, its comments and, for a pull
/// request, its detail and reviews.
pub fn parse_thread(issue: &Value, comments: &Value, pull: Option<(&Value, &Value)>) -> Thread {
    let who = |v: &Value| v.get("user").map(|u| s(u, "login")).filter(|l| !l.is_empty()).unwrap_or_else(|| "ghost".into());
    let when = |v: &Value, k: &str| parse_iso(&s(v, k)).unwrap_or(0);
    let mut entries: Vec<Entry> = comments.as_array().into_iter().flatten()
        .map(|c| Entry { author: who(c), at: when(c, "created_at"), body: s(c, "body"), verdict: None })
        .collect();
    let mut more = n(issue, "comments") as usize > entries.len();
    let mut standing = if s(issue, "state") == "closed" { Standing::Closed } else { Standing::Open };
    let mut branches = None;
    let (mut requested, mut reviewed) = (Vec::new(), Vec::new());
    if let Some((detail, reviews)) = pull {
        requested = crate::triage::logins(detail, "requested_reviewers");
        reviewed = crate::triage::reviewed_of(reviews);
        if b(detail, "merged") {
            standing = Standing::Merged;
        } else if standing == Standing::Open && b(detail, "draft") {
            standing = Standing::Draft;
        }
        let branch = |k: &str| detail.get(k).map(|x| s(x, "ref")).unwrap_or_default();
        branches = Some((branch("head"), branch("base"))).filter(|(h, b)| !h.is_empty() && !b.is_empty());
        let list: Vec<&Value> = reviews.as_array().into_iter().flatten().collect();
        more |= list.len() >= 100;
        for r in list {
            let verdict = match s(r, "state").as_str() {
                "APPROVED" => Verdict::Approved,
                "CHANGES_REQUESTED" => Verdict::ChangesRequested,
                // A review not sent yet is only its author's; a dismissed one no longer counts.
                "PENDING" | "DISMISSED" => continue,
                _ => Verdict::Commented,
            };
            let body = s(r, "body");
            // A "commented" review with no text only carries comments on lines, shown on GitHub.
            if verdict == Verdict::Commented && body.trim().is_empty() {
                continue;
            }
            entries.push(Entry { author: who(r), at: when(r, "submitted_at"), body, verdict: Some(verdict) });
        }
    }
    entries.sort_by_key(|e| e.at);
    entries.insert(0, Entry { author: who(issue), at: when(issue, "created_at"), body: s(issue, "body"), verdict: None });
    Thread {
        title: s(issue, "title"),
        author: who(issue),
        standing,
        url: s(issue, "html_url"),
        labels: labels_of(issue),
        assignees: logins_of(issue, "assignees"),
        milestone: crate::triage::milestone_of(issue),
        requested,
        reviewed,
        branches,
        entries,
        more,
        checks: None,
        checks_total: 0,
    }
}

/// How many check runs GitHub has for the commit (its `total_count`), which may be more than
/// the page read.
pub fn check_runs_total(v: &Value) -> usize {
    v.get("total_count").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// The check runs in GitHub's answer for a commit's `check-runs`.
pub fn parse_check_runs(v: &Value) -> Vec<crate::thread::Check> {
    use crate::thread::{Check, CheckState};
    v.get("check_runs").and_then(Value::as_array).into_iter().flatten().map(|r| {
        let state = if s(r, "status") != "completed" {
            CheckState::Running
        } else {
            match s(r, "conclusion").as_str() {
                "success" => CheckState::Passed,
                "failure" | "timed_out" | "action_required" | "startup_failure" => CheckState::Failed,
                _ => CheckState::Other,
            }
        };
        Check { name: s(r, "name"), state, url: s(r, "html_url") }
    }).collect()
}

pub fn parse_issue(v: &Value) -> Issue {
    let created_at = s(v, "created_at");
    Issue { labels: labels_of(v), assignees: logins_of(v, "assignees"), author: v.get("user").map(|u| s(u, "login")).unwrap_or_default(),
        number: n(v, "number"),
        title: s(v, "title"),
        repo: repo_from_url(&s(v, "repository_url")),
        label: v.get("labels").and_then(Value::as_array).and_then(|l| l.first()).map(|l| (s(l, "name"), hex_hue(&s(l, "color")))),
        created_at: parse_iso(&created_at).unwrap_or(0),
        comments: n(v, "comments"),
        url: s(v, "html_url"),
    }
}

pub const LOG_TAIL_LINES: usize = 200;

fn checks_and_label(status: &str, conclusion: &str) -> (Checks, String) {
    if status != "completed" {
        let label = match status {
            "queued" => "queued",
            "waiting" => "waiting",
            "pending" => "pending",
            _ => "in progress",
        };
        (Checks::Running, label.into())
    } else {
        match conclusion {
            "success" => (Checks::Passing, "success".into()),
            "skipped" => (Checks::Passing, "skipped".into()),
            "neutral" => (Checks::Passing, "neutral".into()),
            "cancelled" => (Checks::Failing, "cancelled".into()),
            "timed_out" => (Checks::Failing, "timed out".into()),
            "action_required" => (Checks::Failing, "action required".into()),
            "startup_failure" => (Checks::Failing, "startup failure".into()),
            "" => (Checks::Failing, "failed".into()),
            other => (Checks::Failing, other.replace('_', " ")),
        }
    }
}

pub fn parse_workflow_runs(v: &Value, full: &str, now: i64) -> Vec<Run> {
    v.get("workflow_runs").and_then(Value::as_array).into_iter().flatten().map(|r| {
        let url = s(r, "html_url");
        let id = r.get("id").and_then(Value::as_u64).or_else(|| {
            url.rsplit('/').next().and_then(|x| x.parse::<u64>().ok())
        }).unwrap_or(0);
        let (checks, _) = checks_and_label(&s(r, "status"), &s(r, "conclusion"));
        let start = parse_iso(&s(r, "run_started_at"));
        let end = parse_iso(&s(r, "updated_at"));
        let pull_numbers = r.get("pull_requests").and_then(Value::as_array).into_iter().flatten()
            .map(|p| n(p, "number"))
            .filter(|&num| num > 0)
            .collect();
        Run {
            id,
            workflow: s(r, "name"),
            repo: full.to_owned(),
            branch: s(r, "head_branch"),
            duration: match (start, end) {
                (Some(a), Some(b)) if checks != Checks::Running => duration(b - a),
                (Some(a), _) => duration(now - a),
                _ => String::new(),
            },
            updated_at: end.unwrap_or(0),
            checks,
            url,
            pull_numbers,
        }
    }).collect()
}

pub fn parse_run_jobs(v: &Value, now: i64) -> Vec<RunJob> {
    v.get("jobs").and_then(Value::as_array).into_iter().flatten().map(|j| {
        let status = s(j, "status");
        let (checks, status_label) = checks_and_label(&status, &s(j, "conclusion"));
        let start = parse_iso(&s(j, "started_at"));
        let end = parse_iso(&s(j, "completed_at"));
        let dur = match (start, end) {
            (Some(a), Some(b)) if status == "completed" => duration(b - a),
            (Some(a), _) if status == "in_progress" => duration(now - a),
            _ => String::new(),
        };
        let steps = j.get("steps").and_then(Value::as_array).into_iter().flatten().map(|st| {
            let st_status = s(st, "status");
            let (st_checks, st_label) = checks_and_label(&st_status, &s(st, "conclusion"));
            let st_start = parse_iso(&s(st, "started_at"));
            let st_end = parse_iso(&s(st, "completed_at"));
            let st_dur = match (st_start, st_end) {
                (Some(a), Some(b)) if st_status == "completed" => duration(b - a),
                (Some(a), _) if st_status == "in_progress" => duration(now - a),
                _ => String::new(),
            };
            RunStep {
                number: n(st, "number"),
                name: s(st, "name"),
                checks: st_checks,
                status_label: st_label,
                duration: st_dur,
            }
        }).collect();
        RunJob {
            id: j.get("id").and_then(Value::as_u64).unwrap_or(0),
            name: s(j, "name"),
            checks,
            status_label,
            duration: dur,
            url: s(j, "html_url"),
            steps,
        }
    }).collect()
}

/// Strips terminal ANSI escape sequences (CSI colours, OSC hyperlinks/titles) from raw job logs.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                continue;
            }
            out.push('\n');
            continue;
        }
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(c) if ('('..='/').contains(&c) => {
                chars.next();
                chars.next();
            }
            Some(c) if ('\x40'..='\x5f').contains(&c) => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Keeps the last `max_lines` lines of `raw` after stripping ANSI escape codes.
pub fn tail_log(raw: &str, max_lines: usize) -> JobLog {
    let cleaned = strip_ansi(raw);
    let lines: Vec<&str> = cleaned.lines().collect();
    let total_lines = lines.len();
    let truncated = total_lines > max_lines;
    let start = total_lines.saturating_sub(max_lines);
    JobLog {
        text: lines[start..].join("\n"),
        total_lines,
        truncated,
    }
}

/// One file of a pull request, as GitHub lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullFile {
    pub path: String,
    /// Its old name, for a renamed file.
    pub previous: Option<String>,
    /// added, removed, modified, renamed, copied, changed, unchanged.
    pub status: String,
    /// The unified diff's hunks (no file headers); `None` for binary or very large files.
    pub patch: Option<String>,
}

/// A pull request's line comments as read: the commit its diff shows, the comments, and
/// whether GitHub has more than the first page.
#[derive(Clone, Debug, Default)]
pub struct LineComments {
    pub head: String,
    pub comments: Vec<LineComment>,
    pub more: bool,
}

/// GitHub's page of line comments: more when its `Link` header has a next page, or (without
/// one) when the page is full.
pub fn parse_line_comments(head: String, list: &Value, link: Option<&str>, api_base: &str) -> LineComments {
    let items = list.as_array().map(Vec::as_slice).unwrap_or_default();
    let more = match link {
        Some(l) => next_page_path(Some(l), api_base).is_some(),
        None => items.len() >= 100,
    };
    LineComments { head, comments: items.iter().filter_map(parse_line_comment).collect(), more }
}
