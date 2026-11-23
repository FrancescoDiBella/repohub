//! Triage: the labels, assignees, milestone and reviewers of an issue or a pull request, and
//! editing its title and description.
//!
//! - The repository's choices (`/labels`, `/assignees`, `/milestones`) load when a picker first
//!   opens and are kept per repository in [`Cache`]; a late answer to an earlier request is
//!   dropped by its epoch.
//! - One [`Picker`] is open at a time: in the open conversation ([`Edits`]) or in the New issue
//!   form ([`Picks`]). It has a filter, the choices as a checklist (one choice for the milestone),
//!   and **Apply** / **Cancel**.
//! - **Apply** sends one [`Change`] (`github::Write::Triage`). GitHub answers with the issue or
//!   pull request as it is now, read into an [`Applied`] that updates the conversation and the
//!   list rows without a reload (`App::apply_triaged`). A refusal is said once, in the details
//!   block, and the old values stay.
//!
//! The page is `triage_page`; the requests go through `github`.
use std::collections::HashMap;

use rune::TextEdit;
use serde_json::{Value, json};

use crate::model::{hex_hue, same_repo};
use crate::state::{Facet, Load};
use crate::thread::{Open, Thread, Verdict};
use crate::{App, github, runs_page};

mod app;

/// GitHub takes at most this many assignees on one issue.
pub const MAX_ASSIGNEES: usize = 10;

/// How many choices a picker lists before asking you to type to narrow them.
pub const SHOWN: usize = 30;

/// Where a picker's choices come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Labels,
    /// The people who can be assigned: assignees, and the reviewers to ask.
    People,
    Milestones,
}

impl Source {
    pub fn of(f: Facet) -> Source {
        match f {
            Facet::Labels => Source::Labels,
            Facet::Assignees | Facet::Reviewers => Source::People,
            Facet::Milestone => Source::Milestones,
        }
    }

    /// The request that lists them (the first 100; [`Choices::more`] says when there are more).
    pub fn path(self, full: &str) -> String {
        match self {
            Source::Labels => format!("/repos/{full}/labels?per_page=100"),
            Source::People => format!("/repos/{full}/assignees?per_page=100"),
            Source::Milestones => format!("/repos/{full}/milestones?state=open&per_page=100"),
        }
    }

    /// The plural, for sentences: "Reading labels…".
    pub fn noun(self) -> &'static str {
        match self {
            Source::Labels => "labels",
            Source::People => "people",
            Source::Milestones => "open milestones",
        }
    }

    pub fn read(self, v: &Value) -> Vec<Choice> {
        match self {
            Source::Labels => parse_labels(v),
            Source::People => parse_people(v),
            Source::Milestones => parse_milestones(v),
        }
    }
}

/// One thing a picker offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// What GitHub is sent: a label's name, a login, a milestone's number.
    pub id: String,
    /// What's shown: the name, the login, the milestone's title.
    pub name: String,
    /// A label's colour.
    pub hue: Option<f32>,
    /// A second line: a label's description, a milestone's due date. May be empty.
    pub note: String,
}

impl Choice {
    fn plain(id: &str) -> Choice {
        Choice { id: id.to_owned(), name: id.to_owned(), hue: None, note: String::new() }
    }
}

/// A repository's choices of one kind.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Choices {
    pub items: Vec<Choice>,
    /// GitHub has more than the 100 that were read.
    pub more: bool,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// `GET /repos/{r}/labels`.
pub fn parse_labels(v: &Value) -> Vec<Choice> {
    v.as_array().into_iter().flatten()
        .map(|l| (s(l, "name"), l))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, l)| Choice { id: name.clone(), name, hue: Some(hex_hue(&s(l, "color"))), note: s(l, "description") })
        .collect()
}

/// `GET /repos/{r}/assignees`: the people who can be assigned (and asked to review).
pub fn parse_people(v: &Value) -> Vec<Choice> {
    v.as_array().into_iter().flatten().map(|u| s(u, "login")).filter(|l| !l.is_empty()).map(|l| Choice::plain(&l)).collect()
}

/// `GET /repos/{r}/milestones?state=open`.
pub fn parse_milestones(v: &Value) -> Vec<Choice> {
    v.as_array().into_iter().flatten().filter_map(|m| {
        let number = m.get("number").and_then(Value::as_u64).filter(|n| *n > 0)?;
        let due = s(m, "due_on");
        let note = match due.get(..10) {
            Some(day) => format!("Due on {day}"),
            None => "No due date".to_owned(),
        };
        Some(Choice { id: number.to_string(), name: s(m, "title"), hue: None, note })
    }).collect()
}

/// An issue's or pull request's milestone, as `(number, title)`.
pub fn milestone_of(v: &Value) -> Option<(u32, String)> {
    let m = v.get("milestone").filter(|m| m.is_object())?;
    let number = m.get("number").and_then(Value::as_u64).filter(|n| *n > 0)? as u32;
    Some((number, s(m, "title")))
}

/// Who answered a review request, each with their latest verdict: an approval or a request for
/// changes replaces what came before; a comment only counts when there's nothing else. Pending
/// (not sent) and dismissed reviews don't count.
pub fn reviewed_of(reviews: &Value) -> Vec<(String, Verdict)> {
    let mut list: Vec<&Value> = reviews.as_array().into_iter().flatten().collect();
    list.sort_by_key(|r| crate::model::parse_iso(&s(r, "submitted_at")).unwrap_or(0));
    let mut out: Vec<(String, Verdict)> = Vec::new();
    for r in list {
        let verdict = match s(r, "state").as_str() {
            "APPROVED" => Verdict::Approved,
            "CHANGES_REQUESTED" => Verdict::ChangesRequested,
            "COMMENTED" => Verdict::Commented,
            _ => continue,
        };
        let who = r.get("user").map(|u| s(u, "login")).unwrap_or_default();
        if who.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(w, _)| w.eq_ignore_ascii_case(&who)) {
            Some(slot) if verdict != Verdict::Commented => slot.1 = verdict,
            Some(_) => {}
            None => out.push((who, verdict)),
        }
    }
    out
}

/// The logins in an array of users (`assignees`, `requested_reviewers`).
pub fn logins(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().map(|u| s(u, "login")).filter(|l| !l.is_empty()).collect()
}

// ------------------------------------------------------------------ the choices, per repository

/// The repositories' choices, loaded once per repository and kind.
#[derive(Default)]
pub struct Cache {
    lists: HashMap<(String, Source), Load<Choices>>,
    /// The request each list waits for: a late answer to an earlier one is dropped.
    asked: HashMap<(String, Source), u64>,
    epoch: u64,
    /// The writes Apply and Save started, newest last (tests read it: a headless run has no
    /// background worker to send them).
    #[cfg(test)]
    pub sent: Vec<(String, github::Write)>,
}

#[cfg(test)]
impl Cache {
    /// The epoch the answer for a list must bring back, while it's on its way.
    pub fn waiting(&self, repo: &str, source: Source) -> Option<u64> {
        self.asked.get(&key(repo, source)).copied()
    }
}

fn key(repo: &str, source: Source) -> (String, Source) {
    (repo.to_ascii_lowercase(), source)
}

impl Cache {
    pub fn get(&self, repo: &str, source: Source) -> Option<&Load<Choices>> {
        self.lists.get(&key(repo, source))
    }

    /// Starts asking for a list, unless it's here or on its way (`again`: a retry, or a list
    /// that failed). Returns the epoch the answer must bring back.
    pub fn ask(&mut self, repo: &str, source: Source, again: bool) -> Option<u64> {
        let k = key(repo, source);
        match self.lists.get(&k) {
            Some(Load::Ready(_)) | Some(Load::Loading) if !again => return None,
            _ => {}
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.asked.insert(k.clone(), self.epoch);
        if !matches!(self.lists.get(&k), Some(Load::Ready(_))) {
            self.lists.insert(k, Load::Loading);
        }
        Some(self.epoch)
    }

    /// Takes an answer. False when it isn't the one waited for (a newer request went out, or the
    /// account changed and everything was forgotten).
    pub fn answer(&mut self, repo: &str, source: Source, epoch: u64, r: Result<Choices, String>) -> bool {
        let k = key(repo, source);
        if self.asked.get(&k) != Some(&epoch) {
            return false;
        }
        self.asked.remove(&k);
        let keep_old = r.is_err() && matches!(self.lists.get(&k), Some(Load::Ready(_)));
        if !keep_old {
            self.lists.insert(k, match r {
                Ok(c) => Load::Ready(c),
                Err(e) => Load::Failed(e),
            });
        }
        true
    }

    /// Puts a list in place at once (the sample data).
    pub fn put(&mut self, repo: &str, source: Source, c: Choices) {
        let k = key(repo, source);
        self.asked.remove(&k);
        self.lists.insert(k, Load::Ready(c));
    }

    /// Forgets everything (another account).
    pub fn clear(&mut self) {
        self.lists.clear();
        self.asked.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// A label's colour, when the repository's labels are here.
    pub fn hue(&self, repo: &str, label: &str) -> Option<f32> {
        match self.get(repo, Source::Labels) {
            Some(Load::Ready(c)) => c.items.iter().find(|l| l.id.eq_ignore_ascii_case(label)).and_then(|l| l.hue),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------ the picker

/// An open picker: what it edits, the filter typed, what was ticked when it opened and now.
#[derive(Clone, Debug)]
pub struct Picker {
    pub facet: Facet,
    pub filter: TextEdit,
    pub before: Vec<String>,
    pub picked: Vec<String>,
    /// Why the last tick didn't take (too many assignees).
    pub note: Option<String>,
}

impl Picker {
    pub fn new(facet: Facet, current: Vec<String>) -> Picker {
        let mut filter = TextEdit::new("");
        filter.focused = true;
        Picker { facet, filter, before: current.clone(), picked: current, note: None }
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.picked.iter().any(|p| p.eq_ignore_ascii_case(id))
    }

    /// Ticks or unticks `id`. The milestone is one choice: ticking one unticks the other, and
    /// `""` is "No milestone".
    pub fn toggle(&mut self, id: &str) {
        self.note = None;
        if self.facet == Facet::Milestone {
            self.picked = if id.is_empty() || self.is_on(id) { Vec::new() } else { vec![id.to_owned()] };
            return;
        }
        if self.is_on(id) {
            self.picked.retain(|p| !p.eq_ignore_ascii_case(id));
        } else if self.facet == Facet::Assignees && self.picked.len() >= MAX_ASSIGNEES {
            self.note = Some(format!("GitHub allows up to {MAX_ASSIGNEES} assignees: untick someone first."));
        } else {
            self.picked.push(id.to_owned());
        }
    }

    /// Every word typed is in the choice's name or its second line (any case).
    pub fn matches(&self, c: &Choice) -> bool {
        let q = self.filter.text.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let hay = format!("{} {}", c.name, c.note).to_lowercase();
        q.split_whitespace().all(|w| hay.contains(w))
    }

    /// The choices to list, in the repository's order: those the filter keeps, with what's
    /// ticked but not among them (a label beyond the first 100, an assignee who left) first, so
    /// it can be unticked. `hidden` (a pull request's author, for reviewers) is left out.
    pub fn shown(&self, choices: &[Choice], hidden: Option<&str>) -> Vec<Choice> {
        let known = |id: &str| choices.iter().any(|c| c.id.eq_ignore_ascii_case(id));
        let extra = self.picked.iter().filter(|p| !known(p)).map(|p| Choice::plain(p));
        extra.chain(choices.iter().cloned())
            .filter(|c| hidden.is_none_or(|h| !c.id.eq_ignore_ascii_case(h)))
            .filter(|c| self.matches(c))
            .collect()
    }

    /// What's ticked now and wasn't when the picker opened.
    pub fn added(&self) -> Vec<String> {
        self.picked.iter().filter(|p| !self.before.iter().any(|b| b.eq_ignore_ascii_case(p))).cloned().collect()
    }

    /// What was ticked when the picker opened and isn't now.
    pub fn removed(&self) -> Vec<String> {
        self.before.iter().filter(|b| !self.picked.iter().any(|p| p.eq_ignore_ascii_case(b))).cloned().collect()
    }

    /// Something changed since the picker opened (the order doesn't count).
    pub fn changed(&self) -> bool {
        !self.added().is_empty() || !self.removed().is_empty()
    }

    /// The write Apply sends, or `None` when nothing changed. `milestones` gives the title of the
    /// milestone picked.
    pub fn change(&self, milestones: &[Choice]) -> Option<Change> {
        if !self.changed() {
            return None;
        }
        Some(match self.facet {
            Facet::Labels => Change::Labels(self.picked.clone()),
            Facet::Assignees => Change::Assignees(self.picked.clone()),
            Facet::Milestone => Change::Milestone(self.picked.first().and_then(|id| {
                let number = id.parse::<u32>().ok()?;
                let title = milestones.iter().find(|m| m.id == *id).map(|m| m.name.clone()).unwrap_or_else(|| format!("#{number}"));
                Some((number, title))
            })),
            Facet::Reviewers => Change::Reviewers { add: self.added(), remove: self.removed() },
        })
    }
}

/// What the open conversation's triage is doing.
#[derive(Default)]
pub struct Edits {
    /// The picker open in the details block.
    pub picker: Option<Picker>,
    /// The title and the description being edited.
    pub title: Option<TextEdit>,
    pub body: Option<TextEdit>,
    /// The change on its way to GitHub ([`Change::part`]).
    pub saving: Option<&'static str>,
    /// GitHub refused the last change: what it was about ([`Change::part`]) and why. Said once,
    /// where the change was made; the old values stay.
    pub error: Option<(&'static str, String)>,
}

impl Edits {
    pub fn saving(&self, part: &str) -> bool {
        self.saving == Some(part)
    }

    pub fn error(&self, part: &str) -> Option<&str> {
        self.error.as_ref().filter(|(p, _)| *p == part).map(|(_, e)| e.as_str())
    }
}

/// The labels and assignees chosen in the New issue form, and its open picker.
#[derive(Default)]
pub struct Picks {
    /// The repository they're for (the form's).
    pub repo: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub picker: Option<Picker>,
}

impl Picks {
    pub fn for_repo(repo: &str) -> Picks {
        Picks { repo: repo.to_owned(), ..Picks::default() }
    }

    fn values(&self, f: Facet) -> Vec<String> {
        match f {
            Facet::Labels => self.labels.clone(),
            _ => self.assignees.clone(),
        }
    }
}

/// A new issue's write with the labels and assignees picked in the form.
pub fn with_picks(w: github::Write, picks: &Picks) -> github::Write {
    match w {
        github::Write::Issue { title, body, .. } => github::Write::Issue { title, body, labels: picks.labels.clone(), assignees: picks.assignees.clone() },
        other => other,
    }
}

/// Where a picker lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// The details block of the open conversation.
    Thread,
    /// The New issue form.
    Form,
}

// ------------------------------------------------------------------ writes

/// One change sent to GitHub: a kind of edit, so the toast can say what changed.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Labels(Vec<String>),
    Assignees(Vec<String>),
    /// The milestone's number and title (only the number is sent); `None` removes it.
    Milestone(Option<(u32, String)>),
    /// Reviewers to ask, and requests to take back.
    Reviewers { add: Vec<String>, remove: Vec<String> },
    Title(String),
    Body(String),
}

impl Change {
    /// What it's about: the details block's section, or the title or description.
    pub fn part(&self) -> &'static str {
        match self {
            Change::Labels(_) => "labels",
            Change::Assignees(_) => "assignees",
            Change::Milestone(_) => "milestone",
            Change::Reviewers { .. } => "reviewers",
            Change::Title(_) => "title",
            Change::Body(_) => "body",
        }
    }

    /// The requests that make it, in order: `(method, path, body)`. An issue's own fields are
    /// one `PATCH /repos/{r}/issues/{n}` (pull requests too); reviewers are asked with `POST` and
    /// taken back with `DELETE` on `/repos/{r}/pulls/{n}/requested_reviewers`.
    pub fn requests(&self, full: &str, number: u32) -> Vec<(&'static str, String, Value)> {
        let issue = format!("/repos/{full}/issues/{number}");
        let reviewers = format!("/repos/{full}/pulls/{number}/requested_reviewers");
        match self {
            Change::Labels(l) => vec![("PATCH", issue, json!({ "labels": l }))],
            Change::Assignees(a) => vec![("PATCH", issue, json!({ "assignees": a }))],
            Change::Milestone(m) => vec![("PATCH", issue, json!({ "milestone": m.as_ref().map(|(n, _)| *n) }))],
            Change::Title(t) => vec![("PATCH", issue, json!({ "title": t }))],
            Change::Body(b) => vec![("PATCH", issue, json!({ "body": b }))],
            Change::Reviewers { add, remove } => {
                let mut out = Vec::new();
                if !remove.is_empty() {
                    out.push(("DELETE", reviewers.clone(), json!({ "reviewers": remove })));
                }
                if !add.is_empty() {
                    out.push(("POST", reviewers, json!({ "reviewers": add })));
                }
                out
            }
        }
    }
}

/// What an issue or pull request is now, after a change: read from GitHub's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    Labels(Vec<(String, f32)>),
    Assignees(Vec<String>),
    Milestone(Option<(u32, String)>),
    /// The reviewers still asked.
    Reviewers(Vec<String>),
    Title(String),
    Body(String),
}

impl Applied {
    /// From GitHub's answer to the last request (the issue, or the pull request); what was asked
    /// when the answer doesn't say (then the reviewers listed are only those just asked).
    pub fn from_answer(change: &Change, v: &Value) -> Applied {
        let has = |k: &str| v.get(k).is_some();
        match change {
            Change::Labels(asked) if !has("labels") => Applied::Labels(asked.iter().map(|l| (l.clone(), 250.0)).collect()),
            Change::Labels(_) => Applied::Labels(v.get("labels").and_then(Value::as_array).into_iter().flatten()
                .map(|l| (s(l, "name"), hex_hue(&s(l, "color")))).filter(|(n, _)| !n.is_empty()).collect()),
            Change::Assignees(asked) if !has("assignees") => Applied::Assignees(asked.clone()),
            Change::Assignees(_) => Applied::Assignees(logins(v, "assignees")),
            Change::Milestone(asked) if !has("milestone") => Applied::Milestone(asked.clone()),
            Change::Milestone(_) => Applied::Milestone(milestone_of(v)),
            Change::Reviewers { add, .. } if !has("requested_reviewers") => Applied::Reviewers(add.clone()),
            Change::Reviewers { .. } => Applied::Reviewers(logins(v, "requested_reviewers")),
            Change::Title(t) => Applied::Title(Some(s(v, "title")).filter(|x| !x.is_empty()).unwrap_or_else(|| t.clone())),
            Change::Body(b) => Applied::Body(if has("body") { s(v, "body") } else { b.clone() }),
        }
    }

    /// The toast: what changed on `#number`.
    pub fn toast(&self, number: u32, change: &Change) -> String {
        let list = |v: &[String]| v.join(", ");
        match (self, change) {
            (Applied::Labels(l), _) if l.is_empty() => format!("Removed the labels of #{number}"),
            (Applied::Labels(l), _) => format!("Labels of #{number}: {}", list(&l.iter().map(|x| x.0.clone()).collect::<Vec<_>>())),
            (Applied::Assignees(a), _) if a.is_empty() => format!("#{number} is assigned to nobody now"),
            (Applied::Assignees(a), _) => format!("Assigned #{number} to {}", list(a)),
            (Applied::Milestone(None), _) => format!("Removed the milestone of #{number}"),
            (Applied::Milestone(Some((_, t))), _) => format!("#{number} is in the milestone {t}"),
            (Applied::Reviewers(_), Change::Reviewers { add, remove }) => match (add.is_empty(), remove.is_empty()) {
                (false, true) => format!("Asked {} to review #{number}", list(add)),
                (true, false) => format!("{} no longer asked to review #{number}", list(remove)),
                _ => format!("Reviewers of #{number}: asked {}, no longer {}", list(add), list(remove)),
            },
            (Applied::Reviewers(_), _) => format!("Reviewers of #{number} changed"),
            (Applied::Title(_), _) => format!("Title of #{number} changed"),
            (Applied::Body(_), _) => format!("Description of #{number} saved"),
        }
    }
}

/// The key a triage write runs under, so it doesn't make the row's Close or Merge wait.
pub fn busy_key(repo: &str, number: u32) -> String {
    format!("{}#{number}:triage", repo.to_ascii_lowercase())
}

/// Whether you may edit the title and description: you wrote it, or it's in one of your (or
/// your organizations') repositories. GitHub has the last word; a refusal is shown.
pub fn may_edit(s: &App, o: &Open, t: &Thread) -> bool {
    let me = s.data.profile.login.as_str();
    (!me.is_empty() && t.author.eq_ignore_ascii_case(me)) || s.data.repos.iter().any(|r| same_repo(&r.full_name, &o.repo))
}

// ------------------------------------------------------------------ the sample data

/// The sample data's choices: made up, and plausible next to the sample lists.
pub fn sample_choices(source: Source, d: &crate::model::Data) -> Choices {
    let items = match source {
        Source::Labels => {
            let mut l: Vec<Choice> = [
                ("bug", 25.0, "Something isn't working"),
                ("docs", 240.0, "Improvements or additions to documentation"),
                ("enhancement", 200.0, "New feature or request"),
                ("good first issue", 140.0, "Good for newcomers"),
                ("help wanted", 160.0, "Extra attention is needed"),
                ("performance", 40.0, "Faster, lighter, smoother"),
                ("question", 300.0, "Further information is requested"),
                ("research", 300.0, "Needs measuring before deciding"),
            ].iter().map(|(n, h, note)| Choice { id: (*n).into(), name: (*n).into(), hue: Some(*h), note: (*note).into() }).collect();
            l.sort_by_key(|c| c.name.to_lowercase());
            l
        }
        Source::People => {
            let mut p: Vec<String> = vec![d.profile.login.clone(), "claude".into(), "contributor42".into(), "hubot".into(), "octocat".into()];
            p.retain(|x| !x.is_empty());
            p.dedup();
            p.iter().map(|x| Choice::plain(x)).collect()
        }
        Source::Milestones => vec![
            Choice { id: "3".into(), name: "v0.3 · Mixed reality".into(), hue: None, note: "Due on 2026-12-15".into() },
            Choice { id: "4".into(), name: "Thesis submission".into(), hue: None, note: "Due on 2027-02-01".into() },
            Choice { id: "5".into(), name: "Backlog".into(), hue: None, note: "No due date".into() },
        ],
    };
    Choices { items, more: false }
}

/// Dresses the sample conversation with the row's labels and assignees, and asks you to review
/// the pull request that waits for you.
pub fn sample_thread(t: &mut Thread, d: &crate::model::Data, repo: &str, number: u32, pull: bool) {
    if pull {
        if let Some(p) = d.pulls.iter().find(|p| p.number == number && same_repo(&p.repo, repo)) {
            if !p.labels.is_empty() {
                t.labels = p.labels.clone();
            }
            t.assignees = p.assignees.clone();
            if p.review_requested && !t.requested.iter().any(|r| r.eq_ignore_ascii_case(&d.profile.login)) {
                t.requested.insert(0, d.profile.login.clone());
            }
            t.requested.retain(|r| !r.eq_ignore_ascii_case(&t.author));
        }
    } else if let Some(x) = d.issues.iter().find(|x| x.number == number && same_repo(&x.repo, repo)) {
        t.labels = x.labels.clone();
        t.assignees = x.assignees.clone();
    }
}

#[cfg(test)]
mod tests;

/// The requests as GitHub receives them, from a local stand-in for its API.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod http_tests;
