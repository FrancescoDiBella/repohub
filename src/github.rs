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

/// Extracts the next page path from a GitHub Link header, relative to api_base.
pub fn next_page_path(link: Option<&str>, api_base: &str) -> Option<String> {
    let link = link?;
    for part in link.split(',') {
        let mut sections = part.split(';');
        let url_part = sections.next()?.trim();
        let is_next = sections.any(|s| {
            let s = s.trim();
            s.eq_ignore_ascii_case("rel=\"next\"")
                || s.eq_ignore_ascii_case("rel='next'")
                || s.eq_ignore_ascii_case("rel=next")
        });
        if is_next && url_part.starts_with('<') && url_part.ends_with('>') {
            let full_url = &url_part[1..url_part.len() - 1];
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
        }
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

/// One of GitHub's notification threads; `None` for one with no id or repository.
pub fn parse_notification(v: &Value) -> Option<Notification> {
    let id = s(v, "id");
    let repo = v.get("repository").map(|r| s(r, "full_name")).unwrap_or_default();
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
    if !id_ok || repo.is_empty() || !b(v, "unread") {
        return None;
    }
    let subject = v.get("subject")?;
    let kind = match s(subject, "type").as_str() {
        "Issue" => Subject::Issue,
        "PullRequest" => Subject::Pull,
        _ => Subject::Other,
    };
    // The subject's API address ends with the number: …/repos/o/r/pulls/12.
    let number = s(subject, "url").rsplit('/').next().and_then(|n| n.parse::<u32>().ok()).filter(|_| kind != Subject::Other);
    let page = match (kind, number) {
        (Subject::Issue, Some(n)) => format!("https://github.com/{repo}/issues/{n}"),
        (Subject::Pull, Some(n)) => format!("https://github.com/{repo}/pull/{n}"),
        _ => format!("https://github.com/{repo}"),
    };
    Some(Notification { id, repo, title: s(subject, "title"), subject: kind, number, reason: s(v, "reason"), updated_at: parse_iso(&s(v, "updated_at")).unwrap_or(0), url: page })
}

/// One of GitHub's comments on a diff line; `None` for one that isn't on a line any more.
pub fn parse_line_comment(v: &Value) -> Option<LineComment> {
    let line = v.get("line").and_then(Value::as_u64)? as u32;
    let side = if s(v, "side") == "LEFT" { rune_kit::diff::Side::Old } else { rune_kit::diff::Side::New };
    Some(LineComment {
        path: s(v, "path"),
        line,
        side,
        author: v.get("user").map(|u| s(u, "login")).filter(|l| !l.is_empty()).unwrap_or_else(|| "ghost".into()),
        body: s(v, "body"),
        at: parse_iso(&s(v, "created_at")).unwrap_or(0),
    })
}

fn labels_of(v: &Value) -> Vec<(String, f32)> {
    v.get("labels").and_then(Value::as_array).into_iter().flatten().map(|l| (s(l, "name"), hex_hue(&s(l, "color")))).filter(|(n, _)| !n.is_empty()).collect()
}

fn logins_of(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().map(|u| s(u, "login")).filter(|l| !l.is_empty()).collect()
}

pub fn parse_pull(v: &Value) -> Pull {
    let merged = v.get("pull_request").and_then(|p| p.get("merged_at")).is_some_and(|m| !m.is_null());
    let state = if merged { PullState::Merged } else if s(v, "state") == "closed" { PullState::Closed } else if b(v, "draft") { PullState::Draft } else { PullState::Open };
    let updated_at = s(v, "updated_at");
    Pull { labels: labels_of(v), assignees: logins_of(v, "assignees"), review_requested: false,
        number: n(v, "number"),
        title: s(v, "title"),
        repo: repo_from_url(&s(v, "repository_url")),
        author: v.get("user").map(|u| s(u, "login")).unwrap_or_default(),
        state,
        checks: None,
        updated_at: parse_iso(&updated_at).unwrap_or(0),
        comments: n(v, "comments"),
        url: s(v, "html_url"),
    }
}

/// Builds a conversation from GitHub's answers: the issue, its comments and, for a pull
/// request, its detail and reviews.
pub fn parse_thread(issue: &Value, comments: &Value, pull: Option<(&Value, &Value)>) -> Thread {
    let who = |v: &Value| v.get("user").map(|u| s(u, "login")).filter(|l| !l.is_empty()).unwrap_or_else(|| "ghost".into());
    let when = |v: &Value, k: &str| parse_iso(&s(v, k)).unwrap_or(0);
    let mut entries: Vec<Entry> = comments.as_array().into_iter().flatten()
        .map(|c| Entry { author: who(c), at: when(c, "created_at"), body: s(c, "body"), verdict: None })
        .collect();
    let mut more = n(issue, "comments") as usize > entries.len();
    let mut standing = if s(issue, "state") == "closed" { Standing::Closed } else { Standing::Open };
    let mut branches = None;
    let (mut requested, mut reviewed) = (Vec::new(), Vec::new());
    if let Some((detail, reviews)) = pull {
        requested = crate::triage::logins(detail, "requested_reviewers");
        reviewed = crate::triage::reviewed_of(reviews);
        if b(detail, "merged") {
            standing = Standing::Merged;
        } else if standing == Standing::Open && b(detail, "draft") {
            standing = Standing::Draft;
        }
        let branch = |k: &str| detail.get(k).map(|x| s(x, "ref")).unwrap_or_default();
        branches = Some((branch("head"), branch("base"))).filter(|(h, b)| !h.is_empty() && !b.is_empty());
        let list: Vec<&Value> = reviews.as_array().into_iter().flatten().collect();
        more |= list.len() >= 100;
        for r in list {
            let verdict = match s(r, "state").as_str() {
                "APPROVED" => Verdict::Approved,
                "CHANGES_REQUESTED" => Verdict::ChangesRequested,
                // A review not sent yet is only its author's; a dismissed one no longer counts.
                "PENDING" | "DISMISSED" => continue,
                _ => Verdict::Commented,
            };
            let body = s(r, "body");
            // A "commented" review with no text only carries comments on lines, shown on GitHub.
            if verdict == Verdict::Commented && body.trim().is_empty() {
                continue;
            }
            entries.push(Entry { author: who(r), at: when(r, "submitted_at"), body, verdict: Some(verdict) });
        }
    }
    entries.sort_by_key(|e| e.at);
    entries.insert(0, Entry { author: who(issue), at: when(issue, "created_at"), body: s(issue, "body"), verdict: None });
    Thread {
        title: s(issue, "title"),
        author: who(issue),
        standing,
        url: s(issue, "html_url"),
        labels: labels_of(issue),
        assignees: logins_of(issue, "assignees"),
        milestone: crate::triage::milestone_of(issue),
        requested,
        reviewed,
        branches,
        entries,
        more,
        checks: None,
    }
}

/// The check runs in GitHub's answer for a commit's `check-runs`.
pub fn parse_check_runs(v: &Value) -> Vec<crate::thread::Check> {
    use crate::thread::{Check, CheckState};
    v.get("check_runs").and_then(Value::as_array).into_iter().flatten().map(|r| {
        let state = if s(r, "status") != "completed" {
            CheckState::Running
        } else {
            match s(r, "conclusion").as_str() {
                "success" => CheckState::Passed,
                "failure" | "timed_out" | "action_required" | "startup_failure" => CheckState::Failed,
                _ => CheckState::Other,
            }
        };
        Check { name: s(r, "name"), state, url: s(r, "html_url") }
    }).collect()
}

pub fn parse_issue(v: &Value) -> Issue {
    let created_at = s(v, "created_at");
    Issue { labels: labels_of(v), assignees: logins_of(v, "assignees"),
        number: n(v, "number"),
        title: s(v, "title"),
        repo: repo_from_url(&s(v, "repository_url")),
        label: v.get("labels").and_then(Value::as_array).and_then(|l| l.first()).map(|l| (s(l, "name"), hex_hue(&s(l, "color")))),
        created_at: parse_iso(&created_at).unwrap_or(0),
        comments: n(v, "comments"),
        url: s(v, "html_url"),
    }
}

impl Gh {
    fn repo(&self, v: &Value, starred: &HashSet<String>) -> Repo {
        let full = s(v, "full_name");
        let language = Some(s(v, "language")).filter(|l| !l.is_empty()).unwrap_or_else(|| "—".into());
        let r = Repo {
            owner: v.get("owner").map(|o| s(o, "login")).unwrap_or_default(),
            name: s(v, "name"),
            description: s(v, "description"),
            hue: lang_hue(&language),
            language,
            stars: n(v, "stargazers_count"),
            forks: n(v, "forks_count"),
            issues: n(v, "open_issues_count"),
            pushed: parse_iso(&s(v, "pushed_at")).unwrap_or(0),
            private: b(v, "private"),
            fork: b(v, "fork"),
            archived: b(v, "archived"),
            branch: s(v, "default_branch"),
            topics: v.get("topics").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            url: s(v, "html_url"),
            starred: starred.contains(&full),
            full_name: full,
            keys: std::sync::OnceLock::new(),
        };
        r.precompute_keys();
        r
    }

    /// The account's repositories and those of its organizations (`cfg.orgs`), most recently
    /// pushed first. With the account's own token, `/user/repos` lists both, private ones
    /// included; otherwise the public repositories of the user and of its public organizations.
    pub async fn repos(&self, cfg: &Config, login: &str, own: bool, warnings: &mut Vec<String>) -> Result<(Vec<Repo>, bool), String> {
        use crate::state::Orgs;
        let starred: HashSet<String> = if self.t.has_token() {
            let mut set = HashSet::new();
            let mut next = Some("/user/starred?per_page=100".to_owned());
            let mut pages = 0;
            while let Some(path) = next {
                if pages >= 10 {
                    break;
                }
                pages += 1;
                match self.get_resp(&path).await {
                    Ok((v, resp)) => {
                        let items = v.as_array().cloned().unwrap_or_default();
                        let n = items.len();
                        set.extend(items.iter().map(|r| s(r, "full_name")));
                        if n < 100 {
                            break;
                        }
                        next = next_page_path(resp.link.as_deref(), &self.api_base);
                    }
                    Err(e) => {
                        warnings.push(format!("Starred repositories couldn't load: {e}"));
                        break;
                    }
                }
            }
            set
        } else {
            HashSet::new()
        };
        let wanted = |r: &Repo| (!r.fork || cfg.include_forks) && (!r.archived || cfg.include_archived)
            && (r.owner.eq_ignore_ascii_case(login) || cfg.orgs.includes(&r.owner));
        let mut out = Vec::new();
        let mut repos_capped = false;
        if own {
            // The token's owner also sees private repositories, and its organizations', here.
            let affiliation = if cfg.orgs == Orgs::Off { "owner" } else { "owner,organization_member" };
            let mut next = Some(format!("/user/repos?affiliation={affiliation}&sort=pushed&per_page=100"));
            let mut page = 1;
            while let Some(path) = next {
                if out.len() >= cfg.max_repos || page > 10 {
                    repos_capped = true;
                    break;
                }
                page += 1;
                let (v, resp) = self.get_resp(&path).await?;
                let items = v.as_array().cloned().unwrap_or_default();
                out.extend(items.iter().map(|r| self.repo(r, &starred)).filter(wanted));
                if items.len() < 100 {
                    break;
                }
                next = next_page_path(resp.link.as_deref(), &self.api_base);
            }
        } else {
            let mut next = Some(format!("/users/{login}/repos?type=owner&sort=pushed&per_page=100"));
            let mut page = 1;
            while let Some(path) = next {
                if out.len() >= cfg.max_repos || page > 10 {
                    repos_capped = true;
                    break;
                }
                page += 1;
                let (v, resp) = self.get_resp(&path).await?;
                let items = v.as_array().cloned().unwrap_or_default();
                out.extend(items.iter().map(|r| self.repo(r, &starred)).filter(wanted));
                if items.len() < 100 {
                    break;
                }
                next = next_page_path(resp.link.as_deref(), &self.api_base);
            }
            // Public organizations, then their public repositories.
            let orgs: Vec<String> = match &cfg.orgs {
                Orgs::Off => Vec::new(),
                Orgs::Only(list) => list.clone(),
                Orgs::All => {
                    let mut list = Vec::new();
                    let mut next = Some(format!("/users/{login}/orgs?per_page=100"));
                    let mut pages = 0;
                    while let Some(path) = next {
                        if pages >= 5 {
                            break;
                        }
                        pages += 1;
                        match self.get_resp(&path).await {
                            Ok((v, resp)) => {
                                let items = v.as_array().cloned().unwrap_or_default();
                                let n = items.len();
                                list.extend(items.iter().map(|o| s(o, "login")).filter(|o| !o.is_empty()));
                                if n < 100 {
                                    break;
                                }
                                next = next_page_path(resp.link.as_deref(), &self.api_base);
                            }
                            Err(e) => {
                                warnings.push(format!("Organizations couldn't load: {e}"));
                                break;
                            }
                        }
                    }
                    list
                }
            };
            if orgs.len() > 50 {
                repos_capped = true;
            }
            let mut org_errs = Vec::new();
            for org in orgs.iter().take(50) {
                let mut next = Some(format!("/orgs/{org}/repos?type=public&sort=pushed&per_page=100"));
                let mut pages = 0;
                while let Some(path) = next {
                    if pages >= 5 {
                        repos_capped = true;
                        break;
                    }
                    pages += 1;
                    match self.get_resp(&path).await {
                        Ok((v, resp)) => {
                            let items = v.as_array().cloned().unwrap_or_default();
                            let n = items.len();
                            out.extend(items.iter().map(|r| self.repo(r, &starred)).filter(wanted));
                            if n < 100 {
                                break;
                            }
                            next = next_page_path(resp.link.as_deref(), &self.api_base);
                            if next.is_none() {
                                break;
                            }
                            if pages == 5 && n == 100 {
                                repos_capped = true;
                            }
                        }
                        Err(e) => {
                            org_errs.push((org.clone(), e));
                            break;
                        }
                    }
                }
            }
            if !org_errs.is_empty() {
                if org_errs.len() == 1 {
                    warnings.push(format!("Repositories for {} couldn't load: {}", org_errs[0].0, org_errs[0].1));
                } else {
                    warnings.push(format!("Repositories for {} organizations couldn't load: {}", org_errs.len(), org_errs[0].1));
                }
            }
            out.sort_by_key(|r| std::cmp::Reverse(r.pushed));
        }
        if out.len() > cfg.max_repos {
            repos_capped = true;
            out.truncate(cfg.max_repos);
        }
        Ok((out, repos_capped))
    }

    pub async fn search_page(&self, q: &str, page: usize) -> Result<(Vec<Value>, usize), String> {
        let q = q.replace(' ', "+").replace(':', "%3A");
        let v = self.get(&format!("/search/issues?q={q}&sort=updated&order=desc&per_page={}&page={page}", crate::model::SEARCH_CAP)).await?;
        let total = v.get("total_count").and_then(Value::as_u64).unwrap_or(0) as usize;
        let items = v.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok((items, total))
    }

    pub async fn pulls_page(&self, login: &str, open: bool, page: usize) -> Result<(Vec<Pull>, usize), String> {
        let q = if open {
            format!("is:pr is:open involves:{login}")
        } else {
            format!("is:pr is:closed involves:{login}")
        };
        let (items, total) = self.search_page(&q, page).await?;
        let pulls = items.iter().map(parse_pull).collect();
        Ok((pulls, total))
    }

    pub async fn pulls(&self, login: &str) -> Result<(Vec<Pull>, usize, usize), String> {
        let (open_items, open_total) = self.pulls_page(login, true, 1).await?;
        let (closed_items, closed_total) = self.pulls_page(login, false, 1).await?;
        let mut pulls = open_items;
        for p in closed_items {
            if !pulls.iter().any(|existing| existing.number == p.number && existing.repo.eq_ignore_ascii_case(&p.repo)) {
                pulls.push(p);
            }
        }
        Ok((pulls, open_total, closed_total))
    }

    /// Open issues in the account's repositories and in those of `orgs` (search ORs the owners).
    pub async fn issues_page(&self, login: &str, orgs: &[String], page: usize) -> Result<(Vec<Issue>, usize), String> {
        if orgs.is_empty() {
            let (items, total) = self.search_page(&format!("is:issue is:open archived:false user:{login}"), page).await?;
            let issues = items.iter().map(parse_issue).collect();
            return Ok((issues, total));
        }
        let mut all_issues: Vec<Issue> = Vec::new();
        let mut grand_total = 0;
        for (idx, chunk) in orgs.chunks(20).enumerate() {
            let owners: String = chunk.iter().map(|o| format!(" org:{o}")).collect();
            let q = if idx == 0 {
                format!("is:issue is:open archived:false user:{login}{owners}")
            } else {
                format!("is:issue is:open archived:false{owners}")
            };
            match self.search_page(&q, page).await {
                Ok((items, total)) => {
                    grand_total += total;
                    for v in &items {
                        let issue = parse_issue(v);
                        if !all_issues.iter().any(|existing| existing.number == issue.number && existing.repo.eq_ignore_ascii_case(&issue.repo)) {
                            all_issues.push(issue);
                        }
                    }
                }
                Err(e) => {
                    if idx == 0 {
                        return Err(e);
                    }
                }
            }
        }
        all_issues.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        Ok((all_issues, grand_total))
    }

    pub async fn issues(&self, login: &str, orgs: &[String]) -> Result<(Vec<Issue>, usize), String> {
        self.issues_page(login, orgs, 1).await
    }

    pub async fn runs(&self, full: &str) -> Result<Vec<Run>, String> {
        let v = self.get(&format!("/repos/{full}/actions/runs?per_page=4")).await?;
        Ok(parse_workflow_runs(&v, full, self.now))
    }

    pub async fn run_jobs(&self, full: &str, run_id: u64) -> Result<Vec<RunJob>, String> {
        let v = self.get(&format!("/repos/{full}/actions/runs/{run_id}/jobs?per_page=100")).await?;
        Ok(parse_run_jobs(&v, self.now))
    }

    pub async fn job_log(&self, full: &str, job_id: u64) -> Result<JobLog, String> {
        let r = self.t.send("GET", &format!("/repos/{full}/actions/jobs/{job_id}/logs"), JSON).await?;
        match r.status {
            200..=299 => Ok(tail_log(&r.body, LOG_TAIL_LINES)),
            404 | 410 => Err("Logs for this job are not available yet or have expired on GitHub.".into()),
            _ => Err(self.explain(&r)),
        }
    }

    pub async fn rerun_run(&self, full: &str, run_id: u64, failed_only: bool) -> Result<(), String> {
        let action = if failed_only { "rerun-failed-jobs" } else { "rerun" };
        let r = self.t.send_body("POST", &format!("/repos/{full}/actions/runs/{run_id}/{action}"), JSON, "{}").await?;
        if (200..300).contains(&r.status) {
            Ok(())
        } else {
            Err(self.explain_actions_write(&r))
        }
    }

    /// The owner's commits per week over the last 26 weeks (zeros while GitHub computes them).
    pub async fn participation(&self, full: &str) -> Result<Vec<u32>, String> {
        let v = self.get(&format!("/repos/{full}/stats/participation")).await?;
        if v.is_null() {
            return Ok(vec![0; 26]);
        }
        let weeks: Vec<u32> = v.get("owner").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u32).collect()).unwrap_or_default();
        Ok(if weeks.len() >= 26 { weeks[weeks.len() - 26..].to_vec() } else { vec![0; 26] })
    }

    /// Files, README and last commit of `full` at branch `at`; languages for the whole repository.
    pub async fn detail(&self, full: &str, at: &str) -> Detail {
        let mut d = Detail::default();
        let at = enc(at);
        match self.get(&format!("/repos/{full}/contents?ref={at}")).await {
            Ok(v) => {
                let mut files: Vec<(String, bool)> = v.as_array().cloned().unwrap_or_default().iter().map(|f| (s(f, "name"), s(f, "type") == "dir")).collect();
                files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                d.files = files;
            }
            Err(e) => d.error = Some(e),
        }
        // The whole README (up to 200 KB), so Copy copies all of it.
        match self.get_raw(&format!("/repos/{full}/readme?ref={at}")).await {
            Ok(Some(t)) => d.readme = Some(t.chars().take(200_000).collect()),
            Ok(None) => {}
            Err(e) => d.readme_error = Some(e),
        }
        match self.get(&format!("/repos/{full}/languages")).await {
            Ok(v) => {
                let mut langs: Vec<(String, u64)> = v.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0))).collect()).unwrap_or_default();
                langs.sort_by_key(|l| std::cmp::Reverse(l.1));
                d.languages = langs;
            }
            Err(e) => {
                if d.error.is_none() {
                    d.error = Some(e);
                }
            }
        }
        match self.get(&format!("/repos/{full}/commits?sha={at}&per_page=1")).await {
            Ok(v) => {
                if let Some(c) = v.as_array().and_then(|a| a.first()) {
                    let commit = c.get("commit").cloned().unwrap_or(Value::Null);
                    let author = commit.get("author").cloned().unwrap_or(Value::Null);
                    let date = s(&author, "date");
                    d.commit = Some(Commit {
                        author: c.get("author").filter(|a| !a.is_null()).map(|a| s(a, "login")).unwrap_or_else(|| s(&author, "name")),
                        message: s(&commit, "message").lines().next().unwrap_or("").to_owned(),
                        sha: s(c, "sha").chars().take(7).collect(),
                        committed_at: parse_iso(&date).unwrap_or(0),
                    });
                }
            }
            Err(e) => {
                if d.error.is_none() {
                    d.error = Some(e);
                }
            }
        }
        d
    }

    /// Branch names, the default branch first.
    pub async fn branches(&self, full: &str, default: &str) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = Vec::new();
        let mut next = Some(format!("/repos/{full}/branches?per_page=100"));
        let mut pages = 0;
        while let Some(path) = next {
            if pages >= 10 {
                break;
            }
            pages += 1;
            let (v, resp) = self.get_resp(&path).await?;
            let items = v.as_array().cloned().unwrap_or_default();
            let n = items.len();
            names.extend(items.iter().map(|b| s(b, "name")).filter(|n| !n.is_empty()));
            if n < 100 {
                break;
            }
            next = next_page_path(resp.link.as_deref(), &self.api_base);
        }
        if !default.is_empty() && !names.iter().any(|n| n == default) {
            names.push(default.to_owned());
        }
        names.sort_by_key(|n| (n != default, n.to_lowercase()));
        names.dedup();
        Ok(names)
    }

    pub async fn create_issue(&self, full: &str, title: &str, body: &str, labels: &[String], assignees: &[String]) -> Result<Created, String> {
        let v = self.write("POST", &format!("/repos/{full}/issues"), issue_body(title, body, labels, assignees)).await?;
        Ok(Created { number: n(&v, "number"), url: s(&v, "html_url") })
    }

    /// Creates branch `name` at the tip of branch `from`.
    pub async fn create_branch(&self, full: &str, name: &str, from: &str) -> Result<(), String> {
        let tip = self.get(&format!("/repos/{full}/git/ref/heads/{}", enc_path(from))).await?;
        let sha = tip.get("object").map(|o| s(o, "sha")).unwrap_or_default();
        if sha.is_empty() {
            return Err(format!("Couldn't find the last commit of {from}."));
        }
        self.write("POST", &format!("/repos/{full}/git/refs"), json!({ "ref": format!("refs/heads/{name}"), "sha": sha })).await.map(|_| ())
    }

    pub async fn create_pull(&self, full: &str, p: &NewPull) -> Result<Created, String> {
        let v = self.write("POST", &format!("/repos/{full}/pulls"), json!({
            "title": p.title, "body": p.body, "head": p.head, "base": p.base, "draft": p.draft,
        })).await?;
        Ok(Created { number: n(&v, "number"), url: s(&v, "html_url") })
    }

    /// The conversation of issue or pull request `number`: its description, comments and, for a
    /// pull request, reviews (the first 100 of each).
    pub async fn thread(&self, full: &str, number: u32) -> Result<Thread, String> {
        let issue = self.get(&format!("/repos/{full}/issues/{number}")).await?;
        let comments = self.get(&format!("/repos/{full}/issues/{number}/comments?per_page=100")).await?;
        let pull = if issue.get("pull_request").is_some_and(|p| !p.is_null()) {
            let detail = self.get(&format!("/repos/{full}/pulls/{number}")).await?;
            let reviews = self.get(&format!("/repos/{full}/pulls/{number}/reviews?per_page=100")).await?;
            Some((detail, reviews))
        } else {
            None
        };
        let mut t = parse_thread(&issue, &comments, pull.as_ref().map(|(d, r)| (d, r)));
        // The checks are extra: a token that can't read them still shows the conversation.
        if let Some(sha) = pull.as_ref().and_then(|(d, _)| d.get("head")).map(|h| s(h, "sha")).filter(|x| !x.is_empty()) {
            t.checks = self.get(&format!("/repos/{full}/commits/{sha}/check-runs?per_page=100")).await.ok().map(|v| parse_check_runs(&v));
        }
        Ok(t)
    }

    /// Your unread notifications, newest first (the first 50).
    pub async fn notifications(&self) -> Result<Vec<Notification>, String> {
        let v = self.get("/notifications?per_page=50").await?;
        Ok(v.as_array().into_iter().flatten().filter_map(parse_notification).collect())
    }

    pub async fn mark_read(&self, id: &str) -> Result<(), String> {
        let r = self.t.send_body("PATCH", &format!("/notifications/threads/{id}"), JSON, "").await?;
        match r.status {
            200..=299 | 304 => Ok(()),
            _ => Err(self.explain(&r)),
        }
    }

    /// The commit pull request `number` shows, and the comments on lines of its diff (the
    /// first 100). A comment on a line that changed since (`line` is null) is left out.
    pub async fn line_comments(&self, full: &str, number: u32) -> Result<(String, Vec<LineComment>), String> {
        let detail = self.get(&format!("/repos/{full}/pulls/{number}")).await?;
        let head = detail.get("head").map(|h| s(h, "sha")).unwrap_or_default();
        let list = self.get(&format!("/repos/{full}/pulls/{number}/comments?per_page=100")).await?;
        Ok((head, list.as_array().into_iter().flatten().filter_map(parse_line_comment).collect()))
    }

    /// Writes a comment on lines of a pull request's diff.
    pub async fn line_comment(&self, full: &str, number: u32, c: &NewLineComment) -> Result<(), String> {
        let mut v = json!({ "body": c.body, "commit_id": c.commit, "path": c.path, "line": c.line, "side": c.side });
        if let Some((line, side)) = &c.start {
            v["start_line"] = json!(line);
            v["start_side"] = json!(side);
        }
        self.write("POST", &format!("/repos/{full}/pulls/{number}/comments"), v).await.map(|_| ())
    }

    pub async fn comment(&self, full: &str, number: u32, body: &str) -> Result<(), String> {
        self.write("POST", &format!("/repos/{full}/issues/{number}/comments"), json!({ "body": body })).await.map(|_| ())
    }

    /// Sends a review of pull request `number`: `event` is APPROVE, REQUEST_CHANGES or COMMENT.
    pub async fn review(&self, full: &str, number: u32, event: &str, body: &str) -> Result<(), String> {
        let mut v = json!({ "event": event });
        if !body.is_empty() {
            v["body"] = json!(body);
        }
        self.write("POST", &format!("/repos/{full}/pulls/{number}/reviews"), v).await.map(|_| ())
    }

    pub async fn reopen_issue(&self, full: &str, number: u32) -> Result<(), String> {
        self.write("PATCH", &format!("/repos/{full}/issues/{number}"), json!({ "state": "open" })).await.map(|_| ())
    }

    /// Closes (`open` false) or reopens a pull request, through the issues API like an issue.
    pub async fn set_pull_open(&self, full: &str, number: u32, open: bool) -> Result<(), String> {
        self.write("PATCH", &format!("/repos/{full}/issues/{number}"), json!({ "state": if open { "open" } else { "closed" } })).await.map(|_| ())
    }

    pub async fn close_issue(&self, full: &str, number: u32) -> Result<(), String> {
        self.write("PATCH", &format!("/repos/{full}/issues/{number}"), json!({ "state": "closed", "state_reason": "completed" })).await.map(|_| ())
    }

    /// A repository's labels, assignable people or open milestones (the first 100).
    pub async fn choices(&self, full: &str, source: crate::triage::Source) -> Result<crate::triage::Choices, String> {
        let (v, resp) = self.get_resp(&source.path(full)).await?;
        let items = source.read(&v);
        let more = next_page_path(resp.link.as_deref(), &self.api_base).is_some() || v.as_array().is_some_and(|a| a.len() >= 100);
        Ok(crate::triage::Choices { items, more })
    }

    /// Sends a triage change and reads what the issue or pull request is now from GitHub's
    /// answer to the last request (both `PATCH` on the issue and the reviewer requests answer
    /// with the whole item).
    pub async fn triage(&self, full: &str, number: u32, change: &crate::triage::Change) -> Result<crate::triage::Applied, String> {
        let mut last = Value::Null;
        for (method, path, body) in change.requests(full, number) {
            last = self.write(method, &path, body).await?;
        }
        Ok(crate::triage::Applied::from_answer(change, &last))
    }

    /// Open pull requests that wait for `login`'s review (the first page), and how many GitHub
    /// counts.
    pub async fn review_requested(&self, login: &str) -> Result<(Vec<Pull>, usize), String> {
        let (items, total) = self.search_page(&format!("is:pr is:open review-requested:{login}"), 1).await?;
        Ok((items.iter().map(parse_pull).map(|p| Pull { review_requested: true, ..p }).collect(), total))
    }

    /// The files a pull request changes, with each one's patch (GitHub leaves out the patch of
    /// very large or binary files). At most 300 files.
    pub async fn pull_files(&self, full: &str, number: u32) -> Result<Vec<PullFile>, String> {
        let mut out = Vec::new();
        for page in 1..=3 {
            let v = self.get(&format!("/repos/{full}/pulls/{number}/files?per_page=100&page={page}")).await?;
            let list = v.as_array().cloned().unwrap_or_default();
            let n = list.len();
            out.extend(list.iter().map(|f| {
                let s = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_owned);
                PullFile { path: s("filename").unwrap_or_default(), previous: s("previous_filename"), status: s("status").unwrap_or_default(), patch: s("patch") }
            }));
            if n < 100 {
                break;
            }
        }
        Ok(out)
    }

    /// Page `page` of branch `branch`'s commits ([`crate::commits::PAGE`] a page), newest first,
    /// and whether GitHub has more.
    pub async fn commits(&self, full: &str, branch: &str, page: usize) -> Result<(Vec<crate::commits::Row>, bool), String> {
        let per = crate::commits::PAGE;
        let (v, resp) = self.get_resp(&format!("/repos/{full}/commits?sha={}&per_page={per}&page={page}", enc(branch))).await?;
        let rows = crate::commits::parse_rows(&v);
        // GitHub links the next page when there is one; without a Link header, a full page may have more.
        let more = match resp.link.as_deref() {
            Some(link) => next_page_path(Some(link), &self.api_base).is_some(),
            None => v.as_array().is_some_and(|a| a.len() >= per),
        };
        Ok((rows, more))
    }

    /// Commit `sha` with its files (GitHub lists the first 300).
    pub async fn commit(&self, full: &str, sha: &str) -> Result<crate::commits::Detail, String> {
        if !crate::commits::valid_sha(sha) {
            return Err(format!("“{sha}” isn't a commit id."));
        }
        let (v, resp) = self.get_resp(&format!("/repos/{full}/commits/{sha}")).await?;
        Ok(crate::commits::parse_detail(&v, next_page_path(resp.link.as_deref(), &self.api_base).is_some()))
    }

    /// Branch `head` compared with branch `base`, and the open pull request from one into the
    /// other, if there is one.
    pub async fn compare(&self, full: &str, base: &str, head: &str) -> Result<crate::commits::Comparison, String> {
        if !valid_branch(base) || !valid_branch(head) {
            return Err("Those aren't branch names GitHub can compare.".into());
        }
        let v = match self.get(&format!("/repos/{full}/compare/{}...{}", enc_path(base), enc_path(head))).await {
            Ok(v) => v,
            // Two histories with nothing in common: nothing to compare, not an error.
            Err(e) if e.contains("No common ancestor") => return Ok(crate::commits::Comparison::unrelated(base, head)),
            Err(e) => return Err(e),
        };
        let mut c = crate::commits::parse_comparison(&v, base, head);
        if c.ahead > 0 {
            // Whether a pull request is already open for it decides if New pull request shows; if
            // the search fails, the button shows and GitHub's answer to it says so.
            let q = format!("is:pr is:open repo:{full} head:{head} base:{base}");
            if let Ok(found) = self.get(&format!("/search/issues?q={}&per_page=1", enc(&q))).await {
                c.open_pull = found.get("items").and_then(Value::as_array).and_then(|a| a.first()).map(|p| (n(p, "number"), s(p, "html_url")));
            }
        }
        Ok(c)
    }

    pub async fn merge_pull(&self, full: &str, number: u32, method: &str) -> Result<(), String> {
        self.write("PUT", &format!("/repos/{full}/pulls/{number}/merge"), json!({ "merge_method": method })).await.map(|_| ())
    }

    pub async fn set_star(&self, full: &str, on: bool) -> Result<(), String> {
        let r = self.t.send(if on { "PUT" } else { "DELETE" }, &format!("/user/starred/{full}"), JSON).await?;
        if (200..300).contains(&r.status) { Ok(()) } else { Err(self.explain(&r)) }
    }
}

/// Everything the overview and lists need, in one go (about 4 + 2 × `activity_repos` requests).
pub async fn load_all(cfg: &Config) -> Result<Data, LoadError> {
    load_all_with(cfg, |_: Data| {}).await
}

/// How many requests a load sends at once.
pub const AT_ONCE: usize = 4;

/// [`load_all`], with the parts that don't need each other side by side (at most [`AT_ONCE`]
/// requests at a time), and `early` called with the profile and the repositories as soon as
/// they're in, while pull requests, issues, runs and activity still load.
///
/// The answer is the same as reading everything one after the other: the same lists, the same
/// warnings in the same order (`load_tests` checks it against the sequential order).
///
/// ```text
/// profile ─┬─ repositories ─┬─ issues (needs the organizations)
/// (viewer) │                ├─ runs + activity of each active repository
///          │                └─ (early: profile and repositories)
///          └─ pull requests ── waiting for your review (needs your account)
/// ```
pub async fn load_all_with(cfg: &Config, early: impl Fn(Data) + net::MaybeSend + net::MaybeSync) -> Result<Data, LoadError> {
    let gh = Gh::new(cfg).limited(AT_ONCE);
    let gh = &gh;
    // With a user name and a token: whether the token is that user's, next to the profile.
    let ask_viewer = gh.t.has_token() && cfg.user.is_some();
    let (profile, viewer) = net::join(gh.profile(cfg), async move { if ask_viewer { gh.viewer_login().await } else { None } }).await;
    let profile = match profile {
        Ok(p) => p,
        Err(e) => return Err(LoadError::new(e, gh.retry_after())),
    };
    let own = gh.t.has_token() && match &cfg.user {
        None => true,
        Some(u) => viewer.is_some_and(|v| v.eq_ignore_ascii_case(u)),
    };
    let login = profile.login.as_str();
    let early = &early;
    let profile_ref = &profile;
    let (head, tail) = net::join(
        // The repositories, then what needs them.
        async move {
            let mut warnings = Vec::new();
            let (repos, repos_capped) = gh.repos(cfg, login, own, &mut warnings).await?;
            early(Data { profile: profile_ref.clone(), repos: repos.clone(), repos_capped, ..Data::default() });
            // The organizations whose repositories were loaded: their issues too.
            let mut orgs: Vec<String> = repos.iter().map(|r| r.owner.clone()).filter(|o| !o.eq_ignore_ascii_case(login)).collect();
            orgs.sort_by_key(|o| o.to_lowercase());
            orgs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            let active: Vec<String> = repos.iter().filter(|r| !r.archived).take(cfg.activity_repos).map(|r| r.full_name.clone()).collect();
            let (issues, activity) = net::join(
                async { gh.issues(login, &orgs).await },
                net::join_all(active.iter().map(|full| net::join(gh.runs(full), gh.participation(full))).collect()),
            ).await;
            Ok::<_, String>((repos, repos_capped, warnings, issues, activity))
        },
        // Pull requests, then those waiting for your review (with a token: GitHub only knows
        // whose review is asked for an account).
        async move {
            let pulls = gh.pulls(login).await;
            let waiting = if own && pulls.is_ok() { Some(gh.review_requested(login).await) } else { None };
            (pulls, waiting)
        },
    ).await;
    let (repos, repos_capped, mut warnings, issues, activity_parts) = match head {
        Ok(h) => h,
        Err(e) => return Err(LoadError::new(e, gh.retry_after())),
    };
    let (pulls, waiting) = tail;
    let mut pulls_failed = false;
    let mut open_pulls_total = None;
    let mut closed_pulls_total = None;
    let mut pulls = match pulls {
        Ok((p, open_tot, closed_tot)) => {
            open_pulls_total = Some(open_tot);
            closed_pulls_total = Some(closed_tot);
            p
        }
        Err(e) => {
            pulls_failed = true;
            warnings.push(format!("Pull requests couldn't load: {e}"));
            Vec::new()
        }
    };
    // Each pull request waiting for your review shows once; one not in the list yet joins it.
    let mut review_total = None;
    match waiting {
        Some(Ok((waiting, total))) => {
            review_total = Some(total);
            merge_review_requested(&mut pulls, waiting, &mut open_pulls_total);
        }
        Some(Err(e)) => warnings.push(format!("Pull requests waiting for your review couldn't load: {e}")),
        None => {}
    }
    let mut issues_failed = false;
    let mut issues_total = None;
    let issues = match issues {
        Ok((i, total)) => {
            issues_total = Some(total);
            i
        }
        Err(e) => {
            issues_failed = true;
            warnings.push(format!("Issues couldn't load: {e}"));
            Vec::new()
        }
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut runs_failed = false;
    let mut activity = vec![0u32; 26];
    let mut run_error: Option<String> = None;
    let mut participation_error: Option<String> = None;
    // In the repositories' order, as when they were read one after the other.
    for (r, part) in activity_parts {
        match r {
            Ok(r) => runs.extend(r),
            Err(e) => {
                runs_failed = true;
                if run_error.is_none() {
                    run_error = Some(e);
                }
            }
        }
        match part {
            Ok(part) => {
                for (i, w) in part.iter().enumerate() {
                    activity[i] += w;
                }
            }
            Err(e) => {
                if participation_error.is_none() {
                    participation_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = run_error {
        warnings.push(format!("Workflow runs couldn't load: {e}"));
    }
    if let Some(e) = participation_error {
        warnings.push(format!("Commit activity couldn't load: {e}"));
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
    for p in &mut pulls {
        if p.checks.is_none()
            && let Some(r) = runs.iter().find(|r| same_repo(&r.repo, &p.repo) && r.pull_numbers.contains(&p.number))
        {
            p.checks = Some(r.checks);
        }
    }
    let runs_total = runs.len();
    runs.truncate(20);
    Ok(Data {
        profile,
        repos,
        pulls,
        issues,
        runs,
        activity,
        warnings,
        pulls_failed,
        issues_failed,
        runs_failed,
        retry_after: gh.retry_after(),
        sample: false,
        open_pulls_total,
        closed_pulls_total,
        issues_total,
        runs_total,
        repos_capped,
        review_total,
    })
}

pub const LOG_TAIL_LINES: usize = 200;

fn checks_and_label(status: &str, conclusion: &str) -> (Checks, String) {
    if status != "completed" {
        let label = match status {
            "queued" => "queued",
            "waiting" => "waiting",
            "pending" => "pending",
            _ => "in progress",
        };
        (Checks::Running, label.into())
    } else {
        match conclusion {
            "success" => (Checks::Passing, "success".into()),
            "skipped" => (Checks::Passing, "skipped".into()),
            "neutral" => (Checks::Passing, "neutral".into()),
            "cancelled" => (Checks::Failing, "cancelled".into()),
            "timed_out" => (Checks::Failing, "timed out".into()),
            "action_required" => (Checks::Failing, "action required".into()),
            "startup_failure" => (Checks::Failing, "startup failure".into()),
            "" => (Checks::Failing, "failed".into()),
            other => (Checks::Failing, other.replace('_', " ")),
        }
    }
}

pub fn parse_workflow_runs(v: &Value, full: &str, now: i64) -> Vec<Run> {
    v.get("workflow_runs").and_then(Value::as_array).into_iter().flatten().map(|r| {
        let url = s(r, "html_url");
        let id = r.get("id").and_then(Value::as_u64).or_else(|| {
            url.rsplit('/').next().and_then(|x| x.parse::<u64>().ok())
        }).unwrap_or(0);
        let (checks, _) = checks_and_label(&s(r, "status"), &s(r, "conclusion"));
        let start = parse_iso(&s(r, "run_started_at"));
        let end = parse_iso(&s(r, "updated_at"));
        let pull_numbers = r.get("pull_requests").and_then(Value::as_array).into_iter().flatten()
            .map(|p| n(p, "number"))
            .filter(|&num| num > 0)
            .collect();
        Run {
            id,
            workflow: s(r, "name"),
            repo: full.to_owned(),
            branch: s(r, "head_branch"),
            duration: match (start, end) {
                (Some(a), Some(b)) if checks != Checks::Running => duration(b - a),
                (Some(a), _) => duration(now - a),
                _ => String::new(),
            },
            updated_at: end.unwrap_or(0),
            checks,
            url,
            pull_numbers,
        }
    }).collect()
}

pub fn parse_run_jobs(v: &Value, now: i64) -> Vec<RunJob> {
    v.get("jobs").and_then(Value::as_array).into_iter().flatten().map(|j| {
        let status = s(j, "status");
        let (checks, status_label) = checks_and_label(&status, &s(j, "conclusion"));
        let start = parse_iso(&s(j, "started_at"));
        let end = parse_iso(&s(j, "completed_at"));
        let dur = match (start, end) {
            (Some(a), Some(b)) if status == "completed" => duration(b - a),
            (Some(a), _) if status == "in_progress" => duration(now - a),
            _ => String::new(),
        };
        let steps = j.get("steps").and_then(Value::as_array).into_iter().flatten().map(|st| {
            let st_status = s(st, "status");
            let (st_checks, st_label) = checks_and_label(&st_status, &s(st, "conclusion"));
            let st_start = parse_iso(&s(st, "started_at"));
            let st_end = parse_iso(&s(st, "completed_at"));
            let st_dur = match (st_start, st_end) {
                (Some(a), Some(b)) if st_status == "completed" => duration(b - a),
                (Some(a), _) if st_status == "in_progress" => duration(now - a),
                _ => String::new(),
            };
            RunStep {
                number: n(st, "number"),
                name: s(st, "name"),
                checks: st_checks,
                status_label: st_label,
                duration: st_dur,
            }
        }).collect();
        RunJob {
            id: j.get("id").and_then(Value::as_u64).unwrap_or(0),
            name: s(j, "name"),
            checks,
            status_label,
            duration: dur,
            url: s(j, "html_url"),
            steps,
        }
    }).collect()
}

/// Strips terminal ANSI escape sequences (CSI colours, OSC hyperlinks/titles) from raw job logs.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                continue;
            }
            out.push('\n');
            continue;
        }
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(c) if ('('..='/').contains(&c) => {
                chars.next();
                chars.next();
            }
            Some(c) if ('\x40'..='\x5f').contains(&c) => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Keeps the last `max_lines` lines of `raw` after stripping ANSI escape codes.
pub fn tail_log(raw: &str, max_lines: usize) -> JobLog {
    let cleaned = strip_ansi(raw);
    let lines: Vec<&str> = cleaned.lines().collect();
    let total_lines = lines.len();
    let truncated = total_lines > max_lines;
    let start = total_lines.saturating_sub(max_lines);
    JobLog {
        text: lines[start..].join("\n"),
        total_lines,
        truncated,
    }
}

pub async fn load_more_pulls(cfg: &Config, login: &str, open: bool, page: usize) -> Result<(Vec<Pull>, usize), String> {
    Gh::new(cfg).pulls_page(login, open, page).await
}

pub async fn load_more_issues(cfg: &Config, login: &str, orgs: &[String], page: usize) -> Result<(Vec<Issue>, usize), String> {
    Gh::new(cfg).issues_page(login, orgs, page).await
}

/// One file of a pull request, as GitHub lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullFile {
    pub path: String,
    /// Its old name, for a renamed file.
    pub previous: Option<String>,
    /// added, removed, modified, renamed, copied, changed, unchanged.
    pub status: String,
    /// The unified diff's hunks (no file headers); `None` for binary or very large files.
    pub patch: Option<String>,
}

pub async fn load_notifications(cfg: &Config) -> Result<Vec<Notification>, String> {
    Gh::new(cfg).notifications().await
}

pub async fn mark_read(cfg: &Config, id: &str) -> Result<(), String> {
    Gh::new(cfg).mark_read(id).await
}

pub async fn load_line_comments(cfg: &Config, full: &str, number: u32) -> Result<(String, Vec<LineComment>), String> {
    Gh::new(cfg).line_comments(full, number).await
}

/// Flags the pull requests in `waiting` as waiting for your review: those in `pulls` already are
/// marked, the others join the list (and the open total, which counted only the first search).
pub fn merge_review_requested(pulls: &mut Vec<Pull>, waiting: Vec<Pull>, open_total: &mut Option<usize>) {
    for w in waiting {
        match pulls.iter_mut().find(|p| p.number == w.number && p.repo.eq_ignore_ascii_case(&w.repo)) {
            Some(p) => p.review_requested = true,
            None => {
                pulls.push(w);
                if let Some(t) = open_total.as_mut() {
                    *t += 1;
                }
            }
        }
    }
}

/// The body of `POST /repos/{r}/issues`: labels and assignees only when there are some.
pub fn issue_body(title: &str, body: &str, labels: &[String], assignees: &[String]) -> Value {
    let mut v = json!({ "title": title, "body": body });
    if !labels.is_empty() {
        v["labels"] = json!(labels);
    }
    if !assignees.is_empty() {
        v["assignees"] = json!(assignees);
    }
    v
}

pub async fn load_choices(cfg: &Config, full: &str, source: crate::triage::Source) -> Result<crate::triage::Choices, String> {
    Gh::new(cfg).choices(full, source).await
}

pub async fn load_thread(cfg: &Config, full: &str, number: u32) -> Result<Thread, String> {
    Gh::new(cfg).thread(full, number).await
}

pub async fn load_pull_files(cfg: &Config, full: &str, number: u32) -> Result<Vec<PullFile>, String> {
    Gh::new(cfg).pull_files(full, number).await
}

pub async fn load_commits(cfg: &Config, full: &str, branch: &str, page: usize) -> Result<(Vec<crate::commits::Row>, bool), String> {
    Gh::new(cfg).commits(full, branch, page).await
}

pub async fn load_commit(cfg: &Config, full: &str, sha: &str) -> Result<crate::commits::Detail, String> {
    Gh::new(cfg).commit(full, sha).await
}

pub async fn load_compare(cfg: &Config, full: &str, base: &str, head: &str) -> Result<crate::commits::Comparison, String> {
    Gh::new(cfg).compare(full, base, head).await
}

pub async fn load_detail(cfg: &Config, full: &str, at: &str) -> Detail {
    Gh::new(cfg).detail(full, at).await
}

pub async fn load_run_jobs(cfg: &Config, full: &str, run_id: u64) -> Result<Vec<RunJob>, String> {
    Gh::new(cfg).run_jobs(full, run_id).await
}

pub async fn load_job_log(cfg: &Config, full: &str, job_id: u64) -> Result<JobLog, String> {
    Gh::new(cfg).job_log(full, job_id).await
}

/// What GitHub created: its number and page.
#[derive(Clone, Debug)]
pub struct Created {
    pub number: u32,
    pub url: String,
}

/// A pull request to open.
#[derive(Clone, Debug)]
pub struct NewPull {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
}

/// A comment to write on lines of a pull request's diff.
#[derive(Clone, Debug)]
pub struct NewLineComment {
    pub body: String,
    /// The commit the diff shows.
    pub commit: String,
    pub path: String,
    pub line: u32,
    /// `RIGHT` (the new side) or `LEFT`.
    pub side: &'static str,
    /// Where a comment on several lines starts.
    pub start: Option<(u32, &'static str)>,
}

/// One of the writes RepoHub makes on GitHub. Running it returns a message for the toast.
#[derive(Clone, Debug)]
pub enum Write {
    /// A new issue, with the labels and assignees picked in the form (may be empty).
    Issue { title: String, body: String, labels: Vec<String>, assignees: Vec<String> },
    Branch { name: String, from: String },
    Pull(NewPull),
    CloseIssue(u32),
    ReopenIssue(u32),
    /// Closing (`open` false) or reopening a pull request without merging it.
    SetPullOpen { number: u32, open: bool },
    Merge { number: u32, method: &'static str },
    /// A comment on an issue or a pull request.
    Comment { number: u32, body: String },
    /// A review of a pull request; `body` may be empty for an approval.
    Review { number: u32, verdict: Verdict, body: String },
    /// A comment on lines of a pull request's diff.
    LineComment { number: u32, comment: NewLineComment },
    /// Re-running all jobs (`failed_only: false`) or only failed jobs (`failed_only: true`) of a workflow run.
    RerunRun { run_id: u64, failed_only: bool },
    /// Labels, assignees, milestone, reviewers, title or description of an issue or pull
    /// request: one kind of change at a time, so the toast can say what changed.
    Triage { number: u32, change: crate::triage::Change },
}

impl Write {
    /// The issue or pull request the write is about, when there is one.
    pub fn number(&self) -> Option<u32> {
        match self {
            Write::CloseIssue(n) | Write::ReopenIssue(n) | Write::SetPullOpen { number: n, .. } | Write::Merge { number: n, .. } | Write::Comment { number: n, .. } | Write::Review { number: n, .. } | Write::LineComment { number: n, .. } | Write::Triage { number: n, .. } => Some(*n),
            Write::Issue { .. } | Write::Branch { .. } | Write::Pull(_) | Write::RerunRun { .. } => None,
        }
    }
}

/// What a finished write changed, for updating the screen without waiting for a reload.
#[derive(Clone, Debug)]
pub enum Done {
    Issue(Created),
    Branch(String),
    Pull(Created),
    Closed(u32),
    Reopened(u32),
    Merged(u32),
    /// A pull request closed (`false`) or reopened (`true`).
    PullOpen(u32, bool),
    Commented(u32),
    Reviewed(u32, Verdict),
    LineCommented(u32),
    Reran { run_id: u64, failed_only: bool },
    /// A triage change was made: what was asked, and what GitHub says it is now.
    Triaged { number: u32, change: crate::triage::Change, applied: crate::triage::Applied },
}

pub async fn run(cfg: &Config, full: &str, w: Write) -> Result<Done, String> {
    let gh = Gh::new(cfg);
    match w {
        Write::Issue { title, body, labels, assignees } => gh.create_issue(full, &title, &body, &labels, &assignees).await.map(Done::Issue),
        Write::Branch { name, from } => gh.create_branch(full, &name, &from).await.map(|_| Done::Branch(name)),
        Write::Pull(p) => gh.create_pull(full, &p).await.map(Done::Pull),
        Write::CloseIssue(n) => gh.close_issue(full, n).await.map(|_| Done::Closed(n)),
        Write::ReopenIssue(n) => gh.reopen_issue(full, n).await.map(|_| Done::Reopened(n)),
        Write::Comment { number, body } => gh.comment(full, number, &body).await.map(|_| Done::Commented(number)),
        Write::Review { number, verdict, body } => gh.review(full, number, verdict.event(), &body).await.map(|_| Done::Reviewed(number, verdict)),
        Write::LineComment { number, comment } => gh.line_comment(full, number, &comment).await.map(|_| Done::LineCommented(number)),
        Write::SetPullOpen { number, open } => gh.set_pull_open(full, number, open).await.map(|_| Done::PullOpen(number, open)),
        Write::Merge { number, method } => gh.merge_pull(full, number, method).await.map(|_| Done::Merged(number)),
        Write::RerunRun { run_id, failed_only } => gh.rerun_run(full, run_id, failed_only).await.map(|_| Done::Reran { run_id, failed_only }),
        Write::Triage { number, change } => gh.triage(full, number, &change).await.map(|applied| Done::Triaged { number, change, applied }),
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

pub async fn branches(cfg: &Config, full: &str, default: &str) -> Result<Vec<String>, String> {
    Gh::new(cfg).branches(full, default).await
}

pub async fn set_star(cfg: &Config, full: &str, on: bool) -> Result<(), String> {
    Gh::new(cfg).set_star(full, on).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_runs_read_as_passed_failed_running_or_skipped() {
        use crate::thread::CheckState::*;
        let v = serde_json::json!({"check_runs": [
            {"name": "test", "status": "completed", "conclusion": "success", "html_url": "u1"},
            {"name": "lint", "status": "completed", "conclusion": "failure"},
            {"name": "build", "status": "in_progress", "conclusion": null},
            {"name": "deploy", "status": "completed", "conclusion": "skipped"},
        ]});
        let list = super::parse_check_runs(&v);
        let states: Vec<_> = list.iter().map(|c| c.state).collect();
        assert_eq!(states, vec![Passed, Failed, Running, Other]);
        assert_eq!(list[0].url, "u1");
        assert_eq!(crate::thread::checks_summary(&list), "1 failed, 1 running, 1 passed, 1 skipped");
        assert_eq!(crate::thread::checks_summary(&[]), "No checks on the last commit.");
    }

    #[test]
    fn line_comments_read_their_side_and_skip_outdated_ones() {
        let v = serde_json::json!({"path": "src/a.rs", "line": 7, "side": "LEFT", "user": {"login": "bob"}, "body": "hm", "created_at": "2026-01-02T03:04:05Z"});
        let c = super::parse_line_comment(&v).expect("on a line");
        assert_eq!((c.path.as_str(), c.line, c.side, c.author.as_str()), ("src/a.rs", 7, rune_kit::diff::Side::Old, "bob"));
        let gone = serde_json::json!({"path": "src/a.rs", "line": null, "side": "RIGHT", "user": null, "body": ""});
        assert!(super::parse_line_comment(&gone).is_none(), "an outdated comment isn't on a line any more");
    }

    #[test]
    fn repo_name_from_url() {
        assert_eq!(super::repo_from_url("https://api.github.com/repos/octo/hello"), "octo/hello");
    }

    #[test]
    fn branch_names_and_encoding() {
        use super::{enc, enc_path, valid_branch};
        for ok in ["main", "feature/login", "fix-12", "v1.2"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in ["", "has space", "-x", "a..b", "a/", "x.lock", "a:b", "a~1", "/a", "a//b"] {
            assert!(!valid_branch(bad), "{bad}");
        }
        assert_eq!(enc("feature/x y"), "feature%2Fx%20y");
        assert_eq!(enc_path("feature/x"), "feature/x");
    }

    #[test]
    fn workflow_runs_and_jobs_parse_with_steps_and_pr_numbers() {
        use crate::model::Checks;
        let now = crate::model::parse_iso("2026-11-18T12:02:00Z").unwrap();
        let runs_json = serde_json::json!({"workflow_runs": [
            {
                "id": 7001,
                "name": "CI",
                "head_branch": "fix-bug",
                "status": "completed",
                "conclusion": "failure",
                "run_started_at": "2026-11-18T12:00:00Z",
                "updated_at": "2026-11-18T12:00:45Z",
                "html_url": "https://github.com/acme/app/actions/runs/7001",
                "pull_requests": [{"number": 19}]
            },
            {
                "name": "Fallback ID",
                "head_branch": "main",
                "status": "in_progress",
                "conclusion": null,
                "run_started_at": "2026-11-18T12:01:00Z",
                "updated_at": "2026-11-18T12:01:30Z",
                "html_url": "https://github.com/acme/app/actions/runs/7002"
            }
        ]});
        let runs = super::parse_workflow_runs(&runs_json, "acme/app", now);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].id, runs[0].checks, runs[0].duration.as_str()), (7001, Checks::Failing, "45s"));
        assert_eq!(runs[0].pull_numbers, vec![19]);
        assert_eq!((runs[1].id, runs[1].checks, runs[1].duration.as_str()), (7002, Checks::Running, "1m 00s"));

        let jobs_json = serde_json::json!({"jobs": [
            {
                "id": 910,
                "name": "test (ubuntu-latest)",
                "status": "completed",
                "conclusion": "failure",
                "started_at": "2026-11-18T12:00:02Z",
                "completed_at": "2026-11-18T12:00:44Z",
                "html_url": "https://github.com/acme/app/actions/runs/7001/job/910",
                "steps": [
                    {"number": 1, "name": "Checkout", "status": "completed", "conclusion": "success", "started_at": "2026-11-18T12:00:02Z", "completed_at": "2026-11-18T12:00:05Z"},
                    {"number": 2, "name": "cargo test", "status": "completed", "conclusion": "failure", "started_at": "2026-11-18T12:00:05Z", "completed_at": "2026-11-18T12:00:44Z"}
                ]
            }
        ]});
        let jobs = super::parse_run_jobs(&jobs_json, now);
        assert_eq!(jobs.len(), 1);
        assert_eq!((jobs[0].id, jobs[0].checks, jobs[0].status_label.as_str(), jobs[0].duration.as_str()), (910, Checks::Failing, "failure", "42s"));
        assert_eq!(jobs[0].steps.len(), 2);
        assert_eq!((jobs[0].steps[1].number, jobs[0].steps[1].checks, jobs[0].steps[1].duration.as_str()), (2, Checks::Failing, "39s"));
    }

    #[test]
    fn strip_ansi_and_tail_log_keep_last_200_lines() {
        let raw = "\x1b[32mPASS\x1b[0m step 1\r\n\x1b]8;;https://example.com\x07link\x1b]8;;\x07\n\x1b[1;31mFAIL\x1b[0m exit 1";
        assert_eq!(super::strip_ansi(raw), "PASS step 1\nlink\nFAIL exit 1");

        let many: String = (1..=250).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let tailed = super::tail_log(&many, super::LOG_TAIL_LINES);
        assert!(tailed.truncated);
        assert_eq!(tailed.total_lines, 250);
        assert_eq!(tailed.text.lines().count(), 200);
        assert!(tailed.text.starts_with("line 51\n"));
        assert!(tailed.text.ends_with("\nline 250"));
    }

    #[test]
    fn explain_actions_write_mentions_actions_permission_on_403() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let gh = super::Gh::new(&crate::config::Config::load());
        let resp = crate::net::Resp {
            status: 403,
            body: r#"{"message":"Resource not accessible by personal access token"}"#.into(),
            remaining: Some("4990".into()),
            reset: None,
            retry_after: None,
            link: None,
            etag: None,
        };
        let msg = gh.explain_actions_write(&resp);
        assert!(msg.contains("Actions write permission"), "{msg}");
        assert!(msg.contains("Actions: Read and write"), "{msg}");
    }
}
