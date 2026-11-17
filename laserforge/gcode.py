"""G-code generation for laser cutting with native air assist support.

Output for GRBL-family controllers:
- Air assist: M7 (mist) / M8 (flood), M9 off
- Power via S (0..$30), M3 constant / M4 dynamic
- Fill: scanline engraving inside closed shapes with overscan, bidirectional, cross-hatch
- Offset fill, tabs (bridges), kerf offset, rotary Y scaling, frame trace
"""

import math
import re
from dataclasses import dataclass, field
from enum import Enum
from typing import Optional

from shapely.geometry import LineString, Point, Polygon
from shapely.validation import make_valid


class AirAssistMode(Enum):
    """Per-layer air assist is just on/off, like LightBurn: the actual command (M8 or M7) is a device setting."""
    OFF = "off"
    ON = "on"

    @classmethod
    def _missing_(cls, value):
        # legacy values from older files/presets ("m7", "m8", True) all mean "on"
        if value in (False, None, "", 0, "0", "none", "OFF", "Off"):
            return cls.OFF
        return cls.ON


class LaserMode(Enum):
    CONSTANT = "M3"
    DYNAMIC = "M4"


class CutMode(Enum):
    LINE = "line"
    FILL = "fill"
    OFFSET_FILL = "offset_fill"
    LINE_FILL = "line_fill"


@dataclass
class LayerSettings:
    name: str = "C00"
    color: str = "#000000"
    speed: float = 1000.0
    power_min: float = 0.0
    power_max: float = 100.0
    passes: int = 1
    cut_mode: CutMode = CutMode.LINE
    air_assist: AirAssistMode = AirAssistMode.ON
    air_power: float = 100.0
    air_cut_only: bool = False   # LightBurn cut setting "Enable the air assist only when cutting": off during travel
    z_offset: float = 0.0
    laser_mode: LaserMode = LaserMode.DYNAMIC
    enabled: bool = True
    line_interval: float = 0.1
    overscan: float = 2.5
    overscan_percent: bool = False
    scan_angle: float = 0.0
    cross_hatch: bool = False
    bidir_scan: bool = True
    dwell_time: float = 0.0
    start_delay: float = 0.0
    end_delay: float = 0.0
    ramp_length: float = 0.0
    tab_count: int = 0
    tab_width: float = 2.0
    kerf_offset: float = 0.0
    power_scale: float = 100.0
    dot_mode: bool = False
    dot_time: float = 1.0
    z_step: float = 0.0
    tool: bool = False           # T1/T2 tool layer: shown on screen, never output
    lead_in: float = 0.0         # mm: start the cut this far before the shape's start point (no burn mark on the edge)
    lead_out: float = 0.0        # mm: run on past the end point
    overcut: float = 0.0         # mm: closed shapes are cut past their start again so the part drops out
    perforate: bool = False      # LightBurn "Perforation Mode": dashed cut
    perf_cut: float = 1.0        # mm burned per dash
    perf_skip: float = 1.0       # mm skipped per dash
    flood_fill: bool = False     # LightBurn "Flood Fill": fill one area at a time instead of whole rows across
    sublayers: list = field(default_factory=list)   # LightBurn sub-layers: extra passes with their own settings
    pass_through: bool = False   # images: use the bitmap's own pixels as they are (already dithered artwork)
    dot_width: float = 0.0       # images: Dot Width Correction, mm taken off each burned dot (beam size compensation)
    bottom_up: bool = False      # images: engrave from the bottom of the design up (default top-down, like LightBurn)


@dataclass
class MachineSettings:
    max_speed: float = 6000.0
    max_power: int = 1000
    workspace_x: float = 400.0
    workspace_y: float = 400.0
    home_on_start: bool = True
    return_to_origin: bool = True
    baud_rate: int = 115200
    air_assist_default: AirAssistMode = AirAssistMode.ON
    use_g0_overscan: bool = False
    frame_power: float = 1.0
    rotary_enabled: bool = False
    rotary_type: str = "roller"
    rotary_mm_per_rev: float = 100.0
    rotary_diameter: float = 50.0
    start_gcode: str = ""
    end_gcode: str = ""
    flavor: str = "grbl"          # grbl | grblhal | fluidnc | marlin | smoothie | linuxcnc
    origin: str = "fl"            # machine 0,0 corner: fl front-left, rl rear-left, rr rear-right, fr front-right
    air_command: str = "M8"       # LightBurn default for GRBL: M8 (flood). M7 (mist) is disabled in stock GRBL builds
    marlin_air_fan: bool = False  # Marlin: air pump on the part fan (M106/M107) instead of M7/M8/M9
    transfer_mode: str = "buffered"  # "buffered" (fills GRBL's serial buffer, like LightBurn) or "synchronous"
    acceleration: float = 1000.0     # mm/s² (GRBL $120/$121), for the time estimate
    backlash_x: float = 0.0          # mm of play added when an axis reverses (LightBurn "Hide Backlash")
    backlash_y: float = 0.0
    finish_position: str = "origin"  # where the head goes after a job: origin | none | custom
    finish_x: float = 0.0
    finish_y: float = 0.0
    fire_power: float = 1.0          # % used by the Fire button
    fire_enabled: bool = False       # LightBurn "Enable Laser Fire Button" (off by default)
    z_enabled: bool = False          # LightBurn "Enable Z axis": layer Z offset / step per pass and Z jog
    z_speed: float = 5.0             # mm/s for Z moves
    z_relative_only: bool = False    # "Relative Z moves only": machines without Z homing
    rotary_axis: str = "Y"           # rotary on "Y" (replaces Y) or a real "A" axis
    raster_relative: bool = True     # images sent as G91 deltas like LightBurn (30-40 % shorter G-code)
    fast_whitespace: bool = False   # images: cross blank gaps faster (LightBurn "Fast Whitespace Scan")
    whitespace_speed: float = 6000.0  # minimum speed over blank areas when fast_whitespace is on (mm/min)
    scan_offset_enabled: bool = False  # bidirectional scanning: shift each line back by the measured lag
    scan_offsets: str = ""           # "speed:offset" pairs (mm/min:mm), one per line, interpolated by speed
    feeder_enabled: bool = False  # repeat the job, advancing the material between runs
    feeder_axis: str = "A"        # axis that drives the feeder/conveyor (A, Y, Z) or "custom"
    feeder_distance: float = 100.0
    feeder_repeats: int = 1
    feeder_speed: float = 1000.0
    feeder_gcode: str = ""        # used when feeder_axis == "custom"


@dataclass
class GCodePoint:
    x: float
    y: float
    power: Optional[float] = None


class GPath(list):
    """Path of GCodePoints that can carry manual tab positions (fractions of its length)."""
    tabs = None


def is_closed(path: list[GCodePoint], tol: float = 1e-3) -> bool:
    return len(path) > 2 and abs(path[0].x - path[-1].x) < tol and abs(path[0].y - path[-1].y) < tol


def fill_rows(paths: list[list[GCodePoint]], interval: float, angle_deg: float):
    """Even-odd scanline fill. Returns (rows, unrotate) where rows = [(y, [(x0, x1), ...]), ...] in scan space."""
    a = math.radians(angle_deg)
    ca, sa = math.cos(a), math.sin(a)
    edges = []
    for path in paths:
        if len(path) < 3:
            continue
        pts = [(p.x * ca + p.y * sa, -p.x * sa + p.y * ca) for p in path]
        if pts[0] != pts[-1]:
            pts.append(pts[0])
        for (x1, y1), (x2, y2) in zip(pts, pts[1:]):
            if y1 != y2:
                edges.append((min(y1, y2), max(y1, y2), x1, y1, x2, y2))

    def unrotate(x, y):
        return x * ca - y * sa, x * sa + y * ca

    if not edges or interval <= 0:
        return [], unrotate
    edges.sort()
    ymin = edges[0][0]
    ymax = max(e[1] for e in edges)
    rows, active, idx = [], [], 0
    y = ymin + interval / 2
    while y < ymax:
        while idx < len(edges) and edges[idx][0] <= y:
            active.append(edges[idx])
            idx += 1
        active = [e for e in active if e[1] > y]
        xs = sorted(x1 + (y - y1) * (x2 - x1) / (y2 - y1) for (_, _, x1, y1, x2, y2) in active)
        segs = [(xs[i], xs[i + 1]) for i in range(0, len(xs) - 1, 2) if xs[i + 1] - xs[i] > 1e-6]
        if segs:
            rows.append((y, segs))
        y += interval
    return rows, unrotate


def path_depths(polys: list[Polygon]) -> list[int]:
    """How many other closed paths contain each path (odd depth = hole)."""
    depths = []
    for i, p in enumerate(polys):
        pt = Point(p.exterior.coords[0])
        depths.append(sum(1 for j, q in enumerate(polys) if j != i and make_valid(q).contains(pt)))
    return depths


def ring_points(coords) -> list[GCodePoint]:
    return [GCodePoint(x, y) for x, y in coords]


def offset_paths(paths: list[list[GCodePoint]], distance: float) -> list[list[GCodePoint]]:
    """Kerf compensation: outer outlines grow, holes shrink. Open paths are unchanged."""
    closed = [p for p in paths if is_closed(p)]
    open_ = [p for p in paths if not is_closed(p)]
    raw = [Polygon([(q.x, q.y) for q in p]) for p in closed]
    depths = path_depths(raw)
    out = []
    for poly, depth in zip(raw, depths):
        d = distance if depth % 2 == 0 else -distance
        g = make_valid(poly).buffer(d, join_style="mitre", mitre_limit=4)
        for part in getattr(g, "geoms", [g]):
            if isinstance(part, Polygon) and not part.is_empty:
                out.append(ring_points(part.exterior.coords))
    return out + open_


def air_on_command(machine) -> str:
    """Same rule as LightBurn's GRBL driver: M7 only when the device is set to use M7, otherwise M8."""
    return "M7" if str(getattr(machine, "air_command", "M8")).upper() == "M7" else "M8"


JOB_HEADER = "G00 G17 G40 G21 G54"     # LightBurn's preamble: rapid, XY plane, no cutter comp, mm, work offset 1


def to_relative(gcode: str, start: tuple, user_origin: bool = False) -> str:
    """LightBurn sends "Current position" jobs in relative mode (G91) and "User origin" jobs as
    G90 G0 <origin> then G91: the job then lands where the head is, whatever the controller's work offset.
    `gcode` is the absolute job (designed with its start at `start`); every X/Y/Z becomes a delta."""
    word = re.compile(r"([XYZ])\s*(-?\d*\.?\d+)", re.I)
    pos = {"X": float(start[0]), "Y": float(start[1]), "Z": 0.0}
    out = []
    rel_in = False                         # input already relative here (e.g. feeder moves): pass through
    header_done = False
    for raw in gcode.splitlines():
        head, sep, tail = raw.partition(";")
        code = head.upper()
        if not header_done:
            out.append(raw)
            if code.strip().startswith(JOB_HEADER):
                if user_origin:
                    out += ["G90", f"G0 X{start[0]:.3f} Y{start[1]:.3f}"]
                out.append("G91")
                header_done = True
            continue
        if code.lstrip().startswith("$"):
            out.append(raw)
            continue
        if "G91" in code:
            rel_in = True
            head = re.sub(r"G91\s*", "", head, flags=re.I)
        if "G90" in code:
            rel_in = False
            head = re.sub(r"G90\s*", "", head, flags=re.I)   # output stays relative
        if rel_in:
            for ax, v in word.findall(head):
                pos[ax.upper()] += float(v)
            new_head = head
        else:
            def delta(m):
                ax, v = m.group(1).upper(), float(m.group(2))
                d = v - pos[ax]
                pos[ax] = v
                return f"{ax}{d:.3f}"
            new_head = word.sub(delta, head)
        if new_head.strip() or tail.strip():
            out.append((new_head.rstrip() + (" " if new_head.strip() and sep else "") + sep + tail).strip())
    if header_done:
        # back to absolute at the end, as LightBurn leaves the controller after relative output
        idx = next((k for k in range(len(out) - 1, -1, -1) if out[k].strip().upper().startswith("M2")), len(out))
        out.insert(idx, "G90")
    return "\n".join(out)


def _relativize(lines: list) -> list:
    """Raster block in relative mode, as LightBurn sends images (G91 between G90s): after the first rapid every
    X/Y is a delta. The position follows the rounded deltas actually written, so rounding never accumulates."""
    word = re.compile(r"([XY])(-?\d*\.?\d+)")
    out, pos, started = [], None, False
    for ln in lines:
        if not started and ln.startswith("G0 ") and "X" in ln and "Y" in ln:
            m = dict(word.findall(ln))
            pos = {"X": float(m["X"]), "Y": float(m["Y"])}
            out += [ln, "G91"]
            started = True
            continue
        if started and re.match(r"G[01]\b", ln):
            def delta(mm):
                ax, v = mm.group(1), float(mm.group(2))
                dv = round(v - pos[ax], 3)
                pos[ax] = round(pos[ax] + dv, 6)
                return f"{ax}{dv:.3f}"
            ln = word.sub(delta, ln)
        out.append(ln)
    if started:
        out.append("G90")
    return out


class GCodeGenerator:
    """Generates G-code with native air assist support."""

    def __init__(self, machine: MachineSettings):
        self.machine = machine
        self.lines: list[str] = []
        self._current_air: Optional[AirAssistMode] = None

    # ---- primitives ----
    def _s(self, percent: float) -> int:
        return int(round(max(0.0, percent) / 100.0 * self.machine.max_power))

    def _add(self, line: str):
        self.lines.append(line)

    def _y(self, y: float) -> float:
        m = self.machine
        if m.rotary_enabled and m.rotary_diameter > 0:
            return y * m.rotary_mm_per_rev / (math.pi * m.rotary_diameter)
        return y

    def set_air(self, mode: AirAssistMode, power: float = 100.0):
        if mode == self._current_air:
            return
        if mode == AirAssistMode.OFF:
            self._add("M9 ; Air assist OFF")
        else:
            self._add(f"{air_on_command(self.machine)} ; Air assist ON")
        self._current_air = mode

    def rapid(self, x: float, y: float):
        self._add(f"G0 X{x:.3f} Y{self._y(y):.3f}")

    def cut(self, x: float, y: float, speed: float, power: float):
        self._add(f"G1 X{x:.3f} Y{self._y(y):.3f} F{speed:.0f} S{self._s(power)}")

    def arc(self, x: float, y: float, i: float, j: float, speed: float, power: float, cw: bool = True):
        self._add(f"{'G2' if cw else 'G3'} X{x:.3f} Y{self._y(y):.3f} I{i:.3f} J{self._y(j):.3f} F{speed:.0f} S{self._s(power)}")

    def dwell(self, ms: float):
        if ms > 0:
            self._add(f"G4 P{ms / 1000.0:.3f}")

    def set_z(self, z: float):
        m = self.machine
        if not m.z_enabled:
            self._add(f"; Z {z:.3f} skipped (Z axis not enabled)")
            return
        feed = max(1.0, m.z_speed) * 60
        if m.z_relative_only:
            cur = getattr(self, "_z_now", 0.0)
            dz = z - cur
            self._z_now = z
            if abs(dz) > 1e-6:
                self._add(f"G91 G1 Z{dz:.3f} F{feed:.0f} S0")
                self._add("G90")
        else:
            self._add(f"G1 Z{z:.3f} F{feed:.0f} S0")

    def _power(self, power: float, layer: LayerSettings) -> float:
        return min(100.0, power * layer.power_scale / 100.0)

    def _in_bed(self, x: float, y: float):
        """Keep laser-off travel (overscan) inside the machine: at the edge of the bed the head would hit the limits."""
        m = self.machine
        x = min(max(x, 0.0), m.workspace_x)
        if not m.rotary_enabled:
            y = min(max(y, 0.0), m.workspace_y)
        return x, y

    def scan_offset(self, speed: float) -> float:
        """LightBurn's Scanning Offset Adjustment: lag (mm) at this speed, linearly interpolated between entries."""
        m = self.machine
        if not m.scan_offset_enabled or not m.scan_offsets.strip():
            return 0.0
        pts = []
        for ln in m.scan_offsets.replace(";", "\n").splitlines():
            parts = ln.replace(",", " ").replace(":", " ").split()
            try:
                pts.append((float(parts[0]), float(parts[1])))
            except (IndexError, ValueError):
                continue
        if not pts:
            return 0.0
        pts.sort()
        if speed <= pts[0][0]:
            return pts[0][1]
        for (s0, o0), (s1, o1) in zip(pts, pts[1:]):
            if speed <= s1:
                return o0 + (o1 - o0) * (speed - s0) / ((s1 - s0) or 1)
        return pts[-1][1]

    def _overscan(self, layer: LayerSettings) -> float:
        if layer.overscan_percent:
            return layer.speed / 60.0 * layer.overscan / 100.0
        return layer.overscan

    # ---- header ----
    def header(self, layer: LayerSettings):
        self._add(f"; Layer: {layer.name}")
        self._add(f"; Speed: {layer.speed} mm/min | Power: {layer.power_min}-{layer.power_max}%")
        self._add(f"; Air Assist: {layer.air_assist.value} | Passes: {layer.passes}")
        self._add(f"; Mode: {layer.cut_mode.value} | Laser: {layer.laser_mode.value}")
        if layer.cut_mode != CutMode.LINE:
            self._add(f"; Interval: {layer.line_interval}mm | Angle: {layer.scan_angle} | Overscan: {layer.overscan}mm")
        if layer.kerf_offset:
            self._add(f"; Kerf offset: {layer.kerf_offset}mm")
        if layer.tab_count:
            self._add(f"; Tabs: {layer.tab_count} x {layer.tab_width}mm")
        if self.machine.rotary_enabled:
            self._add(f"; Rotary: {self.machine.rotary_mm_per_rev}mm/rev, object diameter {self.machine.rotary_diameter}mm")
        self._add("G21 ; mm")
        self._add("G90 ; absolute")
        self._add(f"{layer.laser_mode.value} S0")

    # ---- fill ----
    def _emit_fill(self, layer: LayerSettings, paths: list[list[GCodePoint]], angle: float):
        closed = [p for p in paths if is_closed(p)]
        if layer.flood_fill and len(closed) > 1:
            # LightBurn "Flood Fill": each separate area is filled on its own, so the head never scans the gaps
            geom = None
            for p in closed:
                poly = make_valid(Polygon([(q.x, q.y) for q in p]))
                geom = poly if geom is None else geom.symmetric_difference(poly)
            parts = [g for g in getattr(geom, "geoms", [geom]) if isinstance(g, Polygon) and not g.is_empty]
            if len(parts) > 1:
                cur = (0.0, 0.0)
                self._add(f"; Flood fill: {len(parts)} areas")
                while parts:
                    k = min(range(len(parts)), key=lambda i: Point(cur).distance(parts[i]))
                    part = parts.pop(k)
                    rings = [[GCodePoint(x, y) for x, y in r.coords] for r in [part.exterior, *part.interiors]]
                    self._emit_fill_rows(layer, rings, angle)
                    b = part.bounds
                    cur = (b[2], b[3])
                return
        self._emit_fill_rows(layer, closed, angle)

    def _emit_fill_rows(self, layer: LayerSettings, closed: list[list[GCodePoint]], angle: float):
        rows, unrot = fill_rows(closed, max(0.01, layer.line_interval), angle)
        if not rows:
            return
        power = self._power(layer.power_max, layer)
        over = self._overscan(layer)
        lag = self.scan_offset(layer.speed)
        self._add(f"; Fill {len(rows)} lines @ {angle:.0f} deg")
        for n, (y, segs) in enumerate(rows):
            forward = (not layer.bidir_scan) or n % 2 == 0
            if not forward:
                segs = [(b, a) for a, b in reversed(segs)]
            sign = 1 if forward else -1
            if lag:
                segs = [(a - sign * lag, b - sign * lag) for a, b in segs]
            sx, sy = self._in_bed(*unrot(segs[0][0] - sign * over, y))
            self.rapid(sx, sy)
            for a, b in segs:
                ax, ay = unrot(a, y)
                bx, by = unrot(b, y)
                self.cut(ax, ay, layer.speed, 0)
                self.cut(bx, by, layer.speed, power)
            ex, ey = self._in_bed(*unrot(segs[-1][1] + sign * over, y))
            self.cut(ex, ey, layer.speed, 0)

    def _emit_offset_fill(self, layer: LayerSettings, paths: list[list[GCodePoint]]):
        closed = [p for p in paths if is_closed(p)]
        if not closed:
            return
        geom = None
        for p in closed:
            poly = make_valid(Polygon([(q.x, q.y) for q in p]))
            geom = poly if geom is None else geom.symmetric_difference(poly)
        step = max(0.02, layer.line_interval)
        power = self._power(layer.power_max, layer)
        rings = 0
        current = geom
        while current is not None and not current.is_empty and rings < 5000:
            for part in getattr(current, "geoms", [current]):
                if not isinstance(part, Polygon) or part.is_empty:
                    continue
                for ring in [part.exterior, *part.interiors]:
                    pts = list(ring.coords)
                    self._travel(layer, *pts[0])
                    for x, y in pts[1:]:
                        self.cut(x, y, layer.speed, power)
                    rings += 1
            current = current.buffer(-step, join_style="round")
        self._add(f"; Offset fill: {rings} rings")

    # ---- line ----
    def _tab_gaps(self, path: list[GCodePoint], layer: LayerSettings) -> list[tuple[float, float]]:
        manual = getattr(path, "tabs", None)
        if manual:
            total = LineString([(p.x, p.y) for p in path]).length
            half = layer.tab_width / 2
            return [(max(0.0, f * total - half), min(total, f * total + half)) for f in sorted(manual)]
        if layer.tab_count <= 0 or not is_closed(path):
            return []
        total = LineString([(p.x, p.y) for p in path]).length
        spacing = total / layer.tab_count
        half = layer.tab_width / 2
        return [(spacing * (i + 0.5) - half, spacing * (i + 0.5) + half) for i in range(layer.tab_count)]

    def _air_cut_only(self, layer: LayerSettings) -> bool:
        return bool(layer.air_cut_only) and layer.air_assist != AirAssistMode.OFF

    def _travel(self, layer: LayerSettings, x: float, y: float):
        """Rapid to the next shape; with "air only when cutting" the air is off while travelling."""
        if self._air_cut_only(layer):
            self.set_air(AirAssistMode.OFF)
        self.rapid(x, y)
        if self._air_cut_only(layer):
            self.set_air(layer.air_assist, layer.air_power)

    def _lead_len(self, x: float, y: float, ux: float, uy: float, length: float) -> float:
        """Shorten a lead so it ends on the bed edge instead of past it (limit switches)."""
        m = self.machine
        t = length
        if ux > 1e-9:
            t = min(t, (m.workspace_x - x) / ux)
        elif ux < -1e-9:
            t = min(t, (0.0 - x) / ux)
        if not m.rotary_enabled:
            if uy > 1e-9:
                t = min(t, (m.workspace_y - y) / uy)
            elif uy < -1e-9:
                t = min(t, (0.0 - y) / uy)
        return max(0.0, t)

    def _with_leads(self, path: list[GCodePoint], layer: LayerSettings) -> list[GCodePoint]:
        """Lead-in along the first segment's tangent, overcut past the start of a closed shape, lead-out
        along the last direction (LightBurn's leadIn / leadOut / overcut)."""
        pts = [GCodePoint(p.x, p.y) for p in path]
        if is_closed(pts) and layer.overcut > 0:
            left, i, extra = layer.overcut, 1, []
            while left > 1e-6 and len(pts) > 2:
                a, b = pts[(i - 1) % (len(pts) - 1)], pts[i % (len(pts) - 1)]
                seg = math.hypot(b.x - a.x, b.y - a.y)
                if seg >= left:
                    t = left / seg if seg else 1
                    extra.append(GCodePoint(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))
                    break
                extra.append(GCodePoint(b.x, b.y))
                left -= seg
                i += 1
                if i > 10 * len(pts):
                    break
            pts += extra
        def unit(a, b):
            d = math.hypot(b.x - a.x, b.y - a.y) or 1.0
            return (b.x - a.x) / d, (b.y - a.y) / d
        if layer.lead_in > 0 and len(pts) > 1:
            ux, uy = unit(pts[0], pts[1])
            ln = self._lead_len(pts[0].x, pts[0].y, -ux, -uy, layer.lead_in)
            if ln > 1e-3:
                pts.insert(0, GCodePoint(pts[0].x - ux * ln, pts[0].y - uy * ln))
        if layer.lead_out > 0 and len(pts) > 1:
            ux, uy = unit(pts[-2], pts[-1])
            ln = self._lead_len(pts[-1].x, pts[-1].y, ux, uy, layer.lead_out)
            if ln > 1e-3:
                pts.append(GCodePoint(pts[-1].x + ux * ln, pts[-1].y + uy * ln))
        return pts

    def _emit_line_path(self, layer: LayerSettings, path: list[GCodePoint]):
        power = self._power(layer.power_max, layer)
        tab_gaps = self._tab_gaps(path, layer)
        if not tab_gaps and not layer.dot_mode and (layer.lead_in > 0 or layer.lead_out > 0 or layer.overcut > 0):
            path = self._with_leads(path, layer)
        self._travel(layer, path[0].x, path[0].y)
        self.dwell(layer.start_delay)
        if layer.dot_mode:
            for p in path[1:]:
                self.rapid(p.x, p.y)
                self._add(f"G1 S{self._s(power)} F{layer.speed:.0f}")
                self.dwell(layer.dot_time)
                self._add("S0")
            return
        gaps = list(tab_gaps)
        if layer.perforate and layer.perf_cut > 0 and layer.perf_skip > 0:
            total = sum(math.hypot(b.x - a.x, b.y - a.y) for a, b in zip(path, path[1:]))
            period, k = layer.perf_cut + layer.perf_skip, 0
            while k * period < total:
                gaps.append((k * period + layer.perf_cut, min(total, (k + 1) * period)))
                k += 1
        dist = 0.0
        for i in range(1, len(path)):
            a, b = path[i - 1], path[i]
            seg = math.hypot(b.x - a.x, b.y - a.y)
            # split the segment at tab and ramp boundaries
            cuts = [0.0, seg]
            for g0, g1 in gaps:
                for g in (g0, g1):
                    if dist < g < dist + seg:
                        cuts.append(g - dist)
            if layer.ramp_length > 0 and dist < layer.ramp_length:
                reach = min(seg, layer.ramp_length - dist)
                steps = max(1, int(reach / 0.5))
                cuts += [reach * k / steps for k in range(1, steps + 1)]
            cuts = sorted(set(round(c, 6) for c in cuts))
            for c0, c1 in zip(cuts, cuts[1:]):
                mid = dist + (c0 + c1) / 2
                t = c1 / seg if seg else 1
                x, y = a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t
                in_gap = any(g0 <= mid <= g1 for g0, g1 in gaps)
                p = power
                if layer.ramp_length > 0 and dist + c1 <= layer.ramp_length:
                    p = layer.power_min + (power - layer.power_min) * (dist + c1) / layer.ramp_length
                if in_gap:
                    self.rapid(x, y)
                else:
                    self.cut(x, y, layer.speed, p)
            dist += seg
            if layer.dwell_time > 0 and i < len(path) - 1:
                self.dwell(layer.dwell_time)
        self.dwell(layer.end_delay)

    # ---- layers ----
    def generate_layer(self, layer: LayerSettings, paths: list[list[GCodePoint]]) -> str:
        self.lines = []
        self.header(layer)
        if not layer.enabled:
            self._add("; Layer disabled")
            return "\n".join(self.lines)
        if layer.kerf_offset and layer.cut_mode in (CutMode.LINE, CutMode.LINE_FILL):
            line_paths = offset_paths(paths, layer.kerf_offset)
        else:
            line_paths = paths
        # air on for the layer; with "only when cutting" it stays off until the first cut
        self.set_air(AirAssistMode.OFF if self._air_cut_only(layer) else layer.air_assist, layer.air_power)
        for n in range(layer.passes):
            if layer.passes > 1:
                self._add(f"; Pass {n + 1}/{layer.passes}")
            z = layer.z_offset - layer.z_step * n
            if z:
                self.set_z(z)
            if layer.cut_mode in (CutMode.FILL, CutMode.LINE_FILL):
                self._emit_fill(layer, paths, layer.scan_angle)
                if layer.cross_hatch:
                    self._emit_fill(layer, paths, layer.scan_angle + 90)
            if layer.cut_mode == CutMode.OFFSET_FILL:
                self._emit_offset_fill(layer, paths)
            if layer.cut_mode in (CutMode.LINE, CutMode.LINE_FILL):
                for path in line_paths:
                    if len(path) >= 2:
                        self._emit_line_path(layer, path)
        self._add("S0")
        if layer.z_offset or layer.z_step:
            self.set_z(0)
        return "\n".join(self.lines)

    def generate_raster(self, layer: LayerSettings, raster: dict) -> str:
        """Image engraving: raster has x, y, w, h, cols, rows and data (row-major, row 0 at the lowest machine Y,
        0 = no burn, 255 = full). Follows LightBurn's GRBL output: the bitmap is rotated to the scan angle, equal
        neighbouring pixels become one move, white is skipped, F is written only when it changes, moves shorter
        than 0.0025 mm are dropped. Optional: Dot Width Correction, Fast Whitespace Scan, Scanning Offset."""
        import numpy as np
        self.lines = []
        x0, y0 = float(raster["x"]), float(raster["y"])
        w, h = float(raster["w"]), float(raster["h"])
        cols, rows = int(raster["cols"]), int(raster["rows"])
        data = raster["data"]
        if cols < 1 or rows < 1 or len(data) < cols * rows:
            return "; empty image"
        a = np.asarray(data, dtype=np.uint8)[:cols * rows].reshape(rows, cols)
        px_w, px_h = w / cols, h / rows
        angle = float(raster.get("angle", layer.scan_angle) or 0.0) % 180.0
        cx, cy = x0 + w / 2, y0 + h / 2
        if abs(angle) > 1e-6:
            from PIL import Image as _Image
            img = _Image.fromarray(a)
            if abs(px_w - px_h) > 1e-3 * px_h:
                img = img.resize((max(1, round(w / px_h)), rows), _Image.NEAREST)
            # PIL turns counter-clockwise with row 0 on top; our row 0 is the lowest Y, so this is -angle on the bed
            img = img.rotate(angle, resample=_Image.NEAREST, expand=True, fillcolor=0)
            a = np.asarray(img)
            rows, cols = a.shape
            px_w = px_h
            t = math.radians(angle)
            c_, s_ = math.cos(t), math.sin(t)
            u0, v0 = -cols * px_w / 2, -rows * px_h / 2

            def P(u, v):
                return cx + u * c_ - v * s_, cy + u * s_ + v * c_
        else:
            u0, v0 = x0, y0

            def P(u, v):
                return u, v
        over = self._overscan(layer)
        span = layer.power_max - layer.power_min
        lag = self.scan_offset(layer.speed)
        dw = max(0.0, float(layer.dot_width or 0.0))
        m = self.machine
        ws_speed = max(layer.speed, float(m.whitespace_speed)) if m.fast_whitespace else layer.speed
        self._add(f"; Image {cols}x{rows}px, {w:.1f}x{h:.1f}mm, interval {px_h:.3f}mm, angle {angle:.1f}")
        self._add("G21")
        self._add("G90")
        self._add("M4 S0 ; dynamic power for raster")
        self.set_air(layer.air_assist, layer.air_power)
        state = {"x": None, "y": None, "f": None, "s": None}

        def move(u, v, s_val, feed, rapid=False):
            x, y = self._in_bed(*P(u, v))
            dx = state["x"] is None or abs(x - state["x"]) >= 0.0025
            dy = state["y"] is None or abs(y - state["y"]) >= 0.0025
            if not dx and not dy:
                return
            if rapid:
                self._add(f"G0 X{x:.3f} Y{self._y(y):.3f}")
                state.update(x=x, y=y, merge=None)
                return
            if s_val == 0 and state["s"] == 0 and feed == state["f"] and state.get("merge") == len(self.lines) - 1:
                self.lines.pop()                       # consecutive blank moves become one
                state.update(state["prev"])
                dx = abs(x - state["x"]) >= 0.0025
                dy = abs(y - state["y"]) >= 0.0025
            prev = {"x": state["x"], "y": state["y"], "f": state["f"], "s": state["s"]}
            parts = ["G1"]
            if dx:
                parts.append(f"X{x:.3f}")
            if dy:
                parts.append(f"Y{self._y(y):.3f}")
            if feed != state["f"]:
                parts.append(f"F{feed:.0f}")
                state["f"] = feed
            parts.append(f"S{s_val}")
            state["s"] = s_val
            self._add(" ".join(parts))
            state.update(x=x, y=y, prev=prev, merge=len(self.lines) - 1 if s_val == 0 else None)

        def power_s(v):
            return 0 if v == 0 else self._s(self._power(layer.power_min + span * v / 255.0, layer))

        # top-down in design terms by default: with a front origin the design top is the highest machine Y (last row)
        order = list(range(rows))
        if (m.origin in ("fl", "fr")) != bool(layer.bottom_up):
            order.reverse()
        for _ in range(layer.passes):
            k = 0
            for r in order:
                row = a[r]
                nz = np.flatnonzero(row)
                if nz.size == 0:
                    continue
                first, last = int(nz[0]), int(nz[-1])
                seg = row[first:last + 1]
                cuts = np.flatnonzero(np.diff(seg.astype(np.int16))) + 1
                bounds = [0, *cuts.tolist(), len(seg)]
                runs = [(first + bounds[i], first + bounds[i + 1], int(seg[bounds[i]])) for i in range(len(bounds) - 1)]
                forward = (not layer.bidir_scan) or k % 2 == 0
                k += 1
                d = 1 if forward else -1
                shift = -d * lag
                v = v0 + (r + 0.5) * px_h
                if not forward:
                    runs = [(e, s0, val) for s0, e, val in reversed(runs)]
                else:
                    runs = [(s0, e, val) for s0, e, val in runs]
                ustart = u0 + runs[0][0] * px_w + shift
                move(ustart - d * over, v, 0, layer.speed, rapid=True)
                move(ustart, v, 0, layer.speed)
                for s0, e, val in runs:
                    us, ue = u0 + s0 * px_w + shift, u0 + e * px_w + shift
                    if val == 0:
                        move(ue, v, 0, ws_speed if abs(ue - us) > 1.0 else layer.speed)
                        continue
                    if dw > 0:
                        length = abs(ue - us)
                        cut = min(dw, length * 0.8) / 2
                        move(us + d * cut, v, 0, layer.speed)
                        move(ue - d * cut, v, power_s(val), layer.speed)
                        move(ue, v, 0, layer.speed)
                    else:
                        move(ue, v, power_s(val), layer.speed)
                uend = u0 + runs[-1][1] * px_w + shift
                move(uend + d * over, v, 0, layer.speed)
        if m.raster_relative:
            self.lines = _relativize(self.lines)
        self._add("S0")
        return "\n".join(self.lines)

    def generate_frame(self, bounds: tuple, laser_on: bool = False, power: float = None, speed: float = 3000,
                       relative_to: Optional[tuple] = None, head: Optional[tuple] = None) -> str:
        """Rectangle around the job, as LightBurn traces it: G00 G17 G40 G21 G54 preamble, the trace starts at the
        corner nearest to the head, beam off unless laser_on (then `power` % with M4).

        With `relative_to` (head position, "Start from: current position") the moves are relative (G91) from where
        the head is and the head comes back to its starting point; otherwise absolute (G90)."""
        self.lines = []
        min_x, min_y, max_x, max_y = bounds
        ring = [(min_x, min_y), (max_x, min_y), (max_x, max_y), (min_x, max_y)]
        here = relative_to if relative_to is not None else head
        k = min(range(4), key=lambda i: math.hypot(ring[i][0] - here[0], ring[i][1] - here[1])) if here else 0
        trace = ring[k:] + ring[:k] + [ring[k]]
        p = self.machine.frame_power if power is None else power
        s_on = self._s(p) if laser_on else 0
        self._add("; Frame trace" + (" from current position" if relative_to is not None else "") +
                  (" (laser on)" if laser_on else " (beam off)"))
        self._add(JOB_HEADER)
        self._add("M4 S0" if laser_on else "M5")
        if relative_to is not None:
            self._add("G91")
            cx, cy = relative_to
            pts = trace + [tuple(relative_to)]
            for i, (x, y) in enumerate(pts):
                dx, dy = x - cx, y - cy
                cx, cy = x, y
                if abs(dx) < 1e-6 and abs(dy) < 1e-6:
                    continue
                if i == 0 or i == len(pts) - 1:
                    self._add(f"G0 X{dx:.3f} Y{self._y(dy):.3f}")
                else:
                    self._add(f"G1 X{dx:.3f} Y{self._y(dy):.3f} F{speed:.0f} S{s_on}")
        else:
            self._add("G90")
            self.rapid(*trace[0])
            for x, y in trace[1:]:
                self._add(f"G1 X{x:.3f} Y{self._y(y):.3f} F{speed:.0f} S{s_on}")
        self._add("G0")
        self._add("G90")
        self._add("S0")
        self._add("M5")
        return "\n".join(self.lines)

    def generate_job(self, layers: list[tuple[LayerSettings, list[list[GCodePoint]]]],
                     rasters: Optional[list[tuple[LayerSettings, dict]]] = None, finish: Optional[tuple] = None) -> str:
        out = ["; === LaserForge Job ===", f"; Layers: {len(layers)}  Images: {len(rasters or [])}", ""]
        # no $H here: like LightBurn, homing happens when connecting ("Auto-home on startup"), not in every job
        if self.machine.start_gcode.strip():
            out += [";USER START SCRIPT", self.machine.start_gcode.strip(), ";USER START SCRIPT", ""]
        out += [JOB_HEADER, "G90"]
        self._current_air = None
        for layer, raster in rasters or []:
            out.append(f"; ======== Image on {layer.name} ========")
            out.append(self.generate_raster(layer, raster))
            out.append("")
        for layer, paths in layers:
            out.append(f"; ======== {layer.name} ========")
            out.append(self.generate_layer(layer, paths))
            out.append("")
        out += ["; --- job end ---", "M5 ; laser off", "M9 ; air assist off"]
        if self.machine.return_to_origin:
            if finish is not None:              # current position / user origin: back to where the job started
                out.append(f"G0 X{finish[0]:.3f} Y{self._y(finish[1]):.3f} ; return to start")
            elif self.machine.finish_position == "custom":
                out.append(f"G0 X{self.machine.finish_x:.3f} Y{self.machine.finish_y:.3f} ; finish position")
            else:
                out.append("G0 X0 Y0 ; return to origin")
        if self.machine.end_gcode.strip():
            out += [";USER END SCRIPT", self.machine.end_gcode.strip(), ";USER END SCRIPT"]
        out += ["; === Job Complete ===", "M2"]
        self._current_air = None
        return "\n".join(out)

    def estimate_time(self, layers: list[tuple[LayerSettings, list[list[GCodePoint]]]]) -> dict:
        cut_dist = 0.0
        travel = 0.0
        cut_time = 0.0
        dwell = 0.0
        last = (0.0, 0.0)
        for layer, paths in layers:
            if not layer.enabled or layer.speed <= 0:
                continue
            v = layer.speed / 60.0
            for _ in range(layer.passes):
                if layer.cut_mode in (CutMode.LINE, CutMode.LINE_FILL):
                    for path in paths:
                        if len(path) < 2:
                            continue
                        travel += math.hypot(path[0].x - last[0], path[0].y - last[1])
                        d = sum(math.hypot(b.x - a.x, b.y - a.y) for a, b in zip(path, path[1:]))
                        cut_dist += d
                        cut_time += d / v
                        last = (path[-1].x, path[-1].y)
                        dwell += (layer.start_delay + layer.end_delay + layer.dwell_time * max(0, len(path) - 2)) / 1000.0
                if layer.cut_mode in (CutMode.FILL, CutMode.LINE_FILL, CutMode.OFFSET_FILL):
                    angles = [layer.scan_angle] + ([layer.scan_angle + 90] if layer.cross_hatch else [])
                    over = self._overscan(layer)
                    for ang in angles:
                        rows, _ = fill_rows([p for p in paths if is_closed(p)], max(0.01, layer.line_interval), ang)
                        for _, segs in rows:
                            cut_dist += sum(b - a for a, b in segs)
                            cut_time += (segs[-1][1] - segs[0][0] + 2 * over) / v
        travel_time = travel / (self.machine.max_speed / 60.0) if self.machine.max_speed > 0 else 0
        total = cut_time + travel_time + dwell
        return {
            "total_seconds": round(total, 1),
            "cut_distance_mm": round(cut_dist, 1),
            "rapid_distance_mm": round(travel, 1),
            "estimated": f"{int(total // 60)}m {int(total % 60)}s",
        }


_WORD = re.compile(r"([A-Z])(-?\d+(?:\.\d+)?)")


def resume_from(gcode: str, line_index: int) -> str:
    """G-code that continues `gcode` from line `line_index` ("Start from here").

    Replays the modal state reached just before that line (units, distance mode, laser mode M3/M4,
    air assist, feed, power, Z) and moves there with the laser off before continuing."""
    lines = gcode.split("\n")
    line_index = max(0, min(line_index, len(lines)))
    state = {"units": "G21", "dist": "G90", "laser": None, "air": "M9", "F": None, "S": 0.0, "X": 0.0, "Y": 0.0, "Z": None}
    for raw in lines[:line_index]:
        ln = raw.split(";")[0].strip().upper()
        if not ln:
            continue
        for code in ("G20", "G21"):
            if ln.startswith(code):
                state["units"] = code
        for code in ("G90", "G91"):
            if code in ln:
                state["dist"] = code
        if ln.startswith(("M3", "M4")):
            state["laser"] = ln[:2]
        if ln.startswith("M5"):
            state["laser"] = None
        if ln.startswith(("M7", "M8", "M9")):
            state["air"] = ln[:2]
        for k, v in _WORD.findall(ln):
            if k in ("X", "Y", "Z") and state["dist"] == "G91" and not ln.startswith("$"):
                state[k] = (state[k] or 0.0) + float(v)       # relative sections (images): keep the absolute spot
            elif k in ("F", "S", "X", "Y", "Z"):
                state[k] = float(v)
    out = ["; === LaserForge: resumed job ===", f"; continues from line {line_index + 1}", state["units"], "G90"]
    if state["air"] != "M9":
        out.append(state["air"])
    if state["Z"] is not None:
        out.append(f"G0 Z{state['Z']:.3f}")
    out.append(f"G0 X{state['X']:.3f} Y{state['Y']:.3f} S0")
    if state["laser"]:
        out.append(f"{state['laser']} S0")
    if state["dist"] == "G91":
        out.append("G91")
    # feed only: every cut line carries its own S, and an S on a motionless G1 could fire an M3 laser in place
    first = f"G1 F{state['F']:.0f}" if state["F"] else None
    if first and state["laser"]:
        out.append(first)
    out.append("; --- original job continues ---")
    out += lines[line_index:]
    return "\n".join(out)


FLAVORS = {"grbl": "GRBL", "grblhal": "grblHAL", "fluidnc": "FluidNC", "marlin": "Marlin", "smoothie": "Smoothieware", "linuxcnc": "LinuxCNC"}
_S_WORD = re.compile(r"S(\d+(?:\.\d+)?)")
_G4 = re.compile(r"^G4\s+P([\d.]+)")


def translate_flavor(gcode: str, machine: MachineSettings) -> str:
    """Rewrite LaserForge's GRBL-style output for another G-code controller."""
    fl = machine.flavor
    if fl in ("grbl", "grblhal", "fluidnc"):
        return gcode
    smax = max(1, machine.max_power)
    out = []
    for line in gcode.split("\n"):
        code, sep, comment = line.partition(";")
        c = code.strip()
        if c:
            if c.startswith("$H"):
                c = {"marlin": "G28 X Y", "smoothie": "G28.2 X0 Y0", "linuxcnc": ""}[fl]
                if not c:
                    comment = (comment + " homing is done from the LinuxCNC interface").strip()
            m = _G4.match(c)
            if m and fl in ("marlin", "smoothie"):
                c = f"G4 P{round(float(m.group(1)) * 1000)}"          # these controllers take milliseconds
            if fl == "marlin":
                c = _S_WORD.sub(lambda mm: f"S{round(float(mm.group(1)) / smax * 255)}", c)
                if machine.marlin_air_fan:
                    c = {"M7": "M106 S255", "M8": "M106 S255", "M9": "M107"}.get(c, c)
                if c == "M2":
                    c = "M5"
            elif fl == "smoothie":
                c = _S_WORD.sub(lambda mm: f"S{float(mm.group(1)) / smax:.3f}", c)
                c = re.sub(r"^M4\b", "M3", c)
            elif fl == "linuxcnc":
                c = re.sub(r"^M4\b", "M3", c)
        out.append(c + ((" " if c else "") + sep + comment if sep else ""))
    text = "\n".join(l.rstrip() for l in out)
    if fl == "linuxcnc":
        text = "%\n" + text + "\n%"
    return text



def apply_feeder(gcode: str, machine: MachineSettings) -> str:
    """Repeat the job `feeder_repeats` times, advancing the material between runs (LightBurn: Feeder setup)."""
    if not machine.feeder_enabled or machine.feeder_repeats < 1:
        return gcode
    lines = gcode.split("\n")
    # keep the job body; the original end (return to origin / M2) runs once at the very end
    tail_start = next((i for i, l in enumerate(lines) if l.startswith("; --- job end ---")), None)
    if tail_start is None:
        tail_start = next((i for i, l in enumerate(lines) if l.startswith("; === Job Complete") or l.startswith("G0 X0 Y0 ; return")), len(lines))
    body, tail = lines[:tail_start], lines[tail_start:]
    if machine.feeder_axis == "custom":
        feed = [machine.feeder_gcode.strip()] if machine.feeder_gcode.strip() else []
    else:
        a = machine.feeder_axis
        feed = ["M5", "G91", f"G1 {a}{machine.feeder_distance:.3f} F{machine.feeder_speed:.0f}", "G90"]
        if a not in ("X", "Y"):
            feed.append(f"G92 {a}0")          # keep the rotary/conveyor axis from running away
    out = []
    for n in range(machine.feeder_repeats):
        out += [f"; --- feeder run {n + 1}/{machine.feeder_repeats} ---"] + body
        if n < machine.feeder_repeats - 1:
            out += ["; advance material"] + feed
    return "\n".join(out + tail)


_WORD = re.compile(r"([A-Z])\s*(-?\d*\.?\d+)")


def _moves(gcode: str):
    """Yields (kind, x, y, feed_mm_min, dwell_s) for every motion/dwell line, tracking G90/G91 and modal F."""
    x = y = 0.0
    feed = None
    rel = False
    motion = "G0"
    for raw in gcode.splitlines():
        ln = raw.split(";", 1)[0].strip().upper()
        if not ln or ln.startswith("$"):
            continue
        if "G91" in ln:
            rel = True
        if "G90" in ln:
            rel = False
        words = dict(_WORD.findall(ln))
        g = words.get("G")
        if g is not None:
            gi = float(g)
            if gi in (0, 1, 2, 3):
                motion = f"G{int(gi)}"
            elif gi == 4:
                yield ("dwell", x, y, feed, float(words.get("P", 0)))   # GRBL: P in seconds
                continue
        if "F" in words:
            feed = float(words["F"])
        if "X" not in words and "Y" not in words:
            continue
        nx = (x + float(words.get("X", 0))) if rel else float(words.get("X", x))
        ny = (y + float(words.get("Y", 0))) if rel else float(words.get("Y", y))
        yield (motion, nx, ny, feed, 0.0)
        x, y = nx, ny


def estimate_gcode_time(gcode: str, machine) -> float:
    """Job time in seconds with a trapezoidal acceleration model and junction slowdowns (like GRBL's planner):
    each move accelerates/decelerates at `machine.acceleration`, corners slow down by the angle between moves."""
    acc = max(1.0, float(getattr(machine, "acceleration", 1000.0) or 1000.0))
    rapid = max(1.0, machine.max_speed / 60.0)
    segs = []          # (length, vmax, ux, uy)
    dwell = 0.0
    px = py = 0.0
    for kind, x, y, feed, dw in _moves(gcode):
        if kind == "dwell":
            dwell += dw
            continue
        dx, dy = x - px, y - py
        L = math.hypot(dx, dy)
        px, py = x, y
        if L < 1e-6:
            continue
        v = rapid if kind == "G0" or not feed else min(rapid, feed / 60.0)
        segs.append((L, v, dx / L, dy / L))
    n = len(segs)
    if not n:
        return dwell
    # junction speeds: full speed when going straight on, zero on a reversal
    vj = [0.0] * (n + 1)
    for i in range(1, n):
        c = segs[i - 1][2] * segs[i][2] + segs[i - 1][3] * segs[i][3]
        vj[i] = min(segs[i - 1][1], segs[i][1]) * max(0.0, c) ** 2
    # forward / backward passes: speed changes limited by acceleration over each move
    for i in range(n):
        vj[i + 1] = min(vj[i + 1], math.sqrt(vj[i] ** 2 + 2 * acc * segs[i][0]))
    for i in range(n - 1, -1, -1):
        vj[i] = min(vj[i], math.sqrt(vj[i + 1] ** 2 + 2 * acc * segs[i][0]))
    t = dwell
    for i, (L, v, _ux, _uy) in enumerate(segs):
        v0, v1 = vj[i], vj[i + 1]
        d_acc = max(0.0, (v * v - v0 * v0) / (2 * acc))
        d_dec = max(0.0, (v * v - v1 * v1) / (2 * acc))
        if d_acc + d_dec <= L:
            t += (v - v0) / acc + (v - v1) / acc + (L - d_acc - d_dec) / v
        else:                                   # never reaches full speed: triangle profile
            vp = math.sqrt(max(0.0, (2 * acc * L + v0 * v0 + v1 * v1) / 2))
            t += max(0.0, vp - v0) / acc + max(0.0, vp - v1) / acc
    return t


def apply_backlash(gcode: str, bx: float, by: float) -> str:
    """LightBurn's "Hide Backlash": when an axis reverses, the commanded position on that axis is shifted by the
    measured play, so the carriage really arrives where it should. Works on absolute moves and on relative (G91)
    sections such as images: there a reversal adds the play to that move's delta."""
    if bx <= 0 and by <= 0:
        return gcode
    out = []
    rel = False
    last = {"X": None, "Y": None}
    dirn = {"X": 1, "Y": 1}
    comp = {"X": float(bx), "Y": float(by)}
    word = re.compile(r"([XY])(-?\d*\.?\d+)")
    for raw in gcode.splitlines():
        code = raw.split(";", 1)[0].upper()
        if "G91" in code:
            rel = True
        if "G90" in code:
            rel = False
        if code.lstrip().startswith("$") or not word.search(code) or not re.match(r"\s*(G9[01]\s*)?(G0?[0123]\b|[XY])", code):
            out.append(raw)
            continue
        if rel:
            def fix_rel(m):
                ax, d = m.group(1), float(m.group(2))
                extra = 0.0
                if abs(d) > 1e-6:
                    nd = 1 if d > 0 else -1
                    if nd != dirn[ax]:
                        extra = comp[ax] * nd        # + → −: −b ; − → +: +b (same offsets as the absolute branch)
                        dirn[ax] = nd
                if last[ax] is not None:
                    last[ax] += d
                return f"{ax}{d + extra:.3f}"
            head, sep, tail = raw.partition(";")
            out.append(word.sub(fix_rel, head) + sep + tail)
            continue

        def fix(m):
            ax, val = m.group(1), float(m.group(2))
            prev = last[ax]
            if prev is not None and abs(val - prev) > 1e-6:
                dirn[ax] = 1 if val > prev else -1
            last[ax] = val
            return f"{ax}{val - (comp[ax] if dirn[ax] < 0 else 0.0):.3f}"
        head, sep, tail = raw.partition(";")
        out.append(word.sub(fix, head) + sep + tail)
    return "\n".join(out)


def apply_rotary_axis(gcode: str, machine) -> str:
    """Rotary on a real A axis (LightBurn "Rotary Axis: A"): the job's Y travel is sent as A, Y stays put."""
    if not (machine.rotary_enabled and getattr(machine, "rotary_axis", "Y") == "A"):
        return gcode
    out = []
    for ln in gcode.splitlines():
        head, sep, tail = ln.partition(";")
        if re.match(r"\s*(G0?[0123]\b|\$J=)", head, re.I):
            head = re.sub(r"(?<![A-Z])Y(-?\d*\.?\d+)", r"A\1", head)
        out.append(head + sep + tail)
    return "\n".join(out)
