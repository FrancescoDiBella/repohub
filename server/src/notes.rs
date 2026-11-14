//! Local notes: Excalidraw drawings stored on disk per repository and branch.
//!
//! Files live at `DATA_DIR/notes/<owner>/<repo>/<branch>/<name>.excalidraw`, each segment
//! percent-encoded so a branch such as `feature/x` stays one folder. Nothing is sent to GitHub.
//!
//! - `GET    /api/notes?repo=owner/name&branch=b` → `[{"name","updated","size"}]`, newest first
//! - `GET    /api/notes/file?repo=…&branch=…&name=…` → the drawing (Excalidraw JSON)
//! - `PUT    /api/notes/file?…` with the drawing as the body → saves it
//! - `DELETE /api/notes/file?…` → removes it
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tiny_http::{Method, Request, Response};

/// Largest drawing accepted (embedded images make files big).
const MAX_BYTES: u64 = 25 << 20;

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

pub fn handle(data: &Path, mut req: Request) {
    let url = req.url().to_owned();
    let q = query(&url);
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let (repo, branch) = (get("repo"), get("branch"));
    if !valid_repo(&repo) || !valid_branch(&branch) {
        return text(req, 400, "repo (owner/name) and branch are required");
    }
    let folder = dir(data, &repo, &branch);
    let path_only = url.split('?').next().unwrap_or("");
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
        Method::Delete => match std::fs::remove_file(&file) {
            Ok(()) => text(req, 204, ""),
            Err(_) => text(req, 404, "No such drawing"),
        },
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
}
