//! The Claude tab's engine (desktop): runs the user's own Claude Code in the repository's local
//! clone and turns its `stream-json` output into chat items.
//!
//! Each message is one `claude -p --output-format stream-json --verbose` run with the prompt on
//! stdin (no shell quoting), `--permission-mode` from the tab's switch, and `--resume <session>`
//! after the first answer so the conversation goes on. Claude Code keeps its own login, so
//! RepoHub never sees an API key.
//!
//! ```text
//!            send                 init               result / exit
//! Idle ──────────────▶ Starting ─────────▶ Running ──────────────────▶ Idle
//!                          │ Stop             │ Stop
//!                          ▼                  ▼
//!                       Stopping ─────── exit ───────▶ Idle
//! ```
use serde_json::Value;

/// The chat's run state (drawn above).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Run {
    #[default]
    Idle,
    /// The process started; Claude Code hasn't reported its session yet.
    Starting,
    Running,
    /// Stop was pressed; waiting for the process to end.
    Stopping,
}

impl Run {
    pub fn is_busy(self) -> bool {
        self != Run::Idle
    }
}

/// What Claude may do without asking. The tab can't answer permission prompts, so these are the
/// headless modes: read-only planning, or editing files in the clone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Plan,
    Edit,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Plan, Mode::Edit];
    pub const LABELS: [&'static str; 2] = ["Plan (read-only)", "Edit files"];

    /// The value of `--permission-mode`.
    pub fn flag(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Edit => "acceptEdits",
        }
    }

    pub fn index(self) -> usize {
        Mode::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }
}

/// One thing in the conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// What the user sent.
    User(String),
    /// Claude's text, Markdown.
    Text(String),
    /// A tool Claude used: its name, a one-line summary of the input, and its result once back.
    Tool { id: String, name: String, summary: String, result: Option<String>, error: bool },
    /// The end of an answer.
    Done { seconds: f64, cost: f64, turns: u32, error: Option<String>, denied: Vec<String> },
    /// Something went wrong outside Claude's answer (not installed, not logged in, crashed).
    Problem(String),
}

/// One repository's conversation.
#[derive(Default)]
pub struct Chat {
    pub items: Vec<Item>,
    /// Claude Code's session, for `--resume`.
    pub session: Option<String>,
    pub model: Option<String>,
    pub run: Run,
    /// Counts runs, so output still arriving from a stopped run doesn't land in a new one.
    pub run_id: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub proc: Option<Proc>,
}

impl Chat {
    /// Applies one parsed event.
    pub fn apply(&mut self, e: Event) {
        match e {
            Event::Session { id, model } => {
                self.session = Some(id);
                if model.is_some() {
                    self.model = model;
                }
                if self.run == Run::Starting {
                    self.run = Run::Running;
                }
            }
            Event::Text(t) => {
                // Consecutive text blocks read as one message.
                if let Some(Item::Text(prev)) = self.items.last_mut() {
                    prev.push_str("\n\n");
                    prev.push_str(&t);
                } else {
                    self.items.push(Item::Text(t));
                }
            }
            Event::ToolUse { id, name, summary } => self.items.push(Item::Tool { id, name, summary, result: None, error: false }),
            Event::ToolResult { id, text, error } => {
                if let Some(Item::Tool { result, error: e, .. }) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Tool { id: x, .. } if *x == id)) {
                    *result = Some(text);
                    *e = error;
                }
            }
            Event::Done { seconds, cost, turns, error, denied, session } => {
                if session.is_some() {
                    self.session = session;
                }
                self.items.push(Item::Done { seconds, cost, turns, error, denied });
            }
        }
    }

    /// The process ended. `problem` explains an exit that produced no answer.
    pub fn finished(&mut self, problem: Option<String>) {
        let stopped = self.run == Run::Stopping;
        self.run = Run::Idle;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.proc = None;
        }
        let answered = matches!(self.items.last(), Some(Item::Done { .. }));
        if stopped && !answered {
            self.items.push(Item::Problem("Stopped.".into()));
        } else if let Some(p) = problem.filter(|_| !answered) {
            self.items.push(Item::Problem(p));
        }
    }

    /// Total cost of the conversation so far, in dollars.
    pub fn cost(&self) -> f64 {
        self.items.iter().map(|i| if let Item::Done { cost, .. } = i { *cost } else { 0.0 }).sum()
    }
}

/// What one line of `stream-json` says.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Session { id: String, model: Option<String> },
    Text(String),
    ToolUse { id: String, name: String, summary: String },
    ToolResult { id: String, text: String, error: bool },
    Done { seconds: f64, cost: f64, turns: u32, error: Option<String>, denied: Vec<String>, session: Option<String> },
}

fn st(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// A tool's input in a few words: the file, the command, the pattern…
pub fn summarize(name: &str, input: &Value) -> String {
    let pick = |keys: &[&str]| keys.iter().find_map(|k| input.get(*k).and_then(Value::as_str).map(str::to_owned));
    let s = match name {
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => pick(&["file_path", "notebook_path"]),
        "Bash" => pick(&["command"]),
        "Grep" | "Glob" => pick(&["pattern"]).map(|p| match pick(&["path"]) { Some(at) => format!("{p}  in {at}"), None => p }),
        "WebFetch" => pick(&["url"]),
        "WebSearch" => pick(&["query"]),
        "Task" | "Agent" => pick(&["description", "prompt"]),
        "TodoWrite" => input.get("todos").and_then(Value::as_array).map(|t| format!("{} items", t.len())),
        _ => None,
    };
    let s = s.or_else(|| input.as_object().and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_owned)))).unwrap_or_default();
    let line = s.lines().next().unwrap_or("").trim().to_owned();
    if line.chars().count() > 120 { format!("{}…", line.chars().take(119).collect::<String>()) } else { line }
}

/// A tool result's text, whatever shape it came in.
fn result_text(c: &Value) -> String {
    match c.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Parses one line of Claude Code's `stream-json` output. Unknown lines say nothing.
pub fn parse_line(line: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return Vec::new() };
    match (st(&v, "type").as_str(), st(&v, "subtype").as_str()) {
        ("system", "init") => vec![Event::Session { id: st(&v, "session_id"), model: Some(st(&v, "model")).filter(|m| !m.is_empty()) }],
        ("assistant", _) => {
            // Messages of sub-agents (`parent_tool_use_id`) stay inside their Task row.
            if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
                return Vec::new();
            }
            let content = v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
            content.iter().filter_map(|c| match st(c, "type").as_str() {
                "text" => Some(st(c, "text")).filter(|t| !t.trim().is_empty()).map(Event::Text),
                "tool_use" => {
                    let name = st(c, "name");
                    let summary = summarize(&name, c.get("input").unwrap_or(&Value::Null));
                    Some(Event::ToolUse { id: st(c, "id"), name, summary })
                }
                _ => None,
            }).collect()
        }
        ("user", _) => {
            let content = v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
            content.iter().filter(|c| st(c, "type") == "tool_result").map(|c| Event::ToolResult {
                id: st(c, "tool_use_id"),
                text: result_text(c),
                error: c.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            }).collect()
        }
        ("result", sub) => {
            let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false) || sub.starts_with("error");
            let denied = v.get("permission_denials").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|d| {
                let name = st(d, "tool_name");
                let what = summarize(&name, d.get("tool_input").unwrap_or(&Value::Null));
                if what.is_empty() { name } else { format!("{name}: {what}") }
            }).collect();
            let error = is_error.then(|| {
                let r = st(&v, "result");
                if !r.is_empty() { r } else if sub == "error_max_turns" { "Claude stopped: too many turns.".into() } else { format!("Claude Code ended with an error ({sub}).") }
            });
            vec![Event::Done {
                seconds: v.get("duration_ms").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0,
                cost: v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0),
                turns: v.get("num_turns").and_then(Value::as_u64).unwrap_or(0) as u32,
                error,
                denied,
                session: Some(st(&v, "session_id")).filter(|s| !s.is_empty()),
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{Proc, start};
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use native::find;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use native::command;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};

    use super::{Event, Mode, parse_line};

    /// A running `claude`. Stopping kills it.
    pub struct Proc {
        child: Arc<Mutex<Option<std::process::Child>>>,
    }

    impl Proc {
        pub fn stop(&self) {
            if let Ok(mut c) = self.child.lock()
                && let Some(c) = c.as_mut()
            {
                let _ = c.kill();
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            self.stop();
        }
    }

    /// Where `claude` is: `REPOHUB_CLAUDE`, else the usual install places. `None`: let the login
    /// shell find it on its PATH (apps opened from the Dock don't get the shell's PATH).
    pub fn find() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("REPOHUB_CLAUDE")
            && !p.trim().is_empty()
        {
            return Some(crate::clones_shim::expand(&p));
        }
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).map(PathBuf::from).ok()?;
        let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
        let dirs = [
            home.join(".local/bin"),
            home.join(".claude/local"),
            home.join(".npm-global/bin"),
            home.join("AppData/Roaming/npm"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
    }

    /// The command line: `claude -p … --resume <session>`, through the login shell when needed.
    pub(crate) fn command(flags: &[String]) -> Command {
        let mut cmd = match find() {
            Some(bin) if bin.extension().is_some_and(|e| e == "cmd") => {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(bin).args(flags);
                c
            }
            Some(bin) => {
                let mut c = Command::new(bin);
                c.args(flags);
                c
            }
            None if cfg!(windows) => {
                let mut c = Command::new("cmd");
                c.args(["/C", "claude"]).args(flags);
                c
            }
            None => {
                let shell = std::env::var("SHELL").ok().filter(|s| Path::new(s).exists()).unwrap_or_else(|| "/bin/sh".into());
                let mut c = Command::new(shell);
                c.args(["-l", "-c", "exec claude \"$@\"", "claude"]).args(flags);
                c
            }
        };
        crate::config::scrub_child_env(&mut cmd);
        cmd
    }

    /// Starts one answer: `prompt` in `cwd`. `on_event` runs for each event (on a reader
    /// thread); `on_exit` once at the end, with an explanation when no answer came.
    pub fn start(
        cwd: &Path,
        prompt: &str,
        mode: Mode,
        resume: Option<&str>,
        on_event: impl Fn(Event) + Send + 'static,
        on_exit: impl FnOnce(Option<String>) + Send + 'static,
    ) -> Result<Proc, String> {
        let mut flags: Vec<String> = ["-p", "--output-format", "stream-json", "--verbose", "--permission-mode", mode.flag()].map(String::from).to_vec();
        if let Some(id) = resume {
            flags.extend(["--resume".into(), id.into()]);
        }
        let mut cmd = command(&flags);
        cmd.current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).env("REPOHUB", "1");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up
        }
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start Claude Code: {e}. Install it from claude.ai/code, or set REPOHUB_CLAUDE to its path."))?;
        // The prompt goes on stdin, so nothing in it is read by a shell.
        if let Some(mut stdin) = child.stdin.take() {
            let p = prompt.to_owned();
            std::thread::spawn(move || {
                let _ = stdin.write_all(p.as_bytes());
            });
        }
        let stdout = child.stdout.take().ok_or("no output from claude")?;
        let mut stderr = child.stderr.take().ok_or("no error output from claude")?;
        let err_text = Arc::new(Mutex::new(String::new()));
        let err_sink = err_text.clone();
        let err_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            if let Ok(mut e) = err_sink.lock() {
                *e = s;
            }
        });
        let child = Arc::new(Mutex::new(Some(child)));
        let waiter = child.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                for e in parse_line(&line) {
                    on_event(e);
                }
            }
            let _ = err_thread.join();
            let status = waiter.lock().ok().and_then(|mut c| c.as_mut().and_then(|c| c.wait().ok()));
            let stderr = err_text.lock().map(|s| s.trim().to_owned()).unwrap_or_default();
            let problem = match status {
                Some(s) if s.success() => None,
                Some(s) if s.code() == Some(127) || stderr.contains("command not found") || stderr.contains("not recognized") =>
                    Some("Claude Code isn't installed, or RepoHub can't find it. Install it from claude.ai/code, or set REPOHUB_CLAUDE to the claude program's path.".into()),
                Some(s) => Some(if stderr.is_empty() { format!("Claude Code ended with {s}.") } else { last_lines(&stderr, 6) }),
                None => Some("Claude Code ended unexpectedly.".into()),
            };
            on_exit(problem);
        });
        Ok(Proc { child })
    }

    fn last_lines(s: &str, n: usize) -> String {
        let lines: Vec<&str> = s.lines().collect();
        lines[lines.len().saturating_sub(n)..].join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines as Claude Code 2.1 prints them (trimmed), plus a tool round trip.
    const STREAM: &str = r#"{"type":"active_goal","value":null,"session_id":"s1"}
{"type":"system","subtype":"init","cwd":"/r","session_id":"s1","tools":["Read"],"model":"claude-opus-5-5","permissionMode":"plan"}
{"type":"assistant","message":{"content":[{"type":"text","text":"I'll look at the files."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la\necho done","description":"List"}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"README.md\nsrc","is_error":false}]},"session_id":"s1"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Inside a sub-agent"}]},"parent_tool_use_id":"t9","session_id":"s1"}
{"type":"assistant","message":{"content":[{"type":"text","text":"There are **two** entries."}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"rate_limit_event","rate_limit_info":{},"session_id":"s1"}
{"type":"result","subtype":"success","is_error":false,"duration_ms":4220,"num_turns":2,"result":"There are two entries.","session_id":"s1","total_cost_usd":0.1007,"permission_denials":[{"tool_name":"Edit","tool_use_id":"t2","tool_input":{"file_path":"/r/README.md"}}]}"#;

    #[test]
    fn a_stream_becomes_a_chat() {
        let mut chat = Chat { run: Run::Starting, ..Default::default() };
        for line in STREAM.lines() {
            for e in parse_line(line) {
                chat.apply(e);
            }
        }
        assert_eq!(chat.session.as_deref(), Some("s1"));
        assert_eq!(chat.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(chat.run, Run::Running);
        assert_eq!(chat.items.len(), 4, "{:#?}", chat.items);
        assert_eq!(chat.items[0], Item::Text("I'll look at the files.".into()));
        assert_eq!(chat.items[1], Item::Tool { id: "t1".into(), name: "Bash".into(), summary: "ls -la".into(), result: Some("README.md\nsrc".into()), error: false });
        assert_eq!(chat.items[2], Item::Text("There are **two** entries.".into()), "the sub-agent's text is left out");
        match &chat.items[3] {
            Item::Done { seconds, turns, error, denied, .. } => {
                assert!((seconds - 4.22).abs() < 1e-9 && *turns == 2 && error.is_none());
                assert_eq!(denied, &vec!["Edit: /r/README.md".to_owned()]);
            }
            other => panic!("{other:?}"),
        }
        assert!((chat.cost() - 0.1007).abs() < 1e-9);
        chat.finished(None);
        assert_eq!(chat.run, Run::Idle);
        assert_eq!(chat.items.len(), 4, "a clean exit adds nothing");
    }

    #[test]
    fn exits_and_errors() {
        let mut chat = Chat { run: Run::Starting, ..Default::default() };
        chat.finished(Some("Claude Code isn't installed".into()));
        assert!(matches!(chat.items.last(), Some(Item::Problem(p)) if p.contains("installed")));
        let mut stopped = Chat { run: Run::Stopping, ..Default::default() };
        stopped.finished(Some("killed".into()));
        assert_eq!(stopped.items.last(), Some(&Item::Problem("Stopped.".into())));
        let e = parse_line(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"duration_ms":10,"num_turns":30,"session_id":"s"}"#);
        assert!(matches!(&e[0], Event::Done { error: Some(m), .. } if m.contains("too many turns")));
        assert_eq!(summarize("Grep", &serde_json::json!({"pattern": "fn main", "path": "src"})), "fn main  in src");
        assert_eq!(Mode::Edit.flag(), "acceptEdits");
        assert!(parse_line("not json").is_empty());
    }
}
