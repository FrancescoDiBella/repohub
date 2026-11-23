//! The Issues page's filters: label, search and clear.

use crate::{App, Page};

fn press(rt: &mut rune::Runtime<App>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
}

#[test]
fn the_issues_page_narrows_by_label_search_and_clear() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = Page::Issues;
    let all: Vec<String> = s.data.issues.iter().map(|x| format!("issue-{}-{}", x.repo, x.number)).collect();
    let bugs: Vec<String> = s.data.issues.iter().filter(|x| x.labels.iter().any(|l| l.0 == "bug")).map(|x| format!("issue-{}-{}", x.repo, x.number)).collect();
    assert!(bugs.len() >= 2 && bugs.len() < all.len(), "the sample data has bugs and other issues");
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let shown = |rt: &rune::Runtime<App>| all.iter().filter(|k| rt.find(k).is_some()).cloned().collect::<Vec<_>>();
    assert_eq!(shown(&rt), all);
    assert!(rt.find("issues-clear").is_none(), "nothing to clear yet");

    press(&mut rt, "issues-label-bug");
    assert_eq!(shown(&rt), bugs, "only the bugs stay");
    // Searching inside them, by a word of one title.
    let word = rt.state.data.issues.iter().find(|x| x.labels.iter().any(|l| l.0 == "bug")).unwrap().title.split_whitespace().next().unwrap().to_owned();
    rt.update(|s, _| s.issue_filter.search = rune::TextEdit::new(word.clone()));
    rt.frame(0.3);
    let narrowed = shown(&rt);
    assert!(!narrowed.is_empty() && narrowed.len() < bugs.len() && narrowed.iter().all(|k| bugs.contains(k)));
    // Nothing matches: the page says so, and not that there are no issues.
    rt.update(|s, _| s.issue_filter.search = rune::TextEdit::new("zzz-no-such-title"));
    rt.frame(0.4);
    assert!(shown(&rt).is_empty());
    assert!(rt.elements().iter().any(|e| e.text.as_deref() == Some("No issues match these filters")));
    press(&mut rt, "issues-clear");
    assert_eq!(shown(&rt), all);
    assert!(rt.state.issue_filter.is_empty());
    // The Pull requests page has its own filter.
    assert!(rt.state.pull_filter.is_empty());
}

/// Created by me, Assigned to me and the author chips, on the Issues page; the Pull requests page
/// has them too.
#[test]
fn created_by_me_assigned_to_me_and_the_author() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.page = Page::Issues;
    let me = s.data.profile.login.clone();
    let keys = |f: &dyn Fn(&crate::model::Issue) -> bool, s: &App| s.data.issues.iter().filter(|x| f(x)).map(|x| format!("issue-{}-{}", x.repo, x.number)).collect::<Vec<_>>();
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let shown = |rt: &rune::Runtime<App>| rt.state.data.issues.iter().map(|x| format!("issue-{}-{}", x.repo, x.number)).filter(|k| rt.find(k).is_some()).collect::<Vec<_>>();
    let mine = keys(&|x| x.author == me, &rt.state);
    assert!(!mine.is_empty() && mine.len() < rt.state.data.issues.len());
    press(&mut rt, "issues-mine");
    assert_eq!(shown(&rt), mine, "Created by me");
    press(&mut rt, "issues-mine");
    let assigned = keys(&|x| x.assignees.contains(&me), &rt.state);
    press(&mut rt, "issues-assigned");
    assert_eq!(shown(&rt), assigned, "Assigned to me");
    press(&mut rt, "issues-assigned");
    let octocat = keys(&|x| x.author == "octocat", &rt.state);
    press(&mut rt, "issues-author-octocat");
    assert_eq!(shown(&rt), octocat, "the author chip");
    assert_eq!(rt.state.issue_filter.author.as_deref(), Some("octocat"));
    press(&mut rt, "issues-clear");
    rt.update(|s, _| s.go(Page::Pulls));
    rt.frame(0.5);
    for key in ["pulls-mine", "pulls-assigned", "pulls-review", "pulls-drafts", "pulls-failing"] {
        assert!(rt.find(key).is_some(), "{key} on the Pull requests page");
    }
}

/// Narrowing 200 issues by a label and a search draws the next frame at once (a guard against
/// the old cost on shared machines, not the one-frame target).
#[test]
fn filtering_two_hundred_issues_on_the_page_is_quick() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    let mut data = crate::model::Data::sample();
    let base = data.issues[0].clone();
    data.issues = (0..200u32).map(|n| crate::model::Issue {
        number: n + 1,
        title: format!("Issue about thing {n}"),
        labels: vec![(if n % 3 == 0 { "bug" } else { "idea" }.to_owned(), 25.0)],
        ..base.clone()
    }).collect();
    s.data = data;
    s.data_changed();
    s.pending_place = None;
    s.page = Page::Issues;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 900.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.1);
    let started = std::time::Instant::now();
    rt.update(|s, _| {
        s.issue_filter.label = Some("bug".into());
        s.issue_filter.search = rune::TextEdit::new("thing 1");
    });
    rt.frame(0.2);
    let took = started.elapsed();
    let kept = rt.state.data.issues.iter().filter(|x| rt.state.issue_filter.issue(x, "me")).count();
    assert!(kept > 0 && kept < 200);
    assert!(took < std::time::Duration::from_millis(400), "narrowing 200 issues took {took:?}");
}
