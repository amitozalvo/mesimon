#!/usr/bin/env python3
"""Build and test the local encrypted Teams vertical slice with disposable state."""
import argparse
import json
import os
import signal
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "team/Cargo.toml"


def run(command, *, env=None, timeout=1200):
    process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
    try:
        status = process.wait(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        # Let the database/fixture owners execute cleanup before forced exit.
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=45)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        raise
    if status:
        raise subprocess.CalledProcessError(status, command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--postgres-bin", type=Path,
                        help="PostgreSQL server bin directory; Docker is the default")
    parser.add_argument("--database-ready", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    cargo = ["cargo", "test", "--manifest-path", str(MANIFEST), "--locked"]
    if args.database_ready:
        if not os.environ.get("MESIMON_TEAMS_TEST_DATABASE_URL_FILE") or not os.environ.get("MESIMON_TEST_RUN"):
            parser.error("the internal phase requires both disposable database and fixture guards")
        run(cargo + ["-p", "mesimon-teams-service-server", "--test", "postgres",
                     "--", "--ignored", "--test-threads=1"])
        run(cargo + ["-p", "mesimon-teams-client", "--test", "vertical_slice", "--",
                     "--exact", "encrypted_question_to_owner_worktree_private_reply_and_requester_mcp",
                     "--ignored", "--nocapture"])
        return
    run(["cargo", "build", "-p", "mesimon", "--locked"])
    run(["cargo", "build", "--manifest-path", str(MANIFEST), "--locked",
         "-p", "mesimon-teams-client", "-p", "mesimon-teams-service-server"])
    run(cargo + ["--workspace", "--lib"])
    metadata = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"],
                              cwd=ROOT, check=True, capture_output=True, text=True, timeout=60)
    binary = Path(json.loads(metadata.stdout)["target_directory"]) / "debug/mesimon"
    environment = dict(os.environ, MESIMON_TEAMS_TEST_MESIMON_BIN=str(binary))
    command = [sys.executable, "-B", str(ROOT / "team/service-server/tests/run_postgres.py")]
    if args.postgres_bin:
        command += ["--postgres-bin", str(args.postgres_bin.resolve())]
    command += ["--", sys.executable, "-B", str(ROOT / "ci/test-run.py"), "--",
                sys.executable, "-B", str(Path(__file__).resolve()), "--database-ready"]
    # Covers database setup + the guarded 20-minute command + cleanup.
    run(command, env=environment, timeout=1500)


if __name__ == "__main__":
    try:
        main()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"Teams verification failed: {error}", file=sys.stderr)
        raise SystemExit(1)
