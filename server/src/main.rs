//! repohub-server: serves the RepoHub web build and proxies the GitHub API for it.
//!
//! - `GET /` and static files from `WEB_DIR`, brotli or gzip when precompressed copies are there
//!   (`files.rs`).
//! - `GET /config.json`: the non-secret settings (user, title, theme, pinned…) and `hasToken`.
//! - `/gh/<path>`: forwarded to `GITHUB_API_URL/<path>` with the token added, so the token stays
//!   in this process and never reaches the browser. Only the exact endpoints RepoHub uses are
//!   allowed, only `PUT`/`DELETE` on `/user/starred/…` and allowed issue/branch/PR creates and merges
//!   may write (when writes are enabled), and requests from other origins or with a foreign `Host`
//!   (DNS rebinding) are refused.
//!
//! Configuration comes from the environment (`docker run --env-file .env`, or `env_file` in
//! compose.yaml), with the same variables as the desktop app's `.env`.
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use tiny_http::{Header, Method, Request, Response, Server};

mod allowlist;
mod files;
mod notes;
mod proxy;
#[cfg(test)]
mod allowlist_cases;

use allowlist::{allowed, board_contents_path, branch_path};
use proxy::proxy;

struct Cfg {
    token: Option<String>,
    api: String,
    web_dir: PathBuf,
    data_dir: PathBuf,
    allowed_hosts: Vec<String>,
    public_json: String,
    agent: ureq::Agent,
    secret: Option<String>,
    allow_writes: bool,
    notes_max_bytes: u64,
    proxy_in_flight: std::sync::atomic::AtomicUsize,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().trim_matches('"').trim_matches('\'').to_owned()).filter(|v| !v.is_empty())
}

fn flag(k: &str, d: bool) -> bool {
    match env(k).map(|v| v.to_ascii_lowercase()).as_deref() {
        Some("1" | "true" | "yes" | "si" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => d,
    }
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

fn is_loopback(addr: &str) -> bool {
    if let Ok(ip) = addr.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    if addr.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let host = if let Some(stripped) = addr.strip_prefix('[') {
        stripped.split(']').next().unwrap_or(stripped)
    } else if let Some((h, _)) = addr.rsplit_once(':') {
        h
    } else {
        addr
    };
    let host = host.trim_matches('[').trim_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    false
}

impl Cfg {
    fn load() -> Result<Cfg, String> {
        let addr = env("REPOHUB_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into());
        Self::load_with_addr(&addr)
    }

    fn load_with_addr(addr: &str) -> Result<Cfg, String> {
        let loopback = is_loopback(addr);
        let secret = env("REPOHUB_SECRET");
        if !loopback && secret.is_none() {
            return Err(format!(
                "REPOHUB_SECRET must be set when binding to a non-loopback address ({addr}). Set REPOHUB_SECRET in .env or bind to 127.0.0.1."
            ));
        }
        let allow_writes = flag("REPOHUB_ALLOW_WRITES", loopback);
        let notes_max_bytes = env("REPOHUB_NOTES_MAX_MB")
            .and_then(|v| v.parse::<u64>().ok())
            .map(|mb| mb << 20)
            .unwrap_or(500 << 20);
        let token = env("GITHUB_TOKEN");
        let api = env("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_owned();
        let opt = |k: &str| env(k).map(|v| json_str(&v)).unwrap_or_else(|| "null".into());
        let pinned: Vec<String> = env("REPOHUB_PINNED").map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let num = |k: &str, d: u64| env(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
        let public_json = format!(
            "{{\"user\":{},\"hasToken\":{},\"api\":{},\"title\":{},\"theme\":{},\"pinned\":[{}],\"includeForks\":{},\"includeArchived\":{},\"maxRepos\":{},\"activityRepos\":{},\"refreshMinutes\":{},\"orgs\":{},\"demo\":{},\"notes\":true,\"allowWrites\":{}}}",
            opt("GITHUB_USER"),
            token.is_some(),
            json_str(&api),
            opt("REPOHUB_TITLE"),
            opt("REPOHUB_THEME"),
            pinned.iter().map(|p| json_str(p)).collect::<Vec<_>>().join(","),
            flag("REPOHUB_INCLUDE_FORKS", true),
            flag("REPOHUB_INCLUDE_ARCHIVED", false),
            num("REPOHUB_MAX_REPOS", 100),
            num("REPOHUB_ACTIVITY_REPOS", 5),
            num("REPOHUB_REFRESH_MINUTES", 10),
            opt("REPOHUB_ORGS"),
            flag("REPOHUB_DEMO", false),
            allow_writes,
        );
        let mut allowed_hosts: Vec<String> = ["localhost", "127.0.0.1", "[::1]"].iter().map(|s| s.to_string()).collect();
        if let Some(extra) = env("REPOHUB_ALLOWED_HOSTS") {
            allowed_hosts.extend(extra.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()));
        }
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(!bypass_proxy(&api))
            .max_idle_connections_per_host(8)
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("RepoHub server")
            .build();
        Ok(Cfg {
            token,
            api,
            web_dir: env("WEB_DIR").map(PathBuf::from).unwrap_or_else(|| "web".into()),
            data_dir: env("DATA_DIR").map(PathBuf::from).unwrap_or_else(|| "data".into()),
            allowed_hosts,
            public_json,
            agent,
            secret,
            allow_writes,
            notes_max_bytes,
            proxy_in_flight: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}

/// True when NO_PROXY (or a loopback host) says to reach `api` directly.
fn bypass_proxy(api: &str) -> bool {
    let host = api.split("://").nth(1).unwrap_or(api).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    if host == "localhost" || host.starts_with("127.") || host == "[::1]" {
        return true;
    }
    let list = env("NO_PROXY").or_else(|| env("no_proxy")).unwrap_or_default();
    list.split(',').map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty())
        .any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

fn header(r: &Request, name: &str) -> Option<String> {
    r.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_owned())
}

fn host_only(h: &str) -> String {
    let h = h.trim().to_ascii_lowercase();
    if h.starts_with('[') {
        return h.split(']').next().map(|x| format!("{x}]")).unwrap_or(h);
    }
    h.split(':').next().unwrap_or("").to_owned()
}

fn parse_host_port(s: &str, default_port: u16) -> Option<(String, u16)> {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix('[') {
        let (ipv6, rest) = stripped.split_once(']')?;
        let port = if let Some(p_str) = rest.strip_prefix(':') {
            p_str.parse::<u16>().ok()?
        } else {
            default_port
        };
        Some((format!("[{ipv6}]"), port))
    } else if let Some((h, p_str)) = s.split_once(':') {
        let port = p_str.parse::<u16>().ok()?;
        Some((h.to_ascii_lowercase(), port))
    } else {
        Some((s.to_ascii_lowercase(), default_port))
    }
}

fn origin_matches_host(origin: &str, host_header: &str) -> bool {
    let origin = origin.trim();
    if origin == "null" {
        return false;
    }
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    if rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return false;
    }
    let default_port = if scheme == "http" { 80 } else { 443 };
    let Some((o_host, o_port)) = parse_host_port(rest, default_port) else {
        return false;
    };
    let Some((h_host, h_port)) = parse_host_port(host_header, default_port) else {
        return false;
    };
    o_host == h_host && o_port == h_port
}

fn referer_origin(referer: &str) -> Option<&str> {
    let (scheme, rest) = referer.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let end = scheme.len() + 3 + authority.len();
    Some(&referer[..end])
}

fn percent_decode_path(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return None;
            }
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
            let val = u8::from_str_radix(hex, 16).ok()?;
            out.push(val);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn validate_and_decode_path(raw_path: &str) -> Option<String> {
    let raw_lower = raw_path.to_ascii_lowercase();
    if raw_lower.contains("%2e")
        || raw_lower.contains("%2f")
        || raw_lower.contains("%5c")
        || raw_path.contains('\\')
        || raw_path.chars().any(|c| c.is_control() || c == '\0')
    {
        return None;
    }

    let decoded = percent_decode_path(raw_path)?;
    if decoded.contains('\\')
        || decoded.contains('%')
        || decoded.chars().any(|c| c.is_control() || c == '\0')
    {
        return None;
    }

    let stripped = decoded.trim_start_matches('/');
    if stripped.is_empty() {
        return Some(decoded);
    }
    for seg in stripped.split('/') {
        if seg.is_empty() || seg.bytes().all(|b| b == b'.') {
            return None;
        }
    }
    Some(decoded)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn percent_encode_cookie(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn cookie_val(req: &Request, name: &str) -> Option<String> {
    let cookies = header(req, "Cookie")?;
    for part in cookies.split(';') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(name) {
            let val = v.trim();
            return percent_decode_path(val);
        }
    }
    None
}

fn is_authenticated(cfg: &Cfg, req: &Request) -> bool {
    let Some(secret) = &cfg.secret else {
        return true;
    };
    let s_bytes = secret.as_bytes();

    if let Some(h) = header(req, "X-RepoHub-Secret")
        && constant_time_eq(h.trim().as_bytes(), s_bytes)
    {
        return true;
    }

    if let Some(auth) = header(req, "Authorization")
        && let Some(token) = auth.trim().strip_prefix("Bearer ")
        && constant_time_eq(token.trim().as_bytes(), s_bytes)
    {
        return true;
    }

    if let Some(val) = cookie_val(req, "repohub_secret")
        && constant_time_eq(val.as_bytes(), s_bytes)
    {
        return true;
    }

    false
}

fn is_https(req: &Request) -> bool {
    header(req, "Origin").is_some_and(|o| o.starts_with("https://"))
        || header(req, "Referer").is_some_and(|r| r.starts_with("https://"))
        || header(req, "X-Forwarded-Proto").is_some_and(|p| p.eq_ignore_ascii_case("https"))
        || header(req, "X-Forwarded-Ssl").is_some_and(|s| s.eq_ignore_ascii_case("on"))
}

fn auth_cookie(secret: &str, https: bool) -> Header {
    let encoded = percent_encode_cookie(secret);
    let mut val = format!("repohub_secret={encoded}; Path=/; HttpOnly; SameSite=Strict");
    if https {
        val.push_str("; Secure");
    }
    h("Set-Cookie", &val)
}

fn signin_page(error: bool, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    let err_html = if error {
        "<div class=\"err\">Invalid secret. Please try again.</div>"
    } else {
        ""
    };
    let html = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>RepoHub · Sign in</title><style>body {{ font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif; background: #0d1117; color: #c9d1d9; display: flex; align-items: center; justify-content: center; min-height: 100vh; margin: 0; }} .card {{ background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 24px; width: 100%; max-width: 320px; box-shadow: 0 4px 16px rgba(0,0,0,0.5); }} h1 {{ font-size: 18px; margin: 0 0 16px; color: #f0f6fc; font-weight: 600; text-align: center; }} input[type=\"password\"] {{ width: 100%; box-sizing: border-box; padding: 8px 12px; background: #0d1117; border: 1px solid #30363d; border-radius: 6px; color: #c9d1d9; font-size: 14px; margin-bottom: 12px; outline: none; }} input[type=\"password\"]:focus {{ border-color: #58a6ff; }} button {{ width: 100%; padding: 8px 12px; background: #238636; color: #fff; border: none; border-radius: 6px; font-size: 14px; font-weight: 500; cursor: pointer; }} button:hover {{ background: #2ea043; }} .err {{ color: #f85149; font-size: 13px; margin-bottom: 12px; text-align: center; }}</style></head><body><div class=\"card\"><h1>RepoHub</h1>{err_html}<form method=\"POST\" action=\"/auth\"><input type=\"password\" name=\"secret\" placeholder=\"Enter REPOHUB_SECRET\" autofocus required /><button type=\"submit\">Unlock</button></form></div></body></html>"
    );
    Response::from_string(html).with_status_code(status).with_header(h("Content-Type", "text/html; charset=utf-8"))
}

fn extract_query_secret(url: &str) -> Option<String> {
    let (_, q) = url.split_once('?')?;
    for part in q.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if k == "secret" {
            return Some(notes::percent_decode(v));
        }
    }
    None
}

fn parse_json_secret(s: &str) -> Option<String> {
    let (_, after) = s.split_once("\"secret\"")?;
    let (_, after_colon) = after.split_once(':')?;
    let after_trim = after_colon.trim_start();
    if !after_trim.starts_with('"') {
        return None;
    }
    let inner = &after_trim[1..];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => {
                let next = chars.next()?;
                match next {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\x08'),
                    'f' => out.push('\x0c'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = (&mut chars).take(4).collect();
                        if hex.len() != 4 {
                            return None;
                        }
                        let code = u32::from_str_radix(&hex, 16).ok()?;
                        let ch = char::from_u32(code)?;
                        out.push(ch);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

fn parse_auth_secret(req: &mut Request) -> Option<String> {
    let mut body = Vec::new();
    let _ = req.as_reader().take(4096).read_to_end(&mut body);
    let s = std::str::from_utf8(&body).ok()?.trim();
    if s.starts_with('{') {
        parse_json_secret(s)
    } else {
        for pair in s.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k == "secret" {
                return Some(notes::percent_decode(v));
            }
        }
        None
    }
}

fn valid_content_type(req: &Request) -> bool {
    if let Some(ct) = header(req, "Content-Type") {
        let ct_lower = ct.to_ascii_lowercase();
        let mime = ct_lower.split(';').next().unwrap_or("").trim();
        mime == "application/json"
    } else {
        false
    }
}

fn is_write(m: &Method) -> bool {
    matches!(m, Method::Post | Method::Put | Method::Patch | Method::Delete)
}

fn check_origin_and_preflight(req: &Request, host_header: &str) -> Result<(), (u16, &'static str)> {
    if let Some(origin) = header(req, "Origin") {
        if !origin_matches_host(&origin, host_header) {
            return Err((403, "Origin not allowed"));
        }
    } else {
        // Critical browser detail: in browsers (WHATWG Fetch spec), Origin is omitted by
        // browsers on same-origin GET/HEAD fetch() while sending Sec-Fetch-Site: same-origin.
        // Over plain HTTP on a trusted LAN host, browsers omit Sec-Fetch-Site, sending Referer.
        let is_get = *req.method() == Method::Get || *req.method() == Method::Head;
        let has_repohub = header(req, "X-RepoHub").as_deref() == Some("1");

        let sec_fetch_ok = header(req, "Sec-Fetch-Site").as_deref() == Some("same-origin")
            && header(req, "Sec-Fetch-Mode").as_deref() != Some("navigate");

        let referer_ok = header(req, "Referer")
            .as_deref()
            .and_then(referer_origin)
            .is_some_and(|ref_orig| origin_matches_host(ref_orig, host_header));

        let is_browser_same_origin_get = is_get && has_repohub && (sec_fetch_ok || referer_ok);

        if !is_browser_same_origin_get {
            return Err((403, "Missing Origin header"));
        }
    }

    if is_write(req.method()) {
        if header(req, "X-RepoHub").as_deref() != Some("1") {
            return Err((403, "Writes require X-RepoHub: 1 header"));
        }
        if !valid_content_type(req) {
            return Err((403, "Writes require Content-Type: application/json"));
        }
    }

    Ok(())
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "css" => "text/css",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn h(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

fn text(code: u16, body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body).with_status_code(code).with_header(h("Content-Type", "text/plain; charset=utf-8"))
}

fn serve_static(cfg: &Cfg, req: Request) {
    let raw = req.url().split('?').next().unwrap_or("/").to_owned();
    if raw.contains(':') || raw.contains('\\') || raw.chars().any(|c| c.is_control() || c == '\0') {
        let _ = req.respond(text(404, "Not found"));
        return;
    }
    let raw_lower = raw.to_ascii_lowercase();
    if raw_lower.contains("%2e") || raw_lower.contains("%2f") || raw_lower.contains("%5c") {
        let _ = req.respond(text(404, "Not found"));
        return;
    }
    let Some(decoded) = percent_decode_path(&raw) else {
        let _ = req.respond(text(404, "Not found"));
        return;
    };
    if decoded.contains(':')
        || decoded.contains('\\')
        || decoded.contains('%')
        || decoded.chars().any(|c| c.is_control() || c == '\0')
    {
        let _ = req.respond(text(404, "Not found"));
        return;
    }
    let rel = decoded.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|seg| seg.starts_with('.') || (!seg.is_empty() && seg.bytes().all(|b| b == b'.'))) {
        let _ = req.respond(text(404, "Not found"));
        return;
    }
    let Ok(canon_web) = std::fs::canonicalize(&cfg.web_dir) else {
        let _ = req.respond(text(404, "Not found"));
        return;
    };
    let path = cfg.web_dir.join(rel);
    let Ok(canon_file) = std::fs::canonicalize(&path) else {
        let _ = req.respond(text(404, "Not found"));
        return;
    };
    if !canon_file.starts_with(&canon_web) || !canon_file.is_file() {
        let _ = req.respond(text(404, "Not found"));
        return;
    }
    // A precompressed copy when the browser takes one, and the caching headers (files.rs).
    files::respond(req, &canon_file, &canon_web, mime(&canon_file));
}

fn handle(cfg: &Cfg, mut req: Request) {
    // Refuse a foreign Host (DNS rebinding) and cross-site callers, so no other page can use the token.
    let host = header(&req, "Host").unwrap_or_default();
    let h_only = host_only(&host);
    if !cfg.allowed_hosts.contains(&h_only) {
        let _ = req.respond(text(403, "Host not allowed: add it to REPOHUB_ALLOWED_HOSTS"));
        return;
    }

    // Secret enforcement if REPOHUB_SECRET is configured
    if let Some(secret) = &cfg.secret {
        if req.url().split('?').next() == Some("/auth") && *req.method() == Method::Post {
            let origin = header(&req, "Origin").unwrap_or_default();
            if !origin_matches_host(&origin, &host) {
                let _ = req.respond(text(403, "Origin not allowed"));
                return;
            }
            let candidate = parse_auth_secret(&mut req).unwrap_or_default();
            if constant_time_eq(candidate.as_bytes(), secret.as_bytes()) {
                let resp = Response::empty(303)
                    .with_header(h("Location", "/"))
                    .with_header(auth_cookie(secret, is_https(&req)));
                let _ = req.respond(resp);
            } else {
                let _ = req.respond(signin_page(true, 401));
            }
            return;
        }

        if !is_authenticated(cfg, &req) {
            let path_only = req.url().split('?').next().unwrap_or("/");
            if path_only == "/" && *req.method() == Method::Get {
                if let Some(query_sec) = extract_query_secret(req.url()) {
                    if constant_time_eq(query_sec.as_bytes(), secret.as_bytes()) {
                        let resp = Response::empty(303)
                            .with_header(h("Location", "/"))
                            .with_header(auth_cookie(secret, is_https(&req)));
                        let _ = req.respond(resp);
                        return;
                    } else {
                        let _ = req.respond(signin_page(true, 401));
                        return;
                    }
                }
                let _ = req.respond(signin_page(false, 200));
                return;
            }
            let _ = req.respond(text(401, "Unauthorized: REPOHUB_SECRET required"));
            return;
        }
    }

    let url = req.url().to_owned();
    if url.starts_with("/gh/") || url.starts_with("/api/notes") {
        if let Err((code, msg)) = check_origin_and_preflight(&req, &host) {
            let _ = req.respond(text(code, msg));
            return;
        }
        if is_write(req.method()) && !cfg.allow_writes {
            let _ = req.respond(text(403, "Writes are disabled on this server (set REPOHUB_ALLOW_WRITES=true)"));
            return;
        }
    }

    if url.starts_with("/gh/") {
        proxy(cfg, req);
    } else if url.starts_with("/api/notes") {
        notes::handle(&cfg.data_dir, cfg.allow_writes, cfg.notes_max_bytes, req);
    } else if url.split('?').next() == Some("/config.json") {
        let resp = Response::from_string(cfg.public_json.clone())
            .with_header(h("Content-Type", "application/json"))
            .with_header(h("Cache-Control", "no-store"));
        let _ = req.respond(resp);
    } else if *req.method() == Method::Get || *req.method() == Method::Head {
        serve_static(cfg, req);
    } else {
        let _ = req.respond(text(405, "Method not allowed"));
    }
}

fn main() {
    let cfg = match Cfg::load() {
        Ok(c) => std::sync::Arc::new(c),
        Err(e) => {
            eprintln!("repohub-server: configuration error: {e}");
            std::process::exit(1);
        }
    };
    let addr = env("REPOHUB_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into());
    let server = Server::http(&addr).unwrap_or_else(|e| {
        eprintln!("repohub-server: can't listen on {addr}: {e}");
        std::process::exit(1);
    });
    let port = addr.rsplit(':').next().unwrap_or("8080");
    eprintln!("repohub-server: listening on {addr}. Open http://localhost:{port}");
    eprintln!(
        "repohub-server: GitHub {} · token {} · static files from {} · notes in {} · writes {} · auth {}",
        cfg.api,
        if cfg.token.is_some() { "set" } else { "not set (public data only)" },
        cfg.web_dir.display(),
        cfg.data_dir.display(),
        if cfg.allow_writes { "enabled" } else { "disabled" },
        if cfg.secret.is_some() { "secret required" } else { "none (loopback)" },
    );
    let server = std::sync::Arc::new(server);
    // 24 workers with at most 16 concurrent GitHub proxy calls (proxy::MAX_IN_FLIGHT) leaves at
    // least 8 threads always free for static assets, config.json and /api/notes.
    let workers: Vec<_> = (0..24).map(|_| {
        let (server, cfg) = (server.clone(), cfg.clone());
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                handle(&cfg, req);
            }
        })
    }).collect();
    for w in workers {
        let _ = w.join();
    }
}

#[cfg(test)]
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;
