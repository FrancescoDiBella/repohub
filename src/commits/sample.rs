//! The sample history: made-up commits, a commit page and a comparison for the sample repositories.

use super::*;

// ------------------------------------------------------------------ sample data

/// A made-up commit id, the same every time for the same words.
fn sample_sha(seed: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut out = String::with_capacity(40);
    for round in 0..3u64 {
        for b in seed.bytes().chain(round.to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        out.push_str(&format!("{h:016x}"));
    }
    out.truncate(40);
    out
}

/// Who wrote a sample commit: you, Claude, a collaborator, someone with no GitHub account, a bot.
#[derive(Clone, Copy)]
enum Who {
    Me,
    Claude,
    Contributor,
    GitName,
    Bot,
}

/// The sample history of `name`, newest first: (message, who, body).
fn sample_messages(name: &str) -> &'static [(&'static str, Who, &'static str)] {
    use Who::*;
    match name {
        "francescodibella.github.io" => &[
            ("Add RepoHub, a Rune-based GitHub repository manager UI", Me, "RepoHub reads the repositories, pull requests and issues from the GitHub API\nand shows them in a native window. The page links to the desktop builds."),
            ("Bunny: keep the poncho outside the arms when they're lowered", Claude, "The poncho's lower edge followed the torso only, so the arms went through it\nwhen they hung down. It now follows the shoulders too.\n\nFixes #31"),
            ("Frame the camera on small screens", Me, ""),
            ("Load the bunny scene in chunks so the first frame shows sooner", Me, "The geometry loads first and the materials after it, so the page shows\nsomething within a second on a slow connection."),
            ("Shell: add the widgets shelf", Me, ""),
            ("Follow the system's dark mode", GitName, ""),
            ("Compress the hero images (420 KB less)", Me, ""),
            ("Fix the dock covering the footer on iPad", Me, ""),
            ("Open the terminal widget with Ctrl+`", Me, ""),
            ("Show system fonts until the web font has loaded", GitName, ""),
            ("Bump three from 0.164.1 to 0.170.0", Bot, "Bumps three from 0.164.1 to 0.170.0.\n\nRelease notes: https://github.com/mrdoob/three.js/releases"),
            ("Weather widget: cache the forecast for ten minutes", Me, ""),
            ("Add a 404 page that matches the shell", Me, ""),
            ("Fix the clock widget drifting after sleep", Me, ""),
            ("Bunny: softer ears, fewer polygons", Claude, ""),
            ("Move the shell's styles into css/shell.css", Me, ""),
            ("Add Open Graph tags for link previews", Me, ""),
            ("Keyboard focus rings on every dock icon", GitName, ""),
            ("Fix the window manager losing the top window on resize", Me, ""),
            ("Snap dragged windows to the screen edges", Me, ""),
            ("README: how to run the site locally", Me, ""),
            ("Add the notes widget, saved in localStorage", Me, ""),
            ("Lazy-load the project screenshots", Me, ""),
            ("Fix the bunny's shadow on Safari", Claude, ""),
            ("Add a reduced-motion mode for the boot animation", Me, ""),
            ("Projects page: cards with tags and links", Me, ""),
            ("Switch the build to plain ES modules", Me, ""),
            ("Remove jQuery", Me, ""),
            ("Add the boot animation", Me, ""),
            ("Shell UI: desktop, dock and windows", Me, ""),
            ("Add the procedural bunny (first version)", Me, ""),
            ("Deploy with GitHub Pages", Me, ""),
            ("About page and contact links", Me, ""),
            ("Initial commit", Me, ""),
        ],
        "ngsi-ld-xr-contexts" => &[
            ("Mixed reality context: anchors and spatial meshes", Me, "Adds SpatialAnchor and SpatialMesh, with examples for both."),
            ("Fix @vocab for VR entity relationships", Contributor, ""),
            ("Validate every example in CI", Me, ""),
            ("Add the AR marker entity", Me, ""),
            ("Document the VR scene graph", GitName, ""),
            ("Rename hasPose to pose (the old name stays as an alias)", Me, ""),
            ("Add JSON-LD examples for each entity", Me, ""),
            ("Split the context into ar, vr and mr files", Me, ""),
            ("Add the validate-jsonld workflow", Me, ""),
            ("Initial AR and VR contexts", Me, ""),
        ],
        "xr-digital-twin-thesis" => &[
            ("Chapter 4: latency measurements", Me, "Frame pacing and end-to-end latency over Wi-Fi 5, Wi-Fi 6 and Ethernet."),
            ("Plot frame pacing per network", Me, ""),
            ("Stream the cell's state over WebRTC data channels", Me, ""),
            ("Fix the robot arm's joint order", Me, ""),
            ("Chapter 3: architecture", Me, ""),
            ("Add the OPC UA bridge", Me, ""),
            ("Interpolate poses between updates", Me, ""),
            ("Bibliography: add the WebXR papers", Me, ""),
            ("Set up the LaTeX build", Me, ""),
            ("Initial project", Me, ""),
        ],
        "spatial-anchors" => &[
            ("Share anchors between sessions through the relay", Me, ""),
            ("Persist anchors in IndexedDB", Contributor, ""),
            ("Fix drift after the headset sleeps", Me, ""),
            ("Add the anchor inspector overlay", GitName, ""),
            ("Resolve anchors by id on join", Me, ""),
            ("Add tests for the relay protocol", Contributor, ""),
            ("Use WebXR's anchors module when available", Me, ""),
            ("Initial relay and client", Me, ""),
        ],
        "dotfiles" => &[
            ("zsh: faster prompt with async git status", Me, ""),
            ("neovim: switch to lazy.nvim", Me, ""),
            ("Add the bootstrap script for macOS", Me, ""),
            ("tmux: true colour and mouse support", Me, ""),
            ("git: sign commits with SSH keys", Me, ""),
            ("Add aliases for docker compose", Me, ""),
            ("Initial dotfiles", Me, ""),
        ],
        "rune" => &[
            ("Repo manager example on the web target", Me, ""),
            ("Merge upstream dev", Me, ""),
            ("Fix the example's scrolling on touch screens", Me, ""),
            ("Add a README for the example", Me, ""),
            ("Merge upstream dev", Me, ""),
        ],
        "lab-handbook" => &[
            ("Booking: the headsets are now in room 2.14", GitName, ""),
            ("Add the publishing checklist", Me, ""),
            ("Onboarding: accounts to ask for in the first week", Contributor, ""),
            ("Fix broken links to the wiki", Me, ""),
            ("Initial handbook", Me, ""),
        ],
        "iot-sensor-gateway" => &[
            ("Batch MQTT messages to save battery", Me, "The CO2 node now sends every 5 minutes instead of every 30 seconds.\n\nRefs #7"),
            ("Map CO2 readings to NGSI-LD AirQualityObserved", Me, ""),
            ("Reconnect to the broker with backoff", Me, ""),
            ("Add pytest for the mapper", Me, ""),
            ("Read the configuration from environment variables", Me, ""),
            ("Initial MQTT bridge", Me, ""),
        ],
        "university-notes" => &[
            ("Distributed systems: Raft and Paxos", Me, ""),
            ("Fix the search index for accented words", Me, ""),
            ("Add MathJax", Me, ""),
            ("Computer graphics: shading", Me, ""),
            ("Switch to mkdocs-material", Me, ""),
            ("Initial notes", Me, ""),
        ],
        _ => &[("Update the README", Me, ""), ("Fix a typo", Me, ""), ("Initial commit", Me, "")],
    }
}

/// A believable history of sample repository `r` at `branch`, newest first: the newest is
/// the last commit the Code tab shows.
pub fn sample_history(r: &Repo, branch: &str, me: &str) -> Vec<Row> {
    let mut at = r.pushed;
    sample_messages(&r.name).iter().enumerate().map(|(n, (msg, who, body))| {
        let sha = sample_sha(&format!("{}@{branch}#{n}", r.full_name));
        let (author, account) = match who {
            Who::Me => (me.to_owned(), true),
            Who::Claude => ("claude".to_owned(), true),
            Who::Contributor => ("contributor42".to_owned(), true),
            Who::GitName => ("Laura Bianchi".to_owned(), false),
            Who::Bot => ("dependabot[bot]".to_owned(), true),
        };
        let row = Row {
            title: (*msg).to_owned(),
            body: (*body).to_owned(),
            verified: matches!(who, Who::Me | Who::Bot) && n % 3 != 2,
            author,
            account,
            at,
            sha,
        };
        // A few hours to a few days between commits.
        at -= 3600 * (3 + (u64::from_str_radix(&row.sha[..4], 16).unwrap_or(0) % 40) as i64);
        row
    }).collect()
}

/// The changes of a sample commit.
fn sample_files(r: &Repo, title: &str) -> Vec<PullFile> {
    let readme = PullFile {
        path: "README.md".into(),
        previous: None,
        status: "modified".into(),
        patch: Some(format!("@@ -1,6 +1,8 @@\n # {}\n \n-{}\n+{}\n+\n+Latest change: {title}.\n \n ## Quick start\n ", r.name, r.description, r.description.trim_end_matches('.'))),
    };
    if r.name == "francescodibella.github.io" {
        let mut files = diffs::sample_pull_files();
        files.push(PullFile { path: "assets/bunny-preview.png".into(), previous: None, status: "added".into(), patch: None });
        files.push(readme);
        files
    } else {
        vec![readme]
    }
}

/// Lines added and removed by the files that have a patch (as GitHub counts them).
fn sample_totals(files: &[PullFile]) -> (usize, usize) {
    let with_patch: Vec<PullFile> = files.iter().filter(|f| f.patch.is_some()).cloned().collect();
    diffs::totals(&diffs::pull_files(&with_patch))
}

/// Sample commit `sha` of `r`: the row of its history, with made-up changes.
pub fn sample_detail(r: &Repo, sha: &str, me: &str) -> Option<Detail> {
    let rows = sample_history(r, &r.branch, me);
    let at = rows.iter().position(|x| x.sha.starts_with(sha) || sha.starts_with(x.short()))?;
    Some(sample_detail_of(r, &rows[at], rows.get(at + 1).map(|p| p.sha.clone())))
}

/// A sample commit's page from its row: made-up changes, and `parent`.
pub fn sample_detail_of(r: &Repo, row: &Row, parent: Option<String>) -> Detail {
    let files = sample_files(r, &row.title);
    let (add, del) = sample_totals(&files);
    let who = Person { name: row.author.clone(), login: row.account.then(|| row.author.clone()), at: row.at };
    Detail {
        sha: row.sha.clone(),
        title: row.title.clone(),
        body: row.body.clone(),
        committer: if row.author == "dependabot[bot]" { Person { name: "GitHub".into(), login: Some("web-flow".into()), at: row.at } } else { who.clone() },
        author: who,
        parents: parent.into_iter().collect(),
        verified: row.verified,
        files,
        more_files: false,
        additions: add as u32,
        deletions: del as u32,
        url: format!("{}/commit/{}", r.url, row.sha),
    }
}

/// A sample comparison of `head` with `base`.
pub fn sample_comparison(r: &Repo, base: &str, head: &str, me: &str, now: i64) -> Comparison {
    if base == head {
        return Comparison { url: format!("{}/compare/{base}...{head}", r.url), ..Comparison::unrelated(base, head) }.identical();
    }
    let words = head.rsplit('/').next().unwrap_or(head).replace(['-', '_'], " ");
    let titles = [format!("Start {words}"), format!("{}: handle the empty case", capitalize(&words)), format!("Tests for {words}")];
    // The first title is the oldest commit; the list shows the newest first.
    let commits: Vec<Row> = titles.iter().enumerate().rev().map(|(n, t)| Row {
        sha: sample_sha(&format!("{}:{head}:{n}", r.full_name)),
        title: t.clone(),
        body: String::new(),
        author: me.to_owned(),
        account: true,
        at: now - 3600 * (2 + 20 * (titles.len() - 1 - n) as i64),
        verified: n != 1,
    }).collect();
    let files = sample_files(r, &titles[0]);
    let (additions, deletions) = sample_totals(&files);
    Comparison {
        base: base.to_owned(),
        head: head.to_owned(),
        standing: Standing::Diverged,
        ahead: commits.len() as u32,
        behind: 1,
        commits,
        more_commits: false,
        files,
        more_files: false,
        additions: additions as u32,
        deletions: deletions as u32,
        url: format!("{}/compare/{base}...{head}", r.url),
        open_pull: None,
    }
}

impl Comparison {
    fn identical(mut self) -> Comparison {
        self.standing = Standing::Identical;
        self
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
