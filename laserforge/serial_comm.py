"""Serial communication for GRBL controllers.

- Reader and writer threads (buffered character-counting or synchronous streaming)
- Baud rates: 115200 (standard GRBL), 460800 (some boards)
- GRBL responds with 'ok' or 'error:N' after each command
"""

import threading
import time
import queue
from dataclasses import dataclass
from enum import Enum
from typing import Optional, Callable

try:
    import serial
    import serial.tools.list_ports
    HAS_SERIAL = True
except ImportError:
    HAS_SERIAL = False


class ConnectionState(Enum):
    DISCONNECTED = "disconnected"
    CONNECTING = "connecting"
    CONNECTED = "connected"
    BUSY = "busy"
    ERROR = "error"
    ALARM = "alarm"


@dataclass
class MachineStatus:
    state: str = "Unknown"       # Idle, Run, Hold, Alarm, etc.
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    feed: float = 0.0
    speed: int = 0
    buffer_available: int = 15   # GRBL rx buffer slots
    line_number: int = 0


class SocketPort:
    """Minimal serial-like wrapper over TCP (GRBL/grblHAL/FluidNC over Wi-Fi or Ethernet, usually port 23)."""

    def __init__(self, host, port, timeout=1.0):
        import socket
        self.sock = socket.create_connection((host, port), timeout=5)
        self.sock.settimeout(timeout)
        self.buf = b""
        self.is_open = True

    def write(self, data):
        self.sock.sendall(data)
        return len(data)

    def _fill(self):
        import socket
        try:
            chunk = self.sock.recv(4096)
            if chunk:
                self.buf += chunk
        except (socket.timeout, BlockingIOError):
            pass

    @property
    def in_waiting(self):
        import select
        r, _, _ = select.select([self.sock], [], [], 0)
        if r:
            self._fill()
        return len(self.buf)

    def readline(self):
        import time as _t
        end = _t.time() + 1.0
        while b"\n" not in self.buf and _t.time() < end:
            self._fill()
        if b"\n" in self.buf:
            line, self.buf = self.buf.split(b"\n", 1)
            return line + b"\n"
        line, self.buf = self.buf, b""
        return line

    def flushInput(self):
        self.in_waiting
        self.buf = b""

    reset_input_buffer = flushInput

    def close(self):
        self.is_open = False
        try:
            self.sock.close()
        except OSError:
            pass


def detect_controllers(baud=115200, timeout=2.5):
    """Probe serial ports for a laser controller banner. Returns [(device, flavor, banner)]."""
    found = []
    if not HAS_SERIAL:
        return found
    for p in serial.tools.list_ports.comports():
        try:
            with serial.Serial(p.device, baud, timeout=0.3) as sp:
                time.sleep(1.5)
                sp.write(b"\r\n\x18")
                time.sleep(0.4)
                sp.write(b"$I\nM115\nversion\n")
                end, text = time.time() + timeout, ""
                while time.time() < end:
                    text += sp.read(sp.in_waiting or 1).decode("utf-8", errors="replace")
                low = text.lower()
                flavor = ("fluidnc" if "fluidnc" in low else "grblhal" if "grblhal" in low else "grbl" if "grbl" in low else
                          "marlin" if "marlin" in low else "smoothie" if "smoothie" in low else None)
                if flavor:
                    found.append((p.device, flavor, text.strip().splitlines()[0][:80] if text.strip() else flavor))
        except Exception:
            continue
    return found


class GRBLConnection:
    """Manages serial connection to a GRBL controller."""

    flavor = "grbl"

    GRBL_BUFFER_SIZE = 127  # bytes
    STATUS_POLL_INTERVAL = 0.25  # seconds

    def __init__(self):
        self.port: Optional[serial.Serial] = None
        self.state = ConnectionState.DISCONNECTED
        self.status = MachineStatus()
        self._send_queue: queue.Queue = queue.Queue()
        self._response_queue: queue.Queue = queue.Queue()
        self._worker: Optional[threading.Thread] = None
        self._poller: Optional[threading.Thread] = None
        self._running = False
        self._callbacks: dict[str, list[Callable]] = {
            "status": [],
            "response": [],
            "state_change": [],
            "error": [],
            "progress": [],
        }
        self._gcode_lines: list[str] = []
        self._lines_sent: int = 0
        self._lines_total: int = 0
        self.buffered = True          # LightBurn "Transfer mode: Buffered" (character counting) vs "Synchronous"
        self._generation = 0          # bumped by stop(): the writer forgets lines GRBL threw away on reset

    @staticmethod
    def list_ports() -> list[dict]:
        """List available serial ports."""
        if not HAS_SERIAL:
            return []
        ports = []
        for p in serial.tools.list_ports.comports():
            ports.append({
                "device": p.device,
                "description": p.description,
                "hwid": p.hwid,
                "manufacturer": p.manufacturer or "",
            })
        return ports

    def connect(self, port: str, baud: int = 115200) -> bool:
        """Connect to GRBL controller."""
        if not HAS_SERIAL:
            self._emit("error", {"message": "pyserial not installed"})
            return False

        try:
            self.state = ConnectionState.CONNECTING
            self._emit("state_change", {"state": self.state.value})

            if port.startswith("tcp://"):
                host, _, tport = port[6:].partition(":")
                self.port = SocketPort(host, int(tport or 23))
            else:
                self.port = serial.Serial(
                    port=port,
                    baudrate=baud,
                    timeout=0.2,
                    write_timeout=5,
                )

            # Wait for GRBL startup message
            time.sleep(2)
            self.port.flushInput()

            # Send soft reset (GRBL family only)
            if self.flavor in ("grbl", "grblhal", "fluidnc", "smoothie"):
                self.port.write(b"\x18")
            time.sleep(0.5)

            # Read startup
            startup = ""
            while self.port.in_waiting:
                line = self.port.readline().decode("utf-8", errors="replace").strip()
                startup += line + "\n"

            self.state = ConnectionState.CONNECTED
            self._running = True
            if "alarm" in startup.lower() or "'$x' to unlock" in startup.lower() or "$x to unlock" in startup.lower():
                self.state = ConnectionState.ALARM

            # Threads: one reader owns the port input, one writer streams lines, one poller asks for status
            self._reader = threading.Thread(target=self._read_worker, daemon=True)
            self._reader.start()
            self._worker = threading.Thread(target=self._send_worker, daemon=True)
            self._worker.start()
            self._poller = threading.Thread(target=self._status_poller, daemon=True)
            self._poller.start()

            self._emit("state_change", {
                "state": self.state.value,
                "startup": startup.strip(),
            })
            return True

        except Exception as e:
            self.state = ConnectionState.ERROR
            self._emit("error", {"message": str(e)})
            return False

    def disconnect(self):
        """Disconnect from controller."""
        self._running = False
        for t in (self._worker, self._poller, getattr(self, "_reader", None)):
            if t:
                t.join(timeout=2)
        if self.port and self.port.is_open:
            try:
                self.port.close()
            except Exception:
                pass
        self.state = ConnectionState.DISCONNECTED
        self._emit("state_change", {"state": self.state.value})

    def is_connected(self) -> bool:
        return self.state in (ConnectionState.CONNECTED, ConnectionState.BUSY, ConnectionState.ALARM)

    def send_command(self, cmd: str, wait: bool = False, timeout: float = 10.0) -> Optional[str]:
        """Queue one command. With wait=True, block (not from the UI thread) until GRBL answers ok/error."""
        if not self.is_connected():
            return None
        box: queue.Queue = queue.Queue() if wait else None
        self._send_queue.put((cmd.strip(), box))
        if not wait:
            return None
        try:
            return box.get(timeout=timeout)
        except queue.Empty:
            return None

    def unlock(self):
        """Clear an alarm ($X)."""
        self.send_command("$X")

    def send_gcode(self, gcode: str):
        """Queue a complete G-code program for streaming."""
        lines = [l.strip() for l in gcode.split("\n")
                 if l.strip() and not l.strip().startswith(";")]
        self._gcode_lines = lines
        self._lines_sent = 0
        self._lines_total = len(lines)
        self.state = ConnectionState.BUSY
        self._emit("state_change", {"state": self.state.value})

        for line in lines:
            self._send_queue.put((line, None))

    def realtime(self, byte: int):
        """GRBL real-time command (overrides etc.), bypassing the line queue."""
        if self.port and self.port.is_open:
            self.port.write(bytes([byte]))

    _held = False

    def pause(self):
        """Feed hold (GRBL '!'); other controllers: stop streaming new lines."""
        self._held = True
        if self.port and self.port.is_open and self.flavor in ("grbl", "grblhal", "fluidnc", "smoothie"):
            self.port.write(b"!")

    def resume(self):
        """Cycle start (GRBL '~'); other controllers: continue streaming."""
        self._held = False
        if self.port and self.port.is_open and self.flavor in ("grbl", "grblhal", "fluidnc", "smoothie"):
            self.port.write(b"~")

    def stop(self):
        """Emergency stop - soft reset."""
        self._generation += 1
        # Clear queue
        while not self._send_queue.empty():
            try:
                self._send_queue.get_nowait()
            except queue.Empty:
                break
        if self.port and self.port.is_open:
            self.port.write(b"\x18")  # Ctrl-X soft reset
        self._gcode_lines = []
        self._lines_sent = 0
        self._lines_total = 0
        self._held = False
        while not self._response_queue.empty():
            try:
                self._response_queue.get_nowait()
            except queue.Empty:
                break
        if self.state != ConnectionState.ALARM:
            self.state = ConnectionState.CONNECTED
        self._emit("state_change", {"state": self.state.value})

    def home(self):
        """Home all axes."""
        self.send_command({"marlin": "G28 X Y", "smoothie": "G28.2 X0 Y0", "linuxcnc": "G28"}.get(self.flavor, "$H"))

    def jog(self, x: float = 0, y: float = 0, z: float = 0,
            feed: float = 1000, relative: bool = True, a: float = 0):
        """Jog movement."""
        mode = "G91" if relative else "G90"
        if self.flavor in ("marlin", "linuxcnc"):
            axes = " ".join(f"{a}{v:.3f}" for a, v in (("X", x), ("Y", y), ("Z", z)) if v)
            for c in ([mode, f"G0 {axes} F{feed:.0f}", "G90"] if axes else []):
                self.send_command(c)
            if self.flavor == "marlin":
                self.status.x += x if relative else 0
                self.status.y += y if relative else 0
            return
        parts = [f"$J={mode}"]
        if x != 0:
            parts.append(f"X{x:.3f}")
        if y != 0:
            parts.append(f"Y{y:.3f}")
        if z != 0:
            parts.append(f"Z{z:.3f}")
        if a != 0:
            parts.append(f"A{a:.3f}")
        parts.append(f"F{feed:.0f}")
        self.send_command(" ".join(parts))

    def cancel_jog(self):
        """Cancel current jog."""
        if self.port and self.port.is_open:
            self.port.write(b"\x85")

    def on(self, event: str, callback: Callable):
        """Register event callback."""
        if event in self._callbacks:
            self._callbacks[event].append(callback)

    def _emit(self, event: str, data: dict):
        for cb in self._callbacks.get(event, []):
            try:
                cb(data)
            except Exception:
                pass

    def _parse_status(self, line: str):
        """Parse GRBL status response: <Idle|MPos:0.000,0.000,0.000|...>"""
        if not line.startswith("<") or not line.endswith(">"):
            return
        line = line[1:-1]
        parts = line.split("|")
        if parts:
            self.status.state = parts[0]
            for part in parts[1:]:
                if part.startswith("WCO:"):
                    try:
                        self._wco = [float(v) for v in part[4:].split(",")]
                    except ValueError:
                        pass
            for part in parts[1:]:
                if part.startswith("MPos:") or part.startswith("WPos:"):
                    coords = [float(v) for v in part.split(":")[1].split(",")]
                    if part.startswith("MPos:"):
                        # MPos − WCO = WPos: positions in the same frame as the job's G54 coordinates
                        wco = getattr(self, "_wco", None) or [0.0, 0.0, 0.0]
                        coords = [c - (wco[i] if i < len(wco) else 0.0) for i, c in enumerate(coords)]
                    if len(coords) >= 2:
                        self.status.x = coords[0]
                        self.status.y = coords[1]
                    if len(coords) >= 3:
                        self.status.z = coords[2]
                elif part.startswith("FS:"):
                    fs = part[3:].split(",")
                    self.status.feed = float(fs[0])
                    if len(fs) > 1:
                        self.status.speed = int(fs[1])
                elif part.startswith("Bf:"):
                    bf = part[3:].split(",")
                    self.status.buffer_available = int(bf[0])

            if self.status.state.startswith("Alarm"):
                if self.state != ConnectionState.ALARM:
                    self.state = ConnectionState.ALARM
                    self._emit("state_change", {"state": self.state.value})
            elif self.state == ConnectionState.ALARM:
                # unlocked ($X) or homed ($H): back to normal
                self.state = ConnectionState.BUSY if not self._send_queue.empty() else ConnectionState.CONNECTED
                self._emit("state_change", {"state": self.state.value})
            elif self.status.state == "Idle" and self.state == ConnectionState.BUSY:
                if self._send_queue.empty():
                    self.state = ConnectionState.CONNECTED
                    self._emit("state_change", {"state": self.state.value})

            self._emit("status", {
                "state": self.status.state,
                "x": self.status.x,
                "y": self.status.y,
                "z": self.status.z,
                "feed": self.status.feed,
                "speed": self.status.speed,
            })

    def _read_worker(self):
        """The only reader of the port: status reports, ok/error replies, alarms and messages."""
        while self._running:
            try:
                if not (self.port and self.port.is_open):
                    time.sleep(0.1)
                    continue
                raw = self.port.readline()
                if not raw:
                    continue
                line = raw.decode("utf-8", errors="replace").strip()
                if not line:
                    continue
                if line.startswith("<"):
                    self._parse_status(line)
                elif line == "ok" or line.startswith("error"):
                    self._response_queue.put(line)
                    if line.startswith("error"):
                        self._emit("error", {"message": f"{line} — {explain_grbl(line)}"})
                elif line.startswith("ALARM"):
                    self.state = ConnectionState.ALARM
                    self._emit("state_change", {"state": self.state.value})
                    self._emit("error", {"message": f"{line} — {explain_grbl(line)}"})
                    self._abort_stream()
                else:
                    self._emit("response", {"command": "", "response": line})
                    if "unlock" in line.lower() and "'$" in line:
                        self.state = ConnectionState.ALARM
                        self._emit("state_change", {"state": self.state.value})
            except Exception as e:
                if self._running:
                    self._emit("error", {"message": str(e)})
                    time.sleep(0.5)

    def _abort_stream(self):
        """Drop queued job lines (an alarm stops the job; GRBL would reject them anyway)."""
        self._generation += 1
        while not self._send_queue.empty():
            try:
                self._send_queue.get_nowait()
            except queue.Empty:
                break
        self._lines_total = 0

    def _use_buffer(self):
        return self.buffered and self.flavor in ("grbl", "grblhal", "fluidnc")

    def _send_worker(self):
        if self._use_buffer():
            self._buffered_worker()
        else:
            self._sync_worker()

    def _finish_line(self, cmd, box, response):
        if box is not None:
            box.put(response)
        if response is not None and not response.startswith("error"):
            self._emit("response", {"command": cmd, "response": response})
        self._lines_sent += 1
        if self._lines_total > 0:
            self._emit("progress", {"sent": self._lines_sent, "total": self._lines_total,
                                    "percent": self._lines_sent / self._lines_total * 100})

    def _buffered_worker(self):
        """Character-counting streaming, like LightBurn's Buffered transfer mode: keep GRBL's 127-byte serial
        buffer full instead of waiting for every 'ok'. Short raster moves then reach the planner without gaps,
        so the head no longer stutters (no dark dots in images). '$' system commands and homing still go one at a time."""
        from collections import deque
        inflight: deque = deque()            # (cmd, box, nbytes, sent_at)
        pending = None
        gen = self._generation
        while self._running:
            try:
                if not self._use_buffer() and not inflight and pending is None:
                    return self._sync_worker()
                if gen != self._generation:  # reset/alarm: GRBL dropped its buffer, no ok will come for these
                    for cmd, box, _n, _t, _x in inflight:
                        if box is not None:
                            box.put(None)
                    inflight.clear()
                    pending = None
                    gen = self._generation
                    while not self._response_queue.empty():
                        try:
                            self._response_queue.get_nowait()
                        except queue.Empty:
                            break
                if pending is None and not self._held:
                    try:
                        item = self._send_queue.get_nowait()
                        pending = item if isinstance(item, tuple) else (item, None)
                    except queue.Empty:
                        pending = None
                sent_any = False
                if pending is not None and self.port and self.port.is_open:
                    cmd, box = pending
                    n = len(cmd) + 1
                    exclusive = cmd.startswith("$") and not cmd.startswith("$J=")
                    used = sum(x[2] for x in inflight)
                    if (not exclusive and used + n <= self.GRBL_BUFFER_SIZE and not (inflight and inflight[-1][4])) or not inflight:
                        self.port.write((cmd + "\n").encode())
                        inflight.append((cmd, box, n, time.time(), exclusive))
                        self.max_inflight = max(getattr(self, "max_inflight", 0), len(inflight))
                        self.max_inflight_bytes = max(getattr(self, "max_inflight_bytes", 0), used + n)
                        pending = None
                        sent_any = True
                if sent_any:
                    continue
                if not inflight:
                    if pending is None:
                        time.sleep(0.01 if self._held else 0.005)
                    continue
                try:
                    response = self._response_queue.get(timeout=0.05)
                except queue.Empty:
                    cmd, box, _n, t0, _x = inflight[0]
                    wait = 180 if cmd.upper().startswith(("$H", "G28")) else 60
                    if time.time() - t0 > wait:
                        self._emit("error", {"message": f"Nessuna risposta dal controller a «{cmd}»"})
                        inflight.popleft()
                        self._finish_line(cmd, box, None)
                    continue
                if gen != self._generation:
                    continue
                cmd, box, _n, _t, _x = inflight.popleft()
                self._finish_line(cmd, box, response)
            except Exception as e:
                self._emit("error", {"message": str(e)})
                time.sleep(0.05)

    def _sync_worker(self):
        """Synchronous transfer: sends one line, waits for its ok/error."""
        while self._running:
            if self._use_buffer():
                return self._buffered_worker()
            try:
                while self._held and self._running:
                    time.sleep(0.05)
                item = self._send_queue.get(timeout=0.5)
                cmd, box = item if isinstance(item, tuple) else (item, None)
                if not (self.port and self.port.is_open):
                    continue
                self.port.write((cmd + "\n").encode())
                # homing replies only when finished; normal lines as soon as they are buffered
                wait = 180 if cmd.upper().startswith(("$H", "G28")) else 60
                try:
                    response = self._response_queue.get(timeout=wait)
                except queue.Empty:
                    response = None
                    self._emit("error", {"message": f"Nessuna risposta dal controller a «{cmd}»"})
                self._finish_line(cmd, box, response)
            except queue.Empty:
                continue
            except Exception as e:
                self._emit("error", {"message": str(e)})

    def _status_poller(self):
        """Asks for a status report ('?'); the reader thread parses the answer."""
        while self._running:
            if self.flavor not in ("marlin", "linuxcnc") and self.port and self.port.is_open:
                try:
                    self.port.write(b"?")
                except Exception:
                    pass
            time.sleep(self.STATUS_POLL_INTERVAL)


GRBL_ALARMS = {
    "1": "finecorsa toccato (hard limit): la macchina si è fermata. Libera la testa e fai Home ($H) o sblocca ($X)",
    "2": "movimento fuori dall'area della macchina (soft limit): il lavoro o il frame esce dai limiti. Controlla area di lavoro, "
         "origine e 'Avvia da', poi sblocca ($X) o fai Home ($H)",
    "3": "reset durante il movimento: la posizione potrebbe essere persa. Fai Home ($H) o sblocca ($X)",
    "4": "sonda in stato errato all'inizio del ciclo",
    "5": "la sonda non ha toccato nulla",
    "6": "homing interrotto da un reset",
    "7": "homing fallito: porta di sicurezza aperta",
    "8": "homing fallito: il finecorsa non si è liberato",
    "9": "homing fallito: finecorsa non trovato (controlla cablaggio e $130/$131)",
    "10": "homing fallito: secondo finecorsa (dual axis) non trovato",
}
GRBL_ERRORS = {
    "1": "comando G-code senza lettera", "2": "valore numerico mancante", "3": "comando $ non supportato",
    "8": "comando $ accettato solo a macchina ferma", "9": "macchina in allarme: sblocca con $X o fai Home ($H)",
    "10": "soft limit attivo ma la macchina non ha fatto Home", "15": "jog oltre i limiti della macchina",
    "16": "comando di jog non valido", "20": "comando G-code non supportato", "22": "velocità F non impostata",
    "24": "due comandi dello stesso gruppo sulla stessa riga", "33": "destinazione dell'arco non valida",
}


def explain_grbl(line: str) -> str:
    code = line.split(":", 1)[1].strip() if ":" in line else ""
    if line.startswith("ALARM"):
        return GRBL_ALARMS.get(code, "allarme: sblocca con $X o fai Home ($H)")
    return GRBL_ERRORS.get(code, "vedi la documentazione GRBL")


class SimulatedConnection(GRBLConnection):
    """Simulated connection for testing without hardware."""

    realtime_log: list = None

    def realtime(self, byte: int):
        if self.realtime_log is None:
            self.realtime_log = []
        self.realtime_log.append(byte)

    def connect(self, port: str = "SIM", baud: int = 115200) -> bool:
        self.state = ConnectionState.CONNECTED
        self._running = True
        self._emit("state_change", {
            "state": self.state.value,
            "startup": "Grbl 1.1h [LaserForge Simulator]",
        })
        self._worker = threading.Thread(target=self._sim_worker, daemon=True)
        self._worker.start()
        return True

    def disconnect(self):
        self._running = False
        if self._worker:
            self._worker.join(timeout=2)
        self.state = ConnectionState.DISCONNECTED
        self._emit("state_change", {"state": self.state.value})

    def _sim_worker(self):
        while self._running:
            try:
                item = self._send_queue.get(timeout=0.5)
                cmd, box = item if isinstance(item, tuple) else (item, None)
                time.sleep(0.01)  # Simulate processing
                if box is not None:
                    box.put("ok")
                # Parse movement for position tracking
                if cmd.startswith("$J=G91"):
                    for part in cmd[6:].split():
                        try:
                            if part.startswith("X"):
                                self.status.x += float(part[1:])
                            elif part.startswith("Y"):
                                self.status.y += float(part[1:])
                            elif part.startswith("Z"):
                                self.status.z += float(part[1:])
                        except ValueError:
                            pass
                elif "X" in cmd or "Y" in cmd:
                    for part in cmd.split():
                        if part.startswith("X"):
                            try:
                                self.status.x = float(part[1:])
                            except ValueError:
                                pass
                        elif part.startswith("Y"):
                            try:
                                self.status.y = float(part[1:])
                            except ValueError:
                                pass
                self._emit("response", {"command": cmd, "response": "ok"})
                self._lines_sent += 1
                if self._lines_total > 0:
                    self._emit("progress", {
                        "sent": self._lines_sent,
                        "total": self._lines_total,
                        "percent": self._lines_sent / self._lines_total * 100,
                    })
                self._emit("status", {
                    "state": "Run" if not self._send_queue.empty() else "Idle",
                    "x": self.status.x,
                    "y": self.status.y,
                    "z": self.status.z,
                    "feed": 1000,
                    "speed": 0,
                })
            except queue.Empty:
                continue


def discover_network(timeout: float = 2.0) -> list:
    """LightBurn's MDNSDeviceFinderWizard: find GRBL-family controllers on the network via Bonjour (macOS dns-sd).
    Returns [(name, host, port)]. grblHAL/FluidNC advertise telnet on port 23; FluidNC also an http page."""
    import re as _re
    import shutil
    import socket
    import subprocess
    if not shutil.which("dns-sd"):
        return []

    def run(args, secs):
        try:
            p = subprocess.Popen(["dns-sd", *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            try:
                out, _ = p.communicate(timeout=secs)
            except subprocess.TimeoutExpired:
                p.kill()
                out, _ = p.communicate()
            return out or ""
        except OSError:
            return ""

    found = {}
    for stype in ("_telnet._tcp", "_http._tcp"):
        for ln in run(["-B", stype, "local."], timeout).splitlines():
            m = _re.search(r"\sAdd\s+\S+\s+\S+\s+\S+\s+" + _re.escape(stype) + r"\.\s+(.+)$", ln)
            if not m:
                continue
            name = m.group(1).strip()
            if stype == "_http._tcp" and not _re.search(r"fluid|grbl|laser|cnc", name, _re.I):
                continue
            if name in found:
                continue
            res = run(["-L", name, stype, "local."], 1.5)
            r = _re.search(r"can be reached at\s+(\S+?)\.?:(\d+)", res)
            if not r:
                continue
            host, port = r.group(1), int(r.group(2))
            if stype == "_http._tcp":
                port = 23                      # FluidNC: the G-code stream is on telnet
            try:
                host = socket.gethostbyname(host)
            except OSError:
                pass
            found[name] = (name, host, port)
    return list(found.values())
