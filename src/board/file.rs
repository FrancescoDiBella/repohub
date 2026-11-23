//! The `.excalidraw` file: versions on each write, saving, loading, merging a copy changed elsewhere,
//! and importing a file as a new board.

use super::*;

impl Board {
    // ---------------------------------------------------------------- file format

    /// Gives every element that changed since the last write a new version (and every element
    /// gone since a deletion mark), as Excalidraw does on each edit. Undo and redo bring back
    /// older content under a newer version, so they win over the copy they replace.
    fn stamp(&mut self) {
        let now = crate::sync::now_ms();
        let bump = |meta: &mut HashMap<String, Meta>, id: &str| {
            let m = meta.entry(id.to_owned()).or_insert(Meta { version: 0, nonce: 0, updated: 0 });
            m.version += 1;
            m.nonce = crate::sync::random() as u32;
            m.updated = now.max(m.updated);
        };
        let here: HashSet<&str> = self.elements.iter().map(|e| e.id.as_str()).collect();
        for e in &self.elements {
            if self.stamped.get(&e.id) != Some(e) {
                bump(&mut self.meta, &e.id);
            }
        }
        let mut gone: Vec<&Element> = self.stamped.values().filter(|e| !here.contains(e.id.as_str()) && !self.deleted.iter().any(|d| d.id == e.id)).collect();
        gone.sort_by(|a, b| a.id.cmp(&b.id));
        for e in gone {
            bump(&mut self.meta, &e.id);
            self.deleted.push(e.clone());
        }
        self.deleted.retain(|d| !here.contains(d.id.as_str()));
        self.stamped = self.elements.iter().map(|e| (e.id.clone(), e.clone())).collect();
    }

    /// The drawing as an Excalidraw document. Elements changed since the last call get a new
    /// version first.
    pub fn save_json(&mut self) -> String {
        self.stamp();
        self.json()
    }

    /// The drawing as an Excalidraw document, as last stamped (tests read it without writing).
    pub fn json(&self) -> String {
        let live = self.elements.iter().map(|e| (e, false));
        let gone = self.deleted.iter().map(|e| (e, true));
        let els: Vec<Value> = live.chain(gone).enumerate().map(|(i, (e, is_deleted))| {
            let m = self.meta.get(&e.id).copied().unwrap_or(Meta { version: 1, nonce: e.seed ^ 0x5bd1, updated: 0 });
            let mut v = json!({
                "id": e.id, "type": e.kind.excalidraw(), "x": e.x, "y": e.y, "width": e.w, "height": e.h,
                "angle": 0, "strokeColor": e.stroke, "backgroundColor": e.fill.clone().unwrap_or_else(|| "transparent".into()),
                "fillStyle": "solid", "strokeWidth": e.width, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
                "groupIds": [], "frameId": null, "roundness": null, "seed": e.seed, "version": m.version, "versionNonce": m.nonce,
                "isDeleted": is_deleted, "boundElements": null, "updated": m.updated, "link": null, "locked": false, "index": format!("a{i}"),
            });
            if e.kind.uses_points() {
                v["points"] = json!(e.points);
                v["lastCommittedPoint"] = Value::Null;
            }
            match e.kind {
                Kind::Arrow | Kind::Line => {
                    v["startBinding"] = Value::Null;
                    v["endBinding"] = Value::Null;
                    v["startArrowhead"] = Value::Null;
                    v["endArrowhead"] = if e.kind == Kind::Arrow { json!("arrow") } else { Value::Null };
                }
                Kind::Freedraw => {
                    v["pressures"] = json!([]);
                    v["simulatePressure"] = json!(true);
                }
                Kind::Text => {
                    v["text"] = json!(e.text);
                    v["originalText"] = json!(e.text);
                    v["fontSize"] = json!(e.font);
                    v["fontFamily"] = json!(1);
                    v["textAlign"] = json!("left");
                    v["verticalAlign"] = json!("top");
                    v["containerId"] = Value::Null;
                    v["autoResize"] = json!(true);
                    v["lineHeight"] = json!(1.25);
                }
                _ => {}
            }
            v
        }).collect();
        let doc = json!({
            "type": "excalidraw", "version": 2, "source": "https://github.com/FrancescoDiBella/repohub",
            "elements": els, "appState": { "viewBackgroundColor": "#ffffff", "gridSize": null }, "files": {},
        });
        serde_json::to_string_pretty(&doc).unwrap_or_default()
    }

    /// Reads an Excalidraw document; element types RepoHub doesn't draw are skipped.
    pub fn load_json(&mut self, src: &str) -> Result<(), String> {
        self.read_elements(src)?;
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        self.mode = Mode::Idle;
        self.text_edit = TextEdit::new("");
        // Open on what's drawn: its middle in the middle of the canvas.
        let mut centre = [WORLD[0] / 2.0, WORLD[1] / 2.0];
        if !self.elements.is_empty() {
            let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
            for e in &self.elements {
                let [x0, y0, x1, y1] = e.bbox();
                a = [a[0].min(x0), a[1].min(y0)];
                b = [b[0].max(x1), b[1].max(y1)];
            }
            centre = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        }
        self.view = View { zoom: 0.0, x: centre[0], y: centre[1] };
        Ok(())
    }

    /// Reads the elements of an Excalidraw document (drawn and deleted) and their versions,
    /// leaving the view and the tools as they are.
    fn read_elements(&mut self, src: &str) -> Result<(), String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("Not an Excalidraw file: {e}"))?;
        let f = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let st = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let (mut live, mut gone, mut meta) = (Vec::new(), Vec::new(), HashMap::new());
        let mut ids: HashSet<String> = HashSet::new();
        for (n, e) in v.get("elements").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let Some(kind) = Kind::from_excalidraw(&st(e, "type")) else { continue };
            let fill = Some(st(e, "backgroundColor")).filter(|c| !c.is_empty() && c != "transparent");
            let seed = e.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32;
            // An element needs an id of its own, the same on every device: a missing one is made
            // from its content (as `sync::merge` matches it), a repeated one (older files) from
            // its place.
            let mut id = st(e, "id");
            if id.is_empty() {
                id = crate::sync::derived_id(e);
            }
            if ids.contains(&id) {
                id = format!("{id}~{n}");
            }
            ids.insert(id.clone());
            let el = Element {
                id: id.clone(),
                kind,
                x: f(e, "x"),
                y: f(e, "y"),
                w: f(e, "width"),
                h: f(e, "height"),
                points: e.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(|p| {
                    let p = p.as_array()?;
                    Some([p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])
                }).collect()).unwrap_or_default(),
                stroke: Some(st(e, "strokeColor")).filter(|c| !c.is_empty()).unwrap_or_else(|| STROKES[0].into()),
                fill,
                width: f(e, "strokeWidth").max(1.0),
                text: st(e, "text"),
                font: Some(f(e, "fontSize")).filter(|s| *s > 0.0).unwrap_or(20.0),
                seed,
            };
            meta.insert(id, Meta {
                version: e.get("version").and_then(Value::as_u64).unwrap_or(1).max(1),
                nonce: e.get("versionNonce").and_then(Value::as_u64).map(|n| n as u32).unwrap_or(seed ^ 0x5bd1),
                updated: e.get("updated").and_then(Value::as_i64).unwrap_or(0),
            });
            if e.get("isDeleted").and_then(Value::as_bool) == Some(true) {
                gone.push(el);
            } else {
                live.push(el);
            }
        }
        self.stamped = live.iter().map(|e: &Element| (e.id.clone(), e.clone())).collect();
        self.elements = live;
        self.deleted = gone;
        self.meta = meta;
        self.next = self.elements.len() as u32 + 1;
        Ok(())
    }

    /// Shows `src` in place of what's drawn, keeping the view (the file changed underneath and
    /// nothing here is unsaved). Undo starts afresh.
    pub fn reload_json(&mut self, src: &str) -> Result<(), String> {
        let selected = self.selected.and_then(|i| self.elements.get(i)).map(|e| e.id.clone());
        self.read_elements(src)?;
        self.undo.clear();
        self.redo.clear();
        self.selected = selected.and_then(|id| self.elements.iter().position(|e| e.id == id));
        Ok(())
    }

    /// Takes in another copy of this drawing (the file changed underneath: another device's
    /// edits arrived): elements from it that are newer, or that this copy doesn't have, come
    /// in; this copy's own newer edits stay. Returns true when something came in; undo then
    /// starts afresh, so it can't take the other device's edits away.
    pub fn merge_json(&mut self, src: &str) -> Result<bool, String> {
        let mine = self.save_json();
        let Some(merged) = crate::sync::merge(&mine, src, crate::sync::Tie::First)? else { return Ok(false) };
        let selected = self.selected.and_then(|i| self.elements.get(i)).map(|e| e.id.clone());
        self.read_elements(&merged)?;
        self.undo.clear();
        self.redo.clear();
        self.selected = selected.and_then(|id| self.elements.iter().position(|e| e.id == id));
        Ok(true)
    }
}

/// An `.excalidraw` file as a new board (Import): what RepoHub draws of it, with its ids,
/// versions and deletions, then how many elements that is and how many it skipped.
pub fn import(src: &str) -> Result<(String, usize, usize), String> {
    let v: Value = serde_json::from_str(src).map_err(|e| format!("it isn't JSON ({e})"))?;
    let Some(els) = v.get("elements").and_then(Value::as_array) else { return Err("it has no elements".into()) };
    let skipped = els.iter()
        .filter(|e| e.get("isDeleted").and_then(Value::as_bool) != Some(true))
        .filter(|e| Kind::from_excalidraw(e.get("type").and_then(Value::as_str).unwrap_or("")).is_none())
        .count();
    let mut b = Board::new("", "", "");
    b.load_json(src)?;
    let kept = b.elements.len();
    Ok((b.save_json(), kept, skipped))
}
