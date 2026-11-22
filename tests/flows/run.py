#!/usr/bin/env python3
"""Runs RepoHub's flows: each file in tests/flows/cases/ drives the real program headless (against
the mock GitHub in tests/mock/ or on the sample data), checks the element tree and what the mock
received, and saves a still.

    python3 tests/flows/run.py [--app target/release/repohub] [--out target/flows] [--only NAME[,NAME]]
                               [--list] [--keep]

Build the program first (cargo build --release). The stills (NAME.png) and each run's output
(NAME.log) go to the output folder, with the tree (NAME.tree.json) of a flow that failed. The exit
status is 1 when a flow failed. Nothing leaves this computer: the program gets a made-up token, its
own empty home and data folders, only the mock's address, and a proxy that refuses every
connection, so a request to any other host fails instead of reaching it.
"""
import argparse
import importlib.util
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass, field

sys.dont_write_bytecode = True  # no __pycache__ folders in the checkout
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CASES = os.path.join(HERE, "cases")
sys.path[:0] = [HERE, CASES]  # flowkit, and the helpers the flows share (cases/_*.py)
from flowkit import Flow, shown  # noqa: E402

TOKEN = "flows-token-not-real"
FPS = 20
TIMEOUT = 120
# What the program may keep from the environment: the tools it runs (git) and the GPU's
# software driver. Everything else, tokens included, stays out.
KEEP = ("PATH", "TMPDIR", "LD_LIBRARY_PATH", "XDG_RUNTIME_DIR")
KEEP_PREFIXES = ("VK_", "WGPU_", "RUNE_", "MESA_", "LIBGL_", "GALLIUM_", "LP_")


@dataclass
class Run:
    """One flow's run, for its `setup` and `check`: its folders and the program's environment."""
    flow: Flow
    work: str           # a throwaway folder; the program runs in it
    data: str           # REPOHUB_DATA_DIR
    home: str           # HOME
    env: dict
    mock: str           # the mock's address
    root: str = ROOT    # this checkout (plugins/checklist, …)
    log: list = field(default_factory=list)  # what the mock received (after the run)


def text_of(out):
    """What a timed-out process printed (bytes or text, depending on Python's version)."""
    return out.decode(errors="replace") if isinstance(out, bytes) else (out or "")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def call(url, method="GET"):
    req = urllib.request.Request(url, method=method, data=b"" if method == "POST" else None)
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read() or b"null")


def start_mock(out):
    port = free_port()
    env = {**{k: v for k, v in os.environ.items() if k in KEEP}, "MOCK_DELAY_MS": "0", "MOCK_REPOS": "12"}
    log = open(os.path.join(out, "mock.log"), "w")
    proc = subprocess.Popen([sys.executable, os.path.join(ROOT, "tests", "mock", "mock_github.py"), str(port)],
                            env=env, stdout=log, stderr=subprocess.STDOUT)
    url = f"http://127.0.0.1:{port}"
    for _ in range(100):
        try:
            call(url + "/_stats")
            return proc, url
        except OSError:
            if proc.poll() is not None:
                sys.exit(f"the mock didn't start: see {log.name}")
            time.sleep(0.1)
    proc.kill()
    sys.exit("the mock didn't answer within 10 seconds")


def load_flows():
    flows = []
    for name in sorted(os.listdir(CASES)):
        if not name.endswith(".py") or name.startswith("_"):
            continue
        spec = importlib.util.spec_from_file_location(f"flow_{name[:-3]}", os.path.join(CASES, name))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        flow = getattr(mod, "FLOW", None)
        if not isinstance(flow, Flow):
            sys.exit(f"tests/flows/cases/{name} has no FLOW = Flow(...)")
        flows.append(flow)
    names = [f.name for f in flows]
    if len(set(names)) != len(names):
        sys.exit("two flows have the same name")
    return flows


def environment(flow, work, data, home, mock):
    env = {k: v for k, v in os.environ.items() if k in KEEP or k.startswith(KEEP_PREFIXES)}
    refuse = "http://127.0.0.1:9"  # the discard port: nothing listens, every connection fails
    env.update({
        "HOME": home, "REPOHUB_DATA_DIR": data, "LANG": "C.UTF-8", "TZ": "UTC",
        "REPOHUB_UPDATES": "off", "REPOHUB_TRAY": "0", "REPOHUB_CACHE": "off", "GIT_CONFIG_NOSYSTEM": "1",
        "HTTPS_PROXY": refuse, "HTTP_PROXY": refuse, "ALL_PROXY": refuse,
        "https_proxy": refuse, "http_proxy": refuse, "all_proxy": refuse,
        "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
    })
    if flow.account == "mock":
        env.update({"GITHUB_API_URL": mock, "GITHUB_TOKEN": TOKEN, "REPOHUB_BLOCKING": "1"})
    elif flow.account != "sample":
        raise ValueError(f"{flow.name}: account is 'sample' or 'mock', not {flow.account!r}")
    if flow.compact:
        env["REPOHUB_COMPACT"] = "1"
    env.update(flow.env)
    return env


def run_flow(flow, app, out, mock, keep):
    """Runs one flow; returns (what went wrong, seconds)."""
    call(mock + "/_reset", "POST")
    for path, status in flow.fail.items():
        call(f"{mock}/_fail?{urllib.parse.urlencode({'path': path, 'status': status})}", "POST")
    work = tempfile.mkdtemp(prefix=f"repohub-flow-{flow.name}-")
    data, home = os.path.join(work, "data"), os.path.join(work, "home")
    os.makedirs(data)
    os.makedirs(home)
    run = Run(flow, work, data, home, environment(flow, work, data, home, mock), mock)
    problems = []
    t0 = time.monotonic()
    try:
        if flow.setup:
            flow.setup(run)
        still, tree_path = os.path.join(out, f"{flow.name}.png"), os.path.join(work, "tree.json")
        for stale in (still, os.path.join(out, f"{flow.name}.tree.json")):
            if os.path.exists(stale):
                os.remove(stale)
        cmd = [app, "--snap", still, "--tree", tree_path, "--size", flow.size, "--scale", "1",
               "--fps", str(FPS), "--at", str(flow.at)]
        if flow.script:
            script = os.path.join(work, "script.json")
            with open(script, "w") as f:
                json.dump(flow.script, f, indent=1)
            cmd += ["--script", script]
        if flow.args:
            cmd += ["--", *flow.args]
        try:
            p = subprocess.run(cmd, env=run.env, cwd=work, capture_output=True, text=True, timeout=TIMEOUT)
            output, status = p.stdout + p.stderr, p.returncode
        except subprocess.TimeoutExpired as e:
            output = text_of(e.stdout) + text_of(e.stderr)
            status = None
            problems.append(f"didn't finish within {TIMEOUT} s")
        with open(os.path.join(out, f"{flow.name}.log"), "w") as f:
            f.write(" ".join(cmd) + "\n\n" + output)
        panics = [line for line in output.splitlines() if "panicked at" in line]
        if panics:
            problems.append("panicked: " + "; ".join(panics[:3]))
        if status not in (0, None):
            tail = [line for line in output.splitlines() if line.strip() and not line.startswith("rune: ")][-3:]
            problems.append(f"exited with status {status}" + (": " + " / ".join(tail) if tail else ""))
        tree = None
        if os.path.exists(tree_path):
            with open(tree_path) as f:
                tree = json.load(f)
        elif status == 0:
            problems.append("wrote no element tree")
        if tree is not None:
            els = shown(tree)
            for m in flow.expect:
                if not any(m.holds(e) for e in els):
                    problems.append(f"missing: {m}")
            for m in flow.absent:
                if any(m.holds(e) for e in els):
                    problems.append(f"still there: {m}")
        run.log = call(mock + "/_log")
        for r in flow.requests:
            if not any(r.holds(e) for e in run.log):
                # The last few of that method, the most recent first: what the flow did instead.
                sent = [f"{e['method']} {e['path']} -> {e['status']}" for e in reversed(run.log) if e["method"] == r.method]
                problems.append(f"the mock didn't get {r}" + (f" (the last it got: {', '.join(sent[:3])})" if sent else ""))
        for e in run.log:
            if e["status"] == 404 and e["path"].split("?")[0] not in flow.fail:
                problems.append(f"the mock has no answer for {e['method']} {e['path']}: add it to tests/mock or fix the path")
        if tree is not None and flow.check:
            problems += flow.check(run, tree) or []
        if problems and tree is not None:
            shutil.copy(tree_path, os.path.join(out, f"{flow.name}.tree.json"))
    except Exception as e:  # a broken flow file is a failure of that flow, not of the run
        problems.append(f"the flow itself failed: {type(e).__name__}: {e}")
    finally:
        if not keep:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"  kept {work}")
    return problems, time.monotonic() - t0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", default=os.path.join(ROOT, "target", "release", "repohub"))
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "flows"))
    ap.add_argument("--only", action="append", default=[], help="run only these flows (names, comma-separated)")
    ap.add_argument("--list", action="store_true", help="list the flows and stop")
    ap.add_argument("--keep", action="store_true", help="keep each run's folder (data, home, tree)")
    a = ap.parse_args()
    flows = load_flows()
    if a.list:
        for f in flows:
            print(f"{f.name:<22} {f.about}")
        return 0
    only = {n for o in a.only for n in o.split(",") if n}
    unknown = only - {f.name for f in flows}
    if unknown:
        sys.exit(f"no flow named {', '.join(sorted(unknown))} (see --list)")
    flows = [f for f in flows if not only or f.name in only]
    app = os.path.abspath(a.app)
    if not os.access(app, os.X_OK):
        sys.exit(f"{app} isn't there: build it first (cargo build --release)")
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    mock, url = start_mock(out)
    failed, rows, t0 = [], [], time.monotonic()
    try:
        for flow in flows:
            problems, secs = run_flow(flow, app, out, url, a.keep)
            rows.append((flow, problems, secs))
            print(f"{'ok  ' if not problems else 'FAIL'} {flow.name:<22} {secs:5.1f} s  {flow.about}", flush=True)
            for p in problems:
                print(f"       {p}", flush=True)
            if problems:
                failed.append(flow.name)
    finally:
        mock.kill()
        mock.wait()
    total = time.monotonic() - t0
    print(f"\n{len(flows) - len(failed)} of {len(flows)} flows passed in {total:.1f} s; stills in {out}")
    if os.environ.get("GITHUB_ACTIONS") == "true":
        for flow, problems, _ in rows:
            for p in problems:
                print(f"::error title=Flow {flow.name}::{p}")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a") as f:
                f.write(f"### Flows: {len(flows) - len(failed)} of {len(flows)} passed in {total:.0f} s\n\n| Flow | | Seconds | What it checks |\n| --- | --- | --- | --- |\n")
                for flow, problems, secs in rows:
                    f.write(f"| {flow.name} | {'ok' if not problems else '**failed**'} | {secs:.1f} | {flow.about} |\n")
                f.write("\nThe stills are in the run's `flows` artifact.\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
