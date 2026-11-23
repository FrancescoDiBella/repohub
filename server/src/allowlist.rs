//! Which GitHub requests the proxy forwards: `allowed`, and the board and branch paths checked on the raw
//! address before anything is decoded.

use super::*;

/// One segment of a board's path in a notes repository, as the app writes it (its notes
/// segment encoding, decoded once): `[A-Za-z0-9_.-]` and `%XX`, not empty, no leading dot.
fn board_segment(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b[0] == b'.' || s.len() > 255 {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.') => i += 1,
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => i += 3,
            _ => return false,
        }
    }
    true
}

/// `boards/<owner>/<repo>/<branch>/<name>.excalidraw` (after `contents/`): the only files board
/// sync reads and writes in the notes repository.
fn board_file(rest: &[&str]) -> bool {
    rest.len() == 6 && rest[0] == "contents" && rest[1] == "boards" && rest[2..].iter().all(|s| board_segment(s))
        && rest[5].strip_suffix(".excalidraw").is_some_and(|stem| !stem.is_empty())
}

/// A board's contents path, checked on the raw address before anything is decoded:
/// `/repos/{o}/{r}/contents/boards/…` where each board segment is `[A-Za-z0-9_.-]` or `%25XX`
/// (the app's `%XX`, itself encoded). No `..`, no empty segment, no encoded slash (`%2F`) or dot:
/// GitHub reads `%25` back as `%`, so the file names keep their `%` signs. Returns the path
/// decoded once, for [`allowed`].
pub(super) fn board_contents_path(raw: &str) -> Option<String> {
    let seg: Vec<&str> = raw.strip_prefix('/')?.split('/').collect();
    let name = |s: &str| !s.is_empty() && !s.starts_with('.') && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let raw_segment = |s: &str| {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                c if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.') => i += 1,
                b'%' if i + 4 < b.len() && &b[i + 1..i + 3] == b"25" && b[i + 3].is_ascii_hexdigit() && b[i + 4].is_ascii_hexdigit() => i += 5,
                _ => return false,
            }
        }
        !b.is_empty() && b[0] != b'.'
    };
    let ok = seg.len() == 9 && seg[0] == "repos" && name(seg[1]) && name(seg[2]) && seg[3] == "contents" && seg[4] == "boards" && seg[5..].iter().all(|s| raw_segment(s));
    ok.then(|| raw.replace("%25", "%"))
}

/// A branch name GitHub takes (the app's `github::valid_branch`): no spaces, no `..`, no
/// leading `-`, `/` or `.`, none of `~^:?*[\`.
fn branch_name(name: &str) -> bool {
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

/// `base...head` of a comparison: two branch names around the three dots.
fn compare_spec(spec: &str) -> bool {
    spec.split_once("...").is_some_and(|(base, head)| branch_name(base) && branch_name(head))
}

/// A path naming branches, checked on the raw address before anything is decoded:
/// `/repos/{o}/{r}/git/ref/heads/<branch>` and `/repos/{o}/{r}/compare/<base>...<head>`.
///
/// A branch name may hold `%`, which the app sends as `%25` (GitHub decodes the path once).
/// Decoded once, a `%` may not start an escape (`%` and two hex digits), so nothing that reads
/// the path again can make another character of it: `%252e%252e` (`..` encoded twice) and
/// `%2541` stay refused, `50%25-off` passes. An encoded dot, slash, backslash or NUL (`%2e`,
/// `%2f`, `%5c`, `%00`) is refused anywhere, the owner and repository can't hold `%`, and the
/// names must be branch names. Returns the path decoded once, for [`allowed`].
pub(super) fn branch_path(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if ["%2e", "%2f", "%5c", "%00"].iter().any(|x| lower.contains(x)) || raw.contains('\\') || raw.chars().any(|c| c.is_control()) {
        return None;
    }
    let seg: Vec<&str> = raw.strip_prefix('/')?.split('/').collect();
    let name = |s: &str| !s.is_empty() && !s.starts_with('.') && s.len() <= 100 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if seg.len() < 5 || seg[0] != "repos" || !name(seg[1]) || !name(seg[2]) {
        return None;
    }
    let (at, names) = match &seg[3..] {
        ["git", "ref", "heads", rest @ ..] if !rest.is_empty() => ("git/ref/heads", rest.join("/")),
        ["compare", rest @ ..] if !rest.is_empty() => ("compare", rest.join("/")),
        _ => return None,
    };
    let decoded = percent_decode_path(&names)?;
    let b = decoded.as_bytes();
    let escape_left = (0..b.len()).any(|i| b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit());
    if escape_left || decoded.contains('\\') || decoded.chars().any(|c| c.is_control()) {
        return None;
    }
    let ok = if at == "compare" { compare_spec(&decoded) } else { branch_name(&decoded) };
    ok.then(|| format!("/repos/{}/{}/{at}/{decoded}", seg[1], seg[2]))
}

/// Which proxied API calls are allowed: the exact read endpoints RepoHub uses, starring, and the few
/// writes the app makes (create an issue, a branch, a pull request or a release, close an issue,
/// merge a pull request, publish a draft release, triage an issue, and board sync's writes to
/// one board file in the notes repository). Nothing that deletes repositories, branches, releases
/// or tags, rewrites, uploads, or exposes administrative repository data.
pub(super) fn allowed(method: &Method, path: &str) -> bool {
    let p = path.split('?').next().unwrap_or("");
    let seg: Vec<&str> = p.trim_start_matches('/').split('/').collect();
    let repo = seg.len() >= 3 && seg[0] == "repos" && !seg[1].is_empty() && !seg[2].is_empty();
    let rest = if repo { &seg[3..] } else { &[][..] };
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let sha = |s: &str| (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());

    match method {
        Method::Get => {
            if p == "/user" || p == "/user/repos" || p == "/user/starred" || p == "/search/issues" || p == "/notifications" {
                return true;
            }
            if seg.len() == 2 && seg[0] == "users" && !seg[1].is_empty() {
                return true;
            }
            if seg.len() == 3 && seg[0] == "users" && !seg[1].is_empty() && (seg[2] == "repos" || seg[2] == "orgs") {
                return true;
            }
            if seg.len() == 3 && seg[0] == "orgs" && !seg[1].is_empty() && seg[2] == "repos" {
                return true;
            }
            if repo {
                // Board sync: the notes repository itself (exists, private, default branch), its
                // file list, and its boards.
                if rest.is_empty() || board_file(rest) {
                    return true;
                }
                if let ["git", "trees", r] = rest
                    && !r.is_empty() && !r.starts_with('.') && r.len() <= 100 && r.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                {
                    return true;
                }
                match rest {
                    ["actions", "runs"]
                    | ["stats", "participation"]
                    | ["contents"]
                    | ["readme"]
                    | ["languages"]
                    | ["commits"]
                    | ["branches"]
                    | ["issues"]
                    // The releases, the tags, and the latest release (not one release by id or tag).
                    | ["releases"]
                    | ["releases", "latest"]
                    | ["tags"] => return true,
                    // A repository's labels, assignable people and milestones (triage pickers).
                    | ["labels"]
                    | ["assignees"]
                    | ["milestones"] => return true,
                    _ => {}
                }
                // The last commit of one branch (creating a branch from it).
                if rest.len() >= 4 && rest[0] == "git" && rest[1] == "ref" && rest[2] == "heads" && rest[3..].iter().all(|s| !s.is_empty()) && branch_name(&rest[3..].join("/")) {
                    return true;
                }
                if rest.len() == 3 && rest[0] == "pulls" && number(rest[1]) && matches!(rest[2], "files" | "reviews" | "comments") {
                    return true;
                }
                // One issue or pull request and its conversation.
                if rest.len() == 2 && (rest[0] == "issues" || rest[0] == "pulls") && number(rest[1]) {
                    return true;
                }
                if rest.len() == 3 && rest[0] == "issues" && number(rest[1]) && rest[2] == "comments" {
                    return true;
                }
                if rest.len() == 4 && rest[0] == "actions" && rest[1] == "runs" && number(rest[2]) && rest[3] == "jobs" {
                    return true;
                }
                if rest.len() == 4 && rest[0] == "actions" && rest[1] == "jobs" && number(rest[2]) && rest[3] == "logs" {
                    return true;
                }
                // The checks on a pull request's last commit (a hex commit id).
                if rest.len() == 3 && rest[0] == "commits" && sha(rest[1]) && rest[2] == "check-runs" {
                    return true;
                }
                // One commit with its files, by its id only.
                if rest.len() == 2 && rest[0] == "commits" && sha(rest[1]) {
                    return true;
                }
                // Two branches compared: `base...head`, each a branch name (which may hold `/`).
                if rest.len() >= 2 && rest[0] == "compare" && compare_spec(&rest[1..].join("/")) {
                    return true;
                }
            }
            false
        }
        Method::Put if repo && rest.len() == 3 && rest[0] == "pulls" && number(rest[1]) && rest[2] == "merge" => true,
        // Triage also edits an issue's labels, assignees, milestone, title and description (the
        // PATCH below) and asks for reviews (POST). Taking back a review request on one pull
        // request is the only DELETE in a repository.
        Method::Delete if repo && rest.len() == 3 && rest[0] == "pulls" && number(rest[1]) && rest[2] == "requested_reviewers" => true,
        // Board sync writes one board, under boards/ and ending in .excalidraw, nothing else.
        Method::Put | Method::Delete if repo && board_file(rest) => true,
        Method::Put | Method::Delete => seg.len() == 4 && seg[0] == "user" && seg[1] == "starred" && !seg[2].is_empty() && !seg[3].is_empty(),
        // Creating an issue, a pull request, a branch or a release (and its generated notes);
        // commenting; reviewing a pull request; asking for a review; re-running a workflow.
        Method::Post => repo && (matches!(rest, ["issues"] | ["pulls"] | ["git", "refs"] | ["releases"] | ["releases", "generate-notes"])
            || (rest.len() == 3 && number(rest[1]) && matches!((rest[0], rest[2]), ("issues", "comments") | ("pulls", "reviews") | ("pulls", "comments") | ("pulls", "requested_reviewers")))
            || (rest.len() == 4 && rest[0] == "actions" && rest[1] == "runs" && number(rest[2]) && matches!(rest[3], "rerun" | "rerun-failed-jobs"))),
        // Closing or reopening an issue, and editing its labels, assignees, milestone, title and
        // description (pull requests too); saving or publishing a draft release (by its number);
        // marking one notification as read.
        Method::Patch => (repo && rest.len() == 2 && (rest[0] == "issues" || rest[0] == "releases") && number(rest[1]))
            || (seg.len() == 3 && seg[0] == "notifications" && seg[1] == "threads" && number(seg[2])),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
