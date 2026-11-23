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

// ------------------------------------------------------------------ what the app does

impl App {
    pub(crate) fn picker_mut(&mut self, host: Host) -> Option<&mut Picker> {
        match host {
            Host::Thread => self.thread.as_mut().and_then(|t| t.edit.picker.as_mut()),
            Host::Form => self.issue_picks.picker.as_mut(),
        }
    }

    pub(crate) fn picker(&self, host: Host) -> Option<&Picker> {
        match host {
            Host::Thread => self.thread.as_ref().and_then(|t| t.edit.picker.as_ref()),
            Host::Form => self.issue_picks.picker.as_ref(),
        }
    }

    /// The repository a host's picker reads its choices from.
    pub(crate) fn picker_repo(&self, host: Host) -> Option<String> {
        match host {
            Host::Thread => self.thread.as_ref().map(|t| runs_page::full_repo_of(self, &t.repo)),
            Host::Form => self.form.as_ref().map(|f| f.repo.clone()),
        }
    }

    /// Opens the picker for `facet` (closing the one open), ticked as it is now, and asks for
    /// the repository's choices if they aren't here yet.
    pub(crate) fn open_picker(&mut self, host: Host, facet: Facet) {
        let current = match host {
            Host::Thread => {
                let Some(o) = self.thread.as_ref() else { return };
                let Load::Ready(t) = &o.load else { return };
                match facet {
                    Facet::Labels => t.labels.iter().map(|l| l.0.clone()).collect(),
                    Facet::Assignees => t.assignees.clone(),
                    Facet::Milestone => t.milestone.iter().map(|m| m.0.to_string()).collect(),
                    Facet::Reviewers => t.requested.clone(),
                }
            }
            Host::Form => self.issue_picks.values(facet),
        };
        let picker = Some(Picker::new(facet, current));
        match host {
            Host::Thread => if let Some(o) = self.thread.as_mut() {
                o.edit.picker = picker;
                o.edit.error = None;
            },
            Host::Form => self.issue_picks.picker = picker,
        }
        // Choices read before show at once and are read again behind the picker (with their tag,
        // so an unchanged list costs a 304).
        if let Some(repo) = self.picker_repo(host) {
            let again = matches!(self.choices.get(&repo, Source::of(facet)), Some(Load::Ready(_)));
            self.load_choices(&repo, Source::of(facet), again);
        }
    }

    pub(crate) fn close_picker(&mut self, host: Host) {
        match host {
            Host::Thread => if let Some(o) = self.thread.as_mut() { o.edit.picker = None },
            Host::Form => self.issue_picks.picker = None,
        }
    }

    pub(crate) fn toggle_choice(&mut self, host: Host, id: &str) {
        if let Some(p) = self.picker_mut(host) {
            p.toggle(id);
        }
    }

    /// Reads a repository's choices from GitHub (`again`: even if they're here), or the made-up
    /// ones of the sample data.
    pub(crate) fn load_choices(&mut self, full: &str, source: Source, again: bool) {
        if self.data.sample {
            if self.choices.get(full, source).is_none() {
                let c = sample_choices(source, &self.data);
                self.choices.put(full, source, c);
            }
            return;
        }
        let Some(epoch) = self.choices.ask(full, source, again) else { return };
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let full = full.to_owned();
        let asked = full.clone();
        crate::net::spawn(h, move || async move { github::load_choices(&cfg, &asked, source).await }, move |s: &mut App, r| {
            // Another account: everything was forgotten. A newer request: this one is old.
            if s.load_gen != account {
                return;
            }
            s.choices.answer(&full, source, epoch, r);
        });
    }

    /// Apply: sends the picker's change (or, in the New issue form, keeps it for the form).
    pub(crate) fn apply_picker(&mut self, host: Host) {
        let Some(repo) = self.picker_repo(host) else { return };
        let milestones = match self.choices.get(&repo, Source::Milestones) {
            Some(Load::Ready(c)) => c.items.clone(),
            _ => Vec::new(),
        };
        let Some(p) = self.picker(host).cloned() else { return };
        match host {
            Host::Form => {
                match p.facet {
                    Facet::Labels => self.issue_picks.labels = p.picked,
                    _ => self.issue_picks.assignees = p.picked,
                }
                self.issue_picks.picker = None;
            }
            Host::Thread => {
                let Some(change) = p.change(&milestones) else {
                    self.close_picker(host);
                    return;
                };
                self.send_change(change);
            }
        }
    }

    /// Sends a change of the open conversation; it says when GitHub answers.
    pub(crate) fn send_change(&mut self, change: Change) {
        if crate::actions::blocked(self).is_some() {
            return;
        }
        let Some(o) = self.thread.as_mut() else { return };
        if o.edit.saving.is_some() {
            return;
        }
        o.edit.saving = Some(change.part());
        o.edit.error = None;
        let (repo, number) = (o.repo.clone(), o.number);
        let w = github::Write::Triage { number, change };
        #[cfg(test)]
        self.choices.sent.push((repo.clone(), w.clone()));
        self.write(&repo, w);
    }

    pub(crate) fn edit_title(&mut self) {
        if let Some(o) = self.thread.as_mut()
            && let Load::Ready(t) = &o.load
        {
            let mut e = TextEdit::new(t.title.clone());
            e.focused = true;
            o.edit.title = Some(e);
            o.edit.error = None;
        }
    }

    pub(crate) fn edit_body(&mut self) {
        if let Some(o) = self.thread.as_mut()
            && let Load::Ready(t) = &o.load
        {
            let mut e = TextEdit::new(t.entries.first().map(|e| e.body.clone()).unwrap_or_default());
            e.focused = true;
            o.edit.body = Some(e);
            o.edit.error = None;
        }
    }

    /// Save on the title being edited; an unchanged title just closes the field.
    pub(crate) fn save_title(&mut self) {
        let Some(o) = self.thread.as_mut() else { return };
        let (Some(e), Load::Ready(t)) = (&o.edit.title, &o.load) else { return };
        let title = e.text.trim().to_owned();
        if title.is_empty() {
            return;
        }
        if title == t.title {
            o.edit.title = None;
            return;
        }
        self.send_change(Change::Title(title));
    }

    /// Save on the description being edited; an unchanged one just closes the editor.
    pub(crate) fn save_body(&mut self) {
        let Some(o) = self.thread.as_mut() else { return };
        let (Some(e), Load::Ready(t)) = (&o.edit.body, &o.load) else { return };
        let body = e.text.trim_end().to_owned();
        if t.entries.first().is_some_and(|d| d.body.trim_end() == body) {
            o.edit.body = None;
            return;
        }
        self.send_change(Change::Body(body));
    }

    /// The labels (with their colours) and assignees picked in the New issue form for `full`,
    /// taken for the row of the issue just created.
    pub(crate) fn picked_for_new_issue(&mut self, full: &str) -> (Vec<(String, f32)>, Vec<String>) {
        if !same_repo(&self.issue_picks.repo, full) {
            return (Vec::new(), Vec::new());
        }
        let picks = std::mem::take(&mut self.issue_picks);
        let labels = picks.labels.iter().map(|l| (l.clone(), self.choices.hue(full, l).unwrap_or(250.0))).collect();
        (labels, picks.assignees)
    }

    /// A finished change: the conversation shows it, and a toast says it.
    pub(crate) fn apply_triaged(&mut self, full: &str, number: u32, change: Change, applied: Applied) {
        if let Some(o) = self.thread.as_mut().filter(|o| o.is(full, number)) {
            o.edit.saving = None;
            o.edit.error = None;
            match &applied {
                Applied::Title(_) => o.edit.title = None,
                Applied::Body(_) => o.edit.body = None,
                _ => {
                    if o.edit.picker.as_ref().is_some_and(|p| p.facet.key() == change.part()) {
                        o.edit.picker = None;
                    }
                }
            }
            if let Load::Ready(t) = &mut o.load {
                match &applied {
                    Applied::Labels(l) => t.labels = l.clone(),
                    Applied::Assignees(a) => t.assignees = a.clone(),
                    Applied::Milestone(m) => t.milestone = m.clone(),
                    Applied::Reviewers(r) => t.requested = r.clone(),
                    Applied::Title(x) => t.title = x.clone(),
                    Applied::Body(b) => if let Some(d) = t.entries.first_mut() { d.body = b.clone() },
                }
            }
        }
        // The list rows are changed by `apply_done` (`replay::Change::Triaged`).
        let msg = applied.toast(number, &change);
        self.notify(msg);
    }

    /// A change GitHub refused: said once where it was made, and the old values stay. A change
    /// made of several requests (reviewers asked and taken back) may have half happened, so the
    /// conversation is read again. False when that conversation isn't open any more.
    pub(crate) fn triage_failed(&mut self, full: &str, number: u32, part: &'static str, partial: bool, e: &str) -> bool {
        let Some(o) = self.thread.as_mut().filter(|o| o.is(full, number)) else { return false };
        o.edit.saving = None;
        o.edit.error = Some((part, explain(e)));
        if partial {
            self.load_thread();
        }
        true
    }
}

/// GitHub's refusal, in words for the details block.
pub fn explain(e: &str) -> String {
    if e.contains("(403)") || e.contains("(404)") {
        format!("{e} Your token may not have triage or write access to this repository. Nothing was changed.")
    } else {
        format!("{e} Nothing was changed.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_people_and_milestones_are_read_from_githubs_answers() {
        let labels = parse_labels(&json!([
            { "name": "bug", "color": "d73a4a", "description": "Something isn't working" },
            { "name": "", "color": "ffffff" },
            { "name": "docs", "color": "0075ca", "description": null },
        ]));
        assert_eq!(labels.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["bug", "docs"], "a label with no name is left out");
        assert!((labels[0].hue.unwrap() - 354.0).abs() < 2.0);
        assert_eq!((labels[0].note.as_str(), labels[1].note.as_str()), ("Something isn't working", ""));

        let people = parse_people(&json!([{ "login": "alice" }, { "login": "" }, { "id": 3 }, { "login": "bob" }]));
        assert_eq!(people.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["alice", "bob"]);

        let ms = parse_milestones(&json!([
            { "number": 3, "title": "v1.0", "due_on": "2026-12-01T08:00:00Z" },
            { "number": 4, "title": "Later", "due_on": null },
            { "title": "no number" },
        ]));
        assert_eq!(ms.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.note.as_str())).collect::<Vec<_>>(),
            [("3", "v1.0", "Due on 2026-12-01"), ("4", "Later", "No due date")]);

        assert_eq!(milestone_of(&json!({ "milestone": { "number": 3, "title": "v1.0" } })), Some((3, "v1.0".into())));
        assert_eq!(milestone_of(&json!({ "milestone": null })), None);
        assert_eq!(logins(&json!({ "requested_reviewers": [{ "login": "carol" }, { "login": "dave" }] }), "requested_reviewers"), ["carol", "dave"]);
        assert_eq!(Source::Labels.path("o/r"), "/repos/o/r/labels?per_page=100");
        assert_eq!(Source::People.path("o/r"), "/repos/o/r/assignees?per_page=100");
        assert_eq!(Source::Milestones.path("o/r"), "/repos/o/r/milestones?state=open&per_page=100");
        assert_eq!((Source::of(Facet::Reviewers), Source::of(Facet::Assignees)), (Source::People, Source::People));
    }

    #[test]
    fn each_reviewer_keeps_their_latest_verdict() {
        let r = reviewed_of(&json!([
            { "user": { "login": "carol" }, "state": "APPROVED", "submitted_at": "2026-11-04T10:00:00Z" },
            { "user": { "login": "carol" }, "state": "COMMENTED", "submitted_at": "2026-11-05T10:00:00Z" },
            { "user": { "login": "dave" }, "state": "CHANGES_REQUESTED", "submitted_at": "2026-11-01T10:00:00Z" },
            { "user": { "login": "dave" }, "state": "APPROVED", "submitted_at": "2026-11-03T10:00:00Z" },
            { "user": { "login": "erin" }, "state": "COMMENTED", "submitted_at": "2026-11-02T10:00:00Z" },
            { "user": { "login": "me" }, "state": "PENDING" },
            { "user": { "login": "frank" }, "state": "DISMISSED", "submitted_at": "2026-11-02T14:00:00Z" },
        ]));
        assert_eq!(r, [("dave".to_owned(), Verdict::Approved), ("erin".to_owned(), Verdict::Commented), ("carol".to_owned(), Verdict::Approved)],
            "a comment after an approval doesn't undo it; pending and dismissed reviews don't count");
    }

    fn choice(id: &str, name: &str, note: &str) -> Choice {
        Choice { id: id.into(), name: name.into(), hue: None, note: note.into() }
    }

    #[test]
    fn the_picker_filters_ticks_and_says_what_changed() {
        let choices = [choice("bug", "bug", "Something isn't working"), choice("docs", "docs", "Documentation"), choice("good first issue", "good first issue", "For newcomers")];
        let mut p = Picker::new(Facet::Labels, vec!["bug".into(), "wontfix".into()]);
        assert!(!p.changed() && p.change(&[]).is_none());
        // What's ticked but not offered comes first, so it can be unticked.
        assert_eq!(p.shown(&choices, None).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["wontfix", "bug", "docs", "good first issue"]);
        // Every word, in the name or the description, any case.
        p.filter = TextEdit::new("NEW");
        assert_eq!(p.shown(&choices, None).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["good first issue"]);
        p.filter = TextEdit::new("doc ment");
        assert_eq!(p.shown(&choices, None).len(), 1);
        p.filter = TextEdit::new("");
        p.toggle("docs");
        p.toggle("BUG");
        assert_eq!((p.added(), p.removed()), (vec!["docs".to_owned()], vec!["bug".to_owned()]));
        assert_eq!(p.change(&[]), Some(Change::Labels(vec!["wontfix".into(), "docs".into()])));
        // Back to how it was, in another order: nothing to send.
        p.toggle("docs");
        p.toggle("bug");
        assert!(!p.changed());

        // The milestone is one choice, or none.
        let ms = [choice("3", "v1.0", ""), choice("4", "v2.0", "")];
        let mut m = Picker::new(Facet::Milestone, vec!["3".into()]);
        m.toggle("4");
        assert_eq!(m.picked, ["4"]);
        assert_eq!(m.change(&ms), Some(Change::Milestone(Some((4, "v2.0".into())))));
        m.toggle("");
        assert!(m.picked.is_empty());
        assert_eq!(m.change(&ms), Some(Change::Milestone(None)));

        // Reviewers: who to ask and whom to stop asking; the author can't review their own.
        let people = [choice("alice", "alice", ""), choice("bob", "bob", ""), choice("carol", "carol", "")];
        let mut r = Picker::new(Facet::Reviewers, vec!["bob".into()]);
        assert_eq!(r.shown(&people, Some("alice")).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["bob", "carol"]);
        r.toggle("carol");
        r.toggle("bob");
        assert_eq!(r.change(&[]), Some(Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }));

        // At most ten assignees.
        let mut a = Picker::new(Facet::Assignees, (0..10).map(|n| format!("u{n}")).collect());
        a.toggle("one-more");
        assert_eq!(a.picked.len(), 10);
        assert!(a.note.as_deref().is_some_and(|n| n.contains("up to 10")));
        a.toggle("u0");
        assert!(a.note.is_none() && a.picked.len() == 9);
    }

    #[test]
    fn each_change_is_its_own_request() {
        let req = |c: Change| c.requests("o/r", 7).into_iter().map(|(m, p, b)| (m, p, b.to_string())).collect::<Vec<_>>();
        let issue = "/repos/o/r/issues/7".to_owned();
        let rr = "/repos/o/r/pulls/7/requested_reviewers".to_owned();
        assert_eq!(req(Change::Labels(vec!["bug".into(), "ui".into()])), [("PATCH", issue.clone(), r#"{"labels":["bug","ui"]}"#.to_owned())]);
        assert_eq!(req(Change::Labels(Vec::new())), [("PATCH", issue.clone(), r#"{"labels":[]}"#.to_owned())], "no labels is a change too");
        assert_eq!(req(Change::Assignees(vec!["alice".into()])), [("PATCH", issue.clone(), r#"{"assignees":["alice"]}"#.to_owned())]);
        assert_eq!(req(Change::Milestone(Some((3, "v1.0".into())))), [("PATCH", issue.clone(), r#"{"milestone":3}"#.to_owned())]);
        assert_eq!(req(Change::Milestone(None)), [("PATCH", issue.clone(), r#"{"milestone":null}"#.to_owned())]);
        assert_eq!(req(Change::Title("New \"title\"".into())), [("PATCH", issue.clone(), r#"{"title":"New \"title\""}"#.to_owned())]);
        assert_eq!(req(Change::Body("# Hi\n".into())), [("PATCH", issue.clone(), r##"{"body":"# Hi\n"}"##.to_owned())]);
        assert_eq!(req(Change::Reviewers { add: vec!["carol".into()], remove: Vec::new() }), [("POST", rr.clone(), r#"{"reviewers":["carol"]}"#.to_owned())]);
        assert_eq!(req(Change::Reviewers { add: Vec::new(), remove: vec!["bob".into()] }), [("DELETE", rr.clone(), r#"{"reviewers":["bob"]}"#.to_owned())]);
        assert_eq!(req(Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }).len(), 2, "taken back, then asked");
        assert_eq!(busy_key("Octo/App", 7), "octo/app#7:triage");
        assert_ne!(busy_key("o/r", 7), crate::actions::busy_key("o/r", 7), "a triage write doesn't make Close wait");
    }

    #[test]
    fn githubs_answer_says_what_it_is_now() {
        let answer = json!({ "title": "T", "body": "B", "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "alice" }],
            "milestone": { "number": 3, "title": "v1.0" }, "requested_reviewers": [{ "login": "carol" }] });
        assert!(matches!(Applied::from_answer(&Change::Labels(vec!["x".into()]), &answer), Applied::Labels(l) if l.len() == 1 && l[0].0 == "bug"));
        assert_eq!(Applied::from_answer(&Change::Assignees(Vec::new()), &answer), Applied::Assignees(vec!["alice".into()]));
        assert_eq!(Applied::from_answer(&Change::Milestone(None), &answer), Applied::Milestone(Some((3, "v1.0".into()))));
        assert_eq!(Applied::from_answer(&Change::Reviewers { add: vec![], remove: vec![] }, &answer), Applied::Reviewers(vec!["carol".into()]));
        // No answer to read: what was asked.
        let none = Value::Null;
        assert_eq!(Applied::from_answer(&Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }, &none), Applied::Reviewers(vec!["carol".into()]));
        assert_eq!(Applied::from_answer(&Change::Title("New".into()), &none), Applied::Title("New".into()));
        assert_eq!(Applied::from_answer(&Change::Milestone(Some((4, "v2".into()))), &none), Applied::Milestone(Some((4, "v2".into()))));
        // The toasts say what changed.
        let c = Change::Reviewers { add: vec!["carol".into()], remove: vec![] };
        assert_eq!(Applied::Reviewers(vec!["carol".into()]).toast(7, &c), "Asked carol to review #7");
        assert_eq!(Applied::Labels(Vec::new()).toast(7, &Change::Labels(vec![])), "Removed the labels of #7");
        assert_eq!(Applied::Milestone(Some((3, "v1.0".into()))).toast(7, &Change::Milestone(None)), "#7 is in the milestone v1.0");
    }

    #[test]
    fn choices_are_kept_per_repository_and_late_answers_are_dropped() {
        let mut c = Cache::default();
        let first = c.ask("o/a", Source::Labels, false).expect("asked");
        assert!(c.ask("O/A", Source::Labels, false).is_none(), "on its way already");
        assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Loading)));
        // Retry: the first answer is now old.
        let second = c.ask("o/a", Source::Labels, true).expect("asked again");
        assert!(!c.answer("o/a", Source::Labels, first, Ok(Choices { items: vec![choice("old", "old", "")], more: false })));
        assert!(c.answer("o/a", Source::Labels, second, Ok(Choices { items: vec![choice("bug", "bug", "")], more: true })));
        assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Ready(l)) if l.items[0].id == "bug" && l.more));
        assert!(c.ask("o/a", Source::Labels, false).is_none(), "kept for the repository");
        assert!(c.get("o/b", Source::Labels).is_none() && c.get("o/a", Source::People).is_none(), "per repository and kind");
        // A failed retry keeps the list that was there.
        let again = c.ask("o/a", Source::Labels, true).unwrap();
        assert!(c.answer("o/a", Source::Labels, again, Err("offline".into())));
        assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Ready(_))));
        // Another account forgets everything; its answers no longer land.
        let late = c.ask("o/b", Source::People, false).unwrap();
        c.clear();
        assert!(!c.answer("o/b", Source::People, late, Ok(Choices::default())));
        assert!(c.get("o/a", Source::Labels).is_none());
    }
}

/// The requests as GitHub receives them, from a local stand-in for its API.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod http_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::github::{Done, Write as GhWrite};

    type Log = Arc<Mutex<Vec<(String, String, String)>>>;

    /// Answers each request with `answer(method, path, body)`, and keeps what it was asked.
    struct Mock {
        url: String,
        log: Log,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Mock {
        fn new(answer: impl Fn(&str, &str, &str) -> (u16, String, Option<String>) + Send + 'static) -> Mock {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (log, stop): (Log, _) = (Arc::default(), Arc::new(AtomicBool::new(false)));
            let (log2, stop2) = (log.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                while !stop2.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    stream.set_nonblocking(false).unwrap();
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // The head, then as much body as it announces.
                    let head_end = loop {
                        let n = stream.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break None;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break Some(at + 4);
                        }
                    };
                    let Some(head_end) = head_end else { continue };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    while buf.len() < head_end + len {
                        let n = stream.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let mut first = head.lines().next().unwrap_or("").split_whitespace();
                    let (method, path) = (first.next().unwrap_or("").to_owned(), first.next().unwrap_or("").to_owned());
                    let (status, reply, link) = answer(&method, &path, &body);
                    log2.lock().unwrap().push((method, path, body));
                    let link = link.map(|l| format!("Link: {l}\r\n")).unwrap_or_default();
                    let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{link}Content-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                }
            });
            Mock { url, log, stop, thread: Some(thread) }
        }

        fn cfg(&self) -> crate::config::Config {
            let mut cfg = crate::config::Config::from_json("{}").unwrap();
            cfg.api = self.url.clone();
            cfg.token = Some("test-token".into());
            cfg
        }

        fn asked(&self) -> Vec<(String, String, String)> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Drop for Mock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    #[test]
    fn every_change_reaches_github_as_its_api_expects() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let server = Mock::new(|method, path, body| {
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            match (method, path) {
                ("GET", p) if p.starts_with("/repos/o/r/labels") => (200, r#"[{"name":"bug","color":"d73a4a","description":"Broken"},{"name":"docs","color":"0075ca"}]"#.into(),
                    Some(r#"</repos/o/r/labels?per_page=100&page=2>; rel="next""#.into())),
                ("GET", p) if p.starts_with("/repos/o/r/assignees") => (200, r#"[{"login":"alice"},{"login":"bob"}]"#.into(), None),
                ("GET", p) if p.starts_with("/repos/o/r/milestones") => (200, r#"[{"number":3,"title":"v1.0","due_on":null}]"#.into(), None),
                // GitHub answers a PATCH with the issue as it is now.
                ("PATCH", "/repos/o/r/issues/7") => {
                    let labels: Vec<Value> = v["labels"].as_array().into_iter().flatten().map(|l| json!({ "name": l, "color": "d73a4a" })).collect();
                    let assignees: Vec<Value> = v["assignees"].as_array().into_iter().flatten().map(|l| json!({ "login": l })).collect();
                    let milestone = v["milestone"].as_u64().map(|n| json!({ "number": n, "title": "v1.0" })).unwrap_or(Value::Null);
                    (200, json!({ "number": 7, "title": v["title"].as_str().unwrap_or("Old"), "body": v["body"].as_str().unwrap_or(""),
                        "labels": labels, "assignees": assignees, "milestone": milestone }).to_string(), None)
                }
                ("PATCH", "/repos/o/r/issues/9") => (403, r#"{"message":"Must have admin rights to Repository."}"#.into(), None),
                ("DELETE", "/repos/o/r/pulls/8/requested_reviewers") => (200, r#"{"number":8,"requested_reviewers":[{"login":"dave"}]}"#.into(), None),
                ("POST", "/repos/o/r/pulls/8/requested_reviewers") => (201, r#"{"number":8,"requested_reviewers":[{"login":"dave"},{"login":"carol"}]}"#.into(), None),
                ("POST", "/repos/o/r/issues") => (201, r#"{"number":50,"html_url":"https://github.com/o/r/issues/50"}"#.into(), None),
                _ => (404, r#"{"message":"Not Found"}"#.into(), None),
            }
        });
        let cfg = server.cfg();
        let run = |w: GhWrite| pollster::block_on(github::run(&cfg, "o/r", w));

        // The choices: the first 100, and whether GitHub has more.
        let labels = pollster::block_on(github::load_choices(&cfg, "o/r", Source::Labels)).unwrap();
        assert_eq!((labels.items.len(), labels.more, labels.items[0].note.as_str()), (2, true, "Broken"));
        let people = pollster::block_on(github::load_choices(&cfg, "o/r", Source::People)).unwrap();
        assert_eq!((people.items.len(), people.more), (2, false));
        let ms = pollster::block_on(github::load_choices(&cfg, "o/r", Source::Milestones)).unwrap();
        assert_eq!(ms.items[0].id, "3");

        // Each change, and what GitHub says it is now.
        let triaged = |d: Done| match d { Done::Triaged { applied, .. } => applied, other => panic!("{other:?}") };
        assert!(matches!(triaged(run(GhWrite::Triage { number: 7, change: Change::Labels(vec!["bug".into(), "docs".into()]) }).unwrap()), Applied::Labels(l) if l.len() == 2));
        assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Assignees(vec!["alice".into()]) }).unwrap()), Applied::Assignees(vec!["alice".into()]));
        assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Milestone(Some((3, "v1.0".into()))) }).unwrap()), Applied::Milestone(Some((3, "v1.0".into()))));
        assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Milestone(None) }).unwrap()), Applied::Milestone(None));
        assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Title("New title".into()) }).unwrap()), Applied::Title("New title".into()));
        assert_eq!(triaged(run(GhWrite::Triage { number: 7, change: Change::Body("New body".into()) }).unwrap()), Applied::Body("New body".into()));
        assert_eq!(triaged(run(GhWrite::Triage { number: 8, change: Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] } }).unwrap()),
            Applied::Reviewers(vec!["dave".into(), "carol".into()]));
        // A new issue with its labels and assignees.
        assert!(matches!(run(GhWrite::Issue { title: "T".into(), body: String::new(), labels: vec!["bug".into()], assignees: vec!["alice".into()] }).unwrap(), Done::Issue(c) if c.number == 50));
        // A refusal says why, with GitHub's words.
        let err = run(GhWrite::Triage { number: 9, change: Change::Labels(vec!["bug".into()]) }).unwrap_err();
        assert!(err.contains("(403)") && err.contains("admin rights"), "{err}");
        assert!(explain(&err).ends_with("Nothing was changed."));

        let asked: Vec<(String, String, Value)> = server.asked().into_iter().map(|(m, p, b)| (m, p, serde_json::from_str(&b).unwrap_or(Value::Null))).collect();
        let writes: Vec<_> = asked.iter().filter(|(m, ..)| m != "GET").cloned().collect();
        assert_eq!(writes, [
            ("PATCH".to_owned(), "/repos/o/r/issues/7".to_owned(), json!({ "labels": ["bug", "docs"] })),
            ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "assignees": ["alice"] })),
            ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "milestone": 3 })),
            ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "milestone": null })),
            ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "title": "New title" })),
            ("PATCH".into(), "/repos/o/r/issues/7".into(), json!({ "body": "New body" })),
            ("DELETE".into(), "/repos/o/r/pulls/8/requested_reviewers".into(), json!({ "reviewers": ["bob"] })),
            ("POST".into(), "/repos/o/r/pulls/8/requested_reviewers".into(), json!({ "reviewers": ["carol"] })),
            ("POST".into(), "/repos/o/r/issues".into(), json!({ "title": "T", "body": "", "labels": ["bug"], "assignees": ["alice"] })),
            ("PATCH".into(), "/repos/o/r/issues/9".into(), json!({ "labels": ["bug"] })),
        ]);
        assert!(asked.iter().any(|(m, p, _)| m == "GET" && p == "/repos/o/r/milestones?state=open&per_page=100"));
        // A new issue without labels or assignees doesn't send them.
        assert_eq!(github::issue_body("T", "B", &[], &[]), json!({ "title": "T", "body": "B" }));
    }

    #[test]
    fn the_conversation_reads_assignees_milestone_and_reviewers_from_its_answers() {
        let issue = json!({ "title": "T", "state": "open", "created_at": "2026-11-01T10:00:00Z", "body": "", "user": { "login": "alice" },
            "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "bob" }, { "login": "carol" }],
            "milestone": { "number": 3, "title": "v1.0" }, "pull_request": { "url": "x" } });
        let detail = json!({ "draft": false, "merged": false, "head": { "ref": "f" }, "base": { "ref": "main" }, "requested_reviewers": [{ "login": "dave" }] });
        let reviews = json!([{ "user": { "login": "erin" }, "state": "APPROVED", "body": "", "submitted_at": "2026-11-02T10:00:00Z" }]);
        let t = github::parse_thread(&issue, &json!([]), Some((&detail, &reviews)));
        assert_eq!(t.assignees, ["bob", "carol"]);
        assert_eq!(t.milestone, Some((3, "v1.0".into())));
        assert_eq!(t.requested, ["dave"]);
        assert_eq!(t.reviewed, [("erin".to_owned(), Verdict::Approved)]);
        // An issue: no reviewers; no milestone is none.
        let issue = json!({ "title": "T", "state": "open", "created_at": "2026-11-01T10:00:00Z", "user": { "login": "alice" }, "milestone": null });
        let t = github::parse_thread(&issue, &json!([]), None);
        assert!(t.requested.is_empty() && t.reviewed.is_empty() && t.milestone.is_none() && t.assignees.is_empty());
    }

    #[test]
    fn pull_requests_waiting_for_review_are_flagged_once() {
        use crate::model::{Pull, PullState};
        let pull = |n: u32, repo: &str| Pull { number: n, title: format!("#{n}"), repo: repo.into(), author: "x".into(), state: PullState::Open, checks: None,
            updated_at: 0, comments: 0, url: String::new(), labels: Vec::new(), assignees: Vec::new(), review_requested: false };
        let mut list = vec![pull(1, "o/a"), pull(2, "o/a")];
        let mut total = Some(2);
        let waiting = vec![Pull { review_requested: true, ..pull(2, "O/A") }, Pull { review_requested: true, ..pull(3, "o/b") }];
        github::merge_review_requested(&mut list, waiting, &mut total);
        assert_eq!(list.iter().map(|p| (p.number, p.review_requested)).collect::<Vec<_>>(), [(1, false), (2, true), (3, true)], "#2 isn't added twice");
        assert_eq!(total, Some(3), "the one added counts");
        assert!(github::parse_pull(&json!({ "assignees": [{ "login": "a" }] })).assignees == ["a"]);
    }
}
