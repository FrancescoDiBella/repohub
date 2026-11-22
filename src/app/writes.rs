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

    /// Opens form `kind` for repository `i` on the section it belongs to.
    pub(crate) fn open_form(&mut self, i: usize, kind: Compose) {
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
        // What the screen shows for the new item until the lists reload.
        self.form_title = Some(f.title.text.trim().to_owned());
        self.pull_meta = Some((f.title.text.trim().to_owned(), f.draft));
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| s.form_sent(epoch, &repo_cb, r));
    }

    /// The form's write finished: the form closes, or stays with what was typed and GitHub's
    /// reason. A form opened since (or none, after Cancel) is left as it is, and a failure is
    /// then said in a toast.
    pub(crate) fn form_sent(&mut self, epoch: u64, repo: &str, r: Result<github::Done, String>) {
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
                self.apply_done(repo, done);
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
        self.confirm_merge = None;
        let full_cb = full.clone();
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
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
                Ok(done) => s.apply_done(&full_cb, done),
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
        let about = match &done {
            github::Done::Closed(n) | github::Done::Reopened(n) | github::Done::Merged(n) | github::Done::PullOpen(n, _) | github::Done::Commented(n) | github::Done::Reviewed(n, _) => Some(*n),
            _ => None,
        };
        let sent = matches!(done, github::Done::Commented(_) | github::Done::Reviewed(..));
        self.apply_done_inner(repo, done);
        // Counts, states and lists changed in place: the cached lists follow.
        self.data_changed();
        self.data_gen = self.data_gen.wrapping_add(1);
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

    pub(crate) fn apply_done_inner(&mut self, repo: &str, done: github::Done) {
        let full_name = self.data.repos.iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(repo) || (!repo.contains('/') && r.name.eq_ignore_ascii_case(repo)))
            .map(|r| r.full_name.clone()).unwrap_or_else(|| repo.to_owned());
        match done {
            github::Done::Issue(c) => {
                let title = self.form_title.take().unwrap_or_default();
                let (labels, assignees) = self.picked_for_new_issue(&full_name);
                self.data.issues.insert(0, model::Issue { label: labels.first().cloned(), labels, assignees, number: c.number, title, repo: full_name.clone(), created_at: self.now, comments: 0, url: c.url });
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues += 1;
                }
                if let Some(t) = self.data.issues_total.as_mut() {
                    *t += 1;
                }
                self.notify(format!("Opened issue #{} in {full_name}", c.number));
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
                let (title, draft) = self.pull_meta.take().unwrap_or_default();
                let author = self.data.profile.login.clone();
                self.data.pulls.insert(0, model::Pull { labels: Vec::new(), assignees: Vec::new(), review_requested: false,
                    number: c.number, title, repo: full_name.clone(), author,
                    state: if draft { PullState::Draft } else { PullState::Open },
                    checks: None, updated_at: self.now, comments: 0, url: c.url,
                });
                if let Some(t) = self.data.open_pulls_total.as_mut() {
                    *t += 1;
                }
                // GitHub's count of a repository's open issues includes its pull requests.
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues += 1;
                }
                self.notify(format!("Opened pull request #{}", c.number));
            }
            github::Done::Closed(n) => {
                let closed = self.data.issues.iter().find(|x| x.number == n && same_repo(&x.repo, &full_name)).cloned();
                self.data.issues.retain(|x| !(x.number == n && same_repo(&x.repo, &full_name)));
                if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                    r.issues = r.issues.saturating_sub(1);
                }
                if let Some(t) = self.data.issues_total.as_mut() {
                    *t = t.saturating_sub(1);
                }
                match closed {
                    Some(issue) => self.notify_undo(format!("Closed issue #{n}"), state::Undo::Reopen { repo: full_name.clone(), issue }),
                    None => self.notify(format!("Closed issue #{n}")),
                }
            }
            github::Done::Reopened(n) => {
                // An issue closed from the list and reopened with Undo goes back where it was.
                if let Some(at) = self.reopening.iter().position(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                    let issue = self.reopening.remove(at);
                    if !self.data.issues.iter().any(|x| x.number == n && same_repo(&x.repo, &full_name)) {
                        self.data.issues.insert(0, issue);
                        if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                            r.issues += 1;
                        }
                        if let Some(t) = self.data.issues_total.as_mut() {
                            *t += 1;
                        }
                    }
                }
                self.notify(format!("Reopened issue #{n}"));
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
                let mut changed = false;
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    if matches!(p.state, PullState::Open | PullState::Draft) {
                        changed = true;
                    }
                    p.state = PullState::Merged;
                }
                if changed {
                    if let Some(t) = self.data.open_pulls_total.as_mut() {
                        *t = t.saturating_sub(1);
                    }
                    if let Some(t) = self.data.closed_pulls_total.as_mut() {
                        *t += 1;
                    }
                    if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                        r.issues = r.issues.saturating_sub(1);
                    }
                }
                self.notify(format!("Merged pull request #{n}"));
            }
            github::Done::PullOpen(n, open) => {
                let mut changed = false;
                for p in self.data.pulls.iter_mut().filter(|p| p.number == n && same_repo(&p.repo, &full_name)) {
                    let was_open = matches!(p.state, PullState::Open | PullState::Draft);
                    if was_open != open {
                        changed = true;
                        p.state = if open { PullState::Open } else { PullState::Closed };
                    }
                }
                if changed {
                    let (from, to) = if open { (&mut self.data.closed_pulls_total, &mut self.data.open_pulls_total) } else { (&mut self.data.open_pulls_total, &mut self.data.closed_pulls_total) };
                    if let Some(t) = from.as_mut() {
                        *t = t.saturating_sub(1);
                    }
                    if let Some(t) = to.as_mut() {
                        *t += 1;
                    }
                    if let Some(r) = self.data.repos.iter_mut().find(|r| r.full_name.eq_ignore_ascii_case(&full_name)) {
                        r.issues = if open { r.issues + 1 } else { r.issues.saturating_sub(1) };
                    }
                }
                if open {
                    self.notify(format!("Reopened pull request #{n}"));
                } else {
                    self.notify_undo(format!("Closed pull request #{n}"), state::Undo::ReopenPull { repo: full_name.clone(), number: n });
                }
            }
            github::Done::Release(r) => self.release_done(&full_name, *r),
            github::Done::Triaged { number, change, applied } => self.apply_triaged(&full_name, number, change, applied),
            github::Done::Reran { run_id, failed_only } => {
                for r in self.data.runs.iter_mut().filter(|r| r.id == run_id && same_repo(&r.repo, &full_name)) {
                    r.checks = model::Checks::Running;
                    r.updated_at = self.now;
                }
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
