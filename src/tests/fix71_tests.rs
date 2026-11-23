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
