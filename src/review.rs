//! Comments on the lines of a pull request's diff: reading the ones on GitHub, and writing one.
//!
//! GitHub keeps them under `/pulls/{n}/comments`, each on a file's line, on the new side
//! (`RIGHT`) or the old one (`LEFT`). The diff shows them under their line (`DiffView`
//! annotations); the "+" by a line number opens a box to write one there.
use rune::{El, TextEdit, col, row, text};
use rune_kit::diff::{Annotation, FileDiff, LineRange, LineRef, Side};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, TextArea};

use crate::{App, actions, model, ui};

/// One comment on a line, as GitHub has it.
#[derive(Clone, Debug, PartialEq)]
pub struct LineComment {
    pub path: String,
    pub line: u32,
    pub side: Side,
    pub author: String,
    pub body: String,
    pub at: i64,
}

/// A comment being written, on the lines `start`..=`end` of file `file`.
pub struct Draft {
    pub file: usize,
    pub path: String,
    pub start: LineRef,
    pub end: LineRef,
    pub text: TextEdit,
    pub sending: bool,
}

/// What the diff on screen knows about the pull request's line comments.
#[derive(Default)]
pub struct Review {
    /// The commit the diff shows; GitHub needs it to place a new comment.
    pub head: String,
    pub comments: Vec<LineComment>,
    pub draft: Option<Draft>,
}

/// The path a line of `f` is known by on `side`.
fn path_on(f: &FileDiff, side: Side) -> Option<&str> {
    match side {
        Side::New => f.new_path.as_deref().or(f.old_path.as_deref()),
        Side::Old => f.old_path.as_deref().or(f.new_path.as_deref()),
    }
}

/// GitHub's word for a side.
pub fn side_word(side: Side) -> &'static str {
    match side {
        Side::New => "RIGHT",
        Side::Old => "LEFT",
    }
}

impl Review {
    /// Where each comment and the draft show in `files`: under their line. A comment on a file
    /// or a line the diff doesn't show is left out (it's on GitHub).
    pub fn annotations(&self, files: &[FileDiff]) -> Vec<Annotation> {
        let mut out = Vec::new();
        for (n, c) in self.comments.iter().enumerate() {
            if let Some(file) = files.iter().position(|f| path_on(f, c.side) == Some(c.path.as_str())) {
                out.push(Annotation::new(file, LineRef::new(c.side, c.line), format!("c{n}")));
            }
        }
        if let Some(d) = &self.draft {
            out.push(Annotation::new(d.file, d.end, "draft"));
        }
        out
    }

    /// Opens a box to write a comment on `range` of `files`.
    pub fn start(&mut self, files: &[FileDiff], range: LineRange) -> bool {
        let Some(path) = files.get(range.file).and_then(|f| path_on(f, range.end.side)) else { return false };
        // GitHub places a comment on lines of one side; a range across both keeps its last line.
        let start = if range.start.side == range.end.side { range.start } else { range.end };
        let (start, end) = if start.line <= range.end.line { (start, range.end) } else { (range.end, start) };
        let text = self.draft.take().map(|d| d.text).unwrap_or_else(|| TextEdit::new(""));
        self.draft = Some(Draft { file: range.file, path: path.to_owned(), start, end, text, sending: false });
        true
    }
}

/// The widget under a line: a comment, or the box to write one.
pub fn annotation(s: &App, k: &Kit, a: &Annotation) -> El {
    let Some(r) = s.pull_review.as_ref() else { return col() };
    if a.id == "draft" {
        let Some(d) = &r.draft else { return col() };
        let lines = if d.start == d.end { format!("line {}", d.end.line) } else { format!("lines {}–{}", d.start.line, d.end.line) };
        let blocked = actions::blocked(s);
        let empty = d.text.text.trim().is_empty();
        let mut box_ = col().w_full().px(12.0).py(10.0).gap(8.0).fill(k.c.layer_01).border(1.0, k.c.border_base).radius(k.radius.md).key("line-draft")
            .child(text(format!("Comment on {} {lines}", d.path)).font(k.ty.small_medium).color(k.c.text_muted))
            .child(TextArea::new(&d.text).placeholder("Write a comment (Markdown)").label("Comment on these lines").auto_grow(2, 10)
                .on_edit(|s: &mut App| &mut s.pull_review.as_mut().expect("diff open").draft.as_mut().expect("draft open").text).build(k).key("line-draft-text").w_full())
            .child(row().gap(8.0)
                .child(Button::new(if d.sending { "Sending…" } else { "Comment" }).variant(ButtonVariant::Submit).control_size(ControlSize::Small)
                    .disabled(empty || d.sending || blocked.is_some()).on_press(|s: &mut App| s.send_line_comment()).build(k).key("line-draft-send"))
                .child(Button::new("Cancel").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                    .on_press(|s: &mut App| s.cancel_line_comment()).build(k).key("line-draft-cancel")));
        if let Some(why) = blocked {
            box_ = box_.child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap());
        }
        return box_;
    }
    let Some(c) = a.id.strip_prefix('c').and_then(|n| n.parse::<usize>().ok()).and_then(|n| r.comments.get(n)) else { return col() };
    col().w_full().px(12.0).py(10.0).gap(6.0).fill(k.c.layer_01).border(1.0, k.c.border_weak).radius(k.radius.md).key(a.id.clone())
        .child(row().gap(8.0).center_y()
            .child(ui::user(k, &c.author, 18.0))
            .child(text(c.author.clone()).font(k.ty.small_medium).color(k.c.text_strong))
            .child(text(model::fmt_ago(c.at, s.now)).font(k.ty.text_small).color(k.c.text_faint)))
        .child(crate::pages::markdown_view(k, &c.body, &format!("line-code-{}", a.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<FileDiff> {
        crate::diffs::pull_files(&crate::diffs::sample_pull_files())
    }

    #[test]
    fn comments_show_under_their_line_of_the_right_file() {
        let files = files();
        let r = Review {
            head: "abc".into(),
            comments: vec![
                LineComment { path: "src/bunny.js".into(), line: 3, side: Side::New, author: "a".into(), body: "x".into(), at: 0 },
                LineComment { path: "src/scene.js".into(), line: 11, side: Side::Old, author: "b".into(), body: "y".into(), at: 0 },
                LineComment { path: "gone.rs".into(), line: 1, side: Side::New, author: "c".into(), body: "z".into(), at: 0 },
            ],
            draft: None,
        };
        let a = r.annotations(&files);
        let bunny = files.iter().position(|f| f.new_path.as_deref() == Some("src/bunny.js")).unwrap();
        let scene = files.iter().position(|f| f.new_path.as_deref() == Some("src/scene.js")).unwrap();
        assert_eq!(a.len(), 2, "a comment on a file the diff doesn't show is left out");
        assert_eq!((a[0].file, a[0].at, a[0].id.as_str()), (bunny, LineRef::new(Side::New, 3), "c0"));
        assert_eq!((a[1].file, a[1].at), (scene, LineRef::new(Side::Old, 11)));
    }

    #[test]
    fn the_plus_opens_a_box_on_its_lines() {
        let files = files();
        let mut r = Review::default();
        let range = LineRange { file: 1, start: LineRef::new(Side::New, 4), end: LineRef::new(Side::New, 2) };
        assert!(r.start(&files, range));
        let d = r.draft.as_ref().unwrap();
        assert_eq!((d.start.line, d.end.line), (2, 4), "upward selections are put in order");
        assert_eq!(d.path, files[1].new_path.clone().unwrap());
        assert_eq!(r.annotations(&files).last().unwrap().id, "draft");
        // Lines on both sides: the comment goes on the last one.
        let mixed = LineRange { file: 0, start: LineRef::new(Side::Old, 11), end: LineRef::new(Side::New, 12) };
        assert!(r.start(&files, mixed));
        let d = r.draft.as_ref().unwrap();
        assert_eq!((d.start, d.end), (LineRef::new(Side::New, 12), LineRef::new(Side::New, 12)));
        assert!(!r.start(&files, LineRange::line(99, LineRef::new(Side::New, 1))), "no such file");
        assert_eq!((side_word(Side::New), side_word(Side::Old)), ("RIGHT", "LEFT"));
    }
}
