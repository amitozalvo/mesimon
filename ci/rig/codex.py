"""Sequential native Codex acceptance through the production daemon.

Each case owns a disposable board and Codex home through test_guard.py. The
native terminal submits prompts; app-server RPC is used only for observations.
"""
import argparse
import datetime
import importlib.util
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
SMOKE = ("complete", "mcp", "brief", "queue", "sleep-resume", "handover")
EXTENDED = ("guard", "plan", "interrupt", "child", "stop", "compact", "identity", "question")
TURNS = {"complete": 1, "mcp": 1, "brief": 2, "queue": 2, "sleep-resume": 2,
         "handover": 2, "guard": 2, "plan": 2, "interrupt": 1, "child": 2,
         "stop": 2, "compact": 3, "identity": 2, "question": 1}
LIFECYCLE = {"stop", "compact", "identity", "question"}


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "ci" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def add_arguments(parser):
    parser.add_argument("--live", action="store_true", help="authorize Codex model calls")
    parser.add_argument("--model", help="Codex model, checked against authenticated model/list")
    parser.add_argument("--effort", help="Codex effort, checked before submission (default: low)")
    parser.add_argument("--suite", choices=("smoke", "extended"), default="smoke")
    parser.add_argument("--max-turns", type=int, help="Codex suite turn allowance (default: 12)")
    parser.add_argument("--max-tokens", type=int,
                        help="stop at observed native total tokens; not a billing cap")
    parser.add_argument("--case-timeout", type=int)


def resolve_options(args, previous):
    defaults = dict(model="gpt-6-luna", effort="low", max_turns=12, max_tokens=600000, case_timeout=180)
    for name, default in defaults.items():
        if getattr(args, name) is None:
            setattr(args, name, (previous or {}).get(name, default) if args.failed else default)


def select_cases(args, previous):
    available = SMOKE + EXTENDED
    selected = list(SMOKE if args.suite == "smoke" else available)
    if args.only:
        selected = args.only.split(",")
        unknown = set(selected) - set(available)
        if unknown or len(selected) != len(set(selected)):
            raise ValueError("unknown or duplicate Codex cases: " + args.only)
    if args.failed:
        if previous is None:
            raise ValueError("no previous Codex verdicts for --failed")
        failed = [row["case"] for row in previous["cases"] if row["outcome"] != "passed"]
        selected = [case for case in failed if case in selected] if args.only else failed
    if not 1 <= args.case_timeout <= 300 or args.max_turns < 1 or args.max_tokens < 1:
        raise ValueError("positive budgets and --case-timeout 1..300 are required")
    needed = sum(TURNS[case] for case in selected)
    if needed > args.max_turns:
        raise ValueError(f"selected cases allow {needed} turns; --max-turns is {args.max_turns}")
    return selected


def choose_model(pages, model, effort):
    models = [entry for page in pages for entry in page.get("data", [])]
    selected = next((entry for entry in models if entry.get("model") == model), None)
    if selected is None:
        raise ValueError(f"requested model {model!r} absent from authenticated model/list; no fallback")
    supported = {item["reasoningEffort"] for item in selected.get("supportedReasoningEfforts", [])}
    if effort not in supported:
        raise ValueError(f"{model} does not advertise effort {effort!r}; supported: {sorted(supported)}")
    return selected


def read_history(path, home):
    path.resolve().relative_to(home.resolve())
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("native fixture history exceeded 16 MiB")
    rows = []
    for line in path.read_text().splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            continue
    return rows


def usage_totals(rows):
    totals = [row.get("payload", {}).get("info", {}).get("total_token_usage") or {}
              for row in rows if row.get("payload", {}).get("type") == "token_count"
              and isinstance(row.get("payload", {}).get("info"), dict)]
    return {key: max((int(total.get(key, 0)) for total in totals), default=0)
            for key in ("input_tokens", "cached_input_tokens", "output_tokens", "reasoning_output_tokens", "total_tokens")}


def usage_of(rows):
    return usage_totals(rows)["total_tokens"]


def fixture_configuration(original, args):
    settings = "plan_mode_reasoning_effort = " + json.dumps(args.effort) + "\n"
    settings += original.replace('model_reasoning_effort = "low"',
                                 "model_reasoning_effort = " + json.dumps(args.effort))
    enabled = "true" if args.case == "child" else "false"
    agents = ("enabled = " + enabled + "\ndefault_subagent_model = " + json.dumps(args.model)
              + "\ndefault_subagent_reasoning_effort = " + json.dumps(args.effort) + "\n")
    if "[agents]\n" in settings:
        settings = settings.replace("[agents]\n", "[agents]\n" + agents)
    else:
        settings += "\n[agents]\n" + agents
    return settings + "\n[features]\nmulti_agent = " + enabled + "\n"


def adapter(base):
    class Verification(base):
        def __init__(self, args, out, manifest, guard):
            super().__init__(args, out, manifest, guard)
            self.last_usage_check = 0
            self.observed_tokens = 0
            config = self.home / "config.toml"
            config.write_text(fixture_configuration(config.read_text(), args))
            wrapper = guard.root / "codex-isolated"
            original = wrapper.read_text()
            wrapper.write_text(original.replace("#!/bin/sh\n", "#!/bin/sh\n" +
                "unset OPENAI_API_KEY OPENAI_BASE_URL CODEX_API_KEY AZURE_OPENAI_API_KEY\n", 1))
            if args.case in ("mcp", "brief"):
                for column in self.client.board()["columns"]:
                    settings = {key: value for key, value in column.items() if key not in ("name", "order")}
                    settings["agent_tools"] = "read"
                    self.request("set_column_settings", name=column["name"], settings=settings)

        def preflight(self):
            endpoint = self.guard.root / "preflight.sock"
            executable = self.guard.root / "codex-isolated"
            pid = self.guard.request(op="spawn", argv=[str(executable), "app-server", "--listen",
                                     "unix://" + str(endpoint)], env=self.env)
            deadline = time.monotonic() + 20
            while not endpoint.exists():
                if time.monotonic() > deadline or self.guard.request(op="poll", pid=pid) is not None:
                    raise RuntimeError("model preflight app-server did not start")
                time.sleep(0.1)
            probe = module("rig_preflight", "codex-runtime-probe.py")
            rpc = probe.Rpc(endpoint, self.out, "preflight")
            try:
                rpc.initialize()
                response = rpc.call("config/read", {"includeLayers": False, "cwd": str(self.repo)})
                configuration = response.get("result", {}).get("config", {})
                if (configuration.get("model") != self.args.model
                        or configuration.get("model_reasoning_effort") != self.args.effort
                        or configuration.get("plan_mode_reasoning_effort") != self.args.effort
                        or configuration.get("agents", {}).get("enabled") != (self.args.case == "child")):
                    raise RuntimeError("native configuration did not confirm the fixture model, efforts and child policy")
                pages, cursor = [], None
                for _ in range(20):
                    params = {"includeHidden": False, "limit": 100}
                    if cursor:
                        params["cursor"] = cursor
                    response = rpc.call("model/list", params)
                    if "error" in response:
                        raise RuntimeError("model/list refused: " + str(response["error"]))
                    pages.append(response["result"])
                    cursor = response["result"].get("nextCursor")
                    if not cursor:
                        break
                else:
                    raise RuntimeError("model/list pagination exceeded its bound")
                self.manifest["selected_model"] = choose_model(pages, self.args.model, self.args.effort)
            finally:
                rpc.close()
            self.mark(assertion="authenticated model and effort available before any turn", passed=True)

        def reserve_turn(self):
            if self.calls >= self.args.user_turn_budget:
                raise AssertionError("case submission budget exhausted")
            self.check_usage(force=True)

        def start(self, prompt):
            self.reserve_turn()
            return super().start(prompt)

        def ask(self, sid, prompt):
            self.reserve_turn()
            return super().ask(sid, prompt)

        def check_usage(self, force=False):
            if not force and time.monotonic() - self.last_usage_check < 1:
                return
            self.last_usage_check = time.monotonic()
            histories = list(self.home.glob("sessions/**/*.jsonl"))
            totals = usage_totals([])
            for path in histories:
                reading = usage_totals(read_history(path, self.home))
                for key in totals:
                    totals[key] += reading[key]
            self.observed_tokens = totals["total_tokens"]
            self.manifest["observed_total_tokens"] = self.observed_tokens
            self.manifest["observed_usage"] = totals
            if self.observed_tokens >= self.args.remaining_tokens:
                raise AssertionError("observed token allowance exhausted; stopping owned fixture")

        def observe(self, sid):
            self.check_usage()
            return super().observe(sid)

        def mark(self, **fields):
            super().mark(**fields)
            if fields.get("assertion"):
                print("  " + fields["assertion"], flush=True)

        def run(self):
            if self.args.case == "interrupt":
                sid = self.start("Run sleep 25 once, then reply exactly MESIMON_INTERRUPTED. No other tools.")
                self.wait(sid, "native model activity precedes interruption", lambda session, ticket, screen:
                          session["state"]["state"] == "running" and self.observed_tokens > 0
                          and session.get("codex_turn_id"))
                self.tm("send-keys", "-t", sid.replace("-", "")[:16], "Escape")
                self.wait(sid, "interruption never promotes to REVIEW", lambda session, ticket, screen:
                          session["state"] == {"state": "idle", "stop_reason": "interrupted"}
                          and ticket["column"] == "IN PROGRESS")
                return
            if self.args.case != "queue":
                return super().run()
            sid = self.start("Run sleep 8 once, then reply exactly MESIMON_QUEUE_FIRST. No other tools.")
            session, _, _ = self.wait(sid, "queue fixture is working", lambda session, ticket, screen:
                                      session["state"]["state"] == "running")
            thread = session["codex_thread_id"]
            self.reserve_turn()
            self.calls += 1
            receipt = self.request("prompt_session", ticket=session["ticket"], queued=True,
                                   text="Reply exactly MESIMON_QUEUE_SECOND. Do not use tools.")
            if receipt.get("resp") != "queued":
                raise AssertionError("running turn did not queue the second prompt: " + str(receipt))
            final, _, _ = self.completed(sid, "MESIMON_QUEUE_SECOND")
            if final["codex_thread_id"] != thread:
                raise AssertionError("queued prompt changed native thread")
            rows = read_history(Path(final["transcript_path"]), self.home)
            messages = [row["payload"] for row in rows if row.get("type") == "response_item"
                        and row.get("payload", {}).get("role") == "assistant"]
            replies = [" ".join(part.get("text", "") for part in message.get("content", []))
                       for message in messages if message.get("phase") in (None, "final_answer")]
            first = [index for index, text in enumerate(replies) if text.strip() == "MESIMON_QUEUE_FIRST"]
            second = [index for index, text in enumerate(replies) if text.strip() == "MESIMON_QUEUE_SECOND"]
            if len(first) != 1 or len(second) != 1 or first[0] >= second[0]:
                raise AssertionError("queued turn did not produce exactly one ordered pair of final replies")
            self.mark(assertion="queued prompt completes once on the same thread after the first reply", passed=True)

        def audit(self):
            self.check_usage(force=True)
            if not self.observed_tokens:
                raise RuntimeError("native usage unavailable after model activity; cannot verify budget")
            contexts = []
            for path in self.home.glob("sessions/**/*.jsonl"):
                contexts.extend(row["payload"] for row in read_history(path, self.home)
                                if row.get("type") == "turn_context")
            if not contexts or any(row.get("model") != self.args.model or
                                   row.get("effort", row.get("reasoning_effort")) != self.args.effort
                                   for row in contexts):
                raise AssertionError("native turn contexts did not confirm the selected model and effort")
            self.manifest["submitted_user_turns"] = self.calls
            self.mark(assertion="actual native turns used the selected model and effort", passed=True)

    return Verification


def replace_text(path, text):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(text)
    temporary.replace(path)


def report(directory, manifest):
    directory.mkdir(parents=True, exist_ok=True)
    replace_text(directory / "verdicts.json", json.dumps(manifest, indent=2) + "\n")
    lines = ["# Codex rig", "", f"{manifest['version']} · {manifest['model']} · {manifest['effort']}",
             "", "| Case | Outcome | Submitted turns | Observed tokens | Cleanup |",
             "| --- | --- | ---: | ---: | --- |"]
    for row in manifest["cases"]:
        lines.append(f"| {row['case']} | {row['outcome']} | {row.get('submitted_user_turns', 0)} | "
                     f"{row.get('observed_total_tokens', 0)} | {row.get('cleanup', False)} |")
    for row in manifest["cases"]:
        for key in ("error", "capture_error", "cleanup_error"):
            if row.get(key):
                lines.extend(["", f"{row['case']} ({key}): {row[key]}", ""])
    lines.extend(["", "Token readings may lag and omit native overhead; no dollar cap is claimed.",
                  "Codex questions and plans remain the person's; crown dialog automation is unsupported."])
    replace_text(directory / "verdicts.md", "\n".join(lines) + "\n")


def run(repo, args, parser):
    if args.reset or args.stale or args.flags_off:
        parser.error("Codex fixtures are supervised and reaped after every case; Claude maintenance flags do not apply")
    out = Path(repo) / "target/rig/codex"
    previous = json.loads((out / "verdicts.json").read_text()) if (out / "verdicts.json").exists() else None
    resolve_options(args, previous)
    try:
        cases = select_cases(args, previous)
    except (ValueError, KeyError) as error:
        parser.error(str(error))
    model = args.model
    print(f"Codex rig: {', '.join(cases) or 'nothing to rerun'}; {model}, {args.effort}; "
          f"at most {sum(TURNS[case] for case in cases)} scenario turns", flush=True)
    if args.lay or not cases:
        return 0
    if not args.live:
        parser.error("Codex model calls require --live; --lay lists the selection without starting anything")
    if not os.environ.get("MESIMON_TEST_RUN"):
        command = [sys.executable, "-B", str(ROOT / "ci/test-run.py"), "--timeout",
                   str(min(1200, 120 + len(cases) * (args.case_timeout + 30))), "--", sys.executable,
                   "-B", str(ROOT / "ci/rig.py"), *sys.argv[1:]]
        os.execv(sys.executable, command)
    binary = Path(repo) / "target/debug/mesimon"
    if not args.no_build:
        subprocess.run(["cargo", "build", "-p", "mesimon"], cwd=repo, check=True)
    binary = binary.resolve(strict=True)
    executable = shutil.which("codex")
    tmux = os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux")
    if not executable or not tmux:
        parser.error("installed Codex and tmux are required")
    state = module("rig_codex_state", "codex-state-e2e.py")
    version = subprocess.check_output([executable, "--version"], text=True, timeout=10).strip()
    run_id = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    directory = out / run_id
    directory.mkdir(parents=True, mode=0o700)
    manifest = dict(version=version, model=model, effort=args.effort, run_id=run_id,
                    mesimon_sha256=state.probe.digest(binary), max_turns=args.max_turns,
                    runner_sha256={name: state.probe.digest(ROOT / name) for name in
                                   ("ci/rig.py", "ci/rig/codex.py", "ci/codex-state-e2e.py", "ci/codex-lifecycle-e2e.py")},
                    max_tokens=args.max_tokens, case_timeout=args.case_timeout,
                    cases=[dict(case=case, outcome="not_run", cleanup=False,
                                                          checkpoints=[]) for case in cases])
    report(directory, manifest)
    report(out, manifest)
    used_tokens = 0
    stopped = False
    for result in manifest["cases"]:
        case = result["case"]
        if stopped:
            report(directory, manifest)
            report(out, manifest)
            continue
        print(f"\n▶ Codex {case}", flush=True)
        capture = directory / case
        capture.mkdir(mode=0o700)
        result["capture"] = str(capture)
        result["outcome"] = "running"
        report(directory, manifest)
        report(out, manifest)
        case_args = argparse.Namespace(case=case, model=model, effort=args.effort, timeout=args.case_timeout,
            binary=binary, codex=executable, tmux=tmux, claude=None, claude_model="haiku",
            auth_file=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json",
            user_turn_budget=TURNS[case] - int(case in ("child", "stop", "compact")),
            remaining_tokens=args.max_tokens - used_tokens)
        guard = verification = None
        try:
            guard = state.probe.Guard(args.case_timeout + 25, tmux)
            base = module("rig_codex_lifecycle", "codex-lifecycle-e2e.py").Verification if case in LIFECYCLE else state.Verification
            verification = adapter(base)(case_args, capture, result, guard)
            print("  Watch from an external terminal: cd " + shlex.quote(str(verification.repo))
                  + " && " + shlex.quote(str(binary)), flush=True)
            verification.preflight()
            verification.run()
            verification.audit()
            if any(point.get("passed") is False for point in result["checkpoints"]):
                raise AssertionError("a native checkpoint failed")
            result["outcome"] = "passed"
            result.pop("error", None)
        except AssertionError as error:
            result.update(outcome="failed", error=str(error))
        except Exception as error:
            result.update(outcome="inconclusive", error=type(error).__name__ + ": " + str(error))
        finally:
            if verification:
                result["submitted_user_turns"] = verification.calls
                try:
                    verification.collect()
                    state.write(capture / "board.json", verification.client.board())
                except Exception as error:
                    result.update(outcome="inconclusive", capture_error=str(error))
                finally:
                    try:
                        verification.client.close()
                    except Exception as error:
                        result.update(outcome="inconclusive", capture_error=str(error))
            if guard:
                try:
                    result["cleanup"] = guard.close()
                except Exception as error:
                    result["cleanup_error"] = str(error)
            if not result["cleanup"]:
                result["outcome"] = "inconclusive"
            state.write(capture / "manifest.json", result)
            used_tokens += result.get("observed_total_tokens", 0)
            stopped = result["outcome"] != "passed" or used_tokens >= args.max_tokens
            report(directory, manifest)
            report(out, manifest)
            detail = result.get("error") or result.get("capture_error") or result.get("cleanup_error") or ""
            print(f"  {result['outcome']} · cleanup {result['cleanup']} · {detail}", flush=True)
    print("\nVerdicts: " + str(out / "verdicts.md"), flush=True)
    return int(any(row["outcome"] != "passed" for row in manifest["cases"]))
