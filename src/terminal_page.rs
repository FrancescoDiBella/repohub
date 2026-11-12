//! The Terminal tab: a shell in the repository's local clone, with one-click commands.
use rune::{Color, El, Family, TextStyle, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextInput};

use crate::terminal::{self, Screen};
use crate::ui::{self, panel};
use crate::{App, clones_shim, icons};

const FONT: TextStyle = TextStyle { family: Family::Mono, ..TextStyle::new(13.0, 400.0) };

fn web_only(k: &Kit) -> El {
    panel(k).padding(24.0).gap(10.0)
        .child(row().gap(10.0).center_y().child(ui::ico(&icons::TERMINAL, 18.0, k.c.icon_muted))
            .child(text("The terminal needs the desktop app").font(k.ty.body_medium).color(k.c.text_strong)))
        .child(text("A web page can't start programs on your computer, so the terminal (and running claude, git and your other tools in the repository's folder) is available in the desktop app: cargo run --release.")
            .font(k.ty.ui).color(k.c.text_muted).wrap())
}

pub fn terminal_tab(s: &App, k: &Kit, i: usize, dark: bool) -> El {
    if cfg!(target_arch = "wasm32") {
        return web_only(k);
    }
    let r = &s.data.repos[i];
    let term = s.terms.get(&r.full_name);
    let known = clones_shim::find(&r.full_name);
    let mut out = col().w_full().gap(10.0);
    if let Some(e) = &s.term_error {
        out = out.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap());
    }
    match Panel::of(term, known) {
        Panel::Shell(t) => out.child(toolbar(s, k, i, &t.cwd.display().to_string(), t.is_running())).child(view(s, k, i, t, dark)),
        Panel::Found(dir) => {
            let d = dir.clone();
            out.child(panel(k).padding(20.0).gap(12.0)
                .child(row().gap(10.0).center_y().child(ui::ico(&icons::TERMINAL, 18.0, k.c.icon_muted))
                    .child(text("Open a terminal in this repository").font(k.ty.body_medium).color(k.c.text_strong)))
                .child(row().gap(8.0).center_y().child(ui::ico(&icons::FOLDER, 14.0, k.c.info.icon))
                    .child(text(dir.display().to_string()).font(k.ty.mono).color(k.c.text_base)))
                .child(row().gap(8.0)
                    .child(Button::new("Open terminal").icon(&icons::TERMINAL).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.start_terminal(i, d.clone(), None)).build(k).key("open-terminal"))
                    .children(s.assist.shown().map(|(n, h)| Button::new(format!("Open with {}", h.name)).icon(&icons::SPARK).control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.run_harness(i, n)).build(k).key(format!("open-harness-{n}"))))))
                .child(pick_folder(s, k, i, true))
        }
        Panel::Missing => out.child(pick_folder(s, k, i, false)),
    }
}

/// What the Terminal tab shows for a repository.
///
/// ```text
/// Missing ──type a folder / clone──▶ Shell (running) ──the shell exits──▶ Shell (exited) ──Restart──▶ Shell (running)
/// Found ──Open terminal──▶ Shell (running)
/// ```
enum Panel<'a> {
    /// A shell was started here (it may have exited since: `Term::is_running`).
    Shell(&'a terminal::Term),
    /// No shell yet, but the local clone is known.
    Found(std::path::PathBuf),
    /// No local clone is known: ask for the folder, or offer to clone.
    Missing,
}

impl<'a> Panel<'a> {
    fn of(term: Option<&'a terminal::Term>, clone: Option<std::path::PathBuf>) -> Panel<'a> {
        match (term, clone) {
            (Some(t), _) => Panel::Shell(t),
            (None, Some(dir)) => Panel::Found(dir),
            (None, None) => Panel::Missing,
        }
    }
}

/// Choosing the local folder, or cloning the repository.
pub(crate) fn pick_folder(s: &App, k: &Kit, i: usize, found: bool) -> El {
    let r = &s.data.repos[i];
    let field = TextInput::new(&s.clone_path).placeholder("Folder of your local copy, e.g. ~/Developer/repohub")
        .on_edit(|s: &mut App| &mut s.clone_path).on_submit(move |s: &mut App, _| s.use_folder(i))
        .build(k).key("clone-path").grow(1.0).min_w(260.0);
    let use_it = Button::new("Use this folder").control_size(ControlSize::Small).disabled(s.clone_path.text.trim().is_empty())
        .on_press(move |s: &mut App| s.use_folder(i)).build(k).key("use-folder");
    let parent = clones_shim::default_parent();
    let clone = Button::new(format!("Clone into {}", parent.join(&r.name).display())).icon(&icons::BRANCH).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.clone_repo(i)).build(k).key("clone-repo");
    let mut p = panel(k).padding(20.0).gap(12.0);
    if !found {
        p = p.child(text("No local copy found").font(k.ty.body_medium).color(k.c.text_strong))
            .child(text("RepoHub looks for a checkout named after the repository in REPOHUB_REPOS_DIR, or in ~/Developer, ~/Projects, ~/code, ~/src, ~/GitHub and ~/repos. Pick the folder, or clone the repository.")
                .font(k.ty.small).color(k.c.text_muted).wrap());
    } else {
        p = p.child(text("Use a different folder").font(k.ty.ui_medium).color(k.c.text_base));
    }
    p.child(row().w_full().gap(8.0).center_y().flex_wrap().child(field).child(use_it))
        .child(if found { el() } else { row().child(clone) })
}

fn toolbar(s: &App, k: &Kit, i: usize, cwd: &str, running: bool) -> El {
    let mut cmds = row().gap(6.0).center_y().flex_wrap();
    for (n, h) in s.assist.shown() {
        cmds = cmds.child(Button::new(h.name.clone()).icon(&icons::SPARK).control_size(ControlSize::Small)
            .variant(ButtonVariant::Submit).help(h.command.clone()).disabled(!running)
            .on_press(move |s: &mut App| s.run_harness(i, n)).build(k).key(format!("harness-{n}")));
    }
    for c in s.cfg.terminal_commands.iter().filter(|c| s.assist.allows(c)) {
        let line = format!("{c}\r");
        let icon = if c.starts_with("claude") { &icons::SPARK } else { &icons::PLAY };
        cmds = cmds.child(Button::new(c.clone()).icon(icon).control_size(ControlSize::Small)
            .variant(if c.starts_with("claude") { ButtonVariant::Submit } else { ButtonVariant::Neutral })
            .disabled(!running)
            .on_press(move |s: &mut App| s.term_type(i, &line)).build(k).key(format!("cmd-{c}")));
    }
    let dir = cwd.to_owned();
    let restart = Button::new(if running { "Restart" } else { "Start again" }).icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| {
            if let Some(r) = s.data.repos.get(i) {
                s.terms.remove(&r.full_name);
            }
            s.start_terminal(i, std::path::PathBuf::from(&dir), None);
        }).build(k).key("term-restart");
    let folder = format!("file://{}", cwd.replace('%', "%25").replace(' ', "%20").replace('#', "%23").replace('\\', "/"));
    let copy = Button::new("Copy screen").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press_tx(move |s: &mut App, tx| {
            if let Some(t) = s.data.repos.get(i).and_then(|r| s.terms.get(&r.full_name)) {
                tx.copy(t.text());
            }
        }).build(k).key("term-copy");
    let open = Button::new("Open folder").icon(&icons::FOLDER).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&folder); }).build(k).key("term-folder");
    col().w_full().gap(8.0)
        .child(row().w_full().gap(10.0).center_y()
            .child(ui::ico(&icons::FOLDER, 14.0, k.c.info.icon))
            .child(text(cwd.to_owned()).font(k.ty.mono).color(k.c.text_base).grow(1.0).shrink().max_lines(1))
            .child(if running { ui::pill(k, "Running", k.c.success.icon) } else { ui::pill(k, "Exited", k.c.text_faint) })
            .child(copy)
            .child(open)
            .child(restart))
        .child(cmds)
}

fn view(s: &App, k: &Kit, i: usize, t: &terminal::Term, dark: bool) -> El {
    let screen: Screen = t.screen(dark);
    let [_, lh] = s.term_cell;
    let (bg, fg) = if dark { (rune::color::hex("#0d1117").unwrap_or(Color::BLACK), rune::color::hex("#e6edf3").unwrap_or(Color::WHITE)) } else { (Color::WHITE, rune::color::hex("#1f2328").unwrap_or(Color::BLACK)) };
    let mut lines = col().w_full().gap(0.0);
    for runs in &screen.rows {
        let mut line = row().h(lh).w_full().clip_rounded(0.0);
        for run in runs {
            let mut e = text(run.text.clone()).font(if run.bold { TextStyle { weight: 650.0, ..FONT } } else { FONT }).color(run.fg.unwrap_or(fg)).h(lh);
            if let Some(b) = run.bg {
                e = e.fill(b);
            }
            line = line.child(e);
        }
        lines = lines.child(line);
    }
    // An invisible probe measures the font's cell size, so rows and columns match the shell's.
    let probe = text("0123456789").font(FONT).opacity(0.0).absolute(0.0, 0.0)
        .on_layout_tx(|s: &mut App, [_, _, w, h], _| {
            let cell = [w / 10.0, h.max(12.0)];
            if (cell[0] - s.term_cell[0]).abs() > 0.05 || (cell[1] - s.term_cell[1]).abs() > 0.05 {
                s.term_cell = cell;
            }
        });
    let mut area = col().key("terminal").w_full().h(520.0).padding(10.0).radius(k.radius.lg).fill(bg).border(1.0, k.c.border_weak)
        .clip_rounded(k.radius.lg).focusable(true).role("textbox").label("Terminal")
        .child(probe)
        .child(lines)
        .on_layout_tx(move |s: &mut App, [_, _, w, h], _| {
            let [cw, lh] = s.term_cell;
            let grid = (((h - 20.0) / lh).floor().max(2.0) as u16, ((w - 20.0) / cw).floor().max(10.0) as u16);
            if s.term_grid != grid {
                s.term_grid = grid;
            }
            if let Some(t) = s.data.repos.get(i).and_then(|r| s.terms.get_mut(&r.full_name)) {
                t.resize(grid.0, grid.1);
            }
        })
        .on_key(move |s: &mut App, ev, _| {
            let Some(bytes) = terminal::key_bytes(ev) else { return false };
            s.term_type_bytes(i, &bytes);
            true
        })
        .on_wheel(move |s: &mut App, e, _| {
            let lh = s.term_cell[1];
            if let Some(t) = s.data.repos.get(i).and_then(|r| s.terms.get_mut(&r.full_name)) {
                t.scroll_by((-e.dy / lh).round() as i32);
            }
            true
        });
    if screen.scrolled {
        area = area.child(text("Scrolled back · type to return").font(k.ty.text_small).color(k.c.text_muted)
            .px(8.0).py(3.0).radius(8.0).fill(k.c.bg_base).pin(None, Some(8.0), Some(12.0), None));
    }
    col().w_full().gap(6.0)
        .child(area)
        .child(text("Click the terminal to type. Paste with Ctrl/⌘+V; Ctrl+C stops the running command; Copy screen copies what is shown.").font(k.ty.text_small).color(k.c.text_faint))
}
