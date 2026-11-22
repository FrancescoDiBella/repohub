//! The token stays out of child processes (terminal, git), and signing out clears it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
    temp_dir: Option<PathBuf>,
}

impl EnvGuard {
    fn new(vars: &[&'static str]) -> Self {
        let saved = vars.iter().map(|&k| (k, std::env::var_os(k))).collect();
        Self {
            saved,
            temp_dir: None,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
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
fn child_commands_strip_github_token_and_user_even_when_present_in_parent_env() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvGuard::new(&["GITHUB_TOKEN", "GITHUB_USER"]);

    let temp_dir = std::env::temp_dir().join(format!("repohub-test-child-strip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    guard.temp_dir = Some(temp_dir.clone());

    // Set GITHUB_TOKEN and GITHUB_USER in the parent process environment.
    unsafe {
        std::env::set_var("GITHUB_TOKEN", "parent_secret_token_123");
        std::env::set_var("GITHUB_USER", "parent_user_456");
    }

    // 1. git Command (local::git_command)
    let git_cmd = crate::local::git_command(&temp_dir, &["status"]);
    let git_envs: Vec<_> = git_cmd.get_envs().collect();
    assert!(
        git_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
        "git command must explicitly remove GITHUB_TOKEN (found {git_envs:?})"
    );
    assert!(
        git_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
        "git command must explicitly remove GITHUB_USER (found {git_envs:?})"
    );

    // 2. Claude Code Command (claude::command)
    let claude_cmd = crate::claude::command(&["--version".into()]);
    let claude_envs: Vec<_> = claude_cmd.get_envs().collect();
    assert!(
        claude_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
        "claude command must explicitly remove GITHUB_TOKEN (found {claude_envs:?})"
    );
    assert!(
        claude_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
        "claude command must explicitly remove GITHUB_USER (found {claude_envs:?})"
    );

    // 3. Plugins clone command (plugins::clone_command)
    let clone_cmd = crate::plugins::clone_command("https://example.com/repo.git", &temp_dir.join("dest"));
    let clone_envs: Vec<_> = clone_cmd.get_envs().collect();
    assert!(
        clone_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()),
        "plugins clone command must explicitly remove GITHUB_TOKEN (found {clone_envs:?})"
    );
    assert!(
        clone_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()),
        "plugins clone command must explicitly remove GITHUB_USER (found {clone_envs:?})"
    );

    // 4. Terminal CommandBuilder (terminal::shell)
    let term_cmd = crate::terminal::shell();
    assert_eq!(
        term_cmd.get_env("GITHUB_TOKEN"),
        None,
        "terminal shell must not contain GITHUB_TOKEN in its environment builder"
    );
    assert_eq!(
        term_cmd.get_env("GITHUB_USER"),
        None,
        "terminal shell must not contain GITHUB_USER in its environment builder"
    );
}

#[cfg(unix)]
#[test]
fn spawned_terminal_child_process_does_not_have_github_token_in_env() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvGuard::new(&["GITHUB_TOKEN", "GITHUB_USER"]);

    let temp_dir = std::env::temp_dir().join(format!("repohub-test-term-child-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    guard.temp_dir = Some(temp_dir.clone());

    unsafe {
        std::env::set_var("GITHUB_TOKEN", "leak_canary_token_xyz999");
        std::env::set_var("GITHUB_USER", "leak_canary_user_abc888");
    }

    // Spawn a real terminal using default settings (opt-in is false/None)
    let mut term = crate::terminal::Term::spawn(&temp_dir, 24, 80, None, || {}).expect("shell starts");
    // Print the environment variables inside the terminal shell. Use a split string for the sentinel
    // so the PTY's input echo does not contain "FINISHED" before the shell executes the command.
    term.write(b"printf 'TOK=[%s] USR=[%s] %s\\n' \"$GITHUB_TOKEN\" \"$GITHUB_USER\" \"FIN\"\"ISHED\"\r");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut output = String::new();
    while std::time::Instant::now() < deadline {
        output = term.text();
        if output.contains("FINISHED") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert!(output.contains("FINISHED"), "terminal output should reach sentinel (got: {output})");
    assert!(output.contains("TOK=[]"), "GITHUB_TOKEN must be empty in terminal shell output (got: {output})");
    assert!(output.contains("USR=[]"), "GITHUB_USER must be empty in terminal shell output (got: {output})");
    assert!(!output.contains("leak_canary_token_xyz999"));
    assert!(!output.contains("leak_canary_user_abc888"));
}

#[test]
fn signing_out_takes_effect_immediately_for_new_children_and_clears_process_env() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut guard = EnvGuard::new(&[
        "GITHUB_TOKEN",
        "GITHUB_USER",
        "REPOHUB_DATA_DIR",
        "REPOHUB_TERMINAL_PASS_TOKEN",
    ]);

    let temp_dir = std::env::temp_dir().join(format!("repohub-test-signout-child-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let data_dir = temp_dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    guard.temp_dir = Some(temp_dir);

    unsafe { std::env::set_var("REPOHUB_DATA_DIR", &data_dir) };

    let mut app = crate::init();
    let mut cfg = (*app.cfg).clone();
    cfg.account_locked = false;
    cfg.user = Some("signed_in_user".into());
    cfg.token = Some("signed_in_token_777".into());
    cfg.terminal_pass_token = true; // explicitly opted-in
    app.cfg = std::sync::Arc::new(cfg);

    // Pre-condition: opt-in enabled with active account -> term token would be passed
    let active_term_token = if app.cfg.terminal_pass_token { app.cfg.token.as_deref() } else { None };
    assert_eq!(active_term_token, Some("signed_in_token_777"));

    // User signs out via Settings
    app.save_account(true);

    // 1. Process environment must have NO GITHUB_TOKEN or GITHUB_USER
    assert!(std::env::var("GITHUB_TOKEN").is_err(), "GITHUB_TOKEN must not linger in process env after sign-out");
    assert!(std::env::var("GITHUB_USER").is_err(), "GITHUB_USER must not linger in process env after sign-out");

    // 2. Config has no token or user
    assert_eq!(app.cfg.token, None);
    assert_eq!(app.cfg.user, None);

    // 3. New terminal spawn immediately gets None for token, even if terminal_pass_token was true
    let term_token_after_sign_out = if app.cfg.terminal_pass_token { app.cfg.token.as_deref() } else { None };
    assert_eq!(term_token_after_sign_out, None, "sign-out must clear terminal token immediately");

    // 4. Newly built child commands for git, claude, plugins have NO token
    let git_cmd = crate::local::git_command(Path::new("."), &["status"]);
    let git_envs: Vec<_> = git_cmd.get_envs().collect();
    assert!(git_envs.iter().any(|(k, v)| *k == "GITHUB_TOKEN" && v.is_none()));
    assert!(git_envs.iter().any(|(k, v)| *k == "GITHUB_USER" && v.is_none()));
}
