"""What a flow in tests/flows/cases/ is made of: `Flow`, the script events (`tap`, `write`, `key`),
what must be on screen (`has`) and what the mock must have received (`request`).

A flow runs the real program (target/release/repohub) headless once: its start-up arguments,
its script of taps and keys on Rune's virtual clock, then the element tree (`--tree`) and a still
(`--snap`) at `at` seconds. It passes when the program exits cleanly without a panic, every
`expect` is on screen, no `absent` is, the mock got every `request` and answered nothing with a
404, and `check` (if any) finds nothing wrong. `fail` makes chosen reads fail, to see the error
and its Retry.
"""
from dataclasses import dataclass, field
from typing import Callable, Optional


# ------------------------------------------------------------------ script events
# Times are seconds on the run's virtual clock. Background work (GitHub's answers, git) runs on
# the wall clock: leave a second or more between an event that starts it and the still.

def tap(key, at):
    """Press and release the element with this key (its centre: it must be on screen)."""
    return {"t": at, "type": "tap", "key": key}


def write(text, at):
    """Type text into whatever has the focus."""
    return {"t": at, "type": "type", "text": text}


def key(name, at, **mods):
    """A key press: "Enter", "Escape", "Tab"…; mods: cmd=True, shift=True, ctrl=True, alt=True."""
    return {"t": at, "type": "key", "name": name, **mods}


def drag(key, at, dx, dy, dur=0.4):
    """Press on the element with this key, move by (dx, dy) points over `dur` seconds, release."""
    return {"t": at, "type": "drag", "key": key, "dx": dx, "dy": dy, "dur": dur}


# ------------------------------------------------------------------ what must be there

@dataclass
class Match:
    """An element of the tree: a key (exact, or a prefix ending in `*`), text it contains, a
    label it contains, a role. Every one given must hold for the same element."""
    key: Optional[str] = None
    text: Optional[str] = None
    label: Optional[str] = None
    role: Optional[str] = None

    def holds(self, e):
        k = e.get("key") or ""
        if self.key is not None:
            if self.key.endswith("*"):
                if not k.startswith(self.key[:-1]):
                    return False
            elif k != self.key:
                return False
        if self.text is not None and self.text not in (e.get("text") or ""):
            return False
        if self.label is not None and self.label not in (e.get("label") or ""):
            return False
        if self.role is not None and e.get("role") != self.role:
            return False
        return True

    def __str__(self):
        parts = []
        if self.key is not None:
            parts.append(f"key {self.key!r}")
        if self.role is not None:
            parts.append(f"role {self.role!r}")
        if self.label is not None:
            parts.append(f"a label with {self.label!r}")
        if self.text is not None:
            parts.append(f"text {self.text!r}")
        return "an element with " + " and ".join(parts)


def has(key=None, text=None, label=None, role=None):
    return Match(key, text, label, role)


def shown(tree):
    """The elements on screen: in the tree, not fading out, not transparent."""
    return [e for e in tree["elements"] if not e.get("exiting") and e.get("opacity", 1) > 0.01]


@dataclass
class Request:
    """A request the mock must have received and answered with a 2xx: the method, the path
    (without the query), and for a write, values its JSON body must carry (a set: a list with
    these items, in any order)."""
    method: str
    path: str
    body: Optional[dict] = None

    def holds(self, entry):
        if entry["method"] != self.method or entry["path"].split("?")[0] != self.path:
            return False
        if self.body is not None:
            got = entry.get("body")
            if not isinstance(got, dict):
                return False
            for k, want in self.body.items():
                # A set: the list sent holds these, in any order.
                have = set(got[k]) if isinstance(want, set) and isinstance(got.get(k), list) else got.get(k)
                if have != want:
                    return False
        return 200 <= entry["status"] < 300

    def __str__(self):
        return f"{self.method} {self.path}" + (f" with {self.body}" if self.body else "")


def request(method, path, body=None):
    return Request(method, path, body)


# ------------------------------------------------------------------ a flow

@dataclass
class Flow:
    name: str
    about: str
    # "sample": no account, the sample data. "mock": an account on the mock GitHub, its data
    # loaded before the first frame (REPOHUB_BLOCKING=1).
    account: str = "sample"
    args: list = field(default_factory=list)     # after `--`: page repo, tab notes…
    size: str = "1180x900"
    compact: bool = False                         # the phone layout (REPOHUB_COMPACT=1)
    env: dict = field(default_factory=dict)
    script: list = field(default_factory=list)
    at: float = 1.0                               # when the still and the tree are taken
    expect: list = field(default_factory=list)
    absent: list = field(default_factory=list)
    requests: list = field(default_factory=list)
    # Reads the mock answers with an error for this run: {path (without the query): status}.
    fail: dict = field(default_factory=dict)
    # Prepares the run: gets a `Run` (its folders and environment) before the program starts.
    setup: Optional[Callable] = None
    # Looks at the result: gets the `Run` and the tree, returns what's wrong (strings).
    check: Optional[Callable] = None
