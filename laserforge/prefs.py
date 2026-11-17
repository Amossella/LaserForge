"""Application settings (Edit › Settings): units, grid & snapping, editor, interface, backups.

Options familiar to users of other laser software.
"""

import json
import os
import time
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QLineEdit, QComboBox, QDialog, QDialogButtonBox, QFormLayout, QHBoxLayout, QLabel, QPushButton, QSlider, QTabWidget,
    QVBoxLayout, QWidget,
)

from .ui_common import Segmented, checkbox, dspin, hint, ispin

DEFAULT_PREFS = {
    # general
    "units": "mm",                # mm | in
    "speed_units": "mm/min",      # mm/min | mm/s | in/min
    "theme": "dark",              # dark | light
    "ui_font": 12,                # interface font size (pt)
    "icon_size": 20,              # toolbar icon size (px)
    "backup_enabled": True,
    "backup_minutes": 5,
    "backup_keep": 20,
    # editor
    "grid_show": True,
    "grid_spacing": 0.0,          # 0 = automatic with zoom
    "grid_snap": True,
    "snap_step": 1.0,             # mm
    "obj_snap": True,
    "obj_snap_px": 8,
    "click_tol": 5,               # px
    "trackpad_scroll": "zoom",    # two-finger scroll: "zoom" (like LightBurn) or "pan"
    "node_size": 7,               # px
    "curve_quality": 0.25,        # max chord error in mm (smaller = smoother)
    "nudge": 1.0,
    "nudge_shift": 10.0,
    "nudge_fine": 0.1,            # with ⌥
    # job
    "cut_selected": False,        # send only the selected shapes
    "frame_laser_on": False,      # LightBurn: "Laser On When Framing"
    "frame_power": 1.0,           # % used when framing with the laser on
    "checklist_enabled": False,   # ask the pre-job checklist before every job
    "shx_folder": "",             # folder with AutoCAD .shx single-line fonts
    "default_start_from": "last", # last | absolute | current | user  (applied at startup)
    "default_origin": -1,         # -1 = last used, 0..8 = job origin grid position
    "origin_from_selection": False,  # LightBurn: "Use Selection Origin"
    "ui_scale": 100,              # interface scale % (applied at the next start)
    "remote_enabled": False,      # phone / tablet remote control on the local network
    "remote_port": 8642,
    "remote_pin": "",             # generated on first use
    "camera_quality": 1600,       # overlay resolution (px on the long side)
}

SPEED_FACTORS = {"mm/min": 1.0, "mm/s": 1 / 60, "in/min": 1 / 25.4}
BACKUP_DIR = Path(os.environ.get("LASERFORGE_BACKUP_DIR") or Path.home() / "Library" / "Application Support" / "LaserForge" / "Backups")

THEMES = {
    "dark": {
        "canvas_bg": "#3a3b40", "ruler_bg": "#26272b", "ruler_line": "#6b6d74", "ruler_text": "#a8aab1", "ruler_corner": "#1f2023",
    },
    "light": {
        "canvas_bg": "#c9cbd0", "ruler_bg": "#eceef1", "ruler_line": "#9a9ca3", "ruler_text": "#45474d", "ruler_corner": "#dcdee2",
    },
}

# dark -> light colour mapping for the shared stylesheet and palette
LIGHT_MAP = {
    "#2b2c30": "#f3f3f5", "#dfe1e5": "#1d1e21", "#303136": "#e7e8ec", "#45474d": "#c3c5cb", "#3d3f45": "#d9dbe0",
    "#34353a": "#e3e4e8", "#c9cbd1": "#2b2c30", "#232427": "#ececef", "#a8aab1": "#55575d", "#1f2023": "#ffffff",
    "#26272b": "#f7f7f9", "#3a3c42": "#e2e4e8", "#4a4c53": "#c3c5cb", "#45474e": "#d6d8dd", "#2f3136": "#cfd1d6",
    "#2f3034": "#e9eaee", "#33353a": "#e0e2e6", "#b9bbc1": "#45474d", "#6b6d74": "#9a9ca3", "#777980": "#8a8c92",
    "#8e9097": "#6a6c72", "#24426f": "#cfe0fb",
}


def load_prefs(settings):
    import secrets
    raw = settings.value("prefs")
    try:
        stored = json.loads(raw) if raw else {}
    except (TypeError, ValueError):
        stored = {}
    p = {**DEFAULT_PREFS, **{k: v for k, v in stored.items() if k in DEFAULT_PREFS}}
    if not p.get("remote_pin"):
        p["remote_pin"] = f"{secrets.randbelow(10 ** 6):06d}"
    return p


def save_prefs(settings, prefs):
    settings.setValue("prefs", json.dumps(prefs))


def unit_factor(prefs):
    return 1 / 25.4 if prefs["units"] == "in" else 1.0


def unit_suffix(prefs):
    return " in" if prefs["units"] == "in" else " mm"


def speed_factor(prefs):
    return SPEED_FACTORS.get(prefs["speed_units"], 1.0)


# ====================================================================== theme
def apply_theme(app, theme):
    from PySide6.QtGui import QColor, QPalette

    from . import ui_common as U
    style = U.STYLE
    pal_colors = {
        QPalette.Window: "#2b2c30", QPalette.WindowText: "#dfe1e5", QPalette.Base: "#1f2023",
        QPalette.AlternateBase: "#26272b", QPalette.ToolTipBase: "#1f2023", QPalette.ToolTipText: "#dfe1e5",
        QPalette.Text: "#dfe1e5", QPalette.Button: "#3a3c42", QPalette.ButtonText: "#dfe1e5",
        QPalette.Highlight: U.ACCENT, QPalette.HighlightedText: "#ffffff", QPalette.PlaceholderText: "#777980",
    }
    if theme == "light":
        for dark, light in LIGHT_MAP.items():
            style = style.replace(dark, light)
        pal_colors = {k: LIGHT_MAP.get(v, v) for k, v in pal_colors.items()}
        pal_colors[QPalette.HighlightedText] = "#1d1e21"
        style += """
QTableWidget, QListWidget, QTreeWidget { selection-color: #1d1e21; }
QTabBar::tab:selected { color: #1d1e21; background: #ffffff; }
QMenu::item:selected { color: #ffffff; }
"""
    app.setStyle("Fusion")
    pal = QPalette()
    for role, col in pal_colors.items():
        pal.setColor(role, QColor(col))
    for role in (QPalette.Text, QPalette.ButtonText, QPalette.WindowText):
        pal.setColor(QPalette.Disabled, role, QColor("#6b6d74" if theme == "dark" else "#a0a2a8"))
    app.setPalette(pal)
    app.setStyleSheet(style)
    U.ICON_COLOR = "#dfe1e5" if theme == "dark" else "#2b2c30"


# ====================================================================== backups
def write_backup(win):
    BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    base = os.path.splitext(os.path.basename(win.project_path))[0] if win.project_path else "senza-titolo"
    path = BACKUP_DIR / f"{base}-{time.strftime('%Y%m%d-%H%M%S')}.lfproj"
    path.write_text(win.project_json(), encoding="utf-8")
    files = sorted(BACKUP_DIR.glob("*.lfproj"), key=lambda p: p.stat().st_mtime, reverse=True)
    for old in files[max(1, int(win.prefs["backup_keep"])):]:
        try:
            old.unlink()
        except OSError:
            pass
    return path


# ====================================================================== dialog
class SettingsDialog(QDialog):
    def __init__(self, win):
        super().__init__(win)
        self.win = win
        L = win.L
        self.p = dict(win.prefs)
        self.setWindowTitle(L("Impostazioni", "Settings"))
        self.setMinimumWidth(520)
        lay = QVBoxLayout(self)
        tabs = QTabWidget()
        lay.addWidget(tabs, 1)

        # ---------------- general
        g = QWidget()
        f = QFormLayout(g)
        self.units = Segmented([("mm", L("Millimetri", "Millimetres")), ("in", L("Pollici", "Inches"))])
        self.speed = Segmented([("mm/min", "mm/min"), ("mm/s", "mm/s"), ("in/min", "in/min")])
        self.theme = Segmented([("dark", L("Scuro", "Dark")), ("light", L("Chiaro", "Light"))])
        self.ui_font = ispin(12, 9, 20, suffix=" pt")
        self.icon_size = ispin(20, 14, 36, suffix=" px")
        f.addRow(L("Unità di misura", "Units"), self.units)
        f.addRow(L("Unità velocità", "Speed units"), self.speed)
        f.addRow(L("Tema", "Theme"), self.theme)
        f.addRow(L("Testo interfaccia", "Interface text"), self.ui_font)
        f.addRow(L("Icone barre strumenti", "Toolbar icons"), self.icon_size)
        self.backup = checkbox(L("Backup automatico del progetto", "Automatic project backup"))
        self.backup_min = ispin(5, 1, 120, suffix=" min")
        self.backup_keep = ispin(20, 1, 500)
        f.addRow("", self.backup)
        f.addRow(L("Ogni", "Every"), self.backup_min)
        f.addRow(L("Copie da tenere", "Copies to keep"), self.backup_keep)
        f.addRow("", hint(L("I backup si trovano in ", "Backups are stored in ") + str(BACKUP_DIR) +
                          L(" e si aprono da File › Apri backup.", " and open from File › Open Backup.")))
        self.ui_scale = ispin(100, 75, 250, 25, " %")
        f.addRow(L("Scala interfaccia", "Interface scale"), self.ui_scale)
        f.addRow("", hint(L("La scala dell'interfaccia si applica al prossimo avvio di LaserForge.", "Interface scale applies the next time LaserForge starts.")))
        tabs.addTab(g, L("Generale", "General"))

        # ---------------- editor
        e = QWidget()
        f = QFormLayout(e)
        self.grid_show = checkbox(L("Mostra griglia", "Show grid"))
        self.grid_spacing = dspin(0, 0, 500, 1, 2, " mm")
        self.grid_spacing.setSpecialValueText(L("Automatica", "Automatic"))
        self.grid_snap = checkbox(L("Aggancia alla griglia", "Snap to grid"))
        self.snap_step = dspin(1, 0.01, 100, 0.5, 2, " mm")
        self.obj_snap = checkbox(L("Aggancia agli oggetti (nodi, centri, punti medi)", "Snap to objects (nodes, centres, midpoints)"))
        self.obj_px = ispin(8, 2, 30, suffix=" px")
        self.click_tol = ispin(5, 1, 20, suffix=" px")
        self.node_size = ispin(7, 4, 20, suffix=" px")
        self.curve = QSlider(Qt.Horizontal)
        self.curve.setRange(1, 10)
        self.nudge = dspin(1, 0.01, 100, 0.5, 2, " mm")
        self.nudge_shift = dspin(10, 0.01, 500, 1, 2, " mm")
        self.nudge_fine = dspin(0.1, 0.001, 10, 0.05, 3, " mm")
        f.addRow("", self.grid_show)
        f.addRow(L("Passo griglia", "Grid spacing"), self.grid_spacing)
        f.addRow("", self.grid_snap)
        f.addRow(L("Passo aggancio", "Snap step"), self.snap_step)
        f.addRow("", self.obj_snap)
        f.addRow(L("Distanza aggancio oggetti", "Object snap distance"), self.obj_px)
        f.addRow(L("Tolleranza clic di selezione", "Click selection tolerance"), self.click_tol)
        from PySide6.QtWidgets import QComboBox as _QCB
        self.trackpad_scroll = _QCB()
        self.trackpad_scroll.addItem(L("Zoom (come LightBurn) — Shift + due dita per spostare", "Zoom (like LightBurn) — Shift + two fingers to pan"), "zoom")
        self.trackpad_scroll.addItem(L("Sposta — ⌘ + due dita per lo zoom", "Pan — ⌘ + two fingers to zoom"), "pan")
        f.addRow(L("Trackpad: scorrimento con due dita", "Trackpad: two-finger scroll"), self.trackpad_scroll)
        f.addRow(L("Dimensione nodi", "Node size"), self.node_size)
        crow = QWidget()
        h = QHBoxLayout(crow)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(QLabel(L("Veloce", "Fast")))
        h.addWidget(self.curve, 1)
        h.addWidget(QLabel(L("Precisa", "Smooth")))
        f.addRow(L("Qualità curve", "Curve quality"), crow)
        f.addRow(L("Sposta con frecce", "Arrow-key nudge"), self.nudge)
        f.addRow(L("… con ⇧ Shift", "… with ⇧ Shift"), self.nudge_shift)
        f.addRow(L("… con ⌥ Opzione", "… with ⌥ Option"), self.nudge_fine)
        from PySide6.QtWidgets import QFileDialog, QLineEdit
        self.shx = QLineEdit()
        self.shx.setPlaceholderText(L("nessuna", "none"))
        shx_btn = QPushButton(L("Scegli…", "Choose…"))
        shx_btn.clicked.connect(lambda: self.shx.setText(QFileDialog.getExistingDirectory(self, L("Cartella font SHX", "SHX font folder"), self.shx.text()) or self.shx.text()))
        shx_row = QWidget()
        sh = QHBoxLayout(shx_row)
        sh.setContentsMargins(0, 0, 0, 0)
        sh.addWidget(self.shx, 1)
        sh.addWidget(shx_btn)
        f.addRow(L("Cartella font SHX", "SHX font folder"), shx_row)
        tabs.addTab(e, "Editor")

        # ---------------- job
        j = QWidget()
        f = QFormLayout(j)
        self.cut_sel = checkbox(L("Lavora solo le forme selezionate", "Cut selected graphics only"))
        self.cam_q = Segmented([(800, L("Bassa", "Low")), (1600, L("Media", "Medium")), (2400, L("Alta", "High"))])
        self.frame_on = checkbox(L("Accendi il laser durante il frame", "Laser on when framing"))
        self.frame_power = dspin(1.0, 0.1, 20, 0.5, 1, " %")
        self.frame_on.toggled.connect(self.frame_power.setEnabled)
        f.addRow("", self.frame_on)
        f.addRow(L("Potenza durante il frame", "Framing power"), self.frame_power)
        f.addRow("", hint(L("Utile con i laser a diodo per vedere il contorno sul materiale. Tieni la potenza bassissima (0,5–2 %) per non marcare, "
                            "usa gli occhiali protettivi e attiva la modalità laser di GRBL ($32=1). Vale per il frame normale, elastico e continuo.",
                            "Useful on diode lasers to see the outline on the material. Keep the power very low (0.5–2 %) so it doesn't mark, "
                            "wear safety glasses and enable GRBL laser mode ($32=1). Applies to normal, rubber-band and continuous framing.")))
        f.addRow("", self.cut_sel)
        self.remote_on = checkbox(L("Controllo da telefono o tablet (rete locale)", "Remote control from phone or tablet (local network)"))
        self.remote_port = ispin(8642, 1024, 65535)
        self.remote_pin = QLineEdit()
        self.remote_pin.setMaxLength(12)
        f.addRow("", self.remote_on)
        f.addRow(L("Porta", "Port"), self.remote_port)
        f.addRow("PIN", self.remote_pin)
        self.remote_hint = hint("")
        f.addRow("", self.remote_hint)
        f.addRow("", hint(L("Quando è attivo, Avvia, Anteprima e Salva G-code usano solo la selezione (se c'è).",
                            "When on, Start, Preview and Save G-code use only the selection (if any).")))
        f.addRow(L("Qualità immagine camera", "Camera image quality"), self.cam_q)
        self.def_start = QComboBox()
        for k, lab in (("last", L("Come l'ultima volta", "As last time")), ("absolute", L("Coordinate assolute", "Absolute coords")),
                       ("current", L("Posizione attuale", "Current position")), ("user", L("Origine utente", "User origin"))):
            self.def_start.addItem(lab, k)
        self.def_origin = QComboBox()
        self.def_origin.addItem(L("Come l'ultima volta", "As last time"), -1)
        names = [L(n_it, n_en) for n_it, n_en in (("In alto a sinistra", "Top left"), ("In alto al centro", "Top centre"), ("In alto a destra", "Top right"),
                                                   ("Al centro a sinistra", "Middle left"), ("Centro", "Centre"), ("Al centro a destra", "Middle right"),
                                                   ("In basso a sinistra", "Bottom left"), ("In basso al centro", "Bottom centre"), ("In basso a destra", "Bottom right"))]
        for i, n in enumerate(names):
            self.def_origin.addItem(n, i)
        self.sel_origin = checkbox(L("Calcola l'origine del lavoro sulla selezione", "Use the selection for the job origin"))
        f.addRow(L("Avvia da (all'apertura)", "Start from (at startup)"), self.def_start)
        f.addRow(L("Origine lavoro (all'apertura)", "Job origin (at startup)"), self.def_origin)
        f.addRow("", self.sel_origin)
        tabs.addTab(j, L("Lavoro", "Job"))

        bb = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        self.reset_btn = bb.addButton(L("Ripristina predefiniti", "Restore defaults"), QDialogButtonBox.ResetRole)
        bb.accepted.connect(self.accept)
        bb.rejected.connect(self.reject)
        self.reset_btn.clicked.connect(lambda: self._load(DEFAULT_PREFS))
        lay.addWidget(bb)
        self._load(self.p)

    def _load(self, p):
        self.units.setValue(p["units"])
        self.speed.setValue(p["speed_units"])
        self.theme.setValue(p["theme"])
        self.ui_font.setValue(int(p["ui_font"]))
        self.icon_size.setValue(int(p["icon_size"]))
        self.backup.setChecked(bool(p["backup_enabled"]))
        self.backup_min.setValue(int(p["backup_minutes"]))
        self.backup_keep.setValue(int(p["backup_keep"]))
        self.grid_show.setChecked(bool(p["grid_show"]))
        self.grid_spacing.setValue(float(p["grid_spacing"]))
        self.grid_snap.setChecked(bool(p["grid_snap"]))
        self.snap_step.setValue(float(p["snap_step"]))
        self.obj_snap.setChecked(bool(p["obj_snap"]))
        self.obj_px.setValue(int(p["obj_snap_px"]))
        self.click_tol.setValue(int(p["click_tol"]))
        self.trackpad_scroll.setCurrentIndex(max(0, self.trackpad_scroll.findData(p.get("trackpad_scroll", "zoom"))))
        self.node_size.setValue(int(p["node_size"]))
        self.curve.setValue(max(1, min(10, round(11 - float(p["curve_quality"]) / 0.05))))
        self.nudge.setValue(float(p["nudge"]))
        self.nudge_shift.setValue(float(p["nudge_shift"]))
        self.nudge_fine.setValue(float(p["nudge_fine"]))
        self.cut_sel.setChecked(bool(p["cut_selected"]))
        self.shx.setText(p.get("shx_folder", ""))
        self.def_start.setCurrentIndex(max(0, self.def_start.findData(p.get("default_start_from", "last"))))
        self.def_origin.setCurrentIndex(max(0, self.def_origin.findData(int(p.get("default_origin", -1)))))
        self.sel_origin.setChecked(bool(p.get("origin_from_selection")))
        self.ui_scale.setValue(int(p.get("ui_scale", 100)))
        self.remote_on.setChecked(bool(p.get("remote_enabled")))
        self.remote_port.setValue(int(p.get("remote_port", 8642)))
        self.remote_pin.setText(str(p.get("remote_pin") or self.win.prefs.get("remote_pin", "")))
        from .remote import RemoteServer
        L = self.win.L
        self.remote_hint.setText(L(f"Sul telefono, collegato alla stessa rete Wi-Fi, apri http://{RemoteServer.local_ip()}:{self.remote_port.value()} e inserisci il PIN. "
                                   "Chiunque conosca il PIN e sia sulla tua rete può comandare il laser: usalo solo su reti di cui ti fidi.",
                                   f"On your phone, on the same Wi-Fi, open http://{RemoteServer.local_ip()}:{self.remote_port.value()} and enter the PIN. "
                                   "Anyone with the PIN on your network can control the laser: use it only on networks you trust."))
        self.cam_q.setValue(int(p["camera_quality"]))
        self.frame_on.setChecked(bool(p["frame_laser_on"]))
        self.frame_power.setValue(float(p["frame_power"]))
        self.frame_power.setEnabled(bool(p["frame_laser_on"]))

    def values(self):
        return {
            "units": self.units.value(), "speed_units": self.speed.value(), "theme": self.theme.value(),
            "ui_font": self.ui_font.value(), "icon_size": self.icon_size.value(),
            "backup_enabled": self.backup.isChecked(), "backup_minutes": self.backup_min.value(), "backup_keep": self.backup_keep.value(),
            "grid_show": self.grid_show.isChecked(), "grid_spacing": self.grid_spacing.value(), "grid_snap": self.grid_snap.isChecked(),
            "snap_step": self.snap_step.value(), "obj_snap": self.obj_snap.isChecked(), "obj_snap_px": self.obj_px.value(),
            "click_tol": self.click_tol.value(), "trackpad_scroll": self.trackpad_scroll.currentData(), "node_size": self.node_size.value(),
            "curve_quality": round((11 - self.curve.value()) * 0.05, 3),
            "nudge": self.nudge.value(), "nudge_shift": self.nudge_shift.value(), "nudge_fine": self.nudge_fine.value(),
            "cut_selected": self.cut_sel.isChecked(), "camera_quality": self.cam_q.value(),
            "frame_laser_on": self.frame_on.isChecked(), "frame_power": self.frame_power.value(),
            "shx_folder": self.shx.text().strip(), "default_start_from": self.def_start.currentData(),
            "default_origin": self.def_origin.currentData(), "origin_from_selection": self.sel_origin.isChecked(),
            "ui_scale": self.ui_scale.value(), "remote_enabled": self.remote_on.isChecked(),
            "remote_port": self.remote_port.value(), "remote_pin": self.remote_pin.text().strip() or self.win.prefs.get("remote_pin", ""),
        }



# ====================================================================== settings bundle
BUNDLE_KEYS = ["prefs", "opt", "machine", "layers", "devices", "macros", "positions", "shortcuts", "imagePresets",
               "checklist", "camera", "cylinder", "view", "lang", "startFrom", "origin", "userOrigin"]


def export_bundle(win, path):
    """One .lfbundle (zip) with every preference, the material library and the art library."""
    import zipfile
    from .extras import ART_DIR
    from .library import LIB_PATH, save_library
    data = {k: win.settings.value(k) for k in BUNDLE_KEYS if win.settings.value(k) is not None}
    data["prefs"] = json.dumps(win.prefs)
    data["machine"] = json.dumps(win.engine.machine_dict())
    data["layers"] = json.dumps(win.engine.layer_dicts())
    save_library(win.library_panel.entries)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("settings.json", json.dumps({"app": "LaserForge", "version": 1, "values": data}, indent=1))
        if LIB_PATH.exists():
            z.write(LIB_PATH, "materials.json")
        if ART_DIR.exists():
            for f in ART_DIR.glob("*.json"):
                z.write(f, "art/" + f.name)


def import_bundle(win, path):
    import zipfile
    from .extras import ART_DIR
    from .library import LIB_PATH, load_library
    with zipfile.ZipFile(path) as z:
        values = json.loads(z.read("settings.json"))["values"]
        for k, v in values.items():
            if k in BUNDLE_KEYS:
                win.settings.setValue(k, v)
        if "materials.json" in z.namelist():
            LIB_PATH.parent.mkdir(parents=True, exist_ok=True)
            LIB_PATH.write_bytes(z.read("materials.json"))
        ART_DIR.mkdir(parents=True, exist_ok=True)
        for n in z.namelist():
            if n.startswith("art/") and n.endswith(".json") and "/" not in n[4:] and ".." not in n:
                (ART_DIR / n[4:]).write_bytes(z.read(n))
    # apply what can change live
    win.prefs = load_prefs(win.settings)
    machine = win._load_json("machine", None)
    if machine:
        win.engine.update_machine(machine)
    for i, l in enumerate((win._load_json("layers", None) or [])[:len(win.engine.layers)]):
        win.engine.update_layer(i, l)
    win.opt.update(win._load_json("opt", {}))
    win.library_panel.entries = load_library()
    win.library_panel.refresh()
    win.art_panel.reload()
    win.macro_panel.macros = win._load_json("macros", None) or win.macro_panel.macros
    win.macro_panel.rebuild()
    win.apply_shortcuts()
    win.apply_prefs(theme_changed=True)
    win.cuts.load(win.doc.active_layer)
    win.move_panel.load()
    win.laser_panel.refresh_devices()
