//! The terminal fits a phone screen.

/// On a phone the terminal leaves room for the page around it; on a computer it keeps 520 pt.
#[test]
fn the_terminal_fits_a_phone_screen() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.backdrop_size = [390.0, 844.0];
    s.compact = false;
    assert_eq!(crate::terminal_page::term_h(&s), 520.0);
    s.compact = true;
    let h = crate::terminal_page::term_h(&s);
    assert!((240.0..844.0 - 250.0).contains(&h), "{h}");
    s.backdrop_size = [390.0, 400.0];
    assert_eq!(crate::terminal_page::term_h(&s), 240.0, "never smaller than a few lines");
}
