--!strict
-- Checklist: an example RepoHub plugin.
--
-- `repo(view)` draws the "Checklist" section of a repository's page and `page(view)` the
-- "Checklists" page in the sidebar (plugin.json names both). Edit this file while RepoHub runs:
-- the plugin reloads in place and keeps its ticks (saveHotState / loadHotState).
local ui = require("@rune/ui")
local repohub = require("@repohub")

-- The steps, the same for every repository.
local STEPS = {
	"Tests pass on main",
	"CHANGELOG.md updated",
	"Version bumped",
	"Docs and README up to date",
	"Release tagged",
}

-- Ticked steps for each repository: done[fullName][step] = true.
local done: { [string]: { [number]: boolean } } = {}

-- What's on screen. `changed` fires whenever it changes; reading it again in a render is fine.
local ctx = repohub.context()
repohub.changed:Connect(function(new)
	ctx = new
end)

local function count(fullName: string): number
	local n = 0
	for _, ticked in done[fullName] or {} do
		if ticked then
			n += 1
		end
	end
	return n
end

local function toggle(fullName: string, step: number)
	local ticks = done[fullName] or {}
	ticks[step] = not ticks[step]
	done[fullName] = ticks
	if count(fullName) == #STEPS then
		repohub.notify(`{fullName} is ready to release`)
	end
end

local function bar(n: number, width: number)
	return ui.box({ w = width, h = 6, radius = 3, fill = "border" }, {
		ui.box({ w = width * n / #STEPS, h = 6, radius = 3, fill = if n == #STEPS then "success" else "accent" }),
	})
end

local function card(children)
	return ui.col({ w = "full", pad = 16, gap = 10, radius = 10, fill = "panel", border = "border", borderWidth = 1 }, children)
end

local function step(fullName: string, i: number, label: string)
	local ticked = (done[fullName] or {})[i] == true
	return ui.row({
		key = `step-{i}`, w = "full", gap = 10, align = "center", px = 8, py = 6, radius = 6,
		hover = "hover", press = "press", role = "checkbox", label = label,
		onPress = function()
			toggle(fullName, i)
		end,
	}, {
		ui.box({ w = 16, h = 16, radius = 4, borderWidth = 1.5, border = if ticked then "accent" else "muted", fill = if ticked then "accent" else nil }),
		ui.text(label, { size = 13, color = if ticked then "muted" else "text" }),
	})
end

local function repo(view)
	local r = ctx.repo
	if not r then
		return ui.text("Open a repository to see its checklist.", { color = "muted" })
	end
	local n = count(r.fullName)
	local rows = {}
	for i, label in STEPS do
		table.insert(rows, step(r.fullName, i, label))
	end
	return ui.col({ w = "full", gap = 12 }, {
		card({
			ui.row({ w = "full", gap = 10, align = "center" }, {
				ui.text(`Release checklist · {r.name}`, { size = 15, weight = "semibold", color = "text" }),
				ui.spacer(),
				ui.text(`{n} of {#STEPS}`, { size = 12, color = "muted" }),
				bar(n, 120),
			}),
			ui.col({ w = "full", gap = 2 }, rows),
		}),
		card({
			ui.text("Before you tag", { size = 13, weight = "semibold", color = "text" }),
			ui.text(`{#ctx.pulls} open pull requests and {#ctx.issues} open issues in {r.fullName}.`, { size = 12, color = "muted" }),
			ui.row({
				key = "open-releases", px = 10, py = 5, radius = 6, fill = "control-off", hover = "hover", press = "press",
				onPress = function()
					repohub.open(r.url .. "/releases")
				end,
			}, { ui.text("Releases on GitHub", { size = 12, weight = "medium", color = "text" }) }),
		}),
	})
end

local function page(view)
	local rows = {}
	for _, r in ctx.repos do
		local n = count(r.fullName)
		table.insert(rows, ui.row({
			key = r.fullName, w = "full", gap = 12, align = "center", px = 12, py = 8, radius = 6,
			hover = "hover", press = "press",
			onPress = function()
				repohub.openRepo(r.fullName)
			end,
		}, {
			ui.text(r.name, { size = 13, weight = "medium", color = "text", w = 220, truncate = true }),
			bar(n, 160),
			ui.text(`{n} of {#STEPS}`, { size = 12, color = "muted" }),
			ui.spacer(),
			ui.text(`★ {r.stars}`, { size = 12, color = "muted" }),
		}))
	end
	return ui.col({ w = "full", gap = 16 }, {
		ui.col({ gap = 4 }, {
			ui.text("Checklists", { size = 24, weight = "semibold", color = "text" }),
			ui.text("Release progress for each repository. Click one to open its checklist.", { size = 13, color = "muted" }),
		}),
		card(rows),
	})
end

return {
	repo = repo,
	page = page,
	saveHotState = function()
		return done
	end,
	loadHotState = function(saved)
		done = saved
	end,
}
