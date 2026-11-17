"""Design canvas: renders the work area and shapes, handles selection, drawing tools and transforms."""

import math

import numpy as np
from PySide6.QtCore import QEvent, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QBrush, QColor, QFont, QImage, QPainter, QPainterPath, QPen
from PySide6.QtWidgets import QWidget

from . import model as M

RULER = 18
HANDLE_CURSORS = {
    "n": Qt.SizeVerCursor, "s": Qt.SizeVerCursor, "e": Qt.SizeHorCursor, "w": Qt.SizeHorCursor,
    "nw": Qt.SizeFDiagCursor, "se": Qt.SizeFDiagCursor, "ne": Qt.SizeBDiagCursor, "sw": Qt.SizeBDiagCursor,
}


def handle_positions(b):
    x, y, w, h = b
    return [("nw", x, y), ("n", x + w / 2, y), ("ne", x + w, y), ("e", x + w, y + h / 2),
            ("se", x + w, y + h), ("s", x + w / 2, y + h), ("sw", x, y + h), ("w", x, y + h / 2)]


class Canvas(QWidget):
    mouseMoved = Signal(float, float)
    status = Signal(str)
    zoomChanged = Signal(float)
    requestText = Signal(float, float)
    editShape = Signal(object)
    positionLaser = Signal(float, float)

    def __init__(self, win):
        super().__init__()
        self.win = win
        self.doc = win.doc
        self.zoom, self.pan_x, self.pan_y = 1.0, 40.0, 40.0
        self.tool = "select"
        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setAttribute(Qt.WA_OpaquePaintEvent)
        self.setAcceptDrops(True)
        self.setMinimumSize(300, 200)
        # interaction state
        self.mode = None
        self.start = None
        self.last = None
        self.points = []
        self.orig = None
        self.start_b = None
        self.handle = None
        self.marquee = None
        self.measure = None
        self.node = None
        self.space = False
        self.moved = False
        self.last_mouse = None
        self.pan_start = None
        self._qimg = {}
        self.snap_hit = None
        from .nodes import NodeEditor
        from . import tools as T
        self.handlers = {"nodes": NodeEditor(self), "scissors": T.ScissorsTool(self), "trim": T.TrimTool(self),
                         "tabs": T.TabsTool(self), "start": T.StartPointTool(self), "twopoint": T.TwoPointTool(self),
                         "deform": T.DeformTool(self), "warp": T.DeformTool(self, warp=True)}

    # ------------------------------------------------------------ coordinates
    def to_world(self, sx, sy):
        return ((sx - self.pan_x) / self.zoom, (sy - self.pan_y) / self.zoom)

    def to_screen(self, x, y):
        return (x * self.zoom + self.pan_x, y * self.zoom + self.pan_y)

    def snap(self, p, exclude=None):
        """Object snapping (nodes, centres, midpoints) has priority over grid snapping."""
        prefs = self.win.prefs
        self.snap_hit = None
        if prefs["obj_snap"]:
            q = self.object_snap(p, exclude)
            if q is not None:
                self.snap_hit = q
                return q
        if not self.win.snap_on:
            return p
        g = self.win.snap_grid
        return (round(p[0] / g) * g, round(p[1] / g) * g)

    def object_snap(self, p, exclude=None):
        tol = self.win.prefs["obj_snap_px"] / self.zoom
        excl = set(exclude if exclude is not None else (self.doc.selected if self.mode in ("move", "resize") else []))
        best, best_d = None, tol
        for s in self.doc.shapes:
            if s["id"] in excl:
                continue
            b = M.bounds(s)
            if not M.in_bounds(p, b, tol):
                continue
            cands = [M.center(b)]
            if s["type"] == "poly" and len(s["points"]) <= 400:
                pts = s["points"]
                cands += [(q["x"], q["y"]) for q in pts]
                n = len(pts)
                for i in range(n if s.get("closed") else n - 1):
                    a, c = pts[i], pts[(i + 1) % n]
                    if "c2" not in a and "c1" not in c:
                        cands.append(((a["x"] + c["x"]) / 2, (a["y"] + c["y"]) / 2))
            elif s["type"] in ("rect", "image"):
                x, y, w, h = b
                cands += [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x + w / 2, y), (x + w, y + h / 2), (x + w / 2, y + h), (x, y + h / 2)]
            elif s["type"] == "circle":
                cands += [(s["cx"] + s["rx"], s["cy"]), (s["cx"] - s["rx"], s["cy"]), (s["cx"], s["cy"] + s["ry"]), (s["cx"], s["cy"] - s["ry"])]
            for c in cands:
                d = math.hypot(c[0] - p[0], c[1] - p[1])
                if d < best_d:
                    best, best_d = c, d
        return best

    # ------------------------------------------------------------ zoom / pan
    MAX_ZOOM = 120.0          # screen px per mm (~0.008 mm per pixel)

    def real_px_per_mm(self):
        """Screen pixels for one real millimetre: 100 % zoom shows the design at its true size."""
        scr = self.screen() if hasattr(self, "screen") else None
        dpi = scr.physicalDotsPerInch() if scr else 96.0
        return max(1.0, (dpi or 96.0) / 25.4)

    def fit_zoom(self, pad=30):
        ws = self.win.workspace()
        if self.width() < 10 or ws[0] <= 0 or ws[1] <= 0:
            return 1.0
        return max(1e-3, min((self.width() - pad * 2) / ws[0], (self.height() - pad * 2) / ws[1]))

    def min_zoom(self):
        return self.fit_zoom() * 0.2

    def _clamp_view(self):
        """Never lose the work area: at least 60 px of it stays on screen whatever the pan or zoom."""
        ws = self.win.workspace()
        keep = 60.0
        W, H = self.width(), self.height()
        aw, ah = ws[0] * self.zoom, ws[1] * self.zoom
        self.pan_x = min(max(self.pan_x, keep - aw), W - keep)
        self.pan_y = min(max(self.pan_y, keep - ah), H - keep)

    def _apply_view(self, zoom, pan_x, pan_y, interactive=True):
        if interactive and self.isVisible():
            self._begin_interactive()
        self.zoom, self.pan_x, self.pan_y = zoom, pan_x, pan_y
        self._clamp_view()
        self.zoomChanged.emit(self.zoom)
        self.update()

    def zoom_at(self, mx, my, factor, animate=False):
        base = self._anim_target[0] if animate and self._anim_target else self.zoom
        target = max(self.min_zoom(), min(self.MAX_ZOOM, base * factor))
        if animate:
            bz, bpx, bpy = self._anim_target or (self.zoom, self.pan_x, self.pan_y)
            k = target / bz
            self._animate_to(target, mx - (mx - bpx) * k, my - (my - bpy) * k)
            return
        self._stop_anim()
        k = target / self.zoom
        self._apply_view(target, mx - (mx - self.pan_x) * k, my - (my - self.pan_y) * k)

    def zoom_to_box(self, b, pad=30, animate=True):
        if not b or b[2] <= 0 or b[3] <= 0 or self.width() < 10:
            return
        z = max(1e-3, min(self.MAX_ZOOM, min((self.width() - pad * 2) / b[2], (self.height() - pad * 2) / b[3])))
        px = (self.width() - b[2] * z) / 2 - b[0] * z
        py = (self.height() - b[3] * z) / 2 - b[1] * z
        if animate and self.isVisible():
            self._animate_to(z, px, py)
        else:
            self._stop_anim()
            self._apply_view(z, px, py)

    def zoom_page(self, animate=True):
        ws = self.win.workspace()
        self.zoom_to_box((0, 0, ws[0], ws[1]), animate=animate)

    def zoom_step(self, f):
        self.zoom_at(self.width() / 2, self.height() / 2, f, animate=True)

    def zoom_real(self, factor=1.0, at=None):
        """Zoom to a percentage of the true size (1.0 = 100 %), around the cursor or the view centre."""
        mx, my = at if at else (self.width() / 2, self.height() / 2)
        base = self._anim_target[0] if self._anim_target else self.zoom
        self.zoom_at(mx, my, self.real_px_per_mm() * factor / base, animate=True)

    def zoom_percent(self):
        return self.zoom / self.real_px_per_mm() * 100

    # smooth animation (ease-out) for wheel notches, menu zooms and fits
    def _animate_to(self, zoom, pan_x, pan_y, ms=170):
        import time as _t
        if not hasattr(self, "_anim_timer"):
            from PySide6.QtCore import QTimer as _QT
            self._anim_timer = _QT(self)
            self._anim_timer.setInterval(8)
            self._anim_timer.timeout.connect(self._anim_tick)
        self._anim_from = (self.zoom, self.pan_x, self.pan_y)
        self._anim_target = (zoom, pan_x, pan_y)
        self._anim_t0, self._anim_ms = _t.monotonic(), ms
        self._anim_timer.start()

    def _anim_tick(self):
        import time as _t
        if not self._anim_target:
            self._anim_timer.stop()
            return
        u = min(1.0, (_t.monotonic() - self._anim_t0) * 1000 / self._anim_ms)
        e = 1 - (1 - u) ** 3
        (z0, x0, y0), (z1, x1, y1) = self._anim_from, self._anim_target
        # zoom interpolated geometrically, pan so the zoom centre stays put
        z = z0 * (z1 / z0) ** e
        if abs(z1 - z0) > 1e-9 * z0:
            # screen point that is fixed by the full transform from->target
            k = z1 / z0
            if abs(1 - k) > 1e-9:
                cx, cy = (x1 - k * x0) / (1 - k), (y1 - k * y0) / (1 - k)
                kk = z / z0
                x, y = cx - (cx - x0) * kk, cy - (cy - y0) * kk
                if u >= 1.0:
                    x, y = x1, y1
            else:
                x, y = x0 + (x1 - x0) * e, y0 + (y1 - y0) * e
        else:
            x, y = x0 + (x1 - x0) * e, y0 + (y1 - y0) * e
        self._apply_view(z, x, y)
        if u >= 1.0:
            self._anim_target = None
            self._anim_timer.stop()

    def _stop_anim(self):
        if getattr(self, "_anim_target", None):
            self._anim_target = None
            self._anim_timer.stop()

    _anim_target = None

    def set_tool(self, name):
        self.tool = name
        self.points, self.start, self.last, self.measure, self.node = [], None, None, None, None
        for h in self.handlers.values():
            h.reset()
        self.setCursor(Qt.ArrowCursor if name == "select" else Qt.OpenHandCursor if name == "pan" else Qt.CrossCursor)
        self.update()

    # ------------------------------------------------------------ painting
    # During a pinch / scroll / animation the scene is not redrawn shape by shape: a snapshot (with a margin
    # around the view) is scaled and moved instead, which costs ~2 ms per frame whatever is on the bed.
    # When the gesture stops for a moment the view is redrawn sharp.
    _interactive = False
    _cache = None
    SETTLE_MS = 110
    CACHE_MARGIN = 0.35

    def _begin_interactive(self):
        from PySide6.QtCore import QTimer as _QT
        if not hasattr(self, "_settle"):
            self._settle = _QT(self)
            self._settle.setSingleShot(True)
            self._settle.timeout.connect(self._end_interactive)
        if not self._interactive or self._cache is None:
            self._cache = self._render_cache()
            self._interactive = True
        else:
            z0 = self._cache[0]
            k = self.zoom / z0
            if k > 2.2 or k < 0.45:                # snapshot too blurry / too small: take a fresh one
                self._cache = self._render_cache()
        self._settle.start(self.SETTLE_MS)

    def _end_interactive(self):
        if self._anim_target or getattr(self, "_pinching", False):
            self._settle.start(self.SETTLE_MS)   # still moving: wait
            return
        self._interactive = False
        self._cache = None
        self.update()

    def _render_cache(self):
        from PySide6.QtGui import QPixmap
        W, H = self.width(), self.height()
        mx, my = int(W * self.CACHE_MARGIN), int(H * self.CACHE_MARGIN)
        dpr = self.devicePixelRatioF()
        pm = QPixmap(max(1, int((W + 2 * mx) * dpr)), max(1, int((H + 2 * my) * dpr)))
        pm.setDevicePixelRatio(dpr)
        p = QPainter(pm)
        p.translate(mx, my)
        self._paint_scene(p, W, H, mx, my)
        p.end()
        return (self.zoom, self.pan_x, self.pan_y, pm, mx, my)

    def _paint_cached(self, p, W, H):
        z0, px0, py0, pm, mx, my = self._cache
        th = self.win.theme_colors()
        p.fillRect(0, 0, W, H, QColor(th["canvas_bg"]))
        k = self.zoom / z0
        p.save()
        p.setRenderHint(QPainter.SmoothPixmapTransform, True)
        p.translate(self.pan_x, self.pan_y)
        p.scale(k, k)
        p.translate(-px0 - mx, -py0 - my)
        p.drawPixmap(0, 0, pm)
        p.restore()

    def paintEvent(self, _ev):
        p = QPainter(self)
        W, H = self.width(), self.height()
        if self._interactive and self._cache is not None:
            self._paint_cached(p, W, H)
        else:
            self._paint_scene(p, W, H, 0, 0)
        if self.win.view["rulers"]:
            p.setRenderHint(QPainter.Antialiasing, False)
            self._draw_rulers(p, W, H)
        p.end()

    def _paint_scene(self, p, W, H, mx, my):
        p.setRenderHint(QPainter.Antialiasing, True)
        self._grid_margin = (mx, my)
        th = self.win.theme_colors()
        p.fillRect(QRectF(-mx, -my, W + 2 * mx, H + 2 * my), QColor(th["canvas_bg"]))
        p.save()
        p.translate(self.pan_x, self.pan_y)
        p.scale(self.zoom, self.zoom)
        ws = self.win.workspace()
        p.fillRect(QRectF(0, 0, ws[0], ws[1]), QColor("#ffffff"))
        if self.win.camera_overlay is not None and self.win.camera_overlay_visible:
            p.setOpacity(self.win.camera_cal.get("opacity", 70) / 100)
            p.drawImage(QRectF(0, 0, ws[0], ws[1]), self.win.camera_overlay)
            p.setOpacity(1.0)
        if self.win.view["grid"]:
            p.setRenderHint(QPainter.Antialiasing, False)     # straight 1-px lines: much faster without AA
            self._draw_grid(p, ws)
            p.setRenderHint(QPainter.Antialiasing, True)
        p.setPen(self._pen("#555"))
        p.setBrush(Qt.NoBrush)
        p.drawRect(QRectF(0, 0, ws[0], ws[1]))

        sel_ids = set(self.doc.selected)
        for s in self.doc.shapes:
            self._draw_shape(p, s, s.get("layer", 0), s["id"] in sel_ids)

        sel = self.doc.selected_shapes()
        if sel:
            b = M.multi_bounds(sel)
            pen = self._pen("#2f7bf5")
            pen.setDashPattern([4, 3])
            p.setPen(pen)
            p.setBrush(Qt.NoBrush)
            p.drawRect(QRectF(*b))
            if self.tool == "select":
                hs = 4 / self.zoom
                p.setPen(self._pen("#2f7bf5"))
                p.setBrush(QColor("#ffffff"))
                for _, hx, hy in handle_positions(b):
                    p.drawRect(QRectF(hx - hs, hy - hs, hs * 2, hs * 2))
            self._draw_center_dot(p, sel, b)

        handler = self.handlers.get(self.tool)
        if handler is not None:
            handler.draw(p)
        if self.snap_hit is not None and (self.mode in ("move", "draw", "resize", "handler") or self.tool not in ("select", "pan")):
            k = 5 / self.zoom
            p.setPen(self._pen("#e0218a", 1.5))
            p.setBrush(Qt.NoBrush)
            p.drawRect(QRectF(self.snap_hit[0] - k, self.snap_hit[1] - k, 2 * k, 2 * k))
        if self.tool != "tabs":
            from .tools import draw_tabs
            draw_tabs(self, p)
        self._draw_in_progress(p)

        if self.win.connected:
            hx, hy = self.win.to_canvas(self.win.machine_pos[:2])     # machine → screen orientation
            r = 6 / self.zoom
            p.setPen(self._pen("#e01b1b", 1.5))
            p.setBrush(Qt.NoBrush)
            p.drawEllipse(QPointF(hx, hy), r, r)
            p.drawLine(QPointF(hx - r * 1.8, hy), QPointF(hx + r * 1.8, hy))
            p.drawLine(QPointF(hx, hy - r * 1.8), QPointF(hx, hy + r * 1.8))
        p.restore()

        if self.marquee:
            a, b2 = self.marquee
            ax, ay = self.to_screen(*a)
            bx, by = self.to_screen(*b2)
            right = b2[0] >= a[0]
            pen = QPen(QColor("#2f7bf5" if right else "#1a9d4a"), 1)
            if not right:
                pen.setDashPattern([5, 3])
            p.setPen(pen)
            p.setBrush(QColor(47, 123, 245, 20) if right else QColor(26, 157, 74, 20))
            p.drawRect(QRectF(QPointF(ax, ay), QPointF(bx, by)).normalized())

    def _pen(self, color, width=1.0):
        pen = QPen(QColor(color), width)
        pen.setCosmetic(True)
        return pen

    def _draw_grid(self, p, ws):
        """Grid levels fade in and out with the on-screen spacing, so zooming never makes the grid jump."""
        g = float(self.win.prefs.get("grid_spacing") or 0)
        base_rgb = (0, 0, 0)                      # the work area is always drawn white
        # only the visible part of the bed: deep zooms stay fast
        gmx, gmy = getattr(self, "_grid_margin", (0, 0))
        vx0, vy0 = self.to_world(-gmx, -gmy)
        vx1, vy1 = self.to_world(self.width() + gmx, self.height() + gmy)
        x_lo, x_hi = max(0.0, vx0), min(ws[0], vx1)
        y_lo, y_hi = max(0.0, vy0), min(ws[1], vy1)
        if x_lo >= x_hi or y_lo >= y_hi:
            return
        if g > 0:
            levels = [g, g * 10]
        else:
            levels = [0.1, 1.0, 10.0, 100.0, 1000.0]
        drawn = 0
        for step in levels:
            px = step * self.zoom
            if px < 6 or (step - 1e-9) > max(ws):
                continue
            # 6 px → invisible, 40 px and above → full strength; the coarsest visible level stays stronger
            a = max(0.0, min(1.0, (px - 6) / 34))
            alpha = int((0.10 + 0.14 * min(1.0, px / 200)) * 255 * a)
            if alpha < 3:
                continue
            col = QColor(*base_rgb, alpha)
            drawn += 1
            path = QPainterPath()
            x = max(step, math.floor(x_lo / step) * step)
            while x < x_hi:
                if x > 0:
                    path.moveTo(x, y_lo)
                    path.lineTo(x, y_hi)
                x += step
            y = max(step, math.floor(y_lo / step) * step)
            while y < y_hi:
                if y > 0:
                    path.moveTo(x_lo, y)
                    path.lineTo(x_hi, y)
                y += step
            pen = QPen(col, 1.0)
            pen.setCosmetic(True)
            p.setPen(pen)
            p.drawPath(path)

    def _layer_color(self, idx):
        layers = self.win.engine.layers
        return layers[idx].color if 0 <= idx < len(layers) else "#000000"

    def _paths_qpath(self, paths):
        qp = QPainterPath()
        qp.setFillRule(Qt.OddEvenFill)
        for path in paths:
            if len(path) < 2:
                continue
            qp.moveTo(*path[0])
            for pt in path[1:]:
                qp.lineTo(*pt)
        return qp

    def _cached_paths(self, s):
        """QPainterPaths of a shape, rebuilt only when its geometry changes: zooming and panning just
        redraw the cached paths, which keeps the trackpad smooth with many shapes on the bed."""
        import json as _json
        try:
            sig = _json.dumps({k: v for k, v in s.items() if k not in ("id", "name", "locked", "layer", "src", "priority")},
                              sort_keys=True, default=str)
        except Exception:
            sig = None
        cache = self.__dict__.setdefault("_path_cache", {})
        hit = cache.get(id(s))
        if sig is not None and hit is not None and hit[0] == sig:
            return hit[1], hit[2]
        paths = M.shape_paths(s)
        closed = [pp for pp in paths if M.is_closed_path(pp)]
        res = (self._paths_qpath(paths), self._paths_qpath(closed) if closed else None)
        if len(cache) > 5000:
            cache.clear()
        if sig is not None:
            cache[id(s)] = (sig, *res)
        return res

    def _image_qimage(self, s):
        interval = max(self.win.layer_interval(s.get("layer", 0)), s["w"] / 900)
        res = M.process_image(s, interval, 900)
        if res is None:
            return None
        burn, cols, rows = res
        key = id(burn)
        hit = self._qimg.get(key)
        if hit is not None and hit[0] is burn:
            return hit[1]
        gray = np.ascontiguousarray(255 - burn)
        img = QImage(gray.data, cols, rows, cols, QImage.Format_Grayscale8).copy()
        if len(self._qimg) > 40:
            self._qimg.clear()
        self._qimg[key] = (burn, img)
        return img

    def _draw_shape(self, p, s, layer_idx, selected):
        layer = s.get("layer")
        color = QColor(self._layer_color(layer if layer is not None else layer_idx))
        pen = self._pen(color, 1.6 if selected else 1.0)
        li = layer if layer is not None else layer_idx
        layers_ = self.win.engine.layers
        is_tool = 0 <= li < len(layers_) and layers_[li].tool
        if selected:
            pen.setDashPattern([5, 3])
        elif is_tool:
            pen.setDashPattern([6, 4])            # T1/T2 guides: dashed, never burned
        t = s["type"]
        ws = self.win.workspace()

        def outside(x, y, w, h):
            return x > ws[0] + 1e-6 or y > ws[1] + 1e-6 or x + w < -1e-6 or y + h < -1e-6
        prev_op = p.opacity()
        if t == "group":
            for c in s["children"]:
                self._draw_shape(p, c, c.get("layer") if c.get("layer") is not None else layer_idx, selected)
        elif t == "image":
            if outside(s["x"], s["y"], s["w"], s["h"]):
                p.setOpacity(prev_op * 0.35)      # off the bed: the laser ignores it
            img = self._image_qimage(s)
            if img is not None:
                p.setRenderHint(QPainter.SmoothPixmapTransform, True)
                p.drawImage(QRectF(s["x"], s["y"], s["w"], s["h"]), img)
            else:
                p.fillRect(QRectF(s["x"], s["y"], s["w"], s["h"]), QColor("#dddddd"))
            p.setPen(pen)
            p.setBrush(Qt.NoBrush)
            p.drawRect(QRectF(s["x"], s["y"], s["w"], s["h"]))
            if s.get("crop"):
                cp = self._pen("#8e44ad", 1.2)
                cp.setDashPattern([3, 2])
                p.setPen(cp)
                p.drawPath(self._paths_qpath([[(s["x"] + u * s["w"], s["y"] + v * s["h"]) for u, v in r] + [(s["x"] + r[0][0] * s["w"], s["y"] + r[0][1] * s["h"])]
                                              for r in s["crop"]]))
        else:
            qp, closed_qp = self._cached_paths(s)
            br = qp.boundingRect()
            if outside(br.x(), br.y(), br.width(), br.height()):
                p.setOpacity(prev_op * 0.35)      # off the bed: the laser ignores it
            if (t == "text" and not M.is_single_line(s)) or (self.win.view["filled"] and not (t == "text" and M.is_single_line(s))):
                if closed_qp is not None:
                    fill = QColor(color)
                    fill.setAlphaF(1.0 if t == "text" and not self.win.view["filled"] else 0.35)
                    if t == "text" and not self.win.view["filled"]:
                        fill.setAlphaF(0.85)
                    p.setPen(Qt.NoPen)
                    p.setBrush(fill)
                    p.drawPath(closed_qp)
            p.setPen(pen)
            p.setBrush(Qt.NoBrush)
            p.drawPath(qp)
        p.setOpacity(prev_op)
        if s.get("locked"):
            b = M.bounds(s)
            k = 4 / self.zoom
            p.fillRect(QRectF(b[0] + b[2] - k, b[1] - k, 2 * k, 2 * k), QColor(243, 156, 18, 230))

    def center_container(self, sel, sb):
        best, best_area = None, math.inf
        eps = 0.01
        for s in self.doc.shapes:
            if s in sel:
                continue
            b = M.bounds(s)
            if b[0] <= sb[0] + eps and b[1] <= sb[1] + eps and b[0] + b[2] >= sb[0] + sb[2] - eps and b[1] + b[3] >= sb[1] + sb[3] - eps:
                area = b[2] * b[3]
                if sb[2] * sb[3] + eps < area < best_area:
                    best, best_area = b, area
        return best

    def _draw_center_dot(self, p, sel, b):
        box = self.center_container(sel, b)
        if not box:
            return
        cx, cy = M.center(box)
        sx, sy = M.center(b)
        if abs(cx - sx) > 0.05 or abs(cy - sy) > 0.05:
            return
        pen = self._pen(QColor(230, 20, 20, 140))
        pen.setDashPattern([3, 3])
        p.setPen(pen)
        p.drawLine(QPointF(box[0], cy), QPointF(box[0] + box[2], cy))
        p.drawLine(QPointF(cx, box[1]), QPointF(cx, box[1] + box[3]))
        r = 5 / self.zoom
        p.setPen(self._pen("#ffffff", 1.5))
        p.setBrush(QColor("#ff1a1a"))
        p.drawEllipse(QPointF(cx, cy), r, r)

    def _draw_in_progress(self, p):
        pen = self._pen(self._layer_color(self.doc.active_layer))
        pen.setDashPattern([4, 3])
        p.setPen(pen)
        p.setBrush(Qt.NoBrush)
        a, b = self.start, self.last or self.start
        if self.mode == "draw" and a:
            if self.tool == "rect":
                p.drawRect(QRectF(QPointF(*a), QPointF(*b)).normalized())
            elif self.tool == "ellipse":
                p.drawEllipse(QRectF(QPointF(*a), QPointF(*b)).normalized())
            elif self.tool in ("polygon", "star"):
                r = math.hypot(b[0] - a[0], b[1] - a[1])
                pts = M.star_points(a[0], a[1], r, r * 0.45, 5) if self.tool == "star" else M.regular_polygon(a[0], a[1], r, self.win.polygon_sides)
                qp = QPainterPath()
                qp.moveTo(pts[0]["x"], pts[0]["y"])
                for q in pts[1:]:
                    qp.lineTo(q["x"], q["y"])
                qp.closeSubpath()
                p.drawPath(qp)
        if self.tool == "poly" and self.points:
            qp = QPainterPath()
            qp.moveTo(*self.points[0])
            for q in self.points[1:]:
                qp.lineTo(*q)
            if self.last:
                qp.lineTo(*self.last)
            p.drawPath(qp)
        if self.measure:
            m1, m2 = self.measure
            p.setPen(self._pen("#e01b1b"))
            p.drawLine(QPointF(*m1), QPointF(*m2))
            d = math.hypot(m2[0] - m1[0], m2[1] - m1[1])
            ang = math.degrees(math.atan2(-(m2[1] - m1[1]), m2[0] - m1[0]))
            p.save()
            p.resetTransform()
            sx, sy = self.to_screen(*m2)
            p.setFont(QFont("Helvetica", 12))
            p.setPen(QColor("#e01b1b"))
            p.drawText(QPointF(sx + 8, sy - 8), f"{self.win.fmt_len(d)}  {ang:.1f}°")
            p.restore()

    def _draw_rulers(self, p, W, H):
        th = self.win.theme_colors()
        u = self.win.unit_factor()
        inch = self.win.prefs["units"] == "in"
        steps = [1 / 16, 1 / 8, 1 / 4, 1 / 2, 1, 2, 5, 10, 20, 50, 100] if inch else [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000]
        major = next((st for st in steps if st / u * self.zoom >= 50), steps[-1])
        minor = major / (4 if inch and major < 1 else 5 if (major * 1000) % 5000 == 0 or major == 1 else 4)
        p.setRenderHint(QPainter.Antialiasing, False)
        p.fillRect(0, 0, W, RULER, QColor(th["ruler_bg"]))
        p.fillRect(0, 0, RULER, H, QColor(th["ruler_bg"]))
        f = QFont("Menlo")
        f.setPixelSize(9)
        p.setFont(f)

        def is_major(v):
            return abs(v / major - round(v / major)) < 1e-6

        def label(v):
            return (f"{v:.3f}".rstrip("0").rstrip(".") if inch else str(round(v)))

        v = math.floor(self.to_world(RULER, 0)[0] * u / minor) * minor
        vend = self.to_world(W, 0)[0] * u
        while v <= vend:
            sx = round(v / u * self.zoom + self.pan_x)
            if sx >= RULER:
                big = is_major(v)
                p.setPen(QColor(th["ruler_line"]))
                p.drawLine(sx, RULER, sx, 3 if big else RULER - 5)
                if big:
                    p.setPen(QColor(th["ruler_text"]))
                    p.drawText(sx + 3, 10, label(v))
            v += minor
        v = math.floor(self.to_world(0, RULER)[1] * u / minor) * minor
        vend = self.to_world(0, H)[1] * u
        while v <= vend:
            sy = round(v / u * self.zoom + self.pan_y)
            if sy >= RULER:
                big = is_major(v)
                p.setPen(QColor(th["ruler_line"]))
                p.drawLine(RULER, sy, 3 if big else RULER - 5, sy)
                if big:
                    p.save()
                    p.translate(10, sy - 3)
                    p.rotate(-90)
                    p.setPen(QColor(th["ruler_text"]))
                    p.drawText(0, 0, label(v))
                    p.restore()
            v += minor
        if self.last_mouse:
            mx, my = self.to_screen(*self.last_mouse)
            p.setPen(QColor("#e94560"))
            p.drawLine(int(mx), 0, int(mx), RULER)
            p.drawLine(0, int(my), RULER, int(my))
        p.fillRect(0, 0, RULER, RULER, QColor(th["ruler_corner"]))

    # ------------------------------------------------------------ hit testing
    def hit_test(self, pt):
        tol = self.win.prefs["click_tol"] / self.zoom
        for s in reversed(self.doc.shapes):
            if s.get("locked"):
                continue
            if s["type"] in ("image", "text"):
                if M.in_bounds(pt, M.bounds(s), tol):
                    return s
                continue
            for path in M.shape_paths(s):
                for k in range(1, len(path)):
                    if M.dist_to_segment(pt, path[k - 1], path[k]) < tol:
                        return s
        best, best_area = None, math.inf
        for s in self.doc.shapes:
            if s.get("locked"):
                continue
            b = M.bounds(s)
            if M.in_bounds(pt, b) and b[2] * b[3] < best_area:
                best, best_area = s, b[2] * b[3]
        return best

    def hit_handle(self, pt):
        sel = self.doc.selected_shapes()
        if not sel or self.tool != "select":
            return None
        tol = (self.win.prefs["click_tol"] + 1) / self.zoom
        for hid, hx, hy in handle_positions(M.multi_bounds(sel)):
            if abs(hx - pt[0]) <= tol and abs(hy - pt[1]) <= tol:
                return hid
        return None

    # ------------------------------------------------------------ mouse
    def _world(self, ev):
        pos = ev.position()
        return self.to_world(pos.x(), pos.y())

    def restore_orig(self):
        if not self.orig:
            return
        for o in self.orig:
            for i, s in enumerate(self.doc.shapes):
                if s["id"] == o["id"]:
                    self.doc.shapes[i] = M.clone(o)
                    break

    def apply_from_orig(self, fn):
        self.restore_orig()
        for s in self.doc.selected_shapes():
            fn(s)

    def mousePressEvent(self, ev):
        self.setFocus()
        pt = self._world(ev)
        sp = self.snap(pt)
        if ev.button() == Qt.MiddleButton or self.space or (self.tool == "pan" and ev.button() == Qt.LeftButton):
            self.mode = "pan"
            self.pan_start = (ev.position().x(), ev.position().y(), self.pan_x, self.pan_y)
            self.setCursor(Qt.ClosedHandCursor)
            return
        if ev.button() != Qt.LeftButton:
            return
        shift = bool(ev.modifiers() & Qt.ShiftModifier)
        doc = self.doc
        if self.tool == "select":
            h = self.hit_handle(pt)
            if h:
                self.mode, self.handle = "resize", h
                self.start_b = M.multi_bounds(doc.selected_shapes())
                self.orig = M.clone(doc.selected_shapes())
                return
            hit = self.hit_test(pt)
            if hit:
                if shift:
                    ids = [i for i in doc.selected if i != hit["id"]] if hit["id"] in doc.selected else doc.selected + [hit["id"]]
                    doc.set_selection(ids)
                elif hit["id"] not in doc.selected:
                    doc.set_selection([hit["id"]])
                self.mode, self.start, self.moved = "move", pt, False
                self.orig = M.clone(doc.selected_shapes())
                self.start_b = M.multi_bounds(doc.selected_shapes())
            else:
                if not shift:
                    doc.set_selection([])
                self.mode = "marquee"
                self.marquee = (pt, pt)
            self.update()
            return
        handler = self.handlers.get(self.tool)
        if handler is not None:
            self.mode = "handler"
            handler.press(pt, shift)
            return
        if self.tool == "poly":
            if not self.points:
                self.points = [sp]
            else:
                f = self.points[0]
                if len(self.points) > 2 and math.hypot(sp[0] - f[0], sp[1] - f[1]) < 6 / self.zoom:
                    self.finish_poly(True)
                    return
                self.points.append(sp)
            self.update()
            return
        if self.tool == "text":
            self.requestText.emit(*sp)
            return
        if self.tool == "measure":
            self.mode, self.measure = "measure", (sp, sp)
            return
        if self.tool == "position":
            self.positionLaser.emit(*sp)
            return
        self.mode, self.start, self.last = "draw", sp, sp

    def mouseMoveEvent(self, ev):
        pt = self._world(ev)
        sp = self.snap(pt)
        self.last_mouse = sp
        self.mouseMoved.emit(*pt)
        alt = bool(ev.modifiers() & Qt.AltModifier)
        shift = bool(ev.modifiers() & Qt.ShiftModifier)
        doc = self.doc
        if self.mode == "pan":
            x0, y0, px, py = self.pan_start
            self._stop_anim()
            self._begin_interactive()
            self.pan_x = px + ev.position().x() - x0
            self.pan_y = py + ev.position().y() - y0
            self._clamp_view()
            self.update()
            return
        if self.mode == "move":
            dx, dy = pt[0] - self.start[0], pt[1] - self.start[1]
            if not self.moved and math.hypot(dx, dy) * self.zoom < 3:
                return
            self.moved = True
            if self.win.snap_on and not alt:
                nx, ny = self.start_b[0] + dx, self.start_b[1] + dy
                snx, sny = self.snap((nx, ny), exclude=self.doc.selected)
                dx += snx - nx
                dy += sny - ny
            self.apply_from_orig(lambda s: M.move_shape(s, dx, dy))
            sel = doc.selected_shapes()
            b = M.multi_bounds(sel)
            box = self.center_container(sel, b)
            if box and not alt:
                cx, cy = M.center(box)
                sx, sy = M.center(b)
                tol = 8 / self.zoom
                if abs(cx - sx) < tol and abs(cy - sy) < tol:
                    for s in sel:
                        M.move_shape(s, cx - sx, cy - sy)
                    self.status.emit(self.win.tr("centered"))
            self.win.update_transform_fields()
            self.update()
            return
        if self.mode == "resize":
            self.resize_from_handle(pt, shift)
            return
        if self.mode == "marquee":
            self.marquee = (self.marquee[0], pt)
            self.update()
            return
        handler = self.handlers.get(self.tool)
        if handler is not None and self.mode in ("handler", None):
            handler.move(pt)
            return
        if self.mode == "measure":
            self.measure = (self.measure[0], sp)
            self.update()
            return
        if self.mode == "draw":
            self.last = self.constrain_square(self.start, sp) if shift else sp
            self.update()
            return
        if self.tool == "poly" and self.points:
            self.last = sp
        if self.tool == "select" and not self.space:
            h = self.hit_handle(pt)
            self.setCursor(HANDLE_CURSORS[h] if h else Qt.ArrowCursor)
        self.update()

    def mouseReleaseEvent(self, ev):
        mode, self.mode = self.mode, None
        doc = self.doc
        if not mode:
            return
        if mode == "pan":
            self.setCursor(Qt.OpenHandCursor if self.tool == "pan" or self.space else Qt.ArrowCursor if self.tool == "select" else Qt.CrossCursor)
            return
        if mode in ("move", "resize"):
            if mode == "resize" or self.moved:
                after = M.clone(doc.shapes)
                self.restore_orig()
                doc.push_undo_snapshot(M.clone(doc.shapes), self.win.L("Ridimensiona", "Resize") if mode == "resize" else self.win.L("Sposta", "Move"))
                doc.shapes = after
                doc.changed.emit()
            self.orig = None
            self.start = None
            self.win.update_transform_fields()
            self.update()
            return
        if mode == "marquee":
            a, b = self.marquee
            self.marquee = None
            box = (min(a[0], b[0]), min(a[1], b[1]), abs(b[0] - a[0]), abs(b[1] - a[1]))
            ids = list(doc.selected)
            if box[2] * self.zoom > 2 or box[3] * self.zoom > 2:
                contain = b[0] >= a[0]
                for s in doc.shapes:
                    if s.get("locked") or s["id"] in ids:
                        continue
                    sb = M.bounds(s)
                    if contain:
                        ok = sb[0] >= box[0] and sb[1] >= box[1] and sb[0] + sb[2] <= box[0] + box[2] and sb[1] + sb[3] <= box[1] + box[3]
                    else:
                        ok = sb[0] <= box[0] + box[2] and sb[0] + sb[2] >= box[0] and sb[1] <= box[1] + box[3] and sb[1] + sb[3] >= box[1]
                    if ok:
                        ids.append(s["id"])
            doc.set_selection(ids)
            self.update()
            return
        if mode == "handler":
            handler = self.handlers.get(self.tool)
            if handler is not None:
                handler.release()
            return
        if mode == "measure":
            a, b = self.measure
            self.status.emit(f"{math.hypot(b[0] - a[0], b[1] - a[1]):.2f} mm")
            return
        if mode == "draw":
            self.finish_draw()

    def mouseDoubleClickEvent(self, ev):
        if self.tool == "poly" and len(self.points) > 1:
            self.finish_poly(False)
        elif self.tool == "select":
            sel = self.doc.selected_shapes()
            if sel:
                self.editShape.emit(sel[0])

    def _is_trackpad(self, ev):
        try:
            from PySide6.QtGui import QInputDevice
            dev = ev.device()
            if dev is not None and dev.type() == QInputDevice.DeviceType.TouchPad:
                return True
        except Exception:
            pass
        # fallback: trackpads send pixel deltas with a scroll phase; classic wheels send 120-step angles only
        return not ev.pixelDelta().isNull() and ev.phase() != Qt.NoScrollPhase

    def wheelEvent(self, ev):
        """Trackpad: two fingers pan, pinch zooms (gesture), ⌘/Ctrl + two fingers zooms.
        Mouse wheel: smooth zoom at the cursor; Shift + wheel pans sideways, ⌥ + wheel pans vertically."""
        pos = ev.position()
        pd, ad = ev.pixelDelta(), ev.angleDelta()
        mods = ev.modifiers()
        zoom_mod = bool(mods & (Qt.ControlModifier | Qt.MetaModifier))
        if self._is_trackpad(ev):
            scroll_zooms = self.win.prefs.get("trackpad_scroll", "zoom") != "pan"
            if ev.phase() == Qt.ScrollMomentum and scroll_zooms and not (mods & Qt.ShiftModifier):
                ev.accept()                       # no zoom drift after the fingers lift
                return
            if zoom_mod or (scroll_zooms and not (mods & Qt.ShiftModifier)):
                # LightBurn: two fingers up/down zoom at the cursor (up = zoom in)
                dy = pd.y() if not pd.isNull() else ad.y() / 8
                self.zoom_at(pos.x(), pos.y(), math.exp(max(-60.0, min(60.0, dy)) * 0.006))
            else:
                if ev.phase() == Qt.ScrollMomentum and getattr(self, "_pinching", False):
                    return
                self._stop_anim()
                d = pd if not pd.isNull() else ad / 4
                self._apply_view(self.zoom, self.pan_x + d.x(), self.pan_y + d.y())
            ev.accept()
            return
        # classic mouse wheel
        if mods & Qt.ShiftModifier:
            step = (ad.y() or ad.x()) / 120 * 60
            self._stop_anim()
            self._apply_view(self.zoom, self.pan_x + step, self.pan_y)
        elif mods & Qt.AltModifier:
            step = (ad.y() or ad.x()) / 120 * 60
            self._stop_anim()
            self._apply_view(self.zoom, self.pan_x, self.pan_y + step)
        else:
            notches = ad.y() / 120.0 if ad.y() else pd.y() / 50.0
            if notches:
                self.zoom_at(pos.x(), pos.y(), 1.2 ** notches, animate=True)
        ev.accept()

    def event(self, ev):
        if ev.type() == QEvent.NativeGesture:
            g = ev.gestureType()
            pos = ev.position()
            if g == Qt.BeginNativeGesture:
                self._pinching = True
                self._stop_anim()
            elif g == Qt.EndNativeGesture:
                self._pinching = False
            elif g == Qt.ZoomNativeGesture:
                v = max(-0.5, min(0.5, ev.value()))          # one pinch step; clamps odd spikes
                self.zoom_at(pos.x(), pos.y(), math.exp(v))
            elif g == Qt.SmartZoomNativeGesture:
                # two-finger double tap: fit the work area, or zoom in 3x around the fingers if already fitted
                if self.zoom <= self.fit_zoom() * 1.05:
                    self.zoom_at(pos.x(), pos.y(), 3.0, animate=True)
                else:
                    self.zoom_page()
            return True
        return super().event(ev)

    def resizeEvent(self, ev):
        super().resizeEvent(ev)
        self._clamp_view()

    def leaveEvent(self, _ev):
        self.last_mouse = None
        self.update()

    def keyPressEvent(self, ev):
        k = ev.key()
        if k == Qt.Key_Space and not ev.isAutoRepeat():
            self.space = True
            self.setCursor(Qt.OpenHandCursor)
            return
        if self.tool == "poly" and self.points:
            if k in (Qt.Key_Return, Qt.Key_Enter):
                self.finish_poly(False)
                return
        if k in (Qt.Key_Left, Qt.Key_Right, Qt.Key_Up, Qt.Key_Down) and self.doc.selected and not (ev.modifiers() & Qt.ControlModifier):
            pr = self.win.prefs
            d = pr["nudge_shift"] if ev.modifiers() & Qt.ShiftModifier else pr["nudge_fine"] if ev.modifiers() & Qt.AltModifier else pr["nudge"]
            dx = -d if k == Qt.Key_Left else d if k == Qt.Key_Right else 0
            dy = -d if k == Qt.Key_Up else d if k == Qt.Key_Down else 0
            self.doc.save_undo()
            for s in self.doc.selected_shapes():
                M.move_shape(s, dx, dy)
            self.doc.notify()
            return
        handler = self.handlers.get(self.tool)
        if handler is not None and not (ev.modifiers() & (Qt.ControlModifier | Qt.MetaModifier)) and ev.text() and handler.key(ev.text()):
            self.update()
            return
        if k == Qt.Key_G and not ev.modifiers():
            self.win.toggle_snap()
            return
        super().keyPressEvent(ev)

    def keyReleaseEvent(self, ev):
        if ev.key() == Qt.Key_Space and not ev.isAutoRepeat():
            self.space = False
            self.set_tool(self.tool) if not self.points else None
            return
        super().keyReleaseEvent(ev)

    def cancel_in_progress(self):
        if self.points or self.measure or self.mode:
            self.points, self.last, self.measure, self.mode, self.start = [], None, None, None, None
            self.update()
            return True
        return False

    # ------------------------------------------------------------ transforms
    @staticmethod
    def constrain_square(a, b):
        d = max(abs(b[0] - a[0]), abs(b[1] - a[1]))
        sx = math.copysign(1, (b[0] - a[0]) or 1)
        sy = math.copysign(1, (b[1] - a[1]) or 1)
        return (a[0] + sx * d, a[1] + sy * d)

    def resize_from_handle(self, pt, free):
        x, y, w, h = self.start_b
        hd = self.handle
        ax = x + w if "w" in hd else x if "e" in hd else x + w / 2
        ay = y + h if "n" in hd else y if "s" in hd else y + h / 2
        sx = (pt[0] - ax) / (((x if "w" in hd else x + w) - ax) or 1e-9) if ("e" in hd or "w" in hd) else 1
        sy = (pt[1] - ay) / (((y if "n" in hd else y + h) - ay) or 1e-9) if ("n" in hd or "s" in hd) else 1
        if len(hd) == 2 and self.win.lock_aspect != free:
            k = max(abs(sx), abs(sy))
            sx = math.copysign(k, sx or 1)
            sy = math.copysign(k, sy or 1)
        sx, sy = max(0.001, abs(sx)), max(0.001, abs(sy))
        self.apply_from_orig(lambda s: M.scale_shape(s, ax, ay, sx, sy))
        self.win.update_transform_fields()
        self.update()

    def finish_draw(self):
        a, b = self.start, self.last or self.start
        self.start = self.last = None
        doc = self.doc
        shape = None
        if self.tool == "rect":
            w, h = abs(b[0] - a[0]), abs(b[1] - a[1])
            if w >= 0.5 and h >= 0.5:
                shape = doc.create("rect", x=min(a[0], b[0]), y=min(a[1], b[1]), w=w, h=h)
        elif self.tool == "ellipse":
            rx, ry = abs(b[0] - a[0]) / 2, abs(b[1] - a[1]) / 2
            if rx >= 0.25 and ry >= 0.25:
                shape = doc.create("circle", cx=(a[0] + b[0]) / 2, cy=(a[1] + b[1]) / 2, rx=rx, ry=ry)
        elif self.tool in ("polygon", "star"):
            r = math.hypot(b[0] - a[0], b[1] - a[1])
            if r >= 0.5:
                star = self.tool == "star"
                pts = M.star_points(a[0], a[1], r, r * 0.45, 5) if star else M.regular_polygon(a[0], a[1], r, self.win.polygon_sides)
                regular = {"cx": a[0], "cy": a[1], "r": r, "n": 5 if star else self.win.polygon_sides, "rot": -math.pi / 2}
                if star:
                    regular["inner"] = 0.45
                shape = doc.create("poly", points=pts, closed=True, regular=regular)
        if shape:
            doc.current_label = self.win.L("Disegna", "Draw")
            doc.add_shapes([shape])
            doc.current_label = None
        self.update()

    def finish_poly(self, closed):
        pts = self.points[:]
        self.points, self.last = [], None
        if len(pts) >= 2:
            self.doc.add_shapes([self.doc.create("poly", points=[{"x": x, "y": y} for x, y in pts], closed=closed)])
        self.update()

    # ------------------------------------------------------------ drag & drop
    def dragEnterEvent(self, ev):
        if ev.mimeData().hasUrls():
            ev.acceptProposedAction()

    def dropEvent(self, ev):
        for url in ev.mimeData().urls():
            if url.isLocalFile():
                self.win.open_dropped(url.toLocalFile())
                break
