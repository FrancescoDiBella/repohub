//! Drawings: one write at a time, saves waiting behind it, and late loads of another drawing.

use crate::*;
use crate::board::{Element, Kind};
use crate::state::Save;
use super::load_gen_tests::TempDataDir;

fn setup_app() -> App {
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data.repos[0].owner = "owner".into();
    s.data.repos[0].name = "repo".into();
    s.data.repos[0].full_name = "owner/repo".into();
    s.data.repos[0].branch = "main".into();
    s
}

#[test]
fn stale_open_board_load_does_not_overwrite_another_drawing() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();

    // 1. Open drawing A ("Arch").
    s.open_board(0, "Arch".into(), false);
    let id_a = (full.clone(), branch.clone(), "Arch".into());
    let epoch_a = s.load_gen;
    let board_epoch_a = s.board_load_gen;
    assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Arch"));
    assert_eq!(s.board.as_ref().map(|b| &b.save), Some(&Save::Loading));

    // 2. Before Arch finishes loading, user switches to drawing B ("Plan").
    s.open_board(0, "Plan".into(), false);
    let id_b = (full.clone(), branch.clone(), "Plan".into());
    let epoch_b = s.load_gen;
    let board_epoch_b = s.board_load_gen;
    assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Plan"));
    assert_ne!(board_epoch_a, board_epoch_b);

    // 3. Stale load result arrives for Arch with some elements.
    let arch_json = serde_json::json!({
        "type": "excalidraw", "version": 2,
        "elements": [{
            "id": "arch-rect", "type": "rectangle",
            "x": 10.0, "y": 10.0, "width": 50.0, "height": 50.0,
            "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
            "strokeWidth": 2.0, "seed": 1
        }]
    }).to_string();

    s.apply_board_load(epoch_a, board_epoch_a, id_a, Ok(Some(arch_json)));

    // Stale result for Arch MUST be ignored: Plan is still open and empty.
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.name, "Plan");
    assert!(b.elements.is_empty(), "Plan must not receive elements from Arch");
    assert_eq!(b.save, Save::Loading);

    // 4. Now valid load result for Plan arrives.
    let plan_json = serde_json::json!({
        "type": "excalidraw", "version": 2,
        "elements": [{
            "id": "plan-ellipse", "type": "ellipse",
            "x": 20.0, "y": 20.0, "width": 60.0, "height": 60.0,
            "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
            "strokeWidth": 2.0, "seed": 2
        }]
    }).to_string();

    s.apply_board_load(epoch_b, board_epoch_b, id_b, Ok(Some(plan_json)));
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.name, "Plan");
    assert_eq!(b.elements.len(), 1);
    assert_eq!(b.elements[0].id, "plan-ellipse");
    assert_eq!(b.save, Save::Saved);
}

#[test]
fn stale_open_board_load_does_not_overwrite_newly_created_drawing() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();

    // 1. Open drawing A ("Arch") to load.
    s.open_board(0, "Arch".into(), false);
    let id_a = (full.clone(), branch.clone(), "Arch".into());
    let epoch_a = s.load_gen;
    let board_epoch_a = s.board_load_gen;

    // 2. User creates a new drawing with the same name (new=true).
    s.open_board(0, "Arch".into(), true);
    assert_eq!(s.board.as_ref().map(|b| b.name.as_str()), Some("Arch"));
    assert_eq!(s.board.as_ref().map(|b| &b.save), Some(&Save::Saved));
    assert!(s.board.as_ref().unwrap().elements.is_empty());

    // 3. Stale load for Arch arrives.
    let arch_json = serde_json::json!({
        "type": "excalidraw", "version": 2,
        "elements": [{
            "id": "stale-1", "type": "rectangle",
            "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0,
            "points": [], "strokeColor": "#000", "backgroundColor": "transparent",
            "strokeWidth": 2.0, "seed": 1
        }]
    }).to_string();

    s.apply_board_load(epoch_a, board_epoch_a, id_a, Ok(Some(arch_json)));

    // The newly created drawing MUST remain clean.
    let b = s.board.as_ref().unwrap();
    assert!(b.elements.is_empty(), "New drawing must not be overwritten by stale load");
    assert_eq!(b.save, Save::Saved);
}

#[test]
fn flushing_during_write_chains_cleanly_without_concurrent_writes() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();
    let id = (full.clone(), branch.clone(), "Drawing1".into());

    // Open drawing as new board.
    s.open_board(0, "Drawing1".into(), true);
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-1".into(), kind: Kind::Rectangle,
        x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
        stroke: "#000".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 1,
    });

    // Simulate save_board starting an in-flight write
    assert!(b.save.edit());
    assert_eq!(b.save, Save::Saving { dirty: false });
    s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

    // User makes another edit while write 1 is running
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-2".into(), kind: Kind::Ellipse,
        x: 50.0, y: 50.0, w: 30.0, h: 30.0, points: vec![],
        stroke: "#f00".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 2,
    });
    s.save_board();
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });
    // Still only 1 in-flight write, no parallel write started
    assert_eq!(s.saving_notes.len(), 1);
    assert!(s.pending_note_saves.is_empty());

    // User closes drawing while write 1 is in-flight
    s.close_board(0);
    assert!(s.board.is_none(), "board should be closed");
    // flush_board should have queued the pending snapshot instead of starting a concurrent write!
    assert_eq!(s.saving_notes.len(), 1, "never start concurrent writes");
    assert!(s.pending_note_saves.contains_key(&id), "latest snapshot must be queued in pending_note_saves");
    let Some(crate::NoteOp::Save(queued_json)) = s.pending_note_saves.get(&id) else { panic!("a queued save") };
    assert!(queued_json.contains("elem-2"), "queued snapshot contains the latest edits");

    // Write 1 finishes: apply_board_write runs
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    // Queued write is consumed
    assert!(!s.pending_note_saves.contains_key(&id));
}

#[test]
fn reopen_board_with_pending_save_loads_pending_state() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();
    let id = (full.clone(), branch.clone(), "Drawing2".into());

    let pending_json = serde_json::json!({
        "type": "excalidraw", "version": 2,
        "elements": [{
            "id": "pending-rect", "type": "rectangle",
            "x": 100.0, "y": 100.0, "width": 80.0, "height": 40.0,
            "points": [], "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
            "strokeWidth": 2.0, "seed": 10
        }]
    }).to_string();

    s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));
    s.pending_note_saves.insert(id.clone(), crate::NoteOp::Save(pending_json));

    // Opening the board while a save is pending should load the pending in-memory JSON
    s.open_board(0, "Drawing2".into(), false);
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.elements.len(), 1);
    assert_eq!(b.elements[0].id, "pending-rect");
    assert_eq!(b.save, Save::Saving { dirty: true });
    assert!(!s.pending_note_saves.contains_key(&id));
}

#[test]
fn reopen_board_from_pending_saves_during_in_flight_write_and_edit_again() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();
    let id = (full.clone(), branch.clone(), "ReopenEdit".into());

    // 1. Open drawing as new and add elem-1
    s.open_board(0, "ReopenEdit".into(), true);
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-1".into(), kind: Kind::Rectangle,
        x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
        stroke: "#000".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 1,
    });

    // 2. Write 1 starts in-flight (snapshot with elem-1)
    assert!(b.save.edit());
    s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

    // 3. User edits elem-2 while write 1 is in flight
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-2".into(), kind: Kind::Ellipse,
        x: 50.0, y: 50.0, w: 30.0, h: 30.0, points: vec![],
        stroke: "#f00".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 2,
    });
    s.save_board();
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

    // 4. User closes drawing while write 1 is in-flight -> queued in pending_note_saves
    s.close_board(0);
    assert!(s.board.is_none());
    assert!(s.pending_note_saves.contains_key(&id));

    // 5. Re-open drawing while write 1 is STILL in flight
    s.open_board(0, "ReopenEdit".into(), false);
    // It should have removed id from pending_note_saves and set Save::Saving { dirty: true }
    assert!(!s.pending_note_saves.contains_key(&id));
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.elements.len(), 2);
    assert_eq!(b.save, Save::Saving { dirty: true });

    // 6. User adds elem-3 while still in-flight
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-3".into(), kind: Kind::Diamond,
        x: 80.0, y: 80.0, w: 40.0, h: 40.0, points: vec![],
        stroke: "#0f0".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 3,
    });
    s.save_board();
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

    // 7. Write 1 finishes!
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    // Board is open, so apply_board_write calls written(Ok(())) which returns true and triggers write_board()!
    // b.save becomes Saving { dirty: false }
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.save, Save::Saving { dirty: false });
    // The newly written JSON contains elem-1, elem-2, AND elem-3!
    let json = b.json();
    assert!(json.contains("elem-1"));
    assert!(json.contains("elem-2"));
    assert!(json.contains("elem-3"));

    // Write 2 finishes
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saved);
    assert!(s.saving_notes.is_empty());
}

#[test]
fn edit_saved_new_board_while_saving_notes_contains_id_triggers_write() {
    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();
    let id = (full.clone(), branch.clone(), "NewWhileSaving".into());

    // Simulate an old in-flight write (of an empty drawing) for this same drawing name
    let empty = board::Board::new(&full, &branch, "NewWhileSaving").save_json();
    s.saving_notes.insert(id.clone(), crate::NoteOp::Save(empty));

    // User creates new drawing with same name: it opens on the write that is running (its
    // content is the newest one), so the board is Saving, not a blank Saved board.
    s.open_board(0, "NewWhileSaving".into(), true);
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.save, Save::Saving { dirty: false });
    assert!(b.elements.is_empty());

    // User adds an element and triggers save_board()
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "elem-new".into(), kind: Kind::Rectangle,
        x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
        stroke: "#000".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 1,
    });
    s.save_board();

    // Because in_flight was true, b.save must be transitioned to Saving { dirty: true }
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

    // When the old in-flight write completes:
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));

    // It detects dirty: true, calls write_board() with elem-new, and transitions to dirty: false
    let b = s.board.as_ref().unwrap();
    assert_eq!(b.save, Save::Saving { dirty: false });
    let json = b.json();
    assert!(json.contains("elem-new"));

    // When write for elem-new completes:
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saved);
    assert!(s.saving_notes.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn flushing_during_write_chains_cleanly_and_persists_newest_content() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempDataDir::new("fix56-chain-save");

    let mut s = setup_app();
    let (full, branch) = s.notes_branch(0).unwrap();
    let id = (full.clone(), branch.clone(), "E2E".into());

    // 1. Open drawing as new
    s.open_board(0, "E2E".into(), true);
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "shape-1".into(), kind: Kind::Rectangle,
        x: 10.0, y: 10.0, w: 20.0, h: 20.0, points: vec![],
        stroke: "#000".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 1,
    });

    // 2. First write starts in-flight (snapshot 1)
    let json_1 = b.save_json();
    assert!(b.save.edit());
    s.saving_notes.insert(id.clone(), crate::NoteOp::Save(String::new()));

    // 3. User makes newer edits while write 1 is in-flight
    let b = s.board.as_mut().unwrap();
    b.elements.push(Element {
        id: "shape-2".into(), kind: Kind::Line,
        x: 30.0, y: 30.0, w: 40.0, h: 40.0, points: vec![[0.0, 0.0], [40.0, 40.0]],
        stroke: "#f00".into(), fill: None, width: 2.0,
        text: String::new(), font: 20.0, seed: 2,
    });
    s.save_board();
    assert_eq!(s.board.as_ref().unwrap().save, Save::Saving { dirty: true });

    // 4. User closes drawing while write 1 is in-flight
    s.close_board(0);
    assert!(s.board.is_none(), "board is closed");
    assert_eq!(s.saving_notes.len(), 1, "never start concurrent writes");
    assert!(s.pending_note_saves.contains_key(&id), "queued snapshot must be in pending_note_saves");
    let Some(crate::NoteOp::Save(queued_json)) = s.pending_note_saves.get(&id).cloned() else { panic!("a queued save") };

    // Older write 1 lands on disk first
    pollster::block_on(crate::notes::save(&id.0, &id.1, &id.2, json_1)).unwrap();

    // 5. Write 1 finishes: apply_board_write runs
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    assert!(!s.pending_note_saves.contains_key(&id), "queued write popped");

    // Write 2 lands on disk with the newest content
    pollster::block_on(crate::notes::save(&id.0, &id.1, &id.2, queued_json)).unwrap();
    s.apply_note_op(s.load_gen, id.clone(), Ok(()));
    assert!(s.saving_notes.is_empty());

    // 6. Verify drawing file on disk has the newest edits (both shape-1 and shape-2)
    let content = pollster::block_on(crate::notes::load(&full, &branch, "E2E")).unwrap().expect("drawing must exist");
    assert!(content.contains("shape-1"), "persisted content contains shape-1");
    assert!(content.contains("shape-2"), "persisted content contains shape-2 (newest edits)");
}
