# Changelog

What changes in each version of RepoHub, newest first. Every pull request that changes the app
adds a line under **Unreleased**. When a version is tagged (`v0.2.0`), its section is renamed
from Unreleased to that version, and the Desktop workflow publishes the section as the release
notes. The Latest build shows the Unreleased section, and so does the app's **What's new**.

The sections are **Added**, **Changed** and **Fixed**.

## Unreleased

## 0.1.0 — 2026-11-18

The first numbered release.

### Added
- **Update channel** in Settings → About: **Stable** (numbered releases, the default for a
  numbered build) or **Preview** (the Latest build, as before). The version shows there and in
  the status-bar menu; a release tag must match `Cargo.toml`, or the release workflow stops.
- The Pull requests page filters **Drafts** and **Failing checks**, and the status-bar menu
  shows how many notifications are unread (it opens the Notifications page).
- A pull request's conversation shows its **checks** (the CI jobs on its last commit, with a
  summary and a link to each), and **Close pull request** / **Reopen pull request**.
- **Comments on a pull request's lines**: the comments people left on its diff show under their
  lines, and the **+** by a line number (or a dragged range of lines) opens a box to write one.
- **Undo** after closing an issue (it's reopened and comes back to the list) and after deleting
  a theme (it comes back where it was, as your light or dark theme if it was one).
- Keyboard: Esc closes what's open on top of the page (a merge confirmation, a form, a diff, a
  conversation, a file, a message), ⌘/Ctrl+[ goes back, and Ctrl+Shift+Tab leaves the
  terminal. ⌘/Ctrl+K and the page shortcuts work while the terminal has the keyboard.
  Settings lists them all.
- Notifications: a page with what GitHub wants you to look at (a review asked of you, a
  mention, an assignment), unread first and grouped by repository, with the count in the
  sidebar and a bell on the phone. Clicking one opens its conversation and marks it as read.
  It needs a classic token: GitHub doesn't give notifications to fine-grained ones.
- Filters on the Pull requests and Issues pages: search by title or number, pick a repository
  or a label, and show only the pull requests you opened or the issues assigned to you.
  Issues show all their labels, not only the first.
- Conversations in the app: click an issue or a pull request to read its description, comments
  and reviews, and reply with a comment. Approve a pull request or request changes, see its
  changes from the same page, and close or reopen an issue. The lists no longer send you to
  GitHub for this; **Open on GitHub** is on the page.
- An Android app (APK, Android 12 or later). It needs no server: you sign in from Settings, the
  token stays encrypted on the phone, and drawings are kept there too. It has the overview,
  repositories, pull requests, issues, workflow runs, README and code, and you can create
  issues, branches and pull requests and merge. The back button goes back inside the app.
- Drawings on a phone open full screen, made for touch: draw with a finger, zoom with the plus
  and minus buttons, and drag the drawing with the new **Move the view** tool. A drawing opens
  on what's drawn in it.
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
- Contrast checks in the theme editor: colours that are hard to read show their WCAG contrast
  ratio and a **Fix contrast** button that adjusts their lightness, main buttons pick readable
  ink in every theme, and issue label chips reach at least 4.5:1 contrast on light and dark
  panels.
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
- New README pictures (a pull request with its checks and diff, the Changes tab, the theme
  gallery, the phone layout), made by `scripts/screenshots.sh`; a feature overview and contents at
  the top of the README, and what the server forwards brought up to date.
- Settings is split into sections, one on screen at a time: **Account**, **Appearance**,
  **Assistants**, **Plugins** and **About** (version, updates, keyboard shortcuts). The
  connection's environment variables and files are under **Show details**. The phone's tabs say
  Repositories and Pull requests.
- The phone layout fits the newer features: a pull request's or an issue's buttons sit under
  its title instead of squeezing it, diffs are always unified and no taller than the screen
  allows, pinned repositories are at the top of the Repositories page, and the theme editor's
  colour rows wrap.
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
- Screen readers and keyboard navigation: clickable workflow runs, compact repository rows and
  repository card titles now announce their role and label; search, branch filter, new drawing and
  local folder fields and the draft pull request switch are labelled; CI, pull request, issue and
  git status marks carry accessible labels and distinct shapes instead of relying on colour or
  hover tooltips alone.
- On a phone: a repository's buttons (Open on GitHub, Fork, Pin, Star) wrap instead of running off
  the screen, the Changes tab's commit note wraps, the Terminal's buttons go under its folder, and
  the Terminal and the Claude chat are sized to the screen.
- Long pages, the command palette and the Claude chat show a scroll bar again, so you can see
  where you are (it follows the system's *Show scroll bars* setting).
- macOS: coloured parts of the window no longer flicker when you type or move the pointer. The
  app now draws with the Metal backend of wgpu.
- Every action now says how it went: **Copy screen** in the Terminal confirms the copy, and
  Commit, Push or Pull on a repository with no local copy says so instead of doing nothing.
- The command palette keeps the highlighted result in sight when you move past the first
  rows with the arrow keys, and screen readers hear which one is selected.
- Lists, the overview, the Code tab and the Changes tab now show loading placeholders while GitHub
  or `git` is still working, and show the error with **Retry** if a load fails, instead of saying
  nothing is there.
- Security (updates): the app downloads updates only from GitHub's release hosts, every
  redirect included. Builds made with a signing key refuse an update whose checksums aren't
  signed by it. macOS checks the new app's code signature before replacing the old one, and on
  Windows an install folder with an apostrophe in its name no longer breaks updates.
- Security (plugins): a plugin can no longer open a link or copy text by itself. It asks, and
  RepoHub shows the address or the text with **Open**/**Copy** and **Don't**. Only `https://`
  links, and copied text has no control characters. Installing from a Git address runs in the
  background, never waits for a password, and stops after a minute. A plugin with links to
  other folders, or more than 500 files or 10 MB, is refused.
- Security: text pasted into the Terminal can no longer end the paste early and run as typed
  commands (control characters are removed). The **Clone into …** command is refused when the
  repository's address or name has characters a shell could read as a command.
- A folder that only shares a repository's name is no longer taken for its clone: one of its
  git remotes must be that repository, or you choose it with **Use it anyway**. Commit, merge,
  Claude and the Terminal can't act on the wrong project by accident.
- The phone layout's tab bar (Overview, Repos, Pulls, Issues, Actions) is on screen again: a
  long page pushed it below the bottom edge.
- Lists page through GitHub results with the `Link` header rather than stopping at the first page:
  branches and starred repositories page beyond 100 with default branches preserved first, public orgs
  and repositories page via `Link` headers, workflow runs across all active repositories are sorted newest
  first before truncating, and pull requests are split into open and closed queries so open PRs are never
  pushed out by closed PRs. Pull requests and Issues pages include a **Load more** button to fetch subsequent
  pages up to 1,000 items, and UI badges, metrics, and headings display accurate counts and capped indicators.
- Data loading and caching handles partial failures and rate limits gracefully: failed repository
  details and branch lists are retried rather than permanently cached, errored details are dropped
  on refresh while clean details for unpushed repositories are preserved, and deleted branches fall
  back to the default branch. Secondary rate limits and HTTP 429 responses display accurate
  rate-limit explanations instead of misleading write-permission errors, respect the `retry-after`
  backoff before subsequent refreshes, and surface partial list failures in the warning banner and
  status-bar menu.
- `repohub-server` is hardened against unauthorized access, CORS bypasses, and directory traversal:
  a shared secret (`REPOHUB_SECRET`) is required when bound to non-loopback addresses and accepted via
  an `HttpOnly` cookie or bearer header; writes default to disabled on non-loopback bindings and require
  `REPOHUB_ALLOW_WRITES=true`, `X-RepoHub: 1` and `Content-Type: application/json`; full `Origin`
  schemes, hosts and ports are verified against `Host`; encoded directory traversal (`%2e%2e`, `\`,
  control characters) is blocked; the proxied GET allowlist is narrowed to exact app endpoints; notes
  storage enforces a configurable total size cap; and static file serving validates canonical paths.
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
