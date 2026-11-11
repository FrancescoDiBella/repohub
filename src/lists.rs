//! Pull requests, issues, Actions and settings pages.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::model::{Checks, Issue, Pull, PullState, Repo, Run};
use crate::pages::{heading, open_button};
use crate::ui::{self, panel, panel_head, rule};
use crate::state::{Account, Theme};
use crate::{App, Status, icons};

/// Makes a row open `url` in the browser when clicked.
fn link(e: El, url: &str) -> El {
    if url.is_empty() {
        return e;
    }
    let url = url.to_owned();
    e.on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); })
}

pub fn pull_row(k: &Kit, p: &Pull, compact: bool) -> El {
    let mut meta = row().gap(10.0).center_y().flex_wrap()
        .child(text(format!("#{} · {} · {}", p.number, p.repo, p.age)).font(k.ty.text_small).color(k.c.text_muted));
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
    link(r.key(format!("pr-{}-{}", p.repo, p.number)), &p.url)
}

pub fn issue_row(k: &Kit, i: &Issue) -> El {
    let mut title = row().gap(8.0).center_y().flex_wrap().child(text(i.title.clone()).font(k.ty.ui_medium).color(k.c.text_base));
    if let Some((name, hue)) = &i.label {
        title = title.child(ui::label_chip(k, name, *hue));
    }
    let r = row().w_full().px(16.0).py(11.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .child(ui::ico(&icons::ISSUE, 16.0, k.c.success.icon))
        .child(col().grow(1.0).shrink().gap(3.0)
            .child(title)
            .child(text(format!("#{} · {} · opened {}", i.number, i.repo, i.age)).font(k.ty.text_small).color(k.c.text_muted)))
        .child(ui::stat(k, &icons::COMMENT, i.comments.to_string()));
    link(r.key(format!("issue-{}-{}", i.repo, i.number)), &i.url)
}

pub fn run_row(k: &Kit, r: &Run, compact: bool) -> El {
    let state = match r.checks {
        Checks::Passing => ("Succeeded", k.c.success.icon),
        Checks::Failing => ("Failed", k.c.danger.icon),
        Checks::Running => ("Running", k.c.warning.icon),
    };
    let when = text(format!("{} · {}", r.duration, r.age)).font(k.ty.text_small).color(k.c.text_faint);
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
    link(e, &r.url)
}

fn listing(k: &Kit, rows: Vec<El>, empty: &str) -> El {
    if rows.is_empty() {
        return panel(k).padding(28.0).items_center().child(text(empty.to_owned()).font(k.ty.ui).color(k.c.text_muted));
    }
    rows.into_iter().enumerate().fold(panel(k), |p, (i, r)| if i == 0 { p.child(r) } else { p.child(rule(k)).child(r) })
}

/// The states the Pull requests page can show, in the order of its tabs.
pub const PR_STATES: [PullState; 4] = [PullState::Open, PullState::Draft, PullState::Merged, PullState::Closed];
pub const PR_LABELS: [&str; 4] = ["Open", "Drafts", "Merged", "Closed"];

pub fn pulls(s: &App, k: &Kit) -> El {
    let rows: Vec<El> = s.data.pulls.iter().filter(|p| p.state == s.pr_filter).map(|p| crate::actions::pull_line(s, k, p, s.compact)).collect();
    let sub = if s.data.sample { "Pull requests you opened or that wait for your review." } else { "Pull requests you're involved in (author, reviewer, assignee or mentioned). Click one to open it on GitHub." };
    col().w_full().gap(16.0)
        .child(heading(k, "Pull requests", sub, None))
        .child(SegmentedControl::new(&PR_LABELS, PR_STATES.iter().position(|p| *p == s.pr_filter)).label("State").fit()
            .on_pick(|s: &mut App, i, _| s.pr_filter = PR_STATES[i.min(3)]).build(k).key("pr-filter"))
        .child(listing(k, rows, "No pull requests in this state"))
}

pub fn issues(s: &App, k: &Kit) -> El {
    col().w_full().gap(16.0)
        .child(heading(k, "Issues", "Open issues in your repositories. Click one to open it on GitHub.", None))
        .child(listing(k, s.data.issues.iter().map(|i| crate::actions::issue_line(s, k, i)).collect(), "No open issues"))
}

pub fn actions(s: &App, k: &Kit) -> El {
    let ok = s.data.runs.iter().filter(|r| r.checks == Checks::Passing).count();
    let sub = format!("{ok} of {} recent runs succeeded · {} most recently pushed repositories", s.data.runs.len(), s.cfg.activity_repos);
    col().w_full().gap(16.0)
        .child(heading(k, "Actions", &sub, None))
        .child(listing(k, s.data.runs.iter().map(|r| run_row(k, r, s.compact)).collect(), "No recent runs"))
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
    let mut conn = panel(k).child(panel_head(k, "GitHub connection", if cfg.live() {
        Some(Button::new("Refresh").icon(&icons::REFRESH).control_size(ControlSize::Small).disabled(s.status.is_busy())
            .on_press(|s: &mut App| s.refresh()).build(k).key("refresh-settings"))
    } else { None }));
    for (label, value) in [
        ("State", source),
        ("Last update", match (s.freshness(), s.refresh_in(), cfg.refresh_every()) {
            (None, _, _) => "not loaded yet".into(),
            (Some(f), Some(left), Some(m)) => format!("{f} · next in {} (every {m} min, REPOHUB_REFRESH_MINUTES)", minutes(left)),
            (Some(f), _, _) => format!("{f} · automatic refresh off (REPOHUB_REFRESH_MINUTES=0, or no account)"),
        }),
        ("Settings file", cfg.env_file.clone().unwrap_or_else(|| "none yet".into())),
        ("User (GITHUB_USER)", cfg.user.clone().unwrap_or_else(|| "from the token".into())),
        ("Token (GITHUB_TOKEN)", if cfg.token.is_some() { "set (not shown)".into() } else { "not set: public data only, 60 requests an hour".into() }),
        ("API", cfg.api.clone()),
        ("Pinned (REPOHUB_PINNED)", if cfg.pinned.is_empty() { "the 3 most recently pushed".into() } else { cfg.pinned.join(", ") }),
        ("Notes", if cfg.notes { notes_where() } else { "not available here (needs the desktop app or Docker)".into() }),
        ("Filters", format!("forks {}, archived {}, at most {} repositories", if cfg.include_forks { "yes" } else { "no" }, if cfg.include_archived { "yes" } else { "no" }, cfg.max_repos)),
    ] {
        conn = conn.child(rule(k)).child(info(k, label, value));
    }
    let account = account_panel(s, k);
    let appearance = panel(k).child(panel_head(k, "Appearance", None)).child(rule(k))
        .child(row().w_full().px(16.0).py(12.0).gap(16.0).center_y().justify_between().flex_wrap()
            .child(col().grow(1.0).shrink().min_w(200.0).gap(2.0)
                .child(text("Theme").font(k.ty.ui_medium).color(k.c.text_base))
                .child(text("System follows your computer's light or dark setting. Your choice is remembered; REPOHUB_THEME in .env only sets the default.")
                    .font(k.ty.text_small).color(k.c.text_muted).wrap()))
            .child(SegmentedControl::new(&Theme::LABELS, Some(s.theme.index())).label("Theme").fit()
                .on_pick(|s: &mut App, i, _| s.set_theme(Theme::ALL[i.min(2)])).build(k).key("theme-pick")));
    let keys = panel(k).child(panel_head(k, "Keyboard shortcuts", None));
    let keys = [
        ("⌘/Ctrl + K", "Search and commands: jump to any repository or page, run any action"),
        ("⌘/Ctrl + 1 … 5", "Overview, Repositories, Pull requests, Issues, Actions"),
        ("⌘/Ctrl + ,", "Settings"),
        ("⌘/Ctrl + R", "Refresh from GitHub"),
    ].iter().fold(keys, |p, (key, what)| p.child(rule(k)).child(info(k, key, (*what).to_owned())));
    col().w_full().gap(16.0).max_w(760.0)
        .child(heading(k, "Settings", "Profile, appearance and the GitHub connection.", None))
        .child(profile)
        .child(account)
        .child(appearance)
        .child(conn)
        .child(keys)
}

/// The GitHub account form (desktop): user name and token, saved in the app's settings file.
fn account_panel(s: &App, k: &Kit) -> El {
    if cfg!(target_arch = "wasm32") {
        return rune::el();
    }
    let cfg = &s.cfg;
    let head = panel(k).child(panel_head(k, "GitHub account", Some(open_button(k, "Create a token", "https://github.com/settings/personal-access-tokens/new", "new-token")))).child(rule(k));
    let account = s.account();
    if account == Account::Locked {
        let from = cfg.env_file.clone().unwrap_or_else(|| "the environment".into());
        return head.child(col().w_full().padding(16.0).gap(4.0)
            .child(text("Set outside the app").font(k.ty.ui_medium).color(k.c.text_base))
            .child(text(format!("GITHUB_USER or GITHUB_TOKEN comes from {from} or from the environment, and that wins. Change it there, or remove it to manage the account here."))
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
        .child(text("With only a user name RepoHub shows public repositories (60 requests an hour). A token adds private repositories, starring and 5,000 requests an hour: a fine-grained token with read access to Metadata and Actions, and read and write access to Contents, Issues and Pull requests (to create branches, issues and pull requests from the app). It is saved on this computer only, in a file only you can read.")
            .font(k.ty.text_small).color(k.c.text_muted).wrap());
    if let Some(e) = &s.acct_error {
        body = body.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    head.child(body.child(buttons))
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
                .child(text("RepoHub never changes repositories: do this on GitHub.").font(k.ty.text_small).color(k.c.text_muted).wrap()))
            .child(open_button(k, "Settings on GitHub", &settings_url, "gh-settings")));
    let general = panel(k).child(panel_head(k, &format!("General · {}", r.name), None))
        .child(rule(k)).child(info(k, "Full name", r.full_name.clone()))
        .child(rule(k)).child(info(k, "Visibility", if r.private { "private".into() } else { "public".into() }))
        .child(rule(k)).child(info(k, "Default branch", r.branch.clone()))
        .child(rule(k)).child(info(k, "Last push", r.updated.clone()));
    let _ = s;
    col().w_full().gap(16.0).child(general).child(danger.child(row().h(0.0)))
        .child(Button::new("Open the repository on GitHub").variant(ButtonVariant::Ghost).icon(&icons::EXTERNAL).control_size(ControlSize::Small)
            .on_press_tx({ let u = r.url.clone(); move |_: &mut App, tx| { let _ = tx.open_url(&u); } }).build(k).self_start())
}
