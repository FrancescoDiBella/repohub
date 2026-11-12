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

    fn make(&self, handle: &AppHandle<App>, i: usize) -> Option<Script> {
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
                api.set("open", lua.create_function(move |_, url: String| {
                    if url.starts_with("https://") || url.starts_with("http://") {
                        hh.update(move |_: &mut App, tx| { let _ = tx.open_url(&url); });
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
                api.set("copy", lua.create_function(move |_, text: String| {
                    hh.update(move |s: &mut App, tx| { tx.copy(text); s.notify("Copied"); });
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

    /// What plugin `i`'s function `name` (`repo` or `page`) draws.
    pub fn surface(&self, i: usize, cx: &rune::Cx<App>, name: &str) -> Option<rune::El> {
        Some(self.list.get(i)?.script.as_ref()?.surface(cx, name))
    }

    /// The plugins with a section on repository pages: (position, label).
    pub fn repo_tabs(&self) -> Vec<(usize, String)> {
        self.list.iter().enumerate().filter(|(_, p)| p.script.is_some()).filter_map(|(i, p)| Some((i, p.manifest.repo_tab.clone()?))).collect()
    }

    /// The plugins with a page in the sidebar: (position, label).
    pub fn pages(&self) -> Vec<(usize, String)> {
        self.list.iter().enumerate().filter(|(_, p)| p.script.is_some()).filter_map(|(i, p)| Some((i, p.manifest.page.clone()?))).collect()
    }
}

/// Installs a plugin from a folder (copied) or a Git address (cloned); returns its id.
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
        let out = std::process::Command::new("git").args(["clone", "--depth", "1", "--", source]).arg(&dest).output()
            .map_err(|e| format!("Couldn't run git: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(format!("git clone failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
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
    if let Err(e) = std::fs::read_to_string(dest.join("plugin.json")).map_err(|e| e.to_string()).and_then(|j| Manifest::parse(&j)) {
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
        ("starred", Data::Bool(r.starred)), ("updated", text(&r.updated)), ("pushed", Data::Integer(r.pushed)),
        ("topics", Data::List(r.topics.iter().map(|t| text(t)).collect())),
    ]);
    let (page, repo) = match s.page {
        Page::Repo(i) => ("repo", s.data.repos.get(i)),
        Page::Plugin(_) => ("plugin", None),
        Page::Overview => ("overview", None),
        Page::Repos => ("repos", None),
        Page::Pulls => ("pulls", None),
        Page::Issues => ("issues", None),
        Page::Actions => ("actions", None),
        Page::Settings => ("settings", None),
    };
    let in_repo = |name: &str| repo.is_some_and(|r| name.eq_ignore_ascii_case(&r.full_name) || name.eq_ignore_ascii_case(&r.name));
    let issues: Vec<Data> = s.data.issues.iter().filter(|x| in_repo(&x.repo)).map(|x| map(vec![
        ("number", int(x.number)), ("title", text(&x.title)), ("url", text(&x.url)), ("age", text(&x.age)),
        ("comments", int(x.comments)), ("label", text(x.label.as_ref().map_or("", |l| l.0.as_str()))),
    ])).collect();
    let pulls: Vec<Data> = s.data.pulls.iter().filter(|x| in_repo(&x.repo)).map(|x| map(vec![
        ("number", int(x.number)), ("title", text(&x.title)), ("url", text(&x.url)), ("age", text(&x.age)),
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
