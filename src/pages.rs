//! Dashboard, repository list and repository detail.
use rune::{El, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::lists::{issue_row, pull_row, run_row};
use crate::model::{PullState, Repo, lang_hue};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, Page, icons};

pub fn heading(k: &Kit, title: &str, subtitle: &str, action: Option<El>) -> El {
    let mut r = row().w_full().center_y().justify_between().gap(12.0).flex_wrap()
        .child(col().gap(4.0).grow(1.0).shrink().min_w(200.0)
            .child(text(title.to_owned()).font(ui::H1).color(k.c.text_strong).wrap())
            .child(text(subtitle.to_owned()).font(k.ty.ui).color(k.c.text_muted).wrap()));
    if let Some(a) = action {
        r = r.child(a);
    }
    r
}

fn new_repo_button(k: &Kit) -> El {
    Button::new("New repository").variant(ButtonVariant::Submit).icon(&icons::PLUS)
        .on_press_tx(|_: &mut App, tx| { let _ = tx.open_url("https://github.com/new"); })
        .build(k).key("new-repo")
}

/// A button that opens `url` in the browser.
pub fn open_button(k: &Kit, label: &str, url: &str, key: &str) -> El {
    let url = url.to_owned();
    Button::new(label).icon(&icons::EXTERNAL).control_size(ControlSize::Small)
        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); })
        .build(k).key(key.to_owned())
}

// ---------------------------------------------------------------- dashboard

pub fn overview(s: &App, k: &Kit) -> El {
    let d = &s.data;
    let open_prs = d.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let issues = if d.sample { d.repos.iter().map(|r| r.issues).sum::<u32>() as usize } else { d.issues.len() };
    let metrics = row().w_full().gap(12.0).flex_wrap()
        .child(ui::metric(k, &icons::REPO, &d.repos.len().to_string(), "Repositories", k.c.info.icon))
        .child(ui::metric(k, &icons::STAR, &s.starred_count().to_string(), "Stars received", k.c.warning.icon))
        .child(ui::metric(k, &icons::PULL, &open_prs.to_string(), "Open pull requests", k.c.success.icon))
        .child(ui::metric(k, &icons::ISSUE, &issues.to_string(), "Open issues", k.c.danger.icon));
    let commits: u32 = d.activity.iter().sum();
    let activity = panel(k)
        .child(panel_head(k, "Activity · last 26 weeks", Some(text(format!("{commits} commits")).font(k.ty.small).color(k.c.text_muted))))
        .child(col().w_full().px(16.0).pb(16.0).child(ui::sparkline(k, &d.activity, 72.0)));
    let recent = d.repos.iter().enumerate().take(4).fold(
        panel(k).child(panel_head(k, "Recent repositories", Some(see_all(k, "See all", Page::Repos)))),
        |p, (i, r)| p.child(rule(k)).child(compact_repo(k, i, r)),
    );
    let review = d.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).take(4).fold(
        panel(k).child(panel_head(k, "Open pull requests", Some(see_all(k, "All pull requests", Page::Pulls)))),
        |p, pr| p.child(rule(k)).child(pull_row(k, pr, true)),
    );
    let first = d.profile.name.split_whitespace().next().unwrap_or("").to_owned();
    let hello = if first.is_empty() { "Welcome back".to_owned() } else { format!("Welcome back, {first}") };
    col().w_full().gap(16.0)
        .child(heading(k, &hello, "Here's what's happening in your repositories today.", Some(new_repo_button(k))))
        .child(metrics)
        .child(activity)
        .child(row().w_full().gap(16.0).flex_wrap()
            .child(col().grow(1.0).min_w(if s.compact { 260.0 } else { 320.0 }).child(recent))
            .child(col().grow(1.0).min_w(if s.compact { 260.0 } else { 320.0 }).child(review)))
}

fn see_all(k: &Kit, label: &str, page: Page) -> El {
    Button::new(label).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.go(page)).build(k)
}

fn compact_repo(k: &Kit, i: usize, r: &Repo) -> El {
    row().w_full().px(16.0).py(10.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .on_press(move |s: &mut App| s.open(i)).key(format!("recent-{}", r.full_name))
        .child(ui::avatar(k, &r.name, r.hue, 28.0))
        .child(col().grow(1.0).shrink().gap(2.0)
            .child(text(r.name.clone()).font(k.ty.ui_medium).color(k.c.text_base))
            .child(row().gap(12.0).child(ui::language(k, &r.language, r.hue)).child(ui::stat(k, &icons::STAR, r.stars.to_string()))))
        .child(text(r.updated.clone()).font(k.ty.text_small).color(k.c.text_faint))
}

// ---------------------------------------------------------------- repositories

pub const FILTERS: [&str; 4] = ["All", "Public", "Private", "Fork"];

pub fn repos(s: &App, k: &Kit) -> El {
    let q = s.search.text.to_lowercase();
    let all = &s.data.repos;
    let shown: Vec<usize> = (0..all.len()).filter(|&i| {
        let r = &all[i];
        let f = match s.filter {
            1 => !r.private,
            2 => r.private,
            3 => r.fork,
            _ => true,
        };
        f && (q.is_empty() || r.name.to_lowercase().contains(&q) || r.description.to_lowercase().contains(&q) || r.topics.iter().any(|t| t.contains(&q)))
    }).collect();
    let search = TextInput::new(&s.search).placeholder("Search by name, description or topic…").leading_icon(k.glyph.search)
        .on_edit(|s: &mut App| &mut s.search).build(k).key("search").grow(1.0).min_w(200.0);
    let seg = SegmentedControl::new(&FILTERS, Some(s.filter)).label("Filter").fit()
        .on_pick(|s: &mut App, i, _| s.filter = i).build(k).key("filter");
    let mut list = col().w_full().gap(12.0);
    for &i in &shown {
        list = list.child(repo_card(k, i, &all[i]));
    }
    if shown.is_empty() {
        let msg = if all.is_empty() { "No repositories to show" } else { "No repositories match your search" };
        list = list.child(panel(k).padding(32.0).items_center().gap(6.0)
            .child(ui::ico(&icons::REPO, 24.0, k.c.icon_faint))
            .child(text(msg).font(k.ty.ui).color(k.c.text_muted)));
    }
    col().w_full().gap(16.0)
        .child(heading(k, "Repositories", &format!("{} of {} repositories", shown.len(), all.len()), Some(new_repo_button(k))))
        .child(row().w_full().gap(12.0).center_y().flex_wrap().child(search).child(seg))
        .child(list)
}

fn star_button(k: &Kit, i: usize, r: &Repo, with_count: bool, key: String) -> El {
    let on = r.starred;
    let label = match (on, with_count) {
        (true, true) => format!("Starred · {}", r.stars),
        (false, true) => format!("Star · {}", r.stars),
        (true, false) => "Starred".into(),
        (false, false) => "Star".into(),
    };
    Button::new(label).icon(if on { &icons::STAR_ON } else { &icons::STAR })
        .variant(if on { ButtonVariant::Warning } else { ButtonVariant::Neutral }).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.toggle_star(i)).build(k).key(key)
}

fn repo_card(k: &Kit, i: usize, r: &Repo) -> El {
    let vis = if r.private { Badge::new("Private").variant(BadgeVariant::Subtle).build(k) } else { Badge::new("Public").build(k) };
    let mut title = row().gap(8.0).center_y().flex_wrap()
        .child(text(r.name.clone()).font(k.ty.body_medium).color(k.c.text_accent)
            .on_press(move |s: &mut App| s.open(i)).key(format!("open-{}", r.full_name)))
        .child(vis);
    if r.fork {
        title = title.child(Badge::new("Fork").variant(BadgeVariant::Accent).build(k));
    }
    if r.archived {
        title = title.child(ui::pill(k, "Archived", k.c.warning.icon));
    }
    let topics = row().gap(6.0).flex_wrap().children(r.topics.iter().take(8).map(|t| ui::topic(k, t)));
    let foot = row().gap(16.0).center_y().flex_wrap()
        .child(ui::language(k, &r.language, r.hue))
        .child(ui::stat(k, &icons::STAR, r.stars.to_string()))
        .child(ui::stat(k, &icons::FORK, r.forks.to_string()))
        .child(ui::stat(k, &icons::ISSUE, r.issues.to_string()))
        .child(text(format!("Updated {}", r.updated)).font(k.ty.text_small).color(k.c.text_faint));
    let desc = if r.description.is_empty() { "No description".to_owned() } else { r.description.clone() };
    panel(k).padding(16.0).gap(10.0).key(format!("card-{}", r.full_name))
        .child(row().w_full().justify_between().gap(12.0)
            .child(col().grow(1.0).shrink().gap(6.0)
                .child(title)
                .child(text(desc).font(k.ty.ui).color(k.c.text_muted).wrap()))
            .child(star_button(k, i, r, false, format!("star-{}", r.full_name))))
        .child(topics)
        .child(foot)
}

// ---------------------------------------------------------------- repository detail

pub const TABS: [&str; 7] = ["Code", "Issues", "Pull requests", "Actions", "Notes", "Terminal", "Settings"];
pub const TERMINAL_TAB: usize = 5;

pub fn repo_detail(s: &App, k: &Kit, i: usize) -> El {
    let r = &s.data.repos[i];
    let back = Button::new("Repositories").icon(&icons::BACK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.go(Page::Repos)).build(k).key("back").self_start();
    let vis = if r.private { ui::pill(k, "Private", k.c.warning.icon) } else { ui::pill(k, "Public", k.c.text_muted) };
    let header = row().w_full().center_y().justify_between().gap(12.0).flex_wrap()
        .child(row().gap(10.0).center_y()
            .child(ui::avatar(k, &r.name, r.hue, 36.0))
            .child(col().gap(2.0)
                .child(text(format!("{} /", r.owner)).font(k.ty.text_small).color(k.c.text_muted))
                .child(row().gap(8.0).center_y().child(text(r.name.clone()).font(k.ty.body_medium).color(k.c.text_strong)).child(vis))))
        .child(row().gap(8.0).flex_wrap()
            .child(open_button(k, "Open on GitHub", &r.url, "open-github"))
            .child(Button::new(format!("Fork · {}", r.forks)).icon(&icons::FORK).control_size(ControlSize::Small).build(k).key("fork"))
            .child(star_button(k, i, r, true, "detail-star".into())));
    let tabs = SegmentedControl::new(&TABS, Some(s.tab)).label("Section").fit().on_pick(move |s: &mut App, t, _| {
        s.tab = t;
        if t == 4 {
            s.load_notes(i);
        }
        if t == TERMINAL_TAB {
            s.open_terminal(i);
        }
    }).build(k).key("tabs");
    // Seven sections don't fit a phone's width: there the strip scrolls sideways.
    let tabs = if s.compact { row().w_full().scroll_x().child(tabs) } else { tabs };
    let in_repo = |name: &str| name.eq_ignore_ascii_case(&r.full_name) || name.eq_ignore_ascii_case(&r.name);
    let body = match s.tab {
        0 => code_tab(s, k, r),
        1 => list_of(k, "Open issues", s.data.issues.iter().filter(|x| in_repo(&x.repo)).map(|x| issue_row(k, x)).collect()),
        2 => list_of(k, "Pull requests", s.data.pulls.iter().filter(|x| in_repo(&x.repo)).map(|x| pull_row(k, x, s.compact)).collect()),
        3 => list_of(k, "Workflow runs", s.data.runs.iter().filter(|x| in_repo(&x.repo)).map(|x| run_row(k, x, s.compact)).collect()),
        4 => crate::notes_page::notes_tab(s, k, i),
        TERMINAL_TAB => crate::terminal_page::terminal_tab(s, k, i, s.prefs[0] || k.c.page.r + k.c.page.g + k.c.page.b < 1.0),
        _ => crate::lists::repo_settings(s, k, r),
    };
    col().w_full().gap(16.0).child(back).child(header).child(tabs).child(body)
}

fn list_of(k: &Kit, title: &str, rows: Vec<El>) -> El {
    let empty = rows.is_empty();
    let mut p = panel(k).child(panel_head(k, title, None));
    for r in rows {
        p = p.child(rule(k)).child(r);
    }
    if empty {
        p = p.child(rule(k)).child(col().w_full().padding(28.0).items_center()
            .child(text("Nothing to show here").font(k.ty.ui).color(k.c.text_muted)));
    }
    p
}

fn code_tab(s: &App, k: &Kit, r: &Repo) -> El {
    let detail = s.details.get(&r.full_name);
    let branch = Button::new(r.branch.clone()).icon(&icons::BRANCH).control_size(ControlSize::Small).build(k).key("branch");
    let clone_url = format!("{}.git", r.url);
    let clone = Button::new("Copy clone URL").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .on_press_tx(move |s: &mut App, tx| { tx.copy(clone_url.clone()); s.toast = Some("Clone URL copied".into()); })
        .build(k).key("clone");
    let mut files = panel(k);
    match detail {
        None => files = files.child(col().w_full().padding(24.0).items_center().child(text("Open the repository to load its files").font(k.ty.ui).color(k.c.text_muted))),
        Some(d) if d.loading => files = files.child(col().w_full().padding(24.0).items_center().child(text("Loading files…").font(k.ty.ui).color(k.c.text_muted))),
        Some(d) => {
            if let Some(c) = &d.commit {
                files = files.child(row().w_full().px(16.0).py(10.0).gap(10.0).center_y().fill(k.c.layer_01).radius(k.radius.lg)
                    .child(ui::user(k, &c.author, 22.0))
                    .child(text(c.author.clone()).font(k.ty.small_medium).color(k.c.text_base))
                    .child(text(c.message.clone()).font(k.ty.small).color(k.c.text_muted).grow(1.0).shrink().max_lines(1))
                    .child(text(if s.compact { c.age.clone() } else { format!("{} · {}", c.sha, c.age) }).font(k.ty.mono).color(k.c.text_faint)));
            }
            if let Some(e) = &d.error {
                files = files.child(col().w_full().padding(16.0).child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap()));
            }
            for (name, dir) in d.files.iter().take(40) {
                files = files.child(rule(k)).child(
                    row().w_full().px(16.0).py(9.0).gap(10.0).center_y().hover(k.c.hover)
                        .child(ui::ico(if *dir { &icons::FOLDER } else { &icons::FILE }, 16.0, if *dir { k.c.info.icon } else { k.c.icon_muted }))
                        .child(text(name.clone()).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink()),
                );
            }
        }
    }
    let readme_body = match detail.and_then(|d| d.readme.as_deref()) {
        Some(src) => rune_kit::Markdown::new(crate::markdown::prepare(src, &r.full_name, &r.branch))
            .on_link_tx(|_: &mut App, url: &str, tx| { let _ = tx.open_url(url); })
            .build(k).w_full(),
        None => text(if detail.is_some_and(|d| d.loading) { "Loading…" } else { "This repository has no README." }).font(k.ty.ui).color(k.c.text_muted),
    };
    let readme_panel = panel(k)
        .child(panel_head(k, "README", Some(ui::ico(&icons::BOOK, 16.0, k.c.icon_muted))))
        .child(rule(k))
        .child(col().w_full().padding(if s.compact { 14.0 } else { 20.0 }).child(readme_body));
    // Language bar from GitHub's byte counts.
    let langs: Vec<(String, u64)> = detail.map(|d| d.languages.clone()).unwrap_or_default();
    let total: u64 = langs.iter().map(|l| l.1).sum::<u64>().max(1);
    let shown: Vec<&(String, u64)> = langs.iter().take(5).collect();
    let bar = row().w_full().h(8.0).radius(4.0).clip_rounded(4.0).fill(k.c.layer_01)
        .children(shown.iter().map(|(l, b)| el().grow(*b as f32).h_full().fill(rune::oklch(0.68, 0.15, lang_hue(l)))));
    let legend = row().gap(12.0).flex_wrap().children(shown.iter().map(|(l, b)| ui::language(k, &format!("{l} {:.0}%", *b as f64 * 100.0 / total as f64), lang_hue(l))));
    let desc = if r.description.is_empty() { "No description".to_owned() } else { r.description.clone() };
    let about = panel(k).padding(16.0).gap(12.0)
        .child(text("About").font(k.ty.ui_medium).color(k.c.text_base))
        .child(text(desc).font(k.ty.small).color(k.c.text_muted).wrap())
        .child(row().gap(6.0).flex_wrap().children(r.topics.iter().map(|t| ui::topic(k, t))))
        .child(col().gap(8.0)
            .child(ui::stat(k, &icons::STAR, format!("{} stars", r.stars)))
            .child(ui::stat(k, &icons::FORK, format!("{} forks", r.forks)))
            .child(ui::stat(k, &icons::ISSUE, format!("{} open issues and pull requests", r.issues)))
            .child(ui::stat(k, &icons::BRANCH, format!("default branch: {}", r.branch))))
        .child(rule(k))
        .child(text("Languages").font(k.ty.ui_medium).color(k.c.text_base))
        .child(if langs.is_empty() { text("—").font(k.ty.small).color(k.c.text_faint) } else { col().w_full().gap(8.0).child(bar).child(legend) });
    row().w_full().gap(16.0).flex_wrap()
        .child(col().grow(3.0).min_w(if s.compact { 260.0 } else { 380.0 }).gap(12.0)
            .child(row().w_full().justify_between().child(branch).child(clone))
            .child(files)
            .child(readme_panel))
        .child(col().grow(1.0).min_w(240.0).child(about))
}
