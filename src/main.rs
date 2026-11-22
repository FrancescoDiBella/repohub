#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Rune's image decoder catches a codec's panic instead of letting a broken picture end the app,
// which only works when panics unwind: no profile may build the desktop program with abort.
#[cfg(not(panic = "unwind"))]
compile_error!("the desktop program needs panic = \"unwind\" (see the profiles in Cargo.toml)");
fn main() { repohub::start(); }
