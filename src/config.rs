//! Settings read from the environment and from a `.env` file (see `.env.example`).
//!
//! The token is only ever sent to the configured API host; it is never shown in the UI or logged.

/// Where a connection setting comes from. Settings shows it; the label is only for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A variable already set when the app started.
    Environment,
    /// The app's own settings file (`<data dir>/.env`).
    UserFile(String),
    /// The file `REPOHUB_ENV` points to.
    ExplicitFile(String),
    /// A `.env` found in the current folder or one of its parents.
    ProjectFile(String),
    Default,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Environment => f.write_str("the environment"),
            Source::UserFile(p) => f.write_str(p),
            Source::ExplicitFile(p) => write!(f, "explicit REPOHUB_ENV ({p})"),
            Source::ProjectFile(p) => write!(f, "project .env ({p})"),
            Source::Default => f.write_str("default (https://api.github.com)"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigSources {
    pub token: Option<Source>,
    pub user: Option<Source>,
    pub api: Option<Source>,
    pub orgs: Option<Source>,
    pub pinned: Option<Source>,
}

#[derive(Clone)]
pub struct Config {
    /// Personal access token (read-only is enough; "Starring" write to star from the app).
    pub token: Option<String>,
    /// Whose repositories to show; defaults to the token's owner.
    pub user: Option<String>,
    /// REST API base, `https://api.github.com` or a GitHub Enterprise `https://host/api/v3`.
    pub api: String,
    pub title: String,
    /// `Some(true)` dark, `Some(false)` light, `None` follows the system.
    pub theme: crate::state::Theme,
    /// Repository names (or `owner/name`) shown under "Pinned".
    pub pinned: Vec<String>,
    pub include_forks: bool,
    pub include_archived: bool,
    pub max_repos: usize,
    /// How many recently pushed repositories to read workflow runs and commit activity from.
    pub activity_repos: usize,
    /// Which organizations' repositories to show (`REPOHUB_ORGS`).
    pub orgs: crate::state::Orgs,
    /// Minutes between automatic refreshes (`REPOHUB_REFRESH_MINUTES`, 0: off). See `refresh_every`.
    pub refresh_minutes: u32,
    /// Force the sample data even when credentials are present.
    pub demo: bool,
    /// Notes (Excalidraw drawings) are available: true when `repohub-server` serves the app.
    pub notes: bool,
    /// Buttons above the terminal, each typing one command (`REPOHUB_TERMINAL_COMMANDS`, `;`-separated).
    pub terminal_commands: Vec<String>,
    /// Whether to pass `GITHUB_TOKEN` to the terminal shell (`REPOHUB_TERMINAL_PASS_TOKEN`, off by default).
    pub terminal_pass_token: bool,
    /// The `.env` file that was loaded, if any.
    pub env_file: Option<String>,
    /// True when the user or token comes from the environment or a project `.env`, so the
    /// account form in Settings can't change it.
    pub account_locked: bool,
    /// Source attribution for connection settings.
    pub sources: ConfigSources,
    /// Settings a project `.env` isn't allowed to change, found (and ignored) in the one loaded.
    pub ignored_keys: Vec<String>,
    /// The phone app (Android): the account and the drawings are kept on the device.
    pub device: bool,
    /// Whether write actions (creating issues, branches, PRs, merging, starring) are allowed.
    pub allow_writes: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("token", &self.token.as_ref().map(|_| "<set>"))
            .field("user", &self.user)
            .field("api", &self.api)
            .field("env_file", &self.env_file)
            .field("sources", &self.sources)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// Root URL for opening web pages on GitHub (or GitHub Enterprise).
    pub fn web_root(&self) -> String {
        let api = self.api.trim_end_matches('/');
        if api == "https://api.github.com" {
            "https://github.com".into()
        } else if let Some(stripped) = api.strip_suffix("/api/v3") {
            stripped.into()
        } else if let Some((scheme, rest)) = api.split_once("://") {
            let host = rest.split(['/', ':']).next().unwrap_or(rest);
            format!("{scheme}://{host}")
        } else {
            "https://github.com".into()
        }
    }
}

fn flag(v: Option<String>, default: bool) -> bool {
    match v.as_deref().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("1" | "true" | "yes" | "si" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

/// Removes `GITHUB_TOKEN` and `GITHUB_USER` from a child process command,
/// ensuring sensitive credentials are never leaked to child processes.
#[cfg(not(target_arch = "wasm32"))]
pub fn scrub_child_env(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd.env_remove("GITHUB_TOKEN").env_remove("GITHUB_USER")
}

/// Removes `GITHUB_TOKEN` and `GITHUB_USER` from a pseudo-terminal command builder,
/// ensuring credentials are never inherited by terminal shells.
#[cfg(not(target_arch = "wasm32"))]
pub fn scrub_pty_env(cmd: &mut portable_pty::CommandBuilder) -> &mut portable_pty::CommandBuilder {
    cmd.env_remove("GITHUB_TOKEN");
    cmd.env_remove("GITHUB_USER");
    cmd
}

/// Returns true if a key must never be loaded from an upward-searched project `.env`.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_blocked_from_upward_env(key: &str) -> bool {
    matches!(
        key,
        "GITHUB_API_URL"
            | "REPOHUB_UPDATE_API"
            | "REPOHUB_CLAUDE"
            | "REPOHUB_DATA_DIR"
            | "REPOHUB_TERMINAL_COMMANDS"
            | "REPOHUB_REPOS_DIR"
            | "REPOHUB_TERMINAL_PASS_TOKEN"
    )
}

/// Where a project `.env` file was found and whether it was specified explicitly.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvFileLocation {
    Explicit(std::path::PathBuf),
    Upward(std::path::PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
impl EnvFileLocation {
    pub fn path(&self) -> &std::path::Path {
        match self {
            EnvFileLocation::Explicit(p) | EnvFileLocation::Upward(p) => p,
        }
    }

    pub fn is_explicit(&self) -> bool {
        matches!(self, EnvFileLocation::Explicit(_))
    }

    fn source(&self) -> Source {
        let p = self.path().display().to_string();
        if self.is_explicit() { Source::ExplicitFile(p) } else { Source::ProjectFile(p) }
    }
}

/// Finds the project `.env` file: `REPOHUB_ENV` (explicit), else `.env` in the current folder or
/// one of its parents. A file found by searching isn't trusted with the settings in
/// [`is_blocked_from_upward_env`]: whoever wrote it may not be the user.
#[cfg(not(target_arch = "wasm32"))]
fn find_env_file() -> Option<EnvFileLocation> {
    if let Ok(p) = std::env::var("REPOHUB_ENV")
        && !p.trim().is_empty()
    {
        return Some(EnvFileLocation::Explicit(p.into()));
    }
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..4 {
        let candidate = dir.join(".env");
        if candidate.is_file() {
            return Some(EnvFileLocation::Upward(candidate));
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// The settings file the app itself writes (the account saved in Settings): `.env` in the data
/// folder, e.g. `~/Library/Application Support/RepoHub/.env`. An installed app has no project
/// folder, so this is where its configuration lives; any `.env.example` setting works in it.
#[cfg(not(target_arch = "wasm32"))]
pub fn user_env_file() -> std::path::PathBuf {
    crate::notes::data_dir().join(".env")
}

/// Puts a `.env` file's pairs into the process environment; variables already set are kept.
/// If `filter` returns false for a key, that key is skipped.
/// `GITHUB_TOKEN` and `GITHUB_USER` are never written to the process environment.
#[cfg(not(target_arch = "wasm32"))]
fn load_into_env_filtered<F>(path: &std::path::Path, filter: F) -> bool
where
    F: Fn(&str) -> bool,
{
    let Ok(src) = std::fs::read_to_string(path) else { return false };
    for (k, v) in parse_env(&src) {
        if k == "GITHUB_TOKEN" || k == "GITHUB_USER" {
            continue;
        }
        if filter(&k) && std::env::var_os(&k).is_none() {
            // SAFETY: called once at startup, before any other thread exists.
            unsafe { std::env::set_var(k, v) };
        }
    }
    true
}

/// The connection settings one `.env` file sets (empty values don't count).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct FileVals {
    token: Option<String>,
    user: Option<String>,
    api: Option<String>,
    orgs: bool,
    pinned: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn file_vals(src: &str, allowed: impl Fn(&str) -> bool) -> FileVals {
    let mut f = FileVals::default();
    for (k, v) in parse_env(src) {
        let v = v.trim();
        if v.is_empty() || !allowed(&k) {
            continue;
        }
        // Like the process environment, the first line that sets a key wins.
        match k.as_str() {
            "GITHUB_TOKEN" if f.token.is_none() => f.token = Some(v.to_owned()),
            "GITHUB_USER" if f.user.is_none() => f.user = Some(v.to_owned()),
            "GITHUB_API_URL" if f.api.is_none() => f.api = Some(v.to_owned()),
            "REPOHUB_ORGS" => f.orgs = true,
            "REPOHUB_PINNED" => f.pinned = true,
            _ => {}
        }
    }
    f
}

/// What the `.env` files and the starting environment say about the connection.
#[cfg(not(target_arch = "wasm32"))]
struct LoadedEnvFiles {
    env_file: Option<String>,
    account_locked: bool,
    token: Option<String>,
    user: Option<String>,
    api: String,
    sources: ConfigSources,
    ignored_keys: Vec<String>,
}

/// Loads the project `.env`, then the app's own settings file for whatever is still unset.
#[cfg(not(target_arch = "wasm32"))]
fn load_env_files() -> LoadedEnvFiles {
    // 1. What the process environment had before any file was read.
    let proc_val = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let env_token = proc_val("GITHUB_TOKEN");
    let env_user = proc_val("GITHUB_USER");
    let env_api = proc_val("GITHUB_API_URL");
    let (env_orgs, env_pinned) = (proc_val("REPOHUB_ORGS").is_some(), proc_val("REPOHUB_PINNED").is_some());

    // 2. The settings file's place, before a project `.env` could move REPOHUB_DATA_DIR.
    let user_path = user_env_file();

    // 3. The project `.env`.
    let mut project_loc = find_env_file();
    let mut project = FileVals::default();
    let mut ignored_keys = Vec::new();
    match project_loc.as_ref().map(|loc| (loc, std::fs::read_to_string(loc.path()))) {
        Some((loc, Ok(src))) => {
            let explicit = loc.is_explicit();
            let allowed = |k: &str| explicit || !is_blocked_from_upward_env(k);
            for (k, v) in parse_env(&src) {
                if !allowed(&k) && !v.trim().is_empty() && !ignored_keys.contains(&k) {
                    ignored_keys.push(k);
                }
            }
            project = file_vals(&src, allowed);
            load_into_env_filtered(loc.path(), allowed);
        }
        _ => project_loc = None,
    }
    let project_src = project_loc.as_ref().map(EnvFileLocation::source);

    // 4. The app's settings file (<data dir>/.env).
    let user_src = Source::UserFile(user_path.display().to_string());
    let (user_file_loaded, user_file) = match std::fs::read_to_string(&user_path) {
        Ok(src) => {
            load_into_env_filtered(&user_path, |_| true);
            (true, file_vals(&src, |_| true))
        }
        Err(_) => (false, FileVals::default()),
    };

    let account_locked = env_token.is_some() || env_user.is_some() || project.token.is_some() || project.user.is_some();

    // The first that has a value wins: environment, project file, settings file.
    let pick = |env: Option<String>, proj: Option<String>, user: Option<String>| -> Option<(String, Source)> {
        env.map(|v| (v, Source::Environment))
            .or_else(|| proj.zip(project_src.clone()))
            .or_else(|| user.map(|v| (v, user_src.clone())))
    };
    let token = pick(env_token, project.token, user_file.token);
    let user = pick(env_user, project.user, user_file.user);
    // A token saved in Settings is only ever sent to the host the user chose themselves: the
    // environment, the settings file, or GitHub. No project file can redirect it, not even one
    // named by REPOHUB_ENV.
    let saved_token = matches!(token, Some((_, Source::UserFile(_))));
    let api = pick(env_api, project.api.filter(|_| !saved_token), user_file.api)
        .unwrap_or_else(|| ("https://api.github.com".into(), Source::Default));
    let flag_src = |env: bool, proj: bool, user: bool| {
        if env { Some(Source::Environment) }
        else if proj { project_src.clone() }
        else if user { Some(user_src.clone()) }
        else { None }
    };
    let sources = ConfigSources {
        orgs: flag_src(env_orgs, project.orgs, user_file.orgs),
        pinned: flag_src(env_pinned, project.pinned, user_file.pinned),
        token: token.as_ref().map(|t| t.1.clone()),
        user: user.as_ref().map(|u| u.1.clone()),
        api: Some(api.1),
    };

    let env_file = project_loc.as_ref().map(|loc| loc.path().display().to_string())
        .or_else(|| user_file_loaded.then(|| user_path.display().to_string()));

    LoadedEnvFiles {
        env_file,
        account_locked,
        token: token.map(|t| t.0),
        user: user.map(|u| u.0),
        api: api.0.trim_end_matches('/').to_owned(),
        sources,
        ignored_keys,
    }
}

/// Writes the account to the app's settings file, keeping its other lines. `None` removes a key.
/// The file holds the token, so on Unix only the user can read it.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_account(user: Option<&str>, token: Option<&str>) -> Result<(), String> {
    let path = user_env_file();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old.lines()
        .filter(|l| {
            let k = l.trim().strip_prefix("export ").unwrap_or(l.trim());
            !(k.starts_with("GITHUB_USER=") || k.starts_with("GITHUB_TOKEN="))
        })
        .map(str::to_owned).collect();
    if lines.is_empty() {
        lines.push("# RepoHub settings. The account is written by the app (Settings); any setting from .env.example works here.".into());
    }
    for (k, v) in [("GITHUB_USER", user), ("GITHUB_TOKEN", token)] {
        if let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) {
            if v.contains(['\n', '\r', '"']) {
                return Err(format!("{k} can't contain quotes or line breaks."));
            }
            lines.push(format!("{k}=\"{v}\""));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut f = opts.open(&path).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    f.write_all((lines.join("\n") + "\n").as_bytes()).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

/// `KEY=value` pairs from a `.env` file's text.
pub fn parse_env(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim();
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let v = v.trim();
        let v = if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
            v[1..].split(q).next().unwrap_or("").to_owned()
        } else {
            // An unquoted value ends at " #" (a trailing comment).
            v.split(" #").next().unwrap_or("").trim().to_owned()
        };
        out.push((k.to_owned(), v));
    }
    out
}

/// The terminal's default command buttons (coding assistants come from `harness`).
pub fn default_commands() -> Vec<String> {
    ["git status", "git pull", "git log --oneline -15"].iter().map(|s| s.to_string()).collect()
}

/// Without a token GitHub allows 60 requests an hour, and a refresh takes about
/// 4 + 2 × `activity_repos`: never refresh more often than this then.
pub const MIN_MINUTES_WITHOUT_TOKEN: u32 = 30;

impl Config {
    /// Minutes between automatic refreshes, or `None` when they're off or there's no account.
    pub fn refresh_every(&self) -> Option<u32> {
        if !self.live() || self.refresh_minutes == 0 {
            return None;
        }
        Some(if self.token.is_some() { self.refresh_minutes.max(1) } else { self.refresh_minutes.max(MIN_MINUTES_WITHOUT_TOKEN) })
    }

    /// Reads the project `.env` (from `REPOHUB_ENV`, else the current folder or one of its
    /// parents) and the app's own settings file, then the process environment, which wins.
    pub fn load() -> Config {
        #[cfg(not(target_arch = "wasm32"))]
        let loaded = load_env_files();
        #[cfg(not(target_arch = "wasm32"))]
        let (env_file, account_locked, token, user, api, sources, ignored_keys) =
            (loaded.env_file, loaded.account_locked, loaded.token, loaded.user, loaded.api, loaded.sources, loaded.ignored_keys);
        #[cfg(target_arch = "wasm32")]
        let (env_file, account_locked, token, user, api, sources, ignored_keys) =
            (None::<String>, true, None::<String>, None::<String>, String::from("https://api.github.com"), ConfigSources::default(), Vec::<String>::new());

        let get = |k: &str| -> Option<String> {
            #[cfg(not(target_arch = "wasm32"))]
            {
                std::env::var(k).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = k;
                None
            }
        };

        let num = |k: &str, d: usize| get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Config {
            token,
            user,
            api,
            title: get("REPOHUB_TITLE").unwrap_or_else(|| "RepoHub".into()),
            theme: get("REPOHUB_THEME").and_then(|t| crate::state::Theme::parse(&t)).unwrap_or_default(),
            pinned: get("REPOHUB_PINNED").map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
            include_forks: flag(get("REPOHUB_INCLUDE_FORKS"), true),
            include_archived: flag(get("REPOHUB_INCLUDE_ARCHIVED"), false),
            max_repos: num("REPOHUB_MAX_REPOS", 100).clamp(1, 1000),
            activity_repos: num("REPOHUB_ACTIVITY_REPOS", 5).clamp(0, 20),
            orgs: crate::state::Orgs::parse(&get("REPOHUB_ORGS").unwrap_or_default()),
            refresh_minutes: num("REPOHUB_REFRESH_MINUTES", 10).min(24 * 60) as u32,
            demo: flag(get("REPOHUB_DEMO"), false),
            // The desktop app keeps notes in a local folder; the web build needs repohub-server.
            notes: cfg!(not(target_arch = "wasm32")),
            terminal_commands: get("REPOHUB_TERMINAL_COMMANDS")
                .map(|v| v.split(';').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_else(default_commands),
            terminal_pass_token: flag(get("REPOHUB_TERMINAL_PASS_TOKEN"), false),
            env_file,
            account_locked,
            sources,
            ignored_keys,
            device: false,
            allow_writes: true,
        }
    }

    /// True when the app should read from GitHub instead of showing the sample data.
    pub fn live(&self) -> bool {
        !self.demo && (self.token.is_some() || self.user.is_some())
    }

    /// The web build's configuration, as `repohub-server` publishes it in `config.json`. It never
    /// contains the token, only whether the server has one (`hasToken`).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn from_json(src: &str) -> Option<Config> {
        let v: serde_json::Value = serde_json::from_str(src).ok()?;
        let st = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned).filter(|x| !x.is_empty());
        let bo = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        let nu = |k: &str, d: usize| v.get(k).and_then(|x| x.as_u64()).map(|x| x as usize).unwrap_or(d);
        Some(Config {
            // A marker, not a secret: the server adds the real token to proxied requests.
            token: bo("hasToken", false).then(|| "server".to_owned()),
            user: st("user"),
            api: st("api").unwrap_or_else(|| "https://api.github.com".into()),
            title: st("title").unwrap_or_else(|| "RepoHub".into()),
            theme: st("theme").and_then(|t| crate::state::Theme::parse(&t)).unwrap_or_default(),
            pinned: v.get("pinned").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            include_forks: bo("includeForks", true),
            include_archived: bo("includeArchived", false),
            max_repos: nu("maxRepos", 100).clamp(1, 1000),
            activity_repos: nu("activityRepos", 5).clamp(0, 20),
            orgs: crate::state::Orgs::parse(&st("orgs").unwrap_or_default()),
            refresh_minutes: nu("refreshMinutes", 10).min(24 * 60) as u32,
            demo: bo("demo", false),
            notes: bo("notes", false),
            terminal_commands: default_commands(),
            terminal_pass_token: false,
            env_file: Some(if bo("device", false) { "kept by the app on this device" } else { "provided by repohub-server (Docker)" }.into()),
            // The phone app lets Settings change the account; a server's comes from its `.env`.
            account_locked: !bo("accountEditable", false),
            sources: ConfigSources::default(),
            ignored_keys: Vec::new(),
            device: bo("device", false),
            allow_writes: bo("allowWrites", true),
        })
    }
}

#[cfg(test)]
mod tests;
