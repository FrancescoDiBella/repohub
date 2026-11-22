//! #138: a write made while a load runs isn't undone when the load lands, and isn't counted
//! twice when the load already shows it (closing an issue, merging a pull request, a new issue,
//! a star), during the first load from the saved copy and during a refresh.

use crate::model::{Data, PullState, same_repo};
use crate::{App, Page, Status, github};

/// An app with an account (no handle: the tests deliver GitHub's answers themselves) and
/// GitHub's lists on screen.
fn live() -> App {
    let mut s = crate::init();
    s.pending_place = None;
    let mut cfg = crate::config::Config::from_json("{}").unwrap();
    cfg.api = "http://127.0.0.1:9".into();
    cfg.token = Some("test-token".into());
    s.cfg = std::sync::Arc::new(cfg);
    s.data = lists();
    s.data_changed();
    s.status = Status::Ready;
    s.page = Page::Issues;
    s
}

/// What a load brings: the sample lists as GitHub's, with totals.
fn lists() -> Data {
    let mut d = Data::sample_at(1_790_000_000);
    d.sample = false;
    d.profile.login = "octocat".into();
    d.issues_total = Some(d.issues.len() + 40);
    d.open_pulls_total = Some(30);
    d.closed_pulls_total = Some(60);
    d
}

fn repo_count(s: &App, repo: &str) -> u32 {
    s.data.repos.iter().find(|r| same_repo(&r.full_name, repo)).map(|r| r.issues).unwrap()
}

fn listed(s: &App, repo: &str, n: u32) -> bool {
    s.data.issues.iter().any(|x| x.number == n && same_repo(&x.repo, repo))
}

/// Closing an issue while the load runs: the load's lists were read before (the issue comes
/// back in them) or after (it's gone already). Either way it stays closed and counts once.
fn close_during_a_load(mut s: App) {
    let issue = s.data.issues[0].clone();
    let before = (repo_count(&s, &issue.repo), s.data.issues_total);
    s.refresh();
    assert_eq!(s.status, Status::Loading);
    let epoch = s.load_gen;
    s.apply_done(&issue.repo, github::Done::Closed(issue.number));
    assert!(!listed(&s, &issue.repo, issue.number), "closed at once");
    // The load read the lists before the close: the issue is in them.
    s.apply(epoch, Ok(lists()));
    assert!(!listed(&s, &issue.repo, issue.number), "it doesn't come back when the load lands");
    assert_eq!((repo_count(&s, &issue.repo), s.data.issues_total), (before.0 - 1, before.1.map(|t| t - 1)), "counted once");

    // Again, with a load that read the lists after the close: nothing is counted twice.
    s.refresh();
    let epoch = s.load_gen;
    let issue = s.data.issues[0].clone();
    s.apply_done(&issue.repo, github::Done::Closed(issue.number));
    let mut after = lists();
    after.issues.retain(|x| !(x.number == issue.number && same_repo(&x.repo, &issue.repo)));
    if let Some(r) = after.repos.iter_mut().find(|r| same_repo(&r.full_name, &issue.repo)) {
        r.issues -= 1;
    }
    after.issues_total = after.issues_total.map(|t| t - 1);
    let want = (after.repos.iter().find(|r| same_repo(&r.full_name, &issue.repo)).unwrap().issues, after.issues_total);
    s.apply(epoch, Ok(after));
    assert!(!listed(&s, &issue.repo, issue.number));
    assert_eq!((repo_count(&s, &issue.repo), s.data.issues_total), want, "GitHub's counts, not one lower");
    assert!(s.replays.is_empty(), "nothing kept once the load landed");
}

#[test]
fn an_issue_closed_during_a_refresh_stays_closed() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-refresh");
    close_during_a_load(live());
}

#[test]
fn an_issue_closed_during_the_first_load_from_the_saved_copy_stays_closed() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-copy");
    let mut s = live();
    // At start: the saved copy on screen, the first load not landed yet.
    s.status = Status::Starting;
    s.fresh.from_copy = true;
    close_during_a_load(s);
}

/// A write that started before a load landed and finished after it: the lists it lands on may
/// already show it (they were read while it ran), so nothing goes down twice.
#[test]
fn a_write_finishing_after_a_load_landed_counts_once() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-late");
    let mut s = live();
    let issue = s.data.issues[0].clone();
    let landed = s.landed;
    s.refresh();
    let epoch = s.load_gen;
    let mut after = lists();
    after.issues.retain(|x| x.number != issue.number || !same_repo(&x.repo, &issue.repo));
    after.issues_total = after.issues_total.map(|t| t - 1);
    let total = after.issues_total;
    s.apply(epoch, Ok(after));
    // GitHub answers the close now.
    s.apply_done_since(&issue.repo, github::Done::Closed(issue.number), landed);
    assert_eq!(s.data.issues_total, total, "the load already counted it");
    // A write sent after the load counts as before (an issue on a later page lowers the count).
    s.apply_done(&issue.repo, github::Done::Closed(77_777));
    assert_eq!(s.data.issues_total, total.map(|t| t - 1));
}

#[test]
fn a_pull_request_merged_during_a_load_stays_merged() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-merge");
    let mut s = live();
    s.page = Page::Pulls;
    let p = s.data.pulls.iter().find(|p| p.state == PullState::Open).cloned().unwrap();
    let state = |s: &App| s.data.pulls.iter().find(|x| x.number == p.number && same_repo(&x.repo, &p.repo)).map(|x| x.state);
    s.refresh();
    let epoch = s.load_gen;
    s.apply_done(&p.repo, github::Done::Merged(p.number));
    s.apply(epoch, Ok(lists()));
    assert_eq!(state(&s), Some(PullState::Merged), "still merged");
    assert_eq!((s.data.open_pulls_total, s.data.closed_pulls_total), (Some(29), Some(61)), "counted once");
    // A load that already shows it merged: the totals are GitHub's.
    s.refresh();
    let epoch = s.load_gen;
    s.apply_done(&p.repo, github::Done::Merged(p.number));
    let mut after = lists();
    for x in after.pulls.iter_mut().filter(|x| x.number == p.number && same_repo(&x.repo, &p.repo)) {
        x.state = PullState::Merged;
    }
    after.open_pulls_total = Some(29);
    after.closed_pulls_total = Some(61);
    s.apply(epoch, Ok(after));
    assert_eq!((s.data.open_pulls_total, s.data.closed_pulls_total), (Some(29), Some(61)), "not moved twice");
}

#[test]
fn an_issue_created_during_a_load_is_still_listed() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-new");
    let mut s = live();
    let repo = s.data.repos[0].full_name.clone();
    s.open(0);
    s.open_form(0, crate::state::Compose::Issue);
    s.form.as_mut().unwrap().title = rune::TextEdit::new("Made during a load");
    s.form.as_mut().unwrap().submit = crate::state::Submit::Sending;
    let total = s.data.issues_total;
    s.refresh();
    let epoch = s.load_gen;
    let made = github::Created { number: 4242, title: "Made during a load".into(), ..Default::default() };
    s.form_sent(s.load_gen, &repo, Ok(github::Done::Issue(made.clone())));
    assert!(s.form.is_none() && listed(&s, &repo, 4242));
    s.apply(epoch, Ok(lists()));
    let row = s.data.issues.iter().find(|x| x.number == 4242).expect("still listed after the load");
    assert_eq!(row.title, "Made during a load");
    assert_eq!(s.data.issues_total, total.map(|t| t + 1));
    // A load that already lists it: once, and GitHub's total.
    s.refresh();
    let epoch = s.load_gen;
    s.apply_done(&repo, github::Done::Issue(made));
    let mut after = lists();
    let mut row = after.issues[0].clone();
    (row.number, row.repo, row.title) = (4242, repo.clone(), "Made during a load".into());
    after.issues.insert(0, row);
    let want = after.issues_total;
    s.apply(epoch, Ok(after));
    assert_eq!(s.data.issues.iter().filter(|x| x.number == 4242).count(), 1, "listed once");
    assert_eq!(s.data.issues_total, want);
}

#[test]
fn a_star_changed_during_a_load_stays_and_counts_once() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("replay-star");
    let mut s = live();
    let i = s.data.repos.iter().position(|r| !r.starred).unwrap();
    let full = s.data.repos[i].full_name.clone();
    let stars = s.data.repos[i].stars;
    let star = |s: &App| s.data.repos.iter().find(|r| r.full_name == full).map(|r| (r.starred, r.stars)).unwrap();
    // Starred while the load runs; the load read the stars before.
    s.refresh();
    let epoch = s.load_gen;
    let landed = s.landed;
    s.toggle_star(i);
    assert_eq!(star(&s), (true, stars + 1));
    s.apply(epoch, Ok(lists()));
    assert_eq!(star(&s), (true, stars + 1), "still starred when the load lands");
    // GitHub's answer comes after the load: nothing counted twice.
    s.star_sent(s.load_gen, &full, true, landed, Ok(()));
    assert_eq!(star(&s), (true, stars + 1));
    // Unstarred before a load starts, which lands before GitHub's answer, with the star read
    // before the unstar: the answer puts the unstar back, once.
    let landed = s.landed;
    let i = s.repo_index_of(&full).unwrap();
    s.toggle_star(i);
    assert_eq!(star(&s), (false, stars));
    s.refresh();
    let epoch = s.load_gen;
    let mut before = lists();
    if let Some(r) = before.repos.iter_mut().find(|r| r.full_name == full) {
        (r.starred, r.stars) = (true, stars + 1);
    }
    s.apply(epoch, Ok(before));
    assert_eq!(star(&s), (true, stars + 1), "the load read it before the unstar");
    s.star_sent(s.load_gen, &full, false, landed, Ok(()));
    assert_eq!(star(&s), (false, stars), "GitHub's answer puts it back");
    s.star_sent(s.load_gen, &full, false, landed, Ok(()));
    assert_eq!(star(&s), (false, stars), "once");
    // Refused during a load: back to what it was, and not put back when the load lands.
    s.refresh();
    let epoch = s.load_gen;
    let landed = s.landed;
    let i = s.repo_index_of(&full).unwrap();
    s.toggle_star(i);
    s.star_sent(s.load_gen, &full, true, landed, Err("GitHub refused".into()));
    assert_eq!(star(&s), (false, stars));
    s.apply(epoch, Ok(lists()));
    assert_eq!(star(&s), (false, stars));
}
