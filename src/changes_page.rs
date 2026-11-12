//! Diffs on screen: the Changes tab (the local clone's changes, commit, push and pull) and a
//! pull request's files with Merge, Squash and Rebase.
use rune::{El, col, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl, TextInput};

use crate::diffs::{self, DiffOf};
use crate::state::Load;
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, github, icons};

/// The diff's height: it scrolls inside, so a long diff never makes the page long.
const DIFF_H: f32 = 620.0;

/// Unified / Split, and the files' totals.
fn diff_head(s: &App, k: &Kit) -> El {
    let (add, del) = diffs::totals(s.diff.files());
    let n = s.diff.files().len();
    row().gap(10.0).center_y()
        .child(text(format!("{n} file{}", if n == 1 { "" } else { "s" })).font(k.ty.small).color(k.c.text_muted))
        .child(text(format!("+{add}")).font(k.ty.small_medium).color(k.c.success.icon))
        .child(text(format!("−{del}")).font(k.ty.small_medium).color(k.c.danger.icon))
        .child(SegmentedControl::new(&["Unified", "Split"], Some(usize::from(s.diff.mode() == DiffMode::Split))).label("Diff layout").fit()
            .on_pick(|s: &mut App, i, _| s.diff.set_mode(if i == 1 { DiffMode::Split } else { DiffMode::Unified })).build(k).key("diff-mode"))
}

fn diff_body(s: &App, k: &Kit, empty: &str) -> El {
    match &s.diff_load {
        None | Some(Load::Loading) => col().w_full().padding(24.0).items_center().child(text("Reading the changes…").font(k.ty.ui).color(k.c.text_muted)),
        Some(Load::Failed(e)) => col().w_full().padding(16.0).child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap()),
        Some(Load::Ready(())) if s.diff.files().is_empty() => col().w_full().padding(24.0).items_center().child(text(empty.to_owned()).font(k.ty.ui).color(k.c.text_muted)),
        Some(Load::Ready(())) => col().w_full().h(DIFF_H).clip_rounded(0.0)
            .child(DiffView::new(|s: &App| &s.diff, |s: &mut App| &mut s.diff).key("diff").build(s, k)),
    }
}

/// The Changes tab of repository `i`.
pub fn changes_tab(s: &App, k: &Kit, i: usize) -> El {
    let c = s.changes.as_ref();
    let busy = s.git_job.is_some();
    // Where the branch stands: name, upstream, commits to push and to pull.
    let mut status = row().w_full().px(16.0).py(12.0).gap(10.0).center_y().flex_wrap()
        .child(ui::ico(&icons::BRANCH, 15.0, k.c.icon_muted))
        .child(text(c.map(|c| if c.branch.is_empty() { "detached HEAD".to_owned() } else { c.branch.clone() }).unwrap_or_else(|| "…".into())).font(k.ty.ui_medium).color(k.c.text_base));
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
    status = status
        .child(Button::new("Refresh").icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small).disabled(busy)
            .on_press(move |s: &mut App| s.load_changes(i)).build(k).key("changes-refresh"))
        .child(Button::new("Pull").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).disabled(busy || c.is_none_or(|c| c.upstream.is_none()))
            .help("git pull --ff-only: brings in the upstream's commits, never makes a merge")
            .on_press(move |s: &mut App| s.git_run(i, "Pulling…", "Pulled", |root| crate::local::pull(root).map(|_| String::new()))).build(k).key("changes-pull"))
        .child(Button::new("Push").icon(&icons::PULL).control_size(ControlSize::Small).disabled(busy || c.is_none_or(|c| c.ahead == 0 && c.upstream.is_some()))
            .on_press(move |s: &mut App| s.git_run(i, "Pushing…", "Pushed", |root| crate::local::push(root).map(|_| String::new()))).build(k).key("changes-push"))
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
                .on_press(move |s: &mut App| s.commit(i, true)).build(k).key("commit-push")))
        .child(text("Commits every change below (git add -A), new files included.").font(k.ty.text_small).color(k.c.text_muted));
    if let Some(e) = &s.git_error {
        commit = commit.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    let top = panel(k).key("changes-status").child(status).child(rule(k)).child(commit);
    let diff = panel(k).key("changes-diff").clip_rounded(k.radius.lg)
        .child(panel_head(k, "Changes since the last commit", Some(diff_head(s, k))))
        .child(rule(k))
        .child(diff_body(s, k, "Nothing changed since the last commit."));
    col().w_full().gap(16.0).child(top).child(diff)
}

/// A pull request's files, when one is open (`DiffOf::Pull`), with the merge buttons.
pub fn pull_panel(s: &App, k: &Kit) -> Option<El> {
    let Some(DiffOf::Pull(full, number)) = &s.diff_of else { return None };
    let (full, number) = (full.clone(), *number);
    let pull = s.data.pulls.iter().find(|p| p.number == number && pull_repo(&p.url).is_some_and(|f| f.eq_ignore_ascii_case(&full)));
    let title = pull.map(|p| format!("#{number} {}", p.title)).unwrap_or_else(|| format!("#{number}"));
    let open = pull.is_some_and(|p| p.state == crate::model::PullState::Open);
    let url = pull.map(|p| p.url.clone()).unwrap_or_else(|| format!("https://github.com/{full}/pull/{number}"));
    let busy = s.busy.contains(&crate::actions::busy_key(&full, number));
    let mut tools = row().gap(8.0).center_y().flex_wrap().child(diff_head(s, k));
    if open {
        const METHODS: [&str; 3] = ["merge", "squash", "rebase"];
        let method = METHODS[s.merge_method.min(2)];
        let repo = full.clone();
        tools = tools
            .child(SegmentedControl::new(&["Merge", "Squash", "Rebase"], Some(s.merge_method.min(2))).label("Merge method").fit()
                .on_pick(|s: &mut App, i, _| s.merge_method = i).build(k).key("merge-method"))
            .child(Button::new(if busy { "Merging…" } else { "Merge" }).icon(&icons::MERGE).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .disabled(busy || crate::actions::blocked(s).is_some())
                .help(match method { "squash" => "One commit with all the changes", "rebase" => "The commits replayed on the base, no merge commit", _ => "A merge commit" })
                .on_press(move |s: &mut App| s.write(&repo, github::Write::Merge { number, method })).build(k).key("pull-merge"));
    }
    tools = tools
        .child(Button::new("GitHub").icon(&icons::EXTERNAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&url); }).build(k).key("pull-github"))
        .child(rune_kit::IconButton::new(&icons::CROSS).variant(rune_kit::IconButtonVariant::GhostMuted).label("Close the diff")
            .on_press(|s: &mut App| s.close_diff()).build(k).key("pull-close"));
    Some(panel(k).key("pull-diff").clip_rounded(k.radius.lg)
        .child(row().w_full().px(16.0).py(10.0).gap(12.0).center_y().flex_wrap()
            .child(text(title).font(k.ty.ui_medium).color(k.c.text_base).grow(1.0).shrink().min_w(200.0).max_lines(2))
            .child(tools))
        .child(rule(k))
        .child(diff_body(s, k, "This pull request changes no files.")))
}

/// `owner/name` from a pull request's page address.
pub fn pull_repo(url: &str) -> Option<String> {
    let rest = url.split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    Some(format!("{}/{}", parts.next()?, parts.next()?))
}

/// The Files button on a pull request row.
pub fn files_button(k: &Kit, p: &crate::model::Pull) -> Option<El> {
    let full = pull_repo(&p.url)?;
    let number = p.number;
    Some(Button::new("Files").icon(&icons::FILE).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .help("See what this pull request changes")
        .on_press(move |s: &mut App| s.open_pull_diff(full.clone(), number)).build(k).key(format!("files-{}-{number}", p.repo)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn repo_from_a_pull_url() {
        assert_eq!(super::pull_repo("https://github.com/octo/app/pull/12").as_deref(), Some("octo/app"));
        assert_eq!(super::pull_repo("nope"), None);
    }
}
