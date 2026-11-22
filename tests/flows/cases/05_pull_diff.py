"""Against the mock: open a pull request from the list, its checks and reviews load, then its
diff (the files GitHub sends, with their patches) opens under it."""
from flowkit import Flow, has, request, tap

REPO, NUMBER = "octo/project-003", 4

FLOW = Flow(
    "pull-diff",
    "Open a pull request and its diff: checks, files, added and removed lines",
    account="mock",
    args=["page", "pulls"],
    size="1180x1500",
    script=[
        tap(f"pr-{REPO}-{NUMBER}", at=0.3),
        tap("thread-files", at=1.8),
    ],
    at=3.8,
    expect=[
        has(key="thread"), has(text="Change number 4"), has(text="feature/cache → main"),
        has(key="thread-checks-summary", text="1 failed, 2 passed"),
        has(key="pull-diff"), has(text="3 files"),
        has(key="df0", label="src/cache.rs, 9 added, 3 removed"),
        has(label="removed line 13: const FRESH: Duration = Duration::from_secs(60);"),
        has(label="added line 13: const FRESH: Duration = Duration::from_secs(300);"),
        has(key="pull-merge", label="Merge"),
    ],
    requests=[
        request("GET", f"/repos/{REPO}/issues/{NUMBER}"),
        request("GET", f"/repos/{REPO}/pulls/{NUMBER}"),
        request("GET", f"/repos/{REPO}/pulls/{NUMBER}/reviews"),
        request("GET", f"/repos/{REPO}/commits/e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d5e/check-runs"),
        request("GET", f"/repos/{REPO}/pulls/{NUMBER}/files"),
    ],
)
