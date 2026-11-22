"""A temporary clone of the mock's acme/project-000 for the Changes flows: one commit, then a
changed file and a new one. RepoHub takes the folder as the clone because a remote points at the
repository; REPOHUB_REPOS_DIR is where it looks."""
import os
import subprocess

REPO = "acme/project-000"


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True, text=True).stdout


def make(run):
    repo = os.path.join(run.work, "clones", "project-000")
    os.makedirs(os.path.join(repo, "src"))
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.name", "Flow Test")
    git(repo, "config", "user.email", "flows@example.com")
    git(repo, "config", "commit.gpgsign", "false")
    git(repo, "remote", "add", "origin", f"https://github.com/{REPO}.git")
    with open(os.path.join(repo, "src", "cache.rs"), "w") as f:
        f.write("/// How long an answer stays fresh.\nconst FRESH: u64 = 60;\n")
    with open(os.path.join(repo, "README.md"), "w") as f:
        f.write("# project-000\n\nA tool that does one thing well.\n")
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "First version")
    with open(os.path.join(repo, "src", "cache.rs"), "w") as f:
        f.write("/// How long an answer stays fresh.\nconst FRESH: u64 = 300;\n")
    with open(os.path.join(repo, "src", "status.rs"), "w") as f:
        f.write("pub fn line(hits: usize) -> String {\n    format!(\"{hits} answers from the cache\")\n}\n")
    run.env["REPOHUB_REPOS_DIR"] = os.path.join(run.work, "clones")
    run.repo = repo
