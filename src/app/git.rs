//! The local clone: finding it, its files, git (changes, commit, merge), the terminal, Claude and
//! running a coding assistant.

use super::*;

impl App {
    /// Lists repository `i`'s local clone, if one is known (`again`: even if listed already).
    pub(crate) fn load_local(&mut self, i: usize, again: bool) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = self.known_clone(&full) else { return };
        if !again && matches!(self.local.get(&full), Some(Load::Ready(_) | Load::Loading)) {
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        if !matches!(self.local.get(&full), Some(Load::Ready(_))) {
            self.local.insert(full.clone(), Load::Loading);
        }
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::scan(&dir) }, move |s: &mut App, r| {
            if s.load_gen == epoch {
                s.local.insert(full, r.into());
            }
        });
    }

    /// Reads what changed in repository `i`'s local clone, and shows it as a diff.
    pub(crate) fn load_changes(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        if self.diff_of.as_ref() != Some(&diffs::DiffOf::Local(full.clone())) {
            self.diff.set_files(Vec::new());
            if !matches!(&self.diff_of, Some(diffs::DiffOf::Merge(f, _)) if *f == full) {
                self.changes = None;
            }
        }
        self.merge_preview = None;
        self.diff_of = Some(diffs::DiffOf::Local(full.clone()));
        self.diff_load = Some(Load::Loading);
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::changes(&root) }, move |s: &mut App, r| {
            if s.load_gen != epoch || s.diff_of != Some(diffs::DiffOf::Local(full)) {
                return;
            }
            match r {
                Ok(c) => {
                    s.diff.set_files(diffs::local_files(&c));
                    s.changes = Some(c);
                    s.diff_load = Some(Load::Ready(()));
                }
                Err(e) => s.diff_load = Some(Load::Failed(e)),
            }
        });
    }

    /// Runs a git command (`job` names it on screen) in repository `i`'s clone, then reads the
    /// changes and the file list again.
    pub(crate) fn git_run(&mut self, i: usize, job: &'static str, done: &'static str, f: impl FnOnce(&std::path::Path) -> Result<String, String> + Send + 'static) {
        if self.git_job.is_some() {
            return;
        }
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else {
            // Say why nothing happens rather than doing nothing.
            self.notify_problem(format!("There's no local copy of {full} on this computer: pick its folder in the Terminal section, or clone it there."));
            return;
        };
        let Some(h) = self.handle.clone() else { return };
        self.git_job = Some(job);
        self.git_error = None;
        let epoch = self.load_gen;
        net::spawn(h, move || async move { f(&root) }, move |s: &mut App, r| {
            s.git_job = None;
            if s.load_gen != epoch {
                return;
            }
            match r {
                Ok(extra) => {
                    s.notify(if extra.is_empty() { done.to_owned() } else { format!("{done} {extra}") });
                    if done.starts_with("Committed") {
                        s.commit_msg.clear();
                    }
                }
                Err(e) => s.git_error = Some(e),
            }
            if let Some(curr) = s.repo_index_of(&full) {
                s.load_changes(curr);
                s.load_local(curr, true);
            }
        });
    }

    pub(crate) fn commit(&mut self, i: usize, then_push: bool) {
        let msg = self.commit_msg.text.clone();
        if msg.trim().is_empty() {
            self.git_error = Some("Write a commit message first.".into());
            return;
        }
        if then_push {
            self.git_run(i, "Committing and pushing…", "Committed and pushed", move |root| {
                let sha = local::commit_all(root, &msg)?;
                local::push(root).map(|_| sha)
            });
        } else {
            self.git_run(i, "Committing…", "Committed", move |root| local::commit_all(root, &msg));
        }
    }

    /// Shows what merging local `branch` into the checked-out one would bring.
    pub(crate) fn preview_merge(&mut self, i: usize, branch: String) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(root) = self.known_clone(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        let key = diffs::DiffOf::Merge(full, branch.clone());
        self.diff_of = Some(key.clone());
        self.diff_load = Some(Load::Loading);
        self.diff.set_files(Vec::new());
        self.merge_preview = None;
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::merge_preview(&root, &branch) }, move |s: &mut App, r| {
            if s.load_gen != epoch || s.diff_of != Some(key) {
                return;
            }
            match r {
                Ok(p) => {
                    s.diff.set_files(rune_kit::diff::parse(&p.patch));
                    s.merge_preview = Some(p);
                    s.diff_load = Some(Load::Ready(()));
                }
                Err(e) => s.diff_load = Some(Load::Failed(e)),
            }
        });
    }

    /// Merges local `branch` into the checked-out branch.
    pub(crate) fn merge_local(&mut self, i: usize, branch: String) {
        let b = branch.clone();
        self.git_run(i, "Merging…", "Merged:", move |root| match local::merge(root, &b)? {
            local::Merged::Done(summary) => Ok(summary),
            local::Merged::UpToDate => Ok("already up to date".into()),
            local::Merged::Conflicts(files) => Err(format!("{} file{} in conflict: resolve them, then Commit here to finish the merge (or Abort).", files.len(), if files.len() == 1 { "" } else { "s" })),
        });
    }

    /// Opens `path` in repository `i`'s local clone: a folder lists it, a file shows it.
    pub(crate) fn open_local(&mut self, i: usize, path: String, dir: bool) {
        if dir {
            self.local_dir = path;
            self.local_file = None;
            return;
        }
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let Some(root) = self.known_clone(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        self.local_file = Some((path.clone(), Load::Loading));
        let p = path.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { local::read(&root, &p) }, move |s: &mut App, r| {
            if s.load_gen == epoch && s.local_file.as_ref().is_some_and(|(open, _)| *open == path) {
                s.local_file = Some((path, r.into()));
            }
        });
    }

    /// Sends `prompt` to Claude Code in repository `i`'s local clone, continuing its conversation.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ask_claude(&mut self, i: usize, prompt: String) {
        let prompt = prompt.trim().to_owned();
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(dir) = self.known_clone(&full) else { return };
        let Some(h) = self.handle.clone() else { return };
        let chat = self.chats.entry(full.clone()).or_default();
        if prompt.is_empty() || chat.run.is_busy() {
            return;
        }
        chat.items.push(claude::Item::User(prompt.clone()));
        chat.run = claude::Run::Starting;
        chat.run_id += 1;
        let run = chat.run_id;
        let (h2, f2) = (h.clone(), full.clone());
        let started = claude::start(
            &dir,
            &prompt,
            self.claude_mode,
            chat.session.as_deref(),
            move |e| {
                let full = full.clone();
                h.update(move |s: &mut App, _| {
                    if let Some(c) = s.chats.get_mut(&full).filter(|c| c.run_id == run) {
                        c.apply(e);
                    }
                });
            },
            move |problem| {
                h2.update(move |s: &mut App, _| {
                    if let Some(c) = s.chats.get_mut(&f2).filter(|c| c.run_id == run) {
                        c.finished(problem);
                    }
                });
            },
        );
        match started {
            Ok(p) => {
                chat.proc = Some(p);
                self.claude_prompt.clear();
            }
            Err(e) => chat.finished(Some(e)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn ask_claude(&mut self, _: usize, _: String) {}

    /// Stops the answer Claude is writing for repository `i`.
    pub(crate) fn stop_claude(&mut self, i: usize) {
        let Some(c) = self.data.repos.get(i).and_then(|r| self.chats.get_mut(&r.full_name)) else { return };
        if c.run.is_busy() {
            c.run = claude::Run::Stopping;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(p) = &c.proc {
                p.stop();
            }
        }
    }

    /// Forgets repository `i`'s conversation; the next message starts a new Claude Code session.
    pub(crate) fn new_chat(&mut self, i: usize) {
        if let Some(r) = self.data.repos.get(i) {
            self.chats.remove(&r.full_name);
        }
    }

    /// Starts a shell for repository `i` in `dir` (its clone, or where a clone will go).
    pub(crate) fn start_terminal(&mut self, i: usize, dir: std::path::PathBuf, first: Option<String>) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let Some(h) = self.handle.clone() else { return };
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on_output = move || {
            if !pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let p = pending.clone();
                h.update(move |_: &mut App, _| p.store(false, std::sync::atomic::Ordering::SeqCst));
            }
        };
        let (rows, cols) = self.term_grid;
        let term_token = if self.cfg.terminal_pass_token { self.cfg.token.as_deref() } else { None };
        match terminal::Term::spawn(&dir, rows, cols, term_token, on_output) {
            Ok(mut t) => {
                if let Some(cmd) = first {
                    t.write(cmd.as_bytes());
                }
                self.terms.insert(full, t);
                self.term_error = None;
            }
            Err(e) => self.term_error = Some(e),
        }
    }

    /// Starts harness `n` in repository `i`'s Terminal: typed into the running shell, or as the
    /// first command of a new one in the known clone.
    pub(crate) fn run_harness(&mut self, i: usize, n: usize) {
        if self.ask_before_leaving(move |s| s.run_harness(i, n)) {
            return;
        }
        let Some(h) = self.assist.harnesses.get(n).cloned() else { return };
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        self.go(Page::Repo(full));
        self.set_tab(i, RepoTab::Terminal);
        let line = format!("{}\r", h.command);
        let running = self.data.repos.get(i).and_then(|r| self.terms.get(&r.full_name)).is_some_and(|t| t.is_running());
        if running {
            self.term_type(i, &line);
        } else if let Some(dir) = self.data.repos.get(i).map(|r| r.full_name.clone()).and_then(|full| self.known_clone(&full)) {
            if let Some(r) = self.data.repos.get(i) {
                self.terms.remove(&r.full_name);
            }
            self.start_terminal(i, dir, Some(line));
        } else {
            self.term_error = Some(format!("Choose the repository's folder below, then start {} again.", h.name));
        }
    }

    /// Opens the terminal in the repository's known clone, if any.
    pub(super) fn open_terminal(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        if self.terms.get(&full).is_some_and(|t| t.is_running()) {
            return;
        }
        if let Some(dir) = self.known_clone(&full) {
            self.start_terminal(i, dir, None);
        }
    }

    /// Uses the folder typed in the field as repository `i`'s clone.
    /// Uses the folder that has the repository's name but isn't known to be its clone: the
    /// user says it is.
    pub(crate) fn use_lookalike(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        let Some(dir) = self.lookalike_clone(&full) else { return };
        match clones_shim::remember(&full, &dir) {
            Ok(()) => {
                self.clone_dirs.borrow_mut().insert(full.clone(), Some(dir.clone()));
                self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
                self.term_error = None;
                self.notify(format!("Using {} for {full}", dir.display()));
                self.load_local(i, true);
            }
            Err(e) => self.term_error = Some(e),
        }
    }

    pub(crate) fn use_folder(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let full = r.full_name.clone();
        let dir = clones_shim::expand(self.clone_path.text.trim());
        if !dir.is_dir() {
            self.term_error = Some(format!("{} isn't a folder on this computer.", dir.display()));
            return;
        }
        if let Err(e) = clones_shim::remember(&full, &dir) {
            self.term_error = Some(e);
            return;
        }
        self.clone_dirs.borrow_mut().insert(full.clone(), Some(dir.clone()));
        self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
        self.clone_path.clear();
        self.terms.remove(&full);
        self.load_local(i, true);
        // From the Claude tab the folder is all that's needed; the Terminal tab opens a shell there.
        if self.tab == RepoTab::Terminal {
            self.start_terminal(i, dir, None);
        }
    }

    /// Clones repository `i` into the default folder, in its terminal, and remembers where.
    pub(crate) fn clone_repo(&mut self, i: usize) {
        let Some(r) = self.data.repos.get(i) else { return };
        let (full, name, url) = (r.full_name.clone(), r.name.clone(), r.url.clone());
        let parent = self.default_clone_parent.clone();
        if let Err(e) = std::fs::create_dir_all(&parent) {
            self.term_error = Some(format!("Couldn't create {}: {e}", parent.display()));
            return;
        }
        let command = match terminal::clone_command(&url, &name) {
            Ok(c) => c,
            Err(e) => {
                self.term_error = Some(e);
                return;
            }
        };
        let target = parent.join(&name);
        let _ = clones_shim::remember(&full, &target);
        self.clone_dirs.borrow_mut().insert(full.clone(), Some(target));
        self.lookalike_dirs.borrow_mut().insert(full.clone(), None);
        self.terms.remove(&full);
        // The clone runs in the terminal, so show it.
        if self.tab != RepoTab::Terminal {
            self.set_tab(i, RepoTab::Terminal);
        }
        self.start_terminal(i, parent, Some(command));
    }

    /// Types `text` into repository `i`'s terminal.
    pub(crate) fn term_type(&mut self, i: usize, text: &str) {
        if let Some(t) = self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            t.write(text.as_bytes());
        }
    }

    pub(crate) fn term_type_bytes(&mut self, i: usize, bytes: &[u8]) {
        if let Some(t) = self.data.repos.get(i).and_then(|r| self.terms.get_mut(&r.full_name)) {
            t.write(bytes);
        }
    }

    /// Pastes into the terminal of the repository on screen, when its Terminal tab is open.
    pub(super) fn paste(&mut self, text: &str) -> bool {
        let Page::Repo(full) = &self.page else { return false };
        if self.tab != RepoTab::Terminal {
            return false;
        }
        match self.terms.get_mut(full) {
            Some(t) => {
                t.paste(text);
                true
            }
            None => false,
        }
    }

    /// Cached local clone path for `full`, probing disk/remotes at most once until invalidated.
    pub(crate) fn known_clone(&self, full: &str) -> Option<std::path::PathBuf> {
        if let Some(cached) = self.clone_dirs.borrow().get(full) {
            return cached.clone();
        }
        let found = clones_shim::find(full);
        self.clone_dirs.borrow_mut().insert(full.to_owned(), found.clone());
        found
    }

    /// Cached lookalike checkout path for `full`, probing disk/remotes at most once until invalidated.
    pub(crate) fn lookalike_clone(&self, full: &str) -> Option<std::path::PathBuf> {
        if let Some(cached) = self.lookalike_dirs.borrow().get(full) {
            return cached.clone();
        }
        let found = clones_shim::lookalike(full);
        self.lookalike_dirs.borrow_mut().insert(full.to_owned(), found.clone());
        found
    }

    /// Forgets that repository `i` had no local clone, so the next look checks the disk again:
    /// a clone made outside RepoHub (in a terminal) shows when its page or a section is opened.
    pub(super) fn recheck_missing_clone(&mut self, i: usize) {
        let Some(full) = self.data.repos.get(i).map(|r| r.full_name.clone()) else { return };
        if self.clone_dirs.borrow().get(&full) == Some(&None) {
            self.clone_dirs.borrow_mut().remove(&full);
            self.lookalike_dirs.borrow_mut().remove(&full);
        }
    }

    pub(crate) fn refresh_clone_cache(&mut self) {
        self.clone_dirs.borrow_mut().clear();
        self.lookalike_dirs.borrow_mut().clear();
        self.default_clone_parent = clones_shim::default_parent();
    }
}
