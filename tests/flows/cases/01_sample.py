"""The first start without an account: the overview of the sample data, and the banner that says
so (no account, no network)."""
from flowkit import Flow, has

FLOW = Flow(
    "sample-start",
    "No account: the overview shows the sample data and says so",
    account="sample",
    size="1180x900",
    at=0.5,
    expect=[
        has(key="banner"),
        has(text="This is sample data"),
        has(key="connect", label="Connect"),
        has(key="nav-Overview"), has(key="nav-Repositories"), has(key="nav-Settings"),
        has(key="metric-Repositories"),
        has(key="pin-francescodibella/francescodibella.github.io"),
        has(key="open-palette"), has(key="theme"),
    ],
)
