//! The phone layout: a row's buttons, and the pinned repositories on the Repositories page.

use crate::{App, Page};

fn app(page: Page, compact: bool) -> rune::Runtime<App> {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = page;
    s.compact = compact;
    let size = if compact { (390.0, 3000.0) } else { (1180.0, 3000.0) };
    let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(0.1);
    rt
}

#[test]
fn on_a_phone_a_rows_buttons_go_under_it() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let pr = crate::model::Data::sample().pulls.into_iter().find(|p| p.number == 14).unwrap();
    let (row, merge) = (format!("pr-{}-{}", pr.repo, pr.number), format!("merge-{}", crate::actions::busy_key(&pr.repo, pr.number)));
    let wide = app(Page::Pulls, false);
    let (_, row_y) = wide.find(&row).unwrap();
    let (_, merge_y) = wide.find(&merge).unwrap();
    assert!((row_y - merge_y).abs() < 20.0, "side by side on a wide window");
    let phone = app(Page::Pulls, true);
    assert!(phone.state.compact);
    let (_, row_y) = phone.find(&row).unwrap();
    let (merge_x, merge_y) = phone.find(&merge).unwrap();
    assert!(merge_y > row_y + 20.0, "under the row on a phone ({merge_y} vs {row_y})");
    assert!(merge_x < 390.0, "and on screen");
}

#[test]
fn on_a_phone_pinned_repositories_are_on_the_repositories_page() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(app(Page::Repos, false).find("pinned-row").is_none(), "the sidebar has them on a wide window");
    let phone = app(Page::Repos, true);
    assert!(phone.find("pinned-row").is_some());
    let first = crate::pinned(&phone.state)[0];
    assert!(phone.find(&format!("pinned-chip-{}", phone.state.data.repos[first].full_name)).is_some());
}
