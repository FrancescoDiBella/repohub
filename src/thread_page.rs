//! The conversation of an issue or a pull request: who wrote what, and a box to reply.
//!
//! It takes the page's place (see `App::open_thread`); **Back** returns to the list it came from.
use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextArea};

use crate::state::Load;
use crate::thread::{Check, CheckState, Entry, Open, Standing, Thread, Verdict};
use crate::ui::{self, panel, rule};
use crate::{App, actions, github, icons, model, pages};

fn standing_pill(k: &Kit, st: Standing) -> El {
    let ink = match st {
        Standing::Open => k.c.success.icon,
        Standing::Draft => k.c.text_muted,
        Standing::Closed => k.c.danger.icon,
        Standing::Merged => k.c.text_accent,
    };
    ui::pill(k, st.label(), ink)
}

/// A pull request's checks on its last commit: a summary, then one row per check, and when
/// GitHub has more than were read, how many and where the rest are.
fn checks(k: &Kit, list: &[Check], total: usize, url: &str) -> El {
    let mut p = panel(k).key("thread-checks")
        .child(row().w_full().px(16.0).py(10.0).gap(10.0).center_y().flex_wrap()
            .child(text("Checks").font(k.ty.small_medium).color(k.c.text_strong))
            .child(text(crate::thread::checks_summary(list)).font(k.ty.small).color(k.c.text_muted).key("thread-checks-summary")));
    if total > list.len() {
        let mut more = row().w_full().px(16.0).pb(10.0).gap(8.0).center_y().flex_wrap()
            .child(text(format!("Showing the first {} of {total} checks; the summary counts those only.", list.len())).font(k.ty.small).color(k.c.text_muted).wrap().shrink().key("thread-checks-more"));
        if !url.is_empty() {
            more = more.child(pages::open_button(k, "All checks on GitHub", &format!("{url}/checks"), "thread-checks-all"));
        }
        p = p.child(more);
    }
    for (n, c) in list.iter().enumerate() {
        let (icon, ink) = match c.state {
            CheckState::Passed => (&icons::CHECK, k.c.success.icon),
            CheckState::Failed => (&icons::CROSS, k.c.danger.icon),
            CheckState::Running => (&icons::DOT, k.c.warning.icon),
            CheckState::Other => (&icons::DOT, k.c.text_faint),
        };
        // The word says it; the colour and the mark only repeat it.
        let mut line = row().w_full().px(16.0).py(8.0).gap(10.0).center_y().key(format!("thread-check-{n}"))
            .child(ui::ico(icon, 14.0, ink).label(format!("Check {}", c.state.label().to_lowercase())))
            .child(text(c.name.clone()).font(k.ty.small).color(k.c.text_base).grow(1.0).wrap())
            .child(ui::pill(k, c.state.label(), ink));
        if !c.url.is_empty() {
            line = line.child(pages::open_button(k, "Details on GitHub", &c.url, &format!("thread-check-open-{n}")));
        }
        p = p.child(rule(k)).child(line);
    }
    p
}

/// One description, comment or review; `tool` goes at the end of its head (Edit on the
/// description).
fn entry(k: &Kit, e: &Entry, n: usize, now: i64, tool: Option<El>) -> El {
    let mut head = row().w_full().px(16.0).py(10.0).gap(10.0).center_y().flex_wrap()
        .child(ui::user(k, &e.author, 22.0))
        .child(text(e.author.clone()).font(k.ty.small_medium).color(k.c.text_strong));
    if let Some(v) = e.verdict {
        let ink = match v {
            Verdict::Approved => k.c.success.icon,
            Verdict::ChangesRequested => k.c.danger.icon,
            Verdict::Commented => k.c.text_muted,
        };
        // The words say it; the colour only repeats it.
        head = head.child(ui::pill(k, v.label(), ink));
    }
    head = head.child(el().grow(1.0))
        .child(text(model::fmt_ago(e.at, now)).font(k.ty.text_small).color(k.c.text_faint))
        .children(tool);
    let body = if e.body.trim().is_empty() {
        text(if n == 0 { "No description." } else { "No comment." }).font(k.ty.small).color(k.c.text_faint)
    } else {
        pages::markdown_view(k, &e.body, &format!("thread-code-{n}"))
    };
    panel(k).key(format!("thread-entry-{n}"))
        .child(head)
        .child(rule(k))
        .child(col().w_full().padding(16.0).child(body))
}

/// The box to reply, and what can be done with what's typed.
fn reply(s: &App, k: &Kit, o: &Open, t: &Thread) -> El {
    let blocked = actions::blocked(s);
    let empty = o.reply.text.trim().is_empty();
    let off = o.sending || blocked.is_some();
    let key = actions::busy_key(&o.repo, o.number);
    let busy = s.busy.contains(&key);
    let mut buttons = row().w_full().gap(8.0).center_y().flex_wrap()
        .child(Button::new(if o.sending { "Sending…" } else { "Comment" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .disabled(off || empty).on_press(|s: &mut App| s.send_reply(None)).build(k).key("thread-comment"));
    if o.pull && t.standing.is_open() {
        buttons = buttons
            .child(Button::new("Approve").icon(&icons::CHECK).control_size(ControlSize::Small)
                .help("Approve these changes; what you typed goes with it")
                .disabled(off).on_press(|s: &mut App| s.send_reply(Some(Verdict::Approved))).build(k).key("thread-approve"))
            .child(Button::new("Request changes").control_size(ControlSize::Small)
                .help("Say what must change before this is merged (type it first)")
                .disabled(off || empty).on_press(|s: &mut App| s.send_reply(Some(Verdict::ChangesRequested))).build(k).key("thread-request-changes"));
    }
    if o.pull && matches!(t.standing, Standing::Open | Standing::Draft | Standing::Closed) {
        let (repo, number) = (o.repo.clone(), o.number);
        let open = t.standing == Standing::Closed;
        buttons = buttons.child(el().grow(1.0));
        buttons = buttons.child(
            Button::new(match (open, busy) {
                (false, false) => "Close pull request",
                (false, true) => "Closing…",
                (true, false) => "Reopen pull request",
                (true, true) => "Reopening…",
            }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .help(if open { "Open it again for review" } else { "Close it without merging" })
                .disabled(busy || blocked.is_some())
                .on_press(move |s: &mut App| s.write(&repo, github::Write::SetPullOpen { number, open })).build(k)
                .key(if open { "thread-reopen-pull" } else { "thread-close-pull" }));
    }
    if !o.pull {
        let (repo, number) = (o.repo.clone(), o.number);
        buttons = buttons.child(el().grow(1.0));
        buttons = buttons.child(if t.standing.is_open() {
            Button::new(if busy { "Closing…" } else { "Close issue" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .disabled(busy || blocked.is_some())
                .on_press(move |s: &mut App| s.write(&repo, github::Write::CloseIssue(number))).build(k).key("thread-close-issue")
        } else {
            Button::new(if busy { "Reopening…" } else { "Reopen issue" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .disabled(busy || blocked.is_some())
                .on_press(move |s: &mut App| s.write(&repo, github::Write::ReopenIssue(number))).build(k).key("thread-reopen-issue")
        });
    }
    let mut p = panel(k).key("thread-reply").padding(16.0).gap(10.0)
        .child(TextArea::new(&o.reply).placeholder("Write a comment (Markdown)").label("Your comment").auto_grow(3, 12)
            .on_edit(|s: &mut App| &mut s.thread.as_mut().expect("conversation open").reply).build(k).key("thread-reply-text").w_full())
        .child(buttons);
    if let Some(why) = blocked {
        p = p.child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap().key("thread-blocked"));
    }
    p
}

pub fn view(s: &App, k: &Kit, o: &Open) -> El {
    let what = if o.pull { "pull request" } else { "issue" };
    let back = Button::new("Back").icon(&icons::BACK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.close_thread()).build(k).key("thread-back");
    let mut top = row().w_full().gap(8.0).center_y().child(back).child(el().grow(1.0));
    let mut page = col().w_full().gap(16.0).key("thread");
    let t = match &o.load {
        Load::Loading => {
            return page.child(top)
                .child(text(format!("{} #{}", o.repo, o.number)).font(k.ty.small).color(k.c.text_muted))
                .child(panel(k).padding(28.0).items_center().role("status").key("thread-loading")
                    .child(text(format!("Reading the {what}…")).font(k.ty.ui).color(k.c.text_muted)));
        }
        Load::Failed(e) => {
            return page.child(top)
                .child(panel(k).padding(28.0).items_center().gap(10.0).role("alert").key("thread-failed")
                    .child(text(e.clone()).font(k.ty.small).color(k.c.text_base).wrap())
                    .child(Button::new("Retry").icon(&icons::REFRESH).control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.load_thread()).build(k).key("thread-retry")));
        }
        Load::Ready(t) => t,
    };
    top = top.child(Button::new("Refresh").icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.load_thread()).build(k).key("thread-refresh"));
    if !t.url.is_empty() {
        top = top.child(pages::open_button(k, "Open on GitHub", &t.url, "thread-github"));
    }
    let mut meta = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(standing_pill(k, t.standing))
        .child(text(format!("{} #{} · opened by {}", o.repo, o.number, t.author)).font(k.ty.small).color(k.c.text_muted));
    if let Some((head, base)) = &t.branches {
        meta = meta.child(ui::stat(k, &icons::BRANCH, format!("{head} → {base}")));
    }
    // The labels are in the details block, with the assignees, milestone and reviewers.
    page = page.child(top)
        .child(col().w_full().gap(8.0)
            .child(crate::triage_page::title(s, k, o, t))
            .child(meta));
    let mut body = col().gap(16.0);
    if let Some(list) = &t.checks {
        body = body.child(checks(k, list, t.checks_total, &t.url));
    }
    // A pull request's changes: the diff panel the lists use, opened here.
    let mut diff_open = false;
    if o.pull {
        let active = s.pull_diff_of.as_ref().is_some_and(|(full, n)| *n == o.number && model::same_repo(full, &o.repo));
        diff_open = active;
        let (repo, number) = (o.repo.clone(), o.number);
        let files = Button::new(if active { "Hide the changes" } else { "See the changes" }).icon(&icons::FILE)
            .variant(if active { ButtonVariant::Contrast } else { ButtonVariant::Ghost }).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| if active { s.leave_pull_diff() } else { s.open_pull_diff(repo.clone(), number) })
            .build(k).key("thread-files");
        body = body.child(row().w_full().child(files));
        if active {
            body = body.children(crate::changes_page::pull_panel_for(s, k, None));
        }
    }
    for (n, e) in t.entries.iter().enumerate() {
        body = body.child(match (n, &o.edit.body) {
            (0, Some(editing)) => crate::triage_page::body_editor(s, k, o, e, editing),
            (0, None) => entry(k, e, n, s.now, crate::triage_page::edit_body_button(s, k, o, t)),
            _ => entry(k, e, n, s.now, None),
        });
    }
    if t.more {
        body = body.child(text("This conversation is longer than what's shown here: the rest is on GitHub.").font(k.ty.small).color(k.c.text_muted).wrap().key("thread-more"));
    }
    body = body.child(reply(s, k, o, t));
    // The details beside the conversation on a wide page (not while the diff wants the width),
    // above it on a phone or a narrow window.
    let details = crate::triage_page::details(s, k, o, t);
    if crate::triage_page::side(s) && !diff_open {
        page.child(row().w_full().gap(16.0).items_start()
            .child(body.grow(1.0).shrink().min_w(0.0))
            .child(details.w(crate::triage_page::SIDE_W)))
    } else {
        page.child(details).child(body.w_full())
    }
}
