//! The reads, one method on `Gh` each: repositories, the pull request and issue searches, Actions, a
//! repository's details and branches, conversations, notifications, triage choices, a pull request's files,
//! commits and releases.

use super::*;

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
        if let Some(sha) = pull.as_ref().and_then(|(d, _)| d.get("head")).map(|h| s(h, "sha")).filter(|x| !x.is_empty())
            && let Ok(v) = self.get(&format!("/repos/{full}/commits/{sha}/check-runs?per_page=100")).await
        {
            let list = parse_check_runs(&v);
            t.checks_total = check_runs_total(&v).max(list.len());
            t.checks = Some(list);
        }
        Ok(t)
    }

    /// Your unread notifications, newest first (the first 50).
    pub async fn notifications(&self) -> Result<Vec<Notification>, String> {
        let v = self.get(&format!("/notifications?per_page={}", crate::inbox::READ_AT_MOST)).await?;
        Ok(v.as_array().into_iter().flatten().filter_map(parse_notification).collect())
    }

    /// The commit pull request `number` shows, and the comments on lines of its diff (the
    /// first 100). A comment on a line that changed since (`line` is null) is left out.
    pub async fn line_comments(&self, full: &str, number: u32) -> Result<LineComments, String> {
        let detail = self.get(&format!("/repos/{full}/pulls/{number}")).await?;
        let head = detail.get("head").map(|h| s(h, "sha")).unwrap_or_default();
        let (list, resp) = self.get_resp(&format!("/repos/{full}/pulls/{number}/comments?per_page=100")).await?;
        Ok(parse_line_comments(head, &list, resp.link.as_deref(), &self.api_base))
    }

    /// A repository's labels, assignable people or open milestones (the first 100).
    pub async fn choices(&self, full: &str, source: crate::triage::Source) -> Result<crate::triage::Choices, String> {
        let (v, resp) = self.get_resp(&source.path(full)).await?;
        let items = source.read(&v);
        let more = next_page_path(resp.link.as_deref(), &self.api_base).is_some() || v.as_array().is_some_and(|a| a.len() >= 100);
        Ok(crate::triage::Choices { items, more })
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

    /// Page `page` of the releases ([`crate::releases::PAGE`] a page), newest first, and whether
    /// GitHub has more. GitHub lists drafts only to people who can push to the repository.
    pub async fn releases(&self, full: &str, page: usize) -> Result<(Vec<crate::releases::Release>, bool), String> {
        let per = crate::releases::PAGE;
        let (v, resp) = self.get_resp(&format!("/repos/{full}/releases?per_page={per}&page={page}")).await?;
        let more = match resp.link.as_deref() {
            Some(link) => next_page_path(Some(link), &self.api_base).is_some(),
            None => v.as_array().is_some_and(|a| a.len() >= per),
        };
        Ok((crate::releases::parse_releases(&v), more))
    }

    /// The newest [`crate::releases::TAGS`] tags, and whether GitHub has more.
    pub async fn tags(&self, full: &str) -> Result<(Vec<crate::releases::Tag>, bool), String> {
        let per = crate::releases::TAGS;
        let (v, resp) = self.get_resp(&format!("/repos/{full}/tags?per_page={per}")).await?;
        let more = match resp.link.as_deref() {
            Some(link) => next_page_path(Some(link), &self.api_base).is_some(),
            None => v.as_array().is_some_and(|a| a.len() >= per),
        };
        Ok((crate::releases::parse_tags(&v), more))
    }

    /// The release GitHub shows as the latest; `None` when there's none (GitHub answers 404).
    pub async fn latest_release(&self, full: &str) -> Result<Option<crate::releases::Release>, String> {
        let r = self.t.send("GET", &format!("/repos/{full}/releases/latest"), JSON).await?;
        match r.status {
            200..=299 => serde_json::from_str::<Value>(&r.body).map(|v| crate::releases::parse_release(&v)).map_err(|e| format!("Invalid response from GitHub: {e}")),
            404 => Ok(None),
            _ => Err(self.explain(&r)),
        }
    }

    /// The first read of the Releases section: the first page of releases (and whether there
    /// are more), the tags, and the latest release.
    pub async fn release_listing(&self, full: &str) -> Result<(crate::releases::Listing, bool, Option<crate::releases::Latest>), String> {
        let (releases, more) = self.releases(full, 1).await?;
        let (tags, tags_more) = self.tags(full).await?;
        let latest = self.latest_release(full).await?.map(|l| crate::releases::Latest { id: l.id, tag: l.tag });
        Ok((crate::releases::Listing { releases, tags, tags_more }, more, latest))
    }

    /// The notes GitHub writes for tag `tag` (made from `target` when it's new): the changes
    /// since `previous`, or without it since the release GitHub picks. Nothing is saved.
    pub async fn generate_notes(&self, full: &str, tag: &str, target: &str, previous: Option<&str>) -> Result<String, String> {
        let mut body = json!({ "tag_name": tag, "target_commitish": target });
        if let Some(p) = previous {
            body["previous_tag_name"] = p.into();
        }
        let v = self.release_write("POST", &format!("/repos/{full}/releases/generate-notes"), body).await?;
        Ok(s(&v, "body"))
    }
}
