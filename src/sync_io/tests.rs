use super::*;

#[test]
fn repository_names_as_typed() {
    assert_eq!(parse_repo("octo/repohub-notes", "me").as_deref(), Some("octo/repohub-notes"));
    assert_eq!(parse_repo(" repohub-notes ", "me").as_deref(), Some("me/repohub-notes"));
    assert_eq!(parse_repo("https://github.com/octo/notes.git", "me").as_deref(), Some("octo/notes"));
    for bad in ["", "a/b/c", "../x", "o/.git", "o/a b", "/x"] {
        assert_eq!(parse_repo(bad, "me"), None, "{bad}");
    }
    assert_eq!(create_url(&Config::from_json(r#"{"user":"me"}"#).unwrap()), "https://github.com/new?name=repohub-notes&visibility=private");
}

#[test]
fn repository_answers_and_trees() {
    let v = json!({ "full_name": "me/repohub-notes", "private": true, "default_branch": "main", "permissions": { "push": true } });
    assert_eq!(parse_repo_info(&v), RepoInfo { full: "me/repohub-notes".into(), private: true, branch: "main".into(), push: true });
    let public = parse_repo_info(&json!({ "full_name": "me/x", "private": false, "default_branch": "trunk" }));
    assert!(!public.private && !public.push);
    let tree = json!({ "sha": "t", "truncated": false, "tree": [
        { "path": "README.md", "type": "blob", "sha": "a" },
        { "path": "boards", "type": "tree", "sha": "b" },
        { "path": "boards/o/r/main/Plan.excalidraw", "type": "blob", "sha": "c" },
        { "path": "boards/o/r/feature%2Fx/Map.excalidraw", "type": "blob", "sha": "d" },
    ] });
    let l = parse_tree(&tree.to_string()).unwrap();
    assert!(l.complete);
    assert_eq!(l.files.len(), 2);
    assert_eq!(l.files["boards/o/r/feature%2Fx/Map.excalidraw"], "d");
    assert!(!parse_tree(r#"{"tree":[],"truncated":true}"#).unwrap().complete);
    assert!(simple_branch("main") && simple_branch("release-1.2") && !simple_branch("feature/x") && !simple_branch(".x"));
}

#[test]
fn contents_come_raw_or_as_githubs_json() {
    let board = r#"{"type":"excalidraw","elements":[]}"#;
    assert_eq!(raw_content(board.into()).unwrap(), board);
    let wrapped = json!({ "type": "file", "encoding": "base64", "sha": "x", "content": format!("{}\n", sync::base64_encode(board.as_bytes())) }).to_string();
    assert_eq!(raw_content(wrapped).unwrap(), board);
    assert!(raw_content(r#"{"type":"file","encoding":"base64","content":"!!"}"#.into()).is_err());
}

/// GitHub's Contents and Git Trees API in memory, over HTTP on localhost: enough of it for
/// sync (the same status codes for a stale or missing `sha`).
struct MockGitHub {
    api: String,
    files: std::sync::Arc<std::sync::Mutex<BTreeMap<String, String>>>,
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn mock_github() -> MockGitHub {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let files = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::<String, String>::new()));
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (f, l) = (files.clone(), log.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, target) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
            let (mut len, mut raw) = (0usize, false);
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("accept:") && lower.contains("raw") {
                    raw = true;
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            l.lock().unwrap().push(format!("{method} {target}"));
            let path_only = target.split('?').next().unwrap_or("");
            let mut files = f.lock().unwrap();
            let (code, answer) = match (method.as_str(), path_only.strip_prefix("/repos/me/notes")) {
                ("GET", Some("")) => (200, json!({ "full_name": "me/notes", "private": true, "default_branch": "main", "permissions": { "push": true } }).to_string()),
                ("GET", Some("/git/trees/main")) if files.is_empty() => (409, json!({ "message": "Git Repository is empty." }).to_string()),
                ("GET", Some("/git/trees/main")) => {
                    let tree: Vec<Value> = files.iter().map(|(p, c)| json!({ "path": p, "type": "blob", "sha": sync::blob_sha(c.as_bytes()) })).collect();
                    (200, json!({ "tree": tree, "truncated": false }).to_string())
                }
                (m, Some(rest)) if rest.starts_with("/contents/") => {
                    let path = percent_decode(&rest["/contents/".len()..]);
                    let now = files.get(&path).map(|c| sync::blob_sha(c.as_bytes()));
                    let sha = body.get("sha").and_then(Value::as_str).map(str::to_owned);
                    match m {
                        "GET" => match files.get(&path) {
                            Some(c) if raw => (200, c.clone()),
                            Some(c) => (200, json!({ "type": "file", "encoding": "base64", "content": sync::base64_encode(c.as_bytes()) }).to_string()),
                            None => (404, json!({ "message": "Not Found" }).to_string()),
                        },
                        "PUT" if now.is_some() && sha.is_none() => (422, json!({ "message": "\"sha\" wasn't supplied." }).to_string()),
                        "PUT" if now.is_some() && sha != now => (409, json!({ "message": "is at another sha" }).to_string()),
                        "PUT" => {
                            let content = String::from_utf8(sync::base64_decode(body["content"].as_str().unwrap_or("")).unwrap()).unwrap();
                            let new = sync::blob_sha(content.as_bytes());
                            files.insert(path, content);
                            l.lock().unwrap().push(format!("commit {}", body["message"].as_str().unwrap_or("")));
                            (if now.is_some() { 200 } else { 201 }, json!({ "content": { "sha": new } }).to_string())
                        }
                        "DELETE" if now.is_none() => (404, json!({ "message": "Not Found" }).to_string()),
                        "DELETE" if sha != now => (409, json!({ "message": "is at another sha" }).to_string()),
                        "DELETE" => {
                            files.remove(&path);
                            l.lock().unwrap().push(format!("commit {}", body["message"].as_str().unwrap_or("")));
                            (200, "{}".to_owned())
                        }
                        _ => (405, "{}".to_owned()),
                    }
                }
                _ => (404, json!({ "message": "Not Found" }).to_string()),
            };
            drop(files);
            let mut out = stream;
            let _ = write!(out, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len());
        }
    });
    MockGitHub { api, files, log }
}

/// Two devices (two data folders) and a mock GitHub, through the real client and storage:
/// a board drawn on one opens on the other; changed on both offline, both sets of strokes
/// are there after sync; a deletion follows.
#[test]
fn two_devices_through_a_mock_github() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Puts REPOHUB_DATA_DIR back afterwards, whatever happens.
    let _restore = crate::load_gen_tests::TempDataDir::new("two-devices-env");
    let gh = mock_github();
    let cfg = Config::from_json(&json!({ "user": "me", "hasToken": true, "api": gh.api }).to_string()).unwrap();
    let base = std::env::temp_dir().join(format!("repohub-two-devices-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let device = |name: &str| {
        // SAFETY: ENV_LOCK keeps other tests from reading the environment meanwhile.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", base.join(name)) };
    };
    let sync_on = |name: &str| {
        device(name);
        let (r, private) = pollster::block_on(run(&cfg, "me/notes"));
        assert_eq!(r.error, None, "{name}");
        assert_eq!(private, Some(true));
        r
    };
    let stroke = |id: &str, version: u64, x: f64| json!({ "id": id, "type": "freedraw", "x": x, "y": 1, "width": 10, "height": 10, "points": [[0, 0], [10, 10]], "version": version, "updated": version as i64 * 1000 + x as i64, "isDeleted": false, "seed": 3 });
    let board = |els: Vec<Value>| json!({ "type": "excalidraw", "version": 2, "elements": els, "appState": {}, "files": {} }).to_string();
    let (repo, branch) = ("Octo/App", "feature/x y");
    // Drawn on the laptop.
    device("laptop");
    pollster::block_on(notes::save(repo, branch, "Plan (v2)", board(vec![stroke("a", 1, 1.0)]))).unwrap();
    assert_eq!(sync_on("laptop").uploaded, 1);
    assert!(gh.files.lock().unwrap().contains_key("boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw"));
    assert!(gh.log.lock().unwrap().iter().any(|l| l == "PUT /repos/me/notes/contents/boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw"), "{:?}", gh.log.lock().unwrap());
    assert!(gh.log.lock().unwrap().iter().any(|l| l == "commit Update Plan (v2) (octo/app@feature/x y)"));
    // Opened on the phone: it's there.
    let r = sync_on("phone");
    assert_eq!(r.downloaded, 1);
    device("phone");
    let on_phone = pollster::block_on(notes::load(repo, branch, "Plan (v2)")).unwrap().expect("the board is on the phone");
    device("laptop");
    assert_eq!(Some(on_phone), pollster::block_on(notes::load(repo, branch, "Plan (v2)")).unwrap());
    // Both change it while offline.
    device("phone");
    pollster::block_on(notes::save(repo, branch, "Plan (v2)", board(vec![stroke("a", 1, 1.0), stroke("phone", 1, 2.0)]))).unwrap();
    device("laptop");
    pollster::block_on(notes::save(repo, branch, "Plan (v2)", board(vec![stroke("a", 2, 5.0), stroke("laptop", 1, 3.0)]))).unwrap();
    assert_eq!(sync_on("phone").uploaded, 1);
    assert_eq!(sync_on("laptop").merged, 1);
    assert_eq!(sync_on("phone").downloaded, 1);
    for name in ["phone", "laptop"] {
        device(name);
        let doc: Value = serde_json::from_str(&pollster::block_on(notes::load(repo, branch, "Plan (v2)")).unwrap().unwrap()).unwrap();
        let ids: BTreeSet<&str> = doc["elements"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "laptop", "phone"].into(), "{name}");
        let a = doc["elements"].as_array().unwrap().iter().find(|e| e["id"] == "a").unwrap();
        assert_eq!(a["x"], 5.0, "{name}: the newer version of the shared stroke");
    }
    // Deleted on the phone: deleted on GitHub, then on the laptop (into its trash).
    device("phone");
    pollster::block_on(notes::delete(repo, branch, "Plan (v2)")).unwrap();
    assert_eq!(sync_on("phone").deleted_remote, 1);
    assert!(gh.files.lock().unwrap().is_empty());
    assert_eq!(sync_on("laptop").deleted_local, 1);
    device("laptop");
    assert_eq!(pollster::block_on(notes::load(repo, branch, "Plan (v2)")).unwrap(), None);
    assert!(base.join("laptop/trash").join(sync::date(net::now())).join("octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw").is_file(), "kept in the trash");
    // In step: nothing more to do on either.
    for name in ["phone", "laptop"] {
        let r = sync_on(name);
        assert_eq!((r.uploaded, r.downloaded, r.merged, r.deleted_local, r.deleted_remote), (0, 0, 0, 0, 0), "{name}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// **Stop syncing** while a run goes: that run saves its index when it ends, so the index is
/// forgotten after it, not before (otherwise a board deleted here while sync was off would
/// later be deleted from the repository too). And a run never starts while another goes.
#[test]
fn stop_during_a_run_forgets_its_index_after_it_and_runs_never_overlap() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dir = crate::load_gen_tests::TempDataDir::new("sync-stop");
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data.sample = false;
    s.data_changed();
    let mut cfg = (*s.cfg).clone();
    cfg.token = Some("t".into());
    cfg.allow_writes = true;
    cfg.notes = true;
    s.cfg = std::sync::Arc::new(cfg);
    assert_eq!(s.sync_blocked(), None);
    let repo = format!("{}/repohub-notes", s.data.profile.login);
    s.sync.account = Some(s.data.profile.login.clone());
    s.sync.repo = Some(repo.clone());
    // A run is going: started earlier, its answer not back yet.
    (s.sync.status, s.sync.busy, s.sync.run) = (BoardSync::Running, true, 7);
    let epoch = s.load_gen;
    s.stop_syncing();
    // It ends: it saves what it synced, then its answer arrives.
    let index = format!(r#"{{"version":1,"repo":"{repo}","boards":{{"boards/o/r/main/Plan.excalidraw":{{"local":"{0}","remote":"{0}"}}}}}}"#, "a".repeat(40));
    pollster::block_on(notes::save_index(&repo, &index)).unwrap();
    s.apply_sync(epoch, 7, &repo, (Report::default(), Some(true)));
    assert_eq!(pollster::block_on(notes::load_index(&repo)).unwrap(), "", "what that run synced is forgotten too");
    assert!(!s.sync.busy && s.sync.status == BoardSync::Off);

    // Another account's data arrives while a run goes: its run waits for that one.
    save_choice("someone-else", Some("someone-else/notes"));
    s.sync.repo = Some(repo.clone());
    (s.sync.status, s.sync.busy, s.sync.again) = (BoardSync::Running, true, false);
    let run = s.sync.run;
    s.data.profile.login = "someone-else".into();
    s.sync_after_load();
    assert_eq!(s.sync.repo.as_deref(), Some("someone-else/notes"));
    assert!(s.sync.again, "the next run waits for the one going");
    assert_eq!(s.sync.run, run + 1, "the run going is now an old one; no other started");
}

#[test]
fn the_choice_is_saved_per_account() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dir = crate::load_gen_tests::TempDataDir::new("sync-choice");
    assert_eq!(saved_choice("Alice"), None);
    save_choice("Alice", Some("alice/repohub-notes"));
    save_choice("bob", Some("bob/notes"));
    assert_eq!(saved_choice("alice").as_deref(), Some("alice/repohub-notes"));
    save_choice("alice", None);
    assert_eq!(saved_choice("alice"), None);
    assert_eq!(saved_choice("bob").as_deref(), Some("bob/notes"));
}
