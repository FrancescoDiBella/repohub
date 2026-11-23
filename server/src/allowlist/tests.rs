use super::*;

#[test]
fn proxy_exact_get_allowlist() {
    // All endpoints src/github.rs actually calls must be allowed:
    assert!(allowed(&Method::Get, "/user"));
    assert!(allowed(&Method::Get, "/user/repos"));
    assert!(allowed(&Method::Get, "/user/starred"));
    // Never asked by the app: its organizations come from `/users/{login}/orgs`.
    assert!(!allowed(&Method::Get, "/user/orgs"));
    assert!(allowed(&Method::Get, "/search/issues"));
    assert!(allowed(&Method::Get, "/users/octocat"));
    assert!(allowed(&Method::Get, "/users/octocat/repos"));
    assert!(allowed(&Method::Get, "/users/octocat/orgs"));
    assert!(allowed(&Method::Get, "/orgs/acme/repos"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/actions/runs"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/stats/participation"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/contents"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/readme"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/languages"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/commits"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/branches"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/issues"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/git/ref/heads/main"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/git/ref/heads/feature/x"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/123/files"));

    // Sensitive or unrelated endpoints under /repos/* and /users/* MUST be rejected:
    assert!(!allowed(&Method::Get, "/repos/owner/repo/hooks"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/collaborators"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/variables"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/environments"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/invitations"));
    assert!(!allowed(&Method::Get, "/users/octocat/keys"));
    assert!(!allowed(&Method::Get, "/user/emails"));
    assert!(!allowed(&Method::Get, "/orgs/acme/members"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/notanumber/files"));
    // One issue or pull request and its conversation, and nothing next to them.
    assert!(allowed(&Method::Get, "/repos/owner/repo/issues/12"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/issues/12/comments"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7/reviews"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/issues/bad"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/issues/12/events"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/7/commits"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/pulls/7/reviews/3"));
    assert!(!allowed(&Method::Get, "/gists"));
    // Your notifications, and nothing under them to read.
    assert!(allowed(&Method::Get, "/notifications"));
    assert!(!allowed(&Method::Get, "/notifications/threads/12"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/notifications"));
    // The checks on a commit, by its id only.
    assert!(allowed(&Method::Get, "/repos/owner/repo/commits/0123abcdef0123abcdef0123abcdef0123abcdef/check-runs"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/main/check-runs"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/0123abc/statuses"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/commits/0123abc/check-runs"));
    // One commit, by a hex id of 7 to 64 digits, and nothing under it.
    assert!(allowed(&Method::Get, "/repos/owner/repo/commits/0123abc"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/commits/0123abcdef0123abcdef0123abcdef0123abcdef"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/0123ab"));
    assert!(!allowed(&Method::Get, &format!("/repos/owner/repo/commits/{}", "a".repeat(65))));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/main"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/0123abc/comments"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/commits/0123abc/pulls"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/commits/0123abc"));
    // Two branches compared: valid names around `...`, with `/` inside a name.
    assert!(allowed(&Method::Get, "/repos/owner/repo/compare/main...feature"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/compare/main...feature/login?per_page=100"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/compare/release/1.2...fix/x-y_z"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main..feature"), "two dots isn't a comparison the app makes");
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main....feature"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/...feature"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main..."));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/-main...feature"), "a leading - could pass for an option");
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main...a..b"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main...fea ture"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main...HEAD~1"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/compare/main...x:y"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/compare/main...feature"));
    // Encoded dots and slashes never reach the allowlist.
    assert!(validate_and_decode_path("/repos/owner/repo/compare/main%2e%2e%2efeature").is_none());
    assert!(validate_and_decode_path("/repos/owner/repo/compare/main...feature%2fx").is_none());
    assert!(validate_and_decode_path("/repos/owner/repo/compare/main...fix%2Bx").is_some_and(|p| allowed(&Method::Get, &p)));
    // Workflow run jobs and job logs, by numeric id only.
    assert!(allowed(&Method::Get, "/repos/owner/repo/actions/runs/42/jobs"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/actions/jobs/99/logs"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/runs/bad/jobs"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/jobs/bad/logs"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/actions/runs/42/logs"));
    // Releases and tags: the list, the latest, the tags; nothing by id, tag or asset.
    assert!(allowed(&Method::Get, "/repos/owner/repo/releases"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/releases?per_page=20&page=2"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/releases/latest"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/tags?per_page=50"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/releases/42"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/releases/tags/v1.0.0"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/releases/42/assets"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/releases/assets/7"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/releases/latest/assets"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/tags/v1.0.0"));
    assert!(!allowed(&Method::Get, "/repos/owner/repo/git/tags/0123abc"));
}

#[test]
fn proxy_write_allowlist() {
    assert!(allowed(&Method::Put, "/user/starred/owner/repo"));
    assert!(allowed(&Method::Delete, "/user/starred/owner/repo"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/issues"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/pulls"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/git/refs"));
    assert!(allowed(&Method::Patch, "/repos/owner/repo/issues/12"));
    assert!(allowed(&Method::Put, "/repos/owner/repo/pulls/7/merge"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/rerun"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/rerun-failed-jobs"));
    // Releases: create one, generate its notes, save or publish a draft by its number.
    assert!(allowed(&Method::Post, "/repos/owner/repo/releases"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/releases/generate-notes"));
    assert!(allowed(&Method::Patch, "/repos/owner/repo/releases/42"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/releases/latest"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/releases/4a2"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/releases/-1"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/releases/42/assets"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/releases"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/releases/42"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/releases/42/assets"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/releases/generate-notes/x"));
    assert!(!allowed(&Method::Put, "/repos/owner/repo/releases/42"));
    // Releases, their files and tags are never deleted, and no tag object is made.
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/releases/42"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/releases/assets/7"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/releases/42/assets/7"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/git/tags"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/git/refs/tags/v1.0.0"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/git/refs/tags/v1.0.0"));
    // Marking one notification as read, not all of them, and no unsubscribing.
    assert!(allowed(&Method::Patch, "/notifications/threads/12"));
    assert!(!allowed(&Method::Patch, "/notifications/threads/abc"));
    assert!(!allowed(&Method::Put, "/notifications"));
    assert!(!allowed(&Method::Delete, "/notifications/threads/12"));
    assert!(!allowed(&Method::Put, "/notifications/threads/12/subscription"));

    // Disallowed writes:
    assert!(!allowed(&Method::Delete, "/repos/owner/repo"));
    assert!(!allowed(&Method::Post, "/user/repos"));
    assert!(!allowed(&Method::Put, "/repos/owner/repo/contents/README.md"));
    // Commenting and reviewing are writes the app makes; nothing else under them is.
    assert!(allowed(&Method::Post, "/repos/owner/repo/issues/12/comments"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/pulls/7/reviews"));
    // Comments on lines of a pull request's diff: read and written, nothing under them.
    assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7/comments"));
    assert!(allowed(&Method::Post, "/repos/owner/repo/pulls/7/comments"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/comments/3/replies"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/pulls/comments/3"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/issues/bad/comments"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/merge"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/pulls/7/reviews/3/dismissals"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/issues/comments/9"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/issues/bad"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/git/refs/heads/main"));
    assert!(!allowed(&Method::Put, "/user/starred/owner/repo/extra"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/actions/runs/bad/rerun"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/actions/runs/42/cancel"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/actions/runs/42/logs"));
}

#[test]
fn proxy_triage_allowlist() {
    // The pickers' choices: exactly these three lists, nothing under them, and only read.
    for p in ["/repos/owner/repo/labels", "/repos/owner/repo/labels?per_page=100", "/repos/owner/repo/assignees?per_page=100",
        "/repos/owner/repo/milestones?state=open&per_page=100"] {
        assert!(allowed(&Method::Get, p), "{p}");
    }
    for p in ["/repos/owner/repo/labels/bug", "/repos/owner/repo/assignees/alice", "/repos/owner/repo/milestones/3",
        "/repos/owner/repo/milestones/3/labels", "/repos/owner/repo/collaborators", "/repos/owner/repo/issues/7/labels", "/labels"] {
        assert!(!allowed(&Method::Get, p), "{p}");
    }
    // `/repos/owner/labels` is the repository named "labels" (read for board sync), not a list.
    assert!(allowed(&Method::Get, "/repos/owner/labels"));
    assert!(!allowed(&Method::Get, "/repos/owner/labels/labels/bug"));
    for m in [Method::Post, Method::Put, Method::Patch, Method::Delete] {
        assert!(!allowed(&m, "/repos/owner/repo/labels"), "{m:?} labels: creating labels is out of scope");
        assert!(!allowed(&m, "/repos/owner/repo/milestones"), "{m:?} milestones");
        assert!(!allowed(&m, "/repos/owner/repo/labels/bug"), "{m:?} one label");
    }
    // Labels, assignees, milestone, title and description: one PATCH on the issue.
    assert!(allowed(&Method::Patch, "/repos/owner/repo/issues/7"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/pulls/7"), "a pull request's own fields aren't edited");
    assert!(!allowed(&Method::Post, "/repos/owner/repo/issues/7/labels"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/issues/7/labels/bug"));
    assert!(!allowed(&Method::Post, "/repos/owner/repo/issues/7/assignees"));
    assert!(!allowed(&Method::Delete, "/repos/owner/repo/issues/7/assignees"));
    // Asking for a review and taking the request back: digits for the number, nothing next to it.
    assert!(allowed(&Method::Post, "/repos/owner/repo/pulls/7/requested_reviewers"));
    assert!(allowed(&Method::Delete, "/repos/owner/repo/pulls/7/requested_reviewers"));
    assert!(allowed(&Method::Get, "/repos/owner/repo/pulls/7"), "read with the conversation");
    for p in ["/repos/owner/repo/pulls/x/requested_reviewers", "/repos/owner/repo/pulls/7a/requested_reviewers", "/repos/owner/repo/pulls//requested_reviewers",
        "/repos/owner/repo/pulls/7/requested_reviewers/alice", "/repos/owner/repo/pulls/7/requested_reviewer", "/repos/owner/repo/issues/7/requested_reviewers",
        "/repos/owner/repo/requested_reviewers", "/repos/owner/repo/pulls/requested_reviewers"] {
        assert!(!allowed(&Method::Post, p), "POST {p}");
        assert!(!allowed(&Method::Delete, p), "DELETE {p}");
    }
    assert!(!allowed(&Method::Put, "/repos/owner/repo/pulls/7/requested_reviewers"));
    assert!(!allowed(&Method::Patch, "/repos/owner/repo/pulls/7/requested_reviewers"));
    // DELETE anywhere else in a repository stays refused.
    for p in ["/repos/owner/repo", "/repos/owner/repo/pulls/7", "/repos/owner/repo/issues/7", "/repos/owner/repo/pulls/7/reviews/1",
        "/repos/owner/repo/pulls/7/merge", "/repos/owner/repo/issues/7/comments", "/repos/owner/repo/labels", "/repos/owner/repo/milestones/3",
        "/repos/owner/repo/git/refs/heads/main", "/repos/owner/repo/branches/main"] {
        assert!(!allowed(&Method::Delete, p), "DELETE {p}");
    }
    // Encoded slashes or dots never reach the allowlist.
    assert!(validate_and_decode_path("/repos/owner/repo/pulls/7%2frequested_reviewers").is_none());
    assert!(validate_and_decode_path("/repos/owner/repo/pulls/7/%2e%2e/requested_reviewers").is_none());
}

#[test]
fn board_sync_allowlist() {
    let board = "/repos/me/repohub-notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw";
    for m in [Method::Get, Method::Put, Method::Delete] {
        assert!(allowed(&m, board), "{m:?}");
        assert!(allowed(&m, "/repos/me/notes/contents/boards/o/r/main/Plan.excalidraw"), "{m:?}");
    }
    assert!(!allowed(&Method::Post, board));
    assert!(!allowed(&Method::Patch, board));
    // The repository itself and its file list: read only.
    assert!(allowed(&Method::Get, "/repos/me/repohub-notes"));
    assert!(allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/main?recursive=1"));
    assert!(!allowed(&Method::Put, "/repos/me/repohub-notes"));
    assert!(!allowed(&Method::Delete, "/repos/me/repohub-notes"));
    assert!(!allowed(&Method::Post, "/repos/me/repohub-notes/git/trees"));
    assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/feature/x"));
    assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/trees/.."));
    assert!(!allowed(&Method::Get, "/repos/me/repohub-notes/git/blobs/abc123"));
    // Writes stay narrow: under boards/, four levels deep, ending in .excalidraw.
    for bad in [
        "/repos/o/r/contents/README.md",
        "/repos/o/r/contents/.github/workflows/x.yml",
        "/repos/o/r/contents/boards/../x",
        "/repos/o/r/contents/boards/o/r/main/x.json",
        "/repos/o/r/contents/boards/o/r/main/.excalidraw",
        "/repos/o/r/contents/boards/o/r/x.excalidraw",
        "/repos/o/r/contents/boards/o/r/main/sub/x.excalidraw",
        "/repos/o/r/contents/boards/o//main/x.excalidraw",
        "/repos/o/r/contents/boards/o/r/../x.excalidraw",
        "/repos/o/r/contents/boards/o/r/main/.hidden.excalidraw",
        "/repos/o/r/contents/boards/o/r/main/a b.excalidraw",
        "/repos/o/r/contents/boards/o/r/main/x.excalidraw/extra",
        "/repos/o/r/contents/notes/o/r/main/x.excalidraw",
        "/repos/o/r/contents",
    ] {
        assert!(!allowed(&Method::Put, bad), "PUT {bad}");
        assert!(!allowed(&Method::Delete, bad), "DELETE {bad}");
    }
    assert!(allowed(&Method::Get, "/repos/o/r/contents"), "the Code tab's root listing is unchanged");
    assert!(!allowed(&Method::Get, "/repos/o/r/contents/README.md"), "other files stay unreadable");
    assert!(!allowed(&Method::Get, "/repos/o/r/contents/.github/workflows/x.yml"));
}

#[test]
fn board_paths_are_checked_before_decoding() {
    // What the app sends: its `%XX` segment encoding, itself encoded as `%25XX`.
    let raw = "/repos/me/notes/contents/boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw";
    assert_eq!(board_contents_path(raw).as_deref(), Some("/repos/me/notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw"));
    assert!(allowed(&Method::Put, &board_contents_path(raw).unwrap()));
    // A branch named `..` is one folder, `%2E.`, never a step up.
    assert!(board_contents_path("/repos/me/notes/contents/boards/o/r/%252E./x.excalidraw").is_some());
    for bad in [
        "/repos/me/notes/contents/boards/o/r/feature%2Fx/x.excalidraw", // an encoded slash
        "/repos/me/notes/contents/boards/o/r/%2e%2e/x.excalidraw",
        "/repos/me/notes/contents/boards/o/r/../x.excalidraw",
        "/repos/me/notes/contents/boards/o/r/main/.x.excalidraw",
        "/repos/me/notes/contents/boards/o/r//x.excalidraw",
        "/repos/me/notes/contents/boards/o/r/main/x%25.excalidraw",
        "/repos/me/notes/contents/boards/o/r/main/x%252.excalidraw",
        "/repos/me/notes/contents/boards/o/r/main/x%5Cy.excalidraw",
        "/repos/me/notes/contents/boards/o/r/main/x%00.excalidraw",
        "/repos/me/notes/contents/README.md",
        "/repos/me/notes/contents/boards/o/r/main/sub/x.excalidraw",
        "/repos/.me/notes/contents/boards/o/r/main/x.excalidraw",
    ] {
        assert_eq!(board_contents_path(bad), None, "{bad}");
    }
    // Everything else still goes through the general check, which refuses `%` after decoding.
    assert_eq!(validate_and_decode_path("/repos/me/notes/contents/boards/o/r/feature%252Fx/x.excalidraw"), None);
}

/// A branch name with `%` (sent as `%25`) passes where branches are named, and nothing that
/// a second decoding could turn into something else does.
#[test]
fn branch_names_with_a_percent_sign() {
    let passes = |raw: &str| board_contents_path(raw).or_else(|| branch_path(raw)).or_else(|| validate_and_decode_path(raw)).is_some_and(|d| allowed(&Method::Get, &d));
    assert_eq!(branch_path("/repos/o/r/git/ref/heads/50%25-off").as_deref(), Some("/repos/o/r/git/ref/heads/50%-off"));
    assert_eq!(branch_path("/repos/o/r/compare/main...feature/100%25").as_deref(), Some("/repos/o/r/compare/main...feature/100%"));
    for ok in [
        "/repos/o/r/git/ref/heads/50%25-off",
        "/repos/o/r/git/ref/heads/fix/50%25",
        "/repos/o/r/git/ref/heads/%25",
        "/repos/o/r/compare/main...50%25-off",
        "/repos/o/r/compare/release%25x/1...feature/x",
        // Other characters the app encodes stay as they were.
        "/repos/o/r/git/ref/heads/feature%2Bx",
        "/repos/o/r/git/ref/heads/feature/x",
        "/repos/o/r/compare/main...feature/x",
    ] {
        assert!(passes(ok), "{ok}");
    }
    for bad in [
        // An encoded dot, slash, NUL or backslash, once or twice.
        "/repos/o/r/git/ref/heads/%2e%2e/%2e%2e/user/emails",
        "/repos/o/r/git/ref/heads/%252e%252e/%252e%252e/user/emails",
        "/repos/o/r/git/ref/heads/a%2fb",
        "/repos/o/r/git/ref/heads/a%252fb",
        "/repos/o/r/git/ref/heads/a%00",
        "/repos/o/r/git/ref/heads/a%2500",
        "/repos/o/r/git/ref/heads/a%5Cb",
        "/repos/o/r/git/ref/heads/a%255Cb",
        // A `%` left after one decoding that starts an escape (encoded twice or more).
        "/repos/o/r/git/ref/heads/a%2541",
        "/repos/o/r/git/ref/heads/a%252541",
        "/repos/o/r/compare/main...a%252e%252e",
        "/repos/o/r/compare/a%2541...main",
        // A broken escape, a name that isn't a branch's, `%` outside branch names.
        "/repos/o/r/git/ref/heads/a%2",
        "/repos/o/r/git/ref/heads/a%zz",
        "/repos/o/r/git/ref/heads/a%25..b",
        "/repos/o/r/git/ref/heads/-x%25",
        "/repos/o/r/git/ref/heads/a..b",
        "/repos/o/r/compare/main..a%25",
        "/repos/o%25/r/git/ref/heads/main",
        "/repos/o/r/issues/14%25",
        "/repos/o/r/contents/x%25y",
        "/repos/o/r/git/refs/heads/a%25",
    ] {
        assert!(!passes(bad), "{bad}");
    }
}
