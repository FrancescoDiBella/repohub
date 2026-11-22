//! Notes storage: drawings (`.excalidraw` files) kept on this computer per repository and branch.
//!
//! - Desktop: a local folder, `REPOHUB_DATA_DIR` or the platform's data folder
//!   (`~/.local/share/repohub`, `~/Library/Application Support/RepoHub`, `%APPDATA%\RepoHub`).
//! - Web: `repohub-server` stores them (see `server/src/notes.rs`) in its `DATA_DIR`.
//!
//! Both use the same layout, `notes/<owner>/<repo>/<branch>/<name>.excalidraw`, so a folder can
//! move between the two. A deleted drawing goes to `trash/<date>/…` next to `notes/` and stays
//! there 30 days. Boards reach GitHub only through sync (`sync.rs`, `sync_io.rs`), which keeps
//! its index in `notes-sync/`.
//!
//! Saving merges: if the file changed underneath the editor (sync brought another device's
//! edits), the editor's copy and the file's are merged ([`crate::sync::merge`]) and both kept.

#[derive(Clone, Debug)]
pub struct Note {
    pub name: String,
    pub updated_at: i64,
    pub size: u64,
}

impl Note {
    pub fn updated_ago(&self, now: i64) -> String {
        crate::model::fmt_ago(self.updated_at, now)
    }
}

/// One drawing anywhere in the notes storage, with the git blob id of its file (for sync).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub repo: String,
    pub branch: String,
    pub name: String,
    pub sha: String,
}

/// The notes list for one repository and branch.
#[derive(Clone, Debug, Default)]
pub struct NotesView {
    /// What the list last showed; kept on screen while it reloads.
    pub items: Vec<Note>,
    pub state: crate::state::Refresh,
}

/// Percent-encodes a query value.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One file-name segment, the same encoding the server uses: `[A-Za-z0-9_.-]` kept (no leading
/// dot), everything else percent-encoded, `/` included.
pub fn segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') || (b == b'.' && !out.is_empty()) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The same rule the server applies to drawing names.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 80 && !n.starts_with('.') && n.chars().all(|c| c.is_alphanumeric() || " _-.()".contains(c))
}

/// "12 KB"
pub fn size(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64) } else { format!("{} KB", bytes.div_ceil(1024).max(1)) }
}

// ------------------------------------------------------------------ desktop: local folder

#[cfg(not(target_arch = "wasm32"))]
mod local {
    use std::path::PathBuf;

    use super::{Note, Stored, segment};

    /// Where notes are kept on this computer.
    pub fn data_dir() -> PathBuf {
        if let Ok(d) = std::env::var("REPOHUB_DATA_DIR") {
            return d.into();
        }
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
        if cfg!(target_os = "macos") {
            PathBuf::from(home).join("Library/Application Support/RepoHub")
        } else if cfg!(windows) {
            std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(home)).join("RepoHub")
        } else {
            std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(home).join(".local/share")).join("repohub")
        }
    }

    pub fn folder(repo: &str, branch: &str) -> PathBuf {
        let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
        data_dir().join("notes").join(segment(&owner.to_ascii_lowercase())).join(segment(&name.to_ascii_lowercase())).join(segment(branch))
    }

    pub fn file(repo: &str, branch: &str, name: &str) -> PathBuf {
        folder(repo, branch).join(format!("{}.excalidraw", segment(name)))
    }

    pub fn list(repo: &str, branch: &str) -> Result<Vec<Note>, String> {
        let mut items: Vec<(Note, i64)> = std::fs::read_dir(folder(repo, branch)).into_iter().flatten().flatten().filter_map(|e| {
            let f = e.file_name().to_string_lossy().into_owned();
            let stem = f.strip_suffix(".excalidraw")?;
            let meta = e.metadata().ok()?;
            let t = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            Some((Note { name: super::decode(stem), updated_at: t, size: meta.len() }, t))
        }).collect();
        items.sort_by_key(|x| std::cmp::Reverse(x.1));
        Ok(items.into_iter().map(|x| x.0).collect())
    }

    /// One read-compare-write at a time in this app, so a save and a sync never interleave.
    pub static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn entries(dir: &std::path::Path) -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir).into_iter().flatten().flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        v.sort();
        v
    }

    /// Every drawing of every repository and branch, with its blob id.
    pub fn list_all() -> Result<Vec<Stored>, String> {
        let mut out = Vec::new();
        for (owner, o) in entries(&data_dir().join("notes")).into_iter().filter(|e| e.1.is_dir()) {
            for (repo, r) in entries(&o).into_iter().filter(|e| e.1.is_dir()) {
                for (branch, b) in entries(&r).into_iter().filter(|e| e.1.is_dir()) {
                    for (file, f) in entries(&b) {
                        let Some(stem) = file.strip_suffix(".excalidraw") else { continue };
                        let Ok(bytes) = std::fs::read(&f) else { continue };
                        out.push(Stored {
                            repo: format!("{}/{}", super::decode(&owner), super::decode(&repo)),
                            branch: super::decode(&branch),
                            name: super::decode(stem),
                            sha: crate::sync::blob_sha(&bytes),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// The blob id of the drawing's file now; `None` when there is none.
    pub fn current(path: &std::path::Path) -> Result<Option<(String, String)>, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Some((crate::sync::blob_sha(s.as_bytes()), s))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Couldn't read the drawing: {e}")),
        }
    }

    static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    /// Writes beside and moves in place, so a crash never leaves half a file.
    pub fn write(repo: &str, branch: &str, name: &str, json: &str) -> Result<(), String> {
        let path = file(repo, branch, name);
        let id = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), id));
        std::fs::create_dir_all(folder(repo, branch))
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Couldn't save the drawing: {e}")
            })
    }

    /// Moves the drawing to `trash/<today>/<owner>/<repo>/<branch>/`; a name already there gets a
    /// number. Trash folders older than 30 days are removed.
    pub fn trash(repo: &str, branch: &str, name: &str) -> Result<(), String> {
        let src = file(repo, branch, name);
        let notes = data_dir().join("notes");
        let rel = src.strip_prefix(&notes).map_err(|_| "Couldn't delete the drawing: it isn't in the notes folder".to_owned())?.to_path_buf();
        let today = crate::sync::date(crate::net::now());
        let mut dest = data_dir().join("trash").join(&today).join(&rel);
        let mut n = 2;
        while dest.exists() {
            dest = data_dir().join("trash").join(&today).join(rel.with_file_name(format!("{} {n}.excalidraw", segment(name))));
            n += 1;
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't delete the drawing: {e}"))?;
        }
        std::fs::rename(&src, &dest).map_err(|e| format!("Couldn't delete the drawing: {e}"))?;
        purge_trash();
        Ok(())
    }

    /// Removes trash folders older than 30 days.
    pub fn purge_trash() {
        let cutoff = crate::sync::date(crate::net::now() - 30 * 86_400);
        for (day, path) in entries(&data_dir().join("trash")) {
            let dated = day.len() == 10 && day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
            if dated && day < cutoff {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }

    /// Where sync keeps its index for notes repository `repo`.
    pub fn index_file(repo: &str) -> PathBuf {
        data_dir().join("notes-sync").join(format!("{}.json", segment(&repo.to_ascii_lowercase())))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use local::data_dir;

/// Where the desktop app keeps notes, for the Settings page.
pub fn location() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        local::data_dir().join("notes").display().to_string()
    }
    #[cfg(target_arch = "wasm32")]
    {
        "the repohub-server data folder (Docker volume)".into()
    }
}

#[cfg(target_arch = "wasm32")]
fn query(repo: &str, branch: &str) -> String {
    format!("repo={}&branch={}", enc(repo), enc(branch))
}

/// Lists the drawings of `repo` at `branch`, newest first.
pub async fn list(repo: &str, branch: &str) -> Result<Vec<Note>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        local::list(repo, branch)
    }
    #[cfg(target_arch = "wasm32")]
    {
        use serde_json::Value;
        let r = crate::net::request("GET", &format!("api/notes?{}", query(repo, branch)), "application/json", "").await?;
        if r.status != 200 {
            return Err(format!("The RepoHub server answered {}: {}", r.status, r.body));
        }
        let now = crate::net::now();
        let v: Value = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
        Ok(v.as_array().cloned().unwrap_or_default().iter().map(|n| {
            let t = n.get("updated").and_then(Value::as_i64).unwrap_or(now);
            Note {
                name: n.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                updated_at: t,
                size: n.get("size").and_then(Value::as_u64).unwrap_or(0),
            }
        }).collect())
    }
}

/// Reads a drawing; `None` when it doesn't exist yet.
pub async fn load(repo: &str, branch: &str, name: &str) -> Result<Option<String>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::fs::read_to_string(local::file(repo, branch, name)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("Couldn't read the drawing: {e}")),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let r = crate::net::request("GET", &format!("api/notes/file?{}&name={}", query(repo, branch), enc(name)), "application/json", "").await?;
        match r.status {
            200 => Ok(Some(r.body)),
            404 => Ok(None),
            _ => Err(format!("Couldn't load the drawing: {}", r.body)),
        }
    }
}

/// Saves a drawing (an Excalidraw JSON document) from the editor. If the file changed
/// underneath (sync wrote another device's edits), the two are merged and the merge is written;
/// it comes back as `Some`, for the editor to take in.
pub async fn save(repo: &str, branch: &str, name: &str, json: String) -> Result<Option<String>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = local::lock();
        let path = local::file(repo, branch, name);
        let merged = match local::current(&path)? {
            Some((_, now)) if now != json => match crate::sync::merge(&json, &now, crate::sync::Tie::First) {
                Ok(m) => m,
                // The file isn't a drawing any more (edited by hand?): it goes to the trash
                // rather than being overwritten.
                Err(_) => {
                    local::trash(repo, branch, name)?;
                    None
                }
            },
            _ => None,
        };
        local::write(repo, branch, name, merged.as_deref().unwrap_or(&json))?;
        Ok(merged)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::save(repo, branch, name, json).await
    }
}

/// Writes a drawing only if its file still has blob id `expect` (`None`: only if there is no
/// file). False when it changed meanwhile: then nothing was written. For sync.
pub async fn write_if(repo: &str, branch: &str, name: &str, expect: Option<&str>, json: &str) -> Result<bool, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = local::lock();
        let now = local::current(&local::file(repo, branch, name))?.map(|c| c.0);
        if now.as_deref() != expect {
            return Ok(false);
        }
        local::write(repo, branch, name, json)?;
        Ok(true)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::write_if(repo, branch, name, expect, json).await
    }
}

/// Deletes one drawing: it moves to the trash, where it stays 30 days.
pub async fn delete(repo: &str, branch: &str, name: &str) -> Result<(), String> {
    trash_if(repo, branch, name, None).await.and_then(|done| if done { Ok(()) } else { Err("Couldn't delete the drawing: it changed meanwhile".into()) })
}

/// Moves a drawing to the trash, only if its file still has blob id `expect` (`None`: whatever
/// it holds). False when it changed meanwhile, or is gone.
pub async fn trash_if(repo: &str, branch: &str, name: &str, expect: Option<&str>) -> Result<bool, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = local::lock();
        let Some((now, _)) = local::current(&local::file(repo, branch, name))? else {
            return if expect.is_none() { Err("Couldn't delete the drawing: it isn't there any more".into()) } else { Ok(false) };
        };
        if expect.is_some_and(|e| e != now) {
            return Ok(false);
        }
        local::trash(repo, branch, name)?;
        Ok(true)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::trash_if(repo, branch, name, expect).await
    }
}

/// Every drawing on this device (desktop) or server (web), with blob ids. For sync.
pub async fn list_all() -> Result<Vec<Stored>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        local::list_all()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::list_all().await
    }
}

/// Sync's index for notes repository `repo`; empty when there is none yet.
pub async fn load_index(repo: &str) -> Result<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::fs::read_to_string(local::index_file(repo)) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("Couldn't read what was last synced: {e}")),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let r = crate::net::request("GET", &format!("api/notes/index?repo={}", enc(repo)), "application/json", "").await?;
        match r.status {
            200 => Ok(r.body),
            404 => Ok(String::new()),
            _ => Err(format!("Couldn't read what was last synced: {}", r.body)),
        }
    }
}

/// Saves sync's index for `repo`; an empty `json` removes it (Stop syncing).
pub async fn save_index(repo: &str, json: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = local::index_file(repo);
        if json.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("Couldn't forget what was synced: {e}")),
                _ => Ok(()),
            };
        }
        let tmp = path.with_extension("json.tmp");
        path.parent().map(std::fs::create_dir_all).transpose()
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("Couldn't save what was synced: {e}"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let method = if json.is_empty() { "DELETE" } else { "PUT" };
        let r = crate::net::request(method, &format!("api/notes/index?repo={}", enc(repo)), "*/*", json).await?;
        if (200..300).contains(&r.status) || (json.is_empty() && r.status == 404) { Ok(()) } else { Err(format!("Couldn't save what was synced: {}", r.body)) }
    }
}

// ------------------------------------------------------------------ web: the server's notes API

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::{Stored, enc, query};
    use crate::net::request;

    thread_local! {
        /// The blob id of what the editor last wrote to each drawing, when nothing else wrote it
        /// since: a save that matches it needs no read first.
        static KNOWN: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    fn url(repo: &str, branch: &str, name: &str) -> String {
        format!("api/notes/file?{}&name={}", query(repo, branch), enc(name))
    }

    fn forget(u: &str) {
        KNOWN.with(|k| k.borrow_mut().remove(u));
    }

    pub async fn save(repo: &str, branch: &str, name: &str, json: String) -> Result<Option<String>, String> {
        let u = url(repo, branch, name);
        let mut base: Option<Option<String>> = KNOWN.with(|k| k.borrow().get(&u).cloned()).map(Some);
        for _ in 0..4 {
            let (expect, write, merged) = match base.take() {
                Some(known) => (known, json.clone(), None),
                None => {
                    let r = request("GET", &u, "application/json", "").await?;
                    match r.status {
                        404 => (None, json.clone(), None),
                        200 if r.body == json => (Some(crate::sync::blob_sha(r.body.as_bytes())), json.clone(), None),
                        200 => {
                            let sha = crate::sync::blob_sha(r.body.as_bytes());
                            match crate::sync::merge(&json, &r.body, crate::sync::Tie::First) {
                                Ok(Some(m)) => (Some(sha), m.clone(), Some(m)),
                                _ => (Some(sha), json.clone(), None),
                            }
                        }
                        _ => return Err(format!("Couldn't save the drawing: {}", r.body)),
                    }
                }
            };
            let cond = expect.as_deref().unwrap_or("none");
            let r = request("PUT", &format!("{u}&if={cond}"), "*/*", &write).await?;
            match r.status {
                200..=299 => {
                    KNOWN.with(|k| {
                        let mut k = k.borrow_mut();
                        // After a merge the editor's queued copies lack what came in: their
                        // saves read the file again.
                        if merged.is_some() { k.remove(&u) } else { k.insert(u.clone(), crate::sync::blob_sha(write.as_bytes())) }
                    });
                    return Ok(merged);
                }
                409 => continue,
                _ => return Err(format!("Couldn't save the drawing: {}", r.body)),
            }
        }
        Err("Couldn't save the drawing: it keeps changing underneath. Try again.".into())
    }

    pub async fn write_if(repo: &str, branch: &str, name: &str, expect: Option<&str>, json: &str) -> Result<bool, String> {
        let u = url(repo, branch, name);
        forget(&u);
        let r = request("PUT", &format!("{u}&if={}", expect.unwrap_or("none")), "*/*", json).await?;
        match r.status {
            200..=299 => Ok(true),
            409 => Ok(false),
            _ => Err(format!("Couldn't save the drawing: {}", r.body)),
        }
    }

    pub async fn trash_if(repo: &str, branch: &str, name: &str, expect: Option<&str>) -> Result<bool, String> {
        let u = url(repo, branch, name);
        forget(&u);
        let full = match expect {
            Some(e) => format!("{u}&if={e}"),
            None => u,
        };
        let r = request("DELETE", &full, "*/*", "").await?;
        match r.status {
            200..=299 => Ok(true),
            409 => Ok(false),
            404 if expect.is_some() => Ok(false),
            _ => Err(format!("Couldn't delete the drawing: {}", r.body)),
        }
    }

    pub async fn list_all() -> Result<Vec<Stored>, String> {
        use serde_json::Value;
        let r = request("GET", "api/notes/all", "application/json", "").await?;
        if r.status != 200 {
            return Err(format!("The RepoHub server answered {}: {}", r.status, r.body));
        }
        let v: Value = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
        let s = |n: &Value, k: &str| n.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Ok(v.as_array().into_iter().flatten().map(|n| Stored { repo: s(n, "repo"), branch: s(n, "branch"), name: s(n, "name"), sha: s(n, "sha") }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_and_names() {
        assert_eq!(enc("feature/x y"), "feature%2Fx%20y");
        assert_eq!(segment("feature/x"), "feature%2Fx");
        assert_eq!(segment(".."), "%2E.");
        assert_eq!(decode("Map%20%28v2%29"), "Map (v2)");
        assert!(valid_name("Architecture v2") && !valid_name("../x") && !valid_name("a/b"));
        assert_eq!(size(1500), "2 KB");
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        pollster::block_on(f)
    }

    #[test]
    fn local_round_trip() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-notes-{}", std::process::id()));
        // SAFETY: single-threaded test setup.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        assert_eq!(run(save("Octo/Repo", "feature/x", "Map (v2)", "{\"type\":\"excalidraw\"}".into())).unwrap(), None);
        assert!(dir.join("notes/octo/repo/feature%2Fx/Map%20%28v2%29.excalidraw").is_file());
        let list = run(list("octo/repo", "feature/x")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Map (v2)");
        assert_eq!(run(load("octo/repo", "feature/x", "Map (v2)")).unwrap().as_deref(), Some("{\"type\":\"excalidraw\"}"));
        run(delete("octo/repo", "feature/x", "Map (v2)")).unwrap();
        assert!(run(load("octo/repo", "feature/x", "Map (v2)")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_saves_use_unique_tmp_files_and_succeed() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-notes-concurrent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: ENV_LOCK serializes tests mutating environment variables.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };

        let mut handles = Vec::new();
        for i in 0..10 {
            let json = format!("{{\"type\":\"excalidraw\",\"ver\":{i}}}");
            handles.push(std::thread::spawn(move || {
                run(save("org/repo", "main", "Concurrent", json))
            }));
        }
        for h in handles {
            assert!(h.join().unwrap().is_ok());
        }
        let content = run(load("org/repo", "main", "Concurrent")).unwrap();
        assert!(content.is_some());
        assert!(content.unwrap().contains("\"type\":\"excalidraw\""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
