"""Starts the emulator in fake_grbl.py on a pty for the Rust connection tests.
Prints the pty path. Reads commands on stdin: `banner` makes the controller announce a restart;
at EOF it prints every line the controller received as JSON and exits."""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fake_grbl import FakeGrbl  # noqa: E402

g = FakeGrbl()
g.LIMIT = 1000.0
g.delay = float(os.environ.get("FAKE_DELAY", "0"))
print(g.path, flush=True)
for cmd in sys.stdin:
    if cmd.strip() == "banner":
        g._send("Grbl 1.1h ['$' for help]")
print(json.dumps(g.log), flush=True)
