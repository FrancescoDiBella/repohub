//! The window around the pages: the sidebar, the toolbar, the banners, the toast, and `view`, which
//! draws a frame.

use super::*;

#[allow(dead_code)]
fn kit(dark: bool) -> Kit {
    let c = |h: &str| rune::color::hex(h).expect("valid colour");
    let mut k = if dark {
        Kit::from_palette(PaletteSeed { bg: c("#0d1117"), fg: c("#e6edf3"), accent: c("#2f81f7") })
    } else {
        Kit::from_palette(PaletteSeed { bg: c("#f6f8fa"), fg: c("#1f2328"), accent: c("#0969da") })
    };
    k.radius.lg = 10.0;
    k
}

fn nav(s: &App, k: &Kit, label: &str, icon: &'static rune::Icon, page: Page, badge: Option<String>) -> El {
    let selected = s.page == page || (page == Page::Repos && matches!(s.page, Page::Repo(_)));
    let target = page.clone();
    // The Repositories row shows every owner again.
    let mut r = SidebarRow::new(label).icon(icon).height(32.0).selected(selected).on_select(move |s: &mut App, _| {
        if target == Page::Repos {
            s.owner = OwnerFilter::All;
            s.repos_limit = pages::REPOS_PAGE;
        }
        s.go(target.clone())
    });
    if let Some(b) = badge {
        r = r.badge(b);
    }
    r.build(k).key(format!("nav-{label}"))
}

fn sidebar(s: &App, k: &Kit) -> El {
    let prs_capped = s.data.prs_capped();
    let issues_capped = s.data.issues_capped();
    let open_prs = s.data.pulls.iter().filter(|p| matches!(p.state, PullState::Open | PullState::Draft)).count();
    let brand = row().w_full().h(40.0).px(10.0).gap(10.0).center_y()
        .child(row().size(26.0, 26.0).radius(7.0).center().fill(k.c.text_strong)
            .child(ui::ico(&icons::REPO, 15.0, k.c.page)))
        .child(text(s.cfg.title.clone()).font(k.ty.body_medium).color(k.c.text_strong));
    let pinned: Vec<El> = pinned(s).into_iter().map(|i| {
        let r = &s.data.repos[i];
        let full = r.full_name.clone();
        SidebarRow::new(r.name.clone()).avatar(ui::avatar(k, &r.name, r.hue, 16.0)).height(30.0)
            .selected(matches!(&s.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full))).on_select(move |s: &mut App, _| s.open(i)).build(k).key(format!("pin-{}", r.full_name))
    }).collect();
    let org_rows: Vec<El> = orgs(s).into_iter().map(|(o, n)| {
        let on = s.page == Page::Repos && s.owner == OwnerFilter::Org(o.clone());
        let pick = o.clone();
        SidebarRow::new(o.clone()).avatar(ui::avatar(k, &o, 210.0, 16.0)).height(30.0).badge(n.to_string())
            .selected(on).on_select(move |s: &mut App, _| s.show_owner(OwnerFilter::Org(pick.clone()))).build(k).key(format!("org-{o}"))
    }).collect();
    let mut sb = Sidebar::new()
        .label("Navigation")
        .header(brand)
        .header(nav(s, k, "Overview", &icons::HOME, Page::Overview, None))
        .header(nav(s, k, "Repositories", &icons::REPO, Page::Repos, (!s.data.repos.is_empty()).then(|| s.data.repos.len().to_string())))
        .header(nav(s, k, "Pull requests", &icons::PULL, Page::Pulls, (open_prs > 0 || prs_capped).then(|| model::capped_if(open_prs, prs_capped))))
        .header(nav(s, k, "Issues", &icons::ISSUE, Page::Issues, (!s.data.issues.is_empty() || issues_capped).then(|| model::capped_if(s.data.issues.len(), issues_capped))))
        .header(nav(s, k, "Actions", &icons::PLAY, Page::Actions, None))
        .header(nav(s, k, "Notifications", &icons::BELL, Page::Notifications, Some(s.unread()).filter(|n| *n > 0).map(|n| if n >= inbox::READ_AT_MOST { format!("{n}+") } else { n.to_string() })))
        .section(SidebarSection::new("Pinned").count(pinned.len()).collapsed(!s.pinned_open)
            .on_toggle(|s: &mut App, _| s.pinned_open = !s.pinned_open).rows(pinned));
    if !org_rows.is_empty() {
        sb = sb.section(SidebarSection::new("Organizations").count(org_rows.len()).collapsed(s.orgs_closed)
            .on_toggle(|s: &mut App, _| s.orgs_closed = !s.orgs_closed).rows(org_rows));
    }
    let plugin_rows: Vec<El> = s.plugins.pages().into_iter().map(|(id, label)| {
        let pid = id.clone();
        SidebarRow::new(label.clone()).icon(&icons::SPARK).height(30.0).selected(matches!(&s.page, Page::Plugin(p) if p == &pid))
            .on_select(move |s: &mut App, _| s.go(Page::Plugin(pid.clone()))).build(k).key(format!("plugin-page-{id}"))
    }).collect();
    if !plugin_rows.is_empty() {
        sb = sb.section(SidebarSection::new("Plugins").count(plugin_rows.len()).rows(plugin_rows));
    }
    sb.footer(nav(s, k, "Settings", &icons::GEAR, Page::Settings, None)).build(k)
}

fn title(s: &App) -> String {
    match &s.page {
        Page::Overview => "Overview".into(),
        Page::Repos => "Repositories".into(),
        Page::Repo(full) => full.replace('/', " / "),
        Page::Pulls => "Pull requests".into(),
        Page::Issues => "Issues".into(),
        Page::Actions => "Actions".into(),
        Page::Notifications => "Notifications".into(),
        Page::Settings => "Settings".into(),
        Page::Plugin(id) => s.plugins.list.iter().find(|p| p.id == *id).and_then(|p| p.manifest.page.clone()).unwrap_or_default(),
    }
}

/// A banner when a new version is ready, or while it downloads.
/// A plugin's request to open a link or copy text, with the buttons that answer it.
fn plugin_ask_banner(s: &App, k: &Kit) -> Option<El> {
    use rune_kit::{Button, ButtonVariant};
    let request = s.plugin_ask.as_ref()?;
    let short = |t: &str, n: usize| if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t.to_owned() };
    let (title, detail, yes) = match &request.ask {
        plugins::Ask::Open(url) => (
            format!("The plugin {} wants to open a page on {}", request.plugin, plugins::host(url).unwrap_or("another site")),
            format!("{}\nThe address can carry what the plugin can see here (names of your repositories, issues and pull requests).", short(url, 300)),
            "Open",
        ),
        plugins::Ask::Copy(text) => (
            format!("The plugin {} wants to copy this to the clipboard", request.plugin),
            short(text, 300),
            "Copy",
        ),
    };
    let tint = k.c.warning.icon;
    Some(col().w_full().px(14.0).py(10.0).gap(8.0).radius(k.radius.lg).key("plugin-ask").role("alert")
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(text(title).font(k.ty.small_medium).color(k.c.text_strong).wrap())
        .child(text(detail).font(k.ty.mono).color(k.c.text_base).wrap())
        .child(row().gap(8.0)
            .child(Button::new("Don't").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.answer_plugin(false, tx)).build(k).key("plugin-ask-no"))
            .child(Button::new(yes).control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.answer_plugin(true, tx)).build(k).key("plugin-ask-yes"))))
}

fn update_banner(s: &App, k: &Kit) -> Option<El> {
    use rune_kit::{Button, ButtonVariant};
    let (msg, actions) = match &s.update {
        update::Update::Available(r) if !s.update_later => (
            format!("A new version of RepoHub is ready (built {} from {}).", model::ago(r.published, s.now.max(r.published)), r.short()),
            true,
        ),
        update::Update::Downloading { percent, .. } => (format!("Downloading the new version… {percent}%"), false),
        update::Update::Installing => ("Installing the new version; RepoHub restarts in a moment.".into(), false),
        _ => return None,
    };
    let tint = k.c.success.icon;
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg).key("update-banner")
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(&icons::REFRESH, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    let notes = s.update.available().map(|r| r.notes.clone()).filter(|n| actions && !n.is_empty());
    if notes.is_some() {
        r = r.child(Button::new(if s.show_notes { "Hide what's new" } else { "What's new" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.show_notes = !s.show_notes).build(k).key("update-notes"));
    }
    if actions {
        r = r.child(Button::new("Later").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.update_later = true).build(k).key("update-later"))
            .child(Button::new("Install and restart").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.install_update()).build(k).key("update-install"));
    } else if matches!(s.update, update::Update::Downloading { .. }) {
        r = r.child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.cancel_update()).build(k).key("update-cancel"));
    }
    match notes {
        Some(md) if s.show_notes => Some(col().w_full().gap(0.0).radius(k.radius.lg).fill(tint.alpha(0.06)).border(1.0, tint.alpha(0.45)).key("update-banner-box")
            .child(r.radius(0.0).fill(Color::TRANSPARENT).border(0.0, Color::TRANSPARENT))
            .child(col().w_full().px(16.0).pb(12.0).key("update-notes-body").child(pages::readme_view(k, &md)))),
        _ => Some(r),
    }
}

/// Whether the current page already shows `Status::Failed` in place of its empty list.
fn list_shows_failed(s: &App) -> bool {
    s.data.is_empty() && match &s.page {
        Page::Overview | Page::Repos | Page::Repo(_) | Page::Pulls | Page::Issues | Page::Actions => true,
        Page::Plugin(id) => !s.plugins.list.iter().any(|p| p.id == *id && p.running()),
        Page::Settings | Page::Notifications => false,
    }
}

/// A banner above the page for loading, errors and sample data.
pub(crate) fn banner(s: &App, k: &Kit) -> Option<El> {
    let (msg, tint, action) = match &s.status {
        // Over lists already on screen a load says "Updating…" where "Updated … ago" was.
        Status::Starting | Status::Loading if !s.data.is_empty() => return None,
        Status::Starting | Status::Loading => ("Loading data from GitHub…".to_owned(), k.c.info.icon, false),
        Status::Failed(_) if list_shows_failed(s) => return None,
        Status::Failed(e) if s.fresh.from_copy => (format!("{e}{} {}", if e.ends_with(['.', '!', '?']) { "" } else { "." }, cache_page::shown_copy(s)), k.c.danger.icon, true),
        Status::Failed(e) => (e.clone(), k.c.danger.icon, true),
        Status::Ready if !s.data.warnings.is_empty() => (s.data.warnings.join(" · "), k.c.warning.icon, true),
        Status::Sample if !s.cfg.demo && !s.cfg.account_locked => ("This is sample data. Connect your GitHub account to see your repositories.".to_owned(), k.c.warning.icon, false),
        Status::Sample if !s.cfg.demo => ("Sample data. To see your repositories, create a .env file with GITHUB_USER or GITHUB_TOKEN (see .env.example) and run RepoHub with Docker or as the desktop app.".to_owned(), k.c.warning.icon, false),
        _ => return None,
    };
    let connect = s.status == Status::Sample && !s.cfg.account_locked && !s.cfg.demo && s.page != Page::Settings;
    let icon = if s.status == Status::Ready && !s.data.warnings.is_empty() {
        &icons::BELL
    } else if action {
        &icons::CROSS
    } else {
        &icons::BELL
    };
    let mut r = row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg)
        .fill(tint.alpha(0.12)).border(1.0, tint.alpha(0.45))
        .child(ui::ico(icon, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink());
    if action {
        r = r.child(rune_kit::Button::new("Retry").control_size(ControlSize::Small).on_press(|s: &mut App| s.refresh()).build(k).key("retry"));
    }
    if connect {
        r = r.child(rune_kit::Button::new("Connect").variant(rune_kit::ButtonVariant::Submit).control_size(ControlSize::Small)
            .on_press(|s: &mut App| { s.settings_section = state::SettingsSection::Account; s.go(Page::Settings); }).build(k).key("connect"));
    }
    Some(r.key("banner"))
}

pub(crate) fn view(s: &App, cx: &mut Cx<App>) -> El {
    let system_dark = cx.system_dark();
    let dark = s.theme.is_dark(system_dark);
    let look = s.looks.active(dark);
    let k = themes::kit(look);
    s.shown_dark.set(dark);
    s.plugins.set_colors(&k);
    let content = if let Some(t) = &s.thread {
        thread_page::view(s, &k, t)
    } else if let Some(o) = &s.open_run {
        runs_page::view(s, &k, o)
    } else {
        match &s.page {
            Page::Overview => pages::overview(s, &k),
            Page::Repos => pages::repos(s, &k),
            Page::Repo(full) => {
                if let Some(i) = s.repo_index_of(full) {
                    let extra = s.plugin_tab.as_deref().and_then(|p| s.plugins.surface(p, cx, "repo"));
                    pages::repo_detail(s, &k, i, extra)
                } else {
                    pages::repos(s, &k)
                }
            }
            Page::Plugin(id) => s.plugins.surface(id, cx, "page").unwrap_or_else(|| pages::overview(s, &k)),
            Page::Pulls => lists::pulls(s, &k),
            Page::Issues => lists::issues(s, &k),
            Page::Actions => lists::actions(s, &k),
            Page::Notifications => inbox::page(s, &k),
            Page::Settings => lists::settings(s, &k),
        }
    };
    let mut inner = col().w_full().max_w(1080.0).gap(16.0);
    if let Some(b) = update_banner(s, &k) {
        inner = inner.child(b);
    }
    if let Some(b) = plugin_ask_banner(s, &k) {
        inner = inner.child(b);
    }
    if let Some(b) = banner(s, &k) {
        inner = inner.child(b);
    }
    let pad = if s.compact { 12.0 } else { 24.0 };
    let page = col().w_full().padding(pad).gap(16.0).items_center().child(inner.child(content));
    let picture = match (&s.images, look.backdrop) {
        (Some(cache), themes::Backdrop::Image) if !look.image.trim().is_empty() => {
            let src = look.image.trim();
            let img = if src.starts_with("http://") || src.starts_with("https://") { cache.url(src, 0.0, cx.env().scale) } else { cache.file(src, 0.0, cx.env().scale) };
            Some(img.pixels().clone())
        }
        _ => None,
    };
    let backdrop = themes::backdrop(look, s.backdrop_size, picture.as_ref()).key("backdrop")
        .on_layout_tx(|s: &mut App, [_, _, w, h], _| {
            let size = [w, h];
            if (size[0] / 64.0).ceil() != (s.backdrop_size[0] / 64.0).ceil() || (size[1] / 64.0).ceil() != (s.backdrop_size[1] / 64.0).ceil() {
                s.backdrop_size = size;
            }
        });
    let mut main = col().w_full().h_full().shrink().fill(k.c.page).clip_rounded(0.0)
        .child(backdrop)
        // The scroll bar shows where you are on a long page (as the system's "Show scroll bars" says).
        .child(rune_kit::ScrollBar::scroll_bar(col().key("page").w_full().h_full().shrink().scroll(ScrollSpec { stick_bottom: false }).child(page), &k));
    // On a phone the question a page asks (`confirm`) is a sheet over the page; it's left in
    // `sheet` while the page is drawn.
    let sheet = s.sheet.borrow_mut().take();
    // "Discard what you typed?" floats over the page, at the bottom, until it's answered (in
    // place of the toast); on a phone it's the sheet.
    if let Some(ask) = discard::question(s, &k) {
        if s.compact {
            main = main.child(confirm::sheet(&k, ask, std::rc::Rc::new(|s: &mut App| s.answer_discard(false))));
        } else {
            main = main.child(row().pin(Some(16.0), None, Some(16.0), Some(20.0)).center().child(ask.max_w(560.0)));
        }
    } else if let Some(sheet) = sheet {
        main = main.child(sheet);
    } else if let Some(toast) = &s.toast {
    // The toast floats over the page, at the bottom; it leaves by itself or with its button.
        let (icon, tint) = match toast.kind {
            ToastKind::Done => (&icons::CHECK, k.c.success.icon),
            ToastKind::Problem => (&icons::CROSS, k.c.danger.icon),
        };
        let close = IconButton::new(&icons::CROSS).variant(IconButtonVariant::GhostMuted).label("Dismiss")
            .on_press(|s: &mut App| s.toast = None).build(&k).key("toast-close");
        main = main.child(row().pin(Some(16.0), None, Some(16.0), Some(20.0)).center()
            .child(row().key("toast").gap(10.0).pl(14.0).pr(6.0).py(6.0).center_y().max_w(560.0).radius(k.radius.lg)
                .fill(k.c.bg_base).border(1.0, if toast.kind == ToastKind::Problem { tint.alpha(0.6) } else { k.c.border_base })
                .role(if toast.kind == ToastKind::Problem { "alert" } else { "status" })
                .child(ui::ico(icon, 14.0, tint))
                .child(text(toast.text.clone()).font(k.ty.small).color(k.c.text_base).wrap().shrink())
                .children(toast.undo.is_some().then(|| rune_kit::Button::new("Undo").variant(rune_kit::ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.undo()).build(&k).key("toast-undo")))
                .child(close)));
    }
    let (theme_icon, theme_help) = if dark { (&icons::SUN, "Switch to the light theme") } else { (&icons::MOON, "Switch to the dark theme") };
    let theme = IconButton::new(theme_icon).variant(IconButtonVariant::GhostMuted).label(theme_help)
        .help(theme_help).on_press(move |s: &mut App| s.set_theme(s.theme.toggled(system_dark))).build(&k).key("theme");
    let mut trailing = row().gap(4.0).center_y();
    if s.cfg.live() {
        trailing = trailing.child(IconButton::new(&icons::REFRESH).variant(IconButtonVariant::GhostMuted).label("Refresh")
            .help(match s.freshness() { Some(f) => format!("Refresh from GitHub · {}", f.to_lowercase()), None => "Refresh from GitHub".into() })
            .disabled(s.status.is_busy()).on_press(|s: &mut App| s.refresh()).build(&k).key("refresh"));
    }
    let search = IconButton::new(k.glyph.search).variant(IconButtonVariant::GhostMuted).label("Search and commands")
        .help(format!("Search and commands ({}+K)", palette::MOD)).on_press(|s: &mut App| s.palette = Some(palette::Palette::new()))
        .build(&k).key("open-palette");
    let trailing = row().gap(4.0).center_y().child(search).child(trailing);
    let who = if s.data.profile.login.is_empty() { "?".to_owned() } else { s.data.profile.login.clone() };
    let profile_url = format!("https://github.com/{who}");
    let profile = ui::user(&k, &who, 26.0).ml(6.0).role("button").label("Your profile on GitHub").key("profile")
        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&profile_url); });
    // On a phone an open board takes the whole screen.
    let phone_board = match (&s.page, &s.board) {
        (Page::Repo(full), Some(b)) if s.compact && s.editing_board() && b.repo == *full => s.repo_index_of(full).map(|i| (i, b)),
        _ => None,
    };
    let root = if let Some((i, b)) = phone_board {
        notes_page::phone_editor(&k, i, b, cx.safe_area())
    } else if s.compact {
        let settings = IconButton::new(&icons::GEAR).variant(IconButtonVariant::GhostMuted).label("Settings")
            .expanded(s.page == Page::Settings).on_press(|s: &mut App| s.go(Page::Settings)).build(&k).key("open-settings");
        let name = match &s.page {
            Page::Repo(full) => s.repo_index_of(full).and_then(|i| s.data.repos.get(i)).map(|r| r.name.clone()).unwrap_or_else(|| full.clone()),
            _ => title(s),
        };
        // The phone has no sidebar: the bell is the way to the notifications.
        let unread = s.unread();
        let bell = IconButton::new(&icons::BELL).variant(if unread > 0 { IconButtonVariant::Contrast } else { IconButtonVariant::GhostMuted })
            .label(if unread > 0 { format!("Notifications, {}", inbox::unread_words(unread)) } else { "Notifications".to_owned() })
            .expanded(s.page == Page::Notifications).on_press(|s: &mut App| s.go(Page::Notifications)).build(&k).key("open-notifications");
        mobile::shell(s, &k, cx.safe_area(), name, trailing.child(bell).child(theme).child(settings), main.grow(1.0).min_h(0.0))
    } else {
        let toolbar = Toolbar::new().title(title(s)).trailing(trailing
            .child(theme)
            .child(profile));
        SplitView::new(main)
            .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar(s, &k))
            .toolbar(toolbar)
            .titlebar_inset(cx.titlebar_inset())
            .sidebar_toggle(SidebarToggle::new().size(ControlSize::Large))
            .reduced_motion(cx.is_reduced_motion())
            .build(&k)
    };
    let overlays = palette::layer(s, &k);
    let root = root.on_paste(|s: &mut App, text: &str, _| {
            s.paste(text);
        })
        // The window's width picks the layout: phone-sized windows get the one in `mobile`.
        .on_layout_tx(|s: &mut App, [_, _, w, _], _| {
            let compact = w < mobile::BREAKPOINT;
            if s.compact != compact {
                s.compact = compact;
            }
            // A phone shows diffs unified (see `changes_page`), whatever was picked when wide.
            if compact {
                for d in [&mut s.diff, &mut s.pull_diff, &mut s.commits.commit_diff, &mut s.commits.compare_diff] {
                    if d.mode() != rune_kit::diff::DiffMode::Unified {
                        d.set_mode(rune_kit::diff::DiffMode::Unified);
                    }
                }
            }
        });
    // Overlays (the command palette) go above the whole window.
    rune_kit::root(root, overlays)
}
