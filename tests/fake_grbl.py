"""A tiny GRBL 1.1 emulator on a pseudo-terminal, for testing LaserForge's serial code without hardware.

Soft limits: X/Y must stay within 0..LIMIT; a move outside raises ALARM:2 and locks until $X or $H.
"""
import os
import re
import threading
import time
import tty


class FakeGrbl:
    LIMIT = 100.0

    def __init__(self):
        self.master, slave = os.openpty()
        tty.setraw(slave)
        self.path = os.ttyname(slave)
        self._slave = slave
        self.x = self.y = 0.0
        self.alarm = False
        self.log = []
        self.running = True
        threading.Thread(target=self._loop, daemon=True).start()

    def _send(self, text):
        os.write(self.master, (text + "\r\n").encode())

    def _loop(self):
        buf = b""
        while self.running:
            try:
                data = os.read(self.master, 1024)
            except OSError:
                break
            for b in data:
                ch = bytes([b])
                if ch == b"?":
                    st = "Alarm" if self.alarm else "Idle"
                    self._send(f"<{st}|MPos:{self.x:.3f},{self.y:.3f},0.000|FS:0,0>")
                elif ch == b"\x18":
                    self._send("Grbl 1.1h ['$' for help]")
                    if self.alarm:
                        self._send("[MSG:'$H'|'$X' to unlock]")
                elif ch in (b"!", b"~") or b >= 0x80:
                    pass
                elif ch == b"\n":
                    self._line(buf.decode(errors="replace").strip())
                    buf = b""
                elif ch != b"\r":
                    buf += ch

    def _line(self, line):
        if not line:
            return
        self.log.append(line)
        if getattr(self, "delay", 0):
            time.sleep(self.delay)                      # slow controller / USB latency
        if line == "$$":
            for k, v in sorted(getattr(self, "settings", {22: 1, 32: 1, 30: 1000}).items()):
                self._send(f"${k}={v}")
            self._send("ok")
            return
        if line.startswith("$") and "=" in line and not line.startswith("$J="):
            k, v = line[1:].split("=", 1)
            self.settings = {**getattr(self, "settings", {22: 1, 32: 1, 30: 1000}), int(k): float(v)}
            self._send("ok")
            return
        if line == "$H" and getattr(self, "settings", {}).get(22, 1) == 0:
            self._send("error:5")
            return
        if line in ("$X", "$H"):
            self.alarm = False
            if line == "$H":
                self.x = self.y = 0.0
            self._send("ok")
            return
        if self.alarm and not line.startswith("$"):
            self._send("error:9")
            return
        code = line.split(";")[0].strip()
        if code.startswith("M7"):                             # stock GRBL is built without ENABLE_M7
            self._send("error:20")
            return
        if code.startswith(("M8", "M9")):
            self.air = code.startswith("M8")
            self._send("ok")
            return
        if code.startswith("G91"):
            self.relative = True
        if code.startswith("G90"):
            self.relative = False
        m = dict(re.findall(r"([XY])(-?[\d.]+)", code))
        if getattr(self, "relative", False) and m and not line.startswith("$J="):
            m = {"X": str(self.x + float(m.get("X", 0))), "Y": str(self.y + float(m.get("Y", 0)))}
        if line.startswith("$J=") and "G91" in line:          # relative jog
            nx, ny = self.x + float(m.get("X", 0)), self.y + float(m.get("Y", 0))
            if not (0 <= nx <= self.LIMIT and 0 <= ny <= self.LIMIT):
                self._send("error:15")                        # jog beyond soft limits is rejected, no alarm
                return
            self.x, self.y = nx, ny
            self._send("ok")
            return
        nx, ny = float(m.get("X", self.x)), float(m.get("Y", self.y))
        if not (0 <= nx <= self.LIMIT and 0 <= ny <= self.LIMIT):
            self.alarm = True
            self._send("ALARM:2")
            self._send("[MSG:Reset to continue]")
            return
        self.x, self.y = nx, ny
        time.sleep(0.002)
        self._send("ok")
