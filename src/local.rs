//! The repository's local clone, read from disk (desktop app): its files, their git status and
//! their contents, with no request to GitHub.
//!
//! - [`git`] runs the `git` installed on the computer in the clone (no library is bundled).
//! - [`scan`] lists the files once: `git ls-files` gives tracked and untracked files without the
//!   ignored ones, `git status` marks what changed. A folder that isn't a git checkout is walked
//!   instead, skipping heavy folders (`node_modules`, `target`…), up to [`MAX_FILES`].
//! - [`Tree::entries`] shows one folder of that list, folders first.
//! - [`read`] reads one file: text up to [`MAX_TEXT`] bytes and [`MAX_LINES`] lines, a note for
//!   binary and larger files. Paths can't leave the clone.
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

/// Files listed at most (a walk of a folder that isn't a checkout stops there).
pub const MAX_FILES: usize = 50_000;
/// Bytes of a text file shown at most.
pub const MAX_TEXT: u64 = 1024 * 1024;
/// Lines of a text file shown at most.
pub const MAX_LINES: usize = 5_000;

/// Runs `git args…` in `dir`; its output, or what went wrong.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window flashing up
    }
    let out = cmd.output().map_err(|e| format!("Couldn't run git: {e}. Is git installed?"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { format!("git {} failed.", args.first().unwrap_or(&"")) } else { err })
    }
}

/// How a file differs from the last commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// Not tracked by git yet.
    New,
    Conflicted,
}

impl Change {
    pub fn letter(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::New => "U",
            Change::Conflicted => "!",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Change::Modified => "Modified",
            Change::Added => "Added",
            Change::Deleted => "Deleted",
            Change::Renamed => "Renamed",
            Change::New => "New, not tracked yet",
            Change::Conflicted => "In conflict",
        }
    }

    /// From `git status --porcelain`'s two letters.
    fn parse(xy: &str) -> Option<Change> {
        let b = xy.as_bytes();
        let (x, y) = (*b.first()? as char, *b.get(1)? as char);
        Some(match (x, y) {
            ('?', '?') => Change::New,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => Change::Conflicted,
            ('R', _) | (_, 'R') => Change::Renamed,
            ('A', _) => Change::Added,
            ('D', _) | (_, 'D') => Change::Deleted,
            ('M', _) | (_, 'M') | ('T', _) | (_, 'T') => Change::Modified,
            _ => return None,
        })
    }
}

/// The clone's files, listed once.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub root: PathBuf,
    /// The checked-out branch; empty when it isn't a git checkout (or HEAD is detached).
    pub branch: String,
    /// Paths relative to the root, with `/`.
    pub files: Vec<String>,
    pub changes: HashMap<String, Change>,
    /// More files than [`MAX_FILES`] were found.
    pub cut: bool,
}

/// One line of a folder's listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The path from the root.
    pub path: String,
    pub dir: bool,
    /// A file's change, or for a folder the first change inside it.
    pub change: Option<Change>,
}

/// Lists the clone at `root`.
pub fn scan(root: &Path) -> Result<Tree, String> {
    if !root.is_dir() {
        return Err(format!("{} isn't a folder.", root.display()));
    }
    let mut t = Tree { root: root.to_owned(), ..Tree::default() };
    if root.join(".git").exists() {
        let listed = git(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?;
        t.files = listed.split('\0').filter(|p| !p.is_empty()).map(str::to_owned).collect();
        t.files.sort();
        t.files.dedup();
        if let Ok(status) = git(root, &["status", "--porcelain", "-z"]) {
            t.changes = parse_status(&status);
        }
        t.branch = git(root, &["branch", "--show-current"]).unwrap_or_default().trim().to_owned();
    } else {
        walk(root, root, &mut t.files, &mut t.cut);
        t.files.sort();
    }
    if t.files.len() > MAX_FILES {
        t.files.truncate(MAX_FILES);
        t.cut = true;
    }
    Ok(t)
}

/// `git status --porcelain -z`: `XY path\0`, a rename followed by its old path.
fn parse_status(s: &str) -> HashMap<String, Change> {
    let mut out = HashMap::new();
    let mut parts = s.split('\0');
    while let Some(item) = parts.next() {
        if item.len() < 4 {
            continue;
        }
        let (xy, path) = (&item[..2], &item[3..]);
        if xy.contains('R') || xy.contains('C') {
            parts.next(); // the old path
        }
        if let Some(c) = Change::parse(xy) {
            out.insert(path.trim_end_matches('/').to_owned(), c);
        }
    }
    out
}

const SKIP: [&str; 8] = [".git", "node_modules", "target", ".venv", "venv", "__pycache__", "dist", "build"];

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, cut: &mut bool) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for e in read.flatten() {
        if out.len() > MAX_FILES {
            *cut = true;
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ty) = e.file_type() else { continue };
        if ty.is_dir() {
            if !SKIP.contains(&name.as_str()) {
                walk(root, &e.path(), out, cut);
            }
        } else if ty.is_file()
            && let Ok(rel) = e.path().strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

impl Tree {
    /// The folders and files directly in `dir` (`""` for the root), folders first.
    pub fn entries(&self, dir: &str) -> Vec<Entry> {
        let prefix = if dir.is_empty() { String::new() } else { format!("{}/", dir.trim_end_matches('/')) };
        // The listing is sorted, so a folder's files are one run of it.
        let start = self.files.partition_point(|f| f.as_str() < prefix.as_str());
        let mut dirs: Vec<Entry> = Vec::new();
        let mut files: Vec<Entry> = Vec::new();
        for f in self.files[start..].iter().take_while(|f| f.starts_with(&prefix)) {
            let rest = &f[prefix.len()..];
            match rest.split_once('/') {
                Some((name, _)) => {
                    let change = self.changes.get(f.as_str()).copied();
                    match dirs.last_mut() {
                        Some(d) if d.name == name => d.change = d.change.or(change),
                        _ => dirs.push(Entry { name: name.to_owned(), path: format!("{prefix}{name}"), dir: true, change }),
                    }
                }
                None => files.push(Entry { name: rest.to_owned(), path: f.clone(), dir: false, change: self.changes.get(f.as_str()).copied() }),
            }
        }
        dirs.sort_by(|a, b| rune_kit::natural_cmp(&a.name, &b.name));
        dirs.dedup_by(|a, b| a.name == b.name);
        files.sort_by(|a, b| rune_kit::natural_cmp(&a.name, &b.name));
        dirs.extend(files);
        dirs
    }

    /// How many files changed since the last commit.
    pub fn changed(&self) -> usize {
        self.changes.len()
    }
}

/// A file's contents, as the viewer shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// Text; `cut` when it was longer than [`MAX_TEXT`] bytes or [`MAX_LINES`] lines.
    Text { text: String, lines: usize, cut: bool },
    /// Not text (it has NUL bytes); its size in bytes.
    Binary(u64),
    /// Gone from the disk (deleted, not committed yet).
    Missing,
}

/// Whether `rel` stays inside the clone (no `..`, not absolute).
pub fn safe(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Reads `rel` in the clone at `root`.
pub fn read(root: &Path, rel: &str) -> Result<Content, String> {
    use std::io::Read;
    if !safe(rel) {
        return Err("That path is outside the repository.".into());
    }
    let path = root.join(rel);
    let Ok(meta) = std::fs::metadata(&path) else { return Ok(Content::Missing) };
    let file = std::fs::File::open(&path).map_err(|e| format!("Couldn't open {rel}: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT).read_to_end(&mut bytes).map_err(|e| format!("Couldn't read {rel}: {e}"))?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(Content::Binary(meta.len()));
    }
    let mut cut = meta.len() > MAX_TEXT;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut && let Some(end) = text.rfind('\n') {
        text.truncate(end + 1);
    }
    let mut lines = text.lines().count();
    if lines > MAX_LINES {
        let end = text.match_indices('\n').nth(MAX_LINES - 1).map_or(text.len(), |(i, _)| i + 1);
        text.truncate(end);
        lines = MAX_LINES;
        cut = true;
    }
    Ok(Content::Text { text, lines, cut })
}

/// What changed in the clone since the last commit, and where its branch stands.
#[derive(Clone, Debug, Default)]
pub struct Changes {
    pub branch: String,
    /// The branch it pushes to (`origin/main`), if it has one.
    pub upstream: Option<String>,
    /// Commits not pushed yet, and commits on the upstream not pulled yet.
    pub ahead: u32,
    pub behind: u32,
    /// `git diff HEAD`: staged and unstaged changes to tracked files.
    pub patch: String,
    /// New files git doesn't track yet, with their text (`None`: binary or too large).
    pub untracked: Vec<(String, Option<String>)>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.patch.trim().is_empty() && self.untracked.is_empty()
    }
}

/// Reads what changed in the clone at `root`.
pub fn changes(root: &Path) -> Result<Changes, String> {
    if !root.join(".git").exists() {
        return Err(format!("{} isn't a git checkout.", root.display()));
    }
    let mut c = Changes { branch: git(root, &["branch", "--show-current"])?.trim().to_owned(), ..Changes::default() };
    // A repository with no commit yet has no HEAD to compare with: everything staged is new.
    c.patch = match git(root, &["diff", "HEAD", "--no-color", "--no-ext-diff", "-M"]) {
        Ok(p) => p,
        Err(_) => git(root, &["diff", "--cached", "--no-color", "--no-ext-diff"]).unwrap_or_default(),
    };
    for path in git(root, &["ls-files", "-z", "--others", "--exclude-standard"])?.split('\0').filter(|p| !p.is_empty()) {
        let text = match read(root, path) {
            Ok(Content::Text { text, cut: false, .. }) => Some(text),
            _ => None,
        };
        c.untracked.push((path.to_owned(), text));
    }
    c.upstream = git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]).ok().map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());
    if c.upstream.is_some()
        && let Ok(counts) = git(root, &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
    {
        let mut n = counts.split_whitespace().map(|x| x.parse::<u32>().unwrap_or(0));
        c.behind = n.next().unwrap_or(0);
        c.ahead = n.next().unwrap_or(0);
    }
    Ok(c)
}

/// Commits everything that changed (`git add -A`) with `message`.
pub fn commit_all(root: &Path, message: &str) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first.".into());
    }
    git(root, &["add", "-A"])?;
    git(root, &["commit", "-m", message])?;
    Ok(git(root, &["rev-parse", "--short", "HEAD"])?.trim().to_owned())
}

/// Pushes the branch, setting its upstream on `origin` the first time.
pub fn push(root: &Path) -> Result<(), String> {
    let branch = git(root, &["branch", "--show-current"])?.trim().to_owned();
    if branch.is_empty() {
        return Err("No branch is checked out (detached HEAD): check one out to push.".into());
    }
    git(root, &["push", "-u", "origin", &branch]).map(|_| ())
}

/// Pulls the upstream (fast-forward only, so it never makes a merge by surprise).
pub fn pull(root: &Path) -> Result<(), String> {
    git(root, &["pull", "--ff-only"]).map(|_| ())
}

/// The fence name syntax highlighting knows a file by, from its extension.
pub fn language(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "json" | "jsonld" | "webmanifest" => "json",
        "py" => "python",
        "go" => "go",
        "swift" => "swift",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "html" | "htm" | "xml" | "svg" | "plist" => "html",
        "css" => "css",
        "yml" | "yaml" => "yaml",
        "diff" | "patch" => "diff",
        "toml" => "toml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_entries() {
        let st = parse_status("M  src/a.rs\0?? notes.txt\0R  new.rs\0old.rs\0UU src/x/c.rs\0 D gone.md\0");
        assert_eq!(st.get("src/a.rs"), Some(&Change::Modified));
        assert_eq!(st.get("notes.txt"), Some(&Change::New));
        assert_eq!(st.get("new.rs"), Some(&Change::Renamed));
        assert_eq!(st.get("old.rs"), None);
        assert_eq!(st.get("src/x/c.rs"), Some(&Change::Conflicted));
        assert_eq!(st.get("gone.md"), Some(&Change::Deleted));
        let t = Tree { files: ["README.md", "src/a.rs", "src/b.rs", "src/x/c.rs", "src10/z", "src2/y"].map(String::from).to_vec(), changes: st, ..Tree::default() };
        let root: Vec<_> = t.entries("").into_iter().map(|e| (e.name, e.dir, e.change.map(Change::letter))).collect();
        assert_eq!(root, [("src".into(), true, Some("M")), ("src2".into(), true, None), ("src10".into(), true, None), ("README.md".into(), false, None)]);
        let src: Vec<_> = t.entries("src").into_iter().map(|e| (e.path, e.dir)).collect();
        assert_eq!(src, [("src/x".into(), true), ("src/a.rs".into(), false), ("src/b.rs".into(), false)]);
        assert_eq!(t.entries("src/x")[0].change, Some(Change::Conflicted));
    }

    #[test]
    fn changes_commit_and_push() {
        let dir = std::env::temp_dir().join(format!("repohub-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (origin, work) = (dir.join("origin.git"), dir.join("work"));
        std::fs::create_dir_all(&dir).unwrap();
        let g = |at: &Path, args: &[&str]| git(at, args).unwrap();
        g(&dir, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        g(&dir, &["clone", "-q", origin.to_str().unwrap(), work.to_str().unwrap()]);
        for (k, v) in [("user.name", "Test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")] {
            g(&work, &["config", k, v]);
        }
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        assert!(commit_all(&work, "  ").is_err());
        commit_all(&work, "first").unwrap();
        push(&work).unwrap();
        std::fs::write(work.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(work.join("b.txt"), "new\n").unwrap();
        let c = changes(&work).unwrap();
        assert_eq!((c.branch.as_str(), c.upstream.as_deref(), c.ahead, c.behind), ("main", Some("origin/main"), 0, 0));
        assert!(c.patch.contains("+two"), "{}", c.patch);
        assert_eq!(c.untracked, vec![("b.txt".to_owned(), Some("new\n".to_owned()))]);
        commit_all(&work, "second").unwrap();
        let c = changes(&work).unwrap();
        assert!(c.is_empty());
        assert_eq!(c.ahead, 1);
        push(&work).unwrap();
        assert_eq!(changes(&work).unwrap().ahead, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_safely() {
        let dir = std::env::temp_dir().join(format!("repohub-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/t.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("bin"), [0u8, 1, 2]).unwrap();
        std::fs::write(dir.join("long.txt"), "x\n".repeat(MAX_LINES + 10)).unwrap();
        assert_eq!(read(&dir, "a/t.rs").unwrap(), Content::Text { text: "fn main() {}\n".into(), lines: 1, cut: false });
        assert_eq!(read(&dir, "bin").unwrap(), Content::Binary(3));
        assert!(matches!(read(&dir, "long.txt").unwrap(), Content::Text { lines: MAX_LINES, cut: true, .. }));
        assert_eq!(read(&dir, "nope").unwrap(), Content::Missing);
        assert!(read(&dir, "../etc/passwd").is_err() && read(&dir, "/etc/passwd").is_err());
        // Not a checkout: walked, heavy folders skipped.
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::write(dir.join("node_modules/x/i.js"), "").unwrap();
        let t = scan(&dir).unwrap();
        assert_eq!(t.files, ["a/t.rs", "bin", "long.txt"]);
        assert_eq!(language("x.TSX"), Some("tsx"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
