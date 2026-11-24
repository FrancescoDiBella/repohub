//! Board sync, wired up: GitHub's notes repository ([`GhRemote`]) and this device's notes
//! storage ([`NotesLocal`]) for the engine in `sync.rs`, choosing the repository in Settings,
//! and when sync runs (the Notes section opening, a few seconds after a save, **Sync now**, the
//! app's refresh).
//!
//! API: `GET /repos/{notes}` (exists, private, writable, default branch),
//! `GET /repos/{notes}/git/trees/{branch}?recursive=1` (every board and its blob id),
//! `GET|PUT|DELETE /repos/{notes}/contents/boards/…/<name>.excalidraw`.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use rune::TextEdit;
use serde_json::{Value, json};

use crate::config::Config;
use crate::github::Gh;
use crate::net::{self, Resp};
use crate::state::{BoardSync, NotesRepoCheck, Save};
use crate::sync::{self, Index, Key, Listing, Local, Remote, RemoteError, Report};
use crate::{App, notes, prefs};

const JSON: &str = "application/vnd.github+json";
const RAW: &str = "application/vnd.github.raw+json";

/// How long after a save sync waits, so a burst of strokes goes as one commit.
const AFTER_SAVE_MS: u32 = 4000;

/// The name suggested for the notes repository.
pub const SUGGESTED: &str = "repohub-notes";

/// GitHub's page for a new private repository with the suggested name.
pub fn create_url(cfg: &Config) -> String {
    format!("{}/new?name={SUGGESTED}&visibility=private", cfg.web_root())
}

fn part_ok(p: &str) -> bool {
    !p.is_empty() && p.len() <= 100 && !p.starts_with('.') && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The notes repository as typed: `owner/name`, or `name` for one of `login`'s.
pub fn parse_repo(typed: &str, login: &str) -> Option<String> {
    let t = typed.trim().trim_start_matches("https://github.com/").trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = match t.split_once('/') {
        Some((o, n)) => (o, n),
        None => (login, t),
    };
    (part_ok(owner) && part_ok(name)).then(|| format!("{owner}/{name}"))
}

/// A branch name usable in the trees address as one path segment.
fn simple_branch(b: &str) -> bool {
    part_ok(b)
}

// ------------------------------------------------------------------ GitHub

/// Why GitHub refused, said for the notes repository (the token's permission on it).
pub fn explain(gh: &Gh, r: &Resp, repo: &str) -> String {
    let rate = r.status == 429 || (r.status == 403 && (r.remaining.as_deref() == Some("0") || r.retry_after.is_some() || r.body.to_ascii_lowercase().contains("rate limit")));
    match r.status {
        403 if !rate => format!("GitHub refused (403). The token needs Contents: read and write on {repo}."),
        404 => format!("{repo} wasn't found, or the token can't see it. A fine-grained token needs Contents: read and write on it."),
        _ => gh.explain(r),
    }
}

/// What `GET /repos/{notes}` says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoInfo {
    pub full: String,
    pub private: bool,
    pub branch: String,
    pub push: bool,
}

pub fn parse_repo_info(v: &Value) -> RepoInfo {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    RepoInfo {
        full: s("full_name"),
        private: v.get("private").and_then(Value::as_bool).unwrap_or(false),
        branch: Some(s("default_branch")).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into()),
        push: v.get("permissions").and_then(|p| p.get("push")).and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The boards in a `git/trees/…?recursive=1` answer, and whether GitHub listed all of it.
pub fn parse_tree(body: &str) -> Result<Listing, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("Invalid response from GitHub: {e}"))?;
    let files = v.get("tree").and_then(Value::as_array).into_iter().flatten().filter_map(|e| {
        let path = e.get("path")?.as_str()?;
        let sha = e.get("sha")?.as_str()?;
        (e.get("type")?.as_str()? == "blob" && path.starts_with("boards/") && path.ends_with(".excalidraw")).then(|| (path.to_owned(), sha.to_owned()))
    }).collect();
    Ok(Listing { files, complete: v.get("truncated").and_then(Value::as_bool) != Some(true) })
}

/// A file's content from a contents answer: the raw file as asked for, or, if something on the
/// way answered with GitHub's JSON form instead (`"encoding": "base64"`), its decoded content.
pub fn raw_content(body: String) -> Result<String, String> {
    if body.trim_start().starts_with('{')
        && let Ok(v) = serde_json::from_str::<Value>(&body)
        && v.get("type").and_then(Value::as_str) == Some("file")
        && v.get("encoding").and_then(Value::as_str) == Some("base64")
    {
        let bytes = v.get("content").and_then(Value::as_str).and_then(sync::base64_decode).ok_or("GitHub sent a file that isn't readable")?;
        return String::from_utf8(bytes).map_err(|_| "GitHub sent a file that isn't text".to_owned());
    }
    Ok(body)
}

/// Reads the notes repository before using it: it must exist and take writes, and its default
/// branch must be a simple name.
pub async fn check(cfg: &Config, repo: &str) -> Result<RepoInfo, String> {
    let gh = Gh::new(cfg);
    let r = gh.fetch(&format!("/repos/{repo}"), JSON).await?;
    if r.status != 200 {
        return Err(explain(&gh, &r, repo));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|e| format!("Invalid response from GitHub: {e}"))?;
    let info = parse_repo_info(&v);
    if !info.push {
        return Err(format!("You can't write to {repo}. Use a repository of your own, or ask its owner for write access."));
    }
    if !simple_branch(&info.branch) {
        return Err(format!("{repo}'s default branch, {}, has characters sync can't use. Rename it to a simple name such as main.", info.branch));
    }
    Ok(info)
}

/// The notes repository on GitHub, through the same transport (and proxy) as the rest.
pub struct GhRemote {
    gh: Gh,
    repo: String,
    /// Read from GitHub by `list`.
    branch: RefCell<String>,
    pub private: Cell<Option<bool>>,
}

impl GhRemote {
    pub fn new(cfg: &Config, repo: &str) -> GhRemote {
        GhRemote { gh: Gh::new(cfg), repo: repo.into(), branch: RefCell::new(String::new()), private: Cell::new(None) }
    }

    fn contents(&self, path: &str) -> String {
        format!("/repos/{}/contents/{}", self.repo, sync::url_path(path))
    }
}

impl Remote for GhRemote {
    async fn list(&self) -> Result<Listing, String> {
        let r = self.gh.fetch(&format!("/repos/{}", self.repo), JSON).await?;
        if r.status != 200 {
            return Err(explain(&self.gh, &r, &self.repo));
        }
        let info = parse_repo_info(&serde_json::from_str(&r.body).unwrap_or(Value::Null));
        self.private.set(Some(info.private));
        if !simple_branch(&info.branch) {
            return Err(format!("{}'s default branch, {}, has characters sync can't use. Rename it to a simple name such as main.", self.repo, info.branch));
        }
        *self.branch.borrow_mut() = info.branch.clone();
        let r = self.gh.fetch(&format!("/repos/{}/git/trees/{}?recursive=1", self.repo, info.branch), JSON).await?;
        match r.status {
            200 => parse_tree(&r.body),
            // A repository with no commit yet: nothing there.
            409 => Ok(Listing { files: BTreeMap::new(), complete: true }),
            _ => Err(explain(&self.gh, &r, &self.repo)),
        }
    }

    async fn read(&self, path: &str) -> Result<Option<String>, String> {
        let r = self.gh.fetch(&self.contents(path), RAW).await?;
        match r.status {
            200 => Ok(Some(raw_content(r.body)?)),
            404 => Ok(None),
            _ => Err(explain(&self.gh, &r, &self.repo)),
        }
    }

    async fn write(&self, path: &str, content: &str, sha: Option<&str>, message: &str) -> Result<String, RemoteError> {
        let mut body = json!({ "message": message, "content": sync::base64_encode(content.as_bytes()) });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        let r = self.gh.send("PUT", &self.contents(path), JSON, &body.to_string()).await.map_err(RemoteError::Other)?;
        match r.status {
            200 | 201 => {
                let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
                Ok(v.pointer("/content/sha").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| sync::blob_sha(content.as_bytes())))
            }
            409 | 422 => Err(RemoteError::Conflict),
            _ => Err(RemoteError::Other(explain(&self.gh, &r, &self.repo))),
        }
    }

    async fn delete(&self, path: &str, sha: &str, message: &str) -> Result<(), RemoteError> {
        let body = json!({ "message": message, "sha": sha });
        let r = self.gh.send("DELETE", &self.contents(path), JSON, &body.to_string()).await.map_err(RemoteError::Other)?;
        match r.status {
            200 | 204 | 404 => Ok(()),
            409 | 422 => Err(RemoteError::Conflict),
            _ => Err(RemoteError::Other(explain(&self.gh, &r, &self.repo))),
        }
    }
}

// ------------------------------------------------------------------ this device

/// The notes storage (`notes.rs`): the data folder on the desktop, the server's notes API on
/// the web, the Android app's storage on a phone.
pub struct NotesLocal {
    pub repo: String,
}

impl Local for NotesLocal {
    async fn list(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(notes::list_all().await?.into_iter().map(|s| (Key::new(&s.repo, &s.branch, &s.name).path(), s.sha)).collect())
    }

    async fn read(&self, key: &Key) -> Result<Option<String>, String> {
        notes::load(&key.repo, &key.branch, &key.name).await
    }

    async fn write_if(&self, key: &Key, expect: Option<&str>, content: &str) -> Result<bool, String> {
        notes::write_if(&key.repo, &key.branch, &key.name, expect, content).await
    }

    async fn trash_if(&self, key: &Key, expect: &str) -> Result<bool, String> {
        notes::trash_if(&key.repo, &key.branch, &key.name, Some(expect)).await
    }

    async fn load_index(&self) -> Result<Index, String> {
        Ok(Index::from_json(&notes::load_index(&self.repo).await?))
    }

    async fn save_index(&self, index: &Index) -> Result<(), String> {
        notes::save_index(&self.repo, &index.to_json()).await
    }
}

/// One sync with notes repository `repo`; also whether that repository is private.
pub async fn run(cfg: &Config, repo: &str) -> (Report, Option<bool>) {
    let remote = GhRemote::new(cfg, repo);
    let local = NotesLocal { repo: repo.into() };
    let report = sync::run(&remote, &local, repo, &sync::Options::default()).await;
    (report, remote.private.get())
}

// ------------------------------------------------------------------ the app's part

/// Board sync's part of the app.
#[derive(Default)]
pub struct SyncState {
    /// The chosen notes repository, for `account` (the login it was read for).
    pub repo: Option<String>,
    pub account: Option<String>,
    pub status: BoardSync,
    pub check: NotesRepoCheck,
    /// The repository field in Settings.
    pub field: TextEdit,
    /// Counts runs: a result from an older one is dropped.
    pub run: u64,
    /// Counts saves: only the newest save's timer starts a run.
    pub timer: u64,
    /// Another run was asked for while one ran.
    pub again: bool,
    /// A run (or forgetting an index) is under way: its answer hasn't come back, even if it's no
    /// longer the one on screen. Only one goes at a time, so two never write the same files.
    pub busy: bool,
    /// **Stop syncing** was pressed while a run went: its index is forgotten once it's done
    /// (it saves it at its end).
    pub forget_after: Option<String>,
    /// Boards with changes not sent yet.
    pub to_send: BTreeSet<Key>,
    /// Boards saved while the running sync ran (they wait for the next).
    pub saved_during: BTreeSet<Key>,
    /// Boards in step after the last run.
    pub synced: BTreeSet<Key>,
    /// The notes repository is public (seen on the last run).
    pub public: bool,
    /// GitHub listed only part of the notes repository last time.
    pub partial: bool,
    /// A headless still (`-- sync <state>`): shown as set, never run.
    pub still: bool,
}

/// The notes repository saved for `login`.
pub fn saved_choice(login: &str) -> Option<String> {
    prefs::get("notes_repo")?.get(login.to_ascii_lowercase())?.as_str().map(str::to_owned)
}

fn save_choice(login: &str, repo: Option<&str>) {
    let mut all = prefs::get("notes_repo").and_then(|v| v.as_object().cloned()).unwrap_or_default();
    match repo {
        Some(r) => all.insert(login.to_ascii_lowercase(), Value::from(r)),
        None => all.remove(&login.to_ascii_lowercase()),
    };
    prefs::set("notes_repo", Value::Object(all));
}

impl App {
    /// The account the notes repository is chosen for.
    fn sync_login(&self) -> String {
        Some(self.data.profile.login.clone()).filter(|l| !l.is_empty()).or_else(|| self.cfg.user.clone()).unwrap_or_default()
    }

    /// Why boards can't sync here, if so (shown in Settings and the Notes section).
    pub(crate) fn sync_blocked(&self) -> Option<&'static str> {
        if self.sync.still {
            None
        } else if !self.cfg.notes {
            Some("Boards need the desktop app, the phone app or RepoHub with Docker.")
        } else if self.data.sample || !self.cfg.live() {
            Some("Sync needs your GitHub account: connect it above, with a token.")
        } else if !self.cfg.allow_writes {
            Some("Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true), so boards can't sync.")
        } else if self.cfg.token.is_none() {
            Some("Sync needs a token with Contents: read and write on the notes repository.")
        } else {
            None
        }
    }

    /// The account's data arrived (start, refresh, sign-in): picks up its notes repository and
    /// syncs, as the app's refresh does.
    pub(crate) fn sync_after_load(&mut self) {
        let login = self.sync_login();
        if self.sync.account.as_deref() != Some(login.as_str()) {
            self.sync.account = Some(login.clone());
            self.sync.repo = if login.is_empty() { None } else { saved_choice(&login) };
            self.sync.status = if self.sync.repo.is_some() { BoardSync::Ready } else { BoardSync::Off };
            self.sync.to_send.clear();
            self.sync.synced.clear();
            self.sync.run += 1;
        }
        self.sync_now();
    }

    /// Runs a sync now, or right after the one running.
    pub(crate) fn sync_now(&mut self) {
        if self.sync_blocked().is_some() || self.sync.still {
            return;
        }
        let Some(repo) = self.sync.repo.clone() else { return };
        // One at a time, even when the one going belongs to an account or a choice left since.
        if self.sync.status == BoardSync::Running || self.sync.busy {
            self.sync.again = true;
            return;
        }
        let Some(h) = self.handle.clone() else { return };
        self.sync.run += 1;
        let (run, epoch) = (self.sync.run, self.load_gen);
        self.sync.status = BoardSync::Running;
        self.sync.busy = true;
        self.sync.saved_during.clear();
        let cfg = self.cfg.clone();
        let asked = repo.clone();
        net::spawn(h, move || async move { run_owned(cfg, repo).await }, move |s: &mut App, r| s.apply_sync(epoch, run, &asked, r));
    }

    /// A drawing was written or deleted here: it waits to be sent, and sync runs a few seconds
    /// later, once drawing has paused.
    pub(crate) fn sync_saved(&mut self, id: &(String, String, String)) {
        if self.sync.repo.is_none() || self.sync_blocked().is_some() {
            return;
        }
        let key = Key::new(&id.0, &id.1, &id.2);
        self.sync.synced.remove(&key);
        self.sync.to_send.insert(key.clone());
        if self.sync.status == BoardSync::Running {
            self.sync.saved_during.insert(key);
        }
        self.sync_later();
    }

    fn sync_later(&mut self) {
        let Some(h) = self.handle.clone() else { return };
        self.sync.timer += 1;
        let timer = self.sync.timer;
        net::spawn(h, move || net::sleep(AFTER_SAVE_MS), move |s: &mut App, _| {
            if s.sync.timer != timer {
                return;
            }
            // Never in the middle of a stroke, nor with a write still going.
            if s.board.as_ref().is_some_and(|b| b.busy()) || !s.saving_notes.is_empty() {
                s.sync_later();
                return;
            }
            s.sync_now();
        });
    }

    pub(crate) fn apply_sync(&mut self, epoch: u64, run: u64, repo: &str, (report, private): (Report, Option<bool>)) {
        self.sync.busy = false;
        // Stop syncing was pressed while this run went: what it just remembered is forgotten,
        // before any next run starts.
        if self.sync.forget_after.as_deref() == Some(repo) {
            self.sync.forget_after = None;
            self.forget_index(repo.to_owned());
            return;
        }
        if run != self.sync.run || epoch != self.load_gen || self.sync.repo.is_none() {
            if run == self.sync.run {
                self.sync.status = if self.sync.repo.is_some() { BoardSync::Ready } else { BoardSync::Off };
            }
            // A run asked for while this one went (another account's, another repository).
            if std::mem::take(&mut self.sync.again) {
                self.sync_now();
            }
            return;
        }
        if let Some(p) = private {
            self.sync.public = !p;
        }
        self.sync.partial = report.partial;
        // What sync wrote here: the open drawing takes it in, the list shows it.
        let mut touched: BTreeSet<String> = BTreeSet::new();
        for (key, content) in &report.written {
            touched.insert(key.repo.clone());
            self.sync_board_changed(key, content);
        }
        for key in &report.trashed {
            touched.insert(key.repo.clone());
            if self.board.as_ref().is_some_and(|b| Key::new(&b.repo, &b.branch, &b.name) == *key) {
                self.notify(format!("{} was deleted on another device. Drawing on it brings it back; it's also in the trash for 30 days.", key.name));
            }
        }
        let saved_during = std::mem::take(&mut self.sync.saved_during);
        self.sync.to_send = report.to_send.union(&saved_during).cloned().collect();
        self.sync.synced = report.synced.difference(&saved_during).cloned().collect();
        self.sync.status = match &report.error {
            Some(e) => BoardSync::Failed(e.clone()),
            None => BoardSync::Done { at: net::now() },
        };
        if let Some(name) = report.conflicts.first() {
            let more = if report.conflicts.len() > 1 { format!(" (and {} more)", report.conflicts.len() - 1) } else { String::new() };
            self.notify_problem(format!("Two copies of a board couldn't be merged, so both are kept: the other device's is {name}{more}."));
        } else if let Some(k) = report.too_large.first() {
            self.notify_problem(format!("{} is too large to sync (10 MB at most). It stays on this device.", k.name));
        }
        let shown = match &self.page {
            crate::Page::Repo(full) if self.tab == crate::state::RepoTab::Notes && touched.contains(&full.to_ascii_lowercase()) => self.repo_index_of(full),
            _ => None,
        };
        if let Some(i) = shown {
            self.load_notes(i);
        }
        if std::mem::take(&mut self.sync.again) {
            self.sync_now();
        }
    }

    /// Sync wrote a drawing here. If it's open and nothing is drawn since the last save, the
    /// editor takes it in now; otherwise the next save merges it.
    fn sync_board_changed(&mut self, key: &Key, content: &str) {
        let id = match self.board.as_ref() {
            Some(b) if Key::new(&b.repo, &b.branch, &b.name) == *key => (b.repo.clone(), b.branch.clone(), b.name.clone()),
            _ => return,
        };
        let writing = self.saving_notes.contains_key(&id) || self.pending_note_saves.contains_key(&id);
        if let Some(b) = self.board.as_mut()
            && b.save == Save::Saved
            && !b.busy()
            && !writing
            && let Err(e) = b.reload_json(content)
        {
            self.notify_problem(format!("Couldn't show the changes from another device: {e}"));
        }
    }

    /// A save found the file changed underneath and wrote the merge: the open editor takes it in
    /// (unless a stroke is under way; the next save merges again then).
    pub(crate) fn board_merged(&mut self, id: &(String, String, String), merged: &str) {
        if let Some(b) = self.board.as_mut()
            && (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) == (id.0.as_str(), id.1.as_str(), id.2.as_str())
            && b.save != Save::Loading
            && !b.busy()
        {
            let _ = b.merge_json(merged);
        }
    }

    /// **Use it** in Settings: checks the repository, asks first if it's public, then saves the
    /// choice and syncs.
    pub(crate) fn use_notes_repo(&mut self, confirmed: Option<String>) {
        if let Some(why) = self.sync_blocked() {
            self.sync.check = NotesRepoCheck::Failed(why.into());
            return;
        }
        if let Some(repo) = confirmed {
            self.choose_notes_repo(repo);
            return;
        }
        let login = self.sync_login();
        let Some(repo) = parse_repo(&self.sync.field.text, &login) else {
            self.sync.check = NotesRepoCheck::Failed("Type the repository as owner/name, for example octocat/repohub-notes.".into());
            return;
        };
        let Some(h) = self.handle.clone() else { return };
        self.sync.check = NotesRepoCheck::Checking;
        let cfg = self.cfg.clone();
        let epoch = self.load_gen;
        net::spawn(h, move || async move { check_owned(cfg, repo).await }, move |s: &mut App, r: Result<RepoInfo, String>| {
            if s.load_gen != epoch || s.sync.check != NotesRepoCheck::Checking {
                return;
            }
            match r {
                Ok(info) if !info.private => s.sync.check = NotesRepoCheck::ConfirmPublic(info.full),
                Ok(info) => s.choose_notes_repo(info.full),
                Err(e) => s.sync.check = NotesRepoCheck::Failed(e),
            }
        });
    }

    fn choose_notes_repo(&mut self, repo: String) {
        let login = self.sync_login();
        save_choice(&login, Some(&repo));
        self.sync.account = Some(login);
        self.sync.repo = Some(repo.clone());
        self.sync.check = NotesRepoCheck::Idle;
        self.sync.field = TextEdit::new("");
        self.sync.status = BoardSync::Ready;
        self.sync.to_send.clear();
        self.sync.synced.clear();
        self.sync.public = false;
        self.notify(format!("Boards sync through {repo} now"));
        self.sync_now();
    }

    /// **Stop syncing**: forgets the choice (and what was last synced); the boards here and in the
    /// repository stay as they are.
    pub(crate) fn stop_syncing(&mut self) {
        let Some(repo) = self.sync.repo.take() else { return };
        save_choice(&self.sync_login(), None);
        self.sync.run += 1;
        self.sync.status = BoardSync::Off;
        self.sync.check = NotesRepoCheck::Idle;
        self.sync.to_send.clear();
        self.sync.synced.clear();
        self.sync.again = false;
        // A run going now saves its index when it ends: forget it after that one.
        if self.sync.busy {
            self.sync.forget_after = Some(repo.clone());
        } else {
            self.forget_index(repo.clone());
        }
        self.notify(format!("Boards don't sync any more. They stay here and in {repo}."));
    }

    /// Forgets what was last synced with `repo` (its index); no run starts meanwhile.
    fn forget_index(&mut self, repo: String) {
        let Some(h) = self.handle.clone() else {
            // Tests and headless stills, where nothing runs in the background.
            #[cfg(not(target_arch = "wasm32"))]
            let _ = pollster::block_on(notes::save_index(&repo, ""));
            return;
        };
        self.sync.busy = true;
        net::spawn(h, move || async move { notes::save_index(&repo, "").await }, |s: &mut App, _| {
            s.sync.busy = false;
            if std::mem::take(&mut s.sync.again) {
                s.sync_now();
            }
        });
    }
}

impl App {
    /// For headless stills (`-- sync <state>`, `-- notes-repo owner/name`): board sync shown in
    /// `state` (synced, syncing, waiting, failed, public, setup, confirm) from the sample data,
    /// without running.
    pub(crate) fn sync_still(&mut self, state: &str, repo: Option<String>) {
        let repo = repo.unwrap_or_else(|| format!("{}/{SUGGESTED}", Some(self.data.profile.login.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| "octocat".into())));
        self.sync.still = true;
        self.sync.repo = Some(repo.clone());
        // The app's clock, which "2 minutes ago" is measured against.
        let now = if self.now > 0 { self.now } else { net::now() };
        let listed: Vec<Key> = self.notes.iter().flat_map(|((full, branch), v)| v.items.iter().map(move |n| Key::new(full, branch, &n.name))).collect();
        self.sync.synced = listed.iter().cloned().collect();
        let waiting: Vec<Key> = listed.iter().filter(|k| !crate::sync_page::is_conflict_copy(&k.name)).take(2).cloned().collect();
        self.sync.status = BoardSync::Done { at: now - 120 };
        match state {
            "syncing" => self.sync.status = BoardSync::Running,
            "waiting" => self.sync.to_send = waiting.into_iter().collect(),
            "failed" => {
                self.sync.to_send = waiting.into_iter().collect();
                self.sync.status = BoardSync::Failed("Can't reach https://api.github.com: you seem to be offline.".into());
            }
            "public" => self.sync.public = true,
            "setup" => {
                self.sync.repo = None;
                self.sync.field = TextEdit::new(repo);
            }
            "confirm" => {
                self.sync.repo = None;
                self.sync.field = TextEdit::new(repo.clone());
                self.sync.check = NotesRepoCheck::ConfirmPublic(repo);
            }
            _ => {}
        }
        for k in &self.sync.to_send {
            self.sync.synced.remove(k);
        }
    }
}

// `net::spawn` takes a closure that builds the future; these own their inputs.
async fn run_owned(cfg: std::sync::Arc<Config>, repo: String) -> (Report, Option<bool>) {
    run(&cfg, &repo).await
}

async fn check_owned(cfg: std::sync::Arc<Config>, repo: String) -> Result<RepoInfo, String> {
    check(&cfg, &repo).await
}

#[cfg(test)]
mod tests;
