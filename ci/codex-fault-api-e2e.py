#!/usr/bin/env python3
"""Credential-free, locally injected HTTP faults through native Codex/Mesimon.

This is NOT a real account limit/authentication test or a simulated successful
model response. An owned loopback Responses endpoint always returns 401 or 429.
No existing authentication is imported. One native user prompt is permitted.
Run under ci/test-run.py; --self-test performs no I/O or process launch.

Provider fields follow the official Codex custom-provider configuration:
https://learn.chatgpt.com/docs/config-file/config-advanced#custom-model-providers
"""
import argparse
import datetime
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import threading
import time
from urllib.parse import urlsplit
import uuid

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("local_fault_state", ROOT / "ci/codex-state-e2e.py")
state = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(state)
KEY = "mesimon-local-injected-fault-key"
MAX_REQUESTS = 4


def error_body(status):
    if status == 429:
        error = dict(message="LOCAL_INJECTED_RATE_LIMIT", type="rate_limit_error", code="rate_limit_exceeded")
    elif status == 401:
        error = dict(message="LOCAL_INJECTED_EXPIRED_AUTH", type="authentication_error", code="invalid_api_key")
    else:
        raise ValueError("only the explicitly injected error statuses are supported")
    return json.dumps({"error": error}).encode()


def provider_config(base_url, status):
    endpoint = urlsplit(base_url)
    if (endpoint.scheme != "http" or endpoint.hostname != "127.0.0.1" or not endpoint.port
            or endpoint.path != "/v1" or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment):
        raise ValueError("fault provider must be the owned numeric loopback endpoint")
    retries = 1 if status == 429 else 0
    return ('model_provider = "fixture_fault"\n'
            '[model_providers.fixture_fault]\nname = "Local injected fault only"\n'
            + "base_url = " + json.dumps(base_url) + "\n"
            'wire_api = "responses"\nenv_key = "MESIMON_FAULT_API_KEY"\n'
            'requires_openai_auth = false\nsupports_websockets = false\n'
            + f"request_max_retries = {retries}\nstream_max_retries = {retries}\n"
            + 'stream_idle_timeout_ms = 2000\n')


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def handle_fault(self):
        fixture = self.server.fixture
        self.connection.settimeout(2)
        path = urlsplit(self.path)
        local = not path.netloc and path.path == "/v1/responses" and self.command == "POST"
        # Consume bounded input for a reliable HTTP error, but never retain
        # the request body or request headers in the capture.
        size = int(self.headers.get("Content-Length", "0"))
        if size < 0 or size > 2 * 1024 * 1024 or self.headers.get("Transfer-Encoding"):
            local = False
        elif size:
            self.rfile.read(size)
        auth_matches = self.headers.get("Authorization") == "Bearer " + KEY
        with fixture.lock:
            ordinal = sum(row["local_responses_request"] for row in fixture.requests) + int(local)
            bounded = len(fixture.requests) < 32 and ordinal <= MAX_REQUESTS
            status = fixture.status if local and bounded else 403
            row = dict(at_ms=time.time_ns() // 1000000, method=self.command,
                       path=path.path, local_responses_request=local, status=status,
                       synthetic_key_matches=auth_matches if local else None,
                       body_bytes=size, request_bound_exceeded=not bounded)
            fixture.requests.append(row)
        body = error_body(status) if status in (401, 429) else b'{"error":{"message":"LOCAL_PROXY_DENIED"}}'
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        if status == 429:
            self.send_header("Retry-After", "0")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    do_POST = handle_fault
    do_GET = handle_fault
    do_CONNECT = handle_fault


class Endpoint:
    def __init__(self, status):
        self.status, self.requests, self.lock = status, [], threading.Lock()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.server.fixture = self
        self.origin = "http://127.0.0.1:" + str(self.server.server_port)
        self.base_url = self.origin + "/v1"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def rows(self):
        with self.lock:
            return list(self.requests)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        return not self.thread.is_alive()


class Verification(state.Verification):
    def __init__(self, args, out, manifest, guard, endpoint):
        super().__init__(args, out, manifest, guard)
        self.endpoint = endpoint
        (self.home / "auth.json").unlink()
        with (self.home / "config.toml").open("a") as config:
            config.write(provider_config(endpoint.base_url, endpoint.status))
        wrapper = guard.root / "codex-isolated"
        # Pin at the final CLI boundary, after Mesimon's login-shell capture.
        # All HTTP(S) proxy traffic is sent to our refusing local endpoint;
        # this fixture never forwards a connection to an external destination.
        wrapper.write_text("#!" + sys.executable + "\nimport os,sys\n"
            + "for key in list(os.environ):\n"
            + " if key.startswith(('CODEX_', 'OPENAI_', 'AZURE_OPENAI_', 'CHATGPT_')) or key.lower().endswith('_proxy'): os.environ.pop(key)\n"
            + "os.environ['CODEX_HOME']=" + repr(str(self.home)) + "\n"
            + "os.environ['MESIMON_FAULT_API_KEY']=" + repr(KEY) + "\n"
            + "for key in ('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','http_proxy','https_proxy','all_proxy'): os.environ[key]=" + repr(endpoint.origin) + "\n"
            + "os.environ['NO_PROXY']=os.environ['no_proxy']='127.0.0.1,localhost,::1'\n"
            + "os.execv(" + repr(args.codex) + ", [" + repr(args.codex) + "] + sys.argv[1:])\n")
        wrapper.chmod(0o700)

    def verify_native_home(self, sid):
        super().verify_native_home(sid)
        value = self.native_read(sid, "config/read", {"includeLayers": True, "cwd": str(self.repo)})["config"]
        provider = value.get("model_providers", {}).get("fixture_fault", {})
        expected = dict(base_url=self.endpoint.base_url, wire_api="responses", env_key="MESIMON_FAULT_API_KEY",
                        requires_openai_auth=False, supports_websockets=False,
                        request_max_retries=int(self.endpoint.status == 429), stream_max_retries=int(self.endpoint.status == 429))
        if value.get("model_provider") != "fixture_fault" or any(provider.get(key) != item for key, item in expected.items()):
            raise AssertionError("actual native model provider is not the bounded credential-free local fault endpoint")
        if (self.home / "auth.json").exists():
            raise AssertionError("local fault fixture unexpectedly acquired authentication")

    def run(self):
        ticket = self.request("create_ticket", column="TODO", title="Local injected HTTP fault", workspace=None)["id"]
        sid = self.request("spawn_session", ticket=ticket, kind="claude", submit_prompt=False)["id"]
        self.sessions.append(sid)
        self.wait(sid, "native composer ready without a real account", lambda session,ticket,screen:
            session["state"]["state"] == "idle" and not session.get("pending_prefill", True)
            and not session.get("pending_submit") and not session.get("observation_hold", True))
        self.verify_native_home(sid)
        self.mark(assertion="effective provider points only to owned loopback fault endpoint before input", passed=True,
                  base_url=self.endpoint.base_url, injected_status=self.endpoint.status, credentials_imported=False)
        self.ask(sid, "Reply OK. Do not use tools.")
        while time.monotonic() - self.started < self.args.timeout:
            session, ticket_row, screen = self.observe(sid)
            requests = self.endpoint.rows()
            if any(row["request_bound_exceeded"] for row in requests):
                raise AssertionError("native API requests exceeded the explicit local fault bound")
            if session["state"] == {"state":"idle", "stop_reason":"end_turn"} or ticket_row["column"] == "REVIEW":
                raise AssertionError("injected API failure was reported as successful completion")
            failure = session["state"]["state"] == "failed"
            auth_attention = session["state"] == {"state":"requires_action", "reason":"auth"}
            if failure or auth_attention:
                evidence = ("429", "rate limit", "LOCAL_INJECTED_RATE_LIMIT") if self.endpoint.status == 429 else ("401", "authentication", "LOCAL_INJECTED_EXPIRED_AUTH")
                if not any(marker.lower() in screen.lower() for marker in evidence):
                    raise state.Inconclusive("native fault state was observed without a matching visible HTTP error")
                if auth_attention and not session.get("observation_hold"):
                    raise AssertionError("unresolved API authentication attention released checkout")
                break
            if session["state"]["state"] == "exited":
                raise state.Inconclusive("native terminal exited before structured API failure was captured")
            time.sleep(0.1)
        else:
            raise state.Inconclusive("native local API failure did not settle within the 90-second deadline")
        local = [row for row in self.endpoint.rows() if row["local_responses_request"]]
        if not 1 <= len(local) <= MAX_REQUESTS or any(not row["synthetic_key_matches"] or row["status"] != self.endpoint.status for row in local):
            raise AssertionError("expected bounded native Responses requests with only the synthetic local key")
        thread = session.get("codex_thread_id")
        turns = self.native_read(sid, "thread/turns/list", {"threadId": thread, "limit": 3, "itemsView": "full"})
        state.write(self.out / "native-failed-turns.json", turns)
        if (self.calls != 1 or turns.get("nextCursor") or len(turns.get("data", [])) != 1
                or turns["data"][0].get("status") != "failed"):
            raise AssertionError("native turn history does not prove one failed local prompt")
        if any(item.get("type") == "agentMessage" for item in turns["data"][0].get("items", [])):
            raise AssertionError("fault-only endpoint unexpectedly produced an assistant response")
        self.mark(assertion="actual native CLI reports locally injected HTTP failure without success or automove", passed=True,
                  injected_status=self.endpoint.status, local_requests=len(local), native_state=session["state"],
                  prompt_submissions=1, billable_model_calls=0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--status", type=int, choices=(401, 429), default=429)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mesimon")
    parser.add_argument("--codex", default=shutil.which("codex"))
    parser.add_argument("--tmux", default=os.environ.get("MESIMON_TMUX_BIN") or shutil.which("tmux"))
    args = parser.parse_args()
    if args.self_test:
        for status in (401, 429):
            value = json.loads(error_body(status))
            assert list(value) == ["error"] and value["error"]["message"].startswith("LOCAL_INJECTED_")
        import tomllib
        for status in (401, 429):
            config = tomllib.loads(provider_config("http://127.0.0.1:32123/v1", status))
            provider = config["model_providers"]["fixture_fault"]
            assert provider["base_url"] == "http://127.0.0.1:32123/v1"
            assert provider["request_max_retries"] == int(status == 429)
            assert provider["requires_openai_auth"] is False
        try:
            provider_config("https://api.openai.com/v1", 429)
        except ValueError:
            pass
        else:
            raise AssertionError("fault provider accepted an external endpoint")
        print("Fault payload/config/isolation self-tests passed; no process, socket, credentials, or model used")
        return 0
    if not os.environ.get("MESIMON_TEST_RUN") or not args.codex or not args.tmux:
        parser.error("run under ci/test-run.py with installed Codex and tmux")
    args.binary = args.binary.resolve(strict=True)
    args.case, args.model, args.timeout = "fault-api", "gpt-5.6-luna", 90
    out = ROOT / "target/state-lab/captures" / (datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-codex-injected-http-" + uuid.uuid4().hex[:8])
    out.mkdir(parents=True, mode=0o700)
    manifest = dict(kind="locally_injected_native_http_fault", outcome="inconclusive", cleanup=False,
                    injected_status=args.status, billable_model_calls=0, credentials_imported=False,
                    successful_model_responses=0, checkpoints=[], request_bound=MAX_REQUESTS,
                    mesimon_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    guard = verification = endpoint = None
    try:
        guard = state.probe.Guard(115, args.tmux)
        args.auth_file = guard.root / "empty-auth.json"
        args.auth_file.write_text("{}\n")
        args.auth_file.chmod(0o600)
        endpoint = Endpoint(args.status)
        verification = Verification(args, out, manifest, guard, endpoint)
        verification.run()
        manifest["outcome"] = "verified_injected_fault"
    except AssertionError as error:
        manifest.update(outcome="failed", error=str(error))
    except Exception as error:
        manifest["error"] = type(error).__name__ + ": " + str(error)
    finally:
        if verification:
            manifest["local_prompt_submissions"] = verification.calls
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
        if endpoint:
            state.write(out / "local-http-requests.json", endpoint.rows())
            try:
                manifest["http_cleanup"] = endpoint.close()
                manifest["cleanup"] &= manifest["http_cleanup"]
            except Exception as error:
                manifest.update(cleanup=False, http_cleanup_error=str(error))
        state.write(out / "manifest.json", manifest)
        print(json.dumps(dict(capture=str(out), **manifest)))
    return 0 if manifest["outcome"] == "verified_injected_fault" and manifest["cleanup"] else 1


if __name__ == "__main__":
    sys.exit(main())
