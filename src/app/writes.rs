//! Writes to GitHub and what they say: the toasts (with Undo), the forms, `write` and `apply_done`.

use super::*;

impl App {
    /// Shows a short message at the bottom of the window; it goes away after a few seconds.
    pub(crate) fn notify(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Done, 4500);
    }

    /// Like `notify`, with **Undo**; shown longer, so there's time to press it.
    pub(super) fn notify_undo(&mut self, msg: impl Into<String>, undo: state::Undo) {
        self.show_toast(msg.into(), ToastKind::Done, 9000);
        if let Some(t) = self.toast.as_mut() {
            t.undo = Some(undo);
        }
    }

    /// Puts back what the toast's **Undo** offers.
    pub(crate) fn undo(&mut self) {
        let Some(undo) = self.toast.take().and_then(|t| t.undo) else { return };
        match undo {
            state::Undo::Look { look, at, light, dark } => {
                let name = look.name.clone();
                let id = look.id.clone();
                let at = at.min(self.looks.all.len());
                self.looks.all.insert(at, look);
                if light {
                    self.looks.light = id.clone();
                }
                if dark {
                    self.looks.dark = id;
                }
                self.looks.save();
                self.notify(format!("Restored {name}"));
            }
            state::Undo::Reopen { repo, issue } => {
                let number = issue.number;
                // It comes back to the list once GitHub has reopened it.
                self.reopening.push(issue);
                self.write(&repo, github::Write::ReopenIssue(number));
            }
            state::Undo::ReopenPull { repo, number } => self.write(&repo, github::Write::SetPullOpen { number, open: true }),
            state::Undo::Close { repo, number } => self.write(&repo, github::Write::CloseIssue(number)),
            state::Undo::ClosePull { repo, number } => self.write(&repo, github::Write::SetPullOpen { number, open: false }),
            state::Undo::Star { repo } => {
                if let Some(i) = self.repo_index_of(&repo) {
                    self.toggle_star(i);
                }
            }
            state::Undo::Pin { repo, at } => self.repin(&repo, at),
            state::Undo::Harness { harness, at } => {
                let name = harness.name.clone();
                let at = at.min(self.assist.harnesses.len());
                self.assist.harnesses.insert(at, harness);
                self.assist.save();
                self.notify(format!("Restored {name}"));
            }
        }
    }

    /// Like `notify`, for something that went wrong: marked as a problem and shown longer.
    pub(crate) fn notify_problem(&mut self, msg: impl Into<String>) {
        self.show_toast(msg.into(), ToastKind::Problem, 8000);
    }

    fn show_toast(&mut self, text: String, kind: ToastKind, ms: u32) {
        self.toast_id += 1;
        let id = self.toast_id;
        self.toast = Some(Toast { id, text, kind, undo: None });
        if let Some(h) = self.handle.clone() {
            net::spawn(h, move || net::sleep(ms), move |s: &mut App, _| {
                if s.toast.as_ref().is_some_and(|t| t.id == id) {
                    s.toast = None;
                }
            });
        }
    }

    /// Opens form `kind` for repository `i` on the section it belongs to. A form with typed
    /// text (of any repository) isn't replaced without asking.
    pub(crate) fn open_form(&mut self, i: usize, kind: Compose) {
        if self.ask_first(&[state::Draft::Form], move |s| s.open_form(i, kind)) {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, default, shown) = (r.full_name.clone(), r.branch.clone(), self.branch(i));
        // A pull request starts from the branch on screen, or the first other branch.
        let head = if shown != default {
            shown.clone()
        } else {
            self.branches.get(&full).and_then(Load::ready).and_then(|b| b.iter().find(|n| **n != default).cloned()).unwrap_or(shown.clone())
        };
        self.issue_picks = triage::Picks::for_repo(&full);
        self.form = Some(actions::Form {
            repo: full,
            kind,
            title: TextEdit::new(""),
            body: TextEdit::new(""),
            from: if kind == Compose::Pull { head } else { shown },
            into: default,
            draft: false,
            submit: Submit::Editing,
        });
        self.picker = None;
        if self.tab != kind.tab() {
            self.set_tab(i, kind.tab());
        }
        self.load_branches(i, false);
        // `set_tab` tidies up the section it leaves; the form opened for the new one stays.
        if let Some(f) = self.form.as_mut() {
            f.title.focused = true;
        }
    }

    /// Sends the open form.
    pub(crate) fn submit_form(&mut self) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some(f) = self.form.as_mut() else { return };
        let Ok(w) = f.write() else { return };
        let w = triage::with_picks(w, &self.issue_picks);
        if !f.submit.start() {
            return;
        }
        let repo = f.repo.clone();
        let repo_cb = repo.clone();
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let (epoch, landed) = (self.load_gen, self.landed);
        // The new item's row comes from this write's own answer (`github::Created`).
        net::spawn(h, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| s.form_sent_since(epoch, &repo_cb, landed, r));
    }

    /// `form_sent_since` for a write sent when the lists on screen were loaded.
    #[cfg(test)]
    pub(crate) fn form_sent(&mut self, epoch: u64, repo: &str, r: Result<github::Done, String>) {
        self.form_sent_since(epoch, repo, self.landed, r);
    }

    /// The form's write finished: the form closes, or stays with what was typed and GitHub's
    /// reason. A form opened since (or none, after Cancel) is left as it is, and a failure is
    /// then said in a toast. It was sent when `landed` loads had landed (see `apply_done_since`).
    pub(crate) fn form_sent_since(&mut self, epoch: u64, repo: &str, landed: u64, r: Result<github::Done, String>) {
        if self.load_gen != epoch {
            return;
        }
        let ours = self.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(repo) && f.submit.is_sending());
        match r {
            Ok(done) => {
                if ours {
                    self.form = None;
                    self.picker = None;
                }
                self.apply_done_since(repo, done, landed);
            }
            Err(e) => match self.form.as_mut().filter(|_| ours) {
                Some(f) => f.submit = Submit::Failed(e),
                None => self.notify_problem(e),
            },
        }
    }

    /// Runs a row's write (close an issue, merge a pull request, re-run a workflow) on repository `repo`.
    pub(crate) fn write(&mut self, repo: &str, w: github::Write) {
        if actions::blocked(self).is_some() {
            return;
        }
        let full = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        let number = w.number().unwrap_or(0);
        let rerun_id = match &w {
            github::Write::RerunRun { run_id, .. } => Some(*run_id),
            _ => None,
        };
        // A triage change: what it's about, and whether it takes several requests.
        let triage = match &w {
            github::Write::Triage { change, .. } => Some((change.part(), change.requests(&full, number).len() > 1)),
            _ => None,
        };
        let key = match (rerun_id, triage) {
            (Some(run_id), _) => runs_page::run_busy_key(&full, run_id),
            (None, Some(_)) => triage::busy_key(repo, number),
            (None, None) => actions::busy_key(repo, number),
        };
        if !self.busy.insert(key.clone()) {
            return;
        }
        let full_cb = full.clone();
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let (epoch, landed) = (self.load_gen, self.landed);
        net::spawn(h, move || async move { github::run(&cfg, &full, w).await }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.busy.remove(&key);
            // What was typed stays in the box when sending fails.
            if let Some(t) = s.thread.as_mut().filter(|t| t.is(&full_cb, number)) {
                if triage.is_some() {
                    t.edit.saving = None;
                } else {
                    t.sending = false;
                }
            }
            if s.pull_diff_of.as_ref().is_some_and(|(f, n)| *n == number && same_repo(f, &full_cb))
                && let Some(d) = s.pull_review.as_mut().and_then(|r| r.draft.as_mut())
            {
                d.sending = false;
            }
            match r {
                Ok(done) => s.apply_done_since(&full_cb, done, landed),
                // A refused triage change is said where it was made, once.
                Err(e) if triage.is_some_and(|(part, partial)| s.triage_failed(&full_cb, number, part, partial, &e)) => {}
                Err(e) => {
                    if let Some(run_id) = rerun_id
                        && let Some(o) = s.open_run.as_mut().filter(|o| o.run_id == run_id && same_repo(&o.repo, &full_cb))
                    {
                        o.write_error = Some(e.clone());
                    }
                    s.notify_problem(e);
                }
            }
        });
    }

    /// Shows what a write changed at once, without waiting for GitHub's lists to catch up.
    pub(crate) fn apply_done(&mut self, repo: &str, done: github::Done) {
        self.apply_done_since(repo, done, self.landed);
    }

    /// [`App::apply_done`] for a write that started when `landed` loads had landed: if one
    /// landed since, the lists on screen were read while it ran and may already show it, so
    /// its change is applied exactly (see `replay`).
    pub(crate) fn apply_done_since(&mut self, repo: &str, done: github::Done, landed: u64) {
        let about = match &done {
            github::Done::Closed(n) | github::Done::Reopened(n) | github::Done::Merged(n) | github::Done::PullOpen(n, _) | github::Done::Commented(n) | github::Done::Reviewed(n, _) => Some(*n),
            _ => None,
        };
        let sent = matches!(done, github::Done::Commented(_) | github::Done::Reviewed(..));
        self.done_effects(repo, done, landed != self.landed);
        // Counts, states and lists changed in place: the cached lists follow.
        self.data_changed();
        // The conversation on screen shows the change: read it again.
        if let Some(n) = about
            && let Some(t) = self.thread.as_mut().filter(|t| t.is(repo, n))
        {
            if sent {
                t.reply.clear();
            }
            self.load_thread();
        }
        self.sync_tray();
    }

    /// What a finished write changes in the lists: shown now, and kept while a load runs so the
    /// load doesn't undo it when it lands (`apply` puts it back, exactly).
    pub(crate) fn changed(&mut self, c: replay::Change, exact: bool) {
        let me = self.data.profile.login.clone();
        replay::apply(&mut self.data, &c, &me, exact);
        if self.status == Status::Loading {
            self.replays.push(c);
        }
    }

    /// A load landed: what the writes that finished while it ran changed goes back on top of
    /// what it brought. A change it already shows does nothing.
    pub(crate) fn replay_writes(&mut self) {
        let me = self.data.profile.login.clone();
        for c in std::mem::take(&mut self.replays) {
            if replay::apply(&mut self.data, &c, &me, true) {
                state::trace("replay", &"load", &c.repo());
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn apply_done_inner(&mut self, repo: &str, done: github::Done) {
        self.done_effects(repo, done, false);
    }

    /// The lists, the counts and the toast for a finished write; `exact`: the lists may already
    /// show it.
    fn done_effects(&mut self, repo: &str, done: github::Done, exact: bool) {
        let full_name = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        match done {
            github::Done::Issue(c) => {
                // The labels and assignees GitHub set; the form's picks when its answer doesn't say.
                let (labels, assignees) = match (c.labels, c.assignees) {
                    (Some(l), Some(a)) => (l, a),
                    _ => self.picked_for_new_issue(&full_name),
                };
                let number = c.number;
                self.changed(replay::Change::Opened(model::Issue { label: labels.first().cloned(), labels, assignees, author: self.data.profile.login.clone(), number, title: c.title, repo: full_name.clone(), created_at: self.now, comments: 0, url: c.url }), exact);
                self.notify(format!("Opened issue #{number} in {full_name}"));
            }
            github::Done::Branch(name) => {
                if let Some(Load::Ready(list)) = self.branches.get_mut(&full_name) {
                    list.push(name.clone());
                }
                self.notify(format!("Created branch {name}"));
                if let Some(curr) = self.repo_index_of(&full_name) {
                    self.set_branch(curr, name);
                } else if !full_name.is_empty() {
                    self.branch_of.insert(full_name, name);
                }
            }
            github::Done::Pull(c) => {
                self.commits.pull_opened(&full_name, c.number, &c.url);
                let author = self.data.profile.login.clone();
                let number = c.number;
                self.changed(replay::Change::OpenedPull(model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
                    number, title: c.title, repo: full_name.clone(), author,
                    state: if c.draft { PullState::Draft } else { PullState::Open },
                    checks: None, updated_at: self.now, comments: 0, url: c.url,
                }), exact);
                self.notify(format!("Opened pull request #{number}"));
            }
            github::Done::Closed(n) => {
                let closed = self.data.issues.iter().find(|x| x.number == n && same_repo(&x.repo, &full_name)).cloned();
                self.changed(replay::Change::Closed { repo: full_name.clone(), number: n }, exact);
                match closed {
                    Some(issue) => self.notify_undo(format!("Closed issue #{n}"), state::Undo::Reopen { repo: full_name.clone(), issue }),
                    None => self.notify(format!("Closed issue #{n}")),
                }
            }
            github::Done::Reopened(n) => {
                // An issue closed from the list and reopened with Undo goes back where it was.
                if let Some(at) = self.reopening.iter().position(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                    let issue = self.reopening.remove(at);
                    self.changed(replay::Change::Reopened(issue), exact);
                    self.notify(format!("Reopened issue #{n}"));
                } else {
                    // Reopened from its conversation: Undo closes it again.
                    self.notify_undo(format!("Reopened issue #{n}"), state::Undo::Close { repo: full_name.clone(), number: n });
                }
            }
            github::Done::Commented(n) => {
                for x in self.data.issues.iter_mut().filter(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                    x.comments += 1;
                }
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    p.comments += 1;
                }
                self.notify(format!("Comment added to #{n}"));
            }
            github::Done::LineCommented(n) => {
                if self.pull_diff_of.as_ref().is_some_and(|(f, num)| *num == n && same_repo(f, &full_name)) {
                    if let Some(r) = self.pull_review.as_mut() {
                        r.draft = None;
                    }
                    self.place_line_comments();
                    self.load_line_comments();
                }
                self.notify(format!("Comment added to pull request #{n}"));
            }
            github::Done::Reviewed(n, verdict) => self.notify(match verdict {
                thread::Verdict::Approved => format!("Approved pull request #{n}"),
                thread::Verdict::ChangesRequested => format!("Requested changes on pull request #{n}"),
                thread::Verdict::Commented => format!("Review sent on pull request #{n}"),
            }),
            github::Done::Merged(n) => {
                self.changed(replay::Change::Merged { repo: full_name.clone(), number: n }, exact);
                self.notify(format!("Merged pull request #{n}"));
            }
            github::Done::PullOpen(n, open) => {
                self.changed(replay::Change::PullOpen { repo: full_name.clone(), number: n, open }, exact);
                if open {
                    self.notify_undo(format!("Reopened pull request #{n}"), state::Undo::ClosePull { repo: full_name.clone(), number: n });
                } else {
                    self.notify_undo(format!("Closed pull request #{n}"), state::Undo::ReopenPull { repo: full_name.clone(), number: n });
                }
            }
            github::Done::Release(r) => self.release_done(&full_name, *r),
            github::Done::Triaged { number, change, applied } => {
                self.changed(replay::Change::Triaged { repo: full_name.clone(), number, applied: applied.clone() }, exact);
                self.apply_triaged(&full_name, number, change, applied);
            }
            github::Done::Reran { run_id, failed_only } => {
                self.changed(replay::Change::Reran { repo: full_name.clone(), run_id, at: self.now }, exact);
                let reload = if let Some(o) = self.open_run.as_mut().filter(|o| o.run_id == run_id && same_repo(&o.repo, &full_name)) {
                    o.checks = model::Checks::Running;
                    o.updated_at = self.now;
                    o.write_error = None;
                    true
                } else {
                    false
                };
                if reload {
                    self.reload_open_run();
                }
                self.notify(if failed_only { "Re-running failed jobs" } else { "Re-running workflow" });
            }
        }
    }
}
