//! Updating RepoHub itself (checking, downloading, installing, the channel) and the status-bar
//! widget.

use super::*;

impl App {
    /// Asks GitHub for the Latest build. `asked`: the user pressed Check, so say the outcome.
    pub(crate) fn check_update(&mut self, asked: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(h) = self.handle.clone() else { return };
            let Some(prior) = self.update.start_check(asked) else { return };
            self.update_checked = Some(self.clock);
            let channel = self.update_channel;
            net::spawn(h, move || async move { update::check(channel) }, move |s: &mut App, r| {
                s.apply_update_check(prior, asked, r);
            });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = asked;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn apply_update_check(&mut self, prior: Option<update::Release>, asked: bool, r: Result<update::Release, String>) {
        let outcome = match r {
            Ok(rel) if update::is_newer(&rel, update::build().unwrap_or(""), update::built_at()) => {
                if asked {
                    self.notify(format!("A new version is ready ({})", rel.short()));
                }
                Ok(Some(rel))
            }
            Ok(_) => {
                if asked {
                    self.notify("RepoHub is up to date");
                }
                Ok(None)
            }
            Err(e) => {
                if asked {
                    self.notify_problem(e.clone());
                }
                Err(e)
            }
        };
        self.update.finish_check(prior, outcome);
        self.sync_tray();
    }

    /// Downloads the new version, checks it, puts it in place and restarts into it.
    pub(crate) fn install_update(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(rel) = self.update.available().cloned() else { return };
            let Some(h) = self.handle.clone() else { return };
            let job = UpdateJob::new();
            self.update_cancel = Some(job.clone());
            self.update = update::Update::Downloading { release: rel.clone(), percent: 0 };
            self.sync_tray();
            let progress = h.clone();
            let progress_job = job.clone();
            let done_job = job.clone();
            net::spawn(h, move || async move {
                let bytes = update::download(&rel.url, rel.size, &job.cancel, |pct| {
                    let pj = progress_job.clone();
                    progress.update(move |s: &mut App, _| {
                        if !pj.was_cancelled()
                            && let update::Update::Downloading { percent, .. } = &mut s.update
                        {
                            *percent = pct;
                        }
                    });
                })?;
                let sums = rel.sums.as_deref().ok_or("This build has no checksums (SHA256SUMS), so it isn't installed. Download it from the Releases page.")?;
                let sums = String::from_utf8_lossy(&update::download(sums, 0, &job.cancel, |_| {})?).into_owned();
                if !job.begin_install() {
                    return Err("Download cancelled.".into());
                }
                progress.update(move |s: &mut App, _| {
                    s.update_cancel = None;
                    if matches!(s.update, update::Update::Downloading { .. }) {
                        s.update = update::Update::Installing;
                        s.sync_tray();
                    }
                });
                // Who made this release (the signature), then that the package is the one it lists.
                let signature = match (update::signing_key(), rel.sig.as_deref()) {
                    (Some(_), Some(url)) => Some(String::from_utf8_lossy(&update::download(url, 0, &job.cancel, |_| {})?).into_owned()),
                    _ => None,
                };
                update::verify_signature(&sums, signature.as_deref(), update::signing_key())?;
                update::verify(&bytes, &rel.asset, &sums)?;
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let target = update::install(&bytes, &rel.asset, &exe)?;
                update::relaunch(&target)
            }, move |s: &mut App, r: Result<(), String>| {
                if done_job.was_cancelled() {
                    return;
                }
                s.update_cancel = None;
                match r {
                    Ok(()) => {
                        s.update = update::Update::Installing;
                        if let Some(h) = &s.handle {
                            h.quit();
                        }
                    }
                    Err(e) => {
                        s.notify_problem(e.clone());
                        s.update = update::Update::Failed(e);
                    }
                }
                s.sync_tray();
            });
        }
    }

    /// Stops an in-progress update download and restores the ready update.
    /// Picks where updates come from, and checks there.
    pub(crate) fn set_update_channel(&mut self, channel: update::Channel) {
        if self.update_channel == channel {
            return;
        }
        self.update_channel = channel;
        channel.save();
        // A download from the other channel stops (unless it's already being installed).
        if matches!(self.update, update::Update::Downloading { .. }) {
            self.cancel_update();
        }
        // What was found on the other channel doesn't count here.
        if !matches!(self.update, update::Update::Unavailable(_) | update::Update::Downloading { .. } | update::Update::Installing) {
            self.update = update::Update::Idle;
            self.check_update(false);
        }
    }

    pub(crate) fn cancel_update(&mut self) {
        if !matches!(self.update, update::Update::Downloading { .. }) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(job) = self.update_cancel.take()
            && !job.try_cancel()
        {
            self.update = update::Update::Installing;
            self.sync_tray();
            return;
        }
        if let update::Update::Downloading { release, .. } = &self.update {
            self.update = update::Update::Available(release.clone());
            self.sync_tray();
        }
    }

    /// Shows the status-bar widget (desktop windows; see `tray`).
    pub(super) fn start_tray(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !tray::wanted() || self.tray.is_some() {
                return;
            }
            let Some(h) = self.handle.clone() else { return };
            let entries = tray::menu(self);
            match tray::create(h, &entries, &tray::tooltip(self)) {
                Ok(t) => {
                    self.tray = Some(t);
                    self.tray_menu = entries;
                }
                Err(e) => eprintln!("repohub: no status-bar icon: {e}"),
            }
        }
    }

    /// Pushes the widget's menu again when what it shows changed.
    pub(super) fn sync_tray(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.tray.is_some() {
            let entries = tray::menu(self);
            if entries != self.tray_menu {
                if let Some(t) = &self.tray {
                    t.update(&entries, &tray::tooltip(self));
                }
                self.tray_menu = entries;
            }
        }
    }

    /// A pick from the widget's menu (or a click on its icon: `open`).
    pub(crate) fn tray_action(&mut self, id: &str) {
        let Some(action) = tray::parse(id) else { return };
        let repo = |s: &App, full: &str| s.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full));
        let show = !matches!(action, tray::Action::Refresh | tray::Action::Quit);
        match action {
            tray::Action::Open => {}
            tray::Action::Go(p) => self.go(p),
            tray::Action::MyFailingPulls => {
                let me = self.data.profile.login.trim();
                let is_failing = |p: &model::Pull| {
                    p.checks == Some(model::Checks::Failing)
                        || self.data.runs.iter().any(|r| r.checks == model::Checks::Failing && same_repo(&r.repo, &p.repo) && r.pull_numbers.contains(&p.number))
                };
                let draft_only = !me.is_empty()
                    && !self.data.pulls.iter().any(|p| p.state == PullState::Open && p.author.eq_ignore_ascii_case(me) && is_failing(p))
                    && self.data.pulls.iter().any(|p| p.state == PullState::Draft && p.author.eq_ignore_ascii_case(me) && is_failing(p));
                self.go(Page::Pulls);
                self.pr_filter = if draft_only { PullState::Draft } else { PullState::Open };
                self.pull_filter.clear();
                self.pull_filter.mine = true;
                self.pull_filter.failing = true;
            }
            tray::Action::ReviewRequested => self.show_review_requested(),
            tray::Action::OpenRepo(full) => if let Some(i) = repo(self, &full) { self.open(i) },
            tray::Action::NewIssue(full) => if let Some(i) = repo(self, &full) {
                if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
                    self.open(i);
                }
                self.open_form(i, Compose::Issue);
            },
            tray::Action::Refresh => self.refresh(),
            tray::Action::Update => self.install_update(),
            tray::Action::Quit => if let Some(h) = &self.handle { h.quit() },
        }
        if show && let Some(h) = &self.handle {
            h.open();
        }
    }
}
