"""Notes: name a new drawing, draw on it, close it. The board is saved as an .excalidraw file in
the data folder (by repository and branch), the list shows it, and it opens again from there."""
import json
import os

from flowkit import Flow, drag, has, tap, write

NAME = "Release plan"


def check(run, tree):
    # The folder and file names are percent-encoded (a space is %20).
    path = os.path.join(run.data, "notes", "acme", "project-000", "main", NAME.replace(" ", "%20") + ".excalidraw")
    try:
        with open(path) as f:
            board = json.load(f)
    except (OSError, ValueError) as e:
        return [f"the drawing wasn't saved as {os.path.relpath(path, run.data)}: {e}"]
    shapes = [e for e in board.get("elements", []) if not e.get("isDeleted")]
    if board.get("type") != "excalidraw" or not shapes:
        return [f"the saved drawing has no strokes: {json.dumps(board)[:200]}"]
    return []


FLOW = Flow(
    "notes",
    "A new drawing: name it, draw, close it; the file is saved, listed and opens again",
    account="mock",
    args=["page", "repo", "tab", "notes"],
    size="1180x900",
    script=[
        tap("note-name", at=0.3),
        write(NAME, at=0.4),
        tap("new-note", at=1.2),
        drag("board", at=2.0, dx=180, dy=60),
        tap("close-board", at=3.0),
        tap(f"open-note-{NAME}", at=4.0),
    ],
    at=5.0,
    expect=[has(key="board"), has(key="close-board"), has(key="undo"), has(text=NAME), has(text="Saved on this computer")],
    absent=[has(key="new-note")],
    check=check,
)
