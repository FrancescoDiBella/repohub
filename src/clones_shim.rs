//! The local-clone lookup on the desktop; the web build has no file system to look in.
#[cfg(not(target_arch = "wasm32"))]
pub use crate::clones::{default_parent, expand, find, remember};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::{Path, PathBuf};
    pub fn find(_: &str) -> Option<PathBuf> {
        None
    }
    pub fn expand(p: &str) -> PathBuf {
        PathBuf::from(p)
    }
    pub fn remember(_: &str, _: &Path) -> Result<(), String> {
        Err("Local folders need the desktop app.".into())
    }
    pub fn default_parent() -> PathBuf {
        PathBuf::new()
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{default_parent, expand, find, remember};
