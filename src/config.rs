//! Settings read from the environment and from a `.env` file (see `.env.example`).
//!
//! The token is only ever sent to the configured API host; it is never shown in the UI or logged.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigSources {
    pub token: Option<String>,
    pub user: Option<String>,
    pub api: Option<String>,
    pub orgs: Option<String>,
    pub pinned: Option<String>,
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
    /// The `.env` file that was loaded, if any.
    pub env_file: Option<String>,
    /// True when the user or token comes from the environment or a project `.env`, so the
    /// account form in Settings can't change it.
    pub account_locked: bool,
    /// Source attribution for connection settings.
    pub sources: ConfigSources,
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

fn flag(v: Option<String>, default: bool) -> bool {
    match v.as_deref().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("1" | "true" | "yes" | "si" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
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
    )
}

/// Where a project `.env` file was found and whether it was specified explicitly.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvFileLocation {
    Explicit(std::path::PathBuf),
    #[allow(dead_code)]
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
}

/// Finds the project `.env` file: `REPOHUB_ENV` (explicit), else `.env` in the current folder or one of its
/// parents in debug builds only. Installed/release builds only load a project `.env` when `REPOHUB_ENV` is set.
#[cfg(not(target_arch = "wasm32"))]
fn find_env_file() -> Option<EnvFileLocation> {
    if let Ok(p) = std::env::var("REPOHUB_ENV")
        && !p.trim().is_empty()
    {
        return Some(EnvFileLocation::Explicit(p.into()));
    }
    #[cfg(all(not(debug_assertions), not(test)))]
    {
        None
    }
    #[cfg(any(debug_assertions, test))]
    {
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
#[cfg(not(target_arch = "wasm32"))]
fn load_into_env_filtered<F>(path: &std::path::Path, filter: F) -> bool
where
    F: Fn(&str) -> bool,
{
    let Ok(src) = std::fs::read_to_string(path) else { return false };
    for (k, v) in parse_env(&src) {
        if filter(&k) && std::env::var_os(&k).is_none() {
            // SAFETY: called once at startup, before any other thread exists.
            unsafe { std::env::set_var(k, v) };
        }
    }
    true
}

/// Internal record of env files loaded and account origin.
#[cfg(not(target_arch = "wasm32"))]
struct LoadedEnvFiles {
    env_file: Option<String>,
    account_locked: bool,
    token_from_user_env: bool,
    project_loc: Option<EnvFileLocation>,
    initial_token_in_process: bool,
    initial_user_in_process: bool,
    user_file_has_api: bool,
    user_file_has_user: bool,
    user_file_has_token: bool,
    user_file_has_orgs: bool,
    user_file_has_pinned: bool,
    project_file_has_api: bool,
    project_file_has_user: bool,
    project_file_has_token: bool,
    project_file_has_orgs: bool,
    project_file_has_pinned: bool,
}

/// Loads the project `.env` (explicit or debug upward), then the app's own settings file for whatever is still unset.
#[cfg(not(target_arch = "wasm32"))]
fn load_env_files() -> LoadedEnvFiles {
    // 1. Initial process environment checks.
    let proc_has = |k: &str| std::env::var(k).is_ok_and(|v| !v.trim().is_empty());
    let initial_token_in_process = proc_has("GITHUB_TOKEN");
    let initial_user_in_process = proc_has("GITHUB_USER");

    // 2. Compute user_env_file before any upward project `.env` can touch REPOHUB_DATA_DIR.
    let user_path = user_env_file();

    // 3. Find project .env (explicit or debug upward).
    let project_loc = find_env_file();
    let mut project_file_loaded = false;
    let mut project_file_has_api = false;
    let mut project_file_has_user = false;
    let mut project_file_has_token = false;
    let mut project_file_has_orgs = false;
    let mut project_file_has_pinned = false;

    if let Some(loc) = &project_loc {
        let is_explicit = loc.is_explicit();
        let path = loc.path();
        if let Ok(src) = std::fs::read_to_string(path) {
            project_file_loaded = true;
            for (k, v) in parse_env(&src) {
                if v.trim().is_empty() {
                    continue;
                }
                match k.as_str() {
                    "GITHUB_API_URL" => project_file_has_api = true,
                    "GITHUB_USER" => project_file_has_user = true,
                    "GITHUB_TOKEN" => project_file_has_token = true,
                    "REPOHUB_ORGS" => project_file_has_orgs = true,
                    "REPOHUB_PINNED" => project_file_has_pinned = true,
                    _ => {}
                }
            }
            load_into_env_filtered(path, |k| is_explicit || !is_blocked_from_upward_env(k));
        }
    }

    let token_in_proc_or_proj = proc_has("GITHUB_TOKEN");
    let user_in_proc_or_proj = proc_has("GITHUB_USER");
    let account_set_before_user_file = token_in_proc_or_proj || user_in_proc_or_proj;

    // 4. Load user settings file (<data dir>/.env).
    let mut user_file_loaded = false;
    let mut user_file_has_api = false;
    let mut user_file_has_user = false;
    let mut user_file_has_token = false;
    let mut user_file_has_orgs = false;
    let mut user_file_has_pinned = false;

    if let Ok(src) = std::fs::read_to_string(&user_path) {
        user_file_loaded = true;
        for (k, v) in parse_env(&src) {
            if v.trim().is_empty() {
                continue;
            }
            match k.as_str() {
                "GITHUB_API_URL" => user_file_has_api = true,
                "GITHUB_USER" => user_file_has_user = true,
                "GITHUB_TOKEN" => user_file_has_token = true,
                "REPOHUB_ORGS" => user_file_has_orgs = true,
                "REPOHUB_PINNED" => user_file_has_pinned = true,
                _ => {}
            }
        }
        load_into_env_filtered(&user_path, |_| true);
    }

    let token_now = proc_has("GITHUB_TOKEN");
    let token_from_user_env = !token_in_proc_or_proj && token_now && user_file_has_token;

    let shown = project_loc.as_ref().filter(|_| project_file_loaded)
        .map(|loc| loc.path().display().to_string())
        .or_else(|| if user_file_loaded { Some(user_path.display().to_string()) } else { None });

    let account_locked = account_set_before_user_file;

    LoadedEnvFiles {
        env_file: shown,
        account_locked,
        token_from_user_env,
        project_loc: if project_file_loaded { project_loc } else { None },
        initial_token_in_process,
        initial_user_in_process,
        user_file_has_api,
        user_file_has_user,
        user_file_has_token,
        user_file_has_orgs,
        user_file_has_pinned,
        project_file_has_api,
        project_file_has_user,
        project_file_has_token,
        project_file_has_orgs,
        project_file_has_pinned,
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
    /// parents in debug builds) and the app's own settings file, then the process environment, which wins.
    pub fn load() -> Config {
        #[cfg(not(target_arch = "wasm32"))]
        let initial_api_in_env = std::env::var("GITHUB_API_URL").ok().filter(|v| !v.trim().is_empty());
        #[cfg(not(target_arch = "wasm32"))]
        let loaded = load_env_files();
        #[cfg(not(target_arch = "wasm32"))]
        let (env_file, account_locked) = (loaded.env_file.clone(), loaded.account_locked);
        #[cfg(target_arch = "wasm32")]
        let (env_file, account_locked): (Option<String>, bool) = (None, true);

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

        let token = get("GITHUB_TOKEN");
        let user = get("GITHUB_USER");

        // Determine API URL with safety rule:
        // If token came from user_env_file (<data dir>/.env), GITHUB_API_URL must come from
        // the process environment (if explicitly set before loading project .env),
        // or from <data dir>/.env, or be the default ("https://api.github.com").
        // A project .env (upward or REPOHUB_ENV) MUST NOT redirect the saved token.
        #[cfg(not(target_arch = "wasm32"))]
        let api = {
            if loaded.token_from_user_env {
                if let Some(ref proc_api) = initial_api_in_env {
                    proc_api.trim_end_matches('/').to_owned()
                } else if loaded.user_file_has_api {
                    get("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_owned()
                } else {
                    "https://api.github.com".into()
                }
            } else {
                get("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_owned()
            }
        };
        #[cfg(target_arch = "wasm32")]
        let api = "https://api.github.com".to_owned();

        // Compute source attribution.
        #[cfg(not(target_arch = "wasm32"))]
        let sources = {
            let user_file_str = user_env_file().display().to_string();
            let proj_file_str = loaded.project_loc.as_ref().map(|loc| {
                if loc.is_explicit() {
                    format!("explicit REPOHUB_ENV ({})", loc.path().display())
                } else {
                    format!("project .env ({})", loc.path().display())
                }
            });

            // Token source
            let token_src = if token.is_none() {
                None
            } else if loaded.initial_token_in_process {
                Some("the environment".into())
            } else if loaded.token_from_user_env {
                Some(user_file_str.clone())
            } else if loaded.project_file_has_token {
                proj_file_str.clone().or_else(|| Some("project .env".into()))
            } else if loaded.user_file_has_token {
                Some(user_file_str.clone())
            } else {
                Some("the environment".into())
            };

            // User source
            let user_src = if user.is_none() {
                None
            } else if loaded.initial_user_in_process {
                Some("the environment".into())
            } else if loaded.project_file_has_user {
                proj_file_str.clone().or_else(|| Some("project .env".into()))
            } else if loaded.user_file_has_user {
                Some(user_file_str.clone())
            } else {
                Some("the environment".into())
            };

            // API source
            let api_src = if initial_api_in_env.is_some() {
                Some("the environment".into())
            } else if loaded.token_from_user_env {
                if loaded.user_file_has_api {
                    Some(user_file_str.clone())
                } else {
                    Some("default (https://api.github.com)".into())
                }
            } else if loaded.project_file_has_api && loaded.project_loc.as_ref().is_some_and(|l| l.is_explicit()) {
                proj_file_str.clone().or_else(|| Some("REPOHUB_ENV".into()))
            } else if loaded.user_file_has_api {
                Some(user_file_str.clone())
            } else {
                Some("default (https://api.github.com)".into())
            };

            let orgs_src = if get("REPOHUB_ORGS").is_some() {
                if loaded.project_file_has_orgs {
                    proj_file_str.clone().or_else(|| Some("project .env".into()))
                } else if loaded.user_file_has_orgs {
                    Some(user_file_str.clone())
                } else {
                    Some("the environment".into())
                }
            } else {
                None
            };

            let pinned_src = if get("REPOHUB_PINNED").is_some() {
                if loaded.project_file_has_pinned {
                    proj_file_str.clone().or_else(|| Some("project .env".into()))
                } else if loaded.user_file_has_pinned {
                    Some(user_file_str.clone())
                } else {
                    Some("the environment".into())
                }
            } else {
                None
            };

            ConfigSources {
                token: token_src,
                user: user_src,
                api: api_src,
                orgs: orgs_src,
                pinned: pinned_src,
            }
        };
        #[cfg(target_arch = "wasm32")]
        let sources = ConfigSources::default();

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
            env_file,
            account_locked,
            sources,
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
            env_file: Some("provided by repohub-server (Docker)".into()),
            account_locked: true,
            sources: ConfigSources::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, MIN_MINUTES_WITHOUT_TOKEN, parse_env, save_account, user_env_file};

    #[test]
    fn web_config() {
        let c = Config::from_json(r#"{"user":"octocat","hasToken":true,"pinned":["a","b"],"theme":"dark","maxRepos":20}"#).unwrap();
        assert_eq!(c.user.as_deref(), Some("octocat"));
        assert!(c.live() && c.token.is_some());
        assert_eq!(c.pinned, vec!["a", "b"]);
        assert_eq!((c.theme, c.max_repos), (crate::state::Theme::Dark, 20));
        assert!(!Config::from_json(r#"{"hasToken":false}"#).unwrap().live());
    }

    #[test]
    fn refresh_interval() {
        let base = Config::from_json(r#"{"user":"octocat","hasToken":true,"refreshMinutes":5}"#).unwrap();
        assert_eq!(base.refresh_every(), Some(5));
        let no_token = Config::from_json(r#"{"user":"octocat","hasToken":false,"refreshMinutes":5}"#).unwrap();
        assert_eq!(no_token.refresh_every(), Some(MIN_MINUTES_WITHOUT_TOKEN), "60 requests an hour: at most every 30 minutes");
        let off = Config::from_json(r#"{"user":"octocat","hasToken":true,"refreshMinutes":0}"#).unwrap();
        assert_eq!(off.refresh_every(), None);
        let default = Config::from_json(r#"{"user":"octocat","hasToken":true}"#).unwrap();
        assert_eq!(default.refresh_every(), Some(10));
    }

    #[test]
    fn account_file() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-account-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        save_account(Some("octocat"), Some("ghp_x")).unwrap();
        std::fs::write(user_env_file(), std::fs::read_to_string(user_env_file()).unwrap() + "REPOHUB_TITLE=Mine\n").unwrap();
        save_account(Some("hubot"), None).unwrap();
        let pairs = parse_env(&std::fs::read_to_string(user_env_file()).unwrap());
        assert_eq!(pairs, vec![("REPOHUB_TITLE".to_owned(), "Mine".to_owned()), ("GITHUB_USER".to_owned(), "hubot".to_owned())]);
        assert!(save_account(Some("a\"b"), None).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(user_env_file()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn env_lines() {
        let src = "# comment\nGITHUB_USER=octocat\nREPOHUB_TITLE=RepoHub for Octocat  # title\nexport GITHUB_TOKEN=\"ghp abc\"\nEMPTY=\nREPOHUB_PINNED='a, b'\nbad line\n";
        let v = parse_env(src);
        assert_eq!(v, vec![
            ("GITHUB_USER".into(), "octocat".into()),
            ("REPOHUB_TITLE".into(), "RepoHub for Octocat".into()),
            ("GITHUB_TOKEN".into(), "ghp abc".into()),
            ("EMPTY".into(), "".into()),
            ("REPOHUB_PINNED".into(), "a, b".into()),
        ]);
    }

    struct EnvVarGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        temp_dir: Option<std::path::PathBuf>,
        orig_cwd: Option<std::path::PathBuf>,
    }

    impl EnvVarGuard {
        fn new(vars: &[&'static str]) -> Self {
            let saved = vars.iter().map(|&k| (k, std::env::var_os(k))).collect();
            for &k in vars {
                // SAFETY: serialized by ENV_LOCK
                unsafe { std::env::remove_var(k) };
            }
            Self {
                saved,
                temp_dir: None,
                orig_cwd: None,
            }
        }

        fn set_cwd(&mut self, path: &std::path::Path) {
            if self.orig_cwd.is_none() {
                self.orig_cwd = std::env::current_dir().ok();
            }
            std::env::set_current_dir(path).unwrap();
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(orig) = &self.orig_cwd {
                let _ = std::env::set_current_dir(orig);
            }
            // SAFETY: serialized by ENV_LOCK
            unsafe {
                for (k, v) in &self.saved {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
            if let Some(dir) = &self.temp_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    #[test]
    fn project_env_with_github_api_url_does_not_override_default_when_token_from_settings() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvVarGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "GITHUB_API_URL",
            "REPOHUB_ENV",
            "REPOHUB_DATA_DIR",
            "REPOHUB_UPDATE_API",
            "REPOHUB_CLAUDE",
        ]);

        let test_root = std::env::temp_dir().join(format!("repohub-test-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        let data_dir = test_root.join("data");
        let project_dir = test_root.join("project");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&project_dir).unwrap();
        guard.temp_dir = Some(test_root);

        // Put saved token in user data dir (.env)
        // SAFETY: serialized by ENV_LOCK
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };
        std::fs::write(data_dir.join(".env"), "GITHUB_TOKEN=\"saved_ghp_token\"\nGITHUB_USER=\"legit_user\"\n").unwrap();

        // Put malicious GITHUB_API_URL in project .env
        std::fs::write(project_dir.join(".env"), "GITHUB_API_URL=https://evil.example\n").unwrap();
        guard.set_cwd(&project_dir);

        let cfg = Config::load();
        assert_eq!(cfg.token.as_deref(), Some("saved_ghp_token"));
        assert_eq!(cfg.api, "https://api.github.com", "Project .env must not override GITHUB_API_URL when token is from settings file");
        assert_eq!(cfg.sources.api.as_deref(), Some("default (https://api.github.com)"));
        assert_eq!(cfg.sources.token.as_deref(), Some(data_dir.join(".env").display().to_string().as_str()));
    }

    #[test]
    fn upward_env_cannot_inject_update_api_claude_or_data_dir() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvVarGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "GITHUB_API_URL",
            "REPOHUB_ENV",
            "REPOHUB_DATA_DIR",
            "REPOHUB_UPDATE_API",
            "REPOHUB_CLAUDE",
            "REPOHUB_TERMINAL_COMMANDS",
            "REPOHUB_REPOS_DIR",
        ]);

        let test_root = std::env::temp_dir().join(format!("repohub-test-blocked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        let project_dir = test_root.join("untrusted-repo");
        let sub_dir = project_dir.join("subdir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        guard.temp_dir = Some(test_root);

        // In untrusted-repo/.env, set blocked sensitive keys
        std::fs::write(
            project_dir.join(".env"),
            "REPOHUB_UPDATE_API=https://evil.example/update\n\
             REPOHUB_CLAUDE=./evil.sh\n\
             REPOHUB_DATA_DIR=/tmp/evil-data\n\
             GITHUB_API_URL=https://evil.example/api\n\
             REPOHUB_TERMINAL_COMMANDS=evil-command\n\
             REPOHUB_TITLE=UntrustedProject\n",
        ).unwrap();

        guard.set_cwd(&sub_dir);

        let cfg = Config::load();
        // Safe key allowed in upward debug search:
        assert_eq!(cfg.title, "UntrustedProject");
        // Blocked keys MUST NOT be loaded into process env or update::api or claude or notes::data_dir:
        assert_eq!(crate::update::api(), "https://api.github.com", "REPOHUB_UPDATE_API must not be loaded from upward .env");
        assert_ne!(std::env::var("REPOHUB_CLAUDE").as_deref(), Ok("./evil.sh"));
        assert_ne!(crate::claude::find(), Some(std::path::PathBuf::from("./evil.sh")));
        assert_ne!(std::env::var("REPOHUB_DATA_DIR").as_deref(), Ok("/tmp/evil-data"));
        assert_ne!(crate::notes::data_dir(), std::path::PathBuf::from("/tmp/evil-data"));
        assert_ne!(std::env::var("REPOHUB_TERMINAL_COMMANDS").as_deref(), Ok("evil-command"));
        assert_eq!(cfg.api, "https://api.github.com");
    }

    #[test]
    fn mixed_source_attribution_preserves_distinct_sources() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvVarGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "GITHUB_API_URL",
            "REPOHUB_ENV",
            "REPOHUB_DATA_DIR",
        ]);

        let test_root = std::env::temp_dir().join(format!("repohub-test-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        let data_dir = test_root.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        guard.temp_dir = Some(test_root);

        // Put user in process environment only:
        // SAFETY: serialized by ENV_LOCK
        unsafe {
            std::env::set_var("REPOHUB_DATA_DIR", &data_dir);
            std::env::set_var("GITHUB_USER", "proc_user");
        };

        // Put token in data_dir/.env:
        let user_env_path = data_dir.join(".env");
        std::fs::write(&user_env_path, "GITHUB_TOKEN=\"saved_secret_tok\"\n").unwrap();

        let cfg = Config::load();
        assert_eq!(cfg.user.as_deref(), Some("proc_user"));
        assert_eq!(cfg.token.as_deref(), Some("saved_secret_tok"));
        assert_eq!(cfg.sources.user.as_deref(), Some("the environment"));
        assert_eq!(cfg.sources.token.as_deref(), Some(user_env_path.display().to_string().as_str()));
    }

    #[test]
    fn explicit_repohub_env_precedence_and_source_attribution() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvVarGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "GITHUB_API_URL",
            "REPOHUB_ENV",
            "REPOHUB_DATA_DIR",
        ]);

        let test_root = std::env::temp_dir().join(format!("repohub-test-explicit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        let data_dir = test_root.join("data");
        let explicit_env = test_root.join("custom.env");
        std::fs::create_dir_all(&data_dir).unwrap();
        guard.temp_dir = Some(test_root);

        // Put user in data_dir
        // SAFETY: serialized by ENV_LOCK
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };
        std::fs::write(data_dir.join(".env"), "GITHUB_USER=\"user_from_settings\"\n").unwrap();

        // Put explicit REPOHUB_ENV with enterprise URL and token
        std::fs::write(&explicit_env, "GITHUB_TOKEN=\"token_from_explicit\"\nGITHUB_API_URL=\"https://enterprise.local/api/v3\"\n").unwrap();
        // SAFETY: serialized by ENV_LOCK
        unsafe { std::env::set_var("REPOHUB_ENV", &explicit_env) };

        let cfg = Config::load();
        assert_eq!(cfg.token.as_deref(), Some("token_from_explicit"));
        assert_eq!(cfg.user.as_deref(), Some("user_from_settings"));
        assert_eq!(cfg.api, "https://enterprise.local/api/v3");
        assert!(cfg.account_locked, "Explicit REPOHUB_ENV setting token locks account");

        let explicit_str = format!("explicit REPOHUB_ENV ({})", explicit_env.display());
        assert_eq!(cfg.sources.token.as_deref(), Some(explicit_str.as_str()));
        assert_eq!(cfg.sources.user.as_deref(), Some(data_dir.join(".env").display().to_string().as_str()));
        assert_eq!(cfg.sources.api.as_deref(), Some(explicit_str.as_str()));
    }

    #[test]
    fn process_env_has_highest_precedence() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = EnvVarGuard::new(&[
            "GITHUB_TOKEN",
            "GITHUB_USER",
            "GITHUB_API_URL",
            "REPOHUB_ENV",
            "REPOHUB_DATA_DIR",
        ]);

        let test_root = std::env::temp_dir().join(format!("repohub-test-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&test_root);
        let data_dir = test_root.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        guard.temp_dir = Some(test_root);

        // SAFETY: serialized by ENV_LOCK
        unsafe {
            std::env::set_var("REPOHUB_DATA_DIR", &data_dir);
            std::env::set_var("GITHUB_TOKEN", "process_token");
            std::env::set_var("GITHUB_API_URL", "https://api.process.example");
        };

        std::fs::write(data_dir.join(".env"), "GITHUB_TOKEN=\"settings_token\"\nGITHUB_API_URL=https://settings.example\n").unwrap();

        let cfg = Config::load();
        assert_eq!(cfg.token.as_deref(), Some("process_token"));
        assert_eq!(cfg.api, "https://api.process.example");
        assert_eq!(cfg.sources.token.as_deref(), Some("the environment"));
        assert_eq!(cfg.sources.api.as_deref(), Some("the environment"));
        assert!(cfg.account_locked);
    }
}
