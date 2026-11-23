"""The example plugin's Checklists page says "Click one to open its checklist": clicking a
repository there opens that repository on the plugin's own section, not on Code."""
import os
import shutil

from flowkit import Flow, has, tap


def setup(run):
    shutil.copytree(os.path.join(run.root, "plugins", "checklist"), os.path.join(run.data, "plugins", "checklist"))


FLOW = Flow(
    "plugin-page",
    "The Checklists page: a click opens that repository's checklist",
    account="sample",
    setup=setup,
    script=[
        tap("plugin-page-checklist", at=0.5),
        tap("francescodibella/francescodibella.github.io", at=1.5),
    ],
    at=2.5,
    expect=[
        has(key="luau:checklist:repo"),
        has(text="Release checklist · francescodibella.github.io"),
        has(key="step-1", role="checkbox"),
    ],
    absent=[has(key="readme-copy")],
)
