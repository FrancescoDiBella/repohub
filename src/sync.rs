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

// ------------------------------------------------------------------ carrying out the plan

/// A remote write GitHub refused because the file changed meanwhile (409, or 422 when a `sha`
/// was missing or stale), or any other failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    Conflict,
    Other(String),
}

/// What the notes repository holds: path → blob id, and whether GitHub listed all of it.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub files: BTreeMap<String, String>,
    pub complete: bool,
}

/// The notes repository (GitHub, or a fake in tests).
#[allow(async_fn_in_trait)]
pub trait Remote {
    async fn list(&self) -> Result<Listing, String>;
    /// A file's content; `None` when it isn't there.
    async fn read(&self, path: &str) -> Result<Option<String>, String>;
    /// Creates the file (`sha` None) or replaces the blob `sha`; returns the new blob id.
    async fn write(&self, path: &str, content: &str, sha: Option<&str>, message: &str) -> Result<String, RemoteError>;
    async fn delete(&self, path: &str, sha: &str, message: &str) -> Result<(), RemoteError>;
}

/// The boards on this device (the notes storage, or a fake in tests).
#[allow(async_fn_in_trait)]
pub trait Local {
    /// Every board: notes-repository path → blob id of the local file.
    async fn list(&self) -> Result<BTreeMap<String, String>, String>;
    async fn read(&self, key: &Key) -> Result<Option<String>, String>;
    /// Writes the board only if its file still has blob id `expect` (`None`: doesn't exist).
    /// False when it changed meanwhile, and then nothing was written.
    async fn write_if(&self, key: &Key, expect: Option<&str>, content: &str) -> Result<bool, String>;
    /// Moves the board to the trash only if its file still has blob id `expect`.
    async fn trash_if(&self, key: &Key, expect: &str) -> Result<bool, String>;
    async fn load_index(&self) -> Result<Index, String>;
    async fn save_index(&self, index: &Index) -> Result<(), String>;
}

/// How a run went.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted_remote: usize,
    pub deleted_local: usize,
    pub merged: usize,
    /// Names of the conflict copies made (another device's copy that couldn't be merged).
    pub conflicts: Vec<String>,
    /// Boards whose local file this run wrote, with their new content (the open editor reloads
    /// from it).
    pub written: Vec<(Key, String)>,
    /// Boards this run moved to the trash.
    pub trashed: Vec<Key>,
    /// Boards with local changes still to send.
    pub to_send: BTreeSet<Key>,
    /// Boards known to be in step after the run.
    pub synced: BTreeSet<Key>,
    /// Boards too large to sync.
    pub too_large: Vec<Key>,
    /// GitHub listed only part of the repository.
    pub partial: bool,
    /// Why the run stopped early (offline, refused, rate-limited); the rest waits.
    pub error: Option<String>,
}

/// Limits and the date for conflict names.
#[derive(Clone, Debug)]
pub struct Options {
    /// Largest board synced, in bytes.
    pub max_bytes: usize,
    /// Today, `2026-11-22`.
    pub today: String,
}

impl Default for Options {
    fn default() -> Options {
        Options { max_bytes: 10 << 20, today: date(now_ms() / 1000) }
    }
}

enum Done {
    /// In step now.
    Synced,
    /// Left for the next run.
    Waiting,
    /// Gone on both sides now.
    Gone,
}

struct Runner<'a, R: Remote, L: Local> {
    remote: &'a R,
    local: &'a L,
    index: Index,
    opts: &'a Options,
    report: Report,
    local_names: BTreeSet<Key>,
}

impl<R: Remote, L: Local> Runner<'_, R, L> {
    async fn step(&mut self, s: &Step, retry: bool) -> Result<Done, String> {
        let sha = |c: &str| blob_sha(c.as_bytes());
        match s.action {
            Action::Nothing => Ok(if s.local.is_some() { Done::Synced } else { Done::Gone }),
            Action::Record => {
                if let (Some(l), Some(r)) = (&s.local, &s.remote) {
                    self.index.record(&s.path, l, r);
                }
                Ok(Done::Synced)
            }
            Action::Forget => {
                self.index.boards.remove(&s.path);
                Ok(Done::Gone)
            }
            Action::Upload => {
                let Some(content) = self.local.read(&s.key).await? else { return Ok(Done::Waiting) };
                let l = sha(&content);
                if Some(&l) != s.local.as_ref() {
                    return Ok(Done::Waiting);
                }
                if content.len() > self.opts.max_bytes {
                    self.report.too_large.push(s.key.clone());
                    return Ok(Done::Waiting);
                }
                match self.remote.write(&s.path, &content, s.remote.as_deref(), &message("Update", &s.key)).await {
                    Ok(r) => {
                        self.index.record(&s.path, &l, &r);
                        self.report.uploaded += 1;
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Action::Download => {
                let Some(content) = self.remote.read(&s.path).await? else { return Ok(Done::Waiting) };
                if content.len() > self.opts.max_bytes {
                    self.report.too_large.push(s.key.clone());
                    return Ok(Done::Waiting);
                }
                let c = sha(&content);
                if !self.local.write_if(&s.key, s.local.as_deref(), &content).await? {
                    return Ok(Done::Waiting);
                }
                self.index.record(&s.path, &c, &c);
                self.report.downloaded += 1;
                self.report.written.push((s.key.clone(), content));
                Ok(Done::Synced)
            }
            Action::DeleteRemote => {
                let Some(r) = &s.remote else { return Ok(Done::Gone) };
                match self.remote.delete(&s.path, r, &message("Delete", &s.key)).await {
                    Ok(()) => {
                        self.index.boards.remove(&s.path);
                        self.report.deleted_remote += 1;
                        Ok(Done::Gone)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Action::DeleteLocal => {
                let Some(l) = &s.local else { return Ok(Done::Gone) };
                if !self.local.trash_if(&s.key, l).await? {
                    return Ok(Done::Waiting);
                }
                self.index.boards.remove(&s.path);
                self.report.deleted_local += 1;
                self.report.trashed.push(s.key.clone());
                Ok(Done::Gone)
            }
            Action::Merge => self.merge(s, retry).await,
        }
    }

    /// GitHub refused a write because the file changed meanwhile: read it again and decide once
    /// more, without a further retry.
    async fn again(&mut self, s: &Step) -> Result<Done, String> {
        let now = self.remote.read(&s.path).await?;
        let r = now.as_deref().map(|c| blob_sha(c.as_bytes()));
        let entry = self.index.boards.get(&s.path).cloned().unwrap_or_default();
        let ls = Side::of(entry.local.as_deref(), s.local.as_deref());
        let rs = Side::of(entry.remote.as_deref(), r.as_deref());
        let action = decide(ls, rs, s.local.is_some() && s.local == r);
        let next = Step { path: s.path.clone(), key: s.key.clone(), action, local: s.local.clone(), remote: r };
        Box::pin(self.step(&next, false)).await
    }

    async fn merge(&mut self, s: &Step, retry: bool) -> Result<Done, String> {
        let sha = |c: &str| blob_sha(c.as_bytes());
        let Some(mine) = self.local.read(&s.key).await? else { return Ok(Done::Waiting) };
        let l = sha(&mine);
        if Some(&l) != s.local.as_ref() {
            return Ok(Done::Waiting);
        }
        let Some(theirs) = self.remote.read(&s.path).await? else { return Ok(Done::Waiting) };
        let r = sha(&theirs);
        if l == r {
            self.index.record(&s.path, &l, &r);
            return Ok(Done::Synced);
        }
        if mine.len().max(theirs.len()) > self.opts.max_bytes {
            self.report.too_large.push(s.key.clone());
            return Ok(Done::Waiting);
        }
        match merge(&mine, &theirs, Tie::KeepBoth) {
            Ok(None) => {
                // Everything there is here already. If nothing here is missing there either,
                // the two only differ in layout: remember them as in step.
                if matches!(merge(&theirs, &mine, Tie::KeepBoth), Ok(None)) {
                    self.index.record(&s.path, &l, &r);
                    return Ok(Done::Synced);
                }
                match self.remote.write(&s.path, &mine, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &l, &new_r);
                        self.report.merged += 1;
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) if retry => self.again(s).await,
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Ok(Some(merged)) => {
                if !self.local.write_if(&s.key, Some(&l), &merged).await? {
                    return Ok(Done::Waiting);
                }
                let m = sha(&merged);
                self.report.written.push((s.key.clone(), merged.clone()));
                match self.remote.write(&s.path, &merged, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &m, &new_r);
                        self.report.merged += 1;
                        Ok(Done::Synced)
                    }
                    // The local file holds the merge now; the next run sends it (or merges again
                    // with what arrived there meanwhile).
                    Err(RemoteError::Conflict) if retry => {
                        let next = Step { local: Some(m), ..s.clone() };
                        self.again(&next).await
                    }
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
            Err(_) => {
                // Not mergeable: the other device's copy is kept next to this one, under its own
                // name, and this one goes there.
                let today = self.opts.today.clone();
                let taken = self.local_names.clone();
                let name = conflict_name(&s.key.name, &today, |n| taken.contains(&Key::new(&s.key.repo, &s.key.branch, n)));
                let copy = Key::new(&s.key.repo, &s.key.branch, &name);
                if !self.local.write_if(&copy, None, &theirs).await? {
                    return Ok(Done::Waiting);
                }
                self.local_names.insert(copy.clone());
                self.report.written.push((copy.clone(), theirs.clone()));
                self.report.conflicts.push(name);
                let c = sha(&theirs);
                if let Ok(cr) = self.remote.write(&copy.path(), &theirs, None, &message("Update", &copy)).await {
                    self.index.record(&copy.path(), &c, &cr);
                    self.report.synced.insert(copy.clone());
                } else {
                    self.report.to_send.insert(copy);
                }
                match self.remote.write(&s.path, &mine, Some(&r), &message("Update", &s.key)).await {
                    Ok(new_r) => {
                        self.index.record(&s.path, &l, &new_r);
                        Ok(Done::Synced)
                    }
                    Err(RemoteError::Conflict) => Ok(Done::Waiting),
                    Err(RemoteError::Other(e)) => Err(e),
                }
            }
        }
    }
}

/// Whether a step that didn't happen leaves something here to send.
fn sends(a: Action) -> bool {
    matches!(a, Action::Upload | Action::DeleteRemote | Action::Merge)
}

/// One sync: read both sides and the index, carry out the plan, save the index.
pub async fn run<R: Remote, L: Local>(remote: &R, local: &L, repo: &str, opts: &Options) -> Report {
    let mut report = Report::default();
    let index = match local.load_index().await {
        Ok(i) if i.repo.eq_ignore_ascii_case(repo) => i,
        Ok(_) => Index { repo: repo.to_owned(), boards: BTreeMap::new() },
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    let here = match local.list().await {
        Ok(l) => l,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };
    let there = match remote.list().await {
        Ok(l) => l,
        Err(e) => {
            // Offline or refused: say what waits to be sent, from this side alone.
            for (path, l) in &here {
                if let Some(key) = Key::parse(path)
                    && index.boards.get(path).and_then(|e| e.local.as_ref()) != Some(l)
                {
                    report.to_send.insert(key);
                }
            }
            for (path, e) in &index.boards {
                if e.local.is_some() && !here.contains_key(path)
                    && let Some(key) = Key::parse(path)
                {
                    report.to_send.insert(key);
                }
            }
            report.error = Some(e);
            return report;
        }
    };
    report.partial = !there.complete;
    let steps = plan(&index, &here, &there.files, there.complete);
    let local_names = here.keys().filter_map(|p| Key::parse(p)).collect();
    let mut runner = Runner { remote, local, index, opts, report, local_names };
    let mut stopped = false;
    for s in &steps {
        if stopped {
            if sends(s.action) {
                runner.report.to_send.insert(s.key.clone());
            }
            continue;
        }
        match runner.step(s, true).await {
            Ok(Done::Synced) => {
                runner.report.synced.insert(s.key.clone());
            }
            Ok(Done::Gone) => {}
            Ok(Done::Waiting) => {
                if sends(s.action) {
                    runner.report.to_send.insert(s.key.clone());
                }
            }
            Err(e) => {
                runner.report.error = Some(e);
                stopped = true;
                if sends(s.action) {
                    runner.report.to_send.insert(s.key.clone());
                }
            }
        }
    }
    if let Err(e) = runner.local.save_index(&runner.index).await {
        runner.report.error.get_or_insert(e);
    }
    let mut report = runner.report;
    report.to_send.retain(|k| !report.synced.contains(k));
    report
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn hashes_match_git() {
        let hex = |b: [u8; 20]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        let long = "a".repeat(1000);
        assert_eq!(hex(sha1(long.as_bytes())), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
        // `printf 'hello\n' | git hash-object --stdin`, and the empty blob.
        assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn base64_round_trips_and_reads_githubs_line_breaks() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            assert_eq!(base64_decode(&base64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(&base64_encode(&all)).unwrap(), all);
        assert!(base64_decode("Zm9v!").is_none());
        assert!(base64_decode("Zm8=Zm8=").is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_795_000_000), "2026-11-18");
        assert_eq!(date(951_782_400), "2000-02-29");
    }

    #[test]
    fn paths_encode_odd_branches_and_read_back() {
        let k = Key::new("Octo/App", "feature/x y", "Plan (v2)");
        assert_eq!(k.repo, "octo/app");
        assert_eq!(k.path(), "boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw");
        assert_eq!(url_path(&k.path()), "boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw");
        assert_eq!(Key::parse(&k.path()), Some(k));
        // The same encoding as the local folder: a branch named `..` stays one folder.
        let dots = Key::new("o/r", "..", "x");
        assert_eq!(dots.path(), "boards/o/r/%2E./x.excalidraw");
        assert_eq!(Key::parse(&dots.path()), Some(dots));
        let dot_repo = Key::new("octo/.github", "main", "Ideas");
        assert_eq!(dot_repo.path(), "boards/octo/%2Egithub/main/Ideas.excalidraw");
        assert_eq!(Key::parse(&dot_repo.path()), Some(dot_repo));
        let unicode = Key::new("o/r", "café/ü", "Über");
        assert_eq!(Key::parse(&unicode.path()), Some(unicode));
        // Files RepoHub didn't write are left alone.
        for p in ["README.md", "boards/o/r/main/x.json", "boards/o/r/x.excalidraw", "boards/o/r/main/sub/x.excalidraw",
            "boards/O/r/main/x.excalidraw", "boards/o/r/feature/x/x.excalidraw", "boards/o/r/main/a%2Fb.excalidraw",
            "boards/o/r/main/.hidden.excalidraw", "boards/o/r/main/%2Ehidden.excalidraw", "other/o/r/main/x.excalidraw", "boards/o/r//x.excalidraw"] {
            assert_eq!(Key::parse(p), None, "{p}");
        }
        assert_eq!(message("Update", &Key::new("o/r", "main", "Plan")), "Update Plan (o/r@main)");
    }

    #[test]
    fn conflict_names_are_valid_and_unique() {
        let n = conflict_name("Plan", "2026-11-22", |_| false);
        assert_eq!(n, "Plan (conflict 2026-11-22)");
        assert!(notes::valid_name(&n));
        let n2 = conflict_name("Plan", "2026-11-22", |x| x == "Plan (conflict 2026-11-22)");
        assert_eq!(n2, "Plan (conflict 2026-11-22 2)");
        let long = conflict_name(&"x".repeat(80), "2026-11-22", |_| false);
        assert!(long.len() <= 80 && notes::valid_name(&long), "{long}");
    }

    #[test]
    fn index_round_trips_and_shrugs_off_junk() {
        let mut i = Index { repo: "me/repohub-notes".into(), boards: BTreeMap::new() };
        i.record("boards/o/r/main/x.excalidraw", &"a".repeat(40), &"b".repeat(40));
        i.boards.insert("boards/o/r/main/y.excalidraw".into(), Entry { local: None, remote: Some("c".repeat(40)) });
        assert_eq!(Index::from_json(&i.to_json()), i);
        assert_eq!(Index::from_json("not json"), Index::default());
        let odd = Index::from_json(r#"{"repo":"a/b","boards":{"p":{"local":"../x","remote":7}}}"#);
        assert_eq!(odd.boards["p"], Entry::default());
    }

    /// An index entry that knows only one side (cut short while being written, or edited by
    /// hand) never deletes the board on the side that has it.
    #[test]
    fn a_half_known_board_is_copied_never_deleted() {
        use Side::*;
        assert_eq!(decide(Unchanged, Absent, false), Action::Upload);
        assert_eq!(decide(Absent, Unchanged, false), Action::Download);
        for (l, r) in [(Unchanged, Absent), (Changed, Absent), (New, Absent), (Absent, Unchanged), (Absent, Changed), (Absent, New)] {
            assert!(!matches!(decide(l, r, false), Action::DeleteLocal | Action::DeleteRemote | Action::Forget), "{l:?} / {r:?}");
        }
        // Through the plan: the index knows the local file only, GitHub doesn't have it.
        let path = "boards/o/r/main/x.excalidraw".to_owned();
        let mut index = Index { repo: "me/notes".into(), boards: BTreeMap::new() };
        index.boards.insert(path.clone(), Entry { local: Some("a".repeat(40)), remote: None });
        let local = BTreeMap::from([(path.clone(), "a".repeat(40))]);
        let steps = plan(&index, &local, &BTreeMap::new(), true);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, Action::Upload);
    }

    #[test]
    fn the_decision_table_all_sixteen_cells() {
        use Action::*;
        use Side::*;
        let sides = [Unchanged, Changed, Deleted, New];
        let expect = [
            // remote: unchanged, changed, deleted, new
            [Nothing, Download, DeleteLocal, Merge],   // local unchanged
            [Upload, Merge, Upload, Merge],            // local changed
            [DeleteRemote, Download, Forget, Download], // local deleted
            [Merge, Merge, Upload, Merge],             // local new
        ];
        for (li, l) in sides.iter().enumerate() {
            for (ri, r) in sides.iter().enumerate() {
                assert_eq!(decide(*l, *r, false), expect[li][ri], "local {l:?}, remote {r:?}");
            }
        }
        // Same bytes on both sides: just remember them.
        assert_eq!(decide(Changed, Changed, true), Record);
        assert_eq!(decide(New, New, true), Record);
        assert_eq!(decide(Unchanged, Unchanged, true), Nothing);
        // Never deleted against a change: a deletion only wins over an unchanged side.
        for l in sides {
            for r in sides {
                let a = decide(l, r, false);
                if a == DeleteLocal {
                    assert_eq!(l, Unchanged);
                }
                if a == DeleteRemote {
                    assert_eq!(r, Unchanged);
                }
            }
        }
    }

    #[test]
    fn the_plan_reads_sides_from_the_index() {
        let a = Key::new("o/r", "main", "A").path();
        let b = Key::new("o/r", "main", "B").path();
        let c = Key::new("o/r", "main", "C").path();
        let mut index = Index::default();
        index.record(&a, &"1".repeat(40), &"1".repeat(40));
        index.record(&b, &"2".repeat(40), &"2".repeat(40));
        let local: BTreeMap<_, _> = [(a.clone(), "1".repeat(40)), (c.clone(), "3".repeat(40)), ("junk".to_owned(), "4".repeat(40))].into();
        let remote: BTreeMap<_, _> = [(a.clone(), "5".repeat(40)), (b.clone(), "2".repeat(40))].into();
        let steps = plan(&index, &local, &remote, true);
        let actions: Vec<_> = steps.iter().map(|s| (s.key.name.as_str(), s.action)).collect();
        assert_eq!(actions, vec![("A", Action::Download), ("B", Action::DeleteRemote), ("C", Action::Upload)]);
        // A tree GitHub cut short: nothing is deleted here for a board missing there.
        let partial = plan(&index, &[(a.clone(), "1".repeat(40))].into(), &BTreeMap::new(), false);
        assert_eq!(partial[0].action, Action::Nothing);
    }

    fn el(id: &str, version: u64, updated: i64, x: f64) -> Value {
        json!({ "id": id, "type": "rectangle", "x": x, "y": 0, "width": 10, "height": 10, "version": version, "versionNonce": 1, "updated": updated, "isDeleted": false })
    }

    pub(crate) fn doc(els: &[Value]) -> String {
        json!({ "type": "excalidraw", "version": 2, "source": "test", "elements": els, "appState": {}, "files": {} }).to_string()
    }

    fn ids(s: &str) -> Vec<(String, f64, bool)> {
        let v: Value = serde_json::from_str(s).unwrap();
        v["elements"].as_array().unwrap().iter().map(|e| (e["id"].as_str().unwrap().to_owned(), e["x"].as_f64().unwrap_or(0.0), deleted(e))).collect()
    }

    #[test]
    fn merge_takes_the_union_and_the_newer_version() {
        let mine = doc(&[el("a", 2, 10, 1.0), el("b", 1, 5, 2.0), el("c", 3, 7, 3.0)]);
        let mut gone = el("c", 4, 9, 3.0);
        gone["isDeleted"] = json!(true);
        let theirs = doc(&[el("a", 1, 5, 9.0), el("b", 2, 6, 8.0), gone, el("d", 1, 1, 4.0)]);
        let m = merge(&mine, &theirs, Tie::KeepBoth).unwrap().unwrap();
        assert_eq!(ids(&m), vec![("a".into(), 1.0, false), ("b".into(), 8.0, false), ("c".into(), 3.0, true), ("d".into(), 4.0, false)]);
        // Merging again adds nothing; nor does merging a board with itself.
        assert_eq!(merge(&m, &theirs, Tie::KeepBoth).unwrap(), None);
        assert_eq!(merge(&mine, &mine, Tie::KeepBoth).unwrap(), None);
        // The same version: the later `updated` wins.
        let later = merge(&doc(&[el("a", 2, 10, 1.0)]), &doc(&[el("a", 2, 11, 5.0)]), Tie::KeepBoth).unwrap().unwrap();
        assert_eq!(ids(&later), vec![("a".into(), 5.0, false)]);
    }

    #[test]
    fn merge_keeps_both_when_it_cant_tell() {
        // Two old boards (version 1, no time) whose ids happen to match: both strokes stay.
        let m = merge(&doc(&[el("rh2-1", 1, 0, 1.0)]), &doc(&[el("rh2-1", 1, 0, 2.0)]), Tie::KeepBoth).unwrap().unwrap();
        let got = ids(&m);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("rh2-1".into(), 1.0, false));
        assert!(got[1].0.starts_with("rh2-1~") && got[1].1 == 2.0);
        assert_eq!(merge(&m, &doc(&[el("rh2-1", 1, 0, 2.0)]), Tie::KeepBoth).unwrap(), None, "kept once, not again");
        // The same element written by two writers (its place in the list, `100` for `100.0`):
        // one element, not two.
        let mut moved = el("a", 3, 7, 100.0);
        moved["index"] = json!("a5");
        moved["x"] = json!(100);
        let mut other = el("a", 3, 7, 100.0);
        other["index"] = json!("a0");
        other["roundness"] = json!({ "type": 3 });
        assert_eq!(merge(&doc(&[moved]), &doc(&[other]), Tie::KeepBoth).unwrap(), None);
        // The editor against the file: its own content wins a tie.
        assert_eq!(merge(&doc(&[el("a", 1, 0, 1.0)]), &doc(&[el("a", 1, 0, 2.0)]), Tie::First).unwrap(), None);
        // Deleted on one side only at the same version: the drawn one stays.
        let mut d = el("a", 1, 0, 1.0);
        d["isDeleted"] = json!(true);
        let kept = merge(&doc(std::slice::from_ref(&d)), &doc(&[el("a", 1, 0, 1.0)]), Tie::KeepBoth).unwrap().unwrap();
        assert_eq!(ids(&kept), vec![("a".into(), 1.0, false)]);
        assert_eq!(merge(&doc(&[el("a", 1, 0, 1.0)]), &doc(&[d]), Tie::KeepBoth).unwrap(), None);
    }

    #[test]
    fn merge_keeps_elements_it_cant_draw_and_joins_files() {
        // An element with no id matches the one the editor named from its content.
        let bare = json!({ "type": "rectangle", "x": 1, "y": 2, "version": 1 });
        let mut named = bare.clone();
        named["id"] = Value::from(derived_id(&bare));
        assert_eq!(merge(&doc(&[named]), &doc(std::slice::from_ref(&bare)), Tie::KeepBoth).unwrap(), None);
        assert_eq!(merge(&doc(std::slice::from_ref(&bare)), &doc(std::slice::from_ref(&bare)), Tie::KeepBoth).unwrap(), None);
        let image = json!({ "id": "img", "type": "image", "fileId": "f1", "version": 1, "updated": 1 });
        let theirs = json!({ "type": "excalidraw", "elements": [image], "files": { "f1": { "dataURL": "data:" } } }).to_string();
        let m = merge(&doc(&[el("a", 1, 1, 1.0)]), &theirs, Tie::KeepBoth).unwrap().unwrap();
        let v: Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v["elements"][1]["type"], "image");
        assert!(v["files"]["f1"].is_object());
        assert!(merge("not json", &theirs, Tie::KeepBoth).is_err());
        assert!(merge(&theirs, "[1,2]", Tie::KeepBoth).is_err());
    }

    // -------------------------------------------------------------- fakes for the whole flow

    /// GitHub in memory: path → content, with the same sha checks.
    #[derive(Default)]
    pub(crate) struct FakeRemote {
        pub files: RefCell<BTreeMap<String, String>>,
        /// Something else writes this right before our next write (a third device).
        pub race: RefCell<Option<(String, String)>>,
        pub offline: RefCell<bool>,
        pub writes: RefCell<usize>,
    }

    impl Remote for FakeRemote {
        async fn list(&self) -> Result<Listing, String> {
            if *self.offline.borrow() {
                return Err("Can't reach GitHub".into());
            }
            Ok(Listing { files: self.files.borrow().iter().map(|(p, c)| (p.clone(), blob_sha(c.as_bytes()))).collect(), complete: true })
        }
        async fn read(&self, path: &str) -> Result<Option<String>, String> {
            Ok(self.files.borrow().get(path).cloned())
        }
        async fn write(&self, path: &str, content: &str, sha: Option<&str>, _: &str) -> Result<String, RemoteError> {
            if let Some((p, c)) = self.race.borrow_mut().take() {
                self.files.borrow_mut().insert(p, c);
            }
            let now = self.files.borrow().get(path).map(|c| blob_sha(c.as_bytes()));
            if now.as_deref() != sha {
                return Err(RemoteError::Conflict);
            }
            *self.writes.borrow_mut() += 1;
            self.files.borrow_mut().insert(path.to_owned(), content.to_owned());
            Ok(blob_sha(content.as_bytes()))
        }
        async fn delete(&self, path: &str, sha: &str, _: &str) -> Result<(), RemoteError> {
            if let Some((p, c)) = self.race.borrow_mut().take() {
                self.files.borrow_mut().insert(p, c);
            }
            let now = self.files.borrow().get(path).map(|c| blob_sha(c.as_bytes()));
            if now.as_deref() != Some(sha) {
                return Err(RemoteError::Conflict);
            }
            self.files.borrow_mut().remove(path);
            Ok(())
        }
    }

    /// A device's notes in memory, with a trash.
    #[derive(Default)]
    pub(crate) struct FakeLocal {
        pub files: RefCell<BTreeMap<Key, String>>,
        pub trash: RefCell<Vec<(Key, String)>>,
        pub index: RefCell<Index>,
        /// The editor saves this right before our next local write.
        pub race: RefCell<Option<(Key, String)>>,
    }

    impl FakeLocal {
        fn race_now(&self) {
            if let Some((k, c)) = self.race.borrow_mut().take() {
                self.files.borrow_mut().insert(k, c);
            }
        }
    }

    impl Local for FakeLocal {
        async fn list(&self) -> Result<BTreeMap<String, String>, String> {
            Ok(self.files.borrow().iter().map(|(k, c)| (k.path(), blob_sha(c.as_bytes()))).collect())
        }
        async fn read(&self, key: &Key) -> Result<Option<String>, String> {
            Ok(self.files.borrow().get(key).cloned())
        }
        async fn write_if(&self, key: &Key, expect: Option<&str>, content: &str) -> Result<bool, String> {
            self.race_now();
            let now = self.files.borrow().get(key).map(|c| blob_sha(c.as_bytes()));
            if now.as_deref() != expect {
                return Ok(false);
            }
            self.files.borrow_mut().insert(key.clone(), content.to_owned());
            Ok(true)
        }
        async fn trash_if(&self, key: &Key, expect: &str) -> Result<bool, String> {
            self.race_now();
            let now = self.files.borrow().get(key).map(|c| blob_sha(c.as_bytes()));
            if now.as_deref() != Some(expect) {
                return Ok(false);
            }
            let c = self.files.borrow_mut().remove(key).unwrap_or_default();
            self.trash.borrow_mut().push((key.clone(), c));
            Ok(true)
        }
        async fn load_index(&self) -> Result<Index, String> {
            Ok(self.index.borrow().clone())
        }
        async fn save_index(&self, index: &Index) -> Result<(), String> {
            *self.index.borrow_mut() = index.clone();
            Ok(())
        }
    }

    pub(crate) const REPO: &str = "me/repohub-notes";

    pub(crate) fn sync(remote: &FakeRemote, local: &FakeLocal) -> Report {
        let opts = Options { max_bytes: 10 << 20, today: "2026-11-22".into() };
        pollster::block_on(run(remote, local, REPO, &opts))
    }

    fn key(name: &str) -> Key {
        Key::new("octo/app", "feature/x y", name)
    }

    /// Every element id of a document that isn't deleted.
    fn live(s: &str) -> BTreeSet<String> {
        let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
        v["elements"].as_array().into_iter().flatten().filter(|e| !deleted(e)).filter_map(|e| e["id"].as_str().map(str::to_owned)).collect()
    }

    #[test]
    fn two_devices_converge_and_keep_both_sets_of_strokes() {
        let github = FakeRemote::default();
        let (phone, laptop) = (FakeLocal::default(), FakeLocal::default());
        // Drawn on the phone; the laptop gets it.
        phone.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 1, 100, 1.0)]));
        let r = sync(&github, &phone);
        assert_eq!((r.uploaded, r.error.clone()), (1, None));
        let r = sync(&github, &laptop);
        assert_eq!(r.downloaded, 1);
        assert_eq!(laptop.files.borrow()[&key("Plan")], phone.files.borrow()[&key("Plan")]);
        assert_eq!(r.written.len(), 1, "the laptop's editor hears about it");
        // Both change it while offline.
        phone.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 2, 200, 5.0), el("p2", 1, 210, 2.0)]));
        laptop.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 1, 100, 1.0), el("l1", 1, 220, 3.0)]));
        assert_eq!(sync(&github, &phone).uploaded, 1);
        let r = sync(&github, &laptop);
        assert_eq!(r.merged, 1);
        assert_eq!(sync(&github, &phone).downloaded, 1);
        let all: BTreeSet<String> = ["p1", "p2", "l1"].map(String::from).into();
        for side in [&phone.files.borrow()[&key("Plan")], &laptop.files.borrow()[&key("Plan")], &github.files.borrow()[&key("Plan").path()]] {
            assert_eq!(live(side), all);
        }
        assert_eq!(ids(&phone.files.borrow()[&key("Plan")])[0].1, 5.0, "the newer version of p1");
        // In step now: another run does nothing.
        let r = sync(&github, &laptop);
        assert_eq!((r.uploaded, r.downloaded, r.merged), (0, 0, 0));
        assert!(r.synced.contains(&key("Plan")) && r.to_send.is_empty());
    }

    #[test]
    fn a_deletion_follows_unless_the_other_side_changed_it() {
        let github = FakeRemote::default();
        let (a, b) = (FakeLocal::default(), FakeLocal::default());
        a.files.borrow_mut().insert(key("Old"), doc(&[el("o", 1, 1, 1.0)]));
        a.files.borrow_mut().insert(key("Kept"), doc(&[el("k", 1, 1, 1.0)]));
        sync(&github, &a);
        sync(&github, &b);
        // Deleted on a: gone on GitHub, then on b (into b's trash).
        a.files.borrow_mut().remove(&key("Old"));
        assert_eq!(sync(&github, &a).deleted_remote, 1);
        let r = sync(&github, &b);
        assert_eq!(r.deleted_local, 1);
        assert!(!b.files.borrow().contains_key(&key("Old")));
        assert_eq!(b.trash.borrow()[0].0, key("Old"), "kept in the trash");
        // Deleted on a, changed on b meanwhile: it comes back.
        a.files.borrow_mut().remove(&key("Kept"));
        b.files.borrow_mut().insert(key("Kept"), doc(&[el("k", 2, 5, 9.0)]));
        sync(&github, &b);
        let r = sync(&github, &a);
        assert_eq!(r.downloaded, 1, "changed there, so it comes back here");
        assert!(a.files.borrow().contains_key(&key("Kept")));
    }

    #[test]
    fn a_write_refused_because_github_changed_is_retried_through_the_decision() {
        let github = FakeRemote::default();
        let a = FakeLocal::default();
        a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0)]));
        sync(&github, &a);
        a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0), el("mine", 1, 5, 2.0)]));
        // A third device writes right before our upload: GitHub refuses it, we merge.
        *github.race.borrow_mut() = Some((key("Plan").path(), doc(&[el("a", 1, 1, 1.0), el("theirs", 1, 6, 3.0)])));
        let r = sync(&github, &a);
        assert_eq!(r.error, None);
        assert_eq!(r.merged, 1);
        let ids: BTreeSet<String> = ["a", "mine", "theirs"].map(String::from).into();
        assert_eq!(live(&github.files.borrow()[&key("Plan").path()]), ids);
        assert_eq!(live(&a.files.borrow()[&key("Plan")]), ids);
    }

    #[test]
    fn the_editor_saving_meanwhile_is_never_overwritten() {
        let github = FakeRemote::default();
        let a = FakeLocal::default();
        a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0)]));
        sync(&github, &a);
        github.files.borrow_mut().insert(key("Plan").path(), doc(&[el("a", 1, 1, 1.0), el("remote", 1, 3, 2.0)]));
        // The editor saves a new stroke right before the download lands: the download waits.
        let drawn = doc(&[el("a", 1, 1, 1.0), el("drawn", 1, 4, 3.0)]);
        *a.race.borrow_mut() = Some((key("Plan"), drawn.clone()));
        let r = sync(&github, &a);
        assert_eq!(r.downloaded, 0);
        assert!(r.to_send.is_empty(), "a download waiting sends nothing");
        assert_eq!(a.files.borrow()[&key("Plan")], drawn);
        // Next time both changed: merged.
        let r = sync(&github, &a);
        assert_eq!(r.merged, 1);
        let ids: BTreeSet<String> = ["a", "remote", "drawn"].map(String::from).into();
        assert_eq!(live(&a.files.borrow()[&key("Plan")]), ids);
    }

    #[test]
    fn boards_that_cant_be_merged_keep_both_copies() {
        let github = FakeRemote::default();
        let a = FakeLocal::default();
        a.files.borrow_mut().insert(key("Plan"), "{ broken".into());
        github.files.borrow_mut().insert(key("Plan").path(), doc(&[el("t", 1, 1, 1.0)]));
        let r = sync(&github, &a);
        assert_eq!(r.conflicts, vec!["Plan (conflict 2026-11-22)".to_owned()]);
        let copy = key("Plan (conflict 2026-11-22)");
        assert_eq!(live(&a.files.borrow()[&copy]), ["t".to_owned()].into());
        assert_eq!(github.files.borrow()[&key("Plan").path()], "{ broken");
        assert!(github.files.borrow().contains_key(&copy.path()));
        let r = sync(&github, &a);
        assert_eq!((r.uploaded, r.downloaded, r.merged, r.conflicts.len()), (0, 0, 0, 0), "in step afterwards");
    }

    #[test]
    fn offline_says_what_waits_and_changes_nothing() {
        let github = FakeRemote::default();
        let a = FakeLocal::default();
        a.files.borrow_mut().insert(key("One"), doc(&[]));
        sync(&github, &a);
        a.files.borrow_mut().insert(key("One"), doc(&[el("x", 1, 1, 1.0)]));
        a.files.borrow_mut().insert(key("Two"), doc(&[]));
        *github.offline.borrow_mut() = true;
        let r = sync(&github, &a);
        assert!(r.error.is_some());
        assert_eq!(r.to_send.len(), 2);
        assert_eq!(a.index.borrow().boards.len(), 1, "the index is untouched");
    }

    #[test]
    fn a_new_notes_repository_starts_from_an_empty_index() {
        let github = FakeRemote::default();
        let a = FakeLocal::default();
        a.files.borrow_mut().insert(key("One"), doc(&[]));
        sync(&github, &a);
        // The index belongs to another repository now: nothing is taken as deleted.
        a.index.borrow_mut().repo = "me/other".into();
        github.files.borrow_mut().clear();
        let r = sync(&github, &a);
        assert_eq!((r.uploaded, r.deleted_local), (1, 0));
    }

    /// Builds one cell of the decision table on a real run and checks that nothing drawn is
    /// lost: every element that was on a changed or new side is still somewhere afterwards
    /// (here, there or in the trash), and a side is only deleted when it hadn't changed.
    #[test]
    fn no_data_loss_in_any_cell() {
        use Side::*;
        let base = doc(&[el("base", 1, 1, 1.0)]);
        let here = doc(&[el("base", 1, 1, 1.0), el("here", 1, 5, 2.0)]);
        let there = doc(&[el("base", 1, 1, 1.0), el("there", 1, 6, 3.0)]);
        let k = key("Cell");
        for l in [Unchanged, Changed, Deleted, New] {
            for r in [Unchanged, Changed, Deleted, New] {
                let github = FakeRemote::default();
                let dev = FakeLocal::default();
                let sha = |c: &str| Some(blob_sha(c.as_bytes()));
                let entry = Entry {
                    local: if l == New { None } else { sha(&base) },
                    remote: if r == New { None } else { sha(&base) },
                };
                dev.index.borrow_mut().repo = REPO.into();
                dev.index.borrow_mut().boards.insert(k.path(), entry);
                match l {
                    Unchanged => { dev.files.borrow_mut().insert(k.clone(), base.clone()); }
                    Changed | New => { dev.files.borrow_mut().insert(k.clone(), here.clone()); }
                    _ => {}
                }
                match r {
                    Unchanged => { github.files.borrow_mut().insert(k.path(), base.clone()); }
                    Changed | New => { github.files.borrow_mut().insert(k.path(), there.clone()); }
                    _ => {}
                }
                let rep = sync(&github, &dev);
                assert_eq!(rep.error, None, "{l:?}/{r:?}");
                let after_here = dev.files.borrow().get(&k).cloned();
                let after_there = github.files.borrow().get(&k.path()).cloned();
                let mut kept: BTreeSet<String> = BTreeSet::new();
                for c in [after_here.clone(), after_there.clone()].into_iter().flatten().chain(dev.trash.borrow().iter().map(|t| t.1.clone())) {
                    kept.extend(live(&c));
                }
                if matches!(l, Changed | New) {
                    assert!(kept.contains("here"), "{l:?}/{r:?}: the local stroke is lost");
                    assert!(after_here.is_some(), "{l:?}/{r:?}: a changed board was deleted here");
                }
                if matches!(r, Changed | New) {
                    assert!(kept.contains("there"), "{l:?}/{r:?}: the remote stroke is lost");
                    assert!(after_there.is_some(), "{l:?}/{r:?}: a changed board was deleted there");
                }
                // Both sides end the same, or both gone.
                assert_eq!(after_here.map(|c| live(&c)), after_there.map(|c| live(&c)), "{l:?}/{r:?}: the sides differ after sync");
                // And a second run has nothing left to do.
                let again = sync(&github, &dev);
                assert_eq!((again.uploaded, again.downloaded, again.merged, again.deleted_local, again.deleted_remote), (0, 0, 0, 0, 0), "{l:?}/{r:?}");
            }
        }
    }
}
