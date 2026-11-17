"""LaserForge engine: layers, machine settings, controller connection, G-code, geometry.

Runs in the same process as the Qt interface (like LightBurn) — no internal server.
"""

import math
import re
import threading
import time
from collections import deque
from dataclasses import fields, replace as dc_replace

import segno
from shapely.geometry import LineString, Polygon
from shapely.ops import unary_union
from shapely.validation import make_valid

from .gcode import (
    AirAssistMode, CutMode, GCodeGenerator, GCodePoint, GPath, translate_flavor, LaserMode, LayerSettings, MachineSettings,
    apply_backlash, estimate_gcode_time, apply_rotary_axis,
)
from .serial_comm import ConnectionState, GRBLConnection, SimulatedConnection

LAYER_COLORS = [
    "#000000", "#0000FF", "#FF0000", "#00FF00", "#FF8000", "#00FFFF", "#FF00FF", "#FFFF00",
    "#800080", "#008080", "#808000", "#FF69B4", "#4169E1", "#32CD32", "#FF4500", "#9400D3",
    "#8B4513", "#2E8B57", "#B22222", "#1E90FF", "#DAA520", "#20B2AA", "#C71585", "#6B8E23",
    "#483D8B", "#CD853F", "#00CED1", "#DC143C", "#7B68EE", "#556B2F",
]
# LightBurn's tool layers: shapes on T1/T2 are guides (alignment, jigs, notes) and are never sent to the laser
TOOL_LAYERS = [("T1", "#E07A10"), ("T2", "#3C8DBC")]

MATERIAL_PRESETS = {
    "Compensato 3mm": {"speed": 300, "power_max": 100, "passes": 1, "air_assist": "on", "cut_mode": "line"},
    "Compensato 5mm": {"speed": 200, "power_max": 100, "passes": 2, "air_assist": "on", "cut_mode": "line"},
    "Acrilico 3mm": {"speed": 400, "power_max": 95, "passes": 1, "air_assist": "on", "cut_mode": "line"},
    "Acrilico 5mm": {"speed": 250, "power_max": 100, "passes": 2, "air_assist": "on", "cut_mode": "line"},
    "Pelle": {"speed": 500, "power_max": 60, "passes": 1, "air_assist": "on", "cut_mode": "line"},
    "Cartone": {"speed": 800, "power_max": 40, "passes": 1, "air_assist": "on", "cut_mode": "line"},
    "Tessuto": {"speed": 600, "power_max": 30, "passes": 1, "air_assist": "off", "cut_mode": "line"},
    "Incisione legno": {"speed": 3000, "power_max": 50, "passes": 1, "air_assist": "on", "cut_mode": "fill"},
    "Incisione vetro": {"speed": 2000, "power_max": 80, "passes": 1, "air_assist": "off", "cut_mode": "fill"},
    "Taglio carta": {"speed": 1200, "power_max": 20, "passes": 1, "air_assist": "off", "cut_mode": "line"},
    "MDF 3mm": {"speed": 350, "power_max": 100, "passes": 1, "air_assist": "on", "cut_mode": "line"},
    "MDF 6mm": {"speed": 200, "power_max": 100, "passes": 3, "air_assist": "on", "cut_mode": "line"},
}

ENUM_FIELDS = {"cut_mode": CutMode, "air_assist": AirAssistMode, "laser_mode": LaserMode}


def layer_to_dict(layer: LayerSettings) -> dict:
    d = {}
    for f in fields(LayerSettings):
        v = getattr(layer, f.name)
        d[f.name] = v.value if f.name in ENUM_FIELDS else v
    return d


def apply_layer_dict(layer: LayerSettings, data: dict):
    types = {f.name: f.type for f in fields(LayerSettings)}
    for key, value in data.items():
        if key not in types or value is None:
            continue
        if key in ENUM_FIELDS:
            setattr(layer, key, ENUM_FIELDS[key](value))
        else:
            cur = getattr(layer, key)
            try:
                setattr(layer, key, type(cur)(value))
            except (TypeError, ValueError):
                pass


MACHINE_FIELDS = [f.name for f in fields(MachineSettings)]


class Engine:
    def __init__(self):
        self.machine = MachineSettings()
        self.layers: list[LayerSettings] = []
        self.console: deque = deque(maxlen=800)
        self.connection: GRBLConnection = SimulatedConnection()
        self.grbl_settings: dict = {}      # $n → value, filled from every "$n=v" line GRBL prints
        self._lock = threading.Lock()
        self.reset_layers()
        self._wire()
        self.log("LaserForge avviato", "status")

    # ---------- console ----------
    def log(self, msg: str, level: str = "info"):
        with self._lock:
            self.console.append({"time": time.strftime("%H:%M:%S"), "level": level, "msg": str(msg)})

    def console_lines(self) -> list:
        with self._lock:
            return list(self.console)

    def clear_console(self):
        with self._lock:
            self.console.clear()

    def _wire(self):
        def on_response(data):
            msg = data.get("response", data) if isinstance(data, dict) else data
            msg = str(msg)
            m = re.match(r"^\$(\d+)=(-?[\d.]+)", msg.strip())
            if m:
                self.grbl_settings[int(m.group(1))] = float(m.group(2))
            self.log(msg, "ok" if "ok" in msg.lower() else "plain")

        def on_error(data):
            self.log(data.get("message", data) if isinstance(data, dict) else data, "error")

        self.connection.on("response", on_response)
        self.connection.on("error", on_error)

    # ---------- layers ----------
    def reset_layers(self):
        self.layers = [LayerSettings(name=f"C{i:02d}", color=c) for i, c in enumerate(LAYER_COLORS)]
        self.layers += [LayerSettings(name=n, color=c, tool=True, enabled=False) for n, c in TOOL_LAYERS]

    def layer_dicts(self) -> list:
        return [layer_to_dict(l) for l in self.layers]

    def update_layer(self, idx: int, data: dict):
        if 0 <= idx < len(self.layers):
            lay = self.layers[idx]
            name, color, was_tool = lay.name, lay.color, lay.tool
            apply_layer_dict(lay, {k: v for k, v in data.items() if k != "tool"})
            if was_tool:
                # tool layers keep their identity and never get output turned on
                lay.tool, lay.enabled, lay.name, lay.color = True, False, name, color

    def apply_preset(self, idx: int, name: str) -> bool:
        p = MATERIAL_PRESETS.get(name)
        if not p or not (0 <= idx < len(self.layers)):
            return False
        apply_layer_dict(self.layers[idx], p)
        self.log(f"Preset '{name}' → {self.layers[idx].name}", "ok")
        return True

    # ---------- machine ----------
    def machine_dict(self) -> dict:
        d = {k: getattr(self.machine, k) for k in MACHINE_FIELDS}
        d["air_assist_default"] = self.machine.air_assist_default.value
        return d

    def update_machine(self, data: dict):
        for key, value in data.items():
            if key not in MACHINE_FIELDS or value is None:
                continue
            if key == "air_command":
                self.machine.air_command = "M7" if str(value).upper() == "M7" else "M8"
                continue
            if key == "air_assist_default":
                self.machine.air_assist_default = AirAssistMode(value)
                continue
            cur = getattr(self.machine, key)
            try:
                setattr(self.machine, key, type(cur)(value))
            except (TypeError, ValueError):
                pass

    def generator(self) -> GCodeGenerator:
        return GCodeGenerator(self.machine)

    # ---------- connection ----------
    @staticmethod
    def list_ports() -> list:
        try:
            return GRBLConnection.list_ports()
        except Exception:  # pyserial may be missing a backend
            return []

    def is_connected(self) -> bool:
        return self.connection.state in (ConnectionState.CONNECTED, ConnectionState.BUSY, ConnectionState.ALARM)

    def state(self) -> str:
        return self.connection.state.value

    def position(self) -> tuple:
        st = self.connection.status
        return st.x, st.y, st.z, st.state

    def connect(self, port: str = "SIM", baud: int | None = None) -> bool:
        baud = baud or self.machine.baud_rate
        if self.connection.state != ConnectionState.DISCONNECTED:
            self.connection.disconnect()
        self.connection = SimulatedConnection() if port == "SIM" else GRBLConnection()
        self.connection.flavor = self.machine.flavor
        self.connection.buffered = self.machine.transfer_mode != "synchronous"
        self._wire()
        ok = self.connection.connect(port, baud)
        self.log(f"Connesso a {port} @ {baud}" if ok else f"Connessione fallita: {port}", "ok" if ok else "error")
        if ok and port != "SIM" and self.machine.flavor in ("grbl", "grblhal", "fluidnc"):
            # read the controller settings first (checks, homing), then auto-home like LightBurn
            self.grbl_settings = {}
            self.connection.send_command("$$")
            threading.Timer(1.0, self._after_connect).start()
        return ok

    def _after_connect(self):
        if not self.connection.is_connected():
            return
        for p in self.grbl_problems():
            self.log("⚠ " + p, "error")
        if self.machine.home_on_start:
            if self.grbl_settings.get(22, 1) == 0:
                self.log("Home automatico saltato: homing non abilitato in GRBL ($22=0)", "error")
            else:
                self.connection.send_command("$H")    # LightBurn: Auto-Home on Startup (on connect, not per job)
                self.log("Home automatico alla connessione ($H)", "status")

    def read_settings(self):
        self.grbl_settings = {}
        self.command("$$")

    def grbl_problems(self) -> list:
        """The checks LightBurn makes on a GRBL controller's settings."""
        g, out = self.grbl_settings, []
        if g.get(32) == 0:
            out.append("Modalità laser spenta ($32=0): il laser resta acceso negli spostamenti e rallenta agli angoli. Imposta $32=1.")
        if g.get(20) == 1 and g.get(22) == 0:
            out.append("I limiti software ($20) non funzionano senza homing ($22=0).")
        if g.get(13) == 1:
            out.append("GRBL riporta la posizione in pollici ($13=1): imposta $13=0.")
        if g.get(30) and abs(g[30] - self.machine.max_power) > 0.5:
            out.append(f"Valore S max diverso: GRBL $30={g[30]:g}, LaserForge {self.machine.max_power}. Le potenze sarebbero sbagliate.")
        return out

    def disconnect(self):
        self.connection.disconnect()
        self.log("Disconnesso", "status")

    def command(self, cmd: str):
        cmd = cmd.strip()
        if not cmd:
            return
        self.log(f"> {cmd}", "plain")
        self.connection.send_command(cmd)

    def jog(self, x=0.0, y=0.0, z=0.0, feed=1000):
        m = self.machine
        if z and not m.z_enabled:
            self.log("Asse Z non abilitato (Impostazioni dispositivo → Movimento)", "error")
            z = 0.0
        if m.rotary_enabled and m.rotary_axis == "A" and y:
            # rotary on the A axis: Y jogs turn the rotary instead (same scale as the job)
            a = y * m.rotary_mm_per_rev / (math.pi * m.rotary_diameter) if m.rotary_diameter > 0 else y
            self.connection.jog(x=x, y=0.0, z=z, a=a, feed=feed)
            return
        if x or y or z:
            self.connection.jog(x=x, y=y, z=z, feed=feed)

    def zero_xy(self):
        """LightBurn-style work zero here: G10 L20 makes the head's current spot X0 Y0 of G54."""
        self.command("G10 L20 P1 X0 Y0")

    def clear_zero(self):
        self.command("G10 L2 P1 X0 Y0" + (" Z0" if self.machine.z_enabled else ""))

    def home(self):
        if self.grbl_settings.get(22) == 0:
            self.log("Homing non abilitato in GRBL ($22=0): attivalo in Impostazioni macchina", "error")
            return False
        self.connection.home()
        self.log("Homing...", "status")

    def unlock(self):
        self.connection.send_command("$X")
        self.log("Sblocco ($X)", "status")

    def pause(self):
        self.connection.pause()
        self.log("Pausa", "status")

    def resume(self):
        self.connection.resume()
        self.log("Ripreso", "status")

    def stop(self):
        self.connection.stop()
        self.log("STOP", "error")

    def send(self, gcode: str):
        if gcode:
            self.log(f"Invio G-code ({gcode.count(chr(10)) + 1} righe)...", "status")
            self.connection.send_gcode(gcode)

    # ---------- live overrides (GRBL real-time commands) ----------
    OVR = {("feed", 0): 0x90, ("feed", 10): 0x91, ("feed", -10): 0x92, ("feed", 1): 0x93, ("feed", -1): 0x94,
           ("power", 0): 0x99, ("power", 10): 0x9A, ("power", -10): 0x9B, ("power", 1): 0x9C, ("power", -1): 0x9D}
    feed_ovr = 100
    power_ovr = 100

    def override(self, kind: str, delta: int):
        """kind 'feed' or 'power'; delta 0 resets to 100%, ±10 or ±1 adjusts (GRBL limits 10–200%)."""
        cur = 100 if delta == 0 else max(10, min(200, getattr(self, f"{kind}_ovr") + delta))
        if self.machine.flavor in ("grbl", "grblhal", "fluidnc"):
            self.connection.realtime(self.OVR[(kind, delta)])
        elif kind == "feed":
            self.connection.send_command(f"M220 S{cur}")      # Marlin / Smoothie feed-rate percentage
        else:
            self.log("Override di potenza non disponibile su questo controller", "error")
            return getattr(self, "power_ovr")
        setattr(self, f"{kind}_ovr", cur)
        self.log(f"Override {'velocità' if kind == 'feed' else 'potenza'}: {cur}%", "status")
        return cur

    def progress(self):
        c = self.connection
        total = getattr(c, "_lines_total", 0)
        return (getattr(c, "_lines_sent", 0), total)

    def fire(self, on: bool):
        """Fire button: in GRBL laser mode ($32=1) the beam only lights with G1 active, so set G1 without moving."""
        if on:
            s = self.machine.fire_power / 100 * self.machine.max_power
            s_txt = f"{s:.1f}".rstrip("0").rstrip(".")
            self.connection.send_command("M3")
            self.connection.send_command(f"G1 F100 S{s_txt}")
            self.log(f"Fire: laser acceso S{s_txt}", "status")
        else:
            for c in ("S0", "G0", "M5"):
                self.connection.send_command(c)
            self.log("Fire: laser spento", "status")

    def air(self, mode: str):
        from .gcode import air_on_command
        cmd = "M9" if str(mode).lower() in ("off", "m9", "") else air_on_command(self.machine)
        if self.machine.flavor == "marlin" and self.machine.marlin_air_fan:
            cmd = {"M9": "M107"}.get(cmd, "M106 S255")
        self.connection.send_command(cmd)
        self.log(f"Aria {'OFF' if cmd in ('M9', 'M107') else 'ON'} ({cmd})", "ok")

    # ---------- G-code ----------
    def _job_layers(self, job: dict) -> list:
        out = []
        for cl in job.get("layers", []):
            idx = cl.get("layer_index", 0)
            if not (0 <= idx < len(self.layers)) or not self.layers[idx].enabled or self.layers[idx].tool:
                continue
            paths = []
            for path in cl.get("paths", []):
                if not path:
                    continue
                gp = GPath(GCodePoint(x=p[0], y=p[1]) for p in path)
                gp.tabs = getattr(path, "tabs", None)
                paths.append(gp)
            if paths:
                lay = self.layers[idx]
                out.append((lay, paths))
                # LightBurn sub-layers: the same shapes again with their own settings
                for k, sub in enumerate(lay.sublayers or []):
                    if not isinstance(sub, dict) or not sub.get("enabled", True):
                        continue
                    extra = dc_replace(lay, sublayers=[], name=sub.get("name") or f"{lay.name}.{k + 1}")
                    apply_layer_dict(extra, {kk: v for kk, v in sub.items() if kk not in ("name", "enabled", "sublayers")})
                    out.append((extra, paths))
        return out

    def _rasters(self, job: dict) -> list:
        return [(self.layers[r["layer_index"]], r) for r in job.get("rasters", [])
                if 0 <= r.get("layer_index", -1) < len(self.layers) and self.layers[r["layer_index"]].enabled
                and not self.layers[r["layer_index"]].tool]

    def generate(self, job: dict) -> dict | None:
        job_layers, rasters = self._job_layers(job), self._rasters(job)
        if not job_layers and not rasters:
            return None
        gen = self.generator()
        finish = tuple(job["start"]) if job.get("origin_mode") in ("current", "user") and job.get("start") else None
        raw = apply_backlash(gen.generate_job(job_layers, rasters, finish), self.machine.backlash_x, self.machine.backlash_y)
        gcode = translate_flavor(apply_rotary_axis(raw, self.machine), self.machine)
        est = gen.estimate_time(job_layers)
        # real duration from the G-code itself, with acceleration and corner slowdowns (like LightBurn)
        secs = estimate_gcode_time(raw, self.machine)
        est["total_seconds"] = round(secs, 1)
        est["estimated"] = (f"{int(secs // 3600)}h " if secs >= 3600 else "") + f"{int(secs % 3600 // 60)}m {int(secs % 60)}s"
        lines = gcode.count("\n") + 1
        self.log(f"G-code generato: {lines} righe, tempo stimato {est['estimated']}", "ok")
        return {"gcode": gcode, "lines": lines, "estimate": est}

    def frame(self, bounds, laser_on=False, power=None, speed=3000, relative_to=None, head=None) -> str:
        return translate_flavor(apply_rotary_axis(self.generator().generate_frame(tuple(bounds), laser_on, power, speed, relative_to, head),
                                                  self.machine), self.machine)

    def material_test(self, d: dict) -> str:
        rows = max(1, min(20, int(d.get("rows", 5))))
        cols = max(1, min(20, int(d.get("cols", 5))))
        size, gap = float(d.get("size", 10)), float(d.get("gap", 3))
        x0, y0 = float(d.get("x", 10)), float(d.get("y", 10))
        p_min, p_max = float(d.get("power_min", 10)), float(d.get("power_max", 100))
        s_min, s_max = float(d.get("speed_min", 500)), float(d.get("speed_max", 3000))
        fill = d.get("mode", "fill") == "fill"
        interval = float(d.get("interval", 0.2))
        g = self.generator()
        g.lines = []
        g._add(f"; Material test {rows}x{cols}: power {p_min}-{p_max}% (rows), speed {s_min}-{s_max} mm/min (cols)")
        g._add("G21")
        g._add("G90")
        g._add("M4")
        g.set_air(AirAssistMode(d.get("air_assist", "on")))
        for r in range(rows):
            power = p_min + (p_max - p_min) * (r / (rows - 1) if rows > 1 else 1)
            for c in range(cols):
                speed = s_min + (s_max - s_min) * (c / (cols - 1) if cols > 1 else 1)
                sx, sy = x0 + c * (size + gap), y0 + r * (size + gap)
                g._add(f"; cell r{r} c{c}: {power:.0f}% @ {speed:.0f}")
                if fill:
                    _square_fill(g, sx, sy, size, interval, speed, power)
                else:
                    g.rapid(sx, sy)
                    for px, py in ((sx + size, sy), (sx + size, sy + size), (sx, sy + size), (sx, sy)):
                        g.cut(px, py, speed, power)
        g._add("S0")
        g._add("M5")
        g.set_air(AirAssistMode.OFF)
        self.log(f"Test materiale generato ({rows}x{cols})", "ok")
        return translate_flavor("\n".join(g.lines), self.machine)

    def interval_test(self, d: dict) -> str:
        count = max(2, min(20, int(d.get("count", 6))))
        i_min, i_max = float(d.get("interval_min", 0.05)), float(d.get("interval_max", 0.3))
        size, gap = float(d.get("size", 10)), float(d.get("gap", 3))
        speed, power = float(d.get("speed", 3000)), float(d.get("power", 40))
        x0, y0 = float(d.get("x", 10)), float(d.get("y", 10))
        g = self.generator()
        g.lines = []
        g._add(f"; Interval test: {count} cells {i_min}-{i_max} mm @ {speed} mm/min {power}%")
        g._add("G21")
        g._add("G90")
        g._add("M4 S0")
        g.set_air(AirAssistMode(d.get("air_assist", "on")))
        for i in range(count):
            interval = i_min + (i_max - i_min) * i / (count - 1)
            g._add(f"; cell {i}: interval {interval:.3f}")
            _square_fill(g, x0 + i * (size + gap), y0, size, interval, speed, power)
        g._add("S0")
        g._add("M5")
        g.set_air(AirAssistMode.OFF)
        self.log(f"Test intervallo generato ({count} celle)", "ok")
        return translate_flavor("\n".join(g.lines), self.machine)

    def focus_test(self, d: dict) -> str:
        count = max(2, min(30, int(d.get("count", 11))))
        z_start, z_step = float(d.get("z_start", -5)), float(d.get("z_step", 1))
        length, gap = float(d.get("length", 30)), float(d.get("gap", 4))
        speed, power = float(d.get("speed", 1000)), float(d.get("power", 30))
        x0, y0 = float(d.get("x", 10)), float(d.get("y", 10))
        g = self.generator()
        g.lines = []
        g._add(f"; Focus test: {count} lines, Z {z_start} step {z_step}")
        g._add("G21")
        g._add("G90")
        g._add("M3 S0")
        g.set_air(AirAssistMode(d.get("air_assist", "on")))
        for i in range(count):
            z = z_start + z_step * i
            y = y0 + i * gap
            g._add(f"; line {i}: Z {z:.2f}")
            g.set_z(z)
            g.rapid(x0, y)
            g.cut(x0 + length, y, speed, power)
        g._add("S0")
        g.set_z(0)
        g._add("M5")
        g.set_air(AirAssistMode.OFF)
        self.log(f"Test di fuoco generato ({count} linee)", "ok")
        return translate_flavor("\n".join(g.lines), self.machine)

    # ---------- geometry ----------
    @staticmethod
    def geometry(op: str, shapes: list, params: dict) -> list:
        """shapes: list of shapes, each a list of paths [(x, y), ...]. Returns list of results (list of rings)."""
        geoms = [_shape_geom(s) for s in shapes]
        join = params.get("join", "round") if params.get("join") in ("round", "mitre", "bevel") else "round"
        results = []
        if op == "offset":
            dist = float(params.get("distance", 2))
            direction = params.get("direction", "out")
            for g, lines in geoms:
                for src in ([g] if g is not None else []) + lines:
                    if direction in ("out", "both"):
                        results.append(_rings(src.buffer(dist, join_style=join, mitre_limit=4)))
                    if direction in ("in", "both") and not isinstance(src, LineString):
                        results.append(_rings(src.buffer(-dist, join_style=join, mitre_limit=4)))
        elif op in ("weld", "union"):
            results.append(_rings(unary_union([g for g, _ in geoms if g is not None])))
        elif op == "subtract":
            base = geoms[0][0]
            rest = unary_union([g for g, _ in geoms[1:] if g is not None])
            results.append(_rings(base.difference(rest) if base is not None else None))
        elif op == "subtract_rev":
            base = geoms[-1][0]
            rest = unary_union([g for g, _ in geoms[:-1] if g is not None])
            results.append(_rings(base.difference(rest) if base is not None else None))
        elif op == "xor":
            parts = [g for g, _ in geoms if g is not None]
            acc = parts[0] if parts else None
            for g in parts[1:]:
                acc = acc.symmetric_difference(g)
            results.append(_rings(acc))
        elif op == "intersect":
            parts = [g for g, _ in geoms if g is not None]
            acc = parts[0] if parts else None
            for g in parts[1:]:
                acc = acc.intersection(g)
            results.append(_rings(acc))
        elif op == "round":
            r = float(params.get("radius", 2))
            for g, _ in geoms:
                if g is not None:
                    results.append(_rings(g.buffer(r, join_style="round").buffer(-2 * r, join_style="round").buffer(r, join_style="round")))
        elif op == "hull":
            allg = [g for g, _ in geoms if g is not None] + [ln for _, ls in geoms for ln in ls]
            results.append(_rings(unary_union(allg).convex_hull))
        else:
            raise ValueError(f"unknown op {op}")
        return [r for r in results if r]

    @staticmethod
    def qr_matrix(text: str, error: str = "m") -> list:
        qr = segno.make(text, error=error, micro=False)
        return [[1 if v else 0 for v in row] for row in qr.matrix]


def _square_fill(g: GCodeGenerator, x, y, size, interval, speed, power):
    yy, n = y, 0
    while yy <= y + size + 1e-9:
        a, b = (x, x + size) if n % 2 == 0 else (x + size, x)
        g.rapid(a, yy)
        g.cut(b, yy, speed, power)
        yy += interval
        n += 1


def _shape_geom(paths):
    geom, lines = None, []
    for pts in paths:
        if len(pts) < 2:
            continue
        closed = len(pts) > 3 and abs(pts[0][0] - pts[-1][0]) < 1e-6 and abs(pts[0][1] - pts[-1][1]) < 1e-6
        if closed:
            poly = make_valid(Polygon(pts))
            geom = poly if geom is None else geom.symmetric_difference(poly)
        else:
            lines.append(LineString(pts))
    return geom, lines


def _rings(geom) -> list:
    rings = []
    if geom is None or geom.is_empty:
        return rings
    for part in getattr(geom, "geoms", [geom]):
        if isinstance(part, Polygon) and not part.is_empty:
            for ring in [part.exterior, *part.interiors]:
                rings.append(list(ring.coords))
        elif hasattr(part, "geoms"):
            rings += _rings(part)
    return rings
