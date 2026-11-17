"""Interactive canvas tools and geometry helpers:
scissors, trim at intersections, manual tabs, start point / direction, two-point rotate & scale,
deform (4 corners) and warp (3x3 grid), plus Apollonius tangent circles, slot resizing and taper warp.
"""

import math

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QPainterPath, QPen, QPolygonF
from shapely.geometry import LineString, MultiPoint, Point
from shapely.ops import substring, unary_union

from . import model as M


# ====================================================================== path helpers
def path_length(path):
    return sum(math.hypot(path[i][0] - path[i - 1][0], path[i][1] - path[i - 1][1]) for i in range(1, len(path)))


def fraction_at(path, pt):
    ls = LineString(path)
    return ls.project(Point(pt)) / ls.length if ls.length else 0.0


def point_at_fraction(path, f):
    ls = LineString(path)
    q = ls.interpolate(f * ls.length)
    return (q.x, q.y)


def rotate_closed_path(path, f):
    """Closed path starting at fraction f (inserting the exact start point)."""
    if not M.is_closed_path(path) or f <= 0 or f >= 1:
        return path
    ls = LineString(path)
    L = ls.length
    a = substring(ls, f * L, L)
    b = substring(ls, 0, f * L)
    pts = list(a.coords) + list(b.coords)[1:]
    return pts


def densify(path, step=0.8):
    out = [path[0]]
    for i in range(1, len(path)):
        x0, y0 = path[i - 1]
        x1, y1 = path[i]
        n = max(1, int(math.hypot(x1 - x0, y1 - y0) / step))
        for k in range(1, n + 1):
            out.append((x0 + (x1 - x0) * k / n, y0 + (y1 - y0) * k / n))
    return out


def shape_hit(canvas, pt, kinds=("rect", "circle", "poly")):
    """Nearest top-level shape outline within tolerance → (shape, path, distance)."""
    tol = 6 / canvas.zoom
    best = None
    for s in reversed(canvas.doc.shapes):
        if s.get("locked") or s["type"] not in kinds:
            continue
        for path in M.shape_paths(s):
            if len(path) < 2:
                continue
            d = LineString(path).distance(Point(pt))
            if d < tol and (best is None or d < best[2]):
                best = (s, path, d)
    return best


def _pen(color, width=1.0, dash=None):
    pen = QPen(QColor(color), width)
    pen.setCosmetic(True)
    if dash:
        pen.setDashPattern(dash)
    return pen


class Tool:
    hint = ""

    def __init__(self, canvas):
        self.c = canvas

    @property
    def doc(self):
        return self.c.doc

    @property
    def win(self):
        return self.c.win

    def reset(self):
        pass

    def press(self, pt, shift):
        pass

    def move(self, pt):
        self.c.update()

    def release(self):
        pass

    def key(self, text):
        return False

    def draw(self, p):
        pass


# ====================================================================== scissors
class ScissorsTool(Tool):
    def press(self, pt, shift):
        hit = shape_hit(self.c, pt)
        if not hit:
            return
        s = hit[0]
        self.doc.save_undo()
        M.replace_in_place(s, M.to_poly(s))
        best = None
        for i in range(M.seg_count(s)):
            d, t, _ = M.nearest_on_segment(s, i, pt)
            if best is None or d < best[0]:
                best = (d, i, t)
        _, i, t = best
        a, b = M.seg_ends(s, i)
        if t < 1e-6:
            j = s["points"].index(a)
        elif t > 1 - 1e-6:
            j = s["points"].index(b)
        else:
            j = M.split_segment(s, i, t)
        parts = M.break_at_node(s, j)
        idx = self.doc.shapes.index(s)
        new = []
        for k, pts in enumerate(parts):
            ns = {kk: v for kk, v in s.items() if kk not in ("points", "regular", "tabs", "start")}
            ns.update(points=pts, closed=False)
            if k:
                ns["id"] = self.doc.new_id()
            new.append(ns)
        self.doc.shapes[idx:idx + 1] = new
        self.doc.selected = [n["id"] for n in new]
        self.doc.notify()
        self.win.status(self.win.L("Forma tagliata", "Shape cut"))


# ====================================================================== trim
class TrimTool(Tool):
    """Click a piece of line between two intersections to delete it."""

    def __init__(self, canvas):
        super().__init__(canvas)
        self.preview = None

    def _piece(self, pt):
        hit = shape_hit(self.c, pt, ("rect", "circle", "poly", "group"))
        if not hit:
            return None
        s, path = hit[0], hit[1]
        line = LineString(path)
        others = []
        for o in self.doc.shapes:
            for op in M.shape_paths(o):
                if len(op) >= 2 and (o is not s or op is not path):
                    others.append(LineString(op))
        L = line.length
        cuts = set()
        if others:
            inter = line.intersection(unary_union(others))
            geoms = getattr(inter, "geoms", [inter])
            for g in geoms:
                if g.is_empty:
                    continue
                for q in (g.coords if g.geom_type == "Point" else list(g.coords)):
                    d = line.project(Point(q))
                    if 1e-6 < d < L - 1e-6:
                        cuts.add(round(d, 6))
        closed = M.is_closed_path(path)
        d0 = line.project(Point(pt))
        cuts = sorted(cuts)
        if closed:
            if len(cuts) < 2:
                return s, path, None
            before = [c for c in cuts if c <= d0]
            after = [c for c in cuts if c > d0]
            a = before[-1] if before else cuts[-1]
            b = after[0] if after else cuts[0]
            if a < b:
                keep = [substring(line, b, L), substring(line, 0, a)]
                remove = substring(line, a, b)
            else:
                keep = [substring(line, b, a)]
                remove = unary_union([substring(line, a, L), substring(line, 0, b)])
            kept = []
            for k in keep:
                kept += list(k.coords) if not kept else list(k.coords)[1:]
            return s, path, ([kept] if len(kept) > 1 else [], remove)
        a = max([c for c in cuts if c <= d0], default=0.0)
        b = min([c for c in cuts if c > d0], default=L)
        keep = [list(substring(line, 0, a).coords)] if a > 1e-6 else []
        if b < L - 1e-6:
            keep.append(list(substring(line, b, L).coords))
        return s, path, ([k for k in keep if len(k) > 1], substring(line, a, b))

    def move(self, pt):
        res = self._piece(pt)
        self.preview = res[2][1] if res and res[2] else None
        self.c.update()

    def press(self, pt, shift):
        res = self._piece(pt)
        if not res or res[2] is None:
            self.win.toast(self.win.L("Nessuna intersezione su questa linea", "No intersections on this line"))
            return
        s, path, (keep, _) = res
        self.doc.save_undo()
        rest = list(M.shape_paths(s))
        if path in rest:
            rest.remove(path)
        layer = s.get("layer", 0)
        new = [{"id": self.doc.new_id(), "type": "poly", "layer": layer, "closed": False,
                "points": [{"x": x, "y": y} for x, y in k]} for k in keep]
        new += [{"id": self.doc.new_id(), "type": "poly", "layer": layer, "closed": M.is_closed_path(r),
                 "points": [{"x": x, "y": y} for x, y in (r[:-1] if M.is_closed_path(r) else r)]} for r in rest]
        idx = self.doc.shapes.index(s)
        self.doc.shapes[idx:idx + 1] = new
        self.doc.selected = [n["id"] for n in new]
        self.preview = None
        self.doc.notify()

    def draw(self, p):
        if self.preview is None or self.preview.is_empty:
            return
        p.setPen(_pen("#ff1a1a", 3))
        for g in getattr(self.preview, "geoms", [self.preview]):
            coords = list(g.coords)
            qp = QPainterPath()
            qp.moveTo(*coords[0])
            for q in coords[1:]:
                qp.lineTo(*q)
            p.drawPath(qp)


# ====================================================================== manual tabs
class TabsTool(Tool):
    """Click on an outline to add a tab, click an existing tab to remove it."""

    def press(self, pt, shift):
        hit = shape_hit(self.c, pt)
        if not hit:
            return
        s, path = hit[0], hit[1]
        if not M.is_closed_path(path):
            self.win.toast(self.win.L("I ponticelli servono sulle forme chiuse", "Tabs need closed shapes"))
            return
        f = fraction_at(path, pt)
        L = path_length(path)
        tabs = list(s.get("tabs", []))
        tol = (8 / self.c.zoom) / L
        near = [t for t in tabs if min(abs(t - f), 1 - abs(t - f)) < tol]
        self.doc.save_undo()
        if near:
            tabs.remove(near[0])
        else:
            tabs.append(round(f, 6))
        s["tabs"] = sorted(tabs)
        if not s["tabs"]:
            s.pop("tabs")
        self.doc.changed.emit()

    def draw(self, p):
        draw_tabs(self.c, p, all_shapes=True)


def draw_tabs(canvas, p, all_shapes=False):
    sel = set(canvas.doc.selected)
    for s in canvas.doc.shapes:
        if not s.get("tabs") or (not all_shapes and s["id"] not in sel):
            continue
        path = M.shape_paths(s)[0]
        L = path_length(path)
        width = canvas.win.engine.layers[s.get("layer", 0)].tab_width
        for f in s["tabs"]:
            a = max(0.0, f * L - width / 2)
            b = min(L, f * L + width / 2)
            seg = substring(LineString(path), a, b)
            coords = list(seg.coords)
            if len(coords) < 2:
                continue
            p.setPen(_pen("#ff9800", 4))
            qp = QPainterPath()
            qp.moveTo(*coords[0])
            for q in coords[1:]:
                qp.lineTo(*q)
            p.drawPath(qp)


# ====================================================================== start point & direction
class StartPointTool(Tool):
    """Click an outline to set where the cut starts; Shift-click reverses the direction."""

    def press(self, pt, shift):
        hit = shape_hit(self.c, pt)
        if not hit:
            return
        s, path = hit[0], hit[1]
        self.doc.save_undo()
        if shift:
            s["reverse"] = not s.get("reverse")
            if not s["reverse"]:
                s.pop("reverse")
        elif M.is_closed_path(path):
            s["start"] = round(fraction_at(path, pt), 6)
        self.doc.changed.emit()

    def draw(self, p):
        z = self.c.zoom
        for s in self.doc.shapes:
            if s["type"] not in ("rect", "circle", "poly"):
                continue
            path = M.shape_paths(s)[0]
            if len(path) < 2:
                continue
            path = job_path(s, path)
            x, y = path[0]
            p.setPen(Qt.NoPen)
            p.setBrush(QColor("#1a9d4a"))
            p.drawEllipse(QPointF(x, y), 4 / z, 4 / z)
            # direction arrow a little way along the path
            L = path_length(path)
            if L <= 0:
                continue
            a = point_at_fraction(path, min(0.999, 12 / z / L))
            b = point_at_fraction(path, min(1.0, 18 / z / L))
            ang = math.atan2(b[1] - a[1], b[0] - a[0])
            sz = 6 / z
            tri = QPolygonF([QPointF(b[0], b[1]),
                             QPointF(b[0] - sz * math.cos(ang - 0.5), b[1] - sz * math.sin(ang - 0.5)),
                             QPointF(b[0] - sz * math.cos(ang + 0.5), b[1] - sz * math.sin(ang + 0.5))])
            p.drawPolygon(tri)


def job_path(s, path):
    """Apply a shape's start point and direction to one of its outline paths."""
    if M.is_closed_path(path) and s.get("start"):
        path = rotate_closed_path(path, s["start"])
    if s.get("reverse"):
        path = path[::-1]
    return path


# ====================================================================== two-point rotate & scale
class TwoPointTool(Tool):
    """Click the base point, then a reference point, then where the reference should go.
    Shift on the last click also scales so the reference lands exactly on the target."""

    def __init__(self, canvas):
        super().__init__(canvas)
        self.pts = []
        self.cur = None

    def reset(self):
        self.pts, self.cur = [], None

    def press(self, pt, shift):
        if not self.doc.selected:
            self.win.toast(self.win.tr("selectFirst"))
            return
        sp = self.c.snap(pt)
        self.pts.append(sp)
        if len(self.pts) < 3:
            self.win.status(self.win.L(["", "Clic sul punto di riferimento", "Clic sulla destinazione (⇧ = anche scala)"][len(self.pts)],
                                       ["", "Click the reference point", "Click the destination (⇧ also scales)"][len(self.pts)]))
            self.c.update()
            return
        a, b, c = self.pts
        self.pts = []
        ang = math.degrees(math.atan2(c[1] - a[1], c[0] - a[0]) - math.atan2(b[1] - a[1], b[0] - a[0]))
        k = math.hypot(c[0] - a[0], c[1] - a[1]) / (math.hypot(b[0] - a[0], b[1] - a[1]) or 1)
        self.doc.save_undo()
        for s in self.doc.selected_shapes():
            M.rotate_shape(s, a[0], a[1], ang)
            self.win._fix_child_ids(s)
            if shift:
                M.scale_shape(s, a[0], a[1], k, k)
        M.clear_image_caches()
        self.doc.notify()
        self.win.status(self.win.L(f"Ruotato di {ang:.2f}°", f"Rotated {ang:.2f}°") + (f" · ×{k:.4f}" if shift else ""))

    def move(self, pt):
        self.cur = self.c.snap(pt)
        self.c.update()

    def draw(self, p):
        if not self.pts:
            return
        pts = self.pts + ([self.cur] if self.cur else [])
        p.setPen(_pen("#e01b1b", 1, [4, 3]))
        for i in range(1, len(pts)):
            p.drawLine(QPointF(*pts[0]), QPointF(*pts[i]))
        p.setPen(Qt.NoPen)
        p.setBrush(QColor("#e01b1b"))
        for q in self.pts:
            p.drawEllipse(QPointF(*q), 3.5 / self.c.zoom, 3.5 / self.c.zoom)


# ====================================================================== deform / warp
def bilinear(c, u, v):
    (x00, y00), (x10, y10), (x11, y11), (x01, y01) = c
    return ((1 - u) * (1 - v) * x00 + u * (1 - v) * x10 + u * v * x11 + (1 - u) * v * x01,
            (1 - u) * (1 - v) * y00 + u * (1 - v) * y10 + u * v * y11 + (1 - u) * v * y01)


def quad_patch(g, u, v):
    bu = ((1 - u) ** 2, 2 * u * (1 - u), u * u)
    bv = ((1 - v) ** 2, 2 * v * (1 - v), v * v)
    x = y = 0.0
    for j in range(3):
        for i in range(3):
            w = bu[i] * bv[j]
            x += w * g[j * 3 + i][0]
            y += w * g[j * 3 + i][1]
    return (x, y)


def warp_shapes(win, shapes, box, fn):
    """Replace shapes with densified copies mapped through fn(u, v) over bounding box `box`."""
    x0, y0, w, h = box
    out = []
    for s in shapes:
        if s["type"] == "image":
            out.append(s)
            continue
        paths = M.shape_paths(s)
        polys = []
        for path in paths:
            if len(path) < 2:
                continue
            closed = M.is_closed_path(path)
            dense = densify(path)
            mapped = [fn((x - x0) / (w or 1), (y - y0) / (h or 1)) for x, y in dense]
            if closed:
                mapped = mapped[:-1]
            polys.append({"id": win.doc.new_id(), "type": "poly", "layer": s.get("layer", 0), "closed": closed,
                          "points": [{"x": x, "y": y} for x, y in M.simplify(mapped, 0.02)]})
        if len(polys) == 1:
            polys[0]["id"] = s["id"]
            out.append(polys[0])
        elif polys:
            out.append({"id": s["id"], "type": "group", "layer": s.get("layer", 0), "children": polys})
    return out


class DeformTool(Tool):
    """Drag the handles around the selection: 4 corners (deform) or a 3x3 grid (warp)."""

    def __init__(self, canvas, warp=False):
        super().__init__(canvas)
        self.warp = warp
        self.reset()

    def reset(self):
        self.box = None
        self.grid = None
        self.orig = None
        self.drag = None

    def _ensure(self):
        sel = self.doc.selected_shapes()
        if not sel:
            self.reset()
            return False
        if self.box is None:
            b = M.multi_bounds(sel)
            self.box = (b[0], b[1], max(b[2], 1e-3), max(b[3], 1e-3))
            x0, y0, w, h = self.box
            if self.warp:
                self.grid = [(x0 + w * i / 2, y0 + h * j / 2) for j in range(3) for i in range(3)]
            else:
                self.grid = [(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)]
        return True

    def press(self, pt, shift):
        if not self._ensure():
            hit = self.c.hit_test(pt)
            if hit:
                self.doc.set_selection([hit["id"]])
                self._ensure()
            self.c.update()
            return
        tol = 7 / self.c.zoom
        for i, (x, y) in enumerate(self.grid):
            if abs(x - pt[0]) < tol and abs(y - pt[1]) < tol:
                self.doc.save_undo()
                self.orig = M.clone(self.doc.selected_shapes())
                self.drag = i
                return

    def move(self, pt):
        if self.drag is None:
            self._ensure()
            self.c.update()
            return
        self.grid[self.drag] = self.c.snap(pt)
        fn = (lambda u, v: quad_patch(self.grid, u, v)) if self.warp else (lambda u, v: bilinear(self.grid, u, v))
        ids = [s["id"] for s in self.orig]
        new = warp_shapes(self.win, M.clone(self.orig), self.box, fn)
        by_id = {n["id"]: n for n in new}
        self.doc.shapes = [by_id.get(s["id"], s) if s["id"] in ids else s for s in self.doc.shapes]
        self.c.update()

    def release(self):
        if self.drag is not None:
            self.drag = None
            self.doc.changed.emit()

    def draw(self, p):
        if not self._ensure():
            return
        g = self.grid
        p.setPen(_pen("#8e44ad", 1, [4, 3]))
        if self.warp:
            for j in range(3):
                for i in range(2):
                    p.drawLine(QPointF(*g[j * 3 + i]), QPointF(*g[j * 3 + i + 1]))
            for i in range(3):
                for j in range(2):
                    p.drawLine(QPointF(*g[j * 3 + i]), QPointF(*g[(j + 1) * 3 + i]))
        else:
            for i in range(4):
                p.drawLine(QPointF(*g[i]), QPointF(*g[(i + 1) % 4]))
        hs = 4.5 / self.c.zoom
        p.setPen(_pen("#8e44ad"))
        p.setBrush(QColor("#ffffff"))
        for x, y in g:
            p.drawRect(QRectF(x - hs, y - hs, hs * 2, hs * 2))


def taper(win, shapes, top_pct, bottom_pct):
    b = M.multi_bounds(shapes)
    x0, y0, w, h = b
    cx = x0 + w / 2
    t, bo = top_pct / 100, bottom_pct / 100
    corners = [(cx - w / 2 * t, y0), (cx + w / 2 * t, y0), (cx + w / 2 * bo, y0 + h), (cx - w / 2 * bo, y0 + h)]
    return warp_shapes(win, shapes, b, lambda u, v: bilinear(corners, u, v))


# ====================================================================== Apollonius tangent circles
def apollonius(c1, c2, c3):
    """All circles tangent to three circles (x, y, r): up to 8 solutions."""
    sols = []
    for s1 in (-1, 1):
        for s2 in (-1, 1):
            for s3 in (-1, 1):
                r = _apollonius_one(c1, c2, c3, s1, s2, s3)
                if r is None:
                    continue
                for cand in r:
                    if cand[2] > 1e-6 and not any(abs(cand[0] - o[0]) < 1e-6 and abs(cand[1] - o[1]) < 1e-6 and abs(cand[2] - o[2]) < 1e-6 for o in sols):
                        sols.append(cand)
    return sols


def _apollonius_one(c1, c2, c3, s1, s2, s3):
    (x1, y1, r1), (x2, y2, r2), (x3, y3, r3) = c1, c2, c3
    # |P - Ci| = r + si*ri  (si=+1 external, -1 internal); subtract pairwise -> linear in x, y, r
    a2, b2, c2_, d2 = 2 * (x2 - x1), 2 * (y2 - y1), 2 * (s2 * r2 - s1 * r1), (x2 * x2 + y2 * y2 - r2 * r2) - (x1 * x1 + y1 * y1 - r1 * r1)
    a3, b3, c3_, d3 = 2 * (x3 - x1), 2 * (y3 - y1), 2 * (s3 * r3 - s1 * r1), (x3 * x3 + y3 * y3 - r3 * r3) - (x1 * x1 + y1 * y1 - r1 * r1)
    # a*x + b*y + c*r = d   (with sign convention: subtracting gives -2(...)); solve x, y as functions of r
    det = a2 * b3 - a3 * b2
    if abs(det) < 1e-12:
        return None
    # x = (d2*b3 - d3*b2)/det - r*(c2*b3 - c3*b2)/det, likewise y
    xd, xr = (d2 * b3 - d3 * b2) / det, -(c2_ * b3 - c3_ * b2) / det
    yd, yr = (a2 * d3 - a3 * d2) / det, -(a2 * c3_ - a3 * c2_) / det
    # (x - x1)^2 + (y - y1)^2 = (r + s1*r1)^2
    ex, ey = xd - x1, yd - y1
    A = xr * xr + yr * yr - 1
    B = 2 * (ex * xr + ey * yr) - 2 * s1 * r1
    C = ex * ex + ey * ey - r1 * r1
    out = []
    if abs(A) < 1e-12:
        if abs(B) > 1e-12:
            rs = [-C / B]
        else:
            return None
    else:
        disc = B * B - 4 * A * C
        if disc < 0:
            return None
        sq = math.sqrt(disc)
        rs = [(-B + sq) / (2 * A), (-B - sq) / (2 * A)]
    for r in rs:
        if r > 0:
            out.append((xd + xr * r, yd + yr * r, r))
    return out


def circle_of(s):
    """(cx, cy, r) for a circle shape (or a closed shape treated by its bounding circle)."""
    if s["type"] == "circle":
        return (s["cx"], s["cy"], (s["rx"] + s["ry"]) / 2)
    b = M.bounds(s)
    return (b[0] + b[2] / 2, b[1] + b[3] / 2, (b[2] + b[3]) / 4)


# ====================================================================== slot resizer
def find_slots(s, width, tol=0.05):
    """Slots in a closed straight-edged poly: edges of length ≈ width joining two parallel, opposite edges."""
    pts = s["points"]
    n = len(pts)
    slots = []
    for i in range(n):
        a, b = pts[i], pts[(i + 1) % n]
        if any(k in a or k in b for k in ("c1", "c2")):
            continue
        L = math.hypot(b["x"] - a["x"], b["y"] - a["y"])
        if abs(L - width) > tol:
            continue
        pa, nb = pts[i - 1], pts[(i + 2) % n]
        u = ((b["x"] - a["x"]) / L, (b["y"] - a["y"]) / L)
        da = (a["x"] - pa["x"], a["y"] - pa["y"])
        db = (nb["x"] - b["x"], nb["y"] - b["y"])
        la, lb = math.hypot(*da), math.hypot(*db)
        if la < 1e-6 or lb < 1e-6:
            continue
        # side edges perpendicular to the slot bottom and pointing opposite ways
        if abs(da[0] * u[0] + da[1] * u[1]) / la > 0.02 or abs(db[0] * u[0] + db[1] * u[1]) / lb > 0.02:
            continue
        if (da[0] * db[0] + da[1] * db[1]) / (la * lb) > -0.98:
            continue
        slots.append(i)
    return slots


def resize_slots(s, old_w, new_w, tol=0.05):
    """Widen/narrow every slot of width old_w to new_w, symmetric about its centre line. Returns count."""
    slots = find_slots(s, old_w, tol)
    pts = s["points"]
    n = len(pts)
    moved = {}
    for i in slots:
        ia, ib = i, (i + 1) % n
        a, b = pts[ia], pts[ib]
        L = math.hypot(b["x"] - a["x"], b["y"] - a["y"])
        u = ((b["x"] - a["x"]) / L, (b["y"] - a["y"]) / L)
        d = (new_w - L) / 2
        # move the bottom corners and the mouth corners of each side wall
        for idx, sign in ((ia, -1), ((i - 1) % n, -1), (ib, 1), ((i + 2) % n, 1)):
            moved.setdefault(idx, [0.0, 0.0])
            moved[idx][0] += sign * u[0] * d
            moved[idx][1] += sign * u[1] * d
    for idx, (dx, dy) in moved.items():
        pts[idx]["x"] += dx
        pts[idx]["y"] += dy
    if slots:
        s.pop("regular", None)
    return len(slots)
