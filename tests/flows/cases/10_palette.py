"""The command palette: ⌘K (Ctrl+K) opens it, a few letters find a repository by fuzzy match,
Enter opens it and closes the palette."""
from flowkit import Flow, has, key, write

FLOW = Flow(
    "palette",
    "⌘K, type part of a name, Enter: the repository opens",
    account="sample",
    script=[
        key("k", at=0.3, cmd=True),
        write("dotf", at=0.7),
        key("Enter", at=1.3),
    ],
    at=2.3,
    expect=[
        has(key="title", text="francescodibella / dotfiles"),
        has(key="tabs"), has(key="readme-copy"),
    ],
    absent=[has(key="palette"), has(key="palette-input")],
)
