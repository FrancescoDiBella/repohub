"""The Changes tab on a temporary git repository (see _clone.py): the branch, the changed file and
the new one, with their lines."""
from flowkit import Flow, has

import _clone

FLOW = Flow(
    "changes",
    "The Changes tab on a temporary clone: the branch, the changed and new files",
    account="mock",
    args=["page", "repo", "tab", "changes"],
    size="1180x1100",
    setup=_clone.make,
    at=0.5,
    expect=[
        has(key="changes-status"), has(text="not pushed yet"),
        has(key="commit-msg"), has(key="commit", label="Commit"),
        has(key="changes-diff"), has(text="2 files"),
        has(text="src/cache.rs"), has(text="src/status.rs"),
        has(label="removed line 2: const FRESH: u64 = 60;"),
        has(label="added line 2: const FRESH: u64 = 300;"),
    ],
)
