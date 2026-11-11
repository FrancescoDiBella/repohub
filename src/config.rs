//! Settings read from the environment and from a `.env` file (see `.env.example`).
//!
//! The token is only ever sent to the configured API host; it is never shown in the UI or logged.

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
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("token", &self.token.as_ref().map(|_| "<set>"))
            .field("user", &self.user)
            .field("api", &self.api)
            .field("env_file", &self.env_file)
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

/// Finds the project `.env` file: `REPOHUB_ENV`, else `.env` in the current folder or one of its
/// parents.
#[cfg(not(target_arch = "wasm32"))]
fn find_env_file() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("REPOHUB_ENV") {
        return Some(p.into());
    }
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..4 {
        let candidate = dir.join(".env");
        if candidate.is_file() {
            return Some(candidate);
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
#[cfg(not(target_arch = "wasm32"))]
fn load_into_env(path: &std::path::Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else { return false };
    for (k, v) in parse_env(&src) {
        if std::env::var_os(&k).is_none() {
            // SAFETY: called once at startup, before any other thread exists.
            unsafe { std::env::set_var(k, v) };
        }
    }
    true
}

/// Loads the project `.env`, then the app's own settings file for whatever is still unset.
/// Returns the files read and whether the account was already set before the app's own file.
#[cfg(not(target_arch = "wasm32"))]
fn load_env_files() -> (Option<String>, bool) {
    let project = find_env_file().filter(|p| load_into_env(p));
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.trim().is_empty());
    let locked = set("GITHUB_TOKEN") || set("GITHUB_USER");
    let own = user_env_file();
    let own = load_into_env(&own).then_some(own);
    let shown = project.or(own).map(|p| p.display().to_string());
    (shown, locked)
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

/// The terminal's default command buttons.
pub fn default_commands() -> Vec<String> {
    ["claude", "git status", "git pull", "git log --oneline -15"].iter().map(|s| s.to_string()).collect()
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
        let (env_file, account_locked) = load_env_files();
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
        let num = |k: &str, d: usize| get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Config {
            token: get("GITHUB_TOKEN"),
            user: get("GITHUB_USER"),
            api: get("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_owned(),
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
}
