#!/usr/bin/env python3
"""A small stand-in for the GitHub REST API, for measuring how RepoHub loads (scripts/bench-load.sh).

It answers the requests RepoHub makes for its lists (the profile, 200 repositories, pull requests,
issues, workflow runs, commit activity) and for a repository's page, with made-up but stable
content, so the same request gets the same answer and the same ETag in every run.

- Every answer waits MOCK_DELAY_MS milliseconds first (default 150), like a far-away API;
  searches wait MOCK_SEARCH_DELAY_MS more (default 0), to see the lists arrive one by one.
- MOCK_DOWN=1 answers 503 to everything, to see a failed refresh.
- GET answers carry an ETag; a request whose If-None-Match matches gets 304 with no body and,
  as on GitHub, doesn't count against the rate limit (x-ratelimit-remaining).
- GET /_stats returns what was asked so far: requests, 200s, 304s, body bytes, conditional
  requests, and conditional requests on /search (which RepoHub must not send). POST /_reset
  zeroes it. GET /_log returns every request with its start and end time (milliseconds since
  the reset), to see how many ran at once. POST /_bump changes one issue's title, as a change
  on GitHub would.

Usage: python3 scripts/mock_github.py PORT
Local only: it listens on 127.0.0.1.
"""
import hashlib
import json
import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse

LOGIN = "octo"
ORG = "acme"
BASE = 1795176000  # 2026-11-19T00:00:00Z: fixed, so every run answers the same
DELAY = float(os.environ.get("MOCK_DELAY_MS", "150")) / 1000.0
SEARCH_DELAY = float(os.environ.get("MOCK_SEARCH_DELAY_MS", "0")) / 1000.0
DOWN = os.environ.get("MOCK_DOWN") == "1"
REPOS = int(os.environ.get("MOCK_REPOS", "200"))

LANGS = ["Rust", "TypeScript", "Python", "Go", "JavaScript", "Shell", "C++", "Kotlin"]


def iso(t):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))


def user(login):
    return {
        "login": login, "id": sum(map(ord, login)), "node_id": "MDQ6VXNlcjE=",
        "avatar_url": f"https://avatars.example/{login}", "gravatar_id": "",
        "url": f"https://api.github.com/users/{login}", "html_url": f"https://github.com/{login}",
        "followers_url": f"https://api.github.com/users/{login}/followers",
        "following_url": f"https://api.github.com/users/{login}/following{{/other_user}}",
        "gists_url": f"https://api.github.com/users/{login}/gists{{/gist_id}}",
        "starred_url": f"https://api.github.com/users/{login}/starred{{/owner}}{{/repo}}",
        "subscriptions_url": f"https://api.github.com/users/{login}/subscriptions",
        "organizations_url": f"https://api.github.com/users/{login}/orgs",
        "repos_url": f"https://api.github.com/users/{login}/repos",
        "events_url": f"https://api.github.com/users/{login}/events{{/privacy}}",
        "received_events_url": f"https://api.github.com/users/{login}/received_events",
        "type": "User", "site_admin": False,
    }


def repo(i):
    owner = LOGIN if i % 5 else ORG
    name = f"project-{i:03d}"
    full = f"{owner}/{name}"
    api = f"https://api.github.com/repos/{full}"
    r = {
        "id": 1000 + i, "node_id": f"R_{i}", "name": name, "full_name": full, "private": i % 7 == 0,
        "owner": user(owner), "html_url": f"https://github.com/{full}",
        "description": f"Project number {i}: a tool that does one thing well, with tests and docs.",
        "fork": i % 11 == 0, "url": api,
    }
    for k in ["forks", "keys", "collaborators", "teams", "hooks", "issue_events", "events", "assignees", "branches",
              "tags", "blobs", "git_tags", "git_refs", "trees", "statuses", "languages", "stargazers", "contributors",
              "subscribers", "subscription", "commits", "git_commits", "comments", "issue_comment", "contents",
              "compare", "merges", "archive", "downloads", "issues", "pulls", "milestones", "notifications", "labels",
              "releases", "deployments"]:
        r[f"{k}_url"] = f"{api}/{k}"
    r.update({
        "created_at": iso(BASE - 400 * 86400 + i * 3600), "updated_at": iso(BASE - i * 5000),
        "pushed_at": iso(BASE - i * 7200), "git_url": f"git://github.com/{full}.git",
        "ssh_url": f"git@github.com:{full}.git", "clone_url": f"https://github.com/{full}.git",
        "svn_url": f"https://github.com/{full}", "homepage": None, "size": 1234 + i,
        "stargazers_count": (i * 37) % 300, "watchers_count": (i * 37) % 300, "language": LANGS[i % len(LANGS)],
        "has_issues": True, "has_projects": True, "has_downloads": True, "has_wiki": False, "has_pages": False,
        "has_discussions": False, "forks_count": (i * 7) % 40, "mirror_url": None, "archived": False,
        "disabled": False, "open_issues_count": i % 9, "license": None, "allow_forking": True,
        "is_template": False, "web_commit_signoff_required": False, "topics": ["cli", LANGS[i % len(LANGS)].lower()],
        "visibility": "private" if i % 7 == 0 else "public", "forks": (i * 7) % 40, "open_issues": i % 9,
        "watchers": (i * 37) % 300, "default_branch": "main",
        "permissions": {"admin": True, "maintain": True, "push": True, "triage": True, "pull": True},
    })
    return r


ALL_REPOS = [repo(i) for i in range(REPOS)]
STATE = {"bump": 0}


def item(kind, n, repo_full, state="open", draft=False):
    r = {
        "url": f"https://api.github.com/repos/{repo_full}/issues/{n}",
        "repository_url": f"https://api.github.com/repos/{repo_full}",
        "labels_url": f"https://api.github.com/repos/{repo_full}/issues/{n}/labels{{/name}}",
        "comments_url": f"https://api.github.com/repos/{repo_full}/issues/{n}/comments",
        "html_url": f"https://github.com/{repo_full}/{'pull' if kind == 'pr' else 'issues'}/{n}",
        "id": 50000 + n, "node_id": f"I_{n}", "number": n,
        "title": f"{'Change' if kind == 'pr' else 'Problem'} number {n}" + (f" (edited {STATE['bump']})" if n == 1 and STATE["bump"] else ""),
        "user": user("alice" if n % 2 else LOGIN),
        "labels": [{"id": 1, "name": "bug", "color": "d73a4a", "default": True, "description": "Something isn't working"}] if n % 3 == 0 else [],
        "state": state, "locked": False, "assignee": None, "assignees": [user(LOGIN)] if n % 4 == 0 else [],
        "milestone": None, "comments": n % 6, "created_at": iso(BASE - n * 4000), "updated_at": iso(BASE - n * 3000),
        "closed_at": None if state == "open" else iso(BASE - n * 2000), "author_association": "OWNER",
        "draft": draft, "body": f"Body of number {n}. " * 8, "reactions": {"total_count": 0}, "state_reason": None,
        "score": 1.0,
    }
    if kind == "pr":
        r["pull_request"] = {"url": f"https://api.github.com/repos/{repo_full}/pulls/{n}",
                             "html_url": f"https://github.com/{repo_full}/pull/{n}",
                             "merged_at": iso(BASE - n * 1000) if state == "closed" and n % 2 else None}
    return r


def search(q):
    full = lambda n: ALL_REPOS[n % len(ALL_REPOS)]["full_name"]
    if "is:pr" in q and "review-requested:" in q:
        items = [item("pr", 300 + n, full(n), draft=False) for n in range(3)]
        total = 3
    elif "is:pr" in q and "is:open" in q:
        items = [item("pr", n + 1, full(n), draft=n % 5 == 0) for n in range(25)]
        total = 25
    elif "is:pr" in q:
        items = [item("pr", 100 + n, full(n), state="closed") for n in range(40)]
        total = 120
    else:
        items = [item("issue", 200 + n if n else 1, full(n)) for n in range(40)]
        total = 85
    return {"total_count": total, "incomplete_results": False, "items": items}


def runs(full):
    idx = next((i for i, r in enumerate(ALL_REPOS) if r["full_name"] == full), 0)
    out = []
    for k in range(4):
        rid = 900000 + idx * 10 + k
        out.append({
            "id": rid, "name": ["CI", "Release", "Lint", "Docs"][k], "node_id": f"WR_{rid}", "head_branch": "main",
            "head_sha": "a" * 40, "path": ".github/workflows/ci.yml", "run_number": 100 + k, "event": "push",
            "status": "completed", "conclusion": "success" if (idx + k) % 4 else "failure", "workflow_id": 7,
            "url": f"https://api.github.com/repos/{full}/actions/runs/{rid}",
            "html_url": f"https://github.com/{full}/actions/runs/{rid}",
            "pull_requests": [{"number": (idx + k) % 25 + 1}] if k == 0 else [],
            "created_at": iso(BASE - idx * 900 - k * 60), "updated_at": iso(BASE - idx * 900 - k * 60 + 45),
            "run_started_at": iso(BASE - idx * 900 - k * 60), "actor": user(LOGIN), "run_attempt": 1,
        })
    return {"total_count": 4, "workflow_runs": out}


def route(path, query, accept):
    """(status, body, extra headers) for a GET."""
    p = path.rstrip("/")
    page = int(query.get("page", ["1"])[0])
    per = int(query.get("per_page", ["30"])[0])
    if p in ("/user", f"/users/{LOGIN}"):
        return 200, {**user(LOGIN), "name": "Octo Cat", "bio": "Makes things.", "public_repos": REPOS}, {}
    if p == "/user/starred":
        return 200, [r for r in ALL_REPOS if r["id"] % 6 == 0][:30], {}
    if p in ("/user/repos", f"/users/{LOGIN}/repos", f"/orgs/{ORG}/repos"):
        mine = ALL_REPOS if p == "/user/repos" else [r for r in ALL_REPOS if r["owner"]["login"] == (ORG if "orgs" in p else LOGIN)]
        chunk = mine[(page - 1) * per: page * per]
        headers = {}
        if page * per < len(mine):
            # As GitHub does: the query percent-encoded, so a comma in it can't split the header.
            qs = urlencode([(k, v[0]) for k, v in query.items() if k != "page"] + [("page", page + 1)])
            headers["Link"] = f'<http://mock{p}?{qs}>; rel="next"'
        return 200, chunk, headers
    if p == f"/users/{LOGIN}/orgs":
        return 200, [{"login": ORG, "id": 9}], {}
    if p == "/search/issues":
        return 200, search(query.get("q", [""])[0]), {}
    if p == "/notifications":
        return 200, [], {}
    parts = p.split("/")
    if len(parts) >= 4 and parts[1] == "repos":
        full = f"{parts[2]}/{parts[3]}"
        rest = parts[4:]
        if rest == ["actions", "runs"]:
            return 200, runs(full), {}
        if rest == ["stats", "participation"]:
            seed = sum(map(ord, full))
            weeks = [(seed + w * 3) % 7 for w in range(52)]
            return 200, {"all": [w + 1 for w in weeks], "owner": weeks}, {}
        if rest == ["contents"]:
            return 200, [{"name": n, "type": t, "path": n, "sha": "b" * 40, "size": 10} for n, t in
                         [("src", "dir"), ("docs", "dir"), ("README.md", "file"), ("Cargo.toml", "file")]], {}
        if rest == ["readme"]:
            return 200, f"# {full}\n\n" + "Some words about the project. " * 40, {"raw": True}
        if rest == ["languages"]:
            return 200, {"Rust": 12000, "Shell": 300}, {}
        if rest == ["commits"]:
            return 200, [{"sha": "c" * 40, "commit": {"message": "Make it faster\n\nDetails", "author": {"name": "Octo", "date": iso(BASE - 600)}}, "author": user(LOGIN)}], {}
        if rest == ["branches"]:
            return 200, [{"name": n, "commit": {"sha": "d" * 40}, "protected": False} for n in ["main", "dev", "feature/x"]], {}
        if rest in (["labels"], ["assignees"], ["milestones"]):
            return 200, [{"name": "bug", "color": "d73a4a", "login": LOGIN, "number": 1, "title": "v1"}], {}
        if len(rest) >= 2 and rest[0] in ("issues", "pulls") and rest[1].isdigit():
            n = int(rest[1])
            if len(rest) == 2:
                it = item("pr" if rest[0] == "pulls" or n < 100 else "issue", n, full)
                if rest[0] == "pulls":
                    it.update({"merged": False, "head": {"ref": "feature/x", "sha": "e" * 40}, "base": {"ref": "main"}, "requested_reviewers": []})
                return 200, it, {}
            if rest[2] in ("comments", "reviews"):
                return 200, [{"id": k, "user": user("alice"), "body": f"Comment {k}", "created_at": iso(BASE - k * 60), "state": "COMMENTED", "submitted_at": iso(BASE - k * 60)} for k in range(3)], {}
        if len(rest) == 3 and rest[0] == "commits" and rest[2] == "check-runs":
            return 200, {"total_count": 1, "check_runs": [{"name": "test", "status": "completed", "conclusion": "success", "html_url": "https://example/1"}]}, {}
    return 404, {"message": "Not Found"}, {}


class Stats:
    def __init__(self):
        self.lock = threading.Lock()
        self.reset()

    def reset(self):
        self.t0 = time.monotonic()
        self.requests = self.ok = self.not_modified = self.bytes = self.conditional = self.search_conditional = 0
        self.remaining = 5000
        self.log = []


STATS = Stats()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send(self, status, body=b"", headers=None, ctype="application/json; charset=utf-8"):
        self.send_response(status)
        if status != 304:
            self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if body and self.command != "HEAD":
            self.wfile.write(body)

    def control(self):
        u = urlparse(self.path)
        if u.path == "/_stats":
            with STATS.lock:
                s = {k: getattr(STATS, k) for k in ["requests", "ok", "not_modified", "bytes", "conditional", "search_conditional", "remaining"]}
            self.send(200, json.dumps(s).encode())
        elif u.path == "/_log":
            with STATS.lock:
                self.send(200, json.dumps(STATS.log).encode())
        elif u.path == "/_reset":
            with STATS.lock:
                STATS.reset()
            self.send(200, b"{}")
        elif u.path == "/_bump":
            STATE["bump"] += 1
            self.send(200, b"{}")
        else:
            return False
        return True

    def do_GET(self):
        if self.path.startswith("/_") and self.control():
            return
        start = time.monotonic()
        u = urlparse(self.path)
        time.sleep(DELAY + (SEARCH_DELAY if u.path.startswith("/search/") else 0))
        query = parse_qs(u.query)
        status, body, extra = route(u.path, query, self.headers.get("Accept", ""))
        if DOWN:
            status, body, extra = 503, {"message": "Service unavailable"}, {}
        raw = extra.pop("raw", False)
        data = (body if raw else json.dumps(body, separators=(",", ":"))).encode()
        etag = 'W/"%s"' % hashlib.sha1(data).hexdigest()
        inm = self.headers.get("If-None-Match")
        headers = dict(extra)
        with STATS.lock:
            STATS.requests += 1
            if inm:
                STATS.conditional += 1
                if u.path.startswith("/search/"):
                    STATS.search_conditional += 1
            hit = status == 200 and inm is not None and etag in [e.strip() for e in inm.split(",")]
            if hit:
                STATS.not_modified += 1
            elif status == 200:
                STATS.ok += 1
                STATS.remaining -= 1
                STATS.bytes += len(data)
            headers.update({"x-ratelimit-remaining": str(STATS.remaining), "x-ratelimit-limit": "5000",
                            "x-ratelimit-reset": str(int(time.time()) + 3600)})
        if status == 200:
            headers["ETag"] = etag
            headers["Cache-Control"] = "private, max-age=60, s-maxage=60"
            headers["Vary"] = "Accept, Authorization, Cookie"
        if hit:
            self.send(304, b"", headers)
        else:
            self.send(status, data, headers, "text/plain; charset=utf-8" if raw else "application/json; charset=utf-8")
        end = time.monotonic()
        with STATS.lock:
            STATS.log.append({"path": self.path, "start": round((start - STATS.t0) * 1000), "end": round((end - STATS.t0) * 1000), "status": 304 if hit else status})

    def do_POST(self):
        if self.path.startswith("/_") and self.control():
            return
        self.send(404, b'{"message":"Not Found"}')

    do_PATCH = do_PUT = do_DELETE = do_POST


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    srv.daemon_threads = True
    print(f"mock GitHub on http://127.0.0.1:{port} ({REPOS} repositories, {DELAY * 1000:.0f} ms a request)", flush=True)
    srv.serve_forever()
