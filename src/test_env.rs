//! The environment the tests run in: the same on every machine.
//!
//! The computer running the tests may have an account: `GITHUB_TOKEN` in the environment (this
//! project's CI and its developers' shells do), a project `.env` found upward from the working
//! folder, or one saved in Settings in the data folder. With any of them `crate::init()` starts
//! live, loads from the real GitHub with that token, and a test then sees whatever that account
//! has, or a load of it landing in the middle of another test. Taking [`Lock`] (`crate::ENV_LOCK`,
//! which every test that touches the environment or calls `init` holds) clears all three first.

/// Tests that change process environment variables hold this, so they don't race.
pub(crate) struct Lock(std::sync::Mutex<()>);

impl Lock {
    pub(crate) const fn new() -> Lock {
        Lock(std::sync::Mutex::new(()))
    }

    /// Waits for the other tests, then leaves no account in the environment: no token, user or
    /// API from the machine, no project `.env`, and a data folder of the tests' own.
    pub(crate) fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, ()>> {
        let guard = self.0.lock();
        // SAFETY: the lock is held, and the tests that read or change the environment hold it.
        unsafe {
            for k in ["GITHUB_TOKEN", "GITHUB_USER", "GITHUB_API_URL", "REPOHUB_BLOCKING", "REPOHUB_DEMO"] {
                std::env::remove_var(k);
            }
            // An explicit `.env` that isn't there: the upward search for a project's doesn't run.
            std::env::set_var("REPOHUB_ENV", std::env::temp_dir().join(format!("repohub-tests-{}-no.env", std::process::id())));
            if std::env::var_os("REPOHUB_DATA_DIR").is_none() {
                std::env::set_var("REPOHUB_DATA_DIR", std::env::temp_dir().join(format!("repohub-tests-{}", std::process::id())));
            }
        }
        guard
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A token in the machine's environment (as on this project's CI) doesn't make the app in a
    /// test live: it starts on the sample data and asks GitHub nothing.
    #[test]
    fn the_machines_account_never_reaches_a_test() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                count.fetch_add(1, Ordering::SeqCst);
                let _ = stream.read(&mut [0u8; 512]);
            }
        });
        {
            let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // What the machine running the tests has.
            // SAFETY: ENV_LOCK is held.
            unsafe {
                std::env::set_var("GITHUB_TOKEN", "the-machines-token");
                std::env::set_var("GITHUB_API_URL", &api);
            }
        }
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = crate::init();
        assert!(!s.cfg.live() && s.cfg.token.is_none(), "no account from the machine");
        assert!(s.data.sample, "the sample data");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(asked.load(Ordering::SeqCst), 0, "GitHub (here a stand-in at GITHUB_API_URL) was asked nothing");
    }
}
