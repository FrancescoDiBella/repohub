//! The Releases section of a repository's page: the releases with their notes and files, the tags
//! that have no release, and the form that drafts, edits and publishes one (see `releases`).
//! Also the latest release's tag in the repository's header.
use rune::{El, TextEdit, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Switch, TextArea, TextInput};

use crate::releases::{self, Form, Kind, List, Listing, NotesJob, Release, TagUse};
use crate::state::{BranchSlot, Load, RepoTab, Submit};
use crate::ui::{self, panel, panel_head, rule};
use crate::confirm::{Confirm, Tone};
use crate::{App, actions, icons, model, pages};

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn empty(k: &Kit, msg: &str, key: &str) -> El {
    col().w_full().padding(24.0).items_center().key(key.to_owned())
        .child(text(msg.to_owned()).font(k.ty.ui).color(k.c.text_muted).wrap())
}

/// A note in a panel, with an icon.
fn note(k: &Kit, msg: String, key: String) -> El {
    row().w_full().px(16.0).py(9.0).gap(8.0).center_y().flex_wrap().key(key)
        .child(ui::ico(&icons::TAG, 14.0, k.c.icon_muted))
        .child(text(msg).font(k.ty.small).color(k.c.text_muted).wrap().grow(1.0).shrink().min_w(160.0))
}

/// The latest release's tag, for the repository's header; it opens the Releases section.
pub fn header_pill(s: &App, k: &Kit, i: usize) -> Option<El> {
    let r = s.data.repos.get(i)?;
    let tag = s.releases.latest_of(&r.full_name)?.tag.clone();
    let ink = k.c.success.icon;
    Some(row().gap(4.0).px(8.0).py(2.0).center_y().radius(10.0).border(1.0, ink.alpha(0.5)).hover(k.c.hover).press(k.c.pressed)
        .role("link").label(format!("Latest release {tag}: open Releases")).key("latest-release")
        .on_press(move |s: &mut App| s.set_tab(i, RepoTab::Releases))
        .child(ui::ico(&icons::TAG, 12.0, ink))
        .child(text(tag).font(k.ty.text_xsmall_medium).color(ink)))
}

/// The Releases section of repository `i`.
pub fn releases_tab(s: &App, k: &Kit, i: usize) -> El {
    let r = &s.data.repos[i];
    let list = s.releases.list_of(&r.full_name);
    let mut page = col().w_full().gap(16.0).key("releases");
    if let Some(f) = s.releases.form.as_ref().filter(|f| f.repo.eq_ignore_ascii_case(&r.full_name)) {
        page = page.child(form_panel(s, k, i, f));
    }
    page = page.child(list_panel(s, k, i, list));
    if let Some(tags) = tags_panel(s, k, i, list) {
        page = page.child(tags);
    }
    page
}

fn new_button(k: &Kit, i: usize) -> El {
    Button::new("New release").icon(&icons::TAG).control_size(ControlSize::Small)
        .help("Draft a release or publish one: its tag, title, notes and whether it's a pre-release")
        .on_press_tx(move |s: &mut App, tx| {
            s.open_release_form(i, None);
            tx.scroll_into_view("release-form");
        })
        .build(k).key("release-new")
}

/// The releases, newest first, with Load more at the end.
fn list_panel(s: &App, k: &Kit, i: usize, list: Option<&List>) -> El {
    let r = &s.data.repos[i];
    let head = |count: Option<String>| {
        let mut tools = row().gap(10.0).center_y();
        if let Some(c) = count {
            tools = tools.child(text(c).font(k.ty.small).color(k.c.text_muted).key("releases-count"));
        }
        panel_head(k, "Releases", Some(tools.child(new_button(k, i))))
    };
    let (l, listing) = match list.map(|l| (l, &l.load)) {
        None | Some((_, Load::Loading)) => {
            let p = panel(k).key("skeleton-releases").role("status").label("Loading releases…").child(head(None));
            return ui::skeleton_rows_in_panel(k, p, 4, true);
        }
        Some((_, Load::Failed(e))) => {
            return panel(k).key("releases-failed").child(head(None)).child(rule(k))
                .child(ui::error_inline(k, e, "retry-releases", move |s: &mut App| s.load_releases(i, true)));
        }
        Some((l, Load::Ready(x))) => (l, x),
    };
    let n = listing.releases.len();
    let count = if l.more { format!("{n}+ releases") } else { plural(n, "release") };
    let mut p = panel(k).key("releases-list").child(head(Some(count)));
    if listing.releases.is_empty() {
        p = p.child(rule(k)).child(empty(k, "No releases yet.", "releases-empty"));
    }
    let latest = s.releases.latest_id(&r.full_name);
    for rel in &listing.releases {
        p = p.child(rule(k)).child(release_view(s, k, i, listing, rel, latest));
    }
    if l.more {
        p = p.child(rule(k)).child(row().w_full().px(16.0).py(10.0).gap(12.0).center_y().justify_between().flex_wrap().key("releases-more")
            .child(text("GitHub has more releases.").font(k.ty.small).color(k.c.text_muted))
            .child(Button::new(if l.loading_more { "Loading…" } else { "Load more" }).control_size(ControlSize::Small).disabled(l.loading_more)
                .on_press(|s: &mut App| s.load_more_releases()).build(k).key("releases-load-more")));
    } else if n > 0 {
        p = p.child(rule(k)).child(row().w_full().px(16.0).py(10.0).key("releases-end")
            .child(text("That's all of the releases.").font(k.ty.small).color(k.c.text_faint)));
    }
    p
}

/// One release: its title and state, who published it and when, the notes and the files; a
/// draft also has Edit and Publish.
fn release_view(s: &App, k: &Kit, i: usize, listing: &Listing, rel: &Release, latest: Option<u64>) -> El {
    let r = &s.data.repos[i];
    let id = rel.id;
    let kind = releases::kind(rel, latest);
    let mut title = row().gap(8.0).center_y().flex_wrap().grow(1.0).shrink().min_w(0.0)
        .child(ui::ico(&icons::TAG, 15.0, k.c.icon_muted))
        .child(text(rel.title().to_owned()).font(k.ty.body_medium).color(k.c.text_strong).wrap().selectable(true)
            .role("heading").level(3).key(format!("release-title-{id}")));
    if let Some(word) = kind.word() {
        let ink = match kind {
            Kind::Latest => k.c.success.icon,
            Kind::Pre => k.c.warning.icon,
            _ => k.c.text_muted,
        };
        title = title.child(ui::pill(k, word, ink).key(format!("release-kind-{id}")));
    }
    let url = if rel.url.is_empty() { format!("{}/releases/tag/{}", r.url, rel.tag) } else { rel.url.clone() };
    let github = IconButton::new(&icons::EXTERNAL).variant(IconButtonVariant::GhostMuted).label(format!("Open {} on GitHub", rel.title())).help("Open on GitHub")
        .on_press_tx({ let url = url.clone(); move |_: &mut App, tx| { let _ = tx.open_url(&url); } })
        .build(k).key(format!("release-github-{id}"));
    let ago = model::fmt_ago(rel.at(), s.now);
    let who = if rel.draft { format!("Drafted by {} {ago}", rel.author) } else { format!("Published by {} {ago}", rel.author) };
    let mut meta = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(text(rel.tag.clone()).font(k.ty.mono).color(k.c.text_base).selectable(true))
        .child(text(who).font(k.ty.text_small).color(k.c.text_muted));
    if rel.draft && !listing.tag_exists(&rel.tag) && !rel.target.is_empty() {
        meta = meta.child(text(format!("The tag will be made from {}.", rel.target)).font(k.ty.text_small).color(k.c.text_faint));
    }
    let mut out = col().w_full().px(16.0).py(14.0).gap(10.0).key(format!("release-{id}"))
        .child(row().w_full().gap(8.0).center_y().child(title).child(github))
        .child(meta);
    // The notes, cut after a dozen lines unless Show all was pressed.
    if rel.body.trim().is_empty() {
        out = out.child(text("No notes.").font(k.ty.small).color(k.c.text_faint));
    } else {
        let open = s.releases.expanded.contains(&id);
        let cut = releases::collapsed(&rel.body);
        let shown = match (&cut, open) {
            (Some(c), false) => c.as_str(),
            _ => rel.body.as_str(),
        };
        let md = crate::markdown::prepare(shown, &r.full_name, &rel.tag);
        out = out.child(col().w_full().key(format!("release-notes-{id}")).child(pages::markdown_view(k, &md, &format!("release-code-{id}"))));
        if cut.is_some() {
            out = out.child(Button::new(if open { "Show less" } else { "Show all" }).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(move |s: &mut App| if !s.releases.expanded.remove(&id) { s.releases.expanded.insert(id); })
                .build(k).key(format!("release-more-{id}")).self_start());
        }
    }
    if !rel.assets.is_empty() {
        let mut files = col().w_full().gap(2.0).key(format!("release-files-{id}"))
            .child(text(format!("Files · {}", rel.assets.len())).font(k.ty.text_small_medium).color(k.c.text_muted).role("heading").level(4));
        for (n, a) in rel.assets.iter().enumerate() {
            let (size, link) = (releases::size(a.size), a.url.clone());
            files = files.child(row().w_full().px(8.0).py(7.0).gap(10.0).center_y().radius(k.radius.md).hover(k.c.hover).press(k.c.pressed)
                .role("link").label(format!("Download {} ({size}) in the browser", a.name)).key(format!("release-file-{id}-{n}"))
                .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&link); })
                .child(ui::ico(&icons::DOWNLOAD, 14.0, k.c.icon_muted))
                .child(text(a.name.clone()).font(k.ty.small).color(k.c.text_accent).wrap().grow(1.0).shrink().min_w(0.0))
                .child(text(size).font(k.ty.text_small).color(k.c.text_muted))
                .child(text(releases::downloads(a.downloads)).font(k.ty.text_small).color(k.c.text_faint)));
        }
        out = out.child(files);
    }
    if rel.draft {
        out = out.child(draft_tools(s, k, i, listing, rel, &url));
    }
    out
}

/// Edit, Publish (it asks first) and where files are added, under a draft.
fn draft_tools(s: &App, k: &Kit, i: usize, listing: &Listing, rel: &Release, url: &str) -> El {
    let id = rel.id;
    let busy = s.releases.busy.contains(&id);
    let blocked = actions::blocked(s);
    let mut out = col().w_full().gap(8.0);
    // Publishing asks first (`releases.confirm`): in place of the buttons on a computer, in a
    // sheet on a phone.
    let question = releases::publish_question(&rel.tag, &rel.target, listing.tag_exists(&rel.tag));
    if let Some(q) = Confirm::new(format!("release-confirm-{id}"), &icons::TAG, question)
        .open(s.releases.confirm == Some(id) && !busy)
        .keys(format!("release-publish-yes-{id}"), format!("release-publish-no-{id}"))
        .disabled(blocked.is_some())
        .yes("Publish", Tone::Go, move |s: &mut App| s.publish_draft(i, id))
        .on_no(|s: &mut App| s.releases.confirm = None)
        .show(s, k)
    {
        return out.child(q);
    }
    let files_url = url.to_owned();
    out = out.child(row().gap(8.0).center_y().flex_wrap()
        .child(Button::new("Edit").icon(&icons::PEN).control_size(ControlSize::Small).disabled(busy)
            .help("Change the tag, title, notes or pre-release, and save the draft again")
            .on_press_tx(move |s: &mut App, tx| {
                s.edit_draft(i, id);
                tx.scroll_into_view("release-form");
            }).build(k).key(format!("release-edit-{id}")))
        .child(Button::new(if busy { "Publishing…" } else { "Publish" }).icon(&icons::TAG).control_size(ControlSize::Small).disabled(busy || blocked.is_some())
            .help("Publish this draft: it asks first")
            .on_press(move |s: &mut App| s.releases.confirm = Some(id)).build(k).key(format!("release-publish-{id}")))
        .child(Button::new("Add files on GitHub").icon(&icons::EXTERNAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .help("RepoHub doesn't upload files: add them to the draft on GitHub")
            .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&files_url); }).build(k).key(format!("release-add-files-{id}"))));
    if let Some(b) = blocked {
        out = out.child(text(b).font(k.ty.small).color(k.c.warning.icon).wrap().key(format!("release-blocked-{id}")));
    }
    out
}

/// The tags that have no release, each with Draft a release.
fn tags_panel(s: &App, k: &Kit, i: usize, list: Option<&List>) -> Option<El> {
    let r = &s.data.repos[i];
    let l = list?;
    let listing = l.load.ready()?;
    let left = releases::without_release(&listing.tags, &listing.releases);
    if left.is_empty() {
        return None;
    }
    let count = if listing.tags_more { format!("{}+", left.len()) } else { left.len().to_string() };
    let mut p = panel(k).key("release-tags")
        .child(panel_head(k, "Tags without a release", Some(text(count).font(k.ty.small).color(k.c.text_muted))));
    for t in left {
        let name = t.name.clone();
        let mut line = row().w_full().px(16.0).py(8.0).gap(10.0).center_y().flex_wrap().key(format!("release-tag-{}", t.name))
            .child(ui::ico(&icons::TAG, 14.0, k.c.icon_muted))
            .child(text(t.name.clone()).font(k.ty.mono).color(k.c.text_base).selectable(true));
        if !t.sha.is_empty() {
            line = line.child(text(crate::commits::short(&t.sha).to_owned()).font(k.ty.mono).color(k.c.text_faint));
        }
        line = line.child(el().grow(1.0))
            .child(Button::new("Draft a release").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .help(format!("Open the New release form for {}", t.name))
                .on_press_tx(move |s: &mut App, tx| {
                    s.open_release_form(i, Some(name.clone()));
                    tx.scroll_into_view("release-form");
                }).build(k).key(format!("release-tag-draft-{}", t.name)));
        p = p.child(rule(k)).child(line);
    }
    if listing.tags_more {
        p = p.child(rule(k)).child(note(k, format!("GitHub has more tags: these are among the newest {}.", releases::TAGS), "release-tags-more".into())
            .child(pages::open_button(k, "Open on GitHub", &format!("{}/tags", r.url), "release-tags-github")));
    }
    if l.more {
        p = p.child(rule(k)).child(note(k, "Some of these may have a release further down the list: Load more to check.".into(), "release-tags-partial".into()));
    }
    Some(p)
}

/// Fills the form's tag (a version chip).
fn set_tag(s: &mut App, tag: &str) {
    if let Some(f) = s.releases.form.as_mut() {
        f.tag = TextEdit::new(tag);
        if matches!(f.submit, Submit::Failed(_)) {
            f.submit = Submit::Editing;
        }
    }
}

/// The New release form, or a draft being edited.
fn form_panel(s: &App, k: &Kit, i: usize, f: &Form) -> El {
    let r = &s.data.repos[i];
    let listing = s.releases.listing_of(&r.full_name);
    let sending = f.submit.is_sending();
    let blocked = actions::blocked(s);
    let tag = f.tag.text.trim().to_owned();
    let tag_use = releases::tag_use(&tag, listing, f.draft_id);
    let close = IconButton::new(&icons::CROSS).variant(IconButtonVariant::GhostMuted).label("Close the form")
        .on_press(|s: &mut App| s.cancel_release_form()).build(k).key("release-form-close");
    let mut body = col().w_full().padding(16.0).gap(12.0);

    // The tag, with the next versions to pick from.
    let mut tag_head = row().w_full().gap(8.0).center_y().flex_wrap()
        .child(text("Tag").font(k.ty.small_medium).color(k.c.text_base));
    if let Some(l) = listing {
        tag_head = tag_head.child(text("Next version:").font(k.ty.small).color(k.c.text_muted));
        for v in releases::next_versions(l.tag_names()) {
            let on = tag == v;
            let pick = v.clone();
            tag_head = tag_head.child(Button::new(v.clone()).variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline }).control_size(ControlSize::Small)
                .help(format!("Use the tag {v}")).on_press(move |s: &mut App| set_tag(s, &pick)).build(k).key(format!("release-chip-{v}")));
        }
    }
    body = body.child(tag_head)
        .child(TextInput::new(&f.tag).placeholder("v1.2.3").label("Tag")
            .on_edit(|s: &mut App| &mut s.releases.form.as_mut().expect("form open").tag).build(k).key("release-tag").w_full());
    let status = |msg: String, ink| text(msg).font(k.ty.small).color(ink).wrap().key("release-tag-status");
    let from = row().gap(8.0).center_y().flex_wrap().key("release-target-row");
    match &tag_use {
        TagUse::Existing => body = body.child(row().gap(6.0).center_y().child(ui::ico(&icons::CHECK, 14.0, k.c.success.icon))
            .child(status(format!("Uses the existing tag {tag}."), k.c.text_base))),
        TagUse::Taken(title, draft) => body = body.child(status(if *draft {
            format!("The draft “{title}” already uses {tag}: edit that one, or pick another tag.")
        } else {
            format!("“{title}” already uses {tag}: pick another tag.")
        }, k.c.warning.icon)),
        TagUse::New => body = body.child(from
            .child(text(format!("Creates the tag {tag} from")).font(k.ty.small).color(k.c.text_base))
            .child(actions::branch_button(s, k, i, BranchSlot::ReleaseTarget, &f.target, "release-target"))),
        TagUse::Invalid(why) => {
            if !tag.is_empty() {
                body = body.child(status((*why).to_owned(), k.c.text_muted));
            }
            body = body.child(from
                .child(text("A new tag is made from").font(k.ty.small).color(k.c.text_muted))
                .child(actions::branch_button(s, k, i, BranchSlot::ReleaseTarget, &f.target, "release-target")));
        }
    }
    if !matches!(tag_use, TagUse::Existing | TagUse::Taken(..)) {
        body = body.child(actions::branch_panel(s, k, i, BranchSlot::ReleaseTarget, &f.target));
    }

    // Title and notes.
    let title_hint = if tag.is_empty() { "Title".to_owned() } else { format!("Title (empty: {tag})") };
    body = body.child(TextInput::new(&f.title).placeholder(title_hint).label("Title")
        .on_edit(|s: &mut App| &mut s.releases.form.as_mut().expect("form open").title).build(k).key("release-title").w_full());
    let asking = f.notes_job == NotesJob::Asking;
    let generate = Button::new(if asking { "Generating…" } else { "Generate notes" }).icon(&icons::SPARK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(asking || (blocked.is_some() && !s.data.sample))
        .help("Ask GitHub for the notes since the previous release: the pull requests merged and who made them")
        .on_press(|s: &mut App| s.generate_notes()).build(k).key("release-generate");
    body = body.child(row().w_full().gap(8.0).center_y().justify_between().flex_wrap()
            .child(text("Notes").font(k.ty.small_medium).color(k.c.text_base))
            .child(generate))
        .child(TextArea::new(&f.notes).placeholder("What changed, in Markdown").label("Notes").auto_grow(6, 16)
            .on_edit(|s: &mut App| &mut s.releases.form.as_mut().expect("form open").notes).build(k).key("release-notes").w_full());
    if f.offered.is_some() {
        body = body.child(row().w_full().gap(8.0).center_y().flex_wrap().padding(10.0).radius(k.radius.md).fill(k.c.info.icon.alpha(0.08))
            .role("alertdialog").label("GitHub's notes are ready. Replace what you typed?").key("release-offered")
            .child(text("GitHub's notes are ready. Replace what you typed?").font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink().min_w(160.0))
            .child(Button::new("Replace").control_size(ControlSize::Small).on_press(|s: &mut App| s.take_offered_notes(true)).build(k).key("release-offered-replace"))
            .child(Button::new("Keep mine").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).on_press(|s: &mut App| s.take_offered_notes(false)).build(k).key("release-offered-keep")));
    }
    if let NotesJob::Failed(e) = &f.notes_job {
        body = body.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap().key("release-notes-error"));
    }
    let pre = f.prerelease;
    // The words go next to the switch, where they can wrap on a phone; the switch keeps them as
    // its accessible label.
    const PRE: &str = "Pre-release: not ready for everyone, never shown as the latest";
    body = body.child(row().w_full().gap(10.0).center_y()
        .child(Switch::new(pre).label(PRE).hide_label()
            .on_toggle(|s: &mut App| if let Some(f) = s.releases.form.as_mut() { f.prerelease = !f.prerelease; }).build(k).key("release-pre"))
        .child(text(PRE).font(k.ty.small).color(k.c.text_base).wrap().grow(1.0).shrink().min_w(0.0)));

    // Why it can't be sent: GitHub's refusal first, then the account, then the form's own rule.
    let save_check = f.release(false, listing);
    let publish_check = f.release(true, listing);
    let note = match (&f.submit, blocked, &save_check) {
        (Submit::Failed(e), _, _) => Some((e.clone(), k.c.danger.icon)),
        (_, Some(b), _) => Some((b.to_owned(), k.c.warning.icon)),
        (_, None, Err(e)) if !tag.is_empty() && !matches!(tag_use, TagUse::Taken(..) | TagUse::Invalid(_)) => Some((e.clone(), k.c.text_muted)),
        _ => None,
    };
    if let Some((msg, ink)) = note {
        body = body.child(text(msg).font(k.ty.small).color(ink).wrap().key("release-form-note"));
    }
    let question = Confirm::new("release-confirm", &icons::TAG, releases::publish_question(&tag, &f.target, tag_use == TagUse::Existing))
        .open(f.confirm)
        .disabled(sending || publish_check.is_err() || blocked.is_some())
        .yes("Publish", Tone::Go, |s: &mut App| s.send_release(true))
        .on_no(|s: &mut App| if let Some(f) = s.releases.form.as_mut() { f.confirm = false; })
        .show(s, k);
    let foot = if let Some(q) = question {
        q
    } else {
        let (save_label, publish_label) = match (sending, f.publishing) {
            (true, false) => ("Saving…", "Publish release"),
            (true, true) => ("Save draft", "Publishing…"),
            _ => ("Save draft", "Publish release"),
        };
        row().w_full().gap(8.0).justify_end().flex_wrap()
            .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.cancel_release_form()).build(k).key("release-cancel"))
            .child(Button::new(save_label).control_size(ControlSize::Small)
                .disabled(sending || save_check.is_err() || blocked.is_some())
                .help("Save it as a draft: only people who can push see it, and nothing is published")
                .on_press(|s: &mut App| s.send_release(false)).build(k).key("release-save"))
            .child(Button::new(publish_label).icon(&icons::TAG).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                .disabled(sending || publish_check.is_err() || blocked.is_some())
                .help("Publish it: it asks first")
                .on_press(|s: &mut App| s.send_release(true)).build(k).key("release-publish"))
    };
    panel(k).key("release-form").border(1.0, k.c.border_strong)
        .child(panel_head(k, if f.draft_id.is_some() { "Edit draft" } else { "New release" }, Some(close)))
        .child(rule(k))
        .child(body.child(foot))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rune::TextEdit;
    use serde_json::{Value, json};

    use crate::releases::{BLOCKING, mock};
    use crate::state::{Load, RepoTab, Submit};
    use crate::{App, Page};

    /// The sample data, with repository `repo` open.
    fn app(compact: bool, repo: usize) -> rune::Runtime<App> {
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data_changed();
        s.pending_place = None;
        s.compact = compact;
        s.open(repo);
        let size = if compact { (390.0, 2400.0) } else { (1180.0, 2400.0) };
        let mut rt = rune::Runtime::new(s, crate::view, size, rune::Env::new(true, 1.0));
        rt.on_key = Some(crate::on_key);
        rt.frame(0.0);
        rt
    }

    thread_local! {
        static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }

    /// Moves the clock on, so what moved on screen (a form opening above the list) settles
    /// before the next press.
    fn settle(rt: &mut rune::Runtime<App>) {
        for _ in 0..3 {
            let t = CLOCK.with(|c| {
                c.set(c.get() + 0.4);
                c.get()
            });
            rt.frame(t);
        }
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        settle(rt);
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        settle(rt);
    }

    fn esc(rt: &mut rune::Runtime<App>) {
        rt.key(rune::KeyEvent { key: rune::Key::Escape, mods: rune::Mods::default() });
        settle(rt);
    }

    fn texts(rt: &rune::Runtime<App>) -> Vec<String> {
        rt.elements().iter().filter_map(|e| e.text.clone()).collect()
    }

    fn has_text(rt: &rune::Runtime<App>, t: &str) -> bool {
        texts(rt).iter().any(|x| x == t)
    }

    fn toast(rt: &rune::Runtime<App>) -> String {
        rt.state.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
    }

    #[test]
    fn the_header_opens_the_releases_with_notes_files_and_tags_without_a_release() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(false, 0);
        assert_eq!(rt.state.tab, RepoTab::Code);
        assert!(has_text(&rt, "v1.2.0"), "the header shows the latest release's tag");
        press(&mut rt, "latest-release");
        assert_eq!(rt.state.tab, RepoTab::Releases, "it opens the section");
        assert!(rt.find("release-3003").is_some() && rt.find("release-3002").is_some() && rt.find("release-3001").is_some());
        for word in ["Draft", "Pre-release", "Latest"] {
            assert!(has_text(&rt, word), "{word}");
        }
        assert!(has_text(&rt, "3 releases") && rt.find("releases-end").is_some());
        // Long notes are cut, and Show all shows the rest.
        assert!(!has_text(&rt, "Hero images are 420 KB smaller."));
        press(&mut rt, "release-more-3001");
        assert!(has_text(&rt, "Hero images are 420 KB smaller.") && has_text(&rt, "Show less"));
        // Files, with their size and downloads; the draft says where files are added.
        let file = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("release-file-3001-0")).expect("a file");
        assert_eq!(file.label.as_deref(), Some("Download site-v1.2.0.zip (2.0 MB) in the browser"));
        assert!(has_text(&rt, "148 downloads") && rt.find("release-add-files-3003").is_some());
        // Sample data: nothing is published, and it says why.
        assert!(rt.find("release-blocked-3003").is_some());
        // Two tags have no release; Draft a release opens the form with that tag, and no branch.
        assert!(rt.find("release-tag-v1.1.0").is_some() && rt.find("release-tag-v1.0.0").is_some() && rt.find("release-tag-v1.2.0").is_none());
        press(&mut rt, "release-tag-draft-v1.1.0");
        let f = rt.state.releases.form.as_ref().expect("the form opened");
        assert_eq!(f.tag.text, "v1.1.0");
        assert!(has_text(&rt, "Uses the existing tag v1.1.0.") && rt.find("release-target").is_none(), "an existing tag needs no branch");
        // The next versions fill the tag; a new one is made from the default branch.
        assert!(rt.find("release-chip-v1.3.1").is_some() && rt.find("release-chip-v2.0.0").is_some());
        press(&mut rt, "release-chip-v1.4.0");
        assert_eq!(rt.state.releases.form.as_ref().unwrap().tag.text, "v1.4.0");
        assert!(has_text(&rt, "Creates the tag v1.4.0 from") && rt.find("release-target").is_some());
        // The draft's tag is taken.
        rt.update(|s, _| s.releases.form.as_mut().unwrap().tag = TextEdit::new("v1.3.0"));
        settle(&mut rt);
        assert!(texts(&rt).iter().any(|t| t.starts_with("The draft “Notes widget and window snapping” already uses v1.3.0")));
        // Generate notes works on the sample; saving says why it can't.
        rt.update(|s, _| s.releases.form.as_mut().unwrap().tag = TextEdit::new("v1.4.0"));
        settle(&mut rt);
        press(&mut rt, "release-generate");
        assert!(rt.state.releases.form.as_ref().unwrap().notes.text.contains("## What's Changed"));
        assert!(texts(&rt).iter().any(|t| t.starts_with("This is sample data.")));
        press(&mut rt, "release-publish");
        assert!(rt.find("release-confirm").is_none(), "a disabled Publish asks nothing");
        // Edit fills the form with the draft (the tag picked in the one open is thrown away only
        // after asking); Esc closes it.
        press(&mut rt, "release-edit-3003");
        assert!(rt.find("discard-ask").is_some(), "the tag picked isn't thrown away without asking");
        press(&mut rt, "discard-yes");
        let f = rt.state.releases.form.as_ref().unwrap();
        assert_eq!((f.draft_id, f.tag.text.as_str(), f.title.text.as_str()), (Some(3003), "v1.3.0", "Notes widget and window snapping"));
        assert!(has_text(&rt, "Edit draft"));
        esc(&mut rt);
        if rt.state.releases.form.is_some() {
            esc(&mut rt);
        }
        assert!(rt.state.releases.form.is_none() && rt.state.tab == RepoTab::Releases);
        // The palette opens the form too.
        rt.update(|s, tx| crate::palette::run(s, crate::palette::Command::NewRelease(0), tx));
        settle(&mut rt);
        assert!(rt.state.releases.form.as_ref().is_some_and(|f| f.draft_id.is_none()) && rt.find("release-form").is_some());
    }

    #[test]
    fn draft_with_generated_notes_edit_and_publish_only_after_the_question() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _tmp = crate::load_gen_tests::TempDataDir::new("releases-write");
        let full = "francescodibella/francescodibella.github.io";
        let base = format!("/repos/{full}");
        let (url, seen) = mock::serve(move |method, path, body| {
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let p = path.strip_prefix(base.as_str()).unwrap_or(path);
            match (method, p) {
                ("GET", "/releases?per_page=20&page=1") => (200, String::new(), json!([mock::release(77, "v1.3.0", true, false, "Draft notes"), mock::release(76, "v1.2.0", false, false, "Notes")]).to_string()),
                ("GET", "/tags?per_page=50") => (200, String::new(), json!([{ "name": "v1.2.0", "commit": { "sha": "b".repeat(40) } }, { "name": "v1.1.0", "commit": { "sha": "c".repeat(40) } }]).to_string()),
                ("GET", "/releases/latest") => (200, String::new(), mock::release(76, "v1.2.0", false, false, "Notes").to_string()),
                ("POST", "/releases/generate-notes") => (200, String::new(), json!({ "name": "v1.4.0", "body": "## What's Changed\n* Releases by @ada" }).to_string()),
                ("POST", "/releases") if v["tag_name"] == "v9.9.9" => (422, String::new(), json!({ "message": "Validation Failed", "errors": [{ "code": "already_exists", "field": "tag_name" }] }).to_string()),
                ("POST", "/releases") => (201, String::new(), mock::release(90, v["tag_name"].as_str().unwrap_or(""), v["draft"].as_bool().unwrap_or(true), false, v["body"].as_str().unwrap_or("")).to_string()),
                ("PATCH", "/releases/90") => (200, String::new(), mock::release(90, v["tag_name"].as_str().unwrap_or("v1.4.0"), v["draft"].as_bool().unwrap_or(false), false, v["body"].as_str().unwrap_or("")).to_string()),
                ("PATCH", "/releases/77") => (200, String::new(), mock::release(77, "v1.3.0", false, false, "Draft notes").to_string()),
                _ => (404, String::new(), json!({ "message": "Not Found" }).to_string()),
            }
        });
        BLOCKING.with(|b| b.set(true));
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data.sample = false;
        s.data_changed();
        s.cfg = std::sync::Arc::new(mock::config(&url));
        s.pending_place = None;
        s.page = Page::Repo(full.into());
        s.set_tab(0, RepoTab::Releases);
        assert!(s.releases.listing_of(full).is_some_and(|l| l.releases.len() == 2), "read from GitHub");
        assert_eq!(s.releases.latest_id(full), Some(76));
        let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
        rt.on_key = Some(crate::on_key);
        rt.frame(0.0);
        assert!(rt.find("latest-release").is_some() && has_text(&rt, "v1.2.0"));
        let patches = |seen: &mock::Seen| seen.lock().unwrap().iter().filter(|(m, _)| m.starts_with("PATCH")).count();

        // A new draft: a version chip, your notes kept until you say, then GitHub's.
        press(&mut rt, "release-new");
        press(&mut rt, "release-chip-v1.4.0");
        rt.update(|s, _| s.releases.form.as_mut().unwrap().notes = TextEdit::new("My own notes"));
        press(&mut rt, "release-generate");
        assert!(rt.find("release-offered").is_some(), "typed notes aren't replaced without asking");
        press(&mut rt, "release-offered-keep");
        assert_eq!(rt.state.releases.form.as_ref().unwrap().notes.text, "My own notes");
        press(&mut rt, "release-generate");
        press(&mut rt, "release-offered-replace");
        assert_eq!(rt.state.releases.form.as_ref().unwrap().notes.text, "## What's Changed\n* Releases by @ada");
        press(&mut rt, "release-generate");
        assert!(rt.find("release-offered").is_none(), "GitHub's own notes are replaced without asking");
        let asked = seen.lock().unwrap().iter().find(|(m, _)| m.ends_with("/releases/generate-notes")).map(|(_, b)| serde_json::from_str::<Value>(b).unwrap()).unwrap();
        assert_eq!((asked["tag_name"].as_str(), asked["target_commitish"].as_str()), (Some("v1.4.0"), Some("main")));
        // Save draft doesn't ask.
        press(&mut rt, "release-save");
        assert!(rt.state.releases.form.is_none(), "the form closes");
        assert_eq!(toast(&rt), "Draft saved");
        let listing = rt.state.releases.listing_of(full).unwrap();
        assert!(listing.releases[0].id == 90 && listing.releases[0].draft, "the list shows it at once");
        assert_eq!(rt.state.releases.latest_id(full), Some(76), "a draft isn't the latest");

        // Edit it, and publish: the question comes first, and nothing is sent until it's answered.
        press(&mut rt, "release-edit-90");
        let f = rt.state.releases.form.as_ref().unwrap();
        assert_eq!((f.draft_id, f.tag.text.as_str()), (Some(90), "v1.4.0"));
        press(&mut rt, "release-publish");
        assert!(rt.find("release-confirm").is_some() && has_text(&rt, "Publish v1.4.0 from main? This creates the tag and can't be undone from RepoHub."));
        assert_eq!(patches(&seen), 0, "nothing published yet");
        press(&mut rt, "release-confirm-no");
        assert!(rt.find("release-confirm").is_none() && rt.state.releases.form.is_some());
        press(&mut rt, "release-publish");
        esc(&mut rt);
        assert!(rt.find("release-confirm").is_none() && rt.state.releases.form.is_some(), "Esc answers no first");
        assert_eq!(patches(&seen), 0);
        press(&mut rt, "release-publish");
        press(&mut rt, "release-confirm-yes");
        assert_eq!(patches(&seen), 1);
        let sent = seen.lock().unwrap().iter().find(|(m, _)| m.ends_with("/releases/90")).map(|(_, b)| serde_json::from_str::<Value>(b).unwrap()).unwrap();
        assert_eq!((sent["draft"].as_bool(), sent["tag_name"].as_str()), (Some(false), Some("v1.4.0")));
        assert_eq!(toast(&rt), "Published v1.4.0");
        assert!(rt.state.releases.form.is_none());
        let listing = rt.state.releases.listing_of(full).unwrap();
        assert!(!listing.releases[0].draft && listing.tags.iter().any(|t| t.name == "v1.4.0"), "published, and its tag exists now");
        assert_eq!(rt.state.releases.latest_id(full), Some(90), "the header follows");
        settle(&mut rt);
        let pill = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("latest-release")).expect("the header's tag");
        assert_eq!(pill.label.as_deref(), Some("Latest release v1.4.0: open Releases"));

        // GitHub refuses: the form stays with what was typed, and says why.
        press(&mut rt, "release-new");
        rt.update(|s, _| {
            let f = s.releases.form.as_mut().unwrap();
            f.tag = TextEdit::new("v9.9.9");
            f.title = TextEdit::new("Mine");
            f.notes = TextEdit::new("Typed notes");
        });
        settle(&mut rt);
        press(&mut rt, "release-save");
        let f = rt.state.releases.form.as_ref().expect("still open");
        assert_eq!((f.tag.text.as_str(), f.title.text.as_str(), f.notes.text.as_str()), ("v9.9.9", "Mine", "Typed notes"));
        assert!(matches!(&f.submit, Submit::Failed(e) if e.contains("already has a release")));
        assert!(texts(&rt).iter().any(|t| t.contains("This tag already has a release")));
        press(&mut rt, "release-form-close");

        // A draft in the list publishes after its own question.
        press(&mut rt, "release-publish-77");
        assert!(rt.find("release-confirm-77").is_some() && has_text(&rt, "Publish v1.3.0 from main? This creates the tag and can't be undone from RepoHub."));
        assert_eq!(patches(&seen), 1);
        press(&mut rt, "release-publish-no-77");
        assert!(rt.find("release-confirm-77").is_none() && patches(&seen) == 1);
        press(&mut rt, "release-publish-77");
        press(&mut rt, "release-publish-yes-77");
        assert_eq!(patches(&seen), 2);
        assert_eq!(toast(&rt), "Published v1.3.0");
        assert!(rt.state.releases.listing_of(full).unwrap().releases.iter().all(|r| !r.draft));
        assert!(!seen.lock().unwrap().iter().any(|(m, _)| m.starts_with("DELETE")), "nothing is deleted");
        BLOCKING.with(|b| b.set(false));
    }

    #[test]
    fn loading_failed_and_empty_say_so_with_retry() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut rt = app(false, 0);
        rt.update(|s, _| s.set_tab(0, RepoTab::Releases));
        settle(&mut rt);
        rt.update(|s, _| s.releases.list.as_mut().unwrap().load = Load::Loading);
        settle(&mut rt);
        assert!(rt.find("skeleton-releases").is_some() && rt.find("release-new").is_some());
        rt.update(|s, _| s.releases.list.as_mut().unwrap().load = Load::Failed("GitHub error 500: Server error".into()));
        settle(&mut rt);
        assert!(rt.find("releases-failed").is_some() && has_text(&rt, "GitHub error 500: Server error"));
        press(&mut rt, "retry-releases");
        assert!(rt.state.releases.list.as_ref().and_then(|l| l.load.ready()).is_some_and(|l| l.releases.len() == 3), "Retry reads them again");
        // More on GitHub: Load more, and the tags say some may have a release further down.
        rt.update(|s, _| s.releases.list.as_mut().unwrap().more = true);
        settle(&mut rt);
        assert!(rt.find("releases-load-more").is_some() && rt.find("release-tags-partial").is_some() && has_text(&rt, "3+ releases"));
        // A repository without releases: a sentence, no tags panel, and v0.1.0 to start with.
        let mut rt = app(false, 4);
        rt.update(|s, _| s.set_tab(4, RepoTab::Releases));
        settle(&mut rt);
        assert!(rt.find("releases-empty").is_some() && has_text(&rt, "No releases yet."));
        assert!(rt.find("release-tags").is_none() && rt.find("latest-release").is_none(), "no tags, and no latest in the header");
        press(&mut rt, "release-new");
        assert!(rt.find("release-chip-v0.1.0").is_some() && rt.find("release-chip-v0.1.1").is_none());
    }

    #[test]
    fn on_a_phone_nothing_overflows_and_the_section_bar_shows_releases() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = crate::init();
        s.data = crate::model::Data::sample();
        s.data_changed();
        s.pending_place = None;
        s.compact = true;
        s.open(0);
        s.set_tab(0, RepoTab::Releases);
        let n = s.tabs().iter().position(|t| *t == RepoTab::Releases).unwrap();
        let mut rt = rune::Runtime::new(s, crate::view, (390.0, 2400.0), rune::Env::new(true, 1.0));
        rt.poll = Some(crate::poll);
        rt.frame(0.0);
        for t in 1..6 {
            rt.frame(t as f64 * 0.1);
        }
        let seg = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(&format!("segment{n}"))).expect("the Releases segment");
        assert!(seg.rect[0] >= -0.5 && seg.rect[0] + seg.rect[2] <= 390.5, "Releases is scrolled into view: {:?}", seg.rect);
        rt.update(|s, _| s.open_release_form(0, None));
        settle(&mut rt);
        // Nothing goes past the panels' right edge, which ends inside the window.
        let form = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("release-form")).expect("the form");
        let right = form.rect[0] + form.rect[2];
        assert!(right <= 390.5);
        let mut checked = 0;
        for e in rt.elements() {
            if let Some(key) = e.key.as_ref().filter(|k| (k.starts_with("release") && k.as_str() != "release-form" && k.as_str() != "releases") || k.as_str() == "latest-release") {
                checked += 1;
                assert!(e.rect[0] >= -0.5 && e.rect[0] + e.rect[2] <= right + 0.5, "{key} overflows: {:?} (the panels end at {right})", e.rect);
            }
        }
        assert!(checked > 20, "{checked}");
    }
}
