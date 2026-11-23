use super::*;
use serde_json::json;

#[test]
fn labels_people_and_milestones_are_read_from_githubs_answers() {
    let labels = parse_labels(&json!([
        { "name": "bug", "color": "d73a4a", "description": "Something isn't working" },
        { "name": "", "color": "ffffff" },
        { "name": "docs", "color": "0075ca", "description": null },
    ]));
    assert_eq!(labels.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["bug", "docs"], "a label with no name is left out");
    assert!((labels[0].hue.unwrap() - 354.0).abs() < 2.0);
    assert_eq!((labels[0].note.as_str(), labels[1].note.as_str()), ("Something isn't working", ""));

    let people = parse_people(&json!([{ "login": "alice" }, { "login": "" }, { "id": 3 }, { "login": "bob" }]));
    assert_eq!(people.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["alice", "bob"]);

    let ms = parse_milestones(&json!([
        { "number": 3, "title": "v1.0", "due_on": "2026-12-01T08:00:00Z" },
        { "number": 4, "title": "Later", "due_on": null },
        { "title": "no number" },
    ]));
    assert_eq!(ms.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.note.as_str())).collect::<Vec<_>>(),
        [("3", "v1.0", "Due on 2026-12-01"), ("4", "Later", "No due date")]);

    assert_eq!(milestone_of(&json!({ "milestone": { "number": 3, "title": "v1.0" } })), Some((3, "v1.0".into())));
    assert_eq!(milestone_of(&json!({ "milestone": null })), None);
    assert_eq!(logins(&json!({ "requested_reviewers": [{ "login": "carol" }, { "login": "dave" }] }), "requested_reviewers"), ["carol", "dave"]);
    assert_eq!(Source::Labels.path("o/r"), "/repos/o/r/labels?per_page=100");
    assert_eq!(Source::People.path("o/r"), "/repos/o/r/assignees?per_page=100");
    assert_eq!(Source::Milestones.path("o/r"), "/repos/o/r/milestones?state=open&per_page=100");
    assert_eq!((Source::of(Facet::Reviewers), Source::of(Facet::Assignees)), (Source::People, Source::People));
}

#[test]
fn each_reviewer_keeps_their_latest_verdict() {
    let r = reviewed_of(&json!([
        { "user": { "login": "carol" }, "state": "APPROVED", "submitted_at": "2026-11-04T10:00:00Z" },
        { "user": { "login": "carol" }, "state": "COMMENTED", "submitted_at": "2026-11-05T10:00:00Z" },
        { "user": { "login": "dave" }, "state": "CHANGES_REQUESTED", "submitted_at": "2026-11-01T10:00:00Z" },
        { "user": { "login": "dave" }, "state": "APPROVED", "submitted_at": "2026-11-03T10:00:00Z" },
        { "user": { "login": "erin" }, "state": "COMMENTED", "submitted_at": "2026-11-02T10:00:00Z" },
        { "user": { "login": "me" }, "state": "PENDING" },
        { "user": { "login": "frank" }, "state": "DISMISSED", "submitted_at": "2026-11-02T14:00:00Z" },
    ]));
    assert_eq!(r, [("dave".to_owned(), Verdict::Approved), ("erin".to_owned(), Verdict::Commented), ("carol".to_owned(), Verdict::Approved)],
        "a comment after an approval doesn't undo it; pending and dismissed reviews don't count");
}

fn choice(id: &str, name: &str, note: &str) -> Choice {
    Choice { id: id.into(), name: name.into(), hue: None, note: note.into() }
}

#[test]
fn the_picker_filters_ticks_and_says_what_changed() {
    let choices = [choice("bug", "bug", "Something isn't working"), choice("docs", "docs", "Documentation"), choice("good first issue", "good first issue", "For newcomers")];
    let mut p = Picker::new(Facet::Labels, vec!["bug".into(), "wontfix".into()]);
    assert!(!p.changed() && p.change(&[]).is_none());
    // What's ticked but not offered comes first, so it can be unticked.
    assert_eq!(p.shown(&choices, None).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["wontfix", "bug", "docs", "good first issue"]);
    // Every word, in the name or the description, any case.
    p.filter = TextEdit::new("NEW");
    assert_eq!(p.shown(&choices, None).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["good first issue"]);
    p.filter = TextEdit::new("doc ment");
    assert_eq!(p.shown(&choices, None).len(), 1);
    p.filter = TextEdit::new("");
    p.toggle("docs");
    p.toggle("BUG");
    assert_eq!((p.added(), p.removed()), (vec!["docs".to_owned()], vec!["bug".to_owned()]));
    assert_eq!(p.change(&[]), Some(Change::Labels(vec!["wontfix".into(), "docs".into()])));
    // Back to how it was, in another order: nothing to send.
    p.toggle("docs");
    p.toggle("bug");
    assert!(!p.changed());

    // The milestone is one choice, or none.
    let ms = [choice("3", "v1.0", ""), choice("4", "v2.0", "")];
    let mut m = Picker::new(Facet::Milestone, vec!["3".into()]);
    m.toggle("4");
    assert_eq!(m.picked, ["4"]);
    assert_eq!(m.change(&ms), Some(Change::Milestone(Some((4, "v2.0".into())))));
    m.toggle("");
    assert!(m.picked.is_empty());
    assert_eq!(m.change(&ms), Some(Change::Milestone(None)));

    // Reviewers: who to ask and whom to stop asking; the author can't review their own.
    let people = [choice("alice", "alice", ""), choice("bob", "bob", ""), choice("carol", "carol", "")];
    let mut r = Picker::new(Facet::Reviewers, vec!["bob".into()]);
    assert_eq!(r.shown(&people, Some("alice")).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["bob", "carol"]);
    r.toggle("carol");
    r.toggle("bob");
    assert_eq!(r.change(&[]), Some(Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }));

    // At most ten assignees.
    let mut a = Picker::new(Facet::Assignees, (0..10).map(|n| format!("u{n}")).collect());
    a.toggle("one-more");
    assert_eq!(a.picked.len(), 10);
    assert!(a.note.as_deref().is_some_and(|n| n.contains("up to 10")));
    a.toggle("u0");
    assert!(a.note.is_none() && a.picked.len() == 9);
}

#[test]
fn each_change_is_its_own_request() {
    let req = |c: Change| c.requests("o/r", 7).into_iter().map(|(m, p, b)| (m, p, b.to_string())).collect::<Vec<_>>();
    let issue = "/repos/o/r/issues/7".to_owned();
    let rr = "/repos/o/r/pulls/7/requested_reviewers".to_owned();
    assert_eq!(req(Change::Labels(vec!["bug".into(), "ui".into()])), [("PATCH", issue.clone(), r#"{"labels":["bug","ui"]}"#.to_owned())]);
    assert_eq!(req(Change::Labels(Vec::new())), [("PATCH", issue.clone(), r#"{"labels":[]}"#.to_owned())], "no labels is a change too");
    assert_eq!(req(Change::Assignees(vec!["alice".into()])), [("PATCH", issue.clone(), r#"{"assignees":["alice"]}"#.to_owned())]);
    assert_eq!(req(Change::Milestone(Some((3, "v1.0".into())))), [("PATCH", issue.clone(), r#"{"milestone":3}"#.to_owned())]);
    assert_eq!(req(Change::Milestone(None)), [("PATCH", issue.clone(), r#"{"milestone":null}"#.to_owned())]);
    assert_eq!(req(Change::Title("New \"title\"".into())), [("PATCH", issue.clone(), r#"{"title":"New \"title\""}"#.to_owned())]);
    assert_eq!(req(Change::Body("# Hi\n".into())), [("PATCH", issue.clone(), r##"{"body":"# Hi\n"}"##.to_owned())]);
    assert_eq!(req(Change::Reviewers { add: vec!["carol".into()], remove: Vec::new() }), [("POST", rr.clone(), r#"{"reviewers":["carol"]}"#.to_owned())]);
    assert_eq!(req(Change::Reviewers { add: Vec::new(), remove: vec!["bob".into()] }), [("DELETE", rr.clone(), r#"{"reviewers":["bob"]}"#.to_owned())]);
    assert_eq!(req(Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }).len(), 2, "taken back, then asked");
    assert_eq!(busy_key("Octo/App", 7), "octo/app#7:triage");
    assert_ne!(busy_key("o/r", 7), crate::actions::busy_key("o/r", 7), "a triage write doesn't make Close wait");
}

#[test]
fn githubs_answer_says_what_it_is_now() {
    let answer = json!({ "title": "T", "body": "B", "labels": [{ "name": "bug", "color": "d73a4a" }], "assignees": [{ "login": "alice" }],
        "milestone": { "number": 3, "title": "v1.0" }, "requested_reviewers": [{ "login": "carol" }] });
    assert!(matches!(Applied::from_answer(&Change::Labels(vec!["x".into()]), &answer), Applied::Labels(l) if l.len() == 1 && l[0].0 == "bug"));
    assert_eq!(Applied::from_answer(&Change::Assignees(Vec::new()), &answer), Applied::Assignees(vec!["alice".into()]));
    assert_eq!(Applied::from_answer(&Change::Milestone(None), &answer), Applied::Milestone(Some((3, "v1.0".into()))));
    assert_eq!(Applied::from_answer(&Change::Reviewers { add: vec![], remove: vec![] }, &answer), Applied::Reviewers(vec!["carol".into()]));
    // No answer to read: what was asked.
    let none = Value::Null;
    assert_eq!(Applied::from_answer(&Change::Reviewers { add: vec!["carol".into()], remove: vec!["bob".into()] }, &none), Applied::Reviewers(vec!["carol".into()]));
    assert_eq!(Applied::from_answer(&Change::Title("New".into()), &none), Applied::Title("New".into()));
    assert_eq!(Applied::from_answer(&Change::Milestone(Some((4, "v2".into()))), &none), Applied::Milestone(Some((4, "v2".into()))));
    // The toasts say what changed.
    let c = Change::Reviewers { add: vec!["carol".into()], remove: vec![] };
    assert_eq!(Applied::Reviewers(vec!["carol".into()]).toast(7, &c), "Asked carol to review #7");
    assert_eq!(Applied::Labels(Vec::new()).toast(7, &Change::Labels(vec![])), "Removed the labels of #7");
    assert_eq!(Applied::Milestone(Some((3, "v1.0".into()))).toast(7, &Change::Milestone(None)), "#7 is in the milestone v1.0");
}

#[test]
fn choices_are_kept_per_repository_and_late_answers_are_dropped() {
    let mut c = Cache::default();
    let first = c.ask("o/a", Source::Labels, false).expect("asked");
    assert!(c.ask("O/A", Source::Labels, false).is_none(), "on its way already");
    assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Loading)));
    // Retry: the first answer is now old.
    let second = c.ask("o/a", Source::Labels, true).expect("asked again");
    assert!(!c.answer("o/a", Source::Labels, first, Ok(Choices { items: vec![choice("old", "old", "")], more: false })));
    assert!(c.answer("o/a", Source::Labels, second, Ok(Choices { items: vec![choice("bug", "bug", "")], more: true })));
    assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Ready(l)) if l.items[0].id == "bug" && l.more));
    assert!(c.ask("o/a", Source::Labels, false).is_none(), "kept for the repository");
    assert!(c.get("o/b", Source::Labels).is_none() && c.get("o/a", Source::People).is_none(), "per repository and kind");
    // A failed retry keeps the list that was there.
    let again = c.ask("o/a", Source::Labels, true).unwrap();
    assert!(c.answer("o/a", Source::Labels, again, Err("offline".into())));
    assert!(matches!(c.get("o/a", Source::Labels), Some(Load::Ready(_))));
    // Another account forgets everything; its answers no longer land.
    let late = c.ask("o/b", Source::People, false).unwrap();
    c.clear();
    assert!(!c.answer("o/b", Source::People, late, Ok(Choices::default())));
    assert!(c.get("o/a", Source::Labels).is_none());
}
