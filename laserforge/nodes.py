"""Edit Nodes tool: select/move nodes and Bezier handles, insert/delete/break nodes, corner/smooth, line/curve.

Keys (same letters LightBurn shows in its Edit Nodes help): S smooth node or curve segment, L curve to line,
C corner node, D delete node or segment, I insert node at cursor, M insert node at segment midpoint,
B break shape at node.
"""

import math

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QPainterPath, QPen

from . import model as M


class NodeEditor:
    def __init__(self, canvas):
        self.c = canvas
        self.sel = set()          # {(shape_id, node_index)}
        self.hover_node = None    # (sid, idx)
        self.hover_seg = None     # (sid, seg_index, t, (x, y))
        self.drag = None          # dict describing the current drag
        self.marquee = None

    @property
    def doc(self):
        return self.c.doc

    def reset(self):
        self.sel.clear()
        self.hover_node = self.hover_seg = self.drag = self.marquee = None

    def _polys(self):
        return [s for s in self.doc.selected_shapes() if s["type"] == "poly"]

    def _tol(self):
        return (self.c.win.prefs["click_tol"] + 1) / self.c.zoom

    # ------------------------------------------------------------ hit testing
    def _hit_handle(self, pt):
        tol = self._tol()
        for s in self._polys():
            for i, p in enumerate(s["points"]):
                if (s["id"], i) not in self.sel and not self._neighbour_selected(s, i):
                    continue
                for k in ("c1", "c2"):
                    if k in p and abs(p[k][0] - pt[0]) < tol and abs(p[k][1] - pt[1]) < tol:
                        return s, i, k
        return None

    def _neighbour_selected(self, s, i):
        n = len(s["points"])
        return any((s["id"], j % n) in self.sel for j in (i - 1, i + 1)) or len(s["points"]) <= 12

    def _hit_node(self, pt):
        tol = self._tol()
        for s in self._polys():
            for i, p in enumerate(s["points"]):
                if abs(p["x"] - pt[0]) < tol and abs(p["y"] - pt[1]) < tol:
                    return s, i
        return None

    def _hit_segment(self, pt):
        tol = self._tol()
        best = None
        for s in self._polys():
            for i in range(M.seg_count(s)):
                d, t, q = M.nearest_on_segment(s, i, pt)
                if d < tol and (best is None or d < best[0]):
                    best = (d, s, i, t, q)
        return best

    # ------------------------------------------------------------ mouse
    def press(self, pt, shift):
        h = self._hit_handle(pt)
        if h:
            s, i, k = h
            self.doc.save_undo()
            self.drag = {"kind": "handle", "sid": s["id"], "idx": i, "key": k}
            return
        n = self._hit_node(pt)
        if n:
            s, i = n
            key = (s["id"], i)
            if shift:
                self.sel ^= {key}
            elif key not in self.sel:
                self.sel = {key}
            self.doc.save_undo()
            self.drag = {"kind": "nodes", "start": pt, "orig": {k: M.clone(self.doc.find(k[0])["points"][k[1]]) for k in self.sel if self.doc.find(k[0])}}
            self.c.update()
            return
        seg = self._hit_segment(pt)
        if seg:
            _, s, i, t, q = seg
            if M.is_curve_seg(s, i):
                self.doc.save_undo()
                a, b = M.seg_ends(s, i)
                self.drag = {"kind": "bend", "sid": s["id"], "seg": i, "t": t, "start": pt,
                             "c2": list(a.get("c2", (a["x"], a["y"]))), "c1": list(b.get("c1", (b["x"], b["y"])))}
            elif not shift:
                self.sel.clear()
            self.c.update()
            return
        hit = self.c.hit_test(pt)
        if hit is not None and hit["type"] in ("rect", "circle"):
            self.doc.save_undo()
            M.replace_in_place(hit, M.to_poly(hit))
            self.doc.changed.emit()
        if hit is not None:
            if hit["id"] not in self.doc.selected:
                self.doc.set_selection([hit["id"]])
                self.sel.clear()
        else:
            self.sel.clear() if not shift else None
            self.marquee = (pt, pt)
        self.c.update()

    def move(self, pt):
        d = self.drag
        if d is None:
            if self.marquee:
                self.marquee = (self.marquee[0], pt)
                self.c.update()
                return
            nh = self._hit_node(pt)
            self.hover_node = (nh[0]["id"], nh[1]) if nh else None
            sg = None if nh else self._hit_segment(pt)
            self.hover_seg = (sg[1]["id"], sg[2], sg[3], sg[4]) if sg else None
            self.c.update()
            return
        sp = self.c.snap(pt)
        if d["kind"] == "handle":
            s = self.doc.find(d["sid"])
            p = s["points"][d["idx"]]
            p[d["key"]] = [sp[0], sp[1]]
            M.mirror_handle(p, d["key"])
            s.pop("regular", None)
        elif d["kind"] == "nodes":
            dx, dy = sp[0] - self.c.snap(d["start"])[0], sp[1] - self.c.snap(d["start"])[1]
            for (sid, idx), orig in d["orig"].items():
                s = self.doc.find(sid)
                p = M.clone(orig)
                M.map_point_dict(p, lambda x, y: (x + dx, y + dy))
                s["points"][idx] = p
                s.pop("regular", None)
        elif d["kind"] == "bend":
            s = self.doc.find(d["sid"])
            a, b = M.seg_ends(s, d["seg"])
            t = min(0.9, max(0.1, d["t"]))
            dx, dy = pt[0] - d["start"][0], pt[1] - d["start"][1]
            # move both handles so the grabbed point follows the cursor
            w1, w2 = 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t
            k = 1 / (w1 * w1 + w2 * w2)
            a["c2"] = [d["c2"][0] + dx * w1 * k, d["c2"][1] + dy * w1 * k]
            b["c1"] = [d["c1"][0] + dx * w2 * k, d["c1"][1] + dy * w2 * k]
            M.mirror_handle(a, "c2")
            M.mirror_handle(b, "c1")
            s.pop("regular", None)
        self.c.update()

    def release(self):
        if self.marquee:
            a, b = self.marquee
            x0, x1, y0, y1 = min(a[0], b[0]), max(a[0], b[0]), min(a[1], b[1]), max(a[1], b[1])
            for s in self._polys():
                for i, p in enumerate(s["points"]):
                    if x0 <= p["x"] <= x1 and y0 <= p["y"] <= y1:
                        self.sel.add((s["id"], i))
            self.marquee = None
        if self.drag:
            self.drag = None
            self.doc.changed.emit()
        self.c.update()

    # ------------------------------------------------------------ keys
    def key(self, text):
        k = text.upper()
        if k not in "SLCDIMB" or not k:
            return False
        doc = self.doc
        if k in ("I", "M") and self.hover_seg:
            sid, i, t, _ = self.hover_seg
            s = doc.find(sid)
            doc.save_undo()
            j = M.split_segment(s, i, 0.5 if k == "M" else t)
            self.sel = {(sid, j)}
            self.hover_seg = None
            doc.changed.emit()
            return True
        if k == "S":
            if self.sel:
                doc.save_undo()
                for sid, idx in self.sel:
                    s = doc.find(sid)
                    p = s["points"][idx]
                    M.set_smooth(s, idx, not p.get("smooth"))
                doc.changed.emit()
                return True
            if self.hover_seg:
                sid, i, _, _ = self.hover_seg
                doc.save_undo()
                M.make_curve(doc.find(sid), i)
                doc.changed.emit()
                return True
        if k == "L" and self.hover_seg:
            sid, i, _, _ = self.hover_seg
            doc.save_undo()
            M.make_line(doc.find(sid), i)
            doc.changed.emit()
            return True
        if k == "C" and self.sel:
            doc.save_undo()
            for sid, idx in self.sel:
                M.set_smooth(doc.find(sid), idx, False)
            doc.changed.emit()
            return True
        if k == "D":
            if self.sel:
                doc.save_undo()
                by_shape = {}
                for sid, idx in self.sel:
                    by_shape.setdefault(sid, []).append(idx)
                for sid, idxs in by_shape.items():
                    s = doc.find(sid)
                    if not M.delete_nodes(s, idxs):
                        doc.shapes = [x for x in doc.shapes if x["id"] != sid]
                self.sel.clear()
                doc.notify()
                return True
            if self.hover_seg:
                sid, i, _, _ = self.hover_seg
                self._replace_with_parts(doc.find(sid), M.delete_segment(doc.find(sid), i))
                return True
        if k == "B":
            target = self.hover_node or (next(iter(self.sel)) if len(self.sel) == 1 else None)
            if target:
                s = doc.find(target[0])
                self._replace_with_parts(s, M.break_at_node(s, target[1]))
                return True
        return False

    def _replace_with_parts(self, s, parts):
        doc = self.doc
        doc.save_undo()
        idx = doc.shapes.index(s)
        new = []
        for k, pts in enumerate(parts):
            ns = {kk: v for kk, v in s.items() if kk not in ("points", "regular")}
            ns["points"] = pts
            ns["closed"] = False
            if k:
                ns["id"] = doc.new_id()
            new.append(ns)
        doc.shapes[idx:idx + 1] = new
        doc.selected = [n["id"] for n in new]
        self.sel.clear()
        self.hover_seg = self.hover_node = None
        doc.notify()

    # ------------------------------------------------------------ drawing
    def draw(self, p):
        z = self.c.zoom
        hs = self.c.win.prefs["node_size"] / 2 / z
        if self.hover_seg:
            s = self.doc.find(self.hover_seg[0])
            if s:
                a, b = M.seg_ends(s, self.hover_seg[1])
                qp = QPainterPath()
                qp.moveTo(a["x"], a["y"])
                for x, y in M.segment_points(a, b):
                    qp.lineTo(x, y)
                pen = QPen(QColor("#ff9800"), 3)
                pen.setCosmetic(True)
                p.setPen(pen)
                p.setBrush(Qt.NoBrush)
                p.drawPath(qp)
                x, y = self.hover_seg[3]
                p.setPen(Qt.NoPen)
                p.setBrush(QColor("#ff9800"))
                p.drawEllipse(QPointF(x, y), hs, hs)
        hpen = QPen(QColor("#8e44ad"), 1)
        hpen.setCosmetic(True)
        for s in self._polys():
            for i, pt in enumerate(s["points"]):
                key = (s["id"], i)
                show_handles = key in self.sel or self._neighbour_selected(s, i)
                if show_handles:
                    for kk in ("c1", "c2"):
                        if kk in pt:
                            p.setPen(hpen)
                            p.drawLine(QPointF(pt["x"], pt["y"]), QPointF(*pt[kk]))
                            p.setPen(Qt.NoPen)
                            p.setBrush(QColor("#8e44ad"))
                            p.drawEllipse(QPointF(*pt[kk]), hs * 0.8, hs * 0.8)
                selected = key in self.sel
                hovered = self.hover_node == key
                col = "#ff1a1a" if selected else "#ff9800" if hovered else "#2f7bf5"
                p.setPen(Qt.NoPen)
                p.setBrush(QColor(col))
                if pt.get("smooth"):
                    p.drawEllipse(QPointF(pt["x"], pt["y"]), hs, hs)
                else:
                    p.drawRect(QRectF(pt["x"] - hs, pt["y"] - hs, hs * 2, hs * 2))
        if self.marquee:
            a, b = self.marquee
            pen = QPen(QColor("#2f7bf5"), 1)
            pen.setCosmetic(True)
            pen.setDashPattern([4, 3])
            p.setPen(pen)
            p.setBrush(QColor(47, 123, 245, 20))
            p.drawRect(QRectF(QPointF(*a), QPointF(*b)).normalized())
