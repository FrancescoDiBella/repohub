# Changelog

What changes in each version of RepoHub, newest first. Every pull request that changes the app
adds a line under **Unreleased**. When a version is tagged (`v0.2.0`), its section is renamed
from Unreleased to that version, and the Desktop workflow publishes the section as the release
notes. The Latest build shows the Unreleased section, and so does the app's **What's new**.

The sections are **Added**, **Changed** and **Fixed**.

## Unreleased

### Added
- The Code tab reads files from the repository's local clone when there is one. It's instant and
  works offline, shows uncommitted changes and marks changed files. You can browse folders and
  open files with syntax colours. GitHub is one click away.
- Plugins (desktop app). Anyone can write one: a sandboxed Luau script that adds a section to
  repository pages or a page of its own. Install from a folder or a Git address, and turn each
  one on or off. **Create a plugin** starts from the Checklist example, and the guide is
  `docs/PLUGINS.md`.
- Coding assistants. The Claude section can be turned off in Settings. You can add any harness,
  such as Codex, Aider, opencode, Gemini CLI or your own, with a name and the command that starts
  it. Each gets a button in the Terminal and a ⌘K command.
- Themes: nine ready-made themes, plus your own. You choose a colour for each part of the
  window, a background (a pattern or a picture), a glow on buttons, corner roundness and accent
  buttons. Pick one for the light appearance and one for the dark one.
- Release notes. This changelog is now published as the notes of each release and of the Latest
  build. When a new version is ready, the app can show **What's new**.
- Pinned repositories: pin or unpin any repository from its card, its page or ⌘K. Your pins are
  remembered.
- RepoHub reopens where you left off: the page, the repository and its section, the sidebar,
  and the sort and owner filter on the Repositories page.
- In-app updates. On the desktop, RepoHub checks the Latest build, downloads the package for
  your system, checks it against `SHA256SUMS`, installs it and restarts.
- Repositories of the organizations you belong to, with an owner filter and an Organizations
  section in the sidebar.
- Command palette (⌘K / Ctrl+K): go anywhere, open repositories and run actions by typing.
- Auto-refresh every few minutes, and an "updated X ago" label.
- Status-bar widget (macOS menu bar, Windows notification area, Linux tray) with quick actions.
- Claude tab: chat with Claude Code inside the repository's local clone.
- Work on GitHub from the app: pick a branch; create branches, issues and pull requests; close
  issues and merge pull requests.
- Selectable, copyable README text.
- Desktop builds for macOS, Windows and Linux on every push to `main`, published as the Latest
  build.
- Native desktop app: connect your account from Settings, plus packaging for the three systems.
- Phone layout with a top bar and a bottom tab bar.
- Embedded terminal in each repository's local clone.
- Rendered README with images, tables and code.
- Notes: hand-drawn diagrams for each repository and branch.
- Docker image for amd64 and arm64.

### Changed
- A lighter desktop app: 31 MB instead of 50 MB (no debug symbols, optimized for size and at
  link time), with the same speed.
- A refresh keeps the details already loaded for repositories nobody pushed to since, so opening
  them again needs no new requests to GitHub.
- Plugins get a new context only when what it shows changes, not on every frame.
- The Terminal's default command buttons no longer include `claude`: it's the Claude Code harness
  now.
- The app's states (loading, saving, theme, tabs, filters…) are explicit state machines,
  drawn in `docs/STATE.md`.
- A new look, with a more readable layout.

### Fixed
- Linux builds no longer depend on Ubuntu's package mirrors, so they keep working when the
  mirrors are down.
- Switching the theme works again.
- The Docker image builds again.
