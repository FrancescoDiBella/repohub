//! Sending one static file from `WEB_DIR`, once `serve_static` has checked the path.
//!
//! - **Compression:** `scripts/web-build.sh --precompress` writes `app_bg.wasm.br` and `.gz`
//!   next to each file. A browser that accepts one (`Accept-Encoding`) gets it with
//!   `Content-Encoding`; every answer says `Vary: Accept-Encoding`, so a cache keeps them apart.
//! - **Caching:** a URL that carries the build's version (`app.js?v=…`, which web-build.sh writes
//!   into the page) never changes, so the browser may keep it for a year. Anything else, the page
//!   above all, is `no-cache`: checked each time, and with its `ETag` an unchanged file costs a
//!   `304 Not Modified` instead of the download.
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tiny_http::{Request, Response};

use super::{h, header, text};

/// The precompressed copies looked for, best first: the coding's name and the file's suffix.
const CODINGS: [(&str, &str); 2] = [("br", "br"), ("gzip", "gz")];

/// What a versioned URL may be kept for: a year, without asking again.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// True when an `Accept-Encoding` header allows `coding`: named with a weight above zero, or
/// covered by `*` and not refused by name.
pub(crate) fn accepts(accept: &str, coding: &str) -> bool {
    let mut star = false;
    for item in accept.split(',') {
        let mut parts = item.split(';');
        let name = parts.next().unwrap_or("").trim();
        let q = parts
            .filter_map(|p| p.trim().strip_prefix("q=").or_else(|| p.trim().strip_prefix("Q=")))
            .map(|v| v.trim().parse::<f32>().unwrap_or(0.0))
            .next()
            .unwrap_or(1.0);
        if name.eq_ignore_ascii_case(coding) {
            return q > 0.0;
        }
        if name == "*" {
            star = q > 0.0;
        }
    }
    star
}

/// `Cache-Control` for a request's query string: a year for `v=<letters and digits>`, else
/// `no-cache`.
pub(crate) fn cache_control(query: Option<&str>) -> &'static str {
    let versioned = query.is_some_and(|q| {
        q.split('&').any(|kv| kv.strip_prefix("v=").is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric())))
    });
    if versioned { IMMUTABLE } else { "no-cache" }
}

/// The file to send for `file` (canonical, inside `root`) and its coding: a precompressed copy
/// the browser accepts, if it is a regular file inside `root` (a link out of it is ignored), or
/// else the file itself.
pub(crate) fn choose(file: &Path, root: &Path, accept: Option<&str>) -> (PathBuf, Option<&'static str>) {
    for (coding, suffix) in CODINGS {
        if !accept.is_some_and(|a| accepts(a, coding)) {
            continue;
        }
        let mut name = file.as_os_str().to_owned();
        name.push(".");
        name.push(suffix);
        if let Ok(copy) = std::fs::canonicalize(PathBuf::from(name))
            && copy.starts_with(root)
            && copy.is_file()
        {
            return (copy, Some(coding));
        }
    }
    (file.to_owned(), None)
}

/// A validator for one file as stored: its length and modification time. Each copy has its own,
/// as each is a different body.
fn etag(meta: &std::fs::Metadata) -> String {
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).unwrap_or_default();
    format!("\"{:x}-{:x}-{:x}\"", meta.len(), modified.as_secs(), modified.subsec_nanos())
}

/// True when an `If-None-Match` header names `tag` (or is `*`); weak tags compare as equal.
fn matches(if_none_match: &str, tag: &str) -> bool {
    if_none_match.split(',').map(|t| t.trim()).any(|t| t == "*" || t.trim_start_matches("W/") == tag)
}

/// Answers `req` with `file`, which `serve_static` found inside `root`.
pub(crate) fn respond(req: Request, file: &Path, root: &Path, content_type: &str) {
    let accept = header(&req, "Accept-Encoding");
    let (path, coding) = choose(file, root, accept.as_deref());
    let Ok(body) = std::fs::File::open(&path) else {
        let _ = req.respond(text(404, "Not found"));
        return;
    };
    let Ok(meta) = body.metadata() else {
        let _ = req.respond(text(404, "Not found"));
        return;
    };
    let tag = etag(&meta);
    let cache = cache_control(req.url().split_once('?').map(|(_, q)| q));
    let common = [h("Cache-Control", cache), h("Vary", "Accept-Encoding"), h("ETag", &tag)];
    if header(&req, "If-None-Match").is_some_and(|v| matches(&v, &tag)) {
        let mut resp = Response::empty(304);
        for x in common {
            resp.add_header(x);
        }
        let _ = req.respond(resp);
        return;
    }
    let mut resp = Response::from_file(body).with_header(h("Content-Type", content_type));
    if let Some(coding) = coding {
        resp.add_header(h("Content-Encoding", coding));
    }
    for x in common {
        resp.add_header(x);
    }
    let _ = req.respond(resp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_encoding() {
        let header = "gzip, deflate, br, zstd";
        assert!(accepts(header, "br") && accepts(header, "gzip"));
        assert!(accepts("BR", "br"));
        assert!(accepts("gzip;q=0.5", "gzip"));
        assert!(accepts(" gzip ; q=1.0 ", "gzip"));
        // Refused by weight, by name, or never named.
        assert!(!accepts("gzip;q=0, br", "gzip"));
        assert!(!accepts("br;q=0.000", "br"));
        assert!(!accepts("gzip;q=abc", "gzip"));
        assert!(!accepts("identity", "gzip"));
        assert!(!accepts("", "br"));
        assert!(!accepts("x-br", "br"));
        assert!(!accepts("brotli", "br"));
        // `*` covers what isn't named, but not what is refused by name.
        assert!(accepts("*", "br"));
        assert!(!accepts("*;q=0", "br"));
        assert!(!accepts("*, br;q=0", "br"));
        assert!(accepts("*, br;q=0", "gzip"));
    }

    #[test]
    fn cache_policy() {
        assert_eq!(cache_control(None), "no-cache");
        assert_eq!(cache_control(Some("v=3f2a9c01d4e5b6a7")), IMMUTABLE);
        assert_eq!(cache_control(Some("x=1&v=abc")), IMMUTABLE);
        assert_eq!(cache_control(Some("")), "no-cache");
        assert_eq!(cache_control(Some("v=")), "no-cache");
        assert_eq!(cache_control(Some("version=1")), "no-cache");
        assert_eq!(cache_control(Some("v=../x")), "no-cache");
        assert_eq!(cache_control(Some("xv=1")), "no-cache");
    }

    #[test]
    fn if_none_match() {
        assert!(matches("\"a-1-0\"", "\"a-1-0\""));
        assert!(matches("W/\"a-1-0\"", "\"a-1-0\""));
        assert!(matches("\"x\", \"a-1-0\"", "\"a-1-0\""));
        assert!(matches("*", "\"a-1-0\""));
        assert!(!matches("\"a-1-1\"", "\"a-1-0\""));
        assert!(!matches("", "\"a-1-0\""));
    }

    #[test]
    fn picks_the_copy_the_browser_accepts() {
        let dir = std::env::temp_dir().join(format!("repohub-files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(&web).unwrap();
        let root = std::fs::canonicalize(&web).unwrap();
        for (name, body) in [("app.wasm", "raw"), ("app.wasm.br", "br"), ("app.wasm.gz", "gz"), ("app.js", "js"), ("app.js.gz", "jsgz")] {
            std::fs::write(root.join(name), body).unwrap();
        }
        let wasm = root.join("app.wasm");
        assert_eq!(choose(&wasm, &root, Some("gzip, deflate, br")), (root.join("app.wasm.br"), Some("br")));
        assert_eq!(choose(&wasm, &root, Some("gzip")), (root.join("app.wasm.gz"), Some("gzip")));
        assert_eq!(choose(&wasm, &root, Some("br;q=0, gzip")), (root.join("app.wasm.gz"), Some("gzip")));
        assert_eq!(choose(&wasm, &root, Some("identity")), (wasm.clone(), None));
        assert_eq!(choose(&wasm, &root, None), (wasm.clone(), None));
        // Only the copies that exist.
        let js = root.join("app.js");
        assert_eq!(choose(&js, &root, Some("br, gzip")), (root.join("app.js.gz"), Some("gzip")));
        // A copy that is a link out of the folder, or a folder, is not sent.
        std::fs::write(dir.join("outside"), "secret").unwrap();
        std::fs::write(root.join("page.html"), "page").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("outside"), root.join("page.html.br")).unwrap();
        std::fs::create_dir_all(root.join("page.html.gz")).unwrap();
        let page = root.join("page.html");
        assert_eq!(choose(&page, &root, Some("br, gzip")), (page.clone(), None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Through the whole server: negotiation, headers, versioned URLs and 304.
    #[test]
    fn serves_compressed_and_cached() {
        let dir = std::env::temp_dir().join(format!("repohub-files-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(web.join("index.html"), "<h1>RepoHub</h1>").unwrap();
        std::fs::write(web.join("app_bg.wasm"), b"\0asm raw module").unwrap();
        std::fs::write(web.join("app_bg.wasm.br"), b"brotli bytes").unwrap();
        std::fs::write(web.join("app_bg.wasm.gz"), b"gzip bytes").unwrap();
        let cfg = std::sync::Arc::new(crate::Cfg {
            token: None,
            api: "http://127.0.0.1:9".into(),
            web_dir: web.clone(),
            data_dir: dir.join("data"),
            allowed_hosts: vec!["127.0.0.1".into()],
            public_json: "{}".into(),
            agent: ureq::AgentBuilder::new().try_proxy_from_env(false).build(),
            secret: None,
            allow_writes: false,
            notes_max_bytes: 1 << 20,
        });
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr());
        let (s, c) = (server.clone(), cfg.clone());
        std::thread::spawn(move || {
            for req in s.incoming_requests() {
                crate::handle(&c, req);
            }
        });
        let client = ureq::AgentBuilder::new().try_proxy_from_env(false).timeout(std::time::Duration::from_secs(5)).build();
        let get = |path: &str, headers: &[(&str, &str)]| {
            let mut r = client.get(&format!("{base}{path}"));
            for (k, v) in headers {
                r = r.set(k, v);
            }
            match r.call() {
                Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
                Err(e) => panic!("request failed: {e}"),
            }
        };
        let body = |resp: ureq::Response| {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut resp.into_reader(), &mut out).unwrap();
            out
        };

        // Brotli when accepted, gzip when only that is, the file itself otherwise; always the
        // .wasm's own type, and Vary.
        let versioned = "/app_bg.wasm?v=0123456789abcdef";
        let r = get(versioned, &[("Accept-Encoding", "gzip, deflate, br")]);
        assert_eq!(r.status(), 200);
        assert_eq!(r.header("Content-Encoding"), Some("br"));
        assert_eq!(r.header("Content-Type"), Some("application/wasm"));
        assert_eq!(r.header("Vary"), Some("Accept-Encoding"));
        assert_eq!(r.header("Cache-Control"), Some(IMMUTABLE));
        assert_eq!(body(r), b"brotli bytes");
        let r = get(versioned, &[("Accept-Encoding", "gzip")]);
        assert_eq!(r.header("Content-Encoding"), Some("gzip"));
        assert_eq!(body(r), b"gzip bytes");
        let r = get(versioned, &[]);
        assert_eq!(r.header("Content-Encoding"), None);
        assert_eq!(r.header("Vary"), Some("Accept-Encoding"));
        assert_eq!(body(r), b"\0asm raw module");

        // Without a version: checked each time, and unchanged costs a 304, per copy.
        let r = get("/app_bg.wasm", &[("Accept-Encoding", "br")]);
        assert_eq!(r.header("Cache-Control"), Some("no-cache"));
        let tag = r.header("ETag").expect("an ETag").to_owned();
        let r = get("/app_bg.wasm", &[("Accept-Encoding", "br"), ("If-None-Match", &tag)]);
        assert_eq!(r.status(), 304);
        assert_eq!(r.header("ETag"), Some(tag.as_str()));
        assert!(body(r).is_empty());
        // The plain file has another tag, so the brotli one doesn't match it.
        let r = get("/app_bg.wasm", &[("If-None-Match", &tag)]);
        assert_eq!(r.status(), 200);
        assert_eq!(body(r), b"\0asm raw module");

        // The page is never kept without asking.
        let r = get("/", &[("Accept-Encoding", "br, gzip")]);
        assert_eq!(r.status(), 200);
        assert_eq!(r.header("Cache-Control"), Some("no-cache"));
        assert_eq!(r.header("Content-Type"), Some("text/html; charset=utf-8"));
        assert_eq!(r.header("Content-Encoding"), None);
        assert_eq!(body(r), b"<h1>RepoHub</h1>");
        let r = get("/index.html?v=../../etc", &[]);
        assert_eq!(r.header("Cache-Control"), Some("no-cache"));

        // A missing file is still a 404.
        assert_eq!(get("/missing.js", &[("Accept-Encoding", "br")]).status(), 404);

        server.unblock();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
