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

mod loading;
mod sample;

#[cfg(test)]
pub(crate) use loading::BLOCKING;
pub use sample::sample;
use sample::sample_notes;

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
    /// The tag, title and notes the form opened with (a draft's, or a tag picked): what's
    /// typed is what differs.
    pub opened: [String; 3],
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
            opened: Default::default(),
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
            opened: [r.tag.clone(), r.name.clone(), r.body.clone()],
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

    /// Whether the tag, the title or the notes hold something typed since the form opened
    /// (notes Generate notes put there aren't typed): closing it then asks first.
    pub fn typed(&self) -> bool {
        let notes = self.notes.text.trim();
        self.tag.text.trim() != self.opened[0].trim()
            || self.title.text.trim() != self.opened[1].trim()
            || (notes != self.opened[2].trim() && self.generated.as_deref().map(str::trim) != Some(notes))
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

/// A stand-in for GitHub on 127.0.0.1, for the tests here and in `releases_page`.
#[cfg(test)]
pub(crate) mod mock;

#[cfg(test)]
mod tests;
