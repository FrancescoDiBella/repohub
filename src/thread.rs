//! An issue's or a pull request's conversation: its description, comments and reviews.
//!
//! GitHub keeps both under `/issues/{n}` (a pull request is an issue with code), reviews under
//! `/pulls/{n}/reviews`. The page is `thread_page`; the requests are in `github`; labels,
//! assignees, milestone, reviewers and editing are `triage`.
use rune::TextEdit;

use crate::state::Load;

/// Where an issue or pull request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Open,
    Draft,
    Closed,
    Merged,
}

impl Standing {
    pub fn label(self) -> &'static str {
        match self {
            Standing::Open => "Open",
            Standing::Draft => "Draft",
            Standing::Closed => "Closed",
            Standing::Merged => "Merged",
        }
    }

    /// Open or a draft: it can still be commented on as work in progress, closed or merged.
    pub fn is_open(self) -> bool {
        matches!(self, Standing::Open | Standing::Draft)
    }
}

/// What a review said, besides its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Approved,
    ChangesRequested,
    Commented,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Approved => "Approved",
            Verdict::ChangesRequested => "Requested changes",
            Verdict::Commented => "Reviewed",
        }
    }

    /// The `event` GitHub's API takes for a review with this verdict.
    pub fn event(self) -> &'static str {
        match self {
            Verdict::Approved => "APPROVE",
            Verdict::ChangesRequested => "REQUEST_CHANGES",
            Verdict::Commented => "COMMENT",
        }
    }
}

/// One thing somebody wrote: the description, a comment, or a review.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub author: String,
    /// When it was written (Unix seconds).
    pub at: i64,
    /// Markdown; may be empty (an approval with no text).
    pub body: String,
    /// `Some` for a review.
    pub verdict: Option<Verdict>,
}

/// How one check on a pull request's last commit went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Passed,
    Failed,
    Running,
    /// Skipped, neutral or cancelled: neither passed nor failed.
    Other,
}

impl CheckState {
    pub fn label(self) -> &'static str {
        match self {
            CheckState::Passed => "Passed",
            CheckState::Failed => "Failed",
            CheckState::Running => "Running",
            CheckState::Other => "Skipped",
        }
    }
}

/// One check run (a CI job) on a pull request's last commit.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub url: String,
}

/// "2 passed, 1 failed": the checks in a few words.
pub fn checks_summary(list: &[Check]) -> String {
    if list.is_empty() {
        return "No checks on the last commit.".into();
    }
    let count = |st: CheckState| list.iter().filter(|c| c.state == st).count();
    let parts: Vec<String> = [(CheckState::Failed, "failed"), (CheckState::Running, "running"), (CheckState::Passed, "passed"), (CheckState::Other, "skipped")]
        .into_iter()
        .filter_map(|(st, w)| Some(count(st)).filter(|n| *n > 0).map(|n| format!("{n} {w}")))
        .collect();
    parts.join(", ")
}

/// A conversation as loaded from GitHub.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub title: String,
    pub author: String,
    pub standing: Standing,
    pub url: String,
    /// Label names and hues.
    pub labels: Vec<(String, f32)>,
    /// Who it's assigned to (logins).
    pub assignees: Vec<String>,
    /// Its milestone: number and title.
    pub milestone: Option<(u32, String)>,
    /// A pull request's reviewers who are asked and haven't answered yet (logins).
    pub requested: Vec<String>,
    /// A pull request's reviewers who answered, each with their latest verdict.
    pub reviewed: Vec<(String, Verdict)>,
    /// A pull request's branch and the one it goes into.
    pub branches: Option<(String, String)>,
    /// The description first, then comments and reviews, oldest first.
    pub entries: Vec<Entry>,
    /// GitHub has more comments than the ones loaded.
    pub more: bool,
    /// A pull request's checks on its last commit; `None` for an issue, or when the token
    /// can't read them.
    pub checks: Option<Vec<Check>>,
}

/// The conversation on screen: which one, what's loaded, and the reply being typed.
pub struct Open {
    pub repo: String,
    pub number: u32,
    pub pull: bool,
    pub load: Load<Thread>,
    pub reply: TextEdit,
    /// A comment or a review is being sent.
    pub sending: bool,
    /// Counts loads, so a late answer for an earlier one is dropped.
    pub epoch: u64,
    /// Triage: the open picker, the title or description being edited, GitHub's refusal.
    pub edit: crate::triage::Edits,
}

impl Open {
    pub fn is(&self, repo: &str, number: u32) -> bool {
        self.number == number && crate::model::same_repo(&self.repo, repo)
    }
}

/// A made-up conversation for the sample data, so the page can be tried without an account.
pub fn sample(title: &str, author: &str, pull: bool, standing: Standing, now: i64) -> Thread {
    let day = 86_400;
    let mut entries = vec![Entry {
        author: author.to_owned(),
        at: now - 3 * day,
        body: if pull {
            "## What changes\n- Loads the scene in chunks, so the first frame shows sooner.\n- Keeps the old loader behind a flag.\n\n## How to try it\n```sh\nnpm run dev\n```\nOpen the page and watch the network tab.".into()
        } else {
            "On a slow connection the page stays blank for a few seconds.\n\n**Expected:** a placeholder while it loads.\n\n**Steps:** throttle the network to *Slow 3G* and reload.".into()
        },
        verdict: None,
    }, Entry {
        author: "hubot".into(),
        at: now - 2 * day,
        body: "I can reproduce this. Does it also happen with the cache warm?".into(),
        verdict: None,
    }, Entry {
        author: author.to_owned(),
        at: now - day,
        body: "Only on the first visit.".into(),
        verdict: None,
    }];
    if pull {
        entries.push(Entry { author: "octocat".into(), at: now - 3600, body: "Looks good. One question about the flag's default, otherwise ready.".into(), verdict: Some(Verdict::Commented) });
    }
    Thread {
        title: title.to_owned(),
        author: author.to_owned(),
        standing,
        url: String::new(),
        labels: if pull { vec![("enhancement".into(), 200.0)] } else { vec![("bug".into(), 20.0)] },
        assignees: vec![author.to_owned()],
        milestone: Some((3, "v0.3 · Mixed reality".to_owned())),
        requested: if pull { vec!["hubot".to_owned()] } else { Vec::new() },
        reviewed: if pull { vec![("octocat".to_owned(), Verdict::Commented)] } else { Vec::new() },
        branches: pull.then(|| ("feature/chunked-loading".to_owned(), "main".to_owned())),
        entries,
        more: false,
        checks: pull.then(|| vec![
            Check { name: "build (ubuntu)".into(), state: CheckState::Passed, url: String::new() },
            Check { name: "build (macos)".into(), state: CheckState::Running, url: String::new() },
            Check { name: "test".into(), state: CheckState::Passed, url: String::new() },
        ]),
    }
}
