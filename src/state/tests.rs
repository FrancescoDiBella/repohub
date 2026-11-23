use super::*;

#[test]
fn status_rules() {
    assert!(Status::Starting.can_load() && Status::Ready.can_load() && Status::Failed("x".into()).can_load());
    assert!(!Status::Loading.can_load() && !Status::Sample.can_load());
    assert!(Status::Starting.is_busy() && !Status::Ready.is_busy());
}

#[test]
fn save_queues_edits_made_while_writing() {
    let mut s = Save::Saved;
    assert!(s.edit(), "the first edit starts a write");
    assert!(!s.edit(), "a second edit waits for the running write");
    assert_eq!(s, Save::Saving { dirty: true });
    assert!(s.written(Ok(())), "the waiting edit is written next");
    assert_eq!(s, Save::Saving { dirty: false });
    assert!(!s.written(Ok(())));
    assert_eq!(s, Save::Saved);
    // A failure keeps the message until the next edit, which tries again.
    assert!(s.edit());
    assert!(!s.written(Err("disk full".into())));
    assert_eq!(s, Save::Failed("disk full".into()));
    assert!(s.pending() && s.edit());
    assert!(!Save::Saved.pending() && !(Save::Saving { dirty: false }).pending() && (Save::Saving { dirty: true }).pending());
    // Nothing is written while the drawing loads, and a stray result changes nothing.
    let mut l = Save::Loading;
    assert!(!l.edit() && !l.written(Ok(())));
    l.loaded(Ok(()));
    assert_eq!(l, Save::Saved);
}

#[test]
fn place_and_layout_round_trip() {
    let p = Place { page: "repo".into(), repo: Some("acme/site".into()), tab: Some(RepoTab::Claude) };
    assert_eq!(Place::from_json(&p.to_json()), Some(p));
    let l = Layout { sidebar_width: 301.5, sidebar_open: false, sort: RepoSort::Stars, owner: OwnerFilter::Org("xr-lab".into()) };
    assert_eq!(Layout::from_json(&l.to_json()), Some(l));
    assert_eq!(Place::from_json(&serde_json::json!({"nope": 1})), None);
    let old = Layout::from_json(&serde_json::json!({"sidebarWidth": 200.0})).unwrap();
    assert_eq!((old.sort, old.owner, old.sidebar_open), (RepoSort::Recent, OwnerFilter::All, true), "missing keys take their defaults");
}

#[test]
fn orgs_and_owners() {
    assert_eq!(Orgs::parse(""), Orgs::All);
    assert_eq!(Orgs::parse("None"), Orgs::Off);
    assert_eq!(Orgs::parse("acme, XR-Lab"), Orgs::Only(vec!["acme".into(), "xr-lab".into()]));
    assert!(Orgs::parse("acme,xr-lab").includes("XR-Lab") && !Orgs::parse("acme").includes("other"));
    assert!(!Orgs::Off.includes("acme") && Orgs::All.includes("acme"));
    assert!(OwnerFilter::Mine.matches("Octo", "octo") && !OwnerFilter::Mine.matches("acme", "octo"));
    assert!(OwnerFilter::Org("acme".into()).matches("ACME", "octo"));
}

#[test]
fn submit_sends_once() {
    let mut s = Submit::Editing;
    assert!(s.start() && s.is_sending());
    assert!(!s.start(), "a second click while sending does nothing");
    s = Submit::Failed("422".into());
    assert!(s.start(), "a failed form can be sent again");
    assert_eq!(Compose::Pull.tab(), RepoTab::Pulls);
}

#[test]
fn theme_and_tabs() {
    assert_eq!(Theme::System.toggled(true), Theme::Light);
    assert_eq!(Theme::System.toggled(false), Theme::Dark);
    assert_eq!(Theme::Dark.toggled(false), Theme::Light);
    assert_eq!(Theme::parse("Dark"), Some(Theme::Dark));
    assert_eq!(Theme::parse("blue"), None);
    for (i, t) in RepoTab::ALL.into_iter().enumerate() {
        assert_eq!((t.index(), RepoTab::from_index(i)), (i, t));
    }
    assert_eq!(RepoTab::parse("notes"), Some(RepoTab::Notes));
    assert_eq!(RepoTab::parse("Pull requests"), Some(RepoTab::Pulls));
    assert_eq!(RepoTab::parse("9"), Some(RepoTab::Terminal));
    assert_eq!(RepoTab::parse("commits"), Some(RepoTab::Commits));
    assert_eq!(RepoTab::Commits.index(), 1, "Commits comes after Code");
    assert_eq!(RepoTab::parse("releases"), Some(RepoTab::Releases));
    assert_eq!(RepoTab::ALL[RepoTab::Releases.index() - 1], RepoTab::Actions, "Releases comes after Actions");
    assert_eq!(RepoTab::parse("changes"), Some(RepoTab::Changes));
    assert_eq!(RepoTab::parse("claude"), Some(RepoTab::Claude));
    assert_eq!(RepoTab::from_index(99), RepoTab::Code);
    assert!(RepoFilter::Private.matches(true, false) && !RepoFilter::Fork.matches(true, false));
    for (i, f) in RunStatusFilter::ALL.into_iter().enumerate() {
        assert_eq!((f.index(), RunStatusFilter::from_index(i)), (i, f));
    }
    assert_eq!(RunStatusFilter::from_index(99), RunStatusFilter::All);
    assert!(RunStatusFilter::All.matches(crate::model::Checks::Passing));
    assert!(RunStatusFilter::Failed.matches(crate::model::Checks::Failing) && !RunStatusFilter::Failed.matches(crate::model::Checks::Running));
    assert!(RunStatusFilter::Running.matches(crate::model::Checks::Running) && !RunStatusFilter::Running.matches(crate::model::Checks::Passing));
}
