"""Design document model: shapes stored as plain dicts (same schema as .lfproj files).

Shape types: rect {x,y,w,h}, circle {cx,cy,rx,ry}, poly {points:[{x,y}], closed}, text {x,y,text,size,font},
image {x,y,w,h,src,name,adj,flipX,flipY}, group {children}. All carry id, layer, optional locked.
"""

import base64
import copy
import os
import json
import datetime
import re
import io
import math

import numpy as np
from PIL import Image, ImageOps
from PySide6.QtCore import QPointF
from PySide6.QtGui import QFont, QFontMetricsF, QPainterPath

# ---------------------------------------------------------------- basic geometry

def clone(o):
    return copy.deepcopy(o)


def regular_polygon(cx, cy, r, n, rot=-math.pi / 2):
    return [{"x": cx + r * math.cos(rot + i / n * math.tau), "y": cy + r * math.sin(rot + i / n * math.tau)} for i in range(n)]


def star_points(cx, cy, outer, inner, n):
    pts = []
    for i in range(n * 2):
        a = -math.pi / 2 + i * math.pi / n
        r = outer if i % 2 == 0 else inner
        pts.append({"x": cx + r * math.cos(a), "y": cy + r * math.sin(a)})
    return pts


def path_box(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))


def center(b):
    return (b[0] + b[2] / 2, b[1] + b[3] / 2)


def dist_to_segment(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    l2 = dx * dx + dy * dy
    t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2 if l2 else 0
    t = max(0.0, min(1.0, t))
    return math.hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy))


def in_bounds(p, b, pad=0.0):
    return b[0] - pad <= p[0] <= b[0] + b[2] + pad and b[1] - pad <= p[1] <= b[1] + b[3] + pad


def is_closed_path(p):
    return len(p) > 2 and math.hypot(p[0][0] - p[-1][0], p[0][1] - p[-1][1]) < 1e-6


# ---------------------------------------------------------------- text

_FONT_PX = 100.0
_text_cache: dict = {}


def _font(family, bold=False, italic=False):
    f = QFont(family or "Helvetica")
    f.setPixelSize(int(_FONT_PX))
    f.setBold(bool(bold))
    f.setItalic(bool(italic))
    return f


HERSHEY_PREFIX = "Hershey:"
_hershey = {}


def hershey_names():
    try:
        from HersheyFonts import HersheyFonts
    except ImportError:
        return []
    skip = {"astrology", "markers", "mathlow", "mathupp", "meteorology", "music", "symbolic", "japanese"}
    return [n for n in HersheyFonts().default_font_names if n not in skip]


SHX_PREFIX = "SHX:"
SHX_FOLDER = [""]      # set from Settings › SHX font folder


def is_single_line(s):
    return str(s.get("font") or "").startswith((HERSHEY_PREFIX, SHX_PREFIX))


def single_line_fonts():
    """[(font id, label)] for every single-line font: Hershey styles plus SHX files in the chosen folder."""
    out = [(HERSHEY_PREFIX + n, n) for n in hershey_names()]
    try:
        from .shx import list_fonts
        out += [(SHX_PREFIX + p, "SHX · " + os.path.splitext(os.path.basename(p))[0]) for p in list_fonts(SHX_FOLDER[0])]
    except Exception:
        pass
    return out


def _hershey_strokes(name, text, size):
    from HersheyFonts import HersheyFonts
    f = _hershey.get(name)
    if f is None:
        f = HersheyFonts()
        f.load_default_font(name)
        _hershey[name] = f
    f.normalize_rendering(size)
    return [list(st) for st in f.strokes_for_text(text)]


def single_line_paths(s):
    """Open stroke paths of a single-line (Hershey) font, in mm. Height = size, baseline at 25% from the bottom."""
    text = resolve_text(s)
    size = s["size"]
    out = []
    if s["font"].startswith(SHX_PREFIX):
        from .shx import load
        try:
            strokes, _ = load(s["font"][len(SHX_PREFIX):]).text_strokes(text, size * 0.75)
        except Exception:
            return []
        base = s["y"] + size * 0.75
        return [[(s["x"] + px, base - py) for px, py in st] for st in strokes if len(st) >= 2]
    name = s["font"][len(HERSHEY_PREFIX):]
    for st in _hershey_strokes(name, text, size):
        if len(st) >= 2:
            out.append([(s["x"] + px, s["y"] + size - py) for px, py in st])
    return out


def _tfont(s):
    return _font(s.get("font"), s.get("bold"), s.get("italic"))


_VAR = re.compile(r"\{([^{}]+)\}")


def resolve_text(s):
    """Variable text: {serial} (alias {n}), {date}/{data}, {time}/{ora}, {csv:Column} or {Column} from the merge file."""
    var = s.get("var")
    if not var or not var.get("enabled"):
        return s["text"]
    now = datetime.datetime.now()
    rows = var.get("csv") or []
    row = rows[var.get("row", 0) % len(rows)] if rows else {}

    def sub(m):
        key = m.group(1).strip()
        low = key.lower()
        if low in ("serial", "n", "seriale"):
            return str(int(var.get("serial", 1))).zfill(int(var.get("digits", 1)))
        if low in ("date", "data"):
            return now.strftime(var.get("date_fmt", "%d/%m/%Y"))
        if low in ("time", "ora"):
            return now.strftime("%H:%M")
        if low.startswith("csv:"):
            key = key[4:].strip()
        if key in row:
            return str(row[key])
        return m.group(0)
    return _VAR.sub(sub, s["text"])


def advance_variable(s):
    var = s.get("var")
    if var and var.get("enabled"):
        var["serial"] = int(var.get("serial", 1)) + int(var.get("step", 1))
        if var.get("csv"):
            var["row"] = (var.get("row", 0) + 1) % len(var["csv"])
        return True
    return False


def text_width(s):
    if is_single_line(s):
        paths = text_paths(s)
        if not paths:
            return 0.0
        return max(x for p in paths for x, _ in p) - s["x"]
    text = resolve_text(s)
    w = QFontMetricsF(_tfont(s)).horizontalAdvance(text) * s["size"] / _FONT_PX
    return w + s.get("spacing", 0.0) * max(0, len(text) - 1)


def _text_baseline_path(s):
    """Baseline the glyphs follow: straight (with letter spacing) or an arc of signed radius `bend` (mm)."""
    w = text_width(s)
    x0, base = s["x"], s["y"] + s["size"] * 0.8
    R = s.get("bend", 0.0)
    if not R or abs(R) < 1e-6:
        return [(x0, base), (x0 + w, base)], 0.0
    xc = x0 + w / 2
    n = max(16, int(w / 0.5))
    pts = []
    for i in range(n + 1):
        d = -w / 2 + w * i / n
        t = d / R
        pts.append((xc + R * math.sin(t), base + R * (1 - math.cos(t))))
    return pts, 0.0


def text_paths(s):
    """Exact glyph outlines (Qt font engine), in mm, closed polygons."""
    text = resolve_text(s)
    key = (text, s["size"], s.get("font"), round(s["x"], 4), round(s["y"], 4), s.get("bold"), s.get("italic"),
           s.get("spacing", 0.0), s.get("bend", 0.0))
    hit = _text_cache.get(key)
    if hit is not None:
        return hit
    if is_single_line(s):
        out = single_line_paths(s)
        _text_cache[key] = out
        return out
    if s.get("spacing") or s.get("bend"):
        base, _ = _text_baseline_path(s)
        out = text_on_path(s, base, 0.0, 0.0, s.get("spacing", 0.0))
        _text_cache[key] = out
        return out
    path = QPainterPath()
    path.addText(QPointF(0, 0), _tfont(s), text)
    k = s["size"] / _FONT_PX
    ox, oy = s["x"], s["y"] + s["size"] * 0.8
    out = []
    for poly in path.toSubpathPolygons():
        pts = [(ox + p.x() * k, oy + p.y() * k) for p in poly]
        if len(pts) > 2:
            if pts[0] != pts[-1]:
                pts.append(pts[0])
            out.append(pts)
    if len(_text_cache) > 300:
        _text_cache.clear()
    _text_cache[key] = out
    return out


# ---------------------------------------------------------------- curves

def has_curves(s):
    return s["type"] == "poly" and any("c1" in p or "c2" in p for p in s["points"])


CURVE_TOL = 0.25   # max chord error (mm) when flattening curves; set from Settings › Curve quality


def cubic_points(p0, p1, p2, p3, tol=None):
    tol = tol or CURVE_TOL
    est = math.hypot(p1[0] - p0[0], p1[1] - p0[1]) + math.hypot(p2[0] - p1[0], p2[1] - p1[1]) + math.hypot(p3[0] - p2[0], p3[1] - p2[1])
    n = max(4, min(96, int(est / tol / 2) + 1))
    out = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        out.append((u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0],
                    u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1]))
    return out


def segment_points(a, b):
    """Points after `a` up to and including `b` (cubic if either handle is present)."""
    if "c2" in a or "c1" in b:
        p0 = (a["x"], a["y"])
        p1 = tuple(a.get("c2", p0))
        p3 = (b["x"], b["y"])
        p2 = tuple(b.get("c1", p3))
        return cubic_points(p0, p1, p2, p3)
    return [(b["x"], b["y"])]


def poly_flatten(points, closed):
    if not points:
        return []
    out = [(points[0]["x"], points[0]["y"])]
    n = len(points)
    for i in range(1, n):
        out += segment_points(points[i - 1], points[i])
    if closed and n > 2:
        out += segment_points(points[-1], points[0])
        out[-1] = out[0]
    return out


def map_point_dict(p, fn):
    p["x"], p["y"] = fn(p["x"], p["y"])
    for k in ("c1", "c2"):
        if k in p:
            p[k] = list(fn(*p[k]))


def rounded_rect(x, y, w, h, r):
    pts = []
    k = 16
    for cx, cy, a0 in ((x + w - r, y + r, -90), (x + w - r, y + h - r, 0), (x + r, y + h - r, 90), (x + r, y + r, 180)):
        for i in range(k + 1):
            a = math.radians(a0 + 90 * i / k)
            pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    pts.append(pts[0])
    return pts


def regular_meta_points(meta):
    if meta.get("inner"):
        pts = []
        n = meta["n"]
        for i in range(n * 2):
            a = meta.get("rot", -math.pi / 2) + i * math.pi / n
            r = meta["r"] if i % 2 == 0 else meta["r"] * meta["inner"]
            pts.append({"x": meta["cx"] + r * math.cos(a), "y": meta["cy"] + r * math.sin(a)})
        return pts
    return regular_polygon(meta["cx"], meta["cy"], meta["r"], meta["n"], meta.get("rot", -math.pi / 2))


# ---------------------------------------------------------------- bounds & paths

def bounds(s):
    t = s["type"]
    if t in ("rect", "image"):
        return (s["x"], s["y"], s["w"], s["h"])
    if t == "circle":
        return (s["cx"] - s["rx"], s["cy"] - s["ry"], s["rx"] * 2, s["ry"] * 2)
    if t == "text":
        if s.get("bend"):
            paths = text_paths(s)
            if paths:
                return path_box([p for path in paths for p in path])
        return (s["x"], s["y"], text_width(s), s["size"])
    if t == "poly":
        if not s["points"]:
            return (0, 0, 0, 0)
        if has_curves(s):
            return path_box(shape_paths(s)[0])
        xs = [p["x"] for p in s["points"]]
        ys = [p["y"] for p in s["points"]]
        return (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))
    if t == "group":
        return multi_bounds(s["children"]) or (0, 0, 0, 0)
    return (0, 0, 0, 0)


def multi_bounds(shapes):
    if not shapes:
        return None
    mx = my = math.inf
    Mx = My = -math.inf
    for s in shapes:
        x, y, w, h = bounds(s)
        mx, my = min(mx, x), min(my, y)
        Mx, My = max(Mx, x + w), max(My, y + h)
    return (mx, my, Mx - mx, My - my)


def shape_paths(s):
    """Outline paths as lists of (x, y) tuples; closed paths repeat the first point."""
    t = s["type"]
    if t == "rect":
        x, y, w, h = s["x"], s["y"], s["w"], s["h"]
        r = min(s.get("radius", 0.0), w / 2, h / 2)
        if r > 1e-6:
            return [rounded_rect(x, y, w, h, r)]
        return [[(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]]
    if t == "circle":
        r = max(s["rx"], s["ry"], 1e-6)
        n = max(24, min(720, int(math.tau / math.sqrt(8 * min(CURVE_TOL, r) / r)) + 1))
        return [[(s["cx"] + s["rx"] * math.cos(i / n * math.tau), s["cy"] + s["ry"] * math.sin(i / n * math.tau)) for i in range(n + 1)]]
    if t == "poly":
        return [poly_flatten(s["points"], s.get("closed"))]
    if t == "text":
        return text_paths(s)
    if t == "group":
        return [p for c in s["children"] for p in shape_paths(c)]
    return []


def flatten(shapes):
    out = []
    for s in shapes:
        if s["type"] == "group":
            kids = []
            for c in s["children"]:
                c2 = dict(c)
                if c2.get("layer") is None:
                    c2["layer"] = s.get("layer", 0)
                kids.append(c2)
            out += flatten(kids)
        else:
            out.append(s)
    return out


# ---------------------------------------------------------------- transforms

KAPPA = 0.5522847498


def to_poly(s):
    """Editable path: rectangles keep 4 corners (rounded corners as curves), ellipses become 4 Bezier nodes."""
    if s["type"] in ("poly", "group", "image", "text"):
        return s
    pts = None
    if s["type"] == "circle":
        cx, cy, rx, ry = s["cx"], s["cy"], s["rx"], s["ry"]
        kx, ky = rx * KAPPA, ry * KAPPA
        pts = [{"x": cx + rx, "y": cy, "c1": [cx + rx, cy - ky], "c2": [cx + rx, cy + ky]},
               {"x": cx, "y": cy + ry, "c1": [cx + kx, cy + ry], "c2": [cx - kx, cy + ry]},
               {"x": cx - rx, "y": cy, "c1": [cx - rx, cy + ky], "c2": [cx - rx, cy - ky]},
               {"x": cx, "y": cy - ry, "c1": [cx - kx, cy - ry], "c2": [cx + kx, cy - ry]}]
    elif s["type"] == "rect":
        x, y, w, h = s["x"], s["y"], s["w"], s["h"]
        r = min(s.get("radius", 0.0), w / 2, h / 2)
        if r > 1e-6:
            k = r * KAPPA
            pts = [{"x": x + r, "y": y}, {"x": x + w - r, "y": y, "c2": [x + w - r + k, y]},
                   {"x": x + w, "y": y + r, "c1": [x + w, y + r - k]}, {"x": x + w, "y": y + h - r, "c2": [x + w, y + h - r + k]},
                   {"x": x + w - r, "y": y + h, "c1": [x + w - r + k, y + h]}, {"x": x + r, "y": y + h, "c2": [x + r - k, y + h]},
                   {"x": x, "y": y + h - r, "c1": [x, y + h - r + k]}, {"x": x, "y": y + r, "c2": [x, y + r - k]}]
            pts[0]["c1"] = [x + r - k, y]
        else:
            pts = [{"x": x, "y": y}, {"x": x + w, "y": y}, {"x": x + w, "y": y + h}, {"x": x, "y": y + h}]
    if pts is None:
        path = shape_paths(s)[0][:-1]
        pts = [{"x": x, "y": y} for x, y in path]
    return {"id": s["id"], "type": "poly", "layer": s.get("layer", 0), "locked": s.get("locked", False),
            "points": pts, "closed": True}


def replace_in_place(s, new):
    if new is not s:
        s.clear()
        s.update(new)


def move_shape(s, dx, dy):
    t = s["type"]
    if t in ("rect", "text", "image"):
        s["x"] += dx
        s["y"] += dy
    elif t == "circle":
        s["cx"] += dx
        s["cy"] += dy
    elif t == "poly":
        for p in s["points"]:
            map_point_dict(p, lambda x, y: (x + dx, y + dy))
        if "regular" in s:
            s["regular"]["cx"] += dx
            s["regular"]["cy"] += dy
    elif t == "group":
        for c in s["children"]:
            move_shape(c, dx, dy)


def scale_shape(s, ox, oy, sx, sy):
    fx = lambda v: ox + (v - ox) * sx
    fy = lambda v: oy + (v - oy) * sy
    t = s["type"]
    if t in ("rect", "image"):
        x1, x2, y1, y2 = fx(s["x"]), fx(s["x"] + s["w"]), fy(s["y"]), fy(s["y"] + s["h"])
        s["x"], s["w"], s["y"], s["h"] = min(x1, x2), abs(x2 - x1), min(y1, y2), abs(y2 - y1)
        if s.get("radius"):
            s["radius"] *= min(abs(sx), abs(sy))
    elif t == "circle":
        s["cx"], s["cy"] = fx(s["cx"]), fy(s["cy"])
        s["rx"] *= abs(sx)
        s["ry"] *= abs(sy)
    elif t == "poly":
        for p in s["points"]:
            map_point_dict(p, lambda x, y: (fx(x), fy(y)))
        if "regular" in s:
            if abs(abs(sx) - abs(sy)) < 1e-9:
                m = s["regular"]
                m["cx"], m["cy"], m["r"] = fx(m["cx"]), fy(m["cy"]), m["r"] * abs(sx)
            else:
                s.pop("regular")
    elif t == "text":
        s["x"], s["y"] = fx(s["x"]), fy(s["y"])
        s["size"] *= abs(sy)
        if s.get("spacing"):
            s["spacing"] *= abs(sx)
        if s.get("bend"):
            s["bend"] *= abs(sy)
    elif t == "group":
        for c in s["children"]:
            scale_shape(c, ox, oy, sx, sy)


def rotate_point(p, cx, cy, deg):
    a = math.radians(deg)
    c, sn = math.cos(a), math.sin(a)
    dx, dy = p[0] - cx, p[1] - cy
    return (cx + dx * c - dy * sn, cy + dx * sn + dy * c)


def rotate_shape(s, cx, cy, deg):
    """Rotates in place. Text becomes outlines; images rotate only by multiples of 180° (flip both axes)."""
    t = s["type"]
    if t == "image":
        if deg % 180 == 0 and deg % 360 != 0:
            b = bounds(s)
            nx, ny = rotate_point(center(b), cx, cy, deg)
            s["x"], s["y"] = nx - b[2] / 2, ny - b[3] / 2
            s["flipX"] = not s.get("flipX")
            s["flipY"] = not s.get("flipY")
            return True
        if deg % 90 == 0:
            rotate_image_90(s, cx, cy, deg)
        else:
            rotate_image_any(s, cx, cy, deg)
        return True
    if t == "text":
        kids = [{"id": None, "type": "poly", "layer": s.get("layer", 0), "points": [{"x": x, "y": y} for x, y in p[:-1]], "closed": True}
                for p in text_paths(s)]
        replace_in_place(s, {"id": s["id"], "type": "group", "layer": s.get("layer", 0), "children": kids})
        t = "group"
    if t == "circle" and (abs(s["rx"] - s["ry"]) < 1e-9 or deg % 180 == 0):
        s["cx"], s["cy"] = rotate_point((s["cx"], s["cy"]), cx, cy, deg)
        return True
    if t == "group":
        return all([rotate_shape(c, cx, cy, deg) for c in s["children"]])
    replace_in_place(s, to_poly(s))
    for p in s["points"]:
        map_point_dict(p, lambda x, y: rotate_point((x, y), cx, cy, deg))
    if "regular" in s:
        m = s["regular"]
        m["cx"], m["cy"] = rotate_point((m["cx"], m["cy"]), cx, cy, deg)
        m["rot"] = m.get("rot", -math.pi / 2) + math.radians(deg)
    return True


def crop_mask(rings, cols, rows):
    """uint8 mask (1 inside) from crop rings in image-relative coordinates (0..1), even-odd."""
    from PIL import ImageChops, ImageDraw
    mask = Image.new("1", (cols, rows), 0)
    for ring in rings:
        if len(ring) < 3:
            continue
        r = Image.new("1", (cols, rows), 0)
        ImageDraw.Draw(r).polygon([(u * cols, v * rows) for u, v in ring], fill=1)
        mask = ImageChops.logical_xor(mask, r)
    return np.asarray(mask, dtype=np.uint8)


def set_live_crop(img_shape, crop_shape):
    """Attach a crop path (any closed shape) to an image, stored relative to the image box so it follows moves/scales."""
    x, y, w, h = img_shape["x"], img_shape["y"], img_shape["w"], img_shape["h"]
    rings = [[((px - x) / w, (py - y) / h) for px, py in p] for p in shape_paths(crop_shape) if len(p) > 3 and is_closed_path(p)]
    if rings:
        img_shape["crop"] = rings
    return bool(rings)


def flatten_crop(s):
    """Bake the live crop into the bitmap (white outside) and trim the image to the crop's bounding box."""
    rings = s.get("crop")
    img = decode_image(s["src"])
    if not rings or img is None:
        s.pop("crop", None)
        return
    if s.get("flipX"):
        img = ImageOps.mirror(img)
    if s.get("flipY"):
        img = ImageOps.flip(img)
    W, H = img.size
    mask = Image.fromarray(crop_mask(rings, W, H) * 255, "L")
    white = Image.new("RGB", img.size, (255, 255, 255))
    out = Image.composite(img, white, mask)
    us = [min(max(u, 0), 1) for r in rings for u, _ in r]
    vs = [min(max(v, 0), 1) for r in rings for _, v in r]
    box = (int(min(us) * W), int(min(vs) * H), max(int(min(us) * W) + 1, int(math.ceil(max(us) * W))), max(int(min(vs) * H) + 1, int(math.ceil(max(vs) * H))))
    out = out.crop(box)
    x0, y0, w0, h0 = s["x"], s["y"], s["w"], s["h"]
    s.update({"src": encode_image(out), "x": x0 + box[0] / W * w0, "y": y0 + box[1] / H * h0,
              "w": (box[2] - box[0]) / W * w0, "h": (box[3] - box[1]) / H * h0, "flipX": False, "flipY": False})
    s.pop("crop", None)


def rotate_image_90(s, cx, cy, deg):
    flatten_crop(s)
    img = decode_image(s["src"])
    if img is None:
        return
    if s.get("flipX"):
        img = ImageOps.mirror(img)
    if s.get("flipY"):
        img = ImageOps.flip(img)
    turns = int(round(deg / 90)) % 4
    img = img.rotate(-90 * turns, expand=True)
    b = bounds(s)
    nx, ny = rotate_point(center(b), cx, cy, deg)
    w, h = (b[3], b[2]) if turns % 2 else (b[2], b[3])
    s.update({"src": encode_image(img), "x": nx - w / 2, "y": ny - h / 2, "w": w, "h": h, "flipX": False, "flipY": False})


def rotate_image_any(s, cx, cy, deg):
    """Bakes an arbitrary rotation into the bitmap (white = no burn), keeping the pixel size in mm."""
    flatten_crop(s)
    img = decode_image(s["src"])
    if img is None:
        return
    if s.get("flipX"):
        img = ImageOps.mirror(img)
    if s.get("flipY"):
        img = ImageOps.flip(img)
    kx = img.width / max(s["w"], 1e-6)
    ky = img.height / max(s["h"], 1e-6)
    k = max(kx, ky)
    if abs(kx - ky) > 1e-3 * k:
        img = img.resize((max(1, round(s["w"] * k)), max(1, round(s["h"] * k))), Image.LANCZOS)
    rot = img.rotate(-deg, resample=Image.BICUBIC, expand=True, fillcolor=(255, 255, 255))
    b = bounds(s)
    nx, ny = rotate_point(center(b), cx, cy, deg)
    w, h = rot.width / k, rot.height / k
    s.update({"src": encode_image(rot), "x": nx - w / 2, "y": ny - h / 2, "w": w, "h": h, "flipX": False, "flipY": False})


def mirror_shape(s, axis, cx, cy):
    mx = lambda x: 2 * cx - x
    my = lambda y: 2 * cy - y
    t = s["type"]
    if t in ("rect", "image", "text"):
        b = bounds(s)
        if axis == "h":
            s["x"] = mx(b[0] + b[2])
            if t == "image":
                s["flipX"] = not s.get("flipX")
                if s.get("crop"):
                    s["crop"] = [[(1 - u, v) for u, v in r] for r in s["crop"]]
        else:
            s["y"] = my(b[1] + b[3])
            if t == "image":
                s["flipY"] = not s.get("flipY")
                if s.get("crop"):
                    s["crop"] = [[(u, 1 - v) for u, v in r] for r in s["crop"]]
    elif t == "circle":
        if axis == "h":
            s["cx"] = mx(s["cx"])
        else:
            s["cy"] = my(s["cy"])
    elif t == "poly":
        for p in s["points"]:
            map_point_dict(p, (lambda x, y: (mx(x), y)) if axis == "h" else (lambda x, y: (x, my(y))))
        s.pop("regular", None)
    elif t == "group":
        for c in s["children"]:
            mirror_shape(c, axis, cx, cy)


def set_layer(s, idx):
    s["layer"] = idx
    for c in s.get("children", []):
        set_layer(c, idx)


# ---------------------------------------------------------------- images

_img_cache: dict = {}
_proc_cache: dict = {}


def encode_image(img: Image.Image) -> str:
    alpha = img.info.get("lf_alpha")
    if alpha is not None and alpha.size == img.size:
        img = img.convert("RGBA")          # keep the transparency so it is never engraved
        img.putalpha(alpha)
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode()


def image_from_bytes(data: bytes) -> Image.Image | None:
    try:
        img = Image.open(io.BytesIO(data))
        img.load()
    except Exception:
        return None
    alpha = None
    if img.mode in ("RGBA", "LA", "P", "PA") or (img.mode in ("RGB", "L") and "transparency" in img.info):
        img = img.convert("RGBA")
        a = img.getchannel("A")
        if a.getextrema()[0] < 255:
            alpha = a                  # kept apart: transparent pixels are never engraved (like LightBurn), even inverted
        bg = Image.new("RGBA", img.size, (255, 255, 255, 255))
        bg.alpha_composite(img)
        img = bg
    out = img.convert("RGB")
    out.info["lf_alpha"] = alpha
    return out


def decode_image(src: str) -> Image.Image | None:
    hit = _img_cache.get(src)
    if hit is not None:
        return hit
    try:
        data = base64.b64decode(src.split(",", 1)[1])
    except Exception:
        return None
    img = image_from_bytes(data)
    if img is not None:
        if len(_img_cache) > 20:
            _img_cache.clear()
        _img_cache[src] = img
    return img


def default_adj():
    return {"mode": "dither", "brightness": 0, "contrast": 0, "gamma": 1.0, "invert": False, "threshold": 128,
            "cells_per_inch": 30, "halftone_angle": 22.5, "slices": 10,
            "enhance_radius": 0.0, "enhance_amount": 0, "enhance_denoise": 0}


MAX_RASTER_PX = 16000     # safety limit only; normal jobs keep the exact layer interval (LightBurn has no cap)


def process_image(s, interval, max_px=MAX_RASTER_PX, pass_through=False):
    """Returns (burn uint8 array rows x cols with 0..255 burn, cols, rows) or None.

    Same order as LightBurn: resample to the line interval, adjust (brightness, contrast, gamma, enhance),
    invert, dither, then mask out transparent pixels. With pass_through the image's own pixels are used
    unchanged (already dithered artwork): no resampling to the interval and no adjustments."""
    img = decode_image(s["src"])
    if img is None:
        return None
    adj = {**default_adj(), **(s.get("adj") or {})}
    if pass_through:
        cols, rows = img.size
    else:
        cols = max(1, round(s["w"] / interval))
        rows = max(1, round(s["h"] / interval))
    k = min(1.0, max_px / max(cols, rows))
    cols, rows = max(1, round(cols * k)), max(1, round(rows * k))
    crop = s.get("crop")
    key = (id(img), cols, rows, tuple(sorted(adj.items())), bool(s.get("flipX")), bool(s.get("flipY")),
           json.dumps(crop) if crop else None, bool(pass_through))
    hit = _proc_cache.get(key)
    if hit is not None:
        return hit
    g = img.convert("L")
    if (cols, rows) != g.size:
        g = g.resize((cols, rows), Image.NEAREST if pass_through else Image.LANCZOS)
    alpha = img.info.get("lf_alpha")
    if alpha is not None:
        alpha = alpha.resize((cols, rows), Image.BILINEAR)
    if s.get("flipX"):
        g = ImageOps.mirror(g)
        alpha = ImageOps.mirror(alpha) if alpha is not None else None
    if s.get("flipY"):
        g = ImageOps.flip(g)
        alpha = ImageOps.flip(alpha) if alpha is not None else None
    if pass_through:
        a = np.asarray(g, dtype=np.float32)
        burn = (255 - np.round(a)).astype(np.uint8) if adj["mode"] in ("gray", "slices3d") else np.where(a < 128, 255, 0).astype(np.uint8)
    else:
        amount, radius = int(adj.get("enhance_amount", 0) or 0), float(adj.get("enhance_radius", 0) or 0)
        if amount > 0 and radius > 0:
            from PIL import ImageFilter
            radius_px = radius / max(1e-6, s["w"] / cols)            # radius is in mm, like LightBurn's
            g = g.filter(ImageFilter.UnsharpMask(radius=radius_px, percent=amount, threshold=int(adj.get("enhance_denoise", 0) or 0)))
        a = np.asarray(g, dtype=np.float32)
        c = adj["contrast"] * 2.55
        cf = (259 * (c + 255)) / (255 * (259 - c))
        a = cf * (a - 128) + 128 + adj["brightness"] * 2.55
        a = np.clip(a, 0, 255)
        gamma = float(adj.get("gamma", 1.0) or 1.0)
        if abs(gamma - 1.0) > 1e-3:
            a = 255.0 * np.power(a / 255.0, 1.0 / gamma)          # >1 lightens mid-tones, <1 darkens them
        if adj["invert"]:
            a = 255 - a
        a = np.clip(a, 0, 255)
        burn = dither(a, adj, s["w"] / cols)
    if alpha is not None:
        burn = np.where(np.asarray(alpha) >= 128, burn, 0).astype(np.uint8)
    if crop:
        burn = burn * crop_mask(crop, cols, rows)
    res = (burn, cols, rows)
    if len(_proc_cache) > 40:
        _proc_cache.clear()
    _proc_cache[key] = res
    return res


# error-diffusion kernels: (dx, dy, weight) relative to the current pixel, and the divisor
KERNELS = {
    "jarvis": ([(1, 0, 7), (2, 0, 5), (-2, 1, 3), (-1, 1, 5), (0, 1, 7), (1, 1, 5), (2, 1, 3),
                (-2, 2, 1), (-1, 2, 3), (0, 2, 5), (1, 2, 3), (2, 2, 1)], 48),
    "stucki": ([(1, 0, 8), (2, 0, 4), (-2, 1, 2), (-1, 1, 4), (0, 1, 8), (1, 1, 4), (2, 1, 2),
                (-2, 2, 1), (-1, 2, 2), (0, 2, 4), (1, 2, 2), (2, 2, 1)], 42),
    "atkinson": ([(1, 0, 1), (2, 0, 1), (-1, 1, 1), (0, 1, 1), (1, 1, 1), (0, 2, 1)], 8),   # diffuses 6/8: lighter
    "sierra": ([(1, 0, 5), (2, 0, 3), (-2, 1, 2), (-1, 1, 4), (0, 1, 5), (1, 1, 4), (2, 1, 2),
                (-1, 2, 2), (0, 2, 3), (1, 2, 2)], 32),
}
DITHER_MODES = ["dither", "jarvis", "stucki", "atkinson", "sierra", "bayer", "halftone", "newsprint", "sketch", "gray", "threshold", "slices3d"]

_BAYER8 = np.array([[0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26], [12, 44, 4, 36, 14, 46, 6, 38],
                    [60, 28, 52, 20, 62, 30, 54, 22], [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
                    [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21]], dtype=np.float32)


def _error_diffuse(a, kernel, div):
    """Serpentine error diffusion on a float array 0..255 (255 = white). Returns burn 0/255."""
    h, w = a.shape
    buf = [list(map(float, row)) for row in a]
    out = np.zeros((h, w), dtype=np.uint8)
    for y in range(h):
        row = buf[y]
        rev = y % 2 == 1
        xs = range(w - 1, -1, -1) if rev else range(w)
        sign = -1 if rev else 1
        orow = out[y]
        for x in xs:
            old = row[x]
            new = 255.0 if old >= 128 else 0.0
            if new == 0.0:
                orow[x] = 255
            err = old - new
            if err == 0:
                continue
            for dx, dy, wgt in kernel:
                xx = x + dx * sign
                yy = y + dy
                if 0 <= xx < w and yy < h:
                    buf[yy][xx] += err * wgt / div
    return out


def _halftone(a, px_mm, cell_mm, angle_deg):
    """Clustered round dots on a rotated screen: darker areas get bigger dots."""
    h, w = a.shape
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    xx *= px_mm
    yy *= px_mm
    t = math.radians(angle_deg)
    u = (xx * math.cos(t) + yy * math.sin(t)) / cell_mm
    v = (-xx * math.sin(t) + yy * math.cos(t)) / cell_mm
    du, dv = u - np.floor(u) - 0.5, v - np.floor(v) - 0.5
    dist = np.sqrt(du * du + dv * dv)                   # cell units, 0 at the centre
    darkness = 1 - a / 255.0
    # dot radius whose area equals the darkness (dots merge above ~78 %)
    radius = np.sqrt(darkness / math.pi)
    radius = np.where(darkness > 0.785, np.sqrt(darkness) * 0.7071 + 0.0, radius)
    return np.where(dist < radius, 255, 0).astype(np.uint8)


def _sketch(a, threshold):
    gy, gx = np.gradient(a)
    mag = np.hypot(gx, gy)                                  # grey levels per pixel
    return np.where(mag > (256 - threshold) / 6.0, 255, 0).astype(np.uint8)


def dither(a, adj, px_mm=0.1):
    """Convert a grey image (float 0..255, 255 = white) to burn values 0..255 with the chosen method."""
    mode = adj.get("mode", "dither")
    if mode == "threshold":
        return np.where(a < adj["threshold"], 255, 0).astype(np.uint8)
    if mode in ("gray", "slices3d"):
        return (255 - np.round(a)).astype(np.uint8)
    if mode in KERNELS:
        kernel, div = KERNELS[mode]
        return _error_diffuse(a, kernel, div)
    if mode == "bayer":
        h, w = a.shape
        thr = np.tile((_BAYER8 + 0.5) * 4, (h // 8 + 1, w // 8 + 1))[:h, :w]
        return np.where(a < thr, 255, 0).astype(np.uint8)
    if mode in ("halftone", "newsprint"):
        cell = 25.4 / max(1.0, float(adj.get("cells_per_inch", 30 if mode == "halftone" else 50)))
        angle = float(adj.get("halftone_angle", 22.5)) if mode == "halftone" else 45.0
        return _halftone(a, px_mm, cell, angle)
    if mode == "sketch":
        return _sketch(a, adj.get("threshold", 128))
    dith = Image.fromarray(a.astype(np.uint8), "L").convert("1", dither=Image.Dither.FLOYDSTEINBERG)
    return np.where(np.asarray(dith, dtype=bool), 0, 255).astype(np.uint8)


def clear_image_caches():
    _proc_cache.clear()


# ---------------------------------------------------------------- tracing

def trace_binary(on: np.ndarray):
    """Marching squares on a boolean array (rows x cols). Returns closed loops in pixel-corner coordinates."""
    rows, cols = on.shape
    pad = np.zeros((rows + 2, cols + 2), dtype=np.uint8)
    pad[1:-1, 1:-1] = on
    idx = pad[:-1, :-1] * 8 + pad[:-1, 1:] * 4 + pad[1:, 1:] * 2 + pad[1:, :-1]
    adj: dict = {}

    def link(a, b):
        adj.setdefault(a, []).append(b)
        adj.setdefault(b, []).append(a)

    ys, xs = np.nonzero((idx != 0) & (idx != 15))
    for yy, xx in zip(ys.tolist(), xs.tolist()):
        v = int(idx[yy, xx])
        x, y = xx - 1, yy - 1
        T, R, B, L = (2 * x + 1, 2 * y), (2 * x + 2, 2 * y + 1), (2 * x + 1, 2 * y + 2), (2 * x, 2 * y + 1)
        if v in (1, 14):
            link(L, B)
        elif v in (2, 13):
            link(B, R)
        elif v in (3, 12):
            link(L, R)
        elif v in (4, 11):
            link(T, R)
        elif v in (6, 9):
            link(T, B)
        elif v in (7, 8):
            link(T, L)
        elif v == 5:
            link(T, L)
            link(B, R)
        elif v == 10:
            link(T, R)
            link(L, B)
    loops = []
    for start in list(adj.keys()):
        lst = adj[start]
        while lst:
            path = [start]
            cur, nxt = start, lst.pop()
            adj[nxt].remove(cur)
            while nxt != start:
                path.append(nxt)
                nl = adj[nxt]
                if not nl:
                    break
                n2 = nl.pop()
                adj[n2].remove(nxt)
                cur, nxt = nxt, n2
            if nxt == start:
                path.append(start)
            if len(path) > 3:
                loops.append([(px / 2 + 0.5, py / 2 + 0.5) for px, py in path])
    return loops


def simplify(pts, tol):
    if len(pts) < 3:
        return pts
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    while stack:
        a, b = stack.pop()
        maxd, idx = 0.0, -1
        for i in range(a + 1, b):
            d = dist_to_segment(pts[i], pts[a], pts[b])
            if d > maxd:
                maxd, idx = d, i
        if maxd > tol and idx > 0:
            keep[idx] = True
            stack += [(a, idx), (idx, b)]
    return [p for p, k in zip(pts, keep) if k]


# ---------------------------------------------------------------- SVG

def parse_svg_path(d):
    import re
    subs = []
    cur = None
    x = y = sx = sy = 0.0
    last_c = None
    tokens = re.findall(r"[a-zA-Z]|-?\d*\.?\d+(?:[eE][-+]?\d+)?", d)
    i = 0
    cmd = ""

    def n():
        nonlocal i
        v = float(tokens[i])
        i += 1
        return v

    def push(px, py):
        cur["points"].append((px, py))

    while i < len(tokens):
        if tokens[i].isalpha():
            cmd = tokens[i]
            i += 1
            if cmd in "zZ":
                if cur:
                    cur["closed"] = True
                    x, y = sx, sy
                last_c = None
                continue
        if i >= len(tokens):
            break
        rel = cmd.islower()
        ox, oy = (x, y) if rel else (0.0, 0.0)
        C = cmd.upper()
        try:
            if C == "M":
                x, y = ox + n(), oy + n()
                sx, sy = x, y
                cur = {"points": [(x, y)], "closed": False}
                subs.append(cur)
                cmd = "l" if rel else "L"
                last_c = None
            elif cur is None:
                cur = {"points": [(x, y)], "closed": False}
                subs.append(cur)
                continue
            elif C == "L":
                x, y = ox + n(), oy + n()
                push(x, y)
                last_c = None
            elif C == "H":
                x = (x if rel else 0) + n()
                push(x, y)
                last_c = None
            elif C == "V":
                y = (y if rel else 0) + n()
                push(x, y)
                last_c = None
            elif C in "CS":
                if C == "C":
                    x1, y1 = ox + n(), oy + n()
                else:
                    x1, y1 = (2 * x - last_c[0], 2 * y - last_c[1]) if last_c else (x, y)
                x2, y2, ex, ey = ox + n(), oy + n(), ox + n(), oy + n()
                for k in range(1, 13):
                    t = k / 12
                    u = 1 - t
                    push(u ** 3 * x + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t ** 3 * ex,
                         u ** 3 * y + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t ** 3 * ey)
                last_c = (x2, y2)
                x, y = ex, ey
            elif C in "QT":
                if C == "Q":
                    x1, y1 = ox + n(), oy + n()
                else:
                    x1, y1 = (2 * x - last_c[0], 2 * y - last_c[1]) if last_c else (x, y)
                ex, ey = ox + n(), oy + n()
                for k in range(1, 11):
                    t = k / 10
                    u = 1 - t
                    push(u * u * x + 2 * u * t * x1 + t * t * ex, u * u * y + 2 * u * t * y1 + t * t * ey)
                last_c = (x1, y1)
                x, y = ex, ey
            elif C == "A":
                for _ in range(5):
                    n()
                x, y = ox + n(), oy + n()
                push(x, y)
                last_c = None
            else:
                i += 1
        except (IndexError, ValueError):
            break
    return subs


# ---------------------------------------------------------------- text on path

def text_on_path(s, path_pts, start=0.0, distance=0.0, spacing=0.0, flip=False):
    """Glyph outlines of text shape s laid out along a polyline. Returns closed polygons [(x, y), ...]."""
    if flip:
        path_pts = path_pts[::-1]
    seglen = [math.hypot(path_pts[i][0] - path_pts[i - 1][0], path_pts[i][1] - path_pts[i - 1][1]) for i in range(1, len(path_pts))]
    total = sum(seglen)
    if total <= 0:
        return []

    def at(d):
        d = max(0.0, min(total, d))
        for i, L in enumerate(seglen):
            if d <= L or i == len(seglen) - 1:
                k = d / L if L else 0
                a, b = path_pts[i], path_pts[i + 1]
                return (a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, math.atan2(b[1] - a[1], b[0] - a[0]))
            d -= L
        return (path_pts[-1][0], path_pts[-1][1], 0.0)

    font = _tfont(s)
    fm = QFontMetricsF(font)
    k = s["size"] / _FONT_PX
    text = resolve_text(s)
    out = []
    pos = start
    for ch in text:
        adv = fm.horizontalAdvance(ch) * k
        if ch.strip():
            x, y, ang = at(pos + adv / 2)
            ca, sa = math.cos(ang), math.sin(ang)
            qp = QPainterPath()
            qp.addText(QPointF(0, 0), font, ch)
            for poly in qp.toSubpathPolygons():
                pts = []
                for p in poly:
                    gx = p.x() * k - adv / 2
                    gy = p.y() * k - distance
                    pts.append((x + gx * ca - gy * sa, y + gx * sa + gy * ca))
                if len(pts) > 2:
                    if pts[0] != pts[-1]:
                        pts.append(pts[0])
                    out.append(pts)
        pos += adv + spacing
    return out


# ---------------------------------------------------------------- node operations (paths with Bezier handles)

def seg_count(s):
    n = len(s["points"])
    return n if s.get("closed") and n > 2 else max(0, n - 1)


def seg_ends(s, i):
    pts = s["points"]
    return pts[i], pts[(i + 1) % len(pts)]


def is_curve_seg(s, i):
    a, b = seg_ends(s, i)
    return "c2" in a or "c1" in b


def _lerp(p, q, t):
    return (p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t)


def seg_point(s, i, t):
    a, b = seg_ends(s, i)
    p0, p3 = (a["x"], a["y"]), (b["x"], b["y"])
    if not is_curve_seg(s, i):
        return _lerp(p0, p3, t)
    p1, p2 = tuple(a.get("c2", p0)), tuple(b.get("c1", p3))
    u = 1 - t
    return (u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0],
            u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1])


def nearest_on_segment(s, i, pt, samples=48):
    """(distance, t, (x, y)) of the closest point of segment i to pt."""
    best = (math.inf, 0.0, None)
    n = samples if is_curve_seg(s, i) else 1
    if n == 1:
        a, b = seg_ends(s, i)
        ax, ay, bx, by = a["x"], a["y"], b["x"], b["y"]
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        t = max(0.0, min(1.0, ((pt[0] - ax) * dx + (pt[1] - ay) * dy) / l2)) if l2 else 0.0
        q = (ax + dx * t, ay + dy * t)
        return (math.hypot(pt[0] - q[0], pt[1] - q[1]), t, q)
    prev = seg_point(s, i, 0.0)
    for k in range(1, n + 1):
        t1 = k / n
        cur = seg_point(s, i, t1)
        dx, dy = cur[0] - prev[0], cur[1] - prev[1]
        l2 = dx * dx + dy * dy
        u = max(0.0, min(1.0, ((pt[0] - prev[0]) * dx + (pt[1] - prev[1]) * dy) / l2)) if l2 else 0.0
        q = (prev[0] + dx * u, prev[1] + dy * u)
        d = math.hypot(pt[0] - q[0], pt[1] - q[1])
        if d < best[0]:
            best = (d, (k - 1 + u) / n, q)
        prev = cur
    return best


def split_segment(s, i, t):
    """Insert a node on segment i at parameter t (exact de Casteljau split for curves). Returns new node index."""
    pts = s["points"]
    a, b = seg_ends(s, i)
    j = i + 1
    if not is_curve_seg(s, i):
        x, y = _lerp((a["x"], a["y"]), (b["x"], b["y"]), t)
        pts.insert(j, {"x": x, "y": y})
        return j
    p0, p3 = (a["x"], a["y"]), (b["x"], b["y"])
    p1, p2 = tuple(a.get("c2", p0)), tuple(b.get("c1", p3))
    q0, q1, q2 = _lerp(p0, p1, t), _lerp(p1, p2, t), _lerp(p2, p3, t)
    r0, r1 = _lerp(q0, q1, t), _lerp(q1, q2, t)
    m = _lerp(r0, r1, t)
    a["c2"] = list(q0)
    b["c1"] = list(q2)
    pts.insert(j, {"x": m[0], "y": m[1], "c1": list(r0), "c2": list(r1), "smooth": True})
    return j


def delete_nodes(s, indices):
    keep = [p for k, p in enumerate(s["points"]) if k not in set(indices)]
    s["points"] = keep
    s.pop("regular", None)
    return len(keep) >= 2


def make_curve(s, i):
    a, b = seg_ends(s, i)
    p0, p3 = (a["x"], a["y"]), (b["x"], b["y"])
    a.setdefault("c2", list(_lerp(p0, p3, 1 / 3)))
    b.setdefault("c1", list(_lerp(p0, p3, 2 / 3)))
    s.pop("regular", None)


def make_line(s, i):
    a, b = seg_ends(s, i)
    a.pop("c2", None)
    b.pop("c1", None)
    s.pop("regular", None)


def set_smooth(s, idx, smooth=True):
    """Smooth node: collinear handles (created from the neighbours if missing). Corner node: independent handles."""
    pts = s["points"]
    p = pts[idx]
    s.pop("regular", None)
    if not smooth:
        p.pop("smooth", None)
        return
    n = len(pts)
    closed = s.get("closed") and n > 2
    prev = pts[idx - 1] if (idx > 0 or closed) else None
    nxt = pts[(idx + 1) % n] if (idx < n - 1 or closed) else None
    if prev is None and nxt is None:
        return
    if prev is not None and nxt is not None:
        dx, dy = nxt["x"] - prev["x"], nxt["y"] - prev["y"]
    elif nxt is not None:
        dx, dy = nxt["x"] - p["x"], nxt["y"] - p["y"]
    else:
        dx, dy = p["x"] - prev["x"], p["y"] - prev["y"]
    ln = math.hypot(dx, dy) or 1.0
    ux, uy = dx / ln, dy / ln
    lin = math.hypot(p["x"] - prev["x"], p["y"] - prev["y"]) / 3 if prev is not None else 0
    lout = math.hypot(nxt["x"] - p["x"], nxt["y"] - p["y"]) / 3 if nxt is not None else 0
    if prev is not None:
        p["c1"] = [p["x"] - ux * lin, p["y"] - uy * lin]
    if nxt is not None:
        p["c2"] = [p["x"] + ux * lout, p["y"] + uy * lout]
    p["smooth"] = True


def mirror_handle(p, moved):
    """Keep a smooth node's opposite handle collinear (keeping its own length)."""
    other = "c1" if moved == "c2" else "c2"
    if not p.get("smooth") or other not in p:
        return
    hx, hy = p[moved][0] - p["x"], p[moved][1] - p["y"]
    ln = math.hypot(hx, hy)
    if ln < 1e-9:
        return
    ol = math.hypot(p[other][0] - p["x"], p[other][1] - p["y"])
    p[other] = [p["x"] - hx / ln * ol, p["y"] - hy / ln * ol]


def break_at_node(s, idx):
    """Closed path: opens it at the node. Open path: splits into two paths. Returns list of point lists."""
    pts = s["points"]
    if s.get("closed") and len(pts) > 2:
        rot = pts[idx:] + pts[:idx]
        rot[0] = {k: v for k, v in rot[0].items() if k != "c1"}
        last = clone(pts[idx])
        last.pop("c2", None)
        rot.append(last)
        return [rot]
    if idx <= 0 or idx >= len(pts) - 1:
        return [pts]
    a = pts[:idx + 1]
    b = [clone(pts[idx])] + pts[idx + 1:]
    a[-1] = {k: v for k, v in a[-1].items() if k != "c2"}
    b[0].pop("c1", None)
    return [a, b]


def delete_segment(s, i):
    """Remove the segment between node i and i+1. Returns list of point lists."""
    pts = s["points"]
    n = len(pts)
    if s.get("closed") and n > 2:
        rot = pts[i + 1:] + pts[:i + 1]
        rot[0] = {k: v for k, v in rot[0].items() if k != "c1"}
        rot[-1] = {k: v for k, v in rot[-1].items() if k != "c2"}
        return [rot]
    a = pts[:i + 1]
    b = pts[i + 1:]
    if a:
        a[-1] = {k: v for k, v in a[-1].items() if k != "c2"}
    if b:
        b[0] = {k: v for k, v in b[0].items() if k != "c1"}
    return [x for x in (a, b) if len(x) >= 2]
