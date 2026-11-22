//! Start-up and the frame loop: `start`, the 30-second `tick` and the clock it reads, `poll`,
//! closing the window, the Android back button.

use super::*;

impl App {
    /// Seconds since `wall` (Unix seconds) and `mono` (frame clock), whichever measured more.
    fn since(&self, wall: Option<i64>, mono: Option<f64>) -> Option<i64> {
        let w = wall.map(|t| self.now - t);
        let m = mono.map(|c| (self.clock - c) as i64);
        w.max(m).map(|d| d.max(0))
    }

    /// "Updated 3 minutes ago", once GitHub's data has loaded ("Updating…" while it loads
    /// again, "Data from 2 hours ago" for the saved copy).
    pub(crate) fn freshness(&self) -> Option<String> {
        self.freshness_words(self.since(self.loaded_at, self.loaded_clock))
    }

    /// Seconds until the next automatic refresh (negative: overdue), if they're on. The
    /// interval counts from when the last load started, so a failed load waits a whole one.
    pub(crate) fn refresh_in(&self) -> Option<i64> {
        let every = self.cfg.refresh_every()? as i64 * 60;
        let interval = match self.retry_after.or(self.data.retry_after) {
            Some(ra) => every.max(ra as i64),
            None => every,
        };
        let since = self.since(self.last_load.or(self.loaded_at), self.load_clock.or(self.loaded_clock))?;
        Some(interval - since)
    }

    /// Every 30 seconds: move the labels' clock, and refresh when it's due. A load already
    /// running, the sample data, or a refresh turned off skip it.
    fn tick(&mut self) {
        self.now = net::now();
        if self.refresh_in().is_some_and(|left| left <= 0) && self.status.can_load() {
            state::trace("refresh", &"due", &self.cfg.refresh_every());
            self.refresh();
        }
        self.save_layout();
        self.save_filters();
        // A new Latest build is looked for every 6 hours, unless one is already waiting.
        if self.update.available().is_none() && self.update_checked.is_some_and(|c| self.clock - c >= 6.0 * 3600.0) {
            self.check_update(false);
        }
        self.sync_tray();
    }
}

/// With the window: the image decoder for backgrounds.
fn setup(s: &mut App, waker: rune::Waker) {
    #[cfg(not(target_arch = "wasm32"))]
    s.plugins.set_waker(waker.clone());
    s.images = Some(rune::ImageCache::new(waker));
}

/// Runs at the start of every frame with Rune's clock: ticks every 30 seconds (`App::tick`) and
/// asks Rune to wake for the next one, also while the window is closed. No timer thread.
pub(crate) fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    const EVERY: f64 = 30.0;
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    if FIRST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        cache::trace_time("first frame");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let plugins_ran = {
        if s.plugins.list.iter().any(|p| p.running()) {
            // The context is rebuilt only when what it shows may have changed.
            let key = plugins::context_key(s, s.shown_dark.get());
            if s.plugin_ctx_key != key {
                s.plugin_ctx_key = key;
                let ctx = plugins::context(s, s.shown_dark.get());
                s.plugins.set_context(ctx);
            }
        }
        s.plugins.poll(clock, tx)
    };
    #[cfg(target_arch = "wasm32")]
    let plugins_ran = false;
    s.clock = clock;
    if std::mem::take(&mut s.reveal_tab) {
        pages::reveal_tab(s, tx);
    }
    let due = clock - s.tick_at >= EVERY;
    if due {
        s.tick_at = clock;
        s.tick();
    }
    tx.wake_after(std::time::Duration::from_secs_f64((EVERY - (clock - s.tick_at)).max(1.0)));
    due || plugins_ran
}

/// The window closed: RepoHub stays in the status bar, unless its icon couldn't be shown.
fn on_close(s: &mut App, tx: &mut Tx) {
    #[cfg(not(target_arch = "wasm32"))]
    if s.tray.is_none() {
        tx.quit();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (s, tx);
}

/// Inter for the interface and IBM Plex Mono for code and the terminal (both SIL OFL, see
/// `assets/fonts`). The mono slot prefers the system's monospaced font (SF Mono on macOS).
const FONTS: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf"),
];

const FAMILIES: (&str, &str) = ("Inter", "ui-monospace, IBM Plex Mono");

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// The running app, for [`repohub_back`].
    static BACK: std::cell::RefCell<Option<AppHandle<App>>> = const { std::cell::RefCell::new(None) };
}

/// The Android app calls this when the back button is pressed.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn repohub_back() {
    BACK.with(|b| {
        if let Some(h) = b.borrow().as_ref() {
            h.update(|s: &mut App, _| s.back());
        }
    });
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen(start))]
pub fn start() {
    let app = rune::App::new(AppSpec { on_key: Some(on_key), poll: Some(poll), setup: Some(setup), fonts: FONTS, families: FAMILIES, ..AppSpec::new("RepoHub", (1180.0, 760.0), init, view) });
    // macOS: draw with wgpu's Metal backend rather than Rune's own, which makes coloured parts of
    // the window flicker on every key press and pointer move (#108). `RUNE_GPU_BACKEND=metal`
    // still picks Rune's, to check whether a newer Rune fixed it.
    let mut app = app.gpu_backend(rune::GpuBackend::Wgpu);
    // With the status-bar widget, closing the window keeps RepoHub there (macOS, Windows).
    if tray::keeps_running() {
        app = app.keep_running().on_close(on_close);
    }
    let app = app.on_quit(|s: &mut App, _| { s.save_layout(); s.save_filters(); });
    let handle = app.handle();
    let h2 = handle.clone();
    handle.update(move |s: &mut App, _| {
        s.handle = Some(h2.clone());
        // What the first load found before the window opened.
        if let Some(m) = s.fresh.early.take() {
            m.attach(h2.clone());
        }
        // The place of last time, over the saved copy (before the first frame).
        if !s.data.repos.is_empty() {
            s.restore_place();
        }
        s.start_tray();
        #[cfg(not(target_arch = "wasm32"))]
        s.plugins.start(&h2);
        // Clear what an earlier update left, then look for a newer build.
        #[cfg(not(target_arch = "wasm32"))]
        update::cleanup();
        s.check_update(false);
        #[cfg(not(target_arch = "wasm32"))]
        if s.status == Status::Starting {
            s.refresh(); // the first load, when the app starts with an account
        }
        // The web build asks repohub-server for its configuration; a static host (GitHub Pages)
        // has none, and then shows the sample data.
        #[cfg(target_arch = "wasm32")]
        net::spawn(h2.clone(), net::fetch_config, |s: &mut App, cfg| {
            let live = cfg.as_ref().is_some_and(|c| c.live());
            // The phone app has a configuration before it has an account.
            if let Some(cfg) = cfg.filter(|c| live || c.device) {
                if live && prefs::load_theme().is_none() {
                    s.theme = cfg.theme;
                }
                s.acct_user = TextEdit::new(cfg.user.clone().unwrap_or_default());
                s.cfg = Arc::new(cfg);
            }
            if live {
                s.show_saved_copy();
                s.refresh();
            } else {
                s.data = Data::sample();
                s.data_gen = s.data_gen.wrapping_add(1);
                s.set_status(Status::Sample);
            }
        });
        #[cfg(target_arch = "wasm32")]
        BACK.with(|b| *b.borrow_mut() = Some(h2.clone()));
    });
    app.run();
}
