"""The terminal doesn't keep the app's shortcuts: with the keyboard in a running shell, ⌘/Ctrl+K
still opens the palette (on Windows and Linux Ctrl+K would otherwise be a control key for the
shell)."""
from flowkit import Flow, has, key, tap

import _clone

FLOW = Flow(
    "terminal-keys",
    "A shell in the clone has the keyboard: Ctrl+K opens the palette",
    account="mock",
    args=["page", "repo", "tab", "terminal"],
    setup=_clone.make,
    script=[
        tap("open-terminal", at=0.3),
        tap("terminal", at=1.6),
        key("k", at=2.0, cmd=True, ctrl=True),
    ],
    at=2.6,
    expect=[has(key="terminal"), has(key="palette-input")],
)
