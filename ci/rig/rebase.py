"""A real merge-flow ask on a tiny integration branch owned by the rig ticket."""
from contextlib import contextmanager
from pathlib import Path
import subprocess
import tempfile

POLICY = """On a rebase, preserve both additions. In docs/STALE-MAP.md the base block goes first.
A clean or docs-only rebase runs no checks. After resolving a code conflict run
`make unit`, `make build` and the e2e for the affected command (`make features-e2e`
for features.sh). The other command has its own `make other-e2e`.
"""

MAKEFILE = """\
.PHONY: unit build features-e2e other-e2e
unit:
	@echo $@ >> .checks
	@sh features.sh | grep -qx base
build:
	@echo $@ >> .checks
	@sh -n features.sh
features-e2e:
	@echo $@ >> .checks
	@sh features.sh | grep -qx upstream
	@sh features.sh | grep -qx topic
other-e2e:
	@echo $@ >> .checks
	@sh -n other.sh
"""


def git(repo, *args):
    result = subprocess.run(["git", "-C", repo, "-c", "user.name=Rig", "-c",
                             "user.email=rig@example.invalid", "-c", "commit.gpgsign=false", *args],
                            text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"git {args[0]}: {result.stderr.strip()}")
    return result.stdout.strip()


@contextmanager
def train_fixture(rig, test):
    """A local merge destination for a crowned case, including its train blocker."""
    target = "rig-train-" + test["ticket"].lower()
    previous = rig.board().get("merge_into", "main")
    # Same files and instructions, independent history. Even a wake after
    # restoration cannot fast-forward a fixture commit onto the real base.
    tip = git(rig.repo, "commit-tree", "HEAD^{tree}", "-m", "Rig merge fixture")
    with tempfile.TemporaryDirectory() as tmp:
        checkout = Path(tmp) / "base"
        git(rig.repo, "worktree", "add", "-b", target, str(checkout), tip)
        restored = False
        try:
            test["fixture_checkout"] = checkout
            test["fixture_base"] = tip
            test["fixture_target"] = "branch:" + target
            rig.wire.set_merge_into(test["fixture_target"])
            try:
                yield
            finally:
                # A step exception can bypass the normal train_off. Disarm
                # this connection and hold the ticket before restoring base.
                rig.wire.request({"cmd": "set_automation", "merge_train": False, "merge_notice": False})
                rig.wire.set_manual_merge(test["ticket"], True)
                rig.wire.set_merge_into(previous)
                restored = True
        finally:
            test.pop("fixture_checkout", None)
            test.pop("fixture_target", None)
            test.pop("fixture_base", None)
            final_tip = git(checkout, "rev-parse", "HEAD")
            git(rig.repo, "worktree", "remove", "--force", str(checkout))
            if restored:
                git(rig.repo, "update-ref", "-d", "refs/heads/" + target, final_tip)
            else:
                rig.log(f"kept {target}: could not confirm restoration of {previous}")


def append(repo, path, text):
    with (repo / path).open("a") as f:
        f.write(text)
    git(repo, "commit", "-am", "Add " + text.strip().splitlines()[0])


def run(rig, test, record, kind, wait_for_stop):
    ticket = test["ticket"]
    target = "rig-rebase-" + ticket.lower()
    ref = "refs/heads/" + target
    previous = rig.board().get("merge_into", "main")
    docs = kind == "docs"
    path = "docs/STALE-MAP.md" if docs else "features.sh"
    additions = (["\n## Upstream\n\nBase decision.\n", "\n## Topic\n\nTicket decision.\n"] if docs
                 else ["echo upstream\n", "echo topic\n"])
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        git(base, "init", "-b", "fixture")
        (base / "docs").mkdir()
        (base / "docs/STALE-MAP.md").write_text("# Decisions\n")
        (base / "features.sh").write_text("echo base\n")
        (base / "other.sh").write_text("echo other\n")
        (base / "Makefile").write_text(MAKEFILE)
        (base / ".gitignore").write_text(".checks\n")
        (base / "AGENTS.md").write_text(POLICY)
        (base / "CLAUDE.md").write_text("@AGENTS.md\n")
        git(base, "add", ".")
        git(base, "commit", "-m", "Initial fixture")
        # Import only this fixture's history. Each ticket has its own target;
        # the real checkout and its base are never advanced.
        def import_base():
            git(rig.repo, "fetch", "--no-tags", "--no-write-fetch-head", tmp, "fixture:" + ref)
            return git(base, "rev-parse", "HEAD")

        base_tip = import_base()
        try:
            rig.wire.set_merge_into("branch:" + target)
            if not rig.rig_step(test, "person_start", record):
                return False
            rec = rig.agent_of(rig.board(), ticket)
            worker = Path(rec["cwd"])
            if git(worker, "rev-parse", "HEAD") != base_tip:
                raise RuntimeError("worker was not cut from the fixture")
            log = worker / ".checks"
            if log.exists():
                raise RuntimeError("worker ran a check before the rebase ask")
            branch = git(worker, "symbolic-ref", "--short", "HEAD")
            append(worker, path, additions[1])
            append(base, path, additions[0])
            base_tip = import_base()
            mark = len(rig.feed_lines)
            rig.wire.request({"cmd": "merge_to_agent", "id": ticket, "request": "rebase"})
            if not wait_for_stop(rec, mark):
                raise RuntimeError("worker did not finish its rebase turn")
            if (git(worker, "symbolic-ref", "--short", "HEAD") != branch
                    or git(worker, "rev-parse", "HEAD^") != base_tip
                    or git(worker, "diff", "--name-only", base_tip, "HEAD") != path
                    or git(worker, "status", "--porcelain", "--untracked-files=no")):
                raise RuntimeError("expected one clean rebased ticket commit")
            if docs:
                content = git(worker, "show", f"HEAD:{path}")
                if content.split() != ("# Decisions\n" + "".join(additions)).split():
                    raise RuntimeError("rebased commit did not keep both decision blocks in order")
            else:
                features = subprocess.run(["sh", path], cwd=worker, capture_output=True,
                                          text=True, timeout=5)
                if features.returncode or sorted(features.stdout.splitlines()) != ["base", "topic", "upstream"]:
                    raise RuntimeError("rebased command did not produce all three features once")
            checks = log.read_text().splitlines() if log.exists() else []
            rig.log(f"{test['id']}: {path} rebased, checks ({len(checks)}): {checks}")
            if set(checks) != (set() if docs else {"build", "unit", "features-e2e"}):
                raise RuntimeError(f"unexpected checks for {path}: {checks}")
            return True
        finally:
            rig.wire.set_merge_into(previous)
            git(rig.repo, "update-ref", "-d", ref, base_tip)
