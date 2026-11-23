"""Merging on a phone: Merge on a pull request's row asks in a sheet at the bottom of the screen,
with how to merge (here Squash); its answer sends the merge GitHub (the mock) must get, and the
row says it's merged."""
from flowkit import Flow, has, request, tap

REPO = "octo/project-001"
KEY = f"{REPO}#2"

FLOW = Flow(
    "merge-sheet",
    "Phone: Merge asks in a sheet, Squash is picked there, GitHub gets the squash merge",
    account="mock",
    size="390x2400",
    compact=True,
    args=["page", "pulls"],
    script=[
        tap(f"merge-{KEY}", at=0.5),
        tap("merge-method/segment1", at=1.0),
        tap(f"ask-merge-{KEY}-yes", at=1.4),
    ],
    at=3.0,
    expect=[has(text="Merged pull request #2")],
    absent=[has(key="sheet"), has(key=f"ask-merge-{KEY}")],
    requests=[request("PUT", f"/repos/{REPO}/pulls/2/merge", body={"merge_method": "squash"})],
)
