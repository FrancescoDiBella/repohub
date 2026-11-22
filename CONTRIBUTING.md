# Contributing to RepoHub

## Layout

| Path | What |
| --- | --- |
| `src/lib.rs` | App state, background loading, sidebar, toolbar, banners |
| `src/pages.rs`, `src/lists.rs` | Pages: overview, repositories, repository detail, pull requests, issues, Actions, settings |
| `src/mobile.rs` | The phone layout (top bar and bottom tab bar) used below `mobile::BREAKPOINT`; pages read `App::compact` |
| `src/ui.rs`, `src/icons.rs` | Small building blocks and the icon set |
| `src/state.rs` | The state machines: where data comes from, loading, saving, theme, tabs, filters. Drawn in [`docs/STATE.md`](docs/STATE.md) |
| `src/model.rs` | Data model, sample data, date and colour helpers |
| `src/github.rs` | GitHub REST client: async, shared by the desktop and web builds. Every GET goes through `Gh::fetch` (conditional, see `etag`); `load_all_with` runs the parts of a load side by side and reports the profile and repositories early |
| `src/net.rs` | Transport: blocking `ureq` on the desktop, `fetch` to the `/gh` proxy on the web, both sending `If-None-Match` and reading `ETag`; `spawn` runs work in the background and reports to the UI; `join`/`join_all` (scoped threads on the desktop, polled futures on the web), `Gate` (at most N requests at once), `Mailbox` (the first load, started before the window) |
| `src/etag.rs` | Conditional requests: each GET's ETag with its answer, shared by the process, cleared on sign-out; searches left out |
| `src/cache.rs`, `src/cache_page.rs` | The saved copy of the last load: its format (versioned), where it lives, the size cap, writing it in the background and removing it; showing it at start, the first part of a load, what's kept to show at once (`Fresh`); Settings → About → Saved data |
| `src/load_tests.rs` | A mock GitHub on localhost: the parallel load equals the sequential one, 304s reuse kept answers, at most four at once; the saved copy in the app |
| `src/config.rs` | `.env` parsing (desktop: the project file and the app's own settings file, where Settings saves the account) and `config.json` (web) |
| `src/board.rs` | The Notes whiteboard: model, hand-drawn rendering, gestures, undo/redo, `.excalidraw` read/write |
| `src/actions.rs` | The branch picker, the New issue / branch / pull request forms, Close and Merge on rows |
| `src/notes.rs`, `src/notes_page.rs` | Notes storage (local folder on desktop, `/api/notes` on the web: saving that merges with a file changed underneath, conditional writes, the trash, the list of every drawing, sync's index) and the Notes tab |
| `src/sync.rs` | Boards on every device, the engine (pure, tested): git blob ids and base64, notes-repository paths, the index, the decision table (`decide`, `plan`), merging two boards (`merge`), and the run over the `Remote` and `Local` traits, with fakes in its tests |
| `src/sync_io.rs`, `src/sync_page.rs` | Board sync wired up: GitHub as the notes repository (`GhRemote`), the notes storage (`NotesLocal`), checking and choosing the repository, when sync runs, the app's sync state; the Notes header, the board marks and Settings → Account → Boards |
| `src/board_file.rs` | **Export…** and **Import…** of one board: Rune's file dialogs on the desktop, a download and the file picker on the web, the `RepoHubNative` pickers on Android |
| `src/triage.rs`, `src/triage_page.rs` | Triage: a repository's labels, assignable people and milestones (kept per repository), the picker (filter, ticks, what changed), each change's request and GitHub's answer, applying it to the conversation and the rows; the details block, the pickers, title and description editing, the New issue form's labels and assignees, the Overview's **Waiting for your review** |
| `src/palette.rs` | The command palette (⌘K): the commands for the current state, the fuzzy ranking (both tested), the modal layer |
| `src/update.rs` | In-app updates: the build's commit (`REPOHUB_BUILD`, set by the Desktop workflow), the Latest build's package for this platform, the SHA256SUMS check, install and restart |
| `src/themes.rs`, `src/themes_page.rs` | Themes: the ready-made looks, yours (`Looks`, saved in the preferences), the `Kit` built from a look, background patterns and pictures; the gallery and editor in Settings |
| `src/prefs.rs` | What outlives the app: theme, pins, the place you were (`state::Place`) and the arrangement (`state::Layout`), one key each in `prefs.json` / `localStorage` |
| `src/tray.rs` | The status-bar widget: the menu (a pure function of the state, tested), Rune's `Tray` on Windows and Linux, `tray-icon` on macOS; icons in `assets/tray-*.rgba` |
| `src/claude.rs`, `src/claude_page.rs` | The Claude tab (desktop): runs `claude -p --output-format stream-json` in the clone, parses the stream into chat items (`Run`, `Mode`, `Item`), the chat UI |
| `src/plugins.rs`, `src/plugins_page.rs`, `plugins/checklist/` | Plugins (desktop): the manifest, finding, installing and starting Luau scripts with `rune-luau`, the `@repohub` module and its context; Settings → Plugins; the example plugin. `src/plugins_web.rs` stands in on the web. Guide: [`docs/PLUGINS.md`](docs/PLUGINS.md) |
| `src/harness.rs` | Coding assistants: the Claude switch and the harnesses (name and command), saved in the preferences; Settings → Coding assistants is `lists::assistants_panel` |
| `src/terminal.rs`, `src/terminal_page.rs` | Embedded terminal (desktop): `portable-pty` runs the login shell, `vt100` keeps the screen; key mapping, colours, the Terminal tab |
| `src/local.rs`, `src/local_page.rs` | The local clone read from disk: `git` run on the computer, the file list (`git ls-files`, `git status`), reading a file safely, and the Code tab's folder listing and file viewer |
| `src/commits.rs`, `src/commits_page.rs` | The Commits section: reading GitHub's commit list, one commit and a comparison (tested), the day groups and the filter, the sample history, loading with late answers dropped; the history, the commit page and the comparison form |
| `src/releases.rs`, `src/releases_page.rs` | The Releases section: reading GitHub's releases and tags (tested), the next versions and the tag check, the sample releases, loading with late answers dropped, saving and publishing (publishing only after its question); the list, the tags without a release, the form, and the latest release's tag in the header |
| `src/diffs.rs`, `src/changes_page.rs` | Diffs with rune-kit's `DiffView`: the local changes (`local::changes`, commit, push, pull) in the Changes tab, and a pull request's files (`github::load_pull_files`) with Merge, Squash and Rebase |
| `src/clones.rs` | Finding and remembering each repository's local clone |
| `assets/fonts/` | Inter and IBM Plex Mono (SIL OFL), bundled so mono text is monospaced everywhere |
| `server/allowlist.txt` | Every GitHub request the app makes, and ones it must never make: the proxy (`server/src/allowlist_cases.rs`), the Android app (`BackendTest.sharedAllowlist`) and the app's own requests (`src/allowlist_tests.rs`) are all tested against it, so the three allowlists stay equal |
| `src/test_env.rs` | The tests' environment: taking `ENV_LOCK` clears the machine's account (`GITHUB_TOKEN`, a project `.env`, the saved one), so no test reaches GitHub or depends on who runs it |
| `server/` | `repohub-server`: static files (`files.rs`: the precompressed `.br`/`.gz` copy the browser accepts, `Cache-Control` and `ETag`), `config.json`, the `/gh/*` GitHub proxy that holds the token, and `/api/notes` (drawings in `DATA_DIR`; `all`, conditional writes with `if=`, the trash, sync's `index`) |
| `web/manifest.webmanifest`, `web/icon.svg` | Add to Home Screen on phones |
| `web/index.html` | Page that loads the WebAssembly build (`app.js` and `app_bg.wasm` are build outputs) |
| `Dockerfile`, `compose.yaml` | Container: builds the web app and the server, runs the server on port 8080 |
| `packaging/` | Desktop packaging: the icon, the macOS bundle script and `Info.plist`, the Linux launcher and installer |
| `CHANGELOG.md`, `scripts/release-notes.sh` | Release notes: one section per version, printed by the script for the Desktop workflow |
| `scripts/web-build.sh`, `scripts/install-wasm-opt.sh` | The web build that Docker, Android and Pages ship: cargo's `web` profile, wasm-bindgen, wasm-opt, the versioned page, `--precompress`; and the pinned wasm-opt it needs |
| `scripts/size.sh`, `scripts/size-baseline.json` | What we ship, measured (see [Size](#size)), and the numbers CI compares with |
| `.github/workflows/` | CI (tests and builds), image publishing (GHCR), GitHub Pages, desktop builds and releases (`release.yml`) |

## How data flows

- **Desktop:** `Config::load` reads `.env`. `github::load_all` runs on a worker thread (driven by
  `pollster`) and calls the API directly with the token. `AppHandle::update` applies the result.
- **Web with `repohub-server`:** the page fetches `config.json`, then calls `gh/<api path>`. The
  server checks the path against an allowlist, adds the token and forwards the call to GitHub.
- **Static web (GitHub Pages):** there is no `config.json`, so the app shows the sample data.

## Build and test

```sh
cargo test --release                                      # app tests (.env parser, dates, config.json)
cargo test --release --manifest-path server/Cargo.toml    # proxy allowlist and helpers
cargo run --release                                       # desktop app
```

The tests don't use the account of the machine running them: a test that touches the
environment or builds the app (`crate::init()`) takes `crate::ENV_LOCK`, which clears
`GITHUB_TOKEN`, `GITHUB_USER`, `GITHUB_API_URL` and the project `.env` first, and sets the data
it needs itself (`s.data = Data::sample()` then `s.data_changed()`).

Headless screenshots (Rune renders without a window, on the same renderer):

```sh
REPOHUB_BLOCKING=1 target/release/repohub --snap out.png --size 1180x760 -- page repos
# pages: overview (default), repos, repo, pulls, issues, actions, settings
```

`REPOHUB_BLOCKING=1` loads the data before the first frame, so the snapshot shows it (and skips
the saved copy). Without it, a live run shows the saved copy of the last load, if there is one,
and loads behind it; `REPOHUB_CACHE=off` turns the copy off.
Add `tab notes` (a section's name) after `page repo`, and `REPOHUB_TRACE=1` to print every state change.
`REPOHUB_COMPACT=1` with `--size 390x844` starts in the phone layout.
`thread 14` after `page pulls` (or `page issues`) opens that conversation; `picker labels` (or
`assignees`, `milestone`, `reviewers`) then opens that picker in its details, and `edit title` (or
`body`) the title's or the description's editor. After `page repo tab issues`, `picker labels` (or
`assignees`) opens the New issue form with that picker.
`diff 14` also opens that pull request's diff, and `section appearance` picks the Settings section.
`tab commits` reads the history before the first frame; `commit 0123abc` (or `commit 2`, the
second in the list) opens a commit, and `compare main...feature/x` compares two branches.
`tab releases` reads the releases before the first frame; `release new` opens the New release
form, `release notes` also picks the next minor version and generates the notes, `release
publish` adds the publish question, and `release edit` opens the first draft in the form.
With `tab changes` and a clone found (`REPOHUB_REPOS_DIR`), its changes are read before the first frame.

`scripts/screenshots.sh` regenerates the README's pictures in `docs/` from the sample data, with a
throwaway data folder and a made-up clone. `docs/notes.png`, `docs/terminal.png` and
`docs/claude.png` need a drawing, a shell and Claude Code, and are taken by hand.
After `page repo tab notes`, `board Name` opens that drawing and `zoom 3` zooms it (the phone's full-screen editor in the phone layout).
With `tab notes` the drawings of the data folder are listed before the first frame. `sync waiting`
shows board sync in that state from the sample data without running it (`synced`, `syncing`,
`waiting`, `failed`, `public`, and in Settings `setup` and `confirm`), with
`notes-repo owner/name` for the repository shown.
The Android app is described in [`android/README.md`](android/README.md).

### Web build

You need the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` at the version in
`Cargo.lock` (0.2.129) and `wasm-opt` (Binaryen):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
scripts/install-wasm-opt.sh ~/.local/bin        # Linux; on a Mac, brew install binaryen
scripts/web-build.sh dist/web                   # the page, app.js, app_bg.wasm (add --precompress for .br/.gz)
```

`scripts/web-build.sh` is the build the Docker image, the Android app and GitHub Pages ship
(the `web` profile, wasm-bindgen, wasm-opt, and the page naming the files with the build's hash).
Then serve it with the proxy:

```sh
WEB_DIR=dist/web cargo run --release --manifest-path server/Cargo.toml    # REPOHUB_ADDR=127.0.0.1:8080
```

While working on the app, the quicker build is the release profile without wasm-opt, into `web/`
(the server's default `WEB_DIR`):

```sh
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web --out-name app target/wasm32-unknown-unknown/release/repohub.wasm
cargo run --release --manifest-path server/Cargo.toml
```

The server reads its settings from the environment, using the same variables as `.env`.

### Size

`scripts/size.sh` measures what RepoHub ships that a Linux machine can build: the desktop
program, the web build at each step (the `.wasm` from cargo, after wasm-bindgen, after wasm-opt,
and with gzip and brotli, which is what a browser downloads) and how many copies of each font
the two carry. It builds nothing, so build first:

```sh
cargo build --profile dist --bin repohub    # the program, as the Desktop workflow builds it
scripts/web-build.sh dist/web               # the web build, as Docker, Android and Pages make it
scripts/size.sh                             # the table
scripts/size.sh --crates                    # and the biggest crates (cargo install cargo-bloat twiggy);
                                            # this rebuilds the program with symbols
```

The profiles (`Cargo.toml`):

- `release` (`cargo run`, `cargo test --release`): optimized for size, thin LTO. Quick to build.
- `dist` (the desktop packages): `release` with fat LTO and one codegen unit. Measured on Linux
  x86_64: the program 22% smaller (32.8 → 25.5 MB, the font copies Rune and RepoHub both
  embed are merged too), the 500-repository timing test 4–7% faster, frame times the same. A
  rebuild after a change takes longer, as fat LTO runs on one core (2.3 → 3.9 minutes on a busy
  4-core machine; a build from scratch about the same, 7–8 minutes).
  `opt-level = "z"` would make the program another 10% smaller, but typing in the palette 15%
  slower, so it isn't used.
- `web` (the web build): `release` with fat LTO and one codegen unit. wasm-opt runs light
  (`-O1`): `-Oz` makes the file 10% smaller but its gzip and brotli 2–4% bigger, and the browser
  downloads it compressed.

Panics unwind in every desktop profile (Rune's image decoder catches a codec's panic);
`src/main.rs` refuses to compile with `panic = "abort"`. Rune's `embedded-fonts` feature is
off: RepoHub registers the same fonts itself (`FONTS` in `src/lib.rs`), and with the feature the
web build carried them twice.

`repohub-server` sends the `.br` or `.gz` copy that `scripts/web-build.sh --precompress` made
(the Docker image has them) to a browser that accepts it, with a year's `Cache-Control` for the
versioned names (`app.js?v=…`) and `no-cache` with an `ETag` for the page (`server/src/files.rs`).
The Android APK compresses the files itself, and R8 shrinks its code.

CI's `size` job builds `dist` and `web` and runs
`scripts/size.sh --compare scripts/size-baseline.json`: the table goes to the job summary, with a
warning (never a failure) when the program, the `.wasm` or the brotli download grows by more than
3%. The baseline is a committed file rather than `main`'s last build, so CI builds once, and
growth that comes 1% at a time still adds up to a warning. It changes only on purpose: when a
pull request's growth is worth it, put its run's `size` artifact (`size.json`) in
`scripts/size-baseline.json` in the same pull request, and say why.

### Testing without a real token

Set `GITHUB_API_URL` to a local server that answers like the GitHub API. Everything else stays
the same; the clients bypass HTTP proxies for `localhost` and `NO_PROXY` hosts.

`scripts/mock_github.py PORT` is one: 200 repositories with pull requests, issues, runs and
activity, ETags and 304s, a delay on every answer (`MOCK_DELAY_MS`, default 150), slower
searches (`MOCK_SEARCH_DELAY_MS`, to watch the lists arrive one by one), `MOCK_DOWN=1` to see a
failed refresh, and `/_stats` and `/_log` with what was asked.

### Measuring how it loads

```sh
scripts/bench-load.sh [target/release/repohub] [delay-ms]
```

starts the mock, runs the app headless three times against it (a first start, the next start from
the saved copy, and a refresh with nothing changed) and prints, for each, the time to the first
content and to the end of the load, the requests (answered 200 and 304), the kilobytes GitHub
sent and the most requests at once. It reads the app's `REPOHUB_TRACE=1` lines, so an older build
works too (without its first-frame time). Times include the headless renderer's start-up.

### Notes on the whiteboard

- The board works in a fixed world of `board::WORLD` points (1200 × 720), which the canvas scales
  to the space it has. Gestures arrive in canvas points and are converted with the size in `Drag`.
- The file format is a subset of Excalidraw's. Unknown element types are skipped on load, but a
  save merges with the file (`notes::save`), so images or frames from excalidraw.com stay in it.
- Elements carry Excalidraw's `id` (random, as Excalidraw's), `version`, `versionNonce`,
  `updated` (milliseconds) and `isDeleted`. `Board::save_json` gives every element changed since
  the last write a new version, and keeps deleted ones as `isDeleted`, so another device's copy
  learns which edit is newer and what was deleted. Undo brings old content back under a newer
  version. Older files (every `version` 1, `updated` 0, ids that may repeat) still read.
- `board::tests` drives a full gesture sequence and checks the `.excalidraw` round trip, versions
  and deletions; `board_file::tests` checks that Export then Import gives the same board.

### Board sync

- The decision for each board is `sync::decide` (all 16 cells tested), the merge `sync::merge`.
  When in doubt both copies are kept: a local write lands only if the file still has the blob id
  the plan read (`notes::write_if`), a GitHub write only with the `sha` the plan read, and the
  index records a board only once both sides hold the same content.
- `sync::tests` runs whole syncs between fake devices and a fake GitHub (two devices converging,
  deletions, a write refused because GitHub changed meanwhile, the editor saving during a sync,
  boards that can't be merged, offline), and checks that no cell of the table loses a stroke.
- `sync_io::tests::two_devices_through_a_mock_github` runs the real GitHub client and the real
  notes storage against a mock of GitHub's Contents and Git Trees API on localhost, with two data
  folders as two devices: drawn on one, opened on the other, changed on both, deleted. To try
  the app or the server by hand, point `GITHUB_API_URL` at a server that answers those calls.
- A board deleted here (by you or by sync) moves to `trash/<date>/<owner>/<repo>/<branch>/` in
  the data folder and is removed after 30 days; to undo a wrong delete, move the file back into
  `notes/`. Sync's index is `notes-sync/<owner>%2F<name>.json` (per notes repository).

## Guidelines

- State that has more than two values gets a named type in `src/state.rs`, and one method that
  changes it. No tab numbers, no pairs of booleans.
- Never commit `.env` or anything that contains a token.
- Keep the proxy allowlist (`server/src/main.rs`, `allowed`) as narrow as the app needs. Add an
  endpoint there only together with the feature that uses it.
- RepoHub creates issues, branches, pull requests and releases (drafts, and publishing them),
  closes issues, merges pull requests, edits an issue's or pull request's labels, assignees,
  milestone, title and description and who is asked to review, and, in the notes repository the
  user chose for board sync, writes and deletes board files under `boards/` (one commit each),
  and nothing more: deleting repositories, branches, releases or tags, uploading release files
  and rewriting history stay on GitHub. The only other `DELETE` in a repository takes back a
  review request. A new write goes through `github::Write` (board sync through
  `sync_io::GhRemote` on the same transport), the proxy allowlist (`server/src/main.rs`) and its
  test. Every new request, read or write, is added to `server/allowlist.txt` (with its refused
  neighbours as `deny` lines): the app's test fails until it's there, and the proxy's and the
  Android app's tests until both allowlists take it. The board paths are checked on the raw address (`board_contents_path`), as narrowly as
  the app writes them.
- Text in the UI is plain and specific: say what happened and how to fix it.
- Every action gives feedback when it finishes or fails: a toast (`notify`, `notify_problem`) or
  a message where it happened. Nothing returns silently.
- An action that changes something on GitHub or deletes local data either asks first (merging,
  publishing a release, removing a plugin, deleting a drawing) or offers **Undo** in its toast (`notify_undo`, with a
  `state::Undo` that knows how to put it back: closing an issue, deleting a theme).
- The Android app forwards the same writes: add a new one to `android/.../GitHubPaths.java` too,
  with its test.
- The repository lists, the pinned set and the palette's entries are cached between frames and
  rebuilt when `App::data_gen` moves. Code that changes `data` in place (a count, a star, a
  state) calls `App::data_changed()`; `apply_done` does it for every finished write.
- What RepoHub keeps on disk besides your settings and drawings: the saved copy of the last load
  (`cache`), never the token or anything made from it (it's built from `Data`, not `Config`).
  Change `cache::FORMAT` whenever what it writes changes meaning.
- A new GitHub read goes through `Gh::get` / `get_resp` (so it's conditional) unless it's a
  search; a new part of `load_all_with` keeps the lists and warnings in the order the sequential
  load gave (`load_tests` checks it).
- Timing tests only guard against a return to the old cost on shared runners. To check the
  one-frame target on a quiet machine: `REPOHUB_PERF_ASSERT=1 cargo test --release measure_500`.
- Plugins see the app only through `@repohub` (`plugins::context` and the actions in
  `Plugins::make`). Give them more only on purpose: no files, processes or network.
- Every pressable element gets a `.role(...)` (`"button"`, `"link"`, `"option"`, …) and a
  `.label(...)`, and every `TextInput`, `TextArea` or `Switch` gets a `.label(...)`. Never convey
  status by colour, icon shape or hover tooltip alone: pair status marks with a distinct shape and
  an accessible label or visible text.
- A pull request that changes the app adds a line to [`CHANGELOG.md`](CHANGELOG.md) under
  **Unreleased** (Added, Changed or Fixed), written for the people who use RepoHub. CI warns
  when it's missing.

## Releasing a version

1. In `CHANGELOG.md`, rename `## Unreleased` to `## 0.2.0`, then start a new, empty
   `## Unreleased` above it. Set `version = "0.2.0"` in `Cargo.toml`.
2. Merge that, then tag it: `git tag v0.2.0 && git push origin v0.2.0`.
   The release workflow stops if the tag isn't the version in `Cargo.toml`.
3. The Desktop workflow builds the three packages and publishes the release. Its notes are the
   `0.2.0` section (`scripts/release-notes.sh 0.2.0` prints it), followed by GitHub's list of
   pull requests.

## Signing updates

The app checks an update's package against `SHA256SUMS`, which only shows the download isn't
damaged: both come from the same release. A signature on `SHA256SUMS` shows who made the
release. It is off until the repository has a key; from then on, builds refuse unsigned updates.

Set it up once, on your own computer. The private key never goes into the repository, an issue
or a chat.

1. Install [minisign](https://jedisct1.github.io/minisign/) and make a key without a password
   (CI can't type one): `minisign -G -W -p repohub.pub -s repohub.key`.
2. Keep `repohub.key` somewhere safe (a password manager). If it's lost, signed builds can no
   longer update themselves and people must download a new build by hand.
3. In the repository's **Settings → Secrets and variables → Actions**:
   - **Secrets** → `MINISIGN_SECRET_KEY`: the whole content of `repohub.key` (both lines).
   - **Variables** → `REPOHUB_UPDATE_PUBKEY`: the second line of `repohub.pub` (it starts with
     `RW`).
   Add the secret first. With the variable but no secret, the release job stops with an error
   instead of publishing a release the apps would refuse.
4. Push to `main`. The Desktop workflow builds the apps with the public key inside, signs
   `SHA256SUMS` and attaches `SHA256SUMS.minisig`.

Builds made before step 4 have no key and keep updating as before; the first build they update
to has it. To change the key, publish one release signed with the old key that carries the new
public key, then switch the secret.

