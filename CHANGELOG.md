# Changelog

What changes in each version of RepoHub, newest first. Every pull request that changes the app
adds a line under **Unreleased**. When a version is tagged (`v0.2.0`), its section is renamed
from Unreleased to that version, and the Desktop workflow publishes the section as the release
notes. The Latest build shows the Unreleased section, and so does the app's **What's new**.

The sections are **Added**, **Changed** and **Fixed**.

## Unreleased

### Added
- **Settings → Plugins** has an **Open** button for each plugin's page, also where a phone (which
  has no sidebar) finds it.
- The Pull requests and Issues pages have **Created by me** and **Assigned to me** on both, and
  chips for who opened them (**Author**), next to the repository and label chips.
- More of RepoHub works from the keyboard: `/` searches the page (Repositories, Pull requests,
  Issues, a repository's Commits; elsewhere it opens the palette), `j` / `k` or ↓ / ↑ move through
  the list on screen and Enter opens the marked row, `i` and `p` start a New issue or a New pull
  request on a repository's page, and `?` lists every shortcut in the new **Settings →
  Keyboard**. None of them fires while you type in a field, in the terminal or on a drawing.
- RepoHub asks before aborting a merge, pulling over uncommitted changes, pushing (or committing
  and pushing) straight to the repository's default branch, signing out and clearing the saved
  data.
- **Undo** in the toast after unpinning a repository, removing a coding assistant, reopening an
  issue or a pull request, and starring or unstarring (which now says it was done).
- Esc, **Cancel**, opening another form or leaving a conversation asks "Discard what you typed?"
  (**Discard** or **Keep editing**) when a New issue, branch or pull request form, the release
  form, an edited title or description, or a comment you're writing holds text not sent yet. A
  form open for another repository is no longer replaced without asking. An empty one still
  closes at once.
- A pull request's checks and the comments on its lines say when GitHub has more than the first
  100 read, with a link to the rest.

### Changed
- On a phone Settings is a list of its sections, each saying what it holds; one opens with a
  way back (**Settings**, Esc or the back button). The theme editor opens in a view of its own,
  with **Themes** (or Esc) back to the gallery.
- One word for each thing: "mode" is Light, Dark or System and "theme" the colours each mode
  uses (the toolbar's button says "Switch to light mode"), a repository's parts are sections,
  and links to GitHub say what they open ("All comments on GitHub", "Details on GitHub", "All on
  GitHub").
- Every empty list says what's empty and what to do, e.g. "No open issues in rune" with "New
  issue starts one" (no more "Nothing to show here").
- Public and Private look the same on the repository cards and on a repository's page, Private
  with a lock.
- Each Settings section fits on one screen: the account in two columns, five themes to a row,
  the keyboard shortcuts in a section of their own, and the kind of token to make behind
  **Which token?** (which also says that only a classic token reads your notifications).
- The palette's Pin and Unpin show the pin the way the pin button does.
- Every question asked before something that can't be undone looks and works the same: merging
  a pull request (from its row or its diff, where you now pick a merge commit, squash or rebase
  in the question), publishing a release, deleting a drawing, removing a plugin. On a computer
  it takes the button's place; on a phone it's a sheet at the bottom of the screen, like
  "Discard what you typed?". **Cancel**, Esc or the back button answer no.
- With only a token and no user name, the lists of last time show once GitHub has said whose
  the token is (one request), so a token changed outside the app never shows the previous
  account's lists.
- The Android back button closes things in the order Esc does: pickers, editors, the branch
  picker, questions and forms first.

### Fixed
- On a phone the theme editor puts each label above its field and its title above its buttons,
  so nothing overlaps, and the Terminal's hint wraps instead of running past the window.
- The repository's section bar: when the sections don't fit, its edges fade and an arrow at each
  end scrolls to the hidden ones, and the section on screen is always scrolled into view (a
  plugin's section opened from the palette too).
- The example plugin's Checklists page opens the repository's checklist, as it says; plugins
  can open their own section with `repohub.openRepo(fullName, true)`.
- On a phone, the Terminal section's **Clone** button no longer runs past the screen when the
  folder's path is long.
- Closing an issue, merging, a new issue or a star made while a refresh (or the first load) runs
  is no longer undone when it lands, and isn't counted twice when GitHub already shows it.
- Branch names with `%` work in the web app and on Android (creating a branch from one,
  comparing).
- **Load more** pressed just before a refresh no longer adds an old page to the new list.
- Two forms sent one after the other each get their own title in the list.
- The comments on a diff's lines no longer go back to an older answer that came late.
- Starring on the sample data says that nothing was sent to GitHub.
- Lists read across several pages follow GitHub's `Link` header exactly, also when an address
  holds a comma.

## 0.2.0 — 2026-11-22

Four new features (Commits, Releases, triage in conversations, boards on every device), a faster
start and smaller downloads.

### Added
- RepoHub opens at once: the lists of the last load show in the first frame, with "Updating…"
  until GitHub answers. If it can't, the lists stay and the banner says how old they are. The
  copy never holds your token, is removed when you sign out or switch accounts, and **Settings →
  About → Clear saved data** removes it; `REPOHUB_CACHE=off` turns it off.
- **Releases** on a repository's page: its releases with their notes, files and download counts,
  20 at a time with **Load more**, marked **Latest**, **Pre-release** or **Draft**, and the tags
  that have no release. **New release** offers the next versions as the tag, picks the branch a
  new tag is made from, and fills the notes with **Generate notes**; save it as a draft, or
  publish it after a question. Drafts have **Edit** and **Publish**. The repository's header
  shows the latest release's tag, and the palette has "Releases of …" and "New release in …".
  RepoHub doesn't delete releases or tags or upload files.
- Triage in a conversation: a details block (beside it on a wide window, above it on a phone)
  with **Labels**, **Assignees**, **Milestone** and, on pull requests, **Reviewers** (who is
  asked, and who reviewed with their verdict). **Edit** opens a picker with a filter and the
  repository's choices; **Apply** sends the change. Edit the **title** and the **description**
  in place. The conversation and the lists follow at once; when GitHub refuses, it says so and
  keeps the old values.
- **New issue** takes optional labels and assignees.
- **Review requested**: a chip on the Pull requests page, a **Waiting for your review** panel on
  the Overview and a line in the status-bar menu ("2 pull requests wait for your review"). Rows
  show up to three assignees.
- The Pull requests and Issues filters are kept between visits and restarts; **Clear** resets
  them.
- **Boards on every device:** choose a private repository of yours in Settings → Account →
  Boards (for example `repohub-notes`), and boards sync through it: draw on the phone, open the
  computer, and the board is there. Changes made on two devices are merged, so both sets of
  strokes stay; if two copies can't be merged, both are kept and a toast says so. Deleting a
  board deletes it on the other devices, unless one changed it since. The Notes tab says when it
  last synced and what waits to be sent, and marks each board. RepoHub warns before using a
  public repository and never creates one; **Stop syncing** leaves everything in place.
- **Export…** and **Import…** of a board as an `.excalidraw` file (a save dialog on the desktop,
  a download in the browser, the phone's pickers on Android). They need no account.
- **Commits** on a repository's page: the history of the branch you picked, grouped by day,
  30 at a time with **Load more**, a filter by message, author or commit id, and Copy id and
  Open on GitHub on each commit. Click one to read its whole message, who wrote and committed
  it, its parents and what it changed as a diff. **Compare** shows how two branches differ (the
  commits and the combined diff) and opens the New pull request form with both filled in.
  The palette has "Commits of …", and on the desktop the list says when your clone has commits
  to push or to pull.
- Actions: filter workflow runs by status (**All**, **Failed**, **Running**), repository and
  workflow; click a run to inspect its jobs, steps and the last 200 lines of any job's log (with
  **Copy log**), and re-run failed jobs or the whole workflow (**Actions: Read and write** /
  `workflow` scope). The status-bar icon also flags your open pull requests with failing checks.
- Contrast checks in the theme editor: colours that are hard to read show their WCAG contrast
  ratio and a **Fix contrast** button that adjusts their lightness, main buttons pick readable
  ink in every theme, and issue label chips reach at least 4.5:1 contrast on light and dark
  panels.

### Changed
- **Smaller downloads.** The desktop app is about a fifth smaller (on Linux the program went
  from 32.8 to 25.5 MB) and no slower. In Docker, the server now sends the web app compressed:
  the first visit downloads 2.4 MB (brotli) or 3.5 MB (gzip) instead of 16.8 MB, and the browser
  keeps the files until the next version instead of downloading them again on every visit. The
  Android app's APK is several megabytes smaller: its page is now compressed inside it.
- Loading is about 2.5 times faster: the parts of a load run side by side (at most four requests
  at a time), and on a first start your repositories show before pull requests, issues and runs
  are in.
- A refresh with nothing new costs almost none of GitHub's rate limit: RepoHub asks "changed
  since?" and reuses what it has when GitHub answers "not modified". Through `repohub-server`
  and the Android app too.
- Repository pages, branch lists, labels and conversations already read show at once when opened
  again, and are read again behind them, instead of a skeleton.
- A refresh over lists already on screen no longer shows the "Loading data from GitHub…" banner:
  "Updating…" shows where "Updated … ago" was.
- A deleted drawing goes to a trash folder next to the drawings and stays there 30 days.
- Drawing on a board saved from excalidraw.com keeps the elements RepoHub doesn't draw (images,
  frames) in the file.
- The repository page's section bar scrolls sideways when its sections don't fit, instead of
  squeezing their names.
- Faster **Repositories** page, **⌘K** command palette and **Terminal** on large accounts:
  searching and filtering reuse pre-computed repository keys and cached lists, the repository
  list shows 60 cards at a time with **Show more** and memoizes unchanged cards, local clone
  paths are cached instead of probing the disk on every frame, and terminal lines merge
  same-background runs and skip rebuilding between shell outputs.

### Fixed
- Board sync: pressing **Stop syncing** while a sync was running didn't forget what had been
  synced, so choosing the same repository again later could delete from it, and from your other
  devices, boards you had deleted on this one while sync was off. And a sync no longer starts
  while another is still running (after switching accounts, or Stop then **Use it**).
- **Esc** closes an open labels, assignees, milestone or reviewers picker, the title or
  description being edited, the branch picker and the drawing's delete question, before the
  conversation or page under them; and it no longer throws away a New issue or pull request
  form left open on a repository's page, with what you typed, while you're on another page.
- A New issue, branch or pull request that GitHub refused after you pressed **Cancel** (or left)
  now says so in a toast instead of failing silently, and a form you opened meanwhile is no
  longer closed or marked with the other one's result.
- **Close pull request** offers **Undo** in its toast, as closing an issue does.
- A repository's "open issues and pull requests" count didn't change when a pull request was
  opened, merged, closed or reopened in RepoHub (only issues counted); it does now.
- With only a token set, changing it outside the app to another person's showed the previous
  account's saved lists (private repositories included) until the whole first load finished,
  or for good if it failed; they're replaced as soon as the new account's profile arrives.
- Switching the update channel in Settings while an update was downloading installed the other
  channel's build anyway; the download now stops.
- With 50 unread notifications (the most RepoHub reads), the Notifications page and the phone's
  bell said "50 unread" as if that were all; they say these are the newest 50, or "50 or more".
- A repository's Issues, Pull requests and Actions sections said "Nothing to show here" while
  their lists were still loading; they say Loading….
- The repository page had an extra gap under its section bar, and the Code tab under its
  buttons. The pull request form's Draft switch showed its words twice.
- The Notes section says drawings are also synced through your notes repository when board sync
  is on (and that they're on the RepoHub server on the web), not only "saved on this computer".
- On a phone, the theme editor's Background choices ran off the screen; the row scrolls now.
- A job log larger than 10 MB says so in plain words instead of "response too big for
  into_string".
- The diff's and the file viewer's **GitHub** buttons say **Open on GitHub**, as every other
  such button does, and **Push** has an icon of its own instead of the pull request one.
- Screen readers and keyboard navigation: clickable workflow runs, compact repository rows and
  repository card titles now announce their role and label; search, branch filter, new drawing and
  local folder fields and the draft pull request switch are labelled; CI, pull request, issue and
  git status marks carry accessible labels and distinct shapes instead of relying on colour or
  hover tooltips alone.

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
