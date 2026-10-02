#!/usr/bin/env python3
"""The spike's stand-in for `mesimon approve`'s wait (T-573, row 4).

Run by the mod inside `$.process.run` while a permission dialog is up: waits
up to 40 s for one `*.json` file under the given directory, prints its text
(a `PermissionRequestDecision`: `{"behavior":"allow"}` or
`{"behavior":"deny","message":…}`) and renames it `.done`. Prints nothing on
the timeout, so the mod passes the event on.
"""
import os
import sys
import time

d = sys.argv[1]
os.makedirs(d, exist_ok=True)
end = time.time() + 40
while time.time() < end:
    for name in sorted(os.listdir(d)):
        if name.endswith(".json"):
            path = os.path.join(d, name)
            text = open(path).read()
            os.rename(path, path + ".done")
            sys.stdout.write(text)
            sys.stdout.flush()
            sys.exit(0)
    time.sleep(0.1)
