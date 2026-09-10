#!/usr/bin/env python3
"""One-turn, isolated production MCP form elicitation acceptance.

Run explicitly through ci/test-run.py with --live. The generated stdio server
implements the official MCP 2025-11-25 elicitation/create form protocol:
https://modelcontextprotocol.io/specification/2025-11-25/client/elicitation
Native form keys follow installed Codex 0.153.4's mcp_server_elicitation.rs:
one selected enum field is submitted by Enter; Escape cancels it. No observer
answers an RPC. --answer defaults to accept; each case permits one model turn.
"""
import argparse
import datetime
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("elicitation_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)

SERVER = r'''import json,signal,sys,time
from pathlib import Path

TOOL = {"name":"fixture_confirmation",
        "description":"Returns a disposable fixture confirmation selected in a one-field form.",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":False},
        "annotations":{"readOnlyHint":True,"destructiveHint":False,"openWorldHint":False}}
FORM = {"mode":"form","message":"Disposable Mesimon fixture confirmation.",
        "requestedSchema":{"type":"object","properties":{"confirmation":{
            "type":"string","title":"Confirmation","enum":["ACCEPT_FIXTURE"],
            "default":"ACCEPT_FIXTURE"}},"required":["confirmation"]}}

def serve(source, sink, audit):
    pending = None
    calls = 0
    supports_form = False
    def send(value):
        audit({"direction":"server_to_client","frame":value})
        sink.write(json.dumps(value)+"\n")
        sink.flush()
    def result(ident, value):
        send({"jsonrpc":"2.0","id":ident,"result":value})
    for index, line in enumerate(source):
        if index >= 128 or len(line) > 32768:
            raise RuntimeError("fixture MCP message bound exceeded")
        frame = json.loads(line)
        audit({"direction":"client_to_server","frame":frame})
        method, ident = frame.get("method"), frame.get("id")
        if method == "initialize":
            capability = frame.get("params",{}).get("capabilities",{}).get("elicitation")
            supports_form = isinstance(capability,dict) and (not capability or "form" in capability)
            result(ident, {"protocolVersion":"2025-11-25","capabilities":{"tools":{}},
                           "serverInfo":{"name":"elicitation_fixture","version":"1"}})
        elif method == "tools/list":
            result(ident, {"tools":[TOOL]})
        elif method == "ping":
            result(ident, {})
        elif method == "tools/call":
            calls += 1
            if calls != 1 or frame.get("params",{}).get("name") != TOOL["name"] or not supports_form:
                result(ident, {"isError":True,"content":[{"type":"text","text":"Fixture call or capability bound rejected."}]})
                continue
            pending = ident
            send({"jsonrpc":"2.0","id":"fixture-form-1","method":"elicitation/create","params":FORM})
        elif method is None and ident == "fixture-form-1" and pending is not None:
            answer = frame.get("result",{})
            accepted = answer.get("action") == "accept" and answer.get("content") == {"confirmation":"ACCEPT_FIXTURE"}
            cancelled = answer.get("action") == "cancel" and answer.get("content") in (None, {})
            result(pending, {"isError":not (accepted or cancelled),"content":[{"type":"text",
                "text":"MESIMON_FORM_ACCEPTED" if accepted else "MESIMON_FORM_CANCELLED" if cancelled else "Fixture form response was invalid."}]})
            pending = None
        elif method and ident is not None:
            send({"jsonrpc":"2.0","id":ident,"error":{"code":-32601,"message":"Method not found"}})

if __name__ == "__main__":
    signal.alarm(125)
    path = Path(sys.argv[1])
    def audit(value):
        with path.open("a") as log:
            log.write(json.dumps(dict(at_ms=time.time_ns()//1000000, **value))+"\n")
    serve(sys.stdin, sys.stdout, audit)
'''


def protocol_self_test():
    namespace = {"__name__": "fixture_protocol_test"}
    exec(compile(SERVER, "fixture-mcp.py", "exec"), namespace)
    messages = [
        {"id": 1, "method": "initialize", "params": {"capabilities": {"elicitation": {"form": {}}}}},
        {"id": 2, "method": "tools/list"},
        {"id": 3, "method": "tools/call", "params": {"name": "fixture_confirmation", "arguments": {}}},
        {"id": "fixture-form-1", "result": {"action": "accept", "content": {"confirmation": "ACCEPT_FIXTURE"}}},
    ]
    for action, content, marker in [
        ("accept", {"confirmation":"ACCEPT_FIXTURE"}, "MESIMON_FORM_ACCEPTED"),
        ("cancel", None, "MESIMON_FORM_CANCELLED"),
    ]:
        answer = {"action":action}
        if content is not None:
            answer["content"] = content
        trial = messages[:-1] + [{"id":"fixture-form-1","result":answer}]
        output, audit = io.StringIO(), []
        namespace["serve"](io.StringIO("".join(json.dumps(value)+"\n" for value in trial)), output, audit.append)
        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        form = next(frame for frame in frames if frame.get("method") == "elicitation/create")
        assert form["params"]["mode"] == "form" and "_meta" not in form["params"]
        assert frames[-1]["id"] == 3 and not frames[-1]["result"]["isError"]
        assert frames[-1]["result"]["content"][0]["text"] == marker
        validate_answer(answer, action)
        # Both answer paths enforce the same one-call bound and capabilities.
        for extra, capability in [(True, {"elicitation": {}}), (False, {})]:
            bounded = [dict(messages[0], params={"capabilities": capability}), messages[2]]
            if extra:
                bounded += [trial[-1], dict(messages[2], id=4)]
            output = io.StringIO()
            namespace["serve"](io.StringIO("".join(json.dumps(value)+"\n" for value in bounded)), output, lambda _: None)
            frames = [json.loads(line) for line in output.getvalue().splitlines()]
            assert sum(frame.get("method") == "elicitation/create" for frame in frames) == int(extra)
            assert frames[-1]["result"]["isError"]
    # A cancel response carrying accepted form content is not cancellation.
    invalid = {"action":"cancel","content":{"confirmation":"ACCEPT_FIXTURE"}}
    try:
        validate_answer(invalid, "cancel")
    except AssertionError:
        pass
    else:
        raise AssertionError("accepted content was allowed in a cancel response")
    output = io.StringIO()
    trial = messages[:-1] + [{"id":"fixture-form-1","result":invalid}]
    namespace["serve"](io.StringIO("".join(json.dumps(value)+"\n" for value in trial)), output, lambda _: None)
    assert json.loads(output.getvalue().splitlines()[-1])["result"]["isError"]
    print("Generated MCP protocol self-test passed; no process or model launched")


def validate_answer(answer, expected):
    if answer.get("action") != expected:
        raise AssertionError("native form response does not match the requested user gesture")
    if expected == "accept" and answer.get("content") != {"confirmation":"ACCEPT_FIXTURE"}:
        raise AssertionError("native form did not deliver the selected fixture answer")
    if expected == "cancel" and answer.get("content") not in (None, {}):
        raise AssertionError("cancelled native form delivered accepted content")


class Verification(state.Verification):
    def __init__(self, args, out, manifest, guard):
        super().__init__(args, out, manifest, guard)
        self.tool_log = guard.root / "elicitation-mcp.jsonl"
        self.tool_script = guard.root / "elicitation-mcp.py"
        self.tool_script.write_text(SERVER)
        self.tool_script.chmod(0o700)
        with (self.home / "config.toml").open("a") as config:
            config.write("[mcp_servers.elicitation_fixture]\ncommand = " + json.dumps(sys.executable)
                         + "\nargs = " + json.dumps([str(self.tool_script), str(self.tool_log)]) + "\n")
        self.approved_tool = False

    def frames(self):
        if not self.tool_log.exists():
            return []
        result = []
        for line in self.tool_log.read_text().splitlines():
            try:
                result.append(json.loads(line))
            except ValueError:
                pass  # The owned stdio server may still be appending its last line.
        return result

    def startup_consent(self, sid, screen):
        super().startup_consent(sid, screen)
        session = next(row for row in self.client.board()["sessions"] if row["id"] == sid)
        if (not self.approved_tool and session["state"] == {"state":"requires_action", "reason":"permission"}
                and "fixture_confirmation" in screen and "elicitation_fixture" in screen and "Allow" in screen):
            self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Enter")
            self.approved_tool = True
            self.mark(action="native_allow_generated_fixture_tool_once", session=sid)

    def run(self):
        ticket = self.request("create_ticket", column="TODO", title="Elicitation fixture", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        self.wait(sid, "native composer ready before the single paid elicitation turn", lambda session,ticket,screen:
            session["state"]["state"] == "idle" and not session.get("pending_prefill", True)
            and not session.get("pending_submit") and not session.get("observation_hold", True))
        def initialized():
            frames = self.frames()
            initialize = next((row["frame"] for row in frames if row["frame"].get("method") == "initialize"), None)
            listed = any(row["direction"] == "server_to_client" and row["frame"].get("result",{}).get("tools") for row in frames)
            if not initialize or not listed:
                return False
            capability = initialize.get("params",{}).get("capabilities",{}).get("elicitation")
            if not isinstance(capability, dict) or (capability and "form" not in capability):
                raise state.Inconclusive("native client did not advertise MCP form support; no turn submitted")
            return True
        state.lab.wait_for(initialized, timeout=15)
        self.verify_native_home(sid)
        self.mark(assertion="generated MCP tool and native form capability verified before model submission", passed=True)
        completion_marker = "MESIMON_ELICITATION_DONE" if self.args.answer == "accept" else "MESIMON_ELICITATION_CANCEL_DONE"
        self.ask(sid, "Call elicitation_fixture fixture_confirmation once. After its form is answered or cancelled, reply "
                 + completion_marker + ". Use no other tools and do not retry the form.")
        held, _, _ = self.wait(sid, "genuine MCP form maps to Elicitation and holds checkout", lambda session,ticket,screen:
            session["state"] == {"state":"requires_action", "reason":"elicitation"}
            and session.get("observation_hold") and ticket["column"] == "IN PROGRESS"
            and "Disposable Mesimon fixture confirmation." in screen
            and "Confirmation" in screen and "ACCEPT_FIXTURE" in screen)
        requests = [row["frame"] for row in self.frames() if row["direction"] == "server_to_client"
                    and row["frame"].get("method") == "elicitation/create"]
        if len(requests) != 1 or requests[0]["params"].get("mode") != "form" or "_meta" in requests[0]["params"]:
            raise AssertionError("expected exactly one genuine form without native tool-approval metadata")
        turn = held["codex_turn_id"]
        end = time.monotonic() + 1
        while time.monotonic() < end:
            session, ticket_row, _ = self.observe(sid)
            if (session["state"] != held["state"] or not session.get("observation_hold")
                    or session.get("codex_turn_id") != turn or ticket_row["column"] != "IN PROGRESS"):
                raise AssertionError("unanswered native form released checkout or changed the turn")
            if any(row["direction"] == "client_to_server" and row["frame"].get("id") == "fixture-form-1" for row in self.frames()):
                raise AssertionError("an observer answered the form before native user input")
            time.sleep(0.1)
        # Native Enter submits the default enum; native Escape cancels.
        # The observer never sends an elicitation response.
        key = "Enter" if self.args.answer == "accept" else "Escape"
        self.tm("send-keys", "-t", sid.replace("-", "")[:16], key)
        self.mark(action="native_" + self.args.answer + "_fixture_form_once", session=sid)
        def answered():
            response = next((row["frame"] for row in self.frames() if row["direction"] == "client_to_server"
                             and row["frame"].get("id") == "fixture-form-1"), None)
            if not response:
                return False
            result = response.get("result", {})
            validate_answer(result, self.args.answer)
            return True
        state.lab.wait_for(answered, timeout=10)
        completed, _, _ = self.completed(sid, completion_marker)
        if self.calls != 1 or completed.get("codex_turn_id") != turn:
            raise AssertionError("elicitation used more than its one foreground turn")
        calls = [row for row in self.frames() if row["frame"].get("method") == "tools/call"]
        if len(calls) != 1:
            raise AssertionError("model called the fixture tool more than once")
        reply = next((row["frame"].get("result", {}) for row in self.frames()
                      if row["direction"] == "server_to_client" and row["frame"].get("id") == calls[0]["frame"]["id"]), {})
        tool_marker = "MESIMON_FORM_ACCEPTED" if self.args.answer == "accept" else "MESIMON_FORM_CANCELLED"
        if reply.get("isError") or not any(item.get("text") == tool_marker for item in reply.get("content", [])):
            raise AssertionError("native form outcome did not complete the actual MCP tool")
        self.mark(assertion="native form answer reaches MCP tool and the same turn completes afterward", passed=True,
                  model_turns=1, turn_id=turn, answer=self.args.answer)

    def collect(self):
        if self.tool_log.exists():
            shutil.copyfile(self.tool_log, self.out / "elicitation-mcp.jsonl")
        super().collect()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--answer", choices=("accept", "cancel"), default="accept")
    parser.add_argument("--self-test", action="store_true", help="check generated protocol in memory without any process or model")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    parser.add_argument("--auth-file", type=Path, default=Path(os.environ.get("CODEX_HOME", str(Path.home()/".codex"))) / "auth.json")
    args = parser.parse_args()
    if args.self_test:
        protocol_self_test()
        return 0
    if not args.live or not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("run through ci/test-run.py with --live; exactly one Luna turn is permitted")
    if not args.codex or not args.tmux:
        parser.error("installed Codex and tmux are required")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "elicitation", "gpt-5.6-luna", 120
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-elicitation-" + args.answer + "-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="production_native_mcp_elicitation", outcome="inconclusive", cleanup=False,
                    model=args.model, reasoning_effort="low", answer=args.answer,
                    model_turn_budget=1, checkpoints=[], cost_usd=None)
    guard = verification = None
    try:
        guard = state.probe.Guard(145, args.tmux)
        verification = Verification(args, out, manifest, guard)
        verification.run()
        manifest["outcome"] = "passed"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
            manifest["model_turns_submitted"] = verification.calls
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
