"""Against the mock: the Commits section as it opens: the branch's history grouped by day, with
the verified marks, and the filter keeping the commits that match."""
from flowkit import Flow, has, tap, write

FLOW = Flow(
    "history",
    "The branch's history by day, then filtered to the matching commits",
    account="mock",
    args=["page", "repo", "tab", "commits"],
    size="1180x1000",
    script=[
        tap("commits-filter", at=0.3),
        write("status", at=0.4),
    ],
    at=1.5,
    expect=[
        has(key="commits-history"), has(text="History of main"),
        # A day's heading ("Yesterday", "17 November"…) depends on today's date: any will do.
        has(key="commits-day-*", role="heading"),
        has(key="commit-c9b0a1928374655f4e3d2c1b0a9f8e7d6c5b4a39", label="Fix the status line on narrow terminals"),
    ],
    absent=[
        has(key="commit-e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d5e"),
        has(key="commit-85b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e"),
    ],
)
