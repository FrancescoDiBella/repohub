//! The writes: issues, branches, pull requests, comments and reviews, triage, releases, merging and starring,
//! and `Write`, `Done` and `run`, which the app's forms and buttons go through.

use super::*;

impl Gh {
    pub async fn rerun_run(&self, full: &str, run_id: u64, failed_only: bool) -> Result<(), String> {
        let action = if failed_only { "rerun-failed-jobs" } else { "rerun" };
        let r = self.t.send_body("POST", &format!("/repos/{full}/actions/runs/{run_id}/{action}"), JSON, "{}").await?;
        if (200..300).contains(&r.status) {
            Ok(())
        } else {
            Err(self.explain_actions_write(&r))
        }
    }

    pub async fn create_issue(&self, full: &str, title: &str, body: &str, labels: &[String], assignees: &[String]) -> Result<Created, String> {
        let v = self.write("POST", &format!("/repos/{full}/issues"), issue_body(title, body, labels, assignees)).await?;
        Ok(parse_created(&v, title, false))
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
        Ok(parse_created(&v, &p.title, p.draft))
    }

    pub async fn mark_read(&self, id: &str) -> Result<(), String> {
        let r = self.t.send_body("PATCH", &format!("/notifications/threads/{id}"), JSON, "").await?;
        match r.status {
            200..=299 | 304 => Ok(()),
            _ => Err(self.explain(&r)),
        }
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

    /// A write to the releases, with GitHub's refusals explained in the releases' terms.
    pub(super) async fn release_write(&self, method: &str, path: &str, body: Value) -> Result<Value, String> {
        let r = self.t.send_body(method, path, JSON, &body.to_string()).await?;
        match r.status {
            200..=299 => Ok(serde_json::from_str(&r.body).unwrap_or(Value::Null)),
            _ => Err(self.explain_release_write(&r)),
        }
    }

    /// Creates a release (a draft, or published), or saves draft `id` again (and publishes it
    /// when `n.draft` is false).
    pub async fn save_release(&self, full: &str, id: Option<u64>, n: &NewRelease) -> Result<crate::releases::Release, String> {
        let body = json!({ "tag_name": n.tag, "target_commitish": n.target, "name": n.name, "body": n.body, "draft": n.draft, "prerelease": n.prerelease });
        let v = match id {
            Some(id) => self.release_write("PATCH", &format!("/repos/{full}/releases/{id}"), body).await?,
            None => self.release_write("POST", &format!("/repos/{full}/releases"), body).await?,
        };
        Ok(crate::releases::parse_release(&v).unwrap_or_else(|| crate::releases::Release::unread(&n.tag, n.draft, n.prerelease)))
    }

    /// Publishes draft `id` as it is.
    pub async fn publish_release(&self, full: &str, id: u64) -> Result<crate::releases::Release, String> {
        let v = self.release_write("PATCH", &format!("/repos/{full}/releases/{id}"), json!({ "draft": false })).await?;
        Ok(crate::releases::parse_release(&v).unwrap_or_else(|| crate::releases::Release::unread("", false, false)))
    }

    pub async fn merge_pull(&self, full: &str, number: u32, method: &str) -> Result<(), String> {
        self.write("PUT", &format!("/repos/{full}/pulls/{number}/merge"), json!({ "merge_method": method })).await.map(|_| ())
    }

    pub async fn set_star(&self, full: &str, on: bool) -> Result<(), String> {
        let r = self.t.send(if on { "PUT" } else { "DELETE" }, &format!("/user/starred/{full}"), JSON).await?;
        if (200..300).contains(&r.status) { Ok(()) } else { Err(self.explain(&r)) }
    }
}

pub async fn mark_read(cfg: &Config, id: &str) -> Result<(), String> {
    Gh::new(cfg).mark_read(id).await
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

/// What GitHub created: its number and page, and what its row shows until the lists reload.
/// Each write carries its own, so two forms sent at once each get their own row.
#[derive(Clone, Debug, Default)]
pub struct Created {
    pub number: u32,
    pub url: String,
    pub title: String,
    /// A draft pull request.
    pub draft: bool,
    /// The labels and assignees GitHub set; `None` when its answer doesn't say (the form's
    /// picks are used then).
    pub labels: Option<Vec<(String, f32)>>,
    pub assignees: Option<Vec<String>>,
}

/// GitHub's answer to creating an issue or a pull request; what was sent where it doesn't say.
pub fn parse_created(v: &Value, title: &str, draft: bool) -> Created {
    let answered = s(v, "title");
    Created {
        number: n(v, "number"),
        url: s(v, "html_url"),
        title: if answered.is_empty() { title.to_owned() } else { answered },
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(draft),
        labels: v.get("labels").and_then(Value::as_array).map(|_| labels_of(v)),
        assignees: v.get("assignees").and_then(Value::as_array).map(|_| logins_of(v, "assignees")),
    }
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

/// A release to create or save: its tag, the branch a new tag is made from, title and notes.
#[derive(Clone, Debug)]
pub struct NewRelease {
    pub tag: String,
    pub target: String,
    pub name: String,
    pub body: String,
    /// Saved as a draft; false publishes it.
    pub draft: bool,
    pub prerelease: bool,
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
    /// A new release: a draft, or published (which creates its tag).
    Release(NewRelease),
    /// Draft `id` saved again, or published with these changes.
    EditRelease { id: u64, release: NewRelease },
    /// Draft `id` published as it is.
    PublishRelease { id: u64 },
    /// Labels, assignees, milestone, reviewers, title or description of an issue or pull
    /// request: one kind of change at a time, so the toast can say what changed.
    Triage { number: u32, change: crate::triage::Change },
}

impl Write {
    /// The issue or pull request the write is about, when there is one.
    pub fn number(&self) -> Option<u32> {
        match self {
            Write::CloseIssue(n) | Write::ReopenIssue(n) | Write::SetPullOpen { number: n, .. } | Write::Merge { number: n, .. } | Write::Comment { number: n, .. } | Write::Review { number: n, .. } | Write::LineComment { number: n, .. } | Write::Triage { number: n, .. } => Some(*n),
            Write::Issue { .. } | Write::Branch { .. } | Write::Pull(_) | Write::RerunRun { .. } | Write::Release(_) | Write::EditRelease { .. } | Write::PublishRelease { .. } => None,
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
    /// A release saved as a draft or published, as GitHub has it now.
    Release(Box<crate::releases::Release>),
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
        Write::Release(n) => gh.save_release(full, None, &n).await.map(|r| Done::Release(Box::new(r))),
        Write::EditRelease { id, release } => gh.save_release(full, Some(id), &release).await.map(|r| Done::Release(Box::new(r))),
        Write::PublishRelease { id } => gh.publish_release(full, id).await.map(|r| Done::Release(Box::new(r))),
        Write::Triage { number, change } => gh.triage(full, number, &change).await.map(|applied| Done::Triaged { number, change, applied }),
    }
}

pub async fn set_star(cfg: &Config, full: &str, on: bool) -> Result<(), String> {
    Gh::new(cfg).set_star(full, on).await
}
