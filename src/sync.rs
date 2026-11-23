//! Boards on every device: the sync engine (pure, tested), used through `sync_io`.
//!
//! Boards sync through one private repository on the user's GitHub account, the *notes
//! repository*, at `boards/<owner>/<repo>/<branch>/<name>.excalidraw` (the segments encoded as
//! the local folder encodes them, [`crate::notes::segment`]).
//!
//! Per board the device remembers what it last synced ([`Index`]): the git blob id of the local
//! file and of the remote file, which are the same bytes once they're in step. Each side is then
//! unchanged, changed, deleted or new ([`Side`]), and [`decide`] picks the [`Action`] for the
//! pair. [`run`] carries out the plan through two small traits, [`Remote`] (GitHub) and
//! [`Local`] (the notes storage), so the whole flow is tested here with fakes.
//!
//! The rule everywhere: when in doubt, keep both copies. A local write only lands when the file
//! still holds what the plan read ([`Local::write_if`]), a remote write only when GitHub still
//! holds the blob the plan read (its `sha`), and the index only records a board once both sides
//! hold the same content.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Value, json};

use crate::notes;

mod runner;

pub use runner::{Listing, Local, Options, Remote, RemoteError, Report, run};

// ------------------------------------------------------------------ hashing and encoding

/// SHA-1 of `data` (git's object ids; not used for anything secret).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let block = |h: &mut [u32; 5], chunk: &[u8]| {
        let mut w = [0u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    };
    let (chunks, rest) = data.as_chunks::<64>();
    for chunk in chunks {
        block(&mut h, chunk);
    }
    let mut tail = rest.to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for chunk in tail.as_chunks::<64>().0 {
        block(&mut h, chunk);
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// The git blob id of a file's content: what GitHub's trees and contents answers call `sha`.
pub fn blob_sha(content: &[u8]) -> String {
    let mut data = format!("blob {}\0", content.len()).into_bytes();
    data.extend_from_slice(content);
    sha1(&data).iter().map(|b| format!("{b:02x}")).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (GitHub's `content` field).
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes base64, skipping the line breaks GitHub puts every 60 characters. `None` when it
/// isn't base64.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut ended = false;
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                ended = true;
                continue;
            }
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        if ended {
            return None;
        }
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Milliseconds since 1970 (Excalidraw's `updated`).
pub fn now_ms() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
}

/// A random number, for element ids and Excalidraw's `versionNonce`.
pub fn random() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::hash::{BuildHasher, Hasher};
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // A fresh randomly keyed hasher each time (the standard library seeds it from the OS).
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        h.write_i64(now_ms());
        h.finish()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let a = (js_sys::Math::random() * 4_294_967_296.0) as u64;
        let b = (js_sys::Math::random() * 4_294_967_296.0) as u64;
        a << 32 | b
    }
}

/// `2026-11-22` for Unix seconds `t` (UTC).
pub fn date(t: i64) -> String {
    let days = t.div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// ------------------------------------------------------------------ where boards go

/// One board: repository (`owner/name`, lowercase as the local folder keeps it), branch, name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub repo: String,
    pub branch: String,
    pub name: String,
}

impl Key {
    pub fn new(repo: &str, branch: &str, name: &str) -> Key {
        Key { repo: repo.to_ascii_lowercase(), branch: branch.into(), name: name.into() }
    }

    /// The file in the notes repository: `boards/<owner>/<repo>/<branch>/<name>.excalidraw`.
    pub fn path(&self) -> String {
        let (owner, name) = self.repo.split_once('/').unwrap_or((&self.repo, ""));
        format!("boards/{}/{}/{}/{}.excalidraw", notes::segment(owner), notes::segment(name), notes::segment(&self.branch), notes::segment(&self.name))
    }

    /// The board a notes repository's file holds; `None` for anything RepoHub didn't write
    /// (another layout, another encoding, a name the app wouldn't take), which sync leaves alone.
    pub fn parse(path: &str) -> Option<Key> {
        let rest = path.strip_prefix("boards/")?;
        let parts: Vec<&str> = rest.split('/').collect();
        let [owner, repo, branch, file] = parts.as_slice() else { return None };
        let name = file.strip_suffix(".excalidraw")?;
        let key = Key { repo: format!("{}/{}", notes::decode(owner), notes::decode(repo)), branch: notes::decode(branch), name: notes::decode(name) };
        let repo_ok = key.repo.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c)));
        let ok = repo_ok && !key.branch.is_empty() && notes::valid_name(&key.name) && key.path() == path;
        ok.then_some(key)
    }
}

/// A notes-repository path as it goes in an API address: `%` (from the segment encoding)
/// becomes `%25`, so GitHub reads the file name back with its `%` signs.
pub fn url_path(path: &str) -> String {
    path.replace('%', "%25")
}

/// The commit message of a change: `Update Plan (octo/app@main)`.
pub fn message(verb: &str, key: &Key) -> String {
    format!("{verb} {} ({}@{})", key.name, key.repo, key.branch)
}

/// A name for the other device's copy when two can't be merged: `Plan (conflict 2026-11-22)`,
/// with a number when that exists too. Always a valid board name.
pub fn conflict_name(name: &str, today: &str, taken: impl Fn(&str) -> bool) -> String {
    for n in 1..100 {
        let tail = if n == 1 { format!(" (conflict {today})") } else { format!(" (conflict {today} {n})") };
        let keep = 80usize.saturating_sub(tail.len());
        let mut base: String = name.chars().collect();
        while base.len() > keep {
            base.pop();
        }
        let candidate = format!("{}{tail}", base.trim_end());
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("Conflict {today} {}", random() % 100_000)
}

// ------------------------------------------------------------------ what was last synced

/// What one board was when it was last in step: the local file's and the remote file's blob ids.
/// `None` on a side means that side wasn't known then.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub local: Option<String>,
    pub remote: Option<String>,
}

/// The index of one device for one notes repository: notes-repository path → [`Entry`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub repo: String,
    pub boards: BTreeMap<String, Entry>,
}

impl Index {
    pub fn to_json(&self) -> String {
        let boards: serde_json::Map<String, Value> = self.boards.iter().map(|(p, e)| (p.clone(), json!({ "local": e.local, "remote": e.remote }))).collect();
        json!({ "version": 1, "repo": self.repo, "boards": boards }).to_string()
    }

    /// Reads an index; anything unreadable is an empty index (every board then counts as new,
    /// which merges or copies, never deletes).
    pub fn from_json(s: &str) -> Index {
        let Ok(v) = serde_json::from_str::<Value>(s) else { return Index::default() };
        let side = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).filter(|x| x.len() == 40 && x.bytes().all(|b| b.is_ascii_hexdigit())).map(str::to_owned);
        Index {
            repo: v.get("repo").and_then(Value::as_str).unwrap_or("").to_owned(),
            boards: v.get("boards").and_then(Value::as_object).into_iter().flatten()
                .map(|(p, e)| (p.clone(), Entry { local: side(e, "local"), remote: side(e, "remote") }))
                .collect(),
        }
    }

    fn record(&mut self, path: &str, local: &str, remote: &str) {
        self.boards.insert(path.to_owned(), Entry { local: Some(local.to_owned()), remote: Some(remote.to_owned()) });
    }
}

// ------------------------------------------------------------------ the decision

/// One side of a board, against what was last synced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Unchanged,
    Changed,
    /// Known, and gone now.
    Deleted,
    /// There now, not known before.
    New,
    /// Not there and not known.
    Absent,
}

impl Side {
    pub fn of(known: Option<&str>, now: Option<&str>) -> Side {
        match (known, now) {
            (Some(k), Some(n)) if k == n => Side::Unchanged,
            (Some(_), Some(_)) => Side::Changed,
            (Some(_), None) => Side::Deleted,
            (None, Some(_)) => Side::New,
            (None, None) => Side::Absent,
        }
    }

    fn exists(self) -> bool {
        matches!(self, Side::Unchanged | Side::Changed | Side::New)
    }
}

/// What to do with one board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Both sides hold the same content: remember it.
    Record,
    Upload,
    Download,
    DeleteRemote,
    /// Move the local file to the trash.
    DeleteLocal,
    /// Gone on both sides: forget it.
    Forget,
    /// Changed on both sides (or new on both): merge, or keep both.
    Merge,
}

/// The decision table. `same` says both files exist with the same content.
///
/// | local \ remote | unchanged | changed | deleted | new |
/// |---|---|---|---|---|
/// | unchanged | nothing | download | delete here | merge |
/// | changed | upload | merge | upload | merge |
/// | deleted | delete there | download | forget | download |
/// | new | merge | merge | upload | merge |
///
/// A side that is deleted loses only to an unchanged other side: a changed board comes back.
/// Where both files exist and hold the same bytes, the board is simply recorded.
pub fn decide(local: Side, remote: Side, same: bool) -> Action {
    use Side::*;
    if local.exists() && remote.exists() && same {
        return if local == Unchanged && remote == Unchanged { Action::Nothing } else { Action::Record };
    }
    match (local, remote) {
        (Unchanged, Unchanged) => Action::Nothing,
        (Unchanged, Changed) => Action::Download,
        (Changed, Unchanged) => Action::Upload,
        (Changed | New, Changed | New) | (Unchanged, New) | (New, Unchanged) => Action::Merge,
        // Only a side that was **known and is gone** deletes the other. A side that was never
        // known (a half-written or hand-edited index) is copied to, never deleted for.
        (Unchanged, Deleted) => Action::DeleteLocal,
        (Unchanged, Absent) => Action::Upload,
        (Changed | New, Deleted | Absent) => Action::Upload,
        (Deleted, Unchanged) => Action::DeleteRemote,
        (Absent, Unchanged) => Action::Download,
        (Deleted | Absent, Changed | New) => Action::Download,
        (Deleted | Absent, Deleted | Absent) => Action::Forget,
    }
}

/// One board's step in a plan, with the blob ids the decision was made on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub path: String,
    pub key: Key,
    pub action: Action,
    pub local: Option<String>,
    pub remote: Option<String>,
}

/// The plan for every board either side or the index knows. With `complete` false (GitHub cut
/// the tree short) a board missing remotely may still be there: nothing is deleted locally then,
/// and uploads go without a `sha` (GitHub refuses one that exists, and the retry merges).
pub fn plan(index: &Index, local: &BTreeMap<String, String>, remote: &BTreeMap<String, String>, complete: bool) -> Vec<Step> {
    let paths: BTreeSet<&String> = index.boards.keys().chain(local.keys()).chain(remote.keys()).collect();
    paths.into_iter().filter_map(|path| {
        let key = Key::parse(path)?;
        let entry = index.boards.get(path).cloned().unwrap_or_default();
        let (l, r) = (local.get(path), remote.get(path));
        let ls = Side::of(entry.local.as_deref(), l.map(String::as_str));
        let rs = Side::of(entry.remote.as_deref(), r.map(String::as_str));
        let mut action = decide(ls, rs, l.is_some() && l == r);
        if !complete && r.is_none() {
            action = match action {
                Action::DeleteLocal | Action::Forget => Action::Nothing,
                other => other,
            };
        }
        Some(Step { path: path.clone(), key, action, local: l.cloned(), remote: r.cloned() })
    }).collect()
}

// ------------------------------------------------------------------ merging two boards

/// What a merge does when two elements have the same id, version and time but differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tie {
    /// Keep the first document's (the editor's own content against the file on disk).
    First,
    /// Keep both: the second's gets a new id (two devices' boards, never drop a stroke).
    KeepBoth,
}

fn version(e: &Value) -> u64 {
    e.get("version").and_then(Value::as_u64).unwrap_or(0)
}

fn updated(e: &Value) -> i64 {
    e.get("updated").and_then(Value::as_i64).unwrap_or(0)
}

fn deleted(e: &Value) -> bool {
    e.get("isDeleted").and_then(Value::as_bool) == Some(true)
}

/// Numbers equal within what an `f32` keeps (RepoHub writes `100.0` where Excalidraw writes
/// `100`), lists and objects alike throughout.
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            (x - y).abs() <= 1e-3_f64.max(x.abs().max(y.abs()) * 1e-6)
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| close(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w))),
        _ => a == b,
    }
}

/// Whether two copies of an element draw the same thing: its place, shape, text and colours.
/// Bookkeeping that differs between writers (`index`, field order, `100` and `100.0`) doesn't count.
fn same_drawing(a: &Value, b: &Value) -> bool {
    const KEYS: [&str; 15] = ["type", "x", "y", "width", "height", "angle", "points", "text", "strokeColor", "backgroundColor", "strokeWidth", "fontSize", "isDeleted", "fileId", "containerId"];
    KEYS.iter().all(|k| close(a.get(*k).unwrap_or(&Value::Null), b.get(*k).unwrap_or(&Value::Null)))
}

fn short_hash(v: &Value) -> String {
    blob_sha(v.to_string().as_bytes())[..8].to_owned()
}

/// The id of an element that has none (hand-made files): made from its content, so every device
/// and every read gives it the same one.
pub fn derived_id(e: &Value) -> String {
    format!("el-{}", short_hash(e))
}

fn id_of(e: &Value) -> String {
    e.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()).map(str::to_owned).unwrap_or_else(|| derived_id(e))
}

fn elements_of(doc: &Value) -> Result<Vec<Value>, String> {
    if !doc.is_object() {
        return Err("not an Excalidraw document".into());
    }
    match doc.get("elements") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err("its elements aren't a list".into()),
    }
}

/// Merges board `other` into board `base`, as Excalidraw reconciles two scenes: the union of the
/// elements by id; for an id on both sides the higher `version`, then the later `updated`
/// (deletions are elements with `isDeleted`, so they win or lose the same way). Images
/// (`files`) are joined; the rest of `base` stays.
///
/// `Ok(None)` when `other` adds nothing to `base`. `Err` when either isn't a board, and then
/// the caller keeps both files.
pub fn merge(base: &str, other: &str, tie: Tie) -> Result<Option<String>, String> {
    let mut doc: Value = serde_json::from_str(base).map_err(|e| format!("This copy isn't readable: {e}"))?;
    let theirs: Value = serde_json::from_str(other).map_err(|e| format!("The other copy isn't readable: {e}"))?;
    let mut mine = elements_of(&doc)?;
    let theirs_els = elements_of(&theirs)?;
    let mut at: HashMap<String, usize> = HashMap::new();
    for (i, e) in mine.iter().enumerate() {
        at.entry(id_of(e)).or_insert(i);
    }
    let mut changed = false;
    for o in theirs_els {
        let id = id_of(&o);
        let Some(&i) = at.get(&id) else {
            at.insert(id, mine.len());
            mine.push(o);
            changed = true;
            continue;
        };
        let m = &mine[i];
        if *m == o {
            continue;
        }
        let order = version(&o).cmp(&version(m)).then(updated(&o).cmp(&updated(m)));
        match order {
            std::cmp::Ordering::Greater => {
                mine[i] = o;
                changed = true;
            }
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => match (tie, deleted(m), deleted(&o)) {
                (Tie::First, _, _) => {}
                // Deleted on one side only, at the same version: the drawn one stays.
                (Tie::KeepBoth, true, false) => {
                    mine[i] = o;
                    changed = true;
                }
                (Tie::KeepBoth, _, true) => {}
                (Tie::KeepBoth, false, false) if same_drawing(m, &o) => {}
                (Tie::KeepBoth, false, false) => {
                    let copy_id = format!("{id}~{}", short_hash(&o));
                    if let std::collections::hash_map::Entry::Vacant(slot) = at.entry(copy_id.clone()) {
                        slot.insert(mine.len());
                        let mut copy = o;
                        copy["id"] = Value::from(copy_id);
                        mine.push(copy);
                        changed = true;
                    }
                }
            },
        }
    }
    if let Some(files) = theirs.get("files").and_then(Value::as_object) {
        let ours = doc.as_object_mut().expect("checked above").entry("files").or_insert_with(|| json!({}));
        if let Some(ours) = ours.as_object_mut() {
            for (k, v) in files {
                if !ours.contains_key(k) {
                    ours.insert(k.clone(), v.clone());
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return Ok(None);
    }
    doc["elements"] = Value::Array(mine);
    serde_json::to_string_pretty(&doc).map(Some).map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) mod tests;
