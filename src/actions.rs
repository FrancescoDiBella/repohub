//! Working on GitHub without leaving the app: the branch picker, the forms that create issues,
//! branches and pull requests, and the Close and Merge buttons on rows.
//!
//! Every write goes through [`App::write`], which runs one [`github::Write`] in the background and
//! applies its [`github::Done`] to the screen at once (a new issue shows before GitHub's search
//! catches up). The forms follow [`Submit`]; the picker fills a [`BranchSlot`].
use rune::{El, TextEdit, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextArea, TextInput};

use crate::github::{self, NewPull, Write};
use crate::model::{Issue, Pull, PullState};
use crate::state::{BranchSlot, Compose, Load, Submit};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons, lists};

/// An open form, for one repository.
pub struct Form {
    pub repo: String,
    pub kind: Compose,
    /// The issue's or pull request's title, or the new branch's name.
    pub title: TextEdit,
    pub body: TextEdit,
    /// Branch: where it starts. Pull request: its branch (`from`) and the one it goes into (`into`).
    pub from: String,
    pub into: String,
    pub draft: bool,
    pub submit: Submit,
}

impl Form {
    /// What the form would send, or why it can't yet.
    pub fn write(&self) -> Result<Write, &'static str> {
        let title = self.title.text.trim().to_owned();
        let body = self.body.text.trim().to_owned();
        match self.kind {
            Compose::Issue if title.is_empty() => Err("Give the issue a title."),
            Compose::Issue => Ok(Write::Issue { title, body }),
            Compose::Branch if !github::valid_branch(&title) => Err("Branch names can't have spaces or any of ~ ^ : ? * [ \\, start with - or /, or contain .."),
            Compose::Branch => Ok(Write::Branch { name: title, from: self.from.clone() }),
            Compose::Pull if self.from == self.into => Err("Pick a branch that differs from the one it goes into."),
            Compose::Pull if title.is_empty() => Err("Give the pull request a title."),
            Compose::Pull => Ok(Write::Pull(NewPull { title, body, head: self.from.clone(), base: self.into.clone(), draft: self.draft })),
        }
    }
}

/// Why nothing can be changed on GitHub right now, if so.
pub fn blocked(s: &App) -> Option<&'static str> {
    if s.data.sample || !s.cfg.live() {
        Some("This is sample data. Connect your GitHub account in Settings to make changes.")
    } else if s.cfg.token.is_none() {
        Some("Changes need a token with write access. Add one in Settings (desktop) or GITHUB_TOKEN in .env (Docker).")
    } else {
        None
    }
}

/// A key for the row a write is running on: `owner/repo#12`.
pub fn busy_key(repo: &str, number: u32) -> String {
    format!("{}#{number}", repo.to_ascii_lowercase())
}

// ------------------------------------------------------------------ branch picker

/// A button showing a branch; it opens the picker for `slot`.
pub fn branch_button(s: &App, k: &Kit, i: usize, slot: BranchSlot, branch: &str, key: &str) -> El {
    let open = s.picker == Some(slot);
    Button::new(format!("{branch}  ▾")).icon(&icons::BRANCH).control_size(ControlSize::Small)
        .variant(if open { ButtonVariant::Contrast } else { ButtonVariant::Neutral })
        .on_press(move |s: &mut App| {
            s.picker = if s.picker == Some(slot) { None } else { Some(slot) };
            s.branch_filter.clear();
            s.load_branches(i);
        })
        .build(k).key(key.to_owned())
}

/// The open picker's list, when it fills `slot`: a filter and the matching branches.
pub fn branch_panel(s: &App, k: &Kit, i: usize, slot: BranchSlot, current: &str) -> El {
    if s.picker != Some(slot) {
        return el();
    }
    let Some(r) = s.data.repos.get(i) else { return el() };
    let q = s.branch_filter.text.trim().to_lowercase();
    let filter = TextInput::new(&s.branch_filter).placeholder("Filter branches").leading_icon(k.glyph.search)
        .on_edit(|s: &mut App| &mut s.branch_filter).build(k).key("branch-filter").w_full();
    let mut list = col().w_full().gap(0.0);
    match s.branches.get(&r.full_name) {
        None | Some(Load::Loading) => list = list.child(text("Loading branches…").font(k.ty.small).color(k.c.text_muted).padding(10.0)),
        Some(Load::Failed(e)) => list = list.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap().padding(10.0)),
        Some(Load::Ready(names)) => {
            let shown: Vec<&String> = names.iter().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).take(30).collect();
            if shown.is_empty() {
                list = list.child(text(if names.is_empty() { "This repository has no branches yet." } else { "No branch matches." })
                    .font(k.ty.small).color(k.c.text_muted).padding(10.0));
            }
            for name in shown {
                let n = name.clone();
                let on = name == current;
                let mut line = row().w_full().px(10.0).py(7.0).gap(8.0).center_y().radius(k.radius.md).hover(k.c.hover).press(k.c.pressed)
                    .role("option").label(name.clone()).key(format!("pick-{name}"))
                    .on_press(move |s: &mut App| s.pick_branch_for(i, slot, n.clone()))
                    .child(ui::ico(if on { &icons::CHECK } else { &icons::BRANCH }, 14.0, if on { k.c.success.icon } else { k.c.icon_muted }))
                    .child(text(name.clone()).font(k.ty.mono).color(k.c.text_base).grow(1.0).shrink().max_lines(1));
                if *name == r.branch {
                    line = line.child(ui::pill(k, "default", k.c.text_muted));
                }
                list = list.child(line);
            }
        }
    }
    panel(k).padding(8.0).gap(6.0).max_w(420.0).key("branch-panel")
        .child(filter)
        .child(list)
}

// ------------------------------------------------------------------ forms

/// The open form for repository `i`, above its section.
pub fn form_panel(s: &App, k: &Kit, i: usize) -> El {
    let Some(r) = s.data.repos.get(i) else { return el() };
    let Some(f) = s.form.as_ref().filter(|f| f.repo == r.full_name) else { return el() };
    let title = match f.kind {
        Compose::Issue => "New issue",
        Compose::Branch => "New branch",
        Compose::Pull => "New pull request",
    };
    let sending = f.submit.is_sending();
    let check = f.write();
    let blocked = blocked(s);
    let cancel = Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| { s.form = None; s.picker = None; }).build(k).key("form-cancel");
    let submit_label = match (f.kind, sending) {
        (_, true) => "Sending…",
        (Compose::Issue, _) => "Create issue",
        (Compose::Branch, _) => "Create branch",
        (Compose::Pull, _) if f.draft => "Open draft pull request",
        (Compose::Pull, _) => "Open pull request",
    };
    let submit = Button::new(submit_label).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .disabled(sending || check.is_err() || blocked.is_some())
        .on_press(move |s: &mut App| s.submit_form(i)).build(k).key("form-submit");
    let mut body = col().w_full().padding(16.0).gap(12.0);
    match f.kind {
        Compose::Branch => {
            body = body
                .child(TextInput::new(&f.title).placeholder("feature/my-change").label("Branch name")
                    .on_edit(|s: &mut App| &mut s.form.as_mut().expect("form open").title)
                    .on_submit(move |s: &mut App, _| s.submit_form(i)).build(k).key("form-title").w_full())
                .child(row().gap(8.0).center_y().flex_wrap()
                    .child(text("Starts from").font(k.ty.small).color(k.c.text_muted))
                    .child(branch_button(s, k, i, BranchSlot::From, &f.from, "form-from")))
                .child(branch_panel(s, k, i, BranchSlot::From, &f.from));
        }
        Compose::Issue | Compose::Pull => {
            if f.kind == Compose::Pull {
                body = body
                    .child(row().gap(8.0).center_y().flex_wrap()
                        .child(text("Merge").font(k.ty.small).color(k.c.text_muted))
                        .child(branch_button(s, k, i, BranchSlot::Head, &f.from, "form-head"))
                        .child(text("into").font(k.ty.small).color(k.c.text_muted))
                        .child(branch_button(s, k, i, BranchSlot::Base, &f.into, "form-base")))
                    .child(branch_panel(s, k, i, BranchSlot::Head, &f.from))
                    .child(branch_panel(s, k, i, BranchSlot::Base, &f.into));
            }
            body = body
                .child(TextInput::new(&f.title).placeholder("Title").label("Title")
                    .on_edit(|s: &mut App| &mut s.form.as_mut().expect("form open").title).build(k).key("form-title").w_full())
                .child(TextArea::new(&f.body).placeholder("Description (Markdown)").label("Description").auto_grow(4, 12)
                    .on_edit(|s: &mut App| &mut s.form.as_mut().expect("form open").body).build(k).key("form-body").w_full());
            if f.kind == Compose::Pull {
                let draft = f.draft;
                body = body.child(row().gap(10.0).center_y()
                    .child(rune_kit::Switch::new(draft).on_toggle(|s: &mut App| if let Some(f) = s.form.as_mut() { f.draft = !f.draft; }).build(k).key("form-draft"))
                    .child(text("Draft: not ready for review yet").font(k.ty.small).color(k.c.text_base)));
            }
        }
    }
    // Why it can't be sent: the account first, then the form's own rule (once something is typed).
    let typed = !f.title.text.trim().is_empty() || f.kind == Compose::Pull;
    let note = match (&f.submit, blocked, &check) {
        (Submit::Failed(e), _, _) => Some((e.clone(), k.c.danger.icon)),
        (_, Some(b), _) => Some((b.to_owned(), k.c.warning.icon)),
        (_, None, Err(e)) if typed => Some(((*e).to_owned(), k.c.text_muted)),
        _ => None,
    };
    if let Some((msg, ink)) = note {
        body = body.child(text(msg).font(k.ty.small).color(ink).wrap().key("form-note"));
    }
    panel(k).key("form").border(1.0, k.c.border_strong)
        .child(panel_head(k, title, Some(ui::ico(match f.kind {
            Compose::Issue => &icons::ISSUE,
            Compose::Branch => &icons::BRANCH,
            Compose::Pull => &icons::PULL,
        }, 16.0, k.c.icon_muted))))
        .child(rule(k))
        .child(body.child(row().w_full().gap(8.0).justify_end().child(cancel).child(submit)))
}

/// A button that opens form `kind` for repository `i`.
pub fn open_form_button(k: &Kit, i: usize, kind: Compose) -> El {
    let (label, icon, key) = match kind {
        Compose::Issue => ("New issue", &icons::ISSUE, "new-issue"),
        Compose::Branch => ("New branch", &icons::BRANCH, "new-branch"),
        Compose::Pull => ("New pull request", &icons::PULL, "new-pull"),
    };
    Button::new(label).icon(icon).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.open_form(i, kind)).build(k).key(key)
}

// ------------------------------------------------------------------ rows

/// An issue row with Close.
pub fn issue_line(s: &App, k: &Kit, x: &Issue) -> El {
    let key = busy_key(&x.repo, x.number);
    let busy = s.busy.contains(&key);
    let (repo, number) = (x.repo.clone(), x.number);
    let close = Button::new(if busy { "Closing…" } else { "Close" }).icon(&icons::CHECK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(busy || blocked(s).is_some())
        .on_press(move |s: &mut App| s.write(&repo, github::Write::CloseIssue(number)))
        .build(k).key(format!("close-{key}"));
    row().w_full().center_y().child(lists::issue_row(k, x, s.now).grow(1.0).shrink()).child(close.mr(12.0))
}

/// A pull request row with Merge (asks first) on open, non-draft pull requests.
pub fn pull_line(s: &App, k: &Kit, p: &Pull, compact: bool) -> El {
    let key = busy_key(&p.repo, p.number);
    let line = lists::pull_row(k, p, s.now, compact).grow(1.0).shrink();
    let files = crate::changes_page::files_button(k, p);
    if p.state != PullState::Open {
        return row().w_full().center_y().child(line).children(files.map(|f| f.mr(12.0)));
    }
    let busy = s.busy.contains(&key);
    let asking = s.confirm_merge.as_deref() == Some(key.as_str());
    let (repo, number) = (p.repo.clone(), p.number);
    let mut tools = row().gap(6.0).center_y().mr(12.0).children(files);
    if asking && !busy {
        let r2 = repo.clone();
        tools = tools
            .child(text(format!("Merge #{number}?")).font(k.ty.small_medium).color(k.c.text_base))
            .child(Button::new("Merge").variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.write(&repo, github::Write::Merge { number, method: "merge" })).build(k).key(format!("merge-yes-{key}")))
            .child(Button::new("Squash").control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.write(&r2, github::Write::Merge { number, method: "squash" })).build(k).key(format!("squash-{key}")))
            .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.confirm_merge = None).build(k).key(format!("merge-no-{key}")));
    } else {
        let k2 = key.clone();
        tools = tools.child(Button::new(if busy { "Merging…" } else { "Merge" }).icon(&icons::MERGE).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .disabled(busy || blocked(s).is_some())
            .on_press(move |s: &mut App| s.confirm_merge = Some(k2.clone())).build(k).key(format!("merge-{key}")));
    }
    row().w_full().center_y().child(line).child(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(kind: Compose, title: &str, from: &str, into: &str) -> Form {
        Form { repo: "o/r".into(), kind, title: TextEdit::new(title), body: TextEdit::new(""), from: from.into(), into: into.into(), draft: false, submit: Submit::Editing }
    }

    #[test]
    fn forms_say_what_is_missing() {
        assert!(form(Compose::Issue, " ", "", "").write().is_err());
        assert!(matches!(form(Compose::Issue, "Bug", "", "").write(), Ok(Write::Issue { .. })));
        assert!(form(Compose::Branch, "bad name", "main", "").write().is_err());
        assert!(matches!(form(Compose::Branch, "feature/x", "main", "").write(), Ok(Write::Branch { ref name, ref from }) if name == "feature/x" && from == "main"));
        assert!(form(Compose::Pull, "Title", "main", "main").write().is_err(), "a branch can't go into itself");
        assert!(matches!(form(Compose::Pull, "Title", "feature/x", "main").write(), Ok(Write::Pull(p)) if p.head == "feature/x" && p.base == "main"));
        assert_eq!(busy_key("Octo/Repo", 3), "octo/repo#3");
    }
}
