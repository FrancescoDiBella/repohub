//! The app's state machines, in one place.
//!
//! Each type here names the states something can be in, so a state that makes no sense can't be
//! written down (a list that is "loading" and "failed" at once, tab number 9, a theme of
//! `Some(None)`). Transitions are methods; the ones with rules have tests at the bottom.
//! `docs/STATE.md` draws them.
//!
//! | Machine | States | Used for |
//! | --- | --- | --- |
//! | [`Status`] | Starting → Loading → Ready / Failed, Sample | where the data comes from |
//! | [`Load`] | (absent) → Loading → Ready / Failed | repository details, branch lists |
//! | [`Refresh`] | Loading / Fresh / Failed, keeping the last items | the drawings list |
//! | [`Save`] | Loading → Saved ⇄ Saving{dirty} → Failed | the open drawing |
//! | [`Theme`] | System / Light / Dark | appearance |
//! | [`RepoTab`] | Code … Settings | the repository page's section |
//! | [`RepoFilter`], [`RepoSort`] | | the Repositories list |
//! | [`Account`] | Locked / SignedOut / SignedIn | the account form in Settings |
//! | [`Toast`] | (none) → shown(kind) → (none) | the message at the bottom |

/// Where the data on screen comes from.
///
/// ```text
/// Starting ──refresh──▶ Loading ──ok──▶ Ready ──refresh──▶ Loading
///     │                    └──error──▶ Failed ──retry────▶ Loading
///     └──no configuration──▶ Sample ──connect (Settings)──▶ Loading
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not asked yet: the window is opening (or the web build waits for `config.json`).
    Starting,
    /// A request to GitHub is running.
    Loading,
    /// GitHub's data is on screen.
    Ready,
    /// The last load failed; the message says why. Retry loads again.
    Failed(String),
    /// The built-in sample data (no configuration, or demo mode).
    Sample,
}

impl Status {
    /// True when a load may start now: never while one runs, never over the sample data.
    pub fn can_load(&self) -> bool {
        matches!(self, Status::Starting | Status::Ready | Status::Failed(_))
    }

    /// True while there is nothing to show yet.
    pub fn is_busy(&self) -> bool {
        matches!(self, Status::Starting | Status::Loading)
    }

    /// One word for logs and the Settings page.
    pub fn name(&self) -> &'static str {
        match self {
            Status::Starting => "starting",
            Status::Loading => "loading",
            Status::Ready => "ready",
            Status::Failed(_) => "failed",
            Status::Sample => "sample",
        }
    }
}

/// Something fetched once. "Not asked yet" is the absence of a `Load` (a missing map entry).
#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }
}

impl<T> From<Result<T, String>> for Load<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

/// A list that is fetched again and again, keeping what it last showed while it reloads.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Refresh {
    #[default]
    Loading,
    Fresh,
    Failed(String),
}

/// Saving the open drawing. Edits made while a write is running are not lost and don't start a
/// second write next to it: they mark the save `dirty`, and one more write follows.
///
/// ```text
/// Loading ──loaded──▶ Saved ──edit──▶ Saving{clean} ──written──▶ Saved
///                                        │ edit           └─error──▶ Failed ──edit──▶ Saving
///                                        ▼
///                                  Saving{dirty} ──written or error──▶ Saving{clean} (writes again)
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Save {
    /// The drawing is being read; edits wait.
    Loading,
    Saved,
    Saving { dirty: bool },
    Failed(String),
}

impl Save {
    /// The drawing changed. Returns true when a write must start now.
    pub fn edit(&mut self) -> bool {
        match self {
            Save::Loading => false,
            Save::Saving { dirty } => {
                *dirty = true;
                false
            }
            Save::Saved | Save::Failed(_) => {
                *self = Save::Saving { dirty: false };
                true
            }
        }
    }

    /// A write finished. Returns true when another write must start (edits arrived meanwhile).
    pub fn written(&mut self, result: Result<(), String>) -> bool {
        match (&*self, result) {
            (Save::Saving { dirty: true }, _) => {
                *self = Save::Saving { dirty: false };
                true
            }
            (Save::Saving { dirty: false }, Ok(())) => {
                *self = Save::Saved;
                false
            }
            (Save::Saving { dirty: false }, Err(e)) => {
                *self = Save::Failed(e);
                false
            }
            _ => false,
        }
    }

    /// The drawing finished loading (or failed to).
    pub fn loaded(&mut self, result: Result<(), String>) {
        *self = match result {
            Ok(()) => Save::Saved,
            Err(e) => Save::Failed(e),
        };
    }

    /// True when closing the editor now would lose edits: some wait behind a running write, or
    /// the last write failed. The caller writes once more before letting go.
    pub fn pending(&self) -> bool {
        matches!(self, Save::Saving { dirty: true } | Save::Failed(_))
    }
}

/// The appearance. `System` follows the computer's light or dark setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];
    pub const LABELS: [&'static str; 3] = ["System", "Light", "Dark"];

    /// Whether the dark palette is used, given the system's setting.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Theme::System => system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }

    /// The theme the toolbar button switches to: the opposite of what is on screen.
    pub fn toggled(self, system_dark: bool) -> Theme {
        if self.is_dark(system_dark) { Theme::Light } else { Theme::Dark }
    }

    pub fn index(self) -> usize {
        Theme::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// The word stored in the preferences and used by `REPOHUB_THEME`.
    pub fn name(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.name() == s.trim().to_ascii_lowercase())
    }
}

/// The sections of a repository's page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepoTab {
    #[default]
    Code,
    Issues,
    Pulls,
    Actions,
    Notes,
    Terminal,
    Settings,
}

impl RepoTab {
    pub const ALL: [RepoTab; 7] = [RepoTab::Code, RepoTab::Issues, RepoTab::Pulls, RepoTab::Actions, RepoTab::Notes, RepoTab::Terminal, RepoTab::Settings];
    pub const LABELS: [&'static str; 7] = ["Code", "Issues", "Pull requests", "Actions", "Notes", "Terminal", "Settings"];

    pub fn index(self) -> usize {
        RepoTab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> RepoTab {
        RepoTab::ALL.get(i).copied().unwrap_or_default()
    }

    /// A tab from its name (`notes`, `pulls`, `pull requests`…) or its number, for `-- tab`.
    pub fn parse(s: &str) -> Option<RepoTab> {
        let s = s.trim().to_ascii_lowercase();
        if let Ok(i) = s.parse::<usize>() {
            return RepoTab::ALL.get(i).copied();
        }
        RepoTab::ALL.into_iter().find(|t| RepoTab::LABELS[t.index()].to_ascii_lowercase() == s || format!("{t:?}").to_ascii_lowercase() == s)
    }
}

/// Which repositories the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepoFilter {
    #[default]
    All,
    Public,
    Private,
    Fork,
}

impl RepoFilter {
    pub const ALL: [RepoFilter; 4] = [RepoFilter::All, RepoFilter::Public, RepoFilter::Private, RepoFilter::Fork];
    pub const LABELS: [&'static str; 4] = ["All", "Public", "Private", "Fork"];

    pub fn index(self) -> usize {
        RepoFilter::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> RepoFilter {
        RepoFilter::ALL.get(i).copied().unwrap_or_default()
    }

    pub fn matches(self, private: bool, fork: bool) -> bool {
        match self {
            RepoFilter::All => true,
            RepoFilter::Public => !private,
            RepoFilter::Private => private,
            RepoFilter::Fork => fork,
        }
    }
}

/// How the repositories list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepoSort {
    /// GitHub's order: most recently pushed first.
    #[default]
    Recent,
    Stars,
    Name,
}

impl RepoSort {
    pub const ALL: [RepoSort; 3] = [RepoSort::Recent, RepoSort::Stars, RepoSort::Name];
    pub const LABELS: [&'static str; 3] = ["Recent", "Stars", "Name"];

    pub fn index(self) -> usize {
        RepoSort::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> RepoSort {
        RepoSort::ALL.get(i).copied().unwrap_or_default()
    }
}

/// Who owns the GitHub account setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    /// Set by the environment or a project `.env` (or by the server): the form can't change it.
    Locked,
    SignedOut,
    SignedIn,
}

/// What a toast is about; picks its icon and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Done,
    Problem,
}

/// The message floating at the bottom of the window. `id` lets its timer hide only itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub kind: ToastKind,
}

/// Prints a state change to stderr when `REPOHUB_TRACE=1`, one line each:
/// `state: page Overview -> Repos`. For debugging and for agents driving the app headless.
pub fn trace(what: &str, from: &dyn std::fmt::Debug, to: &dyn std::fmt::Debug) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *ON.get_or_init(|| std::env::var("REPOHUB_TRACE").is_ok_and(|v| v == "1")) {
            eprintln!("state: {what} {from:?} -> {to:?}");
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (what, from, to);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_rules() {
        assert!(Status::Starting.can_load() && Status::Ready.can_load() && Status::Failed("x".into()).can_load());
        assert!(!Status::Loading.can_load() && !Status::Sample.can_load());
        assert!(Status::Starting.is_busy() && !Status::Ready.is_busy());
    }

    #[test]
    fn save_queues_edits_made_while_writing() {
        let mut s = Save::Saved;
        assert!(s.edit(), "the first edit starts a write");
        assert!(!s.edit(), "a second edit waits for the running write");
        assert_eq!(s, Save::Saving { dirty: true });
        assert!(s.written(Ok(())), "the waiting edit is written next");
        assert_eq!(s, Save::Saving { dirty: false });
        assert!(!s.written(Ok(())));
        assert_eq!(s, Save::Saved);
        // A failure keeps the message until the next edit, which tries again.
        assert!(s.edit());
        assert!(!s.written(Err("disk full".into())));
        assert_eq!(s, Save::Failed("disk full".into()));
        assert!(s.pending() && s.edit());
        assert!(!Save::Saved.pending() && !(Save::Saving { dirty: false }).pending() && (Save::Saving { dirty: true }).pending());
        // Nothing is written while the drawing loads, and a stray result changes nothing.
        let mut l = Save::Loading;
        assert!(!l.edit() && !l.written(Ok(())));
        l.loaded(Ok(()));
        assert_eq!(l, Save::Saved);
    }

    #[test]
    fn theme_and_tabs() {
        assert_eq!(Theme::System.toggled(true), Theme::Light);
        assert_eq!(Theme::System.toggled(false), Theme::Dark);
        assert_eq!(Theme::Dark.toggled(false), Theme::Light);
        assert_eq!(Theme::parse("Dark"), Some(Theme::Dark));
        assert_eq!(Theme::parse("blue"), None);
        for (i, t) in RepoTab::ALL.into_iter().enumerate() {
            assert_eq!((t.index(), RepoTab::from_index(i)), (i, t));
        }
        assert_eq!(RepoTab::parse("notes"), Some(RepoTab::Notes));
        assert_eq!(RepoTab::parse("Pull requests"), Some(RepoTab::Pulls));
        assert_eq!(RepoTab::parse("5"), Some(RepoTab::Terminal));
        assert_eq!(RepoTab::from_index(99), RepoTab::Code);
        assert!(RepoFilter::Private.matches(true, false) && !RepoFilter::Fork.matches(true, false));
    }
}
