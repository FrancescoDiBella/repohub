# RepoHub

A dashboard for your own GitHub repositories, written in Rust with
[Rune](https://github.com/anomalyco/rune) (a GPU-rendered UI framework) and `rune-kit`.

It reads real data from the GitHub REST API: profile, repositories, files, README, languages,
last commit, pull requests, issues, workflow runs and weekly commits. You can star and unstar
repositories from the app, and keep **notes**: hand-drawn diagrams and sketches for each
repository and branch, saved on your computer.

| | |
| --- | --- |
| ![Overview, dark theme](docs/overview.png) | ![Repository page, light theme](docs/repository.png) |

There are three ways to run it:

| | What you need | Data |
| --- | --- | --- |
| **Docker** (recommended) | Docker and a browser with WebGPU | yours, from `.env` |
| **Desktop app** | Rust | yours, from `.env` |
| **Static page** (GitHub Pages) | a browser with WebGPU | sample data only |

## Run it with Docker

All you need is [Docker](https://docs.docker.com/get-docker/) (Docker Desktop on Windows and
macOS).

```sh
git clone https://github.com/FrancescoDiBella/repohub
cd repohub
cp .env.example .env        # on Windows: copy .env.example .env — then set GITHUB_USER and/or GITHUB_TOKEN
docker compose up --build   # the first build compiles everything: 10–20 minutes
```

Then open **http://localhost:8080** in Chrome or Edge, which support WebGPU. Some useful commands:
- `Ctrl+C` stops it.
- `docker compose up -d` runs it in the background.
- `docker compose down` removes it.
- After editing `.env`, `docker compose up -d --force-recreate` applies the change without rebuilding.

### How the token stays safe

The image contains two things:
- the web build of RepoHub, compiled to WebAssembly;
- `repohub-server`, a small server that serves the page and forwards API calls to GitHub,
  adding the token on the way.

What this means for the token:
- The token never leaves the container. The page never receives it: `config.json` only says
  whether a token is set.
- The port is published on this computer only (`127.0.0.1:8080`).
- The server forwards only the endpoints RepoHub uses: reads, and writes for starring only.
- The server refuses requests from other origins or with an unknown `Host` header, which blocks
  DNS rebinding.

To reach RepoHub from another device on your network, change `ports` in `compose.yaml` and add
the host name to `REPOHUB_ALLOWED_HOSTS`. Do this only on a network you trust: anyone who can
reach the port uses your token.

### On your phone

Below 720 points of width RepoHub switches to a phone layout: a top bar, the page in one column
and a tab bar at the bottom (Overview, Repos, Pulls, Issues, Actions) instead of the sidebar. The
same happens in a narrow desktop window. Notes work with touch; the terminal stays desktop-only.

![RepoHub on a phone](docs/mobile.png)

The phone opens the same web app the Docker container serves. Two things to know:

- **Browsers only offer WebGPU on HTTPS pages** (or `localhost`), so `http://192.168.x.x:8080`
  doesn't work from the phone. Put the container behind HTTPS, on a private network. The simplest
  way is [Tailscale Serve](https://tailscale.com/kb/1312/serve): with Tailscale on the computer and
  the phone, `tailscale serve 8080` gives a `https://<computer>.<tailnet>.ts.net` address only
  your devices can open. Add that host name to `REPOHUB_ALLOWED_HOSTS` in `.env`.
- **Browser support:** Chrome on Android 12 or later, and Safari on iOS 26 or later.

Then use the browser's **Add to Home Screen**: RepoHub opens full screen with its own icon.
The server holds your token, so never publish the port on the open internet.

### Prebuilt image

The `image` workflow publishes `ghcr.io/francescodibella/repohub` on every push to `main`, for
Intel/AMD (`linux/amd64`) and ARM (`linux/arm64`, Apple silicon included). Once it's published,
there is nothing to build:

```sh
docker run --rm --env-file .env -p 127.0.0.1:8080:8080 -v repohub-notes:/data ghcr.io/francescodibella/repohub
```

A new GHCR package starts out private. Either make it public (your GitHub profile → Packages →
repohub → Package settings) or run `docker login ghcr.io` first.

## Desktop app

A native app for macOS, Windows and Linux: its own window, no browser and no Docker. It is the
only version with the [terminal](#terminal-desktop-app).

### Install

Download the build for your system from the
[Releases](https://github.com/FrancescoDiBella/repohub/releases) page:

- **macOS** (Apple silicon and Intel): open `RepoHub-<version>-macos.dmg` and drag RepoHub to
  Applications. The app isn't notarized by Apple, so the first time right-click it and choose
  **Open**, or run `xattr -dr com.apple.quarantine /Applications/RepoHub.app`.
- **Windows**: unzip `RepoHub-<version>-windows.zip` and run `RepoHub.exe`. SmartScreen may ask
  for confirmation (*More info* → *Run anyway*).
- **Linux** (x86-64): unpack `RepoHub-<version>-linux-x86_64.tar.gz` and run `./install.sh`, which
  puts the program in `~/.local/bin` and adds a launcher entry.

There is no release yet? Run the **Desktop** workflow by hand (Actions → Desktop → Run workflow)
and download its artifacts, or push a tag such as `v0.1.0` to publish one.

### Connect your account

Open **Settings → GitHub account**, type your user name and (optionally) a token, and press
**Connect**. No `.env` file is needed.

- With only a user name RepoHub shows your public repositories.
- A token adds private repositories and starring. The **Create a token** button opens the right
  GitHub page.
- The account is saved in the app's settings file, which only you can read:
  `~/Library/Application Support/RepoHub/.env` (macOS), `%APPDATA%\RepoHub\.env` (Windows) or
  `~/.local/share/repohub/.env` (Linux). Any setting from [`.env.example`](.env.example) works
  in that file too.
- A `GITHUB_USER` or `GITHUB_TOKEN` from the environment or a project `.env` wins over the saved
  account; Settings says so when that happens.

### Build from source

You need [Rust](https://rustup.rs). The toolchain version is pinned in `rust-toolchain.toml` and
installed automatically.
- Linux: you also need `libxkbcommon-dev` and `libwayland-dev`.
- Windows: you also need the Visual Studio C++ Build Tools.

```sh
cargo run --release                   # the first build takes a few minutes
packaging/macos/bundle.sh 0.1.0       # macOS: builds dist/RepoHub.app and the .dmg
```

## Notes

Each repository has a **Notes** tab with a small whiteboard in the spirit of
[Excalidraw](https://excalidraw.com), built natively with Rune.

![Notes: a diagram drawn in RepoHub](docs/notes.png)

- **Branches:** pick a branch, then name a drawing to create it or open an existing one. Each
  branch keeps its own drawings.
- **Shapes:** rectangles, ellipses, diamonds, arrows, lines, freehand strokes and text, with the
  hand-drawn look Excalidraw is known for.
- **Style:** six colours, filled shapes and bold strokes.
- **Editing:** select and drag to move, Delete to remove, Ctrl/⌘+Z to undo and Ctrl/⌘+Shift+Z to
  redo.
- **Saving:** every change is saved at once, in Excalidraw's `.excalidraw` format, so a drawing
  also opens on excalidraw.com (*Open* → pick the file).

Drawings never go to GitHub. They are stored at
`notes/<owner>/<repo>/<branch>/<name>.excalidraw` inside:
- the desktop app: `REPOHUB_DATA_DIR`, or the platform's data folder (`~/.local/share/repohub`,
  `~/Library/Application Support/RepoHub`, `%APPDATA%\RepoHub`);
- Docker: the `notes` volume (`/data` in the container), which survives rebuilds and updates.

The static GitHub Pages build has nowhere to store them, so the tab explains that instead.

## Terminal (desktop app)

Each repository has a **Terminal** tab: your login shell in a real pseudo-terminal, opened in the
repository's **local clone**, so `claude`, `git`, `npm`, `cargo` and the rest run with the right
working directory and your usual environment. Full-screen programs (Claude Code, vim, htop) work,
with colours, scrollback (mouse wheel) and paste.

![Terminal tab running git in the repository's clone](docs/terminal.png)

- **Finding the clone:** RepoHub looks for a checkout named after the repository in
  `REPOHUB_REPOS_DIR` (comma-separated folders) or in `~/Developer`, `~/Projects`, `~/code`,
  `~/src`, `~/GitHub`, `~/repos` and `~/Documents/GitHub`. You can also type the folder, or press
  **Clone into …** to clone it there. A folder you pick is remembered.
- **Command buttons:** one click types `claude`, `git status`, `git pull` or `git log`. Set your
  own list with `REPOHUB_TERMINAL_COMMANDS` (`;`-separated). **Open with claude** starts Claude Code
  right away.
- **Keys:** click the terminal to type. Ctrl+C, arrows, Tab, Ctrl+R… go to the shell;
  Ctrl/⌘+V pastes.

The terminal is in the desktop app only. A web page can't start programs on your computer. In
Docker a shell would run inside the container, without your tools, credentials or files, and
anyone who could reach the port would get it.

## Configuration

Settings live in `.env`. It is ignored by git (`.gitignore`) and by Docker builds
(`.dockerignore`), so it is never committed or baked into an image.

```sh
# Public data only (60 API requests an hour, no private repositories)
GITHUB_USER=your-username

# Everything, private repositories included (5,000 requests an hour)
GITHUB_TOKEN=github_pat_...
```

[`.env.example`](.env.example) documents every option and the token permissions to grant. The
options cover the title, theme, pinned repositories, forks and archived repositories, the
maximum number of repositories, how many repositories to read activity from, demo mode and
GitHub Enterprise.

- Variables already set in the environment win over the file. Watch out if your shell already
  exports a `GITHUB_TOKEN` (desktop app).
- Desktop app: `REPOHUB_ENV=/path/to/other.env` loads another file. Otherwise `.env` is looked up
  in the current folder and its parents.
- Docker: `REPOHUB_ALLOWED_HOSTS` adds host names besides `localhost`.

## What it shows

| Page | Data and actions | API |
| --- | --- | --- |
| Overview | totals, commits over the last 26 weeks, recent repositories, open pull requests | `/user` or `/users/{u}`, `/user/repos` or `/users/{u}/repos`, `/search/issues`, `/repos/{r}/stats/participation` |
| Repositories | search by name, description and topic; public, private and fork filters; star | `/user/starred`, `PUT`/`DELETE /user/starred/{r}` |
| Repository | root files, README, languages, last commit, copy clone URL, open on GitHub | `/repos/{r}/contents`, `/readme`, `/languages`, `/commits` |
| Pull requests | pull requests you're involved in: open, draft, merged, closed; click to open | `/search/issues?q=is:pr involves:{u}` |
| Issues | open issues in your repositories; click to open | `/search/issues?q=is:issue is:open user:{u}` |
| Actions | latest runs of the most recently pushed repositories | `/repos/{r}/actions/runs` |
| Repository → Notes | drawings per branch: create, open, draw, delete | `/repos/{r}/branches` (the drawings stay on your computer) |
| Settings | connection status, the configuration in use (the token is never shown) and where notes are kept | — |

How it behaves:
- **Refresh** (toolbar icon, or ⌘R / Ctrl+R) reloads everything.
- Requests run in the background, so the window stays responsive.
- Errors (rejected token, rate limit, unknown user) show a banner with the reason and **Retry**.
- RepoHub never creates, changes or deletes repositories. "New repository" and "Settings on
  GitHub" open GitHub in the browser.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for the project layout, how to test and how the pieces fit
together.

## Status

Rune is pre-release, so RepoHub pins a revision of its `dev` branch in `Cargo.toml`.
