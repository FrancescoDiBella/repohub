//! repohub-server: serves the RepoHub web build and proxies the GitHub API for it.
//!
//! - `GET /` and static files from `WEB_DIR`.
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

mod notes;

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

/// One segment of a board's path in a notes repository, as the app writes it (its notes
/// segment encoding, decoded once): `[A-Za-z0-9_.-]` and `%XX`, not empty, no leading dot.
fn board_segment(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b[0] == b'.' || s.len() > 255 {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.') => i += 1,
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => i += 3,
            _ => return false,
        }
    }
    true
}

/// `boards/<owner>/<repo>/<branch>/<name>.excalidraw` (after `contents/`): the only files board
/// sync reads and writes in the notes repository.
fn board_file(rest: &[&str]) -> bool {
    rest.len() == 6 && rest[0] == "contents" && rest[1] == "boards" && rest[2..].iter().all(|s| board_segment(s))
        && rest[5].strip_suffix(".excalidraw").is_some_and(|stem| !stem.is_empty())
}

/// A board's contents path, checked on the raw address before anything is decoded:
/// `/repos/{o}/{r}/contents/boards/…` where each board segment is `[A-Za-z0-9_.-]` or `%25XX`
/// (the app's `%XX`, itself encoded). No `..`, no empty segment, no encoded slash (`%2F`) or dot:
/// GitHub reads `%25` back as `%`, so the file names keep their `%` signs. Returns the path
/// decoded once, for [`allowed`].
fn board_contents_path(raw: &str) -> Option<String> {
    let seg: Vec<&str> = raw.strip_prefix('/')?.split('/').collect();
    let name = |s: &str| !s.is_empty() && !s.starts_with('.') && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let raw_segment = |s: &str| {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                c if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.') => i += 1,
                b'%' if i + 4 < b.len() && &b[i + 1..i + 3] == b"25" && b[i + 3].is_ascii_hexdigit() && b[i + 4].is_ascii_hexdigit() => i += 5,
                _ => return false,
            }
        }
        !b.is_empty() && b[0] != b'.'
    };
    let ok = seg.len() == 9 && seg[0] == "repos" && name(seg[1]) && name(seg[2]) && seg[3] == "contents" && seg[4] == "boards" && seg[5..].iter().all(|s| raw_segment(s));
    ok.then(|| raw.replace("%25", "%"))
}

/// Which proxied API calls are allowed: the exact read endpoints RepoHub uses, starring, and the few
/// writes the app makes (create an issue, a branch or a pull request, close an issue, merge a
/// pull request, and board sync's writes to one board file in the notes repository). Nothing that
/// deletes repositories or branches, rewrites, or exposes administrative repository data.
fn allowed(method: &Method, path: &str) -> bool {
    let p = path.split('?').next().unwrap_or("");
    let seg: Vec<&str> = p.trim_start_matches('/').split('/').collect();
    let repo = seg.len() >= 3 && seg[0] == "repos" && !seg[1].is_empty() && !seg[2].is_empty();
    let rest = if repo { &seg[3..] } else { &[][..] };
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let sha = |s: &str| (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());

    match method {
        Method::Get => {
            if p == "/user" || p == "/user/repos" || p == "/user/starred" || p == "/user/orgs" || p == "/search/issues" || p == "/notifications" {
                return true;
            }
            if seg.len() == 2 && seg[0] == "users" && !seg[1].is_empty() {
                return true;
            }
            if seg.len() == 3 && seg[0] == "users" && !seg[1].is_empty() && (seg[2] == "repos" || seg[2] == "orgs") {
                return true;
            }
            if seg.len() == 3 && seg[0] == "orgs" && !seg[1].is_empty() && seg[2] == "repos" {
                return true;
            }
            if repo {
                // Board sync: the notes repository itself (exists, private, default branch), its
                // file list, and its boards.
                if rest.is_empty() || board_file(rest) {
                    return true;
                }
                if let ["git", "trees", r] = rest
                    && !r.is_empty() && !r.starts_with('.') && r.len() <= 100 && r.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                {
                    return true;
                }
                match rest {
                    ["actions", "runs"]
                    | ["stats", "participation"]
                    | ["contents"]
                    | ["readme"]
                    | ["languages"]
                    | ["commits"]
                    | ["branches"]
                    | ["issues"] => return true,
                    _ => {}
                }
                if rest.len() >= 4 && rest[0] == "git" && rest[1] == "ref" && rest[2] == "heads" && rest[3..].iter().all(|s| !s.is_empty()) {
                    return true;
                }
                if rest.len() == 3 && rest[0] == "pulls" && number(rest[1]) && matches!(rest[2], "files" | "reviews" | "comments") {
                    return true;
                }
                // One issue or pull request and its conversation.
                if rest.len() == 2 && (rest[0] == "issues" || rest[0] == "pulls") && number(rest[1]) {
                    return true;
                }
                if rest.len() == 3 && rest[0] == "issues" && number(rest[1]) && rest[2] == "comments" {
                    return true;
                }
                if rest.len() == 4 && rest[0] == "actions" && rest[1] == "runs" && number(rest[2]) && rest[3] == "jobs" {
                    return true;
                }
                if rest.len() == 4 && rest[0] == "actions" && rest[1] == "jobs" && number(rest[2]) && rest[3] == "logs" {
                    return true;
                }
                // The checks on a pull request's last commit (a hex commit id).
                if rest.len() == 3 && rest[0] == "commits" && sha(rest[1]) && rest[2] == "check-runs" {
                    return true;
                }
            }
            false
        }
        Method::Put if repo && rest.len() == 3 && rest[0] == "pulls" && number(rest[1]) && rest[2] == "merge" => true,
        // Board sync writes one board, under boards/ and ending in .excalidraw, nothing else.
        Method::Put | Method::Delete if repo && board_file(rest) => true,
        Method::Put | Method::Delete => seg.len() == 4 && seg[0] == "user" && seg[1] == "starred" && !seg[2].is_empty() && !seg[3].is_empty(),
        // Creating an issue, a pull request or a branch; commenting; reviewing a pull request; re-running a workflow.
        Method::Post => repo && (matches!(rest, ["issues"] | ["pulls"] | ["git", "refs"])
            || (rest.len() == 3 && number(rest[1]) && matches!((rest[0], rest[2]), ("issues", "comments") | ("pulls", "reviews") | ("pulls", "comments")))
            || (rest.len() == 4 && rest[0] == "actions" && rest[1] == "runs" && number(rest[2]) && matches!(rest[3], "rerun" | "rerun-failed-jobs"))),
        // Closing or reopening an issue; marking one notification as read.
        Method::Patch => (repo && rest.len() == 2 && rest[0] == "issues" && number(rest[1]))
            || (seg.len() == 3 && seg[0] == "notifications" && seg[1] == "threads" && number(seg[2])),
        _ => false,
    }
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
    match std::fs::read(&canon_file) {
        Ok(bytes) => {
            let resp = Response::from_data(bytes)
                .with_header(h("Content-Type", mime(&canon_file)))
                .with_header(h("Cache-Control", "no-cache"));
            let _ = req.respond(resp);
        }
        Err(_) => {
            let _ = req.respond(text(404, "Not found"));
        }
    }
}

fn proxy(cfg: &Cfg, mut req: Request) {
    let full_path = req.url()["/gh".len()..].to_owned();
    if full_path.chars().any(|c| c.is_control() || c == '\0') {
        let _ = req.respond(text(403, "Invalid characters in URL"));
        return;
    }
    let raw_path = full_path.split('?').next().unwrap_or("");
    let board = board_contents_path(raw_path);
    let Some(decoded_path) = board.clone().or_else(|| validate_and_decode_path(raw_path)) else {
        let _ = req.respond(text(403, "Path traversal or invalid characters in path"));
        return;
    };
    if !allowed(req.method(), &decoded_path) {
        let _ = req.respond(text(403, "Endpoint not allowed by the RepoHub proxy"));
        return;
    }
    let accept = header(&req, "Accept").unwrap_or_else(|| "application/vnd.github+json".into());
    let method = req.method().as_str().to_owned();
    // A board (base64 in JSON) may be large; anything else is a short JSON body.
    let limit: u64 = if board.is_some() { 16 << 20 } else { 1 << 16 };
    let mut body = Vec::new();
    let _ = req.as_reader().take(limit + 1).read_to_end(&mut body);
    if body.len() as u64 > limit {
        let _ = req.respond(text(413, "Too much to send"));
        return;
    }
    let mut r = cfg.agent.request(&method, &format!("{}{}", cfg.api, full_path))
        .set("Accept", &accept)
        .set("X-GitHub-Api-Version", "2022-11-28");
    if let Some(t) = &cfg.token {
        r = r.set("Authorization", &format!("Bearer {t}"));
    }
    // Writes carry a JSON body; a body-less PUT (starring) still needs a length.
    let result = if !body.is_empty() {
        r.set("Content-Type", "application/json").send_bytes(&body)
    } else if method == "PUT" {
        r.set("Content-Length", "0").send_string("")
    } else {
        r.call()
    };
    let resp = match result {
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
        Err(e) => {
            let _ = req.respond(text(502, &format!("Can't reach GitHub: {e}")));
            return;
        }
    };
    let status = resp.status();
    let mut headers = vec![h("Cache-Control", "no-store")];
    for name in ["Content-Type", "x-ratelimit-remaining", "x-ratelimit-reset", "x-ratelimit-limit", "retry-after", "link"] {
        if let Some(v) = resp.header(name) {
            headers.push(h(name, v));
        }
    }
    let mut data = Vec::new();
    let _ = resp.into_reader().take(32 << 20).read_to_end(&mut data);
    let mut out = Response::from_data(data).with_status_code(status);
    for x in headers {
        out.add_header(x);
    }
    let _ = req.respond(out);
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
    let workers: Vec<_> = (0..8).map(|_| {
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
mod tests {
    use super::*;

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1:8080"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.0.2:9000"));
        assert!(is_loopback("localhost:8080"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("[::1]:8080"));
        assert!(is_loopback("[::1]"));
        assert!(is_loopback("::1"));

        assert!(!is_loopback("0.0.0.0:8080"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("[::]:8080"));
        assert!(!is_loopback("[::]"));
        assert!(!is_loopback("::"));
        assert!(!is_loopback("192.168.1.5:8080"));
        assert!(!is_loopback("example.com:8080"));
    }

    #[test]
    fn origin_matching() {
        assert!(origin_matches_host("http://localhost:8080", "localhost:8080"));
        assert!(origin_matches_host("http://example.com", "example.com:80"));
        assert!(origin_matches_host("http://example.com:80", "example.com"));
        assert!(origin_matches_host("https://example.com", "example.com:443"));
        assert!(origin_matches_host("https://example.com", "example.com"));
        assert!(origin_matches_host("http://[::1]:8080", "[::1]:8080"));

        // Different ports or schemes rejected
        assert!(!origin_matches_host("http://localhost:3000", "localhost:8080"));
        assert!(!origin_matches_host("http://example.com:443", "example.com:80"));
        assert!(!origin_matches_host("https://example.com", "example.com:80"));
        assert!(!origin_matches_host("null", "localhost:8080"));
        assert!(!origin_matches_host("file:///path", "localhost:8080"));
        assert!(!origin_matches_host("http://other.com:8080", "localhost:8080"));
    }

    #[test]
    fn referer_origin_extraction() {
        assert_eq!(referer_origin("http://nas.local:8080/index.html"), Some("http://nas.local:8080"));
        assert_eq!(referer_origin("https://nas.local/repo/notes?q=1"), Some("https://nas.local"));
        assert_eq!(referer_origin("ftp://example.com"), None);
    }

    #[test]
    fn path_traversal_and_encoding_validation() {
        assert_eq!(validate_and_decode_path("/user/repos"), Some("/user/repos".into()));
        assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/feature/x"), Some("/repos/o/r/git/ref/heads/feature/x".into()));
        assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/feature+x"), Some("/repos/o/r/git/ref/heads/feature+x".into()));

        // Raw encoded traversals rejected
        assert_eq!(validate_and_decode_path("/repos/o/r/%2e%2e/%2e%2e/user/emails"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/%2E%2E/etc"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/%2f/etc"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/%5c/etc"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/.."), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/..."), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/."), None);
        assert_eq!(validate_and_decode_path("/repos/o/r//contents"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r\\readme"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r\0readme"), None);

        // Multi-encoded traversals (%252e%252e) MUST be rejected:
        assert_eq!(validate_and_decode_path("/repos/o/r/%252e%252e"), None);
        assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/%252e%252e/%252e%252e/user/emails"), None);
    }

    #[test]
    fn proxy_exact_get_allowlist() {
        // All endpoints src/github.rs actually calls must be allowed:
        assert!(allowed(&Method::Get, "/user"));
        assert!(allowed(&Method::Get, "/user/repos"));
        assert!(allowed(&Method::Get, "/user/starred"));
        assert!(allowed(&Method::Get, "/user/orgs"));
        assert!(allowed(&Method::Get, "/search/issues"));
        assert!(allowed(&Method::Get, "/users/octocat"));
        assert!(allowed(&Method::Get, "/users/octocat/repos"));
        assert!(allowed(&Method::Get, "/users/octocat/orgs"));
        assert!(allowed(&Method::Get, "/orgs/acme/repos"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/actions/runs"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/stats/participation"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/contents"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/readme"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/languages"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/commits"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/branches"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/issues"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/git/ref/heads/main"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/git/ref/heads/feature/x"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/123/files"));

        // Sensitive or unrelated endpoints under /repos/* and /users/* MUST be rejected:
        assert!(!allowed(&Method::Get, "/repos/owner/repo/hooks"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/collaborators"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/variables"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/environments"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/invitations"));
        assert!(!allowed(&Method::Get, "/users/octocat/keys"));
        assert!(!allowed(&Method::Get, "/user/emails"));
        assert!(!allowed(&Method::Get, "/orgs/acme/members"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/notanumber/files"));
        // One issue or pull request and its conversation, and nothing next to them.
        assert!(allowed(&Method::Get, "/repos/owner/repo/issues/12"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/issues/12/comments"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7/reviews"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/issues/bad"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/issues/12/events"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/7/commits"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/7/reviews/3"));
        assert!(!allowed(&Method::Get, "/gists"));
        // Your notifications, and nothing under them to read.
        assert!(allowed(&Method::Get, "/notifications"));
        assert!(!allowed(&Method::Get, "/notifications/threads/12"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/notifications"));
        // The checks on a commit, by its id only.
        assert!(allowed(&Method::Get, "/repos/owner/repo/commits/0123abcdef0123abcdef0123abcdef0123abcdef/check-runs"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/main/check-runs"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/0123abc/statuses"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/commits/0123abc/check-runs"));
        // Workflow run jobs and job logs, by numeric id only.
        assert!(allowed(&Method::Get, "/repos/owner/repo/actions/runs/42/jobs"));
        assert!(allowed(&Method::Get, "/repos/owner/repo/actions/jobs/99/logs"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/runs/bad/jobs"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/jobs/bad/logs"));
        assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/runs/42/logs"));
    }

    #[test]
    fn proxy_write_allowlist() {
        assert!(allowed(&Method::Put, "/user/starred/owner/repo"));
        assert!(allowed(&Method::Delete, "/user/starred/owner/repo"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/issues"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/pulls"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/git/refs"));
        assert!(allowed(&Method::Patch, "/repos/owner/repo/issues/12"));
        assert!(allowed(&Method::Put, "/repos/owner/repo/pulls/7/merge"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/rerun"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/rerun-failed-jobs"));
        // Marking one notification as read, not all of them, and no unsubscribing.
        assert!(allowed(&Method::Patch, "/notifications/threads/12"));
        assert!(!allowed(&Method::Patch, "/notifications/threads/abc"));
        assert!(!allowed(&Method::Put, "/notifications"));
        assert!(!allowed(&Method::Delete, "/notifications/threads/12"));
        assert!(!allowed(&Method::Put, "/notifications/threads/12/subscription"));

        // Disallowed writes:
        assert!(!allowed(&Method::Delete, "/repos/owner/repo"));
        assert!(!allowed(&Method::Post, "/user/repos"));
        assert!(!allowed(&Method::Put, "/repos/owner/repo/contents/README.md"));
        // Commenting and reviewing are writes the app makes; nothing else under them is.
        assert!(allowed(&Method::Post, "/repos/owner/repo/issues/12/comments"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/pulls/7/reviews"));
        // Comments on lines of a pull request's diff: read and written, nothing under them.
        assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7/comments"));
        assert!(allowed(&Method::Post, "/repos/owner/repo/pulls/7/comments"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/comments/3/replies"));
        assert!(!allowed(&Method::Patch, "/repos/owner/repo/pulls/comments/3"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/issues/bad/comments"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/merge"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/reviews/3/dismissals"));
        assert!(!allowed(&Method::Delete, "/repos/owner/repo/issues/comments/9"));
        assert!(!allowed(&Method::Patch, "/repos/owner/repo"));
        assert!(!allowed(&Method::Patch, "/repos/owner/repo/issues/bad"));
        assert!(!allowed(&Method::Delete, "/repos/owner/repo/git/refs/heads/main"));
        assert!(!allowed(&Method::Put, "/user/starred/owner/repo/extra"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/actions/runs/bad/rerun"));
        assert!(!allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/cancel"));
        assert!(!allowed(&Method::Delete, "/repos/owner/repo/actions/runs/42/logs"));
    }

    #[test]
    fn board_sync_allowlist() {
        let board = "/repos/me/repohub-notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw";
        for m in [Method::Get, Method::Put, Method::Delete] {
            assert!(allowed(&m, board), "{m:?}");
            assert!(allowed(&m, "/repos/me/notes/contents/boards/o/r/main/Plan.excalidraw"), "{m:?}");
        }
        assert!(!allowed(&Method::Post, board));
        assert!(!allowed(&Method::Patch, board));
        // The repository itself and its file list: read only.
        assert!(allowed(&Method::Get, "/repos/me/repohub-notes"));
        assert!(allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/main?recursive=1"));
        assert!(!allowed(&Method::Put, "/repos/me/repohub-notes"));
        assert!(!allowed(&Method::Delete, "/repos/me/repohub-notes"));
        assert!(!allowed(&Method::Post, "/repos/me/repohub-notes/git/trees"));
        assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/feature/x"));
        assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/.."));
        assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/blobs/abc123"));
        // Writes stay narrow: under boards/, four levels deep, ending in .excalidraw.
        for bad in [
            "/repos/o/r/contents/README.md",
            "/repos/o/r/contents/.github/workflows/x.yml",
            "/repos/o/r/contents/boards/../x",
            "/repos/o/r/contents/boards/o/r/main/x.json",
            "/repos/o/r/contents/boards/o/r/main/.excalidraw",
            "/repos/o/r/contents/boards/o/r/x.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/sub/x.excalidraw",
            "/repos/o/r/contents/boards/o//main/x.excalidraw",
            "/repos/o/r/contents/boards/o/r/../x.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/.hidden.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/a b.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/x.excalidraw/extra",
            "/repos/o/r/contents/notes/o/r/main/x.excalidraw",
            "/repos/o/r/contents",
        ] {
            assert!(!allowed(&Method::Put, bad), "PUT {bad}");
            assert!(!allowed(&Method::Delete, bad), "DELETE {bad}");
        }
        assert!(allowed(&Method::Get, "/repos/o/r/contents"), "the Code tab's root listing is unchanged");
        assert!(!allowed(&Method::Get, "/repos/o/r/contents/README.md"), "other files stay unreadable");
        assert!(!allowed(&Method::Get, "/repos/o/r/contents/.github/workflows/x.yml"));
    }

    #[test]
    fn board_paths_are_checked_before_decoding() {
        // What the app sends: its `%XX` segment encoding, itself encoded as `%25XX`.
        let raw = "/repos/me/notes/contents/boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw";
        assert_eq!(board_contents_path(raw).as_deref(), Some("/repos/me/notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw"));
        assert!(allowed(&Method::Put, &board_contents_path(raw).unwrap()));
        // A branch named `..` is one folder, `%2E.`, never a step up.
        assert!(board_contents_path("/repos/me/notes/contents/boards/o/r/%252E./x.excalidraw").is_some());
        for bad in [
            "/repos/me/notes/contents/boards/o/r/feature%2Fx/x.excalidraw", // an encoded slash
            "/repos/me/notes/contents/boards/o/r/%2e%2e/x.excalidraw",
            "/repos/me/notes/contents/boards/o/r/../x.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/.x.excalidraw",
            "/repos/me/notes/contents/boards/o/r//x.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/x%25.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/x%252.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/x%5Cy.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/x%00.excalidraw",
            "/repos/me/notes/contents/README.md",
            "/repos/me/notes/contents/boards/o/r/main/sub/x.excalidraw",
            "/repos/.me/notes/contents/boards/o/r/main/x.excalidraw",
        ] {
            assert_eq!(board_contents_path(bad), None, "{bad}");
        }
        // Everything else still goes through the general check, which refuses `%` after decoding.
        assert_eq!(validate_and_decode_path("/repos/me/notes/contents/boards/o/r/feature%252Fx/x.excalidraw"), None);
    }

    #[test]
    fn constant_time_comparison() {
        assert!(constant_time_eq(b"secret123", b"secret123"));
        assert!(!constant_time_eq(b"secret123", b"secret124"));
        assert!(!constant_time_eq(b"secret123", b"secret12"));
        assert!(!constant_time_eq(b"", b"secret"));
    }

    #[test]
    fn cookie_encoding_and_parsing() {
        let sec = "p@ss;word=1 2\"3";
        let encoded = percent_encode_cookie(sec);
        assert!(!encoded.contains(';') && !encoded.contains('=') && !encoded.contains(' ') && !encoded.contains('"'));

        // Simulating Cookie header with malformed prefix
        let req_header = format!("malformed_cookie; other=1; repohub_secret={encoded}; final=x");
        let fake_req = |cookie_hdr: &str| -> Option<String> {
            for part in cookie_hdr.split(';') {
                let Some((k, v)) = part.split_once('=') else {
                    continue;
                };
                if k.trim().eq_ignore_ascii_case("repohub_secret") {
                    return percent_decode_path(v.trim());
                }
            }
            None
        };
        assert_eq!(fake_req(&req_header), Some(sec.to_string()));
    }

    #[test]
    fn json_secret_parsing() {
        assert_eq!(parse_json_secret(r#"{"secret": "simple"}"#), Some("simple".into()));
        assert_eq!(parse_json_secret(r#"{"other": 1, "secret": "foo:bar,baz"}"#), Some("foo:bar,baz".into()));
        assert_eq!(parse_json_secret(r#"{"secret": "hello \"world\""}"#), Some("hello \"world\"".into()));
        assert_eq!(parse_json_secret(r#"{"secret": "line1\nline2"}"#), Some("line1\nline2".into()));
        assert_eq!(parse_json_secret(r#"{"invalid": true}"#), None);
    }

    #[test]
    fn hosts() {
        assert_eq!(host_only("localhost:8080"), "localhost");
        assert_eq!(host_only("[::1]:8080"), "[::1]");
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn secret_required_on_non_loopback() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let res = Cfg::load_with_addr("0.0.0.0:8080");
        match res {
            Err(e) => assert!(e.contains("REPOHUB_SECRET must be set")),
            Ok(_) => panic!("expected failure on non-loopback without secret"),
        }

        // Loopback succeeds without secret
        let res = Cfg::load_with_addr("127.0.0.1:8080");
        assert!(res.is_ok());
        let cfg = res.unwrap();
        assert!(cfg.allow_writes);
        assert_eq!(cfg.secret, None);
    }

    #[test]
    fn static_path_validation() {
        let temp_dir = std::env::temp_dir().join(format!("repohub-static-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("index.html"), "<h1>Home</h1>").unwrap();
        let sub = temp_dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("app.js"), "console.log(1);").unwrap();

        let cfg = Cfg {
            token: None,
            api: "https://api.github.com".into(),
            web_dir: temp_dir.clone(),
            data_dir: temp_dir.clone(),
            allowed_hosts: vec!["localhost".into()],
            public_json: "{}".into(),
            agent: ureq::AgentBuilder::new().build(),
            secret: None,
            allow_writes: true,
            notes_max_bytes: 500 << 20,
        };

        let canon_web = std::fs::canonicalize(&cfg.web_dir).unwrap();

        // Check relative paths stay within web_dir
        let check_rel = |rel: &str| -> bool {
            if rel.contains(':') || rel.contains('\\') || rel.contains('%') || rel.chars().any(|c| c.is_control() || c == '\0') {
                return false;
            }
            if rel.split('/').any(|seg| seg.starts_with('.') || (!seg.is_empty() && seg.bytes().all(|b| b == b'.'))) {
                return false;
            }
            let path = cfg.web_dir.join(rel);
            if let Ok(canon_file) = std::fs::canonicalize(&path) {
                canon_file.starts_with(&canon_web) && canon_file.is_file()
            } else {
                false
            }
        };

        assert!(check_rel("index.html"));
        assert!(check_rel("sub/app.js"));
        assert!(!check_rel("../secret.txt"));
        assert!(!check_rel("..\\secret.txt"));
        assert!(!check_rel("sub/../.."));
        assert!(!check_rel(".hidden"));
        assert!(!check_rel("C:/Windows/System32"));
        assert!(!check_rel("file:test"));
        assert!(!check_rel("%252e%252e"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn origin_and_preflight_checks() {
        // origin matches host: ok
        assert!(origin_matches_host("http://localhost:8080", "localhost:8080"));
        // missing origin or mismatched origin
        assert!(!origin_matches_host("http://localhost:3000", "localhost:8080"));
        assert!(!origin_matches_host("null", "localhost:8080"));
    }

    fn http_status(res: Result<ureq::Response, ureq::Error>) -> (u16, String, Option<String>) {
        match res {
            Ok(r) => {
                let status = r.status();
                let set_cookie = r.header("Set-Cookie").map(str::to_owned);
                let body = r.into_string().unwrap_or_default();
                (status, body, set_cookie)
            }
            Err(ureq::Error::Status(code, r)) => {
                let set_cookie = r.header("Set-Cookie").map(str::to_owned);
                let body = r.into_string().unwrap_or_default();
                (code, body, set_cookie)
            }
            Err(e) => panic!("Network request failed: {e}"),
        }
    }

    #[test]
    fn end_to_end_http_server_tests() {
        let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // 1. Mock upstream GitHub API
        let mock_upstream = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
        let upstream_addr = format!("http://{}", mock_upstream.server_addr());
        let up = mock_upstream.clone();
        std::thread::spawn(move || {
            for req in up.incoming_requests() {
                let auth = req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("Authorization")).map(|h| h.value.as_str().to_owned());
                let body = format!("{{\"ok\":true,\"auth\":{}}}", json_str(&auth.unwrap_or_default()));
                let resp = Response::from_string(body)
                    .with_status_code(200)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(Header::from_bytes("Link", "<https://api.github.com/user/repos?page=2>; rel=\"next\"").unwrap());
                let _ = req.respond(resp);
            }
        });

        // 2. Setup web_dir and data_dir
        let temp_dir = std::env::temp_dir().join(format!("repohub-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let web_dir = temp_dir.join("web");
        let data_dir = temp_dir.join("data");
        std::fs::create_dir_all(&web_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(web_dir.join("index.html"), "<h1>RepoHub App</h1>").unwrap();

        // Symlink pointing outside web_dir
        let outside_file = temp_dir.join("secret_outside.txt");
        std::fs::write(&outside_file, "super secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, web_dir.join("leak_link.txt")).unwrap();

        // 3. Create RepoHub server with secret authentication
        let server = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
        let s_addr = format!("{}", server.server_addr());
        let cfg = std::sync::Arc::new(Cfg {
            token: Some("gh_token_12345".into()),
            api: upstream_addr,
            web_dir: web_dir.clone(),
            data_dir: data_dir.clone(),
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            public_json: "{\"notes\":true}".into(),
            agent: ureq::AgentBuilder::new().try_proxy_from_env(false).build(),
            secret: Some("p@ss;word=1 2".into()),
            allow_writes: true,
            notes_max_bytes: 100, // Small limit for testing
        });

        let s_clone = server.clone();
        let c_clone = cfg.clone();
        std::thread::spawn(move || {
            for req in s_clone.incoming_requests() {
                handle(&c_clone, req);
            }
        });

        let client = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout(std::time::Duration::from_secs(5))
            .build();

        let base_url = format!("http://{s_addr}");
        let matching_origin = format!("http://{s_addr}");

        // Case A: Unauthenticated requests without secret
        // GET /gh/user -> 401
        let (status, _, _) = http_status(client.get(&format!("{base_url}/gh/user")).set("Origin", &matching_origin).call());
        assert_eq!(status, 401);

        // GET / -> 200 with sign-in HTML
        let (status, body, _) = http_status(client.get(&format!("{base_url}/")).call());
        assert_eq!(status, 200);
        assert!(body.contains("RepoHub · Sign in"));

        // Case B: Cross-origin POST /auth rejected (CSRF)
        let (status, _, _) = http_status(
            client.post(&format!("{base_url}/auth"))
                .set("Origin", "http://evil.com")
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string("secret=wrong")
        );
        assert_eq!(status, 403);

        // Case C: Same-origin POST /auth with wrong secret -> 401
        let (status, body, _) = http_status(
            client.post(&format!("{base_url}/auth"))
                .set("Origin", &matching_origin)
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string("secret=wrong_pass")
        );
        assert_eq!(status, 401);
        assert!(body.contains("Invalid secret"));

        // Case D: Same-origin POST /auth with correct secret (with ;, =, spaces)
        // Using form urlencoded
        let encoded_secret = percent_encode_cookie("p@ss;word=1 2");
        let (status, _, set_cookie) = http_status(
            client.post(&format!("{base_url}/auth"))
                .set("Origin", &matching_origin)
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string(&format!("secret={encoded_secret}"))
        );
        assert_eq!(status, 303);
        assert!(set_cookie.is_some());
        let cookie_header = set_cookie.unwrap();
        assert!(cookie_header.contains("repohub_secret="));
        assert!(cookie_header.contains("HttpOnly"));

        // Extract cookie value for subsequent authenticated calls
        let cookie_val = cookie_header.split(';').next().unwrap();

        // Also test JSON POST /auth
        let (status, _, json_cookie) = http_status(
            client.post(&format!("{base_url}/auth"))
                .set("Origin", &matching_origin)
                .set("Content-Type", "application/json")
                .send_string("{\"secret\":\"p@ss;word=1 2\"}")
        );
        assert_eq!(status, 303);
        assert!(json_cookie.is_some());

        // Case E: Authenticated requests using Bearer token or Cookie
        // 1. Missing Origin on /gh/user -> 403
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/user"))
                .set("Cookie", cookie_val)
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Missing Origin header"));

        // 2. Different port Origin on /gh/user -> 403
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/user"))
                .set("Cookie", cookie_val)
                .set("Origin", "http://127.0.0.1:3000")
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Origin not allowed"));

        // 3. Traversal in path: %2e%2e -> 403 (send raw HTTP bytes so HTTP client doesn't normalize URL)
        let (status, body) = {
            use std::io::Write;
            let mut stream = std::net::TcpStream::connect(&s_addr).unwrap();
            let raw_req = format!(
                "GET /gh/repos/o/r/%2e%2e/%2e%2e/user/emails HTTP/1.1\r\nHost: {s_addr}\r\nOrigin: {matching_origin}\r\nCookie: {cookie_val}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(raw_req.as_bytes()).unwrap();
            let mut resp_bytes = Vec::new();
            stream.read_to_end(&mut resp_bytes).unwrap();
            let resp_str = String::from_utf8_lossy(&resp_bytes);
            let code = resp_str.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
            (code, resp_str.to_string())
        };
        assert_eq!(status, 403, "expected 403 on %2e%2e, body was: {body}");
        assert!(body.contains("Path traversal"), "body was: {body}");

        // 4. Double-encoded traversal: %252e%252e -> 403
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/repos/o/r/git/ref/heads/%252e%252e/%252e%252e/user/emails"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Path traversal"));

        // 5. Endpoint outside allowlist -> 403
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/repos/o/r/hooks"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Endpoint not allowed"));

        // 6. Allowed endpoint with matching Origin -> 200 and forwards token and link header
        let r_user = client.get(&format!("{base_url}/gh/user"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .call()
            .unwrap();
        assert_eq!(r_user.status(), 200);
        assert_eq!(r_user.header("link"), Some("<https://api.github.com/user/repos?page=2>; rel=\"next\""));
        let body = r_user.into_string().unwrap_or_default();
        assert!(body.contains("gh_token_12345"));

        // 7. Allowed endpoint via Authorization: Bearer
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/user"))
                .set("Authorization", "Bearer p@ss;word=1 2")
                .set("Origin", &matching_origin)
                .call()
        );
        assert_eq!(status, 200);
        assert!(body.contains("gh_token_12345"));

        // 8. Plain-HTTP LAN browser simulation: no Origin, but X-RepoHub: 1 + Referer matching Host
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/gh/user"))
                .set("Cookie", cookie_val)
                .set("X-RepoHub", "1")
                .set("Referer", &format!("{base_url}/index.html"))
                .call()
        );
        assert_eq!(status, 200);
        assert!(body.contains("gh_token_12345"));

        // 9. Writes without X-RepoHub: 1 -> 403
        let (status, body, _) = http_status(
            client.put(&format!("{base_url}/gh/user/starred/owner/repo"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .set("Content-Type", "application/json")
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Writes require X-RepoHub: 1"));

        // 10. Writes without Content-Type: application/json -> 403
        let (status, body, _) = http_status(
            client.put(&format!("{base_url}/gh/user/starred/owner/repo"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .set("X-RepoHub", "1")
                .set("Content-Type", "text/plain")
                .call()
        );
        assert_eq!(status, 403);
        assert!(body.contains("Content-Type: application/json"));

        // 11. Notes total size cap: max_notes_bytes is 100 bytes
        // Save first note of 60 bytes -> 204
        let note1_body = "{\"type\":\"excalidraw\",\"tag\":\"60_bytes_payload_data_here\"}";
        assert!(note1_body.len() <= 100);
        let (status, _, _) = http_status(
            client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n1"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .set("X-RepoHub", "1")
                .set("Content-Type", "application/json")
                .send_string(note1_body)
        );
        assert_eq!(status, 204);

        // Save second note of 60 bytes (total 120 > 100) -> 413
        let (status, body, _) = http_status(
            client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n2"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .set("X-RepoHub", "1")
                .set("Content-Type", "application/json")
                .send_string(note1_body)
        );
        assert_eq!(status, 413);
        assert!(body.contains("Total notes storage limit exceeded"));

        // Overwrite n1 with note of 70 bytes -> (60 - 60) + 70 = 70 <= 100 -> 204
        let note1_updated = "{\"type\":\"excalidraw\",\"tag\":\"70_bytes_payload_data_longer_update\"}";
        let (status, _, _) = http_status(
            client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n1"))
                .set("Cookie", cookie_val)
                .set("Origin", &matching_origin)
                .set("X-RepoHub", "1")
                .set("Content-Type", "application/json")
                .send_string(note1_updated)
        );
        assert_eq!(status, 204);

        // 12. Static file serving (with authenticated cookie)
        // Valid file -> 200
        let (status, body, _) = http_status(
            client.get(&format!("{base_url}/index.html"))
                .set("Cookie", cookie_val)
                .call()
        );
        assert_eq!(status, 200);
        assert!(body.contains("RepoHub App"));

        // Symlink escaping web_dir -> 404
        #[cfg(unix)]
        {
            let (status, _, _) = http_status(
                client.get(&format!("{base_url}/leak_link.txt"))
                    .set("Cookie", cookie_val)
                    .call()
            );
            assert_eq!(status, 404);
        }

        // Multi-encoded path traversal in static file -> 404
        let (status, _, _) = http_status(
            client.get(&format!("{base_url}/%252e%252e/secret.txt"))
                .set("Cookie", cookie_val)
                .call()
        );
        assert_eq!(status, 404);

        server.unblock();
        mock_upstream.unblock();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
