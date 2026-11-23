//! What the app does with triage: the pickers opened and closed, the choices read, a change sent and
//! GitHub's answer applied to the conversation and the rows, and GitHub's refusals in words.

use super::*;

// ------------------------------------------------------------------ what the app does

impl App {
    pub(crate) fn picker_mut(&mut self, host: Host) -> Option<&mut Picker> {
        match host {
            Host::Thread => self.thread.as_mut().and_then(|t| t.edit.picker.as_mut()),
            Host::Form => self.issue_picks.picker.as_mut(),
        }
    }

    pub(crate) fn picker(&self, host: Host) -> Option<&Picker> {
        match host {
            Host::Thread => self.thread.as_ref().and_then(|t| t.edit.picker.as_ref()),
            Host::Form => self.issue_picks.picker.as_ref(),
        }
    }

    /// The repository a host's picker reads its choices from.
    pub(crate) fn picker_repo(&self, host: Host) -> Option<String> {
        match host {
            Host::Thread => self.thread.as_ref().map(|t| runs_page::full_repo_of(self, &t.repo)),
            Host::Form => self.form.as_ref().map(|f| f.repo.clone()),
        }
    }

    /// Opens the picker for `facet` (closing the one open), ticked as it is now, and asks for
    /// the repository's choices if they aren't here yet.
    pub(crate) fn open_picker(&mut self, host: Host, facet: Facet) {
        let current = match host {
            Host::Thread => {
                let Some(o) = self.thread.as_ref() else { return };
                let Load::Ready(t) = &o.load else { return };
                match facet {
                    Facet::Labels => t.labels.iter().map(|l| l.0.clone()).collect(),
                    Facet::Assignees => t.assignees.clone(),
                    Facet::Milestone => t.milestone.iter().map(|m| m.0.to_string()).collect(),
                    Facet::Reviewers => t.requested.clone(),
                }
            }
            Host::Form => self.issue_picks.values(facet),
        };
        let picker = Some(Picker::new(facet, current));
        match host {
            Host::Thread => if let Some(o) = self.thread.as_mut() {
                o.edit.picker = picker;
                o.edit.error = None;
            },
            Host::Form => self.issue_picks.picker = picker,
        }
        // Choices read before show at once and are read again behind the picker (with their tag,
        // so an unchanged list costs a 304).
        if let Some(repo) = self.picker_repo(host) {
            let again = matches!(self.choices.get(&repo, Source::of(facet)), Some(Load::Ready(_)));
            self.load_choices(&repo, Source::of(facet), again);
        }
    }

    pub(crate) fn close_picker(&mut self, host: Host) {
        match host {
            Host::Thread => if let Some(o) = self.thread.as_mut() { o.edit.picker = None },
            Host::Form => self.issue_picks.picker = None,
        }
    }

    pub(crate) fn toggle_choice(&mut self, host: Host, id: &str) {
        if let Some(p) = self.picker_mut(host) {
            p.toggle(id);
        }
    }

    /// Reads a repository's choices from GitHub (`again`: even if they're here), or the made-up
    /// ones of the sample data.
    pub(crate) fn load_choices(&mut self, full: &str, source: Source, again: bool) {
        if self.data.sample {
            if self.choices.get(full, source).is_none() {
                let c = sample_choices(source, &self.data);
                self.choices.put(full, source, c);
            }
            return;
        }
        let Some(epoch) = self.choices.ask(full, source, again) else { return };
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let full = full.to_owned();
        let asked = full.clone();
        crate::net::spawn(h, move || async move { github::load_choices(&cfg, &asked, source).await }, move |s: &mut App, r| {
            // Another account: everything was forgotten. A newer request: this one is old.
            if s.load_gen != account {
                return;
            }
            s.choices.answer(&full, source, epoch, r);
        });
    }

    /// Apply: sends the picker's change (or, in the New issue form, keeps it for the form).
    pub(crate) fn apply_picker(&mut self, host: Host) {
        let Some(repo) = self.picker_repo(host) else { return };
        let milestones = match self.choices.get(&repo, Source::Milestones) {
            Some(Load::Ready(c)) => c.items.clone(),
            _ => Vec::new(),
        };
        let Some(p) = self.picker(host).cloned() else { return };
        match host {
            Host::Form => {
                match p.facet {
                    Facet::Labels => self.issue_picks.labels = p.picked,
                    _ => self.issue_picks.assignees = p.picked,
                }
                self.issue_picks.picker = None;
            }
            Host::Thread => {
                let Some(change) = p.change(&milestones) else {
                    self.close_picker(host);
                    return;
                };
                self.send_change(change);
            }
        }
    }

    /// Sends a change of the open conversation; it says when GitHub answers.
    pub(crate) fn send_change(&mut self, change: Change) {
        if crate::actions::blocked(self).is_some() {
            return;
        }
        let Some(o) = self.thread.as_mut() else { return };
        if o.edit.saving.is_some() {
            return;
        }
        o.edit.saving = Some(change.part());
        o.edit.error = None;
        let (repo, number) = (o.repo.clone(), o.number);
        let w = github::Write::Triage { number, change };
        #[cfg(test)]
        self.choices.sent.push((repo.clone(), w.clone()));
        self.write(&repo, w);
    }

    pub(crate) fn edit_title(&mut self) {
        if let Some(o) = self.thread.as_mut()
            && let Load::Ready(t) = &o.load
        {
            let mut e = TextEdit::new(t.title.clone());
            e.focused = true;
            o.edit.title = Some(e);
            o.edit.error = None;
        }
    }

    pub(crate) fn edit_body(&mut self) {
        if let Some(o) = self.thread.as_mut()
            && let Load::Ready(t) = &o.load
        {
            let mut e = TextEdit::new(t.entries.first().map(|e| e.body.clone()).unwrap_or_default());
            e.focused = true;
            o.edit.body = Some(e);
            o.edit.error = None;
        }
    }

    /// Save on the title being edited; an unchanged title just closes the field.
    pub(crate) fn save_title(&mut self) {
        let Some(o) = self.thread.as_mut() else { return };
        let (Some(e), Load::Ready(t)) = (&o.edit.title, &o.load) else { return };
        let title = e.text.trim().to_owned();
        if title.is_empty() {
            return;
        }
        if title == t.title {
            o.edit.title = None;
            return;
        }
        self.send_change(Change::Title(title));
    }

    /// Save on the description being edited; an unchanged one just closes the editor.
    pub(crate) fn save_body(&mut self) {
        let Some(o) = self.thread.as_mut() else { return };
        let (Some(e), Load::Ready(t)) = (&o.edit.body, &o.load) else { return };
        let body = e.text.trim_end().to_owned();
        if t.entries.first().is_some_and(|d| d.body.trim_end() == body) {
            o.edit.body = None;
            return;
        }
        self.send_change(Change::Body(body));
    }

    /// The labels (with their colours) and assignees picked in the New issue form for `full`,
    /// taken for the row of the issue just created.
    pub(crate) fn picked_for_new_issue(&mut self, full: &str) -> (Vec<(String, f32)>, Vec<String>) {
        if !same_repo(&self.issue_picks.repo, full) {
            return (Vec::new(), Vec::new());
        }
        let picks = std::mem::take(&mut self.issue_picks);
        let labels = picks.labels.iter().map(|l| (l.clone(), self.choices.hue(full, l).unwrap_or(250.0))).collect();
        (labels, picks.assignees)
    }

    /// A finished change: the conversation shows it, and a toast says it.
    pub(crate) fn apply_triaged(&mut self, full: &str, number: u32, change: Change, applied: Applied) {
        if let Some(o) = self.thread.as_mut().filter(|o| o.is(full, number)) {
            o.edit.saving = None;
            o.edit.error = None;
            match &applied {
                Applied::Title(_) => o.edit.title = None,
                Applied::Body(_) => o.edit.body = None,
                _ => {
                    if o.edit.picker.as_ref().is_some_and(|p| p.facet.key() == change.part()) {
                        o.edit.picker = None;
                    }
                }
            }
            if let Load::Ready(t) = &mut o.load {
                match &applied {
                    Applied::Labels(l) => t.labels = l.clone(),
                    Applied::Assignees(a) => t.assignees = a.clone(),
                    Applied::Milestone(m) => t.milestone = m.clone(),
                    Applied::Reviewers(r) => t.requested = r.clone(),
                    Applied::Title(x) => t.title = x.clone(),
                    Applied::Body(b) => if let Some(d) = t.entries.first_mut() { d.body = b.clone() },
                }
            }
        }
        // The list rows are changed by `apply_done` (`replay::Change::Triaged`).
        let msg = applied.toast(number, &change);
        self.notify(msg);
    }

    /// A change GitHub refused: said once where it was made, and the old values stay. A change
    /// made of several requests (reviewers asked and taken back) may have half happened, so the
    /// conversation is read again. False when that conversation isn't open any more.
    pub(crate) fn triage_failed(&mut self, full: &str, number: u32, part: &'static str, partial: bool, e: &str) -> bool {
        let Some(o) = self.thread.as_mut().filter(|o| o.is(full, number)) else { return false };
        o.edit.saving = None;
        o.edit.error = Some((part, explain(e)));
        if partial {
            self.load_thread();
        }
        true
    }
}

/// GitHub's refusal, in words for the details block.
pub fn explain(e: &str) -> String {
    if e.contains("(403)") || e.contains("(404)") {
        format!("{e} Your token may not have triage or write access to this repository. Nothing was changed.")
    } else {
        format!("{e} Nothing was changed.")
    }
}
