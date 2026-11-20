"""agy through the production daemon (T-1232): one disposable board per case.

`python3 -B ci/rig.py --provider agy` runs up to the seat at its prompt and
calls no model; `--live` types the ticket's title into the pane and reads
agy's own transcript. agy runs with the person's own agy home (its sign-in
lives there), so a live case leaves one conversation in agy's list.
An optional --model selects a board-local tier for the case.

Case `brief`: the ticket's description holds a nonce and the typed prompt is
the title alone. It passes when the brief reached the conversation, the
first call is the eager mesimon get_ticket, and the reply carries the nonce
without asking the person.

Case `worktree-trust` reads the board and get_ticket while the new seat is
at its folder dialog. It leaves the decision pending and calls no model.
"""
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
CASES = ("brief", "worktree-trust", "file")
TITLE = "Answer the question on this ticket"
BRIEF_MARK = "This session was started by mesimon on a ticket."


class Inconclusive(Exception):
    """Ran and proved neither way."""


def module(name, filename):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, ROOT / "ci" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def agy_home():
    return Path(os.environ.get("MESIMON_AGY_HOME") or Path.home() / ".gemini" / "antigravity-cli")


def transcript_path(conversation):
    return agy_home() / "brain" / conversation / ".system_generated" / "logs" / "transcript_full.jsonl"


def read_transcript(path):
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def tool_calls(rows):
    """Every tool call in step order, as `(name, args)`."""
    calls = []
    for row in rows:
        raw = row.get("tool_calls") or []
        if isinstance(raw, str):
            try:
                raw = json.loads(raw)
            except ValueError:
                raw = []
        for call in raw:
            args = call.get("args") or {}
            if isinstance(args, str):
                try:
                    args = json.loads(args)
                except ValueError:
                    args = {"raw": args}
            calls.append((call.get("name", ""), args))
    return calls


def verdict(rows, nonce):
    """The `brief` case's checkpoints off agy's transcript: a list of
    `(name, passed, detail)`."""
    points = []
    braced = [row for row in rows if BRIEF_MARK in json.dumps(row)]
    points.append(("brief in the conversation", bool(braced),
                   braced[0].get("type", "") if braced else "no step carries the brief"))
    prompts = [row.get("content", "") for row in rows if row.get("type") == "USER_INPUT"]
    request, closing, _ = (prompts[0] if prompts else "").partition("</USER_REQUEST>")
    title_only = len(prompts) == 1 and bool(closing) and request.strip() == "<USER_REQUEST>\n" + TITLE
    points.append(("only the title was submitted", title_only, TITLE))
    calls = tool_calls(rows)
    first = calls[0] if calls else None
    points.append(("get_ticket first", bool(first and first[0] == "mcp_mesimon_get_ticket"),
                   json.dumps(first)[:300] if first else "no tool call"))
    replies = [row.get("content", "") for row in rows
               if row.get("type") == "PLANNER_RESPONSE" and not row.get("tool_calls")]
    said = replies[-1] if replies else ""
    points.append(("reply carries the ticket's nonce", nonce in said, said[:300]))
    return points


class Case:
    def __init__(self, args, out, result, guard):
        self.args, self.out, self.result, self.guard = args, out, result, guard
        self.lab = module("rig_agy_lab", "state-lab.py")
        self.started = time.monotonic()
        self.repo = guard.root / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True, timeout=5)
        (self.repo / "README.md").write_text("Disposable agy acceptance fixture.\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True, timeout=5)
        subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=rig", "-c", "user.email=rig@localhost",
                        "commit", "-qm", "fixture"], check=True, timeout=5)
        self.runtime, self.state = self.lab.paths(self.repo)
        guard.request(op="register", repo=str(self.repo), state=str(self.state),
                      runtime=str(self.runtime), sock=str(self.runtime / "tmux.sock"))
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("MESIMON_") and key not in ("TMUX", "TMUX_PANE")}
        # The brief fixture trusts its checkout. The trust case waits on the
        # new worktree's dialog and never accepts it. Only the fixture's
        # settings are written here; agy keeps its own home and sign-in.
        trust = guard.root / "agy-trust"
        trust.mkdir(mode=0o700)
        trusted = [] if result["case"] == "worktree-trust" else [str(self.repo.resolve())]
        (trust / "settings.json").write_text(json.dumps({"trustedWorkspaces": trusted}))
        env.update(TERM="xterm-256color", MESIMON_AGY="1", MESIMON_AGY_HOME=str(trust), MESIMON_TMUX_BIN=args.tmux,
                   MESIMON_HOOK_BIN=str(args.binary), MESIMON_NO_DAEMON_AUTORESTART="1",
                   MESIMON_AGY_BIN=args.agy)
        guard.request(op="spawn", argv=[str(args.binary), "daemon", "--repo", str(self.repo)], env=env)
        self.lab.wait_for(lambda: (self.runtime / "orch.sock").exists(), timeout=15)
        self.client = self.lab.Client(self.repo)
        self.nonce = "PERIWINKLE-" + uuid.uuid4().hex[:8].upper()

    def mark(self, name, passed, detail=""):
        self.result["checkpoints"].append(dict(
            at_ms=int((time.monotonic() - self.started) * 1000), name=name, passed=passed, detail=detail))
        print(f"  {'✓' if passed else '✗'} {name}: {detail}", flush=True)

    def request(self, command, **fields):
        response = self.client.request(command, **fields)
        if response.get("resp") in ("err", "error", "denied"):
            raise AssertionError(f"{command} refused: {response}")
        return response

    def tm(self, *argv):
        out = subprocess.run([self.args.tmux, "-S", str(self.runtime / "tmux.sock"), *argv],
                             capture_output=True, text=True, timeout=5)
        if out.returncode:
            raise Inconclusive("tmux: " + out.stderr.strip())
        return out.stdout

    def panes(self):
        out = subprocess.run([self.args.tmux, "-S", str(self.runtime / "tmux.sock"), "list-panes", "-a",
                              "-F", "#{pane_id}"], capture_output=True, text=True, timeout=5)
        return out.stdout.split()

    def screen(self):
        return self.tm("capture-pane", "-p", "-t", self.pane) if self.pane else ""

    def session(self):
        return next(s for s in self.client.board()["sessions"] if s["id"] == self.sid)

    def wait(self, what, test, timeout):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            value = test()
            if value:
                return value
            time.sleep(0.5)
        (self.out / "screen.txt").write_text(self.screen())
        raise Inconclusive(f"{what} not reached in {timeout} s")

    def run(self):
        self.request("set_agent_provider", provider="agy")
        self.request("set_mcp_tools", on=True)
        self.request("set_system_prompt", on=True)
        if self.result["case"] == "worktree-trust":
            self.check_worktree_trust()
            return
        title = TITLE if self.args.live else "/mcp"
        ticket = self.request("create_ticket", column="TODO", title=title,
                              workspace="shared_checkout")["id"]
        first_word = self.nonce if self.result["case"] == "brief" else "READY"
        self.request("write_note", ticket=ticket, note=None, text=(
            f"# Question\n\nReply with the word {first_word} and nothing else. "
            "Run no command and change no file."))
        if self.args.model:
            tier = dict(id="rig-agy", name="rig-agy", provider="agy", model=self.args.model)
            self.request("save_tier", scope="board", tier=tier)
            self.request("set_ticket_tier", id=ticket, tier=tier["id"])
        self.sid = self.request("spawn_session", ticket=ticket, kind="agy", submit_prompt=False,
                                plan=False)["id"]
        self.pane = None
        self.pane = self.wait("agy's pane", lambda: self.panes()[:1], 30)[0]
        launch = self.session().get("argv") or []
        folder = Path(launch[launch.index("--add-dir") + 1])
        config = json.loads((folder / ".agents" / "mcp_config.json").read_text())
        self.mark("mcp_config.json names the shim", "mesimon" in config.get("mcpServers", {}),
                  " ".join(config["mcpServers"]["mesimon"]["args"][:1]))
        # Start without submitting the composed ticket (which includes its note).
        # The board prefills the title alone. Verify the draft before its one Enter.
        def prefilled():
            screen = self.screen()
            if "trust the contents" in screen:
                raise AssertionError("agy asked to trust the fixture")
            return self.session().get("typed_title") == title and any(
                line.strip() == "> " + title for line in screen.splitlines())
        self.wait("agy's prefilled title", prefilled, 60)
        self.tm("send-keys", "-t", self.pane, "Enter")
        if not self.args.live:
            self.check_server_list()
            return
        self.wait("agy's turn", lambda: self.session()["state"]["state"] == "running", 60)

        def ended():
            state = self.session()["state"]
            if state["state"] == "requires_action":
                (self.out / "screen-asked.txt").write_text(self.screen())
                raise AssertionError(f"agy asked the person: {state}")
            return state["state"] == "idle"
        self.wait("the turn's end", ended, self.args.timeout)
        if self.result["case"] == "file":
            from wire import file_prompt
            file_prompt(self.request, ticket, self.nonce)
            self.wait("the file turn", lambda: self.session()["state"]["state"] == "running", 60)
            self.wait("file turn complete", ended, self.args.timeout)
        self.mark("no board tool asked the person", True)
        (self.out / "screen-end.txt").write_text(self.screen())
        conversation = self.session().get("agy_conversation_id")
        if not conversation:
            raise Inconclusive("no conversation id learned")
        path = transcript_path(conversation)
        self.wait("agy's transcript", path.exists, 20)
        rows = read_transcript(path)
        shutil.copyfile(path, self.out / "transcript_full.jsonl")
        models = sorted({row["model"] for row in rows if row.get("model")})
        self.result["model"] = ", ".join(models)
        if self.result["case"] == "file":
            replied = any(self.nonce in row.get("content", "") for row in rows if row.get("type") == "PLANNER_RESPONSE")
            self.mark("agy replies with the word held only in the file", replied)
            self.mark("agy reads the ticket attachment", any(name.endswith("read_attachment") or "read_attachment" in str(args)
                      for name, args in tool_calls(rows)))
            return
        for name, passed, detail in verdict(rows, self.nonce):
            self.mark(name, passed, detail)

    def check_worktree_trust(self):
        ticket = self.request("create_ticket", column="TODO", title="Inspect the worktree",
                              workspace="worktree")["id"]
        self.request("spawn_session", ticket=ticket, kind="agy", submit_prompt=False, plan=False)
        self.pane = None
        def stopped():
            return next((s for s in self.client.board()["sessions"] if s["ticket"] == ticket
                         and s["state"] == {"state": "requires_action", "reason": "trust"}), None)
        seat = self.wait("the worktree trust stop", stopped, 60)
        hint = f"Trusting {self.repo.resolve()} once in agy covers this board's worktrees"
        self.mark("the stop names the checkout", hint in (seat.get("detail") or ""))
        response = self.client.agent(seat["id"], "agent_get_ticket", full=False)
        reason = response["ticket"]["needs_you"]["reason"]
        self.mark("get_ticket names the checkout", reason == "trust ∙ " + hint, reason)

    def check_server_list(self):
        def listed():
            row = [line for line in self.screen().splitlines() if " mesimon " in line + " "]
            return row if row and "Tools:" in row[0] else None
        try:
            row = self.wait("mesimon in agy's server list", listed, 30)[0].strip()
            self.mark("agy lists the board's tools", "✓" in row and "get_ticket" in row, row[:200])
        except Inconclusive:
            self.mark("agy lists the board's tools", False, self.screen()[-600:])
        (self.out / "screen-mcp.txt").write_text(self.screen())


def run(repo, args, parser):
    if args.effort:
        parser.error("pass agy's model variant including its level with --model")
    cases = args.only.split(",") if args.only else ["brief", "worktree-trust"]
    unknown = set(cases) - set(CASES)
    if unknown:
        parser.error(f"unknown agy cases: {', '.join(sorted(unknown))}")
    print(f"agy rig: {', '.join(cases)}; {'live' if args.live else 'no model call'}", flush=True)
    if args.lay:
        return 0
    if "file" in cases and not args.live:
        parser.error("the file case needs --live to verify the agent's answer")
    if not os.environ.get("MESIMON_TEST_RUN"):
        command = [sys.executable, "-B", str(ROOT / "ci/test-run.py"), "--cargo-manifest", str(Path(repo) / "Cargo.toml"),
                   "--timeout", "900", "--",
                   sys.executable, "-B", str(ROOT / "ci/rig.py"), *sys.argv[1:]]
        os.execv(sys.executable, command)
    binary = args.binary or Path(repo) / "target/debug/mesimon"
    if not args.no_build and args.binary is None:
        subprocess.run(["cargo", "build", "-p", "mesimon"], cwd=repo, check=True)
    binary = binary.resolve(strict=True)
    agy = shutil.which("agy")
    tmux = os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux")
    if not agy or not tmux:
        parser.error("installed agy and tmux are required")
    version = subprocess.run([agy, "--version"], capture_output=True, text=True, timeout=10).stdout.strip()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = Path(repo) / "target/rig/agy" / stamp
    out.mkdir(parents=True, mode=0o700)
    probe = module("rig_agy_probe", "codex-runtime-probe.py")
    manifest = dict(version=version, live=args.live, cases=[])
    failed = False
    for case in cases:
        print(f"\n▶ agy {case}", flush=True)
        result = dict(case=case, outcome="running", checkpoints=[])
        manifest["cases"].append(result)
        capture = out / case
        capture.mkdir(mode=0o700)
        guard = probe.Guard(args.case_timeout or 300, tmux)
        verification = None
        try:
            import argparse
            case_args = argparse.Namespace(binary=binary, agy=agy, tmux=tmux, live=args.live,
                                           model=args.model, timeout=args.case_timeout or 240)
            verification = Case(case_args, capture, result, guard)
            print("  Watch: cd " + str(verification.repo) + " && " + str(binary), flush=True)
            verification.run()
            result["outcome"] = ("passed" if all(p["passed"] for p in result["checkpoints"])
                                 else "failed")
        except AssertionError as error:
            result.update(outcome="failed", error=str(error))
        except Exception as error:
            result.update(outcome="inconclusive", error=type(error).__name__ + ": " + str(error))
        finally:
            if verification:
                try:
                    (capture / "board.json").write_text(json.dumps(verification.client.board(), indent=2))
                    verification.client.close()
                except Exception as error:
                    result["capture_error"] = str(error)
            result["cleanup"] = guard.close()
        failed |= result["outcome"] != "passed"
        print(f"  {result['outcome']} · cleanup {result['cleanup']} · {result.get('error', '')}", flush=True)
    (out / "verdicts.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("\nVerdicts: " + str(out / "verdicts.json"), flush=True)
    return int(failed)
