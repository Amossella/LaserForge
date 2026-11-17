"""LaserForge main window: native macOS menu bar, toolbars, docks and all editing / laser actions."""

import json
import math
import os
from pathlib import Path

from PySide6.QtCore import QByteArray, QSettings, QSize, Qt, QTimer, QUrl
from PySide6.QtGui import QAction, QActionGroup, QColor, QDesktopServices, QIcon, QKeySequence
from PySide6.QtWidgets import (
    QApplication, QDockWidget, QFileDialog, QInputDialog, QLabel, QMainWindow, QMessageBox, QStatusBar, QToolBar, QToolButton,
    QWidget,
)

from . import dialogs as D
from . import model as M
from .canvas import Canvas
from .defs import APP_MENU_LABELS, EXTRA_LABELS, I18N, MAIN_TOOLBAR, MENUS, MODBAR, TOOLBOX
from .document import Document
from .engine import Engine
from .extras import ArtLibraryPanel, CameraPanel, MacroPanel
from .props import PagesPanel, PropertiesPanel, undo_history_dialog
from . import prefs as P
from . import devices as DV
from .panels import ORIGIN_POS, ConsolePanel, CutsPanel, LaserPanel, MovePanel, ObjectsPanel
from .library import LibraryPanel
from .ui_common import ICON_SIZE, dspin, svg_icon

APP_VERSION = "0.4.0-beta"
EDIT_TEXT_IDS = {"cut", "copy", "paste", "selectAll", "undo", "redo"}
TOOL_OF = {"toolScissors": "scissors", "toolTrim": "trim", "toolTabs": "tabs", "toolStartPoints": "start",
           "toolTwoPoint": "twopoint", "toolDeform": "deform", "toolWarp": "warp", "toolSelect": "select", "toolNodes": "nodes", "toolPoly": "poly", "toolRect": "rect", "toolEllipse": "ellipse",
           "toolPolygon": "polygon", "toolStar": "star", "toolText": "text", "toolPan": "pan", "toolPosition": "position",
           "measure": "measure"}
EXTRA_I18N = {
    "it": {"centered": "Centrato", "rotateImageNA": "Le immagini si ruotano solo di multipli di 90°"},
    "en": {"centered": "Centered", "rotateImageNA": "Images rotate only in 90° steps"},
}


def qt_shortcut(k: str):
    if not k:
        return []
    k = k.replace("Esc", "Escape").replace("Alt+Ctrl+", "Ctrl+Alt+")
    if k == "Del":
        return [QKeySequence(QKeySequence.Delete), QKeySequence(Qt.Key_Backspace)]
    seqs = [QKeySequence(k)]
    if k == "Ctrl+=":
        seqs.append(QKeySequence("Ctrl++"))
    return seqs


class JobPath(list):
    """Outline path for the job; may carry manual tab positions (fractions of its length) and ordering metadata."""
    tabs = None
    group = None
    priority = 0
    fixed = False


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        ini = os.environ.get("LASERFORGE_SETTINGS")
        self.settings = QSettings(ini, QSettings.IniFormat) if ini else QSettings("LaserForge", "LaserForge")
        self.lang = self.settings.value("lang", "it")
        self.engine = Engine()
        self.doc = Document()
        self.clipboard = []
        self.gcode = ""
        self.project_path = None
        self.prefs = P.load_prefs(self.settings)
        self.snap_on, self.snap_grid = bool(self.prefs["grid_snap"]), float(self.prefs["snap_step"])
        M.CURVE_TOL = float(self.prefs["curve_quality"])
        P.apply_theme(QApplication.instance(), self.prefs["theme"])
        self._apply_font()
        self.lock_aspect = True
        self.polygon_sides = 6
        self.view = {"filled": False, "grid": True, "rulers": True, **self._load_json("view", {})}
        self.view["grid"] = bool(self._load_json("prefs", {}).get("grid_show", self.view["grid"]))
        self.opt = {"innerFirst": True, "nearest": True, "order": "layer", "bestStart": True, "bestDirection": True,
                    "direction": "keep", "removeOverlap": False, **self._load_json("opt", {})}
        self.start_from = self.settings.value("startFrom", "absolute")
        self.job_origin = int(self.settings.value("origin", 0))
        _p = self._load_json("prefs", {})
        if _p.get("default_start_from", "last") != "last":
            self.start_from = _p["default_start_from"]
        if self.start_from == "user" and not self.settings.value("userOriginSet", False, type=bool):
            self.start_from = "absolute"        # a user origin that was never set would throw the job off the bed
        if int(_p.get("default_origin", -1)) >= 0:
            self.job_origin = int(_p["default_origin"])
        self.user_origin = tuple(self._load_json("userOrigin", [0, 0]))
        self.print_cut = None
        self.project_notes = {}
        self.connected = False
        self.machine_pos = (0.0, 0.0)
        self.camera_cal = {"opacity": 70, **self._load_json("camera", {})}
        self.camera_overlay = None
        self.camera_overlay_visible = True
        machine = self._load_json("machine", None)
        if machine:
            self.engine.update_machine(machine)
        layers = self._load_json("layers", None)
        if layers:
            for i, l in enumerate(layers[:len(self.engine.layers)]):
                self.engine.update_layer(i, l)

        self.setWindowTitle("LaserForge")
        self.setUnifiedTitleAndToolBarOnMac(False)
        self.setDockOptions(QMainWindow.AnimatedDocks | QMainWindow.AllowTabbedDocks)
        self.canvas = Canvas(self)
        self.setCentralWidget(self.canvas)

        self.actions = {}
        self._build_actions()
        self._build_menus()
        self._build_toolbars()
        self._build_docks()
        self._build_statusbar()

        self.doc.changed.connect(self._on_doc_changed)
        self.doc.selectionChanged.connect(self._on_selection_changed)
        self.doc.dirtyChanged.connect(lambda d: self._update_title())
        self.canvas.mouseMoved.connect(lambda x, y: self.lbl_mouse.setText(f"{self.fmt_num(x)}, {self.fmt_num(y)}{P.unit_suffix(self.prefs)}"))
        self.canvas.status.connect(self.status)
        self.canvas.zoomChanged.connect(lambda z: self.lbl_zoom.setText(f"{self.canvas.zoom_percent():.0f}%"))
        self.canvas.requestText.connect(lambda x, y: D.text_dialog(self, (x, y)))
        self.canvas.editShape.connect(self._edit_shape)
        self.canvas.positionLaser.connect(self._position_laser)

        self.timer = QTimer(self)
        self.timer.timeout.connect(self._poll)
        self.timer.start(300)
        from .remote import RemoteServer
        self.remote = RemoteServer(self)
        self.backup_timer = QTimer(self)
        self.backup_timer.timeout.connect(self._auto_backup)
        self._last_backup_state = None
        self.apply_prefs(initial=True)

        self.cuts.load(0)
        self.move_panel.load()
        self.laser_panel.refresh_origin()
        self._update_title()
        self.refresh_checks()
        avail = self.screen().availableGeometry() if self.screen() else None
        if avail is not None:
            self.resize(int(avail.width() * 0.92), int(avail.height() * 0.92))
        else:
            self.resize(1500, 900)
        geo = self.settings.value("geometry")
        if isinstance(geo, QByteArray):
            self.restoreGeometry(geo)
        state = self.settings.value("windowState")
        if isinstance(state, QByteArray):
            self.restoreState(state, self.LAYOUT_VERSION)     # a layout saved by an older version is ignored
        QTimer.singleShot(0, self._style_dock_tabs)
        QTimer.singleShot(0, self.fit_to_screen)
        self.status(self.tr("ready"))
        QTimer.singleShot(0, self.canvas.zoom_page)

    # ---------------------------------------------------------------- machine origin (LightBurn: Device Settings › Origin)
    def to_machine(self, p):
        """Design/canvas point (Y down, top = back of the machine) -> machine coordinates for the chosen origin corner.
        The mapping is its own inverse, so the same function converts machine positions back to the canvas."""
        W, H = self.workspace()
        o = self.engine.machine.origin
        x, y = p
        return (x if o in ("fl", "rl") else W - x, H - y if o in ("fl", "fr") else y)

    to_canvas = to_machine

    def jog_signs(self):
        o = self.engine.machine.origin
        return (1 if o in ("fl", "rl") else -1, -1 if o in ("fl", "fr") else 1)

    def job_bounds(self, job):
        """Bounding box (machine coordinates) of everything the job burns."""
        pts = [p for l in job["layers"] for path in l["paths"] for p in path]
        for r in job["rasters"]:
            pts += [(r["x"], r["y"]), (r["x"] + r["w"], r["y"] + r["h"])]
        return M.path_box(pts) if pts else None

    def confirm_in_area(self, b):
        """Warn when a job/frame (machine bbox) leaves the work area. Returns False to cancel."""
        if not b:
            return True
        W, H = self.workspace()
        over = max(0.0, -b[0], -b[1], b[0] + b[2] - W, b[1] + b[3] - H)
        if over < 0.05:
            return True
        names = {"absolute": self.tr("sfAbsolute"), "current": self.tr("sfCurrent"), "user": self.tr("sfUser")}
        box = QMessageBox(self)
        box.setIcon(QMessageBox.Warning)
        box.setWindowTitle("LaserForge")
        box.setText(self.L(f"Il lavoro esce dall'area di lavoro di {over:.1f} mm: la testa andrebbe contro i finecorsa.",
                           f"The job goes {over:.1f} mm outside the work area: the head would hit the limit switches."))
        box.setInformativeText(self.L(f"«Avvia da» è impostato su «{names.get(self.start_from, self.start_from)}». "
                                      "Con «Assolute» il lavoro va esattamente dove l'hai messo sulla griglia.",
                                      f"“Start from” is set to “{names.get(self.start_from, self.start_from)}”. "
                                      "With “Absolute” the job goes exactly where you placed it on the grid."))
        to_abs = box.addButton(self.L("Usa «Assolute»", "Use “Absolute”"), QMessageBox.AcceptRole) if self.start_from != "absolute" else None
        go = box.addButton(self.L("Continua comunque", "Continue anyway"), QMessageBox.DestructiveRole)
        box.addButton(self.tr("cancel"), QMessageBox.RejectRole)
        box.exec()
        if to_abs is not None and box.clickedButton() is to_abs:
            self.set_start_from("absolute")
            return "retry"
        return box.clickedButton() is go

    def goto_canvas(self, x, y):
        mx, my = self.to_machine((x, y))
        self.engine.command(f"G0 X{mx:.3f} Y{my:.3f}")

    def fit_to_screen(self):
        """Keep the whole window (title bar included) inside the visible area: menu bar and Dock excluded."""
        scr = self.screen()
        if scr is None or self.isFullScreen():
            return
        avail = scr.availableGeometry()
        frame = self.frameGeometry()
        extra_w, extra_h = frame.width() - self.width(), frame.height() - self.height()
        w = min(self.width(), avail.width() - extra_w)
        h = min(self.height(), avail.height() - extra_h)
        if (w, h) != (self.width(), self.height()):
            self.resize(w, h)
        frame = self.frameGeometry()
        x = min(max(frame.x(), avail.x()), avail.right() - frame.width() + 1)
        y = min(max(frame.y(), avail.y()), avail.bottom() - frame.height() + 1)
        if (x, y) != (frame.x(), frame.y()):
            self.move(x, y)

    # ================================================================ helpers
    def tr(self, key):
        return EXTRA_I18N.get(self.lang, {}).get(key) or I18N.get(self.lang, {}).get(key) or I18N["it"].get(key) or key

    def L(self, it, en):
        return en if self.lang == "en" else it

    def _load_json(self, key, default):
        raw = self.settings.value(key)
        if raw is None:
            return default
        try:
            return json.loads(raw)
        except (TypeError, ValueError):
            return default

    def _store_json(self, key, value):
        self.settings.setValue(key, json.dumps(value))

    def status(self, msg):
        self.lbl_status.setText(msg)

    def toast(self, msg):
        self.statusBar().showMessage(msg, 3500)

    def workspace(self):
        return (self.engine.machine.workspace_x, self.engine.machine.workspace_y)

    def layer_color(self, idx):
        layers = self.engine.layers
        return layers[idx].color if 0 <= idx < len(layers) else "#000000"

    def layer_interval(self, idx):
        layers = self.engine.layers
        return max(0.05, layers[idx].line_interval if 0 <= idx < len(layers) else 0.1)

    def invalidate_gcode(self):
        self.gcode = ""

    def action_label(self, aid):
        def find(items):
            for it in items:
                if isinstance(it, list) and it[0] == aid:
                    return it
                if isinstance(it, dict) and "items" in it:
                    f = find(it["items"])
                    if f:
                        return f
            return None
        for m in MENUS:
            f = find(m["items"])
            if f:
                return f[2] if self.lang == "en" else f[1]
        if aid in EXTRA_LABELS:
            return self.L(*EXTRA_LABELS[aid])
        return aid

    # ---------------------------------------------------------------- settings helpers
    def unit_factor(self):
        return P.unit_factor(self.prefs)

    def fmt_num(self, mm):
        return f"{mm * self.unit_factor():.{4 if self.prefs['units'] == 'in' else 2}f}"

    def fmt_len(self, mm):
        return self.fmt_num(mm) + P.unit_suffix(self.prefs)

    def theme_colors(self):
        return P.THEMES.get(self.prefs["theme"], P.THEMES["dark"])

    def _apply_font(self):
        app = QApplication.instance()
        f = app.font()
        f.setPointSize(int(self.prefs["ui_font"]))
        app.setFont(f)

    def settings_dialog(self):
        dlg = P.SettingsDialog(self)
        if dlg.exec():
            old_theme = self.prefs["theme"]
            self.prefs.update(dlg.values())
            P.save_prefs(self.settings, self.prefs)
            self.apply_prefs(theme_changed=self.prefs["theme"] != old_theme)

    def apply_prefs(self, initial=False, theme_changed=False):
        pr = self.prefs
        self.snap_on, self.snap_grid = bool(pr["grid_snap"]), float(pr["snap_step"])
        self.view["grid"] = bool(pr["grid_show"])
        M.CURVE_TOL = float(pr["curve_quality"])
        M.SHX_FOLDER[0] = pr.get("shx_folder", "")
        M._text_cache.clear()
        if theme_changed:
            P.apply_theme(QApplication.instance(), pr["theme"])
            from . import ui_common as U
            U._icon_cache.clear()
            for b in MAIN_TOOLBAR + TOOLBOX + MODBAR:
                if isinstance(b, list):
                    self.actions[b[0]].setIcon(svg_icon(b[1]))
            self.actions["lockAspect"].setIcon(svg_icon("lock"))
        if not initial:
            self._apply_font()
        size = QSize(int(pr["icon_size"]), int(pr["icon_size"]))
        for tb in (self.tb_main, self.tb_mod, self.tb_tools):
            tb.setIconSize(size)
        u, suf = self.unit_factor(), P.unit_suffix(pr)
        dec = 4 if pr["units"] == "in" else 2
        for k in ("x", "y", "w", "h"):
            sp = self.tx[k]
            sp.blockSignals(True)
            sp.setDecimals(dec)
            sp.setSuffix(suf)
            sp.setRange(-10000 * u, 10000 * u)
            sp.setSingleStep(0.1 if pr["units"] == "mm" else 0.01)
            sp.blockSignals(False)
        self.cuts.set_speed_units(pr["speed_units"])
        self._style_snap()
        if pr.get("remote_enabled"):
            try:
                url = self.remote.start(int(pr["remote_port"]), pr["remote_pin"])
                if not initial:
                    self.toast(self.L("Controllo remoto attivo: ", "Remote control on: ") + url)
            except OSError as exc:
                self.toast(self.L("Controllo remoto non avviato: ", "Remote control not started: ") + str(exc))
        else:
            self.remote.stop()
        self.backup_timer.stop()
        if pr["backup_enabled"]:
            self.backup_timer.start(int(pr["backup_minutes"]) * 60000)
        self.update_transform_fields()
        self.refresh_checks()
        self.canvas.update()

    def _auto_backup(self):
        state = (self.doc.dirty, len(self.doc.undo_stack), self.doc.next_id)
        if not self.doc.dirty or state == self._last_backup_state:
            return
        try:
            P.write_backup(self)
            self._last_backup_state = state
        except OSError as exc:
            self.toast(str(exc))

    def export_bundle(self):
        path, _ = QFileDialog.getSaveFileName(self, self.action_label("exportBundle"), str(Path.home() / "LaserForge.lfbundle"), "LaserForge bundle (*.lfbundle)")
        if path:
            P.export_bundle(self, path)
            self.toast(self.L("Pacchetto esportato: ", "Bundle exported: ") + os.path.basename(path))

    def import_bundle(self):
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("importBundle"), str(Path.home()), "LaserForge bundle (*.lfbundle)")
        if not path:
            return
        try:
            P.import_bundle(self, path)
        except Exception as exc:
            self.toast(self.L("Pacchetto non valido: ", "Invalid bundle: ") + str(exc))
            return
        self.toast(self.L("Impostazioni importate", "Settings imported"))

    def open_backup(self):
        P.BACKUP_DIR.mkdir(parents=True, exist_ok=True)
        if not self.confirm_discard():
            return
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("openBackup"), str(P.BACKUP_DIR), "LaserForge (*.lfproj)")
        if path:
            self.open_path(path, confirmed=True)
            self.project_path = None
            self.doc.set_dirty(True)
            self._update_title()

    def need_sel(self, minimum=1):
        sel = self.doc.selected_shapes()
        if len(sel) < minimum:
            if minimum <= 1:
                self.toast(self.tr("selectFirst"))
            else:
                self.toast(self.L(f"Seleziona almeno {minimum} oggetti", f"Select at least {minimum} objects"))
            return []
        return sel

    # ================================================================ actions / menus
    def _action_map(self):
        d = self
        return {
            "new": d.new_project, "open": d.open_project, "import": d.import_file, "save": lambda: d.save_project(False),
            "saveAs": lambda: d.save_project(True), "exportSvg": d.export_svg, "saveGcode": d.save_gcode,
            "exportPrefs": d.export_prefs, "importPrefs": d.import_prefs, "exit": d.close,
            "undo": d.doc.undo, "redo": d.doc.redo,
            "selectAll": lambda: d.select_by(lambda s: True),
            "invertSelection": d.invert_selection,
            "selectOpen": lambda: d.select_by(lambda s: s["type"] == "poly" and not s.get("closed")),
            "selectContained": d.select_contained,
            "selectLayer": lambda: d.select_by(lambda s: s.get("layer") == d.doc.active_layer),
            "cut": d.cut_sel, "copy": d.copy_sel, "duplicate": d.duplicate_sel, "paste": lambda: d.paste(False),
            "pasteInPlace": lambda: d.paste(True), "delete": d.delete_selected,
            "deleteDuplicates": d.delete_duplicates, "closePath": d.close_path, "convertToPath": d.convert_to_path,
            "autoJoin": d.auto_join, "breakApart": d.break_apart,
            "lockSel": lambda: d.set_locked(True), "unlockSel": lambda: d.set_locked(False),
            "deviceSettings": lambda: D.device_settings_dialog(d), "settings": d.settings_dialog,
            "openBackup": d.open_backup, "exportBundle": d.export_bundle, "importBundle": d.import_bundle,
            "toolSelect": d.escape_or_select, "toolNodes": lambda: d.set_tool("nodes"), "toolPoly": lambda: d.set_tool("poly"),
            "toolRect": lambda: d.set_tool("rect"), "toolEllipse": lambda: d.set_tool("ellipse"),
            "toolPolygon": lambda: D.polygon_dialog(d), "toolStar": lambda: d.set_tool("star"), "toolText": lambda: d.set_tool("text"),
            "toolPan": lambda: d.set_tool("pan"), "toolPosition": lambda: d.set_tool("position"), "measure": lambda: d.set_tool("measure"),
            "shapeTriangle": lambda: d.insert_shape("triangle"), "shapePentagon": lambda: d.insert_shape("pentagon"),
            "shapeOctagon": lambda: d.insert_shape("octagon"), "shapeStar": lambda: d.insert_shape("star"),
            "shapeDualStar": lambda: d.insert_shape("dualStar"),
            "offset": lambda: D.offset_dialog(d), "weld": lambda: d.geometry_op("weld", {}, minimum=2),
            "union": lambda: d.geometry_op("union", {}, minimum=2), "subtract": lambda: d.geometry_op("subtract", {}, minimum=2),
            "intersect": lambda: d.geometry_op("intersect", {}, minimum=2), "roundCorners": lambda: D.round_corners_dialog(d),
            "rubberBand": lambda: d.geometry_op("hull", {}, replace=False),
            "gridArray": lambda: D.grid_array_dialog(d), "circularArray": lambda: D.circular_array_dialog(d),
            "copyAlongPath": lambda: D.copy_along_path_dialog(d), "mirrorLine": d.mirror_across_line,
            "createQR": lambda: D.qr_dialog(d), "traceImage": lambda: D.trace_image_dialog(d),
            "adjustImage": lambda: D.adjust_image_dialog(d), "cropImage": d.crop_image, "applyMask": d.crop_image, "booleanAssistant": lambda: D.boolean_assistant(d),
            "group": d.group_sel, "ungroup": d.ungroup_sel, "mirrorH": lambda: d.mirror_sel("h"), "mirrorV": lambda: d.mirror_sel("v"),
            "rotate90cw": lambda: d.rotate_selection(90), "rotate90ccw": lambda: d.rotate_selection(-90),
            "alignLeft": lambda: d.align_sel("left"), "alignRight": lambda: d.align_sel("right"), "alignTop": lambda: d.align_sel("top"),
            "alignBottom": lambda: d.align_sel("bottom"), "alignCenterH": lambda: d.align_sel("centerH"),
            "alignCenterV": lambda: d.align_sel("centerV"), "alignCenters": lambda: d.align_sel("centers"),
            "distH": lambda: d.distribute_sel("h", False), "distV": lambda: d.distribute_sel("v", False),
            "distHC": lambda: d.distribute_sel("h", True), "distVC": lambda: d.distribute_sel("v", True),
            "moveUL": lambda: d.move_sel_to("UL"), "moveUR": lambda: d.move_sel_to("UR"), "moveLL": lambda: d.move_sel_to("DL"),
            "moveLR": lambda: d.move_sel_to("DR"), "moveCenter": lambda: d.move_sel_to("C"), "moveLeft": lambda: d.move_sel_to("L"),
            "moveRight": lambda: d.move_sel_to("R"), "moveTop": lambda: d.move_sel_to("U"), "moveBottom": lambda: d.move_sel_to("D"),
            "moveToLaser": lambda: d.move_sel_to("laser"),
            "sameWidth": lambda: d.same_size("w"), "sameHeight": lambda: d.same_size("h"), "nesting": lambda: D.nesting_dialog(d),
            "pushFront": lambda: d.reorder("front"), "pushForward": lambda: d.reorder("forward"),
            "pushBackward": lambda: d.reorder("backward"), "pushBack": lambda: d.reorder("back"),
            "lockAspect": d.toggle_lock_aspect,
            "materialTest": lambda: D.material_test_dialog(d), "intervalTest": lambda: D.test_pattern_dialog(d, "interval"),
            "focusTest": lambda: D.test_pattern_dialog(d, "focus"),
            "laserUL": lambda: d.laser_to("UL"), "laserUR": lambda: d.laser_to("UR"), "laserLL": lambda: d.laser_to("DL"),
            "laserLR": lambda: d.laser_to("DR"), "laserCenter": lambda: d.laser_to("C"),
            "frame": d.frame_job, "unlock": d.engine.unlock, "frameRubber": d.rubber_band_frame, "frameContinuous": d.toggle_continuous_frame, "homeLaser": d.home_laser, "gotoOrigin": lambda: d.engine.command("G0 X0 Y0"),
            "setUserOrigin": d.set_user_origin, "optimizeSettings": lambda: D.optimize_dialog(d),
            "airAllOn": lambda: d.set_all_layers(lambda l: {"air_assist": "on"} if l.air_assist.value == "off" else None),
            "airAllOff": lambda: d.set_all_layers(lambda l: {"air_assist": "off"}),
            "airInvert": lambda: d.set_all_layers(lambda l: {"air_assist": "on" if l.air_assist.value == "off" else "off"}),
            "layersAllOn": lambda: d.set_all_layers(lambda l: {"enabled": True}),
            "layersAllOff": lambda: d.set_all_layers(lambda l: {"enabled": False}),
            "layersInvert": lambda: d.set_all_layers(lambda l: {"enabled": not l.enabled}),
            "rotarySetup": lambda: D.rotary_dialog(d), "centerFinder": lambda: D.center_finder_dialog(d),
            "printAndCut": lambda: D.print_and_cut_dialog(d),
            "textOnPath": lambda: D.text_on_path_dialog(d), "varText": lambda: D.variable_text_dialog(d),
            "cameraCapture": d.camera_capture, "cameraAlign": lambda: D.camera_align_dialog(d),
            "cameraImport": lambda: d.camera_panel.import_image(),
            "cameraLens": lambda: D.lens_correction_dialog(d), "cameraTargets": lambda: D.target_alignment_dialog(d),
            "toolScissors": lambda: d.set_tool("scissors"), "toolTrim": lambda: d.set_tool("trim"),
            "toolTabs": lambda: d.set_tool("tabs"), "toolStartPoints": lambda: d.set_tool("start"),
            "toolTwoPoint": lambda: d.set_tool("twopoint"), "toolDeform": lambda: d.set_tool("deform"),
            "toolWarp": lambda: d.set_tool("warp"),
            "fillet": lambda: D.fillet_dialog(d), "dogbone": lambda: D.dogbone_dialog(d),
            "slotResize": lambda: D.slot_resize_dialog(d), "taper": lambda: D.taper_dialog(d),
            "tangentCircles": lambda: D.tangent_circles(d),
            "clearTabs": lambda: d.set_shape_attr("tabs", None), "resetStart": d.reset_start_points,
            "removeCrop": d.remove_crop, "flattenCrop": d.flatten_crop,
            "convertBitmap": lambda: D.convert_to_bitmap_dialog(d), "replaceImage": lambda: d.replace_image(False),
            "replaceImageFit": lambda: d.replace_image(True), "reloadImage": d.reload_image,
            "editHotkeys": lambda: D.shortcuts_editor(d),
            "closeTolerance": lambda: D.close_tolerance_dialog(d), "deleteOverlaps": d.delete_overlapping,
            "selectNested": d.select_nested, "selectCircles": d.select_same_circles, "selectSmaller": d.select_smaller,
            "cylinder": lambda: D.cylinder_dialog(d), "createBarcode": lambda: D.barcode_dialog(d),
            "devices": lambda: DV.devices_dialog(d), "newDevice": lambda: D.new_device_wizard(d),
            "feeder": lambda: D.feeder_dialog(d), "controllerFiles": lambda: D.controller_files_dialog(d), "checklist": lambda: DV.checklist_editor(d), "projectNotes": lambda: DV.notes_dialog(d),
            "undoHistory": lambda: undo_history_dialog(d), "flattenArray": d.flatten_arrays,
            "winMacros": lambda: d.toggle_dock(d.dock_macros),
            "winProps": lambda: d.toggle_dock(d.dock_props), "winPages": lambda: d.toggle_dock(d.dock_pages),
            "winArt": lambda: d.toggle_dock(d.dock_art), "winCamera": lambda: d.toggle_dock(d.dock_camera),
            "machineSettings": lambda: D.machine_settings_dialog(d), "calibrateAxis": lambda: D.calibrate_axis_dialog(d), "controllerInfo": lambda: d.send_and_show_console("$I"),
            "boxMaker": lambda: D.box_maker_dialog(d), "gearMaker": lambda: D.gear_maker_dialog(d),
            "docs": lambda: QDesktopServices.openUrl(QUrl("https://docs.lightburnsoftware.com/")),
            "winCuts": lambda: d.toggle_dock(d.dock_cuts), "winLaser": lambda: d.toggle_dock(d.dock_laser),
            "winMove": lambda: d.toggle_dock(d.dock_move), "winConsole": lambda: d.toggle_dock(d.dock_console),
            "winObjects": lambda: d.toggle_dock(d.dock_objects), "winLibrary": lambda: d.toggle_dock(d.dock_library),
            "winMainToolbar": lambda: d.toggle_dock(d.tb_main), "winModbar": lambda: d.toggle_dock(d.tb_mod),
            "winToolbox": lambda: d.toggle_dock(d.tb_tools), "winPalette": lambda: d.toggle_dock(d.tb_palette),
            "resetLayout": d.reset_layout,
            "viewFilled": lambda: d.toggle_view("filled"), "showGrid": lambda: d.toggle_view("grid"),
            "showRulers": lambda: d.toggle_view("rulers"),
            "zoomIn": lambda: d.canvas.zoom_step(1.25), "zoomOut": lambda: d.canvas.zoom_step(0.8),
            "zoomPage": d.canvas.zoom_page, "zoomSel": d.zoom_sel,
            "minimize": d.showMinimized, "fullscreen": d.toggle_fullscreen, "resetLayout": d.reset_layout,
            "langIt": lambda: d.set_lang("it"), "langEn": lambda: d.set_lang("en"),
            "shortcuts": lambda: D.shortcuts_dialog(d), "about": lambda: D.about_dialog(d),
            "startLaser": d.start_laser, "pauseLaser": d.engine.pause, "resumeLaser": d.engine.resume,
            "stopLaser": d.stop_laser, "preview": lambda: D.preview_dialog(d),
        }

    def checks(self):
        return {
            "lockAspect": self.lock_aspect, "viewFilled": self.view["filled"], "frameContinuous": getattr(self, "frame_continuous", False), "showGrid": self.view["grid"],
            "showRulers": self.view["rulers"], "langIt": self.lang == "it", "langEn": self.lang == "en",
            **{aid: self.canvas.tool == tool for aid, tool in TOOL_OF.items()},
        }

    def run(self, aid):
        fn = self.funcs.get(aid)
        if fn:
            self.doc.current_label = self.action_label(aid).replace("...", "")
            try:
                fn()
            finally:
                self.doc.current_label = None
        else:
            self.toast(self.tr("notAvailable"))

    def default_shortcut(self, aid):
        seqs = qt_shortcut(self._default_keys.get(aid, ""))
        return seqs[0] if seqs else QKeySequence()

    def apply_shortcuts(self):
        """Default shortcuts with the user's overrides from Edit › Keyboard Shortcuts."""
        overrides = self._load_json("shortcuts", {})
        for aid, a in self.actions.items():
            if aid in overrides:
                a.setShortcuts([QKeySequence(overrides[aid])] if overrides[aid] else [])
            else:
                a.setShortcuts(qt_shortcut(self._default_keys.get(aid, "")))
        if hasattr(self, "recent_menu"):
            self._retitle_actions()

    def _build_actions(self):
        self._default_keys = {}
        self.funcs = self._action_map()
        checkable = {"lockAspect", "viewFilled", "showGrid", "showRulers", "langIt", "langEn", *TOOL_OF,
                     "winCuts", "winLaser", "winMove", "winConsole", "winObjects", "winLibrary", "winArt", "winCamera", "winProps", "winPages", "winMacros", "frameContinuous",
                     "winMainToolbar", "winModbar", "winToolbox", "winPalette"}

        def walk(items):
            for it in items:
                if isinstance(it, list):
                    aid, _, _, key = it
                    if aid in self.actions:
                        continue
                    a = QAction(self)
                    a.setShortcuts(qt_shortcut(key))
                    self._default_keys[aid] = key
                    a.setMenuRole(QAction.NoRole)
                    a.setCheckable(aid in checkable)
                    a.triggered.connect(lambda _=False, i=aid: self.run(i))
                    self.actions[aid] = a
                elif isinstance(it, dict) and "items" in it:
                    walk(it["items"])

        for m in MENUS:
            walk(m["items"])
        for aid in list(EXTRA_LABELS) + [b[0] for b in MAIN_TOOLBAR + TOOLBOX + MODBAR if isinstance(b, list)]:
            if aid not in self.actions:
                a = QAction(self)
                a.setCheckable(aid in checkable)
                a.triggered.connect(lambda _=False, i=aid: self.run(i))
                self.actions[aid] = a
        self.actions["about"].setMenuRole(QAction.AboutRole)
        self.actions["settings"].setMenuRole(QAction.PreferencesRole)
        self.actions["exit"].setMenuRole(QAction.QuitRole)
        self.actions["langOther"].setEnabled(False)
        self.actions["exit"].triggered.disconnect()
        self.actions["exit"].triggered.connect(self.close)
        lang_group = QActionGroup(self)
        lang_group.addAction(self.actions["langIt"])
        lang_group.addAction(self.actions["langEn"])
        for b in MAIN_TOOLBAR + TOOLBOX + MODBAR:
            if isinstance(b, list):
                self.actions[b[0]].setIcon(svg_icon(b[1]))
        self.actions["lockAspect"].setIcon(svg_icon("lock"))
        self.apply_shortcuts()

    def _build_menus(self):
        mb = self.menuBar()
        mb.clear()
        self.recent_menu = None

        def fill(menu, items):
            for it in items:
                if it == "-":
                    menu.addSeparator()
                elif isinstance(it, list):
                    menu.addAction(self.actions[it[0]])
                elif it.get("dynamic") == "recent":
                    self.recent_menu = menu.addMenu(it["en"] if self.lang == "en" else it["it"])
                    self.recent_menu.aboutToShow.connect(self._fill_recent)
                    self._fill_recent()
                else:
                    sub = menu.addMenu(it["en"] if self.lang == "en" else it["it"])
                    fill(sub, it["items"])

        for m in MENUS:
            menu = mb.addMenu(m["en"] if self.lang == "en" else m["it"])
            fill(menu, m["items"])
        self._retitle_actions()

    def _retitle_actions(self):
        app = APP_MENU_LABELS.get(self.lang, APP_MENU_LABELS["it"])
        for aid, a in self.actions.items():
            a.setText(self.action_label(aid))
            key = a.shortcut().toString(QKeySequence.NativeText)
            a.setToolTip(self.action_label(aid) + (f"  ({key})" if key else ""))
        self.actions["about"].setText(app["about"])
        self.actions["settings"].setText(app["settings"].replace("...", "…"))
        self.actions["exit"].setText(app["quit"])

    def _fill_recent(self):
        m = self.recent_menu
        if m is None:
            return
        m.clear()
        recent = self._load_json("recent", [])
        if not recent:
            a = m.addAction(self.L("(vuoto)", "(empty)"))
            a.setEnabled(False)
        for path in recent:
            a = m.addAction(os.path.basename(path))
            a.setToolTip(path)
            a.triggered.connect(lambda _=False, p=path: self.open_path(p))

    def _build_toolbars(self):
        def make(name, items, area=Qt.TopToolBarArea, orient=Qt.Horizontal):
            tb = QToolBar(name)
            tb.setObjectName(name)
            tb.setIconSize(ICON_SIZE)
            tb.setOrientation(orient)
            tb.setMovable(True)
            for b in items:
                if b == "|":
                    tb.addSeparator()
                else:
                    tb.addAction(self.actions[b[0]])
            self.addToolBar(area, tb)
            return tb

        self.tb_main = make("mainToolbar", MAIN_TOOLBAR)
        self.addToolBarBreak()
        self.tb_mod = QToolBar("modbar")
        self.tb_mod.setObjectName("modbar")
        self.tb_mod.setIconSize(ICON_SIZE)
        self.tx = {}
        self.tx_labels = {}
        for key, lab in (("x", "X"), ("y", "Y"), ("w", "width"), ("h", "height")):
            l = QLabel(" " + (lab if len(lab) == 1 else self.tr(lab)) + " ")
            self.tx_labels[key] = (l, lab)
            sp = dspin(0, -10000, 10000, 0.1, 2, " mm", 92)
            sp.editingFinished.connect(lambda k=key: self.apply_transform_field(k))
            self.tb_mod.addWidget(l)
            self.tb_mod.addWidget(sp)
            self.tx[key] = sp
            if key == "h":
                self.tb_mod.addAction(self.actions["lockAspect"])
        l = QLabel(" " + self.tr("rotate") + " ")
        self.tx_labels["r"] = (l, "rotate")
        self.tb_mod.addWidget(l)
        self.tx["r"] = dspin(0, -360, 360, 1, 1, " °", 70)
        self.tx["r"].editingFinished.connect(lambda: self.apply_transform_field("r"))
        self.tb_mod.addWidget(self.tx["r"])
        self.tb_mod.addSeparator()
        for b in MODBAR:
            if b == "|":
                self.tb_mod.addSeparator()
            else:
                self.tb_mod.addAction(self.actions[b[0]])
        self.addToolBar(Qt.TopToolBarArea, self.tb_mod)
        self.tb_tools = make("toolbox", TOOLBOX, Qt.LeftToolBarArea, Qt.Vertical)

        self.tb_palette = QToolBar("palette")
        self.tb_palette.setObjectName("palette")
        self.palette_btns = []
        for i, l in enumerate(self.engine.layers):
            b = QToolButton()
            b.setText(l.name if l.tool else f"{i:02d}")
            b.setCheckable(True)
            b.setFixedSize(28, 22)
            fg = "#000" if QColor(l.color).lightness() > 140 else "#fff"
            border = "1px dashed #ddd" if l.tool else "1px solid #111"
            b.setStyleSheet(f"QToolButton {{ background:{l.color}; color:{fg}; border:{border}; border-radius:3px; font-size:10px; }}"
                            f"QToolButton:checked {{ border:2px solid #ffffff; }}")
            b.setToolTip(l.name + (self.L(" — livello tecnico: si vede ma non va al laser", " — tool layer: visible, never sent to the laser")
                                   if l.tool else ""))
            if l.tool and i == len(self.engine.layers) - 2:
                self.tb_palette.addSeparator()
            b.clicked.connect(lambda _=False, k=i: self.palette_clicked(k))
            self.tb_palette.addWidget(b)
            self.palette_btns.append(b)
        self.addToolBar(Qt.BottomToolBarArea, self.tb_palette)

    def _build_docks(self):
        def dock(name, widget, area=Qt.RightDockWidgetArea):
            from PySide6.QtWidgets import QFrame, QScrollArea
            d = QDockWidget(self)
            d.setObjectName(name)
            # scrollable content: panels never force the window to be taller than the screen
            scroll = QScrollArea()
            scroll.setWidgetResizable(True)
            scroll.setFrameShape(QFrame.NoFrame)
            scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAsNeeded)
            scroll.setWidget(widget)
            scroll.setMinimumSize(160, 80)
            d.setWidget(scroll)
            d.setFeatures(QDockWidget.DockWidgetMovable | QDockWidget.DockWidgetFloatable | QDockWidget.DockWidgetClosable)
            self.addDockWidget(area, d)
            d.visibilityChanged.connect(lambda _v: QTimer.singleShot(0, self.refresh_checks))
            return d

        self.cuts = CutsPanel(self)
        self.laser_panel = LaserPanel(self)
        self.move_panel = MovePanel(self)
        self.console_panel = ConsolePanel(self)
        self.objects_panel = ObjectsPanel(self)
        self.library_panel = LibraryPanel(self)
        self.art_panel = ArtLibraryPanel(self)
        self.camera_panel = CameraPanel(self)
        self.props_panel = PropertiesPanel(self)
        self.pages_panel = PagesPanel(self)
        self.macro_panel = MacroPanel(self)
        self.dock_cuts = dock("cuts", self.cuts)
        self.dock_laser = dock("laser", self.laser_panel)
        self.dock_move = dock("move", self.move_panel)
        self.dock_console = dock("console", self.console_panel)
        self.dock_objects = dock("objects", self.objects_panel)
        self.dock_library = dock("library", self.library_panel)
        self.dock_art = dock("art", self.art_panel)
        self.dock_camera = dock("camera", self.camera_panel)
        self.dock_props = dock("props", self.props_panel)
        self.dock_pages = dock("pages", self.pages_panel)
        self.dock_macros = dock("macros", self.macro_panel)
        self.splitDockWidget(self.dock_cuts, self.dock_laser, Qt.Vertical)
        for d in (self.dock_move, self.dock_console, self.dock_library, self.dock_objects, self.dock_art, self.dock_camera):
            self.tabifyDockWidget(self.dock_laser, d)
        self.tabifyDockWidget(self.dock_cuts, self.dock_props)
        self.tabifyDockWidget(self.dock_cuts, self.dock_pages)
        self.tabifyDockWidget(self.dock_laser, self.dock_macros)
        self._dock_titles()
        self._default_layout()

    LAYOUT_VERSION = 3        # bump when the default dock layout changes: an older saved layout is then ignored once

    def _default_layout(self):
        """Tidy default: Cuts/Properties/Pages on top; Laser, Move, Console, Library below.
        The other panels are tabbed in the same place but hidden (Window menu shows them)."""
        for d in (self.dock_cuts, self.dock_laser, self.dock_move, self.dock_console, self.dock_library,
                  self.dock_props, self.dock_pages):
            d.setFloating(False)
            d.show()
        for d in (self.dock_objects, self.dock_art, self.dock_camera, self.dock_macros):
            d.hide()
        self.dock_cuts.raise_()
        self.dock_laser.raise_()
        self.resizeDocks([self.dock_cuts], [390], Qt.Horizontal)
        self.resizeDocks([self.dock_cuts, self.dock_laser], [500, 400], Qt.Vertical)
        QTimer.singleShot(0, self._style_dock_tabs)

    def reset_layout(self):
        self.removeDockWidget(self.dock_laser)
        self.addDockWidget(Qt.RightDockWidgetArea, self.dock_laser)
        for d in (self.dock_cuts, self.dock_props, self.dock_pages, self.dock_move, self.dock_console,
                  self.dock_objects, self.dock_library, self.dock_art, self.dock_camera, self.dock_macros):
            self.removeDockWidget(d)
            self.addDockWidget(Qt.RightDockWidgetArea, d)
        self.splitDockWidget(self.dock_cuts, self.dock_laser, Qt.Vertical)
        for d in (self.dock_move, self.dock_console, self.dock_library, self.dock_objects, self.dock_art,
                  self.dock_camera, self.dock_macros):
            self.tabifyDockWidget(self.dock_laser, d)
        self.tabifyDockWidget(self.dock_cuts, self.dock_props)
        self.tabifyDockWidget(self.dock_cuts, self.dock_pages)
        for tb in (self.tb_main, self.tb_mod, self.tb_tools, self.tb_palette):
            tb.show()
        self._default_layout()
        self.refresh_checks()

    def _style_dock_tabs(self):
        """Dock tab labels in full (no "La… Sp… Co…"), scroll arrows only when they really don't fit."""
        from PySide6.QtWidgets import QTabBar
        for tb in self.findChildren(QTabBar):
            if tb.parent() is self:
                tb.setElideMode(Qt.ElideNone)
                tb.setUsesScrollButtons(True)
                tb.setExpanding(False)
                tb.setDrawBase(False)

    def _dock_titles(self):
        self.dock_cuts.setWindowTitle(self.tr("dockCuts"))
        self.dock_laser.setWindowTitle(self.tr("tabLaser"))
        self.dock_move.setWindowTitle(self.tr("tabMove"))
        self.dock_console.setWindowTitle(self.tr("tabConsole"))
        self.dock_objects.setWindowTitle(self.tr("tabObjects"))
        self.dock_library.setWindowTitle(self.tr("tabLibrary"))
        self.dock_art.setWindowTitle(self.L("Libreria grafica", "Art Library"))
        self.dock_camera.setWindowTitle(self.L("Camera", "Camera"))
        self.dock_props.setWindowTitle(self.L("Proprietà forma", "Shape Properties"))
        self.dock_pages.setWindowTitle(self.L("Pagine", "Pages"))
        self.dock_macros.setWindowTitle(self.L("Macro", "Macros"))

    def _build_statusbar(self):
        sb = QStatusBar()
        self.setStatusBar(sb)
        self.lbl_status = QLabel()
        sb.addWidget(self.lbl_status, 1)
        self.lbl_snap = QLabel("SNAP")
        self.lbl_sel = QLabel("")
        self.lbl_mouse = QLabel("0.00, 0.00 mm")
        # zoom indicator = menu: fit, true-size percentages (100 % = real millimetres on screen)
        from PySide6.QtWidgets import QToolButton, QMenu
        self.lbl_zoom = QToolButton()
        self.lbl_zoom.setText("100%")
        self.lbl_zoom.setAutoRaise(True)
        self.lbl_zoom.setPopupMode(QToolButton.InstantPopup)
        self.lbl_zoom.setToolTip(self.L("Zoom: 100% = dimensione reale sullo schermo", "Zoom: 100% = true size on screen"))
        zm = QMenu(self.lbl_zoom)
        zm.addAction(self.L("Adatta alla pagina", "Zoom to page"), self.canvas.zoom_page)
        zm.addAction(self.L("Adatta alla selezione", "Zoom to selection"), self.zoom_sel)
        zm.addSeparator()
        for pct in (25, 50, 100, 200, 400, 800):
            zm.addAction(f"{pct}%" + (self.L("  (dimensione reale)", "  (true size)") if pct == 100 else ""),
                         lambda p_=pct: self.canvas.zoom_real(p_ / 100))
        self.lbl_zoom.setMenu(zm)
        self.lbl_conn = QLabel()
        self.lbl_pos = QLabel("X 0.000 Y 0.000")
        for w in (self.lbl_snap, self.lbl_sel, self.lbl_mouse, self.lbl_zoom, self.lbl_conn, self.lbl_pos):
            sb.addPermanentWidget(w)
        self._style_snap()
        self._style_conn("disconnected")

    def _style_snap(self):
        self.lbl_snap.setStyleSheet("color:#7bd88f; font-weight:600;" if self.snap_on else "color:#6b6d74;")

    def _style_conn(self, state):
        col = {"connected": "#7bd88f", "busy": "#f2c94c", "alarm": "#ff9800"}.get(state, "#ff6b6b")
        txt = {"connected": self.tr("connected"), "busy": self.tr("busy"), "alarm": self.L("Allarme", "Alarm")}.get(state, self.tr("disconnected"))
        self.lbl_conn.setText(f"● {txt}")
        self.lbl_conn.setStyleSheet(f"color:{col};")

    # ================================================================ state sync
    def refresh_checks(self):
        self._style_dock_tabs()
        self._refresh_checks()

    def _refresh_checks(self):
        if not hasattr(self, "dock_cuts"):
            return
        for aid, val in self.checks().items():
            if aid in self.actions:
                self.actions[aid].setChecked(bool(val))
        for aid, w in (("winCuts", self.dock_cuts), ("winLaser", self.dock_laser), ("winMove", self.dock_move),
                       ("winConsole", self.dock_console), ("winObjects", self.dock_objects), ("winLibrary", self.dock_library),
                       ("winArt", self.dock_art), ("winCamera", self.dock_camera),
                       ("winProps", self.dock_props), ("winPages", self.dock_pages), ("winMacros", self.dock_macros),
                       ("winMainToolbar", self.tb_main), ("winModbar", self.tb_mod), ("winToolbox", self.tb_tools),
                       ("winPalette", self.tb_palette)):
            self.actions[aid].setChecked(w.isVisible())
        for i, b in enumerate(self.palette_btns):
            b.setChecked(i == self.doc.active_layer)

    def _on_doc_changed(self):
        self.gcode = ""
        self.cuts.refresh()
        self.cuts.update_fill_visibility()
        self.objects_panel.refresh()
        self.update_transform_fields()
        self.canvas.update()

    def _on_selection_changed(self):
        self.update_transform_fields()
        self.objects_panel.refresh()
        self.canvas.update()

    def _update_title(self):
        name = os.path.basename(self.project_path) if self.project_path else self.tr("untitled")
        self.setWindowTitle(f"{name}[*] — LaserForge")
        self.setWindowModified(self.doc.dirty)
        if self.project_path:
            self.setWindowFilePath(self.project_path)

    def update_transform_fields(self):
        sel = self.doc.selected_shapes()
        if not sel:
            for k in ("x", "y", "w", "h"):
                if not self.tx[k].hasFocus():
                    self.tx[k].blockSignals(True)
                    self.tx[k].setValue(0)
                    self.tx[k].blockSignals(False)
            self.lbl_sel.setText("")
            return
        b = M.multi_bounds(sel)
        u = self.unit_factor()
        for k, v in zip(("x", "y", "w", "h"), b):
            if not self.tx[k].hasFocus():
                self.tx[k].blockSignals(True)
                self.tx[k].setValue(v * u)
                self.tx[k].blockSignals(False)
        self.lbl_sel.setText(f"{sel[0]['type']} #{sel[0]['id']}" if len(sel) == 1 else f"{len(sel)} obj")

    def apply_transform_field(self, key):
        sel = self.doc.selected_shapes()
        if not sel:
            return
        b = M.multi_bounds(sel)
        v = self.tx[key].value()
        if key == "r":
            if v:
                self.tx["r"].setValue(0)
                self.rotate_selection(v)
            return
        v = v / self.unit_factor()
        cur = {"x": b[0], "y": b[1], "w": b[2], "h": b[3]}[key]
        if abs(cur - v) < 1e-4:
            return
        self.doc.save_undo()
        if key == "x":
            for s in sel:
                M.move_shape(s, v - b[0], 0)
        elif key == "y":
            for s in sel:
                M.move_shape(s, 0, v - b[1])
        elif v > 0:
            sx = v / (b[2] or 1) if key == "w" else 1
            sy = v / (b[3] or 1) if key == "h" else 1
            if self.lock_aspect:
                sx, sy = (sx, sx) if key == "w" else (sy, sy)
            for s in sel:
                M.scale_shape(s, b[0], b[1], sx, sy)
        self.doc.notify()

    def _poll(self):
        st = self.engine.state()
        connected = st in ("connected", "busy", "alarm")
        if st == "alarm" and getattr(self, "_last_state", None) != "alarm":
            self.toast(self.L("GRBL è in ALLARME: guarda la Console, poi premi Sblocca ($X) o Home",
                              "GRBL is in ALARM: check the Console, then press Unlock ($X) or Home"))
        self._last_state = st
        self.laser_panel.show_alarm(st == "alarm")
        x, y, _z, _ms = self.engine.position()
        cpos = self.to_canvas((x, y))
        if connected != self.connected or cpos != self.machine_pos:
            self.connected = connected
            self.machine_pos = cpos
            self.laser_panel.set_connected(connected)
            self.canvas.update()
        self._style_conn(st)
        self.lbl_pos.setText(f"X {x:.3f} Y {y:.3f}")
        self.laser_panel.show_progress(*self.engine.progress(), st == "busy")
        self._repeat_frame()
        if self.dock_console.isVisible():
            self.console_panel.refresh()

    # ================================================================ tools & view
    def set_tool(self, name):
        self.canvas.set_tool(name)
        self.refresh_checks()

    def escape_or_select(self):
        if self.canvas.cancel_in_progress():
            return
        if self.canvas.tool != "select":
            self.set_tool("select")
        else:
            self.doc.set_selection([])

    def toggle_snap(self):
        self.snap_on = not self.snap_on
        self.prefs["grid_snap"] = self.snap_on
        P.save_prefs(self.settings, self.prefs)
        self._style_snap()

    def toggle_lock_aspect(self):
        self.lock_aspect = not self.lock_aspect
        self.refresh_checks()

    def toggle_view(self, key):
        self.view[key] = not self.view[key]
        if key == "grid":
            self.prefs["grid_show"] = self.view["grid"]
            P.save_prefs(self.settings, self.prefs)
        self._store_json("view", self.view)
        self.canvas.update()
        self.refresh_checks()

    def toggle_dock(self, w):
        w.setVisible(not w.isVisible())
        if isinstance(w, QDockWidget) and w.isVisible():
            w.raise_()
        self.refresh_checks()

    def toggle_fullscreen(self):
        self.showNormal() if self.isFullScreen() else self.showFullScreen()

    def reset_layout(self):
        docks = (self.dock_cuts, self.dock_laser, self.dock_move, self.dock_console, self.dock_objects, self.dock_library, self.dock_art, self.dock_camera)
        self.dock_macros.setVisible(True)
        self.dock_macros.setFloating(False)
        self.tabifyDockWidget(self.dock_laser, self.dock_macros)
        for extra in (self.dock_props, self.dock_pages):
            extra.setVisible(True)
            extra.setFloating(False)
            self.tabifyDockWidget(self.dock_cuts, extra)
        for w in (self.tb_main, self.tb_mod, self.tb_tools, self.tb_palette) + docks:
            w.setVisible(True)
        for d in docks:
            d.setFloating(False)
        self.addDockWidget(Qt.RightDockWidgetArea, self.dock_cuts)
        self.addDockWidget(Qt.RightDockWidgetArea, self.dock_laser)
        self.splitDockWidget(self.dock_cuts, self.dock_laser, Qt.Vertical)
        for d in docks[2:]:
            self.tabifyDockWidget(self.dock_laser, d)
        self.dock_laser.raise_()
        self.refresh_checks()

    def zoom_sel(self):
        b = M.multi_bounds(self.doc.selected_shapes() or self.doc.shapes)
        if b:
            self.canvas.zoom_to_box((b[0], b[1], max(b[2], 1), max(b[3], 1)), 60)

    def set_lang(self, lang):
        self.lang = lang
        self.settings.setValue("lang", lang)
        app = QApplication.instance()
        old = getattr(app, "_qt_tr", None)
        if old is not None:
            app.removeTranslator(old)
        from PySide6.QtCore import QLibraryInfo, QTranslator
        tr = QTranslator(app)
        if tr.load(f"qtbase_{lang}", QLibraryInfo.path(QLibraryInfo.TranslationsPath)):
            app.installTranslator(tr)
        app._qt_tr = tr
        self._build_menus()
        self._dock_titles()
        for key, (lab, name) in self.tx_labels.items():
            lab.setText(" " + (name if len(name) == 1 else self.tr(name)) + " ")
        for p in (self.cuts, self.laser_panel, self.move_panel, self.console_panel, self.library_panel, self.art_panel, self.camera_panel,
                  self.props_panel, self.pages_panel, self.macro_panel):
            p.retranslate()
        self._style_conn(self.engine.state())
        self._update_title()
        self.refresh_checks()
        self.status(self.tr("ready"))

    # ================================================================ layers
    def select_layer(self, idx):
        self.doc.active_layer = idx
        self.cuts.load(idx)
        self.cuts.refresh()
        self.refresh_checks()

    def palette_clicked(self, idx):
        sel = self.doc.selected_shapes()
        if sel:
            self.doc.save_undo()
            for s in sel:
                M.set_layer(s, idx)
            M.clear_image_caches()
        self.select_layer(idx)
        if sel:
            self.doc.notify()

    def save_layer_settings(self, data):
        self.engine.update_layer(self.doc.active_layer, data)
        self.persist_layers()
        self.gcode = ""
        M.clear_image_caches()
        self.cuts.refresh()
        self.canvas.update()

    def persist_layers(self):
        self._store_json("layers", self.engine.layer_dicts())

    def set_all_layers(self, fn):
        for i, l in enumerate(self.engine.layers):
            patch = fn(l)
            if patch:
                self.engine.update_layer(i, patch)
        self.persist_layers()
        self.gcode = ""
        self.cuts.load(self.doc.active_layer)
        self.cuts.refresh()

    def apply_preset(self, name):
        if self.engine.apply_preset(self.doc.active_layer, name):
            self.persist_layers()
            self.cuts.load(self.doc.active_layer)
            self.cuts.refresh()
            self.gcode = ""
            self.toast(f"{name} → {self.engine.layers[self.doc.active_layer].name}")

    # ================================================================ selection & editing
    def select_by(self, pred):
        self.doc.set_selection([s["id"] for s in self.doc.shapes if not s.get("locked") and pred(s)])

    def set_shape_attr(self, key, value):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        for s in sel:
            if value is None:
                s.pop(key, None)
            else:
                s[key] = value
        self.doc.notify()

    def reset_start_points(self):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        for s in sel:
            s.pop("start", None)
            s.pop("reverse", None)
        self.doc.notify()

    def invert_selection(self):
        cur = set(self.doc.selected)
        self.select_by(lambda s: s["id"] not in cur)

    def select_contained(self):
        sel = self.need_sel()
        if not sel:
            return
        boxes = [M.bounds(s) for s in sel]

        def inside(s):
            sb = M.bounds(s)
            return any(sb[0] >= b[0] and sb[1] >= b[1] and sb[0] + sb[2] <= b[0] + b[2] and sb[1] + sb[3] <= b[1] + b[3] for b in boxes)
        self.select_by(lambda s: s not in sel and inside(s))

    def delete_selected(self):
        if not self.doc.selected:
            return
        self.doc.save_undo()
        self.doc.shapes = [s for s in self.doc.shapes if s["id"] not in self.doc.selected or s.get("locked")]
        self.doc.selected = []
        self.doc.notify()

    def copy_sel(self):
        sel = self.need_sel()
        if sel:
            self.clipboard = M.clone(sel)
            self.status(self.L(f"{len(sel)} copiati", f"{len(sel)} copied"))

    def cut_sel(self):
        self.copy_sel()
        self.delete_selected()

    def paste(self, in_place):
        if not self.clipboard:
            return
        items = [self.doc.reid(s) for s in M.clone(self.clipboard)]
        if not in_place:
            b = M.multi_bounds(items)
            target = self.canvas.last_mouse or (b[0] + 10 + b[2] / 2, b[1] + 10 + b[3] / 2)
            for s in items:
                M.move_shape(s, target[0] - b[0] - b[2] / 2, target[1] - b[1] - b[3] / 2)
        self.doc.add_shapes(items)

    def duplicate_sel(self):
        sel = self.need_sel()
        if sel:
            self.doc.add_shapes([self.doc.reid(s) for s in M.clone(sel)])

    def group_sel(self):
        sel = self.need_sel(2)
        if not sel:
            return
        self.doc.save_undo()
        g = self.doc.create("group", children=M.clone(sel), layer=sel[0].get("layer", 0))
        first = min(self.doc.shapes.index(s) for s in sel)
        self.doc.shapes = [s for s in self.doc.shapes if s["id"] not in self.doc.selected]
        self.doc.shapes.insert(first, g)
        self.doc.selected = [g["id"]]
        self.doc.notify()

    def ungroup_sel(self):
        groups = [s for s in self.doc.selected_shapes() if s["type"] == "group"]
        if not groups:
            return
        self.doc.save_undo()
        ids = []
        for g in groups:
            idx = self.doc.shapes.index(g)
            kids = []
            for c in g["children"]:
                if c.get("layer") is None:
                    c["layer"] = g.get("layer", 0)
                if c.get("id") is None:
                    c["id"] = self.doc.new_id()
                kids.append(c)
                ids.append(c["id"])
            self.doc.shapes[idx:idx + 1] = kids
        self.doc.selected = ids
        self.doc.notify()

    def mirror_sel(self, axis):
        sel = self.need_sel()
        if not sel:
            return
        cx, cy = M.center(M.multi_bounds(sel))
        self.doc.save_undo()
        for s in sel:
            M.mirror_shape(s, axis, cx, cy)
        M.clear_image_caches()
        self.doc.notify()

    def rotate_selection(self, deg):
        sel = self.need_sel()
        if not sel:
            return
        cx, cy = M.center(M.multi_bounds(sel))
        self.doc.save_undo()
        skipped = [s for s in sel if not M.rotate_shape(s, cx, cy, deg)]
        for s in sel:
            self._fix_child_ids(s)
        if skipped:
            self.toast(self.tr("rotateImageNA"))
        M.clear_image_caches()
        self.doc.notify()

    def _fix_child_ids(self, s):
        for c in s.get("children", []):
            if c.get("id") is None:
                c["id"] = self.doc.new_id()
            self._fix_child_ids(c)

    def align_sel(self, mode):
        sel = self.need_sel(2)
        if not sel:
            return
        ref = M.bounds(self.doc.selected_in_order()[-1])
        self.doc.save_undo()
        for s in sel:
            b = M.bounds(s)
            dx = dy = 0
            if mode == "left":
                dx = ref[0] - b[0]
            if mode == "right":
                dx = ref[0] + ref[2] - b[0] - b[2]
            if mode == "top":
                dy = ref[1] - b[1]
            if mode == "bottom":
                dy = ref[1] + ref[3] - b[1] - b[3]
            if mode in ("centerH", "centers"):
                dx = ref[0] + ref[2] / 2 - b[0] - b[2] / 2
            if mode in ("centerV", "centers"):
                dy = ref[1] + ref[3] / 2 - b[1] - b[3] / 2
            M.move_shape(s, dx, dy)
        self.doc.notify()

    def distribute_sel(self, axis, centers):
        sel = self.need_sel(3)
        if not sel:
            return
        k, sz = (0, 2) if axis == "h" else (1, 3)
        items = sorted(((s, M.bounds(s)) for s in sel), key=lambda it: it[1][k] + it[1][sz] / 2)
        self.doc.save_undo()
        first, last = items[0][1], items[-1][1]
        n = len(items)

        def mv(s, d):
            M.move_shape(s, d if axis == "h" else 0, 0 if axis == "h" else d)
        if centers:
            c0, c1 = first[k] + first[sz] / 2, last[k] + last[sz] / 2
            for i, (s, b) in enumerate(items):
                mv(s, c0 + (c1 - c0) * i / (n - 1) - (b[k] + b[sz] / 2))
        else:
            total = sum(b[sz] for _, b in items)
            gap = (last[k] + last[sz] - first[k] - total) / (n - 1)
            pos = first[k]
            for s, b in items:
                mv(s, pos - b[k])
                pos += b[sz] + gap
        self.doc.notify()

    def move_sel_to(self, where):
        sel = self.need_sel()
        if not sel:
            return
        b = M.multi_bounds(sel)
        W, H = self.workspace()
        tx, ty = b[0], b[1]
        if "L" in where and where != "laser":
            tx = 0
        if "R" in where:
            tx = W - b[2]
        if "U" in where:
            ty = 0
        if "D" in where:
            ty = H - b[3]
        if where == "C":
            tx, ty = (W - b[2]) / 2, (H - b[3]) / 2
        if where == "laser":
            tx, ty = self.machine_pos[0] - b[2] / 2, self.machine_pos[1] - b[3] / 2
        self.doc.save_undo()
        for s in sel:
            M.move_shape(s, tx - b[0], ty - b[1])
        self.doc.notify()

    def same_size(self, dim):
        sel = self.need_sel(2)
        if not sel:
            return
        ref = M.bounds(self.doc.selected_in_order()[-1])
        self.doc.save_undo()
        for s in sel:
            b = M.bounds(s)
            if dim == "w" and b[2]:
                M.scale_shape(s, b[0], b[1], ref[2] / b[2], 1)
            if dim == "h" and b[3]:
                M.scale_shape(s, b[0], b[1], 1, ref[3] / b[3])
        self.doc.notify()

    def reorder(self, how):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        rest = [s for s in self.doc.shapes if s not in sel]
        if how == "front":
            self.doc.shapes = rest + sel
        elif how == "back":
            self.doc.shapes = sel + rest
        else:
            arr = self.doc.shapes[:]
            order = range(len(arr) - 1, -1, -1) if how == "forward" else range(len(arr))
            for i in order:
                j = i + 1 if how == "forward" else i - 1
                if 0 <= j < len(arr) and arr[i] in sel and arr[j] not in sel:
                    arr[i], arr[j] = arr[j], arr[i]
            self.doc.shapes = arr
        self.doc.notify()

    def delete_duplicates(self):
        seen, keep, removed = set(), [], 0
        for s in self.doc.shapes:
            sig = json.dumps({**s, "id": 0}, sort_keys=True, default=str)
            if sig in seen:
                removed += 1
            else:
                seen.add(sig)
                keep.append(s)
        if not removed:
            self.toast(self.L("Nessun duplicato", "No duplicates"))
            return
        self.doc.save_undo()
        self.doc.shapes = keep
        self.toast(self.L(f"{removed} duplicati rimossi", f"{removed} duplicates removed"))
        self.doc.notify()

    def close_path(self):
        polys = [s for s in self.need_sel() if s["type"] == "poly" and not s.get("closed")]
        if polys:
            self.doc.save_undo()
            for s in polys:
                s["closed"] = True
            self.doc.notify()

    def _text_to_group(self, s):
        kids = [{"id": self.doc.new_id(), "type": "poly", "layer": s.get("layer", 0),
                 "points": [{"x": x, "y": y} for x, y in p[:-1]], "closed": True} for p in M.text_paths(s)]
        return {"id": s["id"], "type": "group", "layer": s.get("layer", 0), "children": kids}

    def convert_to_path(self):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        self.doc.shapes = [(self._text_to_group(s) if s["type"] == "text" else M.to_poly(s)) if s in sel else s for s in self.doc.shapes]
        self.doc.notify()

    def set_locked(self, v):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        for s in sel:
            s["locked"] = v
        if v:
            self.doc.selected = []
        self.doc.notify()

    def insert_shape(self, kind):
        W, H = self.workspace()
        cx, cy, r = W / 2, H / 2, 25
        pts = {
            "triangle": lambda: M.regular_polygon(cx, cy, r, 3),
            "pentagon": lambda: M.regular_polygon(cx, cy, r, 5),
            "octagon": lambda: M.regular_polygon(cx, cy, r, 8, -math.pi / 2 + math.pi / 8),
            "star": lambda: M.star_points(cx, cy, r, r * 0.45, 5),
            "dualStar": lambda: M.star_points(cx, cy, r, r * 0.75, 10),
        }[kind]()
        meta = {"triangle": (3, 0), "pentagon": (5, 0), "octagon": (8, 0), "star": (5, 0.45), "dualStar": (10, 0.75)}[kind]
        rot = -math.pi / 2 + (math.pi / 8 if kind == "octagon" else 0)
        regular = {"cx": cx, "cy": cy, "r": r, "n": meta[0], "rot": rot, **({"inner": meta[1]} if meta[1] else {})}
        self.doc.add_shapes([self.doc.create("poly", points=pts, closed=True, regular=regular)])

    def flatten_arrays(self):
        arrays = [s for s in self.doc.selected_shapes() if s["type"] == "group" and "array" in s]
        if not arrays:
            self.toast(self.L("Seleziona un array dinamico", "Select a live array"))
            return
        self.doc.save_undo()
        for g in arrays:
            g.pop("array", None)
        self.doc.notify()

    def auto_join(self, tol=0.1):
        sel = [s for s in self.need_sel() if s["type"] == "poly" and not s.get("closed")]
        if not sel:
            return
        tol = float(tol)
        near = lambda a, b: math.hypot(a["x"] - b["x"], a["y"] - b["y"]) <= tol
        pool = M.clone(sel)
        done = []
        while pool:
            cur = pool.pop(0)
            grew = True
            while grew:
                grew = False
                for i, o in enumerate(pool):
                    a, b = cur["points"], o["points"]
                    merged = None
                    if near(a[-1], b[0]):
                        merged = a + b[1:]
                    elif near(a[-1], b[-1]):
                        merged = a + b[:-1][::-1]
                    elif near(a[0], b[-1]):
                        merged = b + a[1:]
                    elif near(a[0], b[0]):
                        merged = b[::-1] + a[1:]
                    if merged:
                        cur["points"] = merged
                        pool.pop(i)
                        grew = True
                        break
            if len(cur["points"]) > 2 and near(cur["points"][0], cur["points"][-1]):
                cur["points"].pop()
                cur["closed"] = True
            done.append(cur)
        self.doc.save_undo()
        ids = {s["id"] for s in sel}
        self.doc.shapes = [s for s in self.doc.shapes if s["id"] not in ids] + done
        self.doc.selected = [s["id"] for s in done]
        self.toast(f"{len(sel)} → {len(done)}")
        self.doc.notify()

    # ---------------------------------------------------------------- shape cleanup
    def close_with_tolerance(self, tol):
        """Join open paths whose ends are within tol, then close the ones that end where they start."""
        self.auto_join(tol)
        closed = 0
        for s in self.doc.selected_shapes():
            if s["type"] == "poly" and not s.get("closed") and len(s["points"]) > 2:
                a, b = s["points"][0], s["points"][-1]
                if math.hypot(a["x"] - b["x"], a["y"] - b["y"]) <= tol:
                    if math.hypot(a["x"] - b["x"], a["y"] - b["y"]) < 1e-9:
                        s["points"].pop()
                    s["closed"] = True
                    closed += 1
        self.doc.notify()
        self.toast(self.L(f"{closed} forme chiuse", f"{closed} shapes closed"))

    def delete_overlapping(self):
        """Replace the selection with the same lines, each overlapping piece kept once."""
        from shapely.geometry import LineString
        from shapely.ops import linemerge, unary_union
        sel = [s for s in self.doc.selected_shapes() if s["type"] not in ("image",)]
        if not sel:
            self.toast(self.tr("selectFirst"))
            return
        lines = [LineString(p) for s in sel for p in M.shape_paths(s) if len(p) >= 2]
        before = sum(ln.length for ln in lines)
        merged = linemerge(unary_union(lines))
        geoms = getattr(merged, "geoms", [merged])
        layer = sel[0].get("layer", 0)
        new = []
        for g in geoms:
            pts = list(g.coords)
            if len(pts) < 2:
                continue
            closed = len(pts) > 3 and math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]) < 1e-6
            if closed:
                pts = pts[:-1]
            new.append({"id": self.doc.new_id(), "type": "poly", "layer": layer, "closed": closed,
                        "points": [{"x": x, "y": y} for x, y in pts]})
        after = sum(g.length for g in geoms)
        self.doc.save_undo()
        self.doc.shapes = [s for s in self.doc.shapes if s not in sel] + new
        self.doc.selected = [n["id"] for n in new]
        self.doc.notify()
        self.toast(self.L(f"Rimossi {before - after:.1f} mm di linee sovrapposte", f"Removed {before - after:.1f} mm of overlapping lines"))

    def select_nested(self):
        """Select every shape that lies inside another closed shape."""
        from shapely.geometry import Polygon
        polys = []
        for s in self.doc.shapes:
            path = M.shape_paths(s)[0] if M.shape_paths(s) else []
            if len(path) > 3 and M.is_closed_path(path):
                pg = Polygon(path)
                if pg.is_valid:
                    polys.append((s["id"], pg))
        ids = []
        for s in self.doc.shapes:
            b = M.bounds(s)
            from shapely.geometry import box as sbox
            bb = sbox(b[0], b[1], b[0] + b[2], b[1] + b[3])
            if any(pid != s["id"] and pg.contains(bb) for pid, pg in polys):
                ids.append(s["id"])
        self.doc.set_selection(ids)
        self.toast(self.L(f"{len(ids)} forme annidate", f"{len(ids)} nested shapes"))

    def select_same_circles(self):
        ref = next((s for s in self.doc.selected_shapes() if s["type"] == "circle"), None)
        if ref is None:
            self.toast(self.L("Seleziona un cerchio", "Select a circle"))
            return
        tol = 0.05
        self.select_by(lambda s: s["type"] == "circle" and abs(s["rx"] - ref["rx"]) <= tol and abs(s["ry"] - ref["ry"]) <= tol)

    def select_smaller(self):
        sel = self.need_sel()
        if not sel:
            return
        limit = max(b[2] * b[3] for b in (M.bounds(s) for s in sel))
        self.select_by(lambda s: (lambda b: b[2] * b[3] < limit)(M.bounds(s)))

    def break_apart(self):
        sel = self.need_sel()
        if not sel:
            return
        self.doc.save_undo()
        out = []

        def explode(s):
            if s["type"] == "group":
                for c in s["children"]:
                    if c.get("layer") is None:
                        c["layer"] = s.get("layer", 0)
                    explode(c)
            elif s["type"] == "text":
                out.extend(self._text_to_group(s)["children"])
            else:
                if s.get("id") is None:
                    s["id"] = self.doc.new_id()
                out.append(s)
        for s in sel:
            explode(s)
        self.doc.shapes = [s for s in self.doc.shapes if s not in sel] + out
        self.doc.selected = [s["id"] for s in out]
        self.doc.notify()

    def mirror_across_line(self):
        sel = self.doc.selected_in_order()
        if len(sel) < 2 or sel[-1]["type"] != "poly" or len(sel[-1]["points"]) != 2:
            self.toast(self.L("Seleziona gli oggetti e per ultima una linea retta", "Select the objects, then a straight line last"))
            return
        a, b = sel[-1]["points"]
        dx, dy = b["x"] - a["x"], b["y"] - a["y"]
        l2 = dx * dx + dy * dy

        def refl(x, y):
            t = ((x - a["x"]) * dx + (y - a["y"]) * dy) / l2
            fx, fy = a["x"] + t * dx, a["y"] + t * dy
            return 2 * fx - x, 2 * fy - y

        def mirror(s):
            if s["type"] == "group":
                return all([mirror(c) for c in s["children"]])
            if s["type"] in ("text", "image"):
                return False
            if s["type"] == "circle" and abs(s["rx"] - s["ry"]) < 1e-9:
                s["cx"], s["cy"] = refl(s["cx"], s["cy"])
                return True
            M.replace_in_place(s, M.to_poly(s))
            for p in s["points"]:
                M.map_point_dict(p, refl)
            s.pop("regular", None)
            return True
        copies = [c for c in (self.doc.reid(s) for s in M.clone(sel[:-1])) if mirror(c)]
        if copies:
            self.doc.add_shapes(copies)

    def crop_image(self):
        """Apply a closed shape as a live crop path on the image (LightBurn: Apply Crop Path to Image)."""
        sel = self.doc.selected_shapes()
        img = next((s for s in sel if s["type"] == "image"), None)
        crop = next((s for s in sel if s["type"] != "image"), None)
        if not img or not crop:
            self.toast(self.L("Seleziona un'immagine e una forma di ritaglio", "Select an image and a crop shape"))
            return
        self.doc.save_undo()
        if not M.set_live_crop(img, crop):
            self.toast(self.L("La forma di ritaglio deve essere chiusa", "The crop shape must be closed"))
            return
        self.doc.shapes = [s for s in self.doc.shapes if s is not crop]
        self.doc.selected = [img["id"]]
        M.clear_image_caches()
        self.doc.notify()

    def remove_crop(self):
        img = self.selected_image()
        if img and img.get("crop"):
            self.doc.save_undo()
            img.pop("crop")
            M.clear_image_caches()
            self.doc.notify()

    def flatten_crop(self):
        img = self.selected_image()
        if img and img.get("crop"):
            self.doc.save_undo()
            M.flatten_crop(img)
            M.clear_image_caches()
            self.doc.notify()

    # ---------------------------------------------------------------- bitmap tools
    def selected_image(self):
        s = next((x for x in self.doc.selected_shapes() if x["type"] == "image"), None)
        if s is None:
            self.toast(self.tr("selectImage"))
        return s

    def replace_image(self, fit=False):
        s = self.selected_image()
        if s is None:
            return
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("replaceImage"), self.settings.value("lastDir", str(Path.home())),
                                              "Immagini (*.png *.jpg *.jpeg *.bmp *.gif *.webp *.tif *.tiff)")
        if not path:
            return
        with open(path, "rb") as f:
            img = M.image_from_bytes(f.read())
        if img is None:
            self.toast(self.L("Immagine non supportata", "Unsupported image"))
            return
        self.doc.save_undo()
        s.update({"src": M.encode_image(img), "name": os.path.basename(path), "path": path, "flipX": False, "flipY": False})
        s.pop("crop", None)
        if not fit:
            # keep position and width, follow the new image's proportions
            s["h"] = s["w"] * img.height / img.width
        M.clear_image_caches()
        self.doc.notify()

    def reload_image(self):
        s = self.selected_image()
        if s is None:
            return
        path = s.get("path")
        if not path or not os.path.exists(path):
            self.toast(self.L("File originale non trovato: usa Sostituisci immagine", "Original file not found: use Replace Image"))
            return
        with open(path, "rb") as f:
            img = M.image_from_bytes(f.read())
        if img is None:
            return
        self.doc.save_undo()
        s["src"] = M.encode_image(img)
        M.clear_image_caches()
        self.doc.notify()
        self.toast(self.L("Immagine ricaricata", "Image reloaded"))

    def convert_to_bitmap(self, px_mm, fill, stroke_mm, keep):
        from PIL import Image, ImageChops, ImageDraw
        sel = [x for x in self.doc.selected_shapes() if x["type"] != "image"]
        if not sel:
            self.toast(self.tr("selectFirst"))
            return
        b = M.multi_bounds(sel)
        pad = max(stroke_mm, px_mm)
        x0, y0 = b[0] - pad, b[1] - pad
        W, H = max(1, round((b[2] + 2 * pad) / px_mm)), max(1, round((b[3] + 2 * pad) / px_mm))
        if W * H > 40_000_000:
            self.toast(self.L("Immagine troppo grande: aumenta la dimensione del pixel", "Image too large: increase the pixel size"))
            return
        ink = Image.new("1", (W, H), 0)
        lw = max(1, round(stroke_mm / px_mm))
        for s in M.flatten(sel):
            for path in M.shape_paths(s):
                pts = [((x - x0) / px_mm, (y - y0) / px_mm) for x, y in path]
                if len(pts) < 2:
                    continue
                if fill and M.is_closed_path(path) and len(pts) > 3:
                    ring = Image.new("1", (W, H), 0)
                    ImageDraw.Draw(ring).polygon(pts, fill=1)
                    ink = ImageChops.logical_xor(ink, ring)
                else:
                    ImageDraw.Draw(ink).line(pts, fill=1, width=lw)
        img = Image.eval(ink.convert("L"), lambda v: 255 - v).convert("RGB")
        self.doc.save_undo()
        if not keep:
            self.doc.shapes = [x for x in self.doc.shapes if x not in sel]
        im = self.doc.create("image", src=M.encode_image(img), name="bitmap.png", x=x0, y=y0, w=W * px_mm, h=H * px_mm,
                             adj={**M.default_adj(), "mode": "threshold"}, layer=sel[0].get("layer", 0))
        self.doc.shapes.append(im)
        self.doc.selected = [im["id"]]
        self.doc.notify()

    def nest_true_shape(self, spacing=2.0, rotations=(0, 90, 180, 270), margin=3.0):
        from .nest import nest
        items = [s for s in (self.doc.selected_shapes() or self.doc.shapes) if not s.get("locked")]
        if not items:
            return 0, 0
        W, H = self.workspace()
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            placed, unplaced = nest(items, (margin, margin, W - 2 * margin, H - 2 * margin), spacing, rotations)
        finally:
            QApplication.restoreOverrideCursor()
        self.doc.save_undo()
        for s, ang, dx, dy in placed:
            if ang:
                M.rotate_shape(s, 0.0, 0.0, ang)
                self._fix_child_ids(s)
            M.move_shape(s, dx, dy)
        M.clear_image_caches()
        self.doc.notify()
        if unplaced:
            self.toast(self.L(f"{len(unplaced)} pezzi non entrano nell'area di lavoro", f"{len(unplaced)} parts don't fit in the work area"))
        else:
            self.toast(self.L(f"{len(placed)} pezzi disposti", f"{len(placed)} parts nested"))
        return len(placed), len(unplaced)

    def nesting(self):
        items = [s for s in (self.doc.selected_shapes() or self.doc.shapes) if not s.get("locked")]
        if not items:
            return
        gap = 3
        W, H = self.workspace()
        self.doc.save_undo()
        x = y = gap
        row_h, overflow = 0, False
        for s, b in sorted(((s, M.bounds(s)) for s in items), key=lambda it: -it[1][3]):
            if x + b[2] > W - gap and x > gap:
                x, y, row_h = gap, y + row_h + gap, 0
            if y + b[3] > H:
                overflow = True
            M.move_shape(s, x - b[0], y - b[1])
            x += b[2] + gap
            row_h = max(row_h, b[3])
        if overflow:
            self.toast(self.L("Non tutto entra nell'area di lavoro", "Not everything fits in the work area"))
        self.doc.notify()

    def geometry_op(self, op, params, replace=True, minimum=1):
        sel = [s for s in self.doc.selected_in_order() if s["type"] != "image"]
        if len(sel) < minimum:
            self.toast(self.L(f"Seleziona almeno {minimum} forme vettoriali", f"Select at least {minimum} vector shapes") if minimum > 1 else self.tr("selectFirst"))
            return
        try:
            results = self.engine.geometry(op, [M.shape_paths(s) for s in sel], params)
        except Exception as exc:
            self.toast(str(exc))
            return
        layer = sel[0].get("layer", 0)
        created = []
        for rings in results:
            polys = [{"id": self.doc.new_id(), "type": "poly", "layer": layer, "points": [{"x": x, "y": y} for x, y in r[:-1]], "closed": True}
                     for r in rings if len(r) > 3]
            if len(polys) == 1:
                created.append(polys[0])
            elif polys:
                created.append({"id": self.doc.new_id(), "type": "group", "layer": layer, "children": polys})
        if not created:
            self.toast(self.L("Risultato vuoto", "Empty result"))
            return
        self.doc.save_undo()
        if replace:
            self.doc.shapes = [s for s in self.doc.shapes if s not in sel]
        self.doc.shapes.extend(created)
        self.doc.selected = [s["id"] for s in created]
        self.doc.notify()

    def _edit_shape(self, s):
        if s["type"] == "text":
            D.edit_text_dialog(self, s)
        elif s["type"] == "image":
            D.adjust_image_dialog(self)

    # ================================================================ images / import
    def add_image_bytes(self, data: bytes, name: str, path: str = None):
        img = M.image_from_bytes(data)
        if img is None:
            self.toast(self.L("Immagine non supportata", "Unsupported image"))
            return
        W, H = self.workspace()
        w, h = img.width * 25.4 / 254, img.height * 25.4 / 254
        k = min(1, W * 0.6 / w, H * 0.6 / h)
        w, h = w * k, h * k
        s = self.doc.create("image", src=M.encode_image(img), name=name, x=(W - w) / 2, y=(H - h) / 2, w=w, h=h, adj=M.default_adj())
        if path:
            s["path"] = path
        self.doc.add_shapes([s])
        self.toast(f"{name}: {w:.1f} × {h:.1f} mm")

    def import_svg(self, text):
        import xml.etree.ElementTree as ET
        try:
            root = ET.fromstring(text)
        except ET.ParseError:
            self.toast(self.L("SVG non valido", "Invalid SVG"))
            return
        unit = 25.4 / 96
        wattr = root.get("width", "")
        vb = [float(v) for v in (root.get("viewBox") or "").replace(",", " ").split()] if root.get("viewBox") else []
        if len(vb) == 4 and vb[2]:
            for suf, mul in (("mm", 1), ("cm", 10), ("in", 25.4)):
                if wattr.endswith(suf):
                    unit = float(wattr[:-len(suf)]) * mul / vb[2]
        num = lambda el, a: float(el.get(a) or 0)
        sc = lambda x, y: {"x": x * unit, "y": y * unit}
        shapes = []
        create = self.doc.create
        for el in root.iter():
            tag = el.tag.split("}")[-1]
            if tag == "line":
                shapes.append(create("poly", points=[sc(num(el, "x1"), num(el, "y1")), sc(num(el, "x2"), num(el, "y2"))], closed=False))
            elif tag == "rect":
                shapes.append(create("rect", x=num(el, "x") * unit, y=num(el, "y") * unit, w=num(el, "width") * unit, h=num(el, "height") * unit))
            elif tag == "circle":
                r = num(el, "r") * unit
                shapes.append(create("circle", cx=num(el, "cx") * unit, cy=num(el, "cy") * unit, rx=r, ry=r))
            elif tag == "ellipse":
                shapes.append(create("circle", cx=num(el, "cx") * unit, cy=num(el, "cy") * unit, rx=num(el, "rx") * unit, ry=num(el, "ry") * unit))
            elif tag in ("polyline", "polygon"):
                n = [float(v) for v in (el.get("points") or "").replace(",", " ").split()]
                pts = [sc(n[i], n[i + 1]) for i in range(0, len(n) - 1, 2)]
                if len(pts) > 1:
                    shapes.append(create("poly", points=pts, closed=tag == "polygon"))
            elif tag == "path":
                for sub in M.parse_svg_path(el.get("d") or ""):
                    if len(sub["points"]) > 1:
                        shapes.append(create("poly", points=[sc(x, y) for x, y in sub["points"]], closed=sub["closed"]))
        if not shapes:
            self.toast(self.L("Nessun vettore trovato", "No vectors found"))
            return
        self.doc.add_shapes(shapes)
        self.toast(self.L(f"{len(shapes)} forme importate", f"{len(shapes)} shapes imported"))

    def import_vector_file(self, path):
        from . import importers as IM
        ext = os.path.splitext(path)[1].lower()
        try:
            if ext == ".dxf":
                with open(path, encoding="utf-8", errors="replace") as f:
                    paths = IM.dxf_to_paths(f.read())
            else:
                with open(path, "rb") as f:
                    data = f.read()
                pages = IM.pdf_page_count(data)
                page = 0
                if pages > 1:
                    page, ok = QInputDialog.getInt(self, self.L("Importa PDF", "Import PDF"),
                                                   self.L(f"Il file ha {pages} pagine. Quale importare?", f"The file has {pages} pages. Which one?"),
                                                   1, 1, pages)
                    if not ok:
                        return
                    page -= 1
                images = []
                paths = IM.pdf_to_paths(data, page, images)
        except Exception as exc:
            self.toast(self.L("Impossibile importare: ", "Cannot import: ") + str(exc))
            return
        shapes = []
        for im, x, y, w_, h_ in (images if ext != ".dxf" else []):
            if w_ > 0.1 and h_ > 0.1:
                shapes.append(self.doc.create("image", src=M.encode_image(im), name=os.path.basename(path), x=x, y=y, w=w_, h=h_, adj=M.default_adj()))
        for pts, closed in paths:
            if closed and len(pts) > 2 and math.hypot(pts[0][0] - pts[-1][0], pts[0][1] - pts[-1][1]) < 1e-6:
                pts = pts[:-1]
            if len(pts) >= 2:
                shapes.append(self.doc.create("poly", points=[{"x": x, "y": y} for x, y in M.simplify(pts, 0.005)], closed=closed))
        if not shapes:
            self.toast(self.L("Nessun vettore trovato (testo e immagini nei PDF non vengono importati)",
                              "No vectors found (text and images inside PDFs are not imported)"))
            return
        self.doc.add_shapes(shapes)
        self.toast(self.L(f"{len(shapes)} forme importate da {os.path.basename(path)}", f"{len(shapes)} shapes imported from {os.path.basename(path)}"))

    def import_pcb_file(self, path):
        from . import pcb
        try:
            with open(path, encoding="latin-1") as f:
                text = f.read()
            drill = "M48" in text[:2000] or path.lower().endswith((".drl", ".xln"))
            geom = pcb.excellon_geometry(text) if drill else pcb.gerber_geometry(text)
        except Exception as exc:
            self.toast(self.L("File PCB non leggibile: ", "Cannot read PCB file: ") + str(exc))
            return
        rings = pcb.geometry_rings(geom, self.workspace()[1])
        if not rings:
            self.toast(self.L("Nessuna geometria nel file PCB", "No geometry in the PCB file"))
            return
        kids = [{"id": self.doc.new_id(), "type": "poly", "layer": self.doc.active_layer, "closed": True,
                 "points": [{"x": x, "y": y} for x, y in M.simplify(pts[:-1], 0.003)]} for pts, _ in rings if len(pts) > 3]
        g = self.doc.create("group", children=kids, name=os.path.basename(path))
        self.doc.add_shapes([g])
        self.toast(self.L(f"{os.path.basename(path)}: {len(kids)} contorni ({'fori' if drill else 'rame/serigrafia'})",
                          f"{os.path.basename(path)}: {len(kids)} outlines ({'drills' if drill else 'copper/silk'})"))

    def import_file(self):
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("import"), self.settings.value("lastDir", str(Path.home())),
                                              "Immagini, vettori, 3D, PCB (*.svg *.dxf *.pdf *.ai *.png *.jpg *.jpeg *.bmp *.gif *.webp *.tif *.tiff *.stl *.obj *.ply *.3mf *.amf "
                                              "*.gbr *.ger *.gtl *.gbl *.gto *.gbo *.gts *.gbs *.gko *.drl *.xln);;Tutti (*)")
        if path:
            self.settings.setValue("lastDir", os.path.dirname(path))
            self.open_dropped(path)

    def open_dropped(self, path):
        ext = os.path.splitext(path)[1].lower()
        if ext == ".lfproj":
            self.open_path(path)
        elif ext == ".svg":
            with open(path, encoding="utf-8", errors="replace") as f:
                self.import_svg(f.read())
        elif ext in (".dxf", ".pdf", ".ai"):
            self.import_vector_file(path)
        elif ext in (".stl", ".obj", ".ply", ".3mf", ".amf"):
            D.import_3d_dialog(self, path)
        elif ext in (".gbr", ".ger", ".gtl", ".gbl", ".gto", ".gbo", ".gts", ".gbs", ".gko", ".gm1", ".drl", ".xln", ".nc_drill", ".txt"):
            self.import_pcb_file(path)
        else:
            with open(path, "rb") as f:
                self.add_image_bytes(f.read(), os.path.basename(path), path)

    # ================================================================ projects
    def confirm_discard(self):
        if not self.doc.dirty:
            return True
        r = QMessageBox.question(self, "LaserForge", self.tr("unsaved"), QMessageBox.Discard | QMessageBox.Cancel | QMessageBox.Save)
        if r == QMessageBox.Save:
            return self.save_project(False)
        return r == QMessageBox.Discard

    def new_project(self):
        if not self.confirm_discard():
            return
        self.engine.reset_layers()
        self.persist_layers()
        self.project_path = None
        self.print_cut = None
        self.project_notes = {}
        self.doc.active_layer = 0
        self.doc.reset()
        self.cuts.load(0)
        self._update_title()

    def open_project(self):
        if not self.confirm_discard():
            return
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("open"), self.settings.value("lastDir", str(Path.home())),
                                              "LaserForge (*.lfproj);;JSON (*.json)")
        if path:
            self.open_path(path, confirmed=True)

    def open_path(self, path, confirmed=False):
        if not confirmed and not self.confirm_discard():
            return
        try:
            with open(path, encoding="utf-8") as f:
                d = json.load(f)
        except (OSError, ValueError):
            self.toast(self.L("File progetto non valido o mancante", "Invalid or missing project file"))
            self._store_json("recent", [p for p in self._load_json("recent", []) if p != path])
            return
        self.settings.setValue("lastDir", os.path.dirname(path))
        ws = d.get("workspace")
        if ws:
            self.engine.update_machine({"workspace_x": ws["x"], "workspace_y": ws["y"]})
            self.move_panel.load()
        for i, l in enumerate((d.get("layers") or [])[:len(self.engine.layers)]):
            self.engine.update_layer(i, {k: v for k, v in l.items() if k != "index"})
        self.persist_layers()
        self.project_path = path
        self.add_recent(path)
        M.clear_image_caches()
        self.doc.load(d.get("shapes") or [], d.get("nextId"), d.get("pages"))
        self.project_notes = d.get("notes") or {}
        if self.project_notes.get("show_on_load") and self.project_notes.get("text"):
            QTimer.singleShot(200, lambda: QMessageBox.information(self, self.L("Note del progetto", "Project notes"), self.project_notes["text"]))
        self.cuts.load(self.doc.active_layer)
        self._update_title()
        self.canvas.zoom_page()

    def project_json(self):
        W, H = self.workspace()
        return json.dumps({"app": "LaserForge", "version": 4, "workspace": {"x": W, "y": H},
                           "layers": self.engine.layer_dicts(), "shapes": self.doc.pages[0]["shapes"],
                           "pages": self.doc.pages, "nextId": self.doc.next_id, "notes": self.project_notes})

    def save_project(self, as_new):
        path = self.project_path
        if as_new or not path:
            start = path or os.path.join(self.settings.value("lastDir", str(Path.home())), self.tr("untitled") + ".lfproj")
            path, _ = QFileDialog.getSaveFileName(self, self.action_label("saveAs"), start, "LaserForge (*.lfproj)")
            if not path:
                return False
            if not path.endswith(".lfproj"):
                path += ".lfproj"
        with open(path, "w", encoding="utf-8") as f:
            f.write(self.project_json())
        self.project_path = path
        self.settings.setValue("lastDir", os.path.dirname(path))
        self.add_recent(path)
        self.doc.set_dirty(False)
        self._update_title()
        self.toast(self.L("Salvato", "Saved") + f": {os.path.basename(path)}")
        return True

    def add_recent(self, path):
        self._store_json("recent", ([path] + [p for p in self._load_json("recent", []) if p != path])[:10])

    def save_text_file(self, text, default_name, filt):
        start = os.path.join(self.settings.value("lastDir", str(Path.home())), default_name)
        path, _ = QFileDialog.getSaveFileName(self, "LaserForge", start, filt)
        if path:
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            self.toast(os.path.basename(path))

    def base_name(self):
        return os.path.splitext(os.path.basename(self.project_path))[0] if self.project_path else "design"

    def export_svg(self):
        W, H = self.workspace()
        parts = []
        for s in M.flatten(self.doc.shapes):
            col = self.layer_color(s.get("layer", 0))
            if s["type"] == "image":
                parts.append(f'<image x="{s["x"]}" y="{s["y"]}" width="{s["w"]}" height="{s["h"]}" href="{s["src"]}" preserveAspectRatio="none"/>')
                continue
            for p in M.shape_paths(s):
                pts = " ".join(f"{x:.3f},{y:.3f}" for x, y in p)
                parts.append(f'<polyline fill="none" stroke="{col}" stroke-width="0.1" points="{pts}"/>')
        svg = (f'<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="{W}mm" height="{H}mm" '
               f'viewBox="0 0 {W} {H}">\n' + "\n".join(parts) + "\n</svg>\n")
        self.save_text_file(svg, self.base_name() + ".svg", "SVG (*.svg)")

    def save_gcode(self):
        if not self.gcode and not self.generate_gcode():
            return
        self.save_text_file(self.finalize_gcode(self.gcode), self.base_name() + ".gcode", "G-code (*.gcode *.nc)")

    def export_prefs(self):
        d = {"layers": self.engine.layer_dicts(), "machine": self.engine.machine_dict()}
        self.save_text_file(json.dumps(d, indent=2), "laserforge-prefs.json", "JSON (*.json)")

    def import_prefs(self):
        path, _ = QFileDialog.getOpenFileName(self, self.action_label("importPrefs"), str(Path.home()), "JSON (*.json)")
        if not path:
            return
        try:
            with open(path, encoding="utf-8") as f:
                d = json.load(f)
        except (OSError, ValueError):
            self.toast(self.L("JSON non valido", "Invalid JSON"))
            return
        if d.get("machine"):
            self.engine.update_machine(d["machine"])
            self._store_json("machine", self.engine.machine_dict())
        for i, l in enumerate((d.get("layers") or [])[:len(self.engine.layers)]):
            self.engine.update_layer(i, l)
        self.persist_layers()
        self.cuts.load(self.doc.active_layer)
        self.cuts.refresh()
        self.move_panel.load()
        self.canvas.update()
        self.toast(self.L("Preferenze importate", "Preferences imported"))

    # ================================================================ job / laser
    def optimize_paths(self, paths, cur=(0.0, 0.0)):
        """Order one bucket of paths. Returns (ordered_paths, end_position)."""
        o = self.opt
        lst = list(paths)
        boxes = [M.path_box(p) for p in lst]

        def depth_of(i):
            b = boxes[i]
            return sum(1 for j, q in enumerate(boxes) if j != i and q[0] <= b[0] and q[1] <= b[1] and q[0] + q[2] >= b[0] + b[2]
                       and q[1] + q[3] >= b[1] + b[3] and q[2] * q[3] > b[2] * b[3])
        if o.get("innerFirst", True) and len(lst) < 4000:
            depths = [depth_of(i) for i in range(len(lst))]
            order = sorted(range(len(lst)), key=lambda i: -depths[i])
            groups, band, last = [], [], None
            for i in order:
                if last is not None and depths[i] != last:
                    groups.append(band)
                    band = []
                band.append(lst[i])
                last = depths[i]
            if band:
                groups.append(band)
        else:
            groups = [lst]
        if not o.get("nearest", True):
            out = [p for g in groups for p in g]
            return out, (out[-1][-1] if out else cur)
        best_start = o.get("bestStart", True)
        best_dir = o.get("bestDirection", True)
        heavy = sum(len(p) for p in lst) * len(lst) > 4_000_000
        out = []
        for g in groups:
            pool = list(g)
            while pool:
                best, best_d, mode = 0, math.inf, ("first", 0)
                for i, p in enumerate(pool):
                    closed = M.is_closed_path(p)
                    if closed and best_start and not getattr(p, "fixed", False) and not heavy and len(p) <= 400:
                        k, d = min(((k, math.hypot(q[0] - cur[0], q[1] - cur[1])) for k, q in enumerate(p[:-1])), key=lambda t: t[1])
                        if d < best_d:
                            best, best_d, mode = i, d, ("rotate", k)
                        continue
                    d0 = math.hypot(p[0][0] - cur[0], p[0][1] - cur[1])
                    if d0 < best_d:
                        best, best_d, mode = i, d0, ("first", 0)
                    if not closed and best_dir:
                        d1 = math.hypot(p[-1][0] - cur[0], p[-1][1] - cur[1])
                        if d1 < best_d:
                            best, best_d, mode = i, d1, ("reverse", 0)
                p = pool.pop(best)
                if mode[0] == "reverse":
                    p = self._jp(p[::-1], p, reverse=True)
                elif mode[0] == "rotate" and mode[1]:
                    k = mode[1]
                    p = self._jp(p[k:-1] + p[:k] + [p[k]], p, shift=k / max(1, len(p) - 1))
                out.append(p)
                cur = p[-1]
        return out, cur

    @staticmethod
    def _jp(points, src, reverse=False, shift=0.0):
        """New JobPath keeping the metadata of `src`; tab fractions follow a reversal or a start shift (approximate)."""
        jp = JobPath(points)
        for k in ("group", "priority", "fixed"):
            setattr(jp, k, getattr(src, k, None))
        tabs = getattr(src, "tabs", None)
        if tabs:
            tabs = [(t - shift) % 1.0 for t in tabs]
            jp.tabs = [1.0 - t for t in tabs] if reverse else tabs
        return jp

    @staticmethod
    def _signed_area(path):
        return sum(path[i - 1][0] * path[i][1] - path[i][0] * path[i - 1][1] for i in range(1, len(path))) / 2

    def _remove_overlaps(self, paths):
        """Merge collinear/overlapping cut lines so each piece is cut once (paths with tabs are left alone)."""
        from shapely.geometry import LineString, MultiLineString
        from shapely.ops import linemerge, unary_union
        keep = [p for p in paths if getattr(p, "tabs", None)]
        lines = [LineString(p) for p in paths if not getattr(p, "tabs", None) and len(p) >= 2]
        if len(lines) < 2:
            return paths
        merged = linemerge(unary_union(lines))
        geoms = getattr(merged, "geoms", [merged])
        ref = paths[0]
        out = list(keep)
        for g in geoms:
            coords = [(round(x, 6), round(y, 6)) for x, y in g.coords]
            if len(coords) >= 2:
                out.append(self._jp(coords, ref))
        return out

    def job_transform(self, b):
        pc = self.print_cut
        if pc and pc.get("enabled") and pc.get("src") and all(pc.get("dst", [None, None])):
            (s1, s2), (d1, d2) = pc["src"], pc["dst"]
            vs, vd = (s2[0] - s1[0], s2[1] - s1[1]), (d2[0] - d1[0], d2[1] - d1[1])
            k = math.hypot(*vd) / (math.hypot(*vs) or 1)
            a = math.atan2(vd[1], vd[0]) - math.atan2(vs[1], vs[0])
            c, s = math.cos(a) * k, math.sin(a) * k
            return lambda p: (d1[0] + (p[0] - s1[0]) * c - (p[1] - s1[1]) * s, d1[1] + (p[0] - s1[0]) * s + (p[1] - s1[1]) * c)
        if self.start_from == "absolute" or not b:
            return None
        fx, fy = ORIGIN_POS[self.job_origin]
        ox, oy = b[0] + b[2] * fx, b[1] + b[3] * fy
        tx, ty = self.to_canvas(self.machine_pos if self.start_from == "current" else self.user_origin)
        dx, dy = tx - ox, ty - oy
        return lambda p: (p[0] + dx, p[1] + dy)

    # ---------------------------------------------------------------- what the laser sees
    def _output_layer(self, layer):
        ls = self.engine.layers
        return 0 <= layer < len(ls) and not ls[layer].tool

    def _area_state(self, s):
        """'in', 'partial' or 'out' of the work area (the grid)."""
        W, H = self.workspace()
        b = M.bounds(s)
        eps = 1e-6
        if not b or b[0] > W + eps or b[1] > H + eps or b[0] + b[2] < -eps or b[1] + b[3] < -eps:
            return "out"
        if b[0] < -eps or b[1] < -eps or b[0] + b[2] > W + eps or b[1] + b[3] > H + eps:
            return "partial"
        return "in"

    def output_shapes(self, src, enabled_only=True):
        """Shapes the laser will burn: output layers (not T1/T2), at least partly on the work area."""
        out = []
        for s in M.flatten(src):
            lay = s.get("layer", 0)
            if not self._output_layer(lay) or (enabled_only and not self.engine.layers[lay].enabled):
                continue
            if self._area_state(s) != "out":
                out.append(s)
        return out

    def clip_to_area(self, b):
        """Bounding box (canvas) limited to the work area."""
        if not b:
            return b
        W, H = self.workspace()
        x0, y0 = max(0.0, b[0]), max(0.0, b[1])
        x1, y1 = min(W, b[0] + b[2]), min(H, b[1] + b[3])
        return (x0, y0, max(0.0, x1 - x0), max(0.0, y1 - y0)) if x1 > x0 and y1 > y0 else None

    def _clip_paths(self, paths, fill):
        """Cut vector paths at the edge of the work area: nothing outside the grid reaches the laser."""
        from shapely.geometry import LineString, Polygon, box
        from shapely.validation import make_valid
        W, H = self.workspace()
        B = box(0, 0, W, H)
        out = []
        for path in paths:
            if all(0 <= x <= W and 0 <= y <= H for x, y in path):
                out.append(path)
                continue
            if fill and M.is_closed_path(path) and len(path) >= 4:
                g = make_valid(Polygon(path)).intersection(B)
                for poly in getattr(g, "geoms", [g]):
                    if poly.geom_type == "Polygon" and not poly.is_empty:
                        for ring in [poly.exterior, *poly.interiors]:
                            out.append([tuple(c) for c in ring.coords])
            else:
                g = LineString(path).intersection(B)
                for ln in getattr(g, "geoms", [g]):
                    if ln.geom_type == "LineString" and len(ln.coords) > 1:
                        out.append([tuple(c) for c in ln.coords])
        return out

    def _mask_image_to_area(self, s, burn):
        import numpy as np
        W, H = self.workspace()
        rows, cols = burn.shape
        xs = s["x"] + (np.arange(cols) + 0.5) * s["w"] / cols
        ys = s["y"] + (np.arange(rows) + 0.5) * s["h"] / rows
        burn = burn.copy()
        burn[:, (xs < 0) | (xs > W)] = 0
        burn[(ys < 0) | (ys > H), :] = 0
        return burn

    def build_job(self, only_selection=False):
        from .tools import job_path
        src = self.doc.selected_shapes() if only_selection and self.doc.selected else self.doc.shapes
        o = self.opt
        entries, rasters = [], []          # entries: (group_index, priority, layer, JobPath)
        n_out = n_clip = 0
        for gi, top in enumerate(src):
            for s in M.flatten([top]):
                layer = s.get("layer", 0)
                if not self._output_layer(layer):
                    continue                    # T1/T2 tool layers never reach the laser
                area = self._area_state(s)
                if area == "out":
                    n_out += 1
                    continue                    # off the grid: the laser does not see it
                n_clip += area == "partial"
                if s["type"] == "image":
                    lay_ = self.engine.layers[layer] if 0 <= layer < len(self.engine.layers) else None
                    res = M.process_image(s, self.layer_interval(layer), pass_through=bool(lay_ and lay_.pass_through))
                    if res:
                        burn, cols, rows = res
                        if area == "partial":
                            burn = self._mask_image_to_area(s, burn)
                        want = max(s["w"], s["h"]) / self.layer_interval(layer)
                        if not (lay_ and lay_.pass_through) and want > M.MAX_RASTER_PX + 1:
                            self.toast(self.L(f"Immagine troppo grande per l'intervallo: ridotta a {M.MAX_RASTER_PX} px per lato",
                                              f"Image too large for the interval: reduced to {M.MAX_RASTER_PX} px per side"))
                        adj = {**M.default_adj(), **(s.get("adj") or {})}
                        if adj.get("mode") == "slices3d":
                            # 3D sliced engraving: one full-power pass per depth level, darker = more passes = deeper
                            n = max(2, int(adj.get("slices", 10)))
                            for k in range(n):
                                thr = (k + 0.5) * 255 / n
                                passd = (burn > thr).astype("uint8") * 255
                                if passd.any():
                                    rasters.append({"layer_index": layer, "x": s["x"], "y": s["y"], "w": s["w"], "h": s["h"],
                                                    "cols": cols, "rows": rows, "data": passd.reshape(-1).tolist(), "slice": k + 1})
                        else:
                            rasters.append({"layer_index": layer, "x": s["x"], "y": s["y"], "w": s["w"], "h": s["h"],
                                            "cols": cols, "rows": rows, "data": burn.reshape(-1).tolist()})
                    continue
                prio = int(s.get("priority", top.get("priority", 0)) or 0)
                raw = [p for p in M.shape_paths(s) if len(p) > 1]
                if area == "partial":
                    lay_obj = self.engine.layers[layer]
                    raw = [p for p in self._clip_paths(raw, lay_obj.cut_mode.value != "line") if len(p) > 1]
                for k, path in enumerate(raw):
                    jp = JobPath(job_path(s, path) if area == "in" else path)
                    jp.group, jp.priority = gi, prio
                    jp.fixed = area == "in" and (bool(s.get("start")) or bool(s.get("tabs")))
                    if area == "in" and k == 0 and s.get("tabs") and M.is_closed_path(path):
                        f0 = s.get("start", 0.0)
                        fr = [((t - f0) % 1.0) for t in s["tabs"]]
                        jp.tabs = [1.0 - t for t in fr] if s.get("reverse") else fr
                    entries.append((gi, prio, layer, jp))
        origin_src = self.doc.selected_shapes() if (self.prefs.get("origin_from_selection") and self.doc.selected) else src
        tf = self.job_transform(self.clip_to_area(M.multi_bounds(self.output_shapes(origin_src, False))))
        msg = (n_out, n_clip)
        if any(msg) and msg != getattr(self, "_last_area_msg", None):
            parts = []
            if n_out:
                parts.append(self.L(f"{n_out} forme fuori dall'area ignorate", f"{n_out} shapes outside the area ignored"))
            if n_clip:
                parts.append(self.L(f"{n_clip} tagliate al bordo", f"{n_clip} cut at the edge"))
            self.toast(" · ".join(parts))
        self._last_area_msg = msg
        cyl = self.cylinder_transform()
        direction = o.get("direction", "keep")
        fixed = []
        for gi, prio, layer, jp in entries:
            pts = list(jp)
            if tf:
                pts = [tf(p) for p in pts]
            if cyl:
                pts = [cyl(p) for p in pts]
            pts = [self.to_machine(p) for p in pts]
            new = self._jp(pts, jp)
            if direction != "keep" and M.is_closed_path(pts) and not getattr(jp, "fixed", False):
                # screen coordinates (Y down): positive signed area = clockwise on screen
                cw = self._signed_area(pts) > 0
                if (direction == "cw") != cw:
                    new = self._jp(pts[::-1], new, reverse=True)
            fixed.append((gi, prio, layer, new))
        # buckets in the chosen order
        mode = o.get("order", "layer")
        if mode == "group":
            keys = sorted({(gi, layer) for gi, _, layer, _ in fixed})
            bucket_of = lambda e: (e[0], e[2])
        elif mode == "priority":
            keys = sorted({(prio, layer) for _, prio, layer, _ in fixed})
            bucket_of = lambda e: (e[1], e[2])
        else:
            keys = sorted({layer for _, _, layer, _ in fixed})
            bucket_of = lambda e: e[2]
        buckets = {k: [] for k in keys}
        for e in fixed:
            buckets[bucket_of(e)].append(e[3])
        layers, cur = [], (0.0, 0.0)
        for k in keys:
            paths = buckets[k]
            if o.get("removeOverlap"):
                paths = self._remove_overlaps(paths)
            ordered, cur = self.optimize_paths(paths, cur)
            layers.append({"layer_index": k if mode == "layer" else k[1], "paths": ordered})
        if tf:
            for r in rasters:
                r["x"], r["y"] = tf((r["x"], r["y"]))
        self._rasters_to_machine(rasters)
        job = {"layers": layers, "rasters": rasters}
        pc = self.print_cut
        if tf and self.start_from in ("current", "user") and not (pc and pc.get("enabled")):
            # LightBurn: these modes are sent relative to the head (G91), see finalize_gcode
            job["origin_mode"] = self.start_from
            job["start"] = tuple(self.machine_pos[:2]) if self.start_from == "current" else tuple(self.user_origin[:2])
        return job

    def _rasters_to_machine(self, rasters):
        """Images: move the box to the machine origin and flip the bitmap so row 0 is at the lowest machine Y."""
        import numpy as np
        W, H = self.workspace()
        o = self.engine.machine.origin
        flip_x, flip_y = o in ("rr", "fr"), o in ("fl", "fr")
        for r in rasters:
            a = np.asarray(r["data"], dtype=np.uint8).reshape(int(r["rows"]), int(r["cols"]))
            if flip_x:
                r["x"] = W - (r["x"] + r["w"])
                a = a[:, ::-1]
            if flip_y:
                r["y"] = H - (r["y"] + r["h"])
                a = a[::-1, :]
            r["data"] = a.reshape(-1).tolist()

    def cylinder_transform(self):
        """Cylinder correction (gantry laser, vertical beam): design distance s along the surface lands at
        R·sin(s/R) from the cylinder's top line, so engravings keep their size on round objects."""
        c = self._load_json("cylinder", {})
        if not c.get("enabled") or c.get("diameter", 0) <= 0:
            return None
        R = c["diameter"] / 2
        axis = c.get("axis", "x")              # cylinder axis direction on the bed
        centre = float(c.get("centre", 0.0))   # coordinate of the cylinder's top line across the axis
        D = float(c.get("mirror", 0.0) or 0.0)  # galvo: mirror height above the cylinder top (0 = gantry, vertical beam)
        lim = R * math.pi / 2

        def across(dist):
            th = max(-lim, min(lim, dist)) / R
            lateral = R * math.sin(th)
            if D > 0:
                # the galvo aims at the focal plane (cylinder top): the slanted beam reaches the lower surface farther out,
                # so aim where the ray from the mirror to the surface point crosses that plane
                lateral *= D / (D + R * (1 - math.cos(th)))
            return lateral

        def f(p):
            x, y = p
            if axis == "x":            # cylinder lies along X: correct Y
                return (x, centre + across(y - centre))
            return (centre + across(x - centre), y)
        return f

    def generate_gcode(self):
        job = self.build_job(only_selection=bool(self.prefs["cut_selected"]))
        if not job["layers"] and not job["rasters"]:
            self.toast(self.tr("noLayers"))
            return None
        self.status(self.L("Generazione G-code...", "Generating G-code..."))
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            d = self.engine.generate(job)
        finally:
            QApplication.restoreOverrideCursor()
        if not d:
            self.toast(self.tr("noLayers"))
            return None
        self.gcode = d["gcode"]
        self._gcode_origin = (job.get("origin_mode"), job.get("start"))
        self.laser_panel.show_gcode(d["gcode"], d["estimate"], d["lines"])
        self.status(f"G-code: {d['lines']} {self.L('righe', 'lines')}, {d['estimate']['estimated']}")
        return d

    def finalize_gcode(self, gcode):
        """What actually goes to the laser / file: "Current position" and "User origin" jobs become
        relative (G91) like LightBurn's, so they start exactly where the head is."""
        from .gcode import to_relative
        mode, start = getattr(self, "_gcode_origin", (None, None))
        if mode in ("current", "user") and start is not None:
            return to_relative(gcode, start, user_origin=(mode == "user"))
        return gcode

    def show_gcode(self, gcode):
        self.gcode = gcode
        self._gcode_origin = (None, None)
        self.laser_panel.show_gcode(gcode)

    def start_laser(self):
        if not self.gcode and not self.generate_gcode():
            return
        if self.connected:
            ok = self.confirm_in_area(self.job_bounds(self.build_job(only_selection=bool(self.prefs["cut_selected"]))))
            if ok == "retry":
                self.gcode = ""
                return self.start_laser()
            if not ok:
                return
        if self.connected and not DV.run_checklist(self):
            self.status(self.L("Avvio annullato", "Start cancelled"))
            return
        if not self.connected:
            self.toast(self.L("Connetti prima il laser (pannello Laser)", "Connect a laser first (Laser panel)"))
            self.dock_laser.show()
            self.dock_laser.raise_()
            return
        from .gcode import apply_feeder
        self.engine.send(self.finalize_gcode(apply_feeder(self.gcode, self.engine.machine)))
        self.status(self.L("Lavoro inviato", "Job sent"))
        self.advance_variable_text()

    def advance_variable_text(self):
        changed = False

        def walk(shapes):
            nonlocal changed
            for s in shapes:
                if s["type"] == "text" and M.advance_variable(s):
                    changed = True
                walk(s.get("children", []))
        walk(self.doc.shapes)
        if changed:
            self.doc.set_dirty(True)
            self.doc.notify()

    def camera_capture(self):
        if self.camera_panel.camera is None:
            self.dock_camera.show()
            self.dock_camera.raise_()
            self.camera_panel.start_btn.setChecked(True)
            QTimer.singleShot(1500, self.camera_panel.capture_overlay)
        else:
            self.camera_panel.capture_overlay()

    def frame_job(self):
        src = self.doc.selected_shapes() or self.doc.shapes
        b = self.clip_to_area(M.multi_bounds(self.output_shapes(src)))
        if not b:
            self.toast(self.tr("noLayers"))
            return
        tf = self.job_transform(b)
        if tf:
            pts = [tf(p) for p in ((b[0], b[1]), (b[0] + b[2], b[1]), (b[0], b[1] + b[3]), (b[0] + b[2], b[1] + b[3]))]
            b = M.path_box(pts)
        if not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        b = M.path_box([self.to_machine(p) for p in ((b[0], b[1]), (b[0] + b[2], b[1] + b[3]))])
        ok = self.confirm_in_area(b)
        if ok == "retry":
            return self.frame_job()
        if not ok:
            return
        on = bool(self.prefs["frame_laser_on"])
        rel = tuple(self.machine_pos[:2]) if self.start_from == "current" else None
        self._send_frame(self.engine.frame((b[0], b[1], b[0] + b[2], b[1] + b[3]), on, float(self.prefs["frame_power"]),
                                           self.move_panel.speed.value(), relative_to=rel,
                                           head=tuple(self.machine_pos[:2])))

    def rubber_band_frame(self):
        """Trace the convex outline around the job with the beam off (LightBurn's Rubber-Band Frame)."""
        from shapely.geometry import MultiPoint
        job = self.build_job(only_selection=bool(self.doc.selected))
        pts = [p for l in job["layers"] for path in l["paths"] for p in path]
        for r in job["rasters"]:
            pts += [(r["x"], r["y"]), (r["x"] + r["w"], r["y"]), (r["x"], r["y"] + r["h"]), (r["x"] + r["w"], r["y"] + r["h"])]
        if len(pts) < 3:
            self.toast(self.tr("noLayers"))
            return
        if not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        hull = MultiPoint(pts).convex_hull
        ring = list(hull.exterior.coords) if hull.geom_type == "Polygon" else list(hull.coords)
        speed = self.move_panel.speed.value()
        if self.prefs["frame_laser_on"]:
            s_val = round(float(self.prefs["frame_power"]) / 100 * self.engine.machine.max_power)
            g = ["; Rubber-band frame (laser on)", "G21", "G90", "M4 S0", f"G0 X{ring[0][0]:.3f} Y{ring[0][1]:.3f}"]
            g += [f"G1 X{x:.3f} Y{y:.3f} F{speed:.0f} S{s_val}" for x, y in ring[1:]]
            g += ["S0", "M5"]
        else:
            g = ["; Rubber-band frame (beam off)", "G21", "G90", "M5", f"G0 X{ring[0][0]:.3f} Y{ring[0][1]:.3f}"]
            g += [f"G1 X{x:.3f} Y{y:.3f} F{speed:.0f} S0" for x, y in ring[1:]]
        self._send_frame("\n".join(g))

    def _send_frame(self, gcode):
        self._last_frame = gcode
        self.engine.send(gcode)

    def toggle_continuous_frame(self):
        self.frame_continuous = not getattr(self, "frame_continuous", False)
        if self.frame_continuous and not getattr(self, "_last_frame", None):
            self.frame_job()
        self.status(self.L("Frame continuo attivo: Stop per annullare", "Continuous framing on: Stop to cancel")
                    if self.frame_continuous else self.tr("ready"))
        self.refresh_checks()

    def _repeat_frame(self):
        if getattr(self, "frame_continuous", False) and getattr(self, "_last_frame", None) and self.connected:
            sent, total = self.engine.progress()
            if self.engine.state() != "busy" and (not total or sent >= total):
                self.engine.send(self._last_frame)

    def stop_laser(self):
        if getattr(self, "frame_continuous", False):
            self.frame_continuous = False
            self.refresh_checks()
        self.engine.stop()

    def laser_to(self, where):
        sel = self.need_sel()
        if not sel:
            return
        b = M.multi_bounds(sel)
        x = b[0] if "L" in where else b[0] + b[2] if "R" in where else b[0] + b[2] / 2
        y = b[1] if "U" in where else b[1] + b[3] if "D" in where else b[1] + b[3] / 2
        self.goto_canvas(x, y)

    def _position_laser(self, x, y):
        self.goto_canvas(x, y)
        mx, my = self.to_machine((x, y))
        self.status(f"G0 X{mx:.2f} Y{my:.2f}")

    def set_user_origin(self):
        if not self.connected:
            self.toast(self.L("Connetti il laser: l'origine utente è la posizione attuale della testa", "Connect the laser: the user origin is the head's current position"))
            return
        self.settings.setValue("userOriginSet", True)
        self.user_origin = self.machine_pos
        self._store_json("userOrigin", list(self.user_origin))
        self.gcode = ""
        self.toast(self.L("Origine utente", "User origin") + f": X {self.machine_pos[0]:.2f} Y {self.machine_pos[1]:.2f}")

    def set_start_from(self, v):
        if v == "user" and not self.settings.value("userOriginSet", False, type=bool):
            self.toast(self.L("Imposta prima l'origine utente: porta il laser nel punto voluto e premi «Imposta origine»",
                              "Set the user origin first: jog the laser to the spot and press “Set origin”"))
            v = "absolute"
        self.start_from = v
        self.settings.setValue("startFrom", v)
        self.gcode = ""
        self.laser_panel.refresh_origin()

    def set_job_origin(self, k):
        self.job_origin = k
        self.settings.setValue("origin", k)
        self.gcode = ""
        self.laser_panel.refresh_origin()

    def toggle_connection(self):
        if self.connected:
            self.engine.disconnect()
        else:
            if self.engine.connect(self.laser_panel.port.currentData() or "SIM"):
                QTimer.singleShot(1600, self.check_grbl_settings)
        self._poll()

    def show_position(self):
        if not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        x, y, z, _ms = self.engine.position()
        msg = f"X {x:.3f}  Y {y:.3f}" + (f"  Z {z:.3f}" if self.engine.machine.z_enabled else "")
        self.engine.log(self.L("Posizione: ", "Position: ") + msg, "status")
        self.toast(msg)

    def zero_here(self):
        if not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        self.engine.zero_xy()
        self.toast(self.L("Zero di lavoro impostato qui (G10 L20)", "Work zero set here (G10 L20)"))

    def clear_zero(self):
        if not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        self.engine.clear_zero()
        self.toast(self.L("Zero di lavoro cancellato", "Work zero cleared"))

    def home_laser(self):
        if self.engine.home() is False:
            self.toast(self.L("Homing non abilitato in GRBL ($22=0): attivalo in Laser → Impostazioni macchina",
                              "Homing is not enabled in GRBL ($22=0): enable it in Laser → Machine Settings"))

    def check_grbl_settings(self):
        """After connecting: LightBurn-style warnings about the controller's settings, with a fix for $32."""
        probs = self.engine.grbl_problems()
        if not probs or not self.connected:
            return
        box = QMessageBox(self)
        box.setIcon(QMessageBox.Warning)
        box.setWindowTitle("LaserForge")
        box.setText(self.L("Controlla le impostazioni di GRBL", "Check the GRBL settings"))
        box.setInformativeText("\n\n".join(probs))
        fix = box.addButton(self.L("Attiva modalità laser ($32=1)", "Enable laser mode ($32=1)"), QMessageBox.AcceptRole) \
            if self.engine.grbl_settings.get(32) == 0 else None
        box.addButton(self.L("Apri Impostazioni macchina", "Open Machine Settings"), QMessageBox.ActionRole)
        box.addButton(QMessageBox.Close)
        box.exec()
        if fix is not None and box.clickedButton() is fix:
            self.engine.command("$32=1")
            self.engine.grbl_settings[32] = 1.0
            self.toast(self.L("Modalità laser attivata ($32=1)", "Laser mode enabled ($32=1)"))
        elif box.clickedButton() and box.buttonRole(box.clickedButton()) == QMessageBox.ActionRole:
            D.machine_settings_dialog(self)

    def jog(self, dx, dy, dz=0):
        """dx/dy in screen directions (▲ = towards the back of the machine)."""
        step = self.move_panel.step.value() or 1.0
        self.jog_delta(dx * step, dy * step, dz * step)

    def jog_delta(self, dx, dy, dz=0.0):
        sx, sy = self.jog_signs()
        self.engine.jog(x=dx * sx, y=dy * sy, z=dz, feed=self.move_panel.speed.value())

    def fire_laser(self, on):
        if not self.engine.machine.fire_enabled:
            return
        if on and not self.connected:
            self.toast(self.L("Connetti prima il laser", "Connect a laser first"))
            return
        if on and self.engine.state() == "busy":
            return
        self._firing = bool(on)
        self.engine.fire(on)

    def air_test(self, mode):
        self.engine.air(mode)

    def send_and_show_console(self, cmd):
        self.engine.command(cmd)
        self.dock_console.show()
        self.dock_console.raise_()
        QTimer.singleShot(300, self.console_panel.refresh)

    # ================================================================ window lifecycle
    def closeEvent(self, ev):
        if not self.confirm_discard():
            ev.ignore()
            return
        self.settings.setValue("geometry", self.saveGeometry())
        self.settings.setValue("windowState", self.saveState(self.LAYOUT_VERSION))
        self._store_json("machine", self.engine.machine_dict())
        self.persist_layers()
        self._store_json("camera", self.camera_cal)
        self.camera_panel.shutdown()
        self.remote.stop()
        try:
            if self.connected:
                self.engine.disconnect()
        except Exception:
            pass
        ev.accept()
