//! Loading and sending: the releases and tags read in the background (late answers dropped), saving and
//! publishing, and what the Releases section does on `App`.

use super::*;

// ------------------------------------------------------------------ loading and sending

#[cfg(test)]
thread_local! {
    /// Tests run the requests at once, as a headless still does.
    pub(crate) static BLOCKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    if blocking() {
        let r = pollster::block_on(work());
        done(s, r);
        return true;
    }
    false
}

/// Requests run before the first frame: a headless still (`REPOHUB_BLOCKING=1`), or a test.
fn blocking() -> bool {
    #[cfg(test)]
    if BLOCKING.with(|b| b.get()) {
        return true;
    }
    !cfg!(target_arch = "wasm32") && std::env::var("REPOHUB_BLOCKING").is_ok_and(|v| v == "1")
}

const NOT_READY: &str = "The app isn't ready to do this yet: try again in a moment.";

impl App {
    /// Reads the releases and tags of repository `i` (`again`: even when they're already here;
    /// what's on screen stays until the answer).
    pub(crate) fn load_releases(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let repo = r.full_name.clone();
        let same = self.releases.list_of(&repo);
        if same.is_some_and(|l| !again && !matches!(l.load, Load::Failed(_))) {
            return;
        }
        let kept = if same.is_some() { self.releases.list.take().and_then(|l| match l.load { Load::Ready(x) => Some(x), _ => None }) } else { None };
        let epoch = self.releases.next_epoch();
        crate::state::trace("releases", &repo, &again);
        if self.data.sample {
            let listing = sample(r, &self.data.profile.login, self.now);
            let latest = latest_of(&listing.releases).map(|l| Latest { id: l.id, tag: l.tag.clone() });
            self.releases.list = Some(List { repo: repo.clone(), load: Load::Loading, page: 1, more: false, loading_more: false, epoch });
            self.apply_releases(self.load_gen, epoch, Ok((listing, false, latest)));
            return;
        }
        self.releases.list = Some(List { repo: repo.clone(), load: kept.map_or(Load::Loading, Load::Ready), page: 1, more: false, loading_more: false, epoch });
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_release_listing(&cfg, &repo).await }, move |s: &mut App, r| s.apply_releases(account, epoch, r)) {
            self.apply_releases(account, epoch, Err(NOT_READY.into()));
        }
    }

    /// GitHub's answer for the first read of the section asked with `epoch`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn apply_releases(&mut self, account: u64, epoch: u64, r: Result<(Listing, bool, Option<Latest>), String>) {
        if self.load_gen != account {
            return;
        }
        let Some(l) = self.releases.list.as_mut().filter(|l| l.epoch == epoch) else { return };
        match r {
            Ok((listing, more, latest)) => {
                self.releases.latest.insert(key(&l.repo), Load::Ready(latest));
                l.load = Load::Ready(listing);
                l.page = 1;
                l.more = more;
            }
            // A reload that fails keeps what was read before, and says so.
            Err(e) if matches!(l.load, Load::Ready(_)) => self.notify_problem(e),
            Err(e) => l.load = Load::Failed(e),
        }
    }

    /// The next page of releases.
    pub(crate) fn load_more_releases(&mut self) {
        let Some(l) = self.releases.list.as_mut() else { return };
        if l.loading_more || !l.more || !matches!(l.load, Load::Ready(_)) {
            return;
        }
        let (repo, epoch, page) = (l.repo.clone(), l.epoch, l.page + 1);
        l.loading_more = true;
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::load_releases(&cfg, &repo, page).await }, move |s: &mut App, r| s.apply_more_releases(account, epoch, page, r)) {
            self.apply_more_releases(account, epoch, page, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn apply_more_releases(&mut self, account: u64, epoch: u64, page: usize, r: Result<(Vec<Release>, bool), String>) {
        if self.load_gen != account {
            return;
        }
        let Some(l) = self.releases.list.as_mut().filter(|l| l.epoch == epoch) else { return };
        l.loading_more = false;
        match r {
            Ok((rows, more)) => {
                if let Load::Ready(listing) = &mut l.load {
                    // A release published meanwhile shifts the pages: one already listed isn't listed twice.
                    let fresh: Vec<Release> = rows.into_iter().filter(|x| !listing.releases.iter().any(|y| y.id == x.id)).collect();
                    listing.releases.extend(fresh);
                    l.page = page;
                    l.more = more;
                }
            }
            Err(e) => self.notify_problem(format!("More releases couldn't load: {e}")),
        }
    }

    /// Asks GitHub for repository `i`'s latest release, for the header (`again`: even when known;
    /// the pill stays until the answer).
    pub(crate) fn load_latest_release(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, k) = (r.full_name.clone(), key(&r.full_name));
        match self.releases.latest.get(&k) {
            Some(Load::Loading) => return,
            Some(Load::Ready(_)) if !again => return,
            _ => {}
        }
        if self.data.sample {
            let listing = sample(r, &self.data.profile.login, self.now);
            let latest = latest_of(&listing.releases).map(|l| Latest { id: l.id, tag: l.tag.clone() });
            self.releases.latest.insert(k, Load::Ready(latest));
            return;
        }
        // No window yet (and no headless still): it's asked when the page opens.
        if !self.cfg.live() || (self.handle.is_none() && !blocking()) {
            return;
        }
        // The tag on screen stays until the answer.
        if !matches!(self.releases.latest.get(&k), Some(Load::Ready(_))) {
            self.releases.latest.insert(k.clone(), Load::Loading);
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        let done = move |s: &mut App, r: Result<Option<Release>, String>| {
            if s.load_gen != account {
                return;
            }
            // The header just doesn't show a tag when this fails; the section says why.
            s.releases.latest.insert(k, r.map(|l| l.map(|l| Latest { id: l.id, tag: l.tag })).into());
        };
        run(self, move || async move { github::load_latest_release(&cfg, &repo).await }, done);
    }

    /// After a refresh: the repository on screen (`i`) reads its releases again (the section's
    /// list, or the header's latest release), and the other repositories' are forgotten.
    pub(crate) fn releases_refreshed(&mut self, i: Option<usize>) {
        let keep = i.and_then(|i| self.data.repos.get(i)).map(|r| key(&r.full_name));
        self.releases.latest.retain(|k, _| Some(k) == keep.as_ref());
        if self.releases.list.as_ref().is_some_and(|l| Some(key(&l.repo)) != keep) {
            self.releases.list = None;
            self.releases.expanded.clear();
        }
        match i {
            Some(i) if self.tab == RepoTab::Releases => self.load_releases(i, true),
            Some(i) => self.load_latest_release(i, true),
            None => {}
        }
    }

    // -------------------------------------------------------------- the form

    /// Opens the New release form of repository `i` on its Releases section; `tag`: for a tag
    /// that has no release yet.
    pub(crate) fn open_release_form(&mut self, i: usize, tag: Option<String>) {
        // Another form with typed text isn't replaced without asking.
        let again = tag.clone();
        if self.ask_first(&[crate::state::Draft::Release], move |s| s.open_release_form(i, again)) {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, default) = (r.full_name.clone(), r.branch.clone());
        if self.tab != RepoTab::Releases {
            self.set_tab(i, RepoTab::Releases);
        }
        self.load_branches(i, false);
        let mut f = Form::new(&repo, &default);
        if let Some(t) = tag {
            f.opened[0] = t.clone();
            f.tag = TextEdit::new(t);
            f.tag.focused = false;
            f.title.focused = true;
        }
        self.releases.form = Some(f);
        self.releases.confirm = None;
        self.picker = None;
    }

    /// Opens draft `id` of repository `i` in the form.
    pub(crate) fn edit_draft(&mut self, i: usize, id: u64) {
        if self.ask_first(&[crate::state::Draft::Release], move |s| s.edit_draft(i, id)) {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let (repo, default) = (r.full_name.clone(), r.branch.clone());
        let Some(d) = self.releases.listing_of(&repo).and_then(|l| l.releases.iter().find(|x| x.id == id)).cloned() else { return };
        if self.tab != RepoTab::Releases {
            self.set_tab(i, RepoTab::Releases);
        }
        self.load_branches(i, false);
        let mut f = Form::of(&repo, &d, &default);
        f.title.focused = true;
        self.releases.form = Some(f);
        self.releases.confirm = None;
        self.picker = None;
    }

    pub(crate) fn close_release_form(&mut self) {
        self.releases.form = None;
        if self.picker == Some(BranchSlot::ReleaseTarget) {
            self.picker = None;
        }
    }

    /// The branch picker chose the branch a new tag is made from.
    pub(crate) fn pick_release_target(&mut self, branch: String) {
        if let Some(f) = self.releases.form.as_mut() {
            f.target = branch;
        }
    }

    /// Saves the form as a draft (`publish` false), or publishes it. Publishing only happens
    /// once its question was answered: the first call asks it.
    pub(crate) fn send_release(&mut self, publish: bool) {
        if actions::blocked(self).is_some() {
            return;
        }
        let Releases { list, form, .. } = &mut self.releases;
        let Some(f) = form.as_mut() else { return };
        let listing = list.as_ref().filter(|l| l.repo.eq_ignore_ascii_case(&f.repo)).and_then(|l| l.load.ready());
        let new = match f.release(publish, listing) {
            Ok(n) => n,
            Err(e) => {
                f.submit = Submit::Failed(e);
                f.confirm = false;
                return;
            }
        };
        if publish && !f.confirm {
            f.confirm = true;
            return;
        }
        if !f.submit.start() {
            return;
        }
        f.publishing = publish;
        f.confirm = false;
        let w = match f.draft_id {
            Some(id) => Write::EditRelease { id, release: new },
            None => Write::Release(new),
        };
        let repo = f.repo.clone();
        let (cfg, account, back) = (self.cfg.clone(), self.load_gen, repo.clone());
        if !run(self, move || async move { github::run(&cfg, &repo, w).await }, move |s: &mut App, r| s.release_sent(account, &back, r))
            && let Some(f) = self.releases.form.as_mut()
        {
            f.submit = Submit::Failed(NOT_READY.into());
        }
    }

    /// The form's write finished: it closes and the list shows the release, or it stays with
    /// what was typed and GitHub's reason.
    fn release_sent(&mut self, account: u64, repo: &str, r: Result<github::Done, String>) {
        if self.load_gen != account {
            return;
        }
        let ours = self.releases.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(repo) && f.submit.is_sending());
        match r {
            Ok(done) => {
                if ours {
                    self.close_release_form();
                }
                self.apply_done(repo, done);
            }
            Err(e) => match self.releases.form.as_mut().filter(|_| ours) {
                Some(f) => f.submit = Submit::Failed(e),
                None => self.notify_problem(e),
            },
        }
    }

    /// **Publish** on draft `id` of repository `i`, once its question was answered (`confirm`).
    pub(crate) fn publish_draft(&mut self, i: usize, id: u64) {
        if actions::blocked(self).is_some() || self.releases.confirm != Some(id) {
            return;
        }
        let Some(repo) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        self.releases.confirm = None;
        if !self.releases.busy.insert(id) {
            return;
        }
        let (cfg, account, back) = (self.cfg.clone(), self.load_gen, repo.clone());
        let done = move |s: &mut App, r: Result<github::Done, String>| {
            if s.load_gen != account {
                return;
            }
            s.releases.busy.remove(&id);
            match r {
                Ok(done) => s.apply_done(&back, done),
                Err(e) => s.notify_problem(e),
            }
        };
        if !run(self, move || async move { github::run(&cfg, &repo, Write::PublishRelease { id }).await }, done) {
            self.releases.busy.remove(&id);
            self.notify_problem(NOT_READY);
        }
    }

    /// A release was saved or published in `repo` (`apply_done`): the list and the header
    /// show it at once.
    pub(crate) fn release_done(&mut self, repo: &str, rel: Release) {
        self.releases.busy.remove(&rel.id);
        let msg = match (rel.draft, rel.tag.is_empty()) {
            (true, _) => "Draft saved".to_owned(),
            (false, true) => "Published the release".to_owned(),
            (false, false) => format!("Published {}", rel.tag),
        };
        if !rel.draft && !rel.prerelease && rel.id != 0 {
            self.releases.latest.insert(key(repo), Load::Ready(Some(Latest { id: rel.id, tag: rel.tag.clone() })));
        }
        if rel.id == 0 {
            // GitHub's answer couldn't be read: the list is read again.
            if let Some(i) = self.repo_index_of(repo).filter(|_| self.tab == RepoTab::Releases) {
                self.load_releases(i, true);
            }
        } else if let Some(List { load: Load::Ready(listing), .. }) = self.releases.list.as_mut().filter(|l| same_repo(&l.repo, repo)) {
            if !rel.draft && !listing.tags.iter().any(|t| t.name == rel.tag) {
                listing.tags.insert(0, Tag { name: rel.tag.clone(), sha: String::new() });
            }
            match listing.releases.iter().position(|x| x.id == rel.id) {
                Some(at) => listing.releases[at] = rel,
                None => listing.releases.insert(0, rel),
            }
        }
        self.notify(msg);
    }

    /// **Generate notes**: asks GitHub for the notes since the previous release. They go in the
    /// field, or wait for a yes when it has notes you typed.
    pub(crate) fn generate_notes(&mut self) {
        let sample = self.data.sample;
        if !sample && actions::blocked(self).is_some() {
            return;
        }
        let epoch = self.releases.next_epoch();
        let Some(f) = self.releases.form.as_mut() else { return };
        let (tag, target, repo) = (f.tag.text.trim().to_owned(), f.target.clone(), f.repo.clone());
        if tag_problem(&tag).is_some() {
            f.notes_job = NotesJob::Failed("Pick a tag first: GitHub writes the notes up to it.".into());
            return;
        }
        f.notes_job = NotesJob::Asking;
        f.notes_epoch = epoch;
        f.offered = None;
        if sample {
            let previous = self.releases.latest_of(&repo).map(|l| l.tag.clone()).filter(|p| *p != tag);
            let text = self.data.repos.iter().find(|r| same_repo(&r.full_name, &repo))
                .map(|r| sample_notes(r, &self.data.profile.login, &target, &tag, previous.as_deref()))
                .ok_or_else(|| "This repository isn't in the sample data.".to_owned());
            self.notes_arrived(self.load_gen, epoch, text);
            return;
        }
        let (cfg, account) = (self.cfg.clone(), self.load_gen);
        if !run(self, move || async move { github::generate_release_notes(&cfg, &repo, &tag, &target, None).await }, move |s: &mut App, r| s.notes_arrived(account, epoch, r)) {
            self.notes_arrived(account, epoch, Err(NOT_READY.into()));
        }
    }

    pub(crate) fn notes_arrived(&mut self, account: u64, epoch: u64, r: Result<String, String>) {
        if self.load_gen != account {
            return;
        }
        let Some(f) = self.releases.form.as_mut().filter(|f| f.notes_epoch == epoch && f.notes_job == NotesJob::Asking) else { return };
        match r {
            Ok(text) if f.typed_notes() => {
                f.offered = Some(text);
                f.notes_job = NotesJob::Idle;
            }
            Ok(text) => {
                f.notes = TextEdit::new(text.clone());
                f.generated = Some(text);
                f.notes_job = NotesJob::Idle;
            }
            Err(e) => f.notes_job = NotesJob::Failed(e),
        }
    }

    /// The answer to "Replace your notes?": GitHub's go in the field, or yours stay.
    pub(crate) fn take_offered_notes(&mut self, replace: bool) {
        let Some(f) = self.releases.form.as_mut() else { return };
        let Some(text) = f.offered.take() else { return };
        if replace {
            f.notes = TextEdit::new(text.clone());
            f.generated = Some(text);
        }
    }

    /// Esc on the Releases section: the question on screen, then the form. True when it closed
    /// something.
    pub(crate) fn release_escape(&mut self) -> bool {
        let Some(i) = self.repo_index().filter(|_| self.tab == RepoTab::Releases) else { return false };
        if self.releases.confirm.take().is_some() {
            return true;
        }
        let repo = self.data.repos[i].full_name.clone();
        let Some(f) = self.releases.form.as_mut().filter(|f| f.repo.eq_ignore_ascii_case(&repo)) else { return false };
        if f.confirm {
            f.confirm = false;
        } else if f.offered.take().is_none() {
            // What's typed in it isn't thrown away without asking.
            self.cancel_release_form();
        }
        true
    }

    /// For headless stills: with the Releases section on screen (`-- tab releases`) its list is
    /// read before the first frame, and `-- release new|notes|edit|publish` opens the form (with
    /// generated notes, the first draft, or the publish question). The header's latest release
    /// is read for any repository page.
    pub(crate) fn releases_still(&mut self, what: Option<String>) {
        let Some(i) = self.repo_index() else { return };
        self.load_latest_release(i, false);
        if what.is_none() && self.tab != RepoTab::Releases {
            return;
        }
        self.tab = RepoTab::Releases;
        self.reveal_tab = true;
        self.load_releases(i, false);
        let repo = self.data.repos[i].full_name.clone();
        match what.as_deref() {
            Some("edit") => {
                if let Some(id) = self.releases.listing_of(&repo).and_then(|l| l.releases.iter().find(|r| r.draft)).map(|r| r.id) {
                    self.edit_draft(i, id);
                }
            }
            Some(w @ ("new" | "notes" | "publish")) => {
                self.open_release_form(i, None);
                let next = self.releases.listing_of(&repo).map(|l| next_versions(l.tag_names())).unwrap_or_default();
                if let (Some(f), Some(tag)) = (self.releases.form.as_mut(), next.into_iter().nth(1).filter(|_| w != "new")) {
                    f.tag = TextEdit::new(tag);
                }
                if w != "new" {
                    self.generate_notes();
                }
                if w == "publish"
                    && let Some(f) = self.releases.form.as_mut()
                {
                    f.confirm = true;
                }
            }
            _ => {}
        }
    }
}
