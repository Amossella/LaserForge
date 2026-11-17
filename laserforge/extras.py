"""Art Library and Camera Control panels."""

import json
import math
import os
import re
import time
from pathlib import Path

from PySide6.QtCore import QPointF, QRectF, QSize, Qt
from PySide6.QtGui import QColor, QIcon, QImage, QPainter, QPainterPath, QPen, QPixmap
from PySide6.QtWidgets import (
    QAbstractItemView, QComboBox, QFileDialog, QHBoxLayout, QInputDialog, QLabel, QListView, QListWidget,
    QListWidgetItem, QMessageBox, QPushButton, QSlider, QVBoxLayout, QWidget,
)

from . import model as M
from .ui_common import Segmented, checkbox, dspin, hint

ART_DIR = Path(os.environ.get("LASERFORGE_ART_DIR") or Path.home() / "Library" / "Application Support" / "LaserForge" / "ArtLibrary")


def render_thumbnail(shapes, layer_color, size=96):
    pm = QPixmap(size * 2, size * 2)
    pm.setDevicePixelRatio(2)
    pm.fill(QColor("#ffffff"))
    b = M.multi_bounds(shapes)
    if not b or b[2] <= 0 and b[3] <= 0:
        return pm
    pad = 8
    k = min((size - 2 * pad) / max(b[2], 1e-6), (size - 2 * pad) / max(b[3], 1e-6))
    ox = (size - b[2] * k) / 2 - b[0] * k
    oy = (size - b[3] * k) / 2 - b[1] * k
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    for s in M.flatten(shapes):
        if s["type"] == "image":
            p.fillRect(QRectF(ox + s["x"] * k, oy + s["y"] * k, s["w"] * k, s["h"] * k), QColor("#bbbbbb"))
            continue
        qp = QPainterPath()
        for path in M.shape_paths(s):
            if len(path) > 1:
                qp.moveTo(ox + path[0][0] * k, oy + path[0][1] * k)
                for x, y in path[1:]:
                    qp.lineTo(ox + x * k, oy + y * k)
        p.setPen(QPen(QColor(layer_color(s.get("layer", 0))), 1.2))
        p.drawPath(qp)
    p.end()
    return pm


# ====================================================================== ART LIBRARY
class ArtLibraryPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        ART_DIR.mkdir(parents=True, exist_ok=True)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.list = QListWidget()
        self.list.setViewMode(QListView.IconMode)
        self.list.setIconSize(QSize(96, 96))
        self.list.setGridSize(QSize(116, 128))
        self.list.setResizeMode(QListView.Adjust)
        self.list.setMovement(QListView.Static)
        self.list.setWordWrap(True)
        self.list.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.list.itemDoubleClicked.connect(lambda it: self.insert(it))
        lay.addWidget(self.list, 1)
        h = QHBoxLayout()
        self.add_btn, self.ins_btn, self.imp_btn, self.del_btn = QPushButton(), QPushButton(), QPushButton(), QPushButton()
        self.add_btn.clicked.connect(self.add_selection)
        self.ins_btn.clicked.connect(lambda: [self.insert(it) for it in self.list.selectedItems()])
        self.imp_btn.clicked.connect(self.import_files)
        self.del_btn.clicked.connect(self.delete)
        for b in (self.add_btn, self.ins_btn, self.imp_btn, self.del_btn):
            h.addWidget(b)
        lay.addLayout(h)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self.retranslate()
        self.reload()

    def retranslate(self):
        L = self.win.L
        self.add_btn.setText(L("Aggiungi selezione", "Add selection"))
        self.ins_btn.setText(L("Inserisci", "Insert"))
        self.imp_btn.setText(L("Importa file...", "Import files..."))
        self.del_btn.setText(L("Elimina", "Delete"))
        self.hint.setText(L("Doppio clic per inserire un elemento al centro dell'area di lavoro. Gli elementi sono salvati in ",
                            "Double-click to insert an item at the centre of the work area. Items are stored in ") + str(ART_DIR))

    def reload(self):
        self.list.clear()
        for f in sorted(ART_DIR.glob("*.json"), key=lambda p: p.stat().st_mtime, reverse=True):
            try:
                data = json.loads(f.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            it = QListWidgetItem(QIcon(render_thumbnail(data.get("shapes", []), self.win.layer_color)), data.get("name", f.stem))
            it.setData(Qt.UserRole, str(f))
            it.setToolTip(data.get("name", f.stem))
            self.list.addItem(it)

    def _save(self, name, shapes):
        safe = re.sub(r"[^\w\- ]", "_", name).strip() or "art"
        path = ART_DIR / f"{safe}-{int(time.time() * 1000)}.json"
        path.write_text(json.dumps({"name": name, "shapes": shapes}), encoding="utf-8")

    def add_selection(self):
        sel = self.win.doc.selected_shapes()
        if not sel:
            self.win.toast(self.win.tr("selectFirst"))
            return
        name, ok = QInputDialog.getText(self, self.win.L("Libreria grafica", "Art Library"), self.win.L("Nome:", "Name:"),
                                        text=sel[0].get("name") or sel[0]["type"])
        if not ok:
            return
        self._save(name or "art", M.clone(sel))
        self.reload()

    def import_files(self):
        paths, _ = QFileDialog.getOpenFileNames(self, self.win.L("Importa nella libreria", "Import to library"),
                                                self.win.settings.value("lastDir", str(Path.home())), "SVG / LaserForge (*.svg *.lfproj)")
        for path in paths:
            before = len(self.win.doc.shapes)
            if path.endswith(".lfproj"):
                try:
                    shapes = json.loads(Path(path).read_text(encoding="utf-8")).get("shapes", [])
                except (OSError, ValueError):
                    continue
            else:
                self.win.import_svg(Path(path).read_text(encoding="utf-8", errors="replace"))
                shapes = M.clone(self.win.doc.shapes[before:])
                self.win.doc.undo()
            if shapes:
                self._save(Path(path).stem, shapes)
        self.reload()

    def insert(self, it):
        try:
            data = json.loads(Path(it.data(Qt.UserRole)).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return
        shapes = [self.win.doc.reid(s) for s in M.clone(data.get("shapes", []))]
        if not shapes:
            return
        b = M.multi_bounds(shapes)
        W, H = self.win.workspace()
        for s in shapes:
            M.move_shape(s, W / 2 - b[0] - b[2] / 2, H / 2 - b[1] - b[3] / 2)
        self.win.doc.add_shapes(shapes)

    def delete(self):
        items = self.list.selectedItems()
        if not items:
            return
        if QMessageBox.question(self, "LaserForge", self.win.L(f"Eliminare {len(items)} elementi dalla libreria?",
                                                              f"Delete {len(items)} items from the library?")) != QMessageBox.Yes:
            return
        for it in items:
            try:
                os.remove(it.data(Qt.UserRole))
            except OSError:
                pass
        self.reload()


# ====================================================================== CAMERA
class CameraPanel(QWidget):
    """USB camera preview and workspace overlay (LightBurn 'Camera Control')."""

    def __init__(self, win):
        super().__init__()
        self.win = win
        self.camera = None
        self.session = None
        self.sink = None
        self.frame = None
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        h = QHBoxLayout()
        self.devices = QComboBox()
        self.refresh_btn = QPushButton("↻")
        self.refresh_btn.setFixedWidth(32)
        self.refresh_btn.clicked.connect(self.refresh_devices)
        self.start_btn = QPushButton()
        self.start_btn.setCheckable(True)
        self.start_btn.toggled.connect(self.toggle_camera)
        h.addWidget(self.devices, 1)
        h.addWidget(self.refresh_btn)
        h.addWidget(self.start_btn)
        lay.addLayout(h)
        self.preview = QLabel()
        self.preview.setMinimumHeight(160)
        self.preview.setAlignment(Qt.AlignCenter)
        self.preview.setStyleSheet("background:#111; border-radius:4px; color:#777;")
        lay.addWidget(self.preview, 1)
        h2 = QHBoxLayout()
        self.capture_btn = QPushButton()
        self.capture_btn.setProperty("role", "primary")
        self.capture_btn.clicked.connect(self.capture_overlay)
        self.import_btn = QPushButton()
        self.import_btn.clicked.connect(self.import_image)
        self.align_btn = QPushButton()
        self.align_btn.clicked.connect(lambda: self.win.run("cameraAlign"))
        h2.addWidget(self.capture_btn)
        h2.addWidget(self.import_btn)
        h2.addWidget(self.align_btn)
        lay.addLayout(h2)
        h4 = QHBoxLayout()
        self.lens_btn, self.targets_btn = QPushButton(), QPushButton()
        self.lens_btn.clicked.connect(lambda: self.win.run("cameraLens"))
        self.targets_btn.clicked.connect(lambda: self.win.run("cameraTargets"))
        h4.addWidget(self.lens_btn)
        h4.addWidget(self.targets_btn)
        lay.addLayout(h4)
        h3 = QHBoxLayout()
        self.show_overlay = checkbox("", True)
        self.show_overlay.toggled.connect(self._changed)
        self.opacity = QSlider(Qt.Horizontal)
        self.opacity.setRange(10, 100)
        self.opacity.setValue(int(win.camera_cal.get("opacity", 70)))
        self.opacity.valueChanged.connect(self._changed)
        h3.addWidget(self.show_overlay)
        h3.addWidget(self.opacity, 1)
        lay.addLayout(h3)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self.retranslate()
        self.refresh_devices()

    def retranslate(self):
        L = self.win.L
        self.start_btn.setText(L("Ferma", "Stop") if self.start_btn.isChecked() else L("Avvia camera", "Start camera"))
        self.capture_btn.setText(L("Aggiorna sovrapposizione", "Update overlay"))
        self.import_btn.setText(L("Importa come immagine", "Import as image"))
        self.align_btn.setText(L("Allinea (4 angoli)…", "Align (4 corners)…"))
        self.lens_btn.setText(L("Correzione lente…", "Lens correction…"))
        self.targets_btn.setText(L("Allinea con segni incisi…", "Align with burned targets…"))
        self.show_overlay.setText(L("Mostra sull'area di lavoro · opacità", "Show on work area · opacity"))
        self.hint.setText(L("Allinea la camera una volta (Strumenti laser › Allinea camera) indicando dove cadono gli angoli dell'area di lavoro nell'immagine.",
                            "Align the camera once (Laser Tools › Camera Alignment) by marking where the work-area corners fall in the image."))
        if self.frame is None:
            self.preview.setText(L("Nessuna immagine", "No image"))

    def refresh_devices(self):
        from PySide6.QtMultimedia import QMediaDevices
        self.devices.clear()
        for dev in QMediaDevices.videoInputs():
            self.devices.addItem(dev.description(), dev)
        if not self.devices.count():
            self.devices.addItem(self.win.L("Nessuna camera trovata", "No camera found"), None)

    def toggle_camera(self, on):
        from PySide6.QtMultimedia import QCamera, QMediaCaptureSession, QVideoSink
        if on:
            dev = self.devices.currentData()
            if dev is None:
                self.start_btn.setChecked(False)
                self.win.toast(self.win.L("Nessuna camera trovata", "No camera found"))
                return
            self.camera = QCamera(dev)
            self.session = QMediaCaptureSession()
            self.sink = QVideoSink()
            self.session.setCamera(self.camera)
            self.session.setVideoSink(self.sink)
            self.sink.videoFrameChanged.connect(self._frame)
            self.camera.errorOccurred.connect(lambda _e, msg: self.win.toast(msg))
            self.camera.start()
        elif self.camera:
            self.camera.stop()
            self.camera = self.session = self.sink = None
        self.retranslate()

    def _frame(self, frame):
        img = frame.toImage()
        if img.isNull():
            return
        self.frame = img
        self.preview.setPixmap(QPixmap.fromImage(img).scaled(self.preview.size(), Qt.KeepAspectRatio, Qt.SmoothTransformation))

    def corrected(self):
        """Current frame warped so the work area fills it (using the 4 alignment points)."""
        if self.frame is None:
            return None
        return warp_to_workspace(self.frame, self.win.camera_cal, self.win.workspace(),
                                 int(self.win.prefs.get("camera_quality", 1600)))

    def capture_overlay(self):
        img = self.corrected()
        if img is None:
            self.win.toast(self.win.L("Avvia prima la camera", "Start the camera first"))
            return
        self.win.camera_overlay = img
        self.show_overlay.setChecked(True)
        self.win.canvas.update()

    def import_image(self):
        img = self.corrected()
        if img is None:
            self.win.toast(self.win.L("Avvia prima la camera", "Start the camera first"))
            return
        from PySide6.QtCore import QBuffer, QIODevice
        buf = QBuffer()
        buf.open(QIODevice.WriteOnly)
        img.save(buf, "PNG")
        W, H = self.win.workspace()
        s = self.win.doc.create("image", src="data:image/png;base64," + bytes(buf.data().toBase64()).decode(), name="camera.png",
                                x=0, y=0, w=W, h=H, adj=M.default_adj())
        self.win.doc.add_shapes([s])

    def _changed(self, *_):
        self.win.camera_cal["opacity"] = self.opacity.value()
        self.win.camera_overlay_visible = self.show_overlay.isChecked()
        self.win.canvas.update()

    def shutdown(self):
        if self.camera:
            self.camera.stop()


def qimage_to_pil(img: QImage):
    import numpy as np
    from PIL import Image
    rgb = img.convertToFormat(QImage.Format_RGB888)
    arr = np.frombuffer(rgb.constBits(), dtype=np.uint8, count=rgb.sizeInBytes()).reshape(rgb.height(), rgb.bytesPerLine())
    arr = arr[:, :rgb.width() * 3].reshape(rgb.height(), rgb.width(), 3).copy()
    return Image.fromarray(arr, "RGB")


def pil_to_qimage(pil):
    data = pil.convert("RGB").tobytes()
    return QImage(data, pil.width, pil.height, pil.width * 3, QImage.Format_RGB888).copy()


def undistort(pil, k1=0.0, k2=0.0):
    """Remove radial lens distortion: each output pixel samples the source at r·(1 + k1 r² + k2 r⁴)
    (r normalised to the half-diagonal). Negative k1 fixes barrel distortion (wide-angle lenses)."""
    import numpy as np
    from PIL import Image
    if not k1 and not k2:
        return pil
    a = np.asarray(pil, dtype=np.float32)
    h, w = a.shape[:2]
    cx, cy, norm = (w - 1) / 2, (h - 1) / 2, math.hypot(w / 2, h / 2)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    nx, ny = (xx - cx) / norm, (yy - cy) / norm
    r2 = nx * nx + ny * ny
    f = 1 + k1 * r2 + k2 * r2 * r2
    sx = np.clip(cx + nx * f * norm, 0, w - 1.001)
    sy = np.clip(cy + ny * f * norm, 0, h - 1.001)
    x0, y0 = sx.astype(np.int32), sy.astype(np.int32)
    fx, fy = (sx - x0)[..., None], (sy - y0)[..., None]
    out = (a[y0, x0] * (1 - fx) * (1 - fy) + a[y0, x0 + 1] * fx * (1 - fy) + a[y0 + 1, x0] * (1 - fx) * fy + a[y0 + 1, x0 + 1] * fx * fy)
    return Image.fromarray(out.clip(0, 255).astype(np.uint8), "RGB")


def homography(src, dst):
    """3x3 matrix mapping src points to dst points (least squares, ≥4 pairs)."""
    import numpy as np
    A = []
    for (x, y), (u, v) in zip(src, dst):
        A.append([-x, -y, -1, 0, 0, 0, u * x, u * y, u])
        A.append([0, 0, 0, -x, -y, -1, v * x, v * y, v])
    _, _, vt = np.linalg.svd(np.array(A, dtype=float))
    Hm = vt[-1].reshape(3, 3)
    return Hm / Hm[2, 2]


def apply_h(Hm, x, y):
    w = Hm[2, 0] * x + Hm[2, 1] * y + Hm[2, 2]
    return ((Hm[0, 0] * x + Hm[0, 1] * y + Hm[0, 2]) / w, (Hm[1, 0] * x + Hm[1, 1] * y + Hm[1, 2]) / w)


def image_to_bed_matrix(cal, workspace):
    """Homography image pixel -> bed mm from the alignment data (target points, or the 4 work-area corners)."""
    pts = cal.get("points")
    if pts and len(pts) >= 4:
        return homography([p[0] for p in pts], [p[1] for p in pts])
    corners = cal.get("corners")
    if corners and len(corners) == 4:
        W, H = workspace
        return homography(corners, [(0, 0), (W, 0), (W, H), (0, H)])
    return None


def warp_to_workspace(img: QImage, cal, workspace, max_px=1600):
    """Camera frame -> top-down image of the work area: lens correction, then perspective from the alignment."""
    import numpy as np
    from PIL import Image
    if isinstance(cal, list) or cal is None:          # backwards compatible: list of 4 corners
        cal = {"corners": cal}
    W, H = workspace
    scale = max_px / max(W, H)
    ow, oh = max(1, round(W * scale)), max(1, round(H * scale))
    pil = undistort(qimage_to_pil(img), float(cal.get("k1", 0) or 0), float(cal.get("k2", 0) or 0))
    Hm = image_to_bed_matrix(cal, workspace)
    if Hm is not None:
        # output pixel -> bed mm -> image pixel
        Minv = np.linalg.inv(Hm) @ np.diag([1 / scale, 1 / scale, 1.0])
        Minv = Minv / Minv[2, 2]
        coeffs = [Minv[0, 0], Minv[0, 1], Minv[0, 2], Minv[1, 0], Minv[1, 1], Minv[1, 2], Minv[2, 0], Minv[2, 1]]
        pil = pil.transform((ow, oh), Image.PERSPECTIVE, coeffs, Image.BICUBIC, fillcolor=(255, 255, 255))
    else:
        pil = pil.resize((ow, oh), Image.LANCZOS)
    return pil_to_qimage(pil)


def detect_mark(overlay: QImage, workspace, x_mm, y_mm, search_mm=15.0):
    """Centre (mm) of the darkest blob near (x_mm, y_mm) in a bed-aligned overlay image, or None."""
    import numpy as np
    W, H = workspace
    pil = qimage_to_pil(overlay).convert("L")
    a = np.asarray(pil, dtype=np.float32)
    sx, sy = pil.width / W, pil.height / H
    cx, cy, r = x_mm * sx, y_mm * sy, search_mm * sx
    x0, x1 = int(max(0, cx - r)), int(min(pil.width, cx + r))
    y0, y1 = int(max(0, cy - r)), int(min(pil.height, cy + r))
    win = a[y0:y1, x0:x1]
    if win.size == 0:
        return None
    thr = min(win.mean() - 40, (win.min() + win.mean()) / 2)
    ys, xs = np.nonzero(win < thr)
    if len(xs) < 5:
        return None
    w = (thr - win[ys, xs])
    return ((x0 + (xs * w).sum() / w.sum() + 0.5) / sx, (y0 + (ys * w).sum() / w.sum() + 0.5) / sy)


def fit_k1(points, size):
    """Radial k1 that best straightens a line the user clicked (points in raw image pixels)."""
    import numpy as np
    w, h = size
    cx, cy, norm = (w - 1) / 2, (h - 1) / 2, math.hypot(w / 2, h / 2)
    P = np.array(points, dtype=float)
    best = (math.inf, 0.0)
    for k1 in np.linspace(-0.6, 0.6, 241):
        # invert the sampling model approximately: undistorted point q satisfies p = q (1 + k1 |q|²)
        n = (P - [cx, cy]) / norm
        q = n.copy()
        for _ in range(6):
            r2 = (q ** 2).sum(axis=1, keepdims=True)
            q = n / (1 + k1 * r2)
        q = q * norm + [cx, cy]
        c = q - q.mean(axis=0)
        resid = np.linalg.svd(c, compute_uv=False)[-1]
        if resid < best[0]:
            best = (resid, float(k1))
    return best[1]


def _perspective_coeffs(dst, src):
    """Coefficients mapping output (dst) pixel -> input (src) pixel for PIL's PERSPECTIVE transform."""
    import numpy as np
    A, b = [], []
    for (x, y), (u, v) in zip(dst, src):
        A.append([x, y, 1, 0, 0, 0, -u * x, -u * y])
        A.append([0, 0, 0, x, y, 1, -v * x, -v * y])
        b += [u, v]
    return np.linalg.solve(np.array(A, dtype=float), np.array(b, dtype=float)).tolist()


# ====================================================================== MACROS
DEFAULT_MACROS = [
    {"name": "Aria ON (M8)", "gcode": "M8"},
    {"name": "Aria OFF (M9)", "gcode": "M9"},
    {"name": "Sblocca ($X)", "gcode": "$X"},
    {"name": "Stato ($$)", "gcode": "$$"},
]


class MacroPanel(QWidget):
    """User macro buttons: each one streams a small G-code snippet to the laser."""

    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        from PySide6.QtWidgets import QGridLayout
        self.grid_host = QWidget()
        self.grid = QGridLayout(self.grid_host)
        self.grid.setSpacing(4)
        lay.addWidget(self.grid_host)
        lay.addStretch(1)
        h = QHBoxLayout()
        self.add_btn, self.edit_btn = QPushButton(), QPushButton()
        self.add_btn.clicked.connect(lambda: self.edit(None))
        self.edit_btn.clicked.connect(self.choose_to_edit)
        h.addWidget(self.add_btn)
        h.addWidget(self.edit_btn)
        lay.addLayout(h)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self.macros = win._load_json("macros", None) or [dict(m) for m in DEFAULT_MACROS]
        self.retranslate()

    def retranslate(self):
        L = self.win.L
        self.add_btn.setText(L("Nuova macro", "New macro"))
        self.edit_btn.setText(L("Modifica / elimina...", "Edit / delete..."))
        self.hint.setText(L("Clic su un pulsante per inviare il suo G-code al laser. Puoi usare più righe.",
                            "Click a button to send its G-code to the laser. Multiple lines are allowed."))
        self.rebuild()

    def rebuild(self):
        while self.grid.count():
            w = self.grid.takeAt(0).widget()
            if w:
                w.deleteLater()
        for i, m in enumerate(self.macros):
            b = QPushButton(m["name"])
            b.setToolTip(m["gcode"])
            b.clicked.connect(lambda _=False, k=i: self.run(k))
            self.grid.addWidget(b, i // 2, i % 2)

    def save(self):
        self.win._store_json("macros", self.macros)
        self.rebuild()

    def run(self, k):
        m = self.macros[k]
        if not self.win.connected:
            self.win.toast(self.win.L("Connetti prima il laser", "Connect a laser first"))
            return
        lines = [ln for ln in m["gcode"].splitlines() if ln.split(";")[0].strip()]
        if len(lines) == 1:
            self.win.engine.command(lines[0])
        else:
            self.win.engine.send(m["gcode"])
        self.win.status(self.win.L("Macro: ", "Macro: ") + m["name"])

    def choose_to_edit(self):
        if not self.macros:
            return
        names = [m["name"] for m in self.macros]
        name, ok = QInputDialog.getItem(self, self.win.L("Macro", "Macro"), self.win.L("Quale macro?", "Which macro?"), names, 0, False)
        if ok:
            self.edit(names.index(name))

    def edit(self, k):
        from PySide6.QtWidgets import QDialog, QDialogButtonBox, QFormLayout, QLineEdit, QPlainTextEdit
        from PySide6.QtGui import QFont
        L = self.win.L
        m = self.macros[k] if k is not None else {"name": L("Nuova macro", "New macro"), "gcode": ""}
        d = QDialog(self)
        d.setWindowTitle(L("Macro", "Macro"))
        f = QFormLayout(d)
        name = QLineEdit(m["name"])
        code = QPlainTextEdit(m["gcode"])
        code.setFont(QFont("Menlo", 11))
        code.setMinimumHeight(140)
        f.addRow(L("Nome", "Name"), name)
        f.addRow("G-code", code)
        bb = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        delete = None
        if k is not None:
            delete = bb.addButton(L("Elimina", "Delete"), QDialogButtonBox.DestructiveRole)
            delete.clicked.connect(lambda: (self.macros.pop(k), self.save(), d.reject()))
        bb.accepted.connect(d.accept)
        bb.rejected.connect(d.reject)
        f.addRow(bb)
        if d.exec():
            new = {"name": name.text().strip() or "Macro", "gcode": code.toPlainText()}
            if k is None:
                self.macros.append(new)
            else:
                self.macros[k] = new
            self.save()
