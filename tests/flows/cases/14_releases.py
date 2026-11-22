"""Against the mock: the Releases section lists GitHub's releases (a draft, the latest, a
pre-release) and the tag without one. New release: pick the next minor version, let GitHub write
the notes, save a draft: GitHub gets both requests and the draft shows at the top."""
from flowkit import Flow, has, request, tap

REPO = "acme/project-000"

FLOW = Flow(
    "releases",
    "Releases and tags; a new draft with generated notes",
    account="mock",
    args=["page", "repo", "tab", "releases"],
    size="1180x1400",
    script=[
        tap("release-new", at=0.3),
        tap("release-chip-v1.4.0", at=0.8),
        tap("release-generate", at=1.2),
        tap("release-save", at=2.6),
    ],
    at=4.5,
    expect=[
        has(key="releases-list"),
        has(key="release-kind-9002", text="Latest"), has(key="release-kind-9001", text="Pre-release"),
        has(key="release-9100"), has(key="release-kind-9100", text="Draft"),
        has(text="v1.4.0"),
        has(key="release-tags"), has(text="v1.0.0"),
        has(text="Draft saved"),
    ],
    absent=[has(key="release-form")],
    requests=[
        request("GET", f"/repos/{REPO}/releases"),
        request("GET", f"/repos/{REPO}/tags"),
        request("POST", f"/repos/{REPO}/releases/generate-notes", body={"tag_name": "v1.4.0"}),
        request("POST", f"/repos/{REPO}/releases", body={"tag_name": "v1.4.0", "draft": True}),
    ],
)
