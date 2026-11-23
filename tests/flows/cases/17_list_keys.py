"""The keyboard on a list: `j` marks the first issue, ↓ the next, Enter opens the one marked, as a
click does."""
from flowkit import Flow, has, key, write

FLOW = Flow(
    "list-keys",
    "Issues: j, then ↓, then Enter opens the second issue's conversation",
    account="sample",
    args=["page", "issues"],
    script=[
        write("j", at=0.3),
        key("Down", at=0.5),
        key("Enter", at=0.8),
    ],
    at=1.6,
    expect=[has(key="thread"), has(key="thread-back")],
    absent=[has(key="issues-filters")],
)
