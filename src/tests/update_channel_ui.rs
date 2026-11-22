//! The update channel is picked in Settings and kept.

/// Settings → About shows the version and the channel; picking one is kept.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_channel_is_picked_in_settings_and_kept() {
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = crate::load_gen_tests::TempDataDir::new("update-channel");
    let mut s = crate::init();
    s.pending_place = None;
    s.page = crate::Page::Settings;
    s.settings_section = crate::state::SettingsSection::About;
    let mut rt = rune::Runtime::new(s, crate::view, (1180.0, 1400.0), rune::Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.find("update-channel").is_some());
    let v = crate::update::version();
    assert!(rt.elements().iter().any(|e| e.text.as_deref().is_some_and(|t| t.starts_with(v))), "the version shows");
    rt.update(|s, _| s.set_update_channel(crate::update::Channel::Stable));
    rt.update(|s, _| s.set_update_channel(crate::update::Channel::Preview));
    assert_eq!(rt.state.update_channel, crate::update::Channel::Preview);
    assert_eq!(crate::update::Channel::load(), crate::update::Channel::Preview, "saved");
    rt.update(|s, _| s.set_update_channel(crate::update::Channel::Stable));
    assert_eq!(crate::update::Channel::load(), crate::update::Channel::Stable);
}
