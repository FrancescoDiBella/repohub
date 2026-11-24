use super::*;

fn data() -> Data {
    let mut d = Data::sample_at(1_790_000_000);
    d.sample = false;
    d.warnings = vec!["Issues couldn't load: x".into()];
    d.retry_after = Some(60);
    d.open_pulls_total = Some(12);
    d.issues_total = Some(4);
    d.pulls[1].checks = None;
    d
}

#[test]
fn the_lists_round_trip_without_warnings_or_backoff() {
    let d = data();
    let back = data_of(&data_json(&d)).expect("reads back");
    assert_eq!(data_json(&back), data_json(&d), "every list and count comes back");
    assert!(back.warnings.is_empty() && back.retry_after.is_none() && !back.sample, "not the load's warnings, back-off or sample flag");
    assert_eq!(back.repos[3].full_name, d.repos[3].full_name);
    assert_eq!(back.repos[3].keys().name_lower, d.repos[3].name.to_lowercase(), "search keys are ready");
    assert_eq!(back.pulls[1].checks, None);
    assert_eq!(back.pulls[2].review_requested, d.pulls[2].review_requested);
    assert_eq!(back.issues[2].assignees, d.issues[2].assignees);
    assert_eq!(back.runs[1].pull_numbers, vec![9]);
}

#[test]
fn a_copy_of_another_format_account_or_api_is_not_read() {
    let copy = Copy { saved: 5, api: "https://api.github.com".into(), account: "octo".into(), data: data(), details: vec![(("a/b".into(), "main".into()), crate::model::sample_detail(&data().repos[0]))], etags: vec![("k".into(), etag::Entry { etag: "W/\"1\"".into(), body: "{\"a\":\"\\\"quoted\\\"\"}".into(), link: None })] };
    let text = encode(&copy, LIMIT).unwrap();
    let back = decode(&text, "https://api.github.com", "octo").expect("same account");
    assert_eq!(back.saved, 5);
    assert_eq!(back.details.len(), 1);
    assert_eq!(back.details[0].1.readme, copy.details[0].1.readme);
    assert_eq!(back.etags, copy.etags, "tagged answers come back as they were");
    assert!(decode(&text, "https://api.github.com", "other").is_none(), "another account");
    assert!(decode(&text, "https://ghe.example/api/v3", "octo").is_none(), "another API");
    let older = text.replacen(&format!("\"format\":{FORMAT}"), "\"format\":0", 1);
    assert!(decode(&older, "https://api.github.com", "octo").is_none(), "another format");
    assert!(decode("{not json", "https://api.github.com", "octo").is_none());
    assert!(decode(&text[..text.len() / 2], "https://api.github.com", "octo").is_none(), "half a file");
}

#[test]
fn the_size_limit_drops_tagged_answers_first_then_pages() {
    let big = "x".repeat(50_000);
    let copy = Copy {
        data: data(),
        details: vec![(("a/b".into(), "main".into()), Detail { readme: Some(big.clone()), ..Detail::default() })],
        etags: (0..4).map(|i| (format!("k{i}"), etag::Entry { etag: "t".into(), body: big.as_str().into(), link: None })).collect(),
        ..Copy::default()
    };
    let lists = encode(&Copy { details: Vec::new(), etags: Vec::new(), ..copy.clone() }, LIMIT).unwrap().len();
    let text = encode(&copy, lists + 120_000).unwrap();
    let back = decode(&text, "", "").unwrap();
    assert_eq!(back.details.len(), 1, "the page fits");
    assert_eq!(back.etags.len(), 1, "only what fits of the tagged answers");
    assert!(text.len() <= lists + 120_000);
    assert!(encode(&copy, lists - 1).is_none(), "the lists alone don't fit: nothing");
}

#[test]
fn names_of_copies() {
    assert_eq!(host_of("https://api.github.com"), "api.github.com");
    assert_eq!(host_of("https://GHE.example.com/api/v3"), "ghe.example.com");
    assert_eq!(host_of("http://127.0.0.1:8080"), "127.0.0.1_8080");
    #[cfg(not(target_arch = "wasm32"))]
    {
        let f = file("https://api.github.com", "");
        assert!(f.ends_with("cache/api.github.com-token.json"), "{}", f.display());
        assert!(file("http://127.0.0.1:9", "../etc").ends_with("cache/127.0.0.1_9-___etc.json"), "no folder in the name");
    }
}

#[test]
fn details_of_repositories_pushed_to_are_kept_and_marked() {
    let old = data();
    let mut new = data();
    new.repos[1].pushed += 60;
    new.repos.remove(2);
    let (a, b, gone) = (old.repos[0].full_name.clone(), old.repos[1].full_name.clone(), old.repos[2].full_name.clone());
    let mut details = HashMap::new();
    for full in [&a, &b, &gone] {
        details.insert((full.clone(), "main".to_owned()), Load::Ready(Detail::default()));
    }
    details.insert(("x/failed".into(), "main".into()), Load::Failed("no".into()));
    let mut stale = HashSet::new();
    let kept = keep_details(&old, &new, details, &mut stale);
    assert!(kept.contains_key(&(a.clone(), "main".into())) && kept.contains_key(&(b.clone(), "main".into())));
    assert!(!kept.contains_key(&(gone, "main".into())), "a repository no longer listed");
    assert_eq!(stale, HashSet::from([(b, "main".to_owned())]), "only the one pushed to since is read again");
}

#[test]
fn keep_details_preserves_in_flight_loading_entries() {
    let data = Data::sample_at(1_790_000_000);
    let a = data.repos[0].full_name.clone();
    let mut details = HashMap::new();
    details.insert((a.clone(), "main".into()), Load::Loading);
    let mut stale = HashSet::new();
    let kept = keep_details(&data, &data, details, &mut stale);
    assert!(matches!(kept.get(&(a.clone(), "main".into())), Some(Load::Loading)), "in-flight apply_early load is not dropped");
    assert!(stale.is_empty());
}

#[test]
fn failed_detail_revalidation_re_marks_key_stale() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = Data::sample_at(1_790_000_000);
    s.data.sample = false;
    let key = (s.data.repos[0].full_name.clone(), "main".to_owned());
    s.details.insert(key.clone(), Load::Ready(crate::model::sample_detail(&s.data.repos[0])));
    let failed = Detail { error: Some("temporary timeout".into()), ..Default::default() };
    s.apply_detail(s.load_gen, key.clone(), failed);
    assert!(matches!(s.details.get(&key), Some(Load::Ready(_))), "keeps shown detail on screen");
    assert!(s.fresh.stale.contains(&key), "re-marks key stale so next open retries");
}
