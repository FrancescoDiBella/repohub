//! A refresh keeps the details of repositories nobody pushed to since.

use crate::state::Load;

#[test]
fn a_refresh_keeps_details_of_repositories_nobody_pushed_to() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample_at(1_700_000_000);
    let (a, b) = (s.data.repos[0].full_name.clone(), s.data.repos[1].full_name.clone());
    for full in [&a, &b] {
        s.details.insert((full.clone(), "main".into()), Load::Ready(crate::model::Detail::default()));
    }
    s.details.insert(("gone/repo".into(), "main".into()), Load::Ready(crate::model::Detail::default()));
    let mut next = crate::model::Data::sample_at(1_700_000_000);
    next.repos[1].pushed += 60;
    let epoch = s.load_gen;
    s.apply(epoch, Ok(next));
    assert!(s.details.contains_key(&(a.clone(), "main".into())), "not pushed since: kept");
    assert!(!s.fresh.stale.contains(&(a, "main".into())), "and not read again");
    // Pushed since: it stays on screen and is read again when it's opened (#126).
    assert!(s.details.contains_key(&(b.clone(), "main".into())) && s.fresh.stale.contains(&(b, "main".into())), "pushed since: read again when opened");
    assert!(!s.details.contains_key(&("gone/repo".into(), "main".into())));
}
