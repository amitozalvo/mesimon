#!/usr/bin/env python3
"""Bounded native Codex plan and cold-resume brief experiments.

Run through ci/test-run.py with --live. This is transport evidence, not a
substitute for production Mesimon acceptance. Only disposable conversations
are created; the relay never issues or answers model/approval RPCs.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("codex_plan_runtime", ROOT / "ci/codex-runtime-probe.py")
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def fixture_hook():
    payload = json.loads(sys.stdin.read(65537))
    flag = Path(os.environ["MESIMON_PLAN_BRIEF"])
    context = flag.read_text().strip() if flag.is_file() else ""
    output = ({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}
              if payload.get("hook_event_name") == "SessionStart" and context else {})
    with Path(os.environ["MESIMON_PLAN_HOOK_LOG"]).open("a") as log:
        log.write(json.dumps({"input": payload, "output": output}) + "\n")
    print(json.dumps(output))


class Inconclusive(Exception):
    pass


class Experiment:
    def __init__(self, args, guard, out, manifest):
        self.args, self.guard, self.out, self.manifest = args, guard, out, manifest
        self.start = time.monotonic()
        self.deadline = self.start + args.timeout
        self.home, self.repo = guard.root / "codex-home", guard.root / "repo"
        self.home.mkdir(mode=0o700)
        self.repo.mkdir(mode=0o700)
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True, timeout=5)
        (self.repo / "README.md").write_text("Disposable plan verification fixture.\n")
        if not args.auth_file.is_file():
            raise Inconclusive("existing authentication unavailable; no login attempted")
        shutil.copyfile(args.auth_file, self.home / "auth.json")
        (self.home / "auth.json").chmod(0o600)
        self.developer = "DEVELOPER_" + uuid.uuid4().hex[:10]
        (self.home / "config.toml").write_text(
            'cli_auth_credentials_store = "file"\ncheck_for_update_on_startup = false\n'
            + "model = " + json.dumps(args.model) + "\n"
            + 'sandbox_mode = "workspace-write"\napproval_policy = "on-request"\n'
            + "developer_instructions = " + json.dumps("The fixture developer sentinel is " + self.developer + ".") + "\n")
        self.flag = guard.root / "brief-enabled"
        self.hook_log = out / "hook-inputs.jsonl"
        self.hook_command = '"$MESIMON_PLAN_PYTHON" -B "$MESIMON_PLAN_SCRIPT" hook'
        self.flags = []
        if args.case == "brief-resume":
            self.flag.write_text("BRIEF_INITIAL_" + uuid.uuid4().hex[:10])
            self.flags = ["-c", "hooks.SessionStart=[{hooks=[{type=\"command\",command="
                          + json.dumps(self.hook_command) + ",timeout=5}]}]"]
        self.wrapper = guard.root / "codex-isolated"
        exports = dict(CODEX_HOME=str(self.home), MESIMON_PLAN_PYTHON=sys.executable,
                       MESIMON_PLAN_SCRIPT=str(Path(__file__).resolve()),
                       MESIMON_PLAN_BRIEF=str(self.flag), MESIMON_PLAN_HOOK_LOG=str(self.hook_log))
        self.wrapper.write_text("#!/bin/sh\n" + "\n".join("export " + key + "=" + shlex.quote(value)
                                for key, value in exports.items())
                                + "\nexec " + shlex.quote(args.codex) + ' "$@"\n')
        self.wrapper.chmod(0o700)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("CODEX_", "OPENAI_", "MESIMON_"))
                    and key not in ("TMUX", "TMUX_PANE")}
        self.env.update(TERM="xterm-256color", CODEX_HOME=str(self.home))
        self.sock = guard.root / "tmux.sock"
        guard.request(op="register", repo=str(self.repo), sock=str(self.sock))
        self.rpc = None
        self.stage = 0
        self.calls = 0
        self.last_screen = None
        self.thread = None
        self.server_pid = self.relay_pid = None
        self.pending = {}
        self.frames = []
        self.usage = []
        self.trace_offset = 0
        self.trace_buffer = b""

    def mark(self, **value):
        self.manifest["checkpoints"].append(dict(at_ms=int((time.monotonic()-self.start)*1000), **value))

    def check_time(self):
        if time.monotonic() >= self.deadline:
            raise Inconclusive("whole experiment deadline reached")

    def tm(self, *args):
        self.check_time()
        result = subprocess.run([self.args.tmux, "-S", str(self.sock), *args], env=self.env,
                                capture_output=True, text=True, timeout=5)
        if result.returncode:
            raise Inconclusive("owned tmux command: " + result.stderr.strip())
        return result.stdout

    def call(self, method, params):
        result = self.rpc.call(method, params)
        if "error" in result:
            raise Inconclusive(f"read-only {method} failed: {result['error']}")
        return result["result"]

    def verify_home(self):
        config = self.call("config/read", {"includeLayers": True, "cwd": str(self.repo)})
        origin = config.get("origins", {}).get("model", {}).get("name", {})
        if config.get("config", {}).get("model") != self.args.model or origin.get("file") != str(self.home / "config.toml"):
            raise AssertionError("native effective model/config escaped pinned fixture home")
        if self.developer not in (config.get("config", {}).get("developer_instructions") or ""):
            raise AssertionError("fixture developer instructions were replaced")
        self.mark(assertion="effective model and developer config originate in pinned fixture home", stage=self.stage, passed=True)

    def launch(self, resume=None):
        if self.stage:
            self.usage.extend(r["frame"]["params"] for r in self.frames if r["frame"].get("method") == "thread/tokenUsage/updated")
        self.stage += 1
        self.thread = None
        self.pending, self.frames = {}, []
        self.trace_offset, self.trace_buffer = 0, b""
        self.last_screen = None
        endpoint = self.guard.root / f"app-{self.stage}.sock"
        relay = self.guard.root / f"relay-{self.stage}.sock"
        self.trace = self.out / f"native-{self.stage}.jsonl"
        server = [str(self.wrapper), *self.flags, "app-server", "--listen", "unix://" + str(endpoint)]
        self.server_pid = self.guard.request(op="spawn", argv=server, env=self.env)
        end = min(self.deadline, time.monotonic()+15)
        while not endpoint.exists():
            if time.monotonic() >= end or self.guard.request(op="poll", pid=self.server_pid) is not None:
                raise Inconclusive("owned app-server did not start")
            time.sleep(.05)
        self.rpc = probe.Rpc(endpoint, self.out, f"readonly-{self.stage}")
        self.rpc.initialize()
        self.verify_home()  # Before any native trust or model submission.
        self.relay_pid = self.guard.request(op="spawn", argv=[sys.executable, "-B",
            str(ROOT / "ci/codex-runtime-probe.py"), "relay", str(relay), str(endpoint), str(self.trace), str(self.args.timeout)], env=self.env)
        while not relay.exists():
            self.check_time()
            time.sleep(.05)
        native = [str(self.wrapper), "--remote", "unix://"+str(relay), "--no-alt-screen", "-C", str(self.repo), *self.flags]
        if resume:
            native += ["resume", resume]
        launch = self.guard.root / f"launch-{self.stage}.sh"
        launch.write_text("#!/bin/sh\nexec " + shlex.join(native) + "\n")
        launch.chmod(0o700)
        if self.stage == 1:
            child = self.guard.request(op="spawn", argv=[self.args.tmux, "-S", str(self.sock), "-f", "/dev/null",
                "new-session", "-d", "-s", "probe", "-x", "120", "-y", "40", "-c", str(self.repo), str(launch)], env=self.env)
            while self.guard.request(op="poll", pid=child) is None:
                self.check_time()
                time.sleep(.05)
        else:
            self.tm("respawn-pane", "-k", "-t", "probe", str(launch))
        self.tm("set-option", "-t", "probe", "remain-on-exit", "on")
        self.mark(action="native_launch", stage=self.stage, requested_resume=resume)
        self.wait("native input ready", self.ready, consent=True)
        if resume and self.thread != resume:
            raise AssertionError("cold resume changed the exact thread identity")
        self.verify_home()
        return self.thread

    def read_trace(self):
        if not self.trace.exists():
            return
        with self.trace.open("rb") as source:
            source.seek(self.trace_offset)
            chunk = source.read(32*1024*1024)
            self.trace_offset = source.tell()
        self.trace_buffer += chunk
        lines = self.trace_buffer.split(b"\n")
        self.trace_buffer = lines.pop()
        if len(self.trace_buffer) > 32*1024*1024:
            raise Inconclusive("trace line exceeds bound")
        for line in lines:
            row = json.loads(line)
            frame = row["frame"]
            method = frame.get("method", "")
            if row["direction"] == "native_to_server" and method in ("thread/start", "thread/resume", "thread/fork") and not frame.get("params", {}).get("ephemeral"):
                self.pending[json.dumps(frame.get("id"))] = method
            if row["direction"] == "server_to_native" and not method and json.dumps(frame.get("id")) in self.pending:
                thread = frame.get("result", {}).get("thread", {})
                if thread and not thread.get("ephemeral") and thread.get("threadSource") != "system":
                    path = thread.get("path")
                    if not path:
                        raise Inconclusive("foreground thread did not expose its fixture history path")
                    Path(path).resolve().relative_to(self.home.resolve())
                    self.thread = thread["id"]
                    self.mark(assertion="native foreground thread belongs to fixture", thread=self.thread, history=path, passed=True)
            if method.startswith(("thread/", "turn/", "item/", "hook/", "serverRequest/")) or (not method and "id" in frame):
                self.frames.append(row)
            if len(self.frames) > 20000:
                raise Inconclusive("event capture exceeds bound")

    def screen(self):
        self.read_trace()
        screen = self.tm("capture-pane", "-p", "-t", "probe")
        if screen != self.last_screen:
            with (self.out / "terminal.jsonl").open("a") as log:
                log.write(json.dumps(dict(stage=self.stage, at_ms=int((time.monotonic()-self.start)*1000), screen=screen))+"\n")
            self.last_screen = screen
        if self.tm("display-message", "-p", "-t", "probe", "#{pane_dead}").strip() == "1":
            raise Inconclusive("native terminal exited before checkpoint")
        return screen

    def consent(self, screen):
        if "Do you trust" in screen and "1. Yes, continue" in screen:
            self.verify_home()
            self.tm("send-keys", "-t", "probe", "Enter")
            self.mark(action="native_trust_synthetic_repository")
            time.sleep(.2)
        elif "Hooks need review" in screen:
            self.verify_home()
            hooks = self.call("hooks/list", {"cwds": [str(self.repo)]})
            enabled = [h for group in hooks.get("data", []) for h in group.get("hooks", []) if h.get("enabled")]
            if not enabled or any(h.get("source") != "sessionFlags" or h.get("command") != self.hook_command for h in enabled):
                raise AssertionError("hook review contains a handler outside this fixture")
            # Only the inspected synthetic SessionStart handler exists. Native
            # option 2 grants its normal trust before the first thread starts.
            if "Trust all and continue" not in screen:
                raise Inconclusive("native startup hook trust choices changed")
            self.tm("send-keys", "-t", "probe", "2")
            self.tm("send-keys", "-t", "probe", "Enter")
            self.mark(action="native_trust_inspected_fixture_hook_before_thread")
            time.sleep(.3)

    def ready(self, screen):
        return (self.thread is not None and "›" in screen and ("? for shortcuts" in screen or "% context left" in screen or (self.args.model in screen and str(self.repo) in screen))
                and not any(x in screen for x in ("Hooks need review", "Do you trust", "Sign in with ChatGPT", "Implement this plan?")))

    def wait(self, description, predicate, timeout=55, consent=False):
        end = min(self.deadline, time.monotonic()+timeout)
        while time.monotonic() < end:
            screen = self.screen()
            if predicate(screen):
                self.mark(assertion=description, stage=self.stage, passed=True)
                return screen
            if consent:
                self.consent(screen)
            time.sleep(.1)
        raise Inconclusive("checkpoint not reached: " + description)

    def send(self, text, model=False):
        if model:
            self.verify_home()
            self.calls += 1
            if self.calls > (2 if self.args.case == "plan" else 3):
                raise Inconclusive("bounded model-turn budget exceeded")
        self.tm("send-keys", "-t", "probe", "-l", text)
        time.sleep(.4)
        self.tm("send-keys", "-t", "probe", "Enter")
        self.mark(action="native_submit" if model else "native_command", stage=self.stage, text=text)

    def events(self, method):
        return [r["frame"] for r in self.frames if r["direction"] == "server_to_native"
                and r["frame"].get("method") == method and r["frame"].get("params", {}).get("threadId") == self.thread]

    def completed(self, count=1):
        self.wait("foreground turn completed", lambda _: len(self.events("turn/completed")) >= count)
        turn = self.events("turn/completed")[-1]["params"]["turn"]
        if turn.get("status") != "completed":
            raise Inconclusive("native foreground turn did not complete: " + str(turn.get("status")))
        return turn

    def cold_stop(self):
        self.send("/quit")
        end = min(self.deadline, time.monotonic()+5)
        while self.tm("display-message", "-p", "-t", "probe", "#{pane_dead}").strip() != "1":
            if time.monotonic() >= end:
                raise Inconclusive("native quit did not finish before cold resume")
            time.sleep(.1)
        self.rpc.close()
        self.rpc = None
        for pid in (self.server_pid, self.relay_pid):
            if self.guard.request(op="poll", pid=pid) is None:
                os.kill(pid, signal.SIGTERM)
            while self.guard.request(op="poll", pid=pid) is None:
                self.check_time()
                time.sleep(.05)
        self.mark(action="native_client_and_server_stopped_before_exact_resume", stage=self.stage)

    def plan(self):
        self.launch()
        self.send("/plan")
        self.wait("native plan mode visible", lambda s: "Plan mode" in s or "plan mode" in s)
        self.send("Propose a minimal one-step plan to append the line PLAN_ACCEPTED to README.md. Do not implement it yet. Do not run tools; this fixture is fully described here.", model=True)
        self.completed()
        plan_items = [f["params"]["item"] for f in self.events("item/completed") if f["params"]["item"].get("type") == "plan"]
        requests = [r["frame"] for r in self.frames if r["direction"] == "server_to_native" and "id" in r["frame"] and "method" in r["frame"]]
        starts = [r["frame"] for r in self.frames if r["direction"] == "native_to_server" and r["frame"].get("method") == "turn/start"]
        self.manifest.update(plan_items=plan_items, plan_requests=requests, plan_turn_starts=starts)
        if not any(f.get("params", {}).get("collaborationMode", {}).get("mode") == "plan" for f in starts):
            raise Inconclusive("native plan command did not produce structured plan collaboration mode")
        if not plan_items:
            raise Inconclusive("native planning completed without an authoritative plan item; inspect raw events")
        self.wait("native plan acceptance dialog visible", lambda s: "Implement this plan?" in s and "› 1. Yes, implement this plan" in s)
        self.verify_home()
        self.calls += 1
        self.tm("send-keys", "-t", "probe", "Enter")
        self.mark(action="native_accept_fixture_plan")
        self.completed(2)
        self.wait("accepted plan produced the exact fixture file change", lambda _: (self.repo / "README.md").read_text() == "Disposable plan verification fixture.\nPLAN_ACCEPTED\n")
        accepted_starts = [r["frame"] for r in self.frames if r["direction"] == "native_to_server" and r["frame"].get("method") == "turn/start" and r["frame"].get("params", {}).get("threadId") == self.thread]
        self.manifest["accepted_turn_starts"] = accepted_starts
        if len(accepted_starts) != 2 or accepted_starts[-1].get("params", {}).get("collaborationMode", {}).get("mode") != "default":
            raise Inconclusive("native acceptance did not continue the same thread in default mode")
        self.manifest["file_after"] = (self.repo / "README.md").read_text()
        self.manifest["outcome"] = "native_plan_and_acceptance_observed"

    def brief(self):
        thread = self.launch()
        self.send("Reply with the fixture developer sentinel and the SessionStart brief sentinel you were given. Do not use tools.", model=True)
        self.completed()
        self.manifest["initial_brief"] = self.flag.read_text()
        initial_replies = [f["params"]["item"].get("text", "") for f in self.events("item/completed") if f["params"]["item"].get("type") == "agentMessage"]
        self.manifest["initial_replies"] = initial_replies
        if not any(self.flag.read_text() in reply and self.developer in reply for reply in initial_replies):
            raise Inconclusive("initial brief was not additive to developer instructions")
        self.cold_stop()
        self.flag.unlink()
        self.launch(thread)
        self.send("Reply exactly RESUMED_OFF. Do not use tools.", model=True)
        self.completed()
        self.cold_stop()
        fresh = "BRIEF_RESUMED_" + uuid.uuid4().hex[:10]
        self.flag.write_text(fresh)
        self.launch(thread)
        self.send("Reply with the fixture developer sentinel and the newest SessionStart brief sentinel you were given. Do not use tools.", model=True)
        self.completed()
        hooks = probe.read_frames(self.hook_log)
        self.manifest["hook_observations"] = hooks
        resumed = [row for row in hooks if row.get("input", {}).get("source") == "resume"]
        fresh_hook = any(row.get("output", {}).get("hookSpecificOutput", {}).get("additionalContext") == fresh for row in resumed)
        off_hook = any(not row.get("output") for row in resumed)
        replies = [f["params"]["item"].get("text", "") for f in self.events("item/completed") if f["params"]["item"].get("type") == "agentMessage"]
        self.manifest.update(exact_thread=thread, fresh_brief=fresh, resume_hook_off=off_hook,
                             resume_hook_on=fresh_hook, final_replies=replies)
        if not fresh_hook or not off_hook or not any(fresh in reply and self.developer in reply for reply in replies):
            raise Inconclusive("cold exact resume did not prove both disabled and fresh additive brief behavior")
        self.manifest["outcome"] = "cold_exact_resume_brief_off_on_observed"

    def collect(self):
        self.read_trace()
        self.manifest["model_submissions"] = self.calls
        self.manifest["token_usage"] = self.usage + [r["frame"]["params"] for r in self.frames
            if r["frame"].get("method") == "thread/tokenUsage/updated"]
        if self.rpc and self.thread:
            try:
                self.manifest["final_thread"] = self.call("thread/read", {"threadId": self.thread, "includeTurns": True})
            except Exception as exc:
                self.manifest["history_capture_error"] = str(exc)
        for path in self.guard.root.glob("child-*.log"):
            if path.stat().st_size < 1024*1024:
                shutil.copyfile(path, self.out / path.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--case", choices=("plan", "brief-resume"), default="plan")
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex")))/"auth.json")
    args = parser.parse_args()
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("explicit --live and supervision through ci/test-run.py are required")
    if not 30 <= args.timeout <= 300 or not args.codex or not args.tmux:
        parser.error("requires Codex/tmux and a 30..300 second deadline")
    os.umask(0o077)
    out = ROOT/"target/state-lab/captures"/(datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")+"-codex-plan-"+uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="native_codex_plan_brief_probe", case=args.case, model=args.model,
                    outcome="inconclusive", cleanup=False, checkpoints=[], cost_usd=None,
                    cost_note="Billing unavailable; model submission count and emitted token usage retained",
                    codex_version=subprocess.check_output([args.codex,"--version"],text=True,timeout=10).strip())
    guard = experiment = None
    started = time.monotonic()
    try:
        guard = probe.Guard(args.timeout+20, args.tmux)
        experiment = Experiment(args, guard, out, manifest)
        experiment.plan() if args.case == "plan" else experiment.brief()
    except Inconclusive as exc:
        manifest["reason"] = str(exc)
    except Exception as exc:
        manifest.update(outcome="failed", reason=f"{type(exc).__name__}: {exc}")
    finally:
        if experiment:
            try:
                experiment.collect()
            except Exception as exc:
                manifest["capture_error"] = str(exc)
            if experiment.rpc:
                experiment.rpc.close()
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as exc:
                manifest["cleanup_error"] = str(exc)
        manifest["duration_s"] = round(time.monotonic()-started, 2)
        probe.write(out/"manifest.json", manifest)
    print(json.dumps({"capture":str(out),"outcome":manifest["outcome"],"cleanup":manifest["cleanup"],"reason":manifest.get("reason")}))
    return 0 if manifest["outcome"].endswith("_observed") and manifest["cleanup"] else 1


if __name__ == "__main__":
    if len(sys.argv)>1 and sys.argv[1] == "hook":
        fixture_hook()
    else:
        raise SystemExit(main())
