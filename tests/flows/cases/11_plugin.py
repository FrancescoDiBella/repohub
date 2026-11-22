"""The example plugin (plugins/checklist), installed in a fresh data folder: its page in the
sidebar, then its section on a repository's page (opened from the palette), where ticking two
steps updates the count the Luau script draws."""
import os
import shutil

from flowkit import Flow, has, key, tap, write


def setup(run):
    shutil.copytree(os.path.join(run.root, "plugins", "checklist"), os.path.join(run.data, "plugins", "checklist"))


FLOW = Flow(
    "plugin",
    "The checklist plugin: its sidebar page, its section, two ticked steps",
    account="sample",
    args=["page", "repo"],
    setup=setup,
    script=[
        key("k", at=0.3, cmd=True),
        write("checklist", at=0.7),
        key("Enter", at=1.4),
        tap("step-1", at=2.0),
        tap("step-2", at=2.3),
    ],
    at=3.0,
    expect=[
        has(key="plugin-page-checklist", label="Checklists"),
        has(key="segment10", label="Checklist"),
        has(key="luau:checklist:repo"),
        has(text="Release checklist · francescodibella.github.io"),
        has(key="step-1", role="checkbox", label="Tests pass on main"),
        has(key="step-5", role="checkbox", label="Release tagged"),
        has(text="2 of 5"),
    ],
    absent=[has(key="readme-copy")],
)
