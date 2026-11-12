//! The Code tab's files from the local clone (desktop): a folder listing with each file's git
//! change, and a viewer for the open file. GitHub's list stays one click away.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::local::{Change, Content};
use crate::model::Repo;
use crate::state::Load;
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons};

/// Folder rows shown at most (the rest is counted).
const MAX_ROWS: usize = 400;

/// "This computer | GitHub", when a local clone is known.
pub fn source_switch(s: &App, k: &Kit) -> El {
    SegmentedControl::new(&["This computer", "GitHub"], Some(usize::from(s.files_from_github))).label("Files from").fit()
        .on_pick(|s: &mut App, i, _| s.files_from_github = i == 1).build(k).key("files-source")
}

fn change_color(k: &Kit, c: Change) -> rune::Color {
    match c {
        Change::Added | Change::New => k.c.success.icon,
        Change::Deleted | Change::Conflicted => k.c.danger.icon,
        Change::Modified | Change::Renamed => k.c.warning.icon,
    }
}

fn change_mark(k: &Kit, c: Change) -> El {
    text(c.letter()).font(k.ty.mono).color(change_color(k, c)).w(14.0).help(c.label().to_owned())
}

/// The files panel for the local clone of repository `i`.
pub fn files_panel(s: &App, k: &Kit, i: usize, r: &Repo) -> El {
    let refresh = Button::new("Refresh").icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .help("List the files again (after you changed them outside RepoHub)")
        .on_press(move |s: &mut App| s.load_local(i, true)).build(k).key("local-refresh");
    let head = row().gap(6.0).center_y().child(refresh).child(source_switch(s, k));
    let mut p = panel(k).key("local-files");
    let t = match s.local.get(&r.full_name) {
        None | Some(Load::Loading) => return p.child(panel_head(k, "Files", Some(head))).child(rule(k))
            .child(col().w_full().padding(24.0).items_center().child(text("Reading the local clone…").font(k.ty.ui).color(k.c.text_muted))),
        Some(Load::Failed(e)) => return p.child(panel_head(k, "Files", Some(head))).child(rule(k))
            .child(col().w_full().padding(16.0).child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap())),
        Some(Load::Ready(t)) => t,
    };
    let title = match (t.branch.is_empty(), t.changed()) {
        (true, _) => "Files on this computer".to_owned(),
        (false, 0) => format!("This computer · {}", t.branch),
        (false, n) => format!("This computer · {} · {n} changed", t.branch),
    };
    p = p.child(panel_head(k, &title, Some(head))).child(rule(k));
    // Where we are: the clone's folder, then each folder down to this one.
    let root_name = t.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.name.clone());
    let mut crumbs = row().w_full().px(16.0).py(8.0).gap(4.0).center_y().flex_wrap()
        .child(crumb(k, i, root_name, String::new()));
    let mut at = String::new();
    for part in s.local_dir.split('/').filter(|x| !x.is_empty()) {
        at = if at.is_empty() { part.to_owned() } else { format!("{at}/{part}") };
        crumbs = crumbs.child(text("/").font(k.ty.small).color(k.c.text_faint)).child(crumb(k, i, part.to_owned(), at.clone()));
    }
    p = p.child(crumbs).child(rule(k));
    let entries = t.entries(&s.local_dir);
    if !s.local_dir.is_empty() {
        let up = s.local_dir.rsplit_once('/').map(|(a, _)| a.to_owned()).unwrap_or_default();
        p = p.child(row().w_full().px(16.0).py(7.0).gap(10.0).center_y().hover(k.c.hover).press(k.c.pressed)
            .role("button").label("Up one folder").key("local-up")
            .on_press(move |s: &mut App| s.open_local(i, up.clone(), true))
            .child(ui::ico(&icons::FOLDER, 16.0, k.c.icon_faint))
            .child(text("..").font(k.ty.small).color(k.c.text_muted)))
            .child(rule(k));
    }
    if entries.is_empty() {
        p = p.child(col().w_full().padding(20.0).items_center().child(text("This folder is empty.").font(k.ty.small).color(k.c.text_muted)));
    }
    let open = s.local_file.as_ref().map(|(path, _)| path.as_str());
    let more = entries.len().saturating_sub(MAX_ROWS);
    for (n, e) in entries.into_iter().take(MAX_ROWS).enumerate() {
        if n > 0 {
            p = p.child(rule(k));
        }
        let selected = open == Some(e.path.as_str());
        let (path, dir) = (e.path.clone(), e.dir);
        let icon = if e.dir { &icons::FOLDER } else { rune_kit::file_icon(&e.name) };
        let mut line = row().w_full().px(16.0).py(7.0).gap(10.0).center_y().hover(k.c.hover).press(k.c.pressed)
            .role("button").label(if e.dir { format!("Open folder {}", e.name) } else { format!("Show {}", e.name) }).key(format!("local-{}", e.path))
            .on_press(move |s: &mut App| s.open_local(i, path.clone(), dir))
            .child(ui::ico(icon, 16.0, if e.dir { k.c.info.icon } else { k.c.icon_muted }))
            .child(text(e.name).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink().max_lines(1));
        if selected {
            line = line.fill(k.c.bg_accent.alpha(0.12));
        }
        if let Some(c) = e.change {
            line = line.child(change_mark(k, c));
        }
        p = p.child(line);
    }
    if more > 0 || t.cut {
        let note = if t.cut { format!("Showing the first {} files of a very large folder.", crate::local::MAX_FILES) } else { format!("{more} more not shown.") };
        p = p.child(rule(k)).child(text(note).font(k.ty.text_small).color(k.c.text_muted).px(16.0).py(8.0));
    }
    p
}

fn crumb(k: &Kit, i: usize, label: String, path: String) -> El {
    text(label.clone()).font(k.ty.small_medium).color(k.c.text_accent).px(4.0).py(2.0).radius(4.0).hover(k.c.hover)
        .role("link").label(format!("Open {label}")).key(format!("crumb-{path}"))
        .on_press(move |s: &mut App| s.open_local(i, path.clone(), true))
}

/// The open file: its text with syntax colours (Markdown rendered), or a note.
pub fn viewer(s: &App, k: &Kit, i: usize, r: &Repo) -> Option<El> {
    let (path, load) = s.local_file.as_ref()?;
    let name = path.rsplit('/').next().unwrap_or(path);
    let change = s.local.get(&r.full_name).and_then(Load::ready).and_then(|t| t.changes.get(path.as_str()).copied());
    let gh = format!("{}/blob/{}/{}", r.url, s.branch(i), path);
    let mut tools = row().gap(4.0).center_y();
    if let Some(c) = change {
        tools = tools.child(ui::pill(k, c.label(), change_color(k, c)));
    }
    if let Load::Ready(Content::Text { text: body, .. }) = load {
        let body = body.clone();
        tools = tools.child(Button::new("Copy").icon(&icons::COPY).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press_tx(move |s: &mut App, tx| { tx.copy(body.clone()); s.notify("File copied"); }).build(k).key("file-copy"));
    }
    tools = tools
        .child(Button::new("GitHub").icon(&icons::EXTERNAL).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .help("The committed version on GitHub").on_press_tx(move |_: &mut App, tx| { let _ = tx.open_url(&gh); }).build(k).key("file-github"))
        .child(rune_kit::IconButton::new(&icons::CROSS).variant(rune_kit::IconButtonVariant::GhostMuted).label("Close the file")
            .on_press(|s: &mut App| s.local_file = None).build(k).key("file-close"));
    let body = match load {
        Load::Loading => text("Reading…").font(k.ty.ui).color(k.c.text_muted),
        Load::Failed(e) => text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap(),
        Load::Ready(Content::Missing) => text("This file was deleted here and the deletion isn't committed yet.").font(k.ty.small).color(k.c.text_muted).wrap(),
        Load::Ready(Content::Binary(size)) => text(format!("A binary file ({}): it can't be shown as text.", size_label(*size))).font(k.ty.small).color(k.c.text_muted).wrap(),
        Load::Ready(Content::Text { text: body, lines, cut }) => {
            let md = if name.to_ascii_lowercase().ends_with(".md") || name.to_ascii_lowercase().ends_with(".markdown") {
                crate::markdown::prepare(body, &r.full_name, &s.branch(i))
            } else {
                // A fence longer than any run of backticks in the file keeps it whole.
                let ticks = "`".repeat(body.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2) + 1);
                format!("{ticks}{}\n{body}\n{ticks}\n", crate::local::language(name).unwrap_or(""))
            };
            let mut c = col().w_full().gap(8.0).child(crate::pages::readme_view(k, &md));
            if *cut {
                c = c.child(text(format!("Showing the first {lines} lines. Open the file in your editor or on GitHub for the rest.")).font(k.ty.text_small).color(k.c.text_muted).wrap());
            }
            c
        }
    };
    Some(panel(k).key("file-view")
        .child(panel_head(k, path, Some(tools)))
        .child(rule(k))
        .child(col().w_full().padding(if s.compact { 12.0 } else { 16.0 }).child(body)))
}

fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}
