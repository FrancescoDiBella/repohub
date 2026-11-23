use super::*;
use serde_json::Value;

fn walk<'a>(v: &'a Value, f: &mut dyn FnMut(&'a Value)) {
    f(v);
    for c in v["children"].as_array().into_iter().flatten() {
        walk(c, f);
    }
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if v["key"].as_str() == Some(key) {
        return Some(v);
    }
    v["children"].as_array()?.iter().find_map(|c| find_key(c, key))
}

fn has_named(v: &Value, name: &str) -> bool {
    let mut found = false;
    walk(v, &mut |n| {
        if n["name"].as_str() == Some(name) {
            found = true;
        }
    });
    found
}

fn assert_tree_labelled(scene: &str, tree: &Value) {
    walk(tree, &mut |n| {
        let role = n["role"].as_str().unwrap_or("");
        let name = n["name"].as_str().unwrap_or("").trim();
        let key = n["key"].as_str().unwrap_or("");
        let clickable = n["actions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some("click")));
        if clickable {
            assert!(
                !role.is_empty() && !name.is_empty(),
                "{scene}: clickable node (key={key:?}, role={role:?}) has empty name in a11y tree: {n}"
            );
        }
        if matches!(
            role,
            "text-input" | "multiline-text-input" | "password-input" | "search-input" | "switch"
        ) {
            assert!(
                !name.is_empty(),
                "{scene}: control (key={key:?}, role={role:?}) has empty name in a11y tree: {n}"
            );
        }
    });
}

#[test]
fn custom_rows_inputs_switches_and_status_marks_are_accessible() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("a11y-tree");

    let mut s = crate::init();
    s.pending_place = None;
    s.now = 1_700_000_000;
    s.data = crate::model::Data::sample_at(s.now);
    s.cfg = std::sync::Arc::new(crate::config::Config {
        notes: true,
        ..(*s.cfg).clone()
    });

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2200.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    let mut t = 0.0;
    let mut snap = |rt: &mut rune::Runtime<App>| -> Value {
        t += 0.05;
        rt.frame(t);
        rt.accessibility_json()
    };

    // 1. Overview: compact repo rows and pull rows
    rt.update(|s, _| s.page = crate::Page::Overview);
    let tree = snap(&mut rt);
    assert_tree_labelled("Overview", &tree);
    let first_full = rt.state.data.repos[0].full_name.clone();
    let recent = find_key(&tree, &format!("recent-{first_full}")).expect("compact repo row");
    assert_eq!(recent["role"].as_str(), Some("link"));
    assert!(recent["name"].as_str().unwrap_or("").starts_with("Open "));

    // 2. Repositories: search input and repo card title links
    rt.update(|s, _| s.page = crate::Page::Repos);
    let tree = snap(&mut rt);
    assert_tree_labelled("Repositories", &tree);
    let open_repo = find_key(&tree, &format!("open-{first_full}")).expect("repo card link");
    assert_eq!(open_repo["role"].as_str(), Some("link"));
    assert!(open_repo["name"].as_str().unwrap_or("").starts_with("Open "));
    assert!(has_named(&tree, "Search repositories"));

    // 3. Pull requests across all four states: distinct icons + labels + CI labels
    let mut seen_pr_icons = std::collections::HashSet::new();
    for (state, expected_label) in [
        (PullState::Open, "Pull request open"),
        (PullState::Draft, "Draft pull request"),
        (PullState::Merged, "Pull request merged"),
        (PullState::Closed, "Pull request closed"),
    ] {
        let k = crate::themes::kit(rt.state.looks.active(true));
        let el = ui::pull_icon(&k, state);
        // Verify every state renders and carries its distinct accessible label in the page.
        let _ = el;
        rt.update(|s, _| {
            s.page = crate::Page::Pulls;
            s.pr_filter = state;
            if !s.data.pulls.iter().any(|p| p.state == state) {
                let mut sample = s.data.pulls[0].clone();
                sample.number = 900 + state as u32;
                sample.state = state;
                s.data.pulls.push(sample);
            }
        });
        let tree = snap(&mut rt);
        assert_tree_labelled(&format!("Pulls ({state:?})"), &tree);
        assert!(has_named(&tree, expected_label), "missing {expected_label} on Pulls ({state:?})");
        seen_pr_icons.insert(expected_label);
    }
    assert_eq!(seen_pr_icons.len(), 4);
    assert_ne!(icons::PULL.name, icons::PULL_DRAFT.name);
    assert_ne!(icons::PULL.name, icons::PULL_CLOSED.name);
    assert_ne!(icons::PULL_DRAFT.name, icons::PULL_CLOSED.name);
    assert_ne!(icons::PULL.name, icons::MERGE.name);

    // 4. Issues: open issue icon label + row label
    rt.update(|s, _| s.page = crate::Page::Issues);
    let tree = snap(&mut rt);
    assert_tree_labelled("Issues", &tree);
    assert!(has_named(&tree, "Open issue"));

    // 5. Actions: workflow run rows announce as links with workflow, state, branch and repo, plus CI states
    rt.update(|s, _| s.page = crate::Page::Actions);
    let tree = snap(&mut rt);
    assert_tree_labelled("Actions", &tree);
    assert!(has_named(&tree, "CI passing"));
    assert!(has_named(&tree, "CI failing"));
    assert!(has_named(&tree, "CI running"));
    let r0 = rt.state.data.runs[0].clone();
    // A run GitHub knows by id opens in the app (a button); one without opens GitHub (a link).
    assert!(r0.id > 0, "the sample runs have ids");
    let run_node = find_key(&tree, &format!("run-{}-{}", r0.repo, r0.id)).expect("run row");
    assert_eq!(run_node["role"].as_str(), Some("button"));
    let run_name = run_node["name"].as_str().unwrap_or("");
    assert!(run_name.contains(&r0.workflow) && run_name.contains(&r0.branch) && run_name.contains(&r0.repo), "{run_name}");

    // 6. Repository detail: Branch filter, New PR form + Draft switch, Local git change mark, Notes input, Terminal folder input
    rt.update(|s, _| {
        s.open(0);
        s.set_tab(0, crate::state::RepoTab::Code);
        s.picker = Some(crate::state::BranchSlot::View);
        let full = s.data.repos[0].full_name.clone();
        let mut changes = std::collections::HashMap::new();
        changes.insert("README.md".to_owned(), crate::local::Change::Modified);
        s.local.insert(full, crate::state::Load::Ready(crate::local::Tree {
            root: std::path::PathBuf::from("/tmp/example"),
            branch: "main".into(),
            files: vec!["README.md".into()],
            changes,
            cut: false,
        }));
        s.files_from_github = false;
    });
    let tree = snap(&mut rt);
    assert_tree_labelled("Repo Code + Branch picker", &tree);
    assert!(has_named(&tree, "Filter branches"));

    // Verify local_page::files_panel directly exposes git change_mark's full label ("Modified")
    let local_tree = {
        let mut sub = crate::init();
        sub.pending_place = None;
        sub.data = crate::model::Data::sample_at(1_700_000_000);
        let full = sub.data.repos[0].full_name.clone();
        let mut changes = std::collections::HashMap::new();
        changes.insert("README.md".to_owned(), crate::local::Change::Modified);
        sub.local.insert(full, crate::state::Load::Ready(crate::local::Tree {
            root: std::path::PathBuf::from("/tmp/example"),
            branch: "main".into(),
            files: vec!["README.md".into()],
            changes,
            cut: false,
        }));
        let mut rt_local = rune::Runtime::new(
            sub,
            |s, _cx| {
                let k = crate::themes::kit(s.looks.active(true));
                crate::local_page::files_panel(s, &k, 0, &s.data.repos[0])
            },
            (900.0, 700.0),
            rune::Env::new(true, 1.0),
        );
        rt_local.frame(0.1);
        rt_local.accessibility_json()
    };
    assert_tree_labelled("Local files panel", &local_tree);
    assert!(has_named(&local_tree, "Modified"));

    // New pull request form: Draft switch has label
    rt.update(|s, _| {
        s.picker = None;
        s.open_form(0, crate::state::Compose::Pull);
    });
    let tree = snap(&mut rt);
    assert_tree_labelled("New PR form", &tree);
    let draft_sw = find_key(&tree, "form-draft").expect("form-draft switch");
    assert_eq!(draft_sw["name"].as_str(), Some("Draft: not ready for review yet"));

    // Notes tab: note_name input has label
    rt.update(|s, _| {
        s.form = None;
        s.set_tab(0, crate::state::RepoTab::Notes);
    });
    let tree = snap(&mut rt);
    assert_tree_labelled("Repo Notes tab", &tree);
    assert!(has_named(&tree, "Name of the new drawing"));

    // Terminal tab (missing local folder): clone_path input has label
    let missing_tree = {
        let mut sub = crate::init();
        sub.pending_place = None;
        sub.data = crate::model::Data::sample_at(1_700_000_000);
        let mut rt_term = rune::Runtime::new(
            sub,
            |s, _cx| {
                let k = crate::themes::kit(s.looks.active(true));
                crate::terminal_page::pick_folder(s, &k, 0, false)
            },
            (900.0, 700.0),
            rune::Env::new(true, 1.0),
        );
        rt_term.frame(0.1);
        rt_term.accessibility_json()
    };
    assert_tree_labelled("Terminal pick_folder", &missing_tree);
    assert!(has_named(&missing_tree, "Local repository folder"));
}
