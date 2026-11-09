# Contributing to RepoHub

## Layout

| Path | What |
| --- | --- |
| `src/lib.rs` | App state, background loading, sidebar, toolbar, banners |
| `src/pages.rs`, `src/lists.rs` | Pages: overview, repositories, repository detail, pull requests, issues, Actions, settings |
| `src/ui.rs`, `src/icons.rs` | Small building blocks and the icon set |
| `src/model.rs` | Data model, sample data, date and colour helpers |
| `src/github.rs` | GitHub REST client: async, shared by the desktop and web builds |
| `src/net.rs` | Transport: blocking `ureq` on the desktop, `fetch` to the `/gh` proxy on the web; `spawn` runs work in the background and reports to the UI |
| `src/config.rs` | `.env` parsing (desktop) and `config.json` (web) |
| `server/` | `repohub-server`: static files, `config.json`, and the `/gh/*` GitHub proxy that holds the token |
| `web/index.html` | Page that loads the WebAssembly build (`app.js` and `app_bg.wasm` are build outputs) |
| `Dockerfile`, `compose.yaml` | Container: builds the web app and the server, runs the server on port 8080 |
| `.github/workflows/` | CI (tests and builds), image publishing (GHCR), GitHub Pages |

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

## Guidelines

- Never commit `.env` or anything that contains a token.
- Keep the proxy allowlist (`server/src/main.rs`, `allowed`) as narrow as the app needs. Add an
  endpoint there only together with the feature that uses it.
- RepoHub doesn't change repositories. Destructive actions belong on GitHub.
- Text in the UI is plain and specific: say what happened and how to fix it.
