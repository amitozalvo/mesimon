#!/usr/bin/env python3
"""MR1: deliver the daemon's actual compatibility refusal to a real agent.

The fixture advertises an unsupported reader while attempting one note write
as this test's worker. It prints the unedited refusal and retains the wire
evidence. No production seam and no simulated error wording.
"""
import json
from pathlib import Path
import socket
import sys

from wire import Wire


def probe(sock_path, ticket, receipt):
    with Wire(sock_path, client="rig-mcp-probe") as wire:
        board = wire.board()
        sessions = [s for s in board["sessions"] if s["ticket"] == ticket]
        if len(sessions) != 1:
            raise RuntimeError("expected exactly the test worker's session")
        request = {
            "principal": {"kind": "agent", "session": sessions[0]["id"]},
            "command": {"cmd": "agent_write_note", "text": "MR1 must not be written"},
            "mcp_protocol": 4294967295,
        }
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(15)
            sock.connect(sock_path)
            sock.sendall((json.dumps(request) + "\n").encode())
            with sock.makefile("rb") as reader:
                response = json.loads(reader.readline(1 << 20))
        before = next(t for t in board["tickets"] if t["id"] == ticket)["notes"]
        after = next(t for t in wire.board()["tickets"] if t["id"] == ticket)["notes"]
    Path(receipt).write_text(json.dumps({
        "request": request, "response": response, "unchanged": before == after,
    }) + "\n")
    if response.get("resp") != "err":
        raise RuntimeError(f"expected a compatibility refusal: {response}")
    print(response["message"])


if __name__ == "__main__":
    probe(*sys.argv[1:])
