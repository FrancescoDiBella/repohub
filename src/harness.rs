//! Coding assistants ("harnesses"): Claude Code, Codex, Aider, opencode or any other program
//! that works in a repository's folder.
//!
//! - Each harness is a name and the command that starts it. It gets a button in the Terminal
//!   tab and in ⌘K, which starts it in the repository's local clone.
//! - The Claude section (the Claude tab, and every harness or terminal command that runs
//!   `claude`) can be turned off; it's on by default.
//! - Saved in the preferences: `claude.enabled` and `harnesses`.
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Harness {
    pub name: String,
    pub command: String,
}

impl Harness {
    fn new(name: &str, command: &str) -> Harness {
        Harness { name: name.into(), command: command.into() }
    }
}

/// Well-known harnesses offered with one click in Settings.
pub const SUGGESTED: [(&str, &str); 6] = [
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Aider", "aider"),
    ("opencode", "opencode"),
    ("Gemini CLI", "gemini"),
    ("Goose", "goose session"),
];

/// Whether `command` starts Claude Code (its program is `claude`).
pub fn is_claude(command: &str) -> bool {
    let program = command.split_whitespace().next().unwrap_or("");
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    name.eq_ignore_ascii_case("claude") || name.eq_ignore_ascii_case("claude.exe")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assistants {
    /// The Claude section is on.
    pub claude: bool,
    pub harnesses: Vec<Harness>,
}

impl Default for Assistants {
    fn default() -> Self {
        Assistants { claude: true, harnesses: vec![Harness::new("Claude Code", "claude")] }
    }
}

impl Assistants {
    pub fn load() -> Assistants {
        let mut a = Assistants::default();
        if let Some(on) = crate::prefs::get("claude.enabled").and_then(|v| v.as_bool()) {
            a.claude = on;
        }
        if let Some(list) = crate::prefs::get("harnesses").and_then(|v| v.as_array().cloned()) {
            a.harnesses = list.iter().filter_map(|h| {
                let name = h.get("name")?.as_str()?.trim();
                let command = h.get("command")?.as_str()?.trim();
                (!name.is_empty() && !command.is_empty()).then(|| Harness::new(name, command))
            }).collect();
        }
        a
    }

    pub fn save(&self) {
        crate::prefs::set("claude.enabled", Value::from(self.claude));
        crate::prefs::set("harnesses", Value::from(self.harnesses.iter().map(|h| json!({"name": h.name, "command": h.command})).collect::<Vec<_>>()));
    }

    /// The harnesses to offer: those that run `claude` only while the Claude section is on.
    /// Each comes with its position in `harnesses`.
    pub fn shown(&self) -> impl Iterator<Item = (usize, &Harness)> {
        self.harnesses.iter().enumerate().filter(|(_, h)| self.claude || !is_claude(&h.command))
    }

    /// Whether a terminal command button should show.
    pub fn allows(&self, command: &str) -> bool {
        self.claude || !is_claude(command)
    }

    /// Adds a harness; the error says what's wrong with it.
    pub fn add(&mut self, name: &str, command: &str) -> Result<(), String> {
        let (name, command) = (name.trim(), command.trim());
        if name.is_empty() {
            return Err("Give it a name, such as Codex.".into());
        }
        if command.is_empty() {
            return Err("Write the command that starts it, such as codex.".into());
        }
        if command.contains('\n') || command.contains('\r') {
            return Err("The command has to fit on one line.".into());
        }
        if self.harnesses.iter().any(|h| h.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There is already one called {name}."));
        }
        self.harnesses.push(Harness::new(name, command));
        Ok(())
    }

    pub fn remove(&mut self, i: usize) -> Option<Harness> {
        (i < self.harnesses.len()).then(|| self.harnesses.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_switch_and_list() {
        let mut a = Assistants::default();
        assert!(a.add("Codex", "codex --full-auto").is_ok());
        assert!(a.add("codex", "x").unwrap_err().contains("already"));
        assert!(a.add("", "x").is_err() && a.add("X", " ").is_err() && a.add("X", "a\nb").is_err());
        assert_eq!(a.shown().map(|(_, h)| h.name.as_str()).collect::<Vec<_>>(), ["Claude Code", "Codex"]);
        a.claude = false;
        assert_eq!(a.shown().map(|(i, h)| (i, h.name.as_str())).collect::<Vec<_>>(), [(1, "Codex")]);
        assert!(!a.allows("claude --continue") && a.allows("git status"));
        assert!(is_claude("/usr/local/bin/claude -p") && is_claude("CLAUDE.EXE") && !is_claude("claudette"));
        assert_eq!(a.remove(0).map(|h| h.name), Some("Claude Code".into()));
        assert_eq!(a.remove(5), None);
    }

    #[test]
    fn saved_and_loaded() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-harness-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        assert_eq!(Assistants::load(), Assistants::default());
        let mut a = Assistants::default();
        a.claude = false;
        a.add("Aider", "aider --model sonnet").unwrap();
        a.save();
        assert_eq!(Assistants::load(), a);
        let _ = std::fs::remove_dir_all(dir);
    }
}
