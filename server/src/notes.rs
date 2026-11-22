//! Local notes: Excalidraw drawings stored on disk per repository and branch.
//!
//! Files live at `DATA_DIR/notes/<owner>/<repo>/<branch>/<name>.excalidraw`, each segment
//! percent-encoded so a branch such as `feature/x` stays one folder. A deleted drawing moves to
//! `DATA_DIR/trash/<date>/…` and stays there 30 days. The server itself sends nothing to GitHub:
//! the app syncs boards with the user's notes repository through the `/gh` proxy, and keeps what
//! it last synced here (`notes-sync/`).
//!
//! - `GET    /api/notes?repo=owner/name&branch=b` → `[{"name","updated","size"}]`, newest first
//! - `GET    /api/notes/all` → every drawing: `[{"repo","branch","name","sha","updated","size"}]`
//!   (`sha`: the git blob id of the file)
//! - `GET    /api/notes/file?repo=…&branch=…&name=…` → the drawing (Excalidraw JSON)
//! - `PUT    /api/notes/file?…[&if=<sha>|none]` with the drawing as the body → saves it; with
//!   `if`, only when the file still has that blob id (or doesn't exist), else 409
//! - `DELETE /api/notes/file?…[&if=<sha>]` → moves it to the trash (409 as above)
//! - `GET|PUT|DELETE /api/notes/index?repo=owner/name` → sync's index for that notes repository
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tiny_http::{Method, Request, Response};

/// Largest drawing accepted (embedded images make files big).
const MAX_BYTES: u64 = 25 << 20;

/// Largest sync index accepted.
const MAX_INDEX_BYTES: u64 = 4 << 20;

/// One read-compare-write at a time, so a conditional write can't race another.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// SHA-1, for git blob ids (not for anything secret).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// The git blob id of a file's content (what GitHub calls its `sha`).
pub fn blob_sha(content: &[u8]) -> String {
    let mut data = format!("blob {}\0", content.len()).into_bytes();
    data.extend_from_slice(content);
    sha1(&data).iter().map(|b| format!("{b:02x}")).collect()
}

/// `2026-11-22` for Unix seconds `t` (UTC).
fn date(t: i64) -> String {
    let z = t.div_euclid(86_400) + 719_468;
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

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Moves `file` (inside `data/notes`) to `data/trash/<today>/…`, a taken name getting a
/// number, and removes trash folders older than 30 days.
fn trash(data: &Path, file: &Path) -> std::io::Result<()> {
    let notes = data.join("notes");
    let rel = file.strip_prefix(&notes).map_err(|_| std::io::Error::other("not a drawing"))?.to_path_buf();
    let day = data.join("trash").join(date(now()));
    let mut dest = day.join(&rel);
    let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut n = 2;
    while dest.exists() {
        dest = day.join(rel.with_file_name(format!("{stem} {n}.excalidraw")));
        n += 1;
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::rename(file, &dest)?;
    let cutoff = date(now() - 30 * 86_400);
    for e in std::fs::read_dir(data.join("trash")).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let dated = name.len() == 10 && name.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
        if dated && name < cutoff {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(())
}

fn sorted_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
    v.sort();
    v
}

/// Every drawing under `data/notes`, as `GET /api/notes/all` lists it.
fn list_all(data: &Path) -> String {
    let mut out = Vec::new();
    for (owner, o) in sorted_dirs(&data.join("notes")).into_iter().filter(|e| e.1.is_dir()) {
        for (repo, r) in sorted_dirs(&o).into_iter().filter(|e| e.1.is_dir()) {
            for (branch, b) in sorted_dirs(&r).into_iter().filter(|e| e.1.is_dir()) {
                for (file, f) in sorted_dirs(&b) {
                    let Some(stem) = file.strip_suffix(".excalidraw") else { continue };
                    let (Ok(bytes), Ok(meta)) = (std::fs::read(&f), std::fs::metadata(&f)) else { continue };
                    let updated = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                    out.push(format!(
                        "{{\"repo\":{},\"branch\":{},\"name\":{},\"sha\":\"{}\",\"updated\":{updated},\"size\":{}}}",
                        json_str(&format!("{}/{}", strict_decode(&owner), strict_decode(&repo))), json_str(&strict_decode(&branch)), json_str(&strict_decode(stem)), blob_sha(&bytes), bytes.len()
                    ));
                }
            }
        }
    }
    format!("[{}]", out.join(","))
}

/// Decodes a folder or file name written by `segment` (`+` stays a `+`).
fn strict_decode(s: &str) -> String {
    percent_decode(&s.replace('+', "%2B"))
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One path segment: keeps `[A-Za-z0-9_.-]`, encodes everything else (`/` included).
fn segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') || (b == b'.' && !out.is_empty()) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn query(url: &str) -> Vec<(String, String)> {
    url.split_once('?').map(|(_, q)| q).unwrap_or("").split('&').filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn valid_repo(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.len() <= 100 && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !p.starts_with('.'));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

fn valid_branch(b: &str) -> bool {
    !b.is_empty() && b.len() <= 255 && !b.chars().any(char::is_control)
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 80 && !n.starts_with('.') && n.chars().all(|c| c.is_alphanumeric() || " _-.()".contains(c))
}

fn dir(data: &Path, repo: &str, branch: &str) -> PathBuf {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
    data.join("notes").join(segment(&owner.to_ascii_lowercase())).join(segment(&name.to_ascii_lowercase())).join(segment(branch))
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn reply(req: Request, code: u16, ctype: &str, body: Vec<u8>) {
    let resp = Response::from_data(body).with_status_code(code)
        .with_header(tiny_http::Header::from_bytes("Content-Type", ctype).unwrap())
        .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = req.respond(resp);
}

fn text(req: Request, code: u16, msg: &str) {
    reply(req, code, "text/plain; charset=utf-8", msg.as_bytes().to_vec());
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}


pub fn handle(data: &Path, allow_writes: bool, max_bytes: u64, mut req: Request) {
    let url = req.url().to_owned();
    let q = query(&url);
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let path_only = url.split('?').next().unwrap_or("");
    if path_only == "/api/notes/all" {
        if *req.method() != Method::Get {
            return text(req, 405, "Method not allowed");
        }
        return reply(req, 200, "application/json", list_all(data).into_bytes());
    }
    if path_only == "/api/notes/index" {
        let repo = get("repo");
        if !valid_repo(&repo) {
            return text(req, 400, "repo (owner/name) is required");
        }
        let file = data.join("notes-sync").join(format!("{}.json", segment(&repo.to_ascii_lowercase())));
        return match req.method() {
            Method::Get => match std::fs::read(&file) {
                Ok(b) => reply(req, 200, "application/json", b),
                Err(_) => text(req, 404, "Nothing synced yet"),
            },
            Method::Put | Method::Delete if !allow_writes => text(req, 403, "Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true)"),
            Method::Put => {
                let mut body = Vec::new();
                if req.as_reader().take(MAX_INDEX_BYTES + 1).read_to_end(&mut body).is_err() || body.len() as u64 > MAX_INDEX_BYTES || !body.starts_with(b"{") {
                    return text(req, 400, "Expected a sync index (JSON, 4 MB at most)");
                }
                let tmp = file.with_extension("json.tmp");
                let saved = std::fs::create_dir_all(data.join("notes-sync")).and_then(|_| std::fs::write(&tmp, &body)).and_then(|_| std::fs::rename(&tmp, &file));
                match saved {
                    Ok(()) => text(req, 204, ""),
                    Err(e) => text(req, 500, &format!("Couldn't save the sync index: {e}")),
                }
            }
            Method::Delete => {
                let _ = std::fs::remove_file(&file);
                text(req, 204, "")
            }
            _ => text(req, 405, "Method not allowed"),
        };
    }
    let (repo, branch) = (get("repo"), get("branch"));
    if !valid_repo(&repo) || !valid_branch(&branch) {
        return text(req, 400, "repo (owner/name) and branch are required");
    }
    let folder = dir(data, &repo, &branch);
    if path_only == "/api/notes" {
        if *req.method() != Method::Get {
            return text(req, 405, "Method not allowed");
        }
        let mut items: Vec<(String, u64, u64)> = std::fs::read_dir(&folder).into_iter().flatten().flatten().filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            let name = file.strip_suffix(".excalidraw")?;
            let meta = e.metadata().ok()?;
            let updated = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((percent_decode(name), updated, meta.len()))
        }).collect();
        items.sort_by_key(|x| std::cmp::Reverse(x.1));
        let body = format!("[{}]", items.iter().map(|(n, u, s)| format!("{{\"name\":{},\"updated\":{u},\"size\":{s}}}", json_str(n))).collect::<Vec<_>>().join(","));
        return reply(req, 200, "application/json", body.into_bytes());
    }
    if path_only != "/api/notes/file" {
        return text(req, 404, "Not found");
    }
    let name = get("name");
    if !valid_name(&name) {
        return text(req, 400, "Invalid name: use letters, digits, spaces, _ - . ( ), at most 80 characters");
    }
    let file = folder.join(format!("{}.excalidraw", segment(&name)));
    match req.method() {
        Method::Get => match std::fs::read(&file) {
            Ok(b) => reply(req, 200, "application/json", b),
            Err(_) => text(req, 404, "No such drawing"),
        },
        Method::Put => {
            if !allow_writes {
                return text(req, 403, "Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true)");
            }
            if req.body_length().is_some_and(|n| n as u64 > MAX_BYTES) {
                return text(req, 413, "Drawing too large (25 MB at most)");
            }
            let mut body = Vec::new();
            if req.as_reader().take(MAX_BYTES + 1).read_to_end(&mut body).is_err() || body.len() as u64 > MAX_BYTES {
                return text(req, 413, "Drawing too large (25 MB at most)");
            }
            if !body.starts_with(b"{") {
                return text(req, 400, "Expected an Excalidraw JSON document");
            }
            // `if`: only when the file still has this blob id ("none": only when there is none).
            let cond = q.iter().find(|(k, _)| k == "if").map(|(_, v)| v.clone());
            let _guard = lock();
            if let Some(want) = &cond {
                let now = std::fs::read(&file).ok().map(|b| blob_sha(&b));
                if now.as_deref() != (want != "none").then_some(want.as_str()) {
                    return text(req, 409, "The drawing changed meanwhile");
                }
            }
            let notes_root = data.join("notes");
            let cur_total = dir_size(&notes_root);
            let existing_len = file.metadata().map(|m| m.len()).unwrap_or(0);
            let limit = max_bytes;
            if cur_total.saturating_sub(existing_len) + (body.len() as u64) > limit {
                let mb = limit >> 20;
                let msg = if mb > 0 {
                    format!("Total notes storage limit exceeded ({mb} MB at most)")
                } else {
                    "Total notes storage limit exceeded".into()
                };
                return text(req, 413, &msg);
            }
            static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp = file.with_extension(format!("tmp.{}.{}", std::process::id(), id));
            let saved = std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&tmp, &body)).and_then(|_| std::fs::rename(&tmp, &file));
            if saved.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            match saved {
                Ok(()) => text(req, 204, ""),
                Err(e) => text(req, 500, &format!("Couldn't save the drawing: {e}")),
            }
        }
        Method::Delete => {
            if !allow_writes {
                return text(req, 403, "Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true)");
            }
            let _guard = lock();
            let Ok(bytes) = std::fs::read(&file) else { return text(req, 404, "No such drawing") };
            if let Some((_, want)) = q.iter().find(|(k, _)| k == "if")
                && blob_sha(&bytes) != *want
            {
                return text(req, 409, "The drawing changed meanwhile");
            }
            match trash(data, &file) {
                Ok(()) => text(req, 204, ""),
                Err(e) => text(req, 500, &format!("Couldn't delete the drawing: {e}")),
            }
        }
        _ => text(req, 405, "Method not allowed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert!(valid_repo("FrancescoDiBella/repohub"));
        assert!(!valid_repo("../etc") && !valid_repo("a/b/c") && !valid_repo("a/.git"));
        assert!(valid_name("Architecture (v2)") && !valid_name("../x") && !valid_name(".hidden") && !valid_name("a/b"));
        let d = dir(Path::new("/d"), "Octo/Repo", "feature/x");
        assert_eq!(d, Path::new("/d/notes/octo/repo/feature%2Fx"));
        assert_eq!(segment(".."), "%2E.");
        assert_eq!(percent_decode("feature%2Fx+y"), "feature/x y");
    }

    #[test]
    fn blob_ids_and_dates() {
        assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(blob_sha("a".repeat(1000).as_bytes()), blob_sha("a".repeat(1000).as_bytes()));
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(strict_decode("feature%2Fx%20y"), "feature/x y");
    }

    #[test]
    fn listing_everything_and_the_trash() {
        let data = std::env::temp_dir().join(format!("repohub-notes-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let folder = dir(&data, "Octo/App", "feature/x y");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Plan%20%28v2%29.excalidraw"), "hello\n").unwrap();
        std::fs::write(folder.join("ignored.txt"), "x").unwrap();
        let all = list_all(&data);
        assert!(all.contains("\"repo\":\"octo/app\",\"branch\":\"feature/x y\",\"name\":\"Plan (v2)\",\"sha\":\"ce013625030ba8dba906f756967f9e9ca394464a\""), "{all}");
        assert!(!all.contains("ignored"));
        let file = folder.join("Plan%20%28v2%29.excalidraw");
        trash(&data, &file).unwrap();
        assert!(!file.exists());
        let kept = data.join("trash").join(date(now())).join("octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw");
        assert!(kept.is_file(), "kept in the trash");
        // An old trash folder goes; a newer one stays.
        std::fs::create_dir_all(data.join("trash/2000-01-01/o")).unwrap();
        std::fs::write(&file, "again").unwrap();
        trash(&data, &file).unwrap();
        assert!(!data.join("trash/2000-01-01").exists());
        assert!(data.join("trash").join(date(now())).join("octo/app/feature%2Fx%20y/Plan%20%28v2%29 2.excalidraw").is_file());
        assert_eq!(list_all(&data), "[]");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn conditional_writes_the_trash_and_the_index_over_http() {
        let data = std::env::temp_dir().join(format!("repohub-notes-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr());
        let (srv, d) = (server.clone(), data.clone());
        std::thread::spawn(move || {
            for req in srv.incoming_requests() {
                handle(&d, true, 1 << 30, req);
            }
        });
        let agent = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
        let call = |method: &str, url: &str, body: &str| -> (u16, String) {
            let r = agent.request(method, &format!("{base}{url}"));
            let r = if body.is_empty() { r.call() } else { r.send_string(body) };
            match r {
                Ok(r) | Err(ureq::Error::Status(_, r)) => (r.status(), r.into_string().unwrap_or_default()),
                Err(e) => panic!("{e}"),
            }
        };
        let f = "/api/notes/file?repo=Octo%2FApp&branch=feature%2Fx&name=Plan";
        let (one, two) = ("{\"elements\":[1]}", "{\"elements\":[2]}");
        assert_eq!(call("PUT", &format!("{f}&if=none"), one).0, 204);
        assert_eq!(call("PUT", &format!("{f}&if=none"), two).0, 409, "it exists now");
        let sha = blob_sha(one.as_bytes());
        assert_eq!(call("PUT", &format!("{f}&if={}", "0".repeat(40)), two).0, 409, "another blob id");
        assert_eq!(call("PUT", &format!("{f}&if={sha}"), two).0, 204);
        assert_eq!(call("GET", f, "").1, two);
        let (code, all) = call("GET", "/api/notes/all", "");
        assert_eq!(code, 200);
        assert!(all.contains(&blob_sha(two.as_bytes())) && all.contains("\"branch\":\"feature/x\""), "{all}");
        assert_eq!(call("DELETE", &format!("{f}&if={sha}"), "").0, 409, "changed since");
        assert_eq!(call("DELETE", &format!("{f}&if={}", blob_sha(two.as_bytes())), "").0, 204);
        assert_eq!(call("GET", f, "").0, 404);
        assert!(data.join("trash").join(date(now())).join("octo/app/feature%2Fx/Plan.excalidraw").is_file(), "in the trash");
        // An unconditional save and delete still work as before.
        assert_eq!(call("PUT", f, one).0, 204);
        assert_eq!(call("DELETE", f, "").0, 204);
        // The sync index.
        let i = "/api/notes/index?repo=me%2Frepohub-notes";
        assert_eq!(call("GET", i, "").0, 404);
        assert_eq!(call("PUT", i, "{\"version\":1}").0, 204);
        assert_eq!(call("GET", i, ""), (200, "{\"version\":1}".into()));
        assert_eq!(call("PUT", i, "not json").0, 400);
        assert_eq!(call("GET", "/api/notes/index?repo=..%2Fetc", "").0, 400);
        assert_eq!(call("DELETE", i, "").0, 204);
        assert_eq!(call("GET", i, "").0, 404);
        assert_eq!(call("POST", "/api/notes/all", "x").0, 405);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn test_dir_size_calculation() {
        let tmp = std::env::temp_dir().join(format!("repohub-notes-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let folder = tmp.join("sub");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(tmp.join("a.txt"), "hello").unwrap(); // 5 bytes
        std::fs::write(folder.join("b.txt"), "world!").unwrap(); // 6 bytes
        assert_eq!(dir_size(&tmp), 11);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
