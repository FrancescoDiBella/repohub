//! What Settings changes: the account (and signing out), the theme and the looks, plugins, and the
//! coding assistants.

use super::*;

impl App {
    /// Who owns the account setting: the environment, nobody yet, or the form in Settings.
    pub(crate) fn account(&self) -> state::Account {
        if self.cfg.account_locked {
            state::Account::Locked
        } else if self.cfg.live() {
            state::Account::SignedIn
        } else {
            state::Account::SignedOut
        }
    }

    /// Drops in-flight GitHub loads from the previous account and clears per-account caches,
    /// forms, filters and saved place, while keeping local clone sessions (terminals and Claude
    /// chats).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn reset_account_state(&mut self) {
        self.load_gen = self.load_gen.wrapping_add(1);
        self.board_load_gen = self.board_load_gen.wrapping_add(1);
        // The saved copy and the tagged answers were the other account's.
        self.forget_copy();
        // Drawings are files on this computer (or the server's notes folder), not account data:
        // writes in flight and queued ones still finish, so no edit is lost on an account switch.
        self.leave_tab();
        self.inbox = None;
        self.inbox_busy.clear();
        // Another account has other repositories and labels.
        self.pull_filter.clear();
        self.issue_filter.clear();
        self.choices.clear();
        self.issue_picks = triage::Picks::default();
        self.runs_filter.clear();
        self.data = Data::default();
        self.data_gen = self.data_gen.wrapping_add(1);
        self.repos_limit = pages::REPOS_PAGE;
        self.refresh_clone_cache();
        self.tab = RepoTab::Code;
        self.plugin_tab = None;
        self.pending_place = None;
        self.save_places = true;
        self.filter = RepoFilter::All;
        self.owner = OwnerFilter::All;
        self.pr_filter = PullState::Open;
        self.search.clear();
        self.branch_filter.clear();
        self.details.clear();
        self.branches.clear();
        self.branch_of.clear();
        self.local.clear();
        self.local_dir.clear();
        self.local_file = None;
        self.files_from_github = false;
        self.notes.clear();
        self.note_name.clear();
        self.form = None;
        self.form_title = None;
        self.pull_meta = None;
        self.busy.clear();
        self.diff.set_files(Vec::new());
        self.diff_of = None;
        self.diff_load = None;
        self.close_pull_diff();
        self.commits = commits::Commits::default();
        self.releases = releases::Releases::default();
        self.changes = None;
        self.commit_msg.clear();
        self.git_error = None;
        self.merge_preview = None;
        self.claude_prompt.clear();
        self.clone_path.clear();
        self.loaded_at = None;
        self.last_load = None;
        self.load_clock = None;
        self.loaded_clock = None;
        self.open_pulls_page = 1;
        self.closed_pulls_page = 1;
        self.issues_page = 1;
        self.loading_more_pulls = false;
        self.loading_more_issues = false;
        // `self.data` is now empty, so `go()` -> `save_place()` skips writing `"place"`.
        prefs::set("place", state::Place { page: "overview".into(), tab: None, repo: None }.to_json());
        self.go(Page::Overview);
    }

    /// Saves the account typed in Settings and loads its data. An empty token field keeps the
    /// saved token; `sign_out` removes both.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn save_account(&mut self, sign_out: bool) {
        let user = self.acct_user.text.trim().trim_start_matches('@').to_owned();
        let typed = self.acct_token.text.trim().to_owned();
        let (user, token) = if sign_out {
            (None, None)
        } else {
            (Some(user).filter(|u| !u.is_empty()), Some(typed).filter(|t| !t.is_empty()).or_else(|| self.cfg.token.clone()))
        };
        if !sign_out && user.is_none() && token.is_none() {
            self.acct_error = Some("Type your GitHub user name, a token, or both.".into());
            return;
        }
        if let Err(e) = config::save_account(user.as_deref(), token.as_deref()) {
            self.acct_error = Some(e);
            return;
        }
        // Ensure neither GITHUB_TOKEN nor GITHUB_USER lingers in the process environment.
        unsafe {
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("GITHUB_USER");
        }
        let mut cfg = (*self.cfg).clone();
        cfg.user = user;
        cfg.token = token;
        let user_file = config::user_env_file().display().to_string();
        cfg.env_file = Some(user_file.clone());
        cfg.sources.user = cfg.user.as_ref().map(|_| config::Source::UserFile(user_file.clone()));
        cfg.sources.token = cfg.token.as_ref().map(|_| config::Source::UserFile(user_file.clone()));
        // If the token is now saved in the user settings file, ensure the API host
        // is tied to that file or the default; an external project .env or REPOHUB_ENV
        // must not redirect the newly saved token.
        if cfg.token.is_some() && cfg.sources.api != Some(config::Source::Environment) {
            let user_env_api = std::fs::read_to_string(config::user_env_file()).ok().and_then(|s| {
                    config::parse_env(&s).into_iter().find(|(k, v)| k == "GITHUB_API_URL" && !v.trim().is_empty()).map(|(_, v)| v.trim_end_matches('/').to_owned())
                });
            if let Some(user_api) = user_env_api {
                cfg.api = user_api;
                cfg.sources.api = Some(config::Source::UserFile(user_file));
            } else {
                cfg.api = "https://api.github.com".into();
                cfg.sources.api = Some(config::Source::Default);
            }
        }
        self.cfg = Arc::new(cfg);
        self.acct_error = None;
        self.acct_token.clear();
        if sign_out {
            self.acct_user.clear();
        }
        self.reset_account_state();
        if self.cfg.live() {
            self.set_status(Status::Starting);
            self.refresh();
            self.notify("Account saved. Loading your repositories…");
        } else {
            self.now = net::now();
            self.data = Data::sample();
            self.data_gen = self.data_gen.wrapping_add(1);
            self.set_status(Status::Sample);
            self.notify("Signed out. Showing the sample data.");
        }
    }

    /// The phone app: the account is kept by the app's own side (`PUT api/account`), which
    /// answers with the new configuration. The token never comes back to the page.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn save_account(&mut self, sign_out: bool) {
        if self.cfg.account_locked {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        let user = self.acct_user.text.trim().trim_start_matches('@').to_owned();
        let typed = self.acct_token.text.trim().to_owned();
        if !sign_out && user.is_empty() && typed.is_empty() && self.cfg.token.is_none() {
            self.acct_error = Some("Type your GitHub user name, a token, or both.".into());
            return;
        }
        // An empty token keeps the saved one (`null`).
        let body = serde_json::json!({
            "signOut": sign_out,
            "user": user,
            "token": if typed.is_empty() { serde_json::Value::Null } else { typed.into() },
        }).to_string();
        net::spawn(h, move || async move {
            let r = net::request("PUT", "api/account", "application/json", &body).await?;
            if r.status != 200 {
                return Err(if r.body.trim().is_empty() { format!("The account wasn't saved ({}).", r.status) } else { r.body });
            }
            Config::from_json(&r.body).ok_or_else(|| "The account wasn't saved: the app's answer couldn't be read.".to_owned())
        }, move |s: &mut App, r: Result<Config, String>| match r {
            Err(e) => s.acct_error = Some(e),
            Ok(cfg) => {
                s.cfg = Arc::new(cfg);
                s.acct_error = None;
                s.acct_token.clear();
                if sign_out {
                    s.acct_user.clear();
                }
                s.reset_account_state();
                if s.cfg.live() {
                    s.set_status(Status::Starting);
                    s.refresh();
                    s.notify("Account saved. Loading your repositories…");
                } else {
                    s.now = net::now();
                    s.data = Data::sample();
                    s.data_gen = s.data_gen.wrapping_add(1);
                    s.set_status(Status::Sample);
                    s.notify("Signed out. Showing the sample data.");
                }
            }
        });
    }

    /// Picks the theme and remembers it.
    pub(crate) fn set_theme(&mut self, theme: Theme) {
        state::trace("theme", &self.theme, &theme);
        self.theme = theme;
        prefs::save_theme(theme);
    }

    /// Uses look `id` for its appearance and shows that appearance (`shown_dark` is what's on
    /// screen now). One of your looks opens in the editor.
    pub(crate) fn choose_look(&mut self, id: &str, shown_dark: bool) {
        let Some(dark) = self.looks.choose(id) else { return };
        state::trace("look", &shown_dark, &id);
        if dark != shown_dark {
            self.set_theme(if dark { Theme::Dark } else { Theme::Light });
        }
        self.looks.save();
        self.edit_look(id);
    }

    /// Opens look `id` in the editor (nothing for a ready-made one).
    pub(crate) fn edit_look(&mut self, id: &str) {
        self.look_edit = match self.looks.get(id).filter(|t| !t.builtin) {
            Some(t) => LookEditor::of(t),
            None => LookEditor::default(),
        };
    }

    /// Makes an editable copy of look `id` and opens it.
    pub(crate) fn duplicate_look(&mut self, id: &str, shown_dark: bool) {
        if let Some(new) = self.looks.duplicate(id) {
            self.choose_look(&new, shown_dark);
            self.notify("Copied: change it below");
        }
    }

    pub(crate) fn delete_look(&mut self, id: &str) {
        let Some(at) = self.looks.all.iter().position(|t| t.id == id && !t.builtin) else { return };
        let look = self.looks.all[at].clone();
        let (light, dark) = (self.looks.light == id, self.looks.dark == id);
        if self.looks.delete(id) {
            self.looks.save();
            self.look_edit = LookEditor::default();
            self.notify_undo(format!("Deleted {}", look.name), state::Undo::Look { look, at, light, dark });
        }
    }

    /// Changes the look being edited and saves it.
    pub(crate) fn change_look(&mut self, f: impl FnOnce(&mut themes::Look)) {
        let id = self.look_edit.id.clone();
        if let Some(t) = self.looks.get_mut(&id) {
            f(t);
            self.looks.save();
        }
    }

    /// Takes what was typed in the editor: valid colours apply at once.
    pub(crate) fn look_typed(&mut self) {
        let name = self.look_edit.name.text.trim().to_owned();
        let image = self.look_edit.image.text.trim().to_owned();
        let colors: Vec<Option<String>> = self.look_edit.colors.iter().map(|e| themes::parse_hex(&e.text)).collect();
        self.change_look(|t| {
            if !name.is_empty() {
                t.name = name;
            }
            t.image = image;
            for (i, c) in colors.into_iter().enumerate() {
                if let Some(c) = c {
                    t.colors[i] = c;
                }
            }
        });
    }

    /// A plugin asks to open a link or to copy text. Nothing happens until the user answers;
    /// while one request waits, others are dropped, so a plugin can't bury it or nag.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn plugin_asks(&mut self, request: plugins::Request) {
        if self.plugin_ask.is_none() {
            self.plugin_ask = Some(request);
        }
    }

    /// The user's answer to [`App::plugin_asks`].
    pub(super) fn answer_plugin(&mut self, allow: bool, tx: &mut Tx) {
        let Some(request) = self.plugin_ask.take() else { return };
        if !allow {
            return;
        }
        match request.ask {
            plugins::Ask::Open(url) => {
                let _ = tx.open_url(&url);
            }
            plugins::Ask::Copy(text) => {
                tx.copy(text);
                self.notify("Copied");
            }
        }
    }

    /// Installs the plugin typed in Settings (a folder or a Git address) and starts it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn install_plugin(&mut self, example: bool) {
        if self.plugin_installing {
            return;
        }
        if example {
            self.plugin_installed(plugins::create_example(), true);
            return;
        }
        // Cloning can take a while, or never answer: not on the UI thread.
        let source = self.plugin_src.text.clone();
        match self.handle.clone() {
            Some(h) => {
                self.plugin_installing = true;
                self.plugin_error = None;
                net::spawn(h, move || async move { plugins::install(&source) }, |s: &mut App, r| s.plugin_installed(r, false));
            }
            None => self.plugin_installed(plugins::install(&source), false),
        }
    }

    /// An install finished: starts the plugin, or says why it isn't one.
    #[cfg(not(target_arch = "wasm32"))]
    fn plugin_installed(&mut self, result: Result<String, String>, example: bool) {
        self.plugin_installing = false;
        match result {
            Ok(id) => {
                self.plugin_error = None;
                self.plugin_src.clear();
                self.plugins.rescan();
                if let Some(h) = self.handle.clone() {
                    self.plugins.start(&h);
                }
                let name = self.plugins.list.iter().find(|p| p.id == id).map(|p| p.manifest.name.clone()).unwrap_or(id);
                self.notify(if example { format!("Created {name} in your plugins folder: edit it and it reloads") } else { format!("Installed {name}") });
            }
            Err(e) => self.plugin_error = Some(e),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn remove_plugin(&mut self, id: &str) {
        if self.plugin_confirm.as_deref() != Some(id) {
            self.plugin_confirm = Some(id.to_owned());
            return;
        }
        self.plugin_confirm = None;
        if self.plugin_tab.as_deref() == Some(id) {
            self.plugin_tab = None;
        }
        if matches!(&self.page, Page::Plugin(pid) if pid == id) {
            self.settings_section = state::SettingsSection::Plugins;
            self.go(Page::Settings);
        }
        match plugins::remove(id) {
            Ok(()) => {
                self.plugins.list.retain(|p| p.id != id);
                self.notify(format!("Removed {id}"));
            }
            Err(e) => self.plugin_error = Some(e),
        }
    }

    /// Turns the Claude section on or off.
    pub(crate) fn set_claude(&mut self, on: bool) {
        state::trace("claude", &self.assist.claude, &on);
        self.assist.claude = on;
        self.assist.save();
        if !on && self.tab == RepoTab::Claude {
            self.tab = RepoTab::Code;
            self.save_place();
        }
    }

    /// Adds the harness typed in Settings (or `suggested`, a name and command).
    pub(crate) fn add_harness(&mut self, suggested: Option<(&str, &str)>) {
        let (name, cmd) = match suggested {
            Some((n, c)) => (n.to_owned(), c.to_owned()),
            None => (self.harness_name.text.clone(), self.harness_cmd.text.clone()),
        };
        match self.assist.add(&name, &cmd) {
            Ok(()) => {
                self.assist.save();
                self.harness_error = None;
                if suggested.is_none() {
                    self.harness_name.clear();
                    self.harness_cmd.clear();
                }
                self.notify(format!("Added {}", name.trim()));
            }
            Err(e) => self.harness_error = Some(e),
        }
    }

    pub(crate) fn remove_harness(&mut self, n: usize) {
        if let Some(h) = self.assist.remove(n) {
            self.assist.save();
            self.notify(format!("Removed {}", h.name));
        }
    }
}
