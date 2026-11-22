//! Pinning: the first pin keeps the ones the sidebar showed by default.

#[test]
fn the_first_pin_keeps_the_default_ones() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("repohub-pins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // SAFETY: the lock keeps other environment-changing tests out.
    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pins = None;
    let mut cfg = (*s.cfg).clone();
    cfg.pinned.clear();
    s.cfg = std::sync::Arc::new(cfg);
    assert_eq!(crate::pinned(&s), vec![0, 1, 2], "by default the three most recently pushed");
    s.toggle_pin(4);
    assert_eq!(crate::pinned(&s), vec![0, 1, 2, 4], "pinning one more keeps them");
    s.toggle_pin(1);
    assert_eq!(crate::pinned(&s), vec![0, 2, 4]);
    assert!(s.is_pinned(4) && !s.is_pinned(1));
    let saved = crate::prefs::load_pins().unwrap();
    assert_eq!(saved.len(), 3);
    assert!(saved[2].eq_ignore_ascii_case(&s.data.repos[4].full_name), "remembered by full name");
    let _ = std::fs::remove_dir_all(dir);
}
