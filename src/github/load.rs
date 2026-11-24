//! Loading: `load_all` (the parts of a load side by side) and the `load_*` functions the app runs in the
//! background, one per read.

use super::*;

/// Everything the overview and lists need, in one go (about 4 + 2 × `activity_repos` requests).
pub async fn load_all(cfg: &Config) -> Result<Data, LoadError> {
    load_all_with(cfg, |_: Data| {}).await
}

/// How many requests a load sends at once.
pub const AT_ONCE: usize = 6;

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
///          └─ pull requests + waiting for your review (needs your account)
/// ```
pub async fn load_all_with(cfg: &Config, early: impl Fn(Data) + net::MaybeSend + net::MaybeSync) -> Result<Data, LoadError> {
    load_all_reporting(cfg, |_: String| {}, early).await
}

/// [`load_all_with`], also calling `known` with the account's login as soon as GitHub has
/// said it (the first request), before the repositories are read.
pub async fn load_all_reporting(cfg: &Config, known: impl Fn(String) + net::MaybeSend + net::MaybeSync, early: impl Fn(Data) + net::MaybeSend + net::MaybeSync) -> Result<Data, LoadError> {
    let gh = Gh::new(cfg).limited(AT_ONCE);
    let gh = &gh;
    // With a user name and a token: whether the token is that user's, next to the profile.
    let ask_viewer = gh.t.has_token() && cfg.user.is_some();
    let (read, viewer) = net::join(gh.profile(cfg), async move { if ask_viewer { gh.viewer_login().await } else { None } }).await;
    let profile = match read {
        Ok(p) => {
            known(p.login.clone());
            p
        }
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
        // Pull requests and those waiting for your review (with a token: GitHub only knows
        // whose review is asked for an account) run side by side.
        async move {
            let (pulls, waiting) = net::join(
                gh.pulls(login),
                async move { if own { Some(gh.review_requested(login).await) } else { None } },
            ).await;
            let waiting = if pulls.is_ok() { waiting } else { None };
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

pub async fn load_more_pulls(cfg: &Config, login: &str, open: bool, page: usize) -> Result<(Vec<Pull>, usize), String> {
    Gh::new(cfg).pulls_page(login, open, page).await
}

pub async fn load_more_issues(cfg: &Config, login: &str, orgs: &[String], page: usize) -> Result<(Vec<Issue>, usize), String> {
    Gh::new(cfg).issues_page(login, orgs, page).await
}

pub async fn load_notifications(cfg: &Config) -> Result<Vec<Notification>, String> {
    Gh::new(cfg).notifications().await
}

pub async fn load_line_comments(cfg: &Config, full: &str, number: u32) -> Result<LineComments, String> {
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

pub async fn load_release_listing(cfg: &Config, full: &str) -> Result<(crate::releases::Listing, bool, Option<crate::releases::Latest>), String> {
    Gh::new(cfg).release_listing(full).await
}

pub async fn load_releases(cfg: &Config, full: &str, page: usize) -> Result<(Vec<crate::releases::Release>, bool), String> {
    Gh::new(cfg).releases(full, page).await
}

pub async fn load_latest_release(cfg: &Config, full: &str) -> Result<Option<crate::releases::Release>, String> {
    Gh::new(cfg).latest_release(full).await
}

pub async fn generate_release_notes(cfg: &Config, full: &str, tag: &str, target: &str, previous: Option<&str>) -> Result<String, String> {
    Gh::new(cfg).generate_notes(full, tag, target, previous).await
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

pub async fn branches(cfg: &Config, full: &str, default: &str) -> Result<Vec<String>, String> {
    Gh::new(cfg).branches(full, default).await
}
