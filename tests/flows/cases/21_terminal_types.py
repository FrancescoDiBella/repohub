"""The one-key shortcuts leave the terminal alone: with the keyboard in the shell, `/` and `j` are
typed there, and the palette doesn't open."""
from flowkit import Flow, has, tap, write

import _clone

FLOW = Flow(
    "terminal-types",
    "A shell has the keyboard: / and j go to it, not to the app",
    account="mock",
    args=["page", "repo", "tab", "terminal"],
    setup=_clone.make,
    script=[
        tap("open-terminal", at=0.3),
        tap("terminal", at=1.6),
        write("/j", at=2.0),
    ],
    at=2.6,
    expect=[has(key="terminal")],
    absent=[has(key="palette-input"), has(key="palette")],
)
