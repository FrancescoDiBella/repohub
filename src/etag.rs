//! Conditional requests: each GET's `ETag` with its answer, so asking again for something that
//! didn't change costs GitHub a `304 Not Modified`, which doesn't count against the rate limit.
//!
//! The store is one per process (every `Gh` shares it). `Gh::fetch` sends `If-None-Match` with
//! the tag it knows and, on a 304, hands back the answer it kept. Searches are left out on
//! purpose: GitHub's search results carry no tag worth sending back. Signing out or switching
//! accounts forgets everything ([`clear`]); a request that started before that can't put its
//! answer back (each `Gh` remembers the [`generation`] it was made in). The saved copy
//! (`cache`) keeps the tags too, so the first refresh after a start is conditional as well.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the store may hold in memory; the least recently used answers go first beyond it.
pub const MAX_BYTES: usize = 24 << 20;

/// One remembered answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub etag: String,
    pub body: Arc<str>,
    /// The `Link` header (pages), which a 304 may leave out.
    pub link: Option<String>,
}

#[derive(Default)]
struct Store {
    generation: u64,
    entries: HashMap<String, (Entry, u64)>,
    bytes: usize,
    clock: u64,
    /// Conditional requests answered 304 since the start (for tests and `REPOHUB_TRACE`).
    reused: u64,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Store::default))
}

/// Whether answers from `path` may be asked for again with their tag: not searches.
pub fn conditional(path: &str) -> bool {
    !path.starts_with("/search/")
}

/// The key of one answer: the API, what was accepted and the path with its query.
pub fn key(api: &str, accept: &str, path: &str) -> String {
    format!("{api} {accept} {path}")
}

/// Counts the forgettings: a `Gh` made before the last [`clear`] can't put answers back.
pub fn generation() -> u64 {
    with(|s| s.generation)
}

pub fn get(key: &str) -> Option<Entry> {
    with(|s| {
        s.clock += 1;
        let now = s.clock;
        s.entries.get_mut(key).map(|(e, used)| {
            *used = now;
            e.clone()
        })
    })
}

/// Remembers an answer, unless the store was cleared since `generation`.
pub fn put(generation: u64, key: String, entry: Entry) {
    with(|s| {
        if s.generation != generation {
            return;
        }
        s.clock += 1;
        let size = key.len() + entry.body.len() + entry.etag.len();
        if size > MAX_BYTES / 4 {
            return;
        }
        if let Some((old, _)) = s.entries.insert(key.clone(), (entry, s.clock)) {
            s.bytes -= key.len() + old.body.len() + old.etag.len();
        }
        s.bytes += size;
        while s.bytes > MAX_BYTES {
            let Some(oldest) = s.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone()) else { break };
            if let Some((e, _)) = s.entries.remove(&oldest) {
                s.bytes -= oldest.len() + e.body.len() + e.etag.len();
            }
        }
    });
}

/// Counts a 304 that reused a kept answer.
pub fn reused() {
    with(|s| s.reused += 1);
}

/// How many 304s reused a kept answer since the start.
#[allow(dead_code)]
pub fn reused_count() -> u64 {
    with(|s| s.reused)
}

/// Forgets every answer (sign-out, another account, Clear saved data).
pub fn clear() {
    with(|s| {
        s.generation += 1;
        s.entries.clear();
        s.bytes = 0;
    });
}

/// The answers kept, most recently used first, up to `budget` bytes of bodies (for the saved
/// copy).
pub fn export(budget: usize) -> Vec<(String, Entry)> {
    with(|s| {
        let mut all: Vec<(&String, &(Entry, u64))> = s.entries.iter().collect();
        all.sort_by_key(|(_, (_, used))| std::cmp::Reverse(*used));
        let mut left = budget;
        let mut out = Vec::new();
        for (k, (e, _)) in all {
            let size = k.len() + e.body.len() + e.etag.len();
            if size > left {
                continue;
            }
            left -= size;
            out.push((k.clone(), e.clone()));
        }
        out
    })
}

/// Takes the answers of a saved copy (only into an empty store of the same generation: a load
/// that already ran knows better).
pub fn import(generation: u64, entries: Vec<(String, Entry)>) {
    for (k, e) in entries.into_iter().rev() {
        put(generation, k, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_kept_by_key_and_forgotten_on_clear() {
        // The store is shared by the whole process; tests that clear it (signing out) hold this.
        let _l = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let g = generation();
        let k = key("https://api.github.com", "application/json", "/user/repos?page=1");
        put(g, k.clone(), Entry { etag: "W/\"1\"".into(), body: "[1]".into(), link: Some("<x>; rel=\"next\"".into()) });
        assert_eq!(get(&k).map(|e| e.etag), Some("W/\"1\"".into()));
        assert!(get(&key("https://other.example", "application/json", "/user/repos?page=1")).is_none(), "another API is another key");
        clear();
        assert!(get(&k).is_none(), "cleared");
        // An answer from before the clear doesn't come back.
        put(g, k.clone(), Entry { etag: "W/\"2\"".into(), body: "[2]".into(), link: None });
        assert!(get(&k).is_none(), "a late answer of the previous account is dropped");
        assert!(!conditional("/search/issues?q=x") && conditional("/user/repos"));
    }

    #[test]
    fn export_keeps_the_most_recent_within_the_budget() {
        // The store is shared by the whole process; tests that clear it (signing out) hold this.
        let _l = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let g = generation();
        for i in 0..5 {
            put(g, format!("k{i}"), Entry { etag: "t".into(), body: "x".repeat(100).into(), link: None });
        }
        let _ = get("k1");
        let out = export(250);
        assert_eq!(out.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["k1", "k4"], "most recently used first, within the budget");
        clear();
        import(generation(), out);
        assert!(get("k1").is_some() && get("k4").is_some() && get("k0").is_none());
        clear();
    }
}
