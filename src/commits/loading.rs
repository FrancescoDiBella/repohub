//! Loading: the history, a commit and a comparison read in the background, late answers dropped, and
//! what the Commits section does on `App`.

use super::*;

// ------------------------------------------------------------------ loading

/// Runs `work` in the background and hands its answer to `done`. Before the window exists, a
/// headless still (`REPOHUB_BLOCKING=1`) waits for it instead. False when neither can run.
fn run<R, Fut>(s: &mut App, work: impl FnOnce() -> Fut + Send + 'static, done: impl FnOnce(&mut App, R) + Send + 'static) -> bool
where
    R: Send + 'static,
    Fut: std::future::Future<Output = R> + 'static,
{
    if let Some(h) = s.handle.clone() {
        net::spawn(h, work, done);
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1") {
        let r = pollster::block_on(work());
        done(s, r);
        return true;
    }
    false
}

const NOT_READY: &str = "The app isn't ready to load this yet: try again in a moment.";

impl App {
    /// Reads the history of the branch on screen of repository `i` (`again`: even when it's
    /// already here). Another branch's history waiting for its answer is forgotten.
    pub(crate) fn load_history(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, branch) = (r.full_name.clone(), self.branch(i));
        let same = self.commits.history_of(&repo, &branch);
        if same.is_some_and(|h| !again && !matches!(h.load, Load::Failed(_))) {
            return;
        }
        // A reload keeps the list on screen until the answer.
        let kept = if same.is_some() { self.commits.history.take().and_then(|h| match h.load { Load::Ready(rows) => Some((rows, h.page, h.more)), _ => None }) } else { None };
        let epoch = self.commits.next_epoch();
        state_trace(&repo, &branch);
        if self.data.sample {
            let all = sample_history(r, &branch, &self.data.profile.login);
            let more = all.len() > PAGE;
            self.commits.history = Some(History { repo, branch, load: Load::Ready(all.into_iter().take(PAGE).collect()), page: 1, more, loading_more: false, epoch });
            return;
        }
        let (load, page, more) = match kept {
            Some((rows, page, more)) => (Load::Ready(rows), page, more),
            None => (Load::Loading, 1, false),
        };
        self.commits.history = Some(History { repo: repo.clone(), branch: branch.clone(), load, page, more, loading_more: false, epoch });
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commits(&cfg, &repo, &branch, 1).await }, move |s: &mut App, r| s.apply_history(account, epoch, 1, r)) {
            self.apply_history(account, epoch, 1, Err(NOT_READY.into()));
        }
    }

    /// The next page of the history on screen.
    pub(crate) fn load_more_commits(&mut self) {
        let sample = self.data.sample;
        let Some(h) = self.commits.history.as_mut() else { return };
        if h.loading_more || !h.more || !matches!(h.load, Load::Ready(_)) {
            return;
        }
        let (repo, branch, epoch, page) = (h.repo.clone(), h.branch.clone(), h.epoch, h.page + 1);
        if sample {
            let Some(r) = self.data.repos.iter().find(|r| r.full_name == repo) else { return };
            let all = sample_history(r, &branch, &self.data.profile.login);
            let more = all.len() > page * PAGE;
            let rows = all.into_iter().skip((page - 1) * PAGE).take(PAGE).collect();
            self.apply_history(self.load_gen, epoch, page, Ok((rows, more)));
            return;
        }
        h.loading_more = true;
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commits(&cfg, &repo, &branch, page).await }, move |s: &mut App, r| s.apply_history(account, epoch, page, r)) {
            self.apply_history(account, epoch, page, Err(NOT_READY.into()));
        }
    }

    /// GitHub's answer for page `page` of the history asked with `epoch`.
    pub(crate) fn apply_history(&mut self, account: u64, epoch: u64, page: usize, r: Result<(Vec<Row>, bool), String>) {
        // Another account, another branch, or a reload since: not for this list.
        if self.load_gen != account {
            return;
        }
        let Some(h) = self.commits.history.as_mut().filter(|h| h.epoch == epoch) else { return };
        if page > 1 {
            h.loading_more = false;
        }
        match r {
            Ok((rows, more)) => match &mut h.load {
                // New commits pushed meanwhile shift the pages: a commit already listed isn't listed twice.
                Load::Ready(list) if page > 1 => {
                    list.extend(rows.into_iter().filter(|x| !list.iter().any(|y| y.sha == x.sha)).collect::<Vec<_>>());
                    h.page = page;
                    h.more = more;
                }
                // A reload (a refresh) whose first page ends inside the list: the commits pushed
                // since go on top, and the pages already read stay.
                Load::Ready(list) if rows.last().is_some_and(|last| list.iter().any(|y| y.sha == last.sha)) => {
                    let fresh: Vec<Row> = rows.into_iter().take_while(|x| !list.iter().any(|y| y.sha == x.sha)).collect();
                    list.splice(0..0, fresh);
                }
                // The first page, or a history rewritten under the list (a force push).
                load => {
                    *load = Load::Ready(rows);
                    h.page = page;
                    h.more = more;
                }
            },
            Err(e) if page > 1 => self.notify_problem(format!("More commits couldn't load: {e}")),
            // A reload that fails keeps what was read before, and says so.
            Err(e) if matches!(h.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => h.load = Load::Failed(e),
        }
    }

    /// Opens commit `sha` of `repo` in place of the list. `from_row`: the row it was opened
    /// from, shown again on Back.
    pub(crate) fn open_commit(&mut self, repo: &str, sha: &str, from_row: Option<String>) {
        let epoch = self.commits.next_epoch();
        state_trace(repo, sha);
        self.commits.open = Some(Open { repo: repo.to_owned(), sha: sha.to_owned(), load: Load::Loading, epoch, trail: Vec::new(), from_row });
        self.commits.commit_diff.set_files(Vec::new());
        self.load_commit();
    }

    /// Opens a parent of the commit on screen; Back comes back to this one.
    pub(crate) fn open_parent(&mut self, sha: &str) {
        let epoch = self.commits.next_epoch();
        let Some(o) = self.commits.open.as_mut() else { return };
        let child = std::mem::replace(&mut o.sha, sha.to_owned());
        o.trail.push(child);
        o.epoch = epoch;
        o.load = Load::Loading;
        self.commits.commit_diff.set_files(Vec::new());
        self.load_commit();
    }

    /// Reads the commit on screen (again).
    pub(crate) fn load_commit(&mut self) {
        let sample = self.data.sample;
        let Some(o) = self.commits.open.as_mut() else { return };
        let (repo, sha, epoch) = (o.repo.clone(), o.sha.clone(), o.epoch);
        if sample {
            // A commit of the history, or of the comparison on screen.
            let compared = match self.commits.compare.as_ref().and_then(|c| c.result.as_ref()) {
                Some(Load::Ready(cmp)) => cmp.commits.iter().position(|x| x.sha == sha).map(|at| (cmp.commits[at].clone(), cmp.commits.get(at + 1).map(|p| p.sha.clone()))),
                _ => None,
            };
            let found = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).and_then(|r| match &compared {
                Some((row, parent)) => Some(sample_detail_of(r, row, parent.clone())),
                None => sample_detail(r, &sha, &self.data.profile.login),
            });
            let r = found.ok_or_else(|| format!("There's no commit {} in the sample data.", short(&sha)));
            self.apply_commit(self.load_gen, epoch, r);
            return;
        }
        if !matches!(o.load, Load::Ready(_)) {
            o.load = Load::Loading;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_commit(&cfg, &repo, &sha).await }, move |s: &mut App, r| s.apply_commit(account, epoch, r)) {
            self.apply_commit(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_commit(&mut self, account: u64, epoch: u64, r: Result<Detail, String>) {
        if self.load_gen != account {
            return;
        }
        let c = &mut self.commits;
        let Some(o) = c.open.as_mut().filter(|o| o.epoch == epoch) else { return };
        match r {
            Ok(d) => {
                c.commit_diff.set_files(diffs::pull_files(&d.files));
                o.load = Load::Ready(d);
            }
            Err(e) if matches!(o.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => o.load = Load::Failed(e),
        }
    }

    /// Back from the commit on screen: to the commit whose parent link opened it, else to the
    /// list, with the row it was opened from in view (`tx`, when there is one).
    pub(crate) fn close_commit(&mut self, tx: Option<&mut Tx>) {
        let epoch = self.commits.next_epoch();
        let Some(o) = self.commits.open.as_mut() else { return };
        if let Some(child) = o.trail.pop() {
            o.sha = child;
            o.epoch = epoch;
            o.load = Load::Loading;
            self.commits.commit_diff.set_files(Vec::new());
            self.load_commit();
            return;
        }
        let row = o.from_row.clone();
        self.commits.open = None;
        self.commits.commit_diff.set_files(Vec::new());
        if let (Some(tx), Some(row)) = (tx, row) {
            tx.scroll_key_to(row, 160.0);
        }
    }

    /// Opens the comparison form of repository `i`: the default branch, and the branch on
    /// screen (the first other branch when that's the default one).
    pub(crate) fn open_compare(&mut self, i: usize) {
        self.load_branches(i, false);
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, base, shown) = (r.full_name.clone(), r.branch.clone(), self.branch(i));
        let head = if shown != base {
            shown
        } else {
            self.branches.get(&repo).and_then(Load::ready).and_then(|b| b.iter().find(|n| **n != base).cloned()).unwrap_or(shown)
        };
        self.commits.open = None;
        self.commits.compare_diff.set_files(Vec::new());
        self.commits.compare = Some(Compare { repo, base, head, result: None, epoch: 0, asked_pull: false });
        self.picker = None;
    }

    pub(crate) fn close_compare(&mut self) {
        self.commits.compare = None;
        self.commits.compare_diff.set_files(Vec::new());
        if matches!(self.picker, Some(BranchSlot::CompareBase | BranchSlot::CompareHead)) {
            self.picker = None;
        }
    }

    /// The branch picker chose `branch` for the comparison; a result on screen follows it.
    pub(crate) fn pick_compare(&mut self, slot: BranchSlot, branch: String) {
        let Some(c) = self.commits.compare.as_mut() else { return };
        match slot {
            BranchSlot::CompareBase => c.base = branch,
            BranchSlot::CompareHead => c.head = branch,
            _ => return,
        }
        if c.result.is_some() {
            self.run_compare();
        }
    }

    /// Compares the two branches of the form.
    pub(crate) fn run_compare(&mut self) {
        let epoch = self.commits.next_epoch();
        let sample = self.data.sample;
        let Some(c) = self.commits.compare.as_mut() else { return };
        self.commits.compare_diff.set_files(Vec::new());
        // The same branch twice: the form says so, nothing is asked.
        if c.base == c.head {
            c.result = None;
            return;
        }
        c.epoch = epoch;
        c.asked_pull = false;
        c.result = Some(Load::Loading);
        let (repo, base, head) = (c.repo.clone(), c.base.clone(), c.head.clone());
        if sample {
            let found = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo)).map(|r| sample_comparison(r, &base, &head, &self.data.profile.login, self.now));
            self.apply_compare(self.load_gen, epoch, found.ok_or_else(|| "This repository isn't in the sample data.".to_owned()));
            return;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_compare(&cfg, &repo, &base, &head).await }, move |s: &mut App, r| s.apply_compare(account, epoch, r)) {
            self.apply_compare(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_compare(&mut self, account: u64, epoch: u64, r: Result<Comparison, String>) {
        if self.load_gen != account {
            return;
        }
        let c = &mut self.commits;
        let Some(cmp) = c.compare.as_mut().filter(|x| x.epoch == epoch && x.result.is_some()) else { return };
        if let Ok(result) = &r {
            c.compare_diff.set_files(diffs::pull_files(&result.files));
        }
        cmp.result = Some(r.into());
    }

    /// The New pull request form, from the comparison on screen: its branches filled in, and
    /// the title and description of its commit when there's only one.
    pub(crate) fn pull_from_compare(&mut self) {
        let Some(c) = self.commits.compare.as_mut() else { return };
        let Some(Load::Ready(cmp)) = c.result.as_ref() else { return };
        let (title, body) = match cmp.commits.as_slice() {
            [only] if cmp.ahead == 1 => (only.title.clone(), only.body.clone()),
            _ => (String::new(), String::new()),
        };
        c.asked_pull = true;
        let mut title = TextEdit::new(title);
        title.focused = true;
        self.form = Some(actions::Form {
            repo: c.repo.clone(),
            kind: Compose::Pull,
            title,
            body: TextEdit::new(body),
            from: cmp.head.clone(),
            into: cmp.base.clone(),
            draft: false,
            submit: Submit::Editing,
        });
        self.picker = None;
    }

    /// For headless stills (`-- commit <id>`, `-- compare <base>...<head>`): the Commits
    /// section of the repository on screen, with that commit open or those branches compared.
    pub(crate) fn commits_still(&mut self, commit: Option<String>, compare: Option<String>) {
        let Some(i) = self.repo_index() else { return };
        if commit.is_none() && compare.is_none() && self.tab != crate::state::RepoTab::Commits {
            return;
        }
        self.tab = crate::state::RepoTab::Commits;
        self.load_history(i, false);
        if let Some((base, head)) = compare.as_deref().and_then(|c| c.split_once("...")) {
            self.open_compare(i);
            if let Some(c) = self.commits.compare.as_mut() {
                (c.base, c.head) = (base.to_owned(), head.to_owned());
            }
            self.run_compare();
        }
        if let Some(sha) = commit {
            let full = self.data.repos[i].full_name.clone();
            // `-- commit 2` opens the second commit of the history.
            let sha = match (sha.parse::<usize>(), self.commits.history.as_ref().and_then(|h| h.load.ready())) {
                (Ok(n), Some(rows)) if sha.len() < 7 => rows.get(n.saturating_sub(1)).map(|r| r.sha.clone()).unwrap_or(sha),
                _ => sha,
            };
            self.open_commit(&full, &sha, None);
        }
    }
}

fn state_trace(repo: &str, what: &str) {
    crate::state::trace("commits", &repo, &what);
}
