//! The saved copy of the last load: RepoHub opens with it at once and replaces it when GitHub
//! answers (stale-while-revalidate).
//!
//! - **Where:** desktop `<data dir>/cache/<host>-<account>.json`, readable by you only (mode
//!   0600, like the settings file); web and Android: the browser's storage (`repohub.cache`),
//!   when it fits (the largest parts are left out first).
//! - **What:** the lists (`Data`), the repository pages already read (`Detail`), and the answers
//!   GitHub tagged (`etag`), so the first refresh asks "changed since?" instead of reading again.
//!   Never the token or anything made from it: the copy is built from `Data`, not from `Config`.
//!   Not the warnings of that load, nor the sample data.
//! - **Format:** JSON with a format number ([`FORMAT`]); a copy of another format, of another
//!   account or API, or that can't be read is ignored and removed.
//! - **Size:** at most [`LIMIT`] (the browser: [`WEB_LIMIT`]); tagged answers go first, then
//!   repository pages.
//! - **Gone:** on sign-out, on an account switch and with Settings → About → Clear saved data.
//!   `REPOHUB_CACHE=off` turns it off.
//!
//! A write runs in the background; a sign-out meanwhile wins (see [`forget_all`]).
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::config::Config;
use crate::etag;
use crate::model::{Checks, Commit, Data, Detail, Issue, Profile, Pull, PullState, Repo, Run};
use crate::state::Load;
use crate::{App, net, state};

/// The format of the copy. Change it whenever what's written changes meaning; older copies are
/// then ignored and removed.
pub const FORMAT: u64 = 1;

/// The largest copy written on the desktop.
pub const LIMIT: usize = 12 << 20;

/// The largest copy written to the browser's storage, which holds about 5 million characters
/// for the whole site.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const WEB_LIMIT: usize = 2 << 20;

/// What repository pages may take of the copy.
const DETAILS_BUDGET: usize = 2 << 20;

/// The conversations kept in memory to show at once when opened again.
const THREADS_KEPT: usize = 20;

/// One saved copy, as read or about to be written.
#[derive(Clone, Debug, Default)]
pub struct Copy {
    /// When its data was loaded (Unix seconds).
    pub saved: i64,
    pub api: String,
    /// `GITHUB_USER`, or empty for the token's own account.
    pub account: String,
    pub data: Data,
    pub details: Vec<((String, String), Detail)>,
    pub etags: Vec<(String, etag::Entry)>,
}

/// What Settings → About says about the copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub bytes: usize,
    pub saved: i64,
}

/// What the app knows about where the lists on screen come from, and what it keeps to show at
/// once (one field of `App`).
#[derive(Default)]
pub struct Fresh {
    /// The lists on screen are the saved copy: GitHub hasn't answered yet in this session.
    pub from_copy: bool,
    /// Only the profile and the repositories are in: pull requests, issues, runs and activity
    /// are still loading (see `github::load_all_with`).
    pub partial: bool,
    /// Repository pages kept from before a refresh that are read again when opened.
    pub stale: HashSet<(String, String)>,
    /// Conversations read in this session, most recent last.
    pub threads: Vec<((String, u32), crate::thread::Thread)>,
    /// The copy on disk (or in the browser), once read or written.
    pub info: Option<Info>,
    /// Where the first load reports until the window (and the app's handle) exists.
    pub early: Option<net::Mailbox>,
}

/// Whether the copy is kept: `REPOHUB_CACHE=off` (or `0`, `false`, `no`) turns it off.
pub fn enabled() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        !std::env::var("REPOHUB_CACHE").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        true
    }
}

/// The account a copy belongs to: `GITHUB_USER`, or empty for the token's own account (whose
/// file is then named `<host>-token.json`).
pub fn account_of(cfg: &Config) -> String {
    cfg.user.as_deref().map(str::to_ascii_lowercase).unwrap_or_default()
}

/// `api.github.com`, `ghe.example.com`, `127.0.0.1_8080`: the API's host as a file name.
pub fn host_of(api: &str) -> String {
    let rest = api.split_once("://").map_or(api, |(_, r)| r);
    let host = rest.split('/').next().unwrap_or(rest);
    let name: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    if name.is_empty() { "github".into() } else { name }
}

// ------------------------------------------------------------------ the format

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn i(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn f(v: &Value, k: &str) -> f32 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn opt(v: &Value, k: &str) -> Option<usize> {
    v.get(k).and_then(Value::as_u64).map(|x| x as usize)
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect()
}

fn list<'a>(v: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
    v.get(k).and_then(Value::as_array).into_iter().flatten()
}

fn labels_json(l: &[(String, f32)]) -> Value {
    Value::Array(l.iter().map(|(name, hue)| json!([name, hue])).collect())
}

fn labels_of(v: &Value, k: &str) -> Vec<(String, f32)> {
    list(v, k).filter_map(|x| Some((x.get(0)?.as_str()?.to_owned(), x.get(1)?.as_f64()? as f32))).collect()
}

fn checks_json(c: Checks) -> &'static str {
    match c {
        Checks::Passing => "passing",
        Checks::Failing => "failing",
        Checks::Running => "running",
    }
}

fn checks_of(v: Option<&Value>) -> Option<Checks> {
    match v?.as_str()? {
        "passing" => Some(Checks::Passing),
        "failing" => Some(Checks::Failing),
        "running" => Some(Checks::Running),
        _ => None,
    }
}

fn repo_json(r: &Repo) -> Value {
    json!({
        "fullName": r.full_name, "owner": r.owner, "name": r.name, "description": r.description,
        "language": r.language, "hue": r.hue, "stars": r.stars, "forks": r.forks, "issues": r.issues,
        "pushed": r.pushed, "private": r.private, "fork": r.fork, "archived": r.archived,
        "branch": r.branch, "topics": r.topics, "url": r.url, "starred": r.starred,
    })
}

fn repo_of(v: &Value) -> Repo {
    let r = Repo {
        full_name: s(v, "fullName"),
        owner: s(v, "owner"),
        name: s(v, "name"),
        description: s(v, "description"),
        language: s(v, "language"),
        hue: f(v, "hue"),
        stars: n(v, "stars") as u32,
        forks: n(v, "forks") as u32,
        issues: n(v, "issues") as u32,
        pushed: i(v, "pushed"),
        private: b(v, "private"),
        fork: b(v, "fork"),
        archived: b(v, "archived"),
        branch: s(v, "branch"),
        topics: strings(v, "topics"),
        url: s(v, "url"),
        starred: b(v, "starred"),
        keys: std::sync::OnceLock::new(),
    };
    r.precompute_keys();
    r
}

fn pull_json(p: &Pull) -> Value {
    json!({
        "number": p.number, "title": p.title, "repo": p.repo, "author": p.author,
        "state": match p.state { PullState::Open => "open", PullState::Draft => "draft", PullState::Merged => "merged", PullState::Closed => "closed" },
        "checks": p.checks.map(checks_json), "updatedAt": p.updated_at, "comments": p.comments, "url": p.url,
        "labels": labels_json(&p.labels), "assignees": p.assignees, "reviewRequested": p.review_requested,
    })
}

fn pull_of(v: &Value) -> Pull {
    Pull {
        number: n(v, "number") as u32,
        title: s(v, "title"),
        repo: s(v, "repo"),
        author: s(v, "author"),
        state: match s(v, "state").as_str() {
            "draft" => PullState::Draft,
            "merged" => PullState::Merged,
            "closed" => PullState::Closed,
            _ => PullState::Open,
        },
        checks: checks_of(v.get("checks")),
        updated_at: i(v, "updatedAt"),
        comments: n(v, "comments") as u32,
        url: s(v, "url"),
        labels: labels_of(v, "labels"),
        assignees: strings(v, "assignees"),
        review_requested: b(v, "reviewRequested"),
    }
}

fn issue_json(x: &Issue) -> Value {
    json!({
        "number": x.number, "title": x.title, "repo": x.repo,
        "label": x.label.as_ref().map(|(name, hue)| json!([name, hue])),
        "createdAt": x.created_at, "comments": x.comments, "url": x.url,
        "labels": labels_json(&x.labels), "assignees": x.assignees,
    })
}

fn issue_of(v: &Value) -> Issue {
    Issue {
        number: n(v, "number") as u32,
        title: s(v, "title"),
        repo: s(v, "repo"),
        label: v.get("label").and_then(|l| Some((l.get(0)?.as_str()?.to_owned(), l.get(1)?.as_f64()? as f32))),
        created_at: i(v, "createdAt"),
        comments: n(v, "comments") as u32,
        url: s(v, "url"),
        labels: labels_of(v, "labels"),
        assignees: strings(v, "assignees"),
    }
}

fn run_json(r: &Run) -> Value {
    json!({
        "id": r.id, "workflow": r.workflow, "repo": r.repo, "branch": r.branch, "checks": checks_json(r.checks),
        "duration": r.duration, "updatedAt": r.updated_at, "url": r.url, "pulls": r.pull_numbers,
    })
}

fn run_of(v: &Value) -> Run {
    Run {
        id: n(v, "id"),
        workflow: s(v, "workflow"),
        repo: s(v, "repo"),
        branch: s(v, "branch"),
        checks: checks_of(v.get("checks")).unwrap_or(Checks::Passing),
        duration: s(v, "duration"),
        updated_at: i(v, "updatedAt"),
        url: s(v, "url"),
        pull_numbers: list(v, "pulls").filter_map(|x| x.as_u64().map(|x| x as u32)).collect(),
    }
}

/// The lists as JSON: everything but the warnings of that load, its back-off and `sample`.
pub fn data_json(d: &Data) -> Value {
    json!({
        "profile": { "login": d.profile.login, "name": d.profile.name, "bio": d.profile.bio },
        "repos": d.repos.iter().map(repo_json).collect::<Vec<_>>(),
        "pulls": d.pulls.iter().map(pull_json).collect::<Vec<_>>(),
        "issues": d.issues.iter().map(issue_json).collect::<Vec<_>>(),
        "runs": d.runs.iter().map(run_json).collect::<Vec<_>>(),
        "activity": d.activity,
        "pullsFailed": d.pulls_failed, "issuesFailed": d.issues_failed, "runsFailed": d.runs_failed,
        "openPullsTotal": d.open_pulls_total, "closedPullsTotal": d.closed_pulls_total,
        "issuesTotal": d.issues_total, "runsTotal": d.runs_total, "reposCapped": d.repos_capped,
        "reviewTotal": d.review_total,
    })
}

pub fn data_of(v: &Value) -> Option<Data> {
    let p = v.get("profile")?;
    let login = s(p, "login");
    if login.is_empty() {
        return None;
    }
    Some(Data {
        profile: Profile { login, name: s(p, "name"), bio: s(p, "bio") },
        repos: list(v, "repos").map(repo_of).collect(),
        pulls: list(v, "pulls").map(pull_of).collect(),
        issues: list(v, "issues").map(issue_of).collect(),
        runs: list(v, "runs").map(run_of).collect(),
        activity: list(v, "activity").map(|x| x.as_u64().unwrap_or(0) as u32).collect(),
        warnings: Vec::new(),
        pulls_failed: b(v, "pullsFailed"),
        issues_failed: b(v, "issuesFailed"),
        runs_failed: b(v, "runsFailed"),
        retry_after: None,
        sample: false,
        open_pulls_total: opt(v, "openPullsTotal"),
        closed_pulls_total: opt(v, "closedPullsTotal"),
        issues_total: opt(v, "issuesTotal"),
        runs_total: n(v, "runsTotal") as usize,
        repos_capped: b(v, "reposCapped"),
        review_total: opt(v, "reviewTotal"),
    })
}

fn detail_json(d: &Detail) -> Value {
    json!({
        "files": d.files.iter().map(|(name, dir)| json!([name, dir])).collect::<Vec<_>>(),
        "readme": d.readme,
        "languages": d.languages.iter().map(|(name, bytes)| json!([name, bytes])).collect::<Vec<_>>(),
        "commit": d.commit.as_ref().map(|c| json!({ "author": c.author, "message": c.message, "sha": c.sha, "at": c.committed_at })),
    })
}

fn detail_of(v: &Value) -> Detail {
    Detail {
        error: None,
        readme_error: None,
        files: list(v, "files").filter_map(|x| Some((x.get(0)?.as_str()?.to_owned(), x.get(1)?.as_bool()?))).collect(),
        readme: v.get("readme").and_then(Value::as_str).map(str::to_owned),
        languages: list(v, "languages").filter_map(|x| Some((x.get(0)?.as_str()?.to_owned(), x.get(1)?.as_u64()?))).collect(),
        commit: v.get("commit").filter(|c| c.is_object()).map(|c| Commit { author: s(c, "author"), message: s(c, "message"), sha: s(c, "sha"), committed_at: i(c, "at") }),
    }
}

/// The copy as text, within `limit` bytes: tagged answers are left out first (the least
/// recently used first), then repository pages. `None` when even the lists don't fit.
pub fn encode(copy: &Copy, limit: usize) -> Option<String> {
    let head = json!({
        "format": FORMAT, "saved": copy.saved, "api": copy.api, "account": copy.account,
        "data": data_json(&copy.data),
    });
    let mut text = head.to_string();
    const TAIL: &str = ",\"details\":[],\"etags\":[]";
    if text.len() + TAIL.len() > limit {
        return None;
    }
    // Then repository pages and tagged answers, appended while they fit (each written once).
    let mut left = limit - text.len() - TAIL.len();
    let mut details = Vec::new();
    let mut budget = DETAILS_BUDGET.min(left);
    for ((repo, branch), d) in &copy.details {
        let t = json!({ "repo": repo, "branch": branch, "detail": detail_json(d) }).to_string();
        if t.len() < budget {
            budget -= t.len() + 1;
            left -= t.len() + 1;
            details.push(t);
        }
    }
    let mut etags = Vec::new();
    for (key, e) in &copy.etags {
        let t = json!({ "key": key, "etag": e.etag, "link": e.link, "body": &*e.body }).to_string();
        if t.len() < left {
            left -= t.len() + 1;
            etags.push(t);
        }
    }
    text.pop(); // the closing brace
    text.push_str(",\"details\":[");
    text.push_str(&details.join(","));
    text.push_str("],\"etags\":[");
    text.push_str(&etags.join(","));
    text.push_str("]}");
    Some(text)
}

/// Reads a copy written by [`encode`]: `None` for another format, another account or API, or
/// anything that isn't one.
pub fn decode(text: &str, api: &str, account: &str) -> Option<Copy> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("format").and_then(Value::as_u64) != Some(FORMAT) || s(&v, "api") != api || s(&v, "account") != account {
        return None;
    }
    let data = data_of(v.get("data")?)?;
    let details = list(&v, "details")
        .filter_map(|d| Some(((d.get("repo")?.as_str()?.to_owned(), d.get("branch")?.as_str()?.to_owned()), detail_of(d.get("detail")?))))
        .collect();
    let etags = list(&v, "etags")
        .filter_map(|e| Some((e.get("key")?.as_str()?.to_owned(), etag::Entry {
            etag: e.get("etag")?.as_str()?.to_owned(),
            body: Arc::from(e.get("body")?.as_str()?),
            link: e.get("link").and_then(Value::as_str).map(str::to_owned),
        })))
        .collect();
    Some(Copy { saved: i(&v, "saved"), api: api.to_owned(), account: account.to_owned(), data, details, etags })
}

// ------------------------------------------------------------------ where it lives

/// Counts [`forget_all`]: a write that started before the last one doesn't happen.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Held while a copy is written or removed, so a removal never runs in the middle of a write; it
/// also holds the number of the newest write done, so an older one finishing later is skipped.
static WRITING: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// The folder of the copies (desktop).
#[cfg(not(target_arch = "wasm32"))]
pub fn folder() -> std::path::PathBuf {
    crate::notes::data_dir().join("cache")
}

/// The copy's file for this API and account (desktop).
#[cfg(not(target_arch = "wasm32"))]
pub fn file(api: &str, account: &str) -> std::path::PathBuf {
    let who = if account.is_empty() { "token".to_owned() } else { account.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect() };
    folder().join(format!("{}-{who}.json", host_of(api)))
}

/// Where the copy is, in words, for Settings.
pub fn place(cfg: &Config) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        file(&cfg.api, &account_of(cfg)).display().to_string()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = cfg;
        "this browser's storage for the page".to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
const WEB_KEY: &str = "repohub.cache";

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<wasm_bindgen::JsValue> {
    js_sys::Reflect::get(&js_sys::global(), &"localStorage".into()).ok().filter(|s| !s.is_undefined() && !s.is_null())
}

/// Calls a method of the browser's storage: `Err` when it throws (full, or storage turned off).
#[cfg(target_arch = "wasm32")]
fn storage_call(method: &str, args: &[&str]) -> Result<wasm_bindgen::JsValue, ()> {
    use wasm_bindgen::JsCast;
    let st = storage().ok_or(())?;
    let f: js_sys::Function = js_sys::Reflect::get(&st, &method.into()).map_err(|_| ())?.dyn_into().map_err(|_| ())?;
    let args: js_sys::Array = args.iter().map(|a| wasm_bindgen::JsValue::from_str(a)).collect();
    f.apply(&st, &args).map_err(|_| ())
}

/// The saved copy for `cfg`'s API and account, if there's a good one. One that can't be used is
/// removed.
pub fn read(cfg: &Config) -> Option<(Copy, usize)> {
    if !enabled() {
        return None;
    }
    let account = account_of(cfg);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = file(&cfg.api, &account);
        let text = std::fs::read_to_string(&path).ok()?;
        let copy = decode(&text, &cfg.api, &account);
        if copy.is_none() {
            let _ = std::fs::remove_file(&path);
        }
        copy.map(|c| (c, text.len()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let text = storage_call("getItem", &[WEB_KEY]).ok()?.as_string()?;
        let copy = decode(&text, &cfg.api, &account);
        if copy.is_none() {
            let _ = storage_call("removeItem", &[WEB_KEY]);
        }
        copy.map(|c| (c, text.len()))
    }
}

/// Writes a copy now, unless the copies were removed since `generation` or a newer write
/// (`seq`) already happened. Returns the bytes written.
pub fn write(cfg_api: &str, account: &str, copy: &Copy, generation: u64, seq: u64) -> Result<usize, String> {
    let mut newest = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if self::generation() != generation || *newest > seq {
        return Ok(0);
    }
    *newest = seq;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let text = encode(copy, LIMIT).ok_or("The lists are too large to save.")?;
        let path = file(cfg_api, account);
        write_private(&path, &text)?;
        Ok(text.len())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (cfg_api, account);
        // The whole copy if the browser takes it; else without the tagged answers, then without
        // the repository pages.
        let lighter = [copy.clone(), Copy { etags: Vec::new(), ..copy.clone() }, Copy { etags: Vec::new(), details: Vec::new(), ..copy.clone() }];
        for c in &lighter {
            if let Some(text) = encode(c, WEB_LIMIT)
                && storage_call("setItem", &[WEB_KEY, &text]).is_ok()
            {
                return Ok(text.len());
            }
        }
        let _ = storage_call("removeItem", &[WEB_KEY]);
        Err("The browser's storage is full.".into())
    }
}

/// Writes `text` to `path` readable by its owner only, through a file moved into place so a
/// crash never leaves half a copy.
#[cfg(not(target_arch = "wasm32"))]
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("No folder for the saved copy.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    f.write_all(text.as_bytes()).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

/// Removes every saved copy, and stops any write still on its way.
pub fn forget_all() {
    let _w = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(dir) = std::fs::read_dir(folder()) {
        for e in dir.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json" || x == "tmp") {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = storage_call("removeItem", &[WEB_KEY]);
}

/// Prints how long since the process started when `REPOHUB_TRACE=1`: `time: <what> 123 ms`.
pub fn trace_time(what: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(std::time::Instant::now);
        if what.is_empty() {
            return;
        }
        state::trace("time", &what, &format!("{:.0} ms", start.elapsed().as_secs_f64() * 1000.0));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = what;
}

// ------------------------------------------------------------------ the app

impl App {
    /// At start: shows the saved copy for this account at once, while the first load runs.
    pub(crate) fn show_saved_copy(&mut self) {
        if !self.cfg.live() || !self.data.is_empty() {
            return;
        }
        let Some((copy, bytes)) = read(&self.cfg) else { return };
        etag::import(etag::generation(), copy.etags);
        self.data = copy.data;
        self.data_gen = self.data_gen.wrapping_add(1);
        self.details = copy.details.into_iter().map(|(k, d)| (k, Load::Ready(d))).collect();
        // Every page of the copy is read again when it's opened.
        self.fresh.stale = self.details.keys().cloned().collect();
        self.refresh_clone_cache();
        self.loaded_at = Some(copy.saved);
        self.loaded_clock = None;
        self.fresh.from_copy = true;
        self.fresh.info = Some(Info { bytes, saved: copy.saved });
        state::trace("shown", &"nothing", &"saved copy");
        trace_time("saved copy shown");
    }

    /// After a load: saves what's on screen for the next start (in the background on the
    /// desktop).
    pub(crate) fn save_copy(&mut self) {
        if !enabled() || self.data.sample || self.data.is_empty() || !self.cfg.live() {
            return;
        }
        // Repository pages in the lists' order (most recently pushed first), without errors.
        let mut details = Vec::new();
        for r in &self.data.repos {
            for ((full, branch), load) in &self.details {
                if full == &r.full_name
                    && let Load::Ready(d) = load
                    && d.error.is_none()
                    && d.readme_error.is_none()
                {
                    details.push(((full.clone(), branch.clone()), d.clone()));
                }
            }
        }
        let copy = Copy {
            saved: self.loaded_at.unwrap_or_else(net::now),
            api: self.cfg.api.clone(),
            account: account_of(&self.cfg),
            data: Data { warnings: Vec::new(), retry_after: None, ..self.data.clone() },
            details,
            etags: etag::export(LIMIT),
        };
        let (api, account, generation) = (copy.api.clone(), copy.account.clone(), generation());
        let seq = WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let saved = copy.saved;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(h) = self.handle.clone() else {
                // Headless stills (`REPOHUB_BLOCKING=1`) and tests: written now.
                if let Some(bytes) = write(&api, &account, &copy, generation, seq).ok().filter(|b| *b > 0) {
                    self.fresh.info = Some(Info { bytes, saved });
                }
                return;
            };
            let account_gen = self.load_gen;
            net::spawn(h, move || async move { write(&api, &account, &copy, generation, seq) }, move |s: &mut App, r: Result<usize, String>| {
                if s.load_gen == account_gen && let Some(bytes) = r.ok().filter(|b| *b > 0) {
                    s.fresh.info = Some(Info { bytes, saved });
                    trace_time("saved copy written");
                }
            });
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(bytes) = write(&api, &account, &copy, generation, seq).ok().filter(|b| *b > 0) {
            self.fresh.info = Some(Info { bytes, saved });
        }
    }

    /// Sign-out and account switch: the copies, the tagged answers and what's kept in memory go.
    pub(crate) fn forget_copy(&mut self) {
        forget_all();
        etag::clear();
        self.fresh = Fresh::default();
    }

    /// Settings → About → Clear saved data: the lists on screen stay; nothing is kept for the
    /// next start until the next load.
    pub(crate) fn clear_saved_data(&mut self) {
        forget_all();
        etag::clear();
        self.fresh.info = None;
        self.notify("Saved data cleared. RepoHub saves it again after the next load.");
    }

    /// The profile and the repositories of a load that's still running: shown when nothing else
    /// is on screen (the first start), so the page doesn't wait for the slowest list.
    pub(crate) fn apply_early(&mut self, epoch: u64, d: Data) {
        if epoch != self.load_gen || self.status != state::Status::Loading || !self.data.is_empty() {
            return;
        }
        self.data = d;
        self.data_gen = self.data_gen.wrapping_add(1);
        self.refresh_clone_cache();
        self.fresh.partial = true;
        state::trace("shown", &"nothing", &"profile and repositories");
        trace_time("profile and repositories shown");
        self.restore_place();
    }

    /// Pull requests, issues, runs and activity are still on their way (their lists show a
    /// skeleton).
    pub(crate) fn lists_loading(&self) -> bool {
        self.fresh.partial && self.status.is_busy()
    }

    /// A conversation read in this session, shown at once when it's opened again (and read
    /// again behind it).
    pub(crate) fn remembered_thread(&self, repo: &str, number: u32) -> Option<crate::thread::Thread> {
        self.fresh.threads.iter().rev().find(|((r, n), _)| *n == number && r.eq_ignore_ascii_case(repo)).map(|(_, t)| t.clone())
    }

    pub(crate) fn remember_thread(&mut self, repo: &str, number: u32, t: crate::thread::Thread) {
        let kept = &mut self.fresh.threads;
        kept.retain(|((r, n), _)| !(*n == number && r.eq_ignore_ascii_case(repo)));
        kept.push(((repo.to_owned(), number), t));
        if kept.len() > THREADS_KEPT {
            kept.remove(0);
        }
    }

    /// "Updating…" while a load runs over lists already on screen, "Data from 2 hours ago" for
    /// the saved copy, "Updated 3 minutes ago" once GitHub answered.
    pub(crate) fn freshness_words(&self, age: Option<i64>) -> Option<String> {
        if self.status.is_busy() && !self.data.is_empty() && !self.data.sample {
            return Some("Updating…".into());
        }
        let age = age?;
        let ago = crate::model::ago(self.now - age, self.now);
        Some(if self.fresh.from_copy { format!("Data from {ago}") } else { format!("Updated {ago}") })
    }
}

/// The details kept through a refresh: every page read without errors, of a repository still
/// listed; those of repositories pushed to since are marked to be read again when opened.
pub fn keep_details(old: &Data, new: &Data, details: HashMap<(String, String), Load<Detail>>, stale: &mut HashSet<(String, String)>) -> HashMap<(String, String), Load<Detail>> {
    let pushed = |data: &Data, full: &str| data.repos.iter().find(|r| r.full_name == full).map(|r| r.pushed);
    details.into_iter()
        .filter(|((full, _), v)| matches!(v, Load::Ready(d) if d.error.is_none() && d.readme_error.is_none()) && pushed(new, full).is_some())
        .inspect(|(key, _)| {
            if pushed(old, &key.0) != pushed(new, &key.0) {
                stale.insert(key.clone());
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Data {
        let mut d = Data::sample_at(1_790_000_000);
        d.sample = false;
        d.warnings = vec!["Issues couldn't load: x".into()];
        d.retry_after = Some(60);
        d.open_pulls_total = Some(12);
        d.issues_total = Some(4);
        d.pulls[1].checks = None;
        d
    }

    #[test]
    fn the_lists_round_trip_without_warnings_or_backoff() {
        let d = data();
        let back = data_of(&data_json(&d)).expect("reads back");
        assert_eq!(data_json(&back), data_json(&d), "every list and count comes back");
        assert!(back.warnings.is_empty() && back.retry_after.is_none() && !back.sample, "not the load's warnings, back-off or sample flag");
        assert_eq!(back.repos[3].full_name, d.repos[3].full_name);
        assert_eq!(back.repos[3].keys().name_lower, d.repos[3].name.to_lowercase(), "search keys are ready");
        assert_eq!(back.pulls[1].checks, None);
        assert_eq!(back.pulls[2].review_requested, d.pulls[2].review_requested);
        assert_eq!(back.issues[2].assignees, d.issues[2].assignees);
        assert_eq!(back.runs[1].pull_numbers, vec![9]);
    }

    #[test]
    fn a_copy_of_another_format_account_or_api_is_not_read() {
        let copy = Copy { saved: 5, api: "https://api.github.com".into(), account: "octo".into(), data: data(), details: vec![(("a/b".into(), "main".into()), crate::model::sample_detail(&data().repos[0]))], etags: vec![("k".into(), etag::Entry { etag: "W/\"1\"".into(), body: "{\"a\":\"\\\"quoted\\\"\"}".into(), link: None })] };
        let text = encode(&copy, LIMIT).unwrap();
        let back = decode(&text, "https://api.github.com", "octo").expect("same account");
        assert_eq!(back.saved, 5);
        assert_eq!(back.details.len(), 1);
        assert_eq!(back.details[0].1.readme, copy.details[0].1.readme);
        assert_eq!(back.etags, copy.etags, "tagged answers come back as they were");
        assert!(decode(&text, "https://api.github.com", "other").is_none(), "another account");
        assert!(decode(&text, "https://ghe.example/api/v3", "octo").is_none(), "another API");
        let older = text.replacen(&format!("\"format\":{FORMAT}"), "\"format\":0", 1);
        assert!(decode(&older, "https://api.github.com", "octo").is_none(), "another format");
        assert!(decode("{not json", "https://api.github.com", "octo").is_none());
        assert!(decode(&text[..text.len() / 2], "https://api.github.com", "octo").is_none(), "half a file");
    }

    #[test]
    fn the_size_limit_drops_tagged_answers_first_then_pages() {
        let big = "x".repeat(50_000);
        let copy = Copy {
            data: data(),
            details: vec![(("a/b".into(), "main".into()), Detail { readme: Some(big.clone()), ..Detail::default() })],
            etags: (0..4).map(|i| (format!("k{i}"), etag::Entry { etag: "t".into(), body: big.as_str().into(), link: None })).collect(),
            ..Copy::default()
        };
        let lists = encode(&Copy { details: Vec::new(), etags: Vec::new(), ..copy.clone() }, LIMIT).unwrap().len();
        let text = encode(&copy, lists + 120_000).unwrap();
        let back = decode(&text, "", "").unwrap();
        assert_eq!(back.details.len(), 1, "the page fits");
        assert_eq!(back.etags.len(), 1, "only what fits of the tagged answers");
        assert!(text.len() <= lists + 120_000);
        assert!(encode(&copy, lists - 1).is_none(), "the lists alone don't fit: nothing");
    }

    #[test]
    fn names_of_copies() {
        assert_eq!(host_of("https://api.github.com"), "api.github.com");
        assert_eq!(host_of("https://GHE.example.com/api/v3"), "ghe.example.com");
        assert_eq!(host_of("http://127.0.0.1:8080"), "127.0.0.1_8080");
        #[cfg(not(target_arch = "wasm32"))]
        {
            let f = file("https://api.github.com", "");
            assert!(f.ends_with("cache/api.github.com-token.json"), "{}", f.display());
            assert!(file("http://127.0.0.1:9", "../etc").ends_with("cache/127.0.0.1_9-___etc.json"), "no folder in the name");
        }
    }

    #[test]
    fn details_of_repositories_pushed_to_are_kept_and_marked() {
        let old = data();
        let mut new = data();
        new.repos[1].pushed += 60;
        new.repos.remove(2);
        let (a, b, gone) = (old.repos[0].full_name.clone(), old.repos[1].full_name.clone(), old.repos[2].full_name.clone());
        let mut details = HashMap::new();
        for full in [&a, &b, &gone] {
            details.insert((full.clone(), "main".to_owned()), Load::Ready(Detail::default()));
        }
        details.insert(("x/failed".into(), "main".into()), Load::Failed("no".into()));
        let mut stale = HashSet::new();
        let kept = keep_details(&old, &new, details, &mut stale);
        assert!(kept.contains_key(&(a.clone(), "main".into())) && kept.contains_key(&(b.clone(), "main".into())));
        assert!(!kept.contains_key(&(gone, "main".into())), "a repository no longer listed");
        assert_eq!(stale, HashSet::from([(b, "main".to_owned())]), "only the one pushed to since is read again");
    }
}
