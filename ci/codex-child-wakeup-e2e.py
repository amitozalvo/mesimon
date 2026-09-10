#!/usr/bin/env python3
"""Bounded production wakeup of an independently observed idle Codex child.

Explicit supervised --live only: two parent and two child Luna-low turns, no
retries, 150-second task deadline. Only native Mesimon prompt submission starts
work; independent app-server connections perform read-only observation.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("child_wakeup_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)


def wake_prompt(child):
    # JSON quoting preserves the opaque captured identity inside JavaScript.
    call = 'text(await tools.multi_agent_v1__send_input({target:' + json.dumps(child) + ',message:' + json.dumps(
        'Run sleep 4 once then reply MESIMON_CHILD_AWAKE; no other tools or agents') + '}));'
    return ('Execute this exact JavaScript in exec once: ' + call
            + ' Await only the send_input acknowledgment, not child completion. '
            'Immediately reply MESIMON_PARENT_AWAKE. No other calls, prompts, agents, or responses.')


def bounded_turns(value, limit=2):
    turns = value.get("data", [])
    if value.get("nextCursor") or len(turns) > limit:
        raise AssertionError("native task exceeded its declared model-turn budget")
    if any(not turn.get("id") for turn in turns) or len({turn["id"] for turn in turns}) != len(turns):
        raise AssertionError("native turn identities are absent or duplicated")
    return turns


def items(turns):
    return [item for turn in turns for item in turn.get("items", [])]


class Verification(state.Verification):
    def start(self, prompt):
        if self.calls != 0:
            raise AssertionError("wakeup fixture permits one initial parent start")
        return super().start(prompt)

    def ask(self, sid, prompt):
        if self.calls != 1:
            raise AssertionError("wakeup fixture permits one further parent prompt, without retries")
        super().ask(sid, prompt)

    def turns(self, sid, thread):
        result = self.native_read(sid, "thread/turns/list", {"threadId": thread, "limit": 3, "itemsView": "full"})
        bounded_turns(result)
        return result

    def validate_history(self, path, expected):
        path = Path(path)
        path.resolve().relative_to(self.home.resolve())
        contexts = [json.loads(line)["payload"] for line in path.read_text().splitlines()
                    if json.loads(line).get("type") == "turn_context"]
        if {row.get("turn_id") for row in contexts} != expected:
            raise AssertionError("owned native history does not contain exactly the expected two turn identities")
        if any(row.get("model") != "gpt-5.6-luna" or row.get("effort") != "low" for row in contexts):
            raise AssertionError("parent or child native turn did not use Luna with low reasoning")
        return [dict(turn_id=row["turn_id"], model=row["model"], effort=row["effort"]) for row in contexts]

    def run(self):
        # Reuse the existing measured native spawn lifecycle and its assertions.
        self.child()
        self.manifest["child_turn_budget"] = 2
        checkpoint = next(row for row in reversed(self.manifest["checkpoints"])
                          if row.get("child_thread") and row.get("passed"))
        sid, parent_id, child_id = self.sessions[0], checkpoint["parent_thread"], checkpoint["child_thread"]
        initial_parent = self.turns(sid, parent_id)
        initial_child = self.turns(sid, child_id)
        if len(initial_parent["data"]) != 1 or len(initial_child["data"]) != 1:
            raise AssertionError("first phase did not leave exactly one turn in each thread")
        parent_first, child_first = initial_parent["data"][0]["id"], initial_child["data"][0]["id"]
        initial_meta = self.native_read(sid, "thread/read", {"threadId": child_id, "includeTurns": False})["thread"]
        session, ticket, _ = self.observe(sid)
        if (initial_meta.get("id") != child_id or initial_meta.get("status", {}).get("type") != "idle"
                or initial_child["data"][0].get("status") != "completed"
                or initial_parent["data"][0].get("status") != "completed"
                or session["state"] != {"state": "idle", "stop_reason": "end_turn"}
                or session.get("observation_hold", True) or ticket["column"] != "REVIEW"):
            raise state.Inconclusive("child was not independently observed idle before wakeup; no second prompt submitted")
        self.verify_native_home(sid)
        state.write(self.out / "before-wakeup.json", dict(parent=initial_parent, child=initial_child,
                    child_metadata=initial_meta, board=session))
        self.mark(assertion="same child independently idle before a second parent prompt", passed=True,
                  child_thread=child_id, child_turn=child_first, parent_turn=parent_first)
        self.ask(sid, wake_prompt(child_id))
        saw_active = saw_late_hold = False
        parent_second = child_second = None
        while time.monotonic() - self.started < self.args.timeout:
            session, ticket, screen = self.observe(sid)
            self.startup_consent(sid, screen)
            if session.get("codex_thread_id") != parent_id:
                raise AssertionError("second prompt changed the original parent thread")
            parent, child = self.turns(sid, parent_id), self.turns(sid, child_id)
            parent_new = [turn for turn in parent["data"] if turn["id"] != parent_first]
            child_new = [turn for turn in child["data"] if turn["id"] != child_first]
            meta = self.native_read(sid, "thread/read", {"threadId": child_id, "includeTurns": False})["thread"]
            if meta.get("id") != child_id:
                raise AssertionError("wakeup metadata changed child identity")
            collaboration = [item for item in items(parent["data"]) if item.get("type") == "collabAgentToolCall"]
            spawns = [item for item in collaboration if item.get("tool") == "spawnAgent"]
            sends = [item for item in collaboration if item.get("tool") == "sendInput"]
            if (len(spawns) != 1 or len(sends) > 1
                    or any(item.get("tool") not in ("spawnAgent", "sendInput") for item in collaboration)
                    or any(set(item.get("receiverThreadIds", [])) - {child_id} for item in collaboration)
                    or any(item.get("status") == "completed" and set(item.get("receiverThreadIds", [])) != {child_id}
                           for item in collaboration)
                    or any(item.get("type") == "collabAgentToolCall" for item in items(child["data"]))):
                raise AssertionError("native work exceeded the single-child, single-wakeup tool budget")
            active = meta.get("status", {}).get("type") == "active"
            if parent_new:
                parent_second = parent_new[0]["id"]
            if child_new:
                child_second = child_new[0]["id"]
                saw_active |= active
            parent_done = bool(parent_new and parent_new[0].get("status") == "completed")
            child_done = bool(child_new and child_new[0].get("status") == "completed" and not active)
            late = bool(parent_done and child_new and active and sends and sends[0].get("status") == "completed")
            if late:
                session, ticket, _ = self.observe(sid)
                if (not session.get("observation_hold", True)
                        or session["state"] == {"state": "idle", "stop_reason": "end_turn"}
                        or ticket["column"] == "REVIEW"):
                    raise AssertionError("completed parent released checkout while its previously idle child was active again")
                saw_late_hold = True
            with (self.out / "child-wakeup-observations.jsonl").open("a") as log:
                log.write(json.dumps(dict(at_ms=int((time.monotonic()-self.started)*1000), parent=parent,
                    child=child, child_metadata=meta, late_child=late,
                    board=dict(state=session["state"], column=ticket["column"],
                               observation_hold=session.get("observation_hold")))) + "\n")
            if session["state"]["state"] in ("failed", "exited"):
                raise AssertionError("native wakeup parent failed or exited")
            if parent_done and child_done:
                break
            time.sleep(0.12)
        else:
            raise state.Inconclusive("wakeup acceptance exceeded its 150-second deadline; no retry")
        if not saw_active or not saw_late_hold:
            raise state.Inconclusive("second parent terminal / reactivated child hold window was not captured; no retry")
        self.completed(sid, "MESIMON_PARENT_AWAKE")
        final_parent, final_child = self.turns(sid, parent_id), self.turns(sid, child_id)
        if self.calls != 2 or len(final_parent["data"]) != 2 or len(final_child["data"]) != 2:
            raise AssertionError("expected exactly two parent and two child turns")
        replies = [item.get("text", "") for turn in final_child["data"] if turn["id"] == child_second
                   for item in turn.get("items", []) if item.get("type") == "agentMessage"]
        if not any("MESIMON_CHILD_AWAKE" in reply for reply in replies):
            raise AssertionError("second actual child reply marker is missing")
        session, _, _ = self.observe(sid)
        models = dict(parent=self.validate_history(session["transcript_path"], {parent_first, parent_second}),
                      child=self.validate_history(self.child_histories[child_id], {child_first, child_second}))
        state.write(self.out / "after-wakeup.json", dict(parent=final_parent, child=final_child, models=models))
        self.mark(assertion="previously idle child wakes on same thread and holds checkout after parent completion",
                  passed=True, parent_thread=parent_id, child_thread=child_id, parent_turns=2, child_turns=2,
                  parent_terminal_before_child_observed=True, models=models)


def self_test():
    prompt = wake_prompt('opaque-"-child')
    assert 'multi_agent_v1__send_input({target:"opaque-\\"-child"' in prompt
    assert "spawn_agent" not in prompt and "MESIMON_PARENT_AWAKE" in prompt
    assert len(bounded_turns(dict(data=[dict(id="a"), dict(id="b")]))) == 2
    for invalid in (dict(data=[dict(id="a")]*2), dict(data=[], nextCursor="more"),
                    dict(data=[dict(id=str(index)) for index in range(3)])):
        try:
            bounded_turns(invalid)
        except AssertionError:
            continue
        raise AssertionError("budget guard accepted an invalid native turn list")
    print("Offline wakeup prompt and budget checks passed; no process or model launched")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run through ci/test-run.py with --live; at most two parent and two child Luna-low turns")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "child", "gpt-5.6-luna", 150
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-child-wakeup-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="production_native_child_wakeup", outcome="inconclusive", cleanup=False,
                    model=args.model, reasoning_effort="low", parent_turn_budget=2, child_turn_budget=2,
                    total_model_turn_budget=4, deadline_seconds=150, retries=0, checkpoints=[], cost_usd=None)
    guard = verification = None
    try:
        guard = state.probe.Guard(175, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
            manifest["parent_prompts_submitted"] = verification.calls
            try:
                verification.collect()
                verification.client.close()
            except Exception as error:
                manifest.update(outcome="inconclusive", capture_error=str(error))
        if guard:
            try:
                manifest["cleanup"] = guard.close()
            except Exception as error:
                manifest["cleanup_error"] = str(error)
        state.write(out / "manifest.json", manifest)
        print(json.dumps(dict(capture=str(out), **manifest)))
    return 0 if manifest["outcome"] == "passed" and manifest["cleanup"] else 1


if __name__ == "__main__":
    sys.exit(main())
