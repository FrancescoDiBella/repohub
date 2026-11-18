//! Updating the desktop app from inside it (`.github/workflows/release.yml` makes the builds).
//! Two channels: **Stable**, the numbered releases (`v0.1.0` tags), and **Preview**, the Latest
//! build that every push to `main` publishes. A numbered build follows Stable unless you pick
//! Preview in Settings; a Latest build follows Preview.
//!
//! The build knows its commit (`REPOHUB_BUILD`, set by the workflow) and when it was built. A
//! newer Latest build (another commit, published after this build) can be installed: the package
//! for this platform is downloaded, checked against the release's `SHA256SUMS`, put in place of
//! the running app, and the new app is started before this one quits.
//!
//! ```text
//! Idle ──check──▶ Checking ──▶ Current
//!                    │    └──▶ Failed(why) ──check──▶ Checking
//!                    ▼
//!               Available ──install──▶ Downloading(%) ──▶ Installing ──▶ (restart)
//!                                            └──────────────┴──▶ Failed(why)
//! ```
//! `Unavailable` replaces all of it where the app can't update itself: a build from source, the
//! web build, or `REPOHUB_UPDATES=off`.
use serde_json::Value;

/// Where the Latest build lives.
pub const REPO: &str = "FrancescoDiBella/repohub";

/// The commit this build was made from, when the release workflow built it.
pub fn build() -> Option<&'static str> {
    option_env!("REPOHUB_BUILD").filter(|s| !s.trim().is_empty())
}

/// This build's version: `0.1.0` for a numbered release, `0.1.0-dev` for the Latest build or a
/// build from source (`REPOHUB_VERSION`, set by the release workflow).
pub fn version() -> &'static str {
    option_env!("REPOHUB_VERSION").filter(|s| !s.trim().is_empty()).unwrap_or(concat!(env!("CARGO_PKG_VERSION"), "-dev"))
}

/// Where updates come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Numbered releases only.
    Stable,
    /// The Latest build of `main`, replaced on every push.
    Preview,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Stable, Channel::Preview];
    pub const LABELS: [&'static str; 2] = ["Stable", "Preview"];

    /// A numbered build starts on Stable, any other on Preview (what it was made from).
    pub fn for_version(version: &str) -> Channel {
        if version.contains('-') { Channel::Preview } else { Channel::Stable }
    }

    /// The saved choice, else the one for this build.
    pub fn load() -> Channel {
        match crate::prefs::get("update.channel").as_ref().and_then(Value::as_str) {
            Some("stable") => Channel::Stable,
            Some("preview") => Channel::Preview,
            _ => Channel::for_version(version()),
        }
    }

    pub fn save(self) {
        crate::prefs::set("update.channel", Value::from(match self {
            Channel::Stable => "stable",
            Channel::Preview => "preview",
        }));
    }

    pub fn index(self) -> usize {
        Channel::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    /// What the channel's builds are called in messages.
    pub fn what(self) -> &'static str {
        match self {
            Channel::Stable => "the latest release",
            Channel::Preview => "the Latest build",
        }
    }
}

/// `1.2.3` or `1.2.3-dev` as numbers and whether it's a pre-release.
fn semver(v: &str) -> Option<([u64; 3], bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut n = [0u64; 3];
    let mut parts = core.split('.');
    for slot in &mut n {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some((n, pre))
}

/// Whether version `a` comes after `b` (a pre-release comes before its release).
pub fn version_newer(a: &str, b: &str) -> bool {
    match (semver(a), semver(b)) {
        (Some((an, apre)), Some((bn, bpre))) => an > bn || (an == bn && bpre && !apre),
        _ => false,
    }
}

/// When this build was made (Unix seconds), when the release workflow built it.
pub fn built_at() -> Option<i64> {
    option_env!("REPOHUB_BUILD_DATE").and_then(crate::model::parse_iso)
}

/// The update's state (drawn above).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Update {
    /// The app can't update itself here; the text says how to update instead.
    Unavailable(&'static str),
    #[default]
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading { release: Release, percent: u8 },
    Installing,
    Failed(String),
}

impl Update {
    /// The state to start in.
    pub fn initial() -> Update {
        if cfg!(target_arch = "wasm32") {
            Update::Unavailable("The web version updates with its Docker image: docker compose pull && docker compose up -d.")
        } else if std::env::var("REPOHUB_UPDATES").is_ok_and(|v| matches!(v.trim(), "off" | "0" | "false")) {
            Update::Unavailable("Updates are off (REPOHUB_UPDATES=off).")
        } else if build().is_none() && option_env!("REPOHUB_VERSION").is_none() {
            Update::Unavailable("This copy was built from source: update it with git pull and cargo run --release.")
        } else {
            Update::Idle
        }
    }

    pub fn can_check(&self) -> bool {
        matches!(self, Update::Idle | Update::Current | Update::Failed(_) | Update::Available(_))
    }

    pub fn available(&self) -> Option<&Release> {
        match self {
            Update::Available(r) => Some(r),
            _ => None,
        }
    }

    /// Begins an update check, returning the previously available release when a check should run.
    /// Background checks (`asked == false`) are skipped while an update is already available.
    pub fn start_check(&mut self, asked: bool) -> Option<Option<Release>> {
        if !self.can_check() || (!asked && self.available().is_some()) {
            return None;
        }
        let prior = self.available().cloned();
        *self = Update::Checking;
        Some(prior)
    }

    /// Applies the result of an update check. Keeping any previously available release instead of
    /// overwriting it with `Failed` ensures a transient network failure never hides a ready update.
    pub fn finish_check(&mut self, prior: Option<Release>, outcome: Result<Option<Release>, String>) {
        *self = match outcome {
            Ok(Some(rel)) => Update::Available(rel),
            Ok(None) => Update::Current,
            Err(e) => match prior {
                Some(rel) => Update::Available(rel),
                None => Update::Failed(e),
            },
        };
    }
}

/// The Latest build, as this platform needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// The commit it was built from (for a numbered release GitHub may give a branch name).
    pub sha: String,
    /// A numbered release's version (`0.2.0`, from the `v0.2.0` tag); `None` for the Latest build.
    pub version: Option<String>,
    /// When it was published (Unix seconds).
    pub published: i64,
    /// This platform's package: its file name, download address and size in bytes.
    pub asset: String,
    pub url: String,
    pub size: u64,
    /// The address of `SHA256SUMS`, when the release has one.
    pub sums: Option<String>,
    /// The address of `SHA256SUMS.minisig`, its signature, when the release has one.
    pub sig: Option<String>,
    /// What's new: the release notes (CHANGELOG.md's Unreleased section), Markdown; may be empty.
    pub notes: String,
}

impl Release {
    /// The version of a numbered release, else the commit's first characters.
    pub fn short(&self) -> &str {
        match &self.version {
            Some(v) => v,
            None => &self.sha[..self.sha.len().min(7)],
        }
    }
}

/// The end of this platform's package name.
pub fn suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        "-macos.dmg"
    } else if cfg!(windows) {
        "-windows.zip"
    } else {
        "-linux-x86_64.tar.gz"
    }
}

/// Reads GitHub's answer for a release; `suffix` picks the package.
pub fn parse_release(json: &str, suffix: &str) -> Result<Release, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let assets = v.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let pick = |end: &str| assets.iter().find(|a| s(a, "name").ends_with(end)).cloned();
    let tag = s(&v, "tag_name");
    let version = tag.strip_prefix('v').filter(|t| semver(t).is_some()).map(str::to_owned);
    let what = if version.is_some() { "This release" } else { "The Latest build" };
    let asset = pick(suffix).ok_or_else(|| format!("{what} has no package for this system (*{suffix})."))?;
    let sha = s(&v, "target_commitish");
    if sha.is_empty() && version.is_none() {
        return Err("The Latest build doesn't say which commit it was built from.".into());
    }
    Ok(Release {
        sha,
        version,
        published: crate::model::parse_iso(&s(&v, "published_at")).unwrap_or(0),
        asset: s(&asset, "name"),
        url: s(&asset, "browser_download_url"),
        size: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
        sums: pick("SHA256SUMS").map(|a| s(&a, "browser_download_url")),
        sig: pick("SHA256SUMS.minisig").map(|a| s(&a, "browser_download_url")),
        notes: notes(&s(&v, "body")),
    })
}

/// The release notes in a release's text, without the Latest build's first line about itself.
pub fn notes(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.trim();
    let rest = match body.split_once("\n\n") {
        Some((first, rest)) if first.starts_with("Built automatically") => rest,
        None if body.starts_with("Built automatically") => "",
        _ => body,
    };
    rest.trim().to_owned()
}

/// Whether `release` should replace build `sha` built at `built`: another commit, published
/// after this build (a newer local build isn't "updated" to an older Latest build).
pub fn is_newer(release: &Release, sha: &str, built: Option<i64>) -> bool {
    // A numbered release is newer by its version.
    if let Some(v) = &release.version {
        return version_newer(v, version());
    }
    let same = release.sha.starts_with(sha) || sha.starts_with(&release.sha);
    !same && built.is_none_or(|b| release.published > b)
}

/// Checks `bytes` (the file `name`) against a `SHA256SUMS` text (`<hex>  <name>` lines).
#[cfg(not(target_arch = "wasm32"))]
pub fn verify(bytes: &[u8], name: &str, sums: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let want = sums.lines().find_map(|l| {
        let (hash, file) = l.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.trim().to_ascii_lowercase())
    }).ok_or_else(|| format!("SHA256SUMS doesn't list {name}."))?;
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got == want { Ok(()) } else { Err(format!("The download of {name} is damaged (its checksum doesn't match). Nothing was changed.")) }
}

/// The public key updates are signed with (minisign's base64 line), when this build has one:
/// CI builds it in from the repository's `REPOHUB_UPDATE_PUBKEY` variable. A build without it
/// (your own `cargo build`) can only check that a download isn't damaged.
pub fn signing_key() -> Option<&'static str> {
    option_env!("REPOHUB_UPDATE_PUBKEY").map(str::trim).filter(|k| !k.is_empty())
}

/// Checks that `sums` (the text of `SHA256SUMS`) was signed by the holder of `key`.
///
/// The checksums come from the same release as the package, so by themselves they only show
/// the download isn't damaged. The signature shows who made the release: someone who replaces
/// its files can't make one without the private key, which isn't on GitHub's servers.
/// With no `key` (a build that has none) there's nothing to check against.
#[cfg(not(target_arch = "wasm32"))]
pub fn verify_signature(sums: &str, signature: Option<&str>, key: Option<&str>) -> Result<(), String> {
    let Some(key) = key else { return Ok(()) };
    let refused = |why: &str| format!("This update wasn't installed: {why} Nothing was changed. If it happens again, download RepoHub from the Releases page.");
    let signature = signature.ok_or_else(|| refused("it isn't signed."))?;
    let key = minisign_verify::PublicKey::from_base64(key).map_err(|_| refused("this copy of RepoHub has a signing key it can't read."))?;
    let signature = minisign_verify::Signature::decode(signature).map_err(|_| refused("its signature can't be read."))?;
    key.verify(sums.as_bytes(), &signature, false).map_err(|_| refused("its signature isn't RepoHub's."))
}

/// True when an update may be downloaded from `url`: over https, from GitHub's release hosts.
pub fn trusted_url(url: &str) -> bool {
    const HOSTS: [&str; 3] = ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
    url.strip_prefix("https://").and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|authority| !authority.contains('@') && HOSTS.contains(&authority.to_ascii_lowercase().as_str()))
}

/// A path inside a PowerShell `'…'` string: a quote in it is doubled, or it would end the
/// string and the rest would run as a command (`C:\Users\O'Neil\…`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn powershell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "''"))
}

// ------------------------------------------------------------------ desktop: network and files

#[cfg(not(target_arch = "wasm32"))]
pub use native::{check, cleanup, download, install, relaunch};
#[cfg(all(test, not(target_arch = "wasm32")))]
pub use native::api;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{REPO, Release, parse_release, suffix};

    fn agent() -> ureq::Agent {
        let api = api();
        ureq::AgentBuilder::new()
            .try_proxy_from_env(!crate::net::bypass_proxy(&api))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(30))
            .user_agent("RepoHub updater")
            .build()
    }

    /// GitHub's API (`REPOHUB_UPDATE_API` points it elsewhere, for tests).
    pub fn api() -> String {
        std::env::var("REPOHUB_UPDATE_API").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "https://api.github.com".into()).trim_end_matches('/').to_owned()
    }

    /// Asks GitHub for the newest build on `channel`: the latest numbered release (GitHub's
    /// `releases/latest` skips pre-releases, so never the Latest build), or the Latest build.
    pub fn check(channel: super::Channel) -> Result<Release, String> {
        let path = match channel {
            super::Channel::Stable => "releases/latest",
            super::Channel::Preview => "releases/tags/latest",
        };
        let url = format!("{}/repos/{REPO}/{path}", api());
        let what = channel.what();
        let body = match agent().get(&url).set("Accept", "application/vnd.github+json").call() {
            Ok(r) => r.into_string().map_err(|e| e.to_string())?,
            Err(ureq::Error::Status(404, _)) => return Err(match channel {
                super::Channel::Stable => "There is no numbered release on GitHub yet: pick Preview for the Latest build.".into(),
                super::Channel::Preview => "There is no Latest build on GitHub yet.".into(),
            }),
            Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub answered {code} when asked for {what}.")),
            Err(e) => return Err(format!("Can't reach GitHub: {e}")),
        };
        parse_release(&body, suffix())
    }

    /// Whether downloads may come from anywhere: only where `REPOHUB_UPDATE_API` points the
    /// updater at a test server, in a test or debug build.
    fn any_host() -> bool {
        cfg!(any(test, debug_assertions)) && std::env::var("REPOHUB_UPDATE_API").is_ok_and(|v| !v.trim().is_empty())
    }

    /// Asks for `url`, following redirects itself so that every address on the way is one of
    /// GitHub's release hosts (a release's files are served from another host than its page).
    fn fetch(url: &str) -> Result<ureq::Response, String> {
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(!crate::net::bypass_proxy(&api()))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout_read(std::time::Duration::from_secs(30))
            .user_agent("RepoHub updater")
            .redirects(0)
            .build();
        let mut url = url.to_owned();
        for _ in 0..6 {
            if !any_host() && !super::trusted_url(&url) {
                return Err("This update wasn't downloaded: its address isn't on GitHub. Nothing was changed.".into());
            }
            let resp = match agent.get(&url).call() {
                Ok(r) => r,
                Err(ureq::Error::Status(code, _)) => return Err(format!("Couldn't download the update: the server answered {code}.")),
                Err(e) => return Err(format!("Couldn't download the update: {e}")),
            };
            if !(300..400).contains(&resp.status()) {
                return Ok(resp);
            }
            url = resp.header("Location").map(str::to_owned).ok_or("Couldn't download the update: a redirect without an address.")?;
        }
        Err("Couldn't download the update: too many redirects.".into())
    }

    /// Downloads `url`, reporting progress (0–100) when the size is known, stopping if `cancel` is set.
    pub fn download(url: &str, size: u64, cancel: &std::sync::atomic::AtomicBool, progress: impl Fn(u8)) -> Result<Vec<u8>, String> {
        use std::sync::atomic::Ordering;
        if cancel.load(Ordering::Relaxed) {
            return Err("Download cancelled.".into());
        }
        let resp = fetch(url)?;
        let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(size).max(1);
        let mut out = Vec::with_capacity(total as usize);
        let mut reader = resp.into_reader().take(512 << 20);
        let mut buf = [0u8; 64 * 1024];
        let mut last = 0u8;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Download cancelled.".into());
            }
            let n = reader.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            let pct = ((out.len() as u64 * 100) / total).min(100) as u8;
            if pct >= last + 5 {
                last = pct;
                progress(pct);
            }
        }
        Ok(out)
    }

    fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
        crate::config::scrub_child_env(cmd);
        let out = cmd.output().map_err(|e| format!("Couldn't {what}: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("Couldn't {what}: {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }

    /// A scratch folder for one update, next to `near` so moving out of it stays on one disk.
    fn scratch(near: &Path) -> Result<PathBuf, String> {
        let dir = near.join(format!(".repohub-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Can't write next to the app ({}): {e}. Move RepoHub to a folder you can write to, or update by hand.", near.display()))?;
        Ok(dir)
    }

    /// Puts the package `bytes` (file `name`) in place of the app whose program is `exe`, and
    /// returns what to start afterwards: the app bundle on macOS, the program elsewhere.
    pub fn install(bytes: &[u8], name: &str, exe: &Path) -> Result<PathBuf, String> {
        if cfg!(target_os = "macos") {
            install_macos(bytes, exe)
        } else {
            install_binary(bytes, name, exe)
        }
    }

    /// macOS: the `.app` inside the `.dmg` replaces the bundle `exe` belongs to.
    fn install_macos(bytes: &[u8], exe: &Path) -> Result<PathBuf, String> {
        let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
            .ok_or("RepoHub isn't running from RepoHub.app, so it can't replace itself. Install the new version by hand.")?;
        let parent = bundle.parent().ok_or("no folder above RepoHub.app")?.to_path_buf();
        let tmp = scratch(&parent)?;
        let dmg = tmp.join("update.dmg");
        std::fs::write(&dmg, bytes).map_err(|e| e.to_string())?;
        let mnt = tmp.join("mnt");
        run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-mountpoint"]).arg(&mnt).arg(&dmg), "open the disk image")?;
        let staged = tmp.join("RepoHub.app");
        let copied = run(Command::new("ditto").arg(mnt.join("RepoHub.app")).arg(&staged), "copy the new app");
        let mut detach = Command::new("hdiutil");
        crate::config::scrub_child_env(&mut detach);
        let _ = detach.args(["detach", "-quiet"]).arg(&mnt).output();
        copied?;
        // The new app must be intact and signed as a whole before it replaces this one.
        if let Err(e) = run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&staged), "check the new app's signature") {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("{e}. Nothing was changed."));
        }
        let old = parent.join(".RepoHub-old.app");
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(&bundle, &old).map_err(|e| format!("Couldn't move the old app aside: {e}"))?;
        if let Err(e) = std::fs::rename(&staged, &bundle) {
            let _ = std::fs::rename(&old, &bundle);
            return Err(format!("Couldn't put the new app in place: {e}"));
        }
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(bundle)
    }

    /// Windows and Linux: the program inside the `.zip` / `.tar.gz` replaces `exe`. The running
    /// program can be renamed but not overwritten on Windows, so it moves aside first.
    fn install_binary(bytes: &[u8], name: &str, exe: &Path) -> Result<PathBuf, String> {
        let dir = exe.parent().ok_or("no folder above the program")?.to_path_buf();
        let tmp = scratch(&dir)?;
        let archive = tmp.join(name);
        std::fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let out = tmp.join("x");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        if name.ends_with(".zip") {
            run(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!("Expand-Archive -LiteralPath {} -DestinationPath {} -Force",
                    super::powershell_quote(&archive.display().to_string()), super::powershell_quote(&out.display().to_string()))), "unpack the update")?;
        } else {
            run(Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&out), "unpack the update")?;
        }
        let want = if cfg!(windows) { "repohub.exe" } else { "repohub" };
        let new = find(&out, want).ok_or_else(|| format!("The update has no {want} inside."))?;
        let old = dir.join(if cfg!(windows) { "RepoHub.old.exe" } else { ".repohub.old" });
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(|e| format!("Couldn't move the old program aside: {e}"))?;
        if let Err(e) = std::fs::copy(&new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(format!("Couldn't put the new program in place: {e}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
            let _ = std::fs::remove_file(&old);
        }
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(exe.to_path_buf())
    }

    /// The file named `want` (any case) somewhere under `dir`.
    fn find(dir: &Path, want: &str) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = find(&p, want) {
                    return Some(f);
                }
            } else if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(want)) {
                return Some(p);
            }
        }
        None
    }

    /// Starts the new app a moment after this one quits.
    pub fn relaunch(target: &Path) -> Result<(), String> {
        let spawned = if cfg!(target_os = "macos") {
            let mut cmd = Command::new("sh");
            crate::config::scrub_child_env(&mut cmd);
            cmd.args(["-c", "sleep 1; open \"$0\""]).arg(target).spawn()
        } else if cfg!(windows) {
            let mut cmd = Command::new(target);
            crate::config::scrub_child_env(&mut cmd);
            cmd.spawn()
        } else {
            let mut cmd = Command::new("sh");
            crate::config::scrub_child_env(&mut cmd);
            cmd.args(["-c", "sleep 1; exec \"$0\""]).arg(target).spawn()
        };
        spawned.map(|_| ()).map_err(|e| format!("The update is installed, but RepoHub couldn't restart: {e}. Open it again."))
    }

    /// At start: removes what an earlier update left behind (the old program on Windows).
    pub fn cleanup() {
        let Ok(exe) = std::env::current_exe() else { return };
        if let Some(dir) = exe.parent() {
            let _ = std::fs::remove_file(dir.join("RepoHub.old.exe"));
        }
        if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
            && let Some(parent) = bundle.parent()
        {
            let _ = std::fs::remove_dir_all(parent.join(".RepoHub-old.app"));
        }
    }
}

#[cfg(test)]
mod tests {
    // A key pair made for these tests (its private half was thrown away) and its signature of SUMS.
    #[cfg(not(target_arch = "wasm32"))]
    const KEY: &str = "RWRBZcvGo7o0bdNeYr3KOfnpnyZq6H83eII8FrqVGQkeiOpp3X+6Sirb";
    #[cfg(not(target_arch = "wasm32"))]
    const OTHER_KEY: &str = "RWQJmXGn7wtVFS6/nHe3hg9vc7IcPvgJCKlIEVikpSDrcKH5Bl7qYXV4";
    #[cfg(not(target_arch = "wasm32"))]
    const SUMS: &str = "0f3a  RepoHub-0.1.0-dev-linux-x86_64.tar.gz\n";
    #[cfg(not(target_arch = "wasm32"))]
    const SIGNATURE: &str = "untrusted comment: signature from rsign secret key\nRURBZcvGo7o0bXPwTCJAeQw/bqVbVhVBsgOMVZ9psp1TFTiyclpluwFct1bqep2G9AGNJMLZiR4VB790zLA+Mt2flFCFGOPrtAA=\ntrusted comment: RepoHub test\nHITnbHVk/tPNBskSb5yZrcHy+sCHcilJzj6bDMev/Pz5qXiZ4YVWajxmHefBFMi06f+WZCfV36UBHD0wq3nlDA==\n";

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn only_signed_checksums_are_accepted_by_a_build_with_a_key() {
        assert_eq!(verify_signature(SUMS, Some(SIGNATURE), Some(KEY)), Ok(()));
        // The checksums were changed after signing (someone replaced the package and its sum).
        let tampered = SUMS.replace("0f3a", "beef");
        assert!(verify_signature(&tampered, Some(SIGNATURE), Some(KEY)).unwrap_err().contains("isn't RepoHub's"));
        // Valid checksums with no signature at all.
        assert!(verify_signature(SUMS, None, Some(KEY)).unwrap_err().contains("isn't signed"));
        // Signed, but by someone else's key.
        assert!(verify_signature(SUMS, Some(SIGNATURE), Some(OTHER_KEY)).unwrap_err().contains("isn't RepoHub's"));
        assert!(verify_signature(SUMS, Some("not a signature"), Some(KEY)).unwrap_err().contains("can't be read"));
        for e in [verify_signature(&tampered, Some(SIGNATURE), Some(KEY)), verify_signature(SUMS, None, Some(KEY))] {
            assert!(e.unwrap_err().contains("Nothing was changed"));
        }
        // A build without a key (your own) has nothing to check a signature against.
        assert_eq!(verify_signature(SUMS, None, None), Ok(()));
    }

    #[test]
    fn updates_come_from_githubs_release_hosts_only() {
        for ok in ["https://github.com/FrancescoDiBella/repohub/releases/download/latest/RepoHub.zip", "https://objects.githubusercontent.com/x?y=1", "https://release-assets.githubusercontent.com/a/b", "https://GitHub.com/x"] {
            assert!(trusted_url(ok), "{ok}");
        }
        for bad in ["http://github.com/x", "https://github.com.evil.example/x", "https://evil.example/github.com/x", "https://github.com@evil.example/x", "https://github.com:8443/x", "https://raw.githubusercontent.com/x", "file:///tmp/x", ""] {
            assert!(!trusted_url(bad), "{bad}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_download_from_another_host_is_refused_before_any_request() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::env::var_os("REPOHUB_UPDATE_API");
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::remove_var("REPOHUB_UPDATE_API") };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let err = download("https://evil.example/RepoHub.zip", 0, &cancel, |_| {}).unwrap_err();
        // SAFETY: as above.
        unsafe {
            if let Some(v) = before {
                std::env::set_var("REPOHUB_UPDATE_API", v);
            }
        }
        assert!(err.contains("isn't on GitHub"), "{err}");
    }

    #[test]
    fn the_signature_file_is_found_next_to_the_checksums() {
        let json = r#"{"target_commitish":"abc1234","published_at":"2026-11-10T10:00:00Z","body":"",
          "assets":[{"name":"RepoHub-0.1.0-dev-linux-x86_64.tar.gz","browser_download_url":"https://x/linux.tgz","size":20},
                    {"name":"SHA256SUMS.minisig","browser_download_url":"https://x/sums.minisig","size":3},
                    {"name":"SHA256SUMS","browser_download_url":"https://x/sums","size":3}]}"#;
        let r = parse_release(json, "-linux-x86_64.tar.gz").unwrap();
        assert_eq!((r.sums.as_deref(), r.sig.as_deref()), (Some("https://x/sums"), Some("https://x/sums.minisig")));
    }

    #[test]
    fn a_quote_in_the_install_folder_stays_inside_the_powershell_string() {
        assert_eq!(powershell_quote(r"C:\Users\O'Neil\RepoHub"), r"'C:\Users\O''Neil\RepoHub'");
        assert_eq!(powershell_quote("a'; Remove-Item x; '"), "'a''; Remove-Item x; '''");
    }
    use super::*;

    const RELEASE: &str = r#"{"tag_name":"latest","target_commitish":"abc1234def","published_at":"2026-11-11T12:36:05Z",
      "assets":[{"name":"RepoHub-0.1.0-dev-macos.dmg","browser_download_url":"https://x/mac.dmg","size":10},
                {"name":"RepoHub-0.1.0-dev-linux-x86_64.tar.gz","browser_download_url":"https://x/linux.tgz","size":20},
                {"name":"SHA256SUMS","browser_download_url":"https://x/SHA256SUMS","size":1}]}"#;

    #[test]
    fn release_and_newer() {
        let r = parse_release(RELEASE, "-linux-x86_64.tar.gz").unwrap();
        assert_eq!((r.asset.as_str(), r.url.as_str(), r.size), ("RepoHub-0.1.0-dev-linux-x86_64.tar.gz", "https://x/linux.tgz", 20));
        assert_eq!(r.sums.as_deref(), Some("https://x/SHA256SUMS"));
        assert_eq!(r.short(), "abc1234");
        assert!(parse_release(RELEASE, "-windows.zip").unwrap_err().contains("no package"));
        let built = crate::model::parse_iso("2026-11-10T00:00:00Z");
        assert!(is_newer(&r, "0000000", built), "another commit, published later");
        assert!(!is_newer(&r, "abc1234def", built), "the same commit");
        assert!(!is_newer(&r, "abc1234", None), "a short sha of the same commit");
        assert!(!is_newer(&r, "fff0000", crate::model::parse_iso("2026-11-12T00:00:00Z")), "this build is newer than the release");
        assert_eq!(r.notes, "");
    }

    #[test]
    fn numbered_releases_compare_by_version() {
        assert!(version_newer("0.2.0", "0.1.0") && version_newer("0.1.0", "0.1.0-dev") && version_newer("1.0.0", "0.9.9"));
        assert!(!version_newer("0.1.0", "0.1.0") && !version_newer("0.1.0-dev", "0.1.0") && !version_newer("0.1.0", "0.2.0-dev"));
        assert!(!version_newer("latest", "0.1.0") && !version_newer("0.1", "0.0.1"), "not a version");
        let json = RELEASE.replace(r#""tag_name":"latest","target_commitish":"abc1234def""#, r#""tag_name":"v9.0.0","target_commitish":"main""#);
        let r = parse_release(&json, "-linux-x86_64.tar.gz").unwrap();
        assert_eq!((r.version.as_deref(), r.short()), (Some("9.0.0"), "9.0.0"));
        assert!(is_newer(&r, "abc", None), "a higher version than this build");
        let old = RELEASE.replace(r#""tag_name":"latest""#, r#""tag_name":"v0.0.1""#);
        assert!(!is_newer(&parse_release(&old, "-linux-x86_64.tar.gz").unwrap(), "fff", None), "an older release never replaces this build");
        // A Latest build is still told apart by its commit.
        assert_eq!(parse_release(RELEASE, "-linux-x86_64.tar.gz").unwrap().version, None);
        assert_eq!(Channel::for_version("0.1.0"), Channel::Stable);
        assert_eq!(Channel::for_version("0.1.0-dev"), Channel::Preview);
    }

    #[test]
    fn release_notes() {
        assert_eq!(notes("Built automatically from main at abc1234. It is replaced on every push.\r\n\r\n### Added\r\n- Pins"), "### Added\n- Pins");
        assert_eq!(notes("Built automatically from main at abc1234."), "");
        assert_eq!(notes("### Fixed\n- A crash\n\n### Added\n- B"), "### Fixed\n- A crash\n\n### Added\n- B");
    }

    #[test]
    fn checksums() {
        // sha256("hello")
        let sums = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  pkg.tar.gz\nffff  other\n";
        assert!(verify(b"hello", "pkg.tar.gz", sums).is_ok());
        assert!(verify(b"hellO", "pkg.tar.gz", sums).unwrap_err().contains("damaged"));
        assert!(verify(b"hello", "missing", sums).unwrap_err().contains("doesn't list"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn installs_a_new_program_in_place() {
        let base = std::env::temp_dir().join(format!("repohub-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pkg = base.join("pkg/repohub-1.0");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("repohub"), "#!/bin/sh\necho new\n").unwrap();
        let tgz = base.join("RepoHub-1.0-linux-x86_64.tar.gz");
        assert!(std::process::Command::new("tar").arg("-czf").arg(&tgz).arg("-C").arg(base.join("pkg")).arg("repohub-1.0").status().unwrap().success());
        let app = base.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let exe = app.join("repohub");
        std::fs::write(&exe, "old").unwrap();
        let bytes = std::fs::read(&tgz).unwrap();
        let started = install(&bytes, "RepoHub-1.0-linux-x86_64.tar.gz", &exe).unwrap();
        assert_eq!(started, exe);
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "#!/bin/sh\necho new\n");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0o111, "executable");
        assert_eq!(std::fs::read_dir(&app).unwrap().count(), 1, "nothing left behind");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn background_check_keeps_available() {
        let r = parse_release(RELEASE, "-linux-x86_64.tar.gz").unwrap();

        // Background -> background: skipped while Available, Failed when Idle.
        let mut u = Update::Available(r.clone());
        assert_eq!(u.start_check(false), None, "background check skips Available");
        assert_eq!(u, Update::Available(r.clone()));

        let mut u = Update::Idle;
        let prior = u.start_check(false).unwrap();
        assert_eq!(u, Update::Checking);
        u.finish_check(prior, Err("offline".into()));
        assert_eq!(u, Update::Failed("offline".into()));

        // Manual -> manual: preserves Available on transient failure, or updates to newer.
        let mut u = Update::Available(r.clone());
        let prior = u.start_check(true).expect("manual check can run from Available");
        assert_eq!(u, Update::Checking);
        u.finish_check(prior, Err("offline".into()));
        assert_eq!(u, Update::Available(r.clone()), "failed manual check preserves Available");

        let mut u = Update::Idle;
        let prior = u.start_check(true).unwrap();
        u.finish_check(prior, Err("offline".into()));
        assert_eq!(u, Update::Failed("offline".into()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cancelled_download_stops_immediately() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let err = download("http://127.0.0.1:0/never", 0, &cancel, |_| {}).unwrap_err();
        assert!(err.contains("cancelled"));
    }
}
