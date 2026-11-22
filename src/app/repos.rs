//! The repository lists, the pins and the organizations, cached between frames (`data_gen`);
//! pinning and starring.

use super::*;

impl App {
    /// Stars or unstars repository `i`: on GitHub when a token is configured, locally otherwise
    /// (and the toast says why nothing was sent).
    pub(crate) fn toggle_star(&mut self, i: usize) {
        if !self.cfg.allow_writes {
            self.notify_problem("Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true).");
            return;
        }
        let Some((on, full)) = self.data.repos.get(i).map(|r| (!r.starred, r.full_name.clone())) else { return };
        // Shown at once, and kept through a load that lands before GitHub has it.
        self.changed(replay::Change::Star { repo: full.clone(), on }, false);
        self.data_changed();
        if !self.cfg.live() || self.data.sample {
            let why = actions::blocked(self).unwrap_or("This is sample data.");
            self.notify(format!("Star changed here only, nothing was sent to GitHub. {why}"));
            return;
        }
        if self.cfg.token.is_none() {
            self.notify("Star changed here only: GITHUB_TOKEN is needed to save it on GitHub.");
            return;
        }
        if let Some(h) = self.handle.clone() {
            let cfg = self.cfg.clone();
            let key = full.clone();
            let (epoch, landed) = (self.load_gen, self.landed);
            net::spawn(h, move || async move { github::set_star(&cfg, &full, on).await }, move |s: &mut App, res| s.star_sent(epoch, &key, on, landed, res));
        }
    }

    /// GitHub's answer to a star (`on`) or an unstar of `repo`, asked when `landed` loads had
    /// landed. Done: a load that landed meanwhile may have read the star from before, so it's
    /// put back (exactly: once). Refused: the star goes back to what it was.
    pub(crate) fn star_sent(&mut self, epoch: u64, repo: &str, on: bool, landed: u64, res: Result<(), String>) {
        if self.load_gen != epoch {
            return;
        }
        match res {
            Ok(()) => {
                if self.landed != landed {
                    self.changed(replay::Change::Star { repo: repo.to_owned(), on }, true);
                }
            }
            Err(e) => {
                self.replays.retain(|c| !matches!(c, replay::Change::Star { repo: r, .. } if r.eq_ignore_ascii_case(repo)));
                let me = self.data.profile.login.clone();
                replay::apply(&mut self.data, &replay::Change::Star { repo: repo.to_owned(), on: !on }, &me, true);
                self.notify_problem(format!("Couldn't change the star: {e}"));
            }
        }
        self.data_changed();
    }

    pub(crate) fn starred_count(&self) -> u32 {
        self.data.repos.iter().map(|r| r.stars).sum()
    }

    fn repos_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.data_gen.hash(h);
            self.data.repos.len().hash(h);
            self.data.profile.login.hash(h);
            if let Some(r) = self.data.repos.first() {
                r.full_name.hash(h);
                r.stars.hash(h);
                r.pushed.hash(h);
                r.private.hash(h);
                r.fork.hash(h);
            }
            if let Some(r) = self.data.repos.last() {
                r.full_name.hash(h);
                r.stars.hash(h);
                r.pushed.hash(h);
                r.private.hash(h);
                r.fork.hash(h);
            }
        })
    }

    fn pins_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.repos_stamp().hash(h);
            self.pins.hash(h);
            self.cfg.pinned.hash(h);
        })
    }

    fn ensure_orgs_cache(&self) {
        let stamp = self.repos_stamp();
        if self.repo_cache.borrow().orgs_stamp == Some(stamp) {
            return;
        }
        let me = &self.data.profile.login;
        let mut out: Vec<(String, usize)> = Vec::new();
        let mut mine = 0usize;
        for r in &self.data.repos {
            if r.owner.eq_ignore_ascii_case(me) {
                mine += 1;
            } else {
                match out.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(&r.owner)) {
                    Some((_, n)) => *n += 1,
                    None => out.push((r.owner.clone(), 1)),
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        let mut c = self.repo_cache.borrow_mut();
        c.orgs = out;
        c.mine_count = mine;
        c.orgs_stamp = Some(stamp);
    }

    pub(crate) fn mine_repo_count(&self) -> usize {
        self.ensure_orgs_cache();
        self.repo_cache.borrow().mine_count
    }

    fn ensure_pins_cache(&self) {
        let stamp = self.pins_stamp();
        if self.repo_cache.borrow().pins_stamp == Some(stamp) {
            return;
        }
        let repos = &self.data.repos;
        let list: Vec<usize> = if let Some(pins) = &self.pins {
            pins.iter().filter_map(|p| repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(p))).collect()
        } else if self.cfg.pinned.is_empty() {
            (0..repos.len().min(3)).collect()
        } else {
            self.cfg.pinned.iter().filter_map(|p| repos.iter().position(|r| r.name.eq_ignore_ascii_case(p) || r.full_name.eq_ignore_ascii_case(p))).collect()
        };
        let set: std::collections::HashSet<usize> = list.iter().copied().collect();
        let mut c = self.repo_cache.borrow_mut();
        c.pinned = list;
        c.pinned_set = set;
        c.pins_stamp = Some(stamp);
    }

    /// Indices into `data.repos` matching the current owner, visibility/fork filter, search, and sort.
    pub(crate) fn filtered_repos(&self) -> Vec<usize> {
        use std::hash::Hash;
        let stamp = hash_stamp(|h| {
            self.repos_stamp().hash(h);
            std::mem::discriminant(&self.owner).hash(h);
            if let OwnerFilter::Org(o) = &self.owner {
                o.hash(h);
            }
            self.filter.index().hash(h);
            self.sort.index().hash(h);
            self.search.text.hash(h);
        });
        if self.repo_cache.borrow().shown_stamp == Some(stamp) {
            return self.repo_cache.borrow().shown.clone();
        }
        let q = self.search.text.trim().to_lowercase();
        let all = &self.data.repos;
        let login = &self.data.profile.login;
        let mut shown: Vec<usize> = (0..all.len()).filter(|&i| {
            let r = &all[i];
            if !self.owner.matches(&r.owner, login) || !self.filter.matches(r.private, r.fork) {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            let keys = r.keys();
            keys.name_lower.contains(&q) || keys.desc_lower.contains(&q) || keys.topics_lower.iter().any(|t| t.contains(&q))
        }).collect();
        match self.sort {
            RepoSort::Recent => {} // GitHub's order: most recently pushed first
            RepoSort::Stars => shown.sort_by(|a, b| all[*b].stars.cmp(&all[*a].stars)),
            RepoSort::Name => shown.sort_by(|a, b| all[*a].keys().name_lower.cmp(&all[*b].keys().name_lower)),
        }
        let mut c = self.repo_cache.borrow_mut();
        c.shown = shown.clone();
        c.shown_stamp = Some(stamp);
        shown
    }

    /// Cheap fingerprint of everything [`palette::entries`] reads, so typing in the palette
    /// reuses pre-indexed entries without rebuilding or re-lowercasing them.
    pub(crate) fn palette_stamp(&self) -> u64 {
        use std::hash::Hash;
        hash_stamp(|h| {
            self.pins_stamp().hash(h);
            self.data.pulls.len().hash(h);
            if let Some(p) = self.data.pulls.first() {
                p.number.hash(h);
                std::mem::discriminant(&p.state).hash(h);
            }
            self.data.issues.len().hash(h);
            if let Some(x) = self.data.issues.first() {
                x.number.hash(h);
            }
            std::mem::discriminant(&self.page).hash(h);
            match &self.page {
                Page::Repo(full) => {
                    full.hash(h);
                    self.local.contains_key(full).hash(h);
                }
                Page::Plugin(id) => id.hash(h),
                _ => {}
            }
            self.tab.index().hash(h);
            self.plugin_tab.hash(h);
            std::mem::discriminant(&self.theme).hash(h);
            self.shown_dark.get().hash(h);
            self.looks.light.hash(h);
            self.looks.dark.hash(h);
            self.looks.all.len().hash(h);
            self.assist.claude.hash(h);
            self.assist.harnesses.len().hash(h);
            self.plugins.list.len().hash(h);
            std::mem::discriminant(&self.update).hash(h);
            self.cfg.live().hash(h);
            self.status.can_load().hash(h);
        })
    }

    /// Says `data` changed in place (a count, a star, a state), so what's cached from it is
    /// rebuilt on the next frame.
    pub(crate) fn data_changed(&mut self) {
        self.data_gen = self.data_gen.wrapping_add(1);
    }

    /// Whether repository `i` is in the sidebar's Pinned section.
    pub(crate) fn is_pinned(&self, i: usize) -> bool {
        self.ensure_pins_cache();
        self.repo_cache.borrow().pinned_set.contains(&i)
    }

    /// Pins or unpins repository `i`. The first change starts from what the sidebar shows, so
    /// the pins it had by default stay.
    pub(crate) fn toggle_pin(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let mut list: Vec<String> = match &self.pins {
            Some(p) => p.clone(),
            None => pinned(self).into_iter().map(|j| self.data.repos[j].full_name.clone()).collect(),
        };
        let name = self.data.repos[i].name.clone();
        if let Some(at) = list.iter().position(|p| p.eq_ignore_ascii_case(&full)) {
            list.remove(at);
            self.notify(format!("Unpinned {name}"));
        } else {
            list.push(full);
            self.notify(format!("Pinned {name}"));
        }
        prefs::save_pins(&list);
        self.pins = Some(list);
        self.sync_tray();
    }
}

fn hash_stamp(f: impl FnOnce(&mut std::collections::hash_map::DefaultHasher)) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f(&mut h);
    h.finish()
}

/// The organizations whose repositories are loaded, with how many each, most first.
pub(crate) fn orgs(s: &App) -> Vec<(String, usize)> {
    s.ensure_orgs_cache();
    s.repo_cache.borrow().orgs.clone()
}

/// The pinned repositories: the names in REPOHUB_PINNED, else the three most recently pushed.
pub(crate) fn pinned(s: &App) -> Vec<usize> {
    s.ensure_pins_cache();
    s.repo_cache.borrow().pinned.clone()
}
