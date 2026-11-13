//! Notes storage: drawings (`.excalidraw` files) kept on this computer per repository and branch.
//!
//! - Desktop: a local folder, `REPOHUB_DATA_DIR` or the platform's data folder
//!   (`~/.local/share/repohub`, `~/Library/Application Support/RepoHub`, `%APPDATA%\RepoHub`).
//! - Web: `repohub-server` stores them (see `server/src/notes.rs`) in its `DATA_DIR`.
//!
//! Both use the same layout, `notes/<owner>/<repo>/<branch>/<name>.excalidraw`, so a folder can
//! move between the two. Nothing is sent to GitHub.

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

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
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

    use super::{Note, segment};

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
        items.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(items.into_iter().map(|x| x.0).collect())
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

/// Saves a drawing (an Excalidraw JSON document).
pub async fn save(repo: &str, branch: &str, name: &str, json: String) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = local::file(repo, branch, name);
        let tmp = path.with_extension("excalidraw.tmp");
        std::fs::create_dir_all(local::folder(repo, branch))
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("Couldn't save the drawing: {e}"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let r = crate::net::request("PUT", &format!("api/notes/file?{}&name={}", query(repo, branch), enc(name)), "*/*", &json).await?;
        if (200..300).contains(&r.status) { Ok(()) } else { Err(format!("Couldn't save the drawing: {}", r.body)) }
    }
}

/// Deletes one drawing.
pub async fn delete(repo: &str, branch: &str, name: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::remove_file(local::file(repo, branch, name)).map_err(|e| format!("Couldn't delete the drawing: {e}"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let r = crate::net::request("DELETE", &format!("api/notes/file?{}&name={}", query(repo, branch), enc(name)), "*/*", "").await?;
        if (200..300).contains(&r.status) { Ok(()) } else { Err(format!("Couldn't delete the drawing: {}", r.body)) }
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
        run(save("Octo/Repo", "feature/x", "Map (v2)", "{\"type\":\"excalidraw\"}".into())).unwrap();
        assert!(dir.join("notes/octo/repo/feature%2Fx/Map%20%28v2%29.excalidraw").is_file());
        let list = run(list("octo/repo", "feature/x")).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Map (v2)");
        assert_eq!(run(load("octo/repo", "feature/x", "Map (v2)")).unwrap().as_deref(), Some("{\"type\":\"excalidraw\"}"));
        run(delete("octo/repo", "feature/x", "Map (v2)")).unwrap();
        assert!(run(load("octo/repo", "feature/x", "Map (v2)")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
