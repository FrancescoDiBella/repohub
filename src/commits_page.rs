//! The Commits section of a repository's page: the history of the branch on screen, one commit
//! and its changes in place of the list, and two branches compared (see `commits`).
use rune::{El, col, el, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, SegmentedControl, TextInput};

use crate::commits::{self, Compare, Comparison, Detail, Open, Person, Row};
use crate::github::PullFile;
use crate::model::{self, same_repo};
use crate::state::{BranchSlot, Load, RepoTab};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, actions, icons, pages};

/// The diff's height: it scrolls inside, so a long diff never makes the page long.
const DIFF_H: f32 = 620.0;

/// On a phone the diff leaves room to see what's around it and to scroll the page.
fn diff_h(s: &App) -> f32 {
    if s.compact { (s.backdrop_size[1] - 260.0).clamp(260.0, DIFF_H) } else { DIFF_H }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

/// The section's two diffs: the open commit's and the comparison's.
#[derive(Clone, Copy)]
enum Which {
    Commit,
    Compare,
}

/// The Commits section of repository `i`.
pub fn commits_tab(s: &App, k: &Kit, i: usize) -> El {
    let r = &s.data.repos[i];
    if let Some(o) = s.commits.open.as_ref().filter(|o| same_repo(&o.repo, &r.full_name)) {
        return commit_view(s, k, i, o);
    }
    let at = s.branch(i);
    let comparing = s.commits.compare.as_ref().filter(|c| same_repo(&c.repo, &r.full_name));
    let compare = Button::new("Compare").icon(&icons::COMPARE).control_size(ControlSize::Small)
        .help("Compare two branches: the commits one has that the other doesn't, and what they change")
        .on_press(move |s: &mut App| s.open_compare(i))
        .build(k).key("commits-compare");
    let mut page = col().w_full().gap(16.0).key("commits");
    // While comparing, the form takes the header's place (its × closes it).
    if comparing.is_none() {
        page = page.child(row().w_full().gap(8.0).center_y().flex_wrap()
            .child(actions::branch_button(s, k, i, BranchSlot::View, &at, "commits-branch"))
            .child(TextInput::new(&s.commits.filter).placeholder("Filter by message, author or commit id").label("Filter commits")
                .leading_icon(k.glyph.search).on_edit(|s: &mut App| &mut s.commits.filter).build(k).key("commits-filter").grow(1.0).min_w(200.0))
            .child(compare));
        if s.picker == Some(BranchSlot::View) {
            page = page.child(actions::branch_panel(s, k, i, BranchSlot::View, &at));
        }
    }
    if let Some(note) = clone_note(s, k, i) {
        page = page.child(note);
    }
    match comparing {
        Some(c) => page.child(compare_view(s, k, i, c)),
        None => page.child(history(s, k, i, &at)),
    }
}

/// What the Changes tab knows about the local clone, when it has commits to push or to pull.
/// Nothing new is asked of git for it.
fn clone_note(s: &App, k: &Kit, i: usize) -> Option<El> {
    let r = s.data.repos.get(i)?;
    let c = s.changes.as_ref()?;
    let ours = matches!(&s.diff_of, Some(crate::diffs::DiffOf::Local(f) | crate::diffs::DiffOf::Merge(f, _)) if *f == r.full_name);
    if !ours || (c.ahead == 0 && c.behind == 0) {
        return None;
    }
    let branch = if c.branch.is_empty() { "a detached HEAD".to_owned() } else { c.branch.clone() };
    let upstream = c.upstream.clone().unwrap_or_else(|| "its upstream".into());
    let (ahead, behind) = (plural(c.ahead as usize, "commit"), plural(c.behind as usize, "commit"));
    let msg = match (c.ahead > 0, c.behind > 0) {
        (true, true) => format!("Your local clone, on {branch}, has {ahead} not pushed yet and is {behind} behind {upstream}."),
        (true, false) => format!("Your local clone, on {branch}, has {ahead} not pushed yet."),
        _ => format!("Your local clone, on {branch}, is {behind} behind {upstream}."),
    };
    let tint = k.c.info.icon;
    Some(row().w_full().px(14.0).py(10.0).gap(10.0).center_y().radius(k.radius.lg).fill(tint.alpha(0.10)).border(1.0, tint.alpha(0.4))
        .role("status").key("commits-clone")
        .child(ui::ico(&icons::BRANCH, 15.0, tint))
        .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink())
        .child(Button::new("Changes").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.set_tab(i, RepoTab::Changes)).build(k).key("commits-changes")))
}

/// "Verified": GitHub checked the commit's signature. The word says it; the colour repeats it.
fn verified(k: &Kit) -> El {
    ui::pill(k, "Verified", k.c.success.icon)
}

fn copy_button(k: &Kit, sha: &str, key: String) -> El {
    let sha = sha.to_owned();
    IconButton::new(&icons::COPY).variant(IconButtonVariant::GhostMuted).label("Copy the commit id").help("Copy the commit id")
        .on_press_tx(move |s: &mut App, tx| {
            tx.copy(sha.clone());
            s.notify(format!("Copied the id of commit {}", commits::short(&sha)));
        })
        .build(k).key(key)
}

fn github_button(k: &Kit, url: String, key: String) -> El {
    IconButton::new(&icons::EXTERNAL).variant(IconButtonVariant::GhostMuted).label("Open the commit on GitHub").help("Open on GitHub")
        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); })
        .build(k).key(key)
}

/// One commit in a list: a button that opens it, with Copy id and Open on GitHub beside it.
/// `prefix` keeps the history's rows and the comparison's apart.
fn commit_row(s: &App, k: &Kit, repo: &str, repo_url: &str, c: &Row, prefix: &str) -> El {
    let key = format!("{prefix}-{}", c.sha);
    let mut meta = row().gap(8.0).center_y().flex_wrap()
        .child(text(c.author.clone()).font(k.ty.text_small).color(k.c.text_muted))
        .child(text(c.short().to_owned()).font(k.ty.mono).color(k.c.text_faint))
        .child(text(commits::ago(c.at, s.now, &commits::local_clock)).font(k.ty.text_small).color(k.c.text_faint));
    if c.verified {
        meta = meta.child(verified(k));
    }
    let (repo_open, sha, from) = (repo.to_owned(), c.sha.clone(), key.clone());
    let open = row().grow(1.0).shrink().min_w(0.0).px(16.0).py(10.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .role("button").label(format!("Commit {} by {}: {}", c.short(), c.author, c.title)).key(key.clone())
        .on_press_tx(move |s: &mut App, tx| {
            s.open_commit(&repo_open, &sha, Some(from.clone()));
            tx.scroll_to_y("page", 0.0);
        })
        .child(ui::user(k, &c.author, 24.0))
        .child(col().grow(1.0).shrink().min_w(0.0).gap(3.0)
            .child(text(c.title.clone()).font(k.ty.small_medium).color(k.c.text_base).max_lines(if s.compact { 2 } else { 1 }))
            .child(meta));
    let tools = row().gap(2.0).center_y().pr(10.0)
        .child(copy_button(k, &c.sha, format!("{prefix}-copy-{}", c.sha)))
        .child(github_button(k, format!("{repo_url}/commit/{}", c.sha), format!("{prefix}-github-{}", c.sha)));
    row().w_full().center_y().child(open).child(tools)
}

fn empty(k: &Kit, msg: String, key: &str) -> El {
    col().w_full().padding(24.0).items_center().key(key.to_owned())
        .child(text(msg).font(k.ty.ui).color(k.c.text_muted).wrap())
}

/// The history of `at`, grouped by day, with Load more at the end.
fn history(s: &App, k: &Kit, i: usize, at: &str) -> El {
    let r = &s.data.repos[i];
    let h = s.commits.history_of(&r.full_name, at);
    let title = format!("History of {at}");
    let (h, rows) = match h.map(|h| (h, &h.load)) {
        None | Some((_, Load::Loading)) => {
            let head = panel(k).key("skeleton-commits").role("status").label("Loading commits…").child(panel_head(k, &title, None));
            return ui::skeleton_rows_in_panel(k, head, 6, false);
        }
        Some((_, Load::Failed(e))) => {
            return panel(k).key("commits-failed").child(panel_head(k, &title, None)).child(rule(k))
                .child(ui::error_inline(k, e, "retry-commits", move |s: &mut App| s.load_history(i, true)));
        }
        Some((h, Load::Ready(rows))) => (h, rows),
    };
    let q = s.commits.filter.text.trim().to_owned();
    let shown: Vec<&Row> = rows.iter().filter(|c| commits::matches(c, &q)).collect();
    let count = if q.is_empty() { plural(rows.len(), "commit") } else { format!("{} of {}", shown.len(), plural(rows.len(), "commit")) };
    let mut p = panel(k).key("commits-history")
        .child(panel_head(k, &title, Some(text(count).font(k.ty.small).color(k.c.text_muted).key("commits-count"))));
    if rows.is_empty() {
        p = p.child(rule(k)).child(empty(k, format!("{at} has no commits yet."), "commits-empty"));
    } else if shown.is_empty() {
        let more = if h.more { " Load more to look further back." } else { "" };
        p = p.child(rule(k)).child(empty(k, format!("No commit loaded matches “{q}”.{more}"), "commits-no-match"));
    }
    for (day, list) in commits::days(&shown, s.now, commits::local_clock) {
        p = p.child(rule(k)).child(row().w_full().px(16.0).py(7.0).fill(k.c.layer_01)
            .child(text(day.clone()).font(k.ty.text_small_medium).color(k.c.text_muted).role("heading").level(3).key(format!("commits-day-{day}"))));
        for c in list {
            p = p.child(rule(k)).child(commit_row(s, k, &r.full_name, &r.url, c, "commit"));
        }
    }
    // Where the list ends: more on GitHub, or the first commit.
    let foot = if h.more {
        row().w_full().px(16.0).py(10.0).gap(12.0).center_y().justify_between().flex_wrap().key("commits-more")
            .child(text(format!("GitHub has more commits on {at}.")).font(k.ty.small).color(k.c.text_muted))
            .child(Button::new(if h.loading_more { "Loading…" } else { "Load more" }).control_size(ControlSize::Small).disabled(h.loading_more)
                .on_press(|s: &mut App| s.load_more_commits()).build(k).key("commits-load-more"))
    } else if !rows.is_empty() {
        row().w_full().px(16.0).py(10.0).key("commits-end")
            .child(text(format!("That's all of {at}'s history.")).font(k.ty.small).color(k.c.text_faint))
    } else {
        el()
    };
    if h.more || !rows.is_empty() {
        p = p.child(rule(k)).child(foot);
    }
    p
}

/// Unified or split, and the files' totals.
fn diff_tools(s: &App, k: &Kit, which: Which, files: usize, more: bool, totals: (u32, u32), key: &str) -> El {
    let state = match which {
        Which::Commit => &s.commits.commit_diff,
        Which::Compare => &s.commits.compare_diff,
    };
    let mut r = row().gap(10.0).center_y()
        .child(text(if more { format!("{files}+ files") } else { plural(files, "file") }).font(k.ty.small).color(k.c.text_muted))
        .child(text(format!("+{}", totals.0)).font(k.ty.small_medium).color(k.c.success.icon))
        .child(text(format!("−{}", totals.1)).font(k.ty.small_medium).color(k.c.danger.icon));
    // Side by side needs width a phone doesn't have: there it's always unified.
    if !s.compact && files > 0 {
        r = r.child(SegmentedControl::new(&["Unified", "Split"], Some(usize::from(state.mode() == DiffMode::Split))).label("Diff layout").fit()
            .on_pick(move |s: &mut App, n, _| {
                let d = match which {
                    Which::Commit => &mut s.commits.commit_diff,
                    Which::Compare => &mut s.commits.compare_diff,
                };
                d.set_mode(if n == 1 { DiffMode::Split } else { DiffMode::Unified });
            }).build(k).key(format!("{key}-mode")));
    }
    r
}

/// A note in a panel, with an icon.
fn note(k: &Kit, msg: String, key: String) -> El {
    row().w_full().px(16.0).py(9.0).gap(8.0).center_y().key(key)
        .child(ui::ico(&icons::FILE, 14.0, k.c.icon_muted))
        .child(text(msg).font(k.ty.small).color(k.c.text_muted).wrap().grow(1.0).shrink())
}

/// The files of a commit or a comparison as a diff, with what GitHub left out.
#[allow(clippy::too_many_arguments)]
fn diff_panel(s: &App, k: &Kit, which: Which, files: &[PullFile], more: bool, totals: (u32, u32), url: &str, key: &str, nothing: &str) -> El {
    let mut p = panel(k).key(key.to_owned()).clip_rounded(k.radius.lg)
        .child(row().w_full().px(16.0).py(8.0).gap(10.0).center_y().justify_between().flex_wrap()
            .child(text("Changes").font(k.ty.ui_medium).color(k.c.text_base))
            .child(diff_tools(s, k, which, files.len(), more, totals, key)));
    let missing: Vec<&str> = files.iter().filter(|f| f.patch.is_none()).map(|f| f.path.as_str()).collect();
    if !missing.is_empty() {
        let mut names = missing.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        if missing.len() > 3 {
            names.push_str(&format!(" and {} more", missing.len() - 3));
        }
        p = p.child(rule(k)).child(note(k, format!("GitHub shows no diff for {} (binary or too large): {names}.", plural(missing.len(), "file")), format!("{key}-no-patch")));
    }
    if more {
        let mut line = note(k, format!("GitHub lists only the first {} files here; the rest are on GitHub.", commits::MAX_FILES), format!("{key}-more"));
        if !url.is_empty() {
            line = line.child(pages::open_button(k, "Open on GitHub", url, &format!("{key}-more-github")));
        }
        p = p.child(rule(k)).child(line);
    }
    p = p.child(rule(k));
    if files.is_empty() {
        return p.child(empty(k, nothing.to_owned(), &format!("{key}-empty")));
    }
    let view = match which {
        Which::Commit => DiffView::new(|s: &App| &s.commits.commit_diff, |s: &mut App| &mut s.commits.commit_diff).key("commit-diff-view").build(s, k),
        Which::Compare => DiffView::new(|s: &App| &s.commits.compare_diff, |s: &mut App| &mut s.commits.compare_diff).key("compare-diff-view").build(s, k),
    };
    p.child(col().w_full().h(diff_h(s)).clip_rounded(0.0).child(view))
}

/// Who wrote or committed it, and when.
fn person(s: &App, k: &Kit, what: &str, p: &Person, key: &str) -> El {
    let who = match &p.login {
        Some(l) if !p.name.is_empty() && !p.name.eq_ignore_ascii_case(l) => format!("{what} by {l} ({})", p.name),
        _ => format!("{what} by {}", p.shown()),
    };
    let when = if p.at > 0 { format!("{} · {}", model::fmt_ago(p.at, s.now), commits::date_time(p.at)) } else { String::new() };
    row().w_full().px(16.0).py(10.0).gap(10.0).center_y().key(key.to_owned())
        .child(ui::user(k, p.shown(), 24.0))
        .child(col().grow(1.0).shrink().min_w(0.0).gap(2.0)
            .child(text(who).font(k.ty.small_medium).color(k.c.text_base).wrap())
            .child(text(when).font(k.ty.text_small).color(k.c.text_faint).wrap()))
}

/// One commit, in place of the list.
fn commit_view(s: &App, k: &Kit, i: usize, o: &Open) -> El {
    let r = &s.data.repos[i];
    let back = Button::new("Back").icon(&icons::BACK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .help(if o.trail.is_empty() { "Back to the list" } else { "Back to the commit you came from" })
        .on_press_tx(|s: &mut App, tx| s.close_commit(Some(tx))).build(k).key("commit-back");
    let mut top = row().w_full().gap(8.0).center_y().flex_wrap().child(back).child(el().grow(1.0));
    let page = col().w_full().gap(16.0).key("commit");
    let d: &Detail = match &o.load {
        Load::Loading => {
            return page.child(top)
                .child(text(format!("{} · commit {}", r.full_name, commits::short(&o.sha))).font(k.ty.small).color(k.c.text_muted))
                .child(panel(k).padding(28.0).items_center().role("status").key("commit-loading")
                    .child(text("Reading the commit…").font(k.ty.ui).color(k.c.text_muted)))
                .child(panel(k).clip_rounded(k.radius.lg).child(ui::skeleton_diff(k, "skeleton-commit-diff")));
        }
        Load::Failed(e) => {
            return page.child(top)
                .child(panel(k).padding(28.0).items_center().gap(10.0).role("alert").key("commit-failed")
                    .child(text(e.clone()).font(k.ty.small).color(k.c.text_base).wrap())
                    .child(Button::new("Retry").icon(&icons::REFRESH).control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.load_commit()).build(k).key("commit-retry")));
        }
        Load::Ready(d) => d,
    };
    let url = if d.url.is_empty() { format!("{}/commit/{}", r.url, d.sha) } else { d.url.clone() };
    let sha = d.sha.clone();
    top = top
        .child(Button::new("Copy id").icon(&icons::COPY).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press_tx(move |s: &mut App, tx| {
                tx.copy(sha.clone());
                s.notify(format!("Copied the id of commit {}", commits::short(&sha)));
            }).build(k).key("commit-copy"))
        .child(pages::open_button(k, "Open on GitHub", &url, "commit-github"));
    let mut meta = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(ui::ico(&icons::COMMIT, 15.0, k.c.icon_muted))
        .child(text(format!("{} · commit {}", r.full_name, commits::short(&d.sha))).font(k.ty.small).color(k.c.text_muted));
    if d.verified {
        meta = meta.child(verified(k));
    }
    let mut head = col().w_full().gap(8.0)
        .child(text(d.title.clone()).font(ui::H2).color(k.c.text_strong).wrap().selectable(true).key("commit-title"))
        .child(meta);
    if !d.body.is_empty() {
        // The rest of the message as it was written: plain text, line breaks kept.
        head = head.child(panel(k).padding(16.0)
            .child(text(d.body.clone()).font(k.ty.small).color(k.c.text_base).wrap().selectable(true).key("commit-body")));
    }
    // Who and when, the parents and the full id.
    let mut people = panel(k).key("commit-people").child(person(s, k, "Authored", &d.author, "commit-author"));
    let same_committer = d.committer.shown() == d.author.shown() && (d.committer.at - d.author.at).abs() < 60;
    if !same_committer {
        people = people.child(rule(k)).child(person(s, k, "Committed", &d.committer, "commit-committer"));
    }
    let mut parents = row().w_full().px(16.0).py(10.0).gap(8.0).center_y().flex_wrap().key("commit-parents")
        .child(text(if d.parents.len() == 1 { "Parent" } else { "Parents" }).font(k.ty.small).color(k.c.text_muted));
    if d.parents.is_empty() {
        parents = parents.child(text("none: this is the first commit").font(k.ty.small).color(k.c.text_faint));
    }
    for p in &d.parents {
        let sha = p.clone();
        parents = parents.child(Button::new(commits::short(p).to_owned()).icon(&icons::COMMIT).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .help("Open this commit").on_press_tx(move |s: &mut App, tx| {
                s.open_parent(&sha);
                tx.scroll_to_y("page", 0.0);
            }).build(k).key(format!("commit-parent-{}", commits::short(p))));
    }
    people = people.child(rule(k)).child(parents).child(rule(k))
        .child(row().w_full().px(16.0).py(10.0).gap(8.0).center_y().flex_wrap()
            .child(text("Commit id").font(k.ty.small).color(k.c.text_muted))
            .child(text(d.sha.clone()).font(k.ty.mono).color(k.c.text_base).selectable(true).wrap().key("commit-sha")));
    let diff = diff_panel(s, k, Which::Commit, &d.files, d.more_files, (d.additions, d.deletions), &url, "commit-diff", "This commit changes no files.");
    page.child(top).child(head).child(people).child(diff)
}

/// The comparison form and, once Compare is pressed, its result.
fn compare_view(s: &App, k: &Kit, i: usize, c: &Compare) -> El {
    let loading = matches!(c.result, Some(Load::Loading));
    let close = IconButton::new(&icons::CROSS).variant(IconButtonVariant::GhostMuted).label("Close the comparison")
        .on_press(|s: &mut App| s.close_compare()).build(k).key("compare-close");
    let run = Button::new(if loading { "Comparing…" } else { "Compare" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .disabled(loading || c.base == c.head)
        .on_press(|s: &mut App| s.run_compare()).build(k).key("compare-run");
    let mut body = col().w_full().padding(16.0).gap(10.0)
        .child(row().gap(8.0).center_y().flex_wrap()
            .child(text("Compare").font(k.ty.small).color(k.c.text_muted))
            .child(actions::branch_button(s, k, i, BranchSlot::CompareHead, &c.head, "compare-head"))
            .child(text("with").font(k.ty.small).color(k.c.text_muted))
            .child(actions::branch_button(s, k, i, BranchSlot::CompareBase, &c.base, "compare-base"))
            .child(run));
    match s.picker {
        Some(BranchSlot::CompareHead) => body = body.child(actions::branch_panel(s, k, i, BranchSlot::CompareHead, &c.head)),
        Some(BranchSlot::CompareBase) => body = body.child(actions::branch_panel(s, k, i, BranchSlot::CompareBase, &c.base)),
        _ => {}
    }
    if c.base == c.head {
        body = body.child(text(format!("Pick two different branches: {} compared with itself has nothing to show.", c.head))
            .font(k.ty.small).color(k.c.text_muted).wrap().key("compare-same"));
    } else if c.result.is_none() {
        body = body.child(text(format!("Shows the commits on {} that {} doesn't have, and what they change.", c.head, c.base))
            .font(k.ty.small).color(k.c.text_muted).wrap().key("compare-hint"));
    }
    let form = panel(k).key("compare-form").border(1.0, k.c.border_strong)
        .child(panel_head(k, "Compare branches", Some(close)))
        .child(rule(k))
        .child(body);
    let out = col().w_full().gap(16.0).child(form);
    match &c.result {
        None => out,
        Some(Load::Loading) => {
            let head = panel(k).key("compare-loading").role("status").label("Comparing the branches…").child(panel_head(k, "Comparing…", None));
            out.child(ui::skeleton_rows_in_panel(k, head, 3, false))
                .child(panel(k).clip_rounded(k.radius.lg).child(ui::skeleton_diff(k, "skeleton-compare-diff")))
        }
        Some(Load::Failed(e)) => out.child(ui::error_panel(k, e, "compare-retry", |s: &mut App| s.run_compare())),
        Some(Load::Ready(cmp)) => out.children(comparison(s, k, i, c, cmp)),
    }
}

/// The result: how far apart, New pull request, the commits and the combined diff.
fn comparison(s: &App, k: &Kit, i: usize, c: &Compare, cmp: &Comparison) -> Vec<El> {
    let r = &s.data.repos[i];
    let mut summary = row().w_full().px(16.0).py(12.0).gap(10.0).center_y().flex_wrap()
        .child(ui::ico(&icons::COMPARE, 16.0, k.c.icon_muted))
        .child(text(cmp.summary()).font(k.ty.ui_medium).color(k.c.text_base).wrap().grow(1.0).shrink().min_w(200.0).key("compare-summary"));
    let mut actions_row = row().gap(8.0).center_y().flex_wrap();
    match &cmp.open_pull {
        Some((number, _)) => {
            let (repo, number) = (c.repo.clone(), *number);
            actions_row = actions_row.child(Button::new(format!("Pull request #{number}")).icon(&icons::PULL).control_size(ControlSize::Small)
                .help("A pull request from this branch is already open: read it")
                .on_press(move |s: &mut App| s.open_thread(&repo, number, true)).build(k).key("compare-open-pull"));
        }
        None if cmp.ahead > 0 => {
            actions_row = actions_row.child(Button::new("New pull request").icon(&icons::PULL).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .help(format!("Open a pull request to merge {} into {}", cmp.head, cmp.base))
                .on_press_tx(|s: &mut App, tx| {
                    s.pull_from_compare();
                    tx.scroll_into_view("form");
                }).build(k).key("compare-new-pull"));
        }
        None => {}
    }
    if !cmp.url.is_empty() {
        actions_row = actions_row.child(pages::open_button(k, "Open on GitHub", &cmp.url, "compare-github"));
    }
    summary = summary.child(actions_row);
    let mut out = vec![panel(k).key("compare-result").role("status").child(summary)];
    if cmp.nothing() {
        return out;
    }
    let mut list = panel(k).key("compare-commits")
        .child(panel_head(k, &format!("On {} and not on {}", cmp.head, cmp.base), Some(text(plural(cmp.ahead as usize, "commit")).font(k.ty.small).color(k.c.text_muted))));
    for row_ in &cmp.commits {
        list = list.child(rule(k)).child(commit_row(s, k, &r.full_name, &r.url, row_, "compare-commit"));
    }
    if cmp.more_commits {
        let mut line = note(k, format!("GitHub lists {} of them here; the rest are on GitHub.", cmp.commits.len()), "compare-more-commits".into());
        if !cmp.url.is_empty() {
            line = line.child(pages::open_button(k, "Open on GitHub", &cmp.url, "compare-more-commits-github"));
        }
        list = list.child(rule(k)).child(line);
    }
    out.push(list);
    out.push(diff_panel(s, k, Which::Compare, &cmp.files, cmp.more_files, (cmp.additions, cmp.deletions), &cmp.url, "compare-diff", "These commits change no files."));
    out
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::commits::{Comparison, Row};
    use crate::state::{BranchSlot, Load, RepoTab};
    use crate::{App, Page};

    fn app(compact: bool) -> rune::Runtime<App> {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.compact = compact;
        let full = s.data.repos[0].full_name.clone();
        s.page = Page::Repo(full);
        s.set_tab(0, RepoTab::Commits);
        let size = if compact { (390.0, 2400.0) } else { (1180.0, 2400.0) };
        let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
        rt.on_key = Some(crate::on_key);
        rt.frame(0.0);
        rt
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.1);
    }

    fn esc(rt: &mut rune::Runtime<App>, t: &mut f64) {
        rt.key(rune::KeyEvent { key: rune::Key::Escape, mods: rune::Mods::default() });
        *t += 0.05;
        rt.frame(*t);
    }

    fn texts(rt: &rune::Runtime<App>) -> Vec<String> {
        rt.elements().iter().filter_map(|e| e.text.clone()).collect()
    }

    #[test]
    fn a_commit_opens_from_the_list_and_back_returns_to_it() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(false);
        let rows: Vec<Row> = rt.state.commits.history.as_ref().and_then(|h| h.load.ready()).cloned().expect("the sample history is here");
        assert_eq!(rows.len(), crate::commits::PAGE, "a page at a time");
        assert!(rt.find("commits-load-more").is_some(), "the list says GitHub has more");
        // The newest commit's day: "Today", or "Yesterday" in the first hours after midnight.
        let day = crate::commits::day_label(rows[0].at, rt.state.now, &crate::commits::local_clock);
        assert!(texts(&rt).iter().any(|t| *t == day), "grouped by day");
        // The filter narrows what's loaded, as you type.
        rt.update(|s, _| s.commits.filter = rune::TextEdit::new("poncho"));
        rt.frame(0.1);
        assert!(rt.find(&format!("commit-{}", rows[1].sha)).is_some() && rt.find(&format!("commit-{}", rows[0].sha)).is_none());
        rt.update(|s, _| s.commits.filter.clear());
        rt.frame(0.1);
        // Load more brings the rest, and then the list says it's the whole history.
        press(&mut rt, "commits-load-more");
        let n = rt.state.commits.history.as_ref().and_then(|h| h.load.ready()).map_or(0, Vec::len);
        assert!(n > crate::commits::PAGE && rt.find("commits-end").is_some() && rt.find("commits-load-more").is_none());

        let c = rows[3].clone();
        press(&mut rt, &format!("commit-{}", c.sha));
        let open = rt.state.commits.open.as_ref().expect("the commit opened");
        assert!(matches!(&open.load, Load::Ready(d) if d.sha == c.sha && d.title == c.title));
        assert!(rt.find("commits-history").is_none(), "it takes the list's place");
        assert!(rt.find("commit-diff-view").is_some() && rt.find("commit-github").is_some() && rt.find("commit-copy").is_some());
        assert!(!rt.state.commits.commit_diff.files().is_empty() && rt.state.pull_diff.files().is_empty() && rt.state.diff.files().is_empty(), "its own diff");
        // Its parent opens from it, and Back comes back to it first.
        let parent = rows[4].clone();
        press(&mut rt, &format!("commit-parent-{}", parent.short()));
        assert!(rt.state.commits.open.as_ref().is_some_and(|o| o.sha == parent.sha));
        press(&mut rt, "commit-back");
        assert!(rt.state.commits.open.as_ref().is_some_and(|o| o.sha == c.sha), "back to the child");
        press(&mut rt, "commit-back");
        assert!(rt.state.commits.open.is_none());
        assert!(rt.find("commits-history").is_some() && rt.find(&format!("commit-{}", c.sha)).is_some(), "back to the list, as it was");
        assert!(rt.state.commits.history.as_ref().and_then(|h| h.load.ready()).map_or(0, Vec::len) == n, "the loaded pages stay");

        // Esc closes a commit too, and leaving the section forgets it.
        press(&mut rt, &format!("commit-{}", c.sha));
        let mut t = 1.0;
        esc(&mut rt, &mut t);
        assert!(rt.state.commits.open.is_none(), "Esc goes back to the list");
        press(&mut rt, &format!("commit-{}", c.sha));
        rt.key(rune::KeyEvent { key: rune::Key::Command('['), mods: rune::Mods { cmd: true, ..rune::Mods::default() } });
        rt.frame(1.5);
        assert!(rt.state.commits.open.is_none() && matches!(rt.state.page, Page::Repo(_)), "⌘[ goes back to the list, not out of the repository");
        press(&mut rt, &format!("commit-{}", c.sha));
        rt.update(|s, _| s.set_tab(0, RepoTab::Code));
        assert!(rt.state.commits.open.is_none() && rt.state.commits.commit_diff.files().is_empty());

        // What the Changes tab knows about the local clone shows above the list; nothing else asks git.
        rt.update(|s, _| {
            let full = s.data.repos[0].full_name.clone();
            s.changes = Some(crate::local::Changes { branch: "main".into(), upstream: Some("origin/main".into()), ahead: 2, behind: 1, ..Default::default() });
            s.diff_of = Some(crate::diffs::DiffOf::Local(full));
            s.set_tab(0, RepoTab::Commits);
        });
        rt.frame(2.0);
        assert!(rt.find("commits-clone").is_some() && rt.find("commits-changes").is_some());
        assert!(texts(&rt).iter().any(|t| t == "Your local clone, on main, has 2 commits not pushed yet and is 1 commit behind origin/main."));
        rt.update(|s, _| s.diff_of = Some(crate::diffs::DiffOf::Local("someone/else".into())));
        rt.frame(2.1);
        assert!(rt.find("commits-clone").is_none(), "another repository's clone says nothing here");
    }

    #[test]
    fn back_puts_the_row_it_came_from_in_view() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.pending_place = None;
        s.page = Page::Repo(s.data.repos[0].full_name.clone());
        s.set_tab(0, RepoTab::Commits);
        let sha = s.commits.history.as_ref().and_then(|h| h.load.ready()).map(|r| r[24].sha.clone()).unwrap();
        let key = format!("commit-{sha}");
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
        rt.frame(0.0);
        let k2 = key.clone();
        rt.update(move |_, tx| tx.scroll_key_to(k2.clone(), 300.0));
        let mut t = 0.0;
        for _ in 0..10 {
            t += 0.1;
            rt.frame(t);
        }
        let (_, y) = rt.find(&key).expect("row 25 is laid out");
        assert!((0.0..760.0).contains(&y), "scrolled to row 25 ({y})");
        press(&mut rt, &key);
        assert!(rt.state.commits.open.is_some());
        press(&mut rt, "commit-back");
        for _ in 0..10 {
            t += 0.1;
            rt.frame(t);
        }
        let (_, y) = rt.find(&key).expect("back in the list");
        assert!((0.0..760.0).contains(&y), "the row it was opened from is in view again ({y})");
    }

    #[test]
    fn compare_two_branches_then_open_the_pull_request_form() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(false);
        press(&mut rt, "commits-compare");
        let c = rt.state.commits.compare.as_ref().expect("the form opened");
        assert_eq!((c.base.as_str(), c.head.as_str()), ("main", "feature/readme-selection"), "the default branch, and another one");
        assert!(rt.find("commits-history").is_none() && rt.find("compare-hint").is_some());
        // The same branch twice can't be compared.
        rt.update(|s, _| s.pick_branch_for(0, BranchSlot::CompareHead, "main".into()));
        rt.frame(0.1);
        assert!(rt.find("compare-same").is_some());
        // Picking another branch through the picker.
        press(&mut rt, "compare-head");
        assert_eq!(rt.state.picker, Some(BranchSlot::CompareHead));
        press(&mut rt, "pick-fix/theme-switch");
        assert_eq!(rt.state.commits.compare.as_ref().map(|c| c.head.as_str()), Some("fix/theme-switch"));
        press(&mut rt, "compare-run");
        let cmp: Comparison = match rt.state.commits.compare.as_ref().and_then(|c| c.result.as_ref()) {
            Some(Load::Ready(cmp)) => cmp.clone(),
            other => panic!("compared: {}", other.is_some()),
        };
        assert_eq!(cmp.summary(), "3 commits ahead, 1 behind main");
        assert!(rt.find("compare-summary").is_some() && rt.find("compare-diff-view").is_some());
        assert!(rt.find(&format!("compare-commit-{}", cmp.commits[0].sha)).is_some(), "the same rows as the history");
        assert!(!rt.state.commits.compare_diff.files().is_empty() && rt.state.commits.commit_diff.files().is_empty());
        // A commit of the comparison opens over it, and Back returns to the comparison.
        press(&mut rt, &format!("compare-commit-{}", cmp.commits[0].sha));
        assert!(rt.state.commits.open.is_some() && rt.find("compare-summary").is_none());
        assert!(rt.state.commits.open.as_ref().is_some_and(|o| matches!(&o.load, Load::Ready(d) if d.title == cmp.commits[0].title)));
        press(&mut rt, "commit-back");
        assert!(rt.find("compare-summary").is_some());
        // New pull request: the existing form, with both branches filled in, on this section.
        press(&mut rt, "compare-new-pull");
        let f = rt.state.form.as_ref().expect("the form opened");
        assert_eq!((f.kind, f.from.as_str(), f.into.as_str()), (crate::state::Compose::Pull, "fix/theme-switch", "main"));
        assert_eq!(rt.state.tab, RepoTab::Commits, "the comparison stays on screen under the form");
        assert!(rt.find("form-submit").is_some() && rt.find("form-note").is_some(), "sample data: it says why it can't be sent");
        // Once it's opened, the result shows it instead of offering another.
        rt.update(|s, _| {
            let full = s.data.repos[0].full_name.clone();
            s.apply_done(&full, crate::github::Done::Pull(crate::github::Created { number: 99, url: "https://github.com/x/y/pull/99".into(), ..Default::default() }));
        });
        rt.frame(0.1);
        assert!(rt.find("compare-new-pull").is_none() && rt.find("compare-open-pull").is_some());
        // Esc closes the form first (the focused title may take the first one), then the comparison.
        let mut t = 2.0;
        esc(&mut rt, &mut t);
        if rt.state.form.is_some() {
            esc(&mut rt, &mut t);
        }
        assert!(rt.state.form.is_none() && rt.state.commits.compare.is_some());
        esc(&mut rt, &mut t);
        assert!(rt.state.commits.compare.is_none() && rt.find("commits-history").is_some());
    }

    #[test]
    fn loading_failed_and_empty_say_so_with_retry() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(false);
        let set = |rt: &mut rune::Runtime<App>, load: Load<Vec<Row>>| {
            rt.update(move |s, _| s.commits.history.as_mut().unwrap().load = load);
            rt.frame(0.1);
        };
        set(&mut rt, Load::Loading);
        assert!(rt.find("skeleton-commits").is_some());
        set(&mut rt, Load::Failed("GitHub error 500: Server error".into()));
        assert!(rt.find("commits-failed").is_some() && rt.find("retry-commits").is_some());
        press(&mut rt, "retry-commits");
        assert!(rt.state.commits.history.as_ref().and_then(|h| h.load.ready()).is_some_and(|r| !r.is_empty()), "Retry reads it again");
        rt.update(|s, _| s.commits.history.as_mut().unwrap().more = false);
        set(&mut rt, Load::Ready(Vec::new()));
        assert!(rt.find("commits-empty").is_some() && rt.find("commits-load-more").is_none());
        press(&mut rt, "commits-compare");
        // The comparison: loading, then failed with Retry.
        rt.update(|s, _| {
            let c = s.commits.compare.as_mut().unwrap();
            c.head = "fix/theme-switch".into();
            c.result = Some(Load::Loading);
        });
        rt.frame(0.2);
        assert!(rt.find("compare-loading").is_some() && rt.find("skeleton-compare-diff").is_some());
        rt.update(|s, _| s.commits.compare.as_mut().unwrap().result = Some(Load::Failed("Not found (404)".into())));
        rt.frame(0.3);
        assert!(rt.find("compare-retry").is_some());
        press(&mut rt, "compare-retry");
        assert!(matches!(rt.state.commits.compare.as_ref().and_then(|c| c.result.as_ref()), Some(Load::Ready(_))), "Retry compares again");
        // A commit: loading, then failed with Retry.
        let repo = rt.state.data.repos[0].full_name.clone();
        rt.update(move |s, _| {
            s.open_commit(&repo, "0123abc", None);
            s.commits.open.as_mut().unwrap().load = Load::Loading;
        });
        rt.frame(0.4);
        assert!(rt.find("commit-loading").is_some());
        rt.update(|s, _| s.commits.open.as_mut().unwrap().load = Load::Failed("Not found (404)".into()));
        rt.frame(0.5);
        assert!(rt.find("commit-failed").is_some() && rt.find("commit-retry").is_some());
    }

    #[test]
    fn on_a_phone_nothing_overflows_and_the_diff_is_unified() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(true);
        let sha = rt.state.commits.history.as_ref().and_then(|h| h.load.ready()).map(|r| r[0].sha.clone()).unwrap();
        press(&mut rt, &format!("commit-{sha}"));
        rt.frame(0.2);
        assert_eq!(rt.state.commits.commit_diff.mode(), rune_kit::diff::DiffMode::Unified);
        assert!(rt.find("commit-diff-mode").is_none(), "no Split on a phone");
        let mut checked = 0;
        for e in rt.elements() {
            if let Some(key) = e.key.as_ref().filter(|k| k.starts_with("commit")) {
                checked += 1;
                assert!(e.rect[0] >= -0.5 && e.rect[0] + e.rect[2] <= 390.5, "{key} overflows: {:?}", e.rect);
            }
        }
        assert!(checked > 5);
    }
}
