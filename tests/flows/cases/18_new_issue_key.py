"""On a repository's page `i` opens the New issue form with the keyboard in its title, where the
next letters are typed (an `i` or a `p` there is text, not a shortcut)."""
from flowkit import Flow, has, write

FLOW = Flow(
    "new-issue-key",
    "A repository's page: i opens New issue; what's typed next goes into its title",
    account="sample",
    args=["page", "repo"],
    script=[
        write("i", at=0.3),
        write("Pin the input", at=0.8),
    ],
    at=2.0,
    expect=[has(key="form"), has(text="New issue"), has(text="Pin the input")],
)
