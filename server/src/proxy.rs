//! `/gh/<path>`: the request checked against the allowlist and forwarded to GitHub with the token (and the
//! browser's entity tags), and GitHub's answer passed back.

use super::*;

/// The `If-None-Match` of a GET forwarded to GitHub: entity tags only (`"…"` or `W/"…"`,
/// comma-separated, printable ASCII, at most 512 bytes); anything else isn't sent.
pub(super) fn conditional_tag(method: &str, value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    let tag = |t: &str| {
        let t = t.trim();
        let inner = t.strip_prefix("W/").unwrap_or(t);
        inner.len() >= 2 && inner.starts_with('"') && inner.ends_with('"') && !inner[1..inner.len() - 1].contains('"')
    };
    let ok = method == "GET" && !v.is_empty() && v.len() <= 512 && v.bytes().all(|b| (0x20..0x7f).contains(&b)) && v.split(',').all(tag);
    ok.then(|| v.to_owned())
}

pub(super) fn proxy(cfg: &Cfg, mut req: Request) {
    let full_path = req.url()["/gh".len()..].to_owned();
    if full_path.chars().any(|c| c.is_control() || c == '\0') {
        let _ = req.respond(text(403, "Invalid characters in URL"));
        return;
    }
    let raw_path = full_path.split('?').next().unwrap_or("");
    let board = board_contents_path(raw_path);
    // A branch name keeps its `%` signs too (see `branch_path`).
    let Some(decoded_path) = board.clone().or_else(|| branch_path(raw_path)).or_else(|| validate_and_decode_path(raw_path)) else {
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
    // A conditional read: GitHub answers 304 (not counted against the rate limit) when the
    // app's copy is still current.
    if let Some(tag) = conditional_tag(&method, header(&req, "If-None-Match").as_deref()) {
        r = r.set("If-None-Match", &tag);
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
    let mut headers = vec![h("Cache-Control", "no-store"), h("Vary", "Accept-Encoding")];
    for name in ["Content-Type", "x-ratelimit-remaining", "x-ratelimit-reset", "x-ratelimit-limit", "retry-after", "link", "etag"] {
        if let Some(v) = resp.header(name) {
            headers.push(h(name, v));
        }
    }
    let mut data = Vec::new();
    let _ = resp.into_reader().take(32 << 20).read_to_end(&mut data);
    // Compress non-trivial payloads for browsers that accept gzip (typical GitHub JSON shrinks 5-10x).
    let wants_gzip = data.len() >= 1024
        && (200..300).contains(&status)
        && header(&req, "Accept-Encoding").is_some_and(|a| files::accepts(&a, "gzip"));
    let body = if wants_gzip {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::with_capacity(data.len() / 4), flate2::Compression::fast());
        if enc.write_all(&data).is_ok()
            && let Ok(compressed) = enc.finish()
            && compressed.len() < data.len()
        {
            headers.push(h("Content-Encoding", "gzip"));
            compressed
        } else {
            data
        }
    } else {
        data
    };
    let mut out = Response::from_data(body).with_status_code(status);
    for x in headers {
        out.add_header(x);
    }
    let _ = req.respond(out);
}
