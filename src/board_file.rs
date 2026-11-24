//! **Export…** and **Import…** of one board as an `.excalidraw` file; they need no account.
//!
//! - Desktop: Rune's save and open dialogs (`Tx::save_file`, `Tx::open_file`). Where no dialog
//!   can show (a build or a desktop without one), Export writes to the Downloads folder.
//! - Web: a download, and the browser's file picker.
//! - Android: the system's "save to" and file pickers, through the `RepoHubNative` bridge
//!   (`saveFile`, `openFile` in `MainActivity`).
use rune::Tx;

use crate::{App, board, net, notes};

/// Largest file Import reads (the notes storage takes no more).
const MAX_BYTES: usize = 25 << 20;

/// A board name from a file's name: its stem, with only the characters a name may have.
pub fn name_from_file(file: &str) -> String {
    let stem = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let stem = stem.strip_suffix(".excalidraw").or_else(|| stem.strip_suffix(".json")).unwrap_or(stem);
    let mut name: String = stem.chars().map(|c| if c.is_alphanumeric() || " _-.()".contains(c) { c } else { ' ' }).collect();
    name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_start_matches('.').trim();
    let mut name: String = name.chars().take(70).collect();
    while name.len() > 70 {
        name.pop();
    }
    let name = name.trim().to_owned();
    if notes::valid_name(&name) { name } else { "Imported drawing".into() }
}

/// `base`, or `base 2`, `base 3`… the first that isn't taken.
pub fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..).map(|n| format!("{base} {n}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_owned())
}

#[cfg(target_arch = "wasm32")]
mod js {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "
let seq = 0;
const waiting = new Map();
// The Android app answers its pickers here (MainActivity, fileReply).
globalThis.__repohubFile = (id, reply) => {
    const done = waiting.get(id);
    waiting.delete(id);
    if (done) done(reply);
};
function native(call) {
    return new Promise((done) => { const id = ++seq; waiting.set(id, done); call(id); });
}
export async function save_file(name, content) {
    const n = globalThis.RepoHubNative;
    if (n && n.saveFile) { return await native((id) => n.saveFile(id, name, content)); }
    try {
        const url = URL.createObjectURL(new Blob([content], { type: 'application/json' }));
        const a = document.createElement('a');
        a.href = url; a.download = name; a.style.display = 'none';
        document.body.appendChild(a); a.click(); a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 30000);
        return 'downloaded';
    } catch (e) { return 'error: ' + e; }
}
export async function open_file(max) {
    const n = globalThis.RepoHubNative;
    if (n && n.openFile) { return await native((id) => n.openFile(id)); }
    return await new Promise((done) => {
        const input = document.createElement('input');
        input.type = 'file'; input.accept = '.excalidraw,.json,application/json'; input.style.display = 'none';
        input.addEventListener('change', async () => {
            const f = input.files && input.files[0];
            input.remove();
            if (!f) { done(null); return; }
            if (f.size > max) { done('error: the file is larger than 25 MB'); return; }
            try { done([f.name, await f.text()]); } catch (e) { done('error: ' + e); }
        });
        input.addEventListener('cancel', () => { input.remove(); done(null); });
        document.body.appendChild(input);
        input.click();
    });
}
")]
    extern "C" {
        pub async fn save_file(name: &str, content: &str) -> JsValue;
        pub async fn open_file(max: f64) -> JsValue;
    }
}

/// The Downloads folder (or the data folder when there's none), for an export with no dialog.
#[cfg(not(target_arch = "wasm32"))]
fn downloads() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from);
    match home.map(|h| h.join("Downloads")).filter(|d| d.is_dir()) {
        Some(d) => d,
        None => notes::data_dir().join("exports"),
    }
}

/// Writes `json` as `file` in [`downloads`], with a number when the name is taken.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_to_downloads(file: &str, json: &str) -> Result<std::path::PathBuf, String> {
    let dir = downloads();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't export the drawing: {e}"))?;
    let stem = file.strip_suffix(".excalidraw").unwrap_or(file);
    let name = unique_name(stem, |n| dir.join(format!("{n}.excalidraw")).exists());
    let path = dir.join(format!("{name}.excalidraw"));
    std::fs::write(&path, json).map_err(|e| format!("Couldn't export the drawing: {e}"))?;
    Ok(path)
}

impl App {
    /// **Export…**: drawing `id` (repository, branch, name) as `<name>.excalidraw`: what's on
    /// screen if it's open, else its file.
    pub(crate) fn export_board(&mut self, tx: &mut Tx, id: (String, String, String)) {
        let open = self.board.as_mut()
            .filter(|b| (b.repo.as_str(), b.branch.as_str(), b.name.as_str()) == (id.0.as_str(), id.1.as_str(), id.2.as_str()))
            .map(|b| b.save_json());
        let file = format!("{}.excalidraw", id.2);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let json = match open {
                Some(j) => j,
                None => match pollster::block_on(notes::load(&id.0, &id.1, &id.2)) {
                    Ok(Some(j)) => j,
                    Ok(None) => return self.notify_problem("There's nothing to export yet: the drawing isn't saved."),
                    Err(e) => return self.notify_problem(e),
                },
            };
            let options = rune::FileDialogOptions { title: "Export the drawing".into(), file_name: Some(file.clone()), ..Default::default() }
                .filter("Excalidraw drawing", &["excalidraw"]);
            tx.save_file(options, move |s: &mut App, answer, _| match answer {
                Ok(Some(path)) => match std::fs::write(&path, &json) {
                    Ok(()) => s.notify(format!("Exported to {}", path.display())),
                    Err(e) => s.notify_problem(format!("Couldn't export the drawing: {e}")),
                },
                Ok(None) => {}
                // No dialog here: the Downloads folder.
                Err(_) => match write_to_downloads(&file, &json) {
                    Ok(path) => s.notify(format!("Exported to {}", path.display())),
                    Err(e) => s.notify_problem(e),
                },
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = tx;
            let Some(h) = self.handle.clone() else { return };
            let name = id.2.clone();
            net::spawn(h, move || async move {
                let json = match open {
                    Some(j) => j,
                    None => match notes::load(&id.0, &id.1, &id.2).await {
                        Ok(Some(j)) => j,
                        Ok(None) => return Err("There's nothing to export yet: the drawing isn't saved.".to_owned()),
                        Err(e) => return Err(e),
                    },
                };
                Ok(js::save_file(&file, &json).await.as_string().unwrap_or_default())
            }, move |s: &mut App, r: Result<String, String>| match r.as_deref() {
                Ok("saved") => s.notify(format!("Saved {name}.excalidraw")),
                Ok("downloaded") => s.notify(format!("Downloaded {name}.excalidraw")),
                Ok("cancelled") => {}
                Ok(e) => s.notify_problem(format!("Couldn't export the drawing: {}", e.trim_start_matches("error: "))),
                Err(e) => s.notify_problem(e.clone()),
            });
        }
    }

    /// **Import…**: picks an `.excalidraw` file and adds it as a new drawing of repository `i`'s
    /// branch.
    pub(crate) fn import_board(&mut self, tx: &mut Tx, i: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let options = rune::FileDialogOptions { title: "Import a drawing".into(), ..Default::default() }
                .filter("Excalidraw drawing", &["excalidraw", "json"]);
            tx.open_file(options, move |s: &mut App, answer, _| match answer {
                Ok(Some(path)) => {
                    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    match std::fs::metadata(&path).map(|m| m.len() as usize) {
                        Ok(n) if n > MAX_BYTES => s.notify_problem(format!("{file} is larger than 25 MB.")),
                        _ => match std::fs::read_to_string(&path) {
                            Ok(src) => s.add_imported(i, file, src),
                            Err(e) => s.notify_problem(format!("Couldn't read {file}: {e}")),
                        },
                    }
                }
                Ok(None) => {}
                Err(e) => s.notify_problem(format!("Couldn't open a file picker here: {e}")),
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = tx;
            let Some(h) = self.handle.clone() else { return };
            net::spawn(h, || async { js::open_file(MAX_BYTES as f64).await }, move |s: &mut App, v: wasm_bindgen::JsValue| {
                if v.is_null() || v.is_undefined() {
                    return;
                }
                if let Some(e) = v.as_string() {
                    return s.notify_problem(format!("Couldn't import the drawing: {}", e.trim_start_matches("error: ")));
                }
                let a = js_sys::Array::from(&v);
                let (file, src) = (a.get(0).as_string().unwrap_or_default(), a.get(1).as_string().unwrap_or_default());
                s.add_imported(i, file, src);
            });
        }
    }

    /// Adds the content of an imported file as a new drawing; a name that exists gets a number.
    pub(crate) fn add_imported(&mut self, i: usize, file: String, src: String) {
        let (json, kept, skipped) = match board::import(&src) {
            Ok(x) => x,
            Err(e) => return self.notify_problem(format!("{file} isn't an Excalidraw drawing: {e}")),
        };
        let Some((full, branch)) = self.notes_branch(i) else { return };
        let listed: Vec<String> = self.notes.get(&(full.clone(), branch.clone())).map(|v| v.items.iter().map(|n| n.name.clone()).collect()).unwrap_or_default();
        let base = name_from_file(&file);
        let Some(h) = self.handle.clone() else { return };
        let epoch = self.load_gen;
        net::spawn(h, move || async move {
            // The list on screen may be behind: a name taken on disk moves on to the next one.
            let mut tried: Vec<String> = listed;
            for _ in 0..50 {
                let name = unique_name(&base, |n| tried.iter().any(|t| t == n));
                if notes::write_if(&full, &branch, &name, None, &json).await? {
                    return Ok((full, branch, name));
                }
                tried.push(name);
            }
            Err("Couldn't find a free name for the drawing.".to_owned())
        }, move |s: &mut App, r: Result<(String, String, String), String>| match r {
            Ok(id) => {
                let skipped = match skipped {
                    0 => String::new(),
                    1 => " · 1 element RepoHub can't draw (kept on save)".into(),
                    n => format!(" · {n} elements RepoHub can't draw (kept on save)"),
                };
                let kept = if kept == 1 { "1 element".to_owned() } else { format!("{kept} elements") };
                s.notify(format!("Imported {} ({kept}){skipped}", id.2));
                s.sync_saved(&id);
                if s.load_gen == epoch
                    && let Some(i) = s.repo_index_of(&id.0)
                {
                    s.load_notes(i);
                }
            }
            Err(e) => s.notify_problem(format!("Couldn't import the drawing: {e}")),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, Tool};
    use rune::Phase;

    #[test]
    fn names_from_files() {
        assert_eq!(name_from_file("Plan.excalidraw"), "Plan");
        assert_eq!(name_from_file("/home/me/Downloads/Plan (v2).excalidraw"), "Plan (v2)");
        assert_eq!(name_from_file("C:\\x\\a&b.json"), "a b");
        assert_eq!(name_from_file(".hidden.excalidraw"), "hidden");
        assert_eq!(name_from_file("***.excalidraw"), "Imported drawing");
        assert!(notes::valid_name(&name_from_file(&format!("{}.excalidraw", "x".repeat(200)))));
        assert_eq!(unique_name("Plan", |_| false), "Plan");
        assert_eq!(unique_name("Plan", |n| n == "Plan" || n == "Plan 2"), "Plan 3");
    }

    #[test]
    fn export_then_import_gives_the_same_board() {
        let mut b = Board::new("o/r", "main", "Plan");
        b.tool = Tool::Rectangle;
        let g = |phase, x, y| board::Gesture { phase, x, y, start_x: 100.0, start_y: 100.0, w: 1200.0, h: 720.0, cancelled: false };
        b.on_drag(g(Phase::Down, 100.0, 100.0));
        b.on_drag(g(Phase::Move, 300.0, 200.0));
        assert!(b.on_drag(g(Phase::Up, 300.0, 200.0)));
        b.tool = Tool::Pen;
        b.on_drag(g(Phase::Down, 100.0, 100.0));
        b.on_drag(g(Phase::Move, 150.0, 160.0));
        b.on_drag(g(Phase::Move, 200.0, 120.0));
        assert!(b.on_drag(g(Phase::Up, 200.0, 120.0)));
        b.start_typing([400.0, 400.0]);
        b.text_edit = rune::TextEdit::new("Hello");
        assert!(b.commit_text());
        // Delete one, so the export carries a deletion too.
        b.selected = Some(0);
        assert!(b.delete_selected());
        let exported = b.save_json();
        let (json, kept, skipped) = board::import(&exported).unwrap();
        assert_eq!((kept, skipped), (2, 0));
        let mut back = Board::new("o/r", "main", "Plan");
        back.load_json(&json).unwrap();
        assert_eq!(back.elements, b.elements, "the same elements, ids and all");
        assert_eq!(json, exported, "the same file, versions and the deletion included");
    }

    #[test]
    fn import_skips_what_it_cant_draw_and_says_so() {
        let src = serde_json::json!({ "type": "excalidraw", "elements": [
            { "id": "i", "type": "image", "fileId": "f", "index": "a0" },
            { "id": "a", "type": "rectangle", "x": 1, "y": 2, "width": 3, "height": 4, "version": 3 },
            { "id": "f", "type": "frame", "index": "a2" },
            { "id": "gone", "type": "embeddable", "isDeleted": true },
        ], "files": { "f": { "id": "f", "mimeType": "image/png", "dataURL": "data:image/png;base64,AA==" } } }).to_string();
        let (json, kept, skipped) = board::import(&src).unwrap();
        assert_eq!((kept, skipped), (1, 2));
        assert!(json.contains("\"version\": 3"));
        let saved: serde_json::Value = serde_json::from_str(&json).unwrap();
        let els = saved["elements"].as_array().unwrap();
        let img_pos = els.iter().position(|e| e["type"] == "image" && e["id"] == "i").expect("image preserved");
        let rect_pos = els.iter().position(|e| e["type"] == "rectangle" && e["id"] == "a").expect("rect preserved");
        assert!(img_pos < rect_pos, "background image stays behind drawn rectangle");
        assert_ne!(els[img_pos]["index"], els[rect_pos]["index"], "synthetic index avoids colliding with extra element index");
        assert!(els.iter().any(|e| e["type"] == "frame" && e["id"] == "f"));
        assert_eq!(saved["files"]["f"]["mimeType"], "image/png", "attached files are preserved on save");
        assert!(board::import("not json").is_err());
        assert!(board::import("{\"type\":\"other\"}").is_err());
    }
}
