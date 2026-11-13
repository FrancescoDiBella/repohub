//! GitHub REST API client, shared by the desktop and web builds. Requests go through
//! [`net::Transport`]; the functions here are async and run in the background (see `net::spawn`).
use std::collections::HashSet;

use serde_json::{Value, json};

use crate::config::Config;
use crate::net::{self, Resp, Transport};
use crate::model::{Checks, Commit, Data, Detail, Issue, Profile, Pull, PullState, Repo, Run, duration, hex_hue, lang_hue, parse_iso};

pub struct Gh {
    t: Transport,
    now: i64,
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

impl Gh {
    pub fn new(cfg: &Config) -> Gh {
        Gh { t: Transport::new(cfg), now: net::now() }
    }

    fn explain(&self, r: &Resp) -> String {
        let v = serde_json::from_str::<Value>(&r.body).ok();
        let mut msg = v.as_ref().map(|v| s(v, "message")).unwrap_or_default();
        // Validation errors (422) carry the reason in `errors`: "A pull request already exists…".
        if let Some(detail) = v.as_ref().and_then(|v| v.get("errors")).and_then(Value::as_array).and_then(|e| e.first())
            .map(|e| if e.is_string() { e.as_str().unwrap_or("").to_owned() } else { Some(s(e, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{} {}", s(e, "field"), s(e, "code"))) })
            .filter(|d| !d.trim().is_empty())
        {
            msg = format!("{msg}: {}", detail.trim());
        }
        match r.status {
            401 => "GitHub rejected the token (401): check the token in Settings, or GITHUB_TOKEN in your .env file.".into(),
            403 | 429 if r.remaining.as_deref() == Some("0") => {
                let when = r.reset.map(|t| format!(" Try again in {} minutes.", ((t - self.now) / 60).max(1))).unwrap_or_default();
                format!("API rate limit reached.{when} With GITHUB_TOKEN the limit goes up to 5,000 requests an hour.")
            }
            403 => format!("GitHub refused (403): {msg}. The token needs write access to this repository (Contents, Issues and Pull requests)."),
            404 => format!("Not found (404): {msg}. Check GITHUB_USER or the token's permissions."),
            code if msg.is_empty() => format!("GitHub API error {code}."),
            code => format!("GitHub error {code}: {msg}"),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        let r = self.t.send("GET", path, JSON).await?;
        match r.status {
            202 | 204 => Ok(Value::Null),
            200..=299 => serde_json::from_str(&r.body).map_err(|e| format!("Invalid response from GitHub: {e}")),
            _ => Err(self.explain(&r)),
        }
    }

    async fn get_raw(&self, path: &str) -> Result<Option<String>, String> {
        let r = self.t.send("GET", path, "application/vnd.github.raw+json").await?;
        match r.status {
            200..=299 => Ok(Some(r.body)),
            404 => Ok(None),
            _ => Err(self.explain(&r)),
        }
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

    async fn viewer_login(&self) -> Option<String> {
        if !self.t.has_token() {
            return None;
        }
        self.get("/user").await.ok().map(|v| s(&v, "login"))
    }

    fn repo(&self, v: &Value, starred: &HashSet<String>) -> Repo {
        let full = s(v, "full_name");
        let language = Some(s(v, "language")).filter(|l| !l.is_empty()).unwrap_or_else(|| "—".into());
        Repo {
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
        }
    }

    /// The account's repositories and those of its organizations (`cfg.orgs`), most recently
    /// pushed first. With the account's own token, `/user/repos` lists both, private ones
    /// included; otherwise the public repositories of the user and of its public organizations.
    pub async fn repos(&self, cfg: &Config, login: &str, own: bool) -> Result<Vec<Repo>, String> {
        use crate::state::Orgs;
        let starred: HashSet<String> = if self.t.has_token() {
            self.get("/user/starred?per_page=100").await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
                .iter().map(|r| s(r, "full_name")).collect()
        } else {
            HashSet::new()
        };
        let wanted = |r: &Repo| (!r.fork || cfg.include_forks) && (!r.archived || cfg.include_archived)
            && (r.owner.eq_ignore_ascii_case(login) || cfg.orgs.includes(&r.owner));
        let mut out = Vec::new();
        if own {
            // The token's owner also sees private repositories, and its organizations', here.
            let affiliation = if cfg.orgs == Orgs::Off { "owner" } else { "owner,organization_member" };
            let mut page = 1;
            while out.len() < cfg.max_repos && page <= 10 {
                let v = self.get(&format!("/user/repos?affiliation={affiliation}&sort=pushed&per_page=100&page={page}")).await?;
                let items = v.as_array().cloned().unwrap_or_default();
                out.extend(items.iter().map(|r| self.repo(r, &starred)).filter(wanted));
                if items.len() < 100 {
                    break;
                }
                page += 1;
            }
        } else {
            let mut page = 1;
            while out.len() < cfg.max_repos && page <= 10 {
                let v = self.get(&format!("/users/{login}/repos?type=owner&sort=pushed&per_page=100&page={page}")).await?;
                let items = v.as_array().cloned().unwrap_or_default();
                out.extend(items.iter().map(|r| self.repo(r, &starred)).filter(wanted));
                if items.len() < 100 {
                    break;
                }
                page += 1;
            }
            // Public organizations, then their public repositories (one request each).
            let orgs: Vec<String> = match &cfg.orgs {
                Orgs::Off => Vec::new(),
                Orgs::Only(list) => list.clone(),
                Orgs::All => self.get(&format!("/users/{login}/orgs?per_page=100")).await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
                    .iter().map(|o| s(o, "login")).filter(|o| !o.is_empty()).collect(),
            };
            for org in orgs.iter().take(20) {
                if let Ok(v) = self.get(&format!("/orgs/{org}/repos?type=public&sort=pushed&per_page=100")).await {
                    out.extend(v.as_array().cloned().unwrap_or_default().iter().map(|r| self.repo(r, &starred)).filter(wanted));
                }
            }
            out.sort_by(|a, b| b.pushed.cmp(&a.pushed));
        }
        out.truncate(cfg.max_repos);
        Ok(out)
    }

    async fn search(&self, q: &str) -> Vec<Value> {
        let q = q.replace(' ', "+").replace(':', "%3A");
        self.get(&format!("/search/issues?q={q}&sort=updated&order=desc&per_page={}", crate::model::SEARCH_CAP)).await
            .ok().and_then(|v| v.get("items").and_then(Value::as_array).cloned()).unwrap_or_default()
    }

    pub async fn pulls(&self, login: &str) -> Vec<Pull> {
        self.search(&format!("is:pr involves:{login}")).await.iter().map(|v| {
            let merged = v.get("pull_request").and_then(|p| p.get("merged_at")).is_some_and(|m| !m.is_null());
            let state = if merged { PullState::Merged } else if s(v, "state") == "closed" { PullState::Closed } else if b(v, "draft") { PullState::Draft } else { PullState::Open };
            let updated_at = s(v, "updated_at");
            Pull {
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
        }).collect()
    }

    /// Open issues in the account's repositories and in those of `orgs` (search ORs the owners).
    pub async fn issues(&self, login: &str, orgs: &[String]) -> Vec<Issue> {
        let owners: String = orgs.iter().take(10).map(|o| format!(" org:{o}")).collect();
        self.search(&format!("is:issue is:open archived:false user:{login}{owners}")).await.iter().map(|v| {
            let created_at = s(v, "created_at");
            Issue {
                number: n(v, "number"),
                title: s(v, "title"),
                repo: repo_from_url(&s(v, "repository_url")),
                label: v.get("labels").and_then(Value::as_array).and_then(|l| l.first()).map(|l| (s(l, "name"), hex_hue(&s(l, "color")))),
                created_at: parse_iso(&created_at).unwrap_or(0),
                comments: n(v, "comments"),
                url: s(v, "html_url"),
            }
        }).collect()
    }

    pub async fn runs(&self, full: &str) -> Vec<Run> {
        let v = self.get(&format!("/repos/{full}/actions/runs?per_page=4")).await.unwrap_or(Value::Null);
        v.get("workflow_runs").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|r| {
            let checks = match (s(r, "status").as_str(), s(r, "conclusion").as_str()) {
                ("completed", "success" | "skipped" | "neutral") => Checks::Passing,
                ("completed", _) => Checks::Failing,
                _ => Checks::Running,
            };
            let start = parse_iso(&s(r, "run_started_at"));
            let end = parse_iso(&s(r, "updated_at"));
            Run {
                workflow: s(r, "name"),
                repo: full.to_owned(),
                branch: s(r, "head_branch"),
                duration: match (start, end) {
                    (Some(a), Some(b)) if checks != Checks::Running => duration(b - a),
                    (Some(a), _) => duration(self.now - a),
                    _ => String::new(),
                },
                updated_at: end.unwrap_or(0),
                checks,
                url: s(r, "html_url"),
            }
        }).collect()
    }

    /// The owner's commits per week over the last 26 weeks (zeros while GitHub computes them).
    pub async fn participation(&self, full: &str) -> Vec<u32> {
        let v = self.get(&format!("/repos/{full}/stats/participation")).await.unwrap_or(Value::Null);
        let weeks: Vec<u32> = v.get("owner").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u32).collect()).unwrap_or_default();
        if weeks.len() >= 26 { weeks[weeks.len() - 26..].to_vec() } else { vec![0; 26] }
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
        d.readme = self.get_raw(&format!("/repos/{full}/readme?ref={at}")).await.ok().flatten().map(|t| t.chars().take(200_000).collect());
        if let Ok(v) = self.get(&format!("/repos/{full}/languages")).await {
            let mut langs: Vec<(String, u64)> = v.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0))).collect()).unwrap_or_default();
            langs.sort_by(|a, b| b.1.cmp(&a.1));
            d.languages = langs;
        }
        if let Ok(v) = self.get(&format!("/repos/{full}/commits?sha={at}&per_page=1")).await
            && let Some(c) = v.as_array().and_then(|a| a.first())
        {
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
        d
    }

    /// Branch names, the default branch first.
    pub async fn branches(&self, full: &str, default: &str) -> Result<Vec<String>, String> {
        let v = self.get(&format!("/repos/{full}/branches?per_page=100")).await?;
        let mut names: Vec<String> = v.as_array().cloned().unwrap_or_default().iter().map(|b| s(b, "name")).filter(|n| !n.is_empty()).collect();
        names.sort_by_key(|n| (n != default, n.to_lowercase()));
        Ok(names)
    }

    pub async fn create_issue(&self, full: &str, title: &str, body: &str) -> Result<Created, String> {
        let v = self.write("POST", &format!("/repos/{full}/issues"), json!({ "title": title, "body": body })).await?;
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

    pub async fn close_issue(&self, full: &str, number: u32) -> Result<(), String> {
        self.write("PATCH", &format!("/repos/{full}/issues/{number}"), json!({ "state": "closed", "state_reason": "completed" })).await.map(|_| ())
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

    pub async fn merge_pull(&self, full: &str, number: u32, method: &str) -> Result<(), String> {
        self.write("PUT", &format!("/repos/{full}/pulls/{number}/merge"), json!({ "merge_method": method })).await.map(|_| ())
    }

    pub async fn set_star(&self, full: &str, on: bool) -> Result<(), String> {
        let r = self.t.send(if on { "PUT" } else { "DELETE" }, &format!("/user/starred/{full}"), JSON).await?;
        if (200..300).contains(&r.status) { Ok(()) } else { Err(self.explain(&r)) }
    }
}

/// Everything the overview and lists need, in one go (about 4 + 2 × `activity_repos` requests).
pub async fn load_all(cfg: &Config) -> Result<Data, String> {
    let gh = Gh::new(cfg);
    let profile = gh.profile(cfg).await?;
    let own = gh.t.has_token() && match &cfg.user {
        None => true,
        Some(u) => gh.viewer_login().await.is_some_and(|v| v.eq_ignore_ascii_case(u)),
    };
    let repos = gh.repos(cfg, &profile.login, own).await?;
    let pulls = gh.pulls(&profile.login).await;
    // The organizations whose repositories were loaded: their issues too.
    let mut orgs: Vec<String> = repos.iter().map(|r| r.owner.clone()).filter(|o| !o.eq_ignore_ascii_case(&profile.login)).collect();
    orgs.sort_by_key(|o| o.to_lowercase());
    orgs.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let issues = gh.issues(&profile.login, &orgs).await;
    let active: Vec<String> = repos.iter().filter(|r| !r.archived).take(cfg.activity_repos).map(|r| r.full_name.clone()).collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut activity = vec![0u32; 26];
    for full in &active {
        runs.extend(gh.runs(full).await);
        for (i, w) in gh.participation(full).await.iter().enumerate() {
            activity[i] += w;
        }
    }
    runs.truncate(20);
    Ok(Data { profile, repos, pulls, issues, runs, activity, sample: false })
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

pub async fn load_pull_files(cfg: &Config, full: &str, number: u32) -> Result<Vec<PullFile>, String> {
    Gh::new(cfg).pull_files(full, number).await
}

pub async fn load_detail(cfg: &Config, full: &str, at: &str) -> Detail {
    Gh::new(cfg).detail(full, at).await
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

/// One of the writes RepoHub makes on GitHub. Running it returns a message for the toast.
#[derive(Clone, Debug)]
pub enum Write {
    Issue { title: String, body: String },
    Branch { name: String, from: String },
    Pull(NewPull),
    CloseIssue(u32),
    Merge { number: u32, method: &'static str },
}

/// What a finished write changed, for updating the screen without waiting for a reload.
#[derive(Clone, Debug)]
pub enum Done {
    Issue(Created),
    Branch(String),
    Pull(Created),
    Closed(u32),
    Merged(u32),
}

pub async fn run(cfg: &Config, full: &str, w: Write) -> Result<Done, String> {
    let gh = Gh::new(cfg);
    match w {
        Write::Issue { title, body } => gh.create_issue(full, &title, &body).await.map(Done::Issue),
        Write::Branch { name, from } => gh.create_branch(full, &name, &from).await.map(|_| Done::Branch(name)),
        Write::Pull(p) => gh.create_pull(full, &p).await.map(Done::Pull),
        Write::CloseIssue(n) => gh.close_issue(full, n).await.map(|_| Done::Closed(n)),
        Write::Merge { number, method } => gh.merge_pull(full, number, method).await.map(|_| Done::Merged(number)),
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
}
