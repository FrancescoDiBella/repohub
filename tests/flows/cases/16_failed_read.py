"""Against the mock, with GitHub failing one read: the repository's README answers 500. The page
still shows what did load (the files), and the README says what went wrong with a Retry, rather
than an empty panel or a spinner."""
from flowkit import Flow, has

REPO = "acme/project-000"

FLOW = Flow(
    "failed-read",
    "GitHub fails the README: the error and Retry show, the rest of the page loads",
    account="mock",
    args=["page", "repo"],
    fail={f"/repos/{REPO}/readme": 500},
    at=0.5,
    expect=[
        has(key="file-README.md"), has(text="Keep answers for five minutes"),
        has(key="readme-retry", label="Retry"), has(text="GitHub error 500"),
    ],
    absent=[has(key="readme-copy"), has(text="Some words about the project.")],
)
