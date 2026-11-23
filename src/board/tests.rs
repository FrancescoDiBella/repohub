use super::*;
use super::drawing::draw_element;

fn drag(phase: Phase, x: f32, y: f32, sx: f32, sy: f32) -> Gesture {
    Gesture { phase, x, y, start_x: sx, start_y: sy, w: WORLD[0], h: WORLD[1], cancelled: false }
}

#[test]
fn a_wide_canvas_shows_the_whole_drawing_as_before() {
    let mut b = Board::new("o/r", "main", "t");
    b.tool = Tool::Rectangle;
    // A canvas of half the world's size: canvas points map to world points times two.
    let g = |phase, x, y, sx, sy| Gesture { phase, x, y, start_x: sx, start_y: sy, w: 600.0, h: 360.0, cancelled: false };
    b.on_drag(g(Phase::Down, 50.0, 50.0, 50.0, 50.0));
    b.on_drag(g(Phase::Move, 150.0, 100.0, 50.0, 50.0));
    assert!(b.on_drag(g(Phase::Up, 150.0, 100.0, 50.0, 50.0)));
    assert_eq!((b.elements[0].x, b.elements[0].y, b.elements[0].w, b.elements[0].h), (100.0, 100.0, 200.0, 100.0));
}

#[test]
fn a_phone_canvas_zooms_and_moves() {
    let size = [360.0, 540.0];
    let mut b = Board::new("o/r", "main", "t");
    // Taller than the drawing: it fills the height, showing the middle of the drawing.
    let zoom = b.view.zoom_on(size);
    assert!((zoom - 2.5).abs() < 1e-3, "{zoom}");
    let vis = b.view.visible(size);
    assert!((vis[1] - WORLD[1]).abs() < 0.01 && (vis[0] - 480.0).abs() < 0.01);
    assert_eq!(b.view.origin(size), [360.0, 0.0]);
    // Drawing lands where the finger is.
    b.tool = Tool::Pen;
    let g = |phase, x, y, sx, sy| Gesture { phase, x, y, start_x: sx, start_y: sy, w: size[0], h: size[1], cancelled: false };
    b.on_drag(g(Phase::Down, 0.0, 0.0, 0.0, 0.0));
    b.on_drag(g(Phase::Move, 180.0, 270.0, 0.0, 0.0));
    assert!(b.on_drag(g(Phase::Up, 180.0, 270.0, 0.0, 0.0)));
    let e = &b.elements[0];
    assert_eq!((e.x, e.y), (360.0, 0.0));
    let last = e.points.last().unwrap();
    assert!((e.x + last[0] - 600.0).abs() < 0.01 && (e.y + last[1] - 360.0).abs() < 0.01);
    // The Move tool drags the view, never the drawing, and stops at the edges.
    b.tool = Tool::Pan;
    let before = b.elements.clone();
    b.on_drag(g(Phase::Down, 200.0, 200.0, 200.0, 200.0));
    assert!(!b.on_drag(g(Phase::Move, 125.0, 200.0, 200.0, 200.0)));
    assert!((b.view.origin(size)[0] - 460.0).abs() < 0.01, "75 points left at 0.75 points per unit");
    b.on_drag(g(Phase::Move, -5000.0, 200.0, 200.0, 200.0));
    assert!(!b.on_drag(g(Phase::Up, -5000.0, 200.0, 200.0, 200.0)));
    assert_eq!(b.view.origin(size)[0], WORLD[0] - b.view.visible(size)[0]);
    assert_eq!(b.elements, before);
    // Zooming keeps the middle and its limits.
    let mid = b.view.world(size, [180.0, 270.0]);
    b.view.zoom_by(size, 1.5);
    let after = b.view.world(size, [180.0, 270.0]);
    assert!((mid[1] - after[1]).abs() < 0.01);
    for _ in 0..20 { b.view.zoom_by(size, 1.5); }
    assert_eq!(b.view.zoom, MAX_ZOOM);
    for _ in 0..20 { b.view.zoom_by(size, 1.0 / 1.5); }
    assert_eq!(b.view.zoom, 1.0);
    assert_eq!(b.view.origin(size)[0], 0.0);
}

#[test]
fn a_loaded_drawing_opens_on_its_content() {
    let mut a = Board::new("o/r", "main", "t");
    a.tool = Tool::Rectangle;
    a.on_drag(drag(Phase::Down, 900.0, 100.0, 900.0, 100.0));
    a.on_drag(drag(Phase::Move, 1100.0, 200.0, 900.0, 100.0));
    a.on_drag(drag(Phase::Up, 1100.0, 200.0, 900.0, 100.0));
    let mut b = Board::new("o/r", "main", "t");
    b.load_json(&a.save_json()).unwrap();
    let size = [360.0, 540.0];
    let o = b.view.origin(size);
    assert!(o[0] > 700.0 && o[0] + b.view.visible(size)[0] >= 1100.0, "{o:?}");
}

#[test]
fn edits_get_new_versions_and_deletions_are_kept() {
    let mut b = Board::new("o/r", "main", "t");
    b.tool = Tool::Rectangle;
    b.on_drag(drag(Phase::Down, 100.0, 100.0, 100.0, 100.0));
    b.on_drag(drag(Phase::Move, 300.0, 200.0, 100.0, 100.0));
    assert!(b.on_drag(drag(Phase::Up, 300.0, 200.0, 100.0, 100.0)));
    let v = |b: &mut Board, i: usize| -> Value { serde_json::from_str::<Value>(&b.save_json()).unwrap()["elements"][i].clone() };
    let first = v(&mut b, 0);
    assert_eq!(first["version"], 1);
    assert!(first["updated"].as_i64().unwrap() > 0 && first["id"].as_str().unwrap().len() == 20);
    // Saving again without a change keeps the version.
    assert_eq!(v(&mut b, 0)["version"], 1);
    // Moved: a new version. Undo brings the old place back under a newer one still.
    b.tool = Tool::Select;
    b.on_drag(drag(Phase::Down, 100.0, 150.0, 100.0, 150.0));
    b.on_drag(drag(Phase::Move, 150.0, 150.0, 100.0, 150.0));
    assert!(b.on_drag(drag(Phase::Up, 150.0, 150.0, 100.0, 150.0)));
    assert_eq!(v(&mut b, 0)["version"], 2);
    assert!(b.undo());
    let back = v(&mut b, 0);
    assert_eq!((back["version"].as_u64(), back["x"].as_f64()), (Some(3), Some(100.0)));
    // Deleted: written with isDeleted and a newer version, and read back as deleted.
    b.selected = Some(0);
    assert!(b.delete_selected());
    let gone = v(&mut b, 0);
    assert_eq!((gone["isDeleted"].as_bool(), gone["version"].as_u64()), (Some(true), Some(4)));
    let mut c = Board::new("o/r", "main", "t");
    c.load_json(&b.save_json()).unwrap();
    assert!(c.elements.is_empty());
    assert_eq!(c.json(), b.json(), "the deletion survives a round trip");
    // Undoing the deletion brings it back, newer than the deletion.
    assert!(b.undo());
    let again = v(&mut b, 0);
    assert_eq!((again["isDeleted"].as_bool(), again["version"].as_u64()), (Some(false), Some(5)));
}

#[test]
fn elements_without_ids_stay_one_through_saves() {
    let file = serde_json::json!({ "type": "excalidraw", "elements": [
        { "type": "rectangle", "x": 10, "y": 10, "width": 50, "height": 40 },
        { "id": "dup", "type": "ellipse", "x": 100, "y": 10, "width": 50, "height": 40 },
        { "id": "dup", "type": "diamond", "x": 200, "y": 10, "width": 50, "height": 40 },
    ] }).to_string();
    let mut b = Board::new("o/r", "main", "t");
    b.load_json(&file).unwrap();
    assert_eq!(b.elements.len(), 3);
    let ids: std::collections::HashSet<&str> = b.elements.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "every element has an id of its own");
    // What a save writes over the file (merging with it) adds nothing, again and again.
    let mine = b.save_json();
    let merged = crate::sync::merge(&mine, &file, crate::sync::Tie::First).unwrap();
    assert!(merged.is_none(), "{merged:?}");
    assert!(!b.merge_json(&file).unwrap());
    assert_eq!(b.elements.len(), 3);
}

#[test]
fn another_copy_comes_in_without_losing_what_is_drawn_here() {
    let mut here = Board::new("o/r", "main", "t");
    here.tool = Tool::Pen;
    for (x, y) in [(100.0, 100.0), (400.0, 300.0)] {
        here.on_drag(drag(Phase::Down, x, y, x, y));
        here.on_drag(drag(Phase::Move, x + 40.0, y + 30.0, x, y));
        assert!(here.on_drag(drag(Phase::Up, x + 40.0, y + 30.0, x, y)));
    }
    let saved = here.save_json();
    let mut there = Board::new("o/r", "main", "t");
    there.load_json(&saved).unwrap();
    there.tool = Tool::Pen;
    there.on_drag(drag(Phase::Down, 600.0, 100.0, 600.0, 100.0));
    there.on_drag(drag(Phase::Move, 650.0, 160.0, 600.0, 100.0));
    assert!(there.on_drag(drag(Phase::Up, 650.0, 160.0, 600.0, 100.0)));
    there.selected = Some(0);
    assert!(there.delete_selected());
    // Here a third stroke, unsaved, while the other copy arrives.
    here.on_drag(drag(Phase::Down, 800.0, 500.0, 800.0, 500.0));
    here.on_drag(drag(Phase::Move, 850.0, 560.0, 800.0, 500.0));
    assert!(here.on_drag(drag(Phase::Up, 850.0, 560.0, 800.0, 500.0)));
    assert!(here.merge_json(&there.save_json()).unwrap());
    assert_eq!(here.elements.len(), 3, "the deletion came in, the new stroke there too, and this one stays");
    assert!(!here.merge_json(&there.save_json()).unwrap(), "nothing new the second time");
    // Reloading shows the file exactly.
    here.reload_json(&saved).unwrap();
    assert_eq!(here.elements.len(), 2);
}

#[test]
fn draw_move_undo_and_round_trip() {
    let mut b = Board::new("o/r", "main", "t");
    b.tool = Tool::Rectangle;
    assert!(!b.on_drag(drag(Phase::Down, 100.0, 100.0, 100.0, 100.0)));
    b.on_drag(drag(Phase::Move, 300.0, 200.0, 100.0, 100.0));
    assert!(b.on_drag(drag(Phase::Up, 300.0, 200.0, 100.0, 100.0)));
    assert_eq!((b.elements[0].w, b.elements[0].h), (200.0, 100.0));
    // Move it by its outline.
    b.tool = Tool::Select;
    b.on_drag(drag(Phase::Down, 100.0, 150.0, 100.0, 150.0));
    assert_eq!(b.selected, Some(0));
    b.on_drag(drag(Phase::Move, 150.0, 150.0, 100.0, 150.0));
    assert!(b.on_drag(drag(Phase::Up, 150.0, 150.0, 100.0, 150.0)));
    assert_eq!(b.elements[0].x, 150.0);
    assert!(b.undo());
    assert_eq!(b.elements[0].x, 100.0);
    assert!(b.redo());
    // A tiny click with a shape tool adds nothing.
    b.tool = Tool::Ellipse;
    b.on_drag(drag(Phase::Down, 500.0, 500.0, 500.0, 500.0));
    assert!(!b.on_drag(drag(Phase::Up, 501.0, 501.0, 500.0, 500.0)));
    assert_eq!(b.elements.len(), 1);
    // Text and arrow, then the Excalidraw round trip.
    // A click with the text tool starts typing, and releasing the button keeps it.
    b.tool = Tool::Text;
    b.on_drag(drag(Phase::Down, 400.0, 400.0, 400.0, 400.0));
    assert!(!b.on_drag(drag(Phase::Up, 400.0, 400.0, 400.0, 400.0)));
    assert_eq!(b.typing_at(), Some([400.0, 400.0]));
    b.text_edit = TextEdit::new("Hello");
    b.set_tool(Tool::Select);
    assert!(!b.is_typing() && !b.commit_text(), "picking a tool drops the text being typed");
    b.start_typing([400.0, 400.0]);
    b.text_edit = TextEdit::new("Hello");
    assert!(b.commit_text());
    b.tool = Tool::Arrow;
    b.on_drag(drag(Phase::Down, 10.0, 10.0, 10.0, 10.0));
    b.on_drag(drag(Phase::Move, 90.0, 60.0, 10.0, 10.0));
    assert!(b.on_drag(drag(Phase::Up, 90.0, 60.0, 10.0, 10.0)));
    let json = b.save_json();
    let mut c = Board::new("o/r", "main", "t");
    c.load_json(&json).unwrap();
    assert_eq!(c.elements.len(), 3);
    assert_eq!(c.elements[1].text, "Hello");
    assert_eq!(c.elements[2].kind, Kind::Arrow);
    assert_eq!(c.elements[2].points[1], [80.0, 50.0]);
    let v: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["type"], "excalidraw");
    assert_eq!(v["elements"][2]["endArrowhead"], "arrow");
}

#[test]
fn gestures_and_edits_ignored_during_loading() {
    let mut b = Board::new("o/r", "main", "test");
    b.save = crate::state::Save::Loading;
    b.tool = Tool::Rectangle;
    // Gestures during loading must be ignored and return false
    assert!(!b.on_drag(drag(Phase::Down, 100.0, 100.0, 100.0, 100.0)));
    assert!(!b.on_drag(drag(Phase::Move, 200.0, 200.0, 100.0, 100.0)));
    assert!(!b.on_drag(drag(Phase::Up, 200.0, 200.0, 100.0, 100.0)));
    assert!(b.elements.is_empty(), "canvas gestures during loading must not add elements");

    // Tool change and typing during loading are ignored
    b.set_tool(Tool::Text);
    assert_eq!(b.tool, Tool::Rectangle, "set_tool during loading must be ignored");
    b.start_typing([100.0, 100.0]);
    assert_eq!(b.typing_at(), None, "start_typing during loading must be ignored");
    assert!(!b.commit_text());

    // Undo, redo, and delete during loading do nothing
    assert!(!b.undo());
    assert!(!b.redo());
    assert!(!b.delete_selected());
}

#[test]
fn load_json_resets_mode_and_clears_transient_state() {
    let mut b = Board::new("o/r", "main", "test");
    b.tool = Tool::Rectangle;
    // Start dragging out a rectangle
    b.on_drag(drag(Phase::Down, 50.0, 50.0, 50.0, 50.0));
    assert_eq!(b.mode, Mode::Creating);
    assert_eq!(b.elements.len(), 1);

    // While creating, load_json arrives
    let sample = serde_json::json!({
        "type": "excalidraw",
        "version": 2,
        "elements": [{
            "id": "line-1",
            "type": "line",
            "x": 10.0,
            "y": 10.0,
            "width": 100.0,
            "height": 50.0,
            "points": [[0.0, 0.0], [100.0, 50.0]],
            "strokeColor": "#1e1e1e",
            "backgroundColor": "transparent",
            "strokeWidth": 2.0,
            "seed": 42
        }]
    }).to_string();

    assert!(b.load_json(&sample).is_ok());
    assert_eq!(b.mode, Mode::Idle, "load_json must reset mode to Idle");
    assert_eq!(b.selected, None, "load_json must clear selection");
    assert!(b.text_edit.text.is_empty(), "load_json must clear text_edit");

    // Subsequent drag move/up must not mutate the loaded element as if Creating
    let old_x = b.elements[0].x;
    assert!(!b.on_drag(drag(Phase::Move, 300.0, 300.0, 50.0, 50.0)));
    assert_eq!(b.elements[0].x, old_x, "Move after load_json should not drag loaded element");
}

#[test]
fn arrow_and_line_short_points_never_panic() {
    let mut b = Board::new("o/r", "main", "test");
    // Line with empty points
    b.elements.push(Element {
        id: "l0".into(),
        kind: Kind::Line,
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
        points: vec![],
        stroke: STROKES[0].into(),
        fill: None,
        width: 2.0,
        text: String::new(),
        font: 20.0,
        seed: 1,
    });
    // Arrow with 1 point
    b.elements.push(Element {
        id: "a1".into(),
        kind: Kind::Arrow,
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
        points: vec![[0.0, 0.0]],
        stroke: STROKES[0].into(),
        fill: None,
        width: 2.0,
        text: String::new(),
        font: 20.0,
        seed: 2,
    });

    // bbox on short points does not panic
    let _ = b.elements[0].bbox();
    let _ = b.elements[1].bbox();

    // draw_element on short points does not panic
    let s = Shape::new();
    let _ = draw_element(s.clone(), &b.elements[0], false);
    let _ = draw_element(s, &b.elements[1], false);

    // Simulating Creating move on element with 0 or 1 point does not panic
    b.mode = Mode::Creating;
    assert!(!b.on_drag(drag(Phase::Move, 50.0, 50.0, 0.0, 0.0)));
    assert_eq!(b.elements[1].points.len(), 2, "Phase::Move ensures at least 2 points for arrow");

    // Release on short points handles tiny element safely
    b.mode = Mode::Creating;
    let _ = b.on_drag(drag(Phase::Up, 50.0, 50.0, 0.0, 0.0));

    // Loading JSON with 0 or 1 point arrow/line does not panic
    let raw = serde_json::json!({
        "type": "excalidraw",
        "version": 2,
        "elements": [
            { "id": "e1", "type": "arrow", "x": 0.0, "y": 0.0, "points": [], "strokeColor": "#000" },
            { "id": "e2", "type": "line", "x": 0.0, "y": 0.0, "points": [[0.0, 0.0]], "strokeColor": "#000" }
        ]
    }).to_string();
    assert!(b.load_json(&raw).is_ok());
}
