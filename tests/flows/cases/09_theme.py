"""The theme switch in the toolbar: from the dark look to the light one. The whole window changes
(the shell's background is light), the button now offers the dark theme, and the choice is saved
in the preferences for the next start."""
import json
import os

from flowkit import Flow, has, tap


def light(fill):
    """`#rrggbbaa` → whether it's a light colour."""
    r, g, b = (int(fill[i:i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b > 160


def check(run, tree):
    problems = []
    shell = next((e for e in tree["elements"] if e.get("key") == "shell"), None)
    if shell is None or not light(shell.get("fill", "#000000ff")):
        problems.append(f"the window's background isn't light: {shell and shell.get('fill')}")
    try:
        with open(os.path.join(run.data, "prefs.json")) as f:
            theme = json.load(f).get("theme")
    except (OSError, ValueError) as e:
        theme = f"unreadable ({e})"
    if theme != "light":
        problems.append(f"prefs.json says the theme is {theme!r}, not 'light'")
    return problems


FLOW = Flow(
    "theme",
    "Switch to the light theme: the window, the button, the saved preference",
    account="sample",
    args=["page", "settings", "section", "appearance"],
    script=[tap("theme", at=0.3)],
    at=1.5,
    expect=[
        has(key="theme", label="Switch to the dark theme"),
        has(key="appearance"), has(key="theme-pick"),
        has(key="look-classic-light", label="Use the theme Classic Light"),
    ],
    absent=[has(key="theme", label="Switch to the light theme")],
    check=check,
)
