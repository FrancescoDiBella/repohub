//! Drawings written or deleted while another write runs, a merge question with its diff, stars, names as GitHub spells them.

use crate::*;
use crate::board::{Element, Kind};
use crate::state::Save;

fn app() -> App {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data.repos[0].full_name = "owner/repo".into();
    s.data.repos[0].branch = "main".into();
    // With an account in the environment, `init` waits to reopen the place saved last time
    // (by another test, here): these tests start from the page they set.
    s.pending_place = None;
    s
}

/// A drawing's JSON with one rectangle called `tag`.
fn drawing(tag: &str) -> String {
    let mut b = board::Board::new("owner/repo", "main", "x");
    b.elements.push(Element {
        id: tag.into(), kind: Kind::Rectangle, x: 1.0, y: 1.0, w: 10.0, h: 10.0, points: vec![],
        stroke: "#000".into(), fill: None, width: 2.0, text: String::new(), font: 20.0, seed: 1,
    });
    b.save_json()
}

fn id(s: &App, name: &str) -> (String, String, String) {
    let (full, branch) = s.notes_branch(0).unwrap();
    (full, branch, name.into())
}

#[test]
fn reopening_during_a_clean_write_shows_what_is_being_written() {
    let mut s = app();
    let id = id(&s, "Arch");
    // A write of the newest content is running, nothing queued behind it (the board was
    // closed while Saving { dirty: false }).
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("newest")));
    s.open_board(0, "Arch".into(), false);
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.save, Save::Saving { dirty: false }, "not Loading from the older file");
    assert!(b.json().contains("newest"));
    // The write finishes: the board is Saved, with the same content.
    s.apply_note_op(s.load_gen, id, Ok(()));
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.save, Save::Saved);
    assert!(b.json().contains("newest"));
}

#[test]
fn new_with_the_same_name_keeps_the_queued_edits() {
    let mut s = app();
    let id = id(&s, "Plan");
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
    s.pending_note_saves.insert(id.clone(), NoteOp::Save(drawing("second")));
    // The list doesn't show "Plan" yet, so the user types it again and presses New.
    s.open_board(0, "Plan".into(), true);
    let b = s.board.as_ref().unwrap();
    assert!(b.json().contains("second"), "opened on the newest snapshot, not a blank board");
    assert_eq!(b.save, Save::Saving { dirty: true }, "the snapshot is written after the running write");
    assert!(!s.pending_note_saves.contains_key(&id));
    // The running write ends; the open board's newest content is written next (no handle in
    // tests, so the write isn't sent, but the board moves on to writing it).
    s.apply_note_op(s.load_gen, id, Ok(()));
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: false });
    assert_eq!(s.note_log, [format!("save Plan: {}", s.board.as_ref().unwrap().json().len())]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_account_switch_keeps_queued_drawing_writes() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = app();
    let id = id(&s, "Arch");
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
    let second = drawing("second");
    s.pending_note_saves.insert(id.clone(), NoteOp::Save(second.clone()));
    let before = s.load_gen;
    s.reset_account_state();
    assert!(s.load_gen != before);
    assert!(s.saving_notes.contains_key(&id) && s.pending_note_saves.contains_key(&id), "nothing is dropped");
    // The old write's result arrives after the switch: the queued snapshot is still written.
    s.apply_note_op(before, id.clone(), Ok(()));
    assert!(!s.pending_note_saves.contains_key(&id));
    assert_eq!(s.note_log, [format!("save Arch: {}", second.len())], "the queued snapshot was written");
}

#[test]
fn deleting_during_a_write_waits_for_it_and_wins() {
    let mut s = app();
    let id = id(&s, "Old");
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
    s.pending_note_saves.insert(id.clone(), NoteOp::Save(drawing("second")));
    s.delete_note(0, "Old".into());
    assert_eq!(s.pending_note_saves.get(&id), Some(&NoteOp::Delete), "the queued save is replaced by the delete");
    // The running write finishes: the delete runs next and nothing writes the drawing again.
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    assert!(!s.pending_note_saves.contains_key(&id));
    assert_eq!(s.note_log, ["delete Old"], "only the delete runs, no write of the old snapshot");
}

#[test]
fn drawing_while_it_is_being_deleted_is_written_after_the_delete() {
    let mut s = app();
    let id = id(&s, "Old");
    s.saving_notes.insert(id.clone(), NoteOp::Delete);
    s.open_board(0, "Old".into(), true);
    assert!(s.board.as_ref().unwrap().elements.is_empty(), "opens empty");
    let mut d = board::Board::new("owner/repo", "main", "x");
    d.load_json(&drawing("new")).unwrap();
    s.board.as_mut().unwrap().elements.push(d.elements.remove(0));
    s.save_board();
    assert!(s.note_log.is_empty(), "nothing is written alongside the running delete");
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });
    s.apply_note_op(s.load_gen, id, Ok(()));
    assert_eq!(s.note_log.len(), 1);
    assert!(s.note_log[0].starts_with("save Old"), "the new drawing is written once the delete ends");
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: false });
}

#[test]
fn reopening_says_a_queued_delete_is_cancelled() {
    let mut s = app();
    let id = id(&s, "Old");
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("first")));
    s.pending_note_saves.insert(id.clone(), NoteOp::Delete);
    s.open_board(0, "Old".into(), false);
    assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("won't be deleted")));
    s.apply_note_op(s.load_gen, id, Ok(()));
    assert!(s.note_log.iter().all(|l| !l.starts_with("delete")));
}

#[test]
fn a_failed_write_on_the_open_drawing_shows_once() {
    let mut s = app();
    let id = id(&s, "Arch");
    s.open_board(0, "Arch".into(), true);
    s.board.as_mut().unwrap().save = Save::Saving { dirty: false };
    s.saving_notes.insert(id.clone(), NoteOp::Save(drawing("x")));
    s.apply_note_op(s.load_gen, id, Err("disk full".into()));
    assert!(matches!(s.board.as_ref().unwrap().save, Save::Failed(_)));
    assert!(s.toast.is_none(), "the board shows the failure; no toast on top");
}

#[test]
fn a_merge_confirmation_closes_with_its_diff() {
    let mut s = app();
    s.open_pull_diff("owner/repo".into(), 14);
    s.ask(crate::actions::merge_ask("owner/repo", 14));
    s.open_pull_diff("owner/repo".into(), 9);
    assert_eq!(s.asking, None, "another diff: the old question goes");
    s.ask(crate::actions::merge_ask("owner/repo", 9));
    s.close_pull_diff();
    assert_eq!(s.asking, None);
}

#[test]
fn the_page_keeps_the_name_as_github_spells_it_now() {
    let mut s = app();
    s.page = Page::Repo("OWNER/repo".into());
    let mut next = s.data.clone();
    next.repos[0].full_name = "owner/repo".into();
    let epoch = s.load_gen;
    s.apply(epoch, Ok(next));
    assert_eq!(s.page, Page::Repo("owner/repo".into()));
}

#[test]
fn toggle_star_respects_allow_writes() {
    let mut s = app();
    let mut cfg = (*s.cfg).clone();
    cfg.allow_writes = false;
    s.cfg = std::sync::Arc::new(cfg);

    let initial_starred = s.data.repos[0].starred;
    let initial_stars = s.data.repos[0].stars;

    s.toggle_star(0);

    assert_eq!(s.data.repos[0].starred, initial_starred, "starred status must not change");
    assert_eq!(s.data.repos[0].stars, initial_stars, "star count must not change");
    assert!(s.toast.as_ref().is_some_and(|t| t.text.contains("Writes are disabled on this server")));
}
