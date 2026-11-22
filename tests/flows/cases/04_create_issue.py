"""Against the mock: write a new issue in the form and create it. GitHub (the mock) must get the
POST with the title and description typed, and the new issue must show at once, with a toast."""
from flowkit import Flow, has, request, tap, write

REPO = "acme/project-000"
TITLE = "The status line overlaps the clock"
BODY = "On a narrow terminal the status line runs into the clock."

FLOW = Flow(
    "create-issue",
    "New issue: the form, the POST GitHub gets, the new row and the toast",
    account="mock",
    args=["page", "repo", "tab", "issues"],
    script=[
        tap("new-issue", at=0.3),
        tap("form-title", at=0.7),
        write(TITLE, at=0.8),
        tap("form-body", at=2.4),
        write(BODY, at=2.5),
        tap("form-submit", at=5.0),
    ],
    at=7.0,
    expect=[
        has(key=f"issue-{REPO}-1001", label=TITLE),
        has(text="Opened issue #1001 in acme/project-000"),
    ],
    absent=[has(key="form")],
    requests=[request("POST", f"/repos/{REPO}/issues", body={"title": TITLE, "body": BODY})],
)
