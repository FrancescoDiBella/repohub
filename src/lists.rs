//! Pull requests, issues, Actions and settings pages.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::model::{Checks, Issue, Pull, PullState, Repo, Run};
use crate::pages::{heading, open_button};
use crate::ui::{self, panel, panel_head, rule};
use crate::state::Account;
use crate::{App, Status, icons};

/// Makes a row open `url` in the browser when clicked, announcing itself as a link.
fn link(e: El, url: &str, label: String) -> El {
    if url.is_empty() {
        return e;
    }
    let url = url.to_owned();
    e.role("link").label(label).on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); })
}

pub fn pull_row(k: &Kit, p: &Pull, now: i64, compact: bool) -> El {
    let mut meta = row().gap(10.0).center_y().flex_wrap()
        .child(text(format!("#{} · {} · {}", p.number, p.repo, p.age_ago(now))).font(k.ty.text_small).color(k.c.text_muted));
    match p.state {
        PullState::Draft => meta = meta.child(ui::pill(k, "Draft", k.c.text_faint)),
        PullState::Closed => meta = meta.child(ui::pill(k, "Closed", k.c.danger.icon)),
        _ => {}
    }
    let mut r = row().w_full().px(16.0).py(11.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .child(ui::pull_icon(k, p.state))
        .child(col().grow(1.0).shrink().gap(3.0)
            .child(text(p.title.clone()).font(k.ty.ui_medium).color(k.c.text_base).wrap())
            .child(meta));
    if let Some(c) = p.checks {
        r = r.child(ui::checks(k, c));
    }
    if !compact {
        r = r.child(ui::stat(k, &icons::COMMENT, p.comments.to_string())).child(ui::user(k, &p.author, 22.0));
    }
    let state_word = match p.state {
        PullState::Open => "open",
        PullState::Draft => "draft",
        PullState::Merged => "merged",
        PullState::Closed => "closed",
    };
    let (repo, number) = (crate::changes_page::pull_repo(&p.url).unwrap_or_else(|| p.repo.clone()), p.number);
    r.key(format!("pr-{}-{}", p.repo, p.number))
        .role("button")
        .label(format!("Pull request #{} in {}, {}: {}", p.number, p.repo, state_word, p.title))
        .on_press(move |s: &mut App| s.open_thread(&repo, number, true))
}

pub fn issue_row(k: &Kit, i: &Issue, now: i64) -> El {
    let mut title = row().gap(8.0).center_y().flex_wrap().child(text(i.title.clone()).font(k.ty.ui_medium).color(k.c.text_base));
    // The first few labels; the conversation shows them all.
    for (name, hue) in i.labels.iter().take(3) {
        title = title.child(ui::label_chip(k, name, *hue));
    }
    if i.labels.len() > 3 {
        title = title.child(text(format!("+{}", i.labels.len() - 3)).font(k.ty.text_small).color(k.c.text_faint));
    }
    let r = row().w_full().px(16.0).py(11.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .child(ui::ico(&icons::ISSUE, 16.0, k.c.success.icon).label("Open issue"))
        .child(col().grow(1.0).shrink().gap(3.0)
            .child(title)
            .child(text(format!("#{} · {} · opened {}", i.number, i.repo, i.age_ago(now))).font(k.ty.text_small).color(k.c.text_muted)))
        .child(ui::stat(k, &icons::COMMENT, i.comments.to_string()));
    let (repo, number) = (i.repo.clone(), i.number);
    r.key(format!("issue-{}-{}", i.repo, i.number))
        .role("button")
        .label(format!("Issue #{} in {}, open: {}", i.number, i.repo, i.title))
        .on_press(move |s: &mut App| s.open_thread(&repo, number, false))
}

pub fn run_row(k: &Kit, r: &Run, now: i64, compact: bool) -> El {
    let state = match r.checks {
        Checks::Passing => ("Succeeded", k.c.success.icon),
        Checks::Failing => ("Failed", k.c.danger.icon),
        Checks::Running => ("Running", k.c.warning.icon),
    };
    let age = r.age_ago(now);
    let when = text(format!("{} · {}", r.duration, age)).font(k.ty.text_small).color(k.c.text_faint);
    let mut meta = row().gap(10.0).center_y().flex_wrap()
        .child(text(r.repo.clone()).font(k.ty.text_small).color(k.c.text_muted))
        .child(ui::stat(k, &icons::BRANCH, r.branch.clone()));
    // On a phone the duration moves under the title, so the title keeps its room.
    let trailing = if compact {
        meta = meta.child(when);
        None
    } else {
        Some(when.w(140.0))
    };
    let mut e = row().w_full().px(16.0).py(11.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .child(ui::checks(k, r.checks))
        .child(col().grow(1.0).shrink().gap(3.0)
            .child(text(r.workflow.clone()).font(k.ty.ui_medium).color(k.c.text_base).wrap())
            .child(meta))
        .child(ui::pill(k, state.0, state.1));
    if let Some(t) = trailing {
        e = e.child(t);
    }
    let label = format!("{}, {} on {} in {}, {}", r.workflow, state.0.to_lowercase(), r.branch, r.repo, age);
    if r.id > 0 {
        // A run GitHub knows by id opens in the app: its jobs, steps and logs.
        let run = r.clone();
        e.key(format!("run-{}-{}", r.repo, r.id))
            .role("button")
            .label(label)
            .on_press(move |s: &mut App| s.open_run(&run))
    } else {
        link(e.key(format!("run-{}-{}", r.repo, r.workflow)), &r.url, format!("{label}, opens on GitHub"))
    }
}

fn listing(s: &App, k: &Kit, rows: Vec<El>, empty: &str, retry_key: &str) -> El {
    if rows.is_empty() {
        if s.data.is_empty() {
            if s.status.is_busy() {
                return ui::skeleton_list(k, 5, "skeleton-list");
            }
            if let Status::Failed(e) = &s.status {
                return ui::error_panel(k, e, retry_key, |s: &mut App| s.refresh());
            }
        }
        return panel(k).padding(28.0).items_center().child(text(empty.to_owned()).font(k.ty.ui).color(k.c.text_muted));
    }
    rows.into_iter().enumerate().fold(panel(k), |p, (i, r)| if i == 0 { p.child(r) } else { p.child(rule(k)).child(r) })
}

/// The states the Pull requests page can show, in the order of its tabs.
pub const PR_STATES: [PullState; 4] = [PullState::Open, PullState::Draft, PullState::Merged, PullState::Closed];
pub const PR_LABELS: [&str; 4] = ["Open", "Drafts", "Merged", "Closed"];

pub fn pulls(s: &App, k: &Kit) -> El {
    let me = s.data.profile.login.as_str();
    let in_state: Vec<&Pull> = s.data.pulls.iter().filter(|p| p.state == s.pr_filter).collect();
    let (filter_repos, filter_labels) = crate::filters::pull_choices(in_state.iter().copied());
    let rows: Vec<El> = in_state.iter().filter(|p| s.pull_filter.pull(p, me)).map(|p| crate::actions::pull_line(s, k, p, s.compact)).collect();
    let empty = if s.pull_filter.is_empty() || in_state.is_empty() { "No pull requests in this state" } else { "No pull requests match these filters" };
    let is_open_group = matches!(s.pr_filter, PullState::Open | PullState::Draft);
    let (group_loaded, group_total) = if is_open_group {
        let count = s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
        (count, s.data.open_pulls_total)
    } else {
        let count = s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Closed | PullState::Merged)).count();
        (count, s.data.closed_pulls_total)
    };
    let can_load_more = !s.data.sample && group_total.is_some_and(|tot| group_loaded < tot && group_loaded < 1000);
    let is_capped = can_load_more || (is_open_group && s.data.prs_capped());

    let mut actions = row().gap(8.0).center_y();
    if can_load_more {
        let btn_label = if s.loading_more_pulls { "Loading…" } else { "Load more" };
        actions = actions.child(
            Button::new(btn_label)
                .control_size(ControlSize::Small)
                .disabled(s.loading_more_pulls)
                .on_press(move |s: &mut App| s.load_more_pulls(is_open_group))
                .build(k)
                .key("pulls-load-more")
        );
    }
    if !s.data.sample && is_capped {
        let me = &s.data.profile.login;
        let url = format!("{}/pulls?q=is%3Apr+involves%3A{me}", s.cfg.web_root());
        actions = actions.child(open_button(k, "Open on GitHub", &url, "pulls-gh"));
    }
    let action_el = if can_load_more || (!s.data.sample && is_capped) {
        Some(actions)
    } else {
        None
    };

    let sub = if s.data.sample {
        "Pull requests you opened or that wait for your review.".to_owned()
    } else if let Some(tot) = group_total.filter(|&t| t > group_loaded) {
        format!("Showing {} of {} {} pull requests you're involved in. Click one to read it and reply.", group_loaded, tot, if is_open_group { "open" } else { "closed/merged" })
    } else if let Some(tot) = group_total {
        format!("Showing all {} {} pull requests you're involved in. Click one to read it and reply.", tot, if is_open_group { "open" } else { "closed/merged" })
    } else if s.data.pulls.len() >= crate::model::SEARCH_CAP {
        format!("The {} most recently updated pull requests you're involved in (author, reviewer, assignee or mentioned). Click one to read it and reply.", s.data.pulls.len())
    } else {
        "Pull requests you're involved in (author, reviewer, assignee or mentioned). Click one to read it and reply.".to_owned()
    };
    col().w_full().gap(16.0)
        .child(heading(k, "Pull requests", &sub, action_el))
        .children(crate::changes_page::pull_panel(s, k))
        .child(SegmentedControl::new(&PR_LABELS, PR_STATES.iter().position(|p| *p == s.pr_filter)).label("State").fit()
            .on_pick(|s: &mut App, i, _| s.pr_filter = PR_STATES[i.min(3)]).build(k).key("pr-filter"))
        .child(crate::filters::bar(s, k, crate::filters::Which::Pulls, filter_repos, filter_labels))
        .child(listing(s, k, rows, empty, "retry-pulls"))
}

pub fn issues(s: &App, k: &Kit) -> El {
    let me = s.data.profile.login.as_str();
    let (filter_repos, filter_labels) = crate::filters::issue_choices(s.data.issues.iter());
    let rows: Vec<El> = s.data.issues.iter().filter(|x| s.issue_filter.issue(x, me)).map(|i| crate::actions::issue_line(s, k, i)).collect();
    let empty = if s.issue_filter.is_empty() || s.data.issues.is_empty() { "No open issues" } else { "No issues match these filters" };
    let loaded = s.data.issues.len();
    let can_load_more = !s.data.sample && s.data.issues_total.is_some_and(|tot| loaded < tot && loaded < 1000);
    let is_capped = can_load_more || s.data.issues_capped();

    let mut actions = row().gap(8.0).center_y();
    if can_load_more {
        let btn_label = if s.loading_more_issues { "Loading…" } else { "Load more" };
        actions = actions.child(
            Button::new(btn_label)
                .control_size(ControlSize::Small)
                .disabled(s.loading_more_issues)
                .on_press(|s: &mut App| s.load_more_issues())
                .build(k)
                .key("issues-load-more")
        );
    }
    if !s.data.sample && is_capped {
        let me = &s.data.profile.login;
        let url = format!("{}/issues?q=is%3Aissue+is%3Aopen+archived%3Afalse+user%3A{me}", s.cfg.web_root());
        actions = actions.child(open_button(k, "Open on GitHub", &url, "issues-gh"));
    }
    let action_el = if can_load_more || (!s.data.sample && is_capped) {
        Some(actions)
    } else {
        None
    };

    let sub = if let Some(total) = s.data.issues_total.filter(|&t| t > loaded) {
        format!("Showing {} of {} open issues in your repositories. Click one to read it and reply.", loaded, total)
    } else if let Some(total) = s.data.issues_total {
        format!("Showing all {} open issues in your repositories. Click one to read it and reply.", total)
    } else if loaded >= crate::model::SEARCH_CAP {
        format!("The {} most recently updated open issues in your repositories. Click one to read it and reply.", loaded)
    } else {
        "Open issues in your repositories. Click one to read it and reply.".to_owned()
    };
    col().w_full().gap(16.0)
        .child(heading(k, "Issues", &sub, action_el))
        .child(crate::filters::bar(s, k, crate::filters::Which::Issues, filter_repos, filter_labels))
        .child(listing(s, k, rows, empty, "retry-issues"))
}

pub fn actions(s: &App, k: &Kit) -> El {
    let ok = s.data.runs.iter().filter(|r| r.checks == Checks::Passing).count();
    let (filter_repos, filter_workflows) = crate::runs_page::choices(s.data.runs.iter());
    let rows: Vec<El> = s.data.runs.iter()
        .filter(|r| s.runs_filter.matches(r))
        .map(|r| crate::runs_page::run_line(s, k, r, s.compact))
        .collect();
    let empty = if s.runs_filter.is_empty() || s.data.runs.is_empty() {
        "No recent runs"
    } else {
        "No workflow runs match these filters"
    };
    let action = if !s.data.sample && s.data.runs_total > s.data.runs.len() {
        let me = &s.data.profile.login;
        let url = format!("{}/{me}", s.cfg.web_root());
        Some(open_button(k, "Open on GitHub", &url, "actions-gh"))
    } else {
        None
    };
    let sub = if s.data.is_empty() && s.status.is_busy() {
        "Loading recent workflow runs…".to_owned()
    } else if s.data.is_empty() && matches!(s.status, Status::Failed(_)) {
        "Recent workflow runs across your most recently pushed repositories.".to_owned()
    } else if s.data.runs_total > s.data.runs.len() {
        format!("Showing {} of {} recent runs ({} succeeded) · {} most recently pushed repositories. Click a run to inspect jobs and logs.", s.data.runs.len(), s.data.runs_total, ok, s.cfg.activity_repos)
    } else {
        format!("{ok} of {} recent runs succeeded · {} most recently pushed repositories. Click a run to inspect jobs and logs.", s.data.runs.len(), s.cfg.activity_repos)
    };
    col().w_full().gap(16.0)
        .child(heading(k, "Actions", &sub, action))
        .child(crate::runs_page::filter_bar(s, k, filter_repos, filter_workflows))
        .child(listing(s, k, rows, empty, "retry-actions"))
}

fn info(k: &Kit, label: &str, value: String) -> El {
    row().w_full().px(16.0).py(10.0).gap(16.0).center_y()
        .child(text(label.to_owned()).font(k.ty.small).color(k.c.text_muted).w(170.0))
        .child(text(value).font(k.ty.small_medium).color(k.c.text_base).grow(1.0).shrink().wrap())
}

pub fn settings(s: &App, k: &Kit) -> El {
    let p = &s.data.profile;
    let cfg = &s.cfg;
    let stars = s.starred_count();
    let profile = panel(k).padding(20.0).gap(16.0)
        .child(row().gap(14.0).center_y()
            .child(ui::user(k, if p.login.is_empty() { "?" } else { &p.login }, 56.0))
            .child(col().gap(3.0).shrink()
                .child(text(if p.name.is_empty() { "—".to_owned() } else { p.name.clone() }).font(k.ty.body_medium).color(k.c.text_strong))
                .child(text(format!("@{}{}", p.login, if p.bio.is_empty() { String::new() } else { format!(" · {}", p.bio) })).font(k.ty.small).color(k.c.text_muted).wrap())
                .child(row().gap(14.0).child(ui::stat(k, &icons::REPO, format!("{} repositories", s.data.repos.len()))).child(ui::stat(k, &icons::STAR, format!("{stars} stars"))))));
    let source = match (&s.status, cfg.live()) {
        (Status::Sample, _) if cfg.demo => "Sample data (REPOHUB_DEMO=true)".to_owned(),
        (Status::Sample, _) => "Sample data: no configuration found".to_owned(),
        (Status::Starting | Status::Loading, _) => "Loading…".to_owned(),
        (Status::Failed(e), _) => format!("Error: {e}"),
        (Status::Ready, _) => "GitHub, up to date".to_owned(),
    };
    // The state of the connection; its details (variables, files, sources) on request.
    let mut conn_tools = row().gap(6.0).center_y()
        .child(Button::new(if s.connection_details { "Hide details" } else { "Show details" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.connection_details = !s.connection_details).build(k).key("connection-details"));
    if cfg.live() {
        conn_tools = conn_tools.child(Button::new("Refresh").icon(&icons::REFRESH).control_size(ControlSize::Small).disabled(s.status.is_busy())
            .on_press(|s: &mut App| s.refresh()).build(k).key("refresh-settings"));
    }
    let mut conn = panel(k).key("connection").child(panel_head(k, "GitHub connection", Some(conn_tools)));
    for (label, value) in [
        ("State", source),
        ("Last update", match (s.freshness(), s.refresh_in(), cfg.refresh_every()) {
            (None, _, _) => "not loaded yet".into(),
            (Some(f), Some(left), Some(m)) => format!("{f} · next in {} (every {m} min, REPOHUB_REFRESH_MINUTES)", minutes(left)),
            (Some(f), _, _) => format!("{f} · automatic refresh off (REPOHUB_REFRESH_MINUTES=0, or no account)"),
        }),
        ("Settings file", cfg.env_file.clone().unwrap_or_else(|| "none yet".into())),
        ("Ignored in the project .env", if cfg.ignored_keys.is_empty() { String::new() } else {
            format!("{}: a .env found in a folder can't change these. Set them in the environment or in the app's settings file, or name the file with REPOHUB_ENV.", cfg.ignored_keys.join(", "))
        }),
        ("User (GITHUB_USER)", match (&cfg.user, &cfg.sources.user) {
            (Some(u), Some(src)) => format!("{u} (from {src})"),
            (Some(u), None) => u.clone(),
            (None, _) => "from the token".into(),
        }),
        ("Token (GITHUB_TOKEN)", match (&cfg.token, &cfg.sources.token) {
            (Some(_), Some(src)) => format!("set (not shown, from {src})"),
            (Some(_), None) => "set (not shown)".into(),
            (None, _) => "not set: public data only, 60 requests an hour".into(),
        }),
        ("API", match &cfg.sources.api {
            Some(src) => format!("{} (from {src})", cfg.api),
            None => cfg.api.clone(),
        }),
        ("Organizations (REPOHUB_ORGS)", {
            let base = match &cfg.orgs {
                crate::state::Orgs::All => match crate::orgs(s).iter().map(|o| o.0.clone()).collect::<Vec<_>>().join(", ") {
                    l if l.is_empty() => "all you belong to (none found)".into(),
                    l => format!("all you belong to: {l}"),
                },
                crate::state::Orgs::Off => "none: only your own repositories".into(),
                crate::state::Orgs::Only(l) => l.join(", "),
            };
            match &cfg.sources.orgs {
                Some(src) => format!("{base} (from {src})"),
                None => base,
            }
        }),
        ("Pinned", match (&s.pins, cfg.pinned.is_empty()) {
            (Some(p), _) => format!("{} chosen in the app (pin or unpin from a repository)", p.len()),
            (None, true) => "the 3 most recently pushed (REPOHUB_PINNED is empty)".into(),
            (None, false) => match &cfg.sources.pinned {
                Some(src) => format!("{} (REPOHUB_PINNED, from {src})", cfg.pinned.join(", ")),
                None => format!("{} (REPOHUB_PINNED)", cfg.pinned.join(", ")),
            },
        }),
        ("Notes", if cfg.device { "kept by the app on this device".into() } else if cfg.notes { notes_where() } else { "not available here (needs the desktop app or Docker)".into() }),
        ("Filters", format!("forks {}, archived {}, at most {} repositories", if cfg.include_forks { "yes" } else { "no" }, if cfg.include_archived { "yes" } else { "no" }, cfg.max_repos)),
    ] {
        if label.starts_with("Ignored") && value.is_empty() {
            continue;
        }
        if !s.connection_details && !matches!(label, "State" | "Last update") {
            continue;
        }
        conn = conn.child(rule(k)).child(info(k, label, value));
    }
    let account = account_panel(s, k);
    let appearance = crate::themes_page::appearance(s, k);
    let assistants = assistants_panel(s, k);
    let plugins = crate::plugins_page::plugins_panel(s, k);
    let version = version_panel(s, k);
    let keys = panel(k).child(panel_head(k, "Keyboard shortcuts", None));
    let keys = [
        ("⌘/Ctrl + K", "Search and commands: jump to any repository or page, run any action"),
        ("⌘/Ctrl + 1 … 5", "Overview, Repositories, Pull requests, Issues, Actions"),
        ("⌘/Ctrl + ,", "Settings"),
        ("⌘/Ctrl + R", "Refresh from GitHub"),
        ("⌘/Ctrl + [", "Back: out of a conversation, a commit or a comparison, or from a repository to the list"),
        ("Esc", "Close what's open: a confirmation, a form, a diff, a conversation, a commit, a comparison, a file, a message"),
        ("Ctrl + Shift + Tab", "Leave the terminal, so the keyboard reaches the rest of the app"),
        ("↑ ↓ in ⌘K", "Move through the results; Enter runs the one highlighted"),
    ].iter().fold(keys, |p, (key, what)| p.child(rule(k)).child(info(k, key, (*what).to_owned())));
    use crate::state::SettingsSection as Section;
    let section = s.settings_section;
    let picker = SegmentedControl::new(&Section::LABELS, Some(section.index())).label("Settings section").fit()
        .on_pick(|s: &mut App, i, _| s.settings_section = Section::ALL[i.min(Section::ALL.len() - 1)]).build(k).key("settings-section");
    // Five segments don't fit a phone: the row scrolls sideways there.
    let picker = if s.compact { row().w_full().scroll_x().child(picker) } else { row().w_full().child(picker) };
    let (subtitle, body): (&str, Vec<El>) = match section {
        Section::Account => ("Your profile, the GitHub account, the connection and where boards sync.", vec![profile, account, conn, crate::sync_page::settings_block(s, k)]),
        Section::Appearance => ("Light, dark or the system's, and the themes.", vec![appearance]),
        Section::Assistants => ("Claude and the other coding assistants the Terminal starts.", vec![assistants]),
        Section::Plugins => ("Sections and pages others wrote for RepoHub.", vec![plugins]),
        Section::About => ("This version of RepoHub, its updates and the keyboard shortcuts.", vec![version, keys]),
    };
    col().w_full().gap(16.0).max_w(760.0)
        .child(heading(k, "Settings", subtitle, None))
        .child(picker)
        .children(body)
}

/// Coding assistants: the Claude switch and the harnesses (name and command) the Terminal offers.
fn assistants_panel(s: &App, k: &Kit) -> El {
    use crate::harness::SUGGESTED;
    let a = &s.assist;
    let claude = row().w_full().px(16.0).py(12.0).gap(16.0).center_y().justify_between()
        .child(col().grow(1.0).shrink().gap(2.0)
            .child(text("Claude").font(k.ty.ui_medium).color(k.c.text_base))
            .child(text("The Claude section of each repository, and the buttons that start claude in the Terminal.")
                .font(k.ty.text_small).color(k.c.text_muted).wrap()))
        .child(rune_kit::Switch::new(a.claude).label("Claude section")
            .on_toggle(|s: &mut App| { let on = !s.assist.claude; s.set_claude(on) }).build(k).key("claude-switch"));
    let mut list = col().w_full().px(16.0).py(12.0).gap(8.0)
        .child(text("Harnesses").font(k.ty.ui_medium).color(k.c.text_base))
        .child(text("Each one gets a button in the Terminal and in ⌘K that starts it in the repository's folder.")
            .font(k.ty.text_small).color(k.c.text_muted).wrap());
    if a.harnesses.is_empty() {
        list = list.child(text("None yet: add one below.").font(k.ty.small).color(k.c.text_faint));
    }
    for (n, h) in a.harnesses.iter().enumerate() {
        let off = !a.allows(&h.command);
        list = list.child(row().w_full().gap(10.0).py(4.0).center_y().key(format!("harness-row-{n}"))
            .child(ui::ico(&icons::SPARK, 14.0, if off { k.c.icon_faint } else { k.c.text_accent }))
            .child(text(h.name.clone()).font(k.ty.small_medium).color(if off { k.c.text_faint } else { k.c.text_base }).w(140.0).shrink())
            .child(text(h.command.clone()).font(k.ty.mono).color(k.c.text_muted).grow(1.0).shrink().max_lines(1))
            .child(if off { ui::pill(k, "Claude is off", k.c.text_faint) } else { rune::el() })
            .child(Button::new("Remove").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.remove_harness(n)).build(k).key(format!("harness-remove-{n}"))));
    }
    let name = TextInput::new(&s.harness_name).placeholder("Name, e.g. Codex").label("Harness name")
        .on_edit(|s: &mut App| &mut s.harness_name).on_submit(|s: &mut App, _| s.add_harness(None)).build(k).key("harness-name").w(170.0);
    let cmd = TextInput::new(&s.harness_cmd).placeholder("Command, e.g. codex --full-auto").label("Command")
        .on_edit(|s: &mut App| &mut s.harness_cmd).on_submit(|s: &mut App, _| s.add_harness(None)).build(k).key("harness-command").grow(1.0).min_w(200.0);
    let add = Button::new("Add").icon(&icons::PLUS).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.add_harness(None)).build(k).key("harness-add");
    list = list.child(row().w_full().gap(8.0).pt(6.0).center_y().flex_wrap().child(name).child(cmd).child(add));
    if let Some(e) = &s.harness_error {
        list = list.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    let missing: Vec<_> = SUGGESTED.iter().filter(|(n, _)| !a.harnesses.iter().any(|h| h.name.eq_ignore_ascii_case(n))).collect();
    if !missing.is_empty() {
        let chips = missing.into_iter().map(|&(n, c)| Button::new(n).icon(&icons::PLUS).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .help(format!("Adds {n}: {c}")).on_press(move |s: &mut App| s.add_harness(Some((n, c)))).build(k).key(format!("harness-suggest-{n}")));
        list = list.child(row().w_full().gap(4.0).center_y().flex_wrap()
            .child(text("Add:").font(k.ty.small).color(k.c.text_muted).pr(4.0)).children(chips));
    }
    if cfg!(target_arch = "wasm32") {
        list = list.child(text("They run in the desktop app, where RepoHub can start programs on your computer.").font(k.ty.text_small).color(k.c.text_muted).wrap());
    }
    panel(k).key("assistants").child(panel_head(k, "Coding assistants", None)).child(rule(k))
        .child(claude).child(rule(k)).child(list)
}

/// The GitHub account form (desktop): user name and token, saved in the app's settings file.
fn account_panel(s: &App, k: &Kit) -> El {
    let cfg = &s.cfg;
    // In a browser the account is the server's; the phone app keeps its own.
    if cfg!(target_arch = "wasm32") && cfg.account_locked {
        return rune::el();
    }
    let head = panel(k).child(panel_head(k, "GitHub account", Some(open_button(k, "Create a token", "https://github.com/settings/personal-access-tokens/new", "new-token")))).child(rule(k));
    let account = s.account();
    if account == Account::Locked {
        let from = match &cfg.env_file {
            Some(file) => format!("{file} or from the environment"),
            None => "the environment".into(),
        };
        return head.child(col().w_full().padding(16.0).gap(4.0)
            .child(text("Set outside the app").font(k.ty.ui_medium).color(k.c.text_base))
            .child(text(format!("GITHUB_USER or GITHUB_TOKEN comes from {from}, and that wins. Change it there, or remove it to manage the account here."))
                .font(k.ty.text_small).color(k.c.text_muted).wrap()));
    }
    let label = |t: &str| text(t.to_owned()).font(k.ty.small).color(k.c.text_muted).w(110.0);
    let user = TextInput::new(&s.acct_user).placeholder("octocat").label("GitHub user name")
        .on_edit(|s: &mut App| &mut s.acct_user).on_submit(|s: &mut App, _| s.save_account(false)).build(k).key("acct-user").grow(1.0).min_w(200.0);
    let token = TextInput::new(&s.acct_token).secure().label("Personal access token")
        .placeholder(if cfg.token.is_some() { "A token is saved. Type a new one to replace it" } else { "github_pat_… (optional for public data)" })
        .on_edit(|s: &mut App| &mut s.acct_token).on_submit(|s: &mut App, _| s.save_account(false)).build(k).key("acct-token").grow(1.0).min_w(200.0);
    let mut buttons = row().gap(8.0).center_y().flex_wrap()
        .child(Button::new(if account == Account::SignedIn { "Save" } else { "Connect" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.save_account(false)).build(k).key("acct-save"));
    if account == Account::SignedIn {
        buttons = buttons.child(Button::new("Sign out").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.save_account(true)).build(k).key("acct-sign-out"));
    }
    let mut body = col().w_full().padding(16.0).gap(12.0)
        .child(row().w_full().gap(12.0).center_y().flex_wrap().child(label("User name")).child(user))
        .child(row().w_full().gap(12.0).center_y().flex_wrap().child(label("Token")).child(token))
        .child(text("With only a user name RepoHub shows public repositories (60 requests an hour). A token adds private repositories, starring and 5,000 requests an hour: a fine-grained token with read access to Metadata and Actions, and read and write access to Contents, Issues and Pull requests (to create branches, issues and pull requests from the app). A fine-grained token sees one owner's repositories; for your organizations' private repositories use a classic token with repo and read:org. ".to_owned() + if cfg.device { "It stays on this device, encrypted with a key that never leaves it." } else { "It is saved on this computer only, in a file only you can read." })
            .font(k.ty.text_small).color(k.c.text_muted).wrap());
    if let Some(e) = &s.acct_error {
        body = body.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    head.child(body.child(buttons))
}

/// The app's build and its updates.
fn version_panel(s: &App, k: &Kit) -> El {
    use crate::update::{self, Update};
    let v = update::version();
    let build = match (update::build(), update::built_at()) {
        (Some(b), Some(t)) => format!("{v} · {} · built {}", &b[..b.len().min(7)], crate::model::ago(t, s.now.max(t))),
        (Some(b), None) => format!("{v} · {}", &b[..b.len().min(7)]),
        (None, _) => format!("{v} · built from source"),
    };
    let what = s.update_channel.what();
    let (state, ink) = match &s.update {
        Update::Unavailable(why) => ((*why).to_owned(), k.c.text_muted),
        Update::Idle => ("Not checked yet.".into(), k.c.text_muted),
        Update::Checking => (format!("Checking {what} on GitHub…"), k.c.text_muted),
        Update::Current => (format!("Up to date with {what}."), k.c.success.icon),
        Update::Available(r) => (format!("A new version is ready: {} (built {}).", r.short(), crate::model::ago(r.published, s.now.max(r.published))), k.c.success.icon),
        Update::Downloading { percent, .. } => (format!("Downloading… {percent}%"), k.c.text_muted),
        Update::Installing => ("Installing; RepoHub restarts in a moment.".into(), k.c.text_muted),
        Update::Failed(e) => (e.clone(), k.c.danger.icon),
    };
    let mut buttons = row().gap(8.0);
    if s.update.can_check() {
        buttons = buttons.child(Button::new("Check now").icon(&icons::REFRESH).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.check_update(true)).build(k).key("update-check"));
    }
    if s.update.available().is_some() {
        buttons = buttons.child(Button::new("Install and restart").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.install_update()).build(k).key("update-install-settings"));
    }
    if matches!(s.update, Update::Downloading { .. }) {
        buttons = buttons.child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.cancel_update()).build(k).key("update-cancel-settings"));
    }
    panel(k).child(panel_head(k, "App version", Some(buttons))).child(rule(k))
        .child(info(k, "This build", build))
        .child(rule(k))
        .child(row().w_full().px(16.0).py(10.0).gap(16.0).center_y().flex_wrap()
            .child(text("Channel").font(k.ty.small).color(k.c.text_muted).w(170.0))
            .child(SegmentedControl::new(&update::Channel::LABELS, Some(s.update_channel.index())).label("Update channel").fit()
                .on_pick(|s: &mut App, i, _| s.set_update_channel(update::Channel::ALL[i.min(1)])).build(k).key("update-channel"))
            .child(text(match s.update_channel {
                update::Channel::Stable => "Numbered releases, tested before they're published.",
                update::Channel::Preview => "The Latest build: every change as soon as it's merged.",
            }).font(k.ty.small).color(k.c.text_muted).grow(1.0).shrink().wrap()))
        .child(rule(k))
        .child(row().w_full().px(16.0).py(10.0).gap(16.0).center_y()
            .child(text("Updates").font(k.ty.small).color(k.c.text_muted).w(170.0))
            .child(text(state).font(k.ty.small_medium).color(ink).grow(1.0).shrink().wrap()))
}

/// "7 minutes", "less than a minute".
fn minutes(secs: i64) -> String {
    match secs.max(0) / 60 {
        0 => "less than a minute".into(),
        1 => "1 minute".into(),
        m => format!("{m} minutes"),
    }
}

fn notes_where() -> String {
    crate::notes::location()
}

pub fn repo_settings(s: &App, k: &Kit, r: &Repo) -> El {
    let settings_url = format!("{}/settings", r.url);
    let danger = panel(k).border(1.0, k.c.danger.icon.alpha(0.5))
        .child(panel_head(k, "Danger zone", None))
        .child(rule(k))
        .child(row().w_full().px(16.0).py(12.0).gap(12.0).center_y().justify_between()
            .child(col().grow(1.0).shrink().gap(2.0)
                .child(text("Archive or delete the repository").font(k.ty.ui_medium).color(k.c.text_base))
                .child(text("RepoHub doesn't archive or delete repositories: do this on GitHub.").font(k.ty.text_small).color(k.c.text_muted).wrap()))
            .child(open_button(k, "Settings on GitHub", &settings_url, "gh-settings")));
    let general = panel(k).child(panel_head(k, &format!("General · {}", r.name), None))
        .child(rule(k)).child(info(k, "Full name", r.full_name.clone()))
        .child(rule(k)).child(info(k, "Visibility", if r.private { "private".into() } else { "public".into() }))
        .child(rule(k)).child(info(k, "Default branch", r.branch.clone()))
        .child(rule(k)).child(info(k, "Last push", r.updated_ago(s.now)));
    col().w_full().gap(16.0).child(general).child(danger.child(row().h(0.0)))
        .child(Button::new("Open the repository on GitHub").variant(ButtonVariant::Ghost).icon(&icons::EXTERNAL).control_size(ControlSize::Small)
            .on_press_tx({ let u = r.url.clone(); move |_: &mut App, tx| { let _ = tx.open_url(&u); } }).build(k).self_start())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod a11y_tests {
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
}
