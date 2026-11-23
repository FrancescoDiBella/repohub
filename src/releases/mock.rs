use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// What the mock was asked: "METHOD path", and the body.
pub type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// Answers each request with `answer(method, path, body)`: status, extra headers, body.
pub fn serve(answer: impl Fn(&str, &str, &str) -> (u16, String, String) + Send + Sync + 'static) -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break buf.len(),
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let length = head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.trim().parse::<usize>().ok())).unwrap_or(0);
            while buf.len() < head_end + length {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
            let mut words = head.split_whitespace();
            let (method, path) = (words.next().unwrap_or("").to_owned(), words.next().unwrap_or("/").to_owned());
            log.lock().unwrap().push((format!("{method} {path}"), body.clone()));
            let (status, headers, out) = answer(&method, &path, &body);
            let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{out}", out.len());
        }
    });
    (url, seen)
}

/// A live account that talks to the mock at `url`.
pub fn config(url: &str) -> crate::config::Config {
    let mut cfg = crate::config::Config::from_json("{}").unwrap();
    cfg.api = url.to_owned();
    cfg.token = Some("test".into());
    cfg
}

/// A release as GitHub answers it.
pub fn release(id: u64, tag: &str, draft: bool, pre: bool, body: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "tag_name": tag, "name": "", "body": body, "draft": draft, "prerelease": pre,
        "author": { "login": "ada" }, "created_at": "2026-11-20T10:00:00Z",
        "published_at": if draft { serde_json::Value::Null } else { "2026-11-21T10:00:00Z".into() },
        "target_commitish": "main", "html_url": format!("https://github.com/o/r/releases/tag/{tag}"), "assets": [],
    })
}
