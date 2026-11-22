//! What opens in place of the page: notifications, an issue's or pull request's conversation, a
//! workflow run, and a pull request's diff with its line comments.

use super::*;

impl App {
    /// How many unread notifications are loaded.
    pub(crate) fn unread(&self) -> usize {
        self.inbox.as_ref().and_then(Load::ready).map_or(0, Vec::len)
    }

    /// Reads the unread notifications from GitHub (again). The list on screen stays meanwhile.
    pub(crate) fn load_inbox(&mut self) {
        if self.data.sample || !self.cfg.live() || self.cfg.token.is_none() {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        if !matches!(self.inbox, Some(Load::Ready(_))) {
            self.inbox = Some(Load::Loading);
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_notifications(&cfg).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            s.inbox = Some(match r {
                Ok(list) => Load::Ready(list),
                Err(e) => Load::Failed(inbox::explain(&e)),
            });
            s.sync_tray();
        });
    }

    /// Marks notification `id` as read: it leaves the list at once, and comes back if GitHub
    /// refuses.
    pub(crate) fn mark_read(&mut self, id: &str) {
        let Some(h) = self.handle.clone() else { return };
        let Some(Load::Ready(list)) = self.inbox.as_mut() else { return };
        let Some(at) = list.iter().position(|n| n.id == id) else { return };
        if !self.inbox_busy.insert(id.to_owned()) {
            return;
        }
        let taken = list.remove(at);
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let id = id.to_owned();
        net::spawn(h, { let id = id.clone(); move || async move { github::mark_read(&cfg, &id).await } }, move |s: &mut App, r: Result<(), String>| {
            if s.load_gen != account {
                return;
            }
            s.inbox_busy.remove(&id);
            if let Err(e) = r {
                if let Some(Load::Ready(list)) = s.inbox.as_mut() {
                    list.insert(at.min(list.len()), taken);
                }
                s.notify_problem(inbox::explain(&e));
            }
            s.sync_tray();
        });
    }

    /// Opens what a notification is about (its conversation, or its page on GitHub) and marks
    /// it as read.
    pub(crate) fn open_notification(&mut self, n: &inbox::Notification, tx: &mut Tx) {
        match (n.subject, n.number) {
            (inbox::Subject::Issue, Some(number)) => self.open_thread(&n.repo, number, false),
            (inbox::Subject::Pull, Some(number)) => self.open_thread(&n.repo, number, true),
            _ => {
                let _ = tx.open_url(&n.url);
            }
        }
        self.mark_read(&n.id);
    }

    /// Opens the conversation of issue or pull request `number` of `repo`, in place of the page.
    pub(crate) fn open_thread(&mut self, repo: &str, number: u32, pull: bool) {
        self.close_pull_diff();
        self.confirm_merge = None;
        self.open_run = None;
        self.thread_epoch = self.thread_epoch.wrapping_add(1);
        // The one being left is kept as it is now (with what was written in it).
        if let Some(t) = self.thread.take()
            && let Load::Ready(read) = t.load
            && !self.data.sample
        {
            self.remember_thread(&t.repo, t.number, read);
        }
        // One read before in this session shows at once, and is read again behind it.
        let load = match self.remembered_thread(repo, number) {
            Some(t) if !self.data.sample => Load::Ready(t),
            _ => Load::Loading,
        };
        self.thread = Some(thread::Open { repo: repo.to_owned(), number, pull, load, reply: TextEdit::new(""), sending: false, epoch: self.thread_epoch, edit: Default::default() });
        self.load_thread();
    }

    /// Reads the open conversation from GitHub (again). What's on screen stays until the answer.
    pub(crate) fn load_thread(&mut self) {
        let Some(t) = self.thread.as_mut() else { return };
        let (repo, number, pull, epoch) = (t.repo.clone(), t.number, t.pull, t.epoch);
        // The sample data has no GitHub behind it: a made-up conversation shows the page.
        if self.data.sample {
            let (title, author, standing) = if pull {
                self.data.pulls.iter().find(|p| p.number == number && same_repo(&p.repo, &repo))
                    .map(|p| (p.title.clone(), p.author.clone(), match p.state {
                        PullState::Open => thread::Standing::Open,
                        PullState::Draft => thread::Standing::Draft,
                        PullState::Merged => thread::Standing::Merged,
                        PullState::Closed => thread::Standing::Closed,
                    }))
            } else {
                self.data.issues.iter().find(|x| x.number == number && same_repo(&x.repo, &repo))
                    .map(|x| (x.title.clone(), self.data.profile.login.clone(), thread::Standing::Open))
            }.unwrap_or_else(|| (format!("#{number}"), self.data.profile.login.clone(), thread::Standing::Open));
            let mut sample = thread::sample(&title, &author, pull, standing, self.now);
            triage::sample_thread(&mut sample, &self.data, &repo, number, pull);
            t.load = Load::Ready(sample);
            return;
        }
        let Some(h) = self.handle.clone() else {
            // One read before stays on screen.
            if !matches!(t.load, Load::Ready(_)) {
                t.load = Load::Failed("The app isn't ready to load this yet: try again in a moment.".into());
            }
            return;
        };
        if !matches!(t.load, Load::Ready(_)) {
            t.load = Load::Loading;
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        let full = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).map(|r| r.full_name.clone()).unwrap_or_else(|| repo.clone());
        net::spawn(h, move || async move { github::load_thread(&cfg, &full, number).await }, move |s: &mut App, r| {
            // Another account, another conversation, or this one closed meanwhile: not ours.
            if s.load_gen != account {
                return;
            }
            if let Ok(read) = &r {
                s.remember_thread(&repo, number, read.clone());
            }
            let Some(t) = s.thread.as_mut().filter(|t| t.epoch == epoch) else { return };
            t.load = match r {
                Ok(thread) => Load::Ready(thread),
                // A reload that fails keeps what was read before, and says so.
                Err(e) if matches!(t.load, Load::Ready(_)) => {
                    s.notify_problem(e);
                    return;
                }
                Err(e) => Load::Failed(e),
            };
        });
    }

    /// Sends what's typed in the conversation's box: a comment, or a review with `verdict`.
    pub(crate) fn send_reply(&mut self, verdict: Option<thread::Verdict>) {
        let Some(t) = self.thread.as_mut() else { return };
        if t.sending {
            return;
        }
        let body = t.reply.text.trim().to_owned();
        // Only an approval can go without words.
        if body.is_empty() && verdict != Some(thread::Verdict::Approved) {
            return;
        }
        let (repo, number) = (t.repo.clone(), t.number);
        let w = match verdict {
            Some(verdict) => github::Write::Review { number, verdict, body },
            None => github::Write::Comment { number, body },
        };
        if actions::blocked(self).is_none() {
            if let Some(t) = self.thread.as_mut() {
                t.sending = true;
            }
            self.write(&repo, w);
        }
    }

    /// Opens workflow run `r` in the in-app detail view (jobs, steps, and job logs).
    pub(crate) fn open_run(&mut self, r: &model::Run) {
        self.close_pull_diff();
        self.confirm_merge = None;
        self.thread = None;
        self.run_epoch = self.run_epoch.wrapping_add(1);
        let repo = runs_page::full_repo_of(self, &r.repo);
        self.open_run = Some(runs_page::OpenRun {
            repo,
            run_id: r.id,
            workflow: r.workflow.clone(),
            branch: r.branch.clone(),
            checks: r.checks,
            duration: r.duration.clone(),
            updated_at: r.updated_at,
            url: r.url.clone(),
            epoch: self.run_epoch,
            jobs: Load::Loading,
            selected_job: None,
            logs: HashMap::new(),
            write_error: None,
        });
        self.load_run_jobs();
    }

    /// Reloads jobs and cached logs for the open workflow run.
    pub(crate) fn reload_open_run(&mut self) {
        if let Some(o) = self.open_run.as_mut() {
            o.logs.clear();
            o.write_error = None;
        }
        self.load_run_jobs();
    }

    /// Reads the jobs and steps of the open workflow run from GitHub (or sample data).
    pub(crate) fn load_run_jobs(&mut self) {
        let Some(o) = self.open_run.as_mut() else { return };
        let (repo, run_id, epoch) = (o.repo.clone(), o.run_id, o.epoch);
        if self.data.sample {
            let run = o.as_run();
            let jobs = model::sample_run_jobs(&run);
            let selected = jobs.iter().find(|j| j.checks == model::Checks::Failing)
                .or_else(|| jobs.first())
                .map(|j| j.id);
            o.selected_job = selected;
            o.logs.clear();
            if let Some(jid) = selected
                && let Some(job) = jobs.iter().find(|j| j.id == jid)
            {
                o.logs.insert(jid, Load::Ready(model::sample_job_log(&run, job)));
            }
            o.jobs = Load::Ready(jobs);
            return;
        }
        let Some(h) = self.handle.clone() else {
            o.jobs = Load::Failed("The app isn't ready to load this yet: try again in a moment.".into());
            return;
        };
        if !matches!(o.jobs, Load::Ready(_)) {
            o.jobs = Load::Loading;
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_run_jobs(&cfg, &repo, run_id).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            let Some(o) = s.open_run.as_mut().filter(|o| o.epoch == epoch) else { return };
            match r {
                Ok(jobs) => {
                    let keep = o.selected_job.filter(|id| jobs.iter().any(|j| j.id == *id));
                    let pick = keep
                        .or_else(|| jobs.iter().find(|j| j.checks == model::Checks::Failing).map(|j| j.id))
                        .or_else(|| jobs.first().map(|j| j.id));
                    o.selected_job = pick;
                    o.jobs = Load::Ready(jobs);
                    if let Some(jid) = pick {
                        s.load_job_log(jid);
                    }
                }
                Err(e) if matches!(o.jobs, Load::Ready(_)) => {
                    s.notify_problem(e);
                }
                Err(e) => {
                    o.jobs = Load::Failed(e);
                }
            }
        });
    }

    /// Selects job `job_id` in the open workflow run and loads its log if not yet fetched.
    pub(crate) fn select_run_job(&mut self, job_id: u64) {
        let need_load = match self.open_run.as_mut() {
            Some(o) => {
                o.selected_job = Some(job_id);
                !matches!(o.logs.get(&job_id), Some(Load::Ready(_) | Load::Loading))
            }
            None => false,
        };
        if need_load {
            self.load_job_log(job_id);
        }
    }

    /// Fetches the plain-text log tail (last 200 lines) for `job_id` in the open workflow run.
    pub(crate) fn load_job_log(&mut self, job_id: u64) {
        let Some(o) = self.open_run.as_mut() else { return };
        if self.data.sample {
            let run = o.as_run();
            if let Some(job) = o.jobs.ready().and_then(|jobs| jobs.iter().find(|j| j.id == job_id)) {
                let log = model::sample_job_log(&run, job);
                o.logs.insert(job_id, Load::Ready(log));
            }
            return;
        }
        let Some(h) = self.handle.clone() else {
            o.logs.insert(job_id, Load::Failed("The app isn't ready to load this yet: try again in a moment.".into()));
            return;
        };
        let (repo, epoch) = (o.repo.clone(), o.epoch);
        if !matches!(o.logs.get(&job_id), Some(Load::Ready(_))) {
            o.logs.insert(job_id, Load::Loading);
        }
        let cfg = self.cfg.clone();
        let account = self.load_gen;
        net::spawn(h, move || async move { github::load_job_log(&cfg, &repo, job_id).await }, move |s: &mut App, r| {
            if s.load_gen != account {
                return;
            }
            let Some(o) = s.open_run.as_mut().filter(|o| o.epoch == epoch) else { return };
            match r {
                Ok(log) => {
                    o.logs.insert(job_id, Load::Ready(log));
                }
                Err(e) if matches!(o.logs.get(&job_id), Some(Load::Ready(_))) => {
                    s.notify_problem(e);
                }
                Err(e) => {
                    o.logs.insert(job_id, Load::Failed(e));
                }
            }
        });
    }

    /// Triggers a workflow re-run (`failed_only` = failed jobs only, otherwise the whole workflow).
    pub(crate) fn rerun_open_run(&mut self, failed_only: bool) {
        let Some((repo, run_id)) = self.open_run.as_mut().map(|o| {
            o.write_error = None;
            (o.repo.clone(), o.run_id)
        }) else {
            return;
        };
        self.write(&repo, github::Write::RerunRun { run_id, failed_only });
    }

    pub(crate) fn apply_pull_diff(&mut self, epoch: u64, key: (String, u32), r: Result<Vec<github::PullFile>, String>) {
        if epoch != self.load_gen || self.pull_diff_of != Some(key) {
            return;
        }
        match r {
            Ok(files) => {
                self.pull_diff.set_files(diffs::pull_files(&files));
                self.pull_diff_load = Some(Load::Ready(()));
                self.place_line_comments();
            }
            Err(e) => self.pull_diff_load = Some(Load::Failed(e)),
        }
    }

    /// Shows pull request `number` of `full`'s files as a diff.
    pub(crate) fn open_pull_diff(&mut self, full: String, number: u32) {
        let key = (full.clone(), number);
        // A merge confirmation belongs to the diff it was asked from.
        self.confirm_merge = None;
        self.pull_diff_of = Some(key.clone());
        self.pull_diff_load = Some(Load::Loading);
        self.pull_diff.set_files(Vec::new());
        self.pull_review = Some(review::Review::default());
        if self.data.sample {
            self.pull_diff.set_files(diffs::pull_files(&diffs::sample_pull_files()));
            self.pull_diff_load = Some(Load::Ready(()));
            return;
        }
        self.load_line_comments();
        let Some(h) = self.handle.clone() else {
            self.pull_diff_load = Some(Load::Failed("The app isn't ready to load this diff yet: try again in a moment.".into()));
            return;
        };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::load_pull_files(&cfg, &full, number).await }, move |s: &mut App, r| {
            s.apply_pull_diff(epoch, key, r);
        });
    }

    pub(crate) fn close_pull_diff(&mut self) {
        if self.pull_diff_of.is_some() {
            self.confirm_merge = None;
        }
        self.pull_diff_of = None;
        self.pull_diff_load = None;
        self.pull_review = None;
        self.pull_diff.set_files(Vec::new());
    }

    /// Reads the comments on the open diff's lines (again), with the commit it shows.
    pub(super) fn load_line_comments(&mut self) {
        let Some((full, number)) = self.pull_diff_of.clone() else { return };
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        let key = (full.clone(), number);
        net::spawn(h, move || async move { github::load_line_comments(&cfg, &full, number).await }, move |s: &mut App, r| {
            // Another account or another diff since: not for this one.
            if s.load_gen != epoch || s.pull_diff_of.as_ref() != Some(&key) {
                return;
            }
            match r {
                Ok((head, comments)) => {
                    if let Some(rv) = s.pull_review.as_mut() {
                        rv.head = head;
                        rv.comments = comments;
                    }
                    s.place_line_comments();
                }
                // The diff still shows; only its comments are missing.
                Err(e) => s.notify_problem(format!("The comments on this pull request's lines couldn't load: {e}")),
            }
        });
    }

    /// Puts the line comments and the one being written under their lines.
    pub(super) fn place_line_comments(&mut self) {
        let list = self.pull_review.as_ref().map(|r| r.annotations(self.pull_diff.files())).unwrap_or_default();
        self.pull_diff.set_annotations(list);
    }

    /// The "+" by a line of the pull request's diff: a box to write a comment there.
    pub(crate) fn start_line_comment(&mut self, range: rune_kit::diff::LineRange) {
        let files = self.pull_diff.files().to_vec();
        if self.pull_review.get_or_insert_with(review::Review::default).start(&files, range) {
            self.place_line_comments();
        }
    }

    pub(crate) fn cancel_line_comment(&mut self) {
        if let Some(r) = self.pull_review.as_mut() {
            r.draft = None;
        }
        self.place_line_comments();
    }

    /// Sends the comment being written on the diff's lines.
    pub(crate) fn send_line_comment(&mut self) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Some((full, number)) = self.pull_diff_of.clone() else { return };
        let Some(r) = self.pull_review.as_mut() else { return };
        let head = r.head.clone();
        let Some(d) = r.draft.as_mut() else { return };
        let body = d.text.text.trim().to_owned();
        if body.is_empty() || d.sending {
            return;
        }
        if head.is_empty() {
            self.notify_problem("The pull request's commit isn't known yet: try again in a moment.");
            return;
        }
        d.sending = true;
        let comment = github::NewLineComment {
            body,
            commit: head,
            path: d.path.clone(),
            line: d.end.line,
            side: review::side_word(d.end.side),
            start: (d.start != d.end).then(|| (d.start.line, review::side_word(d.start.side))),
        };
        self.write(&full, github::Write::LineComment { number, comment });
    }
}
