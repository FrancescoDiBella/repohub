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
        }
    }
}

impl Commits {
    fn next_epoch(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
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

// ------------------------------------------------------------------ sample data

/// A made-up commit id, the same every time for the same words.
fn sample_sha(seed: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut out = String::with_capacity(40);
    for round in 0..3u64 {
        for b in seed.bytes().chain(round.to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        out.push_str(&format!("{h:016x}"));
    }
    out.truncate(40);
    out
}

/// Who wrote a sample commit: you, Claude, a collaborator, someone with no GitHub account, a bot.
#[derive(Clone, Copy)]
enum Who {
    Me,
    Claude,
    Contributor,
    GitName,
    Bot,
}

/// The sample history of `name`, newest first: (message, who, body).
fn sample_messages(name: &str) -> &'static [(&'static str, Who, &'static str)] {
    use Who::*;
    match name {
        "francescodibella.github.io" => &[
            ("Add RepoHub, a Rune-based GitHub repository manager UI", Me, "RepoHub reads the repositories, pull requests and issues from the GitHub API\nand shows them in a native window. The page links to the desktop builds."),
            ("Bunny: keep the poncho outside the arms when they're lowered", Claude, "The poncho's lower edge followed the torso only, so the arms went through it\nwhen they hung down. It now follows the shoulders too.\n\nFixes #31"),
            ("Frame the camera on small screens", Me, ""),
            ("Load the bunny scene in chunks so the first frame shows sooner", Me, "The geometry loads first and the materials after it, so the page shows\nsomething within a second on a slow connection."),
            ("Shell: add the widgets shelf", Me, ""),
            ("Follow the system's dark mode", GitName, ""),
            ("Compress the hero images (420 KB less)", Me, ""),
            ("Fix the dock covering the footer on iPad", Me, ""),
            ("Open the terminal widget with Ctrl+`", Me, ""),
            ("Show system fonts until the web font has loaded", GitName, ""),
            ("Bump three from 0.164.1 to 0.170.0", Bot, "Bumps three from 0.164.1 to 0.170.0.\n\nRelease notes: https://github.com/mrdoob/three.js/releases"),
            ("Weather widget: cache the forecast for ten minutes", Me, ""),
            ("Add a 404 page that matches the shell", Me, ""),
            ("Fix the clock widget drifting after sleep", Me, ""),
            ("Bunny: softer ears, fewer polygons", Claude, ""),
            ("Move the shell's styles into css/shell.css", Me, ""),
            ("Add Open Graph tags for link previews", Me, ""),
            ("Keyboard focus rings on every dock icon", GitName, ""),
            ("Fix the window manager losing the top window on resize", Me, ""),
            ("Snap dragged windows to the screen edges", Me, ""),
            ("README: how to run the site locally", Me, ""),
            ("Add the notes widget, saved in localStorage", Me, ""),
            ("Lazy-load the project screenshots", Me, ""),
            ("Fix the bunny's shadow on Safari", Claude, ""),
            ("Add a reduced-motion mode for the boot animation", Me, ""),
            ("Projects page: cards with tags and links", Me, ""),
            ("Switch the build to plain ES modules", Me, ""),
            ("Remove jQuery", Me, ""),
            ("Add the boot animation", Me, ""),
            ("Shell UI: desktop, dock and windows", Me, ""),
            ("Add the procedural bunny (first version)", Me, ""),
            ("Deploy with GitHub Pages", Me, ""),
            ("About page and contact links", Me, ""),
            ("Initial commit", Me, ""),
        ],
        "ngsi-ld-xr-contexts" => &[
            ("Mixed reality context: anchors and spatial meshes", Me, "Adds SpatialAnchor and SpatialMesh, with examples for both."),
            ("Fix @vocab for VR entity relationships", Contributor, ""),
            ("Validate every example in CI", Me, ""),
            ("Add the AR marker entity", Me, ""),
            ("Document the VR scene graph", GitName, ""),
            ("Rename hasPose to pose (the old name stays as an alias)", Me, ""),
            ("Add JSON-LD examples for each entity", Me, ""),
            ("Split the context into ar, vr and mr files", Me, ""),
            ("Add the validate-jsonld workflow", Me, ""),
            ("Initial AR and VR contexts", Me, ""),
        ],
        "xr-digital-twin-thesis" => &[
            ("Chapter 4: latency measurements", Me, "Frame pacing and end-to-end latency over Wi-Fi 5, Wi-Fi 6 and Ethernet."),
            ("Plot frame pacing per network", Me, ""),
            ("Stream the cell's state over WebRTC data channels", Me, ""),
            ("Fix the robot arm's joint order", Me, ""),
            ("Chapter 3: architecture", Me, ""),
            ("Add the OPC UA bridge", Me, ""),
            ("Interpolate poses between updates", Me, ""),
            ("Bibliography: add the WebXR papers", Me, ""),
            ("Set up the LaTeX build", Me, ""),
            ("Initial project", Me, ""),
        ],
        "spatial-anchors" => &[
            ("Share anchors between sessions through the relay", Me, ""),
            ("Persist anchors in IndexedDB", Contributor, ""),
            ("Fix drift after the headset sleeps", Me, ""),
            ("Add the anchor inspector overlay", GitName, ""),
            ("Resolve anchors by id on join", Me, ""),
            ("Add tests for the relay protocol", Contributor, ""),
            ("Use WebXR's anchors module when available", Me, ""),
            ("Initial relay and client", Me, ""),
        ],
        "dotfiles" => &[
            ("zsh: faster prompt with async git status", Me, ""),
            ("neovim: switch to lazy.nvim", Me, ""),
            ("Add the bootstrap script for macOS", Me, ""),
            ("tmux: true colour and mouse support", Me, ""),
            ("git: sign commits with SSH keys", Me, ""),
            ("Add aliases for docker compose", Me, ""),
            ("Initial dotfiles", Me, ""),
        ],
        "rune" => &[
            ("Repo manager example on the web target", Me, ""),
            ("Merge upstream dev", Me, ""),
            ("Fix the example's scrolling on touch screens", Me, ""),
            ("Add a README for the example", Me, ""),
            ("Merge upstream dev", Me, ""),
        ],
        "lab-handbook" => &[
            ("Booking: the headsets are now in room 2.14", GitName, ""),
            ("Add the publishing checklist", Me, ""),
            ("Onboarding: accounts to ask for in the first week", Contributor, ""),
            ("Fix broken links to the wiki", Me, ""),
            ("Initial handbook", Me, ""),
        ],
        "iot-sensor-gateway" => &[
            ("Batch MQTT messages to save battery", Me, "The CO2 node now sends every 5 minutes instead of every 30 seconds.\n\nRefs #7"),
            ("Map CO2 readings to NGSI-LD AirQualityObserved", Me, ""),
            ("Reconnect to the broker with backoff", Me, ""),
            ("Add pytest for the mapper", Me, ""),
            ("Read the configuration from environment variables", Me, ""),
            ("Initial MQTT bridge", Me, ""),
        ],
        "university-notes" => &[
            ("Distributed systems: Raft and Paxos", Me, ""),
            ("Fix the search index for accented words", Me, ""),
            ("Add MathJax", Me, ""),
            ("Computer graphics: shading", Me, ""),
            ("Switch to mkdocs-material", Me, ""),
            ("Initial notes", Me, ""),
        ],
        _ => &[("Update the README", Me, ""), ("Fix a typo", Me, ""), ("Initial commit", Me, "")],
    }
}

/// A believable history of sample repository `r` at `branch`, newest first: the newest is
/// the last commit the Code tab shows.
pub fn sample_history(r: &Repo, branch: &str, me: &str) -> Vec<Row> {
    let mut at = r.pushed;
    sample_messages(&r.name).iter().enumerate().map(|(n, (msg, who, body))| {
        let sha = sample_sha(&format!("{}@{branch}#{n}", r.full_name));
        let (author, account) = match who {
            Who::Me => (me.to_owned(), true),
            Who::Claude => ("claude".to_owned(), true),
            Who::Contributor => ("contributor42".to_owned(), true),
            Who::GitName => ("Laura Bianchi".to_owned(), false),
            Who::Bot => ("dependabot[bot]".to_owned(), true),
        };
        let row = Row {
            title: (*msg).to_owned(),
            body: (*body).to_owned(),
            verified: matches!(who, Who::Me | Who::Bot) && n % 3 != 2,
            author,
            account,
            at,
            sha,
        };
        // A few hours to a few days between commits.
        at -= 3600 * (3 + (u64::from_str_radix(&row.sha[..4], 16).unwrap_or(0) % 40) as i64);
        row
    }).collect()
}

/// The changes of a sample commit.
fn sample_files(r: &Repo, title: &str) -> Vec<PullFile> {
    let readme = PullFile {
        path: "README.md".into(),
        previous: None,
        status: "modified".into(),
        patch: Some(format!("@@ -1,6 +1,8 @@\n # {}\n \n-{}\n+{}\n+\n+Latest change: {title}.\n \n ## Quick start\n ", r.name, r.description, r.description.trim_end_matches('.'))),
    };
    if r.name == "francescodibella.github.io" {
        let mut files = diffs::sample_pull_files();
        files.push(PullFile { path: "assets/bunny-preview.png".into(), previous: None, status: "added".into(), patch: None });
        files.push(readme);
        files
    } else {
        vec![readme]
    }
}

/// Lines added and removed by the files that have a patch (as GitHub counts them).
fn sample_totals(files: &[PullFile]) -> (usize, usize) {
    let with_patch: Vec<PullFile> = files.iter().filter(|f| f.patch.is_some()).cloned().collect();
    diffs::totals(&diffs::pull_files(&with_patch))
}

/// Sample commit `sha` of `r`: the row of its history, with made-up changes.
pub fn sample_detail(r: &Repo, sha: &str, me: &str) -> Option<Detail> {
    let rows = sample_history(r, &r.branch, me);
    let at = rows.iter().position(|x| x.sha.starts_with(sha) || sha.starts_with(x.short()))?;
    Some(sample_detail_of(r, &rows[at], rows.get(at + 1).map(|p| p.sha.clone())))
}

/// A sample commit's page from its row: made-up changes, and `parent`.
pub fn sample_detail_of(r: &Repo, row: &Row, parent: Option<String>) -> Detail {
    let files = sample_files(r, &row.title);
    let (add, del) = sample_totals(&files);
    let who = Person { name: row.author.clone(), login: row.account.then(|| row.author.clone()), at: row.at };
    Detail {
        sha: row.sha.clone(),
        title: row.title.clone(),
        body: row.body.clone(),
        committer: if row.author == "dependabot[bot]" { Person { name: "GitHub".into(), login: Some("web-flow".into()), at: row.at } } else { who.clone() },
        author: who,
        parents: parent.into_iter().collect(),
        verified: row.verified,
        files,
        more_files: false,
        additions: add as u32,
        deletions: del as u32,
        url: format!("{}/commit/{}", r.url, row.sha),
    }
}

/// A sample comparison of `head` with `base`.
pub fn sample_comparison(r: &Repo, base: &str, head: &str, me: &str, now: i64) -> Comparison {
    if base == head {
        return Comparison { url: format!("{}/compare/{base}...{head}", r.url), ..Comparison::unrelated(base, head) }.identical();
    }
    let words = head.rsplit('/').next().unwrap_or(head).replace(['-', '_'], " ");
    let titles = [format!("Start {words}"), format!("{}: handle the empty case", capitalize(&words)), format!("Tests for {words}")];
    // The first title is the oldest commit; the list shows the newest first.
    let commits: Vec<Row> = titles.iter().enumerate().rev().map(|(n, t)| Row {
        sha: sample_sha(&format!("{}:{head}:{n}", r.full_name)),
        title: t.clone(),
        body: String::new(),
        author: me.to_owned(),
        account: true,
        at: now - 3600 * (2 + 20 * (titles.len() - 1 - n) as i64),
        verified: n != 1,
    }).collect();
    let files = sample_files(r, &titles[0]);
    let (additions, deletions) = sample_totals(&files);
    Comparison {
        base: base.to_owned(),
        head: head.to_owned(),
        standing: Standing::Diverged,
        ahead: commits.len() as u32,
        behind: 1,
        commits,
        more_commits: false,
        files,
        more_files: false,
        additions: additions as u32,
        deletions: deletions as u32,
        url: format!("{}/compare/{base}...{head}", r.url),
        open_pull: None,
    }
}

impl Comparison {
    fn identical(mut self) -> Comparison {
        self.standing = Standing::Identical;
        self
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

// ------------------------------------------------------------------ loading

/// Runs `work` in the background and hands its answer to `done`. Before the window exists, a
/// headless still (`REPOHUB_BLOCKING=1`) waits for it instead. False when neither can run.
fn run<R, Fut>(s: &mut App, work: impl FnOnce() -> Fut + Send + 'static, done: impl FnOnce(&mut App, R) + Send + 'static) -> bool
where
    R: Send + 'static,
    Fut: std::future::Future<Output = R> + 'static,
{
    if let Some(h) = s.handle.clone() {
        net::spawn(h, work, done);
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1") {
        let r = pollster::block_on(work());
        done(s, r);
        return true;
    }
    false
}

const NOT_READY: &str = "The app isn't ready to load this yet: try again in a moment.";

impl App {
    /// Reads the history of the branch on screen of repository `i` (`again`: even when it's
    /// already here). Another branch's history waiting for its answer is forgotten.
    pub(crate) fn load_history(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, branch) = (r.full_name.clone(), self.branch(i));
        let same = self.commits.history_of(&repo, &branch);
        if same.is_some_and(|h| !again && !matches!(h.load, Load::Failed(_))) {
            return;
        }
        // A reload keeps the list on screen until the answer.
        let kept = if same.is_some() { self.commits.history.take().and_then(|h| match h.load { Load::Ready(rows) => Some((rows, h.page, h.more)), _ => None }) } else { None };
        let epoch = self.commits.next_epoch();
        state_trace(&repo, &branch);
        if self.data.sample {
            let all = sample_history(r, &branch, &self.data.profile.login);
            let more = all.len() > PAGE;
            self.commits.history = Some(History { repo, branch, load: Load::Ready(all.into_iter().take(PAGE).collect()), page: 1, more, loading_more: false, epoch });
            return;
        }
        let (load, page, more) = match kept {
            Some((rows, page, more)) => (Load::Ready(rows), page, more),
            None => (Load::Loading, 1, false),
        };
        self.commits.history = Some(History { repo: repo.clone(), branch: branch.clone(), load, page, more, loading_more: false, epoch });
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commits(&cfg, &repo, &branch, 1).await }, move |s: &mut App, r| s.apply_history(account, epoch, 1, r)) {
            self.apply_history(account, epoch, 1, Err(NOT_READY.into()));
        }
    }

    /// The next page of the history on screen.
    pub(crate) fn load_more_commits(&mut self) {
        let sample = self.data.sample;
        let Some(h) = self.commits.history.as_mut() else { return };
        if h.loading_more || !h.more || !matches!(h.load, Load::Ready(_)) {
            return;
        }
        let (repo, branch, epoch, page) = (h.repo.clone(), h.branch.clone(), h.epoch, h.page + 1);
        if sample {
            let Some(r) = self.data.repos.iter().find(|r| r.full_name == repo) else { return };
            let all = sample_history(r, &branch, &self.data.profile.login);
            let more = all.len() > page * PAGE;
            let rows = all.into_iter().skip((page - 1) * PAGE).take(PAGE).collect();
            self.apply_history(self.load_gen, epoch, page, Ok((rows, more)));
            return;
        }
        h.loading_more = true;
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commits(&cfg, &repo, &branch, page).await }, move |s: &mut App, r| s.apply_history(account, epoch, page, r)) {
            self.apply_history(account, epoch, page, Err(NOT_READY.into()));
        }
    }

    /// GitHub's answer for page `page` of the history asked with `epoch`.
    pub(crate) fn apply_history(&mut self, account: u64, epoch: u64, page: usize, r: Result<(Vec<Row>, bool), String>) {
        // Another account, another branch, or a reload since: not for this list.
        if self.load_gen != account {
            return;
        }
        let Some(h) = self.commits.history.as_mut().filter(|h| h.epoch == epoch) else { return };
        if page > 1 {
            h.loading_more = false;
        }
        match r {
            Ok((rows, more)) => match &mut h.load {
                // New commits pushed meanwhile shift the pages: a commit already listed isn't listed twice.
                Load::Ready(list) if page > 1 => {
                    list.extend(rows.into_iter().filter(|x| !list.iter().any(|y| y.sha == x.sha)).collect::<Vec<_>>());
                    h.page = page;
                    h.more = more;
                }
                // A reload (a refresh) whose first page ends inside the list: the commits pushed
                // since go on top, and the pages already read stay.
                Load::Ready(list) if rows.last().is_some_and(|last| list.iter().any(|y| y.sha == last.sha)) => {
                    let fresh: Vec<Row> = rows.into_iter().take_while(|x| !list.iter().any(|y| y.sha == x.sha)).collect();
                    list.splice(0..0, fresh);
                }
                // The first page, or a history rewritten under the list (a force push).
                load => {
                    *load = Load::Ready(rows);
                    h.page = page;
                    h.more = more;
                }
            },
            Err(e) if page > 1 => self.notify_problem(format!("More commits couldn't load: {e}")),
            // A reload that fails keeps what was read before, and says so.
            Err(e) if matches!(h.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => h.load = Load::Failed(e),
        }
    }

    /// Opens commit `sha` of `repo` in place of the list. `from_row`: the row it was opened
    /// from, shown again on Back.
    pub(crate) fn open_commit(&mut self, repo: &str, sha: &str, from_row: Option<String>) {
        let epoch = self.commits.next_epoch();
        state_trace(repo, sha);
        self.commits.open = Some(Open { repo: repo.to_owned(), sha: sha.to_owned(), load: Load::Loading, epoch, trail: Vec::new(), from_row });
        self.commits.commit_diff.set_files(Vec::new());
        self.load_commit();
    }

    /// Opens a parent of the commit on screen; Back comes back to this one.
    pub(crate) fn open_parent(&mut self, sha: &str) {
        let epoch = self.commits.next_epoch();
        let Some(o) = self.commits.open.as_mut() else { return };
        let child = std::mem::replace(&mut o.sha, sha.to_owned());
        o.trail.push(child);
        o.epoch = epoch;
        o.load = Load::Loading;
        self.commits.commit_diff.set_files(Vec::new());
        self.load_commit();
    }

    /// Reads the commit on screen (again).
    pub(crate) fn load_commit(&mut self) {
        let sample = self.data.sample;
        let Some(o) = self.commits.open.as_mut() else { return };
        let (repo, sha, epoch) = (o.repo.clone(), o.sha.clone(), o.epoch);
        if sample {
            // A commit of the history, or of the comparison on screen.
            let compared = match self.commits.compare.as_ref().and_then(|c| c.result.as_ref()) {
                Some(Load::Ready(cmp)) => cmp.commits.iter().position(|x| x.sha == sha).map(|at| (cmp.commits[at].clone(), cmp.commits.get(at + 1).map(|p| p.sha.clone()))),
                _ => None,
            };
            let found = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).and_then(|r| match &compared {
                Some((row, parent)) => Some(sample_detail_of(r, row, parent.clone())),
                None => sample_detail(r, &sha, &self.data.profile.login),
            });
            let r = found.ok_or_else(|| format!("There's no commit {} in the sample data.", short(&sha)));
            self.apply_commit(self.load_gen, epoch, r);
            return;
        }
        if !matches!(o.load, Load::Ready(_)) {
            o.load = Load::Loading;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commit(&cfg, &repo, &sha).await }, move |s: &mut App, r| s.apply_commit(account, epoch, r)) {
            self.apply_commit(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_commit(&mut self, account: u64, epoch: u64, r: Result<Detail, String>) {
        if self.load_gen != account {
            return;
        }
        let c = &mut self.commits;
        let Some(o) = c.open.as_mut().filter(|o| o.epoch == epoch) else { return };
        match r {
            Ok(d) => {
                c.commit_diff.set_files(diffs::pull_files(&d.files));
                o.load = Load::Ready(d);
            }
            Err(e) if matches!(o.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => o.load = Load::Failed(e),
        }
    }

    /// Back from the commit on screen: to the commit whose parent link opened it, else to the
    /// list, with the row it was opened from in view (`tx`, when there is one).
    pub(crate) fn close_commit(&mut self, tx: Option<&mut Tx>) {
        let epoch = self.commits.next_epoch();
        let Some(o) = self.commits.open.as_mut() else { return };
        if let Some(child) = o.trail.pop() {
            o.sha = child;
            o.epoch = epoch;
            o.load = Load::Loading;
            self.commits.commit_diff.set_files(Vec::new());
            self.load_commit();
            return;
        }
        let row = o.from_row.clone();
        self.commits.open = None;
        self.commits.commit_diff.set_files(Vec::new());
        if let (Some(tx), Some(row)) = (tx, row) {
            tx.scroll_key_to(row, 160.0);
        }
    }

    /// Opens the comparison form of repository `i`: the default branch, and the branch on
    /// screen (the first other branch when that's the default one).
    pub(crate) fn open_compare(&mut self, i: usize) {
        self.load_branches(i, false);
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, base, shown) = (r.full_name.clone(), r.branch.clone(), self.branch(i));
        let head = if shown != base {
            shown
        } else {
            self.branches.get(&repo).and_then(Load::ready).and_then(|b| b.iter().find(|n| **n != base).cloned()).unwrap_or(shown)
        };
        self.commits.open = None;
        self.commits.compare_diff.set_files(Vec::new());
        self.commits.compare = Some(Compare { repo, base, head, result: None, epoch: 0, asked_pull: false });
        self.picker = None;
    }

    pub(crate) fn close_compare(&mut self) {
        self.commits.compare = None;
        self.commits.compare_diff.set_files(Vec::new());
        if matches!(self.picker, Some(BranchSlot::CompareBase | BranchSlot::CompareHead)) {
            self.picker = None;
        }
    }

    /// The branch picker chose `branch` for the comparison; a result on screen follows it.
    pub(crate) fn pick_compare(&mut self, slot: BranchSlot, branch: String) {
        let Some(c) = self.commits.compare.as_mut() else { return };
        match slot {
            BranchSlot::CompareBase => c.base = branch,
            BranchSlot::CompareHead => c.head = branch,
            _ => return,
        }
        if c.result.is_some() {
            self.run_compare();
        }
    }

    /// Compares the two branches of the form.
    pub(crate) fn run_compare(&mut self) {
        let epoch = self.commits.next_epoch();
        let sample = self.data.sample;
        let Some(c) = self.commits.compare.as_mut() else { return };
        self.commits.compare_diff.set_files(Vec::new());
        // The same branch twice: the form says so, nothing is asked.
        if c.base == c.head {
            c.result = None;
            return;
        }
        c.epoch = epoch;
        c.asked_pull = false;
        c.result = Some(Load::Loading);
        let (repo, base, head) = (c.repo.clone(), c.base.clone(), c.head.clone());
        if sample {
            let found = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).map(|r| sample_comparison(r, &base, &head, &self.data.profile.login, self.now));
            self.apply_compare(self.load_gen, epoch, found.ok_or_else(|| "This repository isn't in the sample data.".to_owned()));
            return;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_compare(&cfg, &repo, &base, &head).await }, move |s: &mut App, r| s.apply_compare(account, epoch, r)) {
            self.apply_compare(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_compare(&mut self, account: u64, epoch: u64, r: Result<Comparison, String>) {
        if self.load_gen != account {
            return;
        }
        let c = &mut self.commits;
        let Some(cmp) = c.compare.as_mut().filter(|x| x.epoch == epoch && x.result.is_some()) else { return };
        if let Ok(result) = &r {
            c.compare_diff.set_files(diffs::pull_files(&result.files));
        }
        cmp.result = Some(r.into());
    }

    /// The New pull request form, from the comparison on screen: its branches filled in, and
    /// the title and description of its commit when there's only one.
    pub(crate) fn pull_from_compare(&mut self) {
        let Some(c) = self.commits.compare.as_mut() else { return };
        let Some(Load::Ready(cmp)) = c.result.as_ref() else { return };
        let (title, body) = match cmp.commits.as_slice() {
            [only] if cmp.ahead == 1 => (only.title.clone(), only.body.clone()),
            _ => (String::new(), String::new()),
        };
        c.asked_pull = true;
        let mut title = TextEdit::new(title);
        title.focused = true;
        self.form = Some(actions::Form {
            repo: c.repo.clone(),
            kind: Compose::Pull,
            title,
            body: TextEdit::new(body),
            from: cmp.head.clone(),
            into: cmp.base.clone(),
            draft: false,
            submit: Submit::Editing,
        });
        self.picker = None;
    }

    /// For headless stills (`-- commit <id>`, `-- compare <base>...<head>`): the Commits
    /// section of the repository on screen, with that commit open or those branches compared.
    pub(crate) fn commits_still(&mut self, commit: Option<String>, compare: Option<String>) {
        let Some(i) = self.repo_index() else { return };
        if commit.is_none() && compare.is_none() && self.tab != crate::state::RepoTab::Commits {
            return;
        }
        self.tab = crate::state::RepoTab::Commits;
        self.load_history(i, false);
        if let Some((base, head)) = compare.as_deref().and_then(|c| c.split_once("...")) {
            self.open_compare(i);
            if let Some(c) = self.commits.compare.as_mut() {
                (c.base, c.head) = (base.to_owned(), head.to_owned());
            }
            self.run_compare();
        }
        if let Some(sha) = commit {
            let full = self.data.repos[i].full_name.clone();
            // `-- commit 2` opens the second commit of the history.
            let sha = match (sha.parse::<usize>(), self.commits.history.as_ref().and_then(|h| h.load.ready())) {
                (Ok(n), Some(rows)) if sha.len() < 7 => rows.get(n.saturating_sub(1)).map(|r| r.sha.clone()).unwrap_or(sha),
                _ => sha,
            };
            self.open_commit(&full, &sha, None);
        }
    }
}

fn state_trace(repo: &str, what: &str) {
    crate::state::trace("commits", &repo, &what);
}

#[cfg(test)]
mod tests {
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
}
