//! Asking first: one question, one look, for what changes GitHub or deletes something on this
//! computer and can't be undone from RepoHub: merging a pull request, publishing a release,
//! deleting a drawing, removing a plugin or a coding assistant's settings… (CONTRIBUTING,
//! Guidelines, has the list). What can be put back offers **Undo** in its toast instead.
//!
//! The button asks ([`App::ask`] with the question's key). Where the button is, the page calls
//! [`Confirm::show`]: on a computer the question takes the button's place, inline; on a phone
//! it's a sheet at the bottom of the screen over the dimmed page (`show` leaves it in
//! `App::sheet` and `chrome::view` draws it, so it's seen wherever the button was). **Cancel**,
//! Esc, the back button or a tap beside the sheet answer no; moving to another section or page
//! forgets the question. "Discard what you typed?" (`discard`) is drawn by the same [`card`].
//!
//! ```text
//! (none) ──button──▶ asking(key) ──yes──▶ the action runs ──▶ (none)
//!    ▲                    └──Cancel / Esc / back / tap beside it / leaving──┘
//! ```

use std::rc::Rc;

use rune::{El, Icon, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::{App, ui};

/// What an answer does.
pub type Action = Rc<dyn Fn(&mut App)>;

/// What the yes button does to what's on screen: throws something away (red), or goes ahead
/// with a change (the main button's colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Danger,
    Go,
}

/// One question, as the page that asks it describes it.
pub struct Confirm {
    key: String,
    open: Option<bool>,
    icon: &'static Icon,
    question: String,
    detail: String,
    yes: String,
    no: String,
    tone: Tone,
    yes_key: Option<String>,
    no_key: Option<String>,
    on_yes: Action,
    on_no: Option<Action>,
    extra: Option<El>,
    disabled: bool,
}

impl Confirm {
    /// The question `key` (what [`App::ask`] was given; also the question's element key, and
    /// `{key}-yes` / `{key}-no` its buttons').
    pub fn new(key: impl Into<String>, icon: &'static Icon, question: impl Into<String>) -> Confirm {
        Confirm {
            key: key.into(),
            open: None,
            icon,
            question: question.into(),
            detail: String::new(),
            yes: "Yes".into(),
            no: "Cancel".into(),
            tone: Tone::Go,
            yes_key: None,
            no_key: None,
            on_yes: Rc::new(|_| {}),
            on_no: None,
            extra: None,
            disabled: false,
        }
    }

    /// What happens, in a sentence or two under the question.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// The yes button: its words, its colour, and what it does (after the question closes).
    pub fn yes(mut self, label: impl Into<String>, tone: Tone, on_yes: impl Fn(&mut App) + 'static) -> Self {
        self.yes = label.into();
        self.tone = tone;
        self.on_yes = Rc::new(on_yes);
        self
    }

    /// What answering no does, for a question whose state lives elsewhere (with [`Self::open`]);
    /// by default it forgets `App::asking`.
    pub fn on_no(mut self, on_no: impl Fn(&mut App) + 'static) -> Self {
        self.on_no = Some(Rc::new(on_no));
        self
    }

    /// Whether it's on screen, for a question whose state lives elsewhere (a release's
    /// publish question); by default, while `App::asking` is its key.
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// Other keys for its buttons (ones tests and flows already know).
    pub fn keys(mut self, yes: impl Into<String>, no: impl Into<String>) -> Self {
        self.yes_key = Some(yes.into());
        self.no_key = Some(no.into());
        self
    }

    /// A choice that goes with the answer, between the words and the buttons (how to merge).
    pub fn extra(mut self, e: El) -> Self {
        self.extra = Some(e);
        self
    }

    /// The yes button can't be pressed now (what's asked about can't go ahead yet).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Whether the question is open now.
    pub fn is_open(&self, s: &App) -> bool {
        self.open.unwrap_or_else(|| s.asking.as_deref() == Some(self.key.as_str()))
    }

    /// The question where it was asked: inline on a computer (`Some`, in place of the button);
    /// on a phone `None` (the page keeps its button) and the sheet goes to `App::sheet`. `None`
    /// too while it isn't asked.
    pub fn show(self, s: &App, k: &Kit) -> Option<El> {
        if !self.is_open(s) {
            return None;
        }
        let compact = s.compact;
        let on_no = self.on_no.clone().unwrap_or_else(|| Rc::new(|s: &mut App| s.asking = None));
        let key = self.key.clone();
        let on_yes = self.on_yes.clone();
        let yes_action: Action = Rc::new(move |s: &mut App| {
            if s.asking.as_deref() == Some(key.as_str()) {
                s.asking = None;
            }
            on_yes(s);
        });
        let yes_key = self.yes_key.clone().unwrap_or_else(|| format!("{}-yes", self.key));
        let no_key = self.no_key.clone().unwrap_or_else(|| format!("{}-no", self.key));
        let c = card(k, compact, Card {
            key: self.key,
            icon: self.icon,
            question: self.question,
            detail: self.detail,
            extra: self.extra,
            no: (self.no, no_key, on_no.clone()),
            yes: (self.yes, yes_key, self.tone, yes_action, self.disabled),
        });
        if compact {
            *s.sheet.borrow_mut() = Some(sheet(k, c, on_no));
            None
        } else {
            Some(c)
        }
    }
}

/// What a question card is made of (see [`card`]).
pub struct Card {
    pub key: String,
    pub icon: &'static Icon,
    pub question: String,
    pub detail: String,
    pub extra: Option<El>,
    /// The no button: words, key, action.
    pub no: (String, String, Action),
    /// The yes button: words, key, colour, action, whether it can't be pressed now.
    pub yes: (String, String, Tone, Action, bool),
}

/// The look every question shares: an icon, the question, what it means, a choice if any, and
/// the answers on the right (on a phone, side by side across the sheet and larger).
pub fn card(k: &Kit, compact: bool, c: Card) -> El {
    let tint = k.c.warning.icon;
    let label = if c.detail.is_empty() { c.question.clone() } else { format!("{} {}", c.question, c.detail) };
    let size = if compact { ControlSize::Regular } else { ControlSize::Small };
    let (no_label, no_key, on_no) = c.no;
    let (yes_label, yes_key, tone, on_yes, disabled) = c.yes;
    let no = Button::new(no_label).variant(ButtonVariant::Ghost).control_size(size)
        .on_press(move |s: &mut App| on_no(s)).build(k).key(no_key);
    let yes = Button::new(yes_label).variant(if tone == Tone::Danger { ButtonVariant::Danger } else { ButtonVariant::Submit }).control_size(size)
        .disabled(disabled).on_press(move |s: &mut App| on_yes(s)).build(k).key(yes_key);
    let mut words = col().gap(2.0).grow(1.0).shrink()
        .child(text(c.question).font(k.ty.small_medium).color(k.c.text_strong).wrap().key(format!("{}-question", c.key)));
    if !c.detail.is_empty() {
        words = words.child(text(c.detail).font(k.ty.small).color(k.c.text_muted).wrap().key(format!("{}-what", c.key)));
    }
    let answers = if compact {
        row().w_full().gap(8.0).child(no.grow(1.0)).child(yes.grow(1.0))
    } else {
        row().w_full().gap(8.0).justify_end().flex_wrap().child(no).child(yes)
    };
    let mut out = col().key(c.key).role("alertdialog").label(label).w_full()
        .pl(14.0).pr(10.0).py(10.0).gap(10.0).radius(k.radius.lg)
        .fill(k.c.bg_base).border(1.0, tint.alpha(0.6))
        .child(row().w_full().gap(10.0).items_start()
            .child(ui::ico(c.icon, 15.0, tint).mt(2.0))
            .child(words));
    if let Some(e) = c.extra {
        out = out.child(e);
    }
    out.child(answers)
}

/// A card as the phone's sheet: at the bottom of the page area, over the dimmed page; a tap
/// beside it or Esc answers no, and nothing behind it can be pressed meanwhile.
pub fn sheet(k: &Kit, card: El, on_no: Action) -> El {
    let dismiss = on_no;
    col().key("sheet-layer").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).justify_end().padding(10.0)
        // The kit's scrim hardly shows on a dark page: darker there.
        .child(el().key("sheet-scrim").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(if k.dark { rune::Color::BLACK.alpha(0.55) } else { k.c.scrim }).a11y_hidden())
        .child(col().key("sheet").w_full().child(card)
            .modal(true, move |s: &mut App| dismiss(s)))
}

impl App {
    /// Asks question `key` (see [`Confirm`]); another question open closes.
    pub(crate) fn ask(&mut self, key: impl Into<String>) {
        let key = key.into();
        crate::state::trace("ask", &self.asking, &key);
        self.asking = Some(key);
    }
}

/// Settings → Account's **Sign out**: the token saved here is deleted (to connect again, it's
/// typed again), with the saved copy of the lists.
pub fn sign_out_question(s: &App, k: &Kit) -> Option<El> {
    let place = if s.cfg.device { "on this device" } else { "on this computer" };
    Confirm::new("ask-sign-out", &crate::icons::LOCK, "Sign out?")
        .detail(format!("The account and the token saved {place} are deleted, and so is the saved copy of your lists. To connect again you'll need the token."))
        .yes("Sign out", Tone::Danger, |s: &mut App| s.save_account(true))
        .show(s, k)
}

/// For headless stills: `-- ask merge` asks to merge the first open pull request (from its diff
/// when one is open, `-- diff 14`); `-- ask sign-out` (or `push-default`, `abort-merge`,
/// `clear-saved-data`, `delete-note-Name`, `remove-plugin-id`…) asks that question.
pub(crate) fn still(s: &mut App, what: Option<String>) {
    match what.as_deref() {
        None => {}
        Some("merge") => {
            let open = match &s.pull_diff_of {
                Some((repo, number)) => Some((repo.clone(), *number)),
                None => s.data.pulls.iter().find(|p| p.state == crate::model::PullState::Open).map(|p| (p.repo.clone(), p.number)),
            };
            if let Some((repo, number)) = open {
                s.ask(crate::actions::merge_ask(&repo, number));
            }
        }
        Some(other) => s.ask(format!("ask-{other}")),
    }
}
