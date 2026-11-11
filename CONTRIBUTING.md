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
| `src/github.rs` | GitHub REST client: async, shared by the desktop and web builds |
| `src/net.rs` | Transport: blocking `ureq` on the desktop, `fetch` to the `/gh` proxy on the web; `spawn` runs work in the background and reports to the UI |
| `src/config.rs` | `.env` parsing (desktop: the project file and the app's own settings file, where Settings saves the account) and `config.json` (web) |
| `src/board.rs` | The Notes whiteboard: model, hand-drawn rendering, gestures, undo/redo, `.excalidraw` read/write |
| `src/actions.rs` | The branch picker, the New issue / branch / pull request forms, Close and Merge on rows |
| `src/notes.rs`, `src/notes_page.rs` | Notes storage (local folder on desktop, `/api/notes` on the web) and the Notes tab |
| `src/palette.rs` | The command palette (⌘K): the commands for the current state, the fuzzy ranking (both tested), the modal layer |
| `src/update.rs` | In-app updates: the build's commit (`REPOHUB_BUILD`, set by the Desktop workflow), the Latest build's package for this platform, the SHA256SUMS check, install and restart |
| `src/tray.rs` | The status-bar widget: the menu (a pure function of the state, tested), Rune's `Tray` on Windows and Linux, `tray-icon` on macOS; icons in `assets/tray-*.rgba` |
| `src/claude.rs`, `src/claude_page.rs` | The Claude tab (desktop): runs `claude -p --output-format stream-json` in the clone, parses the stream into chat items (`Run`, `Mode`, `Item`), the chat UI |
| `src/terminal.rs`, `src/terminal_page.rs` | Embedded terminal (desktop): `portable-pty` runs the login shell, `vt100` keeps the screen; key mapping, colours, the Terminal tab |
| `src/clones.rs` | Finding and remembering each repository's local clone |
| `assets/fonts/` | Inter and IBM Plex Mono (SIL OFL), bundled so mono text is monospaced everywhere |
| `server/` | `repohub-server`: static files, `config.json`, the `/gh/*` GitHub proxy that holds the token, and `/api/notes` (drawings in `DATA_DIR`) |
| `web/manifest.webmanifest`, `web/icon.svg` | Add to Home Screen on phones |
| `web/index.html` | Page that loads the WebAssembly build (`app.js` and `app_bg.wasm` are build outputs) |
| `Dockerfile`, `compose.yaml` | Container: builds the web app and the server, runs the server on port 8080 |
| `packaging/` | Desktop packaging: the icon, the macOS bundle script and `Info.plist`, the Linux launcher and installer |
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

Headless screenshots (Rune renders without a window, on the same renderer):

```sh
REPOHUB_BLOCKING=1 target/release/repohub --snap out.png --size 1180x760 -- page repos
# pages: overview (default), repos, repo, pulls, issues, actions, settings
```

`REPOHUB_BLOCKING=1` loads the data before the first frame, so the snapshot shows it.
Add `tab notes` (a section's name) after `page repo`, and `REPOHUB_TRACE=1` to print every state change.
`REPOHUB_COMPACT=1` with `--size 390x844` starts in the phone layout.

### Web build

You need the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` at the version in
`Cargo.lock` (0.2.129):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web --out-name app target/wasm32-unknown-unknown/release/repohub.wasm
```

Then serve it with the proxy:

```sh
cargo run --release --manifest-path server/Cargo.toml      # WEB_DIR=web, REPOHUB_ADDR=127.0.0.1:8080
```

The server reads its settings from the environment, using the same variables as `.env`.

### Testing without a real token

Set `GITHUB_API_URL` to a local server that answers like the GitHub API. Everything else stays
the same; the clients bypass HTTP proxies for `localhost` and `NO_PROXY` hosts.

### Notes on the whiteboard

- The board works in a fixed world of `board::WORLD` points (1200 × 720), which the canvas scales
  to the space it has. Gestures arrive in canvas points and are converted with the size in `Drag`.
- The file format is a subset of Excalidraw's. Unknown element types are skipped on load, so a
  file saved from excalidraw.com with images or frames loses those elements if it's saved again
  from RepoHub.
- `board::tests` drives a full gesture sequence and checks the `.excalidraw` round trip.

## Guidelines

- State that has more than two values gets a named type in `src/state.rs`, and one method that
  changes it. No tab numbers, no pairs of booleans.
- Never commit `.env` or anything that contains a token.
- Keep the proxy allowlist (`server/src/main.rs`, `allowed`) as narrow as the app needs. Add an
  endpoint there only together with the feature that uses it.
- RepoHub creates (issues, branches, pull requests), closes issues and merges pull requests, and
  nothing more: deleting repositories or branches and rewriting history stay on GitHub. A new
  write goes through `github::Write`, the proxy allowlist (`server/src/main.rs`) and its test.
- Text in the UI is plain and specific: say what happened and how to fix it.
