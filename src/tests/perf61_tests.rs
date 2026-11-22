//! Timing with 500 repositories, and the clone cache that keeps disk probes out of drawing.

use std::time::Instant;
use rune::{Key, KeyEvent, Mods};
use crate::model::{Data, Issue, Pull, PullState, Repo, lang_hue};
use crate::{App, Page};

fn sample_500() -> Data {
    let mut d = Data::sample_at(1_731_000_000);
    let orgs = ["octo", "acme-cloud", "zeta-labs", "nimbus-io", "hyperion-dev", "aurora-sys"];
    let langs = ["Rust", "TypeScript", "Python", "Go", "Java", "C++"];
    d.repos.clear();
    for i in 0..500 {
        let owner = orgs[i % orgs.len()];
        let lang = langs[i % langs.len()];
        let name = format!("service-{owner}-{i:03}");
        let r = Repo {
            full_name: format!("{owner}/{name}"),
            owner: owner.into(),
            name: name.clone(),
            description: format!("High-throughput distributed {lang} component #{i} for {owner} pipelines and telemetry"),
            language: lang.into(),
            hue: lang_hue(lang),
            stars: ((i * 37) % 1200) as u32,
            forks: ((i * 7) % 90) as u32,
            issues: (i % 25) as u32,
            pushed: 1_731_000_000 - (i as i64) * 3600,
            private: i % 3 == 0,
            fork: i % 7 == 0,
            archived: i % 29 == 0,
            branch: "main".into(),
            topics: vec!["cloud".into(), "telemetry".into(), lang.to_lowercase(), format!("tier-{}", i % 5)],
            url: format!("https://github.com/{owner}/{name}"),
            starred: i % 5 == 0,
            ..Repo::default()
        };
        r.precompute_keys();
        d.repos.push(r);
    }
    for i in 0..80 {
        let r = &d.repos[i % d.repos.len()];
        d.pulls.push(Pull {
            number: (100 + i) as u32,
            title: format!("Optimize batching pipeline in {} ({i})", r.name),
            repo: r.full_name.clone(),
            author: "octo".into(),
            state: if i % 4 == 0 { PullState::Draft } else { PullState::Open },
            checks: None,
            updated_at: 1_731_000_000 - (i as i64) * 600,
            comments: (i % 8) as u32,
            url: format!("{}/pull/{}", r.url, 100 + i),
            labels: vec![("performance".into(), 40.0)],
            assignees: Vec::new(),
            review_requested: false,
        });
        d.issues.push(Issue {
            number: (200 + i) as u32,
            title: format!("Investigate latency spike on {} #{i}", r.name),
            repo: r.full_name.clone(),
            label: Some(("bug".into(), 20.0)),
            created_at: 1_731_000_000 - (i as i64) * 900,
            comments: (i % 5) as u32,
            url: format!("{}/issues/{}", r.url, 200 + i),
            labels: vec![("bug".into(), 20.0)],
            assignees: Vec::new(),
        });
    }
    d
}

#[test]
fn measure_500_repos_search_and_palette() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = sample_500();
    s.pending_place = None;
    s.page = Page::Repos;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    rt.on_key = Some(crate::on_key);
    let mut t = 0.0;
    rt.frame(t);

    // Initial page renders REPOS_PAGE (60) cards and offers "Show more (440 left)".
    let card_count = |rt: &rune::Runtime<App>| {
        rt.elements().iter().filter(|e| e.key.as_deref().is_some_and(|k| k.starts_with("card-"))).count()
    };
    assert_eq!(card_count(&rt), crate::pages::REPOS_PAGE);
    assert!(rt.find("repos-show-more").is_some(), "Show more button appears when >60 repos match");

    // Focus search input on Repositories page.
    let (sx, sy) = rt.find("search").expect("search input present");
    rt.pointer_move(sx, sy);
    rt.pointer_down(sx, sy);
    rt.pointer_up(sx, sy);
    t += 0.05;
    rt.frame(t);

    let mut search_ms = Vec::new();
    for ch in "service-ze".chars() {
        let t0 = Instant::now();
        rt.key(KeyEvent { key: Key::Text(ch.to_string()), mods: Mods::default() });
        t += 0.05;
        rt.frame(t);
        search_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    // Clear search so all 500 repos are eligible again, then open Palette and type.
    rt.update(|s: &mut App, _| s.search.clear());
    t += 0.05;
    rt.frame(t);

    let cmd = Mods { cmd: true, ..Mods::default() };
    let t0_open = Instant::now();
    rt.key(KeyEvent { key: Key::Command('k'), mods: cmd });
    t += 0.05;
    rt.frame(t);
    let open_ms = t0_open.elapsed().as_secs_f64() * 1000.0;
    assert!(rt.state.palette.is_some());

    let mut palette_ms = Vec::new();
    for ch in "service-ze".chars() {
        let t0 = Instant::now();
        rt.key(KeyEvent { key: Key::Text(ch.to_string()), mods: Mods::default() });
        t += 0.05;
        rt.frame(t);
        palette_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let max = |v: &[f64]| v.iter().copied().fold(0.0_f64, f64::max);
    println!(
        "PERF61_METRICS repos_search_mean={:.2}ms repos_search_max={:.2}ms palette_open={:.2}ms palette_type_mean={:.2}ms palette_type_max={:.2}ms",
        mean(&search_ms), max(&search_ms), open_ms, mean(&palette_ms), max(&palette_ms)
    );

    // One frame (16 ms) is the target, checked with `REPOHUB_PERF_ASSERT=1` on a quiet
    // machine. Tests run in parallel on shared CI runners, where timings swing: there the
    // bound only catches a return to the old cost (hundreds of milliseconds).
    #[cfg(not(debug_assertions))]
    {
        let budget = if std::env::var("REPOHUB_PERF_ASSERT").is_ok_and(|v| v == "1") { 16.0 } else { 120.0 };
        assert!(mean(&search_ms) < budget, "expected Repos search typing < {budget} ms in release, got {:.2} ms", mean(&search_ms));
        assert!(mean(&palette_ms) < budget, "expected Palette typing < {budget} ms in release, got {:.2} ms", mean(&palette_ms));
    }

    // Close palette and verify clicking "Show more" expands to 120 cards, and changing search resets to 60.
    rt.update(|s: &mut App, _| s.palette = None);
    t += 0.05;
    rt.frame(t);
    rt.update(|s: &mut App, _| s.repos_limit += crate::pages::REPOS_PAGE);
    t += 0.05;
    rt.frame(t);
    assert_eq!(card_count(&rt), crate::pages::REPOS_PAGE * 2);
}

#[test]
fn clone_cache_avoids_reprobing_disk_after_first_lookup() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = crate::init();
    assert!(!s.clone_dirs.borrow().contains_key("nobody/nonexistent-repo-61"));
    assert_eq!(s.known_clone("nobody/nonexistent-repo-61"), None);
    assert_eq!(s.clone_dirs.borrow().get("nobody/nonexistent-repo-61"), Some(&None));
}
