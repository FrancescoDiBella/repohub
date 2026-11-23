//! Diffs on screen: the Changes tab (the local clone's changes, commit, push and pull) and a
//! pull request's files with Merge, Squash and Rebase.
use rune::{El, col, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::diffs::{self, DiffOf};
use crate::state::Load;
use crate::ui::{self, panel, panel_head, rule};
use crate::confirm::{Confirm, Tone};
use crate::{App, icons};

/// The diff's height: it scrolls inside, so a long diff never makes the page long.
const DIFF_H: f32 = 620.0;

/// The diff's height: on a phone it leaves room to see what's around it and to scroll the page.
fn diff_h(s: &App) -> f32 {
    if s.compact { (s.backdrop_size[1] - 260.0).clamp(260.0, DIFF_H) } else { DIFF_H }
}


/// Unified / Split, and the files' totals.
fn diff_head(s: &App, k: &Kit) -> El {
    let n = s.diff.files().len();
    let loading_empty = n == 0 && (matches!(s.diff_load, None | Some(Load::Loading)) || s.git_job.is_some());
    let failed_empty = n == 0 && matches!(s.diff_load, Some(Load::Failed(_)));
    let mut r = row().gap(10.0).center_y();
    if loading_empty {
        r = r.child(ui::skeleton_faint(k, 88.0, 12.0));
    } else if !failed_empty {
        let (add, del) = diffs::totals(s.diff.files());
        r = r
            .child(text(format!("{n} file{}", if n == 1 { "" } else { "s" })).font(k.ty.small).color(k.c.text_muted))
            .child(text(format!("+{add}")).font(k.ty.small_medium).color(k.c.success.icon))
            .child(text(format!("−{del}")).font(k.ty.small_medium).color(k.c.danger.icon));
    }
    // Side by side needs width a phone doesn't have: there it's always unified.
    if s.compact {
        return r;
    }
    r.child(SegmentedControl::new(&["Unified", "Split"], Some(usize::from(s.diff.mode() == DiffMode::Split))).label("Diff layout").fit()
        .on_pick(|s: &mut App, i, _| s.diff.set_mode(if i == 1 { DiffMode::Split } else { DiffMode::Unified })).build(k).key("diff-mode"))
}

fn diff_body(s: &App, k: &Kit, i: usize, empty: &str) -> El {
    if matches!(s.diff_load, None | Some(Load::Loading)) || (s.git_job.is_some() && s.diff.files().is_empty()) {
        return ui::skeleton_diff(k, "skeleton-diff");
    }
    match &s.diff_load {
        None | Some(Load::Loading) => ui::skeleton_diff(k, "skeleton-diff"),
        Some(Load::Failed(e)) => {
            let merge_branch = match &s.diff_of {
                Some(DiffOf::Merge(_, b)) => Some(b.clone()),
                _ => None,
            };
            ui::error_inline(k, e, "retry-changes", move |s: &mut App| match &merge_branch {
                Some(b) => s.preview_merge(i, b.clone()),
                None => s.load_changes(i),
            })
        }
        Some(Load::Ready(())) if s.diff.files().is_empty() => col().w_full().padding(24.0).items_center().child(text(empty.to_owned()).font(k.ty.ui).color(k.c.text_muted)),
        Some(Load::Ready(())) => col().w_full().h(diff_h(s)).clip_rounded(0.0)
            .child(DiffView::new(|s: &App| &s.diff, |s: &mut App| &mut s.diff).key("diff").build(s, k)),
    }
}

/// The Changes tab of repository `i`.
pub fn changes_tab(s: &App, k: &Kit, i: usize) -> El {
    let c = s.changes.as_ref();
    let busy = s.git_job.is_some();
    let branch_label = match c {
        Some(c) => text(if c.branch.is_empty() { "detached HEAD".to_owned() } else { c.branch.clone() }).font(k.ty.ui_medium).color(k.c.text_base),
        None if matches!(s.diff_load, None | Some(Load::Loading)) || busy => ui::skeleton_bar(k, 96.0, 14.0),
        None => text("—").font(k.ty.ui_medium).color(k.c.text_muted),
    };
    // Where the branch stands: name, upstream, commits to push and to pull.
    let mut status = row().w_full().px(16.0).py(12.0).gap(10.0).center_y().flex_wrap()
        .child(ui::ico(&icons::BRANCH, 15.0, k.c.icon_muted))
        .child(branch_label);
    if let Some(c) = c {
        status = status.child(text(match &c.upstream {
            Some(u) => format!("→ {u}"),
            None => "not pushed yet".into(),
        }).font(k.ty.small).color(k.c.text_muted));
        if c.ahead > 0 {
            status = status.child(ui::pill(k, &format!("{} to push", c.ahead), k.c.success.icon));
        }
        if c.behind > 0 {
            status = status.child(ui::pill(k, &format!("{} to pull", c.behind), k.c.warning.icon));
        }
    }
    status = status.child(rune::el().grow(1.0));
    if let Some(job) = s.git_job {
        status = status.child(text(job).font(k.ty.small).color(k.c.text_muted));
    }
    // Pulling over uncommitted changes and pushing to the default branch ask first.
    let dirty = c.is_some_and(|c| !c.is_empty());
    let on_default = c.is_some_and(|c| s.data.repos.get(i).is_some_and(|r| !c.branch.is_empty() && c.branch == r.branch));
    status = status
        .child(Button::new("Refresh").icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small).disabled(busy)
            .on_press(move |s: &mut App| s.load_changes(i)).build(k).key("changes-refresh"))
        .child(Button::new("Pull").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).disabled(busy || c.is_none_or(|c| c.upstream.is_none()))
            .help("git pull --ff-only: brings in the upstream's commits, never makes a merge")
            .on_press(move |s: &mut App| if dirty { s.ask(ASK_PULL) } else { pull(s, i) }).build(k).key("changes-pull"))
        .child(Button::new("Push").icon(&icons::PUSH).control_size(ControlSize::Small).disabled(busy || c.is_none_or(|c| c.ahead == 0 && c.upstream.is_some()))
            .on_press(move |s: &mut App| if on_default { s.ask(ASK_PUSH) } else { push(s, i) }).build(k).key("changes-push"))
        .child(Button::new("New pull request").icon(&icons::PULL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.open_form(i, crate::state::Compose::Pull)).build(k).key("changes-new-pull"));
    let nothing = c.is_none_or(|c| c.is_empty());
    let message = TextInput::new(&s.commit_msg).label("Commit message").placeholder("What changed, in one line")
        .on_edit(|s: &mut App| &mut s.commit_msg).on_submit(move |s: &mut App, _| s.commit(i, false)).build(k).key("commit-msg").grow(1.0).min_w(240.0);
    let mut commit = col().w_full().px(16.0).py(12.0).gap(8.0)
        .child(row().w_full().gap(8.0).center_y().flex_wrap()
            .child(message)
            .child(Button::new("Commit").control_size(ControlSize::Small).disabled(busy || nothing)
                .on_press(move |s: &mut App| s.commit(i, false)).build(k).key("commit"))
            .child(Button::new("Commit and push").variant(ButtonVariant::Submit).control_size(ControlSize::Small).disabled(busy || nothing)
                .on_press(move |s: &mut App| if on_default { s.ask(ASK_COMMIT_PUSH) } else { s.commit(i, true) }).build(k).key("commit-push")))
        .child(text("Commits every change below (git add -A), new files included.").font(k.ty.text_small).color(k.c.text_muted).wrap());
    if let Some(e) = &s.git_error {
        commit = commit.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    let mut top = panel(k).key("changes-status").child(status);
    if let Some(q) = git_question(s, k, i) {
        top = top.child(col().w_full().px(16.0).pb(12.0).child(q));
    }
    let mut top = top.child(rule(k));
    if let Some(m) = merge_section(s, k, i) {
        top = top.child(m).child(rule(k));
    }
    let top = top.child(commit);
    let (title, empty) = match &s.diff_of {
        Some(DiffOf::Merge(_, b)) => (format!("What {b} brings"), "It brings no changes."),
        _ => ("Changes since the last commit".to_owned(), "Nothing changed since the last commit."),
    };
    let mut head = row().gap(8.0).center_y().child(diff_head(s, k));
    if matches!(s.diff_of, Some(DiffOf::Merge(..))) {
        head = head.child(Button::new("My changes").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.load_changes(i)).build(k).key("merge-back"));
    }
    let diff = panel(k).key("changes-diff").clip_rounded(k.radius.lg)
        .child(panel_head(k, &title, Some(head)))
        .child(rule(k))
        .child(diff_body(s, k, i, empty));
    col().w_full().gap(16.0).child(top).child(diff)
}

const ASK_ABORT: &str = "ask-abort-merge";
const ASK_PULL: &str = "ask-pull-over-changes";
const ASK_PUSH: &str = "ask-push-default";
const ASK_COMMIT_PUSH: &str = "ask-commit-push-default";

fn pull(s: &mut App, i: usize) {
    s.git_run(i, "Pulling…", "Pulled", |root| crate::local::pull(root).map(|_| String::new()));
}

fn push(s: &mut App, i: usize) {
    s.git_run(i, "Pushing…", "Pushed", |root| crate::local::push(root).map(|_| String::new()));
}

/// The Changes tab's questions (only one is open at a time): aborting a merge, pulling over
/// uncommitted changes, pushing (or committing and pushing) straight to the default branch.
fn git_question(s: &App, k: &Kit, i: usize) -> Option<El> {
    let c = s.changes.as_ref()?;
    let branch = c.branch.clone();
    let upstream = c.upstream.clone().unwrap_or_else(|| format!("origin/{branch}"));
    let commits = match c.ahead {
        0 => "The branch".to_owned(),
        1 => "Your commit".to_owned(),
        n => format!("Your {n} commits"),
    };
    let asks = [
        Confirm::new(ASK_ABORT, &icons::CROSS, "Abort the merge?")
            .detail("Git puts the files back as they were before it (git merge --abort): conflicts you resolved and changes made since it began are lost.")
            .yes("Abort merge", Tone::Danger, move |s: &mut App| s.git_run(i, "Aborting the merge…", "Merge aborted", |root| crate::local::abort_merge(root).map(|_| String::new()))),
        Confirm::new(ASK_PULL, &icons::REFRESH, "Pull with uncommitted changes?")
            .detail(format!("{upstream}'s commits come in under your changes (git pull --ff-only). If they touch the same files, git stops and changes nothing."))
            .yes("Pull", Tone::Go, move |s: &mut App| pull(s, i)),
        Confirm::new(ASK_PUSH, &icons::PUSH, format!("Push to {branch}, the default branch?"))
            .detail(format!("{commits} goes to {branch} on GitHub straight away, without a pull request. A push can't be undone from RepoHub."))
            .yes("Push", Tone::Go, move |s: &mut App| push(s, i)),
        Confirm::new(ASK_COMMIT_PUSH, &icons::PUSH, format!("Commit and push to {branch}, the default branch?"))
            .detail(format!("The commit goes to {branch} on GitHub straight away, without a pull request. A push can't be undone from RepoHub."))
            .yes("Commit and push", Tone::Go, move |s: &mut App| s.commit(i, true)),
    ];
    asks.into_iter().find_map(|q| q.show(s, k))
}

/// Merging a local branch into the checked-out one: pick a branch, preview, merge; during a
/// merge, its conflicts and Abort.
fn merge_section(s: &App, k: &Kit, i: usize) -> Option<El> {
    let c = s.changes.as_ref()?;
    let busy = s.git_job.is_some();
    if c.merging {
        let tint = if c.conflicts.is_empty() { k.c.success.icon } else { k.c.warning.icon };
        let msg = if c.conflicts.is_empty() {
            "A merge is under way and every conflict is resolved: Commit finishes it.".to_owned()
        } else {
            format!("A merge is under way. {} file{} in conflict: fix them in your editor or the Terminal, then Commit finishes the merge.", c.conflicts.len(), if c.conflicts.len() == 1 { "" } else { "s" })
        };
        let files = c.conflicts.iter().take(12).map(|f| row().gap(8.0).center_y()
            .child(text("!").font(k.ty.mono).color(k.c.danger.icon).w(14.0))
            .child(text(f.clone()).font(k.ty.mono).color(k.c.text_base)));
        return Some(col().w_full().px(16.0).py(12.0).gap(8.0).fill(tint.alpha(0.08)).key("merge-state")
            .child(text(msg).font(k.ty.small).color(k.c.text_base).wrap())
            .children(files)
            .child(row().gap(8.0)
                .child(Button::new("Abort merge").variant(ButtonVariant::Danger).control_size(ControlSize::Small).disabled(busy)
                    .on_press(|s: &mut App| s.ask(ASK_ABORT)).build(k).key("merge-abort"))
                .child(Button::new("Open Terminal").icon(&icons::TERMINAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.set_tab(i, crate::state::RepoTab::Terminal)).build(k).key("merge-terminal"))));
    }
    if c.branches.is_empty() || c.branch.is_empty() {
        return None;
    }
    let picked = match &s.diff_of {
        Some(DiffOf::Merge(_, b)) => Some(b.clone()),
        _ => None,
    };
    let chips = c.branches.iter().take(12).map(|b| {
        let on = picked.as_deref() == Some(b.as_str());
        let name = b.clone();
        Button::new(b.clone()).icon(&icons::BRANCH).variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Ghost }).control_size(ControlSize::Small)
            .help(format!("Preview merging {b} into {}", c.branch))
            .on_press(move |s: &mut App| s.preview_merge(i, name.clone())).build(k).key(format!("merge-pick-{b}"))
    });
    let mut out = col().w_full().px(16.0).py(12.0).gap(8.0).key("merge-pick")
        .child(text(format!("Merge a branch into {}", c.branch)).font(k.ty.small_medium).color(k.c.text_base))
        .child(row().w_full().gap(4.0).flex_wrap().children(chips));
    if let (Some(b), Some(p)) = (picked, s.merge_preview.as_ref().filter(|p| picked_is(&s.diff_of, &p.branch))) {
        let n = p.commits.len();
        let commits = p.commits.iter().take(8).map(|l| text(l.clone()).font(k.ty.mono).color(k.c.text_muted).max_lines(1));
        let dirty = !c.patch.trim().is_empty();
        out = out
            .child(text(if n == 0 { format!("{b} has no commits that {} doesn't have.", c.branch) } else { format!("{n} commit{} to bring in:", if n == 1 { "" } else { "s" }) })
                .font(k.ty.text_small).color(k.c.text_muted))
            .children(commits)
            .child(row().gap(8.0).center_y()
                .child(Button::new(format!("Merge {b} into {}", c.branch)).icon(&icons::MERGE).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                    .disabled(busy || n == 0 || dirty)
                    .on_press(move |s: &mut App| s.merge_local(i, b.clone())).build(k).key("merge-local"))
                .child(if dirty { text("Commit your changes first.").font(k.ty.text_small).color(k.c.warning.icon) } else { rune::el() }));
    }
    Some(out)
}

fn picked_is(of: &Option<DiffOf>, branch: &str) -> bool {
    matches!(of, Some(DiffOf::Merge(_, b)) if b == branch)
}

fn pull_diff_head(s: &App, k: &Kit) -> El {
    let n = s.pull_diff.files().len();
    let loading_empty = n == 0 && matches!(s.pull_diff_load, None | Some(Load::Loading));
    let failed_empty = n == 0 && matches!(s.pull_diff_load, Some(Load::Failed(_)));
    let mut r = row().gap(10.0).center_y();
    if loading_empty {
        r = r.child(ui::skeleton_faint(k, 88.0, 12.0));
    } else if !failed_empty {
        let (add, del) = diffs::totals(s.pull_diff.files());
        r = r
            .child(text(format!("{n} file{}", if n == 1 { "" } else { "s" })).font(k.ty.small).color(k.c.text_muted))
            .child(text(format!("+{add}")).font(k.ty.small_medium).color(k.c.success.icon))
            .child(text(format!("−{del}")).font(k.ty.small_medium).color(k.c.danger.icon));
    }
    if s.compact {
        return r;
    }
    r.child(SegmentedControl::new(&["Unified", "Split"], Some(usize::from(s.pull_diff.mode() == DiffMode::Split))).label("Diff layout").fit()
        .on_pick(|s: &mut App, i, _| s.pull_diff.set_mode(if i == 1 { DiffMode::Split } else { DiffMode::Unified })).build(k).key("pull-diff-mode"))
}

fn pull_diff_body(s: &App, k: &Kit, full: &str, number: u32, empty: &str) -> El {
    match &s.pull_diff_load {
        None | Some(Load::Loading) => ui::skeleton_diff(k, "skeleton-pull-diff"),
        Some(Load::Failed(e)) => {
            let repo = full.to_owned();
            ui::error_inline(k, e, "retry-pull-diff", move |s: &mut App| s.open_pull_diff(repo.clone(), number))
        }
        Some(Load::Ready(())) if s.pull_diff.files().is_empty() => col().w_full().padding(24.0).items_center().child(text(empty.to_owned()).font(k.ty.ui).color(k.c.text_muted)),
        Some(Load::Ready(())) => col().w_full().h(diff_h(s)).clip_rounded(0.0)
            .child({
                // The "+" by a line number writes a comment there; comments show under their line.
                let kit = k.clone();
                DiffView::new(|s: &App| &s.pull_diff, |s: &mut App| &mut s.pull_diff).key("pull-diff-view")
                    .on_comment(|s: &mut App, range, _| s.start_line_comment(range))
                    .annotation(move |s: &App, a| crate::review::annotation(s, &kit, a))
                    .build(s, k)
            }),
    }
}

/// A pull request's files, when one is open (`App::pull_diff_of`), with the merge buttons.
pub fn pull_panel(s: &App, k: &Kit) -> Option<El> {
    pull_panel_for(s, k, None)
}

/// A pull request's files scoped to `repo_filter` (or any repo if None).
pub fn pull_panel_for(s: &App, k: &Kit, repo_filter: Option<&str>) -> Option<El> {
    let (full, number) = s.pull_diff_of.as_ref()?.clone();
    if let Some(target) = repo_filter && !full.eq_ignore_ascii_case(target) {
        return None;
    }
    let pull = s.data.pulls.iter().find(|p| p.number == number && pull_repo(&p.url).is_some_and(|f| f.eq_ignore_ascii_case(&full)));
    let title = pull.map(|p| format!("#{number} {}", p.title)).unwrap_or_else(|| format!("#{number}"));
    let open = pull.is_some_and(|p| p.state == crate::model::PullState::Open);
    let url = pull.map(|p| p.url.clone()).unwrap_or_else(|| format!("https://github.com/{full}/pull/{number}"));
    let files_url = format!("{url}/files");
    let key = crate::actions::busy_key(&full, number);
    let busy = s.busy.contains(&key);
    let mut tools = row().gap(8.0).center_y().flex_wrap().child(pull_diff_head(s, k));
    // Merge asks first (the same question as the row's: how, then Merge); on a computer it
    // goes under the panel's head, in place of the button.
    let title_text = pull.map(|p| p.title.clone()).unwrap_or_default();
    let question = if open && !busy {
        crate::actions::merge_question(s, k, &full, number, &title_text).keys("pull-merge-confirm", "pull-merge-cancel").show(s, k)
    } else {
        None
    };
    if open && question.is_none() {
        let ask = crate::actions::merge_ask(&full, number);
        tools = tools
            .child(Button::new(if busy { "Merging…" } else { "Merge" }).icon(&icons::MERGE).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .disabled(busy || crate::actions::blocked(s).is_some())
                .help("Asks how to merge (a merge commit, squash or rebase), then merges")
                .on_press(move |s: &mut App| s.ask(ask.clone())).build(k).key("pull-merge"));
    }
    tools = tools
        .child(Button::new("Open on GitHub").icon(&icons::EXTERNAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); }).build(k).key("pull-github"))
        .child(rune_kit::IconButton::new(&icons::CROSS).variant(rune_kit::IconButtonVariant::GhostMuted).label("Close the diff")
            .on_press(|s: &mut App| s.leave_pull_diff()).build(k).key("pull-close"));
    // GitHub has more comments on lines than the first page read: they're not all under their lines.
    let more = s.pull_review.as_ref().filter(|r| r.more).map(|r| {
        row().w_full().px(16.0).py(8.0).gap(8.0).center_y().flex_wrap().key("pull-comments-more")
            .child(text(format!("Showing the first {} comments on lines; the rest are on GitHub.", r.comments.len())).font(k.ty.small).color(k.c.text_muted).wrap().shrink())
            .child(crate::pages::open_button(k, "All comments on GitHub", &files_url, "pull-comments-all"))
    });
    let mut out = panel(k).key("pull-diff").clip_rounded(k.radius.lg)
        .child(row().w_full().px(16.0).py(10.0).gap(12.0).center_y().flex_wrap()
            .child(text(title).font(k.ty.ui_medium).color(k.c.text_base).grow(1.0).shrink().min_w(200.0).max_lines(2))
            .child(tools));
    if let Some(q) = question {
        out = out.child(col().w_full().px(16.0).pb(12.0).child(q));
    }
    let mut out = out.child(rule(k));
    if let Some(m) = more {
        out = out.child(m).child(rule(k));
    }
    Some(out.child(pull_diff_body(s, k, &full, number, "This pull request changes no files.")))
}

/// `owner/name` from a pull request's page address.
pub fn pull_repo(url: &str) -> Option<String> {
    let rest = url.split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    Some(format!("{}/{}", parts.next()?, parts.next()?))
}

/// The Files button on a pull request row.
pub fn files_button(k: &Kit, p: &crate::model::Pull, active: bool) -> Option<El> {
    let full = pull_repo(&p.url)?;
    let number = p.number;
    Some(Button::new("Files").icon(&icons::FILE)
        .variant(if active { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
        .control_size(ControlSize::Small)
        .help("See what this pull request changes")
        .on_press_tx(move |s: &mut App, tx| {
            if active {
                s.leave_pull_diff();
            } else {
                s.open_pull_diff(full.clone(), number);
                tx.scroll_into_view("pull-diff");
            }
        }).build(k).key(format!("files-{}-{number}", p.repo)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn repo_from_a_pull_url() {
        assert_eq!(super::pull_repo("https://github.com/octo/app/pull/12").as_deref(), Some("octo/app"));
        assert_eq!(super::pull_repo("nope"), None);
    }
}
