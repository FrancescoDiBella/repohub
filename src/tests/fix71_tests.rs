//! Settings: one section at a time, and the connection's details on request.

use crate::state::SettingsSection;
use crate::{App, Page};

#[test]
fn settings_show_one_section_at_a_time_and_details_on_request() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = Page::Settings;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let shown = |rt: &rune::Runtime<App>, key: &str| rt.find(key).is_some();
    // Account first: the account and the connection, not the themes or the plugins.
    assert_eq!(rt.state.settings_section, SettingsSection::Account);
    assert!(shown(&rt, "connection") && !shown(&rt, "appearance") && !shown(&rt, "plugins"));
    // The connection's details wait to be asked for.
    let texts = |rt: &rune::Runtime<App>| rt.elements().iter().filter_map(|e| e.text.clone()).collect::<Vec<_>>();
    assert!(!texts(&rt).iter().any(|t| t == "Settings file"));
    let (x, y) = rt.find("connection-details").unwrap();
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.1);
    assert!(texts(&rt).iter().any(|t| t == "Settings file"));
    for (section, key) in [(SettingsSection::Appearance, "appearance"), (SettingsSection::Plugins, "plugins")] {
        rt.update(move |s, _| s.settings_section = section);
        rt.frame(0.3);
        assert!(shown(&rt, key) && !shown(&rt, "connection"), "{key}");
    }
    rt.update(|s, _| s.settings_section = SettingsSection::Keyboard);
    rt.frame(0.5);
    assert!(texts(&rt).iter().any(|t| t == "Keyboard shortcuts"));
    assert_eq!(SettingsSection::ALL.len(), SettingsSection::LABELS.len());
}

/// Each Settings section fits one screen of 1180 × 760 (the theme editor, when open, scrolls).
#[test]
fn each_settings_section_fits_a_screen() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("settings-fit");
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.page = Page::Settings;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 760.0), rune::Env::new(true, 1.0));
    rt.reduced_motion = true;
    let mut t = 0.0;
    for section in SettingsSection::ALL {
        rt.update(move |s, _| s.settings_section = section);
        for _ in 0..3 {
            t += 0.1;
            rt.frame(t);
        }
        let bottom = rt.elements().iter().filter(|e| e.path.contains("/page/") && e.rect[3] > 0.0).map(|e| e.rect[1] + e.rect[3]).fold(0.0f32, f32::max);
        assert!(bottom <= 760.0, "{section:?} ends at {bottom}, past the window's 760 points");
    }
}

/// One mark for Public / Private (the cards and the repository's page), and empty sections that
/// say what's empty and what to do.
#[test]
fn one_visibility_mark_and_empty_sections_that_say_what_to_do() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.data_changed();
    s.pending_place = None;
    s.page = Page::Repos;
    let quiet = s.data.repos.iter().position(|r| !s.data.issues.iter().any(|x| x.repo.eq_ignore_ascii_case(&r.name) || x.repo.eq_ignore_ascii_case(&r.full_name))).expect("a repository without issues");
    let name = s.data.repos[quiet].name.clone();
    let private = s.data.repos[quiet].private;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 4000.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    let marks = |rt: &rune::Runtime<App>| rt.elements().iter().filter(|e| matches!(e.label.as_deref(), Some("Public repository" | "Private repository"))).count();
    assert!(marks(&rt) >= rt.state.data.repos.len().min(3), "every card has the mark");
    rt.update(move |s, _| { s.open(quiet); s.set_tab(quiet, crate::state::RepoTab::Issues); });
    rt.frame(0.5);
    let want = if private { "Private repository" } else { "Public repository" };
    assert!(rt.elements().iter().any(|e| e.label.as_deref() == Some(want)), "the repository's page has the same mark");
    let texts: Vec<String> = rt.elements().iter().filter_map(|e| e.text.clone()).collect();
    assert!(texts.contains(&format!("No open issues in {name}")) && texts.iter().any(|t| t.starts_with("New issue starts one")), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Nothing to show here"));
}

/// The theme editor opens in a view of its own, with a way back (the button, or Esc).
#[test]
fn the_theme_editor_is_its_own_view() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("theme-editor-view");
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.page = Page::Settings;
    s.settings_section = SettingsSection::Appearance;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 2400.0), rune::Env::new(true, 1.0));
    rt.on_key = Some(crate::on_key);
    rt.frame(0.0);
    assert!(rt.find("theme-pick").is_some() && rt.find("look-back").is_none(), "the gallery");
    let base = rt.state.looks.all[0].id.clone();
    rt.update(move |s, _| s.duplicate_look(&base, true));
    rt.frame(0.2);
    assert!(rt.find("look-back").is_some() && rt.find("look-color-0").is_some(), "the editor");
    assert!(rt.find("theme-pick").is_none(), "without the gallery and the mode");
    rt.key(rune::KeyEvent { key: rune::Key::Escape, mods: rune::Mods::default() });
    rt.frame(0.4);
    assert!(rt.find("theme-pick").is_some() && rt.find("look-back").is_none(), "Esc goes back to the gallery");
}

/// On a phone Settings is a list of its sections; one opens with a way back (the button, Esc).
#[test]
fn on_a_phone_settings_is_a_list_of_sections() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = crate::init();
    s.data = crate::model::Data::sample();
    s.pending_place = None;
    s.compact = true;
    s.page = Page::Overview;
    let mut rt = rune::Runtime::new(s, crate::view, (390.0, 844.0), rune::Env::new(true, 1.0));
    rt.on_key = Some(crate::on_key);
    rt.reduced_motion = true;
    rt.frame(0.0);
    rt.update(|s, _| s.go(Page::Settings));
    rt.frame(0.1);
    for name in SettingsSection::LABELS {
        assert!(rt.find(&format!("settings-open-{name}")).is_some(), "{name} is in the list");
    }
    assert!(rt.find("connection").is_none(), "no section open yet");
    let (x, y) = rt.find("settings-open-Keyboard").unwrap();
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
    assert!(rt.find("shortcuts").is_some() && rt.find("settings-back").is_some(), "the section, with a way back");
    rt.key(rune::KeyEvent { key: rune::Key::Escape, mods: rune::Mods::default() });
    rt.frame(0.3);
    assert!(rt.find("settings-open-Account").is_some() && rt.find("shortcuts").is_none(), "Esc goes back to the list");
}
