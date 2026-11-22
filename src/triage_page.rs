//! Triage on screen: the details block of a conversation (labels, assignees, milestone,
//! reviewers, each with **Edit** and its picker), editing the title and the description, the
//! pickers of the New issue form, assignees on rows and the Overview's **Waiting for your
//! review** panel. The model and the requests are in `triage`.
use rune::{El, TextEdit, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextArea, TextInput};

use crate::actions::{self, Form};
use crate::model::PullState;
use crate::state::{Draft, Facet, Load};
use crate::thread::{Entry, Open, Thread, Verdict};
use crate::triage::{Choice, Host, Picker, SHOWN, Source};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons};

/// The details block's width beside the conversation.
pub const SIDE_W: f32 = 280.0;

/// Whether the details block goes beside the conversation: a wide page (the column leaves the
/// conversation at least 540 points). On a phone, or a narrow window, it goes above it.
pub fn side(s: &App) -> bool {
    let pad = if s.compact { 24.0 } else { 48.0 };
    !s.compact && (s.backdrop_size[0] - pad).min(1080.0) >= SIDE_W + 16.0 + 540.0
}

/// Up to three assignees' avatars, and how many more, for a row.
pub fn assignees(k: &Kit, list: &[String]) -> Option<El> {
    if list.is_empty() {
        return None;
    }
    let mut r = row().gap(2.0).center_y().label(format!("Assigned to {}", list.join(", ")));
    for a in list.iter().take(3) {
        r = r.child(ui::user(k, a, 18.0).a11y_hidden());
    }
    if list.len() > 3 {
        r = r.child(text(format!("+{}", list.len() - 3)).font(k.ty.text_small).color(k.c.text_faint).pl(2.0).a11y_hidden());
    }
    Some(r)
}

/// The Overview's **Waiting for your review** panel: the open pull requests where your review is
/// requested, when there are any.
pub fn waiting_panel(s: &App, k: &Kit) -> Option<El> {
    let waiting: Vec<_> = s.data.pulls.iter().filter(|p| p.review_requested && matches!(p.state, PullState::Open | PullState::Draft)).collect();
    if waiting.is_empty() {
        return None;
    }
    // The panel shows three; the list shows them all (and says when GitHub has more).
    let (n, more) = crate::tray::waiting_for_review(&s.data);
    let label = if n > 3 || more { format!("See all {}", crate::model::capped_if(n, more)) } else { "See all".to_owned() };
    let all = Button::new(label).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.show_review_requested()).build(k).key("waiting-all")
        .label("See every pull request that waits for your review");
    let mut p = panel(k).key("waiting").child(panel_head(k, "Waiting for your review", Some(all)));
    for pr in waiting.into_iter().take(3) {
        p = p.child(rule(k)).child(crate::lists::pull_row(k, pr, s.now, true));
    }
    Some(p)
}

// ------------------------------------------------------------------ the details block

/// Labels, assignees, milestone and (pull requests) reviewers, each with **Edit**.
pub fn details(s: &App, k: &Kit, o: &Open, t: &Thread) -> El {
    let mut facets = vec![Facet::Labels, Facet::Assignees, Facet::Milestone];
    if o.pull {
        facets.push(Facet::Reviewers);
    }
    let mut p = panel(k).key("triage").padding(14.0).gap(12.0);
    for (n, f) in facets.into_iter().enumerate() {
        if n > 0 {
            p = p.child(rule(k));
        }
        p = p.child(section(s, k, o, t, f));
    }
    p
}

fn section(s: &App, k: &Kit, o: &Open, t: &Thread, f: Facet) -> El {
    let open = o.edit.picker.as_ref().filter(|p| p.facet == f);
    let mut head = row().w_full().gap(8.0).center_y()
        .child(text(f.title()).font(k.ty.small_medium).color(k.c.text_muted).grow(1.0));
    if open.is_none() {
        head = head.child(Button::new("Edit").icon(&icons::PEN).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .disabled(o.edit.saving.is_some())
            .on_press(move |s: &mut App| s.open_picker(Host::Thread, f)).build(k)
            .key(format!("triage-edit-{}", f.key())).label(format!("Edit {}", f.title().to_lowercase())));
    }
    let body = match open {
        Some(p) => {
            let repo = s.picker_repo(Host::Thread).unwrap_or_else(|| o.repo.clone());
            // A pull request's author can't review their own.
            let hidden = (f == Facet::Reviewers).then_some(t.author.as_str());
            picker_panel(s, k, Host::Thread, &repo, p, hidden)
        }
        None => values(k, t, f),
    };
    let mut c = col().w_full().gap(8.0).key(format!("triage-{}", f.key())).child(head).child(body);
    if let Some(e) = o.edit.error(f.key()) {
        c = c.child(text(e.to_owned()).font(k.ty.small).color(k.c.danger.icon).wrap().role("alert").key("triage-error"));
    }
    c
}

fn none(k: &Kit, what: &str) -> El {
    text(what.to_owned()).font(k.ty.small).color(k.c.text_faint)
}

fn person(k: &Kit, login: &str, tag: Option<El>) -> El {
    row().w_full().gap(8.0).center_y()
        .child(ui::user(k, login, 20.0).a11y_hidden())
        .child(text(login.to_owned()).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink().max_lines(1))
        .children(tag)
}

/// What a section shows when its picker is closed.
fn values(k: &Kit, t: &Thread, f: Facet) -> El {
    match f {
        Facet::Labels if t.labels.is_empty() => none(k, "None yet"),
        Facet::Labels => t.labels.iter().fold(row().w_full().gap(6.0).flex_wrap(), |r, (name, hue)| r.child(ui::label_chip(k, name, *hue))),
        Facet::Assignees if t.assignees.is_empty() => none(k, "No one"),
        Facet::Assignees => t.assignees.iter().fold(col().w_full().gap(6.0), |c, a| c.child(person(k, a, None))),
        Facet::Milestone => match &t.milestone {
            Some((_, title)) => text(title.clone()).font(k.ty.small).color(k.c.text_base).wrap(),
            None => none(k, "No milestone"),
        },
        Facet::Reviewers => {
            if t.requested.is_empty() && t.reviewed.is_empty() {
                return none(k, "No reviews asked for");
            }
            let mut c = col().w_full().gap(6.0);
            // Asked and not answered yet; then who answered, with their verdict (in words).
            for r in &t.requested {
                c = c.child(person(k, r, Some(ui::pill(k, "Asked", k.c.text_muted))));
            }
            for (who, v) in t.reviewed.iter().filter(|(w, _)| !t.requested.iter().any(|r| r.eq_ignore_ascii_case(w))) {
                let ink = match v {
                    Verdict::Approved => k.c.success.icon,
                    Verdict::ChangesRequested => k.c.danger.icon,
                    Verdict::Commented => k.c.text_muted,
                };
                c = c.child(person(k, who, Some(ui::pill(k, v.label(), ink))));
            }
            c
        }
    }
}

// ------------------------------------------------------------------ the picker

fn noun(f: Facet) -> &'static str {
    match f {
        Facet::Labels => "labels",
        Facet::Assignees | Facet::Reviewers => "people",
        Facet::Milestone => "milestones",
    }
}

/// Esc in a picker's filter or an editor closes it at once (the field would only lose its
/// focus), as Esc elsewhere does (`App::escape`).
fn esc_closes(s: &mut App, ev: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    ev.key == rune::Key::Escape && ev.mods == rune::Mods::default() && s.escape(tx)
}

/// A picker in place: a filter, the repository's choices as a checklist (one choice for the
/// milestone), **Apply** and **Cancel**.
pub fn picker_panel(s: &App, k: &Kit, host: Host, repo: &str, p: &Picker, hidden: Option<&str>) -> El {
    let source = Source::of(p.facet);
    let what = noun(p.facet);
    let filter = TextInput::new(&p.filter).leading_icon(k.glyph.search).placeholder(format!("Filter {what}")).label(format!("Filter {what}"))
        .on_edit(move |s: &mut App| &mut s.picker_mut(host).expect("picker open").filter).on_key(esc_closes).build(k).key("triage-filter").w_full();
    let mut list = col().w_full().gap(2.0).key("triage-choices");
    match s.choices.get(repo, source) {
        None | Some(Load::Loading) => {
            list = list.child(text(format!("Reading {}…", source.noun())).font(k.ty.small).color(k.c.text_muted).role("status").py(6.0));
        }
        Some(Load::Failed(e)) => {
            let repo = repo.to_owned();
            list = list.child(ui::error_inline(k, e, "triage-retry", move |s: &mut App| s.load_choices(&repo, source, true)).padding(4.0));
        }
        Some(Load::Ready(c)) => {
            if p.facet == Facet::Milestone && p.filter.text.trim().is_empty() {
                list = list.child(choice_row(k, host, p.facet, &Choice { id: String::new(), name: "No milestone".into(), hue: None, note: String::new() }, p.picked.is_empty()));
            }
            let shown = p.shown(&c.items, hidden);
            if shown.is_empty() {
                let empty = match (c.items.is_empty(), p.facet) {
                    (true, Facet::Labels) => "This repository has no labels yet.",
                    (true, Facet::Milestone) => "This repository has no open milestones.",
                    (true, _) => "No one can be assigned in this repository.",
                    (false, _) => "Nothing matches.",
                };
                list = list.child(text(empty).font(k.ty.small).color(k.c.text_muted).wrap().py(6.0));
            }
            for ch in shown.iter().take(SHOWN) {
                list = list.child(choice_row(k, host, p.facet, ch, p.is_on(&ch.id)));
            }
            if shown.len() > SHOWN {
                list = list.child(text(format!("Showing {SHOWN} of {}: type to narrow them.", shown.len())).font(k.ty.text_small).color(k.c.text_muted).wrap().py(4.0));
            }
            if c.more {
                list = list.child(text(format!("GitHub has more than 100 {}: only the first 100 are listed.", source.noun())).font(k.ty.text_small).color(k.c.text_muted).wrap().py(4.0).key("triage-more"));
            }
        }
    }
    let saving = host == Host::Thread && s.thread.as_ref().is_some_and(|o| o.edit.saving(p.facet.key()));
    // The form keeps what's picked until the issue is created: nothing is sent yet.
    let blocked = if host == Host::Thread { actions::blocked(s) } else { None };
    let buttons = row().w_full().gap(8.0).justify_end().flex_wrap()
        .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.close_picker(host)).build(k).key("triage-cancel"))
        .child(Button::new(if saving { "Applying…" } else { "Apply" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .disabled(!p.changed() || saving || blocked.is_some())
            .on_press(move |s: &mut App| s.apply_picker(host)).build(k).key("triage-apply"));
    let mut c = col().w_full().gap(8.0).key("triage-picker")
        .child(filter)
        .child(list)
        .child(buttons);
    if let Some(n) = &p.note {
        c = c.child(text(n.clone()).font(k.ty.small).color(k.c.warning.icon).wrap().key("triage-note"));
    }
    if let Some(why) = blocked {
        c = c.child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap().key("triage-blocked"));
    }
    c
}

/// One choice: a box (round for the milestone) that says ticked or not, the label's colour or
/// the person's avatar, the name and its second line.
fn choice_row(k: &Kit, host: Host, facet: Facet, ch: &Choice, on: bool) -> El {
    let radio = facet == Facet::Milestone;
    let mark = el().size(16.0, 16.0).radius(if radio { 8.0 } else { 4.0 }).center()
        .border(1.0, if on { k.c.text_accent } else { k.c.border_strong })
        .fill(if on { k.c.bg_accent.alpha(0.18) } else { rune::Color::TRANSPARENT })
        .children(on.then(|| ui::ico(&icons::CHECK, 12.0, k.c.text_accent)));
    let lead = match (facet, ch.hue) {
        // In a row, so the chip is as wide as its name.
        (Facet::Labels, Some(hue)) => row().child(ui::label_chip(k, &ch.name, hue)),
        (Facet::Assignees | Facet::Reviewers, _) => row().gap(8.0).center_y()
            .child(ui::user(k, &ch.name, 20.0))
            .child(text(ch.name.clone()).font(k.ty.small).color(k.c.text_base).max_lines(1)),
        _ => text(ch.name.clone()).font(k.ty.small).color(k.c.text_base).wrap(),
    };
    let mut body = col().grow(1.0).shrink().gap(2.0).child(lead);
    if !ch.note.is_empty() {
        body = body.child(text(ch.note.clone()).font(k.ty.text_small).color(k.c.text_faint).wrap());
    }
    let id = ch.id.clone();
    let key = if ch.id.is_empty() { "triage-choice-none".to_owned() } else { format!("triage-choice-{}", ch.id) };
    row().w_full().px(6.0).py(5.0).gap(8.0).center_y().radius(k.radius.md).hover(k.c.hover).press(k.c.pressed)
        .role(if radio { "radio" } else { "checkbox" }).checked(on).label(ch.name.clone()).key(key)
        .on_press(move |s: &mut App| s.toggle_choice(host, &id))
        .child(mark.a11y_hidden())
        .child(body.a11y_hidden())
}

// ------------------------------------------------------------------ title and description

fn edit_buttons(k: &Kit, saving: bool, off: bool, save: fn(&mut App), cancel: fn(&mut App), what: &str) -> El {
    row().w_full().gap(8.0).justify_end().flex_wrap()
        .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(cancel).build(k).key(format!("triage-{what}-cancel")))
        .child(Button::new(if saving { "Saving…" } else { "Save" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
            .disabled(off || saving).on_press(save).build(k).key(format!("triage-{what}-save")))
}

/// What's said under an editor: GitHub's refusal, or why nothing can be saved.
fn edit_notes(s: &App, k: &Kit, o: &Open, part: &str) -> Option<El> {
    if let Some(e) = o.edit.error(part) {
        return Some(text(e.to_owned()).font(k.ty.small).color(k.c.danger.icon).wrap().role("alert").key("triage-error"));
    }
    actions::blocked(s).map(|why| text(why).font(k.ty.small).color(k.c.warning.icon).wrap().key(format!("triage-{part}-blocked")))
}

/// The title, with **Edit** for people who may edit it; or the field that edits it.
pub fn title(s: &App, k: &Kit, o: &Open, t: &Thread) -> El {
    if let Some(e) = &o.edit.title {
        let saving = o.edit.saving("title");
        let off = e.text.trim().is_empty() || actions::blocked(s).is_some();
        return col().w_full().gap(8.0).key("triage-title")
            .child(TextInput::new(e).label("Title").placeholder("Title")
                .on_edit(|s: &mut App| s.thread.as_mut().and_then(|o| o.edit.title.as_mut()).expect("editing the title"))
                .on_submit(|s: &mut App, _| s.save_title()).on_key(esc_closes).build(k).key("triage-title-input").w_full())
            .child(edit_buttons(k, saving, off, |s| s.save_title(), |s| s.cancel_edit(Draft::Title), "title"))
            .children(edit_notes(s, k, o, "title"));
    }
    let mut r = row().w_full().gap(8.0).items_start()
        .child(text(t.title.clone()).font(ui::H2).color(k.c.text_strong).wrap().grow(1.0).shrink());
    if crate::triage::may_edit(s, o, t) {
        r = r.child(Button::new("Edit").icon(&icons::PEN).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(|s: &mut App| s.edit_title()).build(k).key("triage-edit-title").label("Edit the title"));
    }
    r
}

/// **Edit** on the description, for people who may edit it.
pub fn edit_body_button(s: &App, k: &Kit, o: &Open, t: &Thread) -> Option<El> {
    crate::triage::may_edit(s, o, t).then(|| Button::new("Edit").icon(&icons::PEN).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.edit_body()).build(k).key("triage-edit-body").label("Edit the description"))
}

/// The description being edited, in place of the first entry.
pub fn body_editor(s: &App, k: &Kit, o: &Open, first: &Entry, e: &TextEdit) -> El {
    let saving = o.edit.saving("body");
    let off = actions::blocked(s).is_some();
    panel(k).key("thread-entry-0")
        .child(row().w_full().px(16.0).py(10.0).gap(10.0).center_y()
            .child(ui::user(k, &first.author, 22.0))
            .child(text("Editing the description").font(k.ty.small_medium).color(k.c.text_strong)))
        .child(rule(k))
        .child(col().w_full().padding(16.0).gap(10.0)
            .child(TextArea::new(e).label("Description").placeholder("Description (Markdown)").auto_grow(6, 24)
                .on_edit(|s: &mut App| s.thread.as_mut().and_then(|o| o.edit.body.as_mut()).expect("editing the description"))
                .on_key(esc_closes).build(k).key("triage-body-input").w_full())
            .child(edit_buttons(k, saving, off, |s| s.save_body(), |s| s.cancel_edit(Draft::Description), "body"))
            .children(edit_notes(s, k, o, "body")))
}

// ------------------------------------------------------------------ the New issue form

/// The form's **Labels** and **Assignees**: what's picked, **Edit**, and the picker in place.
pub fn form_fields(s: &App, k: &Kit, f: &Form) -> El {
    let picks = &s.issue_picks;
    let mut c = col().w_full().gap(10.0).key("form-triage");
    for facet in [Facet::Labels, Facet::Assignees] {
        let open = picks.picker.as_ref().filter(|p| p.facet == facet);
        let picked = if facet == Facet::Labels { &picks.labels } else { &picks.assignees };
        let mut head = row().w_full().gap(8.0).center_y().flex_wrap()
            .child(text(facet.title()).font(k.ty.small_medium).color(k.c.text_muted).w(84.0));
        if open.is_none() {
            if picked.is_empty() {
                head = head.child(none(k, if facet == Facet::Labels { "None" } else { "No one" }));
            }
            for id in picked {
                head = head.child(match facet {
                    Facet::Labels => ui::label_chip(k, id, s.choices.hue(&f.repo, id).unwrap_or(250.0)),
                    _ => row().gap(6.0).center_y().child(ui::user(k, id, 18.0)).child(text(id.clone()).font(k.ty.small).color(k.c.text_base)),
                });
            }
            head = head.child(el().grow(1.0)).child(Button::new(if picked.is_empty() { "Add" } else { "Edit" }).icon(&icons::PEN).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.open_picker(Host::Form, facet)).build(k)
                .key(format!("form-edit-{}", facet.key())).label(format!("Choose the {}", facet.title().to_lowercase())));
        }
        c = c.child(head);
        if let Some(p) = open {
            c = c.child(panel(k).padding(10.0).child(picker_panel(s, k, Host::Form, &f.repo, p, None)));
        }
    }
    c
}

/// After `page pulls`/`issues` and `thread 14`: opens a picker (`-- picker labels`) or an editor
/// (`-- edit title`, `-- edit body`) for headless stills; with the sample data the choices are
/// here at once.
impl App {
    pub(crate) fn triage_still(&mut self, picker: Option<String>, edit: Option<String>) {
        if let Some(f) = picker.as_deref().and_then(Facet::parse) {
            // On a repository's page, without a conversation: the New issue form's picker.
            if self.thread.is_none()
                && matches!(f, Facet::Labels | Facet::Assignees)
                && let Some(i) = self.repo_index()
            {
                self.open_form(i, crate::state::Compose::Issue);
                if let Some(form) = self.form.as_mut() {
                    form.title.focused = false;
                }
                self.open_picker(Host::Form, f);
            } else {
                self.open_picker(Host::Thread, f);
            }
            if let Some(p) = self.picker_mut(Host::Thread).or(None) {
                p.filter.focused = false;
            }
            if let Some(p) = self.issue_picks.picker.as_mut() {
                p.filter.focused = false;
            }
        }
        match edit.as_deref() {
            Some("title") => self.edit_title(),
            Some("body") => self.edit_body(),
            _ => {}
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rune::TextEdit;

    use crate::github::{Created, Done, Write};
    use crate::model::same_repo;
    use crate::state::{Compose, Facet, Load};
    use crate::thread::Standing;
    use crate::triage::{self, Applied, Change, Source};
    use crate::{App, Page, runs_page, thread};

    /// An account's app (writes allowed, nothing to send them to) with the sample lists.
    fn live() -> App {
        let mut s = crate::init();
        s.pending_place = None;
        let mut cfg = crate::config::Config::from_json("{}").unwrap();
        cfg.api = "http://127.0.0.1:9".into();
        cfg.token = Some("test-token".into());
        s.cfg = std::sync::Arc::new(cfg);
        s.data = crate::model::Data::sample();
        s.data.sample = false;
        s.status = crate::Status::Ready;
        s.data_changed();
        s
    }

    /// Opens a conversation as GitHub would have answered, with the repository's choices here.
    fn open(s: &mut App, number: u32, pull: bool) -> (String, String) {
        let repo = if pull {
            s.data.pulls.iter().find(|p| p.number == number).map(|p| p.repo.clone())
        } else {
            s.data.issues.iter().find(|x| x.number == number).map(|x| x.repo.clone())
        }.expect("in the sample lists");
        s.open_thread(&repo, number, pull);
        let mut t = thread::sample("Title", "contributor42", pull, Standing::Open, s.now);
        triage::sample_thread(&mut t, &s.data, &repo, number, pull);
        s.thread.as_mut().unwrap().load = Load::Ready(t);
        let full = runs_page::full_repo_of(s, &repo);
        for src in [Source::Labels, Source::People, Source::Milestones] {
            s.choices.put(&full, src, triage::sample_choices(src, &s.data));
        }
        (repo, full)
    }

    thread_local! {
        /// The headless clock: it moves on every frame, so what grows has grown before the next press.
        static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }

    fn rt(s: App, w: f32) -> rune::Runtime<App> {
        let mut rt = rune::Runtime::new(s, crate::view, (w, 2400.0), rune::Env::new(true, 1.0));
        rt.reduced_motion = true;
        CLOCK.with(|c| c.set(0.0));
        tick(&mut rt);
        rt
    }

    fn tick(rt: &mut rune::Runtime<App>) {
        let t = CLOCK.with(|c| {
            c.set(c.get() + 1.0);
            c.get()
        });
        rt.frame(t);
        rt.frame(t + 0.5);
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        tick(rt);
    }

    fn shows(rt: &rune::Runtime<App>, words: &str) -> bool {
        rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.contains(words)))
    }

    fn thread_of(s: &App) -> &thread::Thread {
        match &s.thread.as_ref().expect("open").load {
            Load::Ready(t) => t,
            _ => panic!("loaded"),
        }
    }

    fn last_sent(s: &App) -> (String, Write) {
        s.choices.sent.last().cloned().expect("a write was sent")
    }

    #[test]
    fn a_picker_ticks_applies_one_change_and_the_answer_updates_the_conversation_and_the_row() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        let (repo, full) = open(&mut s, 31, false);
        let mut rt = rt(s, 1180.0);
        assert!(rt.find("triage").is_some() && rt.find("triage-reviewers").is_none(), "an issue has no reviewers");
        press(&mut rt, "triage-edit-labels");
        let p = rt.state.thread.as_ref().unwrap().edit.picker.as_ref().expect("the picker opened");
        assert_eq!((p.facet, p.picked.clone()), (Facet::Labels, vec!["bug".to_owned()]));
        assert!(rt.find("triage-filter").is_some() && rt.find("triage-choice-docs").is_some());
        // Nothing changed yet: Apply sends nothing.
        press(&mut rt, "triage-apply");
        assert!(rt.state.choices.sent.is_empty());
        press(&mut rt, "triage-choice-docs");
        press(&mut rt, "triage-apply");
        let (to, w) = last_sent(&rt.state);
        assert!(same_repo(&to, &repo));
        assert!(matches!(&w, Write::Triage { number: 31, change: Change::Labels(l) } if *l == ["bug", "docs"]), "{w:?}");
        assert_eq!(rt.state.thread.as_ref().unwrap().edit.saving, Some("labels"));
        tick(&mut rt);
        assert!(shows(&rt, "Applying…"));
        // The row's Close doesn't wait for a label change.
        assert!(rt.state.busy.contains(&triage::busy_key(&repo, 31)) && !rt.state.busy.contains(&crate::actions::busy_key(&repo, 31)));
        assert!(shows(&rt, "Close issue") && !shows(&rt, "Closing…"));

        // GitHub's answer: the conversation and the row show it, without a reload, with a toast.
        let change = Change::Labels(vec!["bug".into(), "docs".into()]);
        rt.update(|s, _| s.apply_done(&full, Done::Triaged { number: 31, change, applied: Applied::Labels(vec![("bug".into(), 25.0), ("docs".into(), 240.0)]) }));
        tick(&mut rt);
        let o = rt.state.thread.as_ref().unwrap();
        assert!(o.edit.picker.is_none() && o.edit.saving.is_none());
        assert_eq!(thread_of(&rt.state).labels.len(), 2);
        let row = rt.state.data.issues.iter().find(|x| x.number == 31).unwrap();
        assert_eq!((row.labels.len(), row.label.as_ref().map(|l| l.0.as_str())), (2, Some("bug")));
        assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Labels of #31: bug, docs"));
        assert!(shows(&rt, "docs"));
    }

    #[test]
    fn assignees_milestone_and_reviewers_each_send_their_own_change() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        let (_, full) = open(&mut s, 8, true);
        let mut rt = rt(s, 1180.0);
        let answer = |rt: &mut rune::Runtime<App>, applied: Applied| {
            let Write::Triage { change, .. } = last_sent(&rt.state).1 else { panic!("a triage write") };
            let full = full.clone();
            rt.update(move |s, _| {
                s.busy.clear();
                s.apply_done(&full, Done::Triaged { number: 8, change, applied });
            });
            tick(rt);
        };

        // Reviewers: the author isn't offered; ask octocat, stop asking hubot.
        press(&mut rt, "triage-edit-reviewers");
        assert!(rt.find("triage-choice-contributor42").is_none(), "the author can't review their own pull request");
        press(&mut rt, "triage-choice-octocat");
        press(&mut rt, "triage-choice-hubot");
        press(&mut rt, "triage-apply");
        assert!(matches!(last_sent(&rt.state).1, Write::Triage { number: 8, change: Change::Reviewers { ref add, ref remove } } if *add == ["octocat"] && *remove == ["hubot"]));
        answer(&mut rt, Applied::Reviewers(vec!["francescodibella".into(), "octocat".into()]));
        assert_eq!(thread_of(&rt.state).requested, ["francescodibella", "octocat"]);
        assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Reviewers of #8: asked octocat, no longer hubot"));
        assert!(rt.state.data.pulls.iter().find(|p| p.number == 8).unwrap().review_requested);

        // Taking back the request for your own review: it no longer waits for you.
        press(&mut rt, "triage-edit-reviewers");
        press(&mut rt, "triage-choice-francescodibella");
        press(&mut rt, "triage-apply");
        answer(&mut rt, Applied::Reviewers(vec!["octocat".into()]));
        assert!(!rt.state.data.pulls.iter().find(|p| p.number == 8).unwrap().review_requested);
        assert_eq!(rt.state.data.review_total, Some(0));
        assert!(!crate::tray::menu(&rt.state).iter().any(|e| e.id == "go:pulls:review"), "the status-bar line follows");

        // The milestone: one choice, or none.
        press(&mut rt, "triage-edit-milestone");
        press(&mut rt, "triage-choice-none");
        press(&mut rt, "triage-apply");
        assert!(matches!(last_sent(&rt.state).1, Write::Triage { change: Change::Milestone(None), .. }));
        answer(&mut rt, Applied::Milestone(None));
        assert!(thread_of(&rt.state).milestone.is_none() && shows(&rt, "No milestone"));

        // Assignees: the row's avatars follow.
        press(&mut rt, "triage-edit-assignees");
        press(&mut rt, "triage-choice-hubot");
        press(&mut rt, "triage-apply");
        assert!(matches!(last_sent(&rt.state).1, Write::Triage { change: Change::Assignees(ref a), .. } if *a == ["hubot"]));
        answer(&mut rt, Applied::Assignees(vec!["hubot".into()]));
        assert_eq!(rt.state.data.pulls.iter().find(|p| p.number == 8).unwrap().assignees, ["hubot"]);
        assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Assigned #8 to hubot"));
        // Cancel drops what was ticked.
        press(&mut rt, "triage-edit-labels");
        press(&mut rt, "triage-choice-docs");
        press(&mut rt, "triage-cancel");
        assert!(rt.state.thread.as_ref().unwrap().edit.picker.is_none());
        assert_eq!(thread_of(&rt.state).labels.len(), 1);
    }

    #[test]
    fn titles_and_descriptions_are_edited_in_place_and_kept_when_saving_fails() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        let (repo, full) = open(&mut s, 31, false);
        let mut rt = rt(s, 1180.0);
        press(&mut rt, "triage-edit-title");
        assert!(rt.find("triage-title-input").is_some());
        rt.update(|s, _| s.thread.as_mut().unwrap().edit.title = Some(TextEdit::new("Poncho clips through the arms")));
        tick(&mut rt);
        press(&mut rt, "triage-title-save");
        assert!(matches!(last_sent(&rt.state).1, Write::Triage { number: 31, change: Change::Title(ref t) } if t == "Poncho clips through the arms"));
        // GitHub refuses: said once, under the field, and what was typed stays.
        let refused = "GitHub refused (403): Must have admin rights to Repository. The token needs write access to this repository (Contents, Issues and Pull requests).";
        let (f2, r2) = (full.clone(), refused.to_owned());
        rt.update(move |s, _| {
            s.busy.clear();
            assert!(s.triage_failed(&f2, 31, "title", false, &r2));
        });
        tick(&mut rt);
        let o = rt.state.thread.as_ref().unwrap();
        assert_eq!(o.edit.title.as_ref().map(|e| e.text.as_str()), Some("Poncho clips through the arms"));
        assert_eq!(thread_of(&rt.state).title, "Title", "the old title stays");
        assert_eq!(rt.elements().iter().filter(|e| e.key.as_deref() == Some("triage-error")).count(), 1, "said once");
        assert!(shows(&rt, "Nothing was changed.") && rt.state.toast.is_none());
        // Saving again works; the row's title follows.
        press(&mut rt, "triage-title-save");
        let Write::Triage { change, .. } = last_sent(&rt.state).1 else { panic!() };
        let f3 = full.clone();
        rt.update(move |s, _| s.apply_done(&f3, Done::Triaged { number: 31, change, applied: Applied::Title("Poncho clips through the arms".into()) }));
        assert!(rt.state.thread.as_ref().unwrap().edit.title.is_none());
        assert_eq!(rt.state.data.issues.iter().find(|x| x.number == 31).unwrap().title, "Poncho clips through the arms");

        // The description, in the Markdown area.
        tick(&mut rt);
        press(&mut rt, "triage-edit-body");
        assert!(rt.find("triage-body-input").is_some());
        rt.update(|s, _| s.thread.as_mut().unwrap().edit.body = Some(TextEdit::new("**Steps:** lower the arms.")));
        tick(&mut rt);
        press(&mut rt, "triage-body-save");
        assert!(matches!(last_sent(&rt.state).1, Write::Triage { change: Change::Body(ref b), .. } if b == "**Steps:** lower the arms."));
        let Write::Triage { change, .. } = last_sent(&rt.state).1 else { panic!() };
        rt.update(move |s, _| {
            s.busy.clear();
            s.apply_done(&full, Done::Triaged { number: 31, change, applied: Applied::Body("**Steps:** lower the arms.".into()) });
        });
        tick(&mut rt);
        assert_eq!(thread_of(&rt.state).entries[0].body, "**Steps:** lower the arms.");
        assert!(rt.find("triage-body-input").is_none() && shows(&rt, "lower the arms"));
        assert!(same_repo(&repo, "francescodibella.github.io"));
    }

    #[test]
    fn a_refused_change_keeps_the_old_values_and_late_answers_stay_with_their_own_conversation() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        let (_, full31) = open(&mut s, 31, false);
        s.open_picker(triage::Host::Thread, Facet::Labels);
        s.toggle_choice(triage::Host::Thread, "docs");
        s.apply_picker(triage::Host::Thread);
        assert!(s.triage_failed(&full31, 31, "labels", false, "GitHub refused (403): Resource not accessible by integration."));
        let o = s.thread.as_ref().unwrap();
        assert_eq!(thread_of(&s).labels.len(), 1, "the old labels stay");
        assert!(o.edit.picker.as_ref().is_some_and(|p| p.is_on("docs")), "what was ticked stays, to try again or cancel");
        assert!(o.edit.error("labels").is_some_and(|e| e.contains("triage or write access")) && o.edit.saving.is_none());
        let rt = rt(s, 1180.0);
        assert_eq!(rt.elements().iter().filter(|e| e.key.as_deref() == Some("triage-error")).count(), 1);
        let mut s = rt.state;

        // Another conversation opened meanwhile: #31's answer changes #31's row, not #12 on screen.
        s.busy.clear();
        let (_, full12) = open(&mut s, 12, false);
        let labels12 = thread_of(&s).labels.clone();
        s.apply_done(&full31, Done::Triaged { number: 31, change: Change::Labels(vec![]), applied: Applied::Labels(Vec::new()) });
        assert_eq!(thread_of(&s).labels, labels12);
        assert!(s.data.issues.iter().find(|x| x.number == 31).unwrap().labels.is_empty());
        // Its refusal isn't said in #12's block: it's a toast.
        assert!(!s.triage_failed(&full31, 31, "labels", false, "GitHub refused (403)."));
        assert!(s.thread.as_ref().unwrap().edit.error.is_none());

        // A list asked again: the answer to the first request is dropped.
        let other = "xr-lab/spatial-anchors";
        s.load_choices(other, Source::Labels, false);
        let first = s.choices.waiting(other, Source::Labels).expect("asked");
        s.load_choices(other, Source::Labels, true);
        assert!(!s.choices.answer(other, Source::Labels, first, Ok(triage::Choices::default())));
        assert!(matches!(s.choices.get(other, Source::Labels), Some(Load::Loading)));
        assert!(matches!(s.choices.get(&full12, Source::Labels), Some(Load::Ready(_))));
    }

    #[test]
    fn a_new_issue_takes_labels_and_assignees() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        let full = "francescodibella/francescodibella.github.io".to_owned();
        let i = s.repo_index_of(&full).unwrap();
        s.open(i);
        s.open_form(i, Compose::Issue);
        for src in [Source::Labels, Source::People] {
            s.choices.put(&full, src, triage::sample_choices(src, &s.data));
        }
        let mut rt = rt(s, 1180.0);
        press(&mut rt, "form-edit-labels");
        press(&mut rt, "triage-choice-bug");
        press(&mut rt, "triage-apply");
        press(&mut rt, "form-edit-assignees");
        press(&mut rt, "triage-choice-octocat");
        press(&mut rt, "triage-apply");
        assert_eq!((rt.state.issue_picks.labels.clone(), rt.state.issue_picks.assignees.clone()), (vec!["bug".to_owned()], vec!["octocat".to_owned()]));
        assert!(rt.state.issue_picks.picker.is_none() && rt.state.choices.sent.is_empty(), "nothing is sent before the issue is created");
        let mut s = rt.state;
        s.form.as_mut().unwrap().title = TextEdit::new("Poncho clips");
        let w = triage::with_picks(s.form.as_ref().unwrap().write().unwrap(), &s.issue_picks);
        assert!(matches!(&w, Write::Issue { labels, assignees, .. } if *labels == ["bug"] && *assignees == ["octocat"]), "{w:?}");
        // Created: the new row has them.
        s.form = None;
        s.apply_done(&full, Done::Issue(Created { number: 99, title: "Poncho clips".into(), ..Default::default() }));
        let row = s.data.issues.iter().find(|x| x.number == 99).unwrap();
        assert_eq!((row.labels.clone(), row.assignees.clone()), (vec![("bug".to_owned(), 25.0)], vec!["octocat".to_owned()]));
        assert!(s.issue_picks.labels.is_empty(), "the next form starts empty");
    }

    #[test]
    fn review_requested_narrows_the_list_and_the_overview_follows() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        s.page = Page::Overview;
        let mut rt = rt(s, 1180.0);
        assert!(rt.find("waiting").is_some() && shows(&rt, "Waiting for your review"));
        press(&mut rt, "waiting-all");
        assert!(rt.state.page == Page::Pulls && rt.state.pull_filter.review);
        tick(&mut rt);
        assert!(rt.find("pr-ngsi-ld-xr-contexts-8").is_some() && rt.find("pr-francescodibella.github.io-14").is_none());
        press(&mut rt, "pulls-review");
        assert!(rt.find("pr-francescodibella.github.io-14").is_some(), "off again: everything");
        // Nothing waits: no panel.
        rt.update(|s, _| {
            for p in s.data.pulls.iter_mut() {
                p.review_requested = false;
            }
            s.data_changed();
            s.go(Page::Overview);
        });
        tick(&mut rt);
        assert!(rt.find("waiting").is_none());
    }

    #[test]
    fn filters_survive_leaving_the_page_and_restarting() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = crate::load_gen_tests::TempDataDir::new("triage-filters");
        let mut s = live();
        s.page = Page::Pulls;
        let mut rt = rt(s, 1180.0);
        press(&mut rt, "pulls-review");
        rt.update(|s, _| {
            s.pull_filter.search = TextEdit::new("vocab");
            s.issue_filter.label = Some("bug".into());
            s.pr_filter = crate::model::PullState::Draft;
            s.go(Page::Issues);
            s.go(Page::Pulls);
        });
        assert!(rt.state.pull_filter.review && rt.state.pull_filter.search.text == "vocab", "kept between visits");
        // Restarting with an account: they come back.
        // SAFETY: ENV_LOCK is held; the variables are put back below.
        unsafe {
            std::env::set_var("GITHUB_USER", "francescodibella");
            std::env::set_var("GITHUB_API_URL", "http://127.0.0.1:9");
        }
        let again = crate::init();
        unsafe {
            std::env::remove_var("GITHUB_USER");
            std::env::remove_var("GITHUB_API_URL");
        }
        assert!(!again.data.sample);
        assert!(again.pull_filter.review && again.pull_filter.search.text == "vocab");
        assert_eq!(again.issue_filter.label.as_deref(), Some("bug"));
        assert_eq!(again.pr_filter, crate::model::PullState::Draft);
        // Clear resets them, and that's kept too.
        tick(&mut rt);
        press(&mut rt, "pulls-clear");
        assert!(rt.state.pull_filter.is_empty());
        let saved = crate::prefs::get("filters").unwrap();
        assert_eq!(saved["pulls"]["review"], serde_json::json!(false));
        assert_eq!(saved["issues"]["label"], serde_json::json!("bug"), "each page has its own");
        // The sample data saves nothing.
        let mut sample = crate::init();
        sample.pull_filter.mine = true;
        sample.save_filters();
        assert_eq!(crate::prefs::get("filters").unwrap()["pulls"]["mine"], serde_json::json!(false));
    }

    #[test]
    fn the_details_go_beside_the_conversation_when_wide_and_above_it_on_a_phone() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = live();
        open(&mut s, 8, true);
        let rt1 = rt(s, 1180.0);
        let (tx, ty) = rt1.find("triage").unwrap();
        let (ex, ey) = rt1.find("thread-entry-0").unwrap();
        assert!(tx > ex && (ty - ey).abs() < 600.0, "beside: {tx},{ty} vs {ex},{ey}");
        let mut s = rt1.state;
        s.compact = true;
        let rt2 = rt(s, 390.0);
        let (_, ty) = rt2.find("triage").unwrap();
        let (_, ey) = rt2.find("thread-entry-0").unwrap();
        assert!(ty < ey, "above on a phone");
    }
}
