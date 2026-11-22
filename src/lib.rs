//! RepoHub: a personal GitHub repository manager built with Rune and rune-kit.
//!
//! It reads your repositories, pull requests, issues and workflow runs from the GitHub REST API.
//! The desktop app is configured by a `.env` file (see `.env.example`); the web build gets its
//! configuration and a token-adding proxy from `repohub-server` (the Docker image). Without either
//! it shows built-in sample data.
// Desktop-only helpers (local files, the terminal) aren't called from the web build.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod actions;
mod app;
mod board;
mod cache;
mod cache_page;
mod claude;
mod claude_page;
#[cfg(not(target_arch = "wasm32"))]
mod clones;
mod clones_shim;
mod commits;
mod commits_page;
mod config;
mod github;
mod harness;
mod icons;
mod inbox;
mod lists;
mod diffs;
mod etag;
mod filters;
mod local;
mod local_page;
mod changes_page;
mod markdown;
mod mobile;
mod model;
mod net;
mod notes;
mod notes_page;
mod pages;
mod palette;
#[cfg(not(target_arch = "wasm32"))]
mod plugins;
#[cfg(target_arch = "wasm32")]
#[path = "plugins_web.rs"]
mod plugins;
mod plugins_page;
mod prefs;
mod releases;
mod releases_page;
mod state;
mod review;
mod runs_page;
mod sync;
mod sync_io;
mod sync_page;
mod board_file;
mod terminal;
mod thread;
mod thread_page;
mod triage;
mod triage_page;
mod terminal_page;
mod themes;
mod themes_page;
mod tray;
mod ui;
mod update;
#[cfg(test)]
mod test_env;

/// Tests that change process environment variables hold this, so they don't race; taking it
/// also clears any account the machine has (see `test_env`).
#[cfg(test)]
pub(crate) static ENV_LOCK: test_env::Lock = test_env::Lock::new();

// The app lives in `app`: these keep the `crate::…` paths the rest of the crate and the tests name.
pub use app::{App, LookEditor, Page, start};
#[cfg(target_arch = "wasm32")]
pub use app::repohub_back;
pub use state::Status;
pub(crate) use app::{orgs, pinned};
#[cfg(test)]
pub(crate) use app::{NoteOp, banner, init, on_key, poll, shortcuts, view};
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use app::UpdateJob;
#[cfg(test)]
pub(crate) use state::RepoTab;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod load_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod allowlist_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::load_gen_tests;
