use std::cell::RefCell;

use super::*;

#[test]
fn hashes_match_git() {
    let hex = |b: [u8; 20]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(hex(sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(hex(sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    let long = "a".repeat(1000);
    assert_eq!(hex(sha1(long.as_bytes())), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    // `printf 'hello\n' | git hash-object --stdin`, and the empty blob.
    assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
}

#[test]
fn base64_round_trips_and_reads_githubs_line_breaks() {
    for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
        assert_eq!(base64_decode(&base64_encode(s.as_bytes())).unwrap(), s.as_bytes());
    }
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(base64_decode(&base64_encode(&all)).unwrap(), all);
    assert!(base64_decode("Zm9v!").is_none());
    assert!(base64_decode("Zm8=Zm8=").is_none());
}

#[test]
fn dates() {
    assert_eq!(date(0), "1970-01-01");
    assert_eq!(date(1_795_000_000), "2026-11-18");
    assert_eq!(date(951_782_400), "2000-02-29");
}

#[test]
fn paths_encode_odd_branches_and_read_back() {
    let k = Key::new("Octo/App", "feature/x y", "Plan (v2)");
    assert_eq!(k.repo, "octo/app");
    assert_eq!(k.path(), "boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw");
    assert_eq!(url_path(&k.path()), "boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw");
    assert_eq!(Key::parse(&k.path()), Some(k));
    // The same encoding as the local folder: a branch named `..` stays one folder.
    let dots = Key::new("o/r", "..", "x");
    assert_eq!(dots.path(), "boards/o/r/%2E./x.excalidraw");
    assert_eq!(Key::parse(&dots.path()), Some(dots));
    let dot_repo = Key::new("octo/.github", "main", "Ideas");
    assert_eq!(dot_repo.path(), "boards/octo/%2Egithub/main/Ideas.excalidraw");
    assert_eq!(Key::parse(&dot_repo.path()), Some(dot_repo));
    let unicode = Key::new("o/r", "café/ü", "Über");
    assert_eq!(Key::parse(&unicode.path()), Some(unicode));
    // Files RepoHub didn't write are left alone.
    for p in ["README.md", "boards/o/r/main/x.json", "boards/o/r/x.excalidraw", "boards/o/r/main/sub/x.excalidraw",
        "boards/O/r/main/x.excalidraw", "boards/o/r/feature/x/x.excalidraw", "boards/o/r/main/a%2Fb.excalidraw",
        "boards/o/r/main/.hidden.excalidraw", "boards/o/r/main/%2Ehidden.excalidraw", "other/o/r/main/x.excalidraw", "boards/o/r//x.excalidraw"] {
        assert_eq!(Key::parse(p), None, "{p}");
    }
    assert_eq!(message("Update", &Key::new("o/r", "main", "Plan")), "Update Plan (o/r@main)");
}

#[test]
fn conflict_names_are_valid_and_unique() {
    let n = conflict_name("Plan", "2026-11-22", |_| false);
    assert_eq!(n, "Plan (conflict 2026-11-22)");
    assert!(notes::valid_name(&n));
    let n2 = conflict_name("Plan", "2026-11-22", |x| x == "Plan (conflict 2026-11-22)");
    assert_eq!(n2, "Plan (conflict 2026-11-22 2)");
    let long = conflict_name(&"x".repeat(80), "2026-11-22", |_| false);
    assert!(long.len() <= 80 && notes::valid_name(&long), "{long}");
}

#[test]
fn index_round_trips_and_shrugs_off_junk() {
    let mut i = Index { repo: "me/repohub-notes".into(), boards: BTreeMap::new() };
    i.record("boards/o/r/main/x.excalidraw", &"a".repeat(40), &"b".repeat(40));
    i.boards.insert("boards/o/r/main/y.excalidraw".into(), Entry { local: None, remote: Some("c".repeat(40)) });
    assert_eq!(Index::from_json(&i.to_json()), i);
    assert_eq!(Index::from_json("not json"), Index::default());
    let odd = Index::from_json(r#"{"repo":"a/b","boards":{"p":{"local":"../x","remote":7}}}"#);
    assert_eq!(odd.boards["p"], Entry::default());
}

/// An index entry that knows only one side (cut short while being written, or edited by
/// hand) never deletes the board on the side that has it.
#[test]
fn a_half_known_board_is_copied_never_deleted() {
    use Side::*;
    assert_eq!(decide(Unchanged, Absent, false), Action::Upload);
    assert_eq!(decide(Absent, Unchanged, false), Action::Download);
    for (l, r) in [(Unchanged, Absent), (Changed, Absent), (New, Absent), (Absent, Unchanged), (Absent, Changed), (Absent, New)] {
        assert!(!matches!(decide(l, r, false), Action::DeleteLocal | Action::DeleteRemote | Action::Forget), "{l:?} / {r:?}");
    }
    // Through the plan: the index knows the local file only, GitHub doesn't have it.
    let path = "boards/o/r/main/x.excalidraw".to_owned();
    let mut index = Index { repo: "me/notes".into(), boards: BTreeMap::new() };
    index.boards.insert(path.clone(), Entry { local: Some("a".repeat(40)), remote: None });
    let local = BTreeMap::from([(path.clone(), "a".repeat(40))]);
    let steps = plan(&index, &local, &BTreeMap::new(), true);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Upload);
}

#[test]
fn the_decision_table_all_sixteen_cells() {
    use Action::*;
    use Side::*;
    let sides = [Unchanged, Changed, Deleted, New];
    let expect = [
        // remote: unchanged, changed, deleted, new
        [Nothing, Download, DeleteLocal, Merge],   // local unchanged
        [Upload, Merge, Upload, Merge],            // local changed
        [DeleteRemote, Download, Forget, Download], // local deleted
        [Merge, Merge, Upload, Merge],             // local new
    ];
    for (li, l) in sides.iter().enumerate() {
        for (ri, r) in sides.iter().enumerate() {
            assert_eq!(decide(*l, *r, false), expect[li][ri], "local {l:?}, remote {r:?}");
        }
    }
    // Same bytes on both sides: just remember them.
    assert_eq!(decide(Changed, Changed, true), Record);
    assert_eq!(decide(New, New, true), Record);
    assert_eq!(decide(Unchanged, Unchanged, true), Nothing);
    // Never deleted against a change: a deletion only wins over an unchanged side.
    for l in sides {
        for r in sides {
            let a = decide(l, r, false);
            if a == DeleteLocal {
                assert_eq!(l, Unchanged);
            }
            if a == DeleteRemote {
                assert_eq!(r, Unchanged);
            }
        }
    }
}

#[test]
fn the_plan_reads_sides_from_the_index() {
    let a = Key::new("o/r", "main", "A").path();
    let b = Key::new("o/r", "main", "B").path();
    let c = Key::new("o/r", "main", "C").path();
    let mut index = Index::default();
    index.record(&a, &"1".repeat(40), &"1".repeat(40));
    index.record(&b, &"2".repeat(40), &"2".repeat(40));
    let local: BTreeMap<_, _> = [(a.clone(), "1".repeat(40)), (c.clone(), "3".repeat(40)), ("junk".to_owned(), "4".repeat(40))].into();
    let remote: BTreeMap<_, _> = [(a.clone(), "5".repeat(40)), (b.clone(), "2".repeat(40))].into();
    let steps = plan(&index, &local, &remote, true);
    let actions: Vec<_> = steps.iter().map(|s| (s.key.name.as_str(), s.action)).collect();
    assert_eq!(actions, vec![("A", Action::Download), ("B", Action::DeleteRemote), ("C", Action::Upload)]);
    // A tree GitHub cut short: nothing is deleted here for a board missing there.
    let partial = plan(&index, &[(a.clone(), "1".repeat(40))].into(), &BTreeMap::new(), false);
    assert_eq!(partial[0].action, Action::Nothing);
}

fn el(id: &str, version: u64, updated: i64, x: f64) -> Value {
    json!({ "id": id, "type": "rectangle", "x": x, "y": 0, "width": 10, "height": 10, "version": version, "versionNonce": 1, "updated": updated, "isDeleted": false })
}

pub(crate) fn doc(els: &[Value]) -> String {
    json!({ "type": "excalidraw", "version": 2, "source": "test", "elements": els, "appState": {}, "files": {} }).to_string()
}

fn ids(s: &str) -> Vec<(String, f64, bool)> {
    let v: Value = serde_json::from_str(s).unwrap();
    v["elements"].as_array().unwrap().iter().map(|e| (e["id"].as_str().unwrap().to_owned(), e["x"].as_f64().unwrap_or(0.0), deleted(e))).collect()
}

#[test]
fn merge_takes_the_union_and_the_newer_version() {
    let mine = doc(&[el("a", 2, 10, 1.0), el("b", 1, 5, 2.0), el("c", 3, 7, 3.0)]);
    let mut gone = el("c", 4, 9, 3.0);
    gone["isDeleted"] = json!(true);
    let theirs = doc(&[el("a", 1, 5, 9.0), el("b", 2, 6, 8.0), gone, el("d", 1, 1, 4.0)]);
    let m = merge(&mine, &theirs, Tie::KeepBoth).unwrap().unwrap();
    assert_eq!(ids(&m), vec![("a".into(), 1.0, false), ("b".into(), 8.0, false), ("c".into(), 3.0, true), ("d".into(), 4.0, false)]);
    // Merging again adds nothing; nor does merging a board with itself.
    assert_eq!(merge(&m, &theirs, Tie::KeepBoth).unwrap(), None);
    assert_eq!(merge(&mine, &mine, Tie::KeepBoth).unwrap(), None);
    // The same version: the later `updated` wins.
    let later = merge(&doc(&[el("a", 2, 10, 1.0)]), &doc(&[el("a", 2, 11, 5.0)]), Tie::KeepBoth).unwrap().unwrap();
    assert_eq!(ids(&later), vec![("a".into(), 5.0, false)]);
}

#[test]
fn merge_keeps_both_when_it_cant_tell() {
    // Two old boards (version 1, no time) whose ids happen to match: both strokes stay.
    let m = merge(&doc(&[el("rh2-1", 1, 0, 1.0)]), &doc(&[el("rh2-1", 1, 0, 2.0)]), Tie::KeepBoth).unwrap().unwrap();
    let got = ids(&m);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], ("rh2-1".into(), 1.0, false));
    assert!(got[1].0.starts_with("rh2-1~") && got[1].1 == 2.0);
    assert_eq!(merge(&m, &doc(&[el("rh2-1", 1, 0, 2.0)]), Tie::KeepBoth).unwrap(), None, "kept once, not again");
    // The same element written by two writers (its place in the list, `100` for `100.0`):
    // one element, not two.
    let mut moved = el("a", 3, 7, 100.0);
    moved["index"] = json!("a5");
    moved["x"] = json!(100);
    let mut other = el("a", 3, 7, 100.0);
    other["index"] = json!("a0");
    other["roundness"] = json!({ "type": 3 });
    assert_eq!(merge(&doc(&[moved]), &doc(&[other]), Tie::KeepBoth).unwrap(), None);
    // The editor against the file: its own content wins a tie.
    assert_eq!(merge(&doc(&[el("a", 1, 0, 1.0)]), &doc(&[el("a", 1, 0, 2.0)]), Tie::First).unwrap(), None);
    // Deleted on one side only at the same version: the drawn one stays.
    let mut d = el("a", 1, 0, 1.0);
    d["isDeleted"] = json!(true);
    let kept = merge(&doc(std::slice::from_ref(&d)), &doc(&[el("a", 1, 0, 1.0)]), Tie::KeepBoth).unwrap().unwrap();
    assert_eq!(ids(&kept), vec![("a".into(), 1.0, false)]);
    assert_eq!(merge(&doc(&[el("a", 1, 0, 1.0)]), &doc(&[d]), Tie::KeepBoth).unwrap(), None);
}

#[test]
fn merge_keeps_elements_it_cant_draw_and_joins_files() {
    // An element with no id matches the one the editor named from its content.
    let bare = json!({ "type": "rectangle", "x": 1, "y": 2, "version": 1 });
    let mut named = bare.clone();
    named["id"] = Value::from(derived_id(&bare));
    assert_eq!(merge(&doc(&[named]), &doc(std::slice::from_ref(&bare)), Tie::KeepBoth).unwrap(), None);
    assert_eq!(merge(&doc(std::slice::from_ref(&bare)), &doc(std::slice::from_ref(&bare)), Tie::KeepBoth).unwrap(), None);
    let image = json!({ "id": "img", "type": "image", "fileId": "f1", "version": 1, "updated": 1 });
    let theirs = json!({ "type": "excalidraw", "elements": [image], "files": { "f1": { "dataURL": "data:" } } }).to_string();
    let m = merge(&doc(&[el("a", 1, 1, 1.0)]), &theirs, Tie::KeepBoth).unwrap().unwrap();
    let v: Value = serde_json::from_str(&m).unwrap();
    assert_eq!(v["elements"][1]["type"], "image");
    assert!(v["files"]["f1"].is_object());
    assert!(merge("not json", &theirs, Tie::KeepBoth).is_err());
    assert!(merge(&theirs, "[1,2]", Tie::KeepBoth).is_err());
}

// -------------------------------------------------------------- fakes for the whole flow

/// GitHub in memory: path → content, with the same sha checks.
#[derive(Default)]
pub(crate) struct FakeRemote {
    pub files: RefCell<BTreeMap<String, String>>,
    /// Something else writes this right before our next write (a third device).
    pub race: RefCell<Option<(String, String)>>,
    pub offline: RefCell<bool>,
    pub writes: RefCell<usize>,
}

impl Remote for FakeRemote {
    async fn list(&self) -> Result<Listing, String> {
        if *self.offline.borrow() {
            return Err("Can't reach GitHub".into());
        }
        Ok(Listing { files: self.files.borrow().iter().map(|(p, c)| (p.clone(), blob_sha(c.as_bytes()))).collect(), complete: true })
    }
    async fn read(&self, path: &str) -> Result<Option<String>, String> {
        Ok(self.files.borrow().get(path).cloned())
    }
    async fn write(&self, path: &str, content: &str, sha: Option<&str>, _: &str) -> Result<String, RemoteError> {
        if let Some((p, c)) = self.race.borrow_mut().take() {
            self.files.borrow_mut().insert(p, c);
        }
        let now = self.files.borrow().get(path).map(|c| blob_sha(c.as_bytes()));
        if now.as_deref() != sha {
            return Err(RemoteError::Conflict);
        }
        *self.writes.borrow_mut() += 1;
        self.files.borrow_mut().insert(path.to_owned(), content.to_owned());
        Ok(blob_sha(content.as_bytes()))
    }
    async fn delete(&self, path: &str, sha: &str, _: &str) -> Result<(), RemoteError> {
        if let Some((p, c)) = self.race.borrow_mut().take() {
            self.files.borrow_mut().insert(p, c);
        }
        let now = self.files.borrow().get(path).map(|c| blob_sha(c.as_bytes()));
        if now.as_deref() != Some(sha) {
            return Err(RemoteError::Conflict);
        }
        self.files.borrow_mut().remove(path);
        Ok(())
    }
}

/// A device's notes in memory, with a trash.
#[derive(Default)]
pub(crate) struct FakeLocal {
    pub files: RefCell<BTreeMap<Key, String>>,
    pub trash: RefCell<Vec<(Key, String)>>,
    pub index: RefCell<Index>,
    /// The editor saves this right before our next local write.
    pub race: RefCell<Option<(Key, String)>>,
}

impl FakeLocal {
    fn race_now(&self) {
        if let Some((k, c)) = self.race.borrow_mut().take() {
            self.files.borrow_mut().insert(k, c);
        }
    }
}

impl Local for FakeLocal {
    async fn list(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(self.files.borrow().iter().map(|(k, c)| (k.path(), blob_sha(c.as_bytes()))).collect())
    }
    async fn read(&self, key: &Key) -> Result<Option<String>, String> {
        Ok(self.files.borrow().get(key).cloned())
    }
    async fn write_if(&self, key: &Key, expect: Option<&str>, content: &str) -> Result<bool, String> {
        self.race_now();
        let now = self.files.borrow().get(key).map(|c| blob_sha(c.as_bytes()));
        if now.as_deref() != expect {
            return Ok(false);
        }
        self.files.borrow_mut().insert(key.clone(), content.to_owned());
        Ok(true)
    }
    async fn trash_if(&self, key: &Key, expect: &str) -> Result<bool, String> {
        self.race_now();
        let now = self.files.borrow().get(key).map(|c| blob_sha(c.as_bytes()));
        if now.as_deref() != Some(expect) {
            return Ok(false);
        }
        let c = self.files.borrow_mut().remove(key).unwrap_or_default();
        self.trash.borrow_mut().push((key.clone(), c));
        Ok(true)
    }
    async fn load_index(&self) -> Result<Index, String> {
        Ok(self.index.borrow().clone())
    }
    async fn save_index(&self, index: &Index) -> Result<(), String> {
        *self.index.borrow_mut() = index.clone();
        Ok(())
    }
}

pub(crate) const REPO: &str = "me/repohub-notes";

pub(crate) fn sync(remote: &FakeRemote, local: &FakeLocal) -> Report {
    let opts = Options { max_bytes: 10 << 20, today: "2026-11-22".into() };
    pollster::block_on(run(remote, local, REPO, &opts))
}

fn key(name: &str) -> Key {
    Key::new("octo/app", "feature/x y", name)
}

/// Every element id of a document that isn't deleted.
fn live(s: &str) -> BTreeSet<String> {
    let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
    v["elements"].as_array().into_iter().flatten().filter(|e| !deleted(e)).filter_map(|e| e["id"].as_str().map(str::to_owned)).collect()
}

#[test]
fn two_devices_converge_and_keep_both_sets_of_strokes() {
    let github = FakeRemote::default();
    let (phone, laptop) = (FakeLocal::default(), FakeLocal::default());
    // Drawn on the phone; the laptop gets it.
    phone.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 1, 100, 1.0)]));
    let r = sync(&github, &phone);
    assert_eq!((r.uploaded, r.error.clone()), (1, None));
    let r = sync(&github, &laptop);
    assert_eq!(r.downloaded, 1);
    assert_eq!(laptop.files.borrow()[&key("Plan")], phone.files.borrow()[&key("Plan")]);
    assert_eq!(r.written.len(), 1, "the laptop's editor hears about it");
    // Both change it while offline.
    phone.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 2, 200, 5.0), el("p2", 1, 210, 2.0)]));
    laptop.files.borrow_mut().insert(key("Plan"), doc(&[el("p1", 1, 100, 1.0), el("l1", 1, 220, 3.0)]));
    assert_eq!(sync(&github, &phone).uploaded, 1);
    let r = sync(&github, &laptop);
    assert_eq!(r.merged, 1);
    assert_eq!(sync(&github, &phone).downloaded, 1);
    let all: BTreeSet<String> = ["p1", "p2", "l1"].map(String::from).into();
    for side in [&phone.files.borrow()[&key("Plan")], &laptop.files.borrow()[&key("Plan")], &github.files.borrow()[&key("Plan").path()]] {
        assert_eq!(live(side), all);
    }
    assert_eq!(ids(&phone.files.borrow()[&key("Plan")])[0].1, 5.0, "the newer version of p1");
    // In step now: another run does nothing.
    let r = sync(&github, &laptop);
    assert_eq!((r.uploaded, r.downloaded, r.merged), (0, 0, 0));
    assert!(r.synced.contains(&key("Plan")) && r.to_send.is_empty());
}

#[test]
fn a_deletion_follows_unless_the_other_side_changed_it() {
    let github = FakeRemote::default();
    let (a, b) = (FakeLocal::default(), FakeLocal::default());
    a.files.borrow_mut().insert(key("Old"), doc(&[el("o", 1, 1, 1.0)]));
    a.files.borrow_mut().insert(key("Kept"), doc(&[el("k", 1, 1, 1.0)]));
    sync(&github, &a);
    sync(&github, &b);
    // Deleted on a: gone on GitHub, then on b (into b's trash).
    a.files.borrow_mut().remove(&key("Old"));
    assert_eq!(sync(&github, &a).deleted_remote, 1);
    let r = sync(&github, &b);
    assert_eq!(r.deleted_local, 1);
    assert!(!b.files.borrow().contains_key(&key("Old")));
    assert_eq!(b.trash.borrow()[0].0, key("Old"), "kept in the trash");
    // Deleted on a, changed on b meanwhile: it comes back.
    a.files.borrow_mut().remove(&key("Kept"));
    b.files.borrow_mut().insert(key("Kept"), doc(&[el("k", 2, 5, 9.0)]));
    sync(&github, &b);
    let r = sync(&github, &a);
    assert_eq!(r.downloaded, 1, "changed there, so it comes back here");
    assert!(a.files.borrow().contains_key(&key("Kept")));
}

#[test]
fn a_write_refused_because_github_changed_is_retried_through_the_decision() {
    let github = FakeRemote::default();
    let a = FakeLocal::default();
    a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0)]));
    sync(&github, &a);
    a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0), el("mine", 1, 5, 2.0)]));
    // A third device writes right before our upload: GitHub refuses it, we merge.
    *github.race.borrow_mut() = Some((key("Plan").path(), doc(&[el("a", 1, 1, 1.0), el("theirs", 1, 6, 3.0)])));
    let r = sync(&github, &a);
    assert_eq!(r.error, None);
    assert_eq!(r.merged, 1);
    let ids: BTreeSet<String> = ["a", "mine", "theirs"].map(String::from).into();
    assert_eq!(live(&github.files.borrow()[&key("Plan").path()]), ids);
    assert_eq!(live(&a.files.borrow()[&key("Plan")]), ids);
}

#[test]
fn the_editor_saving_meanwhile_is_never_overwritten() {
    let github = FakeRemote::default();
    let a = FakeLocal::default();
    a.files.borrow_mut().insert(key("Plan"), doc(&[el("a", 1, 1, 1.0)]));
    sync(&github, &a);
    github.files.borrow_mut().insert(key("Plan").path(), doc(&[el("a", 1, 1, 1.0), el("remote", 1, 3, 2.0)]));
    // The editor saves a new stroke right before the download lands: the download waits.
    let drawn = doc(&[el("a", 1, 1, 1.0), el("drawn", 1, 4, 3.0)]);
    *a.race.borrow_mut() = Some((key("Plan"), drawn.clone()));
    let r = sync(&github, &a);
    assert_eq!(r.downloaded, 0);
    assert!(r.to_send.is_empty(), "a download waiting sends nothing");
    assert_eq!(a.files.borrow()[&key("Plan")], drawn);
    // Next time both changed: merged.
    let r = sync(&github, &a);
    assert_eq!(r.merged, 1);
    let ids: BTreeSet<String> = ["a", "remote", "drawn"].map(String::from).into();
    assert_eq!(live(&a.files.borrow()[&key("Plan")]), ids);
}

#[test]
fn boards_that_cant_be_merged_keep_both_copies() {
    let github = FakeRemote::default();
    let a = FakeLocal::default();
    a.files.borrow_mut().insert(key("Plan"), "{ broken".into());
    github.files.borrow_mut().insert(key("Plan").path(), doc(&[el("t", 1, 1, 1.0)]));
    let r = sync(&github, &a);
    assert_eq!(r.conflicts, vec!["Plan (conflict 2026-11-22)".to_owned()]);
    let copy = key("Plan (conflict 2026-11-22)");
    assert_eq!(live(&a.files.borrow()[&copy]), ["t".to_owned()].into());
    assert_eq!(github.files.borrow()[&key("Plan").path()], "{ broken");
    assert!(github.files.borrow().contains_key(&copy.path()));
    let r = sync(&github, &a);
    assert_eq!((r.uploaded, r.downloaded, r.merged, r.conflicts.len()), (0, 0, 0, 0), "in step afterwards");
}

#[test]
fn offline_says_what_waits_and_changes_nothing() {
    let github = FakeRemote::default();
    let a = FakeLocal::default();
    a.files.borrow_mut().insert(key("One"), doc(&[]));
    sync(&github, &a);
    a.files.borrow_mut().insert(key("One"), doc(&[el("x", 1, 1, 1.0)]));
    a.files.borrow_mut().insert(key("Two"), doc(&[]));
    *github.offline.borrow_mut() = true;
    let r = sync(&github, &a);
    assert!(r.error.is_some());
    assert_eq!(r.to_send.len(), 2);
    assert_eq!(a.index.borrow().boards.len(), 1, "the index is untouched");
}

#[test]
fn a_new_notes_repository_starts_from_an_empty_index() {
    let github = FakeRemote::default();
    let a = FakeLocal::default();
    a.files.borrow_mut().insert(key("One"), doc(&[]));
    sync(&github, &a);
    // The index belongs to another repository now: nothing is taken as deleted.
    a.index.borrow_mut().repo = "me/other".into();
    github.files.borrow_mut().clear();
    let r = sync(&github, &a);
    assert_eq!((r.uploaded, r.deleted_local), (1, 0));
}

/// Builds one cell of the decision table on a real run and checks that nothing drawn is
/// lost: every element that was on a changed or new side is still somewhere afterwards
/// (here, there or in the trash), and a side is only deleted when it hadn't changed.
#[test]
fn no_data_loss_in_any_cell() {
    use Side::*;
    let base = doc(&[el("base", 1, 1, 1.0)]);
    let here = doc(&[el("base", 1, 1, 1.0), el("here", 1, 5, 2.0)]);
    let there = doc(&[el("base", 1, 1, 1.0), el("there", 1, 6, 3.0)]);
    let k = key("Cell");
    for l in [Unchanged, Changed, Deleted, New] {
        for r in [Unchanged, Changed, Deleted, New] {
            let github = FakeRemote::default();
            let dev = FakeLocal::default();
            let sha = |c: &str| Some(blob_sha(c.as_bytes()));
            let entry = Entry {
                local: if l == New { None } else { sha(&base) },
                remote: if r == New { None } else { sha(&base) },
            };
            dev.index.borrow_mut().repo = REPO.into();
            dev.index.borrow_mut().boards.insert(k.path(), entry);
            match l {
                Unchanged => { dev.files.borrow_mut().insert(k.clone(), base.clone()); }
                Changed | New => { dev.files.borrow_mut().insert(k.clone(), here.clone()); }
                _ => {}
            }
            match r {
                Unchanged => { github.files.borrow_mut().insert(k.path(), base.clone()); }
                Changed | New => { github.files.borrow_mut().insert(k.path(), there.clone()); }
                _ => {}
            }
            let rep = sync(&github, &dev);
            assert_eq!(rep.error, None, "{l:?}/{r:?}");
            let after_here = dev.files.borrow().get(&k).cloned();
            let after_there = github.files.borrow().get(&k.path()).cloned();
            let mut kept: BTreeSet<String> = BTreeSet::new();
            for c in [after_here.clone(), after_there.clone()].into_iter().flatten().chain(dev.trash.borrow().iter().map(|t| t.1.clone())) {
                kept.extend(live(&c));
            }
            if matches!(l, Changed | New) {
                assert!(kept.contains("here"), "{l:?}/{r:?}: the local stroke is lost");
                assert!(after_here.is_some(), "{l:?}/{r:?}: a changed board was deleted here");
            }
            if matches!(r, Changed | New) {
                assert!(kept.contains("there"), "{l:?}/{r:?}: the remote stroke is lost");
                assert!(after_there.is_some(), "{l:?}/{r:?}: a changed board was deleted there");
            }
            // Both sides end the same, or both gone.
            assert_eq!(after_here.map(|c| live(&c)), after_there.map(|c| live(&c)), "{l:?}/{r:?}: the sides differ after sync");
            // And a second run has nothing left to do.
            let again = sync(&github, &dev);
            assert_eq!((again.uploaded, again.downloaded, again.merged, again.deleted_local, again.deleted_remote), (0, 0, 0, 0, 0), "{l:?}/{r:?}");
        }
    }
}
