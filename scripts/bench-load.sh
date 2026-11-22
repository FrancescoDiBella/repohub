#!/usr/bin/env bash
# Measures how RepoHub loads against a local mock of GitHub (scripts/mock_github.py): a cold start
# (nothing saved), a warm start (the copy saved by the cold start) and a refresh with nothing
# changed, each with the time to first content, the time to complete, and the requests and bytes
# GitHub would have served. Nothing leaves this computer: the app talks to 127.0.0.1 only.
#
#   scripts/bench-load.sh [path/to/repohub] [delay-ms]      # defaults: target/release/repohub, 150
#
# It runs the app headless (`--snap`), with a made-up token, its own data folder and
# REPOHUB_TRACE=1, and reads the state changes the app prints. Times are wall-clock milliseconds
# from the start of the process. "content" is the first time the lists show something (the saved
# copy, the first part of a load, or the whole load); "complete" is the end of the load.
# The refresh is the automatic one (REPOHUB_REFRESH_MINUTES=1), which runs once the first load
# is in and a virtual minute has passed.
set -euo pipefail
cd "$(dirname "$0")/.."
APP="${1:-target/release/repohub}"
APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
DELAY="${2:-150}"
PORT="${BENCH_PORT:-8791}"
WORK="$(mktemp -d)"
trap 'kill "$MOCK" 2>/dev/null || true; rm -rf "$WORK"' EXIT

MOCK_DELAY_MS="$DELAY" python3 scripts/mock_github.py "$PORT" >"$WORK/mock.log" 2>&1 &
MOCK=$!
for _ in $(seq 50); do curl -sf "http://127.0.0.1:$PORT/_stats" >/dev/null && break; sleep 0.1; done

APP="$APP" PORT="$PORT" WORK="$WORK" DELAY="$DELAY" python3 - <<'EOF'
import json, os, subprocess, time, urllib.request

app, port, work = os.environ["APP"], os.environ["PORT"], os.environ["WORK"]
mock = f"http://127.0.0.1:{port}"

def call(path, method="GET"):
    req = urllib.request.Request(mock + path, method=method)
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read() or b"{}")

def run(name, data_dir, loads, extra_env=None):
    """Runs the app until `loads` loads have finished; returns the timings and the mock's counts."""
    call("/_reset", "POST")
    env = {k: v for k, v in os.environ.items() if k not in ("GITHUB_TOKEN", "GITHUB_USER", "REPOHUB_ENV")}
    env.update({
        "GITHUB_API_URL": mock, "GITHUB_TOKEN": "bench-token-not-real", "REPOHUB_DATA_DIR": data_dir,
        "REPOHUB_TRACE": "1", "REPOHUB_MAX_REPOS": "200", "REPOHUB_REFRESH_MINUTES": "1", "HOME": data_dir,
    })
    env.update(extra_env or {})
    t0 = time.monotonic()
    p = subprocess.Popen([app, "--snap", os.path.join(work, f"{name}.png"), "--size", "1180x760", "--scale", "1",
                          "--fps", "10", "--at", "3000"], env=env, stderr=subprocess.PIPE, stdout=subprocess.DEVNULL,
                         text=True, cwd=work)
    content = first_frame = None
    done, phases, snaps, starts, start = [], [], [], [], None
    for line in p.stderr:
        ms = (time.monotonic() - t0) * 1000
        line = line.strip()
        if line.startswith("state: time \"first frame\"") and first_frame is None:
            first_frame = ms
        if line.startswith("state: shown") and content is None:
            content = ms
        if line.startswith("state: status") and line.endswith('-> "loading"'):
            start = ms
            starts.append(call("/_stats"))
        if line.startswith("state: status") and (line.endswith('-> "ready"') or line.endswith('-> "failed"')):
            if content is None:
                content = ms
            done.append(ms)
            phases.append((start, ms))
            # What the load asked (the notifications read after it aren't counted).
            snaps.append(call("/_stats"))
            if len(done) >= loads:
                break
    time.sleep(1.5)  # the saved copy is written after the load
    p.kill()
    p.wait()
    stats = call("/_stats")
    log = call("/_log")
    peak = max((sum(1 for o in log if o["start"] <= e["start"] < o["end"]) for e in log), default=0)
    return {"first_frame": first_frame, "content": content, "done": done, "phases": phases, "stats": stats, "peak": peak, "snaps": snaps, "starts": starts}

def fmt(v):
    return "–" if v is None else f"{v:.0f}"

data = os.path.join(work, "data")
os.makedirs(data)
rows = []
cold = run("cold", data, 1)
rows.append(("cold start", cold["first_frame"], cold["content"], cold["done"][0], cold["snaps"][0], cold["peak"]))
warm = run("warm", data, 1)
rows.append(("warm start", warm["first_frame"], warm["content"], warm["done"][0], warm["snaps"][0], warm["peak"]))
# The refresh: the second load of a session, counted from its start.
both = run("refresh", data, 2)
s1, s2 = both["starts"][-1], both["snaps"][1]
s2 = {k: s2[k] - s1[k] for k in ["requests", "ok", "not_modified", "bytes"]}
a, b = both["phases"][1]
rows.append(("refresh", None, None, (b - a) if a is not None else None, s2, None))

print(f"mock GitHub: {os.environ['DELAY']} ms a request, 200 repositories")
print(f"{'':<11} {'first frame':>11} {'content':>8} {'complete':>9} {'requests':>9} {'200':>5} {'304':>5} {'KB':>7} {'at once':>8}")
for name, ff, c, d, s, peak in rows:
    print(f"{name:<11} {fmt(ff):>11} {fmt(c):>8} {fmt(d):>9} {s['requests']:>9} {s['ok']:>5} {s['not_modified']:>5} {s['bytes'] / 1024:>7.0f} {fmt(peak):>8}")
print("times in ms from the start of the process (refresh: from its start); requests: those of the load")
print("(not the notifications read after it); KB: bodies GitHub sent; at once: most requests in flight")
EOF
