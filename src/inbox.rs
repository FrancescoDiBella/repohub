//! Notifications: what GitHub wants you to look at (a review asked of you, a mention, a failed
//! run), unread first. Clicking one opens its conversation and marks it read.
//!
//! GitHub only gives notifications to classic tokens (scope `notifications` or `repo`); a
//! fine-grained token gets a refusal, which the page explains.
use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::pages::heading;
use crate::state::Load;
use crate::ui::{self, panel, rule};
use crate::{App, icons, model};

/// How many notifications are read: GitHub's newest 50 (`per_page=50`).
pub const READ_AT_MOST: usize = 50;

/// "3 unread", or "50 or more unread" when GitHub may have more than were read.
pub fn unread_words(n: usize) -> String {
    if n >= READ_AT_MOST { format!("{n} or more unread") } else { format!("{n} unread") }
}

/// What a notification is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Issue,
    Pull,
    /// A release, a discussion, a failed run…: it opens on GitHub.
    Other,
}

/// One thing GitHub notified you about.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// GitHub's id of the thread, for marking it read.
    pub id: String,
    pub repo: String,
    pub title: String,
    pub subject: Subject,
    /// The issue's or pull request's number, when it has one.
    pub number: Option<u32>,
    /// Why you got it, in GitHub's word: `review_requested`, `mention`, `assign`…
    pub reason: String,
    pub updated_at: i64,
    /// The page to open for a subject the app has no page for.
    pub url: String,
}

impl Notification {
    /// Why you got it, in plain words.
    pub fn why(&self) -> &'static str {
        match self.reason.as_str() {
            "review_requested" => "Your review was requested",
            "mention" | "team_mention" => "You were mentioned",
            "assign" => "Assigned to you",
            "author" => "On something you opened",
            "comment" => "On something you commented on",
            "ci_activity" => "A workflow run finished",
            "state_change" => "You changed its state",
            "subscribed" | "manual" => "You're watching this",
            "security_alert" => "Security alert",
            "invitation" => "Invitation",
            _ => "Notification",
        }
    }
}

/// The message for a notifications request GitHub refused, or the error as it is.
pub fn explain(error: &str) -> String {
    if error.contains("403") || error.contains("404") {
        "GitHub didn't share your notifications with this token. Fine-grained tokens can't read them: use a classic token with the notifications (or repo) scope.".into()
    } else {
        error.to_owned()
    }
}

fn line(s: &App, k: &Kit, n: &Notification) -> El {
    let icon = match n.subject {
        Subject::Pull => &icons::PULL,
        Subject::Issue => &icons::ISSUE,
        Subject::Other => &icons::BELL,
    };
    let id = n.id.clone();
    let busy = s.inbox_busy.contains(&n.id);
    let read = Button::new(if busy { "Marking…" } else { "Mark as read" }).icon(&icons::CHECK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(busy).on_press(move |s: &mut App| s.mark_read(&id)).build(k).key(format!("notif-read-{}", n.id));
    let open = n.clone();
    let body = row().grow(1.0).shrink().px(16.0).py(11.0).gap(12.0).center_y().hover(k.c.hover).press(k.c.pressed)
        .role("button").label(format!("{}: {}", n.why(), n.title)).key(format!("notif-{}", n.id))
        .on_press_tx(move |s: &mut App, tx| s.open_notification(&open, tx))
        .child(ui::ico(icon, 16.0, k.c.icon_muted))
        .child(col().grow(1.0).shrink().gap(3.0)
            .child(text(n.title.clone()).font(k.ty.ui_medium).color(k.c.text_base).wrap())
            .child(text(match n.number {
                Some(num) => format!("{} · {} #{num} · {}", n.why(), n.repo, model::fmt_ago(n.updated_at, s.now)),
                None => format!("{} · {} · {}", n.why(), n.repo, model::fmt_ago(n.updated_at, s.now)),
            }).font(k.ty.text_small).color(k.c.text_muted).wrap()));
    crate::keyboard::mark(k, row().w_full().center_y().child(body).child(read.mr(12.0)), s.marked(&format!("notif-{}", n.id)))
}

pub fn page(s: &App, k: &Kit) -> El {
    let refresh = Button::new("Refresh").icon(&icons::REFRESH).control_size(ControlSize::Small)
        .disabled(matches!(s.inbox, Some(Load::Loading)))
        .on_press(|s: &mut App| s.load_inbox()).build(k).key("inbox-refresh");
    let note = |msg: String, key: &str| panel(k).padding(28.0).items_center().key(key.to_owned()).child(text(msg).font(k.ty.ui).color(k.c.text_muted).wrap());
    let (sub, body) = match &s.inbox {
        _ if s.data.sample || !s.cfg.live() => ("What GitHub wants you to look at.".to_owned(), note("Connect your GitHub account in Settings to see your notifications.".into(), "inbox-sample")),
        _ if s.cfg.token.is_none() => ("What GitHub wants you to look at.".to_owned(), note("Notifications need a token: a classic token with the notifications (or repo) scope.".into(), "inbox-no-token")),
        None | Some(Load::Loading) => ("Reading your notifications…".to_owned(), ui::skeleton_list(k, 4, "inbox-loading")),
        Some(Load::Failed(e)) => ("Couldn't read your notifications.".to_owned(),
            panel(k).padding(28.0).items_center().gap(10.0).role("alert").key("inbox-failed")
                .child(text(e.clone()).font(k.ty.small).color(k.c.text_base).wrap())
                .child(Button::new("Retry").icon(&icons::REFRESH).control_size(ControlSize::Small).on_press(|s: &mut App| s.load_inbox()).build(k).key("inbox-retry"))),
        Some(Load::Ready(list)) if list.is_empty() => ("Nothing unread.".to_owned(), note("You're all caught up.".into(), "inbox-empty")),
        Some(Load::Ready(list)) => {
            // Grouped by repository, in the order the newest notification puts them.
            let mut repos: Vec<&str> = Vec::new();
            for n in list {
                if !repos.iter().any(|r| r.eq_ignore_ascii_case(&n.repo)) {
                    repos.push(&n.repo);
                }
            }
            let mut groups = col().w_full().gap(12.0);
            for repo in repos {
                let mut p = panel(k).child(row().w_full().px(16.0).py(10.0).gap(8.0).center_y()
                    .child(ui::ico(&icons::REPO, 14.0, k.c.icon_muted))
                    .child(text(repo.to_owned()).font(k.ty.small_medium).color(k.c.text_strong))
                    .child(el().grow(1.0)));
                for n in list.iter().filter(|n| n.repo.eq_ignore_ascii_case(repo)) {
                    p = p.child(rule(k)).child(line(s, k, n));
                }
                groups = groups.child(p);
            }
            let how_many = if list.len() >= READ_AT_MOST {
                format!("The {} newest unread: GitHub may have more, all of them are on GitHub.", list.len())
            } else {
                format!("{}.", unread_words(list.len()))
            };
            (format!("{how_many} Click one to open it; it's marked as read."), groups)
        }
    };
    col().w_full().gap(16.0).child(heading(k, "Notifications", &sub, Some(refresh))).child(body)
}
