//! The web build has no plugins: Luau's C++ needs a WASI toolchain there, and a page can't read
//! a plugins folder. This keeps the app's code the same in both builds.
use crate::App;

/// Something a plugin wants that reaches outside the app. It never happens by itself: the app
/// shows it and the user allows it or not.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Open this `https://` address in the browser.
    Open(String),
    /// Put this text on the clipboard.
    Copy(String),
}

/// An [`Ask`] and the plugin (its name) it comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub plugin: String,
    pub ask: Ask,
}

impl Ask {
    /// A link a plugin may ask to open: `https://`, of a sane length, with no spaces or
    /// control characters.
    pub fn open(url: &str) -> Option<Ask> {
        let ok = url.starts_with("https://") && url.len() <= 2000 && host(url).is_some_and(|h| !h.is_empty())
            && !url.chars().any(|c| c.is_control() || c.is_whitespace());
        ok.then(|| Ask::Open(url.to_owned()))
    }

    /// Text a plugin may ask to copy: without control characters (they could run commands
    /// when pasted into a terminal), up to 100,000 characters.
    pub fn copy(text: &str) -> Option<Ask> {
        let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(100_000).collect();
        (!clean.trim().is_empty()).then_some(Ask::Copy(clean))
    }
}

/// The host of an `https://` address ("example.com"), which is what the user is shown first.
pub fn host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // What follows an `@` is the real host; what precedes it only looks like one.
    let host = authority.rsplit('@').next()?;
    Some(host.split(':').next().unwrap_or(host))
}

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
    pub fn surface(&self, _: &str, _: &rune::Cx<App>, _: &str) -> Option<rune::El> {
        None
    }
    pub fn repo_tabs(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    pub fn pages(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    pub fn set_colors(&self, _: &rune_kit::Kit) {}
}
