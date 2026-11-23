//! The sample releases: made-up releases, tags and generated notes for the sample repositories.

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

const SITE_120: &str = "The desktop shell grows widgets, and the bunny gets a scene of its own.

### Added
- **Widgets shelf**: clock, weather (cached for ten minutes), notes and a terminal.
- The procedural **Blue Bunny**, with softer ears and fewer polygons.
- Snap dragged windows to the screen edges.
- A 404 page that matches the shell.

### Fixed
- The dock no longer covers the footer on iPad.
- The clock widget no longer drifts after the computer sleeps.
- The window manager keeps the top window when the screen is resized.

### Changed
- Hero images are 420 KB smaller.
- System fonts show until the web font has loaded.
- The build uses plain ES modules; jQuery is gone.

**Full Changelog**: https://github.com/francescodibella/francescodibella.github.io/compare/v1.1.0...v1.2.0";

/// The sample releases and tags of `r`, newest first.
pub fn sample(r: &Repo, me: &str, now: i64) -> Listing {
    const DAY: i64 = 86_400;
    let rel = |id: u64, tag: &str, name: &str, body: &str, draft: bool, pre: bool, age: i64, assets: Vec<Asset>| Release {
        id,
        tag: tag.into(),
        name: name.into(),
        body: body.into(),
        draft,
        prerelease: pre,
        author: me.into(),
        created: now - age - 3600,
        published: (!draft).then_some(now - age),
        target: r.branch.clone(),
        url: if draft { format!("{}/releases/tag/untagged-{}", r.url, &sample_sha(tag)[..20]) } else { format!("{}/releases/tag/{tag}", r.url) },
        assets,
    };
    let file = |name: &str, size: u64, downloads: u64, tag: &str| Asset { name: name.into(), size, downloads, url: format!("{}/releases/download/{tag}/{name}", r.url) };
    let tag = |name: &str| Tag { name: name.into(), sha: sample_sha(&format!("{}@{name}", r.full_name)) };
    match r.name.as_str() {
        "francescodibella.github.io" => Listing {
            releases: vec![
                rel(3003, "v1.3.0", "Notes widget and window snapping", "### Added\n- The **notes widget**, saved in the browser.\n- Open the terminal widget with Ctrl+`.\n\n### Fixed\n- The bunny's poncho stays outside its arms when they're lowered (#31).", true, false, 2 * DAY, Vec::new()),
                rel(3002, "v1.3.0-rc.1", "v1.3.0 release candidate", "Try the notes widget before it ships. Notes are kept in this browser only.", false, true, 5 * DAY, vec![file("site-v1.3.0-rc.1.zip", 2_205_123, 12, "v1.3.0-rc.1")]),
                rel(3001, "v1.2.0", "Shell widgets and the procedural bunny", SITE_120, false, false, 18 * DAY, vec![
                    file("site-v1.2.0.zip", 2_093_517, 148, "v1.2.0"),
                    file("bunny.glb", 865_402, 61, "v1.2.0"),
                ]),
            ],
            tags: vec![tag("v1.3.0-rc.1"), tag("v1.2.0"), tag("v1.1.0"), tag("v1.0.0")],
            tags_more: false,
        },
        "ngsi-ld-xr-contexts" => Listing {
            releases: vec![rel(4001, "v0.1.0", "First contexts: AR and VR", "The first JSON-LD contexts for augmented and virtual reality entities, with an example for each.", false, false, 40 * DAY, vec![file("contexts-v0.1.0.zip", 48_211, 23, "v0.1.0")])],
            tags: vec![tag("v0.1.0")],
            tags_more: false,
        },
        _ => Listing { releases: Vec::new(), tags: Vec::new(), tags_more: false },
    }
}

/// The notes GitHub would generate for a sample release: the last commits, as pull requests.
pub(super) fn sample_notes(r: &Repo, me: &str, target: &str, tag: &str, previous: Option<&str>) -> String {
    let mut out = String::from("## What's Changed\n");
    for (n, c) in crate::commits::sample_history(r, target, me).iter().take(4).enumerate() {
        out.push_str(&format!("* {} by @{} in {}/pull/{}\n", c.title, c.author, r.url, 40 - n));
    }
    out.push_str(&match previous {
        Some(p) => format!("\n**Full Changelog**: {}/compare/{p}...{tag}", r.url),
        None => format!("\n**Full Changelog**: {}/commits/{tag}", r.url),
    });
    out
}
