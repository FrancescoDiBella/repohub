//! What a finished write changed in the lists, so a load that was already running when it
//! finished doesn't undo it on screen.
//!
//! A load reads GitHub's lists over a few seconds; a write that finishes meanwhile (closing an
//! issue, merging a pull request, a new issue, a star) may or may not be in what the load read.
//! [`App::apply_done`](crate::App) applies each write's [`Change`] to the screen at once and keeps
//! it while a load runs; when the load lands, its changes are applied again on top of what it
//! brought ([`apply`] with `exact`). Applying them again is exact: a change the arriving lists
//! already show does nothing (an issue that's gone isn't closed twice, a count doesn't go down
//! twice, a new issue isn't listed twice, a star isn't counted twice).
//!
//! ```text
//! load starts ── write finishes: Change shown, kept ── load lands: lists replaced, kept Changes
//!                                                                  applied again (exact), dropped
//! write starts ── a load lands ── write finishes: its Change applied exact (the lists it lands
//!                                                 on may already show it)
//! ```

use crate::model::{Checks, Data, Issue, Pull, PullState, same_repo};
use crate::triage::Applied;

/// One write's effect on the lists. Comments are left out: whether GitHub's count already holds
/// a new comment can't be told from the count.
#[derive(Clone, Debug)]
pub enum Change {
    /// A new issue, as its row shows it.
    Opened(Issue),
    /// A new pull request, as its row shows it.
    OpenedPull(Pull),
    Closed { repo: String, number: u32 },
    /// An issue closed from the list and reopened with Undo: its row comes back.
    Reopened(Issue),
    Merged { repo: String, number: u32 },
    /// A pull request closed (`open` false) or reopened.
    PullOpen { repo: String, number: u32, open: bool },
    Star { repo: String, on: bool },
    /// Labels, assignees, title or reviewers set on an issue or pull request.
    Triaged { repo: String, number: u32, applied: Applied },
    /// A workflow run re-run at `at` (Unix seconds).
    Reran { repo: String, run_id: u64, at: i64 },
}

impl Change {
    /// The repository it's about.
    pub fn repo(&self) -> &str {
        match self {
            Change::Opened(x) | Change::Reopened(x) => &x.repo,
            Change::OpenedPull(p) => &p.repo,
            Change::Closed { repo, .. } | Change::Merged { repo, .. } | Change::PullOpen { repo, .. } | Change::Star { repo, .. }
            | Change::Triaged { repo, .. } | Change::Reran { repo, .. } => repo,
        }
    }
}

fn add(n: &mut Option<usize>, by: isize) {
    if let Some(t) = n.as_mut() {
        *t = if by < 0 { t.saturating_sub(by.unsigned_abs()) } else { *t + by as usize };
    }
}

/// GitHub's count of a repository's open issues (pull requests included) moves by `by`.
fn repo_issues(d: &mut Data, repo: &str, by: i32) {
    if let Some(r) = d.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(repo)) {
        r.issues = if by < 0 { r.issues.saturating_sub(by.unsigned_abs()) } else { r.issues + by as u32 };
    }
}

/// Applies `c` to `d`; `me` is the account's login (whose review is asked). With `exact`, the
/// lists may already show the change (they were read after the write): what they show is kept
/// and the counts move only with a row that changes. Without it (the lists were read before the
/// write), closing an issue that isn't listed still lowers the counts, as it's on a later page.
/// True when something changed.
pub fn apply(d: &mut Data, c: &Change, me: &str, exact: bool) -> bool {
    let here = |repo: &str, number: u32, r: &str, n: u32| n == number && same_repo(r, repo);
    match c {
        Change::Opened(x) | Change::Reopened(x) => {
            if d.issues.iter().any(|i| here(&x.repo, x.number, &i.repo, i.number)) {
                return false;
            }
            d.issues.insert(0, x.clone());
            repo_issues(d, &x.repo, 1);
            add(&mut d.issues_total, 1);
            true
        }
        Change::OpenedPull(p) => {
            if d.pulls.iter().any(|q| here(&p.repo, p.number, &q.repo, q.number)) {
                return false;
            }
            d.pulls.insert(0, p.clone());
            add(&mut d.open_pulls_total, 1);
            // GitHub's count of a repository's open issues includes its pull requests.
            repo_issues(d, &p.repo, 1);
            true
        }
        Change::Closed { repo, number } => {
            let listed = d.issues.iter().any(|i| here(repo, *number, &i.repo, i.number));
            if !listed && exact {
                return false;
            }
            d.issues.retain(|i| !here(repo, *number, &i.repo, i.number));
            repo_issues(d, repo, -1);
            add(&mut d.issues_total, -1);
            true
        }
        Change::Merged { repo, number } => {
            let mut changed = false;
            for p in d.pulls.iter_mut().filter(|p| here(repo, *number, &p.repo, p.number)) {
                if matches!(p.state, PullState::Open | PullState::Draft) {
                    changed = true;
                    p.state = PullState::Merged;
                }
            }
            if changed {
                add(&mut d.open_pulls_total, -1);
                add(&mut d.closed_pulls_total, 1);
                repo_issues(d, repo, -1);
            }
            changed
        }
        Change::PullOpen { repo, number, open } => {
            let mut changed = false;
            for p in d.pulls.iter_mut().filter(|p| here(repo, *number, &p.repo, p.number)) {
                let was_open = matches!(p.state, PullState::Open | PullState::Draft);
                // A merged pull request is neither reopened nor closed again.
                if was_open != *open && p.state != PullState::Merged {
                    changed = true;
                    p.state = if *open { PullState::Open } else { PullState::Closed };
                }
            }
            if changed {
                let (from, to) = if *open { (&mut d.closed_pulls_total, &mut d.open_pulls_total) } else { (&mut d.open_pulls_total, &mut d.closed_pulls_total) };
                add(from, -1);
                add(to, 1);
                repo_issues(d, repo, if *open { 1 } else { -1 });
            }
            changed
        }
        Change::Star { repo, on } => {
            let Some(r) = d.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(repo)) else { return false };
            if r.starred == *on {
                return false;
            }
            r.starred = *on;
            r.stars = if *on { r.stars + 1 } else { r.stars.saturating_sub(1) };
            true
        }
        Change::Triaged { repo, number, applied } => {
            let mut changed = false;
            for x in d.issues.iter_mut().filter(|x| here(repo, *number, &x.repo, x.number)) {
                changed = true;
                match applied {
                    Applied::Labels(l) => {
                        x.labels = l.clone();
                        x.label = l.first().cloned();
                    }
                    Applied::Assignees(a) => x.assignees = a.clone(),
                    Applied::Title(t) => x.title = t.clone(),
                    _ => {}
                }
            }
            let mut waiting: Option<bool> = None;
            for p in d.pulls.iter_mut().filter(|p| here(repo, *number, &p.repo, p.number)) {
                changed = true;
                match applied {
                    Applied::Labels(l) => p.labels = l.clone(),
                    Applied::Assignees(a) => p.assignees = a.clone(),
                    Applied::Title(t) => p.title = t.clone(),
                    Applied::Reviewers(r) => {
                        let waits = r.iter().any(|x| x.eq_ignore_ascii_case(me));
                        if p.review_requested != waits {
                            p.review_requested = waits;
                            waiting = Some(waits);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(waits) = waiting {
                add(&mut d.review_total, if waits { 1 } else { -1 });
            }
            changed
        }
        Change::Reran { repo, run_id, at } => {
            let mut changed = false;
            for r in d.runs.iter_mut().filter(|r| r.id == *run_id && same_repo(&r.repo, repo)) {
                // A run GitHub already shows running, or that changed since, is GitHub's to show.
                if exact && (r.checks == Checks::Running || r.updated_at >= *at) {
                    continue;
                }
                r.checks = Checks::Running;
                r.updated_at = *at;
                changed = true;
            }
            changed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Data {
        let mut d = Data::sample_at(1_790_000_000);
        d.sample = false;
        d.issues_total = Some(d.issues.len());
        d.open_pulls_total = Some(10);
        d.closed_pulls_total = Some(20);
        d.review_total = Some(1);
        d
    }

    fn repo_issues_of(d: &Data, repo: &str) -> u32 {
        d.repos.iter().find(|r| r.full_name == repo).map(|r| r.issues).unwrap()
    }

    /// Each change applied twice with `exact` counts once: the second time the lists already
    /// show it.
    #[test]
    fn every_change_applied_again_counts_once() {
        let base = data();
        let issue = base.issues[0].clone();
        let pull = base.pulls.iter().find(|p| p.state == PullState::Open).cloned().unwrap();
        // The changes name a repository by its full name, as `App::apply_done` does.
        let full = |repo: &str| base.repos.iter().find(|r| same_repo(&r.full_name, repo)).unwrap().full_name.clone();
        let repo_of_issue = full(&issue.repo);
        let new_issue = Issue { number: 9001, title: "New".into(), ..issue.clone() };
        let new_pull = Pull { number: 9002, title: "New".into(), ..pull.clone() };
        let star_repo = base.repos[0].clone();
        let changes = [
            Change::Opened(new_issue),
            Change::OpenedPull(new_pull),
            Change::Closed { repo: repo_of_issue.clone(), number: issue.number },
            Change::Merged { repo: full(&pull.repo), number: pull.number },
            Change::Star { repo: star_repo.full_name.clone(), on: !star_repo.starred },
        ];
        for c in &changes {
            let mut once = base.clone();
            assert!(apply(&mut once, c, "octo", true), "{c:?} changes the lists");
            let mut twice = once.clone();
            assert!(!apply(&mut twice, c, "octo", true), "{c:?} applied again changes nothing");
            assert_eq!(format!("{:?}", once.issues.iter().map(|i| i.number).collect::<Vec<_>>()), format!("{:?}", twice.issues.iter().map(|i| i.number).collect::<Vec<_>>()));
            assert_eq!((once.issues_total, once.open_pulls_total, once.closed_pulls_total), (twice.issues_total, twice.open_pulls_total, twice.closed_pulls_total), "{c:?}");
            assert_eq!(once.repos.iter().map(|r| (r.issues, r.stars, r.starred)).collect::<Vec<_>>(), twice.repos.iter().map(|r| (r.issues, r.stars, r.starred)).collect::<Vec<_>>(), "{c:?}");
        }
        // The counts moved once each.
        let mut d = base.clone();
        apply(&mut d, &changes[2], "octo", true);
        assert_eq!(d.issues_total, base.issues_total.map(|t| t - 1));
        assert_eq!(repo_issues_of(&d, &repo_of_issue), repo_issues_of(&base, &repo_of_issue).saturating_sub(1));
    }

    /// Closing an issue that isn't listed: the lists were read before the write (it's on a later
    /// page, the count goes down), or after it (exact: GitHub's count already shows it).
    #[test]
    fn an_issue_not_listed_lowers_the_count_only_when_the_lists_are_older() {
        let base = data();
        let gone = Change::Closed { repo: base.repos[0].full_name.clone(), number: 77_777 };
        let mut older = base.clone();
        assert!(apply(&mut older, &gone, "octo", false));
        assert_eq!(older.issues_total, base.issues_total.map(|t| t - 1));
        let mut newer = base.clone();
        assert!(!apply(&mut newer, &gone, "octo", true));
        assert_eq!(newer.issues_total, base.issues_total);
    }

    #[test]
    fn reviewers_and_reruns_follow_what_github_shows() {
        let mut d = data();
        let p = d.pulls[0].clone();
        let asked = Change::Triaged { repo: p.repo.clone(), number: p.number, applied: Applied::Reviewers(vec!["octo".into()]) };
        let was = d.pulls[0].review_requested;
        apply(&mut d, &asked, "octo", true);
        apply(&mut d, &asked, "octo", true);
        assert!(d.pulls[0].review_requested);
        assert_eq!(d.review_total, Some(if was { 1 } else { 2 }), "counted once");
        let run = d.runs.iter().find(|r| r.checks != Checks::Running).cloned().unwrap();
        let rerun = Change::Reran { repo: run.repo.clone(), run_id: run.id, at: run.updated_at + 10 };
        assert!(apply(&mut d, &rerun, "octo", true));
        let r = d.runs.iter().find(|r| r.id == run.id).unwrap();
        assert_eq!((r.checks, r.updated_at), (Checks::Running, run.updated_at + 10));
        // A run that finished after the re-run was asked is GitHub's to show.
        let mut later = data();
        if let Some(r) = later.runs.iter_mut().find(|r| r.id == run.id) {
            r.updated_at = run.updated_at + 60;
        }
        assert!(!apply(&mut later, &rerun, "octo", true));
    }
}
