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

Case `ask-receipt` uses a fixture crown session to ask a live agy worker.
It checks the same ask id at receipt, at turn end and in the crown wake.

Case `sign-in` launches agy with a disposable HOME, reads the board and
get_ticket at the login chooser, and sends no keys.

Case `worktree-trust` reads the board and get_ticket while the new seat is
at its folder dialog. It leaves the decision pending and calls no model.
"""
import datetime
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
CASES = ("brief", "worktree-trust", "sign-in", "file", "plan", "plan-revision", "ask-receipt")
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


def signed_out_cli(root, agy):
    """Pin the disposable HOME at exec, even through tmux's login shell."""
    home = root / "agy-home"
    home.mkdir(mode=0o700)
    wrapper = root / "agy-signed-out"
    wrapper.write_text("#!/bin/sh\nexec /usr/bin/env -i HOME=" + shlex.quote(str(home)) +
                       ' PATH="$PATH" TERM="$TERM" LANG=en_US.UTF-8 ' + shlex.quote(agy) + ' "$@"\n')
    wrapper.chmod(0o700)
    return str(wrapper)


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
        if result["case"] in ("plan", "plan-revision"):
            (trust / "brain").symlink_to(agy_home() / "brain", target_is_directory=True)
        trusted = [] if result["case"] == "worktree-trust" else [str(self.repo.resolve())]
        (trust / "settings.json").write_text(json.dumps({"trustedWorkspaces": trusted}))
        binary = signed_out_cli(guard.root, args.agy) if result["case"] == "sign-in" else args.agy
        env.update(TERM="xterm-256color", MESIMON_AGY="1", MESIMON_AGY_HOME=str(trust), MESIMON_TMUX_BIN=args.tmux,
                   MESIMON_HOOK_BIN=str(args.binary), MESIMON_NO_DAEMON_AUTORESTART="1",
                   MESIMON_AGY_BIN=binary)
        if result["case"] == "ask-receipt":
            # The receiver is the live provider under test. The fixture
            # issues the crown's wire requests and records its wake.
            crown = guard.root / "fixture-crown"
            crown.write_text("#!/bin/sh\nexec sleep 600\n")
            crown.chmod(0o700)
            env["MESIMON_CLAUDE_BIN"] = str(crown)
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
        if self.result["case"] == "sign-in":
            self.check_sign_in()
            return
        if self.result["case"] in ("plan", "plan-revision"):
            self.check_plan()
            return
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
        if self.result["case"] == "ask-receipt":
            self.check_ask_receipt(ticket)
            return
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

    def check_ask_receipt(self, worker):
        self.pane = None
        self.request("set_crown_mode", mode="autonomous")
        self.request("set_crown_budget", budget=2)
        crown = self.request("create_ticket", column="TODO", title="Coordinate",
                             workspace="shared_checkout", tier="claude")["id"]
        self.request("crown_ticket", id=crown)
        sid = self.request("spawn_session", ticket=crown, kind="claude", submit_prompt=False)["id"]
        key = next(t["short_key"] for t in self.client.board()["tickets"] if t["id"] == worker)

        def as_crown(command, **fields):
            response = self.client.agent(sid, command, key=key, **fields)
            if response.get("resp") in ("err", "error", "denied"):
                raise AssertionError(f"{command} refused: {response}")
            return response

        def view():
            return as_crown("agent_read_ticket")["ticket"]

        as_crown("agent_start_ticket", seen=view()["seen"], workspace="shared_checkout")
        self.sid = self.wait("worker seat", lambda: next((s["id"] for s in self.client.board()["sessions"]
                             if s["ticket"] == worker), None), 30)
        self.pane = self.sid.replace("-", "")[:16]
        self.wait("worker's first turn to finish", lambda: self.session()["state"] ==
                  {"state": "idle", "stop_reason": "end_turn"}, self.args.timeout)
        words = f"Reply with {self.nonce} and nothing else."
        ask = as_crown("agent_ask_ticket", text=words, seen=view()["seen"], deliver="now")["asked"]["ask"]

        def heard(status):
            receipt = view().get("asked") or {}
            return receipt.get("ask") == ask and receipt.get("status") == status

        for status in ("handed", "taken"):
            self.wait(f"ask {ask} {status}", lambda: heard(status), self.args.timeout)
            self.mark(f"get_ticket reads {status}", True, ask)
        self.wait("crown wake naming the taken ask", lambda: any(
            row.get("kind") == "crown_wake" and row.get("worker") == worker
            and row.get("ask") == f"{ask} taken" for row in read_transcript(self.state / "activity.jsonl")), 30)
        self.mark("crown wake names the taken ask", True, ask)
        rows = read_transcript(transcript_path(self.session()["agy_conversation_id"]))
        self.mark("agy replied to the ask", any(self.nonce in r.get("content", "") for r in rows
                                               if r.get("type") == "PLANNER_RESPONSE"))
        self.result["model"] = ", ".join(sorted({r["model"] for r in rows if r.get("model")}))

    def check_sign_in(self):
        ticket = self.request("create_ticket", column="TODO", title="Wait for sign-in",
                              workspace="shared_checkout")["id"]
        self.sid = self.request("spawn_session", ticket=ticket, kind="agy", submit_prompt=True,
                                plan=False)["id"]
        self.pane = None
        self.pane = self.wait("agy's pane", lambda: self.panes()[:1], 30)[0]
        self.wait("sign-in stop", lambda: self.session()["state"] ==
                  {"state": "requires_action", "reason": "auth"}, 60)
        screen = self.screen()
        (self.out / "screen.txt").write_text(screen)
        self.mark("login choices await the person", "Google OAuth" in screen
                  and "Use a Google Cloud project" in screen)
        self.mark("card names sign-in", self.session().get("detail") == "agy asks you to sign in")
        view = self.client.agent(self.sid, "agent_get_ticket", full=False)["ticket"]["needs_you"]
        self.mark("get_ticket identifies the person's step", "only the person" in view["reason"]
                  and "sign in" in view["reason"] and not view.get("request"), view["reason"])

    def enter_plan_decision(self, feedback):
        # A separate, detached terminal owns the board UI. No client attaches
        # to the provider's pane. Alt+Enter opens the ask on legacy terminals.
        board = "rig-review-ui"
        self.tm("new-session", "-d", "-s", board, "-x", "160", "-y", "44", "-c", str(self.repo),
                str(self.args.binary), "open", str(self.repo))
        def screen():
            return self.tm("capture-pane", "-p", "-t", board)
        try:
            self.wait("board or first-run setup", lambda: "quit setup" in screen() or "Write a short" in screen(), 30)
            if "quit setup" in screen():
                self.tm("send-keys", "-t", board, "q")
            self.wait("ticket drawn on the board", lambda: "Write a short" in screen(), 15)
            self.tm("send-keys", "-t", board, "M-Enter")
            self.wait("ticket's plan ask field", lambda: "enter accepts the plan" in screen(), 10)
            if feedback:
                self.tm("send-keys", "-t", board, "-l", feedback)
                self.wait("feedback in the ask field", lambda: "send plan back" in screen(), 10)
            self.tm("send-keys", "-t", board, "Enter")
        finally:
            (self.out / "board.txt").write_text(screen())
        # Leave the TUI on the fixture until the provider receipt. The fixture
        # supervisor owns this tmux session along with the provider's.

    def check_plan(self):
        self.request("set_mcp_tools", on=False)
        self.request("set_column_settings", name="TODO", settings={"agy_mode": "plan"})
        title = ("Write a short implementation plan artifact for greeting the reader in README.md. "
                 "Request review and wait. After approval reply with " + self.nonce +
                 ". No shell commands or workspace edits are needed.")
        ticket = self.request("create_ticket", column="TODO", title=title,
                              workspace="shared_checkout")["id"]
        if self.args.model:
            tier = dict(id="rig-agy", name="rig-agy", provider="agy", model=self.args.model)
            self.request("save_tier", scope="board", tier=tier)
            self.request("set_ticket_tier", id=ticket, tier=tier["id"])
        self.sid = self.request("spawn_session", ticket=ticket, kind="agy", submit_prompt=True,
                                plan=False)["id"]
        self.pane = None
        self.pane = self.wait("agy's pane", lambda: self.panes()[:1], 30)[0]
        argv = self.session()["argv"]
        self.mark("column launched agy in plan mode", argv[argv.index("--mode") + 1] == "plan")
        self.wait("plan review", lambda: self.session()["state"] ==
                  {"state": "requires_action", "reason": "plan"}, self.args.timeout)
        rec = self.session()
        reply = self.request("read_note", ticket=ticket, note=rec["plan_note"])
        text = reply["text"]
        artifact = agy_home() / "brain" / rec["agy_conversation_id"] / rec["agy_plan_request"].split("/")[-1]
        self.mark("ticket note matches the artifact", text.encode() == artifact.read_bytes())
        path = transcript_path(rec["agy_conversation_id"])
        turn_start = len(read_transcript(path))
        revision = self.result["case"] == "plan-revision"
        feedback = "Use the greeting " + self.nonce + " in the revision." if revision else ""
        self.enter_plan_decision(feedback)
        decision = "[Rejected]" if revision else "[Approved]"
        expected = f"{decision} implementation_plan.md"
        def decision_taken():
            return any(expected in row.get("content", "") for row in read_transcript(path)[turn_start:]
                       if row.get("type") == "USER_INPUT")
        self.wait(expected, decision_taken, self.args.timeout)
        event = "plan_rejected" if revision else "plan_accepted"
        def board_receipt():
            feed = self.state / "activity.jsonl"
            return any(row.get("cmd") == event and row.get("ticket") == ticket and row.get("outcome") == "answered"
                       for row in (json.loads(line) for line in feed.read_text().splitlines()))
        self.wait("confirmed board receipt", board_receipt, 15)
        self.mark("agy and the board confirmed " + expected, True)
        if revision:
            self.wait("feedback in the conversation", lambda: any(
                feedback in row.get("content", "") for row in read_transcript(path)[turn_start:]), self.args.timeout)
            self.mark("board delivered the revision words", True)
            return
        self.wait("reply after approval", lambda: any(self.nonce in r.get("content", "")
                  for r in read_transcript(path)[turn_start:] if r.get("type") == "PLANNER_RESPONSE"), self.args.timeout)
        self.mark("agy continued after approval", True)

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
    live_cases = {"file", "plan", "plan-revision", "ask-receipt"}.intersection(cases)
    if live_cases and not args.live:
        parser.error(f"{', '.join(sorted(live_cases))} requires --live to verify the agent's answer")
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
