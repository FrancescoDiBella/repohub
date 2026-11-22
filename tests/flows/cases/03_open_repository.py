"""Against the mock: find a repository with the search box and open it; its page reads the files,
README, languages, last commit and latest release from GitHub after the first frame."""
from flowkit import Flow, has, request, tap, write

REPO = "octo/project-003"

FLOW = Flow(
    "open-repository",
    "Search the repositories, open one: its files, README and latest release load",
    account="mock",
    args=["page", "repos"],
    script=[
        tap("search", at=0.3),
        write("project-003", at=0.5),
        tap(f"open-{REPO}", at=1.4),
    ],
    at=3.5,
    expect=[
        has(key="title", text="octo / project-003"),
        has(key="tabs"), has(key="segment0", label="Code"),
        has(key="file-README.md"), has(key="file-src"),
        has(key="readme-copy"), has(text="Some words about the project."),
        has(text="Keep answers for five minutes"),
        has(key="latest-release", label="v1.2.0"),
    ],
    absent=[has(key=f"open-octo/project-001")],
    requests=[
        request("GET", f"/repos/{REPO}/contents"),
        request("GET", f"/repos/{REPO}/readme"),
        request("GET", f"/repos/{REPO}/languages"),
        request("GET", f"/repos/{REPO}/commits"),
        request("GET", f"/repos/{REPO}/releases/latest"),
    ],
)
