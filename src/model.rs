//! The data the pages show, loaded from GitHub (see `github.rs`) or, without a configuration,
//! the built-in sample set.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PullState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Checks {
    Passing,
    Failing,
    Running,
}

#[derive(Clone, Debug, Default)]
pub struct Repo {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub language: String,
    pub hue: f32,
    pub stars: u32,
    pub forks: u32,
    pub issues: u32,
    /// "2 hours ago" text, used when `pushed == 0` (sample data).
    pub updated: String,
    /// When it was last pushed (Unix seconds), for ordering and formatting relative time.
    pub pushed: i64,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub branch: String,
    pub topics: Vec<String>,
    pub url: String,
    pub starred: bool,
}

impl Repo {
    pub fn updated_ago(&self, now: i64) -> String {
        ago_or(self.pushed, &self.updated, now)
    }
}

#[derive(Clone, Debug)]
pub struct Pull {
    pub number: u32,
    pub title: String,
    pub repo: String,
    pub author: String,
    pub state: PullState,
    pub checks: Option<Checks>,
    pub age: String,
    pub updated_at: i64,
    pub comments: u32,
    pub url: String,
}

impl Pull {
    pub fn age_ago(&self, now: i64) -> String {
        ago_or(self.updated_at, &self.age, now)
    }
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub number: u32,
    pub title: String,
    pub repo: String,
    /// First label and its hue.
    pub label: Option<(String, f32)>,
    pub age: String,
    pub created_at: i64,
    pub comments: u32,
    pub url: String,
}

impl Issue {
    pub fn age_ago(&self, now: i64) -> String {
        ago_or(self.created_at, &self.age, now)
    }
}

#[derive(Clone, Debug)]
pub struct Run {
    pub workflow: String,
    pub repo: String,
    pub branch: String,
    pub checks: Checks,
    pub duration: String,
    pub age: String,
    pub updated_at: i64,
    pub url: String,
}

impl Run {
    pub fn age_ago(&self, now: i64) -> String {
        if self.checks == Checks::Running {
            "running".into()
        } else {
            ago_or(self.updated_at, &self.age, now)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub login: String,
    pub name: String,
    pub bio: String,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub author: String,
    pub message: String,
    pub sha: String,
    pub age: String,
    pub committed_at: i64,
}

impl Commit {
    pub fn age_ago(&self, now: i64) -> String {
        ago_or(self.committed_at, &self.age, now)
    }
}

/// What a repository's Code tab shows; loaded when the repository is opened.
#[derive(Clone, Debug, Default)]
pub struct Detail {
    /// Something couldn't be loaded; what could is still here.
    pub error: Option<String>,
    /// (name, is directory)
    pub files: Vec<(String, bool)>,
    pub readme: Option<String>,
    /// (language, bytes), largest first.
    pub languages: Vec<(String, u64)>,
    pub commit: Option<Commit>,
}

#[derive(Clone, Debug, Default)]
pub struct Data {
    pub profile: Profile,
    pub repos: Vec<Repo>,
    pub pulls: Vec<Pull>,
    pub issues: Vec<Issue>,
    pub runs: Vec<Run>,
    /// Commits per week, oldest first.
    pub activity: Vec<u32>,
    /// True for the built-in sample data.
    pub sample: bool,
}

/// A stable hue for a language name, close to GitHub's colours for the common ones.
pub fn lang_hue(name: &str) -> f32 {
    match name {
        "Rust" => 40.0,
        "JavaScript" => 95.0,
        "TypeScript" => 235.0,
        "Python" => 255.0,
        "HTML" => 30.0,
        "CSS" => 295.0,
        "Shell" => 140.0,
        "Go" => 205.0,
        "Java" => 60.0,
        "Kotlin" => 300.0,
        "Swift" => 35.0,
        "C" => 260.0,
        "C++" => 340.0,
        "C#" => 150.0,
        "PHP" => 275.0,
        "Ruby" => 20.0,
        "Dart" => 200.0,
        "Jupyter Notebook" => 50.0,
        _ => (name.bytes().fold(11u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 360) as f32,
    }
}

/// The hue of a `rrggbb` label colour.
pub fn hex_hue(hex: &str) -> f32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x888888);
    let (r, g, b) = (((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d < 1e-4 {
        return 250.0;
    }
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    h * 60.0
}

/// Seconds since the Unix epoch for an ISO 8601 UTC timestamp (`2026-11-09T09:35:11Z`).
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

/// "3 hours ago", "yesterday", "2 weeks ago"… for a timestamp, relative to `now`.
pub fn ago(ts: i64, now: i64) -> String {
    let s = (now - ts).max(0);
    let (m, h, d) = (s / 60, s / 3600, s / 86400);
    match () {
        _ if s < 60 => "just now".into(),
        _ if m < 60 => if m == 1 { "1 minute ago".into() } else { format!("{m} minutes ago") },
        _ if h < 24 => if h == 1 { "1 hour ago".into() } else { format!("{h} hours ago") },
        _ if d == 1 => "yesterday".into(),
        _ if d < 7 => format!("{d} days ago"),
        _ if d < 30 => if d / 7 == 1 { "1 week ago".into() } else { format!("{} weeks ago", d / 7) },
        _ if d < 365 => if d / 30 == 1 { "1 month ago".into() } else { format!("{} months ago", d / 30) },
        _ => if d / 365 == 1 { "1 year ago".into() } else { format!("{} years ago", d / 365) },
    }
}

/// Formats `ts` relative to `now`, or falls back to `fallback` when `ts <= 0` (sample data).
pub fn ago_or(ts: i64, fallback: &str, now: i64) -> String {
    if ts > 0 { ago(ts, now) } else { fallback.to_owned() }
}

/// Search listings in `github.rs` ask for at most this many items.
pub const SEARCH_CAP: usize = 40;

/// Formats a count from a capped search list ("40+" once it hits [`SEARCH_CAP`]).
pub fn capped(n: usize) -> String {
    if n >= SEARCH_CAP { format!("{SEARCH_CAP}+") } else { n.to_string() }
}

/// "41s", "3m 05s", "1h 02m".
pub fn duration(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// Sample files for the sample repository's Code tab.
pub fn sample_detail(r: &Repo) -> Detail {
    let files = [(".github", true), ("bunny", true), ("css", true), ("js", true), ("repohub", true), ("README.md", false), ("index.html", false)];
    Detail {
        error: None,
        files: files.iter().map(|(n, d)| (n.to_string(), *d)).collect(),
        readme: Some(format!("# {}\n\n{}\n\n## Quick start\n\n```sh\ngit clone {}.git\ncd {}\n```\n\nSee the [contributing guide](CONTRIBUTING.md) for the details.", r.name, r.description, r.url, r.name)),
        languages: vec![(r.language.clone(), 70), ("HTML".into(), 20), ("CSS".into(), 10)],
        commit: Some(Commit { author: r.owner.clone(), message: "Add RepoHub, a Rune-based GitHub repository manager UI".into(), sha: "a6f6543".into(), age: "2 hours ago".into(), committed_at: 0 }),
    }
}

impl Data {
    /// Nothing worth remembering a place in: the sample data, or nothing loaded yet.
    pub fn sample_or_empty(&self) -> bool {
        self.sample || self.repos.is_empty()
    }

    /// Built-in sample data, shown when no GitHub configuration is present. Everything is made up.
    pub fn sample() -> Data {
        let repo_of = |owner: &str, name: &str, desc: &str, lang: &str, stars, forks, issues, updated: &str, private, fork, branch: &str, topics: &[&str], starred| Repo {
            full_name: format!("{owner}/{name}"),
            owner: owner.into(),
            name: name.into(),
            description: desc.into(),
            language: lang.into(),
            hue: lang_hue(lang),
            stars,
            forks,
            issues,
            updated: updated.into(),
            pushed: 0,
            private,
            fork,
            archived: false,
            branch: branch.into(),
            topics: topics.iter().map(|t| t.to_string()).collect(),
            url: format!("https://github.com/{owner}/{name}"),
            starred,
        };
        let repo = |name: &str, desc: &str, lang: &str, stars, forks, issues, updated: &str, private, fork, branch: &str, topics: &[&str], starred|
            repo_of("francescodibella", name, desc, lang, stars, forks, issues, updated, private, fork, branch, topics, starred);
        let repos = vec![
            repo("francescodibella.github.io", "Personal site and playground: shell UI, widgets and a procedural Three.js bunny.", "JavaScript", 12, 2, 3, "2 hours ago", false, false, "main", &["github-pages", "threejs", "portfolio"], false),
            repo("ngsi-ld-xr-contexts", "JSON-LD contexts for AR, VR and MR entities in NGSI-LD smart data models.", "JSON-LD", 41, 9, 5, "yesterday", false, false, "main", &["fiware", "ngsi-ld", "xr"], true),
            repo("xr-digital-twin-thesis", "Master thesis: a digital twin of an industrial cell streamed to a WebXR headset.", "TypeScript", 7, 0, 11, "3 days ago", true, false, "dev", &["webxr", "digital-twin"], false),
            repo_of("xr-lab", "spatial-anchors", "Shared spatial anchors for multi-user WebXR sessions (organization repository).", "TypeScript", 64, 12, 8, "4 days ago", false, false, "main", &["webxr", "anchors"], false),
            repo("dotfiles", "Shell, editor and terminal configuration, bootstrapped with one script.", "Shell", 3, 1, 0, "last week", false, false, "main", &["zsh", "neovim"], false),
            repo("rune", "A native GPU-rendered Rust UI framework for macOS, Windows, Linux and the web.", "Rust", 0, 0, 0, "2 weeks ago", false, true, "dev", &["rust", "gpu", "ui"], false),
            repo_of("xr-lab", "lab-handbook", "How the lab works: onboarding, hardware booking, publishing (organization repository).", "Markdown", 5, 3, 2, "3 weeks ago", true, false, "main", &["docs"], false),
            repo("iot-sensor-gateway", "MQTT to NGSI-LD bridge for low-power environmental sensors.", "Python", 18, 4, 2, "last month", true, false, "main", &["mqtt", "iot", "fiware"], false),
            repo("university-notes", "Lecture notes in Markdown, compiled to a searchable static site.", "Markdown", 25, 6, 1, "2 months ago", false, false, "main", &["notes", "mkdocs"], false),
        ];
        let pull = |number, title: &str, repo: &str, author: &str, state, checks, age: &str, comments| Pull {
            number, title: title.into(), repo: repo.into(), author: author.into(), state, checks: Some(checks), age: age.into(), updated_at: 0, comments,
            url: format!("https://github.com/francescodibella/{repo}/pull/{number}"),
        };
        let pulls = vec![
            pull(14, "Add procedural Three.js Blue Bunny model", "francescodibella.github.io", "claude", PullState::Open, Checks::Passing, "2 hours ago", 3),
            pull(9, "Mixed reality context v0.2: anchors and spatial meshes", "ngsi-ld-xr-contexts", "francescodibella", PullState::Draft, Checks::Running, "yesterday", 1),
            pull(8, "Fix @vocab for VR entity relationships", "ngsi-ld-xr-contexts", "contributor42", PullState::Open, Checks::Failing, "4 days ago", 6),
            pull(3, "Repo manager example on the web target", "rune", "francescodibella", PullState::Open, Checks::Passing, "2 weeks ago", 0),
            pull(21, "Chapter 4: latency measurements", "xr-digital-twin-thesis", "francescodibella", PullState::Merged, Checks::Passing, "3 weeks ago", 12),
        ];
        let issue = |number, title: &str, repo: &str, label: &str, hue, age: &str| Issue {
            number, title: title.into(), repo: repo.into(), label: Some((label.into(), hue)), age: age.into(), created_at: 0, comments: 0,
            url: format!("https://github.com/francescodibella/{repo}/issues/{number}"),
        };
        let issues = vec![
            issue(31, "Bunny poncho clips through lowered arms", "francescodibella.github.io", "bug", 25.0, "1 hour ago"),
            issue(12, "Document the MR context with examples", "ngsi-ld-xr-contexts", "docs", 240.0, "2 days ago"),
            issue(44, "Measure headset frame pacing over Wi-Fi 6", "xr-digital-twin-thesis", "research", 300.0, "5 days ago"),
            issue(7, "Battery drain on the CO2 sensor node", "iot-sensor-gateway", "bug", 25.0, "1 week ago"),
        ];
        let run = |workflow: &str, repo: &str, branch: &str, checks, duration: &str, age: &str| Run {
            workflow: workflow.into(), repo: repo.into(), branch: branch.into(), checks, duration: duration.into(), age: age.into(), updated_at: 0,
            url: format!("https://github.com/francescodibella/{repo}/actions"),
        };
        let runs = vec![
            run("pages-build-deployment", "francescodibella.github.io", "main", Checks::Passing, "41s", "2 hours ago"),
            run("validate-jsonld", "ngsi-ld-xr-contexts", "mr-v0.2", Checks::Running, "1m 12s", "running"),
            run("validate-jsonld", "ngsi-ld-xr-contexts", "fix-vocab", Checks::Failing, "38s", "4 days ago"),
            run("latex-build", "xr-digital-twin-thesis", "dev", Checks::Passing, "3m 05s", "3 days ago"),
            run("pytest", "iot-sensor-gateway", "main", Checks::Passing, "57s", "last month"),
        ];
        Data {
            profile: Profile { login: "francescodibella".into(), name: "Francesco Di Bella".into(), bio: "XR & IoT developer".into() },
            repos,
            pulls,
            issues,
            runs,
            activity: vec![2, 4, 1, 0, 3, 6, 8, 5, 2, 0, 1, 4, 7, 9, 6, 3, 2, 5, 8, 12, 10, 6, 4, 7, 11, 9],
            sample: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_ago() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-11-09T09:35:11Z"), Some(1_794_216_911));
        let t = parse_iso("2026-11-09T09:35:11Z").unwrap();
        assert_eq!(ago(t - 30, t), "just now");
        assert_eq!(ago(t - 7200, t), "2 hours ago");
        assert_eq!(ago(t - 86400 - 10, t), "yesterday");
        assert_eq!(ago(t - 15 * 86400, t), "2 weeks ago");
        assert_eq!(duration(185), "3m 05s");
    }

    #[test]
    fn stored_timestamps_follow_now() {
        let t = 1_794_216_911;
        let repo = Repo { pushed: t, updated: "frozen".into(), ..Repo::default() };
        assert_eq!(repo.updated_ago(t + 10), "just now");
        assert_eq!(repo.updated_ago(t + 30 * 60), "30 minutes ago");
        assert_eq!(repo.updated_ago(t + 2 * 3600), "2 hours ago");

        let issue = Issue {
            number: 1, title: "Bug".into(), repo: "a/b".into(), label: None,
            age: "just now".into(), created_at: t, comments: 0, url: String::new(),
        };
        assert_eq!(issue.age_ago(t), "just now");
        assert_eq!(issue.age_ago(t + 5 * 60), "5 minutes ago");

        let pull = Pull {
            number: 2, title: "Fix".into(), repo: "a/b".into(), author: "u".into(),
            state: PullState::Open, checks: None, age: "just now".into(),
            updated_at: t, comments: 0, url: String::new(),
        };
        assert_eq!(pull.age_ago(t + 3600), "1 hour ago");

        let run = Run {
            workflow: "ci".into(), repo: "a/b".into(), branch: "main".into(),
            checks: Checks::Passing, duration: "10s".into(), age: String::new(),
            updated_at: t, url: String::new(),
        };
        assert_eq!(run.age_ago(t + 120), "2 minutes ago");
        let running = Run { checks: Checks::Running, ..run };
        assert_eq!(running.age_ago(t + 120), "running");

        let commit = Commit {
            author: "u".into(), message: "msg".into(), sha: "abc".into(),
            age: "2 hours ago".into(), committed_at: t,
        };
        assert_eq!(commit.age_ago(t + 180), "3 minutes ago");

        let sample = Data::sample();
        assert_eq!(sample.repos[0].updated_ago(t), "2 hours ago");
        assert_eq!(sample.issues[0].age_ago(t), "1 hour ago");
    }

    #[test]
    fn capped_counts() {
        assert_eq!(capped(0), "0");
        assert_eq!(capped(39), "39");
        assert_eq!(capped(40), "40+");
        assert_eq!(capped(55), "40+");
    }

    #[test]
    fn label_hue() {
        assert!((hex_hue("d73a4a") - 354.0).abs() < 2.0);
        assert!((hex_hue("0075ca") - 205.0).abs() < 2.0);
    }
}
