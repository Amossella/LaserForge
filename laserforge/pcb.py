"""PCB files: Gerber RS-274X (copper/silk/mask layers) and Excellon drill files.

Both are converted to filled outlines in mm. Y is flipped with a fixed reference (the work-area height) so
several files of the same board stay aligned when imported one after the other.
"""

import math
import re

from shapely.geometry import LineString, Point, Polygon, box
from shapely.ops import unary_union


def _num(token, fmt_dec, zeros="L"):
    if "." in token:
        return float(token)
    neg = token.startswith("-")
    t = token.lstrip("+-")
    if zeros == "T":                     # trailing zeros omitted: pad on the right to the full width
        t = t.ljust(fmt_dec[0] + fmt_dec[1], "0")
    v = int(t or 0) / (10 ** fmt_dec[1])
    return -v if neg else v


def _flash(ap, x, y):
    kind, p = ap
    if kind == "C":
        return Point(x, y).buffer(p[0] / 2, 24)
    if kind == "R":
        return box(x - p[0] / 2, y - p[1] / 2, x + p[0] / 2, y + p[1] / 2)
    if kind == "O":
        w, h = p[0], p[1]
        if w >= h:
            return LineString([(x - (w - h) / 2, y), (x + (w - h) / 2, y)]).buffer(h / 2, 16) if w > h else Point(x, y).buffer(w / 2, 24)
        return LineString([(x, y - (h - w) / 2), (x, y + (h - w) / 2)]).buffer(w / 2, 16)
    if kind == "P":
        d, n = p[0], int(p[1]) if len(p) > 1 else 6
        rot = math.radians(p[2]) if len(p) > 2 else 0
        return Polygon([(x + d / 2 * math.cos(rot + i * math.tau / n), y + d / 2 * math.sin(rot + i * math.tau / n)) for i in range(n)])
    return Point(x, y).buffer(0.1)


def _arc_points(x0, y0, x1, y1, cx, cy, cw, step=0.2):
    a0 = math.atan2(y0 - cy, x0 - cx)
    a1 = math.atan2(y1 - cy, x1 - cx)
    r = math.hypot(x0 - cx, y0 - cy)
    if cw:
        while a1 >= a0:
            a1 -= math.tau
    else:
        while a1 <= a0:
            a1 += math.tau
    if abs(x0 - x1) < 1e-9 and abs(y0 - y1) < 1e-9:
        a1 = a0 - math.tau if cw else a0 + math.tau
    n = max(4, int(abs(a1 - a0) * r / step))
    return [(cx + r * math.cos(a0 + (a1 - a0) * k / n), cy + r * math.sin(a0 + (a1 - a0) * k / n)) for k in range(1, n + 1)]


def gerber_geometry(text):
    """Dark area of a Gerber file as a shapely geometry in mm (Gerber coordinates, Y up)."""
    fmt = (2, 4)
    zeros = "L"
    scale = 1.0
    apertures = {}
    cur_ap = None
    x = y = 0.0
    interp = "G01"
    region = None
    dark = True
    items = []          # (geometry, dark)
    text = text.replace("\r", "")
    # extended commands
    for m in re.finditer(r"%([^%]*)%", text, re.S):
        body = m.group(1).replace("\n", "")
        if body.startswith("FS"):
            zeros = "T" if "T" in body[2:4] else "L"
            mm = re.search(r"X(\d)(\d)", body)
            if mm:
                fmt = (int(mm.group(1)), int(mm.group(2)))
        elif body.startswith("MOIN"):
            scale = 25.4
        elif body.startswith("MOMM"):
            scale = 1.0
    EXT = ("FS", "MO", "AD", "AM", "LP", "SR", "TF", "TA", "TO", "TD", "IP", "OF", "SF", "LN", "IN", "AS", "IR", "MI")
    for raw in re.split(r"[*%\n]+", text):
        cmd = raw.strip()
        if not cmd:
            continue
        if cmd.startswith(EXT) or cmd[0].isdigit() or cmd[0] in "$,":
            ad = re.match(r"ADD(\d+)([A-Za-z_][\w.$]*),?([\dX.]*)", cmd)
            if ad:
                params = [float(v) * scale for v in ad.group(3).split("X") if v] or [0.1]
                if ad.group(2) == "P" and len(params) > 1:
                    params[1] /= scale
                    if len(params) > 2:
                        params[2] /= scale
                kind = ad.group(2) if ad.group(2) in ("C", "R", "O", "P") else "C"
                apertures[int(ad.group(1))] = (kind, params)
            elif cmd.startswith("LPD"):
                dark = True
            elif cmd.startswith("LPC"):
                dark = False
            continue
        while cmd:
            g = re.match(r"G0?(\d+)", cmd)
            if g:
                code = int(g.group(1))
                cmd = cmd[g.end():]
                if code in (1, 2, 3):
                    interp = f"G0{code}"
                elif code == 36:
                    region = []
                elif code == 37:
                    if region and len(region) > 2:
                        items.append((Polygon(region).buffer(0), dark))
                    region = None
                elif code == 4:
                    cmd = ""
                continue
            d = re.match(r"D0?(\d+)$", cmd) if not re.search(r"[XYIJ]", cmd) else None
            if d and int(d.group(1)) >= 10:
                cur_ap = apertures.get(int(d.group(1)))
                cmd = ""
                continue
            if cmd.startswith("M02") or cmd.startswith("M00"):
                cmd = ""
                continue
            coords = dict((k, v) for k, v in re.findall(r"([XYIJ])([+-]?[\d.]+)", cmd))
            op = re.search(r"D0?([123])\b", cmd)
            nx = _num(coords["X"], fmt, zeros) * scale if "X" in coords else x
            ny = _num(coords["Y"], fmt, zeros) * scale if "Y" in coords else y
            i = _num(coords.get("I", "0"), fmt, zeros) * scale
            j = _num(coords.get("J", "0"), fmt, zeros) * scale
            dcode = int(op.group(1)) if op else 1
            if dcode == 1:
                if interp == "G01":
                    seg = [(nx, ny)]
                else:
                    seg = _arc_points(x, y, nx, ny, x + i, y + j, interp == "G02")
                if region is not None:
                    if not region:
                        region.append((x, y))
                    region += seg
                elif cur_ap is not None:
                    width = cur_ap[1][0] if cur_ap[0] == "C" else min(cur_ap[1][:2]) if len(cur_ap[1]) > 1 else cur_ap[1][0]
                    items.append((LineString([(x, y)] + seg).buffer(width / 2, 12), dark))
            elif dcode == 2:
                if region is not None and len(region) > 2:
                    items.append((Polygon(region).buffer(0), dark))
                    region = []
            elif dcode == 3 and cur_ap is not None:
                items.append((_flash(cur_ap, nx, ny), dark))
            x, y = nx, ny
            cmd = ""
    geom = None
    for gm, is_dark in items:
        if gm.is_empty:
            continue
        if is_dark:
            geom = gm if geom is None else geom.union(gm)
        elif geom is not None:
            geom = geom.difference(gm)
    return geom


def excellon_geometry(text):
    """Drill holes as circles (mm, Y up)."""
    scale, dec = 1.0, 3
    zeros = "L"
    tools = {}
    cur = None
    holes = []
    in_header = False
    for line in text.replace("\r", "").split("\n"):
        ln = line.strip()
        if not ln or ln.startswith(";"):
            continue
        if ln == "M48":
            in_header = True
            continue
        if ln in ("%", "M95"):
            in_header = False
            continue
        if ln.startswith("INCH") or ln == "M72":
            scale, dec = 25.4, 4
            zeros = "T" if "TZ" in ln else "L"
            continue
        if ln.startswith("METRIC") or ln == "M71":
            scale, dec = 1.0, 3
            zeros = "T" if "TZ" in ln else "L"
            continue
        t = re.match(r"T(\d+)(?:F\d+)?(?:S\d+)?C([\d.]+)", ln)
        if t:
            tools[int(t.group(1))] = float(t.group(2)) * scale
            continue
        t = re.match(r"T(\d+)$", ln)
        if t and not in_header:
            cur = int(t.group(1))
            continue
        c = dict(re.findall(r"([XY])([+-]?[\d.]+)", ln))
        if c and cur is not None and not in_header:
            px = _num(c.get("X", "0"), (6 - dec, dec), zeros) * scale if "X" in c else (holes[-1][0] if holes else 0)
            py = _num(c.get("Y", "0"), (6 - dec, dec), zeros) * scale if "Y" in c else (holes[-1][1] if holes else 0)
            holes.append((px, py, tools.get(cur, 0.8)))
    if not holes:
        return None
    return unary_union([Point(x, y).buffer(d / 2, 24) for x, y, d in holes])


def geometry_rings(geom, ref_height):
    """Closed rings (points, True) in screen space: y' = ref_height - y."""
    out = []
    if geom is None or geom.is_empty:
        return out
    for g in getattr(geom, "geoms", [geom]):
        if not hasattr(g, "exterior"):
            continue
        for ring in [g.exterior, *g.interiors]:
            out.append(([(x, ref_height - y) for x, y in ring.coords], True))
    return out
