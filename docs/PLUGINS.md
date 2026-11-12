# Writing a RepoHub plugin

A plugin adds a section to every repository's page, a page of its own in the sidebar, or both.
It is a small [Luau](https://luau.org) script that builds its UI with Rune's `@rune/ui`. It runs
sandboxed inside the desktop app.

## Start in one minute

1. **Settings → Plugins → Create a plugin**. RepoHub writes the example plugin, *Checklist*, into
   your plugins folder and starts it. A **Checklist** section appears on repository pages, and a
   **Checklists** page in the sidebar.
2. Press **Folder** next to it and open `init.luau` in your editor.
3. Change something and save. The plugin reloads in place and keeps its state.

The plugins folder is `plugins/` inside RepoHub's data folder (Settings shows the path):
`~/Library/Application Support/RepoHub/plugins` on macOS, `%APPDATA%\RepoHub\plugins` on
Windows, `~/.local/share/repohub/plugins` on Linux.

## Files

```text
my-plugin/          the folder's name is the plugin's id (letters, digits, - and _)
  plugin.json       what it is and where it shows
  init.luau         its code; more modules beside it with require("@self/x")
  README.md         optional
```

`plugin.json`:

```json
{
  "name": "My plugin",
  "version": "0.1.0",
  "description": "One line about what it does.",
  "author": "You",
  "repo_tab": "Mine",
  "page": "Mine"
}
```

- `repo_tab` is the label of its section on repository pages, drawn by `repo(view)`.
- `page` is the label of its page in the sidebar, drawn by `page(view)`.

Give at least one of the two.

`init.luau` returns a table with those functions:

```luau
--!strict
local ui = require("@rune/ui")
local repohub = require("@repohub")

local ctx = repohub.context()
repohub.changed:Connect(function(new)
	ctx = new
end)

local presses = 0

return {
	repo = function(view)
		local r = ctx.repo
		return ui.col({ w = "full", gap = 8, pad = 16, radius = 10, fill = "panel" }, {
			ui.text(`{r.name} has {r.stars} stars`, { size = 15, weight = "semibold", color = "text" }),
			ui.row({ key = "press", px = 10, py = 4, radius = 6, fill = "control-off", hover = "hover",
				onPress = function() presses += 1 end }, { ui.text(`Pressed {presses} times`) }),
		})
	end,
}
```

A render function only builds UI. When a handler (such as `onPress`) or a `changed` connection
changes your variables, RepoHub draws the plugin again.

## `@repohub`

| | |
| --- | --- |
| `repohub.context()` | What's on screen, as a table (below). |
| `repohub.changed` | A signal that fires with the new context when it changes: a repository opens, data refreshes, the theme switches. `changed:Connect(fn)`. |
| `repohub.open(url)` | Opens an `https://` link in the browser. |
| `repohub.notify(text)` | Shows a short message at the bottom of the window. |
| `repohub.copy(text)` | Copies text to the clipboard. |
| `repohub.openRepo(fullName)` | Opens a repository's page (`"owner/name"`). |

The context:

```luau
{
  page = "repo",            -- or "overview", "repos", "pulls", "issues", "actions", "settings", "plugin"
  repo = {                  -- the repository on screen, or nil
    name, fullName, owner, description, language, branch, url, updated,  -- strings
    stars, forks, openIssues, pushed,                                    -- numbers
    private, fork, archived, starred,                                    -- booleans
    topics,                                                              -- { string }
  },
  issues = { { number, title, url, age, comments, label } },             -- its open issues
  pulls = { { number, title, url, age, author, state } },                -- its pull requests
  repos = { ... },          -- all your repositories, each like `repo`
  profile = { login, name },
  dark = true,              -- the dark look is on screen
  sample = false,           -- sample data (no account connected)
}
```

## UI

`@rune/ui` builds Rune elements:
- `ui.col(props, children)`, `ui.row(props, children)` and `ui.box(props, children)` are
  containers. A string child is text.
- `ui.text(content, props)` is text.
- `ui.spacer()` takes up the free space.
- `ui.virtualList{…}` shows long lists.

Common props:
- **Size and spacing:** `key`, `w` / `h` (points or `"full"`), `grow`, `pad`, `px`, `py`, `gap`.
- **Alignment:** `align`, `justify`.
- **Look:** `fill`, `hover`, `press`, `border`, `borderWidth`, `radius`, `opacity`.
- **Accessibility and actions:** `role`, `label`, `help`, `onPress`.
- **Text:** `size`, `weight` (`"regular"`, `"medium"`, `"semibold"`, `"bold"`), `color`, `mono`,
  `truncate`, `selectable`.

Colours follow the user's theme. Use these names:

| Name | What it is |
| --- | --- |
| `page`, `panel`, `sidebar` | backgrounds |
| `text`, `muted`, `faint` | text |
| `accent` | links, selection |
| `border` | lines |
| `success`, `warning`, `danger`, `info` | states |
| `hover`, `press`, `control-off` | interactive surfaces |

`ui.hex("#rrggbb")` gives a fixed colour.

Other modules:
- `task.spawn`, `task.delay` and `task.wait` (in tasks, not in a render) run code later.
- `require("@rune/signal").new()` makes your own signals.
- Export `saveHotState()` and `loadHotState(state)` to keep state across reloads.

The rune-luau reference is
[`crates/rune-luau/README.md`](https://github.com/anomalyco/rune/blob/main/crates/rune-luau/README.md)
in the Rune repository.

## Sandbox

A plugin can read only what `repohub.context()` gives it. It can't open files, start programs or
use the network, and its only actions are the ones above.

Problems stay inside the plugin:
- An error shows as a card with **Reload**, in place of the plugin. The message is also in
  Settings → Plugins.
- Code that runs longer than 250 ms at once is stopped.
- A plugin can use at most 64 MB of memory.

Plugins run in the desktop app. The web version has no plugins folder.

## Share it

Put the folder in a Git repository. Others paste its address in **Settings → Plugins** and press
**Install**: RepoHub clones it into their plugins folder. **Install** also takes a folder on the
computer. **Remove** deletes the plugin's folder.
