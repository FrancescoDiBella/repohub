//! GitHub REST API client, shared by the desktop and web builds. Requests go through
//! [`net::Transport`]; the functions here are async and run in the background (see `net::spawn`).
use std::collections::HashSet;

use serde_json::{Value, json};

use crate::config::Config;
use crate::net::{self, Resp, Transport};
use crate::inbox::{Notification, Subject};
use crate::review::LineComment;
use crate::thread::{Entry, Standing, Thread, Verdict};
use crate::model::{Checks, Commit, Data, Detail, Issue, JobLog, Profile, Pull, PullState, Repo, Run, RunJob, RunStep, duration, hex_hue, lang_hue, parse_iso, same_repo};

mod load;
mod parse;
mod reads;
mod writes;

pub use load::{branches, generate_release_notes, load_all_reporting, load_choices, load_commit, load_commits, load_compare, load_detail, load_job_log, load_latest_release, load_line_comments, load_more_issues, load_more_pulls, load_notifications, load_pull_files, load_release_listing, load_releases, load_run_jobs, load_thread};
#[cfg(not(target_arch = "wasm32"))]
pub use load::load_all;
#[cfg(test)]
pub use load::{AT_ONCE, load_all_with, merge_review_requested};
pub use parse::{LOG_TAIL_LINES, LineComments, PullFile, check_runs_total, parse_check_runs, parse_issue, parse_line_comments, parse_notification, parse_pull, parse_run_jobs, parse_thread, parse_workflow_runs, tail_log};
#[cfg(test)]
pub use parse::{parse_line_comment, strip_ansi};
use parse::{labels_of, logins_of};
pub use writes::{Done, NewLineComment, NewPull, NewRelease, Write, mark_read, run, set_star};
#[cfg(test)]
pub use writes::{Created, issue_body, parse_created};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    pub message: String,
    pub retry_after: Option<u64>,
}

impl LoadError {
    pub fn new(message: impl Into<String>, retry_after: Option<u64>) -> LoadError {
        LoadError { message: message.into(), retry_after }
    }
}

impl From<String> for LoadError {
    fn from(message: String) -> LoadError {
        LoadError { message, retry_after: None }
    }
}

impl From<&str> for LoadError {
    fn from(s: &str) -> LoadError {
        LoadError { message: s.into(), retry_after: None }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LoadError {}

pub struct Gh {
    t: Transport,
    api_base: String,
    now: i64,
    retry_after: std::sync::Mutex<Option<u64>>,
    /// At most this many requests at once (the parts of `load_all` run side by side).
    gate: Option<net::Gate>,
    /// The `etag` store's generation when this client was made: answers to an account that
    /// was signed out of meanwhile aren't kept.
    tags: u64,
}

const JSON: &str = "application/vnd.github+json";

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn n(v: &Value, k: &str) -> u32 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32
}

fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// `owner/name` from an API URL ending in `/repos/owner/name`.
fn repo_from_url(url: &str) -> String {
    let parts: Vec<&str> = url.rsplitn(3, '/').collect();
    if parts.len() == 3 { format!("{}/{}", parts[1], parts[0]) } else { url.to_owned() }
}

/// The links of a `Link` header (RFC 8288): each `<address>` with its parameters (names in
/// lower case, values unquoted). The address ends at `>` and may hold commas and semicolons;
/// a quoted value may too. Anything that doesn't read as a link is skipped up to the next one.
pub fn parse_link_header(h: &str) -> Vec<(String, Vec<(String, String)>)> {
    let b = h.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let skip_ws = |i: &mut usize| while *i < b.len() && (b[*i] == b' ' || b[*i] == b'\t') { *i += 1 };
    while i < b.len() {
        while i < b.len() && (b[i] == b',' || b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] != b'<' {
            // Not a link: on to the next `<` after a comma.
            while i < b.len() && !(b[i] == b',' && h[i + 1..].trim_start().starts_with('<')) {
                i += 1;
            }
            continue;
        }
        let Some(end) = h[i + 1..].find('>').map(|e| i + 1 + e) else { break };
        let url = h[i + 1..end].trim().to_owned();
        i = end + 1;
        let mut params = Vec::new();
        loop {
            skip_ws(&mut i);
            if i >= b.len() || b[i] == b',' {
                break;
            }
            if b[i] != b';' {
                i += 1;
                continue;
            }
            i += 1;
            skip_ws(&mut i);
            let start = i;
            while i < b.len() && !matches!(b[i], b'=' | b';' | b',' | b' ' | b'\t') {
                i += 1;
            }
            let name = h[start..i].to_ascii_lowercase();
            skip_ws(&mut i);
            let mut value = String::new();
            if i < b.len() && b[i] == b'=' {
                i += 1;
                skip_ws(&mut i);
                if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                    let q = b[i];
                    i += 1;
                    while i < b.len() && b[i] != q {
                        if b[i] == b'\\' && i + 1 < b.len() {
                            i += 1;
                        }
                        let ch = h[i..].chars().next().unwrap_or(' ');
                        value.push(ch);
                        i += ch.len_utf8();
                    }
                    i += 1;
                } else {
                    let start = i;
                    while i < b.len() && !matches!(b[i], b';' | b',' | b' ' | b'\t') {
                        i += 1;
                    }
                    value = h[start..i].to_owned();
                }
            }
            if !name.is_empty() {
                params.push((name, value));
            }
        }
        out.push((url, params));
    }
    out
}

/// The next page's path from a GitHub `Link` header, relative to `api_base`: the link whose
/// `rel` (one or more relation types) holds `next`.
pub fn next_page_path(link: Option<&str>, api_base: &str) -> Option<String> {
    let links = parse_link_header(link?);
    let (full_url, _) = links.iter().find(|(_, params)| params.iter().any(|(k, v)| k == "rel" && v.split_ascii_whitespace().any(|r| r.eq_ignore_ascii_case("next"))))?;
    let full_url = full_url.as_str();
    if full_url.starts_with('/') {
        return Some(full_url.to_owned());
    }
    let base = api_base.trim_end_matches('/');
    if let Some(rel) = full_url.strip_prefix(base) {
        return Some(if rel.starts_with('/') { rel.to_owned() } else { format!("/{rel}") });
    }
    if let Some(idx) = full_url.find("://")
        && let Some(path_idx) = full_url[idx + 3..].find('/')
    {
        let path = &full_url[idx + 3 + path_idx..];
        let path = path.strip_prefix("/api/v3").unwrap_or(path);
        return Some(if path.starts_with('/') { path.to_owned() } else { format!("/{path}") });
    }
    None
}

impl Gh {
    pub fn new(cfg: &Config) -> Gh {
        Gh { t: Transport::new(cfg), api_base: cfg.api.clone(), now: net::now(), retry_after: std::sync::Mutex::new(None), gate: None, tags: crate::etag::generation() }
    }

    /// The same client, sending at most `n` requests at once.
    pub fn limited(self, n: usize) -> Gh {
        Gh { gate: Some(net::Gate::new(n)), ..self }
    }

    #[cfg(test)]
    pub(crate) fn has_token(&self) -> bool {
        self.t.has_token()
    }

    pub fn record_backoff(&self, secs: u64) {
        if let Ok(mut g) = self.retry_after.lock() {
            *g = Some(g.map_or(secs, |prev| prev.max(secs)));
        }
    }

    pub fn retry_after(&self) -> Option<u64> {
        self.retry_after.lock().ok().and_then(|g| *g)
    }

    pub fn explain(&self, r: &Resp) -> String {
        let v = serde_json::from_str::<Value>(&r.body).ok();
        let mut msg = v.as_ref().map(|v| s(v, "message")).unwrap_or_default();
        // Validation errors (422) carry the reason in `errors`: "A pull request already exists…".
        if let Some(detail) = v.as_ref().and_then(|v| v.get("errors")).and_then(Value::as_array).and_then(|e| e.first())
            .map(|e| if e.is_string() { e.as_str().unwrap_or("").to_owned() } else { Some(s(e, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{} {}", s(e, "field"), s(e, "code"))) })
            .filter(|d| !d.trim().is_empty())
        {
            msg = format!("{msg}: {}", detail.trim());
        }

        if (r.status == 403 || r.status == 429) && r.remaining.as_deref() == Some("0") && r.retry_after.is_none() {
            let when = if let Some(t) = r.reset {
                let secs = (t - self.now).max(1) as u64;
                self.record_backoff(secs);
                format!(" Try again in {} minutes.", (secs / 60).max(1))
            } else {
                String::new()
            };
            return format!("API rate limit reached.{when} With GITHUB_TOKEN the limit goes up to 5,000 requests an hour.");
        }

        let is_secondary = r.status == 429
            || (r.status == 403
                && (r.retry_after.is_some()
                    || msg.to_ascii_lowercase().contains("rate limit")
                    || msg.to_ascii_lowercase().contains("abuse")));

        if is_secondary {
            if let Some(secs) = r.retry_after {
                self.record_backoff(secs);
                if !msg.is_empty() && !msg.eq_ignore_ascii_case("rate limited") {
                    return format!("GitHub rate limit reached: {msg}. Retrying in {secs} s.");
                } else {
                    return format!("rate limited, retrying in {secs} s");
                }
            }
            if let Some(t) = r.reset {
                let secs = (t - self.now).max(1) as u64;
                self.record_backoff(secs);
                let mins = (secs / 60).max(1);
                if !msg.is_empty() {
                    return format!("GitHub rate limit reached: {msg}. Try again in {mins} minutes.");
                } else {
                    return format!("GitHub rate limit reached. Try again in {mins} minutes.");
                }
            }
            if !msg.is_empty() {
                return format!("GitHub rate limit reached: {msg}.");
            }
            return "GitHub rate limit reached.".into();
        }

        match r.status {
            401 => "GitHub rejected the token (401): check the token in Settings, or GITHUB_TOKEN in your .env file.".into(),
            403 => format!("GitHub refused (403): {msg}. The token needs write access to this repository (Contents, Issues and Pull requests)."),
            404 => format!("Not found (404): {msg}. Check GITHUB_USER or the token's permissions."),
            code if msg.is_empty() => format!("GitHub API error {code}."),
            code => format!("GitHub error {code}: {msg}"),
        }
    }

    /// Explains failures on workflow re-run endpoints (`POST .../actions/runs/{id}/rerun*`),
    /// highlighting the Actions write permission when GitHub refuses with 403 or 404.
    pub fn explain_actions_write(&self, r: &Resp) -> String {
        let v = serde_json::from_str::<Value>(&r.body).ok();
        let msg = v.as_ref().map(|v| s(v, "message")).unwrap_or_default();
        let lower = msg.to_ascii_lowercase();
        if r.status == 429
            || r.remaining.as_deref() == Some("0")
            || r.retry_after.is_some()
            || lower.contains("rate limit")
            || lower.contains("abuse")
        {
            return self.explain(r);
        }
        let detail = if msg.is_empty() { String::new() } else { format!(": {msg}") };
        match r.status {
            401 => "GitHub rejected the token (401): check the token in Settings, or GITHUB_TOKEN in your .env file.".into(),
            403 => format!(
                "GitHub refused (403){detail}. Re-running workflows requires the Actions write permission (\"Actions: Read and write\" on fine-grained tokens, or the \"repo\" / \"workflow\" scope on classic tokens)."
            ),
            404 => format!(
                "Not found (404){detail}. Re-running workflows requires a token with the Actions write permission (\"Actions: Read and write\" on fine-grained tokens, or \"repo\" / \"workflow\" on classic tokens)."
            ),
            _ => self.explain(r),
        }
    }

    /// Every GET goes through here: with the tag of the answer it already has, so GitHub answers
    /// 304 when nothing changed and the kept answer comes back as if it had been sent again
    /// (searches excepted, see `etag`).
    async fn fetch(&self, path: &str, accept: &str) -> Result<Resp, String> {
        use crate::etag;
        let key = etag::key(&self.api_base, accept, path);
        let known = if etag::conditional(path) { etag::get(&key) } else { None };
        let mut r = {
            let _place = match &self.gate {
                Some(g) => Some(g.enter().await),
                None => None,
            };
            self.t.send_if(path, accept, known.as_ref().map(|k| k.etag.as_str())).await?
        };
        match (r.status, known) {
            (304, Some(k)) => {
                etag::reused();
                r.status = 200;
                r.body = k.body.to_string();
                r.link = r.link.or(k.link);
                r.etag = Some(k.etag);
            }
            (200, _) if etag::conditional(path) => {
                if let Some(tag) = r.etag.clone() {
                    etag::put(self.tags, key, etag::Entry { etag: tag, body: r.body.as_str().into(), link: r.link.clone() });
                }
            }
            _ => {}
        }
        Ok(r)
    }

    /// Explains a refused write to the releases (`POST`/`PATCH .../releases…`): the Contents
    /// permission on 403 and 404, and GitHub's reason in plain words for a tag it won't take.
    pub fn explain_release_write(&self, r: &Resp) -> String {
        let v = serde_json::from_str::<Value>(&r.body).ok();
        let msg = v.as_ref().map(|v| s(v, "message")).unwrap_or_default();
        let lower = msg.to_ascii_lowercase();
        if r.status == 429 || r.remaining.as_deref() == Some("0") || r.retry_after.is_some() || lower.contains("rate limit") || lower.contains("abuse") {
            return self.explain(r);
        }
        let first = v.as_ref().and_then(|v| v.get("errors")).and_then(Value::as_array).and_then(|e| e.first());
        let (field, code) = first.map(|e| (s(e, "field"), s(e, "code"))).unwrap_or_default();
        let detail = if msg.is_empty() { String::new() } else { format!(": {msg}") };
        const NEEDS: &str = "Releases need \"Contents: Read and write\" on a fine-grained token, or the \"repo\" scope on a classic one.";
        match (r.status, field.as_str(), code.as_str()) {
            (401, ..) => "GitHub rejected the token (401): check the token in Settings, or GITHUB_TOKEN in your .env file.".into(),
            (403, ..) => format!("GitHub refused (403){detail}. {NEEDS}"),
            (404, ..) => format!("Not found (404){detail}. {NEEDS} The token must also reach this repository."),
            (422, "tag_name", "already_exists") => "This tag already has a release (GitHub error 422): pick another tag.".into(),
            (422, "tag_name", _) => "GitHub doesn't take this tag name (error 422): pick another one.".into(),
            (422, "target_commitish", _) => "GitHub can't make the tag from this branch (error 422): check that the branch exists.".into(),
            _ => self.explain(r),
        }
    }

    async fn get_resp(&self, path: &str) -> Result<(Value, Resp), String> {
        let r = self.fetch(path, JSON).await?;
        match r.status {
            202 | 204 => Ok((Value::Null, r)),
            200..=299 => {
                let v = serde_json::from_str(&r.body).map_err(|e| format!("Invalid response from GitHub: {e}"))?;
                Ok((v, r))
            }
            _ => Err(self.explain(&r)),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        self.get_resp(path).await.map(|(v, _)| v)
    }

    async fn get_raw(&self, path: &str) -> Result<Option<String>, String> {
        let r = self.fetch(path, "application/vnd.github.raw+json").await?;
        match r.status {
            200..=299 => Ok(Some(r.body)),
            404 => Ok(None),
            _ => Err(self.explain(&r)),
        }
    }

    /// One request through the same transport, answered as it came (board sync reads the
    /// status itself: a 409 there means "changed meanwhile", not a failure).
    pub async fn send(&self, method: &str, path: &str, accept: &str, body: &str) -> Result<Resp, String> {
        self.t.send_body(method, path, accept, body).await
    }

    /// Sends a JSON body and returns the JSON answer (writes: POST, PATCH, PUT).
    async fn write(&self, method: &str, path: &str, body: Value) -> Result<Value, String> {
        let r = self.t.send_body(method, path, JSON, &body.to_string()).await?;
        match r.status {
            200..=299 => Ok(serde_json::from_str(&r.body).unwrap_or(Value::Null)),
            _ => Err(self.explain(&r)),
        }
    }

    pub async fn profile(&self, cfg: &Config) -> Result<Profile, String> {
        let v = match (&cfg.user, self.t.has_token()) {
            (Some(u), _) => self.get(&format!("/users/{u}")).await?,
            (None, true) => self.get("/user").await?,
            (None, false) => return Err("Set GITHUB_USER or GITHUB_TOKEN in your .env file.".into()),
        };
        let login = s(&v, "login");
        let name = Some(s(&v, "name")).filter(|x| !x.is_empty()).unwrap_or_else(|| login.clone());
        Ok(Profile { login, name, bio: s(&v, "bio") })
    }

    pub(crate) async fn viewer_login(&self) -> Option<String> {
        if !self.t.has_token() {
            return None;
        }
        self.get("/user").await.ok().map(|v| s(&v, "login"))
    }
}

/// A branch name GitHub will take: no spaces, no `..`, no leading `-` or `/`, none of `~^:?*[\`.
pub fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
}

/// Percent-encodes a query value (a branch name in `?ref=`).
fn enc(v: &str) -> String {
    v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Like `enc`, but keeps `/` (a branch name inside a path).
fn enc_path(v: &str) -> String {
    v.split('/').map(enc).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests;
