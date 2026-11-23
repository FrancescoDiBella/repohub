//! Writes land on the intended repository, the pull request diff, update channels and cancelling, plugin pages and tabs.

use crate::Page;

fn sample_release() -> crate::update::Release {
    crate::update::Release {
        sha: "abc1234def".into(),
        version: None,
        published: 1_700_000_000,
        asset: "RepoHub-linux-x86_64.tar.gz".into(),
        url: "https://example.invalid/pkg.tar.gz".into(),
        size: 100,
        sums: None,
        sig: None,
        notes: String::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn failed_background_update_check_keeps_available_and_cancel_restores_it() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    let rel = sample_release();

    // Background check skips Available.
    s.update = crate::update::Update::Available(rel.clone());
    assert_eq!(s.update.start_check(false), None);
    assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

    // Manual check from Available preserves Available on transient failure.
    let prior = s.update.start_check(true).expect("manual check runs from Available");
    s.apply_update_check(prior, true, Err("offline".into()));
    assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

    // Cancelling during Downloading restores Available and marks job cancelled.
    let job = crate::UpdateJob::new();
    s.update_cancel = Some(job.clone());
    s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 45 };
    s.cancel_update();
    assert!(job.was_cancelled());
    assert!(!job.begin_install(), "cancelled job must not proceed to install");
    assert!(s.update_cancel.is_none());
    assert_eq!(s.update, crate::update::Update::Available(rel.clone()));

    // Once the worker begins Installing, cancel_update refuses to revert to Available.
    let job2 = crate::UpdateJob::new();
    assert!(job2.begin_install());
    s.update_cancel = Some(job2.clone());
    s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 100 };
    s.cancel_update();
    assert!(!job2.was_cancelled());
    assert_eq!(s.update, crate::update::Update::Installing);

    // Once already Installing, cancel_update is a no-op.
    s.cancel_update();
    assert_eq!(s.update, crate::update::Update::Installing);
}

/// Switching where updates come from while one downloads stops that download (it's the
/// other channel's build) instead of installing it anyway; once installing, it's too late.
#[test]
fn switching_channel_mid_download_stops_the_other_channels_build() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dir = crate::load_gen_tests::TempDataDir::new("update-channel");
    let mut s = crate::init();
    let rel = sample_release();
    s.update_channel = crate::update::Channel::Preview;
    let job = crate::UpdateJob::new();
    s.update_cancel = Some(job.clone());
    s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 30 };
    s.set_update_channel(crate::update::Channel::Stable);
    assert!(job.was_cancelled() && !job.begin_install(), "the Latest build's download stops");
    assert!(s.update_cancel.is_none());
    assert!(matches!(s.update, crate::update::Update::Idle | crate::update::Update::Checking), "{:?}", s.update);
    // Already installing: nothing changes but the channel.
    s.update = crate::update::Update::Installing;
    s.set_update_channel(crate::update::Channel::Preview);
    assert_eq!(s.update, crate::update::Update::Installing);
    crate::update::Channel::Stable.save();
}

#[test]
fn ui_cancel_only_during_download_and_mixed_pr_count_capped() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let rel = sample_release();
    s.page = crate::Page::Settings;
    // The version and its updates are in Settings → About.
    s.settings_section = crate::state::SettingsSection::About;
    s.update = crate::update::Update::Downloading { release: rel.clone(), percent: 40 };

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);

    // Both banner and Settings show Cancel during Downloading.
    let (x, y) = rt.find("update-cancel").expect("banner Cancel shown while downloading");
    assert!(rt.find("update-cancel-settings").is_some(), "settings Cancel shown while downloading");

    // Clicking Cancel in the banner restores Available and hides both Cancel buttons.
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    assert_eq!(rt.state.update, crate::update::Update::Available(rel));
    assert!(rt.find("update-cancel").is_none(), "no banner Cancel when Available");
    assert!(rt.find("update-cancel-settings").is_none(), "no settings Cancel when Available");

    // Past downloading (Installing): Cancel must not be offered anywhere.
    rt.update(|s, _| s.update = crate::update::Update::Installing);
    rt.frame(0.0);
    assert!(rt.find("update-cancel").is_none(), "no banner Cancel when Installing");
    assert!(rt.find("update-cancel-settings").is_none(), "no settings Cancel when Installing");

    // 40 mixed PRs (12 open + 28 merged) shows "12+" on Overview and Sidebar; 39 shows "12".
    rt.update(|s, _| {
        s.page = crate::Page::Overview;
        s.data.pulls = (0..crate::model::SEARCH_CAP).map(|i| crate::model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
            number: i as u32 + 1,
            title: format!("PR #{i}"),
            repo: "a/b".into(),
            author: "u".into(),
            state: if i < 12 { crate::model::PullState::Open } else { crate::model::PullState::Merged },
            checks: None,
            updated_at: s.now,
            comments: 0,
            url: String::new(),
        }).collect();
    });
    rt.frame(0.0);
    let has_text = |rt: &rune::Runtime<crate::App>, needle: &str| {
        rt.elements().iter().any(|e| e.text.as_deref() == Some(needle))
    };
    let metric_has = |rt: &rune::Runtime<crate::App>, key: &str, needle: &str| {
        rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && e.label.as_deref().is_some_and(|l| l.starts_with(needle)))
    };
    assert!(metric_has(&rt, "metric-Open pull requests", "12+ "), "Overview metric shows 12+");
    assert!(has_text(&rt, "12+"), "Sidebar badge shows 12+");

    rt.update(|s, _| {
        s.data.pulls.pop();
    });
    rt.frame(0.0);
    assert!(metric_has(&rt, "metric-Open pull requests", "12 "), "Overview metric shows 12 below cap");
    assert!(!has_text(&rt, "12+"), "12+ disappears below cap");
    assert!(has_text(&rt, "12"), "Sidebar badge shows 12 below cap");
}

#[test]
fn locally_created_items_store_timestamps_that_advance_with_now() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    s.apply_done_inner(&s.data.repos[0].full_name.clone(), crate::github::Done::Issue(crate::github::Created { number: 999, url: "https://example.invalid/999".into(), title: "New bug".into(), ..Default::default() }));
    assert_eq!(s.data.issues[0].created_at, 1_700_000_000);
    assert_eq!(s.data.issues[0].age_ago(1_700_000_300), "5 minutes ago");

    s.apply_done_inner(&s.data.repos[0].full_name.clone(), crate::github::Done::Pull(crate::github::Created { number: 1000, url: "https://example.invalid/1000".into(), title: "New PR".into(), ..Default::default() }));
    assert_eq!(s.data.pulls[0].updated_at, 1_700_000_000);
    assert_eq!(s.data.pulls[0].age_ago(1_700_007_200), "2 hours ago");
}

#[test]
fn page_repo_tracks_by_identity_across_repo_reordering_and_removal() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample_at(1_700_000_000);
    assert!(s.data.repos.len() >= 2);
    let repo_1 = s.data.repos[1].full_name.clone();

    // Open repository 1, set branch, tab, local_dir, local_file
    s.open(1);
    s.set_tab(1, crate::RepoTab::Terminal);
    s.branch_of.insert(repo_1.clone(), "feature-x".into());
    s.local_dir = "src/components".into();
    s.local_file = Some(("mod.rs".into(), crate::state::Load::Ready(crate::local::Content::Missing)));

    assert_eq!(s.page, Page::Repo(repo_1.clone()));
    assert_eq!(s.tab, crate::RepoTab::Terminal);
    assert_eq!(s.repo_index(), Some(1));
    assert_eq!(s.place().repo.as_deref(), Some(repo_1.as_str()));

    // Simulate refresh reordering data.repos (e.g. repo_1 sorted before repo_0)
    let mut reordered = s.data.clone();
    reordered.repos.swap(0, 1);
    s.apply(s.load_gen, Ok(reordered));

    // The page should still track repo_1, but its index is now 0.
    // Its tab, branch_of, local_dir, and local_file must survive the reorder.
    assert_eq!(s.page, Page::Repo(repo_1.clone()));
    assert_eq!(s.tab, crate::RepoTab::Terminal);
    assert_eq!(s.branch_of.get(&repo_1).map(|b| b.as_str()), Some("feature-x"));
    assert_eq!(s.local_dir, "src/components");
    assert_eq!(s.local_file.as_ref().map(|(f, _)| f.as_str()), Some("mod.rs"));
    assert_eq!(s.repo_index(), Some(0));
    assert_eq!(s.place().repo.as_deref(), Some(repo_1.as_str()));

    // Simulate having a board open for repo_1
    s.open_board(0, "architecture".into(), true);
    assert!(s.board.is_some(), "board editor is open");

    // Simulate refresh where repo_1 was deleted/unloaded from GitHub
    let mut without_repo_1 = s.data.clone();
    without_repo_1.repos.retain(|r| r.full_name != repo_1);
    s.apply(s.load_gen, Ok(without_repo_1));

    // Missing repository falls back safely to Page::Repos and closes board via leave_tab
    assert_eq!(s.page, Page::Repos);
    assert_eq!(s.repo_index(), None);
    assert!(s.board.is_none(), "board must be cleared on fallback to Repos");
}

#[test]
fn apply_done_updates_intended_repository_after_reordering() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    assert!(s.data.repos.len() >= 2);
    let repo_0 = s.data.repos[0].full_name.clone();
    let repo_1 = s.data.repos[1].full_name.clone();
    let issues_1_before = s.data.repos[1].issues;

    // Initialize branches cache for repo_1
    s.branches.insert(repo_1.clone(), crate::state::Load::Ready(vec!["main".into()]));

    // Reorder repos before asynchronous done response arrives
    s.data.repos.swap(0, 1);
    assert_eq!(s.data.repos[0].full_name, repo_1);
    assert_eq!(s.data.repos[1].full_name, repo_0);

    // A Done::Branch for repo_1 arrives
    s.apply_done(&repo_1, crate::github::Done::Branch("feature/reorder".into()));
    if let Some(crate::state::Load::Ready(branches)) = s.branches.get(&repo_1) {
        assert!(branches.contains(&"feature/reorder".to_string()), "branches list updated for repo_1");
    } else {
        panic!("branches cache missing for repo_1");
    }
    assert_eq!(s.branch_of.get(&repo_1).map(|b| b.as_str()), Some("feature/reorder"));

    // A Done::Issue for repo_1 arrives
    s.apply_done(&repo_1, crate::github::Done::Issue(crate::github::Created { number: 888, url: "https://example.invalid/888".into(), title: "New reordered issue".into(), ..Default::default() }));
    assert_eq!(s.data.issues[0].number, 888);
    assert_eq!(s.data.issues[0].repo, repo_1);
    let repo_1_entry = s.data.repos.iter().find(|r| r.full_name == repo_1).unwrap();
    assert_eq!(repo_1_entry.issues, issues_1_before + 1);

    // A Done::Closed for repo_1 arrives
    s.apply_done(&repo_1, crate::github::Done::Closed(888));
    assert!(s.data.issues.iter().all(|x| x.number != 888));
    let repo_1_entry = s.data.repos.iter().find(|r| r.full_name == repo_1).unwrap();
    assert_eq!(repo_1_entry.issues, issues_1_before);
}

#[test]
fn apply_done_on_unloaded_external_repo_does_not_mutate_loaded_same_name_repo() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    // Ensure alice/app is in data.repos
    s.data.repos[0].owner = "alice".into();
    s.data.repos[0].name = "app".into();
    s.data.repos[0].full_name = "alice/app".into();
    s.data.repos[0].issues = 5;

    // Add a pull request and an issue belonging to external repo other-org/app
    s.data.pulls.insert(0, crate::model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
        number: 42,
        title: "External PR".into(),
        repo: "other-org/app".into(),
        author: "bob".into(),
        state: crate::model::PullState::Open,
        checks: None,
        updated_at: s.now,
        comments: 0,
        url: "https://github.com/other-org/app/pull/42".into(),
    });
    s.data.issues.insert(0, crate::model::Issue { labels: Vec::new(), assignees: Vec::new(),
        number: 77,
        title: "External Issue".into(),
        repo: "other-org/app".into(),
        label: None,
        created_at: s.now,
        comments: 0,
        url: "https://github.com/other-org/app/issues/77".into(),
    });

    // Merge PR 42 on other-org/app
    s.apply_done_inner("other-org/app", crate::github::Done::Merged(42));
    assert_eq!(s.data.pulls[0].number, 42);
    assert_eq!(s.data.pulls[0].state, crate::model::PullState::Merged, "external PR updated to merged");

    // Close issue 77 on other-org/app
    s.apply_done_inner("other-org/app", crate::github::Done::Closed(77));
    assert!(s.data.issues.iter().all(|x| !(x.number == 77 && x.repo == "other-org/app")), "external issue removed");

    // alice/app issues count MUST remain unchanged
    assert_eq!(s.data.repos[0].issues, 5, "alice/app issues count must not be decremented by external repo close");
}

#[test]
fn plugin_page_and_plugin_tab_reset_safely_when_unloaded() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();

    // Simulate a plugin tab and plugin page set
    s.page = Page::Plugin("test-plugin".into());
    s.plugin_tab = Some("test-plugin".into());

    // Refresh runs with no plugins matching "test-plugin"
    s.apply(s.load_gen, Ok(s.data.clone()));

    assert_eq!(s.page, Page::Overview, "removed plugin page falls back to Overview");
    assert_eq!(s.plugin_tab, None, "removed plugin tab resets to None");
}

#[test]
fn pull_diff_isolated_from_changes_tab_diff_state() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let repo_full = s.data.repos[0].full_name.clone();

    // Set up Changes tab diff with local changes
    s.diff_of = Some(crate::diffs::DiffOf::Local(repo_full.clone()));
    s.diff_load = Some(crate::state::Load::Ready(()));
    let local_file = rune_kit::diff::FileDiff::between(Some("foo.rs".into()), Some("foo.rs".into()), "old\n", "new\n");
    s.diff.set_files(vec![local_file]);
    assert_eq!(s.diff.files().len(), 1);

    // Open a pull request diff
    s.open_pull_diff(repo_full.clone(), 14);
    assert_eq!(s.pull_diff_of, Some((repo_full.clone(), 14)));
    assert_eq!(s.pull_diff_load, Some(crate::state::Load::Ready(())));
    assert!(!s.pull_diff.files().is_empty(), "sample PR diff loaded");

    // The Changes tab diff must remain completely untouched
    assert_eq!(s.diff_of, Some(crate::diffs::DiffOf::Local(repo_full.clone())));
    assert_eq!(s.diff.files().len(), 1, "local changes diff not clobbered by PR diff");

    // Closing pull diff leaves local diff intact
    s.close_pull_diff();
    assert_eq!(s.pull_diff_of, None);
    assert_eq!(s.pull_diff_load, None);
    assert!(s.pull_diff.files().is_empty());
    assert_eq!(s.diff_of, Some(crate::diffs::DiffOf::Local(repo_full)));
    assert_eq!(s.diff.files().len(), 1);
}

#[test]
fn ui_plus_on_a_diff_line_opens_a_comment_box() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.pending_place = None;
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let repo_full = s.data.repos[0].full_name.clone();
    s.open(0);
    s.set_tab(0, crate::RepoTab::Pulls);
    s.open_pull_diff(repo_full.clone(), 14);
    let side = rune_kit::diff::Side::New;
    s.start_line_comment(rune_kit::diff::LineRange::line(0, rune_kit::diff::LineRef::new(side, 12)));
    assert!(s.pull_review.as_ref().and_then(|r| r.draft.as_ref()).is_some());

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 3000.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("line-draft-text").is_some(), "the box shows under the line");
    let (x, y) = rt.find("line-draft-cancel").expect("Cancel shown");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    assert!(rt.state.pull_review.as_ref().is_some_and(|r| r.draft.is_none()), "Cancel drops the box");
    assert!(rt.find("line-draft-text").is_none());

    // Once GitHub has it, the box goes and the comment shows from GitHub's list.
    let mut s = rt.state;
    s.start_line_comment(rune_kit::diff::LineRange::line(0, rune_kit::diff::LineRef::new(side, 12)));
    s.apply_done(&repo_full, crate::github::Done::LineCommented(14));
    assert!(s.pull_review.as_ref().is_some_and(|r| r.draft.is_none()));
}

#[test]
fn navigation_and_leave_tab_clears_pull_diff() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let repo_full = s.data.repos[0].full_name.clone();

    s.open(0);
    s.open_pull_diff(repo_full.clone(), 14);
    assert!(s.pull_diff_of.is_some());

    // Navigating to Overview cleans up pull diff
    s.go(Page::Overview);
    assert!(s.pull_diff_of.is_none());
    assert!(s.pull_diff_load.is_none());
    assert!(s.pull_diff.files().is_empty());

    // Opening again and switching tab
    s.open(0);
    s.set_tab(0, crate::RepoTab::Pulls);
    s.open_pull_diff(repo_full.clone(), 14);
    assert!(s.pull_diff_of.is_some());

    s.set_tab(0, crate::RepoTab::Code);
    assert!(s.pull_diff_of.is_none(), "switching tabs calls leave_tab and clears pull diff");
}

#[test]
fn pull_panel_merge_requires_confirmation() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    let mut cfg = (*s.cfg).clone();
    cfg.user = Some("alice".into());
    cfg.token = Some("tok_alice".into());
    s.cfg = std::sync::Arc::new(cfg);
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    s.data.sample = false;
    // sample PR 14 is open on francescodibella.github.io
    let pr = s.data.pulls.iter().find(|p| p.number == 14).expect("sample PR 14 exists");
    let repo = crate::changes_page::pull_repo(&pr.url).expect("repo parsed");
    let key = crate::actions::merge_ask(&repo, pr.number);

    s.page = Page::Pulls;
    s.open_pull_diff(repo.clone(), pr.number);

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);

    // Initially pull-merge button is present, confirm/cancel are not
    assert!(rt.find("pull-merge").is_some());
    assert!(rt.find("pull-merge-confirm").is_none());
    assert!(rt.find("pull-merge-cancel").is_none());

    // Click Merge button
    let (x, y) = rt.find("pull-merge").expect("pull-merge button found");
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);

    // Confirmation must now be displayed
    assert_eq!(rt.state.asking.as_deref(), Some(key.as_str()));
    assert!(rt.find("pull-merge-confirm").is_some(), "confirmation merge button shown");
    assert!(rt.find("pull-merge-cancel").is_some(), "cancel button shown");
    assert!(rt.find("pull-merge").is_none(), "initial merge button replaced by confirm");

    // Click Cancel
    let (cx, cy) = rt.find("pull-merge-cancel").expect("cancel button found");
    rt.pointer_move(cx, cy);
    rt.pointer_down(cx, cy);
    rt.pointer_up(cx, cy);
    rt.frame(0.1);

    assert_eq!(rt.state.asking, None);
    assert!(rt.find("pull-merge").is_some(), "initial merge button restored");
    assert!(rt.find("pull-merge-confirm").is_none());
}

#[test]
fn files_button_toggles_diff_and_highlights_active_row() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let pr = s.data.pulls.iter().find(|p| p.number == 14).expect("sample PR 14 exists");
    let repo = crate::changes_page::pull_repo(&pr.url).expect("repo parsed");
    let pr_repo = pr.repo.clone();
    let pr_number = pr.number;

    s.page = Page::Pulls;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);

    // Initially pull diff is closed
    assert_eq!(rt.state.pull_diff_of, None);
    assert!(rt.find("pull-diff").is_none());

    // Click Files button on PR 14
    let files_key = format!("files-{pr_repo}-{pr_number}");
    let (fx, fy) = rt.find(&files_key).expect("files button found on PR 14");
    rt.pointer_move(fx, fy);
    rt.pointer_down(fx, fy);
    rt.pointer_up(fx, fy);
    rt.frame(0.1);

    assert_eq!(rt.state.pull_diff_of, Some((repo.clone(), pr_number)));
    assert!(rt.find("pull-diff").is_some(), "pull diff panel is open");

    // Clicking Files again toggles it closed
    let (fx2, fy2) = rt.find(&files_key).expect("files button found on active PR 14");
    rt.pointer_move(fx2, fy2);
    rt.pointer_down(fx2, fy2);
    rt.pointer_up(fx2, fy2);
    rt.frame(0.1);

    assert_eq!(rt.state.pull_diff_of, None);
    assert!(rt.find("pull-diff").is_none(), "pull diff closed by toggling files button");
}

#[test]
fn repo_tab_pulls_scopes_diff_to_matching_repository() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    let repo0 = s.data.repos[0].full_name.clone();
    let repo1 = s.data.repos[1].full_name.clone();

    // 1. Open diff on Page::Pulls, then navigate to another repo's Pulls tab
    s.page = Page::Pulls;
    s.open_pull_diff(repo0.clone(), 14);
    assert_eq!(s.pull_diff_of, Some((repo0.clone(), 14)));

    // Visit repo 1's Pulls tab: navigation clears pull diff
    s.open(1);
    s.set_tab(1, crate::RepoTab::Pulls);
    assert_eq!(s.pull_diff_of, None, "navigating to repo clears open PR diff");
    assert!(s.pull_diff.files().is_empty());

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("pull-diff").is_none(), "pull-diff not shown when cleared");

    // 2. Even if pull_diff_of is set to a different repo, RepoTab::Pulls refuses to render it
    rt.update(|s, _| {
        s.pull_diff_of = Some((repo0, 14));
        s.pull_diff_load = Some(crate::state::Load::Ready(()));
    });
    rt.frame(0.0);
    assert_eq!(rt.state.page, Page::Repo(repo1));
    assert!(rt.find("pull-diff").is_none(), "foreign repo diff must never render on another repo's Pulls tab");
}

#[test]
fn pressing_files_on_last_row_scrolls_pull_diff_into_view() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.now = 1_700_000_000;
    // Generate enough PRs so the list extends well beyond a 720px viewport
    let mut prs = Vec::new();
    for i in 1..=20 {
        prs.push(crate::model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
            number: i,
            title: format!("Pull request #{i} with long description to take space"),
            repo: "francescodibella.github.io".into(),
            author: "alice".into(),
            state: crate::model::PullState::Open,
            checks: Some(crate::model::Checks::Passing),
            updated_at: s.now - (i as i64 * 3600),
            comments: 2,
            url: format!("https://github.com/francescodibella/francescodibella.github.io/pull/{i}"),
        });
    }
    s.data.pulls = prs;
    s.page = Page::Pulls;

    // Realistic viewport height (1180 x 720)
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 720.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);

    // Find the last row's Files button (files-francescodibella.github.io-20)
    let last_files_key = "files-francescodibella.github.io-20";

    // Scroll the viewport so the last row is brought into view
    rt.update(|_, tx| tx.scroll_into_view(last_files_key));
    rt.frame(0.1);

    let (fx, fy) = rt.find(last_files_key).expect("last PR files button found");
    assert!(fy < 720.0 && fy > 0.0, "files button must now be in viewport: ({fx}, {fy})");

    // Click the Files button
    rt.pointer_move(fx, fy);
    rt.pointer_down(fx, fy);
    rt.pointer_up(fx, fy);

    // Allow scroll and layout to settle
    rt.frame(0.1);
    rt.frame(0.1);

    assert!(rt.state.pull_diff_of.is_some(), "pull diff is opened");
    let diff_el = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("pull-diff")).expect("pull-diff element found");
    let top = diff_el.rect[1];
    let height = diff_el.rect[3];
    assert!(top < 720.0 && top + height > 0.0, "pull-diff must be visible in viewport [0, 720], rect={:?}", diff_el.rect);
}
