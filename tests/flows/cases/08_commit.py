"""The Changes tab commits for real: a message and Commit run `git add -A` and `git commit` in the
temporary clone (see _clone.py); the repository's last commit is then that message, nothing is
left uncommitted, and the tab says so."""
from flowkit import Flow, has, tap, write

import _clone

MESSAGE = "Keep answers for five minutes"


def check(run, tree):
    problems = []
    log = _clone.git(run.repo, "log", "--format=%s").splitlines()
    if log[:1] != [MESSAGE]:
        problems.append(f"the clone's last commit is {log[:1]}, not {MESSAGE!r}")
    left = _clone.git(run.repo, "status", "--porcelain").strip()
    if left:
        problems.append(f"the commit left changes behind: {left}")
    return problems


FLOW = Flow(
    "commit",
    "Commit from the Changes tab: a real commit in the temporary clone",
    account="mock",
    args=["page", "repo", "tab", "changes"],
    size="1180x1100",
    setup=_clone.make,
    script=[
        tap("commit-msg", at=0.5),
        write(MESSAGE, at=0.6),
        tap("commit", at=2.0),
    ],
    at=4.0,
    expect=[
        has(key="toast", role="status"), has(text="Committed "),
        has(text="Nothing changed since the last commit."), has(text="0 files"),
    ],
    absent=[has(text="src/status.rs")],
    check=check,
)
