"""Corner tools: fillet (rounded corner on chosen nodes), dogbone and T-bone reliefs for joints."""

import math

from shapely.geometry import Point, Polygon

from . import model as M


def _unit(x, y):
    ln = math.hypot(x, y)
    return (x / ln, y / ln) if ln > 1e-12 else (0.0, 0.0)


def _corner(pts, i, closed):
    """(A, P, B) for node i when both adjacent segments are straight; None otherwise."""
    n = len(pts)
    if not closed and (i == 0 or i == n - 1):
        return None
    a, p, b = pts[i - 1], pts[i], pts[(i + 1) % n]
    if "c2" in a or "c1" in p or "c2" in p or "c1" in b:
        return None
    return (a["x"], a["y"]), (p["x"], p["y"]), (b["x"], b["y"])


def _arc_nodes(c, r, e1, e2, through):
    """Bezier nodes for the arc of circle (c, r) from e1 to e2 passing through `through`."""
    a1 = math.atan2(e1[1] - c[1], e1[0] - c[0])
    a2 = math.atan2(e2[1] - c[1], e2[0] - c[0])
    ap = math.atan2(through[1] - c[1], through[0] - c[0])
    ccw = (a2 - a1) % math.tau
    sweep = ccw if (ap - a1) % math.tau <= ccw else ccw - math.tau
    segs = max(1, math.ceil(abs(sweep) / (math.pi / 2)))
    step = sweep / segs
    h = 4 / 3 * math.tan(step / 4) * r
    nodes = []
    for k in range(segs + 1):
        ang = a1 + step * k
        x, y = c[0] + r * math.cos(ang), c[1] + r * math.sin(ang)
        tx, ty = -math.sin(ang), math.cos(ang)
        node = {"x": x, "y": y}
        if k > 0:
            node["c1"] = [x - tx * h, y - ty * h]
        if k < segs:
            node["c2"] = [x + tx * h, y + ty * h]
        nodes.append(node)
    return nodes


def fillet(s, radius, only=None):
    """Round the straight corners of poly s (all, or node indices in `only`). Returns number of corners changed."""
    pts, closed = s["points"], s.get("closed")
    out, changed = [], 0
    for i, p in enumerate(pts):
        cor = None if (only is not None and i not in only) else _corner(pts, i, closed)
        if cor is None:
            out.append(p)
            continue
        A, P, B = cor
        u1, u2 = _unit(A[0] - P[0], A[1] - P[1]), _unit(B[0] - P[0], B[1] - P[1])
        cos_t = max(-1.0, min(1.0, u1[0] * u2[0] + u1[1] * u2[1]))
        theta = math.acos(cos_t)
        if theta < 1e-3 or theta > math.pi - 1e-3:
            out.append(p)
            continue
        d = radius / math.tan(theta / 2)
        lim = 0.5 * min(math.hypot(A[0] - P[0], A[1] - P[1]), math.hypot(B[0] - P[0], B[1] - P[1]))
        r = radius
        if d > lim:
            d = lim
            r = d * math.tan(theta / 2)
        t1 = (P[0] + u1[0] * d, P[1] + u1[1] * d)
        t2 = (P[0] + u2[0] * d, P[1] + u2[1] * d)
        b = _unit(u1[0] + u2[0], u1[1] + u2[1])
        cdist = r / math.sin(theta / 2)
        c = (P[0] + b[0] * cdist, P[1] + b[1] * cdist)
        mid = (c[0] - b[0] * r, c[1] - b[1] * r)
        out += _arc_nodes(c, r, t1, t2, mid)
        changed += 1
    s["points"] = out
    s.pop("regular", None)
    return changed


def _ring_depths(shapes):
    rings = [(s, Polygon(M.shape_paths(s)[0])) for s in shapes if len(M.shape_paths(s)[0]) > 3]
    depth = {}
    for s, poly in rings:
        x, y = s["points"][0]["x"], s["points"][0]["y"]
        depth[s["id"]] = sum(1 for o, op in rings if o is not s and op.is_valid and op.contains(Point(x, y)))
    return depth


def relief(shapes, diameter, kind="dogbone", near_90=True):
    """Add dogbone or T-bone reliefs to the material-inner corners of closed polys. Returns corners changed."""
    r = diameter / 2
    depth = _ring_depths(shapes)
    total = 0
    for s in shapes:
        pts = s["points"]
        if not s.get("closed") or len(pts) < 3:
            continue
        area = sum(pts[i - 1]["x"] * pts[i]["y"] - pts[i]["x"] * pts[i - 1]["y"] for i in range(len(pts))) / 2
        orient = 1 if area > 0 else -1
        hole = depth.get(s["id"], 0) % 2 == 1
        out = []
        for i, p in enumerate(pts):
            cor = _corner(pts, i, True)
            if cor is None:
                out.append(p)
                continue
            A, P, B = cor
            cross = (P[0] - A[0]) * (B[1] - P[1]) - (P[1] - A[1]) * (B[0] - P[0])
            concave = cross * orient < 0
            if concave == hole or abs(cross) < 1e-9:
                out.append(p)
                continue
            u1, u2 = _unit(A[0] - P[0], A[1] - P[1]), _unit(B[0] - P[0], B[1] - P[1])
            theta = math.acos(max(-1.0, min(1.0, u1[0] * u2[0] + u1[1] * u2[1])))
            if near_90 and abs(math.degrees(theta) - 90) > 15:
                out.append(p)
                continue
            if kind == "tbone":
                # centre on the longer edge so the relief runs into the material across the shorter one
                la, lb = math.hypot(A[0] - P[0], A[1] - P[1]), math.hypot(B[0] - P[0], B[1] - P[1])
                u = u2 if lb >= la else u1
                c = (P[0] + u[0] * r, P[1] + u[1] * r)
            else:
                b = _unit(u1[0] + u2[0], u1[1] + u2[1])
                c = (P[0] + b[0] * r, P[1] + b[1] * r)
            t1 = 2 * (u1[0] * (c[0] - P[0]) + u1[1] * (c[1] - P[1]))
            t2 = 2 * (u2[0] * (c[0] - P[0]) + u2[1] * (c[1] - P[1]))
            e1 = (P[0] + u1[0] * max(t1, 0), P[1] + u1[1] * max(t1, 0))
            e2 = (P[0] + u2[0] * max(t2, 0), P[1] + u2[1] * max(t2, 0))
            if kind == "tbone":
                # half of the circle beyond the other edge's line, i.e. away from the pocket
                o = u1 if u is u2 else u2
                dot = o[0] * u[0] + o[1] * u[1]
                op = _unit(o[0] - dot * u[0], o[1] - dot * u[1])
                through = (c[0] - op[0] * r, c[1] - op[1] * r)
            else:
                through = P
            out += _arc_nodes(c, r, e1, e2, through)
            total += 1
        s["points"] = out
        s.pop("regular", None)
    return total
