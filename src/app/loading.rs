//! Loading from GitHub: a refresh and `apply`, a repository's details and branches, the branch
//! picked, and loading more pull requests and issues.

use super::*;

impl App {
    pub(crate) fn set_status(&mut self, status: Status) {
        if self.status != status {
            state::trace("status", &self.status.name(), &status.name());
        }
        self.status = status;
        self.sync_tray();
    }

    /// Reloads everything from GitHub in the background.
    pub(crate) fn refresh(&mut self) {
        if !self.cfg.live() || !self.status.can_load() {
            return;
        }
        self.retry_after = None;
        self.last_load = Some(net::now());
        self.load_clock = Some(self.clock);
        self.set_status(Status::Loading);
        // Before the window exists, the first load reports to a mailbox (see `init`).
        let to = match (self.handle.clone(), &self.fresh.early) {
            (Some(h), _) => net::To::Now(h),
            (None, Some(m)) => net::To::Later(m.clone()),
            (None, None) => return,
        };
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        // The profile and the repositories show as soon as they're in (on a first start).
        let early = to.clone();
        let work = move || async move { github::load_all_with(&cfg, move |d| early.post(move |s: &mut App| s.apply_early(epoch, d))).await };
        let done = move |s: &mut App, r| s.apply(epoch, r);
        #[cfg(not(target_arch = "wasm32"))]
        net::spawn_to(to, work, done);
        #[cfg(target_arch = "wasm32")]
        if let net::To::Now(h) = to {
            net::spawn(h, work, done);
        }
    }

    pub(crate) fn apply(&mut self, epoch: u64, result: Result<Data, github::LoadError>) {
        if epoch != self.load_gen {
            return;
        }
        match result {
            Ok(d) => {
                self.retry_after = d.retry_after;
                // Details of repositories nobody pushed to since stay as they are; those of
                // repositories pushed to stay on screen and are read again when opened. Branch
                // lists stay too: opening a repository reads its list again behind it.
                let kept = cache::keep_details(&self.data, &d, std::mem::take(&mut self.details), &mut self.fresh.stale);
                self.fresh.from_copy = false;
                self.fresh.partial = false;
                self.data = d;
                self.data_gen = self.data_gen.wrapping_add(1);
                self.refresh_clone_cache();
                self.details = kept;
                self.loaded_at = Some(net::now());
                self.loaded_clock = Some(self.clock);
                self.now = net::now();
                self.open_pulls_page = 1;
                self.closed_pulls_page = 1;
                self.issues_page = 1;
                self.loading_more_pulls = false;
                self.loading_more_issues = false;
                self.set_status(Status::Ready);
                if let Page::Repo(full) = &self.page.clone() {
                    if let Some(i) = self.repo_index_of(full) {
                        // Keep the name as GitHub spells it now: local clones, terminals and the
                        // title are keyed by it exactly.
                        let current = self.data.repos[i].full_name.clone();
                        if current != *full {
                            self.page = Page::Repo(current);
                            self.save_place();
                            self.sync_tray();
                        }
                        self.load_detail(i);
                        self.load_branches(i, true);
                        if self.tab == RepoTab::Commits {
                            self.load_history(i, true);
                        }
                    } else {
                        self.go(Page::Repos);
                    }
                }
                if let Some(id) = &self.plugin_tab.clone()
                    && !self.plugins.list.iter().any(|p| p.id == *id && p.manifest.repo_tab.is_some())
                {
                    self.plugin_tab = None;
                }
                if let Page::Plugin(id) = &self.page.clone()
                    && !self.plugins.list.iter().any(|p| p.id == *id && p.manifest.page.is_some())
                {
                    self.go(Page::Overview);
                }
                self.releases_refreshed(self.repo_index());
                self.restore_place();
                // With the lists, what's waiting for you.
                self.load_inbox();
                self.sync_after_load();
                cache::trace_time("loaded");
                // For the next start.
                self.save_copy();
            }
            Err(e) => {
                if let Some(ra) = e.retry_after {
                    self.retry_after = Some(ra);
                }
                self.set_status(Status::Failed(e.message));
            }
        }
    }

    /// The branch shown for repository `i`: the one picked, else the default branch.
    pub(crate) fn branch(&self, i: usize) -> String {
        self.data.repos.get(i).map(|r| self.branch_of.get(&r.full_name).cloned().unwrap_or_else(|| r.branch.clone())).unwrap_or_default()
    }

    pub(crate) fn apply_detail(&mut self, epoch: u64, key: (String, String), d: Detail) {
        // Read again behind a page on screen: a failure keeps the page.
        let shown = matches!(self.details.get(&key), Some(Load::Ready(_)));
        if epoch == self.load_gen && !(shown && (d.error.is_some() || d.readme_error.is_some())) {
            let load = if let Some(e) = &d.error
                && d.files.is_empty()
                && d.readme.is_none()
                && d.languages.is_empty()
                && d.commit.is_none()
            {
                Load::Failed(e.clone())
            } else {
                Load::Ready(d)
            };
            self.details.insert(key, load);
        }
    }

    /// Loads files, README and last commit of repository `i` at its shown branch (once).
    pub(crate) fn load_detail(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let key = (full.clone(), self.branch(i));
        if self.data.sample {
            let d = model::sample_detail(r);
            self.load_local(i, false);
            self.details.insert(key, Load::Ready(d));
            return;
        }
        self.load_branches(i, false);
        self.load_local(i, false);
        let mut behind = false;
        if let Some(load) = self.details.get(&key) {
            match load {
                Load::Loading => return,
                // Read before a refresh, or from the saved copy: it stays on screen while it's read
                // again (cheaply: GitHub answers "not modified" when nothing changed).
                Load::Ready(d) if d.error.is_none() && d.readme_error.is_none() => {
                    if !self.fresh.stale.remove(&key) {
                        return;
                    }
                    behind = true;
                }
                _ => {}
            }
        }
        if !behind {
            self.details.insert(key.clone(), Load::Loading);
        }
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let at = key.1.clone();
            let epoch = self.load_gen;
            net::spawn(h, move || async move { github::load_detail(&cfg, &full, &at).await }, move |s: &mut App, d| {
                s.apply_detail(epoch, key, d);
            });
        }
    }

    pub(crate) fn apply_branches(&mut self, epoch: u64, key: String, r: Result<Vec<String>, String>) {
        if epoch != self.load_gen {
            return;
        }
        // A pull request form opened before the list arrived starts from the first other branch.
        if let (Ok(names), Some(f)) = (&r, self.form.as_mut())
            && f.repo == key && f.kind == Compose::Pull && f.from == f.into
            && let Some(other) = names.iter().find(|n| **n != f.into)
        {
            f.from = other.clone();
        }
        if let Ok(names) = &r {
            self.commits.branches_arrived(&key, names);
        }
        if let Ok(names) = &r
            && let Some(picked) = self.branch_of.get(&key)
            && !names.contains(picked)
        {
            self.branch_of.remove(&key);
            if let Page::Repo(full) = &self.page
                && full.eq_ignore_ascii_case(&key)
                && let Some(i) = self.repo_index_of(&key)
            {
                self.load_detail(i);
                if self.tab == RepoTab::Notes {
                    self.load_notes(i);
                }
            }
        }
        self.branches.insert(key, r.into());
    }

    /// Loads repository `i`'s branch list. The sample data has its default branch only.
    pub(crate) fn load_branches(&mut self, i: usize, force: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !force {
            if let Some(entry) = self.branches.get(&full)
                && !matches!(entry, Load::Failed(_))
            {
                return;
            }
        } else if matches!(self.branches.get(&full), Some(Load::Loading)) {
            return;
        }
        if self.data.sample || !self.cfg.live() {
            let mut names = vec![r.branch.clone()];
            names.extend(["feature/readme-selection", "fix/theme-switch"].map(String::from));
            self.branches.insert(full, Load::Ready(names));
            return;
        }
        if !matches!(self.branches.get(&full), Some(Load::Ready(_))) {
            self.branches.insert(full.clone(), Load::Loading);
        }
        let Some(h) = self.handle.clone() else { return };
        let (cfg, default, key) = (self.cfg.clone(), r.branch.clone(), full.clone());
        let epoch = self.load_gen;
        net::spawn(h, move || async move { github::branches(&cfg, &full, &default).await }, move |s: &mut App, r: Result<Vec<String>, String>| {
            s.apply_branches(epoch, key, r);
        });
    }

    /// Shows branch `branch` of repository `i` in the Code and Notes tabs.
    pub(crate) fn set_branch(&mut self, i: usize, branch: String) {
        let Some(r) = self.data.repos.get(i) else { return };
        state::trace("branch", &self.branch(i), &branch);
        if branch == r.branch {
            self.branch_of.remove(&r.full_name);
        } else {
            self.branch_of.insert(r.full_name.clone(), branch);
        }
        self.confirm_delete = None;
        self.load_detail(i);
        if self.tab == RepoTab::Notes {
            self.load_notes(i);
        }
        // The history follows the branch; the answer for the previous one is dropped.
        if self.tab == RepoTab::Commits {
            self.commits.open = None;
            self.load_history(i, false);
        }
    }

    /// The branch picker chose `branch` for `slot`.
    pub(crate) fn pick_branch_for(&mut self, i: usize, slot: BranchSlot, branch: String) {
        self.picker = None;
        match slot {
            BranchSlot::View => self.set_branch(i, branch),
            BranchSlot::From => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Head => if let Some(f) = self.form.as_mut() { f.from = branch },
            BranchSlot::Base => if let Some(f) = self.form.as_mut() { f.into = branch },
            BranchSlot::CompareBase | BranchSlot::CompareHead => self.pick_compare(slot, branch),
            BranchSlot::ReleaseTarget => self.pick_release_target(branch),
        }
    }

    pub fn load_more_pulls(&mut self, open: bool) {
        if self.loading_more_pulls || self.data.sample {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let login = self.data.profile.login.clone();
        if login.is_empty() {
            return;
        }
        let next_page = if open { self.open_pulls_page + 1 } else { self.closed_pulls_page + 1 };
        self.loading_more_pulls = true;
        let epoch = self.load_gen;
        net::spawn(h, move || async move {
            github::load_more_pulls(&cfg, &login, open, next_page).await
        }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.loading_more_pulls = false;
            match r {
                Ok((items, total)) => {
                    if open {
                        s.open_pulls_page = next_page;
                        s.data.open_pulls_total = Some(total);
                    } else {
                        s.closed_pulls_page = next_page;
                        s.data.closed_pulls_total = Some(total);
                    }
                    for p in items {
                        if !s.data.pulls.iter().any(|existing| existing.number == p.number && existing.repo.eq_ignore_ascii_case(&p.repo)) {
                            s.data.pulls.push(p);
                        }
                    }
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.sync_tray();
                }
                Err(e) => {
                    s.notify_problem(format!("Could not load more pull requests: {e}"));
                }
            }
        });
    }

    pub fn load_more_issues(&mut self) {
        if self.loading_more_issues || self.data.sample {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let cfg = self.cfg.clone();
        let login = self.data.profile.login.clone();
        if login.is_empty() {
            return;
        }
        let mut orgs: Vec<String> = self.data.repos.iter().map(|r| r.owner.clone()).filter(|o| !o.eq_ignore_ascii_case(&login)).collect();
        orgs.sort_by_key(|o| o.to_lowercase());
        orgs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let next_page = self.issues_page + 1;
        self.loading_more_issues = true;
        let epoch = self.load_gen;
        net::spawn(h, move || async move {
            github::load_more_issues(&cfg, &login, &orgs, next_page).await
        }, move |s: &mut App, r| {
            if s.load_gen != epoch {
                return;
            }
            s.loading_more_issues = false;
            match r {
                Ok((items, total)) => {
                    s.issues_page = next_page;
                    s.data.issues_total = Some(total);
                    for issue in items {
                        if !s.data.issues.iter().any(|existing| existing.number == issue.number && existing.repo.eq_ignore_ascii_case(&issue.repo)) {
                            s.data.issues.push(issue);
                        }
                    }
                    s.data.issues.sort_by_key(|i| std::cmp::Reverse(i.created_at));
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.sync_tray();
                }
                Err(e) => {
                    s.notify_problem(format!("Could not load more issues: {e}"));
                }
            }
        });
    }
}
