#!/usr/bin/env python3
"""Print a session's spike log as one timeline line per event (T-573)."""
import json
import sys
from pathlib import Path


def brief(ev):
    d = ev["data"] if isinstance(ev["data"], dict) else {"v": ev["data"]}
    name = ev["event"]
    bits = []
    if name.startswith("classic."):
        for k in ("tool_name", "agent_id", "agent_type", "source", "reason", "notification_type", "stop_hook_active", "teammate_name", "trigger", "error", "prompt_id"):
            if k in d:
                bits.append(f"{k}={str(d[k])[:40]}")
        if isinstance(d.get("background_tasks"), list):
            bits.append("tasks=" + ",".join(f"{t.get('type')}:{t.get('status')}:{str(t.get('id'))[:8]}" for t in d["background_tasks"]))
        tr = d.get("tool_response")
        if isinstance(tr, dict):
            for k in ("backgroundTaskId", "agentId", "status", "answers", "plan", "isAgent"):
                if k in tr:
                    bits.append(f"resp.{k}={str(tr[k])[:60]}")
        ti = d.get("tool_input")
        if isinstance(ti, dict):
            for k in ("command", "file_path", "run_in_background", "subagent_type"):
                if k in ti:
                    bits.append(f"in.{k}={str(ti[k])[:40]}")
        if name == "classic.PermissionRequest":
            bits.append(f"suggestions={str(d.get('permission_suggestions'))[:120]}")
        if name == "classic.MessageDisplay":
            bits.append(f"message={str(d.get('message'))[:60]!r}")
    elif name == "turn.complete":
        bits.append(f"reason={d.get('reason')} dur={d.get('durationMs')} agentId={d.get('agentId')} usage={d.get('usage')} answer={str(d.get('answer'))[:50]!r}")
    elif name == "turn.step":
        bits.append(f"idx={d.get('index')} model={d.get('model')} agentId={d.get('agentId')} stop={d.get('stopReason')} tools={d.get('tools')} usage={d.get('usage')}")
    elif name in ("prompt.submit", "prompt.submit.result"):
        bits.append(f"origin={d.get('origin')} turnId={d.get('turnId')} wait={d.get('wait')} chars={d.get('chars')} ctx={d.get('context')} head={str(d.get('head'))[:50]!r}")
    elif name == "tool.check":
        bits.append(f"tool={d.get('tool')} core={d.get('core')} override={d.get('override')} id={d.get('tool_use_id')} origin={d.get('origin')}")
    elif name.startswith("ui.render"):
        bits.append(f"req={str(d.get('requestId'))[:12]} origin={d.get('origin')} from={d.get('from')} text={str(d.get('text'))[:60]!r}")
    else:
        bits.append(json.dumps(d, default=str)[:300])
    return " ".join(bits)


def main(paths):
    for path in paths:
        log = Path(path)
        evs = []
        for p in sorted(log.glob("*.json")):
            try:
                evs.append(json.loads(p.read_text()))
            except ValueError:
                pass
        if not evs:
            print(f"{log}: nothing")
            continue
        t0 = evs[0]["t"]
        print(f"== {log} ({len(evs)} events)")
        for ev in evs:
            print(f"{ev['seq']:5d} {ev['t'] - t0:8.0f}ms  {ev['event']:<32} {brief(ev)}")


if __name__ == "__main__":
    main(sys.argv[1:] or ["."])
