#!/usr/bin/env python3
"""The daemon→mod direction of the spike's bridge (T-573).

Spawned once per session by the mod through `$.process.spawn`, this tails a
spool directory and prints each new file's contents as one line on stdout,
which the mod reads as a stream. (The other direction cannot ride this
child's stdin: in 2.1.287 `$.process.spawn` takes `input` as one string and
closes it. The mod relays up through `$.process.run` per event instead.)
The real bridge would be `mesimon mod-bridge`, subscribed to the daemon.
"""
import json
import os
import sys
import time

spool = sys.argv[1]
seen = set()
while True:
    try:
        names = sorted(os.listdir(spool))
    except OSError:
        names = []
    for name in names:
        if name in seen or not name.endswith(".json"):
            continue
        seen.add(name)
        try:
            with open(os.path.join(spool, name)) as f:
                text = f.read()
            json.loads(text)
        except (OSError, ValueError):
            continue
        sys.stdout.write(text.replace("\n", " ") + "\n")
        sys.stdout.flush()
    time.sleep(0.1)
