#!/usr/bin/env python3
"""A stand-in for `gh`, for the pull-request clip (prs.tape) only.

record.sh puts this on the daemon's PATH as `gh`, so the board reads a
"forge" that exists only in one JSON file, and nothing of the network or of
a sign-in on this machine is touched. It answers the calls the daemon makes
(daemon/src/forge.rs): `auth status`, `pr list`, `pr view`, and `api` reads
of a commit's check runs and status, a pull request's commits, and an
environment's deployments. The stand-in agent (stub-claude.py) runs `gh pr
create` through it as a real agent would, and `gh __push` to move a PR's
head after a fix, as a `git push` would.

Every PR runs on a clock of its own, from the moment it was opened or last
pushed: its pipeline runs `run_s` seconds, then passes (or fails on the job
`fails` names); a passed PR is merged `merge_after` seconds later, as a
teammate would merge it on the forge, with a merge commit made in the
repository so the board can read what each deployment carries; and the
merge reaches each environment in `deploy` the seconds it names after the
merge. seed.py writes the first state (`gh __seed`), where each PR stands
`ago` seconds after its push; those clocks hold still until the take opens
(the DEMO_GO file kitty-term.py makes), then run, so the board the
recording opens on is the one the seed described.
"""

import fcntl
import json
import os
import subprocess
import sys
import time

DIR = os.environ.get("MSMN_FORGE", "")
STATE = os.path.join(DIR, "state.json")
GO = os.environ.get("DEMO_GO", "")
OWNER = "demo/shortlink"
HOST = "https://github.com"
# The environments a merge reaches, in the pipeline's order.
ENVS = ["staging", "production"]


def iso(epoch):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(epoch))


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True,
                          check=True).stdout.strip()


class State:
    """The forge's one file, read and written under a lock: the daemon
    reads while an agent opens or pushes."""

    def __enter__(self):
        self.lock = open(STATE + ".lock", "w")
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        try:
            with open(STATE) as f:
                self.data = json.load(f)
        except FileNotFoundError:
            self.data = {"repo": "", "prs": []}
        self.dirty = False
        return self

    def __exit__(self, *_):
        if self.dirty:
            tmp = STATE + ".tmp"
            with open(tmp, "w") as f:
                json.dump(self.data, f, indent=1)
            os.replace(tmp, STATE)
        fcntl.flock(self.lock, fcntl.LOCK_UN)
        self.lock.close()

    @property
    def repo(self):
        return self.data["repo"]

    @property
    def prs(self):
        return self.data["prs"]

    def settle(self, now):
        """Merge what the clock says is merged: a merge commit on top of
        origin/main, carrying the PR's head, named on the PR. In the order
        the merges happened, so an older merge sits under a newer one and
        a deploy of the newer carries it, never the other way round."""
        due = []
        for pr in self.prs:
            if pr.get("merged") or pr.get("merge_after") is None or pr.get("fails"):
                continue
            at = pushed(pr, now) + pr["run_s"] + pr["merge_after"]
            if now >= at:
                due.append((at, pr))
        for at, pr in sorted(due, key=lambda d: d[0]):
            base = git(self.repo, "rev-parse", "refs/remotes/origin/main")
            tree = git(self.repo, "rev-parse", f"{pr['tip']}^{{tree}}")
            sha = git(self.repo, "commit-tree", tree, "-p", base, "-p", pr["tip"], "-m",
                      f"Merge pull request #{pr['number']} from {OWNER.split('/')[0]}/"
                      f"{pr['branch']}\n\n{pr['title']}")
            git(self.repo, "update-ref", "refs/remotes/origin/main", sha)
            pr["merged"] = at
            pr["merge"] = sha
            self.dirty = True


def pushed(pr, now):
    """When the PR's head was pushed: a seeded PR's `ago` counts from the
    take's start, and stands still until then."""
    if "pushed" in pr:
        return pr["pushed"]
    try:
        start = os.stat(GO).st_mtime
    except OSError:
        start = now
    return start - pr["ago"]


def pipeline(pr, now):
    """The head's check runs, as `statusCheckRollup` lists them."""
    done = now >= pushed(pr, now) + pr["run_s"]
    runs = []
    for job in ("tests", "lint"):
        if not done:
            runs.append({"__typename": "CheckRun", "name": job, "status": "IN_PROGRESS",
                         "conclusion": None})
        elif pr.get("fails") == job:
            runs.append({"__typename": "CheckRun", "name": job, "status": "COMPLETED",
                         "conclusion": "FAILURE"})
        else:
            runs.append({"__typename": "CheckRun", "name": job, "status": "COMPLETED",
                         "conclusion": "SUCCESS"})
    return runs


def pr_json(pr, now):
    merged = pr.get("merged")
    return {
        "number": pr["number"],
        "title": pr["title"],
        "url": f"{HOST}/{OWNER}/pull/{pr['number']}",
        "state": "MERGED" if merged else "OPEN",
        "isDraft": False,
        "headRefName": pr["branch"],
        "headRefOid": pr["tip"],
        "mergeCommit": {"oid": pr["merge"]} if merged else None,
        "mergedAt": iso(merged) if merged else None,
        "mergeable": "MERGEABLE",
        "reviewDecision": pr.get("review", "REVIEW_REQUIRED"),
        "updatedAt": iso(max(pushed(pr, now), merged or 0)),
        "statusCheckRollup": pipeline(pr, now),
    }


def deployments(st, env, now):
    """The environment's deployments, newest first, as GitHub lists them:
    the board takes the newest and reads every merge it carries."""
    out = []
    for pr in st.prs:
        at = pr.get("merged")
        wait = (pr.get("deploy") or {}).get(env)
        if at and wait is not None and now >= at + wait:
            out.append({"id": pr["number"] * 10 + ENVS.index(env), "environment": env,
                        "sha": pr["merge"], "created_at": iso(at + wait), "_at": at + wait})
    out.sort(key=lambda d: (-d["_at"], -d["id"]))
    for d in out:
        del d["_at"]
    return out


def deployed_at(st, dep_id):
    """When the deployment `dep_id` names succeeded: its PR's merge plus
    its environment's wait."""
    number, env = divmod(dep_id, 10)
    pr = next((p for p in st.prs if p["number"] == number), None)
    if pr and pr.get("merged"):
        return pr["merged"] + pr["deploy"][ENVS[env]]
    return 0


def create(st, args, now):
    """`gh pr create` from inside a ticket's worktree: the PR is on that
    branch, at its head, and runs its pipeline from now. MSMN_FORGE_FAILS
    names the job this PR's first run fails on."""
    title = args[args.index("--title") + 1] if "--title" in args else "Untitled"
    branch = git(".", "rev-parse", "--abbrev-ref", "HEAD")
    tip = git(".", "rev-parse", "HEAD")
    number = max([p["number"] for p in st.prs], default=0) + 1
    st.prs.append({"number": number, "title": title, "branch": branch, "tip": tip,
                   "pushed": now, "run_s": 7, "fails": os.environ.get("MSMN_FORGE_FAILS") or None,
                   "review": "APPROVED", "merge_after": 6,
                   "deploy": {"staging": 5, "production": 11}})
    st.dirty = True
    print(f"{HOST}/{OWNER}/pull/{number}")


def push(st, now):
    """`gh __push`, what a `git push` after a fix does to the PR on this
    branch: a new head, and a pipeline run from now that passes."""
    branch = git(".", "rev-parse", "--abbrev-ref", "HEAD")
    tip = git(".", "rev-parse", "HEAD")
    for pr in st.prs:
        if pr["branch"] == branch and not pr.get("merged"):
            pr.update(tip=tip, pushed=now, fails=None)
            st.dirty = True


def seed(st, now):
    """`gh __seed`, from seed.py: the PRs the board opens with, as JSON on
    stdin, each with `ago` seconds since its push once the take opens. A
    tip commit is made for each on the repository's main tree, and
    origin/main starts at main."""
    spec = json.load(sys.stdin)
    st.data["repo"] = spec["repo"]
    git(st.repo, "update-ref", "refs/remotes/origin/main", "main")
    for pr in spec["prs"]:
        tree = git(st.repo, "rev-parse", "HEAD^{tree}")
        tip = git(st.repo, "commit-tree", tree, "-p", "main", "-m", pr["title"])
        git(st.repo, "update-ref", f"refs/forge/pr-{pr['number']}", tip)
        st.prs.append({**pr, "tip": tip})
    st.dirty = True


def main(argv):
    now = time.time()
    if argv[:2] == ["auth", "status"]:
        return 0
    with State() as st:
        st.settle(now)
        if argv[:1] == ["__seed"]:
            seed(st, now)
        elif argv[:1] == ["__push"]:
            push(st, now)
        elif argv[:2] == ["pr", "create"]:
            create(st, argv, now)
        elif argv[:2] == ["pr", "list"]:
            print(json.dumps([pr_json(p, now) for p in st.prs]))
        elif argv[:2] == ["pr", "view"]:
            n = int(argv[2])
            pr = next((p for p in st.prs if p["number"] == n), None)
            if pr is None:
                print(f"GraphQL: Could not resolve to a PullRequest with the number of {n}.",
                      file=sys.stderr)
                return 1
            print(json.dumps(pr_json(pr, now)))
        elif argv[:1] == ["api"]:
            path = argv[-1]
            if "/check-runs" in path:
                print(json.dumps({"check_runs": [
                    {"name": "tests", "status": "completed", "conclusion": "success"}]}))
            elif path.endswith("/status"):
                print(json.dumps({"state": "success", "statuses": []}))
            elif "/pulls/" in path and "/commits" in path:
                n = int(path.split("/pulls/")[1].split("/")[0])
                pr = next((p for p in st.prs if p["number"] == n), None)
                print(json.dumps([{"sha": pr["tip"]}] if pr else []))
            elif "/deployments/" in path and "/statuses" in path:
                dep = int(path.split("/deployments/")[1].split("/")[0])
                print(json.dumps([{"state": "success", "created_at": iso(deployed_at(st, dep))}]))
            elif "/deployments" in path:
                env = path.split("environment=")[1].split("&")[0] if "environment=" in path else ""
                print(json.dumps(deployments(st, env, now)))
            else:
                print("{}")
        else:
            print(f"stand-in gh: {' '.join(argv)}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
