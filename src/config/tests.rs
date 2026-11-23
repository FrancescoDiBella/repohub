use super::{Config, MIN_MINUTES_WITHOUT_TOKEN, parse_env, save_account, user_env_file};

#[test]
fn web_config() {
    let c = Config::from_json(r#"{"user":"octocat","hasToken":true,"pinned":["a","b"],"theme":"dark","maxRepos":20}"#).unwrap();
    assert_eq!(c.user.as_deref(), Some("octocat"));
    assert!(c.live() && c.token.is_some());
    assert!(c.allow_writes);
    assert_eq!(c.pinned, vec!["a", "b"]);
    assert_eq!((c.theme, c.max_repos), (crate::state::Theme::Dark, 20));
    assert!(!Config::from_json(r#"{"hasToken":false}"#).unwrap().live());
    assert!(c.account_locked && !c.device, "a server's account is set in its .env");
    let phone = Config::from_json(r#"{"device":true,"accountEditable":true,"notes":true}"#).unwrap();
    assert!(phone.device && !phone.account_locked && !phone.live() && phone.notes);
    let no_writes = Config::from_json(r#"{"user":"octocat","hasToken":true,"allowWrites":false}"#).unwrap();
    assert!(!no_writes.allow_writes);
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
    assert_eq!(cfg.sources.api.as_ref().map(ToString::to_string).as_deref(), Some("default (https://api.github.com)"));
    assert_eq!(cfg.sources.token.as_ref().map(ToString::to_string).as_deref(), Some(data_dir.join(".env").display().to_string().as_str()));
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
    assert_eq!(cfg.sources.user.as_ref().map(ToString::to_string).as_deref(), Some("the environment"));
    assert_eq!(cfg.sources.token.as_ref().map(ToString::to_string).as_deref(), Some(user_env_path.display().to_string().as_str()));
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
    assert_eq!(cfg.sources.token.as_ref().map(ToString::to_string).as_deref(), Some(explicit_str.as_str()));
    assert_eq!(cfg.sources.user.as_ref().map(ToString::to_string).as_deref(), Some(data_dir.join(".env").display().to_string().as_str()));
    assert_eq!(cfg.sources.api.as_ref().map(ToString::to_string).as_deref(), Some(explicit_str.as_str()));
}

#[test]
fn a_saved_token_goes_to_the_settings_files_api_not_repohub_envs() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvVarGuard::new(&["GITHUB_TOKEN", "GITHUB_USER", "GITHUB_API_URL", "REPOHUB_ENV", "REPOHUB_DATA_DIR"]);
    let test_root = std::env::temp_dir().join(format!("repohub-test-two-apis-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_root);
    let data_dir = test_root.join("data");
    let explicit_env = test_root.join("custom.env");
    std::fs::create_dir_all(&data_dir).unwrap();
    guard.temp_dir = Some(test_root);
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };
    std::fs::write(data_dir.join(".env"), "GITHUB_TOKEN=saved\nGITHUB_API_URL=https://mine.example/api/v3/\n").unwrap();
    std::fs::write(&explicit_env, "GITHUB_API_URL=https://other.example\n").unwrap();
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::set_var("REPOHUB_ENV", &explicit_env) };

    let cfg = Config::load();
    assert_eq!(cfg.token.as_deref(), Some("saved"));
    assert_eq!(cfg.api, "https://mine.example/api/v3", "both files set the API: the saved token's own file wins");
    assert_eq!(cfg.sources.api, Some(super::Source::UserFile(data_dir.join(".env").display().to_string())));

    // Without an API of its own, the saved token goes to GitHub.
    std::fs::write(data_dir.join(".env"), "GITHUB_TOKEN=saved\n").unwrap();
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::remove_var("GITHUB_API_URL") };
    let cfg = Config::load();
    assert_eq!(cfg.api, "https://api.github.com");
    assert_eq!(cfg.sources.api, Some(super::Source::Default));
}

#[test]
fn a_project_env_gives_the_account_but_not_the_blocked_settings() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvVarGuard::new(&["GITHUB_TOKEN", "GITHUB_USER", "GITHUB_API_URL", "REPOHUB_ENV", "REPOHUB_DATA_DIR", "REPOHUB_CLAUDE", "REPOHUB_PINNED"]);
    let test_root = std::env::temp_dir().join(format!("repohub-test-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_root);
    let (data_dir, sub) = (test_root.join("data"), test_root.join("project").join("sub"));
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    guard.temp_dir = Some(test_root.clone());
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };
    let project_env = test_root.join("project").join(".env");
    std::fs::write(&project_env, "GITHUB_TOKEN=ghp_project\nGITHUB_API_URL=https://evil.example\nREPOHUB_CLAUDE=./evil.sh\nREPOHUB_PINNED=a\n").unwrap();
    guard.set_cwd(&sub);

    let cfg = Config::load();
    assert_eq!(cfg.token.as_deref(), Some("ghp_project"), "the documented `.env` + run flow keeps working");
    assert!(matches!(cfg.sources.token, Some(super::Source::ProjectFile(_))));
    assert_eq!(cfg.api, "https://api.github.com");
    assert_eq!(cfg.sources.api, Some(super::Source::Default));
    assert!(std::env::var("REPOHUB_CLAUDE").is_err());
    assert_eq!(cfg.ignored_keys, ["GITHUB_API_URL", "REPOHUB_CLAUDE"], "Settings says what was ignored");
    assert!(matches!(cfg.sources.pinned, Some(super::Source::ProjectFile(_))));

    // The same value from the environment is named as such.
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::set_var("REPOHUB_PINNED", "b") };
    assert_eq!(Config::load().sources.pinned, Some(super::Source::Environment));
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
    assert_eq!(cfg.sources.token.as_ref().map(ToString::to_string).as_deref(), Some("the environment"));
    assert_eq!(cfg.sources.api.as_ref().map(ToString::to_string).as_deref(), Some("the environment"));
    assert!(cfg.account_locked);
}

#[test]
fn config_load_does_not_set_github_token_or_user_in_process_env() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvVarGuard::new(&[
        "GITHUB_TOKEN",
        "GITHUB_USER",
        "GITHUB_API_URL",
        "REPOHUB_ENV",
        "REPOHUB_DATA_DIR",
        "REPOHUB_TERMINAL_PASS_TOKEN",
    ]);

    let test_root = std::env::temp_dir().join(format!("repohub-test-env-leak-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_root);
    let data_dir = test_root.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    guard.temp_dir = Some(test_root);

    // Put token and user in data_dir/.env:
    // SAFETY: serialized by ENV_LOCK
    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };
    std::fs::write(data_dir.join(".env"), "GITHUB_TOKEN=\"secret_ghp_12345\"\nGITHUB_USER=\"secret_octocat\"\n").unwrap();

    let cfg = Config::load();
    assert_eq!(cfg.token.as_deref(), Some("secret_ghp_12345"));
    assert_eq!(cfg.user.as_deref(), Some("secret_octocat"));
    assert!(!cfg.terminal_pass_token, "terminal_pass_token must be off by default");

    // GITHUB_TOKEN and GITHUB_USER MUST NOT be in the process environment!
    assert!(std::env::var("GITHUB_TOKEN").is_err(), "GITHUB_TOKEN must not be in std::env::var");
    assert!(std::env::var("GITHUB_USER").is_err(), "GITHUB_USER must not be in std::env::var");
}

#[test]
fn upward_env_blocks_terminal_pass_token() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvVarGuard::new(&[
        "GITHUB_TOKEN",
        "GITHUB_USER",
        "REPOHUB_ENV",
        "REPOHUB_DATA_DIR",
        "REPOHUB_TERMINAL_PASS_TOKEN",
    ]);

    let test_root = std::env::temp_dir().join(format!("repohub-test-term-pass-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_root);
    let project_dir = test_root.join("untrusted-project");
    let sub_dir = project_dir.join("sub");
    std::fs::create_dir_all(&sub_dir).unwrap();
    guard.temp_dir = Some(test_root);

    std::fs::write(
        project_dir.join(".env"),
        "REPOHUB_TERMINAL_PASS_TOKEN=true\n",
    ).unwrap();

    guard.set_cwd(&sub_dir);

    let cfg = Config::load();
    assert!(!cfg.terminal_pass_token, "REPOHUB_TERMINAL_PASS_TOKEN must not be accepted from upward project .env");
    assert!(std::env::var("REPOHUB_TERMINAL_PASS_TOKEN").is_err());
}
