//! The app's tests that build the whole app (`crate::init`) and drive it headless, one module per
//! file. `crate::load_gen_tests::TempDataDir` is a throwaway data folder for any test.

#[cfg(test)]
mod pin_tests;
#[cfg(test)]
mod cache_tests;
#[cfg(test)]
pub(crate) mod load_gen_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix52_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix63_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix64_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix65_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix64_inbox_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix71_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix69_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix66_tests;
#[cfg(test)]
mod fix60_tests;
#[cfg(test)]
mod fix56_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod fix50_security_tests;
#[cfg(test)]
mod fix83_tests;
#[cfg(test)]
mod fix58_tests;
#[cfg(test)]
mod fix62_tests;
#[cfg(test)]
mod scroll_bar_ui;
#[cfg(test)]
mod phone_sizes;
#[cfg(test)]
mod update_channel_ui;
#[cfg(test)]
mod fix67_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod perf61_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod cache_freshness;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod every_page;
