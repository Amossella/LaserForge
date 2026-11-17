"""True-shape nesting: places parts by their real outline (not their bounding box).

Raster method: each part (grown by half the spacing) is rasterised at several rotations; an FFT
correlation against the sheet occupancy finds every collision-free position, and the part goes to the
top-most, then left-most, free spot. Largest parts are placed first.
"""

import math

import numpy as np
from PIL import Image, ImageDraw
from shapely import affinity
from shapely.geometry import MultiPolygon, Polygon, box
from shapely.ops import unary_union

from . import model as M


def part_outline(s):
    """Filled outline of a shape (closed paths even-odd; open shapes use their thin line hull)."""
    polys, others = [], []
    for path in M.shape_paths(s):
        if len(path) > 3 and M.is_closed_path(path):
            pg = Polygon(path).buffer(0)
            if not pg.is_empty:
                polys.append(pg)
        elif len(path) >= 2:
            from shapely.geometry import LineString
            others.append(LineString(path).buffer(0.05))
    geom = None
    for pg in polys:
        geom = pg if geom is None else geom.symmetric_difference(pg)
    if others:
        geom = unary_union(([geom] if geom is not None else []) + others)
    if geom is None or geom.is_empty:
        b = M.bounds(s)
        geom = box(b[0], b[1], b[0] + max(b[2], 0.1), b[1] + max(b[3], 0.1))
    # holes are not used for placing other parts inside them (kept simple and safe)
    if isinstance(geom, MultiPolygon):
        return unary_union([Polygon(g.exterior) for g in geom.geoms])
    return Polygon(geom.exterior) if hasattr(geom, "exterior") else geom


def _raster(geom, res, ox, oy, W, H):
    img = Image.new("1", (W, H), 0)
    d = ImageDraw.Draw(img)
    for g in getattr(geom, "geoms", [geom]):
        if g.is_empty:
            continue
        pts = [((x - ox) / res, (y - oy) / res) for x, y in g.exterior.coords]
        if len(pts) > 2:
            d.polygon(pts, fill=1)
    return np.asarray(img, dtype=np.float32)


def nest(shapes, sheet, spacing=2.0, rotations=(0, 90, 180, 270), res=None):
    """sheet = (x, y, w, h). Returns (placements, unplaced) with placements [(shape, angle, dx, dy)] where the shape
    is rotated by `angle` about (0, 0) and then moved by (dx, dy)."""
    sx, sy, sw, sh = sheet
    if res is None:
        res = max(0.25, max(sw, sh) / 700)
    GW, GH = int(math.ceil(sw / res)), int(math.ceil(sh / res))
    occ = np.zeros((GH, GW), dtype=np.float32)
    parts = []
    for s in shapes:
        g = part_outline(s).buffer(spacing / 2, join_style="mitre")
        parts.append((s, g))
    parts.sort(key=lambda it: -it[1].area)
    placements, unplaced = [], []
    F = np.fft.rfft2
    IF = np.fft.irfft2
    for s, g in parts:
        best = None
        for ang in rotations:
            rg = affinity.rotate(g, ang, origin=(0, 0))
            minx, miny, maxx, maxy = rg.bounds
            pw, ph = int(math.ceil((maxx - minx) / res)) + 1, int(math.ceil((maxy - miny) / res)) + 1
            if pw > GW or ph > GH:
                continue
            mask = _raster(rg, res, minx, miny, pw, ph)
            # collision count for every offset via FFT correlation (flipped mask convolution)
            shape = (GH + ph, GW + pw)
            corr = IF(F(occ, shape) * F(mask[::-1, ::-1], shape), shape)
            valid = corr[ph - 1:GH, pw - 1:GW] < 0.5        # top-left offsets where the part fits inside the sheet
            ys, xs = np.nonzero(valid)
            if len(ys) == 0:
                continue
            k = np.lexsort((xs, ys))[0]                       # top-most, then left-most
            cand = (int(ys[k]), int(xs[k]), ang, minx, miny, mask)
            if best is None or (cand[0], cand[1]) < (best[0], best[1]):
                best = cand
        if best is None:
            unplaced.append(s)
            continue
        py, px, ang, minx, miny, mask = best
        occ[py:py + mask.shape[0], px:px + mask.shape[1]] = np.maximum(occ[py:py + mask.shape[0], px:px + mask.shape[1]], mask)
        dx = sx + px * res - minx
        dy = sy + py * res - miny
        placements.append((s, ang, dx, dy))
    return placements, unplaced
