//! Where each repository is cloned on this computer (desktop only).
//!
//! A folder you pick is remembered in `<data dir>/clones.json`. Otherwise RepoHub looks for a
//! git checkout named after the repository in `REPOHUB_REPOS_DIR` (comma-separated folders) or,
//! without it, in the usual places: `~/Developer`, `~/Projects`, `~/code`, `~/src`, `~/GitHub`,
//! `~/repos`, `~/Documents/GitHub` and the home folder itself, each also as `<folder>/<owner>/<repo>`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|_| ".".into())
}

/// Expands a leading `~`.
pub fn expand(p: &str) -> PathBuf {
    let p = p.trim();
    match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) => home().join(rest),
        None if p == "~" => home(),
        None => PathBuf::from(p),
    }
}

/// The folders searched for clones, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("REPOHUB_REPOS_DIR")
        && !v.trim().is_empty()
    {
        return v.split(',').filter(|s| !s.trim().is_empty()).map(expand).collect();
    }
    let h = home();
    ["Developer", "Projects", "code", "src", "GitHub", "repos", "Documents/GitHub", ""].iter().map(|d| h.join(d)).collect()
}

/// Where new clones go: the first search folder that exists, else `~/Developer`.
pub fn default_parent() -> PathBuf {
    search_dirs().into_iter().find(|d| d.is_dir() && d != &home()).unwrap_or_else(|| home().join("Developer"))
}

fn store() -> PathBuf {
    crate::notes::data_dir().join("clones.json")
}

fn load_map() -> BTreeMap<String, String> {
    std::fs::read_to_string(store()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// True when `p` is a git working tree.
pub fn is_checkout(p: &Path) -> bool {
    p.join(".git").exists()
}

/// The local clone of `repo` (`owner/name`), if one is known or found.
pub fn find(repo: &str) -> Option<PathBuf> {
    if let Some(p) = load_map().get(&repo.to_ascii_lowercase()).map(|p| expand(p))
        && p.is_dir()
    {
        return Some(p);
    }
    let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
    for dir in search_dirs() {
        for candidate in [dir.join(name), dir.join(owner).join(name), dir.join(name.to_ascii_lowercase())] {
            if is_checkout(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Remembers `path` as the clone of `repo`.
pub fn remember(repo: &str, path: &Path) -> Result<(), String> {
    let mut map = load_map();
    map.insert(repo.to_ascii_lowercase(), path.display().to_string());
    let file = store();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    std::fs::write(file, json).map_err(|e| format!("Couldn't remember the folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_remembers() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("repohub-clones-{}", std::process::id()));
        let repo = base.join("work").join("demo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        // SAFETY: test setup before any thread reads the environment.
        unsafe {
            std::env::set_var("REPOHUB_REPOS_DIR", base.join("work"));
            std::env::set_var("REPOHUB_DATA_DIR", base.join("data"));
        }
        assert_eq!(find("Octo/demo"), Some(repo.clone()));
        assert_eq!(find("octo/missing"), None);
        let other = base.join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        remember("octo/missing", &other).unwrap();
        assert_eq!(find("Octo/Missing"), Some(other));
        assert_eq!(expand("~/x"), home().join("x"));
        let _ = std::fs::remove_dir_all(base);
    }
}
