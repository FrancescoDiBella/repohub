//! Account switches and signing out drop late loads of the other account (`load_gen`); `TempDataDir`, a throwaway data folder for tests.

use crate::model::{Data, Detail, Profile, PullState, Repo};
use crate::state::{Compose, Layout, Load, OwnerFilter, Place, RepoFilter, RepoSort, RepoTab, Status, Submit};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct TempDataDir {
    dir: std::path::PathBuf,
    prev: Option<std::ffi::OsString>,
}

#[cfg(not(target_arch = "wasm32"))]
impl TempDataDir {
    pub(crate) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("repohub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let prev = std::env::var_os("REPOHUB_DATA_DIR");
        // SAFETY: ENV_LOCK serializes tests mutating environment variables.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        Self { dir, prev }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDataDir {
    fn drop(&mut self) {
        // SAFETY: ENV_LOCK is still held while TempDataDir drops inside each test.
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var("REPOHUB_DATA_DIR", v),
                None => std::env::remove_var("REPOHUB_DATA_DIR"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn account_data(login: &str, repo_name: &str, private: bool) -> Data {
    Data {
        profile: Profile { login: login.into(), name: login.into(), bio: String::new() },
        repos: vec![Repo {
            owner: login.into(),
            name: repo_name.into(),
            full_name: format!("{login}/{repo_name}"),
            language: "Rust".into(),
            stars: 1,
            branch: "main".into(),
            private,
            pushed: 1_700_000_000,
            hue: 210.0,
            url: format!("https://github.com/{login}/{repo_name}"),
            ..Repo::default()
        }],
        ..Data::default()
    }
}

fn seed_caches(s: &mut crate::App, owner: &str) {
    let full = format!("{owner}/secret");
    s.details.insert((full.clone(), "main".into()), Load::Ready(Detail::default()));
    s.branches.insert(full.clone(), Load::Ready(vec!["main".into()]));
    s.branch_of.insert(full.clone(), "dev".into());
    s.notes.entry((full.clone(), "main".into())).or_default().state = crate::state::Refresh::Fresh;
    s.diff_of = Some(crate::diffs::DiffOf::Local(full.clone()));
    s.diff_load = Some(Load::Ready(()));
    s.pull_diff_of = Some((full.clone(), 42));
    s.pull_diff_load = Some(Load::Ready(()));
    s.filter = RepoFilter::Private;
    s.pr_filter = PullState::Merged;
    s.owner = OwnerFilter::Org(owner.into());
    s.search = rune::TextEdit::new("secret");
    s.branch_filter = rune::TextEdit::new("dev");
    s.chats.entry(full.clone()).or_default().session = Some("sess-1".into());
    s.claude_prompt = rune::TextEdit::new("ask");
    s.clone_path = rune::TextEdit::new("/tmp/secret");
    s.busy.insert(format!("{full}#42"));
    s.form = Some(crate::actions::Form {
        repo: full.clone(),
        kind: Compose::Issue,
        title: rune::TextEdit::new("Draft"),
        body: rune::TextEdit::new(""),
        from: "main".into(),
        into: "main".into(),
        draft: false,
        submit: Submit::Editing,
    });
    crate::prefs::set("place", Place { page: "repo".into(), tab: Some(RepoTab::Notes), repo: Some(full) }.to_json());
    let old_layout = Layout { sidebar_width: 248.0, sidebar_open: true, sort: RepoSort::Recent, owner: OwnerFilter::Org(owner.into()) };
    crate::prefs::set("layout", old_layout.to_json());
    s.saved_layout = Some(old_layout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn sign_out_during_load_keeps_sample_data_and_drops_stale_callbacks() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("load-gen-signout");

    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.user = Some("alice".into());
    cfg.token = Some("tok_alice".into());
    cfg.account_locked = false;
    s.cfg = std::sync::Arc::new(cfg);
    s.acct_user = rune::TextEdit::new("alice");
    s.data = Data::default();
    s.page = crate::Page::Settings;
    s.tab = RepoTab::Notes;
    s.set_status(Status::Starting);
    seed_caches(&mut s, "alice");

    s.refresh();
    assert_eq!(s.status, Status::Loading);
    let old_gen = s.load_gen;

    s.save_account(true);
    assert_eq!(s.status, Status::Sample);
    assert!(s.data.sample);
    assert!(s.load_gen > old_gen);
    assert_eq!(s.page, crate::Page::Overview, "Settings resets to Overview on sign-out");
    assert_eq!(s.tab, RepoTab::Code);
    assert_eq!(s.filter, RepoFilter::All);
    assert_eq!(s.pr_filter, PullState::Open);
    assert_eq!(s.owner, OwnerFilter::All);
    assert!(s.search.text.is_empty());
    assert!(s.branch_filter.text.is_empty());
    assert_eq!(s.chats.get("alice/secret").and_then(|c| c.session.as_deref()), Some("sess-1"));
    assert!(s.claude_prompt.text.is_empty());
    assert!(s.clone_path.text.is_empty());
    assert!(s.details.is_empty());
    assert!(s.branches.is_empty());
    assert!(s.branch_of.is_empty());
    assert!(s.local.is_empty());
    assert!(s.notes.is_empty());
    assert!(s.form.is_none());
    assert!(s.diff_of.is_none());
    assert!(s.diff_load.is_none());
    assert!(s.pull_diff_of.is_none());
    assert!(s.pull_diff_load.is_none());
    assert!(s.busy.is_empty());

    let saved_place = crate::prefs::get("place").as_ref().and_then(Place::from_json).expect("place saved");
    assert_eq!(saved_place, Place { page: "overview".into(), tab: None, repo: None });
    let saved_layout = crate::prefs::get("layout").as_ref().and_then(Layout::from_json).expect("layout saved");
    assert_eq!(saved_layout.owner, OwnerFilter::All);

    // Deliver late results from before sign-out.
    s.apply(old_gen, Ok(account_data("alice", "secret", true)));
    s.apply_detail(old_gen, ("alice/secret".into(), "main".into()), Detail::default());
    s.apply_branches(old_gen, "alice/secret".into(), Ok(vec!["main".into()]));
    s.pull_diff_of = Some(("alice/secret".into(), 9));
    s.apply_pull_diff(old_gen, ("alice/secret".into(), 9), Ok(crate::diffs::sample_pull_files()));
    s.apply_notes(old_gen, ("alice/secret".into(), "main".into()), Ok(vec![crate::notes::Note { name: "arch".into(), updated_at: 1, size: 10 }]));

    assert_eq!(s.status, Status::Sample);
    assert!(s.data.sample, "late load after sign-out must not replace sample data");
    assert!(s.data.repos.iter().all(|r| r.owner != "alice"));
    assert!(s.details.is_empty());
    assert!(s.branches.is_empty());
    assert!(s.notes.is_empty());
    assert!(s.diff.files().is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn overlapping_account_loads_apply_only_the_newer_account() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("load-gen-switch");

    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.account_locked = false;
    cfg.user = None;
    cfg.token = None;
    s.cfg = std::sync::Arc::new(cfg);

    // Start load for account A from Settings.
    s.page = crate::Page::Settings;
    s.acct_user = rune::TextEdit::new("alice");
    s.acct_token = rune::TextEdit::new("tok_a");
    s.save_account(false);
    assert_eq!(s.page, crate::Page::Overview);
    assert_eq!(s.status, Status::Loading);
    let gen_a = s.load_gen;
    seed_caches(&mut s, "alice");

    // Switch to account B from Settings while A is still loading.
    s.page = crate::Page::Settings;
    s.acct_user = rune::TextEdit::new("bob");
    s.acct_token = rune::TextEdit::new("tok_b");
    s.save_account(false);
    assert_eq!(s.page, crate::Page::Overview);
    assert_eq!(s.status, Status::Loading);
    let gen_b = s.load_gen;
    assert!(gen_b > gen_a);
    assert!(s.details.is_empty() && s.branches.is_empty() && s.notes.is_empty() && s.form.is_none());
    assert_eq!(s.chats.get("alice/secret").and_then(|c| c.session.as_deref()), Some("sess-1"));

    let saved_place = crate::prefs::get("place").as_ref().and_then(Place::from_json).expect("place reset");
    assert_eq!(saved_place, Place { page: "overview".into(), tab: None, repo: None });

    // Case 1: stale A arrives while B is loading -> dropped; then B arrives -> applied.
    s.apply(gen_a, Ok(account_data("alice", "a-private", true)));
    assert_eq!(s.status, Status::Loading);
    assert!(s.data.repos.is_empty());

    s.apply(gen_b, Ok(account_data("bob", "b-repo", false)));
    assert_eq!(s.status, Status::Ready);
    assert_eq!(s.data.profile.login, "bob");
    assert_eq!(s.data.repos.len(), 1);
    assert_eq!(s.data.repos[0].full_name, "bob/b-repo");

    // Case 2: B already landed, then another stale callback from A arrives -> ignored.
    s.apply(gen_a, Err("stale failure".into()));
    s.apply(gen_a, Ok(account_data("alice", "a-private", true)));
    assert_eq!(s.status, Status::Ready);
    assert_eq!(s.data.profile.login, "bob");
    assert_eq!(s.data.repos[0].full_name, "bob/b-repo");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn save_account_keeps_local_sessions_and_running_git_job() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("load-gen-local-sessions");

    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.account_locked = false;
    cfg.user = Some("alice".into());
    cfg.token = Some("tok_alice".into());
    s.cfg = std::sync::Arc::new(cfg);

    let full = "alice/local-repo".to_owned();
    {
        let chat = s.chats.entry(full.clone()).or_default();
        chat.session = Some("sess-42".into());
        chat.run = crate::claude::Run::Running;
    }
    let term = crate::terminal::Term::spawn(&std::env::temp_dir(), 24, 80, None, || {}).expect("shell starts");
    s.terms.insert(full.clone(), term);
    s.git_job = Some("Committing…");

    // Switch account.
    s.acct_user = rune::TextEdit::new("bob");
    s.acct_token = rune::TextEdit::new("tok_bob");
    s.save_account(false);
    assert_eq!(s.chats.get(&full).and_then(|c| c.session.as_deref()), Some("sess-42"));
    assert_eq!(s.chats.get(&full).map(|c| c.run), Some(crate::claude::Run::Running));
    assert!(s.terms.get(&full).is_some_and(|t| t.is_running()), "terminal survives account switch");
    assert_eq!(s.git_job, Some("Committing…"), "running local git job stays marked");

    // Sign out.
    s.save_account(true);
    assert_eq!(s.chats.get(&full).and_then(|c| c.session.as_deref()), Some("sess-42"));
    assert_eq!(s.chats.get(&full).map(|c| c.run), Some(crate::claude::Run::Running));
    assert!(s.terms.get(&full).is_some_and(|t| t.is_running()), "terminal survives sign-out");
    assert_eq!(s.git_job, Some("Committing…"), "running local git job stays marked");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ui_sign_out_in_settings_resets_to_overview_and_drops_late_load() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("load-gen-ui");

    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.user = Some("alice".into());
    cfg.token = Some("tok_alice".into());
    cfg.account_locked = false;
    s.cfg = std::sync::Arc::new(cfg);
    s.acct_user = rune::TextEdit::new("alice");
    s.page = crate::Page::Settings;
    s.refresh();
    let stale_gen = s.load_gen;
    seed_caches(&mut s, "alice");

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);

    let (x, y) = rt.find("acct-sign-out").expect("Sign out button shown when signed in");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
    // It asks first (#66): the token saved here is deleted.
    assert_eq!(rt.state.cfg.token.as_deref(), Some("tok_alice"), "not before the answer");
    let (x, y) = rt.find("ask-sign-out-yes").expect("the question's Sign out");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);

    assert_eq!(rt.state.status, Status::Sample);
    assert_eq!(rt.state.page, crate::Page::Overview, "signing out from Settings returns to Overview");
    assert!(rt.find("connect").is_some(), "Overview sample banner offers Connect");
    assert!(rt.state.form.is_none());

    // Deliver late response for the signed-out account while UI is running.
    rt.update(|s, _| {
        s.apply(stale_gen, Ok(account_data("alice", "secret-repo", true)));
    });
    rt.frame(0.3);

    assert_eq!(rt.state.status, Status::Sample);
    assert_eq!(rt.state.page, crate::Page::Overview);
    assert!(rt.state.data.sample);
    assert!(rt.elements().iter().all(|e| e.text.as_deref() != Some("secret-repo")));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn saving_account_in_settings_resets_api_to_user_env_or_default() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("save-account-api-reset");

    let prev_api = std::env::var_os("GITHUB_API_URL");
    // Simulate REPOHUB_ENV having injected GITHUB_API_URL into the process environment:
    // SAFETY: ENV_LOCK serializes environment access in tests.
    unsafe { std::env::set_var("GITHUB_API_URL", "https://injected-by-project-env.example") };

    struct ApiVarGuard(Option<std::ffi::OsString>);
    impl Drop for ApiVarGuard {
        fn drop(&mut self) {
            // SAFETY: ENV_LOCK serializes environment access in tests.
            unsafe {
                match &self.0 {
                    Some(val) => std::env::set_var("GITHUB_API_URL", val),
                    None => std::env::remove_var("GITHUB_API_URL"),
                }
            }
        }
    }
    let _guard = ApiVarGuard(prev_api);

    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.account_locked = false;
    cfg.api = "https://untrusted-api.example".into();
    cfg.sources.api = Some(crate::config::Source::ProjectFile(".env".into()));
    s.cfg = std::sync::Arc::new(cfg);

    s.acct_user = rune::TextEdit::new("carol");
    s.acct_token = rune::TextEdit::new("ghp_carol_tok");
    s.save_account(false);

    assert_eq!(s.cfg.api, "https://api.github.com", "saving account must reset untrusted API host to default or user env even if GITHUB_API_URL was in process env");
    assert_eq!(s.cfg.sources.api, Some(crate::config::Source::Default));
}
