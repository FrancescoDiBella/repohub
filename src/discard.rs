//! "Discard what you typed?": asked before Esc, a Cancel, opening another form, or moving to
//! another place throws away text typed and not sent: the New issue, branch or pull request
//! form, the release form, the title or description being edited, the comment being written
//! in a conversation or on a diff's lines ([`Draft`]). An empty one closes at once, as before.
//!
//! An action that would throw some away calls [`App::ask_first`] with the drafts it loses and
//! itself as what comes after. Nothing typed: it goes on at once. Otherwise the question shows
//! (floating over the page, so it's seen wherever the text is, a form of another repository
//! included) and the action stops; **Discard** throws the drafts away and runs it again, now
//! with nothing left to lose; **Keep editing** (or Esc) leaves everything as it was.
//!
//! ```text
//! (none) ──action, something typed──▶ asking ──Discard──▶ drafts dropped, action runs ──▶ (none)
//!    ▲                                   └──Keep editing / Esc──────────────────────────────┘
//!    └── action, nothing typed: runs at once
//! ```

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::state::{Compose, Draft, Load};
use crate::{App, icons, ui};

/// The question on screen: what **Discard** throws away, and what happens then.
pub struct Ask {
    pub drafts: Vec<Draft>,
    then: Box<dyn FnOnce(&mut App)>,
}

impl App {
    /// Whether `d` holds text typed and not sent (one on its way to GitHub isn't at risk).
    pub(crate) fn typed(&self, d: Draft) -> bool {
        let thread = self.thread.as_ref();
        match d {
            Draft::Form => self.form.as_ref().is_some_and(|f| {
                !f.submit.is_sending() && (!f.title.text.trim().is_empty() || (f.kind != Compose::Branch && !f.body.text.trim().is_empty()))
            }),
            Draft::Release => self.releases.form.as_ref().is_some_and(|f| !f.submit.is_sending() && f.typed()),
            Draft::Title => thread.is_some_and(|o| !o.edit.saving("title") && match (&o.edit.title, &o.load) {
                (Some(e), Load::Ready(t)) => e.text.trim() != t.title.trim(),
                (Some(e), _) => !e.text.trim().is_empty(),
                _ => false,
            }),
            Draft::Description => thread.is_some_and(|o| !o.edit.saving("body") && match (&o.edit.body, &o.load) {
                (Some(e), Load::Ready(t)) => e.text.trim_end() != t.entries.first().map(|x| x.body.trim_end()).unwrap_or(""),
                (Some(e), _) => !e.text.trim().is_empty(),
                _ => false,
            }),
            Draft::Reply => thread.is_some_and(|o| !o.sending && !o.reply.text.trim().is_empty()),
            Draft::LineComment => self.pull_review.as_ref().and_then(|r| r.draft.as_ref()).is_some_and(|d| !d.sending && !d.text.text.trim().is_empty()),
        }
    }

    /// What leaving the place on screen throws away: what's typed in the conversation (its
    /// title, description and reply) and on the diff's lines.
    pub(crate) fn typed_in_place(&self) -> Vec<Draft> {
        [Draft::Title, Draft::Description, Draft::Reply, Draft::LineComment].into_iter().filter(|d| self.typed(*d)).collect()
    }

    /// Before an action that throws away `drafts`: true when some hold typed text, and the
    /// question now asks first (`then`, the action itself, runs on **Discard**). False: nothing
    /// to lose, go on.
    pub(crate) fn ask_first(&mut self, drafts: &[Draft], then: impl FnOnce(&mut App) + 'static) -> bool {
        // While the question waits, what would throw text away waits for its answer too (a
        // second step of the same action included).
        if self.discard.is_some() {
            return true;
        }
        let typed: Vec<Draft> = drafts.iter().copied().filter(|d| self.typed(*d)).collect();
        if typed.is_empty() {
            return false;
        }
        crate::state::trace("ask", &"nothing", &typed);
        self.discard = Some(Ask { drafts: typed, then: Box::new(then) });
        true
    }

    /// Before leaving the place on screen (another page, section, conversation or run).
    pub(crate) fn ask_before_leaving(&mut self, then: impl FnOnce(&mut App) + 'static) -> bool {
        let lost = self.typed_in_place();
        self.ask_first(&lost, then)
    }

    /// The answer: **Discard** (`true`) throws the drafts away and does what was asked;
    /// **Keep editing** leaves everything as it was.
    pub(crate) fn answer_discard(&mut self, discard: bool) {
        let Some(ask) = self.discard.take() else { return };
        if !discard {
            return;
        }
        for d in &ask.drafts {
            self.drop_draft(*d);
        }
        (ask.then)(self);
    }

    fn drop_draft(&mut self, d: Draft) {
        match d {
            Draft::Form => self.close_form(),
            Draft::Release => self.close_release_form(),
            Draft::Title => if let Some(o) = self.thread.as_mut() { o.edit.title = None },
            Draft::Description => if let Some(o) = self.thread.as_mut() { o.edit.body = None },
            Draft::Reply => if let Some(o) = self.thread.as_mut() { o.reply.clear() },
            Draft::LineComment => self.close_line_draft(),
        }
    }

    /// The New issue, branch or pull request form closes (with its picker and picks).
    pub(crate) fn close_form(&mut self) {
        self.form = None;
        self.picker = None;
        self.issue_picks = Default::default();
    }

    /// The form's **Cancel**: closes it at once when it's empty, else asks first.
    pub(crate) fn cancel_form(&mut self) {
        if !self.ask_first(&[Draft::Form], |_| {}) {
            self.close_form();
        }
    }

    /// The release form's **Cancel** and close button.
    pub(crate) fn cancel_release_form(&mut self) {
        if !self.ask_first(&[Draft::Release], |_| {}) {
            self.close_release_form();
        }
    }

    /// **Cancel** on the title's or the description's editor.
    pub(crate) fn cancel_edit(&mut self, d: Draft) {
        if !self.ask_first(&[d], |_| {}) {
            self.drop_draft(d);
        }
    }

    /// The conversation's **Back** (and ⌘[): it closes, with its diff.
    pub(crate) fn close_thread(&mut self) {
        if self.ask_before_leaving(|s| s.close_thread()) {
            return;
        }
        self.close_pull_diff();
        self.thread = None;
    }

    /// The diff's close button (and **Files** pressed again).
    pub(crate) fn leave_pull_diff(&mut self) {
        if !self.ask_first(&[Draft::LineComment], |s| s.close_pull_diff()) {
            self.close_pull_diff();
        }
    }
}

impl App {
    /// For headless stills: `-- discard form` types a title in the New issue form of the
    /// repository on screen, `-- discard reply` a comment in the conversation on screen, and
    /// then asks as Cancel (or leaving the conversation) would.
    pub(crate) fn discard_still(&mut self, what: Option<String>) {
        match what.as_deref() {
            Some("form") => {
                let Some(i) = self.repo_index() else { return };
                self.open_form(i, Compose::Issue);
                if let Some(f) = self.form.as_mut() {
                    f.title = rune::TextEdit::new("The page stays blank on a slow connection");
                }
                self.cancel_form();
            }
            Some("reply") => {
                let Some(t) = self.thread.as_mut() else { return };
                t.reply = rune::TextEdit::new("Looks good. One question about the flag's default before I approve.");
                self.close_thread();
            }
            _ => {}
        }
    }
}

/// What a draft is, in a sentence that says what's lost.
fn lost(s: &App, d: Draft) -> String {
    let thread = s.thread.as_ref().map(|o| format!("{}#{}", o.repo, o.number)).unwrap_or_default();
    match d {
        Draft::Form => match s.form.as_ref() {
            Some(f) => format!("The new {} in {} hasn't been created.", match f.kind {
                Compose::Issue => "issue",
                Compose::Branch => "branch",
                Compose::Pull => "pull request",
            }, f.repo),
            None => String::new(),
        },
        Draft::Release => match s.releases.form.as_ref() {
            Some(f) if f.draft_id.is_some() => format!("Your changes to the draft release in {} haven't been saved.", f.repo),
            Some(f) => format!("The new release in {} hasn't been saved.", f.repo),
            None => String::new(),
        },
        Draft::Title => format!("Your new title for {thread} hasn't been saved."),
        Draft::Description => format!("Your new description for {thread} hasn't been saved."),
        Draft::Reply => format!("Your comment on {thread} hasn't been sent."),
        Draft::LineComment => match s.pull_review.as_ref().and_then(|r| r.draft.as_ref()) {
            Some(d) => format!("Your comment on {} hasn't been sent.", d.path),
            None => String::new(),
        },
    }
}

/// The question, floating at the bottom of the page while it waits for an answer.
pub fn question(s: &App, k: &Kit) -> Option<El> {
    let ask = s.discard.as_ref()?;
    let detail: Vec<String> = ask.drafts.iter().map(|d| lost(s, *d)).filter(|t| !t.is_empty()).collect();
    let detail = detail.join(" ");
    let tint = k.c.warning.icon;
    const QUESTION: &str = "Discard what you typed?";
    Some(col().key("discard-ask").role("alertdialog").label(format!("{QUESTION} {detail}")).w_full().max_w(560.0)
        .pl(14.0).pr(10.0).py(10.0).gap(10.0).radius(k.radius.lg)
        .fill(k.c.bg_base).border(1.0, tint.alpha(0.6))
        .child(row().w_full().gap(10.0).items_start()
            .child(ui::ico(&icons::PEN, 15.0, tint).mt(2.0))
            .child(col().gap(2.0).grow(1.0).shrink()
                .child(text(QUESTION).font(k.ty.small_medium).color(k.c.text_strong).wrap())
                .child(text(detail).font(k.ty.small).color(k.c.text_muted).wrap().key("discard-what"))))
        .child(row().w_full().gap(8.0).justify_end().flex_wrap()
            .child(Button::new("Keep editing").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.answer_discard(false)).build(k).key("discard-no"))
            .child(Button::new("Discard").variant(ButtonVariant::Danger).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.answer_discard(true)).build(k).key("discard-yes"))))
}
