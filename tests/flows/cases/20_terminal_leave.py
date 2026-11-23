"""Leaving the terminal with the keyboard: Ctrl+Shift+Tab takes the keyboard from the shell, so a
key typed next reaches the app (here `/`, which opens the palette on a page without a search)."""
from flowkit import Flow, has, key, tap, write

import _clone

FLOW = Flow(
    "terminal-leave",
    "Ctrl+Shift+Tab leaves the shell; / then opens the palette",
    account="mock",
    args=["page", "repo", "tab", "terminal"],
    setup=_clone.make,
    script=[
        tap("open-terminal", at=0.3),
        tap("terminal", at=1.6),
        key("Tab", at=2.0, ctrl=True, shift=True),
        write("/", at=2.3),
    ],
    at=2.9,
    expect=[has(key="terminal"), has(key="palette-input")],
)
