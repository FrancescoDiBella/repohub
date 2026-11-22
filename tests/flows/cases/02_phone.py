"""The phone layout from the start (REPOHUB_COMPACT=1 at 390 x 844): the bottom tab bar instead
of the sidebar, and the repositories page."""
from flowkit import Flow, has

FLOW = Flow(
    "phone",
    "The phone layout: tab bar, no sidebar, the repositories list",
    account="sample",
    size="390x844",
    compact=True,
    args=["page", "repos"],
    at=0.5,
    expect=[
        has(key="tab-Overview"), has(key="tab-Repositories"),
        has(key="page"),
        has(text="francescodibella.github.io"),
    ],
    absent=[has(key="sidebar")],
)
