"""Against the mock: open an issue's conversation; its details block shows the labels, assignees
and milestone GitHub sent. Edit labels reads the repository's labels into the picker; ticking
one more and Apply sends them all, and the block and a toast show the answer."""
from flowkit import Flow, has, request, tap

REPO, NUMBER = "octo/project-003", 203

FLOW = Flow(
    "conversation",
    "An issue's conversation: comments, the details block, changing its labels",
    account="mock",
    args=["page", "issues"],
    script=[
        tap(f"issue-{REPO}-{NUMBER}", at=0.3),
        tap("triage-edit-labels", at=1.8),
        tap("triage-choice-documentation", at=3.0),
        tap("triage-apply", at=3.4),
    ],
    at=5.0,
    expect=[
        has(key="thread"), has(text="Problem number 203"),
        has(text="I can reproduce this on Linux too"),
        has(key="triage"), has(key="triage-labels"), has(key="triage-assignees"), has(key="triage-milestone"),
        has(text="v1.3"), has(text="alice"),
        has(text="documentation"),
        has(text="Labels of #203: bug, performance, documentation"),
    ],
    absent=[has(key="triage-picker")],
    requests=[
        request("GET", f"/repos/{REPO}/issues/{NUMBER}"),
        request("GET", f"/repos/{REPO}/issues/{NUMBER}/comments"),
        request("GET", f"/repos/{REPO}/labels"),
        request("PATCH", f"/repos/{REPO}/issues/{NUMBER}", body={"labels": {"bug", "performance", "documentation"}}),
    ],
)
