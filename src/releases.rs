//! The Releases section: a repository's releases and the tags that have none, and the form that
//! drafts, edits and publishes a release.
//!
//! GitHub answers `/repos/{r}/releases?per_page=20&page={n}` (newest first), `/repos/{r}/tags`
//! and `/repos/{r}/releases/latest`; writes are `POST /releases`, `PATCH /releases/{id}` and
//! `POST /releases/generate-notes`. The requests are on `Gh` in `github`; this file reads their
//! answers, keeps what's on screen ([`Releases`]), works out the next versions and checks a tag,
//! and loads and sends (the `App` methods at the bottom). The page is `releases_page`.
//!
//! RepoHub doesn't delete releases or tags and doesn't upload files: those stay on GitHub.
//! Publishing always asks first ([`Form::confirm`], [`Releases::confirm`]); saving a draft
//! doesn't.
use std::collections::{HashMap, HashSet};

use rune::TextEdit;
use serde_json::Value;

use crate::github::{self, NewRelease, Write};
use crate::model::{Repo, parse_iso, same_repo};
use crate::state::{BranchSlot, Load, RepoTab, Submit};
use crate::{App, actions, net};

/// Releases read at a time.
pub const PAGE: usize = 20;

/// Tags read: the newest this many.
pub const TAGS: usize = 50;

/// Notes longer than this many lines are cut, with **Show all**.
pub const NOTES_LINES: usize = 12;

/// One file of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    /// Bytes.
    pub size: u64,
    pub downloads: u64,
    /// Where the browser downloads it.
    pub url: String,
}

/// One release, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// GitHub's number for it (what `PATCH /releases/{id}` takes).
    pub id: u64,
    pub tag: String,
    /// Its title; empty when it has none (the tag stands in, see [`Release::title`]).
    pub name: String,
    /// The notes, in Markdown.
    pub body: String,
    pub draft: bool,
    pub prerelease: bool,
    /// Who made it (a GitHub login).
    pub author: String,
    /// When it was drafted, and when it was published (Unix seconds).
    pub created: i64,
    pub published: Option<i64>,
    /// The branch or commit a new tag is made from.
    pub target: String,
    /// Its page on GitHub.
    pub url: String,
    /// The files uploaded to it (GitHub's source archives aren't among them).
    pub assets: Vec<Asset>,
}

impl Release {
    /// The title, or the tag when it has none.
    pub fn title(&self) -> &str {
        if self.name.trim().is_empty() { &self.tag } else { self.name.trim() }
    }

    /// When it was published, or drafted for a draft.
    pub fn at(&self) -> i64 {
        self.published.unwrap_or(self.created)
    }

    /// A release GitHub saved but whose answer couldn't be read (no id): the list is read again.
    pub fn unread(tag: &str, draft: bool, prerelease: bool) -> Release {
        Release { id: 0, tag: tag.into(), name: String::new(), body: String::new(), draft, prerelease, author: String::new(), created: 0, published: None, target: String::new(), url: String::new(), assets: Vec::new() }
    }
}

/// The word a release's state is shown with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The release GitHub shows as the latest.
    Latest,
    Pre,
    Draft,
    /// Published, and not the latest: no word.
    Published,
}

impl Kind {
    pub fn word(self) -> Option<&'static str> {
        match self {
            Kind::Latest => Some("Latest"),
            Kind::Pre => Some("Pre-release"),
            Kind::Draft => Some("Draft"),
            Kind::Published => None,
        }
    }
}

/// The state of release `r`, given the id of the latest release.
pub fn kind(r: &Release, latest: Option<u64>) -> Kind {
    match () {
        _ if r.draft => Kind::Draft,
        _ if r.prerelease => Kind::Pre,
        _ if latest == Some(r.id) => Kind::Latest,
        _ => Kind::Published,
    }
}

/// One tag.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    /// The commit it points at (empty when not known yet: a tag a publish just made).
    pub sha: String,
}

/// The latest release of a repository, for the header and the list's **Latest**.
#[derive(Clone, Debug, PartialEq)]
pub struct Latest {
    pub id: u64,
    pub tag: String,
}

/// What the first read of the section brings: the first page of releases and the tags.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub releases: Vec<Release>,
    pub tags: Vec<Tag>,
    /// GitHub has more tags than the [`TAGS`] read.
    pub tags_more: bool,
}

impl Listing {
    /// Every tag name known: the tags, and the releases' (a draft's tag may not exist yet).
    pub fn tag_names(&self) -> impl Iterator<Item = &str> {
        self.tags.iter().map(|t| t.name.as_str()).chain(self.releases.iter().map(|r| r.tag.as_str()))
    }

    /// Whether `tag` exists on GitHub: a tag, or a published release's (a draft's isn't made yet).
    pub fn tag_exists(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.name == tag) || self.releases.iter().any(|r| !r.draft && r.tag == tag)
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// One release from GitHub's answer; `None` without an id or a tag.
pub fn parse_release(v: &Value) -> Option<Release> {
    let (id, tag) = (n(v, "id"), s(v, "tag_name"));
    if id == 0 || tag.is_empty() {
        return None;
    }
    let assets = v.get("assets").and_then(Value::as_array).into_iter().flatten().map(|a| Asset {
        name: s(a, "name"),
        size: n(a, "size"),
        downloads: n(a, "download_count"),
        url: s(a, "browser_download_url"),
    }).filter(|a| !a.name.is_empty()).collect();
    Some(Release {
        id,
        tag,
        name: s(v, "name"),
        body: s(v, "body").replace("\r\n", "\n"),
        draft: flag(v, "draft"),
        prerelease: flag(v, "prerelease"),
        author: v.get("author").map(|a| s(a, "login")).filter(|l| !l.is_empty()).unwrap_or_else(|| "unknown".into()),
        created: parse_iso(&s(v, "created_at")).unwrap_or(0),
        published: parse_iso(&s(v, "published_at")),
        target: s(v, "target_commitish"),
        url: s(v, "html_url"),
        assets,
    })
}

pub fn parse_releases(v: &Value) -> Vec<Release> {
    v.as_array().into_iter().flatten().filter_map(parse_release).collect()
}

pub fn parse_tags(v: &Value) -> Vec<Tag> {
    v.as_array().into_iter().flatten().map(|t| Tag {
        name: s(t, "name"),
        sha: t.get("commit").map(|c| s(c, "sha")).unwrap_or_default(),
    }).filter(|t| !t.name.is_empty()).collect()
}

/// The tags that have no release (a draft counts: it's that tag's release, not published yet).
pub fn without_release<'a>(tags: &'a [Tag], releases: &[Release]) -> Vec<&'a Tag> {
    tags.iter().filter(|t| !releases.iter().any(|r| r.tag == t.name)).collect()
}

/// The latest release among `releases` when GitHub hasn't said: the newest published one that
/// isn't a pre-release.
pub fn latest_of(releases: &[Release]) -> Option<&Release> {
    releases.iter().filter(|r| !r.draft && !r.prerelease).max_by_key(|r| r.at())
}

/// "2.1 MB", "845 KB", "12 B".
pub fn size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    let (v, unit) = match bytes {
        0..1024 => return format!("{bytes} B"),
        1024..1_048_576 => (b / K, "KB"),
        1_048_576..1_073_741_824 => (b / (K * K), "MB"),
        _ => (b / (K * K * K), "GB"),
    };
    if v < 10.0 { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") }
}

/// "1 download", "148 downloads".
pub fn downloads(n: u64) -> String {
    if n == 1 { "1 download".into() } else { format!("{n} downloads") }
}

/// The first [`NOTES_LINES`] lines of `body` when it's longer (a code block cut in the middle is
/// closed), else `None`.
pub fn collapsed(body: &str) -> Option<String> {
    let lines: Vec<&str> = body.trim_end().lines().collect();
    if lines.len() <= NOTES_LINES + 2 {
        return None;
    }
    let mut out = lines[..NOTES_LINES].join("\n");
    let fences = lines[..NOTES_LINES].iter().filter(|l| l.trim_start().starts_with("```")).count();
    if fences % 2 == 1 {
        out.push_str("\n```");
    }
    Some(out)
}

// ------------------------------------------------------------------ versions and tags

/// The version a tag stands for, if it looks like one (`v1.2.3`, `1.2.3`, `v1.3.0-rc.1`): its
/// numbers, whether it's a pre-release, and whether it's written with a `v`.
fn version_of(tag: &str) -> Option<([u64; 3], bool, bool)> {
    let t = tag.trim();
    if t.starts_with("vv") || t.starts_with("v-") {
        return None;
    }
    let (numbers, pre) = crate::update::semver(t)?;
    Some((numbers, pre, t.starts_with('v')))
}

/// The next patch, minor and major versions after the newest tag that looks like a version,
/// written the same way (`v1.2.3` → `v1.2.4`, `v1.3.0`, `v2.0.0`; `1.2.3` → `1.2.4`…). After a
/// pre-release (`v1.3.0-rc.1`) the first is its release (`v1.3.0`). With no such tag: `v0.1.0`.
pub fn next_versions<'a>(tags: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut best: Option<(&str, [u64; 3], bool, bool)> = None;
    for t in tags {
        let Some((numbers, pre, v)) = version_of(t) else { continue };
        if best.is_none_or(|(b, ..)| crate::update::version_newer(t, b)) {
            best = Some((t, numbers, pre, v));
        }
    }
    let Some((_, [major, minor, patch], pre, v)) = best else { return vec!["v0.1.0".into()] };
    let p = if v { "v" } else { "" };
    let first = if pre { [major, minor, patch] } else { [major, minor, patch + 1] };
    [first, [major, minor + 1, 0], [major + 1, 0, 0]].iter().map(|[a, b, c]| format!("{p}{a}.{b}.{c}")).collect()
}

/// Why `tag` can't be a release's tag, if so: the rules of a branch name.
pub fn tag_problem(tag: &str) -> Option<&'static str> {
    if tag.trim().is_empty() {
        Some("Give the release a tag, or pick one of the versions above.")
    } else if !github::valid_branch(tag) {
        Some("Tags can't have spaces or any of ~ ^ : ? * [ \\, start with - or /, or contain ..")
    } else {
        None
    }
}

/// What the form's tag will do.
#[derive(Clone, Debug, PartialEq)]
pub enum TagUse {
    /// Nothing typed, or not a tag GitHub takes (the reason).
    Invalid(&'static str),
    /// Another release already has it: its title, and whether it's a draft.
    Taken(String, bool),
    /// The tag exists: the release uses it, and no branch is asked for.
    Existing,
    /// The tag is made when the release is published, from the target branch.
    New,
}

/// What `tag` does for the release being written (`editing`: the draft's id), as far as the
/// releases and tags read (`listing`) tell.
pub fn tag_use(tag: &str, listing: Option<&Listing>, editing: Option<u64>) -> TagUse {
    let tag = tag.trim();
    if let Some(why) = tag_problem(tag) {
        return TagUse::Invalid(why);
    }
    let Some(l) = listing else { return TagUse::New };
    if let Some(r) = l.releases.iter().find(|r| r.tag == tag && Some(r.id) != editing) {
        return TagUse::Taken(r.title().to_owned(), r.draft);
    }
    if l.tag_exists(tag) { TagUse::Existing } else { TagUse::New }
}

/// The question asked before publishing tag `tag` (`exists`: it's on GitHub already).
pub fn publish_question(tag: &str, target: &str, exists: bool) -> String {
    if exists {
        format!("Publish {tag}? It uses the existing tag, notifies the repository's watchers and can't be undone from RepoHub.")
    } else {
        format!("Publish {tag} from {target}? This creates the tag and can't be undone from RepoHub.")
    }
}

// ------------------------------------------------------------------ the form

/// Generate notes: what it's doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum NotesJob {
    #[default]
    Idle,
    Asking,
    Failed(String),
}

/// The New release form, or a draft being edited.
pub struct Form {
    pub repo: String,
    /// The draft being edited, or `None` for a new release.
    pub draft_id: Option<u64>,
    pub tag: TextEdit,
    /// The branch a new tag is made from.
    pub target: String,
    pub title: TextEdit,
    pub notes: TextEdit,
    pub prerelease: bool,
    pub submit: Submit,
    /// The send running (or that failed) publishes, rather than saving a draft.
    pub publishing: bool,
    /// **Publish release** was pressed: the question shows in place of the buttons.
    pub confirm: bool,
    pub notes_job: NotesJob,
    /// The request Generate notes waits for (see [`Releases::epoch`]).
    pub notes_epoch: u64,
    /// GitHub's notes, waiting for a yes because the field has notes you typed.
    pub offered: Option<String>,
    /// The notes Generate notes last put in the field: replaced again without asking.
    pub generated: Option<String>,
}

impl Form {
    pub fn new(repo: &str, target: &str) -> Form {
        let mut tag = TextEdit::new("");
        tag.focused = true;
        Form {
            repo: repo.to_owned(),
            draft_id: None,
            tag,
            target: target.to_owned(),
            title: TextEdit::new(""),
            notes: TextEdit::new(""),
            prerelease: false,
            submit: Submit::Editing,
            publishing: false,
            confirm: false,
            notes_job: NotesJob::Idle,
            notes_epoch: 0,
            offered: None,
            generated: None,
        }
    }

    /// Draft `r` of `repo`, to edit.
    pub fn of(repo: &str, r: &Release, default_branch: &str) -> Form {
        Form {
            draft_id: Some(r.id),
            tag: TextEdit::new(r.tag.clone()),
            target: if r.target.is_empty() { default_branch.to_owned() } else { r.target.clone() },
            title: TextEdit::new(r.name.clone()),
            notes: TextEdit::new(r.body.clone()),
            prerelease: r.prerelease,
            ..Form::new(repo, default_branch)
        }
    }

    /// What saving (`publish` false) or publishing would send, or why it can't yet.
    pub fn release(&self, publish: bool, listing: Option<&Listing>) -> Result<NewRelease, String> {
        let tag = self.tag.text.trim().to_owned();
        match tag_use(&tag, listing, self.draft_id) {
            TagUse::Invalid(why) => return Err(why.into()),
            TagUse::Taken(title, true) => return Err(format!("The draft “{title}” already uses {tag}: edit that one, or pick another tag.")),
            TagUse::Taken(title, false) => return Err(format!("“{title}” already uses {tag}: pick another tag.")),
            TagUse::New if !github::valid_branch(&self.target) => return Err("Pick the branch the tag is made from.".into()),
            TagUse::Existing | TagUse::New => {}
        }
        let title = self.title.text.trim();
        Ok(NewRelease {
            name: if title.is_empty() { tag.clone() } else { title.to_owned() },
            tag,
            target: self.target.clone(),
            body: self.notes.text.trim_end().to_owned(),
            draft: !publish,
            prerelease: self.prerelease,
        })
    }

    /// Whether the notes field holds something you typed (not empty, and not what Generate
    /// notes put there).
    pub fn typed_notes(&self) -> bool {
        let now = self.notes.text.trim();
        !now.is_empty() && self.generated.as_deref().map(str::trim) != Some(now)
    }
}

// ------------------------------------------------------------------ what's on screen

/// The releases of one repository as far as they're loaded.
pub struct List {
    pub repo: String,
    pub load: Load<Listing>,
    /// Pages of releases read so far.
    pub page: usize,
    /// GitHub has more releases.
    pub more: bool,
    pub loading_more: bool,
    /// The request this list waits for (see [`Releases::epoch`]).
    pub epoch: u64,
}

/// The Releases section's state, and every repository's latest release for the header.
#[derive(Default)]
pub struct Releases {
    pub list: Option<List>,
    /// The latest release of each repository (lowercase full name); `Ready(None)`: it has none.
    /// A missing entry hasn't been asked for.
    pub latest: HashMap<String, Load<Option<Latest>>>,
    pub form: Option<Form>,
    /// The draft whose **Publish** waits for a yes (its id).
    pub confirm: Option<u64>,
    /// Releases whose long notes show in full.
    pub expanded: HashSet<u64>,
    /// Drafts being published from the list.
    pub busy: HashSet<u64>,
    /// Counts requests; each part keeps the one it waits for, and drops other answers.
    pub epoch: u64,
}

/// The `latest` map's key for repository `full`.
pub fn key(full: &str) -> String {
    full.to_ascii_lowercase()
}

impl Releases {
    fn next_epoch(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    /// The list of `repo`, if that's the one loaded.
    pub fn list_of(&self, repo: &str) -> Option<&List> {
        self.list.as_ref().filter(|l| l.repo.eq_ignore_ascii_case(repo))
    }

    /// The releases and tags read for `repo`, when they're here.
    pub fn listing_of(&self, repo: &str) -> Option<&Listing> {
        self.list_of(repo).and_then(|l| l.load.ready())
    }

    /// The latest release of `repo`, when it's known and there is one.
    pub fn latest_of(&self, repo: &str) -> Option<&Latest> {
        match self.latest.get(&key(repo)) {
            Some(Load::Ready(Some(l))) => Some(l),
            _ => None,
        }
    }

    /// The id the list marks as **Latest**: GitHub's answer, else worked out from the list.
    pub fn latest_id(&self, repo: &str) -> Option<u64> {
        match self.latest.get(&key(repo)) {
            Some(Load::Ready(l)) => l.as_ref().map(|l| l.id),
            _ => self.listing_of(repo).and_then(|l| latest_of(&l.releases)).map(|r| r.id),
        }
    }

    /// The section is left: questions waiting for a yes are dropped; the list and an open form
    /// stay for next time.
    pub fn leave(&mut self) {
        self.confirm = None;
        if let Some(f) = self.form.as_mut() {
            f.confirm = false;
        }
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

const SITE_120: &str = "The desktop shell grows widgets, and the bunny gets a scene of its own.

### Added
- **Widgets shelf**: clock, weather (cached for ten minutes), notes and a terminal.
- The procedural **Blue Bunny**, with softer ears and fewer polygons.
- Snap dragged windows to the screen edges.
- A 404 page that matches the shell.

### Fixed
- The dock no longer covers the footer on iPad.
- The clock widget no longer drifts after the computer sleeps.
- The window manager keeps the top window when the screen is resized.

### Changed
- Hero images are 420 KB smaller.
- System fonts show until the web font has loaded.
- The build uses plain ES modules; jQuery is gone.

**Full Changelog**: https://github.com/francescodibella/francescodibella.github.io/compare/v1.1.0...v1.2.0";

/// The sample releases and tags of `r`, newest first.
pub fn sample(r: &Repo, me: &str, now: i64) -> Listing {
    const DAY: i64 = 86_400;
    let rel = |id: u64, tag: &str, name: &str, body: &str, draft: bool, pre: bool, age: i64, assets: Vec<Asset>| Release {
        id,
        tag: tag.into(),
        name: name.into(),
        body: body.into(),
        draft,
        prerelease: pre,
        author: me.into(),
        created: now - age - 3600,
        published: (!draft).then_some(now - age),
        target: r.branch.clone(),
        url: if draft { format!("{}/releases/tag/untagged-{}", r.url, &sample_sha(tag)[..20]) } else { format!("{}/releases/tag/{tag}", r.url) },
        assets,
    };
    let file = |name: &str, size: u64, downloads: u64, tag: &str| Asset { name: name.into(), size, downloads, url: format!("{}/releases/download/{tag}/{name}", r.url) };
    let tag = |name: &str| Tag { name: name.into(), sha: sample_sha(&format!("{}@{name}", r.full_name)) };
    match r.name.as_str() {
        "francescodibella.github.io" => Listing {
            releases: vec![
                rel(3003, "v1.3.0", "Notes widget and window snapping", "### Added\n- The **notes widget**, saved in the browser.\n- Open the terminal widget with Ctrl+`.\n\n### Fixed\n- The bunny's poncho stays outside its arms when they're lowered (#31).", true, false, 2 * DAY, Vec::new()),
                rel(3002, "v1.3.0-rc.1", "v1.3.0 release candidate", "Try the notes widget before it ships. Notes are kept in this browser only.", false, true, 5 * DAY, vec![file("site-v1.3.0-rc.1.zip", 2_205_123, 12, "v1.3.0-rc.1")]),
                rel(3001, "v1.2.0", "Shell widgets and the procedural bunny", SITE_120, false, false, 18 * DAY, vec![
                    file("site-v1.2.0.zip", 2_093_517, 148, "v1.2.0"),
                    file("bunny.glb", 865_402, 61, "v1.2.0"),
                ]),
            ],
            tags: vec![tag("v1.3.0-rc.1"), tag("v1.2.0"), tag("v1.1.0"), tag("v1.0.0")],
            tags_more: false,
        },
        "ngsi-ld-xr-contexts" => Listing {
            releases: vec![rel(4001, "v0.1.0", "First contexts: AR and VR", "The first JSON-LD contexts for augmented and virtual reality entities, with an example for each.", false, false, 40 * DAY, vec![file("contexts-v0.1.0.zip", 48_211, 23, "v0.1.0")])],
            tags: vec![tag("v0.1.0")],
            tags_more: false,
        },
        _ => Listing { releases: Vec::new(), tags: Vec::new(), tags_more: false },
    }
}

/// The notes GitHub would generate for a sample release: the last commits, as pull requests.
fn sample_notes(r: &Repo, me: &str, target: &str, tag: &str, previous: Option<&str>) -> String {
    let mut out = String::from("## What's Changed\n");
    for (n, c) in crate::commits::sample_history(r, target, me).iter().take(4).enumerate() {
        out.push_str(&format!("* {} by @{} in {}/pull/{}\n", c.title, c.author, r.url, 40 - n));
    }
    out.push_str(&match previous {
        Some(p) => format!("\n**Full Changelog**: {}/compare/{p}...{tag}", r.url),
        None => format!("\n**Full Changelog**: {}/commits/{tag}", r.url),
    });
    out
}

// ------------------------------------------------------------------ loading and sending

#[cfg(test)]
thread_local! {
    /// Tests run the requests at once, as a headless still does.
    pub(crate) static BLOCKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    if blocking() {
        let r = pollster::block_on(work());
        done(s, r);
        return true;
    }
    false
}

/// Requests run before the first frame: a headless still (`REPOHUB_BLOCKING=1`), or a test.
fn blocking() -> bool {
    #[cfg(test)]
    if BLOCKING.with(|b| b.get()) {
        return true;
    }
    !cfg!(target_arch = "wasm32") && std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1")
}

const NOT_READY: &str = "The app isn't ready to do this yet: try again in a moment.";

impl App {
    /// Reads the releases and tags of repository `i` (`again`: even when they're already here;
    /// what's on screen stays until the answer).
    pub(crate) fn load_releases(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let repo = r.full_name.clone();
        let same = self.releases.list_of(&repo);
        if same.is_some_and(|l| !again && !matches!(l.load, Load::Failed(_))) {
            return;
        }
        let kept = if same.is_some() { self.releases.list.take().and_then(|l| match l.load { Load::Ready(x) => Some(x), _ => None }) } else { None };
        let epoch = self.releases.next_epoch();
        crate::state::trace("releases", &repo, &again);
        if self.data.sample {
            let listing = sample(r, &self.data.profile.login, self.now);
            let latest = latest_of(&listing.releases).map(|l| Latest { id: l.id, tag: l.tag.clone() });
            self.releases.list = Some(List { repo: repo.clone(), load: Load::Loading, page: 1, more: false, loading_more: false, epoch });
            self.apply_releases(self.load_gen, epoch, Ok((listing, false, latest)));
            return;
        }
        self.releases.list = Some(List { repo: repo.clone(), load: kept.map_or(Load::Loading, Load::Ready), page: 1, more: false, loading_more: false, epoch });
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_release_listing(&cfg, &repo).await }, move |s: &mut App, r| s.apply_releases(account, epoch, r)) {
            self.apply_releases(account, epoch, Err(NOT_READY.into()));
        }
    }

    /// GitHub's answer for the first read of the section asked with `epoch`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn apply_releases(&mut self, account: u64, epoch: u64, r: Result<(Listing, bool, Option<Latest>), String>) {
        if self.load_gen != account {
            return;
        }
        let Some(l) = self.releases.list.as_mut().filter(|l| l.epoch == epoch) else { return };
        match r {
            Ok((listing, more, latest)) => {
                self.releases.latest.insert(key(&l.repo), Load::Ready(latest));
                l.load = Load::Ready(listing);
                l.page = 1;
                l.more = more;
            }
            // A reload that fails keeps what was read before, and says so.
            Err(e) if matches!(l.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => l.load = Load::Failed(e),
        }
    }

    /// The next page of releases.
    pub(crate) fn load_more_releases(&mut self) {
        let Some(l) = self.releases.list.as_mut() else { return };
        if l.loading_more || !l.more || !matches!(l.load, Load::Ready(_)) {
            return;
        }
        let (repo, epoch, page) = (l.repo.clone(), l.epoch, l.page + 1);
        l.loading_more = true;
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_releases(&cfg, &repo, page).await }, move |s: &mut App, r| s.apply_more_releases(account, epoch, page, r)) {
            self.apply_more_releases(account, epoch, page, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_more_releases(&mut self, account: u64, epoch: u64, page: usize, r: Result<(Vec<Release>, bool), String>) {
        if self.load_gen != account {
            return;
        }
        let Some(l) = self.releases.list.as_mut().filter(|l| l.epoch == epoch) else { return };
        l.loading_more = false;
        match r {
            Ok((rows, more)) => {
                if let Load::Ready(listing) = &mut l.load {
                    // A release published meanwhile shifts the pages: one already listed isn't listed twice.
                    let fresh: Vec<Release> = rows.into_iter().filter(|x| !listing.releases.iter().any(|y| y.id == x.id)).collect();
                    listing.releases.extend(fresh);
                    l.page = page;
                    l.more = more;
                }
            }
            Err(e) => self.notify_problem(format!("More releases couldn't load: {e}")),
        }
    }

    /// Asks GitHub for repository `i`'s latest release, for the header (`again`: even when known;
    /// the pill stays until the answer).
    pub(crate) fn load_latest_release(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, k) = (r.full_name.clone(), key(&r.full_name));
        match self.releases.latest.get(&k) {
            Some(Load::Loading) => return,
            Some(Load::Ready(_)) if !again => return,
            _ => {}
        }
        if self.data.sample {
            let listing = sample(r, &self.data.profile.login, self.now);
            let latest = latest_of(&listing.releases).map(|l| Latest { id: l.id, tag: l.tag.clone() });
            self.releases.latest.insert(k, Load::Ready(latest));
            return;
        }
        // No window yet (and no headless still): it's asked when the page opens.
        if !self.cfg.live() || (self.handle.is_none() && !blocking()) {
            return;
        }
        // The tag on screen stays until the answer.
        if !matches!(self.releases.latest.get(&k), Some(Load::Ready(_))) {
            self.releases.latest.insert(k.clone(), Load::Loading);
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        let done = move |s: &mut App, r: Result<Option<Release>, String>| {
            if s.load_gen != account {
                return;
            }
            // The header just doesn't show a tag when this fails; the section says why.
            s.releases.latest.insert(k, r.map(|l| l.map(|l| Latest { id: l.id, tag: l.tag })).into());
        };
        run(self, move || async move { github::load_latest_release(&cfg, &repo).await }, done);
    }

    /// After a refresh: the repository on screen (`i`) reads its releases again (the section's
    /// list, or the header's latest release), and the other repositories' are forgotten.
    pub(crate) fn releases_refreshed(&mut self, i: Option<usize>) {
        let keep = i.and_then(|i| self.data.repos.get(i)).map(|r| key(&r.full_name));
        self.releases.latest.retain(|k, _| Some(k) == keep.as_ref());
        if self.releases.list.as_ref().is_some_and(|l| Some(key(&l.repo)) != keep) {
            self.releases.list = None;
            self.releases.expanded.clear();
        }
        match i {
            Some(i) if self.tab == RepoTab::Releases => self.load_releases(i, true),
            Some(i) => self.load_latest_release(i, true),
            None => {}
        }
    }

    // -------------------------------------------------------------- the form

    /// Opens the New release form of repository `i` on its Releases section; `tag`: for a tag
    /// that has no release yet.
    pub(crate) fn open_release_form(&mut self, i: usize, tag: Option<String>) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, default) = (r.full_name.clone(), r.branch.clone());
        if self.tab != RepoTab::Releases {
            self.set_tab(i, RepoTab::Releases);
        }
        self.load_branches(i, false);
        let mut f = Form::new(&repo, &default);
        if let Some(t) = tag {
            f.tag = TextEdit::new(t);
            f.tag.focused = false;
            f.title.focused = true;
        }
        self.releases.form = Some(f);
        self.releases.confirm = None;
        self.picker = None;
    }

    /// Opens draft `id` of repository `i` in the form.
    pub(crate) fn edit_draft(&mut self, i: usize, id: u64) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, default) = (r.full_name.clone(), r.branch.clone());
        let Some(d) = self.releases.listing_of(&repo).and_then(|l| l.releases.iter().find(|x| x.id == id)).cloned() else { return };
        if self.tab != RepoTab::Releases {
            self.set_tab(i, RepoTab::Releases);
        }
        self.load_branches(i, false);
        let mut f = Form::of(&repo, &d, &default);
        f.title.focused = true;
        self.releases.form = Some(f);
        self.releases.confirm = None;
        self.picker = None;
    }

    pub(crate) fn close_release_form(&mut self) {
        self.releases.form = None;
        if self.picker == Some(BranchSlot::ReleaseTarget) {
            self.picker = None;
        }
    }

    /// The branch picker chose the branch a new tag is made from.
    pub(crate) fn pick_release_target(&mut self, branch: String) {
        if let Some(f) = self.releases.form.as_mut() {
            f.target = branch;
        }
    }

    /// Saves the form as a draft (`publish` false), or publishes it. Publishing only happens
    /// once its question was answered: the first call asks it.
    pub(crate) fn send_release(&mut self, publish: bool) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Releases { list, form, .. } = &mut self.releases;
        let Some(f) = form.as_mut() else { return };
        let listing = list.as_ref().filter(|l| l.repo.eq_ignore_ascii_case(&f.repo)).and_then(|l| l.load.ready());
        let new = match f.release(publish, listing) {
            Ok(n) => n,
            Err(e) => {
                f.submit = Submit::Failed(e);
                f.confirm = false;
                return;
            }
        };
        if publish && !f.confirm {
            f.confirm = true;
            return;
        }
        if !f.submit.start() {
            return;
        }
        f.publishing = publish;
        f.confirm = false;
        let w = match f.draft_id {
            Some(id) => Write::EditRelease { id, release: new },
            None => Write::Release(new),
        };
        let repo = f.repo.clone();
        let (cfg, account, back) = (self.cfg.clone(), self.load_gen, repo.clone());
        if !run(self, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| s.release_sent(account, &back, r))
            && let Some(f) = self.releases.form.as_mut()
        {
            f.submit = Submit::Failed(NOT_READY.into());
        }
    }

    /// The form's write finished: it closes and the list shows the release, or it stays with
    /// what was typed and GitHub's reason.
    fn release_sent(&mut self, account: u64, repo: &str, r: Result<github::Done, String>) {
        if self.load_gen != account {
            return;
        }
        let ours = self.releases.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(repo) && f.submit.is_sending());
        match r {
            Ok(done) => {
                if ours {
                    self.close_release_form();
                }
                self.apply_done(repo, done);
            }
            Err(e) => match self.releases.form.as_mut().filter(|_| ours) {
                Some(f) => f.submit = Submit::Failed(e),
                None => self.notify_problem(e),
            },
        }
    }

    /// **Publish** on draft `id` of repository `i`, once its question was answered (`confirm`).
    pub(crate) fn publish_draft(&mut self, i: usize, id: u64) {
        if actions::blocked(self).is_some() || self.releases.confirm != Some(id) {
            return;
        }
        let Some(repo) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        self.releases.confirm = None;
        if !self.releases.busy.insert(id) {
            return;
        }
        let (cfg, account, back) = (self.cfg.clone(), self.load_gen, repo.clone());
        let done = move |s: &mut App, r: Result<github::Done, String>| {
            if s.load_gen != account {
                return;
            }
            s.releases.busy.remove(&id);
            match r {
                Ok(done) => s.apply_done(&back, done),
                Err(e) => s.notify_problem(e),
            }
        };
        if !run(self, move || async move { github::run(&cfg, &repo, Write::PublishRelease { id }).await }, done) {
            self.releases.busy.remove(&id);
            self.notify_problem(NOT_READY);
        }
    }

    /// A release was saved or published in `repo` (`apply_done`): the list and the header
    /// show it at once.
    pub(crate) fn release_done(&mut self, repo: &str, rel: Release) {
        self.releases.busy.remove(&rel.id);
        let msg = match (rel.draft, rel.tag.is_empty()) {
            (true, _) => "Draft saved".to_owned(),
            (false, true) => "Published the release".to_owned(),
            (false, false) => format!("Published {}", rel.tag),
        };
        if !rel.draft && !rel.prerelease && rel.id != 0 {
            self.releases.latest.insert(key(repo), Load::Ready(Some(Latest { id: rel.id, tag: rel.tag.clone() })));
        }
        if rel.id == 0 {
            // GitHub's answer couldn't be read: the list is read again.
            if let Some(i) = self.repo_index_of(repo).filter(|_| self.tab == RepoTab::Releases) {
                self.load_releases(i, true);
            }
        } else if let Some(List { load: Load::Ready(listing), .. }) = self.releases.list.as_mut().filter(|l| same_repo(&l.repo, repo)) {
            if !rel.draft && !listing.tags.iter().any(|t| t.name == rel.tag) {
                listing.tags.insert(0, Tag { name: rel.tag.clone(), sha: String::new() });
            }
            match listing.releases.iter().position(|x| x.id == rel.id) {
                Some(at) => listing.releases[at] = rel,
                None => listing.releases.insert(0, rel),
            }
        }
        self.notify(msg);
    }

    /// **Generate notes**: asks GitHub for the notes since the previous release. They go in the
    /// field, or wait for a yes when it has notes you typed.
    pub(crate) fn generate_notes(&mut self) {
        let sample = self.data.sample;
        if !sample && actions::blocked(self).is_some() {
            return;
        }
        let epoch = self.releases.next_epoch();
        let Some(f) = self.releases.form.as_mut() else { return };
        let (tag, target, repo) = (f.tag.text.trim().to_owned(), f.target.clone(), f.repo.clone());
        if tag_problem(&tag).is_some() {
            f.notes_job = NotesJob::Failed("Pick a tag first: GitHub writes the notes up to it.".into());
            return;
        }
        f.notes_job = NotesJob::Asking;
        f.notes_epoch = epoch;
        f.offered = None;
        if sample {
            let previous = self.releases.latest_of(&repo).map(|l| l.tag.clone()).filter(|p| *p != tag);
            let text = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo))
                .map(|r| sample_notes(r, &self.data.profile.login, &target, &tag, previous.as_deref()))
                .ok_or_else(|| "This repository isn't in the sample data.".to_owned());
            self.notes_arrived(self.load_gen, epoch, text);
            return;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::generate_release_notes(&cfg, &repo, &tag, &target, None).await }, move |s: &mut App, r| s.notes_arrived(account, epoch, r)) {
            self.notes_arrived(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn notes_arrived(&mut self, account: u64, epoch: u64, r: Result<String, String>) {
        if self.load_gen != account {
            return;
        }
        let Some(f) = self.releases.form.as_mut().filter(|f| f.notes_epoch == epoch && f.notes_job == NotesJob::Asking) else { return };
        match r {
            Ok(text) if f.typed_notes() => {
                f.offered = Some(text);
                f.notes_job = NotesJob::Idle;
            }
            Ok(text) => {
                f.notes = TextEdit::new(text.clone());
                f.generated = Some(text);
                f.notes_job = NotesJob::Idle;
            }
            Err(e) => f.notes_job = NotesJob::Failed(e),
        }
    }

    /// The answer to "Replace your notes?": GitHub's go in the field, or yours stay.
    pub(crate) fn take_offered_notes(&mut self, replace: bool) {
        let Some(f) = self.releases.form.as_mut() else { return };
        let Some(text) = f.offered.take() else { return };
        if replace {
            f.notes = TextEdit::new(text.clone());
            f.generated = Some(text);
        }
    }

    /// Esc on the Releases section: the question on screen, then the form. True when it closed
    /// something.
    pub(crate) fn release_escape(&mut self) -> bool {
        let Some(i) = self.repo_index().filter(|_| self.tab == RepoTab::Releases) else { return false };
        if self.releases.confirm.take().is_some() {
            return true;
        }
        let repo = self.data.repos[i].full_name.clone();
        let Some(f) = self.releases.form.as_mut().filter(|f| f.repo.eq_ignore_ascii_case(&repo)) else { return false };
        if f.confirm {
            f.confirm = false;
        } else if f.offered.take().is_none() {
            self.close_release_form();
        }
        true
    }

    /// For headless stills: with the Releases section on screen (`-- tab releases`) its list is
    /// read before the first frame, and `-- release new|notes|edit|publish` opens the form (with
    /// generated notes, the first draft, or the publish question). The header's latest release
    /// is read for any repository page.
    pub(crate) fn releases_still(&mut self, what: Option<String>) {
        let Some(i) = self.repo_index() else { return };
        self.load_latest_release(i, false);
        if what.is_none() && self.tab != RepoTab::Releases {
            return;
        }
        self.tab = RepoTab::Releases;
        self.reveal_tab = true;
        self.load_releases(i, false);
        let repo = self.data.repos[i].full_name.clone();
        match what.as_deref() {
            Some("edit") => {
                if let Some(id) = self.releases.listing_of(&repo).and_then(|l| l.releases.iter().find(|r| r.draft)).map(|r| r.id) {
                    self.edit_draft(i, id);
                }
            }
            Some(w @ ("new" | "notes" | "publish")) => {
                self.open_release_form(i, None);
                let next = self.releases.listing_of(&repo).map(|l| next_versions(l.tag_names())).unwrap_or_default();
                if let (Some(f), Some(tag)) = (self.releases.form.as_mut(), next.into_iter().nth(1).filter(|_| w != "new")) {
                    f.tag = TextEdit::new(tag);
                }
                if w != "new" {
                    self.generate_notes();
                }
                if w == "publish"
                    && let Some(f) = self.releases.form.as_mut()
                {
                    f.confirm = true;
                }
            }
            _ => {}
        }
    }
}

/// A stand-in for GitHub on 127.0.0.1, for the tests here and in `releases_page`.
#[cfg(test)]
pub(crate) mod mock {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    /// What the mock was asked: "METHOD path", and the body.
    pub type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// Answers each request with `answer(method, path, body)`: status, extra headers, body.
    pub fn serve(answer: impl Fn(&str, &str, &str) -> (u16, String, String) + Send + Sync + 'static) -> (String, Seen) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break buf.len(),
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let length = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.trim().parse::<usize>().ok())).unwrap_or(0);
                while buf.len() < head_end + length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                let mut words = head.split_whitespace();
                let (method, path) = (words.next().unwrap_or("").to_owned(), words.next().unwrap_or("/").to_owned());
                log.lock().unwrap().push((format!("{method} {path}"), body.clone()));
                let (status, headers, out) = answer(&method, &path, &body);
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{out}", out.len());
            }
        });
        (url, seen)
    }

    /// A live account that talks to the mock at `url`.
    pub fn config(url: &str) -> crate::config::Config {
        let mut cfg = crate::config::Config::from_json("{}").unwrap();
        cfg.api = url.to_owned();
        cfg.token = Some("test".into());
        cfg
    }

    /// A release as GitHub answers it.
    pub fn release(id: u64, tag: &str, draft: bool, pre: bool, body: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "tag_name": tag, "name": "", "body": body, "draft": draft, "prerelease": pre,
            "author": { "login": "ada" }, "created_at": "2026-11-20T10:00:00Z",
            "published_at": if draft { serde_json::Value::Null } else { "2026-11-21T10:00:00Z".into() },
            "target_commitish": "main", "html_url": format!("https://github.com/o/r/releases/tag/{tag}"), "assets": [],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listing(tags: &[&str], releases: Vec<Release>) -> Listing {
        Listing { releases, tags: tags.iter().map(|t| Tag { name: (*t).into(), sha: String::new() }).collect(), tags_more: false }
    }

    fn release(id: u64, tag: &str, draft: bool, pre: bool, published: Option<i64>) -> Release {
        Release { id, tag: tag.into(), name: String::new(), body: String::new(), draft, prerelease: pre, author: "ada".into(), created: 1, published, target: "main".into(), url: String::new(), assets: Vec::new() }
    }

    #[test]
    fn releases_read_their_state_author_dates_and_files() {
        let v = json!([
            {
                "id": 101, "tag_name": "v0.2.0", "name": "Second", "body": "### Added\r\n- Releases", "draft": false, "prerelease": false,
                "author": { "login": "ada" }, "created_at": "2026-11-20T10:00:00Z", "published_at": "2026-11-21T09:00:00Z",
                "target_commitish": "main", "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
                "zipball_url": "https://api.github.com/repos/o/r/zipball/v0.2.0",
                "assets": [
                    { "name": "app.tar.gz", "size": 2_093_517, "download_count": 148, "browser_download_url": "https://github.com/o/r/releases/download/v0.2.0/app.tar.gz" },
                    { "name": "", "size": 1 },
                ],
            },
            { "id": 102, "tag_name": "v0.3.0", "name": "", "draft": true, "prerelease": false, "author": null, "created_at": "2026-11-22T08:00:00Z", "published_at": null },
            { "id": 0, "tag_name": "broken" },
            { "id": 103, "tag_name": "" },
        ]);
        let list = parse_releases(&v);
        assert_eq!(list.len(), 2, "an entry without an id or a tag is left out");
        let r = &list[0];
        assert_eq!((r.id, r.tag.as_str(), r.title(), r.body.as_str()), (101, "v0.2.0", "Second", "### Added\n- Releases"));
        assert_eq!((r.author.as_str(), r.at()), ("ada", parse_iso("2026-11-21T09:00:00Z").unwrap()), "published: its publication date");
        assert_eq!(r.assets.len(), 1, "a nameless file is left out, and the source archives aren't files");
        assert_eq!((r.assets[0].size, r.assets[0].downloads), (2_093_517, 148));
        let d = &list[1];
        assert_eq!((d.title(), d.author.as_str(), d.published, d.at()), ("v0.3.0", "unknown", None, parse_iso("2026-11-22T08:00:00Z").unwrap()), "a draft: the tag for a title, when it was drafted");
        assert_eq!(kind(r, Some(101)), Kind::Latest);
        assert_eq!(kind(r, Some(7)), Kind::Published);
        assert_eq!(kind(d, Some(102)), Kind::Draft, "a draft is never the latest");
        assert_eq!(kind(&release(5, "v1-rc", false, true, Some(1)), None), Kind::Pre);
        assert_eq!((Kind::Latest.word(), Kind::Pre.word(), Kind::Draft.word(), Kind::Published.word()), (Some("Latest"), Some("Pre-release"), Some("Draft"), None));
        assert_eq!([size(12), size(865_402), size(2_093_517), size(48 * 1_048_576), size(3 * 1_073_741_824)], ["12 B", "845 KB", "2.0 MB", "48 MB", "3.0 GB"]);
        assert_eq!([downloads(1), downloads(148)], ["1 download", "148 downloads"]);
    }

    #[test]
    fn tags_without_a_release_leave_out_drafts_and_published_ones() {
        let tags = parse_tags(&json!([
            { "name": "v1.3.0", "commit": { "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } },
            { "name": "v1.2.0", "commit": { "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" } },
            { "name": "v1.1.0", "commit": { "sha": "cccccccccccccccccccccccccccccccccccccccc" } },
            { "name": "", "commit": {} },
        ]));
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[1].sha, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let releases = vec![release(1, "v1.2.0", false, false, Some(10)), release(2, "v1.3.0", true, false, None)];
        let left: Vec<&str> = without_release(&tags, &releases).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(left, ["v1.1.0"], "a tag with a release, or a draft, has one");
        assert_eq!(without_release(&tags, &[]).len(), 3);
        // The latest, when GitHub hasn't said: the newest published release that isn't a pre-release.
        let all = vec![release(3, "v2.0.0-rc.1", false, true, Some(30)), release(1, "v1.2.0", false, false, Some(10)), release(4, "v1.2.1", false, false, Some(20)), release(2, "v1.3.0", true, false, None)];
        assert_eq!(latest_of(&all).map(|r| r.id), Some(4));
        assert_eq!(latest_of(&[]), None);
    }

    #[test]
    fn the_next_versions_follow_the_newest_version_tag() {
        assert_eq!(next_versions(["v0.1.0"]), ["v0.1.1", "v0.2.0", "v1.0.0"]);
        assert_eq!(next_versions(["1.4.2"]), ["1.4.3", "1.5.0", "2.0.0"], "a tag without v keeps none");
        assert_eq!(next_versions([]), ["v0.1.0"], "no version tag");
        assert_eq!(next_versions(["latest", "nightly", "v1", "release-2"]), ["v0.1.0"], "nothing that looks like a version");
        // The newest by version, not the first listed; v0.10.0 is newer than v0.9.0.
        assert_eq!(next_versions(["v0.9.0", "latest", "v0.10.0", "v0.2.5"]), ["v0.10.1", "v0.11.0", "v1.0.0"]);
        // A pre-release is followed by its release; it's older than the release itself.
        assert_eq!(next_versions(["v1.2.0", "v1.3.0-rc.1"]), ["v1.3.0", "v1.4.0", "v2.0.0"]);
        assert_eq!(next_versions(["v1.3.0", "v1.3.0-rc.1"]), ["v1.3.1", "v1.4.0", "v2.0.0"]);
        assert_eq!(next_versions(["vv1.0.0", "v-1.0.0"]), ["v0.1.0"], "not versions");
    }

    #[test]
    fn a_tag_is_checked_like_a_branch_name_and_against_the_releases_read() {
        for ok in ["v1.2.3", "1.0.0", "release/2026-11", "v1.3.0-rc.1"] {
            assert_eq!(tag_problem(ok), None, "{ok}");
        }
        for bad in ["", "  ", "v1 2", "-v1", "v1..2", "v1~1", "v1^", "a:b", "/v1", "v1/", "v1.lock", "v1?", "v*", "v[1]"] {
            assert!(tag_problem(bad).is_some(), "{bad}");
        }
        let l = listing(&["v1.2.0", "v1.1.0"], vec![release(1, "v1.2.0", false, false, Some(10)), release(2, "v1.3.0", true, false, None)]);
        assert_eq!(tag_use("v1.1.0", Some(&l), None), TagUse::Existing, "a tag without a release");
        assert_eq!(tag_use("v1.4.0", Some(&l), None), TagUse::New);
        assert_eq!(tag_use("v1.2.0", Some(&l), None), TagUse::Taken("v1.2.0".into(), false));
        assert_eq!(tag_use("v1.3.0", Some(&l), None), TagUse::Taken("v1.3.0".into(), true));
        assert_eq!(tag_use("v1.3.0", Some(&l), Some(2)), TagUse::New, "the draft being edited keeps its own tag, not made yet");
        assert_eq!(tag_use("v1.4.0", None, None), TagUse::New, "nothing read yet: GitHub decides");
        assert!(matches!(tag_use("bad tag", Some(&l), None), TagUse::Invalid(_)));
        // The form: the title defaults to the tag, a publish is not a draft, and what's wrong is said.
        let mut f = Form::new("o/r", "main");
        assert!(f.release(false, Some(&l)).unwrap_err().contains("Give the release a tag"));
        f.tag = TextEdit::new("v1.2.0");
        assert!(f.release(true, Some(&l)).unwrap_err().contains("already uses v1.2.0"));
        f.tag = TextEdit::new("v1.3.0");
        assert!(f.release(true, Some(&l)).unwrap_err().contains("edit that one"));
        f.tag = TextEdit::new(" v1.4.0 ");
        let n = f.release(false, Some(&l)).unwrap();
        assert_eq!((n.tag.as_str(), n.name.as_str(), n.target.as_str(), n.draft), ("v1.4.0", "v1.4.0", "main", true));
        f.title = TextEdit::new("Releases");
        f.prerelease = true;
        let n = f.release(true, Some(&l)).unwrap();
        assert_eq!((n.name.as_str(), n.draft, n.prerelease), ("Releases", false, true));
        f.target = "bad branch".into();
        assert!(f.release(true, Some(&l)).is_err(), "a new tag needs a branch to start from");
        f.tag = TextEdit::new("v1.1.0");
        assert!(f.release(true, Some(&l)).is_ok(), "an existing tag needs none");
        assert_eq!(publish_question("v0.2.0", "main", false), "Publish v0.2.0 from main? This creates the tag and can't be undone from RepoHub.");
        assert!(publish_question("v1.1.0", "main", true).contains("existing tag"));
    }

    #[test]
    fn long_notes_are_cut_and_a_code_block_is_closed() {
        let short = "a\nb\nc";
        assert_eq!(collapsed(short), None);
        let long: String = (1..=30).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let cut = collapsed(&long).unwrap();
        assert_eq!(cut.lines().count(), NOTES_LINES);
        assert!(cut.ends_with("line 12"));
        let fenced = format!("intro\n```\n{}\n```", (1..=20).map(|n| format!("code {n}")).collect::<Vec<_>>().join("\n"));
        assert!(collapsed(&fenced).unwrap().ends_with("\n```"), "the fence opened above the cut is closed");
        assert_eq!(collapsed(&(1..=NOTES_LINES + 2).map(|n| n.to_string()).collect::<Vec<_>>().join("\n")), None, "a line or two more isn't worth a Show all");
    }

    #[test]
    fn the_sample_data_has_releases_a_draft_and_tags_without_a_release() {
        let data = crate::model::Data::sample_at(1_794_216_911);
        let site = &data.repos[0];
        let l = sample(site, "me", 1_794_216_911);
        assert_eq!(l.releases.len(), 3);
        assert_eq!(l.releases.iter().filter(|r| r.draft).count(), 1);
        assert_eq!(l.releases.iter().filter(|r| r.prerelease).count(), 1);
        assert!(l.releases.iter().all(|r| !r.body.is_empty()) && l.releases.iter().any(|r| !r.assets.is_empty()));
        assert!(l.releases.windows(2).all(|w| w[0].created > w[1].created), "newest first");
        assert_eq!(without_release(&l.tags, &l.releases).len(), 2);
        assert_eq!(latest_of(&l.releases).map(|r| r.tag.as_str()), Some("v1.2.0"));
        assert!(collapsed(&l.releases[2].body).is_some(), "the latest's notes are long enough to collapse");
        assert_eq!(next_versions(l.tag_names()), ["v1.3.1", "v1.4.0", "v2.0.0"], "the draft's tag counts");
        assert!(sample(&data.repos[4], "me", 0).releases.is_empty(), "another repository has none");
        let notes = sample_notes(site, "me", "main", "v1.4.0", Some("v1.2.0"));
        assert!(notes.contains("## What's Changed") && notes.contains("compare/v1.2.0...v1.4.0"), "{notes}");
    }

    #[test]
    fn requests_page_the_releases_and_explain_what_github_refuses() {
        let (url, seen) = mock::serve(|method, path, body| {
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            match (method, path) {
                ("GET", "/repos/o/r/releases?per_page=20&page=1") => {
                    let page: Vec<Value> = (0..PAGE as u64).map(|n| mock::release(100 + n, &format!("v1.{n}.0"), false, false, "")).collect();
                    (200, "Link: <https://api.github.com/repositories/1/releases?per_page=20&page=2>; rel=\"next\"\r\n".into(), serde_json::to_string(&page).unwrap())
                }
                ("GET", p) if p.starts_with("/repos/o/r/releases?") => (200, String::new(), json!([mock::release(1, "v0.1.0", false, false, "")]).to_string()),
                ("GET", "/repos/o/r/tags?per_page=50") => (200, "Link: <https://api.github.com/repositories/1/tags?per_page=50&page=2>; rel=\"next\"\r\n".into(), json!([{ "name": "v1.0.0", "commit": { "sha": "a".repeat(40) } }]).to_string()),
                ("GET", "/repos/o/r/releases/latest") => (200, String::new(), mock::release(119, "v1.19.0", false, false, "").to_string()),
                ("GET", "/repos/o/none/releases/latest") => (404, String::new(), json!({ "message": "Not Found" }).to_string()),
                ("POST", "/repos/o/r/releases/generate-notes") => (200, String::new(), json!({ "name": "v2.0.0", "body": "## What's Changed\n* Releases by @ada" }).to_string()),
                ("POST", "/repos/o/r/releases") if v["tag_name"] == "v1.0.0" => (422, String::new(), json!({ "message": "Validation Failed", "errors": [{ "resource": "Release", "code": "already_exists", "field": "tag_name" }] }).to_string()),
                ("POST", "/repos/o/r/releases") => (201, String::new(), mock::release(200, v["tag_name"].as_str().unwrap_or(""), v["draft"].as_bool().unwrap_or(true), false, v["body"].as_str().unwrap_or("")).to_string()),
                ("POST", "/repos/o/locked/releases") => (403, String::new(), json!({ "message": "Resource not accessible by personal access token" }).to_string()),
                ("PATCH", "/repos/o/r/releases/200") => (200, String::new(), mock::release(200, "v2.0.0", false, false, "").to_string()),
                ("PATCH", "/repos/o/r/releases/201") => (200, String::new(), "{}".into()),
                _ => (404, String::new(), json!({ "message": "Not Found" }).to_string()),
            }
        });
        let g = crate::github::Gh::new(&mock::config(&url));
        let (rows, more) = pollster::block_on(g.releases("o/r", 1)).unwrap();
        assert_eq!((rows.len(), more), (PAGE, true), "a Link to the next page");
        let (rows, more) = pollster::block_on(g.releases("o/r", 2)).unwrap();
        assert_eq!((rows.len(), more), (1, false));
        let (listing, more, latest) = pollster::block_on(g.release_listing("o/r")).unwrap();
        assert!(more && listing.tags_more && listing.tags.len() == 1, "GitHub has more tags, and says so");
        assert_eq!(latest, Some(Latest { id: 119, tag: "v1.19.0".into() }));
        assert_eq!(pollster::block_on(g.latest_release("o/none")).unwrap(), None, "no release: GitHub answers 404");
        let notes = pollster::block_on(g.generate_notes("o/r", "v2.0.0", "main", None)).unwrap();
        assert_eq!(notes, "## What's Changed\n* Releases by @ada");
        let new = NewRelease { tag: "v2.0.0".into(), target: "main".into(), name: "Two".into(), body: "Notes".into(), draft: true, prerelease: false };
        let saved = pollster::block_on(g.save_release("o/r", None, &new)).unwrap();
        assert_eq!((saved.id, saved.draft, saved.body.as_str()), (200, true, "Notes"));
        let published = pollster::block_on(g.publish_release("o/r", 200)).unwrap();
        assert!(!published.draft);
        assert_eq!(pollster::block_on(g.publish_release("o/r", 201)).unwrap().id, 0, "an answer that can't be read: the list is read again");
        let taken = NewRelease { tag: "v1.0.0".into(), ..new.clone() };
        let e = pollster::block_on(g.save_release("o/r", None, &taken)).unwrap_err();
        assert!(e.contains("This tag already has a release"), "{e}");
        let e = pollster::block_on(g.save_release("o/locked", None, &new)).unwrap_err();
        assert!(e.contains("(403)") && e.contains("Contents: Read and write"), "{e}");
        let e = pollster::block_on(g.save_release("o/gone", None, &new)).unwrap_err();
        assert!(e.contains("Not found (404)") && e.contains("Contents: Read and write"), "{e}");
        let seen = seen.lock().unwrap().clone();
        let body = |what: &str| seen.iter().find(|(m, _)| m == what).map(|(_, b)| serde_json::from_str::<Value>(b).unwrap()).unwrap();
        let asked = body("POST /repos/o/r/releases/generate-notes");
        assert_eq!((asked["tag_name"].as_str(), asked["target_commitish"].as_str(), asked.get("previous_tag_name")), (Some("v2.0.0"), Some("main"), None));
        let created = body("POST /repos/o/r/releases");
        assert_eq!((created["tag_name"].as_str(), created["name"].as_str(), created["draft"].as_bool(), created["prerelease"].as_bool()), (Some("v2.0.0"), Some("Two"), Some(true), Some(false)));
        assert_eq!(body("PATCH /repos/o/r/releases/200"), json!({ "draft": false }), "publishing a draft sends nothing else");
        assert!(!seen.iter().any(|(m, _)| m.starts_with("DELETE")), "nothing is deleted");
    }
}
