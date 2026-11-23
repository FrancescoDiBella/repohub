//! Narrowing the Pull requests and Issues pages: a search field and chips for the repository,
//! the label and whose they are. Each page has its own [`ListFilter`]; nothing is asked of
//! GitHub, the lists already loaded are filtered as you type.
//!
//! The filters are kept between visits, and between runs in the preferences (`"filters"`, next
//! to the place: [`Saved`]); **Clear** resets them.
use rune::{El, TextEdit, col, row, text};
use serde_json::{Value, json};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextInput};

use crate::model::{Issue, Pull, PullState};
use crate::{App, icons};

/// Which page's filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Pulls,
    Issues,
}

/// What a list is narrowed to. Empty means everything.
pub struct ListFilter {
    /// Words from the title, or a number (`#12` or `12`).
    pub search: TextEdit,
    pub repo: Option<String>,
    pub label: Option<String>,
    /// Pull requests you opened; issues assigned to you.
    pub mine: bool,
    /// Pull requests only: drafts, and ones whose checks failed.
    pub drafts: bool,
    pub failing: bool,
    /// Pull requests only: ones where your review is requested.
    pub review: bool,
}

impl Default for ListFilter {
    fn default() -> Self {
        ListFilter { search: TextEdit::new(""), repo: None, label: None, mine: false, drafts: false, failing: false, review: false }
    }
}

impl ListFilter {
    pub fn is_empty(&self) -> bool {
        self.search.text.trim().is_empty() && self.repo.is_none() && self.label.is_none() && !self.mine && !self.drafts && !self.failing && !self.review
    }

    /// What's kept in the preferences.
    pub fn to_json(&self) -> Value {
        json!({
            "search": self.search.text.trim(),
            "repo": self.repo,
            "label": self.label,
            "mine": self.mine,
            "drafts": self.drafts,
            "failing": self.failing,
            "review": self.review,
        })
    }

    /// A filter read back from the preferences; what's missing or odd is left empty.
    pub fn from_json(v: &Value) -> ListFilter {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
        let on = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        ListFilter {
            search: TextEdit::new(text("search").unwrap_or_default()),
            repo: text("repo"),
            label: text("label"),
            mine: on("mine"),
            drafts: on("drafts"),
            failing: on("failing"),
            review: on("review"),
        }
    }

    pub fn clear(&mut self) {
        *self = ListFilter::default();
    }

    /// Every word typed is in the title (any case), or what's typed is the number.
    fn text_matches(&self, number: u32, title: &str) -> bool {
        let q = self.search.text.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        if q.trim_start_matches('#').parse::<u32>().is_ok_and(|n| n == number) {
            return true;
        }
        let title = title.to_lowercase();
        q.split_whitespace().all(|w| title.contains(w))
    }

    fn common(&self, number: u32, title: &str, repo: &str, labels: &[(String, f32)]) -> bool {
        self.text_matches(number, title)
            && self.repo.as_deref().is_none_or(|r| r.eq_ignore_ascii_case(repo))
            && self.label.as_deref().is_none_or(|l| labels.iter().any(|(name, _)| name.eq_ignore_ascii_case(l)))
    }

    pub fn pull(&self, p: &Pull, me: &str) -> bool {
        self.common(p.number, &p.title, &p.repo, &p.labels)
            && (!self.mine || p.author.eq_ignore_ascii_case(me))
            && (!self.drafts || p.state == crate::model::PullState::Draft)
            && (!self.failing || p.checks == Some(crate::model::Checks::Failing))
            && (!self.review || p.review_requested)
    }

    pub fn issue(&self, x: &Issue, me: &str) -> bool {
        self.common(x.number, &x.title, &x.repo, &x.labels) && (!self.mine || x.assignees.iter().any(|a| a.eq_ignore_ascii_case(me)))
    }
}

/// The distinct values of `items`, most frequent first (ties by name), at most `max`.
fn top(items: impl Iterator<Item = String>, max: usize) -> Vec<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for item in items {
        match counts.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(&item)) {
            Some(c) => c.1 += 1,
            None => counts.push((item, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    counts.into_iter().take(max).map(|(name, _)| name).collect()
}

fn of(s: &mut App, which: Which) -> &mut ListFilter {
    match which {
        Which::Pulls => &mut s.pull_filter,
        Which::Issues => &mut s.issue_filter,
    }
}

/// Both pages' filters and the Pull requests page's state, as kept in the preferences.
#[derive(Debug, PartialEq)]
pub struct Saved {
    pub pulls: Value,
    pub issues: Value,
    /// The Pull requests page's state tab: `open`, `draft`, `merged`, `closed`.
    pub state: &'static str,
}

impl Saved {
    pub fn of(s: &App) -> Saved {
        Saved { pulls: s.pull_filter.to_json(), issues: s.issue_filter.to_json(), state: state_word(s.pr_filter) }
    }

    pub fn to_json(&self) -> Value {
        json!({ "pulls": self.pulls, "issues": self.issues, "state": self.state })
    }
}

fn state_word(p: PullState) -> &'static str {
    match p {
        PullState::Open => "open",
        PullState::Draft => "draft",
        PullState::Merged => "merged",
        PullState::Closed => "closed",
    }
}

impl App {
    /// Saves both pages' filters when they changed since last time (an account's, not the
    /// sample data's). Called when you move, when a chip is pressed, on the 30-second tick and
    /// when the app quits, so a search typed is kept too.
    pub(crate) fn save_filters(&mut self) {
        if self.data.sample {
            return;
        }
        let now = Saved::of(self).to_json().to_string();
        if self.saved_filters.as_deref() != Some(now.as_str()) {
            crate::prefs::set("filters", serde_json::from_str(&now).unwrap_or(Value::Null));
            self.saved_filters = Some(now);
        }
    }

    /// Puts back the filters saved last time.
    pub(crate) fn restore_filters(&mut self) {
        let Some(v) = crate::prefs::get("filters") else { return };
        if let Some(p) = v.get("pulls") {
            self.pull_filter = ListFilter::from_json(p);
        }
        if let Some(i) = v.get("issues") {
            self.issue_filter = ListFilter::from_json(i);
        }
        if let Some(st) = v.get("state").and_then(Value::as_str) {
            self.pr_filter = [PullState::Open, PullState::Draft, PullState::Merged, PullState::Closed].into_iter().find(|p| state_word(*p) == st).unwrap_or(PullState::Open);
        }
        self.saved_filters = Some(Saved::of(self).to_json().to_string());
    }

    /// The open pull requests that wait for your review (the status-bar line, the Overview's
    /// panel).
    pub(crate) fn show_review_requested(&mut self) {
        // What's typed in a conversation on screen isn't thrown away without asking.
        if self.ask_before_leaving(|s| s.show_review_requested()) {
            return;
        }
        self.go(crate::Page::Pulls);
        self.pr_filter = PullState::Open;
        self.pull_filter.clear();
        self.pull_filter.review = true;
        self.save_filters();
    }
}

/// The filter bar of a page. `repos` and `labels` are the ones in the list it filters.
pub fn bar(s: &App, k: &Kit, which: Which, repos: Vec<String>, labels: Vec<String>) -> El {
    let (f, tag, what) = match which {
        Which::Pulls => (&s.pull_filter, "pulls", "pull requests"),
        Which::Issues => (&s.issue_filter, "issues", "issues"),
    };
    let search = TextInput::new(&f.search).leading_icon(k.glyph.search).placeholder(format!("Search {what} by title or number…")).label(format!("Search {what}"))
        .on_edit(move |s: &mut App| &mut of(s, which).search).build(k).key(format!("{tag}-search")).grow(1.0).min_w(220.0);
    let mine_label = if which == Which::Pulls { "Opened by me" } else { "Assigned to me" };
    let chip = |label: String, on: bool, key: String| {
        Button::new(label).control_size(ControlSize::Small)
            // Outlined when off, filled when on: a choice, not a link.
            .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline })
            .build(k).key(key)
    };
    let mut top_row = row().w_full().gap(8.0).center_y().flex_wrap().child(search)
        .child(chip(mine_label.to_owned(), f.mine, format!("{tag}-mine")).on_press(move |s: &mut App| { let f = of(s, which); f.mine = !f.mine; s.save_filters(); }));
    if which == Which::Pulls {
        top_row = top_row
            .child(chip("Review requested".to_owned(), f.review, format!("{tag}-review")).on_press(|s: &mut App| { s.pull_filter.review = !s.pull_filter.review; s.save_filters(); }))
            .child(chip("Drafts".to_owned(), f.drafts, format!("{tag}-drafts")).on_press(|s: &mut App| { s.pull_filter.drafts = !s.pull_filter.drafts; s.save_filters(); }))
            .child(chip("Failing checks".to_owned(), f.failing, format!("{tag}-failing")).on_press(|s: &mut App| { s.pull_filter.failing = !s.pull_filter.failing; s.save_filters(); }));
    }
    if !f.is_empty() {
        top_row = top_row.child(Button::new("Clear").icon(&icons::CROSS).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| { of(s, which).clear(); s.save_filters(); }).build(k).key(format!("{tag}-clear")));
    }
    let mut out = col().w_full().gap(8.0).key(format!("{tag}-filters")).child(top_row);
    let group = |title: &str, values: Vec<String>, picked: &Option<String>, kind: &'static str| {
        let mut r = row().w_full().gap(6.0).center_y().flex_wrap()
            .child(text(title.to_owned()).font(k.ty.small_medium).color(k.c.text_muted).pr(4.0));
        for v in values {
            let on = picked.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&v));
            let value = v.clone();
            // Only the repository's own name on the chip: the owner is the same for most.
            let shown = if kind == "repo" { v.rsplit('/').next().unwrap_or(&v).to_owned() } else { v.clone() };
            r = r.child(chip(shown, on, format!("{tag}-{kind}-{v}")).on_press(move |s: &mut App| {
                let f = of(s, which);
                let slot = if kind == "repo" { &mut f.repo } else { &mut f.label };
                *slot = if on { None } else { Some(value.clone()) };
                s.save_filters();
            }));
        }
        r
    };
    // A choice that narrows nothing isn't offered.
    if repos.len() > 1 || f.repo.is_some() {
        out = out.child(group("Repository", repos, &f.repo, "repo"));
    }
    if !labels.is_empty() || f.label.is_some() {
        out = out.child(group("Label", labels, &f.label, "label"));
    }
    out
}

/// The repositories and labels to offer for `pulls`, most used first.
pub fn pull_choices<'a>(pulls: impl Iterator<Item = &'a Pull> + Clone) -> (Vec<String>, Vec<String>) {
    (top(pulls.clone().map(|p| p.repo.clone()), 12), top(pulls.flat_map(|p| p.labels.iter().map(|l| l.0.clone())), 16))
}

pub fn issue_choices<'a>(issues: impl Iterator<Item = &'a Issue> + Clone) -> (Vec<String>, Vec<String>) {
    (top(issues.clone().map(|x| x.repo.clone()), 12), top(issues.flat_map(|x| x.labels.iter().map(|l| l.0.clone())), 16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PullState;

    fn issue(number: u32, title: &str, repo: &str, labels: &[&str], assignees: &[&str]) -> Issue {
        Issue {
            number, title: title.into(), repo: repo.into(), label: None, created_at: 0, comments: 0, url: String::new(),
            labels: labels.iter().map(|l| ((*l).to_owned(), 0.0)).collect(), assignees: assignees.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn pull(number: u32, title: &str, repo: &str, author: &str) -> Pull {
        Pull { number, title: title.into(), repo: repo.into(), author: author.into(), state: PullState::Open, checks: None, updated_at: 0, comments: 0, url: String::new(), labels: Vec::new(), assignees: Vec::new(), review_requested: false }
    }

    #[test]
    fn search_repository_label_and_mine_narrow_together() {
        let a = issue(12, "Crash when the list is empty", "octo/app", &["bug", "ui"], &["alice"]);
        let b = issue(120, "Empty state for the list", "octo/site", &["design"], &[]);
        let mut f = ListFilter::default();
        assert!(f.is_empty() && f.issue(&a, "alice") && f.issue(&b, "alice"));
        // Every word, any case, any order.
        f.search = TextEdit::new("LIST empty");
        assert!(f.issue(&a, "alice") && f.issue(&b, "alice"));
        f.search = TextEdit::new("crash list");
        assert!(f.issue(&a, "alice") && !f.issue(&b, "alice"));
        // A number is that issue, not every title with those digits.
        f.search = TextEdit::new("#12");
        assert!(f.issue(&a, "alice") && !f.issue(&b, "alice"));
        f.search = TextEdit::new("120");
        assert!(!f.issue(&a, "alice") && f.issue(&b, "alice"));
        f.clear();
        f.label = Some("BUG".into());
        assert!(f.issue(&a, "alice") && !f.issue(&b, "alice"));
        f.repo = Some("octo/site".into());
        assert!(!f.issue(&a, "alice") && !f.issue(&b, "alice"), "the filters add up");
        f.clear();
        f.mine = true;
        assert!(f.issue(&a, "Alice") && !f.issue(&a, "bob") && !f.issue(&b, "alice"));
        // For pull requests "mine" is the author.
        assert!(f.pull(&pull(1, "x", "octo/app", "alice"), "alice") && !f.pull(&pull(2, "x", "octo/app", "bob"), "alice"));
        assert!(!f.is_empty());
        // Drafts, and failing checks, for pull requests.
        let mut draft = pull(3, "x", "octo/app", "bob");
        draft.state = PullState::Draft;
        let mut red = pull(4, "x", "octo/app", "bob");
        red.checks = Some(crate::model::Checks::Failing);
        let f = ListFilter { drafts: true, ..ListFilter::default() };
        assert!(f.pull(&draft, "alice") && !f.pull(&red, "alice") && !f.is_empty());
        let f = ListFilter { failing: true, ..ListFilter::default() };
        assert!(!f.pull(&draft, "alice") && f.pull(&red, "alice"));
    }

    #[test]
    fn review_requested_narrows_and_filters_round_trip_through_the_preferences() {
        let mut waiting = pull(8, "Fix vocab", "octo/app", "bob");
        waiting.review_requested = true;
        let other = pull(9, "Other", "octo/app", "bob");
        let f = ListFilter { review: true, ..ListFilter::default() };
        assert!(f.pull(&waiting, "alice") && !f.pull(&other, "alice") && !f.is_empty());

        let f = ListFilter { search: TextEdit::new(" vocab "), repo: Some("octo/app".into()), label: Some("bug".into()), mine: true, drafts: true, failing: true, review: true };
        let back = ListFilter::from_json(&f.to_json());
        assert_eq!(back.search.text, "vocab");
        assert_eq!((back.repo.as_deref(), back.label.as_deref()), (Some("octo/app"), Some("bug")));
        assert!(back.mine && back.drafts && back.failing && back.review);
        // Missing or odd values are left empty.
        let odd = ListFilter::from_json(&serde_json::json!({ "search": 3, "repo": "", "mine": "yes" }));
        assert!(odd.is_empty());
        assert!(ListFilter::from_json(&ListFilter::default().to_json()).is_empty());
    }

    #[test]
    fn choices_are_the_most_used_first() {
        let list = [
            issue(1, "a", "octo/app", &["bug"], &[]),
            issue(2, "b", "octo/site", &["bug", "ui"], &[]),
            issue(3, "c", "octo/site", &["Bug"], &[]),
        ];
        let (repos, labels) = issue_choices(list.iter());
        assert_eq!(repos, ["octo/site", "octo/app"]);
        assert_eq!(labels, ["bug", "ui"], "the same label in another case is one label");
        assert_eq!(top((0..40).map(|n| format!("r{n}")), 12).len(), 12);
    }

    #[test]
    fn two_hundred_issues_filter_in_no_time() {
        let list: Vec<Issue> = (0..200).map(|n| issue(n, &format!("Issue about thing {n}"), if n % 2 == 0 { "octo/app" } else { "octo/site" }, &[if n % 3 == 0 { "bug" } else { "idea" }], &[])).collect();
        let f = ListFilter { search: TextEdit::new("thing 1"), label: Some("bug".into()), ..ListFilter::default() };
        let started = std::time::Instant::now();
        let mut kept = 0;
        for _ in 0..100 {
            kept = list.iter().filter(|x| f.issue(x, "me")).count();
        }
        assert_eq!(kept, list.iter().filter(|x| x.title.contains('1') && x.labels[0].0 == "bug").count());
        assert!(started.elapsed() < std::time::Duration::from_millis(250), "100 passes over 200 issues took {:?}", started.elapsed());
    }
}
