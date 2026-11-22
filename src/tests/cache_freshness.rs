//! The cached lists follow a change in the middle of the list; a clone made in a terminal is found.

use crate::state::{RepoSort, RepoTab};

/// A change to a repository in the middle of the list reaches the cached order.
#[test]
fn a_change_in_the_middle_of_the_list_reorders_it() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.pending_place = None;
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.sort = RepoSort::Stars;
    let before = s.filtered_repos();
    assert!(before.len() >= 3, "the sample has a few repositories");
    let middle = before[before.len() / 2];
    assert_ne!(before[0], middle);
    s.data.repos[middle].stars = 1_000_000;
    s.data_changed();
    assert_eq!(s.filtered_repos()[0], middle, "the most starred repository comes first");
    // A finished write goes through the same door.
    let gen_before = s.data_gen;
    let repo = s.data.repos[middle].full_name.clone();
    s.apply_done(&repo, crate::github::Done::Commented(1));
    assert_ne!(s.data_gen, gen_before);
}

/// A clone made outside the app after "no local copy" was remembered is found when a
/// section of the repository is opened.
#[test]
fn a_clone_made_in_a_terminal_is_found_when_a_section_opens() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::temp_dir().join(format!("repohub-recheck-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let prev = (std::env::var_os("REPOHUB_REPOS_DIR"), std::env::var_os("REPOHUB_DATA_DIR"));
    // SAFETY: the environment lock is held.
    unsafe {
        std::env::set_var("REPOHUB_REPOS_DIR", base.join("work"));
        std::env::set_var("REPOHUB_DATA_DIR", base.join("data"));
    }
    let mut s = crate::init();
    s.pending_place = None;
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.data.repos[0].full_name = "octo/demo".into();
    s.data.repos[0].name = "demo".into();
    assert_eq!(s.known_clone("octo/demo"), None);
    let repo = base.join("work").join("demo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:octo/demo.git\n").unwrap();
    assert_eq!(s.known_clone("octo/demo"), None, "the miss is remembered between frames");
    s.set_tab(0, RepoTab::Code);
    assert_eq!(s.known_clone("octo/demo"), Some(repo));
    // SAFETY: as above.
    unsafe {
        match prev.0 { Some(v) => std::env::set_var("REPOHUB_REPOS_DIR", v), None => std::env::remove_var("REPOHUB_REPOS_DIR") }
        match prev.1 { Some(v) => std::env::set_var("REPOHUB_DATA_DIR", v), None => std::env::remove_var("REPOHUB_DATA_DIR") }
    }
    let _ = std::fs::remove_dir_all(&base);
}
