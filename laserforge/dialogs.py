"""Dialogs for LaserForge tools (native Qt)."""

import math
import re

import numpy as np
from PySide6.QtCore import QPointF, QRectF, Qt, QTimer
from PySide6.QtWidgets import QApplication
from PySide6.QtGui import QColor, QFontDatabase, QImage, QPainter, QPen, QPixmap
from PySide6.QtWidgets import (
    QComboBox, QDialog, QDialogButtonBox, QFormLayout, QGridLayout, QHBoxLayout, QLabel, QLineEdit, QPushButton,
    QSlider, QTableWidget, QTableWidgetItem, QHeaderView, QVBoxLayout, QWidget, QFontComboBox,
)

from . import model as M
from .defs import MENUS
from .ui_common import Segmented, checkbox, dspin, hint, ispin


class Dialog(QDialog):
    def __init__(self, win, title, ok=None, cancel=True, wide=False):
        super().__init__(win)
        self.win = win
        self.setWindowTitle(title)
        self.setModal(True)
        self.lay = QVBoxLayout(self)
        self.lay.setSpacing(10)
        self.form = QFormLayout()
        self.form.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
        self.form.setFieldGrowthPolicy(QFormLayout.ExpandingFieldsGrow)
        self.lay.addLayout(self.form)
        self.buttons = QDialogButtonBox()
        self.ok_btn = None
        if ok:
            self.ok_btn = self.buttons.addButton(ok, QDialogButtonBox.AcceptRole)
            self.ok_btn.setProperty("role", "primary")
            self.ok_btn.setDefault(True)
        if cancel:
            self.buttons.addButton(win.tr("cancel"), QDialogButtonBox.RejectRole)
        self.buttons.accepted.connect(self._accept)
        self.buttons.rejected.connect(self.reject)
        self.on_ok = None
        if wide:
            self.setMinimumWidth(640)
        else:
            self.setMinimumWidth(380)

    def tab(self, title):
        """Start a new tab: following row()/add() calls fill it, in order."""
        from PySide6.QtWidgets import QTabWidget
        if not hasattr(self, "tabs"):
            self.tabs = QTabWidget()
            self.tabs.setDocumentMode(True)
            self.lay.insertWidget(self.lay.count(), self.tabs)
        page = QWidget()
        v = QVBoxLayout(page)
        v.setContentsMargins(12, 12, 12, 8)
        f = QFormLayout()
        f.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
        f.setFieldGrowthPolicy(QFormLayout.ExpandingFieldsGrow)
        v.addLayout(f)
        v.addStretch(1)
        self.tabs.addTab(page, title)
        self.form = f
        self._in_tab = True
        return page

    def row(self, label, widget):
        self.form.addRow(label, widget)
        return widget

    def add(self, widget):
        if getattr(self, "_in_tab", False):
            self.form.addRow(widget)              # full-width row, keeps the order inside the tab
        else:
            self.lay.insertWidget(self.lay.count(), widget)
        return widget

    def finish(self):
        self.lay.addWidget(self.buttons)
        return self.exec()

    def _accept(self):
        if self.on_ok is None or self.on_ok() is not False:
            self.accept()


def L(win, it, en):
    return win.L(it, en)


# ====================================================================== drawing / shapes
def text_dialog(win, at):
    d = Dialog(win, L(win, "Testo", "Text"), win.tr("create"))
    text = d.row(L(win, "Testo", "Text"), QLineEdit("LaserForge"))
    size = d.row(L(win, "Altezza (mm)", "Height (mm)"), dspin(20, 1, 500, 1, 1))
    font = d.row("Font", QFontComboBox())
    font.setCurrentFont(win.font())
    font.setCurrentText(win.settings.value("lastFont", "Helvetica"))
    single = d.row(L(win, "Tratto singolo", "Single-line"), QComboBox())
    single.addItem(L(win, "— no (contorno lettere) —", "— no (letter outlines) —"), None)
    for fid, label in M.single_line_fonts():
        single.addItem(label, fid)
    last_single = win.settings.value("lastSingleFont", "")
    if last_single:
        single.setCurrentIndex(max(0, single.findData(last_single)))
    single.currentIndexChanged.connect(lambda _i: font.setEnabled(single.currentData() is None))
    font.setEnabled(single.currentData() is None)

    def ok():
        t = text.text().strip()
        if not t:
            return False
        win.settings.setValue("lastFont", font.currentFont().family())
        win.settings.setValue("lastSingleFont", single.currentData() or "")
        fam = single.currentData() or font.currentFont().family()
        win.doc.add_shapes([win.doc.create("text", x=at[0], y=at[1], text=t, size=size.value(), font=fam)])
        win.set_tool("select")
    d.on_ok = ok
    text.selectAll()
    d.finish()


def edit_text_dialog(win, s):
    d = Dialog(win, L(win, "Modifica testo", "Edit Text"), win.tr("apply"))
    text = d.row(L(win, "Testo", "Text"), QLineEdit(s["text"]))
    size = d.row(L(win, "Altezza (mm)", "Height (mm)"), dspin(s["size"], 0.5, 500, 1, 2))
    font = d.row("Font", QFontComboBox())
    font.setCurrentText(s.get("font") or "Helvetica")

    def ok():
        win.doc.save_undo()
        s["text"] = text.text() or s["text"]
        s["size"] = size.value()
        s["font"] = font.currentFont().family()
        win.doc.notify()
    d.on_ok = ok
    d.finish()


def polygon_dialog(win):
    d = Dialog(win, L(win, "Poligono", "Polygon"), win.tr("ok"))
    sides = d.row(L(win, "Lati", "Sides"), ispin(win.polygon_sides, 3, 64))

    def ok():
        win.polygon_sides = sides.value()
        win.set_tool("polygon")
    d.on_ok = ok
    d.finish()


def grid_array_dialog(win):
    sel = win.need_sel()
    if not sel:
        return
    d = Dialog(win, L(win, "Copia a griglia", "Grid Array"), win.tr("create"))
    cols = d.row(L(win, "Colonne", "Columns"), ispin(3, 1, 100))
    rows = d.row(L(win, "Righe", "Rows"), ispin(2, 1, 100))
    gx = d.row(L(win, "Spaziatura orizz. (mm)", "Horizontal spacing (mm)"), dspin(5, -1000, 1000, 0.5))
    gy = d.row(L(win, "Spaziatura vert. (mm)", "Vertical spacing (mm)"), dspin(5, -1000, 1000, 0.5))
    live = d.add(checkbox(L(win, "Array dinamico (modificabile dal pannello Proprietà forma)", "Live array (editable from Shape Properties)"), True))

    def ok():
        if live.isChecked():
            from .props import make_array
            g = make_array(win, sel, {"kind": "grid", "cols": cols.value(), "rows": rows.value(), "dx": gx.value(), "dy": gy.value()})
            win.doc.save_undo()
            first = min(win.doc.shapes.index(x) for x in sel)
            win.doc.shapes = [x for x in win.doc.shapes if x not in sel]
            win.doc.shapes.insert(first, g)
            win.doc.selected = [g["id"]]
            win.doc.notify()
            return
        b = M.multi_bounds(sel)
        out = []
        for r in range(rows.value()):
            for c in range(cols.value()):
                if r == 0 and c == 0:
                    continue
                for s in M.clone(sel):
                    win.doc.reid(s)
                    M.move_shape(s, c * (b[2] + gx.value()), r * (b[3] + gy.value()))
                    out.append(s)
        win.doc.add_shapes(out, select=False)
    d.on_ok = ok
    d.finish()


def circular_array_dialog(win):
    sel = win.need_sel()
    if not sel:
        return
    b = M.multi_bounds(sel)
    d = Dialog(win, L(win, "Copia circolare", "Circular Array"), win.tr("create"))
    n = d.row(L(win, "Copie", "Copies"), ispin(6, 2, 360))
    total = d.row(L(win, "Angolo totale", "Total angle"), dspin(360, -360, 360, 15, 1))
    cx = d.row(L(win, "Centro X", "Center X"), dspin(b[0] + b[2] / 2, -10000, 10000, 1))
    cy = d.row(L(win, "Centro Y", "Center Y"), dspin(b[1] + b[3] + 20, -10000, 10000, 1))
    rot = d.add(checkbox(L(win, "Ruota le copie", "Rotate copies"), True))
    live = d.add(checkbox(L(win, "Array dinamico (modificabile dal pannello Proprietà forma)", "Live array (editable from Shape Properties)"), True))

    def ok():
        cnt, tot = n.value(), total.value()
        if live.isChecked():
            from .props import make_array
            g = make_array(win, sel, {"kind": "circular", "n": cnt, "angle": tot, "cx": cx.value(), "cy": cy.value(), "rotate": rot.isChecked()})
            win.doc.save_undo()
            first = min(win.doc.shapes.index(x) for x in sel)
            win.doc.shapes = [x for x in win.doc.shapes if x not in sel]
            win.doc.shapes.insert(first, g)
            win.doc.selected = [g["id"]]
            win.doc.notify()
            return
        step = tot / cnt if abs(tot) >= 360 else tot / (cnt - 1)
        out = []
        for i in range(1, cnt):
            deg = step * i
            a = math.radians(deg)
            for s in M.clone(sel):
                win.doc.reid(s)
                if rot.isChecked() and M.rotate_shape(s, cx.value(), cy.value(), deg):
                    win._fix_child_ids(s)
                    out.append(s)
                    continue
                c0 = M.center(M.bounds(s))
                dx, dy = c0[0] - cx.value(), c0[1] - cy.value()
                M.move_shape(s, cx.value() + dx * math.cos(a) - dy * math.sin(a) - c0[0], cy.value() + dx * math.sin(a) + dy * math.cos(a) - c0[1])
                out.append(s)
        win.doc.add_shapes(out, select=False)
    d.on_ok = ok
    d.finish()


def offset_dialog(win):
    if not [s for s in win.doc.selected_shapes() if s["type"] != "image"]:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, L(win, "Offset forme", "Offset Shapes"), win.tr("ok"))
    dist = d.row(L(win, "Distanza (mm)", "Distance (mm)"), dspin(2, 0.01, 500, 0.1))
    direction = d.row(L(win, "Direzione", "Direction"), Segmented([("out", L(win, "Esterno", "Outward")), ("in", L(win, "Interno", "Inward")), ("both", L(win, "Entrambi", "Both"))]))
    direction.setValue("out")
    join = d.row(L(win, "Angoli", "Corners"), Segmented([("round", L(win, "Arrotondati", "Round")), ("mitre", L(win, "Vivi", "Sharp")), ("bevel", L(win, "Smussati", "Bevel"))]))
    join.setValue("round")
    delete = d.add(checkbox(L(win, "Elimina le forme originali", "Delete original shapes")))
    d.on_ok = lambda: win.geometry_op("offset", {"distance": dist.value(), "direction": direction.value(), "join": join.value()}, replace=delete.isChecked())
    d.finish()


def round_corners_dialog(win):
    if not [s for s in win.doc.selected_shapes() if s["type"] != "image"]:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, win.action_label("roundCorners").replace("...", ""), win.tr("ok"))
    r = d.row(L(win, "Raggio (mm)", "Radius (mm)"), dspin(3, 0.1, 500, 0.5))
    d.on_ok = lambda: win.geometry_op("round", {"radius": r.value()})
    d.finish()


def copy_along_path_dialog(win):
    sel = win.doc.selected_in_order()
    if len(sel) < 2:
        win.toast(L(win, "Seleziona gli oggetti e per ultimo il percorso", "Select the objects, then the path last"))
        return
    path, objs = sel[-1], sel[:-1]
    d = Dialog(win, win.action_label("copyAlongPath").replace("...", ""), win.tr("create"))
    n = d.row(L(win, "Copie", "Copies"), ispin(8, 2, 500))
    rot = d.add(checkbox(L(win, "Ruota seguendo il percorso", "Rotate to follow the path"), True))
    delete = d.add(checkbox(L(win, "Elimina gli originali", "Delete the originals")))

    def point_at(pts, dist):
        for i in range(1, len(pts)):
            seg = math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1])
            if dist <= seg or i == len(pts) - 1:
                k = min(1, dist / seg) if seg else 0
                return (pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * k, pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * k,
                        math.degrees(math.atan2(pts[i][1] - pts[i - 1][1], pts[i][0] - pts[i - 1][0])))
            dist -= seg
        return (pts[0][0], pts[0][1], 0)

    def ok():
        cnt = n.value()
        pts = M.shape_paths(path)[0]
        total = sum(math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]) for i in range(1, len(pts)))
        closed = path["type"] in ("rect", "circle") or path.get("closed")
        step = total / cnt if closed else total / (cnt - 1)
        c0 = M.center(M.multi_bounds(objs))
        out = []
        for i in range(cnt):
            x, y, ang = point_at(pts, step * i)
            for s in M.clone(objs):
                win.doc.reid(s)
                M.move_shape(s, x - c0[0], y - c0[1])
                if rot.isChecked():
                    M.rotate_shape(s, x, y, ang)
                    win._fix_child_ids(s)
                out.append(s)
        win.doc.save_undo()
        if delete.isChecked():
            win.doc.shapes = [s for s in win.doc.shapes if s not in objs]
        win.doc.shapes.extend(out)
        win.doc.selected = [s["id"] for s in out]
        win.doc.notify()
    d.on_ok = ok
    d.finish()


def qr_dialog(win):
    d = Dialog(win, L(win, "Crea codice QR", "Create QR Code"), win.tr("create"))
    text = d.row(L(win, "Testo o URL", "Text or URL"), QLineEdit())
    text.setPlaceholderText("https://")
    size = d.row(L(win, "Dimensione (mm)", "Size (mm)"), dspin(30, 5, 400, 1, 1))
    err = d.row(L(win, "Correzione errori", "Error correction"), Segmented([("l", "L 7%"), ("m", "M 15%"), ("q", "Q 25%"), ("h", "H 30%")]))
    err.setValue("m")
    d.add(hint(L(win, "Il layer attivo viene impostato su Riempi per incidere i moduli.", "The active layer is switched to Fill so the modules get engraved.")))

    def ok():
        t = text.text()
        if not t:
            return False
        matrix = win.engine.qr_matrix(t, err.value())
        n = len(matrix)
        m = size.value() / n
        W, H = win.workspace()
        x0, y0 = (W - size.value()) / 2, (H - size.value()) / 2
        rects = []
        for r, row in enumerate(matrix):
            c = 0
            while c < n:
                if not row[c]:
                    c += 1
                    continue
                start = c
                while c < n and row[c]:
                    c += 1
                rects.append({"id": win.doc.new_id(), "type": "rect", "layer": win.doc.active_layer, "x": x0 + start * m, "y": y0 + r * m, "w": (c - start) * m, "h": m})
        win.doc.add_shapes([win.doc.create("group", children=rects)])
        if win.engine.layers[win.doc.active_layer].cut_mode.value == "line":
            win.save_layer_settings({"cut_mode": "fill"})
            win.cuts.load(win.doc.active_layer)
    d.on_ok = ok
    d.finish()


# ====================================================================== images
class ImagePreview(QLabel):
    def __init__(self, w=380, h=300):
        super().__init__()
        self.setFixedSize(w, h)
        self.setAlignment(Qt.AlignCenter)
        self.setStyleSheet("background:#f4f4f4; border-radius:4px;")

    def show_burn(self, res):
        if res is None:
            self.clear()
            return
        burn, cols, rows = res
        gray = np.ascontiguousarray(255 - burn)
        img = QImage(gray.data, cols, rows, cols, QImage.Format_Grayscale8).copy()
        self.setPixmap(QPixmap.fromImage(img).scaled(self.size(), Qt.KeepAspectRatio, Qt.FastTransformation))


def adjust_image_dialog(win):
    s = next((x for x in win.doc.selected_shapes() if x["type"] == "image"), None)
    if not s:
        win.toast(win.tr("selectImage"))
        return
    a = {**M.default_adj(), **(s.get("adj") or {})}
    d = Dialog(win, L(win, "Regola immagine", "Adjust Image"), win.tr("apply"), wide=True)
    body = QHBoxLayout()
    prev = ImagePreview()
    body.addWidget(prev)
    ctrl = QFormLayout()
    names = {"dither": "Floyd–Steinberg", "jarvis": "Jarvis", "stucki": "Stucki", "atkinson": "Atkinson", "sierra": "Sierra",
             "bayer": L(win, "Bayer (ordinato)", "Bayer (ordered)"), "halftone": L(win, "Mezzitoni", "Halftone"),
             "newsprint": L(win, "Giornale", "Newsprint"), "sketch": L(win, "Schizzo (contorni)", "Sketch (edges)"),
             "gray": L(win, "Scala di grigi", "Grayscale"), "threshold": L(win, "Soglia (bianco/nero)", "Threshold (black/white)"),
             "slices3d": L(win, "3D a strati (mappa di profondità)", "3D slices (depth map)")}
    mode = QComboBox()
    for k in M.DITHER_MODES:
        mode.addItem(names[k], k)
    mode.setCurrentIndex(max(0, mode.findData(a["mode"])))
    ctrl.addRow(L(win, "Modalità", "Mode"), mode)
    cpi = dspin(float(a.get("cells_per_inch", 30)), 5, 200, 1, 0, " lpi")
    hang = dspin(float(a.get("halftone_angle", 22.5)), 0, 90, 1, 1, " °")
    ctrl.addRow(L(win, "Celle per pollice", "Cells per inch"), cpi)
    ctrl.addRow(L(win, "Angolo retino", "Screen angle"), hang)
    slices = ispin(int(a.get("slices", 10)), 2, 100)
    ctrl.addRow(L(win, "Strati 3D", "3D slices"), slices)
    # image presets (LightBurn: Save Image Preset)
    presets = win._load_json("imagePresets", {})
    pcombo = QComboBox()
    pcombo.addItem(L(win, "— preset —", "— preset —"), None)
    for name in sorted(presets):
        pcombo.addItem(name, name)
    psave, pdel = QPushButton(L(win, "Salva", "Save")), QPushButton(L(win, "Elimina", "Delete"))
    prow = QWidget()
    ph = QHBoxLayout(prow)
    ph.setContentsMargins(0, 0, 0, 0)
    ph.addWidget(pcombo, 1)
    ph.addWidget(psave)
    ph.addWidget(pdel)
    ctrl.addRow(L(win, "Preset", "Preset"), prow)

    def slider(lo, hi, v):
        sl = QSlider(Qt.Horizontal)
        sl.setRange(lo, hi)
        sl.setValue(int(v))
        return sl
    br, ct, th = slider(-100, 100, a["brightness"]), slider(-100, 100, a["contrast"]), slider(1, 254, a["threshold"])
    ctrl.addRow(L(win, "Luminosità", "Brightness"), br)
    ctrl.addRow(L(win, "Contrasto", "Contrast"), ct)
    gam = dspin(float(a.get("gamma", 1.0)), 0.1, 5.0, 0.05, 2)
    gam.setToolTip(L(win, "Curva gamma per i mezzitoni: 1.0 = normale, più alto = mezzitoni più chiari",
                     "Gamma curve for mid-tones: 1.0 = normal, higher = lighter mid-tones"))
    ctrl.addRow("Gamma", gam)
    e_amt = ispin(int(a.get("enhance_amount", 0)), 0, 500)
    e_rad = dspin(float(a.get("enhance_radius", 0.0)), 0, 10, 0.1, 2, " mm")
    e_den = ispin(int(a.get("enhance_denoise", 0)), 0, 50)
    ctrl.addRow(L(win, "Nitidezza: quantità %", "Enhance amount %"), e_amt)
    ctrl.addRow(L(win, "Nitidezza: raggio", "Enhance radius"), e_rad)
    ctrl.addRow(L(win, "Nitidezza: riduzione rumore", "Enhance denoise"), e_den)
    ctrl.addRow(L(win, "Soglia", "Threshold"), th)
    inv = checkbox(L(win, "Inverti", "Invert"), a["invert"])
    ctrl.addRow("", inv)
    ctrl.addRow("", hint(L(win, f"La risoluzione segue l'intervallo del layer ({win.layer_interval(s.get('layer', 0))} mm).",
                             f"Resolution follows the layer interval ({win.layer_interval(s.get('layer', 0))} mm).")))
    cw = QWidget()
    cw.setLayout(ctrl)
    body.addWidget(cw, 1)
    d.lay.insertLayout(0, body)

    def read():
        return {"mode": mode.currentData(), "brightness": br.value(), "contrast": ct.value(), "threshold": th.value(),
                "invert": inv.isChecked(), "cells_per_inch": cpi.value(), "halftone_angle": hang.value(), "slices": slices.value(),
                "gamma": round(gam.value(), 2), "enhance_amount": e_amt.value(), "enhance_radius": e_rad.value(),
                "enhance_denoise": e_den.value()}

    def load_adj(v):
        mode.setCurrentIndex(max(0, mode.findData(v.get("mode", "dither"))))
        br.setValue(int(v.get("brightness", 0)))
        ct.setValue(int(v.get("contrast", 0)))
        th.setValue(int(v.get("threshold", 128)))
        inv.setChecked(bool(v.get("invert")))
        cpi.setValue(float(v.get("cells_per_inch", 30)))
        hang.setValue(float(v.get("halftone_angle", 22.5)))
        slices.setValue(int(v.get("slices", 10)))
        gam.setValue(float(v.get("gamma", 1.0)))
        e_amt.setValue(int(v.get("enhance_amount", 0)))
        e_rad.setValue(float(v.get("enhance_radius", 0.0)))
        e_den.setValue(int(v.get("enhance_denoise", 0)))

    def pick_preset(_i):
        name = pcombo.currentData()
        if name in presets:
            load_adj(presets[name])

    def save_preset():
        from PySide6.QtWidgets import QInputDialog
        name, ok = QInputDialog.getText(d, L(win, "Salva preset immagine", "Save image preset"), L(win, "Nome:", "Name:"))
        if ok and name.strip():
            presets[name.strip()] = read()
            win._store_json("imagePresets", presets)
            if pcombo.findData(name.strip()) < 0:
                pcombo.addItem(name.strip(), name.strip())
            pcombo.setCurrentIndex(pcombo.findData(name.strip()))

    def delete_preset():
        name = pcombo.currentData()
        if name in presets:
            presets.pop(name)
            win._store_json("imagePresets", presets)
            pcombo.removeItem(pcombo.currentIndex())
    pcombo.activated.connect(pick_preset)
    psave.clicked.connect(save_preset)
    pdel.clicked.connect(delete_preset)

    def sync_controls():
        m = mode.currentData()
        for w_ in (cpi, hang):
            w_.setEnabled(m in ("halftone", "newsprint"))
        hang.setEnabled(m == "halftone")
        th.setEnabled(m in ("threshold", "sketch"))
        slices.setEnabled(m == "slices3d")

    def paint(*_):
        prev.show_burn(M.process_image({**s, "adj": read()}, max(win.layer_interval(s.get("layer", 0)), s["w"] / 500), 500))
    for w in (br, ct, th):
        w.valueChanged.connect(paint)
    mode.currentIndexChanged.connect(lambda _i: (sync_controls(), paint()))
    cpi.valueChanged.connect(paint)
    slices.valueChanged.connect(paint)
    hang.valueChanged.connect(paint)
    for w_ in (gam, e_amt, e_rad, e_den):
        w_.valueChanged.connect(paint)
    inv.toggled.connect(paint)
    sync_controls()
    paint()

    def ok():
        win.doc.save_undo()
        s["adj"] = read()
        M.clear_image_caches()
        win.doc.notify()
    d.on_ok = ok
    d.finish()


def trace_image_dialog(win):
    s = next((x for x in win.doc.selected_shapes() if x["type"] == "image"), None)
    if not s:
        win.toast(win.tr("selectImage"))
        return
    d = Dialog(win, L(win, "Traccia immagine", "Trace Image"), L(win, "Traccia", "Trace"))
    thr = d.row(L(win, "Soglia", "Cutoff"), ispin(128, 1, 254))
    smooth = d.row(L(win, "Smussatura", "Smoothness"), dspin(0.6, 0, 5, 0.1))
    res = d.row(L(win, "Risoluzione max (px)", "Max resolution (px)"), ispin(600, 100, 2000, 50))
    delete = d.add(checkbox(L(win, "Elimina immagine dopo la traccia", "Delete image after tracing")))

    def ok():
        r = res.value()
        p = M.process_image({**s, "adj": {**(s.get("adj") or M.default_adj()), "mode": "threshold", "threshold": thr.value()}}, max(s["w"], s["h"]) / r, r)
        if not p:
            return False
        burn, cols, rows = p
        loops = M.trace_binary(burn > 127)
        kx, ky = s["w"] / cols, s["h"] / rows
        shapes = []
        for lp in loops:
            pts = M.simplify(lp, smooth.value())
            if len(pts) > 3:
                shapes.append({"id": win.doc.new_id(), "type": "poly", "layer": s.get("layer", 0),
                               "points": [{"x": s["x"] + x * kx, "y": s["y"] + y * ky} for x, y in pts[:-1]], "closed": True})
        if not shapes:
            win.toast(L(win, "Niente da tracciare", "Nothing traced"))
            return False
        win.doc.save_undo()
        if delete.isChecked():
            win.doc.shapes = [x for x in win.doc.shapes if x["id"] != s["id"]]
        g = win.doc.create("group", children=shapes, layer=s.get("layer", 0))
        win.doc.shapes.append(g)
        win.doc.selected = [g["id"]]
        win.doc.notify()
        win.toast(L(win, f"{len(shapes)} percorsi tracciati", f"{len(shapes)} paths traced"))
    d.on_ok = ok
    d.finish()


# ====================================================================== settings
def origin_selector(win, value):
    """Four-corner origin picker laid out like the machine seen from above (back at the top)."""
    w = Segmented([("rl", L(win, "↖ Dietro sx", "↖ Rear left")), ("rr", L(win, "Dietro dx ↗", "Rear right ↗")),
                   ("fl", L(win, "↙ Davanti sx", "↙ Front left")), ("fr", L(win, "Davanti dx ↘", "Front right ↘"))])
    w.setValue(value or "fl")
    w.setToolTip(L(win, "L'angolo dove si trova lo 0,0 della macchina (dove va la testa con Home, o dove era all'accensione se non ci sono finecorsa). "
                        "La maggior parte dei laser a diodo ha l'origine davanti a sinistra.",
                   "The corner where the machine's 0,0 is (where Home sends the head, or where it was at power-on without limit switches). "
                   "Most diode lasers have the origin at the front left."))
    return w


def device_settings_dialog(win):
    m = win.engine.machine
    d = Dialog(win, L(win, "Impostazioni dispositivo", "Device Settings"), win.tr("ok"))
    # ---- Generale
    d.tab(L(win, "Generale", "General"))
    wx = d.row(L(win, "Area di lavoro larghezza (mm)", "Work area width (mm)"), dspin(m.workspace_x, 10, 3000, 10, 1))
    wy = d.row(L(win, "Area di lavoro altezza (mm)", "Work area height (mm)"), dspin(m.workspace_y, 10, 3000, 10, 1))
    origin = d.row(L(win, "Origine (0,0 della macchina)", "Origin (machine 0,0)"), origin_selector(win, m.origin))
    from .gcode import FLAVORS
    flavor = d.row(L(win, "Controller", "Controller"), QComboBox())
    for k, lab in FLAVORS.items():
        flavor.addItem(lab, k)
    flavor.setCurrentIndex(max(0, flavor.findData(m.flavor)))
    baud = d.row("Baud rate", QComboBox())
    for b in (9600, 19200, 38400, 57600, 115200, 230400, 250000, 460800):
        baud.addItem(str(b), b)
    baud.setCurrentText(str(m.baud_rate))
    sp = d.row(L(win, "Velocità max (mm/min)", "Max speed (mm/min)"), dspin(m.max_speed, 100, 100000, 100, 0))
    pw = d.row(L(win, "Valore S max (GRBL $30)", "S-value max (GRBL $30)"), ispin(m.max_power, 1, 100000))
    home = d.add(checkbox(win.tr("homeOnStart"), m.home_on_start))

    # ---- Aria
    d.tab(L(win, "Aria", "Air"))
    air_cmd = d.row(L(win, "Comando aria compressa", "Air assist command"), QComboBox())
    air_cmd.addItem(L(win, "M8 (flood) — consigliato, come LightBurn", "M8 (flood) — recommended, LightBurn default"), "M8")
    air_cmd.addItem("M7 (mist)", "M7")
    air_cmd.setCurrentIndex(max(0, air_cmd.findData(m.air_command)))
    m7_warn = d.add(QLabel(L(win, "⚠ GRBL di serie non supporta M7: il comando viene rifiutato (error:20) e l'aria non parte. Usa M7 solo se il firmware è compilato con ENABLE_M7.",
                             "⚠ Stock GRBL does not support M7: it is rejected (error:20) and the air never starts. Use M7 only if the firmware was built with ENABLE_M7.")))
    m7_warn.setWordWrap(True)
    m7_warn.setStyleSheet("color:#e0a030")
    m7_warn.setVisible(m.air_command == "M7")
    air_cmd.currentIndexChanged.connect(lambda _i: m7_warn.setVisible(air_cmd.currentData() == "M7"))
    air = d.row(L(win, "Aria sui nuovi layer", "Air on new layers"), Segmented([("off", win.tr("airOff")), ("on", L(win, "Accesa", "On"))], air=True))
    air.setValue(m.air_assist_default.value)
    fan = d.add(checkbox(L(win, "Marlin: aria compressa sulla ventola (M106/M107)", "Marlin: air assist on the part fan (M106/M107)"), m.marlin_air_fan))
    d.add(hint(L(win, "«Solo durante il taglio» si imposta per ogni layer, nel riquadro Aria del pannello Tagli.",
                 "“Only while cutting” is set per layer, in the Air box of the Cuts panel.")))

    # ---- Movimento e invio
    d.tab(L(win, "Movimento", "Motion"))
    xfer = d.row(L(win, "Modalità di invio", "Transfer mode"), QComboBox())
    xfer.addItem(L(win, "Bufferizzata — consigliata", "Buffered — recommended"), "buffered")
    xfer.addItem(L(win, "Sincrona (una riga alla volta)", "Synchronous (one line at a time)"), "synchronous")
    xfer.setCurrentIndex(max(0, xfer.findData(m.transfer_mode)))
    acc = d.row(L(win, "Accelerazione (mm/s², $120)", "Acceleration (mm/s², $120)"), dspin(m.acceleration, 10, 50000, 50, 0))
    bl_x = d.row(L(win, "Gioco asse X (mm)", "X backlash (mm)"), dspin(m.backlash_x, 0, 5, 0.01, 3))
    bl_y = d.row(L(win, "Gioco asse Y (mm)", "Y backlash (mm)"), dspin(m.backlash_y, 0, 5, 0.01, 3))
    ret = d.add(checkbox(win.tr("returnOrigin"), m.return_to_origin))
    fin = d.row(L(win, "Posizione di fine lavoro", "Finish position"), QComboBox())
    fin.addItem(L(win, "Origine (0,0)", "Origin (0,0)"), "origin")
    fin.addItem(L(win, "Punto personalizzato", "Custom point"), "custom")
    fin.setCurrentIndex(max(0, fin.findData(m.finish_position)))
    fin_x = d.row(L(win, "Fine lavoro X (mm)", "Finish X (mm)"), dspin(m.finish_x, -3000, 3000, 1, 1))
    fin_y = d.row(L(win, "Fine lavoro Y (mm)", "Finish Y (mm)"), dspin(m.finish_y, -3000, 3000, 1, 1))

    def sync_fin(*_):
        on = ret.isChecked() and fin.currentData() == "custom"
        fin_x.setEnabled(on)
        fin_y.setEnabled(on)
        fin.setEnabled(ret.isChecked())
    fin.currentIndexChanged.connect(sync_fin)
    ret.toggled.connect(sync_fin)
    sync_fin()
    z_on = d.add(checkbox(L(win, "Abilita asse Z (offset e discesa per passata dei layer, tasti Z)", "Enable Z axis (layer Z offset and step, Z jog)"), m.z_enabled))
    z_sp = d.row(L(win, "Velocità asse Z (mm/s)", "Z axis speed (mm/s)"), dspin(m.z_speed, 0.1, 200, 0.5, 1))
    z_rel = d.add(checkbox(L(win, "Solo movimenti Z relativi (macchine senza homing Z)", "Relative Z moves only (no Z homing)"), m.z_relative_only))
    fire_on = d.add(checkbox(L(win, "Abilita pulsante Fire (pannello Sposta)", "Enable Fire button (Move panel)"), m.fire_enabled))
    fire_pw = d.row(L(win, "Potenza Fire (%)", "Fire power (%)"), dspin(m.fire_power, 0.1, 100, 0.5, 1))

    # ---- Immagini e riempimenti
    d.tab(L(win, "Immagini", "Images"))
    fws = d.add(checkbox(L(win, "Scansione veloce degli spazi bianchi", "Fast whitespace scan"), m.fast_whitespace))
    fws_sp = d.row(L(win, "Velocità minima sul bianco (mm/min)", "Min speed over blank (mm/min)"), dspin(m.whitespace_speed, 100, 100000, 100, 0))
    g0 = d.add(checkbox(L(win, "Usa G0 per l'overscan", "Use G0 for overscan"), m.use_g0_overscan))
    rrel = d.add(checkbox(L(win, "Immagini in coordinate relative (G91, come LightBurn)", "Images in relative coordinates (G91, like LightBurn)"), m.raster_relative))
    so_on = d.add(checkbox(L(win, "Correzione offset di scansione (bidirezionale)", "Scanning offset adjustment (bidirectional)"), m.scan_offset_enabled))
    from PySide6.QtWidgets import QPlainTextEdit as _QPTE
    so_tab = _QPTE(m.scan_offsets)
    so_tab.setFixedHeight(80)
    so_tab.setPlaceholderText(L(win, "velocità:offset, una per riga (mm/min:mm)\nes. 1000:0.05\n6000:0.18",
                                "speed:offset, one per line (mm/min:mm)\ne.g. 1000:0.05\n6000:0.18"))
    so_tab.setToolTip(L(win, "Incidi una prova bidirezionale: misura lo sfalsamento tra le righe di andata e ritorno e inserisci metà della distanza.",
                         "Engrave a bidirectional test: measure the gap between forward and reverse lines and enter half of it."))
    d.add(so_tab)

    # ---- G-code
    d.tab("G-code")
    from PySide6.QtWidgets import QPlainTextEdit
    from PySide6.QtGui import QFont as _QFont
    start_g, end_g = QPlainTextEdit(m.start_gcode), QPlainTextEdit(m.end_gcode)
    for ed in (start_g, end_g):
        ed.setFont(_QFont("Menlo", 11))
        ed.setFixedHeight(90)
        ed.setPlaceholderText(L(win, "es. M8 ; aria\nG4 P0.5", "e.g. M8 ; air\nG4 P0.5"))
    d.add(QLabel(L(win, "G-code iniziale (all'inizio di ogni lavoro)", "Start G-code (start of every job)")))
    d.add(start_g)
    d.add(QLabel(L(win, "G-code finale (alla fine di ogni lavoro)", "End G-code (end of every job)")))
    d.add(end_g)
    d._in_tab = False
    d.setMinimumWidth(560)

    def ok():
        win.engine.update_machine({"workspace_x": wx.value(), "workspace_y": wy.value(), "max_speed": sp.value(), "max_power": pw.value(),
                                   "baud_rate": baud.currentData(), "air_assist_default": air.value(), "air_command": air_cmd.currentData(),
                                   "fast_whitespace": fws.isChecked(), "whitespace_speed": fws_sp.value(),
                                   "scan_offset_enabled": so_on.isChecked(), "scan_offsets": so_tab.toPlainText(),
                                   "home_on_start": home.isChecked(), "return_to_origin": ret.isChecked(), "use_g0_overscan": g0.isChecked(), "raster_relative": rrel.isChecked(),
                                   "finish_position": fin.currentData(), "finish_x": fin_x.value(), "finish_y": fin_y.value(),
                                   "transfer_mode": xfer.currentData(), "acceleration": acc.value(),
                                   "backlash_x": bl_x.value(), "backlash_y": bl_y.value(),
                                   "fire_enabled": fire_on.isChecked(), "fire_power": fire_pw.value(),
                                   "z_enabled": z_on.isChecked(), "z_speed": z_sp.value(), "z_relative_only": z_rel.isChecked(),
                                   "start_gcode": start_g.toPlainText(), "end_gcode": end_g.toPlainText(),
                                   "flavor": flavor.currentData(), "marlin_air_fan": fan.isChecked(), "origin": origin.value()})
        win._store_json("machine", win.engine.machine_dict())
        win.engine.connection.buffered = win.engine.machine.transfer_mode != "synchronous"
        win.move_panel.load()
        win.cuts.retranslate()
        win.cuts.refresh()
        win.invalidate_gcode()
        win.canvas.update()
    d.on_ok = ok
    d.finish()


def optimize_dialog(win):
    o = win.opt
    d = Dialog(win, L(win, "Impostazioni ottimizzazione", "Optimization Settings"), win.tr("ok"))
    order = d.row(L(win, "Ordine", "Order"), Segmented([("layer", L(win, "Per layer", "By layer")), ("group", L(win, "Per gruppo", "By group")),
                                                     ("priority", L(win, "Per priorità", "By priority"))]))
    order.setValue(o.get("order", "layer"))
    inner = d.add(checkbox(L(win, "Taglia prima le forme interne", "Cut inner shapes first"), o.get("innerFirst", True)))
    near = d.add(checkbox(L(win, "Ordina per vicinanza (riduce gli spostamenti)", "Order by nearest (reduce travel)"), o.get("nearest", True)))
    start = d.add(checkbox(L(win, "Scegli il punto d'inizio migliore (forme chiuse)", "Choose best starting point (closed shapes)"), o.get("bestStart", True)))
    bdir = d.add(checkbox(L(win, "Scegli la direzione migliore (linee aperte)", "Choose best direction (open lines)"), o.get("bestDirection", True)))
    direction = d.row(L(win, "Direzione forme chiuse", "Closed shape direction"),
                      Segmented([("keep", L(win, "Originale", "As drawn")), ("cw", L(win, "Oraria", "Clockwise")), ("ccw", L(win, "Antioraria", "Counter-CW"))]))
    direction.setValue(o.get("direction", "keep"))
    overlap = d.add(checkbox(L(win, "Rimuovi le linee sovrapposte", "Remove overlapping lines"), o.get("removeOverlap", False)))
    d.add(hint(L(win, "Per layer: un layer alla volta (C00, C01…). Per gruppo: ogni oggetto completo prima del successivo. "
                      "Per priorità: le forme con priorità più bassa per prime (la priorità si imposta in Proprietà forma). "
                      "Le immagini vengono sempre incise per prime.",
                 "By layer: one layer at a time (C00, C01…). By group: each object complete before the next. "
                 "By priority: lower priority first (set it in Shape Properties). Images are always engraved first.")))

    def ok():
        win.opt = {"order": order.value(), "innerFirst": inner.isChecked(), "nearest": near.isChecked(), "bestStart": start.isChecked(),
                   "bestDirection": bdir.isChecked(), "direction": direction.value(), "removeOverlap": overlap.isChecked()}
        win._store_json("opt", win.opt)
        win.invalidate_gcode()
    d.on_ok = ok
    d.finish()


def rotary_dialog(win):
    m = win.engine.machine
    d = Dialog(win, L(win, "Configurazione rotativo", "Rotary Setup"), win.tr("ok"))
    on = d.add(checkbox(L(win, "Abilita rotativo", "Enable rotary"), m.rotary_enabled))
    axis = d.row(L(win, "Asse del rotativo", "Rotary axis"), Segmented([("Y", L(win, "Al posto di Y", "Replaces Y")), ("A", L(win, "Asse A", "A axis"))]))
    axis.setValue(m.rotary_axis)
    kind = d.row(L(win, "Tipo", "Type"), Segmented([("roller", L(win, "A rulli", "Roller")), ("chuck", L(win, "Mandrino", "Chuck"))]))
    kind.setValue(m.rotary_type)
    mm = d.row(L(win, "Movimento Y per giro (mm)", "Travel per rotation (mm)"), dspin(m.rotary_mm_per_rev, 0.1, 10000, 0.1))
    dia = d.row(L(win, "Diametro oggetto (mm)", "Object diameter (mm)"), dspin(m.rotary_diameter, 0.1, 2000, 0.1))
    d.add(hint(L(win, "Gli spostamenti Y vengono scalati: una circonferenza dell'oggetto corrisponde a un giro.",
                 "Y moves are scaled so one object circumference equals one rotation.")))

    def ok():
        win.engine.update_machine({"rotary_enabled": on.isChecked(), "rotary_axis": axis.value(), "rotary_type": kind.value(), "rotary_mm_per_rev": mm.value(), "rotary_diameter": dia.value()})
        win._store_json("machine", win.engine.machine_dict())
        win.invalidate_gcode()
        win.status(L(win, "Rotativo abilitato", "Rotary enabled") if on.isChecked() else win.tr("ready"))
    d.on_ok = ok
    d.finish()


# ====================================================================== laser tools
def material_test_dialog(win):
    d = Dialog(win, L(win, "Test materiale", "Material Test"), L(win, "Genera", "Generate"))
    d.add(hint(L(win, "Le righe variano la potenza, le colonne la velocità.", "Rows vary power, columns vary speed.")))
    rows = d.row(L(win, "Righe (potenza)", "Rows (power)"), ispin(5, 1, 20))
    cols = d.row(L(win, "Colonne (velocità)", "Columns (speed)"), ispin(5, 1, 20))
    pmin = d.row(L(win, "Potenza min %", "Power min %"), dspin(20, 0, 100, 5, 0))
    pmax = d.row(L(win, "Potenza max %", "Power max %"), dspin(100, 0, 100, 5, 0))
    smin = d.row(L(win, "Velocità min", "Speed min"), dspin(500, 1, 100000, 100, 0))
    smax = d.row(L(win, "Velocità max", "Speed max"), dspin(3000, 1, 100000, 100, 0))
    size = d.row(L(win, "Lato cella (mm)", "Cell size (mm)"), dspin(10, 1, 100, 1, 1))
    gap = d.row(L(win, "Spazio (mm)", "Gap (mm)"), dspin(3, 0, 50, 0.5, 1))
    mode = d.row(win.tr("mode"), Segmented([("fill", win.tr("modeFill")), ("line", win.tr("modeLine"))]))
    mode.setValue("fill")
    air = d.row(win.tr("airAssist"), Segmented([("on", "On"), ("off", "Off")], air=True))
    air.setValue("on")

    def ok():
        g = win.engine.material_test({"rows": rows.value(), "cols": cols.value(), "power_min": pmin.value(), "power_max": pmax.value(),
                                      "speed_min": smin.value(), "speed_max": smax.value(), "size": size.value(), "gap": gap.value(),
                                      "mode": mode.value(), "air_assist": air.value(), "x": 10, "y": 10})
        win.show_gcode(g)
        QTimer.singleShot(50, lambda: preview_dialog(win, g))
    d.on_ok = ok
    d.finish()


def test_pattern_dialog(win, kind):
    focus = kind == "focus"
    d = Dialog(win, L(win, "Test di messa a fuoco", "Focus Test") if focus else L(win, "Test intervallo", "Interval Test"), L(win, "Genera", "Generate"))
    if focus:
        d.add(hint(L(win, "Incide una linea per ogni altezza Z: la linea più sottile indica il fuoco migliore.", "Burns one line per Z height. The thinnest line is the best focus.")))
        n = d.row(L(win, "Linee", "Lines"), ispin(11, 2, 30))
        z0 = d.row(L(win, "Z iniziale (mm)", "Start Z (mm)"), dspin(-5, -50, 50, 0.5))
        zs = d.row(L(win, "Passo Z (mm)", "Z step (mm)"), dspin(1, 0.05, 10, 0.1))
        sp = d.row(L(win, "Velocità", "Speed"), dspin(1000, 1, 100000, 100, 0))
        pw = d.row(L(win, "Potenza %", "Power %"), dspin(30, 0, 100, 5, 0))
        d.on_ok = lambda: _show_test(win, win.engine.focus_test({"count": n.value(), "z_start": z0.value(), "z_step": zs.value(), "speed": sp.value(), "power": pw.value()}))
    else:
        d.add(hint(L(win, "Incide quadrati con intervallo di linea crescente.", "Engraves squares with increasing line interval.")))
        n = d.row(L(win, "Celle", "Cells"), ispin(6, 2, 20))
        a = d.row(L(win, "Intervallo min", "Min interval"), dspin(0.05, 0.01, 2, 0.01, 3))
        b = d.row(L(win, "Intervallo max", "Max interval"), dspin(0.3, 0.01, 2, 0.01, 3))
        sp = d.row(L(win, "Velocità", "Speed"), dspin(3000, 1, 100000, 100, 0))
        pw = d.row(L(win, "Potenza %", "Power %"), dspin(40, 0, 100, 5, 0))
        d.on_ok = lambda: _show_test(win, win.engine.interval_test({"count": n.value(), "interval_min": a.value(), "interval_max": b.value(), "speed": sp.value(), "power": pw.value()}))
    d.finish()


def _show_test(win, gcode):
    win.show_gcode(gcode)
    QTimer.singleShot(50, lambda: preview_dialog(win, gcode))


def center_finder_dialog(win):
    d = Dialog(win, L(win, "Trova centro", "Center Finder"), win.tr("close"), cancel=False)
    d.add(hint(L(win, "Porta il laser su tre punti del bordo dell'oggetto rotondo e registra ciascun punto.",
                 "Jog the laser to three points on the edge of the round object and record each one.")))
    pts = [None, None, None]
    labels = []
    grid = QGridLayout()
    res_label = QLabel("--")
    state = {"res": None}

    def compute():
        if any(p is None for p in pts):
            return
        (ax, ay), (bx, by), (cx, cy) = pts
        dd = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
        if abs(dd) < 1e-9:
            res_label.setText(L(win, "I punti sono allineati", "Points are aligned"))
            return
        sq = lambda x, y: x * x + y * y
        ux = (sq(ax, ay) * (by - cy) + sq(bx, by) * (cy - ay) + sq(cx, cy) * (ay - by)) / dd
        uy = (sq(ax, ay) * (cx - bx) + sq(bx, by) * (ax - cx) + sq(cx, cy) * (bx - ax)) / dd
        r = math.hypot(ax - ux, ay - uy)
        state["res"] = (ux, uy, r)
        res_label.setText(f"{L(win, 'Centro', 'Center')}: X {ux:.3f} Y {uy:.3f} · Ø {r * 2:.2f} mm")

    for i in range(3):
        grid.addWidget(QLabel(f"{L(win, 'Punto', 'Point')} {i + 1}"), i, 0)
        lab = QLabel("--")
        lab.setStyleSheet("font-family: Menlo;")
        labels.append(lab)
        grid.addWidget(lab, i, 1)
        b = QPushButton(L(win, "Usa posizione laser", "Use laser position"))

        def use(_=False, k=i):
            pts[k] = win.machine_pos
            labels[k].setText(f"X {pts[k][0]:.3f} Y {pts[k][1]:.3f}")
            compute()
        b.clicked.connect(use)
        grid.addWidget(b, i, 2)
    d.lay.insertLayout(1, grid)
    d.add(res_label)
    h = QHBoxLayout()
    mv = QPushButton(L(win, "Sposta laser al centro", "Move laser to center"))
    mv.clicked.connect(lambda: state["res"] and win.goto_canvas(state["res"][0], state["res"][1]))
    circ = QPushButton(L(win, "Disegna cerchio", "Draw circle"))
    circ.clicked.connect(lambda: state["res"] and win.doc.add_shapes([win.doc.create("circle", cx=state["res"][0], cy=state["res"][1], rx=state["res"][2], ry=state["res"][2])]))
    h.addWidget(mv)
    h.addWidget(circ)
    d.lay.addLayout(h)
    d.finish()


def print_and_cut_dialog(win):
    pc = win.print_cut or {"enabled": False, "src": None, "dst": [None, None]}
    pc = {"enabled": pc["enabled"], "src": pc["src"], "dst": list(pc["dst"])}
    fmt = lambda p: f"X {p[0]:.2f} Y {p[1]:.2f}" if p else "--"
    d = Dialog(win, L(win, "Stampa e taglia", "Print and Cut"), win.tr("ok"), wide=True)
    d.add(hint(L(win, "1) Seleziona due segni di registro nel disegno (il primo e l'ultimo selezionato). 2) Porta il laser su ciascun segno stampato e imposta i target. 3) Abilita l'allineamento: il lavoro viene spostato, ruotato e scalato per combaciare.",
                 "1) Select two registration marks in the design (first and last selected). 2) Jog the laser onto each printed mark and set the targets. 3) Enable alignment: the job is moved, rotated and scaled to match.")))
    grid = QGridLayout()
    src_lab = QLabel(f"{fmt(pc['src'][0])} / {fmt(pc['src'][1])}" if pc["src"] else "--")
    t_labs = [QLabel(fmt(pc["dst"][0])), QLabel(fmt(pc["dst"][1]))]
    grid.addWidget(QLabel(L(win, "Segni nel disegno", "Design marks")), 0, 0)
    grid.addWidget(src_lab, 0, 1)
    use_sel = QPushButton(L(win, "Usa selezione", "Use selection"))

    def from_sel():
        sel = win.doc.selected_in_order()
        if len(sel) < 2:
            win.toast(L(win, "Seleziona due segni", "Select two marks"))
            return
        pc["src"] = [M.center(M.bounds(sel[0])), M.center(M.bounds(sel[-1]))]
        src_lab.setText(f"{fmt(pc['src'][0])} / {fmt(pc['src'][1])}")
    use_sel.clicked.connect(from_sel)
    grid.addWidget(use_sel, 0, 2)
    for i in range(2):
        grid.addWidget(QLabel(L(win, ["Primo target", "Secondo target"][i], ["First target", "Second target"][i])), i + 1, 0)
        grid.addWidget(t_labs[i], i + 1, 1)
        b = QPushButton(L(win, "Imposta dal laser", "Set from laser"))

        def set_t(_=False, k=i):
            pc["dst"][k] = win.machine_pos
            t_labs[k].setText(fmt(pc["dst"][k]))
        b.clicked.connect(set_t)
        grid.addWidget(b, i + 1, 2)
    d.lay.insertLayout(1, grid)
    find = QPushButton(L(win, "Trova i segni con la camera", "Find marks with the camera"))

    def find_marks():
        from .extras import detect_mark
        if not pc["src"]:
            win.toast(L(win, "Imposta prima i segni nel disegno", "Set the design marks first"))
            return
        overlay = win.camera_panel.corrected()
        if overlay is None:
            win.toast(L(win, "Avvia e allinea prima la camera", "Start and align the camera first"))
            return
        found = [detect_mark(overlay, win.workspace(), x, y) for x, y in pc["src"]]
        if any(f is None for f in found):
            win.toast(L(win, "Segni non trovati vicino alle posizioni attese", "Marks not found near the expected positions"))
            return
        for k in range(2):
            pc["dst"][k] = found[k]
            t_labs[k].setText(fmt(found[k]))
    find.clicked.connect(find_marks)
    d.add(find)
    on = d.add(checkbox(L(win, "Allinea l'output ai target", "Align output to targets"), pc["enabled"]))

    def ok():
        pc["enabled"] = on.isChecked()
        if pc["enabled"] and (not pc["src"] or not all(pc["dst"])):
            win.toast(L(win, "Imposta prima i segni e entrambi i target", "Set the marks and both targets first"))
            return False
        win.print_cut = pc
        win.invalidate_gcode()
        win.status(L(win, "Allineamento Stampa e taglia attivo", "Print and Cut alignment active") if pc["enabled"] else win.tr("ready"))
    d.on_ok = ok
    d.finish()


# ====================================================================== web tools (generators)
def _finger_edge(length, t, fw, kind):
    if kind == "flat":
        return [(0, 0), (length, 0)]
    n = max(3, int(length // fw))
    if n % 2 == 0:
        n -= 1
    seg = length / n
    pts = []
    for i in range(n):
        indented = i % 2 == 0 if kind == "out" else i % 2 == 1
        dd = t if indented else 0
        pts += [(i * seg, dd), ((i + 1) * seg, dd)]
    return pts


def _box_panel(x0, y0, W, H, edges, t, fw):
    defs = [(W, lambda a, dd: (x0 + a, y0 + dd)), (H, lambda a, dd: (x0 + W - dd, y0 + a)),
            (W, lambda a, dd: (x0 + W - a, y0 + H - dd)), (H, lambda a, dd: (x0 + dd, y0 + H - a))]
    lists = [_finger_edge(ln, t, fw, edges[i]) for i, (ln, _) in enumerate(defs)]
    pts = []
    for i, (ln, fn) in enumerate(defs):
        lst = lists[i]
        prev_d, next_d = lists[(i + 3) % 4][-1][1], lists[(i + 1) % 4][0][1]
        for k, (a, dd) in enumerate(lst):
            if k == 0:
                a = max(a, prev_d)
            if k == len(lst) - 1:
                a = min(a, ln - next_d)
            p = fn(a, dd)
            if not pts or abs(pts[-1][0] - p[0]) > 1e-6 or abs(pts[-1][1] - p[1]) > 1e-6:
                pts.append(p)
    return [{"x": x, "y": y} for x, y in pts]


def box_maker_dialog(win):
    d = Dialog(win, L(win, "Generatore scatole a incastro", "Box Maker (finger joints)"), win.tr("create"))
    w = d.row(L(win, "Larghezza (mm)", "Width (mm)"), dspin(100, 10, 2000, 1, 1))
    dp = d.row(L(win, "Profondità (mm)", "Depth (mm)"), dspin(60, 10, 2000, 1, 1))
    h = d.row(L(win, "Altezza (mm)", "Height (mm)"), dspin(50, 10, 2000, 1, 1))
    t = d.row(L(win, "Spessore materiale", "Material thickness"), dspin(3, 0.5, 30, 0.1, 2))
    f = d.row(L(win, "Larghezza dente", "Finger width"), dspin(10, 2, 200, 0.5, 1))
    lid = d.add(checkbox(L(win, "Scatola chiusa (aggiungi coperchio)", "Closed box (add lid)")))

    def ok():
        W, D_, H, T, F = w.value(), dp.value(), h.value(), t.value(), f.value()
        top = "in" if lid.isChecked() else "flat"
        g, x, y = 6, 10, 10
        panels = [_box_panel(x, y, W, D_, ["out"] * 4, T, F), _box_panel(x, y + D_ + g, W, H, [top, "out", "in", "out"], T, F),
                  _box_panel(x + W + g, y + D_ + g, W, H, [top, "out", "in", "out"], T, F),
                  _box_panel(x, y + D_ + H + 2 * g, D_, H, [top, "in", "in", "in"], T, F),
                  _box_panel(x + D_ + g, y + D_ + H + 2 * g, D_, H, [top, "in", "in", "in"], T, F)]
        if lid.isChecked():
            panels.append(_box_panel(x + W + g, y, W, D_, ["out"] * 4, T, F))
        kids = [{"id": win.doc.new_id(), "type": "poly", "layer": win.doc.active_layer, "points": p, "closed": True} for p in panels]
        win.doc.add_shapes([win.doc.create("group", children=kids)])
    d.on_ok = ok
    d.finish()


def gear_maker_dialog(win):
    d = Dialog(win, L(win, "Generatore ingranaggi", "Gear Maker"), win.tr("create"))
    n = d.row(L(win, "Denti", "Teeth"), ispin(20, 6, 200))
    m = d.row(L(win, "Modulo (mm)", "Module (mm)"), dspin(2, 0.2, 20, 0.1))
    bore = d.row(L(win, "Foro Ø (mm)", "Bore Ø (mm)"), dspin(6, 0, 200, 0.5, 1))

    def ok():
        N, mod = n.value(), m.value()
        W, H = win.workspace()
        cx, cy = W / 2, H / 2
        rp = mod * N / 2
        ra, rf = rp + mod, max(0.5, rp - 1.25 * mod)
        step = math.tau / N
        at = lambda r, a: {"x": cx + r * math.cos(a), "y": cy + r * math.sin(a)}
        pts = []
        for i in range(N):
            a = i * step
            pts += [at(rf, a), at(rf, a + step * 0.12), at(ra, a + step * 0.3), at(ra, a + step * 0.45), at(rf, a + step * 0.63)]
        kids = [{"id": win.doc.new_id(), "type": "poly", "layer": win.doc.active_layer, "points": pts, "closed": True}]
        if bore.value() > 0:
            kids.append({"id": win.doc.new_id(), "type": "circle", "layer": win.doc.active_layer, "cx": cx, "cy": cy, "rx": bore.value() / 2, "ry": bore.value() / 2})
        win.doc.add_shapes([win.doc.create("group", children=kids)])
    d.on_ok = ok
    d.finish()


# ====================================================================== preview
GCODE_WORD = re.compile(r"([GXYSM])(-?\d+(?:\.\d+)?)")


def parse_gcode(gcode):
    segs = []
    x = y = s = 0.0
    rel = False
    for lineno, line in enumerate(gcode.split("\n")):
        ln = line.split(";")[0].strip()
        if not ln:
            continue
        up = ln.upper()
        if "G91" in up:
            rel = True
        if "G90" in up:
            rel = False
        words = dict(GCODE_WORD.findall(ln))
        if "S" in words:
            s = float(words["S"])
        if ln.startswith(("M5", "M2")):
            s = 0.0
        g = words.get("G")
        if g not in ("0", "1", "2", "3"):
            continue
        if rel:
            nx = x + float(words.get("X", 0))
            ny = y + float(words.get("Y", 0))
        else:
            nx = float(words["X"]) if "X" in words else x
            ny = float(words["Y"]) if "Y" in words else y
        if nx != x or ny != y:
            segs.append((x, y, nx, ny, g == "0" or s == 0, s, lineno))
        x, y = nx, ny
    return segs


class PreviewView(QWidget):
    """Job preview drawn like the design grid (machine coordinates mapped back through the origin setting).
    Anything outside the work area is shown in red."""

    def __init__(self, win, segs):
        super().__init__()
        self.win = win
        self.segs = [(*win.to_canvas((sg[0], sg[1])), *win.to_canvas((sg[2], sg[3])), *sg[4:]) for sg in segs]
        self.n = len(self.segs)
        self.show_rapids = True
        self.max_s = max([sg[5] for sg in segs] + [1])
        W, H = win.workspace()
        burn = [sg for sg in self.segs if not sg[4]]
        self.outside = sum(1 for sg in burn if not (-0.01 <= min(sg[0], sg[2]) and max(sg[0], sg[2]) <= W + 0.01
                                                     and -0.01 <= min(sg[1], sg[3]) and max(sg[1], sg[3]) <= H + 0.01))
        xs = [0, W] + [v for sg in burn for v in (sg[0], sg[2])]
        ys = [0, H] + [v for sg in burn for v in (sg[1], sg[3])]
        self.view = (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))
        self.setMinimumSize(720, 460)

    def paintEvent(self, _ev):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.fillRect(self.rect(), QColor("#e9e9ec"))
        W, H = self.win.workspace()
        vx, vy, vw, vh = self.view
        k = min((self.width() - 20) / max(vw, 1e-6), (self.height() - 20) / max(vh, 1e-6))
        ox = (self.width() - vw * k) / 2 - vx * k
        oy = (self.height() - vh * k) / 2 - vy * k
        p.fillRect(QRectF(ox, oy, W * k, H * k), QColor("#ffffff"))
        grid = QPen(QColor("#ececec"), 1)
        p.setPen(grid)
        step = 10 if W * k / 10 > 40 else 50
        for gx in range(step, int(W), step):
            p.drawLine(QPointF(ox + gx * k, oy), QPointF(ox + gx * k, oy + H * k))
        for gy in range(step, int(H), step):
            p.drawLine(QPointF(ox, oy + gy * k), QPointF(ox + W * k, oy + gy * k))
        p.setPen(QPen(QColor("#666"), 1.5))
        p.drawRect(QRectF(ox, oy, W * k, H * k))
        rapid = QPen(QColor(230, 40, 40, 90), 1)
        rapid.setDashPattern([3, 3])
        bad = QPen(QColor("#e01b1b"), 2)
        for i in range(self.n):
            x1, y1, x2, y2, is_rapid, s = self.segs[i][:6]
            if is_rapid:
                if not self.show_rapids:
                    continue
                p.setPen(rapid)
            elif not (-0.01 <= min(x1, x2) and max(x1, x2) <= W + 0.01 and -0.01 <= min(y1, y2) and max(y1, y2) <= H + 0.01):
                p.setPen(bad)
            else:
                g = round(200 - 200 * s / self.max_s)
                p.setPen(QPen(QColor(g, g, g), 1))
            p.drawLine(QPointF(ox + x1 * k, oy + y1 * k), QPointF(ox + x2 * k, oy + y2 * k))
        if 0 < self.n < len(self.segs):
            sg = self.segs[self.n - 1]
            p.setPen(Qt.NoPen)
            p.setBrush(QColor("#e01b1b"))
            p.drawEllipse(QPointF(ox + sg[2] * k, oy + sg[3] * k), 4, 4)
        if self.outside:
            p.setPen(QColor("#c62828"))
            f = p.font()
            f.setBold(True)
            p.setFont(f)
            p.drawText(QPointF(12, 22), self.win.L("Parte del lavoro è FUORI dall'area di lavoro (in rosso)", "Part of the job is OUTSIDE the work area (in red)"))
        p.end()


def preview_dialog(win, gcode=None):
    if gcode is None:
        if not win.gcode and not win.generate_gcode():
            return
        gcode = win.gcode
    segs = parse_gcode(gcode)
    d = Dialog(win, win.tr("preview"), win.tr("close"), cancel=False, wide=True)
    view = PreviewView(win, segs)
    d.lay.insertWidget(0, view, 1)
    h = QHBoxLayout()
    play = QPushButton("▶ " + L(win, "Simula", "Simulate"))
    pos = QSlider(Qt.Horizontal)
    pos.setRange(0, len(segs))
    pos.setValue(len(segs))
    rap = checkbox(L(win, "Mostra spostamenti", "Show rapids"), True)
    h.addWidget(play)
    h.addWidget(pos, 1)
    h.addWidget(rap)
    d.lay.insertLayout(1, h)
    info = QLabel()
    d.lay.insertWidget(2, info)

    def refresh(n=None):
        view.n = pos.value() if n is None else n
        view.show_rapids = rap.isChecked()
        dist = sum(math.hypot(sg[2] - sg[0], sg[3] - sg[1]) for sg in segs[:view.n] if not sg[4])
        info.setText(f"{view.n}/{len(segs)} {L(win, 'movimenti', 'moves')} · {dist:.0f} mm {L(win, 'di taglio', 'burned')}")
        view.update()
    pos.valueChanged.connect(lambda v: refresh(v))
    rap.toggled.connect(lambda _: refresh())
    timer = QTimer(d)
    step = max(1, math.ceil(len(segs) / 240))

    def tick():
        v = min(len(segs), pos.value() + step)
        pos.setValue(v)
        if v >= len(segs):
            timer.stop()
    timer.timeout.connect(tick)
    play.clicked.connect(lambda: (pos.setValue(0), timer.start(16)))
    hh = QHBoxLayout()
    start_here = QPushButton("▶ " + L(win, "Avvia da qui", "Start from here"))
    start_here.setProperty("role", "primary")
    save_here = QPushButton(L(win, "Salva G-code da qui...", "Save G-code from here..."))
    start_here.setToolTip(L(win, "Riprende il lavoro dal punto del cursore: utile dopo un'interruzione.", "Resumes the job from the slider position: useful after an interruption."))

    def from_here():
        if not segs:
            return None
        k = max(0, min(len(segs) - 1, pos.value()))
        from .gcode import resume_from
        return resume_from(gcode, segs[k][6])

    def do_start():
        g = from_here()
        if g is None:
            return
        if not win.connected:
            win.toast(L(win, "Connetti prima il laser", "Connect a laser first"))
            return
        win.engine.send(g)
        win.status(L(win, f"Lavoro ripreso dal movimento {pos.value() + 1}", f"Job resumed from move {pos.value() + 1}"))
        d.accept()

    def do_save():
        g = from_here()
        if g is not None:
            win.save_text_file(g, win.base_name() + "-da-qui.gcode", "G-code (*.gcode *.nc)")
    start_here.clicked.connect(do_start)
    save_here.clicked.connect(do_save)
    hh.addWidget(start_here)
    hh.addWidget(save_here)
    hh.addStretch(1)
    d.lay.insertLayout(3, hh)
    refresh()
    d.resize(860, 640)
    d.finish()
    timer.stop()


# ====================================================================== help
def shortcuts_dialog(win):
    rows = []

    def walk(items):
        for it in items:
            if isinstance(it, list) and it[3]:
                rows.append((it[2] if win.lang == "en" else it[1], win.actions[it[0]].shortcut().toString(0) if it[0] in win.actions else it[3]))
            elif isinstance(it, dict) and "items" in it:
                walk(it["items"])
    for m in MENUS:
        walk(m["items"])
    extra = [("Pan view", "Space + drag / middle button"), ("Zoom", "Wheel / pinch"), ("Nudge selection", "Arrows (Shift = 10 mm)"),
             ("Toggle snap", "G"), ("Move without snapping", "hold ⌥")] if win.lang == "en" else \
        [("Sposta vista", "Spazio + trascina / tasto centrale"), ("Zoom", "Rotella / pizzico"), ("Sposta selezione", "Frecce (Shift = 10 mm)"),
         ("Attiva/disattiva snap", "G"), ("Sposta senza aggancio", "tieni premuto ⌥")]
    rows += extra
    from PySide6.QtGui import QKeySequence
    d = Dialog(win, win.action_label("shortcuts"), win.tr("close"), cancel=False, wide=True)
    table = QTableWidget(len(rows), 2)
    table.horizontalHeader().hide()
    table.verticalHeader().hide()
    table.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
    table.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeToContents)
    table.setEditTriggers(QTableWidget.NoEditTriggers)
    for r, (a, b) in enumerate(rows):
        table.setItem(r, 0, QTableWidgetItem(a))
        native = QKeySequence(b).toString(QKeySequence.NativeText) if "+" in b and len(b) < 20 else b
        table.setItem(r, 1, QTableWidgetItem(native))
    table.setMinimumHeight(460)
    d.lay.insertWidget(0, table, 1)
    d.finish()


def about_dialog(win):
    from .mainwindow import APP_VERSION
    d = Dialog(win, win.action_label("about"), win.tr("close"), cancel=False)
    t = QLabel(f"<h2>LaserForge {APP_VERSION}</h2>")
    d.add(t)
    d.add(hint(L(win, "Software di taglio e incisione laser per controller GRBL con aria compressa nativa (M7/M8/M9). Applicazione nativa Qt: interfaccia, disegno e generazione del G-code nello stesso processo, come LightBurn.",
                 "Laser cutting and engraving software for GRBL controllers with native air assist (M7/M8/M9). Native Qt application: interface, drawing and G-code generation run in one process, like LightBurn.")))
    credit = QLabel(L(win, 'Icona dell\'app: "CNC Machine" di <a href="https://icons8.com">Icons8</a>',
                      'App icon: "CNC Machine" by <a href="https://icons8.com">Icons8</a>'))
    credit.setOpenExternalLinks(True)
    d.add(credit)
    d.finish()


# ====================================================================== text tools
def text_on_path_dialog(win):
    sel = win.doc.selected_in_order()
    texts = [s for s in sel if s["type"] == "text"]
    path = sel[-1] if sel else None
    if not texts or path is None or path["type"] in ("text", "image"):
        win.toast(L(win, "Seleziona un testo e per ultimo il percorso", "Select a text, then the path last"))
        return
    t = texts[0]
    d = Dialog(win, L(win, "Testo su percorso", "Text on Path"), win.tr("create"))
    start = d.row(L(win, "Inizio lungo il percorso (mm)", "Start along path (mm)"), dspin(0, 0, 100000, 1, 1))
    dist = d.row(L(win, "Distanza dal percorso (mm)", "Distance from path (mm)"), dspin(1, -500, 500, 0.5, 1))
    spacing = d.row(L(win, "Spaziatura lettere (mm)", "Letter spacing (mm)"), dspin(0, -50, 50, 0.1, 2))
    flip = d.add(checkbox(L(win, "Inverti direzione / lato", "Flip direction / side")))
    center_it = d.add(checkbox(L(win, "Centra sul percorso", "Center on path"), True))
    keep = d.add(checkbox(L(win, "Mantieni il testo originale", "Keep the original text")))

    def ok():
        pts = M.shape_paths(path)[0]
        total = sum(math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]) for i in range(1, len(pts)))
        st = start.value()
        if center_it.isChecked():
            tw = M.text_width(t) + spacing.value() * max(0, len(M.resolve_text(t)) - 1)
            st = max(0.0, (total - tw) / 2)
        polys = M.text_on_path(t, pts, st, dist.value(), spacing.value(), flip.isChecked())
        if not polys:
            return False
        kids = [{"id": win.doc.new_id(), "type": "poly", "layer": t.get("layer", 0), "points": [{"x": x, "y": y} for x, y in p[:-1]], "closed": True} for p in polys]
        g = win.doc.create("group", children=kids, layer=t.get("layer", 0))
        win.doc.save_undo()
        if not keep.isChecked():
            win.doc.shapes = [s for s in win.doc.shapes if s is not t]
        win.doc.shapes.append(g)
        win.doc.selected = [g["id"]]
        win.doc.notify()
    d.on_ok = ok
    d.finish()


def variable_text_dialog(win):
    import csv
    from PySide6.QtWidgets import QFileDialog
    t = next((s for s in win.doc.selected_shapes() if s["type"] == "text"), None)
    if t is None:
        win.toast(L(win, "Seleziona un testo", "Select a text"))
        return
    var = dict(t.get("var") or {"enabled": True, "serial": 1, "step": 1, "digits": 3, "csv": [], "row": 0})
    d = Dialog(win, L(win, "Testo variabile", "Variable Text"), win.tr("apply"), wide=True)
    d.add(hint(L(win, "Segnaposto nel testo: {serial} numero di serie · {data} · {ora} · {NomeColonna} dal file CSV. Dopo ogni lavoro inviato il numero avanza e si passa alla riga CSV successiva.",
                 "Placeholders: {serial} serial number · {date} · {time} · {ColumnName} from the CSV file. After each job sent the number advances and the next CSV row is used.")))
    template = d.row(L(win, "Testo / modello", "Text / template"), QLineEdit(t["text"] if "{" in t["text"] else t["text"] + " {serial}"))
    on = d.add(checkbox(L(win, "Abilita testo variabile", "Enable variable text"), var.get("enabled", True)))
    serial = d.row(L(win, "Numero attuale", "Current number"), ispin(int(var.get("serial", 1)), -999999, 999999))
    step = d.row(L(win, "Incremento", "Advance by"), ispin(int(var.get("step", 1)), -1000, 1000))
    digits = d.row(L(win, "Cifre minime", "Minimum digits"), ispin(int(var.get("digits", 3)), 1, 12))
    csv_lab = QLabel(L(win, f"{len(var.get('csv') or [])} righe", f"{len(var.get('csv') or [])} rows") if var.get("csv") else "--")
    load = QPushButton(L(win, "Carica CSV...", "Load CSV..."))
    row_spin = ispin(int(var.get("row", 0)) + 1, 1, 100000)
    h = QHBoxLayout()
    h.addWidget(load)
    h.addWidget(csv_lab, 1)
    h.addWidget(QLabel(L(win, "Riga", "Row")))
    h.addWidget(row_spin)
    w = QWidget()
    w.setLayout(h)
    d.row("CSV", w)
    preview = d.add(QLabel())
    preview.setStyleSheet("background:#1f2a3a; padding:6px; border-radius:4px; color:#9ec5ff; font-size:14px;")

    def current():
        return {"enabled": on.isChecked(), "serial": serial.value(), "step": step.value(), "digits": digits.value(),
                "csv": var.get("csv") or [], "row": row_spin.value() - 1}

    def refresh(*_):
        preview.setText(M.resolve_text({"text": template.text(), "var": current()}))

    def load_csv():
        path, _ = QFileDialog.getOpenFileName(d, "CSV", win.settings.value("lastDir", ""), "CSV (*.csv *.txt)")
        if not path:
            return
        with open(path, newline="", encoding="utf-8-sig", errors="replace") as f:
            sample = f.read(4096)
            f.seek(0)
            try:
                dialect = csv.Sniffer().sniff(sample, delimiters=",;\t")
            except csv.Error:
                dialect = csv.excel
            rows = list(csv.DictReader(f, dialect=dialect))
        var["csv"] = rows
        cols = list(rows[0].keys()) if rows else []
        csv_lab.setText(L(win, f"{len(rows)} righe · colonne: ", f"{len(rows)} rows · columns: ") + ", ".join("{" + c + "}" for c in cols))
        row_spin.setMaximum(max(1, len(rows)))
        refresh()
    load.clicked.connect(load_csv)
    for wd in (serial, step, digits, row_spin):
        wd.valueChanged.connect(refresh)
    template.textChanged.connect(refresh)
    on.toggled.connect(refresh)
    refresh()

    def ok():
        win.doc.save_undo()
        t["text"] = template.text()
        t["var"] = current()
        win.doc.notify()
    d.on_ok = ok
    d.finish()


# ====================================================================== camera alignment
class CornerPicker(QWidget):
    LABELS = ["TL", "TR", "BR", "BL"]

    def __init__(self, img, corners):
        super().__init__()
        self.img = img
        self.corners = list(corners or [])
        self.setMinimumSize(640, 420)
        self.setCursor(Qt.CrossCursor)

    def _geom(self):
        k = min(self.width() / self.img.width(), self.height() / self.img.height())
        return k, (self.width() - self.img.width() * k) / 2, (self.height() - self.img.height() * k) / 2

    def mousePressEvent(self, ev):
        k, ox, oy = self._geom()
        x, y = (ev.position().x() - ox) / k, (ev.position().y() - oy) / k
        if 0 <= x <= self.img.width() and 0 <= y <= self.img.height():
            if len(self.corners) >= 4:
                self.corners = []
            self.corners.append((x, y))
            self.update()

    def paintEvent(self, _ev):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.fillRect(self.rect(), QColor("#111"))
        k, ox, oy = self._geom()
        p.drawImage(QRectF(ox, oy, self.img.width() * k, self.img.height() * k), self.img)
        pts = [QPointF(ox + x * k, oy + y * k) for x, y in self.corners]
        p.setPen(QPen(QColor("#ff2a2a"), 2))
        for i, q in enumerate(pts):
            p.drawEllipse(q, 6, 6)
            p.drawText(q + QPointF(8, -8), self.LABELS[i])
            if i:
                p.drawLine(pts[i - 1], q)
        if len(pts) == 4:
            p.drawLine(pts[3], pts[0])
        p.end()


def camera_align_dialog(win):
    frame = win.camera_panel.frame
    if frame is None:
        win.toast(L(win, "Avvia prima la camera (Finestra › Controllo camera)", "Start the camera first (Window › Camera Control)"))
        win.dock_camera.show()
        win.dock_camera.raise_()
        return
    d = Dialog(win, L(win, "Allinea camera", "Camera Alignment"), win.tr("ok"), wide=True)
    d.add(hint(L(win, "Clicca nell'immagine i 4 angoli dell'area di lavoro in quest'ordine: in alto a sinistra, in alto a destra, in basso a destra, in basso a sinistra. Suggerimento: incidi prima un rettangolo grande quanto l'area di lavoro (o segna gli angoli spostando il laser).",
                 "Click the 4 corners of the work area in this order: top-left, top-right, bottom-right, bottom-left. Tip: first burn a rectangle the size of the work area (or mark the corners by jogging the laser).")))
    picker = CornerPicker(frame.copy(), win.camera_cal.get("corners"))
    d.lay.insertWidget(1, picker, 1)
    reset = QPushButton(L(win, "Azzera punti", "Clear points"))
    reset.clicked.connect(lambda: (setattr(picker, "corners", []), picker.update()))
    d.add(reset)

    def ok():
        if len(picker.corners) != 4:
            win.toast(L(win, "Servono 4 punti", "4 points are needed"))
            return False
        win.camera_cal["corners"] = picker.corners
        win._store_json("camera", win.camera_cal)
        win.camera_panel.capture_overlay()
    d.on_ok = ok
    d.resize(900, 680)
    d.finish()


# ====================================================================== shape editing tools
def _closed_polys_from_selection(win):
    """Selected rect/circle/poly shapes converted to editable paths (in place)."""
    out = []
    for s in win.doc.selected_shapes():
        if s["type"] in ("rect", "circle"):
            M.replace_in_place(s, M.to_poly(s))
        if s["type"] == "poly":
            out.append(s)
        elif s["type"] == "group":
            out += [c for c in M.flatten([s]) if c["type"] == "poly"]
    return out


def fillet_dialog(win):
    from . import corners as C
    if not win.doc.selected:
        win.toast(win.tr("selectFirst"))
        return
    nodes = win.canvas.handlers["nodes"].sel if win.canvas.tool == "nodes" else set()
    d = Dialog(win, L(win, "Raccorda angoli", "Fillet Corners"), win.tr("ok"))
    r = d.row(L(win, "Raggio (mm)", "Radius (mm)"), dspin(2, 0.05, 500, 0.5, 2))
    d.add(hint(L(win, f"{len(nodes)} nodi selezionati: verranno raccordati solo quelli." if nodes else "Nessun nodo selezionato: vengono raccordati tutti gli angoli dritti delle forme selezionate (seleziona nodi con lo strumento Modifica nodi per sceglierli).",
                 f"{len(nodes)} selected nodes: only those are filleted." if nodes else "No nodes selected: every straight corner of the selected shapes is filleted (select nodes with Edit Nodes to choose).")))

    def ok():
        win.doc.save_undo()
        polys = _closed_polys_from_selection(win)
        n = 0
        for s in polys:
            only = {i for sid, i in nodes if sid == s["id"]} if nodes else None
            if only is not None and not only:
                continue
            n += C.fillet(s, r.value(), only)
        win.canvas.handlers["nodes"].sel.clear()
        win.doc.notify()
        win.toast(L(win, f"{n} angoli raccordati", f"{n} corners filleted"))
    d.on_ok = ok
    d.finish()


def dogbone_dialog(win):
    from . import corners as C
    if not win.doc.selected:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, "Dogbone / T-bone", win.tr("ok"))
    kind = d.row(L(win, "Tipo", "Type"), Segmented([("dogbone", "Dogbone"), ("tbone", "T-bone")]))
    kind.setValue("dogbone")
    dia = d.row(L(win, "Diametro (mm)", "Diameter (mm)"), dspin(3, 0.1, 50, 0.1, 2))
    near = d.add(checkbox(L(win, "Solo angoli di circa 90°", "Only corners near 90°"), True))
    d.add(hint(L(win, "Aggiunge uno scarico circolare agli angoli interni del materiale (fessure e fori), così un pezzo ad angolo vivo entra fino in fondo. Utile soprattutto con frese o laser con kerf largo.",
                 "Adds a circular relief to the material's inner corners (slots and holes) so a square-cornered part seats fully. Mostly useful with wide kerf or router bits.")))

    def ok():
        win.doc.save_undo()
        n = C.relief(_closed_polys_from_selection(win), dia.value(), kind.value(), near.isChecked())
        win.doc.notify()
        win.toast(L(win, f"{n} angoli modificati", f"{n} corners relieved"))
    d.on_ok = ok
    d.finish()


def slot_resize_dialog(win):
    from . import tools as T
    polys = [s for s in win.doc.selected_shapes() if s["type"] in ("poly", "rect", "group")]
    if not polys:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, L(win, "Ridimensiona fessure", "Resize Slots"), win.tr("ok"))
    old = d.row(L(win, "Larghezza attuale (mm)", "Current width (mm)"), dspin(3, 0.1, 100, 0.1, 3))
    new = d.row(L(win, "Nuova larghezza (mm)", "New width (mm)"), dspin(3.2, 0.1, 100, 0.1, 3))
    tol = d.row(L(win, "Tolleranza (mm)", "Tolerance (mm)"), dspin(0.05, 0.001, 2, 0.01, 3))
    d.add(hint(L(win, "Trova le fessure larghe come lo spessore del vecchio materiale e le adatta al nuovo spessore, mantenendole centrate.",
                 "Finds slots matching the old material thickness and resizes them to the new thickness, keeping them centred.")))

    def ok():
        win.doc.save_undo()
        n = sum(T.resize_slots(s, old.value(), new.value(), tol.value()) for s in _closed_polys_from_selection(win))
        win.doc.notify()
        win.toast(L(win, f"{n} fessure ridimensionate", f"{n} slots resized"))
    d.on_ok = ok
    d.finish()


def taper_dialog(win):
    from . import tools as T
    sel = win.doc.selected_shapes()
    if not sel:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, L(win, "Rastrema", "Taper Warp"), win.tr("ok"))
    top = d.row(L(win, "Larghezza in alto %", "Top width %"), dspin(70, 1, 500, 5, 1))
    bottom = d.row(L(win, "Larghezza in basso %", "Bottom width %"), dspin(100, 1, 500, 5, 1))
    d.add(hint(L(win, "Utile per incidere su oggetti conici (bicchieri, tazze): stringe la grafica in alto o in basso.",
                 "Useful for engraving on tapered objects (tumblers, cups): narrows the artwork at the top or bottom.")))

    def ok():
        win.doc.save_undo()
        ids = [s["id"] for s in sel]
        new = {n["id"]: n for n in T.taper(win, M.clone(sel), top.value(), bottom.value())}
        win.doc.shapes = [new.get(s["id"], s) if s["id"] in ids else s for s in win.doc.shapes]
        win.doc.notify()
    d.on_ok = ok
    d.finish()


def tangent_circles(win):
    from . import tools as T
    sel = win.doc.selected_in_order()
    if len(sel) != 3:
        win.toast(L(win, "Seleziona esattamente 3 cerchi", "Select exactly 3 circles"))
        return
    sols = T.apollonius(*[T.circle_of(s) for s in sel])
    if not sols:
        win.toast(L(win, "Nessun cerchio tangente possibile", "No tangent circle exists"))
        return
    layer = sel[0].get("layer", 0)
    circles = [{"id": win.doc.new_id(), "type": "circle", "layer": layer, "cx": x, "cy": y, "rx": r, "ry": r} for x, y, r in sols]
    win.doc.add_shapes(circles)
    win.toast(L(win, f"{len(circles)} cerchi tangenti creati: elimina quelli che non servono", f"{len(circles)} tangent circles created: delete the ones you don't need"))



def convert_to_bitmap_dialog(win):
    if not [x for x in win.doc.selected_shapes() if x["type"] != "image"]:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, L(win, "Converti in bitmap", "Convert to Bitmap"), win.tr("ok"))
    px = d.row(L(win, "Dimensione pixel (mm)", "Pixel size (mm)"), dspin(win.layer_interval(win.doc.active_layer), 0.02, 2, 0.01, 3))
    fill = d.row(L(win, "Contenuto", "Content"), Segmented([(True, L(win, "Forme piene", "Filled shapes")), (False, L(win, "Solo contorni", "Outlines only"))]))
    fill.setValue(True)
    stroke = d.row(L(win, "Spessore contorni (mm)", "Outline width (mm)"), dspin(0.3, 0.02, 10, 0.05, 2))
    keep = d.add(checkbox(L(win, "Mantieni le forme originali", "Keep the original shapes")))
    d.add(hint(L(win, "Utile per incidere una grafica vettoriale come immagine (riempimento a scansione, dithering).",
                 "Useful to engrave vector artwork as an image (raster fill, dithering).")))
    d.on_ok = lambda: win.convert_to_bitmap(px.value(), fill.value(), stroke.value(), keep.isChecked())
    d.finish()


# ====================================================================== keyboard shortcuts
def shortcuts_editor(win):
    from PySide6.QtGui import QKeySequence
    from PySide6.QtWidgets import QKeySequenceEdit, QTableWidget, QAbstractItemView
    d = Dialog(win, L(win, "Scorciatoie da tastiera", "Keyboard Shortcuts"), win.tr("ok"), wide=True)
    d.add(hint(L(win, "Seleziona un comando e premi la nuova combinazione di tasti. Canc la rimuove.",
                 "Select a command and press the new key combination. Delete removes it.")))
    search = d.add(QLineEdit())
    search.setPlaceholderText(L(win, "Cerca comando…", "Search command…"))
    ids = sorted((aid for aid, a in win.actions.items() if a.text() and aid not in ("exit",)), key=lambda a: win.action_label(a).lower())
    table = QTableWidget(len(ids), 2)
    table.horizontalHeader().hide()
    table.verticalHeader().hide()
    table.horizontalHeader().setSectionResizeMode(0, QHeaderView.Stretch)
    table.horizontalHeader().setSectionResizeMode(1, QHeaderView.ResizeToContents)
    table.setSelectionBehavior(QAbstractItemView.SelectRows)
    table.setMinimumHeight(380)
    edits = {}
    for r, aid in enumerate(ids):
        it = QTableWidgetItem(win.action_label(aid).replace("...", ""))
        it.setFlags(it.flags() & ~Qt.ItemIsEditable)
        table.setItem(r, 0, it)
        ed = QKeySequenceEdit(win.actions[aid].shortcut())
        ed.setMaximumSequenceLength(1) if hasattr(ed, "setMaximumSequenceLength") else None
        ed.setClearButtonEnabled(True) if hasattr(ed, "setClearButtonEnabled") else None
        table.setCellWidget(r, 1, ed)
        edits[aid] = ed
    d.lay.insertWidget(d.lay.count(), table, 1)

    def filt(text):
        for r, aid in enumerate(ids):
            table.setRowHidden(r, bool(text) and text.lower() not in win.action_label(aid).lower())
    search.textChanged.connect(filt)
    reset = QPushButton(L(win, "Ripristina predefinite", "Restore defaults"))

    def do_reset():
        win._store_json("shortcuts", {})
        for aid, ed in edits.items():
            ed.setKeySequence(win.default_shortcut(aid))
    reset.clicked.connect(do_reset)
    d.add(reset)

    def ok():
        chosen = {aid: ed.keySequence().toString() for aid, ed in edits.items()}
        used = {}
        for aid, ks in chosen.items():
            if ks:
                if ks in used:
                    win.toast(L(win, f"«{ks}» è usata sia da «{win.action_label(used[ks])}» sia da «{win.action_label(aid)}»",
                                f"“{ks}” is used by both “{win.action_label(used[ks])}” and “{win.action_label(aid)}”"))
                    return False
                used[ks] = aid
        overrides = {aid: ks for aid, ks in chosen.items() if ks != win.default_shortcut(aid).toString()}
        win._store_json("shortcuts", overrides)
        win.apply_shortcuts()
    d.on_ok = ok
    d.resize(640, 620)
    d.finish()



def cylinder_dialog(win):
    c = {"enabled": False, "diameter": 80.0, "axis": "x", "centre": win.workspace()[1] / 2, **win._load_json("cylinder", {})}
    d = Dialog(win, L(win, "Correzione cilindrica", "Cylinder Correction"), win.tr("ok"))
    on = d.add(checkbox(L(win, "Abilita correzione cilindrica", "Enable cylinder correction"), c["enabled"]))
    dia = d.row(L(win, "Diametro oggetto (mm)", "Object diameter (mm)"), dspin(c["diameter"], 1, 1000, 1, 2))
    axis = d.row(L(win, "Asse del cilindro", "Cylinder axis"), Segmented([("x", L(win, "Orizzontale (lungo X)", "Horizontal (along X)")),
                                                                        ("y", L(win, "Verticale (lungo Y)", "Vertical (along Y)"))]))
    axis.setValue(c["axis"])
    centre = d.row(L(win, "Linea più alta del cilindro (mm)", "Cylinder top line (mm)"), dspin(c["centre"], -5000, 5000, 1, 2))
    mirror = d.row(L(win, "Distanza specchio (galvo)", "Mirror distance (galvo)"), dspin(float(c.get("mirror", 0.0)), 0, 2000, 5, 1, " mm"))
    mirror.setSpecialValueText(L(win, "0 — laser a portale", "0 — gantry laser"))
    use_laser = QPushButton(L(win, "Usa la posizione del laser", "Use laser position"))
    use_laser.clicked.connect(lambda: centre.setValue(win.machine_pos[1] if axis.value() == "x" else win.machine_pos[0]))
    d.add(use_laser)
    d.add(hint(L(win, "Per incidere su bicchieri, bottiglie e tubi appoggiati sul piano, senza rotativo: la grafica viene compressa "
                      "verso i bordi in modo che sulla superficie curva mantenga la sua misura. Porta il laser sulla linea più alta "
                      "dell'oggetto e premi 'Usa la posizione del laser'. Oltre un quarto di circonferenza per lato non è incidibile. "
                      "Le immagini (raster) non vengono corrette.",
                 "For engraving tumblers, bottles and tubes lying on the bed, without a rotary: the artwork is compressed toward the "
                 "edges so it keeps its size on the curved surface. Jog to the object's top line and press 'Use laser position'. "
                 "More than a quarter of the circumference per side cannot be reached. Raster images are not corrected.")))

    def ok():
        win._store_json("cylinder", {"enabled": on.isChecked(), "diameter": dia.value(), "axis": axis.value(), "centre": centre.value(),
                                     "mirror": mirror.value()})
        win.invalidate_gcode()
        win.status(L(win, "Correzione cilindrica attiva", "Cylinder correction on") if on.isChecked() else win.tr("ready"))
    d.on_ok = ok
    d.finish()



def close_tolerance_dialog(win):
    if not win.doc.selected:
        win.toast(win.tr("selectFirst"))
        return
    d = Dialog(win, L(win, "Chiudi forme con tolleranza", "Close Shapes with Tolerance"), win.tr("ok"))
    tol = d.row(L(win, "Tolleranza (mm)", "Tolerance (mm)"), dspin(0.5, 0.001, 50, 0.1, 3))
    d.add(hint(L(win, "Unisce le linee aperte le cui estremità distano meno della tolleranza e chiude le forme che finiscono dove iniziano. "
                      "Utile per i file DXF e PDF con contorni spezzati.",
                 "Joins open lines whose ends are closer than the tolerance and closes shapes that end where they start. "
                 "Useful for DXF and PDF files with broken outlines.")))
    d.on_ok = lambda: win.close_with_tolerance(tol.value())
    d.finish()



def barcode_dialog(win):
    from . import devices as DV
    d = Dialog(win, L(win, "Crea codice a barre", "Create Barcode"), win.tr("create"))
    kind = d.row(L(win, "Tipo", "Type"), QComboBox())
    for k, label in (("code128", "Code 128"), ("ean13", "EAN-13"), ("ean8", "EAN-8"), ("upca", "UPC-A"), ("qr", "QR")):
        kind.addItem(label, k)
    data = d.row(L(win, "Dati", "Data"), QLineEdit("LF-0001"))
    module = d.row(L(win, "Larghezza modulo (mm)", "Module width (mm)"), dspin(0.33, 0.1, 3, 0.01, 3))
    height = d.row(L(win, "Altezza barre (mm)", "Bar height (mm)"), dspin(15, 2, 200, 1, 1))
    human = d.add(checkbox(L(win, "Testo leggibile sotto le barre", "Human-readable text under the bars"), True))
    d.add(hint(L(win, "Code 128: testo e numeri. EAN-13/EAN-8/UPC-A: solo cifre, la cifra di controllo si aggiunge da sola. "
                      "Il layer attivo passa a Riempi per incidere le barre. Per i codici QR usa Strumenti › Crea codice QR.",
                 "Code 128: text and digits. EAN-13/EAN-8/UPC-A: digits only, the check digit is added automatically. "
                 "The active layer is switched to Fill to engrave the bars. For QR codes use Tools › Create QR Code.")))

    def ok():
        if kind.currentData() == "qr":
            QTimer.singleShot(0, lambda: qr_dialog(win))
            return
        try:
            mods, text = DV.barcode_modules(kind.currentData(), data.text().strip())
        except ValueError as exc:
            win.toast(str(exc))
            return False
        mw, h = module.value(), height.value()
        quiet = 10 * mw
        W, H = win.workspace()
        total = len(mods) * mw
        x0, y0 = (W - total) / 2, (H - h) / 2
        layer = win.doc.active_layer
        kids = [{"id": win.doc.new_id(), "type": "rect", "layer": layer, "x": x, "y": y, "w": w_, "h": hh}
                for x, y, w_, hh in DV.barcode_rects(mods, x0, y0, mw, h)]
        if human.isChecked():
            size = max(1.5, min(6, h * 0.18))
            t = {"id": win.doc.new_id(), "type": "text", "layer": layer, "x": x0, "y": y0 + h + 0.8, "text": text, "size": size, "font": "Helvetica"}
            tw = M.text_width(t)
            t["x"] = x0 + (total - tw) / 2
            kids.append(t)
        win.doc.add_shapes([win.doc.create("group", children=kids, name=f"{kind.currentText()} {text}")])
        if win.engine.layers[layer].cut_mode.value == "line":
            win.save_layer_settings({"cut_mode": "fill"})
            win.cuts.load(layer)
        win.toast(L(win, f"{kind.currentText()}: {text} · {total + 2 * quiet:.1f} mm con i margini", f"{kind.currentText()}: {text} · {total + 2 * quiet:.1f} mm with quiet zones"))
    d.on_ok = ok
    d.finish()



def nesting_dialog(win):
    d = Dialog(win, L(win, "Disponi automaticamente (nesting)", "Nesting"), win.tr("ok"))
    mode = d.row(L(win, "Metodo", "Method"), Segmented([("shape", L(win, "Forma reale", "True shape")), ("box", L(win, "Rettangoli", "Bounding boxes"))]))
    mode.setValue("shape")
    spacing = d.row(L(win, "Distanza tra i pezzi (mm)", "Part spacing (mm)"), dspin(2, 0, 50, 0.5, 2))
    margin = d.row(L(win, "Margine del foglio (mm)", "Sheet margin (mm)"), dspin(3, 0, 100, 1, 1))
    rot = d.row(L(win, "Rotazioni", "Rotations"), Segmented([("0", L(win, "Nessuna", "None")), ("90", "90°"), ("45", "45°"), ("15", "15°")]))
    rot.setValue("90")
    d.add(hint(L(win, "Usa le forme selezionate (o tutte). Forma reale: i pezzi si incastrano seguendo il contorno, "
                      "i più grandi per primi; con passi di rotazione piccoli è più lento ma più compatto. "
                      "L'area utile è l'area di lavoro meno il margine.",
                 "Uses the selected shapes (or all). True shape: parts interlock following their outline, largest first; "
                 "small rotation steps are slower but tighter. The usable area is the work area minus the margin.")))

    def ok():
        if mode.value() == "box":
            win.nesting()
            return
        step = int(rot.value())
        rotations = (0,) if step == 0 else tuple(range(0, 360, step))
        win.nest_true_shape(spacing.value(), rotations, margin.value())
    d.on_ok = ok
    d.finish()



def import_3d_dialog(win, path):
    import os
    from . import mesh3d
    try:
        tris = mesh3d.load_mesh(path)
    except Exception as exc:
        win.toast(L(win, "Modello 3D non leggibile: ", "Cannot read 3D model: ") + str(exc))
        return
    pts = tris.reshape(-1, 3)
    size = pts.max(axis=0) - pts.min(axis=0)
    d = Dialog(win, L(win, "Importa modello 3D", "Import 3D model"), L(win, "Importa", "Import"))
    d.add(hint(L(win, f"{os.path.basename(path)}: {len(tris)} triangoli, {size[0]:.1f} × {size[1]:.1f} × {size[2]:.1f} (unità del file)",
                 f"{os.path.basename(path)}: {len(tris)} triangles, {size[0]:.1f} × {size[1]:.1f} × {size[2]:.1f} (file units)")))
    up = d.row(L(win, "Asse verticale del modello", "Model up axis"), Segmented([("z", "Z"), ("y", "Y")]))
    up.setValue("y" if path.lower().endswith(".obj") else "z")
    width = d.row(L(win, "Larghezza incisione (mm)", "Engraving width (mm)"), dspin(float(min(max(size[0], 10), 300)), 1, 2000, 1, 1))
    px = d.row(L(win, "Risoluzione (mm/pixel)", "Resolution (mm/pixel)"), dspin(0.1, 0.02, 2, 0.02, 3))
    slices = d.row(L(win, "Strati 3D", "3D slices"), ispin(20, 2, 100))
    d.add(hint(L(win, "Il modello viene visto dall'alto: le parti più alte restano chiare (incise poco), quelle più basse scure (incise di più). "
                      "Ogni strato è una passata a piena potenza del layer: più strati = profondità più graduale.",
                 "The model is viewed from above: higher parts stay light (engraved less), lower parts dark (engraved more). "
                 "Each slice is one full-power pass of the layer: more slices = smoother depth.")))

    def ok():
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            img, wmm, hmm = mesh3d.depth_map(tris.copy(), px_mm=px.value() * max(size[0], 1e-6) / width.value(), up=up.value())
        except Exception as exc:
            win.toast(str(exc))
            return False
        finally:
            QApplication.restoreOverrideCursor()
        k = width.value() / max(wmm, 1e-6)
        W, H = win.workspace()
        w_, h_ = wmm * k, hmm * k
        s = win.doc.create("image", src=M.encode_image(img), name=os.path.basename(path), x=(W - w_) / 2, y=(H - h_) / 2, w=w_, h=h_,
                           adj={**M.default_adj(), "mode": "slices3d", "slices": slices.value()})
        win.doc.add_shapes([s])
    d.on_ok = ok
    d.finish()


# ====================================================================== camera: lens correction & target alignment
class PointPicker(QWidget):
    """Shows an image; clicks add points (in image pixels), drawn numbered."""

    def __init__(self, pil_img, max_points=99, guides=None):
        super().__init__()
        from .extras import pil_to_qimage
        self.pil = pil_img
        self.img = pil_to_qimage(pil_img)
        self.points = []
        self.max_points = max_points
        self.grid = False
        self.setMinimumSize(640, 420)
        self.setCursor(Qt.CrossCursor)

    def set_image(self, pil_img):
        from .extras import pil_to_qimage
        self.pil = pil_img
        self.img = pil_to_qimage(pil_img)
        self.update()

    def _geom(self):
        k = min(self.width() / self.img.width(), self.height() / self.img.height())
        return k, (self.width() - self.img.width() * k) / 2, (self.height() - self.img.height() * k) / 2

    def mousePressEvent(self, ev):
        k, ox, oy = self._geom()
        x, y = (ev.position().x() - ox) / k, (ev.position().y() - oy) / k
        if 0 <= x <= self.img.width() and 0 <= y <= self.img.height() and len(self.points) < self.max_points:
            self.points.append((x, y))
            self.update()

    def paintEvent(self, _ev):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.fillRect(self.rect(), QColor("#111"))
        k, ox, oy = self._geom()
        p.drawImage(QRectF(ox, oy, self.img.width() * k, self.img.height() * k), self.img)
        if self.grid:
            p.setPen(QPen(QColor(0, 200, 255, 120), 1))
            for i in range(1, 10):
                x = ox + self.img.width() * k * i / 10
                y = oy + self.img.height() * k * i / 10
                p.drawLine(QPointF(x, oy), QPointF(x, oy + self.img.height() * k))
                p.drawLine(QPointF(ox, y), QPointF(ox + self.img.width() * k, y))
        p.setPen(QPen(QColor("#ff2a2a"), 2))
        for i, (x, y) in enumerate(self.points):
            q = QPointF(ox + x * k, oy + y * k)
            p.drawEllipse(q, 6, 6)
            p.drawText(q + QPointF(8, -8), str(i + 1))
        p.end()


def lens_correction_dialog(win):
    from .extras import fit_k1, qimage_to_pil, undistort
    frame = win.camera_panel.frame
    if frame is None:
        win.toast(L(win, "Avvia prima la camera (Finestra › Controllo camera)", "Start the camera first (Window › Camera Control)"))
        return
    raw = qimage_to_pil(frame)
    small = raw.copy()
    small.thumbnail((900, 700))
    d = Dialog(win, L(win, "Correzione lente camera", "Camera Lens Correction"), win.tr("ok"), wide=True)
    d.add(hint(L(win, "Muovi i cursori finché i bordi dritti dell'area di lavoro appaiono dritti (la griglia aiuta). "
                      "Oppure clicca almeno 4 punti lungo un bordo che deve essere dritto e premi 'Stima dalla linea'.",
                 "Move the sliders until straight edges of the work area look straight (the grid helps). "
                 "Or click at least 4 points along an edge that should be straight and press 'Estimate from line'.")))
    picker = PointPicker(small)
    picker.grid = True
    d.lay.insertWidget(d.lay.count(), picker, 1)
    k1s, k2s = QSlider(Qt.Horizontal), QSlider(Qt.Horizontal)
    for sl, v in ((k1s, win.camera_cal.get("k1", 0.0)), (k2s, win.camera_cal.get("k2", 0.0))):
        sl.setRange(-600, 600)
        sl.setValue(int(round(float(v) * 1000)))
    d.row("k1", k1s)
    d.row("k2", k2s)
    fit = QPushButton(L(win, "Stima dalla linea", "Estimate from line"))
    clear = QPushButton(L(win, "Azzera punti", "Clear points"))
    h = QHBoxLayout()
    h.addWidget(fit)
    h.addWidget(clear)
    d.lay.insertLayout(d.lay.count(), h)

    def refresh(*_):
        picker.set_image(undistort(small, k1s.value() / 1000, k2s.value() / 1000))

    def do_fit():
        if len(picker.points) < 4:
            win.toast(L(win, "Clicca almeno 4 punti lungo un bordo dritto", "Click at least 4 points along a straight edge"))
            return
        # points were clicked on the corrected preview: map them back to raw coordinates with the current k
        k1s.setValue(int(round(fit_k1(picker.points, small.size) * 1000)))
        picker.points = []
        refresh()
    k1s.valueChanged.connect(refresh)
    k2s.valueChanged.connect(refresh)
    fit.clicked.connect(lambda: (k1s.setValue(0), k2s.setValue(0), refresh(), do_fit()) if picker.points else do_fit())
    clear.clicked.connect(lambda: (setattr(picker, "points", []), picker.update()))
    refresh()

    def ok():
        win.camera_cal["k1"], win.camera_cal["k2"] = k1s.value() / 1000, k2s.value() / 1000
        win._store_json("camera", win.camera_cal)
        win.camera_panel.capture_overlay()
    d.on_ok = ok
    d.resize(960, 760)
    d.finish()


def alignment_targets(workspace, n=3, margin=0.15):
    W, H = workspace
    xs = [W * (margin + (1 - 2 * margin) * i / (n - 1)) for i in range(n)]
    ys = [H * (margin + (1 - 2 * margin) * j / (n - 1)) for j in range(n)]
    return [(x, y) for y in ys for x in xs]


def target_alignment_dialog(win):
    from .extras import qimage_to_pil, undistort
    targets = alignment_targets(win.workspace())
    d = Dialog(win, L(win, "Allinea camera con segni incisi", "Camera alignment with burned targets"), win.tr("ok"), wide=True)
    d.add(hint(L(win, "1) Metti un foglio di cartone sul piano e premi 'Incidi i segni': il laser incide 9 crocette in posizioni note. "
                      "2) Premi 'Cattura' e clicca il centro di ogni crocetta nell'ordine dei numeri (da sinistra a destra, dall'alto in basso).",
                 "1) Put scrap card on the bed and press 'Burn targets': the laser burns 9 crosses at known positions. "
                 "2) Press 'Capture' and click the centre of each cross in number order (left to right, top to bottom).")))
    power = d.row(L(win, "Potenza segni %", "Target power %"), dspin(20, 1, 100, 1, 0))
    speed = d.row(L(win, "Velocità segni (mm/min)", "Target speed (mm/min)"), dspin(1000, 50, 20000, 100, 0))
    picker_box = QVBoxLayout()
    d.lay.insertLayout(d.lay.count(), picker_box, 1)
    state = {"picker": None}
    h = QHBoxLayout()
    burn = QPushButton(L(win, "Incidi i segni", "Burn targets"))
    cap = QPushButton(L(win, "Cattura", "Capture"))
    clear = QPushButton(L(win, "Azzera punti", "Clear points"))
    for b in (burn, cap, clear):
        h.addWidget(b)
    d.lay.insertLayout(d.lay.count(), h)

    def do_burn():
        if not win.connected:
            win.toast(L(win, "Connetti prima il laser", "Connect a laser first"))
            return
        s_val = round(power.value() / 100 * win.engine.machine.max_power)
        g = ["; camera alignment targets", "G21", "G90", "M4 S0"]
        for x, y in targets:
            for (ax, ay, bx, by) in ((x - 4, y, x + 4, y), (x, y - 4, x, y + 4)):
                (ax, ay), (bx, by) = win.to_machine((ax, ay)), win.to_machine((bx, by))
                g += [f"G0 X{ax:.3f} Y{ay:.3f}", f"G1 X{bx:.3f} Y{by:.3f} F{speed.value():.0f} S{s_val}", "S0"]
        g += ["M5"]
        win.engine.send("\n".join(g))

    def do_capture():
        frame = win.camera_panel.frame
        if frame is None:
            win.toast(L(win, "Avvia prima la camera", "Start the camera first"))
            return
        raw = undistort(qimage_to_pil(frame), float(win.camera_cal.get("k1", 0)), float(win.camera_cal.get("k2", 0)))
        if state["picker"] is not None:
            state["picker"].setParent(None)
        state["picker"] = PointPicker(raw, max_points=len(targets))
        picker_box.addWidget(state["picker"])
    burn.clicked.connect(do_burn)
    cap.clicked.connect(do_capture)
    clear.clicked.connect(lambda: state["picker"] and (setattr(state["picker"], "points", []), state["picker"].update()))

    def ok():
        pk = state["picker"]
        if pk is None or len(pk.points) < 4:
            win.toast(L(win, "Clicca almeno 4 segni (meglio tutti e 9)", "Click at least 4 targets (all 9 is best)"))
            return False
        win.camera_cal["points"] = [[list(p), list(t)] for p, t in zip(pk.points, targets)]
        win.camera_cal.pop("corners", None)
        win._store_json("camera", win.camera_cal)
        win.camera_panel.capture_overlay()
    d.on_ok = ok
    d.resize(960, 780)
    d.finish()



def new_device_wizard(win):
    """Guided setup of a new laser: controller, connection, work area; saves a device profile."""
    from PySide6.QtWidgets import QStackedWidget
    from .devices import apply_device, load_devices, save_devices
    from .gcode import FLAVORS
    from .serial_comm import detect_controllers
    d = Dialog(win, L(win, "Nuovo dispositivo", "New device"), L(win, "Crea", "Create"))
    name = d.row(L(win, "Nome", "Name"), QLineEdit(L(win, "Il mio laser", "My laser")))
    flavor = d.row(L(win, "Controller", "Controller"), QComboBox())
    for k, lab in FLAVORS.items():
        flavor.addItem(lab, k)
    conn = d.row(L(win, "Collegamento", "Connection"), Segmented([("usb", "USB"), ("tcp", L(win, "Rete", "Network"))]))
    conn.setValue("usb")
    port = d.row(L(win, "Porta USB", "USB port"), QComboBox())
    for p in win.engine.list_ports():
        port.addItem(f"{p['device']}  {p.get('description', '')}".strip(), p["device"])
    host = d.row(L(win, "Indirizzo di rete", "Network address"), QLineEdit("192.168.0.1:23"))
    detect = QPushButton(L(win, "Rileva automaticamente", "Detect automatically"))
    found_lab = QLabel("")
    d.add(detect)
    d.add(found_lab)
    W = d.row(L(win, "Area di lavoro larghezza (mm)", "Work area width (mm)"), dspin(400, 10, 3000, 10, 0))
    H = d.row(L(win, "Area di lavoro altezza (mm)", "Work area height (mm)"), dspin(400, 10, 3000, 10, 0))
    smax = d.row(L(win, "Valore S massimo", "Max S value"), ispin(1000, 1, 100000))
    baud = d.row("Baud", QComboBox())
    for b in (115200, 250000, 230400, 57600, 9600):
        baud.addItem(str(b), b)
    home = d.add(checkbox(L(win, "La macchina ha i finecorsa (home all'avvio)", "The machine has limit switches (home at start)"), True))
    origin = d.row(L(win, "Origine (0,0 della macchina)", "Origin (machine 0,0)"), origin_selector(win, "fl"))

    def sync():
        usb = conn.value() == "usb"
        port.setEnabled(usb)
        host.setEnabled(not usb)
        smax.setValue({"marlin": 255, "smoothie": 1}.get(flavor.currentData(), smax.value() if smax.value() > 1 else 1000))
    conn.changed.connect(lambda _v: sync())
    flavor.currentIndexChanged.connect(lambda _i: sync())

    def do_detect():
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            res = detect_controllers(baud.currentData())
        finally:
            QApplication.restoreOverrideCursor()
        if not res:
            found_lab.setText(L(win, "Nessun controller trovato sulle porte USB.", "No controller found on the USB ports."))
            return
        dev, fl, banner = res[0]
        i = port.findData(dev)
        if i >= 0:
            port.setCurrentIndex(i)
        flavor.setCurrentIndex(max(0, flavor.findData(fl)))
        found_lab.setText(L(win, f"Trovato: {banner} su {dev}", f"Found: {banner} on {dev}"))
    detect.clicked.connect(do_detect)
    sync()

    def ok():
        machine = win.engine.machine_dict()
        machine.update({"flavor": flavor.currentData(), "workspace_x": W.value(), "workspace_y": H.value(), "max_power": smax.value(),
                        "baud_rate": baud.currentData(), "home_on_start": home.isChecked(), "origin": origin.value()})
        address = port.currentData() if conn.value() == "usb" else "tcp://" + host.text().strip()
        dev = {"name": name.text().strip() or "Laser", "machine": machine, "port": address}
        devices = [x for x in load_devices(win) if x["name"] != dev["name"]] + [dev]
        save_devices(win, devices)
        apply_device(win, dev)
        win.laser_panel.select_port(address)
    d.on_ok = ok
    d.finish()



def feeder_dialog(win):
    m = win.engine.machine
    d = Dialog(win, L(win, "Alimentatore automatico", "Feeder Setup"), win.tr("ok"))
    on = d.add(checkbox(L(win, "Ripeti il lavoro facendo avanzare il materiale", "Repeat the job, advancing the material"), m.feeder_enabled))
    axis = d.row(L(win, "Asse dell'alimentatore", "Feeder axis"), Segmented([("A", "A"), ("Y", "Y"), ("Z", "Z"), ("custom", L(win, "G-code", "G-code"))]))
    axis.setValue(m.feeder_axis)
    dist = d.row(L(win, "Avanzamento (mm)", "Advance (mm)"), dspin(m.feeder_distance, 0.1, 10000, 10, 2))
    reps = d.row(L(win, "Ripetizioni", "Repetitions"), ispin(m.feeder_repeats, 1, 1000))
    speed = d.row(L(win, "Velocità avanzamento", "Feed speed"), dspin(m.feeder_speed, 10, 50000, 100, 0, " mm/min"))
    from PySide6.QtWidgets import QPlainTextEdit
    custom = QPlainTextEdit(m.feeder_gcode)
    custom.setFixedHeight(60)
    custom.setPlaceholderText("M42 P10 S255\nG4 P2")
    d.row(L(win, "G-code personalizzato", "Custom G-code"), custom)
    d.add(hint(L(win, "Per macchine con nastro o rullo trascinatore: dopo ogni lavoro l'asse scelto fa avanzare il materiale e il lavoro "
                      "riparte, per il numero di ripetizioni indicato. Con 'G-code' puoi comandare un alimentatore esterno.",
                 "For machines with a conveyor or feed roller: after each run the chosen axis advances the material and the job runs again, "
                 "for the given number of repetitions. 'G-code' lets you drive an external feeder.")))

    def ok():
        win.engine.update_machine({"feeder_enabled": on.isChecked(), "feeder_axis": axis.value(), "feeder_distance": dist.value(),
                                   "feeder_repeats": reps.value(), "feeder_speed": speed.value(), "feeder_gcode": custom.toPlainText()})
        win._store_json("machine", win.engine.machine_dict())
        win.status(L(win, f"Alimentatore: {reps.value()} ripetizioni", f"Feeder: {reps.value()} repetitions") if on.isChecked() else win.tr("ready"))
    d.on_ok = ok
    d.finish()


def parse_file_listing(lines):
    """[(name, size_or_None)] from FluidNC/grblHAL file listing replies like [FILE: /sd/a.gcode|SIZE:1234]."""
    import re as _re
    out = []
    for l in lines:
        m = _re.search(r"\[FILE:\s*([^|\]]+)(?:\|SIZE:(\d+))?", l)
        if m:
            out.append((m.group(1).strip(), int(m.group(2)) if m.group(2) else None))
    return out


def controller_files_dialog(win):
    """Files on the controller's SD card (FluidNC: $SD/..., grblHAL: $F...)."""
    import time as _t
    from PySide6.QtWidgets import QListWidget
    fl = win.engine.machine.flavor
    if fl not in ("fluidnc", "grblhal"):
        win.toast(L(win, "I file sulla scheda sono disponibili con controller FluidNC o grblHAL (Impostazioni dispositivo › Controller)",
                    "Controller files need a FluidNC or grblHAL controller (Device Settings › Controller)"))
        return
    if not win.connected:
        win.toast(L(win, "Connetti prima il laser", "Connect a laser first"))
        return
    d = Dialog(win, L(win, "File sul controller", "Laser files"), win.tr("close"), cancel=False)
    lst = QListWidget()
    lst.setMinimumHeight(260)
    d.lay.insertWidget(0, lst, 1)

    def listing():
        n0 = len(win.engine.console_lines())
        win.engine.command("$SD/List" if fl == "fluidnc" else "$F")
        end = _t.time() + 2.0
        while _t.time() < end:
            QApplication.processEvents()
            _t.sleep(0.05)
        lst.clear()
        for name, size in parse_file_listing([l["msg"] for l in win.engine.console_lines()[n0:]]):
            lst.addItem(name + (f"   ({size // 1024} KB)" if size else ""))
            lst.item(lst.count() - 1).setData(Qt.UserRole, name)
        if not lst.count():
            lst.addItem(L(win, "(nessun file o risposta non riconosciuta: guarda la Console)", "(no files or unrecognised reply: see the Console)"))

    def selected_name():
        it = lst.currentItem()
        return it.data(Qt.UserRole) if it else None

    def run_file():
        name = selected_name()
        if name:
            win.engine.command(f"$SD/Run={name}" if fl == "fluidnc" else f"$F={name}")
            win.status(L(win, "Avviato dalla scheda: ", "Started from card: ") + name)

    def delete_file():
        name = selected_name()
        if name:
            win.engine.command(f"$SD/Delete={name}" if fl == "fluidnc" else f"$FD={name}")
            listing()
    h = QHBoxLayout()
    for text, fn in ((L(win, "Aggiorna", "Refresh"), listing), (L(win, "Avvia", "Run"), run_file), (L(win, "Elimina", "Delete"), delete_file)):
        b = QPushButton(text)
        b.clicked.connect(fn)
        h.addWidget(b)
    d.lay.insertLayout(1, h)
    d.add(hint(L(win, "Per caricare un file sulla scheda usa l'interfaccia web di FluidNC o la scheda SD: LaserForge salva il G-code con File › Salva G-code.",
                 "To put a file on the card use FluidNC's web interface or the SD card: LaserForge saves G-code with File › Save GCode.")))
    QTimer.singleShot(50, listing)
    d.finish()


def read_grbl_settings(win, done):
    """Ask GRBL for $$ and call done({num: value}) with what came back (empty if not connected)."""
    if not win.connected:
        done({})
        return
    win.engine.command("$$")

    def collect():
        vals = {}
        for ln in win.engine.console_lines()[-300:]:      # newest values win
            m_ = re.match(r"\$(\d+)=(-?[\d.]+)", str(ln.get("msg", "")).strip())
            if m_:
                vals[int(m_.group(1))] = float(m_.group(2))
        done(vals)
    QTimer.singleShot(900, collect)


def calibrate_axis_dialog(win):
    """LightBurn "Calibrate Axis": engrave a known length, measure it, and correct GRBL's steps/mm ($100 / $101)."""
    d = Dialog(win, L(win, "Calibra asse", "Calibrate Axis"), L(win, "Scrivi sul controller", "Write to controller"))
    axis = d.row(L(win, "Asse", "Axis"), QComboBox())
    axis.addItem("X ($100)", 100)
    axis.addItem("Y ($101)", 101)
    steps = d.row(L(win, "Passi/mm attuali", "Current steps/mm"), dspin(80.0, 1, 10000, 0.1, 3))
    want = d.row(L(win, "Lunghezza disegnata (mm)", "Designed length (mm)"), dspin(100.0, 1, 3000, 1, 2))
    got = d.row(L(win, "Lunghezza misurata (mm)", "Measured length (mm)"), dspin(100.0, 1, 3000, 0.1, 2))
    result = d.add(QLabel())
    d.add(hint(L(win, "1) Incidi una linea o un quadrato della lunghezza indicata.  2) Misurala con un calibro.  "
                      "3) Inserisci la misura: il nuovo valore viene scritto in GRBL.",
                 "1) Engrave a line or square of the designed length.  2) Measure it with a caliper.  "
                 "3) Enter the measurement: the new value is written to GRBL.")))
    readb = d.add(QPushButton(L(win, "Leggi valori dal controller", "Read values from controller")))
    cache = {}

    def new_value():
        return steps.value() * want.value() / max(0.001, got.value())

    def refresh(*_):
        result.setText(L(win, f"Nuovo valore: <b>{new_value():.3f}</b> passi/mm", f"New value: <b>{new_value():.3f}</b> steps/mm"))

    def pick_axis(*_):
        if axis.currentData() in cache:
            steps.setValue(cache[axis.currentData()])
        refresh()

    def got_settings(vals):
        if not vals:
            win.toast(L(win, "Nessun valore letto: il laser è connesso?", "No values read: is the laser connected?"))
            return
        cache.update(vals)
        if 120 in vals and 121 in vals:
            win.engine.update_machine({"acceleration": min(vals[120], vals[121])})   # also feeds the time estimate
            win._store_json("machine", win.engine.machine_dict())
        pick_axis()
    readb.clicked.connect(lambda: read_grbl_settings(win, got_settings))
    for w_ in (steps, want, got):
        w_.valueChanged.connect(refresh)
    axis.currentIndexChanged.connect(pick_axis)
    refresh()
    if win.connected:
        read_grbl_settings(win, lambda v: (cache.update(v), pick_axis()) if v else None)

    def ok():
        if not win.connected:
            win.toast(L(win, "Connetti il laser per scrivere il valore", "Connect the laser to write the value"))
            return
        cmd = f"${axis.currentData()}={new_value():.3f}"
        win.engine.command(cmd)
        win.toast(L(win, f"Scritto {cmd}", f"Wrote {cmd}"))
    d.on_ok = ok
    d.finish()


GRBL_SETTINGS = {
    0: ("Impulso passo (µs)", "Step pulse (µs)"), 1: ("Ritardo inattività passo (ms)", "Step idle delay (ms)"),
    2: ("Inverti impulso passo (maschera)", "Step port invert (mask)"), 3: ("Inverti direzione (maschera)", "Direction port invert (mask)"),
    4: ("Inverti abilitazione passi", "Step enable invert"), 5: ("Inverti pin finecorsa", "Limit pins invert"),
    6: ("Inverti pin sonda", "Probe pin invert"), 10: ("Rapporto di stato (maschera)", "Status report (mask)"),
    11: ("Deviazione agli angoli (mm)", "Junction deviation (mm)"), 12: ("Tolleranza archi (mm)", "Arc tolerance (mm)"),
    13: ("Rapporti in pollici", "Report inches"), 20: ("Limiti software", "Soft limits"), 21: ("Limiti hardware (finecorsa)", "Hard limits"),
    22: ("Ciclo di homing", "Homing cycle"), 23: ("Direzione homing (maschera)", "Homing direction invert (mask)"),
    24: ("Velocità homing lenta (mm/min)", "Homing feed (mm/min)"), 25: ("Velocità homing ricerca (mm/min)", "Homing seek (mm/min)"),
    26: ("Antirimbalzo homing (ms)", "Homing debounce (ms)"), 27: ("Distacco dopo homing (mm)", "Homing pull-off (mm)"),
    30: ("Valore S max (potenza 100%)", "Max spindle / S-value"), 31: ("Valore S min", "Min spindle / S-value"),
    32: ("Modalità laser", "Laser mode enable"),
    100: ("Passi/mm X", "X steps/mm"), 101: ("Passi/mm Y", "Y steps/mm"), 102: ("Passi/mm Z", "Z steps/mm"), 103: ("Passi/mm A", "A steps/mm"),
    110: ("Velocità max X (mm/min)", "X max rate (mm/min)"), 111: ("Velocità max Y (mm/min)", "Y max rate (mm/min)"),
    112: ("Velocità max Z (mm/min)", "Z max rate (mm/min)"), 113: ("Velocità max A", "A max rate"),
    120: ("Accelerazione X (mm/s²)", "X acceleration (mm/s²)"), 121: ("Accelerazione Y (mm/s²)", "Y acceleration (mm/s²)"),
    122: ("Accelerazione Z (mm/s²)", "Z acceleration (mm/s²)"), 123: ("Accelerazione A", "A acceleration"),
    130: ("Corsa max X (mm)", "X max travel (mm)"), 131: ("Corsa max Y (mm)", "Y max travel (mm)"),
    132: ("Corsa max Z (mm)", "Z max travel (mm)"), 133: ("Corsa max A", "A max travel"),
}


def machine_settings_dialog(win):
    """LightBurn "Machine Settings" for GRBL: the $$ table with names, Read / Write / Save to File / Load from File."""
    from PySide6.QtWidgets import QTableWidget, QTableWidgetItem, QHeaderView, QFileDialog, QMessageBox
    d = Dialog(win, L(win, "Impostazioni macchina (GRBL)", "Machine Settings (GRBL)"), win.tr("close"), cancel=False, wide=True)
    table = QTableWidget(0, 3)
    table.setHorizontalHeaderLabels(["$", L(win, "Impostazione", "Setting"), L(win, "Valore", "Value")])
    table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Stretch)
    table.verticalHeader().setVisible(False)
    table.setMinimumHeight(420)
    d.add(table)
    note = d.add(QLabel())
    note.setWordWrap(True)
    loaded = {}

    def fill(vals):
        table.setRowCount(0)
        for n in sorted(vals):
            r = table.rowCount()
            table.insertRow(r)
            k = QTableWidgetItem(f"${n}")
            k.setFlags(k.flags() & ~Qt.ItemIsEditable)
            name = GRBL_SETTINGS.get(n)
            dsc = QTableWidgetItem(L(win, *name) if name else "")
            dsc.setFlags(dsc.flags() & ~Qt.ItemIsEditable)
            v = vals[n]
            table.setItem(r, 0, k)
            table.setItem(r, 1, dsc)
            table.setItem(r, 2, QTableWidgetItem(f"{v:g}"))
        win.engine.grbl_settings.update(vals)
        probs = win.engine.grbl_problems()
        note.setText("<br>".join("⚠ " + p for p in probs) if probs else L(win, "Nessun problema nelle impostazioni lette.", "No problems in the settings read."))
        note.setStyleSheet("color:#e0a030" if probs else "color:#7bd88f")

    def current():
        out = {}
        for r in range(table.rowCount()):
            try:
                out[int(table.item(r, 0).text()[1:])] = float(table.item(r, 2).text().replace(",", "."))
            except (ValueError, AttributeError):
                pass
        return out

    def do_read():
        def got(vals):
            if not vals:
                win.toast(L(win, "Nessun valore letto: il laser è connesso?", "No values read: is the laser connected?"))
                return
            loaded.clear()
            loaded.update(vals)
            fill(vals)
        read_grbl_settings(win, got)

    def do_write():
        if not win.connected:
            win.toast(L(win, "Connetti il laser per scrivere le impostazioni", "Connect the laser to write the settings"))
            return
        changes = {n: v for n, v in current().items() if loaded.get(n) is None or abs(loaded[n] - v) > 1e-9}
        if not changes:
            win.toast(L(win, "Nessuna modifica da scrivere", "Nothing changed"))
            return
        txt = "\n".join(f"${n}={v:g}" for n, v in sorted(changes.items()))
        if QMessageBox.question(d, "LaserForge", L(win, "Scrivere queste impostazioni nel controller?\n\n", "Write these settings to the controller?\n\n") + txt) != QMessageBox.Yes:
            return
        for n, v in sorted(changes.items()):
            win.engine.command(f"${n}={v:g}")
        loaded.update(changes)
        win.engine.grbl_settings.update(changes)
        win.toast(L(win, f"Scritte {len(changes)} impostazioni", f"Wrote {len(changes)} settings"))

    def do_save():
        path, _ = QFileDialog.getSaveFileName(d, L(win, "Salva impostazioni", "Save settings"), "grbl-settings.txt", "GRBL (*.txt *.nc)")
        if path:
            with open(path, "w", encoding="utf-8") as f:
                f.write("\n".join(f"${n}={v:g}" for n, v in sorted(current().items())) + "\n")

    def do_load():
        path, _ = QFileDialog.getOpenFileName(d, L(win, "Carica impostazioni", "Load settings"), "", "GRBL (*.txt *.nc);;*")
        if not path:
            return
        vals = {}
        try:
            for ln in open(path, encoding="utf-8"):
                m_ = re.match(r"\s*\$(\d+)\s*=\s*(-?[\d.]+)", ln)
                if m_:
                    vals[int(m_.group(1))] = float(m_.group(2))
        except OSError:
            vals = {}
        if not vals:
            win.toast(L(win, "File non valido", "Invalid file"))
            return
        fill({**current(), **vals})
        win.toast(L(win, "Caricate: premi «Scrivi» per mandarle al controller", "Loaded: press “Write” to send them to the controller"))

    row_ = QWidget()
    h = QHBoxLayout(row_)
    h.setContentsMargins(0, 0, 0, 0)
    for txt, fn in ((L(win, "Leggi", "Read"), do_read), (L(win, "Scrivi", "Write"), do_write),
                    (L(win, "Salva su file", "Save to File"), do_save), (L(win, "Carica da file", "Load from File"), do_load)):
        b = QPushButton(txt)
        b.clicked.connect(fn)
        h.addWidget(b)
    d.add(row_)
    if win.engine.grbl_settings:
        loaded.update(win.engine.grbl_settings)
        fill(dict(win.engine.grbl_settings))
    if win.connected:
        do_read()
    d.finish()


def sublayers_dialog(win):
    """LightBurn sub-layers: extra passes over the same shapes, each with its own mode, speed, power and passes."""
    from PySide6.QtWidgets import QTableWidget, QTableWidgetItem, QHeaderView, QCheckBox
    idx = win.doc.active_layer
    lay = win.engine.layers[idx]
    if lay.tool:
        win.toast(L(win, "I livelli tecnici non hanno sotto-layer", "Tool layers have no sub-layers"))
        return
    d = Dialog(win, L(win, f"Sotto-layer di {lay.name}", f"Sub-layers of {lay.name}"), win.tr("ok"), wide=True)
    d.add(hint(L(win, f"Il layer {lay.name} viene eseguito con le sue impostazioni, poi ogni sotto-layer attivo ripassa le stesse forme con le impostazioni qui sotto.",
                 f"Layer {lay.name} runs with its own settings, then every enabled sub-layer goes over the same shapes with the settings below.")))
    cols = [L(win, "Attivo", "On"), L(win, "Nome", "Name"), L(win, "Modalità", "Mode"), "mm/min",
            L(win, "Potenza %", "Power %"), L(win, "Passate", "Passes")]
    table = QTableWidget(0, len(cols))
    table.setHorizontalHeaderLabels(cols)
    table.horizontalHeader().setSectionResizeMode(1, QHeaderView.Stretch)
    table.verticalHeader().setVisible(False)
    table.setMinimumHeight(200)
    d.add(table)
    modes = [("line", L(win, "Linea", "Line")), ("fill", L(win, "Riempi", "Fill")), ("offset_fill", "Offset"), ("line_fill", L(win, "Linea+Riempi", "Line+Fill"))]

    def add_row(sub):
        r = table.rowCount()
        table.insertRow(r)
        cb = QCheckBox()
        cb.setChecked(bool(sub.get("enabled", True)))
        table.setCellWidget(r, 0, cb)
        table.setItem(r, 1, QTableWidgetItem(str(sub.get("name", f"{lay.name}.{r + 1}"))))
        combo = QComboBox()
        for k, lab in modes:
            combo.addItem(lab, k)
        combo.setCurrentIndex(max(0, combo.findData(sub.get("cut_mode", lay.cut_mode.value))))
        table.setCellWidget(r, 2, combo)
        table.setCellWidget(r, 3, dspin(float(sub.get("speed", lay.speed)), 1, 100000, 10, 0))
        table.setCellWidget(r, 4, dspin(float(sub.get("power_max", lay.power_max)), 0, 100, 1, 1))
        table.setCellWidget(r, 5, ispin(int(sub.get("passes", 1)), 1, 100))

    for sub in lay.sublayers or []:
        if isinstance(sub, dict):
            add_row(sub)
    row_ = QWidget()
    h = QHBoxLayout(row_)
    h.setContentsMargins(0, 0, 0, 0)
    add_b, del_b = QPushButton(L(win, "Aggiungi", "Add")), QPushButton(L(win, "Rimuovi", "Remove"))
    add_b.clicked.connect(lambda: add_row({}))
    del_b.clicked.connect(lambda: table.removeRow(table.currentRow()) if table.currentRow() >= 0 else table.removeRow(table.rowCount() - 1))
    h.addWidget(add_b)
    h.addWidget(del_b)
    h.addStretch(1)
    d.add(row_)

    def ok():
        subs = []
        for r in range(table.rowCount()):
            subs.append({"enabled": table.cellWidget(r, 0).isChecked(), "name": (table.item(r, 1).text() if table.item(r, 1) else "").strip(),
                         "cut_mode": table.cellWidget(r, 2).currentData(), "speed": table.cellWidget(r, 3).value(),
                         "power_max": table.cellWidget(r, 4).value(), "passes": table.cellWidget(r, 5).value()})
        win.save_layer_settings({"sublayers": subs})
        win.cuts.load(idx)
    d.on_ok = ok
    d.finish()


def boolean_assistant(win):
    """LightBurn Boolean Assistant: preview every boolean result of two shapes, click the one you want."""
    from PySide6.QtWidgets import QToolButton
    from PySide6.QtCore import QSize
    from PySide6.QtGui import QIcon, QPainterPath
    sel = [s for s in win.doc.selected_in_order() if s["type"] != "image"]
    if len(sel) != 2:
        win.toast(L(win, "Seleziona esattamente 2 forme vettoriali (la prima è A)", "Select exactly 2 vector shapes (the first is A)"))
        return
    paths = [M.shape_paths(s) for s in sel]
    ops = [("union", L(win, "Unione", "Union")), ("subtract", "A − B"), ("subtract_rev", "B − A"),
           ("intersect", L(win, "Intersezione", "Intersection")), ("xor", L(win, "Esclusione", "Exclusion"))]
    allpts = [p for sp in paths for path in sp for p in path]
    x0, y0 = min(p[0] for p in allpts), min(p[1] for p in allpts)
    x1, y1 = max(p[0] for p in allpts), max(p[1] for p in allpts)
    S = 150

    def thumb(rings):
        pm = QPixmap(S, S)
        pm.fill(QColor("#ffffff"))
        p = QPainter(pm)
        p.setRenderHint(QPainter.Antialiasing, True)
        k = (S - 20) / max(x1 - x0, y1 - y0, 1e-6)
        ox, oy = (S - (x1 - x0) * k) / 2, (S - (y1 - y0) * k) / 2
        tr = lambda pt: QPointF(ox + (pt[0] - x0) * k, oy + (pt[1] - y0) * k)
        for sp in paths:
            for path in sp:
                p.setPen(QPen(QColor("#c8c8c8"), 1, Qt.DashLine))
                for a, b in zip(path, path[1:]):
                    p.drawLine(tr(a), tr(b))
        qp = QPainterPath()
        qp.setFillRule(Qt.OddEvenFill)
        for res in rings:
            for r in res:
                if len(r) > 2:
                    qp.moveTo(tr(r[0]))
                    for pt in r[1:]:
                        qp.lineTo(tr(pt))
                    qp.closeSubpath()
        p.setBrush(QColor(47, 123, 245, 110))
        p.setPen(QPen(QColor("#2f7bf5"), 1.6))
        p.drawPath(qp)
        p.end()
        return pm

    d = Dialog(win, L(win, "Assistente booleano", "Boolean Assistant"), None, wide=True)
    d.add(hint(L(win, "Clicca il risultato che vuoi: le due forme vengono sostituite.", "Click the result you want: the two shapes are replaced.")))
    grid = QWidget()
    h = QHBoxLayout(grid)
    for op, label in ops:
        try:
            res = win.engine.geometry(op, paths, {})
        except Exception:
            res = []
        b = QToolButton()
        b.setToolButtonStyle(Qt.ToolButtonTextUnderIcon)
        b.setIcon(QIcon(thumb(res)))
        b.setIconSize(QSize(S, S))
        b.setText(label)
        b.setEnabled(bool(res))

        def pick(_=False, o=op):
            d.accept()
            win.geometry_op(o, {}, minimum=2)
        b.clicked.connect(pick)
        h.addWidget(b)
    d.add(grid)
    d.finish()
