//! The page has a scroll bar.

/// Long pages carry a scroll bar over their right edge, so you can see where you are.
#[test]
fn ui_page_has_a_scroll_bar() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.pending_place = None;
    s.page = crate::Page::Repos;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 600.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("scroll-bar").is_some(), "the page viewport has a scroll bar");
}
