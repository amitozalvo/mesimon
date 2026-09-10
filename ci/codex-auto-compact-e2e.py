#!/usr/bin/env python3
"""One-turn native automatic compaction at an explicit small fixture threshold.

Run under ci/test-run.py with --live. Uses a 14K total-context compaction limit,
not the model's default window. A single command returns 8192 deterministic hex
characters, then the same user task replies. No manual /compact or observer
mutation is allowed. Failure to reach the prerequisite is inconclusive; no retry
or larger context preparation is attempted. Internal summarization costs extra.
"""
import argparse
import datetime
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("auto_compact_lifecycle", ROOT / "ci/codex-lifecycle-e2e.py")
lifecycle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lifecycle)
state = lifecycle.state
LIMIT = 14000
MARKER = "MESIMON_AUTO_COMPACT_DONE"
PAYLOAD_CODE = 'import hashlib; print("".join(hashlib.sha256(str(i).encode()).hexdigest() for i in range(128)))'
COMMAND = shlex.join(["python3", "-c", PAYLOAD_CODE])


def fixture_hook():
    text = sys.stdin.read(65537)
    if len(text) > 65536:
        raise RuntimeError("automatic-compaction fixture hook payload exceeded bound")
    payload = json.loads(text)
    root = Path(os.environ["MESIMON_LIFECYCLE_ROOT"]).resolve(strict=True)
    expected = (root / "expected-thread").read_text().strip() if (root / "expected-thread").exists() else None
    if payload.get("session_id") == expected and expected:
        event = payload.get("hook_event_name")
        if event in ("PreCompact", "UserPromptSubmit"):
            counter = root / ("automatic-compact-count" if event == "PreCompact" else "automatic-user-count")
            count = int(counter.read_text()) + 1 if counter.exists() else 1
            counter.write_text(str(count))
            if count > 1 or (event == "PreCompact" and payload.get("trigger") != "auto"):
                with (root / "automatic-bound.jsonl").open("a") as log:
                    log.write(json.dumps({"event":event,"count":count,"trigger":payload.get("trigger")}) + "\n")
                print(json.dumps({"continue":False,"stopReason":"Disposable automatic-compaction fixture budget exhausted."}))
                return
    # Reuse the measured 12-second PreCompact barrier and exact native receipts.
    sys.stdin = io.StringIO(text)
    lifecycle.fixture_hook()


def validate_page(page, task):
    turns = page.get("data", [])
    if page.get("nextCursor") or len(turns) != 1 or turns[0].get("id") != task:
        raise AssertionError("automatic compaction did not remain within the single native task")
    items = turns[0].get("items", [])
    compactions = [i for i, item in enumerate(items) if item.get("type") == "contextCompaction"]
    replies = [i for i, item in enumerate(items) if item.get("type") == "agentMessage" and MARKER in (item.get("text") or "")]
    if len(compactions) != 1 or not replies or max(replies) <= compactions[0]:
        raise AssertionError("authoritative automatic compaction is not followed by the actual task reply")
    if turns[0].get("status") != "completed":
        raise AssertionError("native task did not complete after automatic compaction")


def self_test():
    output = "".join(hashlib.sha256(str(i).encode()).hexdigest() for i in range(128))
    assert len(output) == 8192 and len(set(output)) == 16
    page = {"data":[{"id":"task","status":"completed","items":[
        {"type":"userMessage","id":"user"}, {"type":"contextCompaction","id":"compact"},
        {"type":"agentMessage","id":"reply","text":MARKER}]}]}
    validate_page(page, "task")
    for bad, task in [({"data":[]}, "task"), (dict(page,nextCursor="more"), "task"), (page,"other"),
                      ({"data":[dict(page["data"][0],items=list(reversed(page["data"][0]["items"]))) ]},"task")]:
        try:
            validate_page(bad,task)
        except AssertionError:
            pass
        else:
            raise AssertionError("incomplete or incorrectly ordered compaction evidence was accepted")
    print("Automatic-compaction fixture self-test passed; no process or model launched")


class Verification(lifecycle.Verification):
    def __init__(self, args, out, manifest, guard):
        # The existing compact case owns the exact Pre/PostCompact trust surface.
        # This subclass never invokes its manual-compaction scenario.
        super().__init__(args, out, manifest, guard)
        with (self.home / "config.toml").open("a") as config:
            config.write(f"model_auto_compact_token_limit = {LIMIT}\nmodel_auto_compact_token_limit_scope = \"total\"\n")
        wrapper = guard.root / "codex-isolated"
        before = "export MESIMON_LIFECYCLE_SCRIPT=" + shlex.quote(str(Path(lifecycle.__file__).resolve()))
        after = "export MESIMON_LIFECYCLE_SCRIPT=" + shlex.quote(str(Path(__file__).resolve()))
        text = wrapper.read_text()
        if text.count(before) != 1:
            raise AssertionError("native wrapper lifecycle hook entry point is not the expected owned definition")
        wrapper.write_text(text.replace(before,after))
        manifest.update(case="automatic_compaction", user_turn_budget=1, manual_compaction_budget=0,
                        automatic_compaction_budget=1, automatic_continuation_budget=0,
                        model_auto_compact_token_limit=LIMIT, model_auto_compact_token_limit_scope="total",
                        generated_tool_output_characters=8193)

    def verify_native_home(self, sid):
        super().verify_native_home(sid)
        value = self.native_read(sid,"config/read",{"includeLayers":True,"cwd":str(self.repo)})
        config = value.get("config", {})
        origin = value.get("origins",{}).get("model_auto_compact_token_limit",{}).get("name",{})
        if (config.get("model_auto_compact_token_limit") != LIMIT
                or config.get("model_auto_compact_token_limit_scope") != "total"
                or origin.get("file") != str(self.home / "config.toml")):
            raise AssertionError("small total-context compaction threshold was not effective in the private native configuration")
        self.mark(assertion="explicit fixture compaction threshold and private origin verified before submission", passed=True,
                  limit=LIMIT, scope="total")

    def ask(self, sid, prompt):
        if self.calls:
            raise AssertionError("automatic-compaction fixture permits only one user turn")
        return super().ask(sid,prompt)

    def slash(self, *_args, **_kwargs):
        raise AssertionError("automatic-compaction fixture forbids native slash commands")

    def run(self):
        sid = self.spawn_unsubmitted()
        self.verify_native_home(sid)
        thread = self.select_expected(sid)
        self.ask(sid, "Run exactly one foreground shell command, preserving its whole bounded output (max_output_tokens 6000): "
                 + COMMAND + ". The generated hex is disposable fixture data; do not repeat or summarize it. "
                 "After the command and any automatic context compaction finish, reply exactly " + MARKER
                 + ". Do not run any other command, request manual compaction, or retry.")
        self.wait(sid,"automatic PreCompact reached its bounded native barrier",lambda s,t,p:bool(self.hooks("PreCompact")),timeout=65)
        receipt = self.hooks("PreCompact")[-1]
        task = receipt["input"].get("turn_id")
        if receipt["input"].get("trigger") != "auto" or not task:
            raise AssertionError("native compaction was not automatic and scoped to the active task")
        self.held_barrier(sid,"release-compact")
        session, _, _ = self.wait(sid,"automatic PostCompact remains on the same native task",lambda s,t,p:
            bool(self.hooks("PostCompact")) and s.get("codex_thread_id") == thread,timeout=65)
        posts = self.hooks("PostCompact")
        if len(posts) != 1 or posts[0]["input"].get("trigger") != "auto" or posts[0]["input"].get("turn_id") != task:
            raise AssertionError("automatic compaction hook identity or count changed")
        completed, _, _ = self.completed(sid,MARKER)
        if completed.get("codex_turn_id") != task or completed.get("codex_thread_id") != thread or self.calls != 1:
            raise AssertionError("automatic compaction completed a different task or exceeded the user-turn budget")
        if len(self.hooks("PreCompact")) != 1 or (self.guard.root / "automatic-bound.jsonl").exists():
            raise AssertionError("fixture exceeded its one automatic compaction budget")
        validate_page(self.page(sid),task)
        history = self.histories.get(thread)
        if not history or not history.is_file() or history.stat().st_size > lifecycle.MAX_FILE:
            raise state.Inconclusive("bounded native history is unavailable for the one-command output audit")
        records = [json.loads(line) for line in history.read_text().splitlines()]
        calls = [row["payload"] for row in records if row.get("type") == "response_item"
                 and row.get("payload",{}).get("type") in ("function_call", "custom_tool_call")]
        outputs = [row["payload"].get("output", "") for row in records if row.get("type") == "response_item"
                   and row.get("payload",{}).get("type") in ("function_call_output", "custom_tool_call_output")]
        expected_output = "".join(hashlib.sha256(str(i).encode()).hexdigest() for i in range(128))
        if len(calls) != 1 or not any(expected_output in (value if isinstance(value,str) else json.dumps(value)) for value in outputs):
            raise AssertionError("native history does not prove one tool call with the complete bounded generated output")
        self.mark(assertion="exactly one native tool call returned the complete generated context", passed=True,
                  output_characters=8192, tool_calls=1)
        self.mark(assertion="automatic compaction holds the active task then allows its real completion in REVIEW", passed=True,
                  user_turns=1, automatic_compactions=1, thread_id=thread, turn_id=task)

    def collect(self):
        bound = self.guard.root / "automatic-bound.jsonl"
        if bound.exists():
            shutil.copyfile(bound,self.out / bound.name)
        super().collect()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live",action="store_true")
    parser.add_argument("--self-test",action="store_true")
    parser.add_argument("--binary",type=Path,default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex",default=shutil.which("codex"))
    parser.add_argument("--tmux",default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file",type=Path,default=Path(os.environ.get("CODEX_HOME",str(Path.home()/".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("explicit --live and ci/test-run.py supervision are required")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    args.case,args.model,args.timeout = "compact","gpt-5.6-luna",150
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-auto-compact-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True,mode=0o700)
    manifest = dict(kind="real_native_automatic_compaction_at_small_fixture_limit",outcome="inconclusive",cleanup=False,
                    model=args.model,reasoning_effort="low",checkpoints=[],cost_usd=None,
                    limitation="Explicit 14K fixture threshold does not certify the default context window; native compaction/title calls add usage.",
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    def deadline(_signum,_frame):
        raise state.Inconclusive("150-second automatic-compaction fixture deadline exceeded")
    signal.signal(signal.SIGALRM,deadline)
    signal.alarm(150)
    guard = verification = None
    try:
        guard = state.probe.Guard(175,args.tmux)
        verification = Verification(args,out,manifest,guard)
        verification.run()
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed",error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        signal.alarm(0)
        if verification:
            try:
                verification.collect()
                verification.client.close()
            except Exception as error:
                manifest.update(outcome="inconclusive",capture_error=str(error))
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as error:
                manifest["cleanup_error"] = str(error)
        state.write(out / "manifest.json",manifest)
        print(json.dumps(dict(capture=str(out),**manifest)))
    return 0 if manifest["outcome"] == "passed" and manifest["cleanup"] else 1


if __name__ == "__main__":
    if sys.argv[1:] == ["hook"]:
        fixture_hook()
    else:
        sys.exit(main())
