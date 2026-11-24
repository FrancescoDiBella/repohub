//! The Commits section: a branch's history, one commit with its changes, and two branches
//! compared.
//!
//! GitHub answers `/repos/{r}/commits?sha={branch}` (the history, [`PAGE`] at a time, newest
//! first), `/repos/{r}/commits/{sha}` (one commit with its files) and
//! `/repos/{r}/compare/{base}...{head}`. The requests are `Gh::commits`, `Gh::commit` and
//! `Gh::compare` in `github`; this file reads their answers, keeps what's on screen
//! ([`Commits`]) and loads it (the `App` methods at the bottom). The page is `commits_page`.
//!
//! What's on screen is a small stack: the history, the comparison over it, and one commit over
//! either. **Back** (Esc, ⌘[) takes the top one away. Every request carries an epoch, so an
//! answer for a branch, commit or comparison that's no longer on screen is dropped.
use rune::{LocalTime, TextEdit, Tx};
use rune_kit::diff::DiffState;
use serde_json::Value;

use crate::github::PullFile;
use crate::model::{Repo, parse_iso, same_repo};
use crate::state::{BranchSlot, Compose, Load, Submit};
use crate::{App, actions, diffs, github, net};

mod loading;
mod sample;

pub use sample::{sample_comparison, sample_detail, sample_detail_of, sample_history};

/// Commits read at a time.
pub const PAGE: usize = 30;

/// GitHub lists at most this many files of a commit or a comparison.
pub const MAX_FILES: usize = 300;

/// One commit in a list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The full commit id.
    pub sha: String,
    /// The message's first line, and the rest.
    pub title: String,
    pub body: String,
    /// The author's GitHub login, or their git name when GitHub knows no account for them.
    pub author: String,
    /// `author` is a GitHub account.
    pub account: bool,
    /// When it was committed (Unix seconds).
    pub at: i64,
    /// GitHub verified its signature.
    pub verified: bool,
}

impl Row {
    pub fn short(&self) -> &str {
        short(&self.sha)
    }
}

/// The 7-character id people read and type.
pub fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// A commit id GitHub's paths take: 7 to 64 hexadecimal digits.
pub fn valid_sha(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Who wrote or committed a commit, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    /// The name in git.
    pub name: String,
    /// The GitHub account, when GitHub knows one for the email.
    pub login: Option<String>,
    pub at: i64,
}

impl Person {
    /// The login when there is one, else the git name.
    pub fn shown(&self) -> &str {
        match &self.login {
            Some(l) => l,
            None if self.name.is_empty() => "unknown",
            None => &self.name,
        }
    }
}

/// One commit, as its page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub sha: String,
    pub title: String,
    /// The rest of the message, line breaks kept.
    pub body: String,
    pub author: Person,
    pub committer: Person,
    pub parents: Vec<String>,
    pub verified: bool,
    pub files: Vec<PullFile>,
    /// GitHub has more files than it listed ([`MAX_FILES`]).
    pub more_files: bool,
    pub additions: u32,
    pub deletions: u32,
    pub url: String,
}

/// How a branch stands against the one it's compared with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Both point at the same commit.
    Identical,
    /// It has commits the base doesn't.
    Ahead,
    /// The base has commits it doesn't, and it has none of its own.
    Behind,
    /// Both have commits of their own.
    Diverged,
    /// They share no history.
    Unrelated,
}

/// Branch `head` compared with branch `base`.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub base: String,
    pub head: String,
    pub standing: Standing,
    pub ahead: u32,
    pub behind: u32,
    /// The commits on `head` that `base` doesn't have, newest first.
    pub commits: Vec<Row>,
    /// GitHub has more commits than it listed (it lists 250).
    pub more_commits: bool,
    /// What those commits change, together.
    pub files: Vec<PullFile>,
    pub more_files: bool,
    pub additions: u32,
    pub deletions: u32,
    pub url: String,
    /// The open pull request from `head` into `base` (number and page), when there is one.
    pub open_pull: Option<(u32, String)>,
}

fn plural(n: u32, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

impl Comparison {
    /// Two branches with no history in common: GitHub has nothing to compare.
    pub fn unrelated(base: &str, head: &str) -> Comparison {
        Comparison {
            base: base.to_owned(),
            head: head.to_owned(),
            standing: Standing::Unrelated,
            ahead: 0,
            behind: 0,
            commits: Vec::new(),
            more_commits: false,
            files: Vec::new(),
            more_files: false,
            additions: 0,
            deletions: 0,
            url: String::new(),
            open_pull: None,
        }
    }

    /// "3 commits ahead, 1 behind main", or why there's nothing to compare.
    pub fn summary(&self) -> String {
        let (head, base) = (&self.head, &self.base);
        match self.standing {
            Standing::Identical => format!("{head} and {base} are identical: there's nothing to compare."),
            Standing::Unrelated => format!("{head} and {base} have no history in common, so there's nothing to compare."),
            _ if self.ahead > 0 && self.behind > 0 => format!("{} ahead, {} behind {base}", plural(self.ahead, "commit"), self.behind),
            _ if self.ahead > 0 => format!("{} ahead of {base}", plural(self.ahead, "commit")),
            _ if self.behind > 0 => format!("{head} has nothing that {base} doesn't have: it's {} behind.", plural(self.behind, "commit")),
            _ => format!("{head} and {base} are identical: there's nothing to compare."),
        }
    }

    /// Nothing on `head` to show: no commits of its own, so no changes either.
    pub fn nothing(&self) -> bool {
        self.ahead == 0 || matches!(self.standing, Standing::Identical | Standing::Unrelated)
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn n(v: &Value, k: &str) -> u32 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32
}

/// A commit message's first line, and the rest without the blank line between them.
pub fn split_message(message: &str) -> (String, String) {
    let m = message.replace("\r\n", "\n");
    let (title, body) = m.split_once('\n').unwrap_or((&m, ""));
    let title = title.trim();
    (if title.is_empty() { "(no message)".to_owned() } else { title.to_owned() }, body.trim_start_matches('\n').trim_end().to_owned())
}

/// The author (`key` "author") or committer of a commit: the name and date in git
/// (`commit.author`), the account GitHub matched (`author`, null when none).
fn person(v: &Value, key: &str) -> Person {
    let git = v.get("commit").and_then(|c| c.get(key)).unwrap_or(&Value::Null);
    let login = v.get(key).filter(|a| !a.is_null()).map(|a| s(a, "login")).filter(|l| !l.is_empty());
    Person { name: s(git, "name"), login, at: parse_iso(&s(git, "date")).unwrap_or(0) }
}

fn verified(v: &Value) -> bool {
    v.get("commit").and_then(|c| c.get("verification")).and_then(|x| x.get("verified")).and_then(Value::as_bool) == Some(true)
}

/// One commit of a list (`/commits`, a comparison's `commits`); `None` without a valid id.
pub fn parse_row(v: &Value) -> Option<Row> {
    let sha = s(v, "sha");
    if !valid_sha(&sha) {
        return None;
    }
    let (author, committer) = (person(v, "author"), person(v, "committer"));
    let (title, body) = split_message(&v.get("commit").map(|c| s(c, "message")).unwrap_or_default());
    Some(Row {
        title,
        body,
        author: author.shown().to_owned(),
        account: author.login.is_some(),
        // The committer's date keeps a rebased branch in order; the author's when it's missing.
        at: if committer.at > 0 { committer.at } else { author.at },
        verified: verified(v),
        sha,
    })
}

pub fn parse_rows(v: &Value) -> Vec<Row> {
    v.as_array().into_iter().flatten().filter_map(parse_row).collect()
}

/// The files of a commit or a comparison, as a pull request's are read.
fn parse_files(v: Option<&Value>) -> (Vec<PullFile>, u32, u32) {
    let list: Vec<&Value> = v.and_then(Value::as_array).into_iter().flatten().collect();
    let (add, del) = list.iter().fold((0, 0), |(a, d), f| (a + n(f, "additions"), d + n(f, "deletions")));
    let files = list.iter().map(|f| {
        let get = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_owned);
        PullFile { path: get("filename").unwrap_or_default(), previous: get("previous_filename"), status: get("status").unwrap_or_default(), patch: get("patch") }
    }).collect();
    (files, add, del)
}

/// One commit with its files. `more_files`: GitHub said it has another page of them.
pub fn parse_detail(v: &Value, more_files: bool) -> Detail {
    let (title, body) = split_message(&v.get("commit").map(|c| s(c, "message")).unwrap_or_default());
    let (files, add, del) = parse_files(v.get("files"));
    let stats = v.get("stats");
    Detail {
        sha: s(v, "sha"),
        title,
        body,
        author: person(v, "author"),
        committer: person(v, "committer"),
        parents: v.get("parents").and_then(Value::as_array).into_iter().flatten().map(|p| s(p, "sha")).filter(|p| valid_sha(p)).collect(),
        verified: verified(v),
        more_files: more_files || files.len() >= MAX_FILES,
        // The commit's own totals count every file, also those past the first 300.
        additions: stats.map_or(add, |x| n(x, "additions")),
        deletions: stats.map_or(del, |x| n(x, "deletions")),
        files,
        url: s(v, "html_url"),
    }
}

/// GitHub's answer for `compare/{base}...{head}`.
pub fn parse_comparison(v: &Value, base: &str, head: &str) -> Comparison {
    let (ahead, behind) = (n(v, "ahead_by"), n(v, "behind_by"));
    let standing = match s(v, "status").as_str() {
        "identical" => Standing::Identical,
        "ahead" => Standing::Ahead,
        "behind" => Standing::Behind,
        "diverged" => Standing::Diverged,
        _ => match (ahead > 0, behind > 0) {
            (true, true) => Standing::Diverged,
            (true, false) => Standing::Ahead,
            (false, true) => Standing::Behind,
            (false, false) => Standing::Identical,
        },
    };
    // GitHub lists them oldest first; the history shows the newest first.
    let mut commits = parse_rows(v.get("commits").unwrap_or(&Value::Null));
    commits.reverse();
    let total = (n(v, "total_commits") as usize).max(ahead as usize);
    let (files, additions, deletions) = parse_files(v.get("files"));
    Comparison {
        base: base.to_owned(),
        head: head.to_owned(),
        standing,
        ahead,
        behind,
        more_commits: total > commits.len(),
        commits,
        more_files: files.len() >= MAX_FILES,
        files,
        additions,
        deletions,
        url: s(v, "html_url"),
        open_pull: None,
    }
}

/// The number of a date's day since 1970-01-01, to tell how many days apart two dates are.
fn day_number(t: &LocalTime) -> i64 {
    // Days from civil (Howard Hinnant).
    let (y, m, d) = (i64::from(t.year), i64::from(t.month), i64::from(t.day));
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The heading of the day `at` falls on: "Today", "Yesterday", "12 November", or
/// "12 November 2025" in another year. `local` turns Unix seconds into the user's clock.
pub fn day_label(at: i64, now: i64, local: &impl Fn(i64) -> LocalTime) -> String {
    let (day, today) = (local(at), local(now));
    match day_number(&today) - day_number(&day) {
        // A commit dated in the future (a clock that's ahead) is today's.
        ..=0 => "Today".into(),
        1 => "Yesterday".into(),
        _ if day.year == today.year => format!("{} {}", day.day, MONTHS[(day.month as usize).clamp(1, 12) - 1]),
        _ => format!("{} {} {}", day.day, MONTHS[(day.month as usize).clamp(1, 12) - 1], day.year),
    }
}

/// How long ago `at` was, in the day headings' terms: minutes and hours on the same day, then
/// days on the calendar ("yesterday" is the day before today, not 24 to 48 hours ago).
pub fn ago(at: i64, now: i64, local: &impl Fn(i64) -> LocalTime) -> String {
    match day_number(&local(now)) - day_number(&local(at)) {
        ..=0 => crate::model::ago(at, now),
        1 => "yesterday".into(),
        days @ 2..=6 => format!("{days} days ago"),
        _ => crate::model::ago(at, now),
    }
}

/// `rows` (newest first) in groups of one day each, with the day's heading.
pub fn days<'a>(rows: &[&'a Row], now: i64, local: impl Fn(i64) -> LocalTime) -> Vec<(String, Vec<&'a Row>)> {
    let mut out: Vec<(String, Vec<&'a Row>)> = Vec::new();
    for r in rows {
        let label = day_label(r.at, now, &local);
        match out.last_mut() {
            Some((l, list)) if *l == label => list.push(r),
            _ => out.push((label, vec![r])),
        }
    }
    out
}

/// The user's clock: their time zone, as the computer or the browser has it.
pub fn local_clock(t: i64) -> LocalTime {
    LocalTime::at(t.saturating_mul(1000))
}

/// "22 November 2026, 14:05" on the user's clock.
pub fn date_time(at: i64) -> String {
    let t = local_clock(at);
    format!("{} {} {}, {:02}:{:02}", t.day, MONTHS[(t.month as usize).clamp(1, 12) - 1], t.year, t.hour, t.minute)
}

/// Whether row `r` matches what's typed in the filter: every word is in its message or its
/// author, or starts its commit id.
pub fn matches(r: &Row, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let (title, body, author) = (r.title.to_lowercase(), r.body.to_lowercase(), r.author.to_lowercase());
    q.split_whitespace().all(|w| title.contains(w) || body.contains(w) || author.contains(w) || r.sha.to_lowercase().starts_with(w))
}

// ------------------------------------------------------------------ what's on screen

/// A branch's history as far as it's loaded.
pub struct History {
    pub repo: String,
    pub branch: String,
    pub load: Load<Vec<Row>>,
    /// Pages read so far.
    pub page: usize,
    /// GitHub has more.
    pub more: bool,
    /// The next page is being read.
    pub loading_more: bool,
    /// The request this history waits for (see [`Commits::epoch`]).
    pub epoch: u64,
}

/// The commit on screen, in place of the list.
pub struct Open {
    pub repo: String,
    pub sha: String,
    pub load: Load<Detail>,
    pub epoch: u64,
    /// The commits this one was reached from by their parent links: Back goes to them first.
    pub trail: Vec<String>,
    /// The list row it was opened from, put back in view on Back.
    pub from_row: Option<String>,
}

/// The comparison form and its result.
pub struct Compare {
    pub repo: String,
    /// The branch the other is compared with (a pull request's base), and the branch compared.
    pub base: String,
    pub head: String,
    /// `None` until Compare is pressed.
    pub result: Option<Load<Comparison>>,
    pub epoch: u64,
    /// The New pull request form was opened from this result.
    pub asked_pull: bool,
}

/// Immutable commit details kept in memory for instant Back / parent navigation.
const COMMITS_KEPT: usize = 32;
/// Maximum total bytes of commit messages and patches kept in memory.
const COMMITS_MAX_BYTES: usize = 8 << 20;

fn detail_bytes(d: &Detail) -> usize {
    d.sha.len()
        + d.title.len()
        + d.body.len()
        + d.url.len()
        + d.files
            .iter()
            .map(|f| f.path.len() + f.previous.as_ref().map_or(0, String::len) + f.patch.as_ref().map_or(0, String::len))
            .sum::<usize>()
}

/// The Commits section's state. It follows the repository and the branch on screen.
pub struct Commits {
    pub history: Option<History>,
    /// What the history is narrowed to.
    pub filter: TextEdit,
    /// Counts requests; each part keeps the one it waits for, and drops other answers.
    pub epoch: u64,
    pub open: Option<Open>,
    pub compare: Option<Compare>,
    /// The open commit's diff and the comparison's: their own, apart from the Changes tab's
    /// and the pull request's (`App::diff`, `App::pull_diff`).
    pub commit_diff: DiffState,
    pub compare_diff: DiffState,
    /// Recently read full-SHA commit details, most recent last (commits are immutable).
    cached: Vec<((String, String), Detail, usize)>,
    cached_bytes: usize,
}

impl Default for Commits {
    fn default() -> Commits {
        Commits {
            history: None,
            filter: TextEdit::new(""),
            epoch: 0,
            open: None,
            compare: None,
            commit_diff: DiffState::new(Vec::new()),
            compare_diff: DiffState::new(Vec::new()),
            cached: Vec::new(),
            cached_bytes: 0,
        }
    }
}

impl Commits {
    fn next_epoch(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    /// A remembered full-SHA commit detail, promoted to most-recently-used.
    pub(crate) fn cached_commit(&mut self, repo: &str, sha: &str) -> Option<Detail> {
        if sha.len() < 40 || !valid_sha(sha) {
            return None;
        }
        let pos = self.cached.iter().position(|((r, s), _, _)| same_repo(r, repo) && s.eq_ignore_ascii_case(sha))?;
        let entry = self.cached.remove(pos);
        let detail = entry.1.clone();
        self.cached.push(entry);
        Some(detail)
    }

    /// Remembers a full-SHA commit detail (evicting the least recently used beyond `COMMITS_KEPT`
    /// or `COMMITS_MAX_BYTES`).
    pub(crate) fn remember_commit(&mut self, repo: &str, detail: &Detail) {
        if detail.sha.len() < 40 || !valid_sha(&detail.sha) {
            return;
        }
        let bytes = detail_bytes(detail);
        if bytes > COMMITS_MAX_BYTES / 2 {
            return;
        }
        let key = (repo.to_ascii_lowercase(), detail.sha.to_ascii_lowercase());
        if let Some(pos) = self.cached.iter().position(|(k, _, _)| *k == key) {
            let (_, _, old_bytes) = self.cached.remove(pos);
            self.cached_bytes -= old_bytes;
        }
        self.cached.push((key, detail.clone(), bytes));
        self.cached_bytes += bytes;
        while self.cached.len() > COMMITS_KEPT || self.cached_bytes > COMMITS_MAX_BYTES {
            let (_, _, evicted) = self.cached.remove(0);
            self.cached_bytes -= evicted;
        }
    }

    /// The section is left: the commit and the comparison close; the history stays for next time.
    pub fn leave(&mut self) {
        self.open = None;
        self.compare = None;
        self.commit_diff.set_files(Vec::new());
        self.compare_diff.set_files(Vec::new());
    }

    /// A pull request was opened in `repo`: the comparison it was asked from shows it.
    pub fn pull_opened(&mut self, repo: &str, number: u32, url: &str) {
        if let Some(c) = self.compare.as_mut().filter(|c| c.asked_pull && same_repo(&c.repo, repo)) {
            c.asked_pull = false;
            if let Some(Load::Ready(cmp)) = c.result.as_mut() {
                cmp.open_pull = Some((number, url.to_owned()));
            }
        }
    }

    /// The branches of `repo` arrived: a comparison opened before them, still between the
    /// default branch and itself, compares the first other branch.
    pub fn branches_arrived(&mut self, repo: &str, names: &[String]) {
        if let Some(c) = self.compare.as_mut().filter(|c| c.repo == repo && c.base == c.head && c.result.is_none())
            && let Some(other) = names.iter().find(|n| **n != c.base)
        {
            c.head = other.clone();
        }
    }

    /// The history on screen for `repo` at `branch`, if that's what's loaded.
    pub fn history_of(&self, repo: &str, branch: &str) -> Option<&History> {
        self.history.as_ref().filter(|h| h.repo == repo && h.branch == branch)
    }
}

#[cfg(test)]
mod tests;
