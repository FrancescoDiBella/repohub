//! `server/allowlist.txt`, the requests the app makes and ones it must never make, through the
//! proxy's checks as a request meets them (`board_contents_path` or
//! `validate_and_decode_path`, then `allowed`). The Android app's `GitHubPaths` is tested
//! against the same file (`BackendTest.sharedAllowlist`), and the app's own requests too
//! (`src/allowlist_tests.rs`), so the three stay equal.
use std::str::FromStr;

use tiny_http::Method;

use super::{allowed, board_contents_path, validate_and_decode_path};

const CASES: &str = include_str!("../allowlist.txt");

/// What the proxy does with `method` on the raw `path_and_query`: true when it forwards it.
fn passes(method: &Method, path_and_query: &str) -> bool {
    let raw = path_and_query.split('?').next().unwrap_or("");
    if raw.chars().any(|c| c.is_control()) {
        return false;
    }
    match board_contents_path(raw).or_else(|| validate_and_decode_path(raw)) {
        Some(decoded) => allowed(method, &decoded),
        None => false,
    }
}

#[test]
fn the_shared_allowlist() {
    let mut wrong = Vec::new();
    let mut count = 0;
    for line in CASES.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts.len(), 3, "{line}");
        let method = Method::from_str(parts[1]).unwrap_or_else(|_| panic!("{line}"));
        let want = match parts[0] {
            "allow" => true,
            "deny" => false,
            other => panic!("{other}: allow or deny"),
        };
        count += 1;
        if passes(&method, parts[2]) != want {
            wrong.push(line);
        }
    }
    assert!(count > 100, "the list is read");
    assert!(wrong.is_empty(), "the proxy doesn't do what server/allowlist.txt says:\n{}", wrong.join("\n"));
}
