#!/usr/bin/env python3
"""A stand-in for the GitHub REST API on 127.0.0.1, for RepoHub's flows (tests/flows) and for
measuring how it loads (scripts/bench-load.sh). Nothing here reaches the internet.

It answers what RepoHub reads with made-up but stable content, so the same request gets the same
answer and the same ETag in every run:
- generated: the profile (`octo`), MOCK_REPOS repositories (default 200; every fifth belongs to
  the organization `acme`), the searches behind the pull request and issue lists, each
  repository's workflow runs, activity, files, README, languages and branches;
- from the fixtures in tests/mock/fixtures/ (GitHub's own shapes; `{full}`, `{owner}`, `{name}`
  and `{number}` are filled in): a conversation's labels, assignees, milestone, comments, reviews
  and checks, a pull request's files, the history and one commit, releases and tags, the labels,
  people and milestones to pick from.
It takes these writes, checks what they carry and answers as GitHub does: a new issue (201), a
comment, a change to an issue or pull request (PATCH: closing, reopening, title, description,
labels, assignees, milestone), the notes of a release, a new release. What they change is read
back. Anything else, read or write, gets a 404 whose message names the request, and the
request is printed on stderr: a wrong path fails loudly.

- Every answer waits MOCK_DELAY_MS milliseconds first (default 150), like a far-away API;
  searches wait MOCK_SEARCH_DELAY_MS more (default 0), to see the lists arrive one by one.
- MOCK_DOWN=1 answers 503 to everything, to see a failed refresh; POST /_fail?path=P&status=500
  makes the reads of path P (without the query) fail, until POST /_reset.
- GET answers carry an ETag; a request whose If-None-Match matches gets 304 with no body and,
  as on GitHub, doesn't count against the rate limit (x-ratelimit-remaining).
- GET /_stats returns what was asked so far: requests, 200s, 304s, body bytes, conditional
  requests, and conditional requests on /search (which RepoHub must not send). POST /_reset
  zeroes it and forgets the writes. GET /_log returns every request with its method, path,
  status, start and end time (milliseconds since the reset) and, for a write, the JSON body it
  carried. POST /_bump changes one pull request's title, as a change on GitHub would.

Usage: python3 tests/mock/mock_github.py PORT
"""
import hashlib
import json
import os
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlencode, urlparse

LOGIN = "octo"
ORG = "acme"
BASE = 1795176000  # 2026-11-19T00:00:00Z: fixed, so every run answers the same
DELAY = float(os.environ.get("MOCK_DELAY_MS", "150")) / 1000.0
SEARCH_DELAY = float(os.environ.get("MOCK_SEARCH_DELAY_MS", "0")) / 1000.0
DOWN = os.environ.get("MOCK_DOWN") == "1"
REPOS = int(os.environ.get("MOCK_REPOS", "200"))
FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")

LANGS = ["Rust", "TypeScript", "Python", "Go", "JavaScript", "Shell", "C++", "Kotlin"]


def iso(t):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))


def fixture(name, full="", number=0):
    """A fixture file, with the repository and number filled in."""
    owner, _, repo_name = full.partition("/")
    with open(os.path.join(FIXTURES, name), encoding="utf-8") as f:
        text = f.read()
    for k, v in (("{full}", full), ("{owner}", owner), ("{name}", repo_name), ("{number}", str(number))):
        text = text.replace(k, v)
    return json.loads(text)


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
KNOWN = {r["full_name"] for r in ALL_REPOS}
# What the writes changed, until POST /_reset: the issues created (by repository, then number),
# the changes made to issues and pull requests (by repository and number), the releases made.
STATE = {"bump": 0, "issues": {}, "edits": {}, "releases": {}, "next_issue": 1001, "next_release": 9100, "fail": {}}
LOCK = threading.Lock()


def is_pull(n):
    """The generated numbers: pull requests below 200 and from 300 (review requests), issues between."""
    return n < 200 or 300 <= n < 400


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
    if "head:" in q:
        # Comparing two branches asks whether a pull request is open between them: none is.
        return {"total_count": 0, "incomplete_results": False, "items": []}
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
        items = [item("issue", 200 + n, full(n)) for n in range(40)]
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


def conversation(full, n, pulls):
    """Issue or pull request `n`, as one read of it answers: the generated row with the
    fixture's labels, people, milestone and description, or the issue a write created."""
    created = STATE["issues"].get(full, {}).get(n)
    if created is not None:
        return {**created, **STATE["edits"].get((full, n), {})}
    pr = pulls or is_pull(n)
    it = item("pr" if pr else "issue", n, full)
    it.update(fixture("conversation.json", full, n))
    it["comments"] = len(fixture("comments.json", full, n))
    it.update(STATE["edits"].get((full, n), {}))
    if pulls:
        it.update({"merged": False, "mergeable": True, "head": {"ref": "feature/cache", "sha": fixture("commits.json", full)[0]["sha"]},
                   "base": {"ref": "main"}, "requested_reviewers": [user("bob")], "additions": 17, "deletions": 4,
                   "changed_files": len(fixture("pull_files.json", full))})
    return it


def page_of(items, query, path, per_default=30):
    """One page of a list, with GitHub's Link header when there's another."""
    page = int(query.get("page", ["1"])[0])
    per = int(query.get("per_page", [str(per_default)])[0])
    chunk = items[(page - 1) * per: page * per]
    headers = {}
    if page * per < len(items):
        # As GitHub does: the query percent-encoded, so a comma in it can't split the header.
        qs = urlencode([(k, v[0]) for k, v in query.items() if k != "page"] + [("page", page + 1)])
        headers["Link"] = f'<http://mock{path}?{qs}>; rel="next"'
    return chunk, headers


def find_commit(full, sha):
    return next((c for c in fixture("commits.json", full) if c["sha"].startswith(sha.lower())), None)


def route(path, query, accept):
    """(status, body, extra headers) for a GET."""
    p = path.rstrip("/")
    if p in ("/user", f"/users/{LOGIN}"):
        return 200, {**user(LOGIN), "name": "Octo Cat", "bio": "Makes things.", "public_repos": REPOS}, {}
    if p == "/user/starred":
        return 200, [r for r in ALL_REPOS if r["id"] % 6 == 0][:30], {}
    if p in ("/user/repos", f"/users/{LOGIN}/repos", f"/orgs/{ORG}/repos"):
        mine = ALL_REPOS if p == "/user/repos" else [r for r in ALL_REPOS if r["owner"]["login"] == (ORG if "orgs" in p else LOGIN)]
        chunk, headers = page_of(mine, query, p)
        return 200, chunk, headers
    if p == f"/users/{LOGIN}/orgs":
        return 200, [{"login": ORG, "id": 9}], {}
    if p == "/search/issues":
        return 200, search(query.get("q", [""])[0]), {}
    if p == "/notifications":
        return 200, [], {}
    parts = p.split("/")
    if len(parts) < 4 or parts[1] != "repos" or f"{parts[2]}/{parts[3]}" not in KNOWN:
        return None
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
        history = fixture("commits.json", full)
        chunk, headers = page_of(history, query, p)
        return 200, chunk, headers
    if len(rest) == 2 and rest[0] == "commits":
        c = find_commit(full, rest[1])
        if c is None:
            return 422, {"message": f"No commit found for SHA: {rest[1]}"}, {}
        files = fixture("pull_files.json", full)
        adds, dels = sum(f["additions"] for f in files), sum(f["deletions"] for f in files)
        return 200, {**c, "files": files, "stats": {"total": adds + dels, "additions": adds, "deletions": dels}}, {}
    if len(rest) == 3 and rest[0] == "commits" and rest[2] == "check-runs":
        return 200, fixture("check_runs.json", full), {}
    if len(rest) >= 2 and rest[0] == "compare" and "..." in "/".join(rest[1:]):
        base, head = unquote("/".join(rest[1:])).split("...", 1)
        history = fixture("commits.json", full)
        files = fixture("pull_files.json", full)
        return 200, {"status": "ahead", "ahead_by": 2, "behind_by": 0, "total_commits": 2,
                     "base_commit": history[2], "merge_base_commit": history[2],
                     "commits": list(reversed(history[:2])), "files": files,
                     "html_url": f"https://github.com/{full}/compare/{base}...{head}"}, {}
    if rest == ["branches"]:
        return 200, [{"name": n, "commit": {"sha": "d" * 40}, "protected": n == "main"} for n in ["main", "dev", "feature/cache"]], {}
    if rest[:3] == ["git", "ref", "heads"] and len(rest) > 3:
        return 200, {"ref": "refs/heads/" + "/".join(rest[3:]), "object": {"type": "commit", "sha": fixture("commits.json", full)[0]["sha"]}}, {}
    if rest == ["labels"]:
        return 200, fixture("labels.json", full), {}
    if rest == ["assignees"]:
        return 200, fixture("assignees.json", full), {}
    if rest == ["milestones"]:
        return 200, fixture("milestones.json", full), {}
    if rest == ["releases"]:
        made = list(STATE["releases"].get(full, {}).values())
        chunk, headers = page_of(made + fixture("releases.json", full), query, p)
        return 200, chunk, headers
    if rest == ["releases", "latest"]:
        return 200, next(r for r in fixture("releases.json", full) if not r["draft"] and not r["prerelease"]), {}
    if rest == ["tags"]:
        chunk, headers = page_of(fixture("tags.json", full), query, p)
        return 200, chunk, headers
    if len(rest) >= 2 and rest[0] in ("issues", "pulls") and rest[1].isdigit():
        n = int(rest[1])
        if len(rest) == 2:
            if rest[0] == "pulls" and not is_pull(n):
                return None
            return 200, conversation(full, n, rest[0] == "pulls"), {}
        if rest[2:] == ["comments"] and rest[0] == "issues":
            return 200, fixture("comments.json", full, n), {}
        if rest[2:] == ["reviews"] and rest[0] == "pulls" and is_pull(n):
            return 200, fixture("reviews.json", full, n), {}
        if rest[2:] == ["comments"] and rest[0] == "pulls" and is_pull(n):
            return 200, [], {}  # no comments on lines
        if rest[2:] == ["files"] and rest[0] == "pulls" and is_pull(n):
            chunk, headers = page_of(fixture("pull_files.json", full, n), query, p)
            return 200, chunk, headers
    return None


def edit(full, n, body):
    """PATCH on an issue or pull request: its state, title, description, labels, assignees or
    milestone; answers the whole item, as GitHub does."""
    change = {}
    for k, v in body.items():
        if k == "state" and v in ("open", "closed"):
            change["state"] = v
        elif k == "state_reason":
            pass
        elif k in ("title", "body") and isinstance(v, str) and (v.strip() or k == "body"):
            change[k] = v
        elif k == "labels" and isinstance(v, list) and all(isinstance(x, str) for x in v):
            known = {l["name"]: l for l in fixture("labels.json", full)}
            change["labels"] = [known.get(x, {"id": 0, "name": x, "color": "ededed", "default": False, "description": ""}) for x in v]
        elif k == "assignees" and isinstance(v, list) and all(isinstance(x, str) for x in v):
            change["assignees"] = [user(x) for x in v]
        elif k == "milestone" and (v is None or isinstance(v, int)):
            found = [m for m in fixture("milestones.json", full) if m["number"] == v]
            if v is not None and not found:
                return 422, {"message": "Validation Failed", "errors": [{"resource": "Issue", "field": "milestone", "code": "invalid"}]}
            change["milestone"] = found[0] if found else None
        else:
            return 422, {"message": "Validation Failed", "errors": [{"resource": "Issue", "field": k, "code": "invalid"}]}
    STATE["edits"].setdefault((full, n), {}).update(change)
    return 200, conversation(full, n, False)


def write(method, path, body):
    """(status, answer) for a write, or None when the mock doesn't take it."""
    parts = path.rstrip("/").split("/")
    if len(parts) < 5 or parts[1] != "repos" or f"{parts[2]}/{parts[3]}" not in KNOWN:
        return None
    full, rest = f"{parts[2]}/{parts[3]}", parts[4:]
    if not isinstance(body, dict):
        return 400, {"message": "Problems parsing JSON"}
    with LOCK:
        if method == "POST" and rest == ["issues"]:
            title = body.get("title")
            if not isinstance(title, str) or not title.strip():
                return 422, {"message": "Validation Failed", "errors": [{"resource": "Issue", "field": "title", "code": "missing_field"}]}
            n = STATE["next_issue"]
            STATE["next_issue"] += 1
            it = item("issue", n, full)
            it.update({"title": title, "body": body.get("body") or None, "user": user(LOGIN), "comments": 0,
                       "created_at": iso(time.time()), "updated_at": iso(time.time()),
                       "labels": [{"id": 0, "name": l, "color": "ededed", "default": False, "description": ""} for l in body.get("labels", [])],
                       "assignees": [user(a) for a in body.get("assignees", [])]})
            STATE["issues"].setdefault(full, {})[n] = it
            return 201, it
        if method == "POST" and len(rest) == 3 and rest[0] == "issues" and rest[1].isdigit() and rest[2] == "comments":
            if not str(body.get("body", "")).strip():
                return 422, {"message": "Validation Failed", "errors": [{"resource": "IssueComment", "field": "body", "code": "missing_field"}]}
            return 201, {"id": 8800, "user": user(LOGIN), "body": body["body"], "created_at": iso(time.time()),
                         "html_url": f"https://github.com/{full}/issues/{rest[1]}#issuecomment-8800"}
        if method == "PATCH" and len(rest) == 2 and rest[0] == "issues" and rest[1].isdigit():
            return edit(full, int(rest[1]), body)
        if method == "POST" and rest == ["releases", "generate-notes"]:
            tag = body.get("tag_name")
            if not tag:
                return 422, {"message": "Validation Failed"}
            return 200, {"name": tag, "body": f"## What's Changed\n* Keep answers for five minutes by @octo\n* Count the answers read from the cache by @alice\n\n**Full Changelog**: https://github.com/{full}/compare/v1.2.0...{tag}"}
        if method == "POST" and rest == ["releases"]:
            if not body.get("tag_name"):
                return 422, {"message": "Validation Failed"}
            rid = STATE["next_release"]
            STATE["next_release"] += 1
            rel = {"id": rid, "node_id": f"RE_{rid}", "tag_name": body["tag_name"], "target_commitish": body.get("target_commitish", "main"),
                   "name": body.get("name", ""), "body": body.get("body", ""), "draft": bool(body.get("draft")),
                   "prerelease": bool(body.get("prerelease")), "created_at": iso(time.time()),
                   "published_at": None if body.get("draft") else iso(time.time()), "author": user(LOGIN),
                   "html_url": f"https://github.com/{full}/releases/tag/{body['tag_name']}", "assets": []}
            STATE["releases"].setdefault(full, {})[rid] = rel
            return 201, rel
    return None


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


def not_found(method, path):
    print(f"mock: 404 {method} {path} (no answer for it in tests/mock)", file=sys.stderr, flush=True)
    return 404, {"message": "Not Found", "mock": f"tests/mock has no answer for {method} {path}",
                 "documentation_url": "https://docs.github.com/rest"}


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
            with LOCK:
                STATE.update({"bump": 0, "issues": {}, "edits": {}, "releases": {}, "next_issue": 1001, "next_release": 9100, "fail": {}})
            self.send(200, b"{}")
        elif u.path == "/_bump":
            STATE["bump"] += 1
            self.send(200, b"{}")
        elif u.path == "/_fail":
            # POST /_fail?path=/repos/o/r/readme&status=500: that path (without its query) fails.
            q = parse_qs(u.query)
            with LOCK:
                STATE["fail"][q["path"][0]] = int(q.get("status", ["500"])[0])
            self.send(200, b"{}")
        else:
            return False
        return True

    def record(self, start, status, body=None):
        entry = {"method": self.command, "path": self.path, "start": round((start - STATS.t0) * 1000),
                 "end": round((time.monotonic() - STATS.t0) * 1000), "status": status}
        if body is not None:
            entry["body"] = body
        with STATS.lock:
            STATS.log.append(entry)

    def do_GET(self):
        if self.path.startswith("/_") and self.control():
            return
        start = time.monotonic()
        u = urlparse(self.path)
        time.sleep(DELAY + (SEARCH_DELAY if u.path.startswith("/search/") else 0))
        query = parse_qs(u.query)
        found = route(u.path, query, self.headers.get("Accept", ""))
        status, body, extra = found if found is not None else (*not_found("GET", self.path), {})
        if DOWN:
            status, body, extra = 503, {"message": "Service unavailable"}, {}
        elif u.path in STATE["fail"]:
            status, body, extra = STATE["fail"][u.path], {"message": "Server Error"}, {}
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
        self.record(start, 304 if hit else status)

    def do_POST(self):
        if self.path.startswith("/_") and self.control():
            return
        start = time.monotonic()
        time.sleep(DELAY)
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw) if raw.strip() else {}
        except ValueError:
            body = raw.decode("utf-8", "replace")
        answer = write(self.command, urlparse(self.path).path, body)
        status, out = answer if answer is not None else not_found(self.command, self.path)
        if DOWN:
            status, out = 503, {"message": "Service unavailable"}
        with STATS.lock:
            STATS.requests += 1
        self.send(status, json.dumps(out, separators=(",", ":")).encode())
        self.record(start, status, body)

    do_PATCH = do_PUT = do_DELETE = do_POST


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    srv.daemon_threads = True
    print(f"mock GitHub on http://127.0.0.1:{port} ({REPOS} repositories, {DELAY * 1000:.0f} ms a request)", flush=True)
    srv.serve_forever()
