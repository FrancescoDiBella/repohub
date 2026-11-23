use super::*;
use serde_json::json;

fn listing(tags: &[&str], releases: Vec<Release>) -> Listing {
    Listing { releases, tags: tags.iter().map(|t| Tag { name: (*t).into(), sha: String::new() }).collect(), tags_more: false }
}

fn release(id: u64, tag: &str, draft: bool, pre: bool, published: Option<i64>) -> Release {
    Release { id, tag: tag.into(), name: String::new(), body: String::new(), draft, prerelease: pre, author: "ada".into(), created: 1, published, target: "main".into(), url: String::new(), assets: Vec::new() }
}

#[test]
fn releases_read_their_state_author_dates_and_files() {
    let v = json!([
        {
            "id": 101, "tag_name": "v0.2.0", "name": "Second", "body": "### Added\r\n- Releases", "draft": false, "prerelease": false,
            "author": { "login": "ada" }, "created_at": "2026-11-20T10:00:00Z", "published_at": "2026-11-21T09:00:00Z",
            "target_commitish": "main", "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
            "zipball_url": "https://api.github.com/repos/o/r/zipball/v0.2.0",
            "assets": [
                { "name": "app.tar.gz", "size": 2_093_517, "download_count": 148, "browser_download_url": "https://github.com/o/r/releases/download/v0.2.0/app.tar.gz" },
                { "name": "", "size": 1 },
            ],
        },
        { "id": 102, "tag_name": "v0.3.0", "name": "", "draft": true, "prerelease": false, "author": null, "created_at": "2026-11-22T08:00:00Z", "published_at": null },
        { "id": 0, "tag_name": "broken" },
        { "id": 103, "tag_name": "" },
    ]);
    let list = parse_releases(&v);
    assert_eq!(list.len(), 2, "an entry without an id or a tag is left out");
    let r = &list[0];
    assert_eq!((r.id, r.tag.as_str(), r.title(), r.body.as_str()), (101, "v0.2.0", "Second", "### Added\n- Releases"));
    assert_eq!((r.author.as_str(), r.at()), ("ada", parse_iso("2026-11-21T09:00:00Z").unwrap()), "published: its publication date");
    assert_eq!(r.assets.len(), 1, "a nameless file is left out, and the source archives aren't files");
    assert_eq!((r.assets[0].size, r.assets[0].downloads), (2_093_517, 148));
    let d = &list[1];
    assert_eq!((d.title(), d.author.as_str(), d.published, d.at()), ("v0.3.0", "unknown", None, parse_iso("2026-11-22T08:00:00Z").unwrap()), "a draft: the tag for a title, when it was drafted");
    assert_eq!(kind(r, Some(101)), Kind::Latest);
    assert_eq!(kind(r, Some(7)), Kind::Published);
    assert_eq!(kind(d, Some(102)), Kind::Draft, "a draft is never the latest");
    assert_eq!(kind(&release(5, "v1-rc", false, true, Some(1)), None), Kind::Pre);
    assert_eq!((Kind::Latest.word(), Kind::Pre.word(), Kind::Draft.word(), Kind::Published.word()), (Some("Latest"), Some("Pre-release"), Some("Draft"), None));
    assert_eq!([size(12), size(865_402), size(2_093_517), size(48 * 1_048_576), size(3 * 1_073_741_824)], ["12 B", "845 KB", "2.0 MB", "48 MB", "3.0 GB"]);
    assert_eq!([downloads(1), downloads(148)], ["1 download", "148 downloads"]);
}

#[test]
fn tags_without_a_release_leave_out_drafts_and_published_ones() {
    let tags = parse_tags(&json!([
        { "name": "v1.3.0", "commit": { "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } },
        { "name": "v1.2.0", "commit": { "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" } },
        { "name": "v1.1.0", "commit": { "sha": "cccccccccccccccccccccccccccccccccccccccc" } },
        { "name": "", "commit": {} },
    ]));
    assert_eq!(tags.len(), 3);
    assert_eq!(tags[1].sha, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    let releases = vec![release(1, "v1.2.0", false, false, Some(10)), release(2, "v1.3.0", true, false, None)];
    let left: Vec<&str> = without_release(&tags, &releases).iter().map(|t| t.name.as_str()).collect();
    assert_eq!(left, ["v1.1.0"], "a tag with a release, or a draft, has one");
    assert_eq!(without_release(&tags, &[]).len(), 3);
    // The latest, when GitHub hasn't said: the newest published release that isn't a pre-release.
    let all = vec![release(3, "v2.0.0-rc.1", false, true, Some(30)), release(1, "v1.2.0", false, false, Some(10)), release(4, "v1.2.1", false, false, Some(20)), release(2, "v1.3.0", true, false, None)];
    assert_eq!(latest_of(&all).map(|r| r.id), Some(4));
    assert_eq!(latest_of(&[]), None);
}

#[test]
fn the_next_versions_follow_the_newest_version_tag() {
    assert_eq!(next_versions(["v0.1.0"]), ["v0.1.1", "v0.2.0", "v1.0.0"]);
    assert_eq!(next_versions(["1.4.2"]), ["1.4.3", "1.5.0", "2.0.0"], "a tag without v keeps none");
    assert_eq!(next_versions([]), ["v0.1.0"], "no version tag");
    assert_eq!(next_versions(["latest", "nightly", "v1", "release-2"]), ["v0.1.0"], "nothing that looks like a version");
    // The newest by version, not the first listed; v0.10.0 is newer than v0.9.0.
    assert_eq!(next_versions(["v0.9.0", "latest", "v0.10.0", "v0.2.5"]), ["v0.10.1", "v0.11.0", "v1.0.0"]);
    // A pre-release is followed by its release; it's older than the release itself.
    assert_eq!(next_versions(["v1.2.0", "v1.3.0-rc.1"]), ["v1.3.0", "v1.4.0", "v2.0.0"]);
    assert_eq!(next_versions(["v1.3.0", "v1.3.0-rc.1"]), ["v1.3.1", "v1.4.0", "v2.0.0"]);
    assert_eq!(next_versions(["vv1.0.0", "v-1.0.0"]), ["v0.1.0"], "not versions");
}

#[test]
fn a_tag_is_checked_like_a_branch_name_and_against_the_releases_read() {
    for ok in ["v1.2.3", "1.0.0", "release/2026-11", "v1.3.0-rc.1"] {
        assert_eq!(tag_problem(ok), None, "{ok}");
    }
    for bad in ["", "  ", "v1 2", "-v1", "v1..2", "v1~1", "v1^", "a:b", "/v1", "v1/", "v1.lock", "v1?", "v*", "v[1]"] {
        assert!(tag_problem(bad).is_some(), "{bad}");
    }
    let l = listing(&["v1.2.0", "v1.1.0"], vec![release(1, "v1.2.0", false, false, Some(10)), release(2, "v1.3.0", true, false, None)]);
    assert_eq!(tag_use("v1.1.0", Some(&l), None), TagUse::Existing, "a tag without a release");
    assert_eq!(tag_use("v1.4.0", Some(&l), None), TagUse::New);
    assert_eq!(tag_use("v1.2.0", Some(&l), None), TagUse::Taken("v1.2.0".into(), false));
    assert_eq!(tag_use("v1.3.0", Some(&l), None), TagUse::Taken("v1.3.0".into(), true));
    assert_eq!(tag_use("v1.3.0", Some(&l), Some(2)), TagUse::New, "the draft being edited keeps its own tag, not made yet");
    assert_eq!(tag_use("v1.4.0", None, None), TagUse::New, "nothing read yet: GitHub decides");
    assert!(matches!(tag_use("bad tag", Some(&l), None), TagUse::Invalid(_)));
    // The form: the title defaults to the tag, a publish is not a draft, and what's wrong is said.
    let mut f = Form::new("o/r", "main");
    assert!(f.release(false, Some(&l)).unwrap_err().contains("Give the release a tag"));
    f.tag = TextEdit::new("v1.2.0");
    assert!(f.release(true, Some(&l)).unwrap_err().contains("already uses v1.2.0"));
    f.tag = TextEdit::new("v1.3.0");
    assert!(f.release(true, Some(&l)).unwrap_err().contains("edit that one"));
    f.tag = TextEdit::new(" v1.4.0 ");
    let n = f.release(false, Some(&l)).unwrap();
    assert_eq!((n.tag.as_str(), n.name.as_str(), n.target.as_str(), n.draft), ("v1.4.0", "v1.4.0", "main", true));
    f.title = TextEdit::new("Releases");
    f.prerelease = true;
    let n = f.release(true, Some(&l)).unwrap();
    assert_eq!((n.name.as_str(), n.draft, n.prerelease), ("Releases", false, true));
    f.target = "bad branch".into();
    assert!(f.release(true, Some(&l)).is_err(), "a new tag needs a branch to start from");
    f.tag = TextEdit::new("v1.1.0");
    assert!(f.release(true, Some(&l)).is_ok(), "an existing tag needs none");
    assert_eq!(publish_question("v0.2.0", "main", false), "Publish v0.2.0 from main? This creates the tag and can't be undone from RepoHub.");
    assert!(publish_question("v1.1.0", "main", true).contains("existing tag"));
}

#[test]
fn long_notes_are_cut_and_a_code_block_is_closed() {
    let short = "a\nb\nc";
    assert_eq!(collapsed(short), None);
    let long: String = (1..=30).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let cut = collapsed(&long).unwrap();
    assert_eq!(cut.lines().count(), NOTES_LINES);
    assert!(cut.ends_with("line 12"));
    let fenced = format!("intro\n```\n{}\n```", (1..=20).map(|n| format!("code {n}")).collect::<Vec<_>>().join("\n"));
    assert!(collapsed(&fenced).unwrap().ends_with("\n```"), "the fence opened above the cut is closed");
    assert_eq!(collapsed(&(1..=NOTES_LINES + 2).map(|n| n.to_string()).collect::<Vec<_>>().join("\n")), None, "a line or two more isn't worth a Show all");
}

#[test]
fn the_sample_data_has_releases_a_draft_and_tags_without_a_release() {
    let data = crate::model::Data::sample_at(1_794_216_911);
    let site = &data.repos[0];
    let l = sample(site, "me", 1_794_216_911);
    assert_eq!(l.releases.len(), 3);
    assert_eq!(l.releases.iter().filter(|r| r.draft).count(), 1);
    assert_eq!(l.releases.iter().filter(|r| r.prerelease).count(), 1);
    assert!(l.releases.iter().all(|r| !r.body.is_empty()) && l.releases.iter().any(|r| !r.assets.is_empty()));
    assert!(l.releases.windows(2).all(|w| w[0].created > w[1].created), "newest first");
    assert_eq!(without_release(&l.tags, &l.releases).len(), 2);
    assert_eq!(latest_of(&l.releases).map(|r| r.tag.as_str()), Some("v1.2.0"));
    assert!(collapsed(&l.releases[2].body).is_some(), "the latest's notes are long enough to collapse");
    assert_eq!(next_versions(l.tag_names()), ["v1.3.1", "v1.4.0", "v2.0.0"], "the draft's tag counts");
    assert!(sample(&data.repos[4], "me", 0).releases.is_empty(), "another repository has none");
    let notes = sample_notes(site, "me", "main", "v1.4.0", Some("v1.2.0"));
    assert!(notes.contains("## What's Changed") && notes.contains("compare/v1.2.0...v1.4.0"), "{notes}");
}

#[test]
fn requests_page_the_releases_and_explain_what_github_refuses() {
    let (url, seen) = mock::serve(|method, path, body| {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        match (method, path) {
            ("GET", "/repos/o/r/releases?per_page=20&page=1") => {
                let page: Vec<Value> = (0..PAGE as u64).map(|n| mock::release(100 + n, &format!("v1.{n}.0"), false, false, "")).collect();
                (200, "Link: <https://api.github.com/repositories/1/releases?per_page=20&page=2>; rel=\"next\"\r\n".into(), serde_json::to_string(&page).unwrap())
            }
            ("GET", p) if p.starts_with("/repos/o/r/releases?") => (200, String::new(), json!([mock::release(1, "v0.1.0", false, false, "")]).to_string()),
            ("GET", "/repos/o/r/tags?per_page=50") => (200, "Link: <https://api.github.com/repositories/1/tags?per_page=50&page=2>; rel=\"next\"\r\n".into(), json!([{ "name": "v1.0.0", "commit": { "sha": "a".repeat(40) } }]).to_string()),
            ("GET", "/repos/o/r/releases/latest") => (200, String::new(), mock::release(119, "v1.19.0", false, false, "").to_string()),
            ("GET", "/repos/o/none/releases/latest") => (404, String::new(), json!({ "message": "Not Found" }).to_string()),
            ("POST", "/repos/o/r/releases/generate-notes") => (200, String::new(), json!({ "name": "v2.0.0", "body": "## What's Changed\n* Releases by @ada" }).to_string()),
            ("POST", "/repos/o/r/releases") if v["tag_name"] == "v1.0.0" => (422, String::new(), json!({ "message": "Validation Failed", "errors": [{ "resource": "Release", "code": "already_exists", "field": "tag_name" }] }).to_string()),
            ("POST", "/repos/o/r/releases") => (201, String::new(), mock::release(200, v["tag_name"].as_str().unwrap_or(""), v["draft"].as_bool().unwrap_or(true), false, v["body"].as_str().unwrap_or("")).to_string()),
            ("POST", "/repos/o/locked/releases") => (403, String::new(), json!({ "message": "Resource not accessible by personal access token" }).to_string()),
            ("PATCH", "/repos/o/r/releases/200") => (200, String::new(), mock::release(200, "v2.0.0", false, false, "").to_string()),
            ("PATCH", "/repos/o/r/releases/201") => (200, String::new(), "{}".into()),
            _ => (404, String::new(), json!({ "message": "Not Found" }).to_string()),
        }
    });
    let g = crate::github::Gh::new(&mock::config(&url));
    let (rows, more) = pollster::block_on(g.releases("o/r", 1)).unwrap();
    assert_eq!((rows.len(), more), (PAGE, true), "a Link to the next page");
    let (rows, more) = pollster::block_on(g.releases("o/r", 2)).unwrap();
    assert_eq!((rows.len(), more), (1, false));
    let (listing, more, latest) = pollster::block_on(g.release_listing("o/r")).unwrap();
    assert!(more && listing.tags_more && listing.tags.len() == 1, "GitHub has more tags, and says so");
    assert_eq!(latest, Some(Latest { id: 119, tag: "v1.19.0".into() }));
    assert_eq!(pollster::block_on(g.latest_release("o/none")).unwrap(), None, "no release: GitHub answers 404");
    let notes = pollster::block_on(g.generate_notes("o/r", "v2.0.0", "main", None)).unwrap();
    assert_eq!(notes, "## What's Changed\n* Releases by @ada");
    let new = NewRelease { tag: "v2.0.0".into(), target: "main".into(), name: "Two".into(), body: "Notes".into(), draft: true, prerelease: false };
    let saved = pollster::block_on(g.save_release("o/r", None, &new)).unwrap();
    assert_eq!((saved.id, saved.draft, saved.body.as_str()), (200, true, "Notes"));
    let published = pollster::block_on(g.publish_release("o/r", 200)).unwrap();
    assert!(!published.draft);
    assert_eq!(pollster::block_on(g.publish_release("o/r", 201)).unwrap().id, 0, "an answer that can't be read: the list is read again");
    let taken = NewRelease { tag: "v1.0.0".into(), ..new.clone() };
    let e = pollster::block_on(g.save_release("o/r", None, &taken)).unwrap_err();
    assert!(e.contains("This tag already has a release"), "{e}");
    let e = pollster::block_on(g.save_release("o/locked", None, &new)).unwrap_err();
    assert!(e.contains("(403)") && e.contains("Contents: Read and write"), "{e}");
    let e = pollster::block_on(g.save_release("o/gone", None, &new)).unwrap_err();
    assert!(e.contains("Not found (404)") && e.contains("Contents: Read and write"), "{e}");
    let seen = seen.lock().unwrap().clone();
    let body = |what: &str| seen.iter().find(|(m, _)| m == what).map(|(_, b)| serde_json::from_str::<Value>(b).unwrap()).unwrap();
    let asked = body("POST /repos/o/r/releases/generate-notes");
    assert_eq!((asked["tag_name"].as_str(), asked["target_commitish"].as_str(), asked.get("previous_tag_name")), (Some("v2.0.0"), Some("main"), None));
    let created = body("POST /repos/o/r/releases");
    assert_eq!((created["tag_name"].as_str(), created["name"].as_str(), created["draft"].as_bool(), created["prerelease"].as_bool()), (Some("v2.0.0"), Some("Two"), Some(true), Some(false)));
    assert_eq!(body("PATCH /repos/o/r/releases/200"), json!({ "draft": false }), "publishing a draft sends nothing else");
    assert!(!seen.iter().any(|(m, _)| m.starts_with("DELETE")), "nothing is deleted");
}
