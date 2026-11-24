//! Where each repository is cloned on this computer (desktop only).
//!
//! A folder you pick is remembered in `<data dir>/clones.json`. Otherwise RepoHub looks for a
//! git checkout named after the repository in `REPOHUB_REPOS_DIR` (comma-separated folders) or,
//! without it, in the usual places: `~/Developer`, `~/Projects`, `~/code`, `~/src`, `~/GitHub`,
//! `~/repos`, `~/Documents/GitHub` and the home folder itself, each also as `<folder>/<owner>/<repo>`.
//! A checkout found this way counts only if one of its remotes is the repository.
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

type Stamp = Option<(std::time::SystemTime, u64)>;

fn file_stamp(path: &Path) -> Stamp {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

struct CachedClones {
    path: PathBuf,
    stamp: Stamp,
    map: BTreeMap<String, String>,
}

static CACHE: std::sync::Mutex<Option<CachedClones>> = std::sync::Mutex::new(None);

fn load_map() -> BTreeMap<String, String> {
    let path = store();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let stamp = file_stamp(&path);
    if let Some(c) = g.as_ref()
        && c.path == path
        && c.stamp == stamp
    {
        return c.map.clone();
    }
    let map: BTreeMap<String, String> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    *g = Some(CachedClones { path, stamp, map: map.clone() });
    map
}

/// True when `p` is a git working tree.
pub fn is_checkout(p: &Path) -> bool {
    p.join(".git").exists()
}

/// The repository (`owner/name`, lower case) a remote's address points to:
/// `https://host/owner/name.git`, `git@host:owner/name.git`, `ssh://git@host/owner/name`.
fn remote_repo(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let url = url.split_once("://").map_or(url, |(_, rest)| rest);
    // What follows the host (and a port): the last two parts are the owner and the name.
    let (_, path) = url.split_once(['/', ':'])?;
    let mut parts = path.rsplit('/');
    let (name, owner) = (parts.next()?, parts.next()?);
    (!name.is_empty() && !owner.is_empty()).then(|| format!("{owner}/{name}").to_ascii_lowercase())
}

/// The addresses of the remotes of the checkout at `dir`, read from its git configuration.
fn remotes(dir: &Path) -> Vec<String> {
    let dot_git = dir.join(".git");
    // A worktree or a submodule has a `.git` file that points to the real folder.
    let git_dir = match std::fs::read_to_string(&dot_git) {
        Ok(s) => match s.trim().strip_prefix("gitdir:") {
            Some(p) => {
                let p = dir.join(p.trim());
                match std::fs::read_to_string(p.join("commondir")) {
                    Ok(common) => p.join(common.trim()),
                    Err(_) => p,
                }
            }
            None => return Vec::new(),
        },
        Err(_) => dot_git,
    };
    let Ok(config) = std::fs::read_to_string(git_dir.join("config")) else { return Vec::new() };
    let mut in_remote = false;
    let mut out = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_remote = line.starts_with("[remote ");
        } else if in_remote
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("url")
        {
            out.push(v.trim().to_owned());
        }
    }
    out
}

/// True when the checkout at `dir` has a remote that is `repo` (`owner/name`).
fn is_clone_of(dir: &Path, repo: &str) -> bool {
    let want = repo.to_ascii_lowercase();
    remotes(dir).iter().any(|u| remote_repo(u).as_deref() == Some(want.as_str()))
}

/// The folders named after `repo` in the search folders that are git checkouts.
fn candidates(repo: &str) -> Vec<PathBuf> {
    let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
    let mut out = Vec::new();
    for dir in search_dirs() {
        for candidate in [dir.join(name), dir.join(owner).join(name), dir.join(name.to_ascii_lowercase())] {
            if is_checkout(&candidate) && !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

/// The local clone of `repo` (`owner/name`), if one is known or found.
///
/// A folder you picked is trusted. A folder found by its name is used only when one of its
/// remotes is this repository: another project with the same name is not its clone, and
/// commits, merges and commands meant for one must not land in the other.
pub fn find(repo: &str) -> Option<PathBuf> {
    if let Some(p) = load_map().get(&repo.to_ascii_lowercase()).map(|p| expand(p))
        && p.is_dir()
    {
        return Some(p);
    }
    candidates(repo).into_iter().find(|c| is_clone_of(c, repo))
}

/// A checkout named like `repo` that isn't its clone (no remote points to it), when no clone
/// was found: the user may know better (a fork, a renamed repository) and choose it.
pub fn lookalike(repo: &str) -> Option<PathBuf> {
    if find(repo).is_some() {
        return None;
    }
    candidates(repo).into_iter().next()
}

/// Remembers `path` as the clone of `repo`.
pub fn remember(repo: &str, path: &Path) -> Result<(), String> {
    let file = store();
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let stamp = file_stamp(&file);
    let mut map: BTreeMap<String, String> = if let Some(c) = g.as_ref()
        && c.path == file
        && c.stamp == stamp
    {
        c.map.clone()
    } else {
        std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    };
    map.insert(repo.to_ascii_lowercase(), path.display().to_string());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    let tmp = file.with_extension(format!("json.tmp.{}", std::process::id()));
    let res = std::fs::write(&tmp, json)
        .and_then(|()| std::fs::rename(&tmp, &file))
        .map_err(|e| format!("Couldn't remember the folder: {e}"));
    if res.is_ok() {
        let new_stamp = file_stamp(&file);
        *g = Some(CachedClones { path: file, stamp: new_stamp, map });
    } else {
        let _ = std::fs::remove_file(&tmp);
        *g = None;
    }
    res
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
        std::fs::write(repo.join(".git/config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:octo/Demo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n").unwrap();
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

    #[test]
    fn a_folder_with_the_same_name_is_not_the_clone() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("repohub-lookalike-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // `~/work/api` is your own project; `acme/api` on GitHub is something else.
        let mine = base.join("work").join("api");
        std::fs::create_dir_all(mine.join(".git")).unwrap();
        std::fs::write(mine.join(".git/config"), "[remote \"origin\"]\n\turl = https://github.com/me/api.git\n").unwrap();
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe {
            std::env::set_var("REPOHUB_REPOS_DIR", base.join("work"));
            std::env::set_var("REPOHUB_DATA_DIR", base.join("data"));
        }
        assert_eq!(find("acme/api"), None, "another repository's folder isn't used");
        assert_eq!(lookalike("acme/api"), Some(mine.clone()), "but it's offered");
        assert_eq!(find("me/api"), Some(mine.clone()));
        assert_eq!(lookalike("me/api"), None);
        // A checkout without remotes is nobody's clone.
        std::fs::write(mine.join(".git/config"), "[core]\n\turl = https://github.com/acme/api\n").unwrap();
        assert_eq!(find("acme/api"), None, "a url outside a remote section doesn't count");
        // Any remote counts (a fork with an upstream), and choosing the folder is trusted.
        std::fs::write(mine.join(".git/config"), "[remote \"origin\"]\n\turl = git@github.com:me/api.git\n[remote \"upstream\"]\n  URL = ssh://git@github.com/Acme/API/\n").unwrap();
        assert_eq!(find("acme/api"), Some(mine.clone()));
        std::fs::write(mine.join(".git/config"), "[remote \"origin\"]\n\turl = https://github.com/me/api.git\n").unwrap();
        remember("acme/api", &mine).unwrap();
        assert_eq!(find("acme/api"), Some(mine.clone()));
        // A worktree's `.git` file leads to the main checkout's configuration.
        let tree = base.join("work").join("tool");
        std::fs::create_dir_all(&tree).unwrap();
        let real = base.join("real.git");
        std::fs::create_dir_all(real.join("worktrees/tool")).unwrap();
        std::fs::write(real.join("config"), "[remote \"origin\"]\n\turl = https://github.com/acme/tool\n").unwrap();
        std::fs::write(real.join("worktrees/tool/commondir"), "../..\n").unwrap();
        std::fs::write(tree.join(".git"), format!("gitdir: {}\n", real.join("worktrees/tool").display())).unwrap();
        assert_eq!(find("acme/tool"), Some(tree));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn remote_addresses() {
        assert_eq!(remote_repo("https://github.com/Octo/Demo.git").as_deref(), Some("octo/demo"));
        assert_eq!(remote_repo("git@github.com:octo/demo.git").as_deref(), Some("octo/demo"));
        assert_eq!(remote_repo("ssh://git@ghe.example:2222/octo/demo/").as_deref(), Some("octo/demo"));
        assert_eq!(remote_repo("https://github.com/demo"), None);
        assert_eq!(remote_repo("../demo"), None);
    }
}
