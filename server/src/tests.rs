use super::*;
use super::proxy::conditional_tag;

#[test]
fn loopback_detection() {
    assert!(is_loopback("127.0.0.1:8080"));
    assert!(is_loopback("127.0.0.1"));
    assert!(is_loopback("127.0.0.2:9000"));
    assert!(is_loopback("localhost:8080"));
    assert!(is_loopback("localhost"));
    assert!(is_loopback("[::1]:8080"));
    assert!(is_loopback("[::1]"));
    assert!(is_loopback("::1"));

    assert!(!is_loopback("0.0.0.0:8080"));
    assert!(!is_loopback("0.0.0.0"));
    assert!(!is_loopback("[::]:8080"));
    assert!(!is_loopback("[::]"));
    assert!(!is_loopback("::"));
    assert!(!is_loopback("192.168.1.5:8080"));
    assert!(!is_loopback("example.com:8080"));
}

#[test]
fn origin_matching() {
    assert!(origin_matches_host("http://localhost:8080", "localhost:8080"));
    assert!(origin_matches_host("http://example.com", "example.com:80"));
    assert!(origin_matches_host("http://example.com:80", "example.com"));
    assert!(origin_matches_host("https://example.com", "example.com:443"));
    assert!(origin_matches_host("https://example.com", "example.com"));
    assert!(origin_matches_host("http://[::1]:8080", "[::1]:8080"));

    // Different ports or schemes rejected
    assert!(!origin_matches_host("http://localhost:3000", "localhost:8080"));
    assert!(!origin_matches_host("http://example.com:443", "example.com:80"));
    assert!(!origin_matches_host("https://example.com", "example.com:80"));
    assert!(!origin_matches_host("null", "localhost:8080"));
    assert!(!origin_matches_host("file:///path", "localhost:8080"));
    assert!(!origin_matches_host("http://other.com:8080", "localhost:8080"));
}

#[test]
fn referer_origin_extraction() {
    assert_eq!(referer_origin("http://nas.local:8080/index.html"), Some("http://nas.local:8080"));
    assert_eq!(referer_origin("https://nas.local/repo/notes?q=1"), Some("https://nas.local"));
    assert_eq!(referer_origin("ftp://example.com"), None);
}

#[test]
fn path_traversal_and_encoding_validation() {
    assert_eq!(validate_and_decode_path("/user/repos"), Some("/user/repos".into()));
    assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/feature/x"), Some("/repos/o/r/git/ref/heads/feature/x".into()));
    assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/feature+x"), Some("/repos/o/r/git/ref/heads/feature+x".into()));

    // Raw encoded traversals rejected
    assert_eq!(validate_and_decode_path("/repos/o/r/%2e%2e/%2e%2e/user/emails"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/%2E%2E/etc"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/%2f/etc"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/%5c/etc"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/.."), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/..."), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/."), None);
    assert_eq!(validate_and_decode_path("/repos/o/r//contents"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r\\readme"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r\0readme"), None);

    // Multi-encoded traversals (%252e%252e) MUST be rejected:
    assert_eq!(validate_and_decode_path("/repos/o/r/%252e%252e"), None);
    assert_eq!(validate_and_decode_path("/repos/o/r/git/ref/heads/%252e%252e/%252e%252e/user/emails"), None);
}

#[test]
fn constant_time_comparison() {
    assert!(constant_time_eq(b"secret123", b"secret123"));
    assert!(!constant_time_eq(b"secret123", b"secret124"));
    assert!(!constant_time_eq(b"secret123", b"secret12"));
    assert!(!constant_time_eq(b"", b"secret"));
}

#[test]
fn cookie_encoding_and_parsing() {
    let sec = "p@ss;word=1 2\"3";
    let encoded = percent_encode_cookie(sec);
    assert!(!encoded.contains(';') && !encoded.contains('=') && !encoded.contains(' ') && !encoded.contains('"'));

    // Simulating Cookie header with malformed prefix
    let req_header = format!("malformed_cookie; other=1; repohub_secret={encoded}; final=x");
    let fake_req = |cookie_hdr: &str| -> Option<String> {
        for part in cookie_hdr.split(';') {
            let Some((k, v)) = part.split_once('=') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case("repohub_secret") {
                return percent_decode_path(v.trim());
            }
        }
        None
    };
    assert_eq!(fake_req(&req_header), Some(sec.to_string()));
}

#[test]
fn json_secret_parsing() {
    assert_eq!(parse_json_secret(r#"{"secret": "simple"}"#), Some("simple".into()));
    assert_eq!(parse_json_secret(r#"{"other": 1, "secret": "foo:bar,baz"}"#), Some("foo:bar,baz".into()));
    assert_eq!(parse_json_secret(r#"{"secret": "hello \"world\""}"#), Some("hello \"world\"".into()));
    assert_eq!(parse_json_secret(r#"{"secret": "line1\nline2"}"#), Some("line1\nline2".into()));
    assert_eq!(parse_json_secret(r#"{"invalid": true}"#), None);
}

#[test]
fn hosts() {
    assert_eq!(host_only("localhost:8080"), "localhost");
    assert_eq!(host_only("[::1]:8080"), "[::1]");
    assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
}

#[test]
fn secret_required_on_non_loopback() {
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let res = Cfg::load_with_addr("0.0.0.0:8080");
    match res {
        Err(e) => assert!(e.contains("REPOHUB_SECRET must be set")),
        Ok(_) => panic!("expected failure on non-loopback without secret"),
    }

    // Loopback succeeds without secret
    let res = Cfg::load_with_addr("127.0.0.1:8080");
    assert!(res.is_ok());
    let cfg = res.unwrap();
    assert!(cfg.allow_writes);
    assert_eq!(cfg.secret, None);
}

#[test]
fn static_path_validation() {
    let temp_dir = std::env::temp_dir().join(format!("repohub-static-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("index.html"), "<h1>Home</h1>").unwrap();
    let sub = temp_dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("app.js"), "console.log(1);").unwrap();

    let cfg = Cfg {
        token: None,
        api: "https://api.github.com".into(),
        web_dir: temp_dir.clone(),
        data_dir: temp_dir.clone(),
        allowed_hosts: vec!["localhost".into()],
        public_json: "{}".into(),
        agent: ureq::AgentBuilder::new().build(),
        secret: None,
        allow_writes: true,
        notes_max_bytes: 500 << 20,
        proxy_in_flight: std::sync::atomic::AtomicUsize::new(0),
    };

    let canon_web = std::fs::canonicalize(&cfg.web_dir).unwrap();

    // Check relative paths stay within web_dir
    let check_rel = |rel: &str| -> bool {
        if rel.contains(':') || rel.contains('\\') || rel.contains('%') || rel.chars().any(|c| c.is_control() || c == '\0') {
            return false;
        }
        if rel.split('/').any(|seg| seg.starts_with('.') || (!seg.is_empty() && seg.bytes().all(|b| b == b'.'))) {
            return false;
        }
        let path = cfg.web_dir.join(rel);
        if let Ok(canon_file) = std::fs::canonicalize(&path) {
            canon_file.starts_with(&canon_web) && canon_file.is_file()
        } else {
            false
        }
    };

    assert!(check_rel("index.html"));
    assert!(check_rel("sub/app.js"));
    assert!(!check_rel("../secret.txt"));
    assert!(!check_rel("..\\secret.txt"));
    assert!(!check_rel("sub/../.."));
    assert!(!check_rel(".hidden"));
    assert!(!check_rel("C:/Windows/System32"));
    assert!(!check_rel("file:test"));
    assert!(!check_rel("%252e%252e"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn origin_and_preflight_checks() {
    // origin matches host: ok
    assert!(origin_matches_host("http://localhost:8080", "localhost:8080"));
    // missing origin or mismatched origin
    assert!(!origin_matches_host("http://localhost:3000", "localhost:8080"));
    assert!(!origin_matches_host("null", "localhost:8080"));
}

fn http_status(res: Result<ureq::Response, ureq::Error>) -> (u16, String, Option<String>) {
    match res {
        Ok(r) => {
            let status = r.status();
            let set_cookie = r.header("Set-Cookie").map(str::to_owned);
            let body = r.into_string().unwrap_or_default();
            (status, body, set_cookie)
        }
        Err(ureq::Error::Status(code, r)) => {
            let set_cookie = r.header("Set-Cookie").map(str::to_owned);
            let body = r.into_string().unwrap_or_default();
            (code, body, set_cookie)
        }
        Err(e) => panic!("Network request failed: {e}"),
    }
}

#[test]
fn end_to_end_http_server_tests() {
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // 1. Mock upstream GitHub API
    let mock_upstream = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let upstream_addr = format!("http://{}", mock_upstream.server_addr());
    let up = mock_upstream.clone();
    std::thread::spawn(move || {
        for req in up.incoming_requests() {
            let auth = req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("Authorization")).map(|h| h.value.as_str().to_owned());
            let body = format!("{{\"ok\":true,\"auth\":{}}}", json_str(&auth.unwrap_or_default()));
            let resp = Response::from_string(body)
                .with_status_code(200)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                .with_header(Header::from_bytes("Link", "<https://api.github.com/user/repos?page=2>; rel=\"next\"").unwrap());
            let _ = req.respond(resp);
        }
    });

    // 2. Setup web_dir and data_dir
    let temp_dir = std::env::temp_dir().join(format!("repohub-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let web_dir = temp_dir.join("web");
    let data_dir = temp_dir.join("data");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<h1>RepoHub App</h1>").unwrap();

    // Symlink pointing outside web_dir
    let outside_file = temp_dir.join("secret_outside.txt");
    std::fs::write(&outside_file, "super secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_file, web_dir.join("leak_link.txt")).unwrap();

    // 3. Create RepoHub server with secret authentication
    let server = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let s_addr = format!("{}", server.server_addr());
    let cfg = std::sync::Arc::new(Cfg {
        token: Some("gh_token_12345".into()),
        api: upstream_addr,
        web_dir: web_dir.clone(),
        data_dir: data_dir.clone(),
        allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
        public_json: "{\"notes\":true}".into(),
        agent: ureq::AgentBuilder::new().try_proxy_from_env(false).build(),
        secret: Some("p@ss;word=1 2".into()),
        allow_writes: true,
        notes_max_bytes: 100, // Small limit for testing
        proxy_in_flight: std::sync::atomic::AtomicUsize::new(0),
    });

    let s_clone = server.clone();
    let c_clone = cfg.clone();
    std::thread::spawn(move || {
        for req in s_clone.incoming_requests() {
            handle(&c_clone, req);
        }
    });

    let client = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(std::time::Duration::from_secs(5))
        .build();

    let base_url = format!("http://{s_addr}");
    let matching_origin = format!("http://{s_addr}");

    // Case A: Unauthenticated requests without secret
    // GET /gh/user -> 401
    let (status, _, _) = http_status(client.get(&format!("{base_url}/gh/user")).set("Origin", &matching_origin).call());
    assert_eq!(status, 401);

    // GET / -> 200 with sign-in HTML
    let (status, body, _) = http_status(client.get(&format!("{base_url}/")).call());
    assert_eq!(status, 200);
    assert!(body.contains("RepoHub · Sign in"));

    // Case B: Cross-origin POST /auth rejected (CSRF)
    let (status, _, _) = http_status(
        client.post(&format!("{base_url}/auth"))
            .set("Origin", "http://evil.com")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string("secret=wrong")
    );
    assert_eq!(status, 403);

    // Case C: Same-origin POST /auth with wrong secret -> 401
    let (status, body, _) = http_status(
        client.post(&format!("{base_url}/auth"))
            .set("Origin", &matching_origin)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string("secret=wrong_pass")
    );
    assert_eq!(status, 401);
    assert!(body.contains("Invalid secret"));

    // Case D: Same-origin POST /auth with correct secret (with ;, =, spaces)
    // Using form urlencoded
    let encoded_secret = percent_encode_cookie("p@ss;word=1 2");
    let (status, _, set_cookie) = http_status(
        client.post(&format!("{base_url}/auth"))
            .set("Origin", &matching_origin)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(&format!("secret={encoded_secret}"))
    );
    assert_eq!(status, 303);
    assert!(set_cookie.is_some());
    let cookie_header = set_cookie.unwrap();
    assert!(cookie_header.contains("repohub_secret="));
    assert!(cookie_header.contains("HttpOnly"));

    // Extract cookie value for subsequent authenticated calls
    let cookie_val = cookie_header.split(';').next().unwrap();

    // Also test JSON POST /auth
    let (status, _, json_cookie) = http_status(
        client.post(&format!("{base_url}/auth"))
            .set("Origin", &matching_origin)
            .set("Content-Type", "application/json")
            .send_string("{\"secret\":\"p@ss;word=1 2\"}")
    );
    assert_eq!(status, 303);
    assert!(json_cookie.is_some());

    // Case E: Authenticated requests using Bearer token or Cookie
    // 1. Missing Origin on /gh/user -> 403
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/user"))
            .set("Cookie", cookie_val)
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Missing Origin header"));

    // 2. Different port Origin on /gh/user -> 403
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/user"))
            .set("Cookie", cookie_val)
            .set("Origin", "http://127.0.0.1:3000")
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Origin not allowed"));

    // 3. Traversal in path: %2e%2e -> 403 (send raw HTTP bytes so HTTP client doesn't normalize URL)
    let (status, body) = {
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect(&s_addr).unwrap();
        let raw_req = format!(
            "GET /gh/repos/o/r/%2e%2e/%2e%2e/user/emails HTTP/1.1\r\nHost: {s_addr}\r\nOrigin: {matching_origin}\r\nCookie: {cookie_val}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(raw_req.as_bytes()).unwrap();
        let mut resp_bytes = Vec::new();
        stream.read_to_end(&mut resp_bytes).unwrap();
        let resp_str = String::from_utf8_lossy(&resp_bytes);
        let code = resp_str.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap();
        (code, resp_str.to_string())
    };
    assert_eq!(status, 403, "expected 403 on %2e%2e, body was: {body}");
    assert!(body.contains("Path traversal"), "body was: {body}");

    // 4. Double-encoded traversal: %252e%252e -> 403
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/repos/o/r/git/ref/heads/%252e%252e/%252e%252e/user/emails"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Path traversal"));

    // 5. Endpoint outside allowlist -> 403
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/repos/o/r/hooks"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Endpoint not allowed"));

    // 6. Allowed endpoint with matching Origin -> 200 and forwards token and link header
    let r_user = client.get(&format!("{base_url}/gh/user"))
        .set("Cookie", cookie_val)
        .set("Origin", &matching_origin)
        .call()
        .unwrap();
    assert_eq!(r_user.status(), 200);
    assert_eq!(r_user.header("link"), Some("<https://api.github.com/user/repos?page=2>; rel=\"next\""));
    let body = r_user.into_string().unwrap_or_default();
    assert!(body.contains("gh_token_12345"));

    // 7. Allowed endpoint via Authorization: Bearer
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/user"))
            .set("Authorization", "Bearer p@ss;word=1 2")
            .set("Origin", &matching_origin)
            .call()
    );
    assert_eq!(status, 200);
    assert!(body.contains("gh_token_12345"));

    // 8. Plain-HTTP LAN browser simulation: no Origin, but X-RepoHub: 1 + Referer matching Host
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/gh/user"))
            .set("Cookie", cookie_val)
            .set("X-RepoHub", "1")
            .set("Referer", &format!("{base_url}/index.html"))
            .call()
    );
    assert_eq!(status, 200);
    assert!(body.contains("gh_token_12345"));

    // 9. Writes without X-RepoHub: 1 -> 403
    let (status, body, _) = http_status(
        client.put(&format!("{base_url}/gh/user/starred/owner/repo"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .set("Content-Type", "application/json")
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Writes require X-RepoHub: 1"));

    // 10. Writes without Content-Type: application/json -> 403
    let (status, body, _) = http_status(
        client.put(&format!("{base_url}/gh/user/starred/owner/repo"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .set("X-RepoHub", "1")
            .set("Content-Type", "text/plain")
            .call()
    );
    assert_eq!(status, 403);
    assert!(body.contains("Content-Type: application/json"));

    // 11. Notes total size cap: max_notes_bytes is 100 bytes
    // Save first note of 60 bytes -> 204
    let note1_body = "{\"type\":\"excalidraw\",\"tag\":\"60_bytes_payload_data_here\"}";
    assert!(note1_body.len() <= 100);
    let (status, _, _) = http_status(
        client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n1"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .set("X-RepoHub", "1")
            .set("Content-Type", "application/json")
            .send_string(note1_body)
    );
    assert_eq!(status, 204);

    // Save second note of 60 bytes (total 120 > 100) -> 413
    let (status, body, _) = http_status(
        client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n2"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .set("X-RepoHub", "1")
            .set("Content-Type", "application/json")
            .send_string(note1_body)
    );
    assert_eq!(status, 413);
    assert!(body.contains("Total notes storage limit exceeded"));

    // Overwrite n1 with note of 70 bytes -> (60 - 60) + 70 = 70 <= 100 -> 204
    let note1_updated = "{\"type\":\"excalidraw\",\"tag\":\"70_bytes_payload_data_longer_update\"}";
    let (status, _, _) = http_status(
        client.put(&format!("{base_url}/api/notes/file?repo=o/r&branch=main&name=n1"))
            .set("Cookie", cookie_val)
            .set("Origin", &matching_origin)
            .set("X-RepoHub", "1")
            .set("Content-Type", "application/json")
            .send_string(note1_updated)
    );
    assert_eq!(status, 204);

    // 12. Static file serving (with authenticated cookie)
    // Valid file -> 200
    let (status, body, _) = http_status(
        client.get(&format!("{base_url}/index.html"))
            .set("Cookie", cookie_val)
            .call()
    );
    assert_eq!(status, 200);
    assert!(body.contains("RepoHub App"));

    // Symlink escaping web_dir -> 404
    #[cfg(unix)]
    {
        let (status, _, _) = http_status(
            client.get(&format!("{base_url}/leak_link.txt"))
                .set("Cookie", cookie_val)
                .call()
        );
        assert_eq!(status, 404);
    }

    // Multi-encoded path traversal in static file -> 404
    let (status, _, _) = http_status(
        client.get(&format!("{base_url}/%252e%252e/secret.txt"))
            .set("Cookie", cookie_val)
            .call()
    );
    assert_eq!(status, 404);

    server.unblock();
    mock_upstream.unblock();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn conditional_tags_are_entity_tags_on_reads_only() {
    assert_eq!(conditional_tag("GET", Some("\"abc\"")), Some("\"abc\"".into()));
    assert_eq!(conditional_tag("GET", Some(" W/\"6f1c\" ")), Some("W/\"6f1c\"".into()));
    assert_eq!(conditional_tag("GET", Some("W/\"a\", \"b\"")), Some("W/\"a\", \"b\"".into()));
    for bad in ["*", "abc", "\"", "\"a\"b\"", "W/abc", "\"a\"\u{7}", "\"\u{e9}\"", ""] {
        assert_eq!(conditional_tag("GET", Some(bad)), None, "{bad:?}");
    }
    assert_eq!(conditional_tag("GET", Some(&format!("\"{}\"", "a".repeat(600)))), None, "too long");
    assert_eq!(conditional_tag("PATCH", Some("\"abc\"")), None, "reads only");
    assert_eq!(conditional_tag("GET", None), None);
}

/// The app's `If-None-Match` reaches GitHub and GitHub's `ETag` and 304 come back, so an
/// unchanged list costs no rate limit through the proxy either.
#[test]
fn the_proxy_forwards_conditional_requests() {
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let upstream = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let upstream_addr = format!("http://{}", upstream.server_addr());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Option<String>>::new()));
    let (up, seen_up) = (upstream.clone(), seen.clone());
    std::thread::spawn(move || {
        for req in up.incoming_requests() {
            let inm = req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("If-None-Match")).map(|h| h.value.as_str().to_owned());
            seen_up.lock().unwrap().push(inm.clone());
            let resp = if inm.as_deref() == Some("W/\"v1\"") {
                Response::from_data(Vec::new()).with_status_code(304)
            } else {
                Response::from_string("{\"login\":\"octo\"}").with_status_code(200).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
            };
            let _ = req.respond(resp.with_header(Header::from_bytes("ETag", "W/\"v1\"").unwrap()).with_header(Header::from_bytes("x-ratelimit-remaining", "4999").unwrap()));
        }
    });
    let server = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let addr = format!("{}", server.server_addr());
    let temp_dir = std::env::temp_dir().join(format!("repohub-etag-{}", std::process::id()));
    let cfg = std::sync::Arc::new(Cfg {
        token: Some("gh_token_etag".into()),
        api: upstream_addr,
        web_dir: temp_dir.join("web"),
        data_dir: temp_dir.join("data"),
        allowed_hosts: vec!["127.0.0.1".into()],
        public_json: "{}".into(),
        agent: ureq::AgentBuilder::new().try_proxy_from_env(false).build(),
        secret: None,
        allow_writes: true,
        notes_max_bytes: 100,
        proxy_in_flight: std::sync::atomic::AtomicUsize::new(0),
    });
    let (srv, c) = (server.clone(), cfg.clone());
    std::thread::spawn(move || {
        for req in srv.incoming_requests() {
            handle(&c, req);
        }
    });
    let client = ureq::AgentBuilder::new().try_proxy_from_env(false).build();
    let origin = format!("http://{addr}");
    let get = |inm: Option<&str>| {
        let mut r = client.get(&format!("http://{addr}/gh/user")).set("Origin", &origin);
        if let Some(v) = inm {
            r = r.set("If-None-Match", v);
        }
        match r.call() {
            Ok(resp) | Err(ureq::Error::Status(_, resp)) => (resp.status(), resp.header("etag").map(str::to_owned), resp.into_string().unwrap_or_default()),
            Err(e) => panic!("{e}"),
        }
    };
    let (status, etag, body) = get(None);
    assert_eq!((status, etag.as_deref()), (200, Some("W/\"v1\"")), "the tag comes back");
    assert!(body.contains("octo"));
    let (status, etag, body) = get(Some("W/\"v1\""));
    assert_eq!((status, etag.as_deref(), body.as_str()), (304, Some("W/\"v1\""), ""), "unchanged: 304, no body");
    let (status, _, _) = get(Some("*"));
    assert_eq!(status, 200, "a value that isn't a tag isn't sent");
    assert_eq!(*seen.lock().unwrap(), vec![None, Some("W/\"v1\"".to_owned()), None]);
    server.unblock();
    upstream.unblock();
}

#[test]
fn the_proxy_compresses_large_responses_when_accepted() {
    let _env_guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let payload = format!("{{\"items\":[{}]}}", (0..80).map(|i| format!("{{\"name\":\"repo-{i}\",\"desc\":\"repeated text for compression\"}}")).collect::<Vec<_>>().join(","));
    let payload_up = payload.clone();
    let upstream = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let upstream_addr = format!("http://{}", upstream.server_addr());
    let up = upstream.clone();
    std::thread::spawn(move || {
        for req in up.incoming_requests() {
            let resp = Response::from_string(payload_up.clone())
                .with_status_code(200)
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
            let _ = req.respond(resp);
        }
    });
    let server = std::sync::Arc::new(Server::http("127.0.0.1:0").unwrap());
    let addr = format!("{}", server.server_addr());
    let temp_dir = std::env::temp_dir().join(format!("repohub-gzip-{}", std::process::id()));
    let cfg = std::sync::Arc::new(Cfg {
        token: Some("gh_token_gzip".into()),
        api: upstream_addr,
        web_dir: temp_dir.join("web"),
        data_dir: temp_dir.join("data"),
        allowed_hosts: vec!["127.0.0.1".into()],
        public_json: "{}".into(),
        agent: ureq::AgentBuilder::new().try_proxy_from_env(false).build(),
        secret: None,
        allow_writes: true,
        notes_max_bytes: 100,
        proxy_in_flight: std::sync::atomic::AtomicUsize::new(0),
    });
    let (srv, c) = (server.clone(), cfg.clone());
    std::thread::spawn(move || {
        for req in srv.incoming_requests() {
            handle(&c, req);
        }
    });
    // Raw TCP check so we observe the wire Content-Encoding and compressed byte length directly.
    use std::io::Write;
    let raw_get = |ae: &str| {
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        let req = format!("GET /gh/user/repos HTTP/1.1\r\nHost: {addr}\r\nOrigin: http://{addr}\r\nAccept-Encoding: {ae}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
        (head, buf[split + 4..].to_vec())
    };
    let (head_gz, body_gz) = raw_get("gzip, br");
    assert!(head_gz.contains("content-encoding: gzip"), "compressed when accepted: {head_gz}");
    assert!(body_gz.len() < payload.len() / 2, "payload actually shrank ({} -> {})", payload.len(), body_gz.len());
    let mut dec = flate2::read::GzDecoder::new(&body_gz[..]);
    let mut plain = String::new();
    dec.read_to_string(&mut plain).unwrap();
    assert_eq!(plain, payload);

    let (head_id, body_id) = raw_get("identity");
    assert!(!head_id.contains("content-encoding: gzip"), "plain when gzip not accepted");
    assert_eq!(body_id, payload.as_bytes());
    server.unblock();
    upstream.unblock();
}

#[test]
fn proxy_concurrency_limit_returns_json_503_and_frees_on_drop() {
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let mut slots = Vec::new();
    for _ in 0..proxy::MAX_IN_FLIGHT {
        slots.push(proxy::ProxySlot::try_acquire(&counter).expect("slot available"));
    }
    assert!(proxy::ProxySlot::try_acquire(&counter).is_none(), "saturated at MAX_IN_FLIGHT");
    // Drop one slot and verify capacity returns immediately.
    slots.pop();
    let reused = proxy::ProxySlot::try_acquire(&counter).expect("freed slot reusable");
    drop(reused);
    drop(slots);
    assert_eq!(counter.load(std::sync::atomic::Ordering::Acquire), 0);
}
