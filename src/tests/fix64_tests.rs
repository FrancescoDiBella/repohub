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
