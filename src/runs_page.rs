//! Actions page filters, quick re-run controls, and the in-app workflow run detail view
//! (jobs, steps, and the last 200 lines of a job's log).
use std::collections::HashMap;

use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, SegmentedControl};

use crate::model::{Checks, JobLog, Run, RunJob, same_repo};
use crate::state::{Load, RunStatusFilter};
use crate::ui::{self, panel, panel_head, rule};
use crate::{App, actions, github, icons, lists, pages};

/// Active filters on the Actions page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunsFilter {
    pub status: RunStatusFilter,
    pub repo: Option<String>,
    pub workflow: Option<String>,
}

impl RunsFilter {
    pub fn is_empty(&self) -> bool {
        self.status == RunStatusFilter::All && self.repo.is_none() && self.workflow.is_none()
    }

    pub fn clear(&mut self) {
        *self = RunsFilter::default();
    }

    pub fn matches(&self, r: &Run) -> bool {
        self.status.matches(r.checks)
            && self.repo.as_deref().is_none_or(|repo| repo.eq_ignore_ascii_case(&r.repo))
            && self.workflow.as_deref().is_none_or(|wf| wf.eq_ignore_ascii_case(&r.workflow))
    }
}

/// Distinct values in `items`, most frequent first (ties broken alphabetically), at most `max`.
fn top(items: impl Iterator<Item = String>, max: usize) -> Vec<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for item in items {
        if item.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(name, _)| name.eq_ignore_ascii_case(&item)) {
            Some(c) => c.1 += 1,
            None => counts.push((item, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    counts.into_iter().take(max).map(|(name, _)| name).collect()
}

/// Top repositories and workflow names present in `runs`.
pub fn choices<'a>(runs: impl Iterator<Item = &'a Run> + Clone) -> (Vec<String>, Vec<String>) {
    (
        top(runs.clone().map(|r| r.repo.clone()), 12),
        top(runs.map(|r| r.workflow.clone()), 12),
    )
}

/// An open workflow run in the detail view.
pub struct OpenRun {
    pub repo: String,
    pub run_id: u64,
    pub workflow: String,
    pub branch: String,
    pub checks: Checks,
    pub duration: String,
    pub updated_at: i64,
    pub url: String,
    pub epoch: u64,
    pub jobs: Load<Vec<RunJob>>,
    pub selected_job: Option<u64>,
    pub logs: HashMap<u64, Load<JobLog>>,
    pub write_error: Option<String>,
}

impl OpenRun {
    pub fn as_run(&self) -> Run {
        Run {
            id: self.run_id,
            workflow: self.workflow.clone(),
            repo: self.repo.clone(),
            branch: self.branch.clone(),
            checks: self.checks,
            duration: self.duration.clone(),
            updated_at: self.updated_at,
            url: self.url.clone(),
            pull_numbers: Vec::new(),
        }
    }
}

/// Busy-set key while a workflow re-run request is in flight.
pub fn run_busy_key(repo: &str, run_id: u64) -> String {
    format!("{}@run:{run_id}", repo.to_ascii_lowercase())
}

/// Resolves a run's `repo` field to `owner/name` when sample data stores only `name`.
pub fn full_repo_of(s: &App, repo: &str) -> String {
    if repo.contains('/') {
        return repo.to_owned();
    }
    s.data.repos.iter()
        .find(|r| same_repo(&r.full_name, repo))
        .map(|r| r.full_name.clone())
        .unwrap_or_else(|| repo.to_owned())
}

/// Filter bar for the Actions page: status (`All` / `Failed` / `Running`), repository, and workflow.
pub fn filter_bar(s: &App, k: &Kit, repos: Vec<String>, workflows: Vec<String>) -> El {
    let f = &s.runs_filter;
    let status = SegmentedControl::new(&RunStatusFilter::LABELS, Some(f.status.index()))
        .label("Filter runs by status")
        .fit()
        .on_pick(|s: &mut App, i, _| s.runs_filter.status = RunStatusFilter::from_index(i))
        .build(k)
        .key("runs-status");
    let mut top_row = row().w_full().gap(8.0).center_y().flex_wrap().child(status);
    if !f.is_empty() {
        top_row = top_row.child(
            Button::new("Clear")
                .icon(&icons::CROSS)
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.runs_filter.clear())
                .build(k)
                .key("runs-clear"),
        );
    }
    let mut out = col().w_full().gap(8.0).key("runs-filters").child(top_row);
    let chip = |label: String, on: bool, key: String| {
        Button::new(label)
            .control_size(ControlSize::Small)
            .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline })
            .build(k)
            .key(key)
    };
    if repos.len() > 1 || f.repo.is_some() {
        let mut r = row().w_full().gap(6.0).center_y().flex_wrap()
            .child(text("Repository").font(k.ty.small_medium).color(k.c.text_muted).pr(4.0));
        for v in &repos {
            let on = f.repo.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(v));
            let short = v.rsplit('/').next().unwrap_or(v);
            let duplicate_short = repos.iter().filter(|other| other.rsplit('/').next().unwrap_or(other).eq_ignore_ascii_case(short)).count() > 1;
            let shown = if duplicate_short { v.clone() } else { short.to_owned() };
            let value = v.clone();
            r = r.child(chip(shown, on, format!("runs-repo-{v}")).on_press(move |s: &mut App| {
                s.runs_filter.repo = if on { None } else { Some(value.clone()) };
            }));
        }
        out = out.child(r);
    }
    if workflows.len() > 1 || f.workflow.is_some() {
        let mut r = row().w_full().gap(6.0).center_y().flex_wrap()
            .child(text("Workflow").font(k.ty.small_medium).color(k.c.text_muted).pr(4.0));
        for v in workflows {
            let on = f.workflow.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(&v));
            let value = v.clone();
            r = r.child(chip(v.clone(), on, format!("runs-workflow-{v}")).on_press(move |s: &mut App| {
                s.runs_filter.workflow = if on { None } else { Some(value.clone()) };
            }));
        }
        out = out.child(r);
    }
    out
}

/// A workflow run row with a quick **Re-run failed** button when the run failed.
pub fn run_line(s: &App, k: &Kit, r: &Run, compact: bool) -> El {
    let key = if r.id > 0 { format!("run-{}-{}", r.repo, r.id) } else { format!("run-{}-{}", r.repo, r.workflow) };
    crate::keyboard::mark(k, run_line_of(s, k, r, compact), s.marked(&key))
}

fn run_line_of(s: &App, k: &Kit, r: &Run, compact: bool) -> El {
    let line = lists::run_row(k, r, s.now, compact);
    if r.checks != Checks::Failing || r.id == 0 {
        return line;
    }
    let full = full_repo_of(s, &r.repo);
    let key = run_busy_key(&full, r.id);
    let busy = s.busy.contains(&key);
    let blocked = actions::blocked(s);
    let run_id = r.id;
    let btn = Button::new(if busy { "Re-running…" } else { "Re-run failed" })
        .icon(&icons::REFRESH)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .help(blocked.unwrap_or("Re-run only the failed jobs of this workflow run"))
        .disabled(busy || blocked.is_some())
        .on_press(move |s: &mut App| s.write(&full, github::Write::RerunRun { run_id, failed_only: true }))
        .build(k)
        .key(format!("rerun-row-{}-{}", r.repo, r.id));
    if compact {
        col().w_full().child(line).child(row().w_full().px(16.0).pb(10.0).gap(6.0).child(btn))
    } else {
        row().w_full().center_y().child(line.grow(1.0).shrink()).child(btn.mr(12.0))
    }
}

fn check_visual(k: &Kit, checks: Checks) -> (&'static rune::Icon, rune::Color) {
    match checks {
        Checks::Passing => (&icons::CHECK, k.c.success.icon),
        Checks::Failing => (&icons::CROSS, k.c.danger.icon),
        Checks::Running => (&icons::DOT, k.c.warning.icon),
    }
}

/// In-app detail view for an open workflow run: controls, jobs & steps, and the selected job's log.
pub fn view(s: &App, k: &Kit, o: &OpenRun) -> El {
    let back = Button::new("Back")
        .icon(&icons::BACK)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.open_run = None)
        .build(k)
        .key("run-back");
    let busy = s.busy.contains(&run_busy_key(&o.repo, o.run_id));
    let blocked = actions::blocked(s);
    let has_failed_job = o.checks == Checks::Failing
        || o.jobs.ready().is_some_and(|jobs| jobs.iter().any(|j| j.checks == Checks::Failing));

    let mut top = row().w_full().gap(8.0).center_y().flex_wrap().child(back).child(el().grow(1.0));
    if has_failed_job {
        top = top.child(
            Button::new(if busy { "Re-running…" } else { "Re-run failed jobs" })
                .icon(&icons::REFRESH)
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .disabled(busy || blocked.is_some())
                .on_press(|s: &mut App| s.rerun_open_run(true))
                .build(k)
                .key("run-rerun-failed"),
        );
    }
    top = top
        .child(
            Button::new(if busy && !has_failed_job { "Re-running…" } else { "Re-run all" })
                .icon(&icons::REFRESH)
                .control_size(ControlSize::Small)
                .disabled(busy || blocked.is_some() || o.checks == Checks::Running)
                .on_press(|s: &mut App| s.rerun_open_run(false))
                .build(k)
                .key("run-rerun-all"),
        )
        .child(
            Button::new("Refresh")
                .icon(&icons::REFRESH)
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.reload_open_run())
                .build(k)
                .key("run-refresh"),
        );
    if !o.url.is_empty() {
        top = top.child(pages::open_button(k, "Open on GitHub", &o.url, "run-github"));
    }

    let (state_text, state_ink) = match o.checks {
        Checks::Passing => ("Succeeded", k.c.success.icon),
        Checks::Failing => ("Failed", k.c.danger.icon),
        Checks::Running => ("Running", k.c.warning.icon),
    };
    let age = o.as_run().age_ago(s.now);
    let mut timing = o.duration.clone();
    if !age.is_empty() {
        if !timing.is_empty() {
            timing.push_str(" · ");
        }
        timing.push_str(&age);
    }
    let mut meta = row().w_full().gap(10.0).center_y().flex_wrap()
        .child(ui::pill(k, state_text, state_ink))
        .child(text(format!("{} · run #{}", o.repo, o.run_id)).font(k.ty.small).color(k.c.text_muted))
        .child(ui::stat(k, &icons::BRANCH, o.branch.clone()));
    if !timing.is_empty() {
        meta = meta.child(text(timing).font(k.ty.text_small).color(k.c.text_faint));
    }

    let mut page = col().w_full().gap(16.0).key("run-detail")
        .child(top)
        .child(col().w_full().gap(6.0).key("run-header")
            .child(row().gap(10.0).center_y().flex_wrap()
                .child(ui::checks(k, o.checks))
                .child(text(o.workflow.clone()).font(ui::H1).color(k.c.text_strong).wrap()))
            .child(meta));

    if let Some(err) = &o.write_error {
        page = page.child(
            panel(k)
                .padding(14.0)
                .border(1.0, k.c.danger.icon.alpha(0.55))
                .role("alert")
                .key("run-write-error")
                .child(row().w_full().gap(10.0).center_y().justify_between().flex_wrap()
                    .child(row().gap(8.0).center_y().grow(1.0).shrink()
                        .child(ui::ico(&icons::CROSS, 15.0, k.c.danger.icon))
                        .child(text(err.clone()).font(k.ty.small).color(k.c.text_base).grow(1.0).shrink().wrap()))
                    .child(Button::new("Dismiss").variant(ButtonVariant::Ghost).control_size(ControlSize::Small)
                        .on_press(|s: &mut App| {
                            if let Some(o) = &mut s.open_run {
                                o.write_error = None;
                            }
                        })
                        .build(k)
                        .key("run-dismiss-error"))),
        );
    } else if let Some(why) = blocked {
        page = page.child(text(why).font(k.ty.small).color(k.c.warning.icon).wrap().key("run-blocked"));
    }

    let jobs = match &o.jobs {
        Load::Loading => {
            return page.child(
                panel(k).padding(28.0).items_center().role("status").key("run-jobs-loading")
                    .child(text("Loading jobs and steps…").font(k.ty.ui).color(k.c.text_muted)),
            );
        }
        Load::Failed(e) => {
            return page.child(
                panel(k).padding(28.0).items_center().gap(10.0).role("alert").key("run-jobs-failed")
                    .child(text(e.clone()).font(k.ty.small).color(k.c.text_base).wrap())
                    .child(Button::new("Retry").icon(&icons::REFRESH).control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.load_run_jobs()).build(k).key("run-jobs-retry")),
            );
        }
        Load::Ready(jobs) if jobs.is_empty() => {
            return page.child(
                panel(k).key("run-jobs-empty")
                    .child(ui::empty(k, "No jobs recorded for this workflow run.", "GitHub may still be starting it: Refresh asks again, Open on GitHub shows it there.")),
            );
        }
        Load::Ready(jobs) => jobs,
    };

    let selected = o.selected_job
        .and_then(|id| jobs.iter().find(|j| j.id == id))
        .or_else(|| jobs.first());

    let mut jobs_panel = panel(k).key("run-jobs")
        .child(panel_head(
            k,
            &format!("Jobs ({})", jobs.len()),
            Some(text("Click a job to inspect its steps and log").font(k.ty.text_small).color(k.c.text_faint)),
        ));
    for job in jobs {
        let on = selected.is_some_and(|sel| sel.id == job.id);
        let (_, ink) = check_visual(k, job.checks);
        let job_id = job.id;
        let a11y = if job.duration.is_empty() {
            format!("Job {}: {}", job.name, job.status_label)
        } else {
            format!("Job {}: {}, {}", job.name, job.status_label, job.duration)
        };
        let mut head = row().w_full().px(16.0).py(10.0).gap(10.0).center_y()
            .hover(k.c.hover).press(k.c.pressed)
            .role("button").label(a11y).key(format!("run-job-{job_id}"))
            .on_press(move |s: &mut App| s.select_run_job(job_id))
            .child(ui::checks(k, job.checks))
            .child(text(job.name.clone()).font(if on { k.ty.small_medium } else { k.ty.small }).color(if on { k.c.text_strong } else { k.c.text_base }).grow(1.0).shrink().wrap())
            .child(ui::pill(k, &job.status_label, ink));
        if !job.duration.is_empty() {
            head = head.child(text(job.duration.clone()).font(k.ty.text_small).color(k.c.text_faint));
        }
        if on {
            head = head.fill(k.c.bg_accent.alpha(0.12));
        }
        jobs_panel = jobs_panel.child(rule(k)).child(head);
        if (on || job.checks == Checks::Failing) && !job.steps.is_empty() {
            let mut step_list = col().w_full().px(20.0).py(8.0).gap(5.0).fill(k.c.layer_01);
            for st in &job.steps {
                let (st_icon, st_ink) = check_visual(k, st.checks);
                let mut st_row = row().w_full().gap(8.0).center_y().key(format!("run-step-{job_id}-{}", st.number))
                    .child(ui::ico(st_icon, 13.0, st_ink))
                    .child(text(format!("{}. {}", st.number, st.name)).font(k.ty.text_small).color(if st.checks == Checks::Failing { k.c.text_strong } else { k.c.text_base }).grow(1.0).shrink().wrap());
                if st.checks != Checks::Passing {
                    st_row = st_row.child(ui::pill(k, &st.status_label, st_ink));
                }
                if !st.duration.is_empty() {
                    st_row = st_row.child(text(st.duration.clone()).font(k.ty.text_small).color(k.c.text_faint));
                }
                step_list = step_list.child(st_row);
            }
            jobs_panel = jobs_panel.child(step_list);
        }
    }

    let log_panel = match selected {
        None => panel(k).padding(24.0).items_center().key("run-log")
            .child(text("Select a job to view its log.").font(k.ty.ui).color(k.c.text_muted)),
        Some(job) => {
            let job_id = job.id;
            let load = o.logs.get(&job_id);
            let mut tools = row().gap(6.0).center_y().flex_wrap();
            if let Some(Load::Ready(log)) = load {
                if log.truncated {
                    tools = tools.child(
                        text(format!("Last {} of {} lines", github::LOG_TAIL_LINES, log.total_lines))
                            .font(k.ty.text_small)
                            .color(k.c.text_faint)
                            .key("run-log-caption"),
                    );
                }
                if !log.text.is_empty() {
                    let copied = log.text.clone();
                    tools = tools.child(
                        Button::new("Copy log")
                            .icon(&icons::COPY)
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Small)
                            .on_press_tx(move |s: &mut App, tx| {
                                tx.copy(copied.clone());
                                s.notify("Log copied");
                            })
                            .build(k)
                            .key("run-copy-log"),
                    );
                }
            }
            if !job.url.is_empty() {
                tools = tools.child(pages::open_button(k, "Full log on GitHub", &job.url, "run-job-github"));
            }
            let body = match load {
                None | Some(Load::Loading) => col().w_full().padding(28.0).items_center().role("status").key("run-log-loading")
                    .child(text("Loading job log…").font(k.ty.ui).color(k.c.text_muted)),
                Some(Load::Failed(e)) => col().w_full().padding(24.0).items_center().gap(10.0).role("alert").key("run-log-failed")
                    .child(text(e.clone()).font(k.ty.small).color(k.c.text_base).wrap())
                    .child(Button::new("Retry").icon(&icons::REFRESH).control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.load_job_log(job_id)).build(k).key("run-log-retry")),
                Some(Load::Ready(log)) => {
                    let shown = if log.text.trim().is_empty() {
                        "(No log output for this job.)".to_owned()
                    } else {
                        log.text.clone()
                    };
                    col().w_full().padding(14.0).fill(k.c.layer_01)
                        .child(text(shown).font(k.ty.mono).color(k.c.text_base).wrap().selectable(true).key("run-log-text"))
                }
            };
            panel(k).key("run-log")
                .child(panel_head(k, &format!("Log · {}", job.name), Some(tools)))
                .child(rule(k))
                .child(body)
        }
    };

    if s.compact {
        page.child(jobs_panel).child(log_panel)
    } else {
        page.child(
            row().w_full().gap(16.0).flex_wrap()
                .child(col().grow(2.0).min_w(300.0).child(jobs_panel))
                .child(col().grow(3.0).min_w(360.0).child(log_panel)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run(id: u64, repo: &str, workflow: &str, checks: Checks) -> Run {
        Run {
            id,
            workflow: workflow.into(),
            repo: repo.into(),
            branch: "main".into(),
            checks,
            duration: "30s".into(),
            updated_at: 1_700_000_000,
            url: format!("https://github.com/{repo}/actions/runs/{id}"),
            pull_numbers: Vec::new(),
        }
    }

    #[test]
    fn runs_filter_combines_status_repo_and_workflow() {
        let a = sample_run(1, "acme/api", "CI", Checks::Failing);
        let b = sample_run(2, "acme/api", "Deploy", Checks::Running);
        let c = sample_run(3, "acme/web", "CI", Checks::Passing);

        let mut f = RunsFilter::default();
        assert!(f.is_empty());
        assert!(f.matches(&a) && f.matches(&b) && f.matches(&c));

        f.status = RunStatusFilter::Failed;
        assert!(f.matches(&a) && !f.matches(&b) && !f.matches(&c));

        f.status = RunStatusFilter::All;
        f.repo = Some("ACME/API".into());
        f.workflow = Some("deploy".into());
        assert!(!f.matches(&a) && f.matches(&b) && !f.matches(&c));
        assert!(!f.is_empty());

        let (repos, workflows) = choices([&a, &b, &c].into_iter());
        assert_eq!(repos, vec!["acme/api", "acme/web"]);
        assert_eq!(workflows, vec!["CI", "Deploy"]);

        f.clear();
        assert!(f.is_empty());
    }
}
