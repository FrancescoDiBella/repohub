//! The web build has no plugins: Luau's C++ needs a WASI toolchain there, and a page can't read
//! a plugins folder. This keeps the app's code the same in both builds.
use crate::App;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub repo_tab: Option<String>,
    pub page: Option<String>,
}

pub struct Plugin {
    pub id: String,
    pub dir: std::path::PathBuf,
    pub manifest: Manifest,
    pub enabled: bool,
}

impl Plugin {
    pub fn running(&self) -> bool {
        false
    }
    pub fn trouble(&self) -> Option<String> {
        None
    }
}

#[derive(Default)]
pub struct Plugins {
    pub list: Vec<Plugin>,
}

impl Plugins {
    pub fn scan() -> Plugins {
        Plugins::default()
    }
    pub fn surface(&self, _: usize, _: &rune::Cx<App>, _: &str) -> Option<rune::El> {
        None
    }
    pub fn repo_tabs(&self) -> Vec<(usize, String)> {
        Vec::new()
    }
    pub fn pages(&self) -> Vec<(usize, String)> {
        Vec::new()
    }
    pub fn set_colors(&self, _: &rune_kit::Kit) {}
}
