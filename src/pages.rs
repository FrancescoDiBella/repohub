//! Dashboard, repository list and repository detail.
use rune::{El, col, el, row, text};
use rune_kit::{Badge, BadgeVariant, Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::lists::pull_row;
use crate::model::{PullState, Repo, lang_hue};
use crate::ui::{self, panel, panel_head, rule};
use crate::state::{BranchSlot, Compose, Load, OwnerFilter, RepoFilter, RepoSort, RepoTab};
use crate::{App, Page, Status, actions, icons};

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
    let empty_loading = d.is_empty() && s.status.is_busy();
    let empty_failed = if d.is_empty() {
        if let Status::Failed(e) = &s.status { Some(e.as_str()) } else { None }
    } else {
        None
    };
    let prs_capped = d.prs_capped();
    let issues_capped = d.issues_capped();
    let open_prs = d.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let issues = d.issues.len();
    let metric = |icon, value: String, caption, tint, page: Page| {
        let target = page.clone();
        let a11y = if value == "—" { format!("{caption}, not loaded") } else { format!("{value} {caption}") };
        ui::metric(k, icon, &value, caption, tint).hover(k.c.hover).press(k.c.pressed)
            .role("button").label(a11y).key(format!("metric-{caption}"))
            .on_press(move |s: &mut App| s.go(target.clone()))
    };
    let metric_loading = |icon, caption, tint, page: Page| {
        let target = page.clone();
        ui::metric_skeleton(k, icon, caption, tint).hover(k.c.hover).press(k.c.pressed)
            .role("button").label(format!("Loading {caption}")).key(format!("metric-{caption}"))
            .on_press(move |s: &mut App| s.go(target.clone()))
    };
    let metrics = if empty_loading {
        row().w_full().gap(12.0).flex_wrap()
            .child(metric_loading(&icons::REPO, "Repositories", k.c.info.icon, Page::Repos))
            .child(metric_loading(&icons::STAR, "Stars received", k.c.warning.icon, Page::Repos))
            .child(metric_loading(&icons::PULL, "Open pull requests", k.c.success.icon, Page::Pulls))
            .child(metric_loading(&icons::ISSUE, "Open issues", k.c.danger.icon, Page::Issues))
    } else if empty_failed.is_some() {
        row().w_full().gap(12.0).flex_wrap()
            .child(metric(&icons::REPO, "—".into(), "Repositories", k.c.info.icon, Page::Repos))
            .child(metric(&icons::STAR, "—".into(), "Stars received", k.c.warning.icon, Page::Repos))
            .child(metric(&icons::PULL, "—".into(), "Open pull requests", k.c.success.icon, Page::Pulls))
            .child(metric(&icons::ISSUE, "—".into(), "Open issues", k.c.danger.icon, Page::Issues))
    } else {
        row().w_full().gap(12.0).flex_wrap()
            .child(metric(&icons::REPO, crate::model::capped_if(d.repos.len(), d.repos_capped), "Repositories", k.c.info.icon, Page::Repos))
            .child(metric(&icons::STAR, s.starred_count().to_string(), "Stars received", k.c.warning.icon, Page::Repos))
            .child(metric(&icons::PULL, crate::model::capped_if(open_prs, prs_capped), "Open pull requests", k.c.success.icon, Page::Pulls))
            .child(metric(&icons::ISSUE, crate::model::capped_if(issues, issues_capped), "Open issues", k.c.danger.icon, Page::Issues))
    };
    let first = d.profile.name.split_whitespace().next().unwrap_or("").to_owned();
    let hello = if first.is_empty() { "Welcome back".to_owned() } else { format!("Welcome back, {first}") };
    let head = heading(k, &hello, &match s.freshness() {
        Some(f) => format!("Here's what's happening in your repositories. {f}."),
        None => "Here's what's happening in your repositories today.".into(),
    }, Some(new_repo_button(k)));
    if let Some(err) = empty_failed {
        return col().w_full().gap(16.0)
            .child(head)
            .child(metrics)
            .child(ui::error_panel(k, err, "retry-overview", |s: &mut App| s.refresh()));
    }
    let commits: u32 = d.activity.iter().sum();
    let activity = if empty_loading {
        panel(k).key("skeleton-activity").role("status").label("Loading activity…")
            .child(panel_head(k, "Activity · last 26 weeks", Some(ui::skeleton_faint(k, 72.0, 12.0))))
            .child(col().w_full().px(16.0).pb(16.0).child(ui::sparkline_skeleton(k, 72.0)))
    } else {
        panel(k)
            .child(panel_head(k, "Activity · last 26 weeks", Some(text(format!("{commits} commits")).font(k.ty.small).color(k.c.text_muted))))
            .child(col().w_full().px(16.0).pb(16.0).child(ui::sparkline(k, &d.activity, 72.0)))
    };
    let recent_head = panel(k).child(panel_head(k, "Recent repositories", Some(see_all(k, "See all", Page::Repos))));
    let recent = if empty_loading {
        ui::skeleton_rows_in_panel(k, recent_head.key("skeleton-recent").role("status").label("Loading recent repositories…"), 4, false)
    } else if d.repos.is_empty() {
        recent_head.child(rule(k)).child(col().w_full().padding(24.0).items_center()
            .child(text("No repositories to show").font(k.ty.ui).color(k.c.text_muted)))
    } else {
        d.repos.iter().enumerate().take(4).fold(
            recent_head,
            |p, (i, r)| p.child(rule(k)).child(compact_repo(k, i, r, &d.profile.login, s.now)),
        )
    };
    let review_head = panel(k).child(panel_head(k, "Open pull requests", Some(see_all(k, "All pull requests", Page::Pulls))));
    let open_list: Vec<_> = d.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).take(4).collect();
    let review = if empty_loading {
        ui::skeleton_rows_in_panel(k, review_head.key("skeleton-pulls").role("status").label("Loading open pull requests…"), 4, true)
    } else if open_list.is_empty() {
        review_head.child(rule(k)).child(col().w_full().padding(24.0).items_center()
            .child(text("No open pull requests").font(k.ty.ui).color(k.c.text_muted)))
    } else {
        open_list.into_iter().fold(
            review_head,
            |p, pr| p.child(rule(k)).child(pull_row(k, pr, s.now, true)),
        )
    };
    // What waits for your review, above the open pull requests, when anything does.
    let waiting = (!empty_loading).then(|| crate::triage_page::waiting_panel(s, k)).flatten();
    col().w_full().gap(16.0)
        .child(head)
        .child(metrics)
        .child(activity)
        .child(row().w_full().gap(16.0).flex_wrap()
            .child(col().grow(1.0).min_w(if s.compact { 260.0 } else { 320.0 }).child(recent))
            .child(col().grow(1.0).min_w(if s.compact { 260.0 } else { 320.0 }).gap(16.0).children(waiting).child(review)))
}

fn see_all(k: &Kit, label: &str, page: Page) -> El {
    Button::new(label).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.go(page.clone())).build(k)
}

fn compact_repo(k: &Kit, i: usize, r: &Repo, me: &str, now: i64) -> El {
    let shown = if r.owner.eq_ignore_ascii_case(me) { &r.name } else { &r.full_name };
    row().w_full().px(16.0).py(10.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .role("link").label(format!("Open {shown}"))
        .on_press(move |s: &mut App| s.open(i)).key(format!("recent-{}", r.full_name))
        .child(ui::avatar(k, &r.name, r.hue, 28.0))
        .child(col().grow(1.0).shrink().gap(2.0)
            .child(text(r.name.clone()).font(k.ty.ui_medium).color(k.c.text_base))
            .child(row().gap(12.0)
                .child(if r.owner.eq_ignore_ascii_case(me) { el() } else { text(r.owner.clone()).font(k.ty.text_small).color(k.c.text_muted) })
                .child(ui::language(k, &r.language, r.hue)).child(ui::stat(k, &icons::STAR, r.stars.to_string()))))
        .child(text(r.updated_ago(now)).font(k.ty.text_small).color(k.c.text_faint))
}

// ---------------------------------------------------------------- repositories

/// How many repository cards render initially before **Show more**.
pub const REPOS_PAGE: usize = 60;

pub fn repos(s: &App, k: &Kit) -> El {
    let all = &s.data.repos;
    let shown = s.filtered_repos();
    // The phone has no sidebar, where the pinned repositories live: they go at the top here.
    let pinned = if s.compact { crate::pinned(s) } else { Vec::new() };
    let pinned_row = (!pinned.is_empty()).then(|| {
        let chips = pinned.iter().map(|&i| {
            Button::new(all[i].name.clone()).icon(&icons::PIN).variant(ButtonVariant::Outline).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.open(i)).build(k).key(format!("pinned-chip-{}", all[i].full_name))
        });
        col().w_full().gap(6.0).key("pinned-row")
            .child(text("Pinned").font(k.ty.small_medium).color(k.c.text_muted))
            .child(row().w_full().scroll_x().child(row().gap(6.0).children(chips)))
    });
    let sort = row().gap(8.0).center_y()
        .child(ui::ico(&icons::SORT, 14.0, k.c.icon_muted))
        .child(SegmentedControl::new(&RepoSort::LABELS, Some(s.sort.index())).label("Sort by").fit()
            .on_pick(|s: &mut App, i, _| {
                s.sort = RepoSort::from_index(i);
                s.repos_limit = REPOS_PAGE;
            }).build(k).key("sort"));
    let search = TextInput::new(&s.search).placeholder("Search by name, description or topic…").leading_icon(k.glyph.search)
        .label("Search repositories")
        .on_edit(|s: &mut App| &mut s.search)
        .on_change(|s: &mut App, _| s.repos_limit = REPOS_PAGE)
        .build(k).key("search").grow(1.0).min_w(200.0);
    let seg = SegmentedControl::new(&RepoFilter::LABELS, Some(s.filter.index())).label("Filter").fit()
        .on_pick(|s: &mut App, i, _| {
            s.filter = RepoFilter::from_index(i);
            s.repos_limit = REPOS_PAGE;
        }).build(k).key("filter");
    // Whose: all, the account's own, or one organization (shown when there are any).
    let orgs = crate::orgs(s);
    let mut owners = row().w_full().gap(6.0).center_y().flex_wrap();
    if !orgs.is_empty() {
        let mine = s.mine_repo_count();
        let mut chips: Vec<(String, OwnerFilter, usize)> = vec![("All".into(), OwnerFilter::All, all.len()), ("Mine".into(), OwnerFilter::Mine, mine)];
        chips.extend(orgs.into_iter().map(|(o, n)| (o.clone(), OwnerFilter::Org(o), n)));
        for (label, f, n) in chips {
            let on = s.owner == f;
            let key = format!("owner-{label}");
            owners = owners.child(Button::new(format!("{label} · {n}")).control_size(ControlSize::Small)
                .icon(if matches!(f, OwnerFilter::Org(_)) { &icons::GLOBE } else { &icons::REPO })
                .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
                .on_press(move |s: &mut App| {
                    s.owner = f.clone();
                    s.repos_limit = REPOS_PAGE;
                }).build(k).key(key));
        }
    }
    let visible_len = shown.len().min(s.repos_limit.max(REPOS_PAGE));
    let visible = &shown[..visible_len];
    let remaining = shown.len().saturating_sub(visible_len);
    let dark = s.shown_dark.get();
    let active_look = s.looks.active(dark);
    let look_ver = (dark, active_look.id.as_str(), &active_look.colors);
    let now_bucket = s.now / 30;
    let me = &s.data.profile.login;
    let mut list = col().w_full().gap(12.0);
    for &i in visible {
        let r = &all[i];
        let show_owner = !r.owner.eq_ignore_ascii_case(me);
        let pinned = s.is_pinned(i);
        let now = s.now;
        list = list.child(rune::memo(
            format!("card-{}", r.full_name),
            ((i, r.pushed, r.stars, r.forks, r.issues, now_bucket), (r.starred, r.private, r.fork, r.archived, show_owner, pinned), &look_ver),
            || repo_card(k, i, r, show_owner, pinned, now),
        ));
    }
    if remaining > 0 {
        list = list.child(row().w_full().justify_center().pt(4.0)
            .child(Button::new(format!("Show more ({remaining} left)"))
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.repos_limit = s.repos_limit.max(REPOS_PAGE) + REPOS_PAGE)
                .build(k).key("repos-show-more")));
    }
    if shown.is_empty() {
        if s.data.is_empty() && s.status.is_busy() {
            list = list.key("skeleton-repos").role("status").label("Loading repositories…");
            for n in 0..4 {
                list = list.child(ui::skeleton_repo_card(k, n));
            }
        } else if s.data.is_empty() && matches!(s.status, Status::Failed(_)) {
            if let Status::Failed(e) = &s.status {
                list = list.child(ui::error_panel(k, e, "retry-repos", |s: &mut App| s.refresh()));
            }
        } else {
            let msg = if all.is_empty() { "No repositories to show" } else { "No repositories match your search" };
            list = list.child(panel(k).padding(32.0).items_center().gap(6.0)
                .child(ui::ico(&icons::REPO, 24.0, k.c.icon_faint))
                .child(text(msg).font(k.ty.ui).color(k.c.text_muted)));
        }
    }
    let sub = if s.data.is_empty() && s.status.is_busy() {
        "Loading repositories…".to_owned()
    } else if s.data.is_empty() && matches!(s.status, Status::Failed(_)) {
        "Couldn't load repositories.".to_owned()
    } else if remaining > 0 {
        if s.data.repos_capped && shown.len() == all.len() {
            format!("Showing {} of {}+ repositories", visible.len(), all.len())
        } else {
            format!("Showing {} of {} repositories", visible.len(), shown.len())
        }
    } else if s.data.repos_capped {
        format!("Showing {} of {}+ repositories", shown.len(), all.len())
    } else {
        format!("{} of {} repositories", shown.len(), all.len())
    };
    let action = if !s.data.sample && s.data.repos_capped {
        let me = &s.data.profile.login;
        let url = format!("{}/{}?tab=repositories", s.cfg.web_root(), me);
        Some(row().gap(8.0).center_y().child(open_button(k, "Open on GitHub", &url, "repos-gh")).child(new_repo_button(k)))
    } else {
        Some(new_repo_button(k))
    };
    col().w_full().gap(16.0)
        .child(heading(k, "Repositories", &sub, action))
        .children(pinned_row)
        .child(owners)
        .child(row().w_full().gap(12.0).center_y().flex_wrap().child(search).child(seg).child(sort))
        .child(list)
}

/// Pins or unpins a repository in the sidebar. `labelled`: a text button (the repository's
/// page); otherwise an icon (cards).
fn pin_button(k: &Kit, i: usize, r: &Repo, pinned: bool, labelled: bool) -> El {
    let (icon, help) = if pinned { (&icons::PIN_ON, "Unpin from the sidebar") } else { (&icons::PIN, "Pin to the sidebar") };
    let key = format!("pin-btn-{}", r.full_name);
    if labelled {
        return Button::new(if pinned { "Pinned" } else { "Pin" }).icon(icon).control_size(ControlSize::Small)
            .variant(if pinned { ButtonVariant::Contrast } else { ButtonVariant::Neutral })
            .on_press(move |s: &mut App| s.toggle_pin(i)).build(k).key(key);
    }
    rune_kit::IconButton::new(icon).variant(if pinned { rune_kit::IconButtonVariant::Ghost } else { rune_kit::IconButtonVariant::GhostMuted })
        .label(help).help(help).on_press(move |s: &mut App| s.toggle_pin(i)).build(k).key(key)
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

/// A repository in the list; `show_owner` prefixes the name with its organization.
fn repo_card(k: &Kit, i: usize, r: &Repo, show_owner: bool, pinned: bool, now: i64) -> El {
    let vis = if r.private { Badge::new("Private").variant(BadgeVariant::Subtle).build(k) } else { Badge::new("Public").build(k) };
    let mut title = row().gap(8.0).center_y().flex_wrap();
    if show_owner {
        title = title.child(text(format!("{} /", r.owner)).font(k.ty.body).color(k.c.text_muted));
    }
    let open_label = format!("Open {}", if show_owner { &r.full_name } else { &r.name });
    let mut title = title
        .child(text(r.name.clone()).font(k.ty.body_medium).color(k.c.text_accent)
            .role("link").label(open_label)
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
        .child(ui::stat(k, &icons::ISSUE, r.issues.to_string()).help(format!("{} open issues and pull requests", r.issues)))
        .child(text(format!("Updated {}", r.updated_ago(now))).font(k.ty.text_small).color(k.c.text_faint));
    let desc = if r.description.is_empty() { "No description".to_owned() } else { r.description.clone() };
    panel(k).padding(16.0).gap(10.0)
        .child(row().w_full().justify_between().gap(12.0)
            .child(col().grow(1.0).shrink().gap(6.0)
                .child(title)
                .child(text(desc).font(k.ty.ui).color(k.c.text_muted).wrap()))
            .child(row().gap(4.0).center_y()
                .child(pin_button(k, i, r, pinned, false))
                .child(star_button(k, i, r, false, format!("star-{}", r.full_name)))))
        .child(topics)
        .child(foot)
}

// ---------------------------------------------------------------- repository detail


/// `plugin`: what the open plugin section draws, when one is open (`App::plugin_tab`).
pub fn repo_detail(s: &App, k: &Kit, i: usize, plugin: Option<El>) -> El {
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
        .child(row().gap(8.0).flex_wrap().shrink().min_w(0.0)
            .child(open_button(k, "Open on GitHub", &r.url, "open-github"))
            .child(Button::new(format!("Fork · {}", r.forks)).icon(&icons::FORK).control_size(ControlSize::Small)
                .on_press_tx({ let u = format!("{}/fork", r.url); move |_: &mut App, tx| { let _ = tx.open_url(&u); } }).build(k).key("fork"))
            .child(pin_button(k, i, r, s.is_pinned(i), true))
            .child(star_button(k, i, r, true, "detail-star".into())));
    // The app's sections, then the plugins' ones.
    let shown = s.tabs();
    let extra = s.plugins.repo_tabs();
    let mut labels: Vec<String> = shown.iter().map(|t| RepoTab::LABELS[t.index()].to_owned()).collect();
    labels.extend(extra.iter().map(|(_, l)| l.clone()));
    let at = match &s.plugin_tab {
        Some(p) => extra.iter().position(|(id, _)| id == p).map(|n| shown.len() + n),
        None => shown.iter().position(|t| *t == s.tab),
    };
    let tabs = SegmentedControl::new(&labels, at).label("Section").fit()
        .on_pick(move |s: &mut App, t, _| {
            let shown = s.tabs();
            match shown.get(t) {
                Some(tab) => s.set_tab(i, *tab),
                None => s.open_plugin_tab(i, s.plugins.repo_tabs().get(t - shown.len()).map(|(id, _)| id.clone())),
            }
        }).build(k).key("tabs");
    // Ten sections and the plugins' don't always fit (a phone, a narrow window): the strip
    // scrolls sideways rather than squeezing the labels.
    let tabs = row().w_full().scroll_x().child(tabs);
    let in_repo = |name: &str| name.eq_ignore_ascii_case(&r.full_name) || name.eq_ignore_ascii_case(&r.name);
    let body = if let Some(p) = plugin.filter(|_| s.plugin_tab.is_some()) { p } else { match s.tab {
        RepoTab::Code => code_tab(s, k, i, r),
        RepoTab::Commits => crate::commits_page::commits_tab(s, k, i),
        RepoTab::Changes => crate::changes_page::changes_tab(s, k, i),
        RepoTab::Issues => list_of(k, "Open issues", Some(actions::open_form_button(k, i, Compose::Issue)),
            s.data.issues.iter().filter(|x| in_repo(&x.repo)).map(|x| actions::issue_line(s, k, x)).collect()),
        RepoTab::Pulls => col().w_full().gap(16.0)
            .children(crate::changes_page::pull_panel_for(s, k, Some(&r.full_name)))
            .child(list_of(k, "Pull requests", Some(actions::open_form_button(k, i, Compose::Pull)),
                s.data.pulls.iter().filter(|x| in_repo(&x.repo)).map(|x| actions::pull_line(s, k, x, s.compact)).collect())),
        RepoTab::Actions => list_of(k, "Workflow runs", None, s.data.runs.iter().filter(|x| in_repo(&x.repo)).map(|x| crate::runs_page::run_line(s, k, x, s.compact)).collect()),
        RepoTab::Notes => crate::notes_page::notes_tab(s, k, i),
        RepoTab::Claude => crate::claude_page::claude_tab(s, k, i),
        RepoTab::Terminal => crate::terminal_page::terminal_tab(s, k, i, k.c.page.r + k.c.page.g + k.c.page.b < 1.5),
        RepoTab::Settings => crate::lists::repo_settings(s, k, r),
    } };
    col().w_full().gap(16.0).child(back).child(header).child(tabs).child(actions::form_panel(s, k, i)).child(body)
}

/// The files and what's under them: from the local clone an open file comes first, then the
/// folder (the README when no file is open); from GitHub, its files and the README.
fn local_or_github(s: &App, k: &Kit, i: usize, r: &Repo, has_local: bool, github: El, readme: El) -> Vec<El> {
    if !has_local || s.files_from_github {
        return vec![github, readme];
    }
    let files = crate::local_page::files_panel(s, k, i, r);
    match crate::local_page::viewer(s, k, i, r) {
        Some(file) => vec![file, files],
        None => vec![files, readme],
    }
}

fn list_of(k: &Kit, title: &str, action: Option<El>, rows: Vec<El>) -> El {
    let empty = rows.is_empty();
    let mut p = panel(k).child(panel_head(k, title, action));
    for r in rows {
        p = p.child(rule(k)).child(r);
    }
    if empty {
        p = p.child(rule(k)).child(col().w_full().padding(28.0).items_center()
            .child(text("Nothing to show here").font(k.ty.ui).color(k.c.text_muted)));
    }
    p
}

/// A README as rendered Markdown that can be selected and copied like a web page.
///
/// - Drag, double- or triple-click to select; Cmd/Ctrl+A selects it all, Cmd/Ctrl+C copies, and
///   a secondary click offers Copy.
/// - Copying whole blocks copies their Markdown source (a code block keeps its fences); a part
///   of one block copies the text as shown.
/// - Each code block has its own Copy button.
pub fn readme_view(k: &Kit, md: &str) -> El {
    markdown_view(k, md, "code-copy")
}

/// Markdown as the app shows it; `prefix` keeps the copy buttons of two texts on one page apart.
pub fn markdown_view(k: &Kit, md: &str, prefix: &str) -> El {
    let kit = k.clone();
    let prefix = prefix.to_owned();
    rune_kit::Markdown::new(md.to_owned())
        .selectable(true)
        .copy_source(true)
        .code_frame(move |card, c| {
            let code = c.source.trim_end_matches('\n').to_owned();
            let copy = rune_kit::IconButton::new(&icons::COPY).variant(rune_kit::IconButtonVariant::GhostMuted).label("Copy code")
                .help("Copy code")
                .on_press_tx(move |s: &mut App, tx| { tx.copy(code.clone()); s.notify("Code copied"); })
                .build(&kit).key(format!("{prefix}-{}", c.index));
            col().w_full().child(card).child(copy.pin(None, Some(6.0), Some(6.0), None))
        })
        .on_link_tx(|_: &mut App, url: &str, tx| { let _ = tx.open_url(url); })
        .build(k).w_full()
}

fn code_tab(s: &App, k: &Kit, i: usize, r: &Repo) -> El {
    let at = s.branch(i);
    let load = s.details.get(&(r.full_name.clone(), at.clone()));
    let detail = load.and_then(Load::ready);
    // The branch on screen: pick another one, or start a branch or pull request from it.
    let branch = actions::branch_button(s, k, i, BranchSlot::View, &at, "branch");
    let clone_url = format!("{}.git", r.url);
    let clone = Button::new("Copy clone URL").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .on_press_tx(move |s: &mut App, tx| { tx.copy(clone_url.clone()); s.notify("Clone URL copied"); })
        .build(k).key("clone");
    // With a local clone the files come from the disk (`local_page`), unless GitHub is picked.
    let has_local = s.local.contains_key(&r.full_name);
    let mut files = panel(k);
    if has_local {
        files = files.child(panel_head(k, "Files on GitHub", Some(crate::local_page::source_switch(s, k)))).child(rule(k));
    }
    match load {
        None => files = files.child(col().w_full().padding(24.0).items_center().child(text("Open the repository to load its files").font(k.ty.ui).color(k.c.text_muted))),
        Some(Load::Loading) => files = files.child(ui::skeleton_files(k, true, "skeleton-files")),
        Some(Load::Failed(e)) => files = files.child(ui::error_inline(k, e, "retry-files", move |s: &mut App| s.load_detail(i))),
        Some(Load::Ready(d)) => {
            if let Some(c) = &d.commit {
                let age = c.age_ago(s.now);
                files = files.child(row().w_full().px(16.0).py(10.0).gap(10.0).center_y().fill(k.c.layer_01).radius(k.radius.lg)
                    .child(ui::user(k, &c.author, 22.0))
                    .child(text(c.author.clone()).font(k.ty.small_medium).color(k.c.text_base))
                    .child(text(c.message.clone()).font(k.ty.small).color(k.c.text_muted).grow(1.0).shrink().max_lines(1))
                    .child(text(if s.compact { age } else { format!("{} · {}", c.sha, age) }).font(k.ty.mono).color(k.c.text_faint)));
            }
            if let Some(e) = &d.error {
                if d.commit.is_some() {
                    files = files.child(rule(k));
                }
                files = files.child(ui::error_inline(k, e, "retry-files", move |s: &mut App| s.load_detail(i)));
            }
            for (name, dir) in d.files.iter().take(40) {
                // A file or folder opens on GitHub.
                let url = format!("{}/{}/{}/{}", r.url, if *dir { "tree" } else { "blob" }, at, name);
                files = files.child(rule(k)).child(
                    row().w_full().px(16.0).py(9.0).gap(10.0).center_y().hover(k.c.hover).press(k.c.pressed)
                        .role("link").label(format!("Open {name} on GitHub")).key(format!("file-{name}"))
                        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); })
                        .child(ui::ico(if *dir { &icons::FOLDER } else { &icons::FILE }, 16.0, if *dir { k.c.info.icon } else { k.c.icon_muted }))
                        .child(text(name.clone()).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink())
                        .child(ui::ico(&icons::EXTERNAL, 13.0, k.c.icon_faint)),
                );
            }
            if d.files.is_empty() && d.error.is_none() {
                if d.commit.is_some() {
                    files = files.child(rule(k));
                }
                files = files.child(col().w_full().padding(24.0).items_center()
                    .child(text("No files in this branch").font(k.ty.ui).color(k.c.text_muted)));
            }
            if d.files.len() > 40 {
                let tree_url = format!("{}/tree/{}", r.url, at);
                files = files.child(rule(k)).child(
                    row().w_full().px(16.0).py(9.0).gap(8.0).center_y().justify_between().hover(k.c.hover).press(k.c.pressed)
                        .role("link").label(format!("Showing 40 of {} files. Open all on GitHub", d.files.len())).key("files-more")
                        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&tree_url); })
                        .child(text(format!("Showing 40 of {} files", d.files.len())).font(k.ty.small).color(k.c.text_muted))
                        .child(row().gap(6.0).center_y().child(text("Open on GitHub").font(k.ty.small).color(k.c.text_accent)).child(ui::ico(&icons::EXTERNAL, 13.0, k.c.text_accent))),
                );
            }
        }
    }
    let readme = detail.and_then(|d| d.readme.clone());
    let readme_err = detail.and_then(|d| d.readme_error.as_deref());
    let readme_body = match load {
        Some(Load::Failed(e)) => text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap(),
        Some(Load::Loading) => ui::skeleton_readme(k, "skeleton-readme"),
        None => text("Open the repository to load its README").font(k.ty.ui).color(k.c.text_muted),
        Some(Load::Ready(_)) => match (&readme, readme_err) {
            (Some(src), _) => readme_view(k, &crate::markdown::prepare(src, &r.full_name, &at)),
            (None, Some(err)) => text(err).font(k.ty.small).color(k.c.danger.icon).wrap(),
            (None, None) => text("This repository has no README.").font(k.ty.ui).color(k.c.text_muted),
        },
    };
    // The README panel's head: copy the whole file as Markdown, or read it on GitHub, or retry if failed.
    let readme_tools = match load {
        Some(Load::Failed(_)) => row().gap(4.0).center_y()
            .child(Button::new("Retry").control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.load_detail(i))
                .build(k).key("readme-retry")),
        _ if readme.is_none() && readme_err.is_some() => row().gap(4.0).center_y()
            .child(Button::new("Retry").control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.load_detail(i))
                .build(k).key("readme-retry")),
        _ => match readme {
            Some(src) => row().gap(4.0).center_y()
                .child(Button::new("Copy").icon(&icons::COPY).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press_tx(move |s: &mut App, tx| { tx.copy(src.clone()); s.notify("README copied as Markdown"); })
                    .build(k).key("readme-copy")),
            None => ui::ico(&icons::BOOK, 16.0, k.c.icon_muted),
        },
    };
    let readme_panel = panel(k)
        .child(panel_head(k, "README", Some(readme_tools)))
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
    let lang_el = if matches!(load, Some(Load::Loading)) && langs.is_empty() {
        col().w_full().gap(8.0).key("skeleton-languages")
            .child(ui::skeleton_faint(k, 200.0, 8.0).w_full())
            .child(row().gap(12.0).child(ui::skeleton_faint(k, 64.0, 11.0)).child(ui::skeleton_faint(k, 52.0, 11.0)))
    } else if langs.is_empty() {
        text("—").font(k.ty.small).color(k.c.text_faint)
    } else {
        col().w_full().gap(8.0).child(bar).child(legend)
    };
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
        .child(lang_el);
    row().w_full().gap(16.0).flex_wrap()
        .child(col().grow(3.0).min_w(if s.compact { 260.0 } else { 380.0 }).gap(12.0)
            .child(row().w_full().justify_between().gap(8.0).flex_wrap()
                .child(row().gap(8.0).center_y().flex_wrap()
                    .child(branch)
                    .child(actions::open_form_button(k, i, Compose::Branch))
                    .child(actions::open_form_button(k, i, Compose::Pull)))
                .child(clone))
            .child(actions::branch_panel(s, k, i, BranchSlot::View, &at))
            .children(local_or_github(s, k, i, r, has_local, files, readme_panel)))
        .child(col().grow(1.0).min_w(240.0).child(about))
}
