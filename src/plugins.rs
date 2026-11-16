//! Plugins (desktop app): sandboxed Luau scripts that add a section to repository pages, a page
//! of their own, or both.
//!
//! A plugin is a folder in `<data dir>/plugins/<id>/`:
//!
//! ```text
//! plugin.json   { "name": "Checklist", "version": "0.1.0", "description": "…", "author": "…",
//!                 "repo_tab": "Checklist", "page": "Checklist" }
//! init.luau     returns { repo = function(view) … end, page = function(view) … end }
//! ```
//!
//! - It runs in Rune's `rune-luau`, which builds UI with `@rune/ui`. The sandbox has no file,
//!   process or network access; a script that runs too long is stopped and its errors are shown
//!   in place, never crashing the app.
//! - `@repohub` gives it what's on screen (`context()`: the repository, its issues and pull
//!   requests, your repositories and profile, the theme) and a few actions: `open(url)`,
//!   `notify(text)`, `copy(text)`, `openRepo(fullName)`. `changed` fires when the context changes.
//! - `open` and `copy` reach outside the app (a link can carry what the plugin read; copied
//!   text gets pasted somewhere), so they only ask: the app shows the request ([`Ask`]) and the
//!   user allows it or not.
//! - Colours follow the app's theme: `accent`, `page`, `panel`, `text`, `muted`, `border`,
//!   `success`, `warning`, `danger`, and rune-luau's own names (`card`, `text-secondary`…).
//! - Installed from a folder or a Git address (copied or cloned into the plugins folder), turned
//!   on and off in Settings (`plugins.disabled`), and reloaded when one of their files is saved.
//!
//! See `docs/PLUGINS.md` for the guide and `plugins/checklist` for an example.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rune::{AppHandle, Color};
use rune_luau::{Data, Script, ScriptOptions, Sources};
use serde_json::Value;

use crate::App;

/// What `plugin.json` says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// The label of its section on repository pages (`repo(view)`), if it has one.
    pub repo_tab: Option<String>,
    /// The label of its page in the sidebar (`page(view)`), if it has one.
    pub page: Option<String>,
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Manifest, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("plugin.json isn't valid JSON: {e}"))?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_owned();
        let label = |k: &str| Some(s(k)).filter(|l| !l.is_empty()).map(|l| l.chars().take(24).collect::<String>());
        let m = Manifest {
            name: s("name"),
            version: s("version"),
            description: s("description"),
            author: s("author"),
            repo_tab: label("repo_tab"),
            page: label("page"),
        };
        if m.name.is_empty() {
            return Err("plugin.json needs a \"name\".".into());
        }
        if m.repo_tab.is_none() && m.page.is_none() {
            return Err("plugin.json needs a \"repo_tab\" or a \"page\" label, or both.".into());
        }
        Ok(m)
    }
}

/// Something a plugin wants that reaches outside the app. It never happens by itself: the app
/// shows it and the user allows it or not.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Open this `https://` address in the browser.
    Open(String),
    /// Put this text on the clipboard.
    Copy(String),
}

/// An [`Ask`] and the plugin (its name) it comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub plugin: String,
    pub ask: Ask,
}

impl Ask {
    /// A link a plugin may ask to open: `https://`, of a sane length, with no spaces or
    /// control characters.
    pub fn open(url: &str) -> Option<Ask> {
        let ok = url.starts_with("https://") && url.len() <= 2000 && host(url).is_some_and(|h| !h.is_empty())
            && !url.chars().any(|c| c.is_control() || c.is_whitespace());
        ok.then(|| Ask::Open(url.to_owned()))
    }

    /// Text a plugin may ask to copy: without control characters (they could run commands
    /// when pasted into a terminal), up to 100,000 characters.
    pub fn copy(text: &str) -> Option<Ask> {
        let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(100_000).collect();
        (!clean.trim().is_empty()).then_some(Ask::Copy(clean))
    }
}

/// The host of an `https://` address ("example.com"), which is what the user is shown first.
pub fn host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // What follows an `@` is the real host; what precedes it only looks like one.
    let host = authority.rsplit('@').next()?;
    Some(host.split(':').next().unwrap_or(host))
}

/// One installed plugin.
pub struct Plugin {
    /// Its folder's name.
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub enabled: bool,
    /// Running while enabled and its manifest is valid.
    pub script: Option<Script>,
    /// Why it isn't running (a bad manifest, a missing init.luau).
    pub problem: Option<String>,
}

impl Plugin {
    pub fn running(&self) -> bool {
        self.script.is_some()
    }

    /// The script's error or the plugin's problem, for Settings.
    pub fn trouble(&self) -> Option<String> {
        self.problem.clone().or_else(|| self.script.as_ref().and_then(|s| s.error()).map(|e| e.message.clone()))
    }
}

/// All plugins, and what they share with the app.
#[derive(Default)]
pub struct Plugins {
    pub list: Vec<Plugin>,
    /// What `repohub.context()` returns.
    context: Rc<RefCell<Data>>,
    /// The theme's colours, by name, for the scripts' palette.
    colors: Rc<RefCell<HashMap<&'static str, Color>>>,
    /// The context last given to the scripts (to fire `changed` only when it changes).
    last: Option<Data>,
    waker: Option<rune::Waker>,
}

/// The plugins folder.
pub fn folder() -> PathBuf {
    crate::notes::data_dir().join("plugins")
}

fn disabled() -> Vec<String> {
    crate::prefs::get("plugins.disabled").and_then(|v| v.as_array().cloned()).unwrap_or_default()
        .iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()
}

/// A folder name usable as an id: letters, digits, `-` and `_`.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Plugins {
    /// Reads the plugins folder (nothing starts until [`Plugins::start`]).
    pub fn scan() -> Plugins {
        let mut p = Plugins::default();
        p.rescan();
        p
    }

    /// Reads the folder again, keeping the plugins already running.
    pub fn rescan(&mut self) {
        let off = disabled();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(folder()).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        dirs.sort();
        let mut old: Vec<Plugin> = std::mem::take(&mut self.list);
        for dir in dirs {
            let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned).filter(|id| valid_id(id)) else { continue };
            if let Some(i) = old.iter().position(|p| p.id == id) {
                self.list.push(old.remove(i));
                continue;
            }
            let (manifest, problem) = match std::fs::read_to_string(dir.join("plugin.json")).map_err(|_| "plugin.json is missing.".to_owned()).and_then(|j| Manifest::parse(&j)) {
                Ok(m) => (m, None),
                Err(e) => (Manifest { name: id.clone(), ..Manifest::default() }, Some(e)),
            };
            let problem = problem.or_else(|| (!dir.join("init.luau").is_file()).then(|| "init.luau is missing.".to_owned()));
            self.list.push(Plugin { enabled: !off.contains(&id), id, dir, manifest, script: None, problem });
        }
    }

    /// Starts every enabled plugin that isn't running.
    pub fn start(&mut self, handle: &AppHandle<App>) {
        for i in 0..self.list.len() {
            let p = &self.list[i];
            if p.enabled && p.script.is_none() && p.problem.is_none() {
                let script = self.make(handle, i);
                self.list[i].script = script;
            }
        }
    }

    fn make(&mut self, handle: &AppHandle<App>, i: usize) -> Option<Script> {
        // A folder put there by hand gets the same check as an installed one.
        if let Err(e) = check_tree(&self.list[i].dir) {
            self.list[i].problem = Some(e);
            return None;
        }
        let p = &self.list[i];
        let sources = match Sources::from_dir(&p.dir) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("RepoHub: plugin {}: {e}", p.id);
                return None;
            }
        };
        let colors = self.colors.clone();
        let palette: rune_luau::Palette = Rc::new(move |name: &str, dark: bool| {
            colors.borrow().get(name).copied().or_else(|| rune_luau::default_palette()(name, dark))
        });
        let context = self.context.clone();
        let h = handle.clone();
        let id = p.id.clone();
        let name = p.manifest.name.clone();
        let log_id = p.id.clone();
        let options = ScriptOptions::new()
            .palette(palette)
            .on_log(move |level, line| eprintln!("RepoHub plugin {log_id} {level:?}: {line}"))
            .setup(move |host| {
                let lua = host.lua();
                let api = lua.create_table()?;
                let ctx = context.clone();
                api.set("context", lua.create_function(move |_, ()| Ok(ctx.borrow().clone()))?)?;
                api.set("changed", host.signal("changed")?)?;
                let hh = h.clone();
                let asks = name.clone();
                api.set("open", lua.create_function(move |_, url: String| {
                    if let Some(ask) = Ask::open(&url) {
                        let request = Request { plugin: asks.clone(), ask };
                        hh.update(move |s: &mut App, _| s.plugin_asks(request));
                    }
                    Ok(())
                })?)?;
                let hh = h.clone();
                let from = id.clone();
                api.set("notify", lua.create_function(move |_, text: String| {
                    let text: String = text.chars().take(200).collect();
                    let from = from.clone();
                    hh.update(move |s: &mut App, _| s.notify(format!("{from}: {text}")));
                    Ok(())
                })?)?;
                let hh = h.clone();
                let asks = name.clone();
                api.set("copy", lua.create_function(move |_, text: String| {
                    if let Some(ask) = Ask::copy(&text) {
                        let request = Request { plugin: asks.clone(), ask };
                        hh.update(move |s: &mut App, _| s.plugin_asks(request));
                    }
                    Ok(())
                })?)?;
                let hh = h.clone();
                api.set("openRepo", lua.create_function(move |_, full: String| {
                    hh.update(move |s: &mut App, _| {
                        if let Some(i) = s.data.repos.iter().position(|r| r.full_name.eq_ignore_ascii_case(&full)) {
                            s.open(i);
                        }
                    });
                    Ok(())
                })?)?;
                host.register_module("@repohub", api)
            });
        let script = Script::with_options(&p.id, sources, options);
        if let Some(w) = &self.waker {
            script.set_waker(w.clone());
        }
        if let Err(e) = script.watch(&p.dir) {
            eprintln!("RepoHub: not watching plugin {}: {e}", p.id);
        }
        // A script that starts after the context was set gets it at once.
        if self.last.is_some() {
            script.fire("changed", [self.context.borrow().clone()]);
        }
        Some(script)
    }

    pub fn set_waker(&mut self, waker: rune::Waker) {
        for s in self.list.iter().filter_map(|p| p.script.as_ref()) {
            s.set_waker(waker.clone());
        }
        self.waker = Some(waker);
    }

    /// Runs what's due in each script; true when script code ran.
    pub fn poll(&self, now: f64, tx: &mut rune::Tx) -> bool {
        self.list.iter().filter_map(|p| p.script.as_ref()).fold(false, |ran, s| s.poll(now, tx) | ran)
    }

    /// Gives the scripts a new context; they hear `changed` only when it differs.
    pub fn set_context(&mut self, data: Data) {
        if self.last.as_ref() == Some(&data) {
            return;
        }
        *self.context.borrow_mut() = data.clone();
        self.last = Some(data.clone());
        for s in self.list.iter().filter_map(|p| p.script.as_ref()) {
            s.fire("changed", [data.clone()]);
        }
    }

    /// The theme's colours for the scripts' palette.
    pub fn set_colors(&self, k: &rune_kit::Kit) {
        let c = &k.c;
        let pairs: [(&'static str, Color); 21] = [
            ("accent", c.bg_accent), ("page", c.page), ("panel", c.bg_base), ("sidebar", c.bg_deep),
            ("text", c.text_base), ("muted", c.text_muted), ("faint", c.text_faint), ("border", c.border_weak),
            ("success", c.success.icon), ("warning", c.warning.icon), ("danger", c.danger.icon), ("info", c.info.icon),
            ("bg", c.page), ("card", c.bg_base), ("card-border", c.border_weak), ("text-secondary", c.text_muted),
            ("separator", c.border_weak), ("hover", c.hover), ("press", c.pressed), ("control-off", c.bg_button_neutral), ("indicator", c.bg_accent),
        ];
        let mut map = self.colors.borrow_mut();
        if pairs.iter().any(|(n, col)| map.get(n) != Some(col)) {
            map.clear();
            map.extend(pairs);
        }
    }

    /// Turns plugin `id` on or off, and remembers it.
    pub fn set_enabled(&mut self, id: &str, on: bool, handle: Option<&AppHandle<App>>) {
        let Some(i) = self.list.iter().position(|p| p.id == id) else { return };
        self.list[i].enabled = on;
        if on {
            if let Some(h) = handle {
                self.start(h);
            }
        } else {
            self.list[i].script = None;
        }
        let off: Vec<Value> = self.list.iter().filter(|p| !p.enabled).map(|p| Value::from(p.id.clone())).collect();
        crate::prefs::set("plugins.disabled", Value::from(off));
    }

    /// Starts plugin `id` again from its files.
    pub fn reload(&mut self, id: &str, handle: &AppHandle<App>) {
        if let Some(i) = self.list.iter().position(|p| p.id == id) {
            let dir = self.list[i].dir.clone();
            self.list.remove(i);
            self.rescan();
            if let Some(p) = self.list.iter_mut().find(|p| p.dir == dir) {
                p.script = None;
            }
            self.start(handle);
        }
    }

    /// What plugin `id`'s function `name` (`repo` or `page`) draws.
    pub fn surface(&self, id: &str, cx: &rune::Cx<App>, name: &str) -> Option<rune::El> {
        Some(self.list.iter().find(|p| p.id == id)?.script.as_ref()?.surface(cx, name))
    }

    /// The plugins with a section on repository pages: (id, label).
    pub fn repo_tabs(&self) -> Vec<(String, String)> {
        self.list.iter().filter(|p| p.script.is_some()).filter_map(|p| Some((p.id.clone(), p.manifest.repo_tab.clone()?))).collect()
    }

    /// The plugins with a page in the sidebar: (id, label).
    pub fn pages(&self) -> Vec<(String, String)> {
        self.list.iter().filter(|p| p.script.is_some()).filter_map(|p| Some((p.id.clone(), p.manifest.page.clone()?))).collect()
    }
}

/// A plugin is a few small text files.
const MAX_FILES: usize = 500;
const MAX_BYTES: u64 = 10 << 20;
/// How long a clone may take.
const CLONE_SECONDS: u64 = 60;

/// Checks a plugin's folder: no symbolic links (the loader would follow them out of the
/// folder, to any file or the whole disk), and not more or bigger files than a plugin needs.
pub(crate) fn check_tree(dir: &Path) -> Result<(), String> {
    fn walk(dir: &Path, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("Can't read {}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let path = e.path();
            // Not `metadata`: that follows links.
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("This plugin has a link to somewhere else ({}): RepoHub doesn't run plugins with links.", e.file_name().to_string_lossy()));
            }
            if meta.is_dir() {
                walk(&path, files, bytes)?;
            } else {
                *files += 1;
                *bytes += meta.len();
            }
            if *files > MAX_FILES || *bytes > MAX_BYTES {
                return Err(format!("This is too big to be a plugin (more than {MAX_FILES} files or {} MB).", MAX_BYTES >> 20));
            }
        }
        Ok(())
    }
    walk(dir, &mut 0, &mut 0)
}

/// `git clone` for a plugin: the newest commit only, and it never stops to ask (a password, a
/// host key), because nobody could answer.
pub(crate) fn clone_command(source: &str, dest: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    crate::config::scrub_child_env(&mut cmd);
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes")
        .args(["-c", "credential.interactive=never", "-c", "protocol.ext.allow=never", "-c", "protocol.file.allow=never"])
        .args(["clone", "--depth", "1", "--single-branch", "--no-tags", "--no-recurse-submodules", "--", source])
        .arg(dest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window flashing up
    }
    cmd
}

/// Runs `cmd` for at most `seconds`; an error says what git said, or that it took too long.
fn run_limited(mut cmd: std::process::Command, seconds: u64) -> Result<(), String> {
    use std::io::Read;
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run git: {e}"))?;
    // Read what git says while it runs, so a chatty failure can't fill the pipe and hang.
    let said = child.stderr.take().map(|mut e| std::thread::spawn(move || {
        let mut s = String::new();
        let _ = e.read_to_string(&mut s);
        s
    }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let said = said.and_then(|t| t.join().ok()).unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(()),
        Some(_) => Err(format!("git clone failed: {}", said.trim())),
        None => Err(format!("Stopped after {seconds} seconds: the address didn't answer, or it's too big to be a plugin.")),
    }
}

/// Installs a plugin from a folder (copied) or a Git address (cloned); returns its id.
/// Cloning can take a while: call it off the UI thread.
pub fn install(source: &str) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("Type a folder on this computer, or a Git address (https://…).".into());
    }
    let into = folder();
    std::fs::create_dir_all(&into).map_err(|e| format!("Can't create {}: {e}", into.display()))?;
    let remote = source.starts_with("https://") || source.starts_with("git@") || source.starts_with("ssh://");
    let name = source.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', '\\', ':']).next().unwrap_or("plugin");
    let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let id = id.trim_matches('-').to_owned();
    if !valid_id(&id) {
        return Err(format!("Can't make a plugin name from {source}."));
    }
    let dest = into.join(&id);
    if dest.exists() {
        return Err(format!("A plugin called {id} is already installed: remove it first."));
    }
    if remote {
        if let Err(e) = run_limited(clone_command(source, &dest), CLONE_SECONDS) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(e);
        }
        // The plugin is its files, not their history.
        let _ = std::fs::remove_dir_all(dest.join(".git"));
    } else {
        let src = crate::clones::expand(source);
        if !src.join("plugin.json").is_file() {
            return Err(format!("{} has no plugin.json.", src.display()));
        }
        copy_dir(&src, &dest).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dest);
            format!("Couldn't copy {}: {e}", src.display())
        })?;
    }
    let checked = check_tree(&dest).and_then(|()| {
        std::fs::read_to_string(dest.join("plugin.json")).map_err(|_| "This isn't a plugin: it has no plugin.json.".to_owned()).and_then(|j| Manifest::parse(&j)).map(|_| ())
    });
    if let Err(e) = checked {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    Ok(id)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name == ".git" {
            continue;
        }
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_dir(&e.path(), &to.join(&name))?;
        } else if ty.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// The example plugin, written into the plugins folder to start from; returns its id.
pub fn create_example() -> Result<String, String> {
    let into = folder();
    let id = (1..).map(|n| if n == 1 { "checklist".to_owned() } else { format!("checklist-{n}") }).find(|id| !into.join(id).exists()).unwrap_or_default();
    let dest = into.join(&id);
    std::fs::create_dir_all(&dest).map_err(|e| format!("Can't create {}: {e}", dest.display()))?;
    for (name, body) in EXAMPLE {
        std::fs::write(dest.join(name), body).map_err(|e| format!("Can't write {name}: {e}"))?;
    }
    Ok(id)
}

/// The files of `plugins/checklist`, built in.
pub const EXAMPLE: [(&str, &str); 3] = [
    ("plugin.json", include_str!("../plugins/checklist/plugin.json")),
    ("init.luau", include_str!("../plugins/checklist/init.luau")),
    ("README.md", include_str!("../plugins/checklist/README.md")),
];

/// Removes plugin `id`'s folder.
pub fn remove(id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("Not a plugin.".into());
    }
    std::fs::remove_dir_all(folder().join(id)).map_err(|e| format!("Couldn't remove {id}: {e}"))
}

/// A fingerprint of what [`context`] reads: when it's unchanged, so is the context.
pub fn context_key(s: &App, dark: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", s.page).hash(&mut h);
    s.loaded_at.hash(&mut h);
    (s.data.repos.len(), s.data.issues.len(), s.data.pulls.len(), s.data.sample, dark).hash(&mut h);
    s.data.repos.iter().filter(|r| r.starred).count().hash(&mut h);
    s.data.profile.login.hash(&mut h);
    h.finish()
}

/// What `repohub.context()` holds, from the app's state.
pub fn context(s: &App, dark: bool) -> Data {
    use crate::Page;
    let text = |t: &str| Data::String(t.to_owned());
    let int = |n: u32| Data::Integer(n as i64);
    let map = |pairs: Vec<(&str, Data)>| Data::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<BTreeMap<_, _>>());
    let repo_data = |r: &crate::model::Repo| map(vec![
        ("name", text(&r.name)), ("fullName", text(&r.full_name)), ("owner", text(&r.owner)),
        ("description", text(&r.description)), ("language", text(&r.language)), ("stars", int(r.stars)),
        ("forks", int(r.forks)), ("openIssues", int(r.issues)), ("branch", text(&r.branch)), ("url", text(&r.url)),
        ("private", Data::Bool(r.private)), ("fork", Data::Bool(r.fork)), ("archived", Data::Bool(r.archived)),
        ("starred", Data::Bool(r.starred)), ("updated", text(&r.updated_ago(s.now))), ("pushed", Data::Integer(r.pushed)),
        ("topics", Data::List(r.topics.iter().map(|t| text(t)).collect())),
    ]);
    let (page, repo) = match &s.page {
        Page::Repo(full) => ("repo", s.data.repos.iter().find(|r| r.full_name.eq_ignore_ascii_case(full))),
        Page::Plugin(_) => ("plugin", None),
        Page::Overview => ("overview", None),
        Page::Repos => ("repos", None),
        Page::Pulls => ("pulls", None),
        Page::Issues => ("issues", None),
        Page::Actions => ("actions", None),
        Page::Notifications => ("notifications", None),
        Page::Settings => ("settings", None),
    };
    let in_repo = |name: &str| repo.is_some_and(|r| name.eq_ignore_ascii_case(&r.full_name) || name.eq_ignore_ascii_case(&r.name));
    let issues: Vec<Data> = s.data.issues.iter().filter(|x| in_repo(&x.repo)).map(|x| map(vec![
        ("number", int(x.number)), ("title", text(&x.title)), ("url", text(&x.url)), ("age", text(&x.age_ago(s.now))),
        ("comments", int(x.comments)), ("label", text(x.label.as_ref().map_or("", |l| l.0.as_str()))),
    ])).collect();
    let pulls: Vec<Data> = s.data.pulls.iter().filter(|x| in_repo(&x.repo)).map(|x| map(vec![
        ("number", int(x.number)), ("title", text(&x.title)), ("url", text(&x.url)), ("age", text(&x.age_ago(s.now))),
        ("author", text(&x.author)), ("state", text(&format!("{:?}", x.state).to_lowercase())),
    ])).collect();
    map(vec![
        ("page", text(page)),
        ("repo", repo.map_or(Data::Nil, repo_data)),
        ("issues", Data::List(issues)),
        ("pulls", Data::List(pulls)),
        ("repos", Data::List(s.data.repos.iter().map(repo_data).collect())),
        ("profile", map(vec![("login", text(&s.data.profile.login)), ("name", text(&s.data.profile.name))])),
        ("dark", Data::Bool(dark)),
        ("sample", Data::Bool(s.data.sample)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests() {
        let m = Manifest::parse(r#"{"name":"Checklist","version":"0.1.0","repo_tab":"Checklist","page":""}"#).unwrap();
        assert_eq!((m.repo_tab.as_deref(), m.page.as_deref()), (Some("Checklist"), None));
        assert!(Manifest::parse(r#"{"repo_tab":"X"}"#).unwrap_err().contains("name"));
        assert!(Manifest::parse(r#"{"name":"X"}"#).unwrap_err().contains("repo_tab"));
        assert!(Manifest::parse("nope").is_err());
        assert!(Manifest::parse(EXAMPLE[0].1).is_ok(), "the example's manifest");
        assert!(valid_id("my-plugin_2") && !valid_id("../x") && !valid_id(""));
    }

    #[test]
    fn what_a_plugin_may_ask_for() {
        assert_eq!(Ask::open("https://example.com/a?b=1"), Some(Ask::Open("https://example.com/a?b=1".into())));
        for bad in ["http://example.com", "file:///etc/passwd", "javascript:alert(1)", "https://", "https:///x", "https://example.com/a b", "https://example.com/\n", &format!("https://example.com/{}", "a".repeat(2000))] {
            assert_eq!(Ask::open(bad), None, "{bad}");
        }
        // The user is shown where the link really goes.
        assert_eq!(host("https://github.com@evil.example/x"), Some("evil.example"));
        assert_eq!(host("https://example.com:8443/x?y#z"), Some("example.com"));
        // Copied text can't carry what a terminal would run.
        assert_eq!(Ask::copy("ls\x1b[201~rm -rf ~\r\nnext\tx"), Some(Ask::Copy("ls[201~rm -rf ~\nnext\tx".into())));
        assert_eq!(Ask::copy(" \x1b\x07 "), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_with_a_link_or_too_much_in_it_is_refused() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-plugin-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        create_example().unwrap();
        let plugin = folder().join("checklist");
        assert_eq!(check_tree(&plugin), Ok(()));
        // A link to the home folder, as a cloned repository could carry.
        std::fs::create_dir_all(plugin.join("lib")).unwrap();
        std::os::unix::fs::symlink("/", plugin.join("lib").join("everything")).unwrap();
        assert!(check_tree(&plugin).unwrap_err().contains("link"));
        // Found in the plugins folder, it's listed with the reason and never started.
        let mut p = Plugins::scan();
        assert_eq!(p.list.len(), 1);
        let i = 0;
        assert!(check_tree(&p.list[i].dir).is_err());
        p.list[i].problem = check_tree(&p.list[i].dir).err();
        assert!(p.list[i].trouble().unwrap().contains("link") && !p.list[i].running());
        std::fs::remove_file(plugin.join("lib").join("everything")).unwrap();
        // Too many files, then one file that is too big.
        for n in 0..=MAX_FILES {
            std::fs::write(plugin.join("lib").join(format!("f{n}.luau")), "").unwrap();
        }
        assert!(check_tree(&plugin).unwrap_err().contains("too big"));
        std::fs::remove_dir_all(plugin.join("lib")).unwrap();
        assert_eq!(check_tree(&plugin), Ok(()));
        std::fs::write(plugin.join("big.bin"), vec![0u8; MAX_BYTES as usize + 1]).unwrap();
        assert!(check_tree(&plugin).unwrap_err().contains("too big"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_clone_never_waits_for_an_answer_and_is_stopped_in_time() {
        let cmd = clone_command("https://example.com/p.git", Path::new("/tmp/p"));
        let env: Vec<(String, String)> = cmd.get_envs().filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned()))).collect();
        assert!(env.contains(&("GIT_TERMINAL_PROMPT".into(), "0".into())));
        assert!(env.contains(&("GIT_SSH_COMMAND".into(), "ssh -oBatchMode=yes".into())));
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|w| w == ["--depth", "1"]) && args.contains(&"protocol.ext.allow=never".to_owned()));
        assert_eq!(&args[args.len() - 3..], ["--", "https://example.com/p.git", "/tmp/p"]);
        // A command that would hang is stopped, and the wait is about the limit, not its length.
        let mut hang = std::process::Command::new("sleep");
        hang.arg("30").stderr(std::process::Stdio::piped());
        let started = std::time::Instant::now();
        assert!(run_limited(hang, 1).unwrap_err().contains("Stopped after 1 seconds"));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        // A failure reports what the program said.
        let mut fails = std::process::Command::new("sh");
        fails.args(["-c", "echo nope >&2; exit 3"]).stderr(std::process::Stdio::piped());
        assert_eq!(run_limited(fails, 5).unwrap_err(), "git clone failed: nope");
    }

    #[test]
    fn install_scan_and_remove() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-plugins-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        let id = create_example().unwrap();
        assert_eq!(id, "checklist");
        // Installing from a folder copies it under a new name.
        let src = dir.join("src").join("My Plugin");
        copy_dir(&folder().join("checklist"), &src).unwrap();
        assert_eq!(install(src.to_str().unwrap()).unwrap(), "my-plugin");
        assert!(install(src.to_str().unwrap()).unwrap_err().contains("already"));
        assert!(install(dir.to_str().unwrap()).unwrap_err().contains("no plugin.json"));
        std::fs::create_dir_all(folder().join("broken")).unwrap();
        let p = Plugins::scan();
        let ids: Vec<_> = p.list.iter().map(|p| (p.id.as_str(), p.problem.is_some())).collect();
        assert_eq!(ids, [("broken", true), ("checklist", false), ("my-plugin", false)]);
        remove("my-plugin").unwrap();
        assert_eq!(Plugins::scan().list.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
