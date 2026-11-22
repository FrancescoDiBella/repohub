"""Against the mock: the Commits section reads the branch's history before the first frame (day
groups, verified marks); the filter narrows it, and a commit opens with its files and lines."""
from flowkit import Flow, has, request, tap, write

REPO = "acme/project-000"
SHA = "d0c1b2a3948576a6b5c4d3e2f1a0b9c8d7e6f5a4"

FLOW = Flow(
    "commits",
    "The history, its filter, one commit with its files",
    account="mock",
    args=["page", "repo", "tab", "commits"],
    size="1180x1200",
    script=[
        tap("commits-filter", at=0.3),
        write("cache", at=0.4),
        tap(f"commit-{SHA}", at=1.2),
    ],
    at=3.0,
    expect=[
        has(key="commit-title", text="Count the answers read from the cache"),
        has(key="commit-sha"), has(text="d0c1b2a"),
        has(text="alice"),
        has(text="src/cache.rs"), has(text="src/status.rs"),
        has(label="added line 13: const FRESH: Duration = Duration::from_secs(300);"),
        has(key="commit-back"),
    ],
    absent=[has(key="commits-history")],
    requests=[
        request("GET", f"/repos/{REPO}/commits"),
        request("GET", f"/repos/{REPO}/commits/{SHA}"),
    ],
)
