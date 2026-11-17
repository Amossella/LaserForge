"""LaserForge self-test: drives the real Qt window offscreen and checks every feature.

Run:  QT_QPA_PLATFORM=offscreen python3 tests/selftest.py
All settings, libraries and backups go to temporary folders, never to your real data.
"""
import io
import math
import os
import sys
import tempfile
import time
import traceback

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
TMP = tempfile.mkdtemp(prefix="lf-test-")
os.environ.update({
    "LASERFORGE_SETTINGS": os.path.join(TMP, "settings.ini"),
    "LASERFORGE_ART_DIR": os.path.join(TMP, "art"),
    "LASERFORGE_BACKUP_DIR": os.path.join(TMP, "backups"),
    "LASERFORGE_MATERIALS": os.path.join(TMP, "materials.json"),
})
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PIL import Image, ImageDraw  # noqa: E402
from PySide6.QtCore import QEvent, QPointF, QSizeF, QRectF, Qt, QTimer  # noqa: E402
from PySide6.QtGui import QKeyEvent, QMouseEvent, QPageSize, QPainter, QPdfWriter  # noqa: E402
from PySide6.QtWidgets import QApplication, QDialog, QFileDialog, QInputDialog, QLineEdit  # noqa: E402

app = QApplication([])
from laserforge import dialogs as D  # noqa: E402
from laserforge import model as M  # noqa: E402
from laserforge.mainwindow import MainWindow  # noqa: E402

results, fails = [], []


def check(name, cond, info=""):
    results.append((name, bool(cond), info))
    if not cond:
        fails.append(name)


def auto_accept(setup=None):
    def go():
        from PySide6.QtWidgets import QMessageBox as _MB
        dlg = next((w for w in app.topLevelWidgets() if isinstance(w, QDialog) and not isinstance(w, _MB) and w.isVisible()), None)
        if dlg is None:
            QTimer.singleShot(30, go)
            return
        if setup:
            setup(dlg)
        dlg._accept() if hasattr(dlg, "_accept") else dlg.accept()
    QTimer.singleShot(30, go)


def png_bytes(img):
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()


try:
    w = MainWindow()
    w.resize(1400, 900)
    w.show()
    app.processEvents()
    doc, c = w.doc, w.canvas
    w.engine.update_machine({"origin": "rl"})      # identity mapping for the generic checks; origin tests below

    def mouse(kind, wx, wy, mods=Qt.NoModifier):
        sx, sy = c.to_screen(wx, wy)
        t = {"press": QEvent.MouseButtonPress, "move": QEvent.MouseMove, "release": QEvent.MouseButtonRelease}[kind]
        ev = QMouseEvent(t, QPointF(sx, sy), QPointF(sx, sy), Qt.LeftButton, Qt.LeftButton if kind != "release" else Qt.NoButton, mods)
        {"press": c.mousePressEvent, "move": c.mouseMoveEvent, "release": c.mouseReleaseEvent}[kind](ev)

    def key(ch):
        c.keyPressEvent(QKeyEvent(QEvent.KeyPress, ord(ch.upper()), Qt.NoModifier, ch))

    # ------------------------------------------------------------ basics
    titles = [m.text() for m in w.menuBar().actions()]
    check("menus", titles == ["File", "Modifica", "Strumenti", "Disponi", "Strumenti laser", "Strumenti web", "Finestra", "Lingua", "Aiuto"], titles)
    w.prefs["obj_snap"] = False
    w.set_tool("rect"); mouse("press", 50, 50); mouse("move", 150, 110); mouse("release", 150, 110)
    check("draw rect", doc.shapes and doc.shapes[-1]["type"] == "rect" and abs(doc.shapes[-1]["w"] - 100) < 1e-6)
    w.set_tool("ellipse"); mouse("press", 90, 50); mouse("move", 150, 110); mouse("release", 150, 110)
    w.set_tool("select")
    doc.set_selection([s["id"] for s in doc.shapes]); w.run("weld")
    check("weld", len(doc.shapes) == 1 and doc.shapes[0]["type"] == "poly")
    w.run("undo"); check("undo", len(doc.shapes) == 2)
    w.run("redo"); check("redo", len(doc.shapes) == 1)

    doc.reset()
    big, small = doc.create("rect", x=100, y=100, w=100, h=100), doc.create("rect", x=10, y=10, w=20, h=20)
    doc.add_shapes([big, small]); doc.set_selection([small["id"]])
    mouse("press", 20, 20); mouse("move", 148.3, 148.6); mouse("release", 148.3, 148.6)
    b = M.bounds(doc.find(small["id"]))
    check("red-dot centring snap", abs(b[0] - 140) < 1e-6 and abs(b[1] - 140) < 1e-6, b)
    doc.set_selection([]); mouse("press", 0, 0); mouse("move", 390, 390); mouse("release", 390, 390)
    check("marquee", len(doc.selected) == 2)
    w.run("group"); check("group", len(doc.shapes) == 1)
    w.run("ungroup"); check("ungroup", len(doc.shapes) == 2)

    # ------------------------------------------------------------ text, QR, images, G-code
    doc.reset()
    t = doc.create("text", x=20, y=20, text="Laser", size=20, font="Helvetica")
    doc.add_shapes([t])
    check("text outlines", len(M.text_paths(t)) >= 5)
    auto_accept(lambda dlg: dlg.findChild(QLineEdit).setText("https://example.com"))
    w.run("createQR")
    check("QR code", doc.shapes[-1]["type"] == "group" and len(doc.shapes[-1]["children"]) > 50)
    img = Image.new("RGB", (200, 120), "white"); ImageDraw.Draw(img).ellipse((40, 20, 160, 100), fill="black")
    w.add_image_bytes(png_bytes(img), "dot.png")
    check("image import", doc.shapes[-1]["type"] == "image" and abs(doc.shapes[-1]["w"] - 20) < 1e-6)
    d = w.generate_gcode()
    check("G-code with air assist and raster", d and "M8" in d["gcode"] and "M7" not in d["gcode"] and "; Image" in d["gcode"], d["lines"] if d else None)
    check("preview parser", len(D.parse_gcode(d["gcode"])) > 50)

    # ------------------------------------------------------------ nodes and shape tools
    doc.reset()
    r1 = doc.create("rect", x=100, y=100, w=60, h=40)
    doc.add_shapes([r1]); doc.set_selection([r1["id"]])
    w.set_tool("nodes"); mouse("press", 130, 100); mouse("release", 130, 100)
    check("nodes: rect to path", doc.find(r1["id"])["type"] == "poly")
    mouse("move", 130, 100); key("I")
    check("nodes: insert", len(doc.find(r1["id"])["points"]) == 5)
    w.canvas.handlers["nodes"].sel.clear(); mouse("move", 100, 120); key("S")
    check("nodes: line to curve", M.has_curves(doc.find(r1["id"])))
    w.set_tool("select")
    from laserforge import corners as C
    from laserforge import tools as T
    sq = M.to_poly({"id": 0, "type": "rect", "x": 0, "y": 0, "w": 20, "h": 10})
    check("fillet", C.fillet(sq, 2) == 4)
    check("tangent circles (Apollonius)", len(T.apollonius((0, 0, 1), (10, 0, 1), (5, 8, 1))) == 8)

    # ------------------------------------------------------------ settings and framing
    doc.reset()
    rs = doc.create("rect", x=25.4, y=50.8, w=50.8, h=25.4)
    doc.add_shapes([rs]); doc.set_selection([rs["id"]])

    def set_prefs(dlg):
        dlg.units.setValue("in"); dlg.speed.setValue("mm/s"); dlg.frame_on.setChecked(True); dlg.frame_power.setValue(1.5)
    auto_accept(set_prefs)
    w.run("settings")
    check("settings: inches", abs(w.tx["x"].value() - 1.0) < 1e-4)
    check("settings: laser on when framing", w.prefs["frame_laser_on"] and w.prefs["frame_power"] == 1.5)
    fr_on = w.engine.frame((0, 0, 10, 10), True, 1.5, 2000)
    fr_off = w.engine.frame((0, 0, 10, 10), False, 1.5, 2000)
    check("frame: laser on uses low power", "M4 S0" in fr_on and "S15" in fr_on, fr_on.replace("\n", " | "))
    check("frame: beam off by default", "M5" in fr_off and "S15" not in fr_off and fr_off.count(" S0") >= 4)
    from laserforge import prefs as P
    w.prefs.update(P.DEFAULT_PREFS); w.prefs["obj_snap"] = False; w.apply_prefs()

    # ------------------------------------------------------------ optimisation (group 2)
    doc.reset()
    a1 = doc.create("rect", x=10, y=10, w=20, h=20, layer=0)
    a2 = doc.create("rect", x=200, y=10, w=20, h=20, layer=1)
    b1 = doc.create("rect", x=60, y=10, w=20, h=20, layer=0)
    b2 = doc.create("rect", x=250, y=10, w=20, h=20, layer=1)
    ga = doc.create("group", children=[a1, a2]); gb = doc.create("group", children=[b1, b2])
    doc.add_shapes([ga, gb])
    for i in (0, 1):
        w.engine.update_layer(i, {"enabled": True, "cut_mode": "line"})
    w.opt.update(order="layer"); job = w.build_job()
    check("order by layer", [l["layer_index"] for l in job["layers"]] == [0, 1])
    w.opt.update(order="group"); job = w.build_job()
    check("order by group", [l["layer_index"] for l in job["layers"]] == [0, 1, 0, 1])
    gb["priority"] = -5
    w.opt.update(order="priority"); job = w.build_job()
    firstx = job["layers"][0]["paths"][0][0][0]
    check("order by priority (lower first)", firstx >= 60 - 1e-6 and job["layers"][0]["layer_index"] == 0, firstx)
    w.opt.update(order="layer", direction="ccw"); job = w.build_job()
    pth = job["layers"][0]["paths"][0]
    check("closed shape direction", w._signed_area(pth) < 0)
    w.opt.update(direction="keep")
    doc.reset()
    l1 = doc.create("poly", points=[{"x": 0, "y": 0}, {"x": 50, "y": 0}], closed=False)
    l2 = doc.create("poly", points=[{"x": 20, "y": 0}, {"x": 80, "y": 0}], closed=False)
    doc.add_shapes([l1, l2])
    w.opt.update(removeOverlap=True); job = w.build_job()
    total = sum(math.hypot(p[i][0] - p[i - 1][0], p[i][1] - p[i - 1][1]) for p in job["layers"][0]["paths"] for i in range(1, len(p)))
    check("remove overlapping lines", abs(total - 80) < 1e-6, total)
    w.opt.update(removeOverlap=False)
    doc.reset()
    ring = doc.create("circle", cx=100, cy=100, rx=30, ry=30)
    doc.add_shapes([ring])
    w.opt.update(bestStart=True); job = w.build_job()
    st = job["layers"][0]["paths"][0][0]
    check("best starting point (nearest to origin)", math.hypot(st[0], st[1]) < math.hypot(130, 100) - 1, st)

    # ------------------------------------------------------------ cleanup tools
    doc.reset()
    o1 = doc.create("poly", points=[{"x": 0, "y": 0}, {"x": 10, "y": 0}, {"x": 10, "y": 10}], closed=False)
    o2 = doc.create("poly", points=[{"x": 10.2, "y": 10.1}, {"x": 0, "y": 10}, {"x": 0.1, "y": 0.2}], closed=False)
    doc.add_shapes([o1, o2]); doc.set_selection([o1["id"], o2["id"]])
    w.close_with_tolerance(0.5)
    check("close with tolerance", len(doc.shapes) == 1 and doc.shapes[0]["closed"])
    doc.reset()
    doc.add_shapes([doc.create("poly", points=[{"x": 0, "y": 0}, {"x": 50, "y": 0}], closed=False),
                    doc.create("poly", points=[{"x": 30, "y": 0}, {"x": 90, "y": 0}], closed=False)])
    doc.set_selection([s["id"] for s in doc.shapes]); w.delete_overlapping()
    check("delete overlapping lines", len(doc.shapes) == 1 and abs(M.bounds(doc.shapes[0])[2] - 90) < 1e-6)
    doc.reset()
    outer = doc.create("rect", x=0, y=0, w=100, h=100); inner = doc.create("circle", cx=50, cy=50, rx=10, ry=10)
    c2 = doc.create("circle", cx=200, cy=50, rx=10, ry=10); c3 = doc.create("circle", cx=250, cy=50, rx=4, ry=4)
    doc.add_shapes([outer, inner, c2, c3])
    w.select_nested(); check("select nested shapes", doc.selected == [inner["id"]])
    doc.set_selection([inner["id"]]); w.select_same_circles()
    check("select same-size circles", set(doc.selected) == {inner["id"], c2["id"]})
    doc.set_selection([inner["id"]]); w.select_smaller()
    check("select smaller than selection", doc.selected == [c3["id"]])

    # ------------------------------------------------------------ cylinder correction
    w._store_json("cylinder", {"enabled": True, "diameter": 100, "axis": "x", "centre": 100})
    doc.reset()
    doc.add_shapes([doc.create("poly", points=[{"x": 10, "y": 100}, {"x": 10, "y": 150}], closed=False)])
    job = w.build_job()
    y_end = max(p[1] for p in job["layers"][0]["paths"][0])
    check("cylinder correction", abs(y_end - (100 + 50 * math.sin(1.0))) < 1e-6, y_end)
    w._store_json("cylinder", {"enabled": False})

    # ------------------------------------------------------------ devices, checklist, notes
    from laserforge import devices as DV
    w.engine.update_machine({"workspace_x": 300, "workspace_y": 200})
    QInputDialog.getText = staticmethod(lambda *a, **k: ("Diodo 10W", True))
    DV.save_devices(w, [{"name": "Diodo 10W", "machine": w.engine.machine_dict()}])
    w.engine.update_machine({"workspace_x": 600, "workspace_y": 400})
    DV.apply_device(w, DV.load_devices(w)[0])
    check("device profile switch", w.engine.machine.workspace_x == 300 and w.laser_panel.device.currentText() == "Diodo 10W")
    w.prefs["checklist_enabled"] = True
    w._store_json("checklist", ["Aria aperta", "Occhiali"])

    def tick(dlg):
        from PySide6.QtWidgets import QCheckBox
        for cb in dlg.findChildren(QCheckBox):
            cb.setChecked(True)
    auto_accept(tick)
    check("checklist (all ticked) allows start", DV.run_checklist(w))
    w.prefs["checklist_enabled"] = False
    w.project_notes = {"text": "Betulla 3 mm, 300 mm/min", "show_on_load": True}
    p_ = os.path.join(TMP, "notes.lfproj"); w.project_path = p_; w.save_project(False)
    w.project_notes = {}
    QTimer.singleShot(400, lambda: [x.accept() for x in app.topLevelWidgets() if x.isVisible() and x.inherits("QMessageBox")])
    w.open_path(p_, confirmed=True)
    check("project notes saved and loaded", w.project_notes.get("text", "").startswith("Betulla"))
    _end = time.time() + 0.8
    while time.time() < _end:                       # let the delayed notes pop-up open and close here
        app.processEvents(); time.sleep(0.02)

    # ------------------------------------------------------------ barcodes
    check("EAN-13 check digit", DV.ean_check_digit("400638133393") == "1")
    check("Code 128 width", len(DV.code128_modules("LF-0042")) == 112)
    doc.reset()

    def bc(dlg):
        from PySide6.QtWidgets import QComboBox
        cb = dlg.findChild(QComboBox); cb.setCurrentIndex(cb.findData("ean13"))
        dlg.findChild(QLineEdit).setText("400638133393")
    auto_accept(bc)
    w.run("createBarcode")
    g = doc.shapes[-1]
    check("barcode group with text", g["type"] == "group" and any(k["type"] == "text" and k["text"] == "4006381333931" for k in g["children"]))

    # ------------------------------------------------------------ group 1 regressions
    lib = w.library_panel
    n0 = len(lib.entries)
    lib._ask = lambda title, e: {"material": "Betulla", "thickness": "4mm", "name": "Prova"}
    lib.new_from_layer()
    check("material library: new from layer", len(lib.entries) == n0 + 1)
    pdf = os.path.join(TMP, "t.pdf")
    pw = QPdfWriter(pdf); pw.setPageSize(QPageSize(QSizeF(100, 100), QPageSize.Millimeter)); pw.setResolution(72)
    pa = QPainter(pw); pa.drawRect(QRectF(20, 20, 56.7, 28.35)); pa.end()
    nb = len(doc.shapes); w.open_dropped(pdf)
    check("PDF import", len(doc.shapes) == nb + 1)
    dxf = os.path.join(TMP, "t.dxf")
    open(dxf, "w").write("\n".join(str(x) for x in [0, "SECTION", 2, "ENTITIES", 0, "CIRCLE", 10, 50, 20, 50, 40, 10, 0, "ENDSEC", 0, "EOF"]))
    nb = len(doc.shapes); w.open_dropped(dxf)
    check("DXF import", len(doc.shapes) == nb + 1)
    w.engine.connect("SIM"); w._poll()
    w.rubber_band_frame()
    check("rubber-band frame", "Rubber-band" in w._last_frame)
    w.engine.disconnect(); w._poll()


    # ------------------------------------------------------------ everything else (round 3)
    import json as _json
    import struct as _struct
    import urllib.request as _ur
    import zipfile as _zip
    from shapely.geometry import Polygon as _Poly
    # true-shape nesting: two L-shapes fit an 85x85 sheet only by interlocking
    doc.reset()
    w.engine.update_machine({"workspace_x": 85, "workspace_y": 85})
    Lp = [(0, 0), (60, 0), (60, 20), (20, 20), (20, 60), (0, 60)]
    doc.add_shapes([doc.create("poly", points=[{"x": x + 200, "y": y} for x, y in Lp], closed=True) for _ in range(2)])
    placed, unplaced = w.nest_true_shape(1, (0, 90, 180, 270), 0)
    pg = [_Poly(M.shape_paths(x)[0]) for x in doc.shapes]
    check("true-shape nesting (interlocking)", placed == 2 and pg[0].intersection(pg[1]).area < 1e-6)
    w.engine.update_machine({"workspace_x": 400, "workspace_y": 400})
    # live crop
    doc.reset()
    w.add_image_bytes(png_bytes(Image.new("RGB", (100, 100), "black")), "nero.png")
    im = doc.shapes[-1]
    circ = doc.create("circle", cx=im["x"] + 5, cy=im["y"] + 5, rx=3, ry=3)
    doc.add_shapes([circ]); doc.set_selection([im["id"], circ["id"]]); w.crop_image()
    res = M.process_image(doc.find(im["id"]), 0.1)
    check("live crop masks the image", im.get("crop") and 0.2 < res[0].mean() / 255 < 0.4, round(res[0].mean() / 255, 3))
    doc.set_selection([im["id"]]); w.flatten_crop()
    check("flatten crop", "crop" not in doc.find(im["id"]) and abs(doc.find(im["id"])["w"] - 6) < 0.3)
    # 3D slices
    grad = Image.fromarray(__import__("numpy").tile(__import__("numpy").linspace(0, 255, 100, dtype="uint8"), (40, 1)), "L").convert("RGB")
    doc.reset(); w.add_image_bytes(png_bytes(grad), "depth.png")
    doc.shapes[-1]["adj"] = {**M.default_adj(), "mode": "slices3d", "slices": 5}
    job = w.build_job()
    check("3D sliced engraving (one raster per slice)", len(job["rasters"]) == 5 and job["rasters"][0]["slice"] == 1)
    # ------------------------------------------------------------ image engraving, LightBurn style
    import numpy as _np
    from laserforge.gcode import GCodeGenerator as _GG, LayerSettings as _LS, MachineSettings as _MS
    logo = Image.new("RGBA", (100, 100), (0, 0, 0, 0)); ImageDraw.Draw(logo).ellipse((25, 25, 75, 75), fill=(255, 255, 255, 255))
    doc.reset(); w.add_image_bytes(png_bytes(logo), "logo.png"); lg = doc.shapes[-1]
    lg["adj"] = {**M.default_adj(), "mode": "threshold", "invert": True}
    b_ = M.process_image(lg, 0.1)[0]
    check("negative image never burns transparent pixels", b_[0, 0] == 0 and b_[b_.shape[0] // 2, b_.shape[1] // 2] == 255,
          (int(b_[0, 0]), int(b_.max())))
    big = {**lg, "w": 300.0, "h": 10.0, "adj": M.default_adj()}
    check("large image keeps the layer interval (no 2000 px cap)", M.process_image(big, 0.1)[1] == 3000)
    gr = {**lg, "src": M.encode_image(grad), "w": 10.0, "h": 4.0, "adj": {**M.default_adj(), "mode": "gray"}}
    g1 = M.process_image(gr, 0.1)[0].mean(); g2 = M.process_image({**gr, "adj": {**gr["adj"], "gamma": 2.0}}, 0.1)[0].mean()
    check("gamma lightens mid-tones", g2 < g1 - 5, (round(g1), round(g2)))
    sh = M.process_image({**gr, "adj": {**gr["adj"], "enhance_amount": 200, "enhance_radius": 0.3}}, 0.1)
    check("enhance (sharpen) runs", sh is not None and sh[0].shape == (40, 100))
    pt = M.process_image({**gr, "w": 50.0}, 0.1, pass_through=True)
    check("pass-through keeps the bitmap's own pixels", pt[1] == 100 and pt[2] == 40)
    gm = _MS(origin="rl", raster_relative=False); gen_ = _GG(gm)       # absolute output to check positions; G91 checked below
    ras = {"x": 0, "y": 0, "w": 10, "h": 1, "cols": 10, "rows": 1, "data": [0, 255, 255, 255, 0, 0, 0, 0, 255, 0]}
    L0 = _LS(speed=1000, overscan=0, bidir_scan=False)
    g0 = gen_.generate_raster(L0, ras)
    check("raster merges equal pixels and skips repeated F", g0.count("F1000") == 1 and "G1 X4.000 F1000 S1000" in g0 and "G1 X8.000 S0" in g0 and "X4.000 S0" not in g0, g0)
    gd = gen_.generate_raster(_LS(speed=1000, overscan=0, dot_width=0.2), ras)
    check("dot width correction shortens dots", "X1.100 F1000 S0" in gd and "X3.900 S1000" in gd, gd)
    gm.fast_whitespace, gm.whitespace_speed = True, 5000
    gw = gen_.generate_raster(L0, ras)
    check("fast whitespace scan speeds up blank gaps", "F5000 S0" in gw and gw.count("F1000") == 2, gw)
    gm.fast_whitespace = False
    gm.scan_offset_enabled, gm.scan_offsets = True, "500:0.1\n1500:0.3"
    check("scanning offset is interpolated by speed", abs(gen_.scan_offset(1000) - 0.2) < 1e-9)
    ras2 = {**ras, "rows": 2, "h": 2, "data": ras["data"] * 2}
    go = gen_.generate_raster(_LS(speed=1000, overscan=0), ras2)
    check("scanning offset shifts each line against its direction", "X0.800" in go and "X9.200" in go, go)
    gm.scan_offset_enabled = False
    ga = gen_.generate_raster(_LS(speed=1000, overscan=0, scan_angle=45), {**ras, "rows": 10, "h": 10, "data": [255] * 100})
    g1_ = [l_ for l_ in ga.splitlines() if l_.startswith("G1") and "S1000" in l_]
    check("scan angle rotates image lines", g1_ and all("X" in l_ and "Y" in l_ for l_ in g1_), ga[:300])
    order_rl = gen_.generate_raster(_LS(speed=1000, overscan=0, bidir_scan=False), ras2 | {"data": [255] * 10 + [0] * 9 + [255]})
    check("top-down by default (rear origin: row 0 first)", order_rl.index("Y0.500") < order_rl.index("Y1.500"))

    from laserforge.gcode import estimate_gcode_time as _est, apply_backlash as _bl
    check("time estimate includes acceleration", abs(_est("G1 X100 F600", _MS(acceleration=10)) - 11.0) < 0.05,
          _est("G1 X100 F600", _MS(acceleration=10)))
    check("corners slow the estimate down", _est("G1 X10 F6000\nG1 X0", _MS(acceleration=500)) > _est("G1 X10 F6000\nG1 X20", _MS(acceleration=500)))
    check("backlash compensation on reversal", _bl("G0 X10\nG1 X5 F100\nG1 X8", 0.2, 0) == "G0 X10.000\nG1 X4.800 F100\nG1 X8.000")
    gfin = _GG(_MS(finish_position="custom", finish_x=5, finish_y=7, home_on_start=False)).generate_job([], [])
    check("custom finish position", "G0 X5.000 Y7.000 ; finish position" in gfin)

    # images as G91 deltas (LightBurn): same path as absolute, shorter G-code, preview and resume still right
    gm.raster_relative = True
    big_ = {"x": 3, "y": 4, "w": 20, "h": 3, "cols": 20, "rows": 3, "data": ([0, 255, 128, 128, 0] * 4) * 3}
    g_rel = gen_.generate_raster(_LS(speed=1000, overscan=1), big_)
    gm.raster_relative = False
    g_abs = gen_.generate_raster(_LS(speed=1000, overscan=1), big_)
    seg_r = [tuple(round(v, 3) for v in sg[:4]) for sg in D.parse_gcode(g_rel) if not sg[4]]
    seg_a = [tuple(round(v, 3) for v in sg[:4]) for sg in D.parse_gcode(g_abs) if not sg[4]]
    check("images in G91 trace exactly the same path", "G91" in g_rel and seg_r == seg_a and len(g_rel) < len(g_abs), (len(g_rel), len(g_abs)))
    from laserforge.gcode import resume_from as _rf
    lines_r = g_rel.splitlines(); k_ = next(i for i, l_ in enumerate(lines_r) if "S" in l_ and l_.startswith("G1") and i > 10)
    lines_a = g_abs.splitlines()
    br_ = [sg for sg in D.parse_gcode(g_rel) if not sg[4]]; ba_ = [sg for sg in D.parse_gcode(g_abs) if not sg[4]]
    j_ = len(br_) // 2                                   # the same burned move in both versions
    rr_ = _rf(g_rel, br_[j_][6]); ra_ = _rf(g_abs, ba_[j_][6])
    import re as _re
    pos_r = _re.search(r"G0 X(-?[\d.]+) Y(-?[\d.]+)", rr_); pos_a = _re.search(r"G0 X(-?[\d.]+) Y(-?[\d.]+)", ra_)
    check("resume from a line inside a G91 image starts at the right spot", "G91" in rr_ and pos_r and pos_a
          and abs(float(pos_r.group(1)) - float(pos_a.group(1))) < 0.002 and abs(float(pos_r.group(2)) - float(pos_a.group(2))) < 0.002,
          (pos_r and pos_r.group(0), pos_a and pos_a.group(0)))
    from laserforge.gcode import apply_backlash as _ab2
    bl_r = [tuple(round(v, 2) for v in sg[:4]) for sg in D.parse_gcode(_ab2(g_rel, 0.2, 0)) if not sg[4]]
    bl_a = [tuple(round(v, 2) for v in sg[:4]) for sg in D.parse_gcode(_ab2(g_abs, 0.2, 0)) if not sg[4]]
    check("backlash compensation also works on G91 images", bl_r == bl_a, (bl_r[:3], bl_a[:3]))

    # 3D model import
    stl = os.path.join(TMP, "p.stl")
    V = {"a": (0, 0, 0), "b": (40, 0, 0), "c": (40, 40, 0), "d": (0, 40, 0), "e": (20, 20, 10)}
    F = [("a", "b", "e"), ("b", "c", "e"), ("c", "d", "e"), ("d", "a", "e"), ("a", "c", "b"), ("a", "d", "c")]
    with open(stl, "wb") as f:
        f.write(b"\0" * 80 + _struct.pack("<I", len(F)))
        for t3 in F:
            f.write(_struct.pack("<3f", 0, 0, 0) + b"".join(_struct.pack("<3f", *V[k]) for k in t3) + b"\0\0")
    from laserforge import mesh3d
    img3, w3, h3 = mesh3d.depth_map(mesh3d.load_mesh(stl), 0.5)
    a3 = __import__("numpy").asarray(img3.convert("L"))
    check("3D model -> depth map", a3[a3.shape[0] // 2, a3.shape[1] // 2] > 200 and a3[a3.shape[0] // 2, 2] < 60)
    # Gerber / Excellon
    from laserforge import pcb
    gbr = "%FSLAX26Y26*%\n%MOMM*%\n%ADD10C,0.5*%\nD10*\nX0Y0D02*\nX10000000Y0D01*\nM02*\n"
    gpath = os.path.join(TMP, "top.gtl"); open(gpath, "w").write(gbr)
    nb = len(doc.shapes); w.open_dropped(gpath)
    check("Gerber import", len(doc.shapes) == nb + 1 and abs(M.bounds(doc.shapes[-1])[2] - 10.5) < 0.05, M.bounds(doc.shapes[-1]))
    check("Excellon holes", len(pcb.excellon_geometry("M48\nMETRIC,LZ\nT1C0.800\n%\nT1\nX001000Y002000\nX003000Y002000\nM30\n").geoms) == 2)
    # SHX font (synthetic)
    from laserforge import shx
    body = [(0, b"t\0" + bytes([9, 2, 0, 0])), (76, b"L\0" + bytes([2, 0x94, 1, 0x9C, 0x60, 2, 0x20, 0]))]
    data = b"AutoCAD-86 shapes 1.0\r\n\x1a" + _struct.pack("<HHH", 0, 76, 2) + b"".join(_struct.pack("<HH", n, len(b)) for n, b in body) + b"".join(b for _, b in body)
    shx_path = os.path.join(TMP, "t.shx"); open(shx_path, "wb").write(data)
    check("SHX font strokes", shx.ShxFont(shx_path).glyph(76)[0] == [[(0.0, 9.0), (0.0, 0.0), (6.0, 0.0)]])
    # flavours, feeder, galvo cylinder
    from laserforge.gcode import MachineSettings, apply_feeder, translate_flavor
    mar = translate_flavor("$H\nM4 S0\nG4 P0.250\nG1 X1 F100 S1000\nM2", MachineSettings(flavor="marlin", max_power=1000))
    check("Marlin flavour", "G28" in mar and "S255" in mar and "G4 P250" in mar and "M2" not in mar, mar.replace("\n", " | "))
    fed = apply_feeder("; === LaserForge Job ===\nG1 X1\n; === Job Complete ===\nM2", MachineSettings(feeder_enabled=True, feeder_repeats=3, feeder_axis="A", feeder_distance=50))
    check("feeder repeats", fed.count("feeder run") == 3 and fed.count("G1 A50.000") == 2)
    w._store_json("cylinder", {"enabled": True, "diameter": 100, "axis": "x", "centre": 100, "mirror": 200})
    fcyl = w.cylinder_transform()
    yg = fcyl((0, 150))[1] - 100
    check("galvo cylinder correction", abs(yg - 50 * math.sin(1) * 200 / (200 + 50 * (1 - math.cos(1)))) < 1e-9, yg)
    w._store_json("cylinder", {"enabled": False})
    # camera homography + mark detection
    from laserforge import extras as X
    import numpy as _np
    Ht = _np.array([[1.6, 0.08, 40], [0.03, 1.55, 25], [0.0002, 0.0001, 1.0]])
    tg = D.alignment_targets((400, 300))
    ip = [X.apply_h(Ht, x, y) for x, y in tg]
    cam = Image.new("RGB", (800, 600), "white"); u, v = X.apply_h(Ht, 123, 87)
    ImageDraw.Draw(cam).ellipse((u - 6, v - 6, u + 6, v + 6), fill="black")
    ov = X.warp_to_workspace(X.pil_to_qimage(cam), {"points": [[list(p), list(t)] for p, t in zip(ip, tg)]}, (400, 300), 1200)
    mk = X.detect_mark(ov, (400, 300), 118, 92)
    check("camera alignment + mark detection", mk and abs(mk[0] - 123) < 1 and abs(mk[1] - 87) < 1, mk)
    # settings bundle round trip
    bpath = os.path.join(TMP, "b.lfbundle")
    w.prefs["nudge"] = 3.5; P.save_prefs(w.settings, w.prefs)
    P.export_bundle(w, bpath)
    w.prefs["nudge"] = 1.0; P.save_prefs(w.settings, w.prefs)
    P.import_bundle(w, bpath)
    check("settings bundle", _zip.is_zipfile(bpath) and w.prefs["nudge"] == 3.5)
    w.prefs.update(P.DEFAULT_PREFS, obj_snap=False); w.apply_prefs()
    # selection origin
    doc.reset()
    ra = doc.create("rect", x=0, y=0, w=10, h=10); rb = doc.create("rect", x=100, y=100, w=10, h=10)
    doc.add_shapes([ra, rb]); doc.set_selection([rb["id"]])
    w.prefs["origin_from_selection"] = True; w.start_from = "user"; w.user_origin = (0.0, 0.0); w.job_origin = 0
    job = w.build_job()
    xs_ = [p_[0] for l_ in job["layers"] for pth in l_["paths"] for p_ in pth]
    check("job origin from selection", abs(min(xs_) + 100) < 1e-6, min(xs_))
    w.prefs["origin_from_selection"] = False; w.start_from = "absolute"
    # remote control
    w.prefs.update(remote_enabled=True, remote_port=18643, remote_pin="424242"); w.apply_prefs()
    try:
        _ur.urlopen(_ur.Request("http://127.0.0.1:18643/api/status", headers={"X-PIN": "1"}))
        refused = False
    except Exception:
        refused = True
    st_ = _json.loads(_ur.urlopen(_ur.Request("http://127.0.0.1:18643/api/status", headers={"X-PIN": "424242"})).read())
    check("remote control (PIN required)", refused and "state" in st_)
    w.prefs["remote_enabled"] = False; w.apply_prefs()
    check("controller file listing parser", D.parse_file_listing(["[FILE: /sd/a.gcode|SIZE:2048]"]) == [("/sd/a.gcode", 2048)])


    # ------------------------------------------------------------ real serial protocol against an emulated GRBL
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from fake_grbl import FakeGrbl
    fg = FakeGrbl()
    w.engine.update_machine({"home_on_start": False})
    w.engine.connect(fg.path); time.sleep(0.4)
    w.engine.send(w.engine.frame((10, 10, 150, 60))); time.sleep(1.0); w._poll()
    check("GRBL alarm keeps the connection (shows Unlock)", w.engine.state() == "alarm" and w.connected and w.laser_panel.alarm_box.isVisible())
    check("reconnect during alarm stays connected", w.engine.connect(fg.path) and w.engine.is_connected())
    time.sleep(0.5); w.run("unlock"); time.sleep(0.8); w._poll()
    w.engine.send(w.engine.frame((10, 10, 90, 60))); time.sleep(1.0)
    check("after $X the job runs to the end", w.engine.state() == "connected" and w.engine.progress()[0] == w.engine.progress()[1] > 0)
    w.engine.disconnect(); fg.running = False; w._poll()


    # ------------------------------------------------------------ machine origin corner
    w.engine.update_machine({"workspace_x": 100, "workspace_y": 100, "origin": "fl", "home_on_start": False})
    doc.reset(); doc.add_shapes([doc.create("rect", x=10, y=10, w=20, h=10)])
    ys_o = [p_[1] for p_ in w.build_job()["layers"][0]["paths"][0]]
    check("origin front-left: top of design goes to the back", abs(min(ys_o) - 80) < 1e-6 and abs(max(ys_o) - 90) < 1e-6, (min(ys_o), max(ys_o)))
    w.engine.update_machine({"origin": "rr"})
    xs_o = [p_[0] for p_ in w.build_job()["layers"][0]["paths"][0]]
    check("origin rear-right mirrors X", abs(min(xs_o) - 70) < 1e-6 and abs(max(xs_o) - 90) < 1e-6, (min(xs_o), max(xs_o)))
    w.engine.update_machine({"origin": "fl"})
    fg2 = FakeGrbl(); fg2.x, fg2.y = 50, 50
    w.engine.connect(fg2.path); time.sleep(0.4); w._poll()
    w.frame_job(); time.sleep(1.0); w._poll()
    check("frame stays inside the machine (no alarm)", not fg2.alarm and w.engine.state() == "connected")
    w.move_panel.step.setValue(10.0); y_b = w.engine.position()[1]; w.jog(0, -1); time.sleep(0.5); w._poll()
    check("jog up moves towards the back", abs(w.engine.position()[1] - y_b - 10) < 1e-6)
    w.engine.disconnect(); fg2.running = False; w._poll()

    # ------------------------------------------------------------ air assist, LightBurn style (M8 default, M7 optional)
    from laserforge.gcode import AirAssistMode as _AM
    check("old m7/m8 layers load as air ON", _AM("m7") is _AM.ON and _AM("m8") is _AM.ON and _AM("off") is _AM.OFF)
    fg4 = FakeGrbl(); fg4.x, fg4.y = 50, 50
    w.engine.connect(fg4.path); time.sleep(0.4)
    w.air_test("on"); time.sleep(0.3)
    check("air test sends M8 and stock GRBL turns air on", "M8" in fg4.log and getattr(fg4, "air", False), fg4.log[-3:])
    w.air_test("off"); time.sleep(0.3)
    check("air off sends M9", fg4.log[-1].startswith("M9") and not fg4.air)
    w.engine.update_machine({"air_command": "M7"})
    check("device set to M7 emits M7", "M7 ;" in w.engine.generate(w.build_job())["gcode"])
    w.engine.update_machine({"air_command": "M8"})
    # buffered streaming (LightBurn "Buffered"): several lines in GRBL's 127-byte buffer, never more
    fg4.delay = 0.01; fg4.log.clear()
    job_lines = "\n".join(f"G1 X{50 + (i % 10) * 0.5:.1f} Y50 F3000 S{i % 7}" for i in range(80))
    t0_ = time.time(); w.engine.send(job_lines)
    while w.engine.progress()[0] < 80 and time.time() - t0_ < 10:
        time.sleep(0.05)
    c_ = w.engine.connection
    check("buffered streaming keeps several lines queued in GRBL", getattr(c_, "max_inflight", 0) >= 3
          and c_.max_inflight_bytes <= 127 and w.engine.progress()[0] == 80, (getattr(c_, "max_inflight", 0), getattr(c_, "max_inflight_bytes", 0)))
    check("buffered streaming keeps line order", [l_ for l_ in fg4.log if l_.startswith("G1")] == job_lines.splitlines())
    fg4.delay = 0
    w.engine.update_machine({"fire_enabled": True, "fire_power": 1.0})
    t_ = time.time()
    while w.engine.state() == "busy" and time.time() - t_ < 3:
        time.sleep(0.05); w._poll()
    fg4.log.clear()
    w._poll(); st_ = (w.engine.state(), w.connected)
    w.fire_laser(True); time.sleep(0.3); w.fire_laser(False); time.sleep(0.3)
    check("Fire button like LightBurn: M3 + G1 F100 S…, then S0 G0 M5", fg4.log == ["M3", "G1 F100 S10", "S0", "G0", "M5"], (list(fg4.log), st_))
    # frame from current position: relative moves (G91) and back to the start, like LightBurn
    w.start_from = "current"; w.job_origin = 0; w.machine_pos = (50.0, 50.0)
    fg4.x, fg4.y = 50.0, 50.0; w._poll(); w.machine_pos = (50.0, 50.0); fg4.log.clear(); w.frame_job(); time.sleep(1.0); w._poll()
    check("current-position frame uses G91 and returns to start", "G91" in fg4.log and abs(fg4.x - 50) < 1e-6 and abs(fg4.y - 50) < 1e-6
          and not fg4.alarm, (fg4.x, fg4.y, fg4.log))
    w.start_from = "absolute"
    w.engine.disconnect(); fg4.running = False; w._poll()
    w.engine.update_machine({"workspace_x": 400, "workspace_y": 400, "origin": "rl"})


    # ------------------------------------------------------------ out-of-area protection
    w.engine.update_machine({"workspace_x": 100, "workspace_y": 100, "origin": "fl", "home_on_start": False})
    w.settings.setValue("userOriginSet", False)
    w.set_start_from("user")
    check("user origin refused until set", w.start_from == "absolute")
    doc.reset(); doc.add_shapes([doc.create("rect", x=40, y=40, w=20, h=20)])
    w.machine_pos = (95.0, 95.0); w.start_from = "current"; w.job_origin = 0     # job would start at the head, near the corner
    segs_o = D.parse_gcode(w.engine.generate(w.build_job())["gcode"])
    check("preview flags parts outside the area", D.PreviewView(w, segs_o).outside > 0)
    fg3 = FakeGrbl(); fg3.x, fg3.y = 50, 50
    w.engine.connect(fg3.path); time.sleep(0.4); w._poll()
    from PySide6.QtWidgets import QMessageBox as _QMB

    def press_absolute():
        box = next((x for x in app.topLevelWidgets() if isinstance(x, _QMB) and x.isVisible()), None)
        if box is None:
            QTimer.singleShot(30, press_absolute)
            return
        next(b for b in box.buttons() if "Assolute" in b.text() or "Absolute" in b.text()).click()
    QTimer.singleShot(50, press_absolute)
    w.machine_pos = (95.0, 95.0)
    w.frame_job(); time.sleep(1.0); w._poll()
    check("out-of-area frame: warning switches to Absolute, frame on the grid", w.start_from == "absolute" and not fg3.alarm,
          (w.start_from, fg3.alarm, "connected" if w.connected else "NOT connected", w.engine.state(), [l["msg"] for l in w.engine.console_lines()[-3:]]))
    w.engine.disconnect(); fg3.running = False; w._poll()
    w.engine.update_machine({"workspace_x": 400, "workspace_y": 400, "origin": "rl"})

    # ------------------------------------------------------------ 30 layers + T1/T2, nothing outside the area
    ls_ = w.engine.layers
    check("30 colour layers plus T1 and T2", len(ls_) == 32 and ls_[29].name == "C29" and [l_.name for l_ in ls_[30:]] == ["T1", "T2"]
          and all(l_.tool and not l_.enabled for l_ in ls_[30:]) and len(w.palette_btns) == 32)
    w.engine.update_layer(30, {"enabled": True, "name": "X"})
    check("tool layers can't be switched to output", not ls_[30].enabled and ls_[30].name == "T1" and ls_[30].tool)
    w.engine.update_machine({"workspace_x": 100, "workspace_y": 100, "origin": "rl", "home_on_start": False})
    w.start_from = "absolute"
    doc.reset()
    r_in = doc.create("rect", x=10, y=10, w=20, h=20)
    r_tool = doc.create("rect", x=40, y=40, w=10, h=10); r_tool["layer"] = 30
    r_out = doc.create("rect", x=150, y=150, w=10, h=10)
    r_half = doc.create("rect", x=90, y=50, w=20, h=10)
    doc.add_shapes([r_in, r_tool, r_out, r_half])
    jb = w.build_job()
    pts_ = [p_ for l_ in jb["layers"] for path in l_["paths"] for p_ in path]
    check("tool layer and off-area shapes are not sent to the laser",
          len([pa for l_ in jb["layers"] for pa in l_["paths"]]) == 2 and all(0 <= x_ <= 100 and 0 <= y_ <= 100 for x_, y_ in pts_),
          [len(l_["paths"]) for l_ in jb["layers"]])
    check("shapes across the edge are cut at the border", any(abs(x_ - 100) < 1e-6 for x_, _ in pts_) and max(x_ for x_, _ in pts_) <= 100)
    g_ = w.engine.generate(jb)["gcode"]
    bad_ = [l_ for l_ in g_.splitlines() if any(float(v) > 100.0001 for v in __import__("re").findall(r"X(-?[\d.]+)", l_))]
    check("G-code never leaves the work area", not bad_, (bad_[:5], ls_[0].cut_mode, ls_[0].kerf_offset, ls_[0].overscan))
    fb = w.clip_to_area(M.multi_bounds(w.output_shapes(doc.shapes)))
    check("frame covers only what the laser burns", fb is not None and abs(fb[0] - 10) < 1e-6 and abs(fb[0] + fb[2] - 100) < 1e-6, fb)
    w.engine.update_machine({"workspace_x": 400, "workspace_y": 400})

    # ------------------------------------------------------------ job structure, relative modes, frame, Fire
    from laserforge.gcode import to_relative as _rel, JOB_HEADER as _JH
    from laserforge.serial_comm import GRBLConnection as _GC
    gj = _GG(_MS(home_on_start=True, start_gcode="M8", end_gcode="M9")).generate_job(
        [], [])
    check("job: no $H, LightBurn preamble, end order M5 M9 → return → end script → M2",
          "$H" not in gj and gj.index(";USER START SCRIPT") < gj.index(_JH) and
          gj.index("M9 ; air") < gj.index("G0 X0 Y0") < gj.index(";USER END SCRIPT") < gj.index("M2"), gj)
    rel_ = _rel("\n".join([_JH, "G90", "G0 X12 Y12", "G1 X22 Y12 F600 S500", "G0 X10.000 Y10.000 ; return to start", "M2"]), (10, 10))
    check("current position job is relative (G91) like LightBurn", rel_.splitlines()[1] == "G91" and "G0 X2.000 Y2.000" in rel_
          and "G1 X10.000 Y0.000 F600 S500" in rel_ and rel_.splitlines()[-2:] == ["G90", "M2"], rel_)
    rel_u = _rel("\n".join([_JH, "G90", "G0 X12 Y12", "M2"]), (10, 10), user_origin=True)
    check("user origin job: G90 G0 to the origin, then G91", rel_u.splitlines()[1:4] == ["G90", "G0 X10.000 Y10.000", "G91"], rel_u)
    gc_ = _GC(); gc_._parse_status("<Idle|MPos:110.000,60.000,0.000|WCO:100.000,50.000,0.000>")
    check("status uses work coordinates (MPos − WCO)", abs(gc_.status.x - 10) < 1e-9 and abs(gc_.status.y - 10) < 1e-9, (gc_.status.x, gc_.status.y))
    fr_ = _GG(_MS()).generate_frame((10, 10, 50, 40), head=(60, 45))
    check("frame: LightBurn preamble, starts at the corner nearest the head", _JH in fr_ and "G0 X50.000 Y40.000" in fr_
          and fr_.strip().splitlines()[-4:] == ["G0", "G90", "S0", "M5"], fr_)
    fg5 = FakeGrbl(); fg5.x, fg5.y = 30, 30
    w.engine.update_machine({"home_on_start": True, "workspace_x": 100, "workspace_y": 100, "origin": "rl"})
    w.engine.connect(fg5.path); time.sleep(1.6); w._poll()
    check("auto-home when connecting (not in the job)", "$H" in fg5.log, fg5.log[:5])
    w.engine.update_machine({"home_on_start": False})
    fg5.x, fg5.y = 40.0, 40.0; time.sleep(0.4); w._poll()
    doc.reset(); doc.add_shapes([doc.create("rect", x=10, y=10, w=20, h=10)])
    w.start_from, w.job_origin, w.machine_pos = "current", 0, (40.0, 40.0)
    w.gcode = ""; w.generate_gcode()
    fin_ = w.finalize_gcode(w.gcode)
    fg5.log.clear(); w.engine.send(fin_)
    t_ = time.time()
    while w.engine.progress()[0] < w.engine.progress()[1] and time.time() - t_ < 6:
        time.sleep(0.05)
    xs5 = [l_ for l_ in fg5.log if l_.startswith(("G0", "G1"))]
    check("current-position job runs relative and returns to the start", "G91" in fg5.log and abs(fg5.x - 40) < 1e-6 and abs(fg5.y - 40) < 1e-6
          and not fg5.alarm, (fg5.x, fg5.y, xs5[:6]))
    w.start_from = "absolute"
    w.engine.disconnect(); fg5.running = False; w._poll()
    w.engine.update_machine({"workspace_x": 400, "workspace_y": 400})

    # ------------------------------------------------------------ air only while cutting (LightBurn layer option)
    from laserforge.gcode import GCodePoint as _GP
    two = [[_GP(0, 0), _GP(10, 0)], [_GP(20, 0), _GP(30, 0)]]
    ga_on = _GG(_MS()).generate_layer(_LS(air_cut_only=True), two).splitlines()
    g0s = [i for i, l_ in enumerate(ga_on) if l_.startswith("G0")]
    check("air only while cutting: M9 before each travel, M8 before each cut",
          all(ga_on[i - 1].startswith("M9") and ga_on[i + 1].startswith("M8") for i in g0s) and len(g0s) == 2, ga_on)
    ga_off = _GG(_MS()).generate_layer(_LS(), two)
    check("normal air: no M9 between shapes", ga_off.count("M8") == 1 and "M9" not in ga_off, ga_off)
    w.engine.update_layer(0, {"air_cut_only": True})
    check("air-only-when-cutting is a layer setting", w.engine.layers[0].air_cut_only and "air_cut_only" in w.engine.layer_dicts()[0])
    w.engine.update_layer(0, {"air_cut_only": False})
    check("default layout: 4 bottom tabs, extra panels hidden", w.dock_library.isVisibleTo(w) and not w.dock_objects.isVisibleTo(w)
          and not w.dock_camera.isVisibleTo(w))
    w.reset_layout()
    check("reset window layout", w.dock_laser.isVisibleTo(w) and w.dock_cuts.isVisibleTo(w))

    # ------------------------------------------------------------ GRBL management like LightBurn
    from laserforge.gcode import apply_rotary_axis as _ara
    fg6 = FakeGrbl(); fg6.x, fg6.y = 20, 20; fg6.settings = {22: 0, 32: 0, 30: 1000, 20: 1, 120: 800}
    w.engine.update_machine({"home_on_start": True, "max_power": 1000})
    w.engine.connect(fg6.path); time.sleep(1.6); w._poll()
    check("settings read on connect ($$)", w.engine.grbl_settings.get(32) == 0 and w.engine.grbl_settings.get(120) == 800, w.engine.grbl_settings)
    pr_ = w.engine.grbl_problems()
    check("GRBL checks: laser mode off and soft limits without homing", any("$32" in p for p in pr_) and any("$20" in p for p in pr_), pr_)
    check("no auto-home when homing is disabled ($22=0)", "$H" not in fg6.log, fg6.log)
    check("Home refused when $22=0", w.engine.home() is False and "$H" not in fg6.log)
    w.engine.zero_xy(); time.sleep(0.3)
    check("Zero X/Y here sends G10 L20", "G10 L20 P1 X0 Y0" in fg6.log, fg6.log[-3:])
    fg6.log.clear(); w.engine.update_machine({"rotary_enabled": True, "rotary_axis": "A", "rotary_mm_per_rev": 100, "rotary_diameter": 100 / math.pi})
    w.engine.jog(y=5, feed=600); time.sleep(0.3)
    check("rotary on A: Y jog turns the A axis", any(l_.startswith("$J=G91 A5.000") for l_ in fg6.log), fg6.log)
    check("rotary on A: job Y becomes A", _ara("G1 X10.000 Y5.000 F600 S100\nG0 X0 Y0", w.engine.machine) == "G1 X10.000 A5.000 F600 S100\nG0 X0 A0")
    w.engine.update_machine({"rotary_enabled": False, "rotary_axis": "Y", "home_on_start": False})
    gz_off = _GG(_MS()).generate_layer(_LS(z_offset=2), [[_GP(0, 0), _GP(5, 0)]])
    gz_on = _GG(_MS(z_enabled=True, z_speed=5)).generate_layer(_LS(z_offset=2), [[_GP(0, 0), _GP(5, 0)]])
    gz_rel = _GG(_MS(z_enabled=True, z_relative_only=True)).generate_layer(_LS(z_offset=2), [[_GP(0, 0), _GP(5, 0)]])
    check("Z moves only when the Z axis is enabled", "G1 Z" not in gz_off and "G1 Z2.000 F300 S0" in gz_on, gz_on)
    check("relative-only Z uses G91", "G91 G1 Z2.000" in gz_rel and "G91 G1 Z-2.000" in gz_rel, gz_rel)
    w.engine.disconnect(); fg6.running = False; w._poll()

    # ------------------------------------------------------------ LightBurn cut options and tools
    sq_ = [_GP(0, 0), _GP(10, 0), _GP(10, 10), _GP(0, 10), _GP(0, 0)]
    sqm = [_GP(20, 20), _GP(30, 20), _GP(30, 30), _GP(20, 30), _GP(20, 20)]
    gl_ = _GG(_MS()).generate_layer(_LS(lead_in=2, lead_out=1, overcut=3), [sqm])
    check("lead-in starts 2 mm before, overcut 3 mm past the start, lead-out 1 mm", "G0 X18.000 Y20.000" in gl_
          and "G1 X23.000 Y20.000" in gl_ and "G1 X24.000 Y20.000" in gl_, gl_)
    ge_ = _GG(_MS()).generate_layer(_LS(lead_in=2), [sq_])
    xs_e = [float(v) for v in __import__("re").findall(r"X(-?[\d.]+)", ge_)]
    check("a lead never leaves the bed (shape on the edge)", min(xs_e) >= 0, ge_)
    gp_ = _GG(_MS()).generate_layer(_LS(perforate=True, perf_cut=2, perf_skip=1), [[_GP(0, 0), _GP(10, 0)]])
    check("perforation: 2 mm cut, 1 mm skip", gp_.count("G0 X") == 4 and "G1 X2.000" in gp_ and "G0 X3.000" in gp_, gp_)
    two_sq = [sq_, [_GP(50, 0), _GP(60, 0), _GP(60, 10), _GP(50, 10), _GP(50, 0)]]
    gf_ = _GG(_MS()).generate_layer(_LS(cut_mode=__import__("laserforge.gcode", fromlist=["CutMode"]).CutMode.FILL, flood_fill=True, line_interval=1), two_sq)
    long_rows = [l_ for l_ in gf_.splitlines() if l_.startswith("G1") and "S0" in l_]
    import re as _re2
    pts_f = []; last_ = None
    for l_ in gf_.splitlines():
        m_ = _re2.search(r"X(-?[\d.]+) Y(-?[\d.]+)", l_)
        if m_ and l_.startswith("G1") and not l_.endswith("S0"):
            x_ = float(m_.group(1))
            if last_ is not None:
                pts_f.append((last_, x_))
            last_ = x_
        elif m_:
            last_ = float(m_.group(1))
    crossing = [p_ for p_ in pts_f if min(p_) <= 10.01 and max(p_) >= 49.99]
    check("flood fill: no burned row crosses the gap between the two areas", "Flood fill: 2 areas" in gf_ and not crossing, crossing[:3])
    w.engine.update_layer(0, {"sublayers": [{"enabled": True, "name": "taglio", "cut_mode": "line", "speed": 300, "power_max": 90, "passes": 2},
                                            {"enabled": False, "name": "spento"}]})
    jl_ = w.engine._job_layers({"layers": [{"layer_index": 0, "paths": [[(0, 0), (5, 0)]]}]})
    check("sub-layers: the same shapes again with their own settings (disabled ones skipped)",
          len(jl_) == 2 and jl_[1][0].name == "taglio" and jl_[1][0].speed == 300 and jl_[1][0].passes == 2, [(l_.name, l_.speed) for l_, _ in jl_])
    w.engine.update_layer(0, {"sublayers": []})
    xo_ = w.engine.geometry("xor", [[[(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]], [[(5, 0), (15, 0), (15, 10), (5, 10), (5, 0)]]], {})
    check("exclusion (XOR) for the Boolean assistant", xo_ and len(xo_[0]) == 2, xo_)
    from laserforge.serial_comm import discover_network as _dn
    check("network discovery runs (Bonjour)", isinstance(_dn(timeout=0.5), list))

    # ------------------------------------------------------------ zoom and pan (trackpad / mouse)
    from PySide6.QtGui import QWheelEvent as _QWE
    from PySide6.QtCore import QPoint as _QP
    cv = w.canvas; cv.zoom_page(animate=False)
    z0 = cv.zoom; wx0 = cv.to_world(200, 150)

    def wheel(pixel, angle, phase, mods=Qt.NoModifier):
        ev = _QWE(QPointF(200, 150), QPointF(200, 150), pixel, angle, Qt.NoButton, mods, phase, False)
        cv.wheelEvent(ev)
    wheel(_QP(0, 0), _QP(0, 120), Qt.NoScrollPhase)               # one mouse-wheel notch
    t_ = time.time()
    while time.time() - t_ < 0.5:
        app.processEvents(); time.sleep(0.01)
    wx1 = cv.to_world(200, 150)
    check("mouse wheel zooms smoothly at the cursor", abs(cv.zoom / z0 - 1.2) < 1e-6 and math.hypot(wx1[0] - wx0[0], wx1[1] - wx0[1]) < 1e-6,
          (cv.zoom / z0, wx0, wx1))
    zb_ = cv.zoom; wb_ = cv.to_world(200, 150)
    wheel(_QP(0, 30), _QP(0, 30), Qt.ScrollUpdate)                # two fingers up/down: zoom, like LightBurn
    wa_ = cv.to_world(200, 150)
    check("trackpad two-finger scroll zooms at the cursor (LightBurn)", cv.zoom > zb_ and math.hypot(wa_[0] - wb_[0], wa_[1] - wb_[1]) < 1e-6)
    zb_ = cv.zoom
    wheel(_QP(0, 300), _QP(0, 300), Qt.ScrollMomentum)
    check("no zoom drift from scroll momentum", cv.zoom == zb_)
    px_ = cv.pan_x
    wheel(_QP(-30, 0), _QP(-30, 0), Qt.ScrollUpdate, Qt.ShiftModifier)   # Shift + two fingers: pan
    check("shift + two fingers pans", abs(cv.pan_x - (px_ - 30)) < 1e-6, (px_, cv.pan_x))
    w.prefs["trackpad_scroll"] = "pan"
    px_ = cv.pan_x
    wheel(_QP(-30, 0), _QP(-30, 0), Qt.ScrollUpdate)
    check("setting: two-finger scroll can pan instead", abs(cv.pan_x - (px_ - 30)) < 1e-6, (px_, cv.pan_x))
    for _ in range(200):
        wheel(_QP(-400, -400), _QP(-400, -400), Qt.ScrollMomentum)
    ws_ = w.workspace()
    vis = cv.to_screen(ws_[0], ws_[1])
    check("panning never loses the work area", vis[0] >= 59 and vis[1] >= 59, vis)
    zr = cv.zoom
    wheel(_QP(0, 40), _QP(0, 40), Qt.ScrollUpdate, Qt.MetaModifier)  # ⌘ + two fingers
    check("cmd + trackpad scroll zooms", cv.zoom > zr)
    w.prefs["trackpad_scroll"] = "zoom"
    cv.zoom_real(1.0); t_ = time.time()
    while time.time() - t_ < 0.5:
        app.processEvents(); time.sleep(0.01)
    check("100% zoom means true size", abs(cv.zoom_percent() - 100) < 0.5, cv.zoom_percent())
    for _ in range(100):
        cv.zoom_at(10, 10, 0.5)
    check("zoom-out stops near the whole bed", cv.zoom >= cv.fit_zoom() * 0.2 - 1e-9)
    cv.zoom_page(animate=False)

    # ------------------------------------------------------------ project round trip and render
    p2 = os.path.join(TMP, "rt.lfproj"); w.project_path = p2; w.save_project(False)
    n = len(doc.shapes); doc.reset(); w.open_path(p2, confirmed=True)
    check("project round trip", len(doc.shapes) == n)
    check("render", not w.canvas.grab().isNull())
    w.grab().save(os.path.join(TMP, "window.png"))
    doc.set_dirty(False)
except Exception:
    results.append(("EXCEPTION", False, traceback.format_exc()))
    fails.append("exception")

for name, ok, info in results:
    print(f"{'OK  ' if ok else 'FAIL'} {name:42s} {info if not ok else ''}")
print(f"\n{len(results) - len(fails)}/{len(results)} passed" + (f" — FAILED: {fails}" if fails else ""))
print("screenshot:", os.path.join(TMP, "window.png"))
sys.exit(1 if fails else 0)
