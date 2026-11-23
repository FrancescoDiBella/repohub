//! Carrying out the plan: the `Remote` and `Local` traits, the report, and `run`, which reads both sides,
//! decides each board and writes, merges or keeps both copies.

use super::*;

// ------------------------------------------------------------------ carrying out the plan

/// A remote write GitHub refused because the file changed meanwhile (409, or 422 when a `sha`
/// was missing or stale), or any other failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    Conflict,
    Other(String),
}

/// What the notes repository holds: path → blob id, and whether GitHub listed all of it.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub files: BTreeMap<String, String>,
    pub complete: bool,
}

/// The notes repository (GitHub, or a fake in tests).
#[allow(async_fn_in_trait)]
pub trait Remote {
    async fn list(&self) -> Result<Listing, String>;
    /// A file's content; `None` when it isn't there.
    async fn read(&self, path: &str) -> Result<Option<String>, String>;
    /// Creates the file (`sha` None) or replaces the blob `sha`; returns the new blob id.
    async fn write(&self, path: &str, content: &str, sha: Option<&str>, message: &str) -> Result<String, RemoteError>;
    async fn delete(&self, path: &str, sha: &str, message: &str) -> Result<(), RemoteError>;
}

/// The boards on this device (the notes storage, or a fake in tests).
#[allow(async_fn_in_trait)]
pub trait Local {
    /// Every board: notes-repository path → blob id of the local file.
    async fn list(&self) -> Result<BTreeMap<String, String>, String>;
    async fn read(&self, key: &Key) -> Result<Option<String>, String>;
    /// Writes the board only if its file still has blob id `expect` (`None`: doesn't exist).
    /// False when it changed meanwhile, and then nothing was written.
    async fn write_if(&self, key: &Key, expect: Option<&str>, content: &str) -> Result<bool, String>;
    /// Moves the board to the trash only if its file still has blob id `expect`.
    async fn trash_if(&self, key: &Key, expect: &str) -> Result<bool, String>;
    async fn load_index(&self) -> Result<Index, String>;
    async fn save_index(&self, index: &Index) -> Result<(), String>;
}

/// How a run went.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_remote: usize,
    pub deleted_local: usize,
    pub merged: usize,
    /// Names of the conflict copies made (another device's copy that couldn't be merged).
    pub conflicts: Vec<String>,
    /// Boards whose local file this run wrote, with their new content (the open editor reloads
    /// from it).
    pub written: Vec<(Key, String)>,
    /// Boards this run moved to the trash.
    pub trashed: Vec<Key>,
    /// Boards with local changes still to send.
    pub to_send: BTreeSet<Key>,
    /// Boards known to be in step after the run.
    pub synced: BTreeSet<Key>,
    /// Boards too large to sync.
    pub too_large: Vec<Key>,
    /// GitHub listed only part of the repository.
    pub partial: bool,
    /// Why the run stopped early (offline, refused, rate-limited); the rest waits.
    pub error: Option<String>,
}

/// Limits and the date for conflict names.
#[derive(Clone, Debug)]
pub struct Options {
    /// Largest board synced, in bytes.
    pub max_bytes: usize,
    /// Today, `2026-11-22`.
    pub today: String,
}

impl Default for Options {
    fn default() -> Options {
        Options { max_bytes: 10 << 20, today: date(now_ms() / 1000) }
    }
}

enum Done {
    /// In step now.
    Synced,
    /// Left for the next run.
    Waiting,
    /// Gone on both sides now.
    Gone,
}

struct Runner<'a, R: Remote, L: Local> {
    remote: &'a R,
    local: &'a L,
    index: Index,
    opts: &'a Options,
    report: Report,
    local_names: BTreeSet<Key>,
}

impl<R: Remote, L: Local> Runner<'_, R, L> {
    async fn step(&mut self, s: &Step, retry: bool) -> Result<Done, String> {
        let sha = |c: &str| blob_sha(c.as_bytes());
        match s.action {
            Action::Nothing => Ok(if s.local.is_some() { Done::Synced } else { Done::Gone }),
            Action::Record => {
                if let (Some(l), Some(r)) = (&s.local, &s.remote) {
                    self.index.record(&s.path, l, r);
                }
                Ok(Done::Synced)
            }
            Action::Forget => {
                self.index.boards.remove(&s.path);
                Ok(Done::Gone)
            }
            Action::Upload => {
                let Some(content) = self.local.read(&s.key).await? else { return Ok(Done::Waiting) };
                let l = sha(&content);
                if Some(&l) != s.local.as_ref() {
                    return Ok(Done::Waiting);
                }
                if content.len() > self.opts.max_bytes {
                    self.report.too_large.push(s.key.clone());
                    return Ok(Done::Waiting);
                }
                match self.remote.write(&s.path, &content, s.remote.as_deref(), &message("Update", &s.key)).await {
                    Ok(r) => {
                        self.index.record(&s.path, &l, &r);
                        self.report.uploaded += 1;
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Action::Download => {
                let Some(content) = self.remote.read(&s.path).await? else { return Ok(Done::Waiting) };
                if content.len() > self.opts.max_bytes {
                    self.report.too_large.push(s.key.clone());
                    return Ok(Done::Waiting);
                }
                let c = sha(&content);
                if !self.local.write_if(&s.key, s.local.as_deref(), &content).await? {
                    return Ok(Done::Waiting);
                }
                self.index.record(&s.path, &c, &c);
                self.report.downloaded += 1;
                self.report.written.push((s.key.clone(), content));
                Ok(Done::Synced)
            }
            Action::DeleteRemote => {
                let Some(r) = &s.remote else { return Ok(Done::Gone) };
                match self.remote.delete(&s.path, r, &message("Delete", &s.key)).await {
                    Ok(()) => {
                        self.index.boards.remove(&s.path);
                        self.report.deleted_remote += 1;
                        Ok(Done::Gone)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Action::DeleteLocal => {
                let Some(l) = &s.local else { return Ok(Done::Gone) };
                if !self.local.trash_if(&s.key, l).await? {
                    return Ok(Done::Waiting);
                }
                self.index.boards.remove(&s.path);
                self.report.deleted_local += 1;
                self.report.trashed.push(s.key.clone());
                Ok(Done::Gone)
            }
            Action::Merge => self.merge(s, retry).await,
        }
    }

    /// GitHub refused a write because the file changed meanwhile: read it again and decide once
    /// more, without a further retry.
    async fn again(&mut self, s: &Step) -> Result<Done, String> {
        let now = self.remote.read(&s.path).await?;
        let r = now.as_deref().map(|c| blob_sha(c.as_bytes()));
        let entry = self.index.boards.get(&s.path).cloned().unwrap_or_default();
        let ls = Side::of(entry.local.as_deref(), s.local.as_deref());
        let rs = Side::of(entry.remote.as_deref(), r.as_deref());
        let action = decide(ls, rs, s.local.is_some() && s.local == r);
        let next = Step { path: s.path.clone(), key: s.key.clone(), action, local: s.local.clone(), remote: r };
        Box::pin(self.step(&next, false)).await
    }

    async fn merge(&mut self, s: &Step, retry: bool) -> Result<Done, String> {
        let sha = |c: &str| blob_sha(c.as_bytes());
        let Some(mine) = self.local.read(&s.key).await? else { return Ok(Done::Waiting) };
        let l = sha(&mine);
        if Some(&l) != s.local.as_ref() {
            return Ok(Done::Waiting);
        }
        let Some(theirs) = self.remote.read(&s.path).await? else { return Ok(Done::Waiting) };
        let r = sha(&theirs);
        if l == r {
            self.index.record(&s.path, &l, &r);
            return Ok(Done::Synced);
        }
        if mine.len().max(theirs.len()) > self.opts.max_bytes {
            self.report.too_large.push(s.key.clone());
            return Ok(Done::Waiting);
        }
        match merge(&mine, &theirs, Tie::KeepBoth) {
            Ok(None) => {
                // Everything there is here already. If nothing here is missing there either,
                // the two only differ in layout: remember them as in step.
                if matches!(merge(&theirs, &mine, Tie::KeepBoth), Ok(None)) {
                    self.index.record(&s.path, &l, &r);
                    return Ok(Done::Synced);
                }
                match self.remote.write(&s.path, &mine, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &l, &new_r);
                        self.report.merged += 1;
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Ok(Some(merged)) => {
                if !self.local.write_if(&s.key, Some(&l), &merged).await? {
                    return Ok(Done::Waiting);
                }
                let m = sha(&merged);
                self.report.written.push((s.key.clone(), merged.clone()));
                match self.remote.write(&s.path, &merged, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &m, &new_r);
                        self.report.merged += 1;
                        Ok(Done::Synced)
                    }
                    // The local file holds the merge now; the next run sends it (or merges again
                    // with what arrived there meanwhile).
                    Err(RemoteError::Conflict) if retry => {
                        let next = Step { local: Some(m), ..s.clone() };
                        self.again(&next).await
                    }
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Err(_) => {
                // Not mergeable: the other device's copy is kept next to this one, under its own
                // name, and this one goes there.
                let today = self.opts.today.clone();
                let taken = self.local_names.clone();
                let name = conflict_name(&s.key.name, &today, |n| taken.contains(&Key::new(&s.key.repo, &s.key.branch, n)));
                let copy = Key::new(&s.key.repo, &s.key.branch, &name);
                if !self.local.write_if(&copy, None, &theirs).await? {
                    return Ok(Done::Waiting);
                }
                self.local_names.insert(copy.clone());
                self.report.written.push((copy.clone(), theirs.clone()));
                self.report.conflicts.push(name);
                let c = sha(&theirs);
                if let Ok(cr) = self.remote.write(&copy.path(), &theirs, None, &message("Update", &copy)).await {
                    self.index.record(&copy.path(), &c, &cr);
                    self.report.synced.insert(copy.clone());
                } else {
                    self.report.to_send.insert(copy);
                }
                match self.remote.write(&s.path, &mine, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &l, &new_r);
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
        }
    }
}

/// Whether a step that didn't happen leaves something here to send.
fn sends(a: Action) -> bool {
    matches!(a, Action::Upload | Action::DeleteRemote | Action::Merge)
}

/// One sync: read both sides and the index, carry out the plan, save the index.
pub async fn run<R: Remote, L: Local>(remote: &R, local: &L, repo: &str, opts: &Options) -> Report {
    let mut report = Report::default();
    let index = match local.load_index().await {
        Ok(i) if i.repo.eq_ignore_ascii_case(repo) => i,
        Ok(_) => Index { repo: repo.to_owned(), boards: BTreeMap::new() },
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    let here = match local.list().await {
        Ok(l) => l,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    let there = match remote.list().await {
        Ok(l) => l,
        Err(e) => {
            // Offline or refused: say what waits to be sent, from this side alone.
            for (path, l) in &here {
                if let Some(key) = Key::parse(path)
                    && index.boards.get(path).and_then(|e| e.local.as_ref()) != Some(l)
                {
                    report.to_send.insert(key);
                }
            }
            for (path, e) in &index.boards {
                if e.local.is_some() && !here.contains_key(path)
                    && let Some(key) = Key::parse(path)
                {
                    report.to_send.insert(key);
                }
            }
            report.error = Some(e);
            return report;
        }
    };
    report.partial = !there.complete;
    let steps = plan(&index, &here, &there.files, there.complete);
    let local_names = here.keys().filter_map(|p| Key::parse(p)).collect();
    let mut runner = Runner { remote, local, index, opts, report, local_names };
    let mut stopped = false;
    for s in &steps {
        if stopped {
            if sends(s.action) {
                runner.report.to_send.insert(s.key.clone());
            }
            continue;
        }
        match runner.step(s, true).await {
            Ok(Done::Synced) => {
                runner.report.synced.insert(s.key.clone());
            }
            Ok(Done::Gone) => {}
            Ok(Done::Waiting) => {
                if sends(s.action) {
                    runner.report.to_send.insert(s.key.clone());
                }
            }
            Err(e) => {
                runner.report.error = Some(e);
                stopped = true;
                if sends(s.action) {
                    runner.report.to_send.insert(s.key.clone());
                }
            }
        }
    }
    if let Err(e) = runner.local.save_index(&runner.index).await {
        runner.report.error.get_or_insert(e);
    }
    let mut report = runner.report;
    report.to_send.retain(|k| !report.synced.contains(k));
    report
}
