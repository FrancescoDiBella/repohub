//! Navigation: pages, a repository's sections, the place saved for the next start, and what Esc and
//! the back button close.

use super::*;

impl App {
    /// The index into `data.repos` of the repository currently open, if any.
    pub fn repo_index(&self) -> Option<usize> {
        match &self.page {
            Page::Repo(full) => self.repo_index_of(full),
            _ => None,
        }
    }

    /// Finds a repository's index by its full name (case-insensitive).
    pub fn repo_index_of(&self, full: &str) -> Option<usize> {
        self.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full))
    }

    /// Opens a page. Leaving a repository's section closes what belonged to it.
    pub(crate) fn go(&mut self, p: Page) {
        // Moving before the saved place was restored: you chose where to be.
        if self.pending_place.take().is_some() {
            self.save_places = true;
        }
        if self.page != p {
            state::trace("page", &self.page, &p);
            self.close_pull_diff();
            self.leave_tab();
        }
        self.page = p;
        self.thread = None;
        self.open_run = None;
        if self.page == Page::Notifications {
            self.load_inbox();
        }
        self.toast = None;
        self.sync_tray();
        self.save_place();
    }

    /// The place on screen, for the next start.
    pub(crate) fn place(&self) -> state::Place {
        let (page, repo) = match &self.page {
            Page::Overview => ("overview", None),
            Page::Repos => ("repos", None),
            Page::Repo(full) => ("repo", Some(full.clone())),
            Page::Pulls => ("pulls", None),
            Page::Issues => ("issues", None),
            Page::Actions => ("actions", None),
            Page::Notifications => ("notifications", None),
            Page::Settings => ("settings", None),
            Page::Plugin(_) => ("overview", None),
        };
        state::Place { page: page.into(), tab: repo.as_ref().map(|_| self.tab), repo }
    }

    pub(super) fn save_place(&mut self) {
        if self.save_places && !self.data.sample_or_empty() {
            prefs::set("place", self.place().to_json());
        }
        self.save_layout();
        self.save_filters();
    }

    /// The sidebar and the list's sort and owner, saved when they changed.
    pub(super) fn save_layout(&mut self) {
        let now = state::Layout { sidebar_width: self.sidebar.width(), sidebar_open: self.sidebar.is_open(), sort: self.sort, owner: self.owner.clone() };
        if self.saved_layout.as_ref() != Some(&now) {
            prefs::set("layout", now.to_json());
            self.saved_layout = Some(now);
        }
    }

    /// Opens the place saved last time, once the repositories are here. A repository that's
    /// gone falls back to the Repositories page.
    pub(crate) fn restore_place(&mut self) {
        let Some(p) = self.pending_place.take() else { return };
        self.save_places = true;
        state::trace("restore", &"place", &p);
        let page = match p.page.as_str() {
            "repos" => Page::Repos,
            "pulls" => Page::Pulls,
            "issues" => Page::Issues,
            "actions" => Page::Actions,
            "notifications" => Page::Notifications,
            "settings" => Page::Settings,
            "repo" => match p.repo.as_ref().and_then(|full| self.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(full))) {
                Some(i) => {
                    self.open(i);
                    if let Some(t) = p.tab {
                        self.set_tab(i, t);
                    }
                    return;
                }
                None => Page::Repos,
            },
            _ => Page::Overview,
        };
        self.go(page);
    }

    /// Tidies up the repository section on screen before another one (or another page) shows:
    /// the open drawing is closed (its last edits are written first), and pending confirmations
    /// and error messages don't follow you to the next place.
    pub(crate) fn leave_tab(&mut self) {
        self.flush_board();
        self.board = None;
        self.confirm_delete = None;
        self.term_error = None;
        self.picker = None;
        self.confirm_merge = None;
        self.thread = None;
        self.open_run = None;
        self.close_pull_diff();
        self.commits.leave();
        self.releases.leave();
    }

    /// Switches the repository page of repository `i` to `tab`.
    /// The sections a repository's page shows, in order: Claude only while it's on.
    pub(crate) fn tabs(&self) -> Vec<RepoTab> {
        let local = match &self.page {
            Page::Repo(full) => self.local.contains_key(full),
            _ => false,
        };
        RepoTab::ALL.into_iter().filter(|t| (*t != RepoTab::Claude || self.assist.claude) && (*t != RepoTab::Changes || local)).collect()
    }

    pub(crate) fn set_tab(&mut self, i: usize, tab: RepoTab) {
        self.plugin_tab = None;
        self.recheck_missing_clone(i);
        let tab = if self.tabs().contains(&tab) { tab } else { RepoTab::Code };
        if self.tab != tab {
            state::trace("tab", &self.tab, &tab);
            self.leave_tab();
        }
        self.tab = tab;
        self.reveal_tab = true;
        self.save_place();
        match tab {
            RepoTab::Notes => {
                self.load_notes(i);
                self.sync_now();
            }
            RepoTab::Terminal => self.open_terminal(i),
            RepoTab::Changes => self.load_changes(i),
            RepoTab::Commits => self.load_history(i, false),
            RepoTab::Releases => self.load_releases(i, false),
            _ => {}
        }
    }

    /// The phone's back button: closes what's open, goes up a level, and leaves the app from
    /// the first page.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn back(&mut self) {
        if self.palette.is_some() {
            self.palette = None;
        } else if self.editing_board() {
            if let Page::Repo(full) = &self.page
                && let Some(i) = self.repo_index_of(full)
            {
                self.close_board(i);
            }
        } else if self.thread.is_some() && self.pull_diff_of.is_none() {
            self.thread = None;
        } else if self.open_run.is_some() {
            self.open_run = None;
        } else if self.pull_diff_of.is_some() {
            self.close_pull_diff();
        } else if self.commits.open.is_some() {
            self.close_commit(None);
        } else if self.commits.compare.is_some() {
            self.close_compare();
        } else if !self.release_escape() {
            match self.page {
                Page::Overview => net::leave_app(),
                Page::Repo(_) => self.go(Page::Repos),
                _ => self.go(Page::Overview),
            }
        }
    }

    pub(crate) fn open(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
            self.local_dir.clear();
            self.local_file = None;
        }
        self.go(Page::Repo(full));
        self.set_tab(i, RepoTab::Code);
        self.load_branches(i, true);
        self.load_detail(i);
        self.load_latest_release(i, false);
    }

    /// Opens plugin `p`'s section on repository `i`'s page.
    pub(crate) fn open_plugin_tab(&mut self, i: usize, p: Option<String>) {
        let Some(p) = p else { return };
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        if !matches!(&self.page, Page::Repo(f) if f.eq_ignore_ascii_case(&full)) {
            self.go(Page::Repo(full));
        }
        self.leave_tab();
        self.plugin_tab = Some(p);
    }

    /// Esc: closes what's open on top of the page, the most recent first (a question, a picker,
    /// an editor, then a form, a conversation…). Only what's on screen: a form left on another
    /// repository's page stays. True when it closed something.
    pub(crate) fn escape(&mut self, tx: &mut Tx) -> bool {
        let form_shown = matches!(&self.page, Page::Repo(full) if self.form.as_ref().is_some_and(|f| f.repo.eq_ignore_ascii_case(full)));
        let thread_edit = self.thread.as_mut().map(|t| &mut t.edit);
        if self.confirm_merge.is_some() {
            self.confirm_merge = None;
        } else if self.confirm_delete.is_some() {
            self.confirm_delete = None;
        } else if self.picker.is_some() {
            self.picker = None;
        } else if let Some(e) = thread_edit.filter(|e| e.picker.is_some() || e.title.is_some() || e.body.is_some()) {
            // A picker, or the title or description being edited: closed without saving.
            if e.picker.is_some() {
                e.picker = None;
            } else if e.title.is_some() {
                e.title = None;
            } else {
                e.body = None;
            }
        } else if form_shown && self.issue_picks.picker.is_some() {
            self.issue_picks.picker = None;
        } else if form_shown {
            self.form = None;
            self.issue_picks = Default::default();
        } else if self.release_escape() {
            // The publish question, GitHub's notes on offer, or the release form.
        } else if self.pull_diff_of.is_some() {
            self.close_pull_diff();
        } else if self.thread.is_some() {
            self.thread = None;
        } else if self.open_run.is_some() {
            self.open_run = None;
        } else if self.commits.open.is_some() {
            self.close_commit(Some(tx));
        } else if self.commits.compare.is_some() {
            self.close_compare();
        } else if self.local_file.is_some() && matches!(self.page, Page::Repo(_)) {
            self.local_file = None;
        } else if self.toast.is_some() {
            self.toast = None;
        } else {
            return false;
        }
        true
    }

    /// Shows the Repositories page for `owner`.
    pub(crate) fn show_owner(&mut self, owner: OwnerFilter) {
        state::trace("owner", &self.owner, &owner);
        self.owner = owner;
        self.repos_limit = pages::REPOS_PAGE;
        self.go(Page::Repos);
    }
}
