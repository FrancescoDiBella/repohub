//! #138: with only a token (no user name), the saved copy isn't shown before GitHub has said
//! whose the token is: the lists show they're loading for that one request, then the copy if
//! it's that account's (another account's is removed, unseen). With a user name the copy shows
//! in the first frame, as before. The token is never in the copy.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::model::Data;
use crate::{App, Status, cache, etag, github};

const TOKEN: &str = "ghp_followups_never_in_the_copy_0123456789";

fn app(user: Option<&str>) -> App {
    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.user = user.map(str::to_owned);
    cfg.token = Some(TOKEN.into());
    cfg.api = "http://127.0.0.1:9".into();
    cfg.account_locked = false;
    cfg.demo = false;
    s.cfg = Arc::new(cfg);
    s.pending_place = None;
    s.data = Data::default();
    s.data_changed();
    s.set_status(Status::Starting);
    s
}

fn loaded(login: &str) -> Data {
    let mut d = Data::sample_at(1_790_000_000);
    d.sample = false;
    d.profile.login = login.into();
    for r in &mut d.repos {
        r.owner = login.into();
        r.full_name = format!("{login}/{}", r.name);
    }
    d
}

fn copies() -> Vec<std::path::PathBuf> {
    std::fs::read_dir(cache::folder()).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// The token's account loads once: its copy is saved (named after the host, without the token).
fn saved_by(login: &str, user: Option<&str>) {
    let mut first = app(user);
    first.refresh();
    let epoch = first.load_gen;
    first.apply(epoch, Ok(loaded(login)));
    let files = copies();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(!std::fs::read_to_string(&files[0]).unwrap().contains(TOKEN), "the copy never holds the token");
}

/// The text on screen, drawn once.
fn shown(s: App) -> (App, Vec<String>) {
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 1600.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let texts = rt.elements().iter().filter_map(|e| e.text.clone()).collect();
    (rt.state, texts)
}

#[test]
fn with_only_a_token_the_copy_waits_for_githubs_answer_then_shows() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("token-only-same");
    etag::clear();
    saved_by("octo", None);
    let mut next = app(None);
    next.show_saved_copy();
    assert!(next.data.is_empty() && !next.fresh.from_copy, "not shown before GitHub says whose the token is");
    next.refresh();
    let epoch = next.load_gen;
    let (mut next, texts) = shown(next);
    assert!(texts.iter().any(|t| t.contains("Loading data from GitHub")), "the loading state meanwhile: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("octo/")), "none of the copy's lists");
    // A late answer of another load changes nothing.
    next.profile_known(epoch.wrapping_sub(1), "octo");
    assert!(next.data.is_empty());
    next.profile_known(epoch, "Octo");
    assert!(next.fresh.from_copy && next.data.profile.login == "octo" && !next.data.repos.is_empty(), "the copy, now that it's known to be octo's");
    assert_eq!(copies().len(), 1, "and kept");
    // The rest of the load replaces it as usual.
    next.apply(epoch, Ok(loaded("octo")));
    assert!(!next.fresh.from_copy && next.fresh.held.is_none());
    etag::clear();
}

#[test]
fn a_token_swapped_outside_the_app_never_shows_the_previous_accounts_copy() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("token-only-swapped");
    etag::clear();
    saved_by("octo", None);
    // The next start, with alice's token in the same place.
    let mut next = app(None);
    next.show_saved_copy();
    next.refresh();
    let epoch = next.load_gen;
    assert!(next.data.is_empty());
    next.profile_known(epoch, "alice");
    assert!(next.data.is_empty() && !next.fresh.from_copy, "octo's lists never show");
    assert!(copies().is_empty(), "octo's copy is removed");
    let mut alice = loaded("alice");
    (alice.pulls, alice.issues) = (Vec::new(), Vec::new());
    next.apply_early(epoch, alice);
    assert_eq!(next.data.profile.login, "alice");
    next.apply(epoch, Err("GitHub is down".into()));
    assert_eq!(next.data.profile.login, "alice", "a failure doesn't bring octo's back");
    assert!(copies().is_empty());
    etag::clear();
}

#[test]
fn with_a_user_name_the_copy_shows_in_the_first_frame() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("user-copy");
    etag::clear();
    saved_by("octo", Some("octo"));
    let mut next = app(Some("octo"));
    next.show_saved_copy();
    assert!(next.fresh.from_copy && next.fresh.held.is_none(), "at once");
    next.refresh();
    let (_, texts) = shown(next);
    assert!(!texts.iter().any(|t| t.contains("Loading data from GitHub")), "no loading state: {texts:?}");
    etag::clear();
}

/// The load says whose the token is as soon as GitHub has answered the first request, before
/// the repositories are read.
#[test]
fn the_load_says_whose_token_it_is_before_the_repositories() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    etag::clear();
    let mock = crate::load_tests::Mock::start(|path: &str| {
        let p = path.split('?').next().unwrap_or("");
        match p {
            "/user" => (200, json!({ "login": "octo", "name": "Octo" })),
            "/user/repos" | "/user/starred" | "/users/octo/orgs" | "/notifications" => (200, json!([])),
            "/search/issues" => (200, json!({ "total_count": 0, "items": [] })),
            _ => (404, json!({ "message": "Not Found" })),
        }
    });
    let mut cfg = crate::config::Config::from_json("{}").unwrap();
    cfg.api = mock.url.clone();
    cfg.token = Some(TOKEN.into());
    let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let d = pollster::block_on(github::load_all_reporting(&cfg, |login| events.lock().unwrap().push(format!("known {login}")), |_| events.lock().unwrap().push("lists".into()))).expect("loads");
    assert_eq!(d.profile.login, "octo");
    assert_eq!(events.into_inner().unwrap(), ["known octo", "lists"]);
    etag::clear();
}
