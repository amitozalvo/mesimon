#!/usr/bin/env python3
"""Credential-free native authentication attention through production Mesimon.

Run under ci/test-run.py. Never submits a prompt, imports existing credentials,
or completes login. The CLI boundary pins an empty private home and clears API
credentials that a login shell might have restored.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("auth_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    args = parser.parse_args()
    if not os.environ.get("MESIMON_TEST_RUN"):
        parser.error("use ci/test-run.py for supervised ownership")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "auth", "gpt-5.6-luna", 60
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-auth-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="credential_free_production_auth", outcome="inconclusive", cleanup=False,
                    model_calls=0, credentials_imported=False, checkpoints=[],
                    codex_version=subprocess.check_output([args.codex, "--version"], text=True, timeout=10).strip(),
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    guard = verification = None
    try:
        guard = state.probe.Guard(85, args.tmux)
        # Reuse fixture construction with synthetic empty authentication, never
        # the caller's auth file. No agent exists during construction.
        args.auth_file = guard.root / "empty-auth.json"
        args.auth_file.write_text("{}\n")
        args.auth_file.chmod(0o600)
        verification = state.Verification(args, out, manifest, guard)
        (verification.home / "auth.json").unlink()
        wrapper = guard.root / "codex-isolated"
        wrapper.write_text("#!" + sys.executable + "\nimport os,sys\n"
            + "for key in list(os.environ):\n"
            + " if key.startswith(('CODEX_', 'OPENAI_', 'AZURE_OPENAI_')): os.environ.pop(key)\n"
            + "os.environ['CODEX_HOME']=" + repr(str(verification.home)) + "\n"
            + "os.execv(" + repr(args.codex) + ", [" + repr(args.codex) + "] + sys.argv[1:])\n")
        wrapper.chmod(0o700)
        ticket = verification.request("create_ticket", column="TODO", title="Authentication fixture", workspace=None)["id"]
        sid = verification.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        verification.sessions.append(sid)
        session, ticket, screen = verification.wait(sid,
            "native login choices map to Auth attention and retain checkout hold",
            lambda session, ticket, screen: session["state"] == {"state":"requires_action", "reason":"auth"}
                and session.get("observation_hold") and ticket["column"] != "REVIEW"
                and ("Sign in with ChatGPT" in screen or "Sign in with an API key" in screen),
            timeout=45, consent=False)
        verification.verify_native_home(sid)
        if verification.calls or session.get("codex_turn_id") or session.get("pending_submit"):
            raise AssertionError("authentication fixture unexpectedly submitted a turn")
        histories = list(verification.home.glob("sessions/**/*.jsonl"))
        if histories:
            raise AssertionError("credential-free login created unexpected conversation history")
        manifest["outcome"] = "passed"
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
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
