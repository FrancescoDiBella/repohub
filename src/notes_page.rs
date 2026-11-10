//! The Notes tab: drawings for a repository, one list per branch, and the board editor.
use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, TextInput};

use crate::board::{self, Board, SaveState, Tool};
use crate::notes::{self, valid_name};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, icons};

fn unavailable(k: &Kit) -> El {
    panel(k).padding(24.0).gap(10.0)
        .child(row().gap(10.0).center_y().child(ui::ico(&icons::PEN, 18.0, k.c.icon_muted))
            .child(text("Notes need the RepoHub server").font(k.ty.body_medium).color(k.c.text_strong)))
        .child(text("Notes are drawings (diagrams, sketches, annotations) saved on your computer for each repository and branch. They work in the desktop app and when RepoHub runs with Docker, not on the static GitHub Pages site, which has nowhere to store them.")
            .font(k.ty.ui).color(k.c.text_muted).wrap())
}

pub fn notes_tab(s: &App, k: &Kit, i: usize) -> El {
    if !s.cfg.notes {
        return unavailable(k);
    }
    let Some((full, branch)) = s.notes_branch(i) else { return col() };
    if let Some(b) = s.board.as_ref().filter(|b| b.repo == full) {
        return editor(s, k, i, b);
    }
    // Branch picker: the default branch first, then the others by name.
    let mut picker = row().w_full().gap(6.0).center_y().flex_wrap()
        .child(text("Branch").font(k.ty.small_medium).color(k.c.text_muted).pr(4.0));
    match s.branches.get(&full) {
        Some(Ok(list)) if !list.is_empty() => {
            for b in list.iter().take(40) {
                let on = *b == branch;
                let name = b.clone();
                let chip = Button::new(b.clone()).icon(&icons::BRANCH).control_size(ControlSize::Small)
                    .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
                    .on_press(move |s: &mut App| s.pick_branch(i, name.clone()))
                    .build(k).key(format!("branch-{b}"));
                picker = picker.child(chip);
            }
            if list.len() > 40 {
                picker = picker.child(text(format!("+{} more", list.len() - 40)).font(k.ty.small).color(k.c.text_faint));
            }
        }
        Some(Err(e)) => picker = picker.child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap()),
        _ => picker = picker.child(text(branch.clone()).font(k.ty.small_medium).color(k.c.text_base))
            .child(text("Loading branches…").font(k.ty.small).color(k.c.text_faint)),
    }
    // New drawing: a name, then the editor opens in a new tab.
    let name_ok = valid_name(s.note_name.text.trim());
    let exists = |n: &str| s.notes.get(&(full.clone(), branch.clone())).is_some_and(|v| v.items.iter().any(|x| x.name == n));
    let taken = exists(s.note_name.text.trim());
    let create = Button::new(if taken { "Open drawing" } else { "New drawing" }).icon(&icons::PLUS).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
        .disabled(!name_ok)
        .on_press(move |s: &mut App| {
            let name = s.note_name.text.trim().to_owned();
            if !valid_name(&name) {
                return;
            }
            s.note_name.clear();
            s.open_board(i, name, !taken);
        })
        .build(k).key("new-note");
    let field = TextInput::new(&s.note_name).placeholder("Name of the new drawing, e.g. Architecture")
        .on_edit(|s: &mut App| &mut s.note_name).build(k).key("note-name").grow(1.0).min_w(220.0);
    let hint = if s.note_name.text.trim().is_empty() || name_ok { "Letters, digits, spaces and _ - . ( ), up to 80 characters." } else { "That name isn't allowed: use letters, digits, spaces and _ - . ( )." };
    let new_panel = panel(k).padding(16.0).gap(10.0)
        .child(row().w_full().gap(10.0).center_y().flex_wrap().child(field).child(create))
        .child(text(hint).font(k.ty.text_small).color(if name_ok || s.note_name.text.trim().is_empty() { k.c.text_faint } else { k.c.danger.icon }));
    // The list for this branch.
    let view = s.notes.get(&(full.clone(), branch.clone()));
    let refresh = Button::new("Refresh").icon(&icons::REFRESH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
        .disabled(view.is_some_and(|v| v.loading))
        .on_press(move |s: &mut App| s.load_notes(i)).build(k).key("notes-refresh");
    let mut list = panel(k).child(panel_head(k, &format!("Drawings on {branch}"), Some(refresh)));
    match view {
        None => list = list.child(rule(k)).child(col().w_full().padding(24.0).items_center().child(text("Loading…").font(k.ty.ui).color(k.c.text_muted))),
        Some(v) if v.loading && v.items.is_empty() => list = list.child(rule(k)).child(col().w_full().padding(24.0).items_center().child(text("Loading…").font(k.ty.ui).color(k.c.text_muted))),
        Some(v) => {
            if let Some(e) = &v.error {
                list = list.child(rule(k)).child(col().w_full().padding(16.0).child(text(e.clone()).font(k.ty.small).color(k.c.danger.icon).wrap()));
            }
            if v.items.is_empty() && v.error.is_none() {
                list = list.child(rule(k)).child(col().w_full().padding(28.0).gap(4.0).items_center()
                    .child(text("No drawings on this branch yet").font(k.ty.ui_medium).color(k.c.text_base))
                    .child(text("Name one above and press New drawing.").font(k.ty.small).color(k.c.text_muted)));
            }
            for n in &v.items {
                list = list.child(rule(k)).child(note_row(s, k, i, &full, &branch, n));
            }
        }
    }
    col().w_full().gap(12.0)
        .child(text("Drawings, diagrams and annotations for this repository, saved on this computer. Each branch has its own set.").font(k.ty.small).color(k.c.text_muted).wrap())
        .child(picker)
        .child(new_panel)
        .child(list)
}

fn note_row(s: &App, k: &Kit, i: usize, full: &str, branch: &str, n: &notes::Note) -> El {
    let _ = (full, branch);
    let open_name = n.name.clone();
    let open = Button::new("Open").icon(&icons::PEN).control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.open_board(i, open_name.clone(), false))
        .build(k).key(format!("open-note-{}", n.name));
    let confirming = s.confirm_delete.as_deref() == Some(n.name.as_str());
    let name = n.name.clone();
    let delete = if confirming {
        Button::new("Delete for good").variant(ButtonVariant::Danger).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.delete_note(i, name.clone()))
            .build(k).key(format!("confirm-delete-{}", n.name))
    } else {
        Button::new("Delete").icon(&icons::TRASH).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.confirm_delete = Some(name.clone()))
            .build(k).key(format!("delete-note-{}", n.name))
    };
    row().w_full().px(16.0).py(10.0).gap(12.0).center_y().hover(k.c.hover)
        .child(ui::ico(&icons::PEN, 16.0, k.c.icon_muted))
        .child(col().grow(1.0).shrink().gap(2.0)
            .child(text(n.name.clone()).font(k.ty.ui_medium).color(k.c.text_base))
            .child(text(format!("Edited {} · {}", n.updated, notes::size(n.size))).font(k.ty.text_small).color(k.c.text_muted)))
        .child(open)
        .child(delete)
}

fn tool_icon(t: Tool) -> &'static rune::Icon {
    match t {
        Tool::Select => &icons::CURSOR,
        Tool::Rectangle => &icons::SQUARE,
        Tool::Ellipse => &icons::CIRCLE,
        Tool::Diamond => &icons::DIAMOND,
        Tool::Arrow => &icons::ARROW,
        Tool::Line => &icons::LINE,
        Tool::Pen => &icons::PEN,
        Tool::Text => &icons::TEXT,
    }
}

/// The board editor: toolbar, canvas and status.
fn editor(s: &App, k: &Kit, i: usize, b: &Board) -> El {
    let dark = s.prefs[0] || k.c.page.r + k.c.page.g + k.c.page.b < 1.0;
    let ib = |icon: &'static rune::Icon, label: &str, on: bool, key: String| {
        IconButton::new(icon).variant(if on { IconButtonVariant::Contrast } else { IconButtonVariant::GhostMuted })
            .label(label.to_owned()).help(label.to_owned()).build(k).key(key)
    };
    let mut tools = row().gap(2.0).center_y();
    for t in Tool::ALL {
        tools = tools.child(ib(tool_icon(t), t.label(), b.tool == t, format!("tool-{}", t.label()))
            .on_press(move |s: &mut App| {
                if let Some(b) = s.board.as_mut() {
                    b.tool = t;
                    b.text_at = None;
                    if t != Tool::Select {
                        b.selected = None;
                    }
                }
            }));
    }
    let mut colors = row().gap(5.0).center_y();
    for (ci, c) in board::STROKES.iter().enumerate() {
        let on = b.color == ci;
        let sw = el().size(20.0, 20.0).radius(10.0).fill(rune::color::hex(c).unwrap_or(k.c.text_base))
            .border(if on { 2.5 } else { 1.0 }, if on { k.c.text_strong } else { k.c.border_weak })
            .role("radio").checked(on).label(format!("Colour {}", ci + 1)).focusable(true)
            .on_press(move |s: &mut App| {
                if let Some(b) = s.board.as_mut() {
                    b.color = ci;
                    // Recolour the selection too, as Excalidraw does.
                    if let Some(e) = b.selected.and_then(|i| b.elements.get_mut(i)) {
                        e.stroke = board::STROKES[ci].to_owned();
                        if e.fill.is_some() {
                            e.fill = Some(board::FILLS[ci].to_owned());
                        }
                        s.save_board();
                    }
                }
            }).key(format!("color-{ci}"));
        colors = colors.child(sw);
    }
    let toggles = row().gap(2.0).center_y()
        .child(ib(&icons::FILL, "Fill new shapes", b.filled, "fill".into()).on_press(|s: &mut App| if let Some(b) = s.board.as_mut() { b.filled = !b.filled; }))
        .child(ib(&icons::BOLD, "Bold strokes and text", b.bold, "bold".into()).on_press(|s: &mut App| if let Some(b) = s.board.as_mut() { b.bold = !b.bold; }));
    let edits = row().gap(2.0).center_y()
        .child(IconButton::new(&icons::UNDO).variant(IconButtonVariant::GhostMuted).label("Undo").help("Undo (Ctrl/⌘ Z)").disabled(!b.can_undo())
            .on_press(|s: &mut App| if s.board.as_mut().is_some_and(|b| b.undo()) { s.save_board(); }).build(k).key("undo"))
        .child(IconButton::new(&icons::REDO).variant(IconButtonVariant::GhostMuted).label("Redo").help("Redo (Ctrl/⌘ Shift Z)").disabled(!b.can_redo())
            .on_press(|s: &mut App| if s.board.as_mut().is_some_and(|b| b.redo()) { s.save_board(); }).build(k).key("redo"))
        .child(IconButton::new(&icons::TRASH).variant(IconButtonVariant::GhostMuted).label("Delete selection").help("Delete the selection (Delete)").disabled(b.selected.is_none())
            .on_press(|s: &mut App| if s.board.as_mut().is_some_and(|b| b.delete_selected()) { s.save_board(); }).build(k).key("delete-element"));
    let sep = || el().w(1.0).h(22.0).fill(k.c.border_weak);
    let toolbar = row().w_full().px(10.0).py(6.0).gap(10.0).center_y().flex_wrap().radius(k.radius.lg)
        .fill(k.c.bg_base).border(1.0, k.c.border_weak)
        .child(tools).child(sep()).child(colors).child(sep()).child(toggles).child(sep()).child(edits);
    let (status, tint) = match &b.save {
        SaveState::Loading => ("Loading…".to_owned(), k.c.text_muted),
        SaveState::Saved => ("Saved on this computer".to_owned(), k.c.success.icon),
        SaveState::Saving => ("Saving…".to_owned(), k.c.text_muted),
        SaveState::Error(e) => (format!("Not saved: {e}"), k.c.danger.icon),
    };
    let head = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(Button::new("Drawings").icon(&icons::BACK).variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
            .on_press(move |s: &mut App| s.close_board(i)).build(k).key("close-board"))
        .child(ui::ico(&icons::PEN, 16.0, k.c.icon_muted))
        .child(text(b.name.clone()).font(k.ty.body_medium).color(k.c.text_strong))
        .child(text(b.branch.clone()).font(k.ty.text_small).color(k.c.text_muted).px(8.0).py(2.0).radius(10.0).border(1.0, k.c.border_weak))
        .child(el().grow(1.0))
        .child(text(status).font(k.ty.text_small).color(tint));
    let scale = rune::memo::layout_size("board").map(|[w, _]| w / board::WORLD[0]).unwrap_or(0.7).max(0.2);
    let canvas = board::canvas_view(
        b,
        dark,
        k.c.text_accent,
        scale,
        |s: &mut App, d, _| {
            let changed = s.board.as_mut().is_some_and(|b| b.on_drag(board::Gesture::from(&*d)));
            if changed {
                s.save_board();
            }
        },
        |s: &mut App| &mut s.board.as_mut().expect("board open").text_edit,
        |s: &mut App, _| {
            if s.board.as_mut().is_some_and(|b| b.commit_text()) {
                s.save_board();
            }
        },
    );
    let hint = match b.tool {
        Tool::Select => "Click a shape to select it, drag to move it. Delete removes it.",
        Tool::Text => "Click where the text goes, type, then press Enter.",
        Tool::Pen => "Drag to draw freehand.",
        _ => "Drag on the canvas to draw.",
    };
    col().w_full().gap(10.0)
        .child(head)
        .child(toolbar)
        .child(canvas.border(1.0, k.c.border_weak))
        .child(row().w_full().justify_between().gap(12.0).flex_wrap()
            .child(text(hint).font(k.ty.text_small).color(k.c.text_muted).wrap().shrink())
            .child(text("Saved as .excalidraw: the file also opens on excalidraw.com.").font(k.ty.text_small).color(k.c.text_faint).wrap().shrink()))
}
