# Changelog

What changes in each version of RepoHub, newest first. Every pull request that changes the app
adds a line under **Unreleased**. When a version is tagged (`v0.2.0`), its section is renamed
from Unreleased to that version, and the Desktop workflow publishes the section as the release
notes. The Latest build shows the Unreleased section, and so does the app's **What's new**.

The sections are **Added**, **Changed** and **Fixed**.

## Unreleased

### Added
- Merge local branches from the Changes tab. You preview the commits and the diff first. On
  conflicts you see the files and can abort; Commit finishes the merge once they're fixed.
- Diffs in the app. **Files** on a pull request shows what it changes, unified or side by side,
  and it can be merged from there with a merge commit, squash or rebase. A new **Changes** tab
  shows what changed in the local clone, with **Commit**, **Commit and push**, **Push**, **Pull**
  and **New pull request**.
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
- `GITHUB_TOKEN` and `GITHUB_USER` are parsed directly into app configuration and never written
  to the process environment. Every child process spawned by RepoHub (terminal shells, Claude Code,
  local git operations, plugin git clones, and updater helper processes) explicitly removes
  `GITHUB_TOKEN` and `GITHUB_USER` by default, preventing credentials from leaking to untrusted
  scripts or tools. An explicit opt-in (`REPOHUB_TERMINAL_PASS_TOKEN=true`) is available for users
  who want the token passed to the terminal, and signing out clears credentials immediately for all
  future children.
- Drawings no longer lose edits when you close one and reopen it, or create one with the same
  name, while it's still saving. Saves still finish after an account switch, and deleting a
  drawing while it saves deletes it once the save ends.
- Only one merge confirmation shows for a pull request whose diff is open, and it goes away
  with its diff.
- A `.env` found in the folder the desktop app is started in can no longer change the sensitive
  settings (`GITHUB_API_URL`, `REPOHUB_UPDATE_API`, `REPOHUB_CLAUDE`, `REPOHUB_DATA_DIR`,
  `REPOHUB_REPOS_DIR`, `REPOHUB_TERMINAL_COMMANDS`, `REPOHUB_TERMINAL_PASS_TOKEN`); Settings lists
  the ones it ignored. A token saved in Settings is only sent to the API host set in the
  environment or in the app's settings file, else to GitHub. Settings shows where the user, the
  token and the API host come from.
- Pull request file diffs no longer clobber or reset uncommitted local diffs on the Changes tab:
  viewing a pull request diff uses dedicated diff state, leaving local changes and merge previews
  intact, and merging from the diff panel requires explicit confirmation before submitting.
- Drawings (Notes) no longer overwrite each other or lose edits: late load callbacks are ignored
  if the drawing on screen does not match, closing a drawing chains pending saves without racing
  concurrent writes, writes use unique temporary files on disk, and canvas gestures are ignored
  while loading with mode reset on load and guarded point accesses.
- Refreshing data or removing a repository no longer switches the repository page to a different
  repository: repository and plugin pages and plugin sections now track their target by identity
  instead of by numeric index, safely falling back if the item is removed.
- Signing out or switching accounts in Settings no longer lets late responses overwrite the sample
  data or the new account, and resets the page, saved place, open forms and previous account
  caches.
- In-app updates can be cancelled while downloading, stalled downloads time out, and ready updates
  stay available if a background check fails (with checks paused until you install or dismiss).
- Relative times ("… ago") across repositories, pull requests, issues, runs, commits, and notes
  stay up to date with the clock instead of freezing at the last refresh.
- Pull request and issue counts match across Overview, the sidebar, and the tray menu, showing `+`
  (such as `40+` or `12+`) when GitHub's 40-item search limit is reached; Repository Settings now
  notes that archiving or deleting happens on GitHub.
- Linux builds no longer depend on Ubuntu's package mirrors, so they keep working when the
  mirrors are down.
- Switching the theme works again.
- The Docker image builds again.
