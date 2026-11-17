"""Dock panels: Cuts/Layers, Laser, Move, Console, Objects, Library."""

import html

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QBrush, QColor, QFont, QTextCursor
from PySide6.QtWidgets import (
    QAbstractItemView, QComboBox, QFrame, QGridLayout, QGroupBox, QHBoxLayout, QHeaderView, QLabel, QLineEdit,
    QListWidget, QListWidgetItem, QPlainTextEdit, QPushButton, QScrollArea, QSizePolicy, QSlider, QTableWidget,
    QTableWidgetItem, QTextEdit, QToolButton, QVBoxLayout, QWidget,
)

from . import model as M
from .engine import MATERIAL_PRESETS
from .ui_common import AIR, Segmented, checkbox, dspin, hint, ispin


def swatch(color, size=12):
    lab = QLabel()
    lab.setFixedSize(size, size)
    lab.setStyleSheet(f"background:{color}; border:1px solid #111; border-radius:2px;")
    return lab


def labeled(text, widget):
    w = QWidget()
    lay = QVBoxLayout(w)
    lay.setContentsMargins(0, 0, 0, 0)
    lay.setSpacing(2)
    lab = QLabel(text)
    lab.setStyleSheet("color:#a8aab1; font-size:11px;")
    lay.addWidget(lab)
    lay.addWidget(widget)
    w.label = lab
    return w


def row(*widgets):
    w = QWidget()
    lay = QHBoxLayout(w)
    lay.setContentsMargins(0, 0, 0, 0)
    lay.setSpacing(8)
    for x in widgets:
        lay.addWidget(x, 1)
    return w


# ====================================================================== CUTS / LAYERS
class CutsPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        self._loading = False
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)

        self.table = QTableWidget(0, 6)
        self.table.verticalHeader().hide()
        self.table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.table.setSelectionMode(QAbstractItemView.SingleSelection)
        self.table.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.table.setShowGrid(False)
        self.table.setAlternatingRowColors(True)
        hh = self.table.horizontalHeader()
        hh.setSectionResizeMode(QHeaderView.ResizeToContents)
        hh.setSectionResizeMode(1, QHeaderView.Stretch)
        self.table.verticalHeader().setDefaultSectionSize(24)
        self.table.cellClicked.connect(self._cell_clicked)
        lay.addWidget(self.table)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        body = QWidget()
        scroll.setWidget(body)
        b = QVBoxLayout(body)
        b.setContentsMargins(2, 2, 6, 2)
        b.setSpacing(8)
        lay.addWidget(scroll, 1)

        head = QHBoxLayout()
        self.sw = swatch("#000", 14)
        self.title = QLabel("C00")
        self.title.setStyleSheet("font-weight:600; font-size:13px;")
        head.addWidget(self.sw)
        head.addWidget(self.title)
        head.addStretch(1)
        b.addLayout(head)

        self.speed = dspin(1000, 1, 100000, 10, 0)
        self.speed_units = "mm/min"
        self.speed_factor = 1.0
        self.passes = ispin(1, 1, 100)
        self.w_speed, self.w_passes = labeled("", self.speed), labeled("", self.passes)
        b.addWidget(row(self.w_speed, self.w_passes))

        def power_row():
            sl = QSlider(Qt.Horizontal)
            sl.setRange(0, 100)
            sp = dspin(100, 0, 100, 1, 1, " %", 70)
            sl.valueChanged.connect(lambda v: (sp.blockSignals(True), sp.setValue(v), sp.blockSignals(False)))
            sl.sliderReleased.connect(self.save)
            sl.valueChanged.connect(lambda _v: None if sl.isSliderDown() else self.save())
            sp.valueChanged.connect(lambda v: (sl.blockSignals(True), sl.setValue(int(round(v))), sl.blockSignals(False), self.save()))
            w = QWidget()
            h = QHBoxLayout(w)
            h.setContentsMargins(0, 0, 0, 0)
            h.addWidget(sl, 1)
            h.addWidget(sp)
            return w, sl, sp

        pmax_w, self.pmax_sl, self.pmax = power_row()
        pmin_w, self.pmin_sl, self.pmin = power_row()
        self.w_pmax, self.w_pmin = labeled("", pmax_w), labeled("", pmin_w)
        b.addWidget(self.w_pmax)
        b.addWidget(self.w_pmin)

        self.mode = Segmented([("line", ""), ("fill", ""), ("offset_fill", ""), ("line_fill", "")])
        self.w_mode = labeled("", self.mode)
        b.addWidget(self.w_mode)
        self.laser = Segmented([("M3", ""), ("M4", "")])
        self.w_laser = labeled("Laser", self.laser)
        b.addWidget(self.w_laser)

        self.fill_box = QGroupBox()
        fb = QVBoxLayout(self.fill_box)
        self.interval = dspin(0.1, 0.01, 5, 0.01, 3)
        self.overscan = dspin(2.5, 0, 50, 0.5, 2)
        self.w_interval, self.w_overscan = labeled("", self.interval), labeled("Overscan (mm)", self.overscan)
        fb.addWidget(row(self.w_interval, self.w_overscan))
        self.angle = Segmented([(0.0, "0°"), (45.0, "45°"), (90.0, "90°"), (135.0, "135°")])
        self.w_angle = labeled("", self.angle)
        fb.addWidget(self.w_angle)
        self.bidir = checkbox("", True)
        self.cross = checkbox("Cross-hatch")
        fb.addWidget(row(self.bidir, self.cross))
        self.flood = checkbox("Flood Fill")
        fb.addWidget(self.flood)
        b.addWidget(self.fill_box)

        # image options (LightBurn: Pass-Through, Dot Width Correction, bottom-up)
        self.img_box = QGroupBox()
        ib = QVBoxLayout(self.img_box)
        self.pass_through = checkbox("")
        self.bottom_up = checkbox("")
        ib.addWidget(row(self.pass_through, self.bottom_up))
        self.dot_width = dspin(0, 0, 1, 0.01, 3)
        self.w_dotw = labeled("", self.dot_width)
        ib.addWidget(self.w_dotw)
        b.addWidget(self.img_box)

        self.air_box = QGroupBox()
        self.air_box.setStyleSheet(f"QGroupBox {{ border-color: {AIR}; }} QGroupBox::title {{ color: {AIR}; }}")
        ab = QVBoxLayout(self.air_box)
        self.air = Segmented([("off", ""), ("on", "")], air=True)
        self.air_power = dspin(100, 0, 100, 5, 0, " %", 64)
        hr = QHBoxLayout()
        hr.addWidget(self.air, 1)
        hr.addWidget(self.air_power)
        ab.addLayout(hr)
        self.air_cut = checkbox("")
        ab.addWidget(self.air_cut)
        tests = QHBoxLayout()
        self.air_test_buttons = []
        for mode, label in (("on", "ON"), ("off", "OFF")):
            bt = QPushButton(label)
            bt.clicked.connect(lambda _=False, m=mode: self.win.air_test(m))
            tests.addWidget(bt)
            self.air_test_buttons.append((mode, bt))
        ab.addLayout(tests)
        b.addWidget(self.air_box)

        self.adv_toggle = QToolButton()
        self.adv_toggle.setCheckable(True)
        self.adv_toggle.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
        self.adv_toggle.setArrowType(Qt.RightArrow)
        self.adv_toggle.setStyleSheet("QToolButton { border:none; font-weight:600; }")
        self.adv = QWidget()
        av = QVBoxLayout(self.adv)
        av.setContentsMargins(0, 0, 0, 0)
        self.z_offset = dspin(0, -50, 50, 0.1, 2)
        self.power_scale = dspin(100, 1, 100, 1, 0)
        self.kerf = dspin(0, -2, 2, 0.01, 3)
        self.dwell = dspin(0, 0, 1000, 1, 0)
        self.tabs = ispin(0, 0, 20)
        self.tab_w = dspin(2, 0.5, 20, 0.5, 1)
        self.ramp = dspin(0, 0, 50, 0.5, 1)
        self.delay = dspin(0, 0, 1000, 1, 0)
        self.z_step = dspin(0, 0, 20, 0.1, 2)
        self.dot = checkbox("")
        self.w_zoff, self.w_pscale = labeled("Z offset (mm)", self.z_offset), labeled("", self.power_scale)
        self.w_kerf, self.w_dwell = labeled("Kerf (mm)", self.kerf), labeled("Dwell (ms)", self.dwell)
        self.w_tabs, self.w_tabw = labeled("", self.tabs), labeled("", self.tab_w)
        self.w_ramp, self.w_delay = labeled("", self.ramp), labeled("", self.delay)
        self.w_zstep = labeled("", self.z_step)
        for a, c in ((self.w_zoff, self.w_pscale), (self.w_kerf, self.w_dwell), (self.w_tabs, self.w_tabw), (self.w_ramp, self.w_delay), (self.w_zstep, self.dot)):
            av.addWidget(row(a, c))
        # LightBurn cut options: lead-in / lead-out, overcut, perforation, sub-layers
        self.lead_in, self.lead_out, self.overcut = dspin(0, 0, 50, 0.5, 1), dspin(0, 0, 50, 0.5, 1), dspin(0, 0, 50, 0.5, 1)
        self.w_lin, self.w_lout, self.w_over = labeled("", self.lead_in), labeled("", self.lead_out), labeled("", self.overcut)
        av.addWidget(row(self.w_lin, self.w_lout))
        av.addWidget(self.w_over)
        self.perf = checkbox("")
        self.perf_cut, self.perf_skip = dspin(1, 0.1, 100, 0.1, 2), dspin(1, 0.1, 100, 0.1, 2)
        self.w_pcut, self.w_pskip = labeled("", self.perf_cut), labeled("", self.perf_skip)
        av.addWidget(self.perf)
        av.addWidget(row(self.w_pcut, self.w_pskip))
        self.sub_btn = QPushButton()
        self.sub_btn.clicked.connect(lambda: __import__("laserforge.dialogs", fromlist=["x"]).sublayers_dialog(self.win))
        av.addWidget(self.sub_btn)
        self.adv.setVisible(False)
        self.adv_toggle.toggled.connect(lambda on: (self.adv.setVisible(on), self.adv_toggle.setArrowType(Qt.DownArrow if on else Qt.RightArrow)))
        b.addWidget(self.adv_toggle)
        b.addWidget(self.adv)
        b.addStretch(1)

        for w in (self.speed, self.passes, self.interval, self.overscan, self.air_power, self.z_offset, self.power_scale,
                  self.kerf, self.dwell, self.tabs, self.tab_w, self.ramp, self.delay, self.z_step, self.dot_width,
                  self.lead_in, self.lead_out, self.overcut, self.perf_cut, self.perf_skip):
            w.valueChanged.connect(self.save)
        for w in (self.mode, self.laser, self.angle, self.air):
            w.changed.connect(self.save)
        for w in (self.bidir, self.cross, self.dot, self.pass_through, self.bottom_up, self.air_cut, self.flood, self.perf):
            w.toggled.connect(self.save)
        self.retranslate()

    def retranslate(self):
        t, L = self.win.tr, self.win.L
        self.w_speed.label.setText(t("speed").split("(")[0].strip() + f" ({self.speed_units})")
        self.w_passes.label.setText(t("passes"))
        self.w_pmax.label.setText(t("powerMax"))
        self.w_pmin.label.setText(t("powerMin"))
        self.w_mode.label.setText(t("mode"))
        self.mode.setLabels({"line": t("modeLine"), "fill": t("modeFill"), "offset_fill": t("modeOffsetFill"), "line_fill": t("modeLineFill")})
        self.laser.setLabels({"M3": t("laserConst"), "M4": t("laserDyn")})
        self.fill_box.setTitle(L("Riempimento", "Fill"))
        self.w_interval.label.setText(t("interval"))
        self.w_angle.label.setText(t("scanAngle"))
        self.bidir.setText(t("bidir"))
        self.img_box.setTitle(L("Immagine", "Image"))
        self.pass_through.setText("Pass-Through")
        self.pass_through.setToolTip(L("Usa i pixel dell'immagine così come sono (già retinata): niente ricampionamento né regolazioni",
                                       "Use the image pixels as they are (already dithered): no resampling, no adjustments"))
        self.bottom_up.setText(L("Dal basso in alto", "Bottom-up"))
        self.w_dotw.label.setText(L("Correzione larghezza punto (mm)", "Dot width correction (mm)"))
        self.dot_width.setToolTip(L("Accorcia ogni punto inciso di questa misura, per compensare la larghezza del raggio",
                                    "Shortens every burned dot by this amount to compensate for the beam width"))
        self.air_box.setTitle(t("airAssist"))
        self.air_cut.setText(L("Solo durante il taglio (spenta negli spostamenti)", "Only while cutting (off during travel)"))
        self.flood.setToolTip(L("Riempie un'area alla volta invece di attraversare tutto il disegno a ogni riga", "Fills one area at a time instead of scanning across the whole design"))
        self.w_lin.label.setText("Lead-in (mm)")
        self.w_lout.label.setText("Lead-out (mm)")
        self.w_over.label.setText(L("Overcut (mm, forme chiuse)", "Overcut (mm, closed shapes)"))
        self.perf.setText(L("Perforazione (taglio tratteggiato)", "Perforation (dashed cut)"))
        self.w_pcut.label.setText(L("Taglio (mm)", "Cut (mm)"))
        self.w_pskip.label.setText(L("Salto (mm)", "Skip (mm)"))
        n_sub = len(self.win.engine.layers[self.win.doc.active_layer].sublayers or []) if self.win.engine.layers else 0
        self.sub_btn.setText(L("Sotto-layer…", "Sub-layers…") + (f" ({n_sub})" if n_sub else ""))
        cmd = self.win.engine.machine.air_command
        self.air.setLabels({"off": t("airOff"), "on": self.win.L("Accesa", "On") + f" ({cmd})"})
        for mode, bt in getattr(self, "air_test_buttons", []):
            bt.setText(self.win.L("Prova ", "Test ") + (f"{cmd} ON" if mode == "on" else "M9 OFF"))
        self.adv_toggle.setText(t("advanced"))
        self.w_pscale.label.setText(t("powerScale"))
        self.w_tabs.label.setText(t("tabs"))
        self.w_tabw.label.setText(t("tabWidth"))
        self.w_ramp.label.setText(t("ramp"))
        self.w_delay.label.setText(t("startDelay"))
        self.w_zstep.label.setText(t("zStep"))
        self.dot.setText(t("dotMode"))
        self.table.setHorizontalHeaderLabels(["#", t("layer"), t("mode"), "Vel/Pot", t("air"), t("output")])
        self.refresh()

    def set_speed_units(self, units):
        from .prefs import SPEED_FACTORS
        self.speed_units = units
        self.speed_factor = SPEED_FACTORS.get(units, 1.0)
        self.speed.blockSignals(True)
        self.speed.setDecimals(0 if units == "mm/min" else 2)
        self.speed.setRange(0.01, 100000 * self.speed_factor)
        self.speed.setSingleStep(10 if units == "mm/min" else 1)
        self.speed.blockSignals(False)
        self.w_speed.label.setText(self.win.tr("speed").split("(")[0].strip() + f" ({units})")
        self.load(self.win.doc.active_layer)
        self.refresh()

    # ---- table
    def refresh(self):
        win = self.win
        layers = win.engine.layers
        used = win.doc.used_layers()
        show = sorted(set(used) | {win.doc.active_layer})
        flat = M.flatten(win.doc.shapes)
        mode_name = {"line": win.tr("modeLine"), "fill": win.tr("modeFill"), "offset_fill": win.tr("modeOffsetFill"), "line_fill": win.L("Lin+Riem", "Line+Fill")}
        self.table.blockSignals(True)
        self.table.setRowCount(len(show))
        self._rows = show
        for r, i in enumerate(show):
            l = layers[i]
            has_img = any(s.get("layer") == i and s["type"] == "image" for s in flat)
            sw = QTableWidgetItem("  ")
            sw.setBackground(QBrush(QColor(l.color)))
            items = [sw, QTableWidgetItem(l.name + ("" if i in used else " •")),
                     QTableWidgetItem(win.L("Immagine", "Image") if has_img else mode_name[l.cut_mode.value]),
                     QTableWidgetItem(f"{l.speed * self.speed_factor:.{0 if self.speed_units == 'mm/min' else 1}f}/{round(l.power_max)}%"),
                     QTableWidgetItem("-" if l.air_assist.value == "off" else self.win.engine.machine.air_command),
                     QTableWidgetItem("T" if l.tool else "◉" if l.enabled else "○")]
            if l.tool:
                items[2].setText(win.L("Tecnico", "Tool"))
                items[3].setText("—")
                items[4].setText("—")
                items[5].setToolTip(win.L("Livello tecnico: mai inviato al laser", "Tool layer: never sent to the laser"))
            if l.air_assist.value != "off":
                items[4].setForeground(QBrush(QColor(AIR)))
            items[5].setTextAlignment(Qt.AlignCenter)
            items[5].setForeground(QBrush(QColor("#4caf50" if l.enabled else "#777")))
            for c, it in enumerate(items):
                self.table.setItem(r, c, it)
            if i == win.doc.active_layer:
                self.table.selectRow(r)
        self.table.blockSignals(False)
        # as tall as its rows (2 to 6 visible): no big empty box above the layer settings
        n = max(2, min(6, len(show)))
        self.table.setFixedHeight(self.table.horizontalHeader().height() + n * self.table.verticalHeader().defaultSectionSize() + 4)

    def _cell_clicked(self, r, c):
        idx = self._rows[r]
        if c == 5 and self.win.engine.layers[idx].tool:
            self.win.toast(self.win.L("T1/T2 sono livelli tecnici: non vanno mai al laser", "T1/T2 are tool layers: never sent to the laser"))
            return
        if c == 5:
            l = self.win.engine.layers[idx]
            self.win.engine.update_layer(idx, {"enabled": not l.enabled})
            self.win.invalidate_gcode()
            self.refresh()
            return
        self.win.select_layer(idx)

    # ---- settings form
    def load(self, idx):
        l = self.win.engine.layers[idx]
        self._loading = True
        self.title.setText(l.name)
        self.sw.setStyleSheet(f"background:{l.color}; border:1px solid #111; border-radius:2px;")
        self.speed.setValue(l.speed * self.speed_factor)
        self.passes.setValue(l.passes)
        self.pmax.setValue(l.power_max)
        self.pmax_sl.setValue(int(round(l.power_max)))
        self.pmin.setValue(l.power_min)
        self.pmin_sl.setValue(int(round(l.power_min)))
        self.mode.setValue(l.cut_mode.value)
        self.laser.setValue(l.laser_mode.value)
        self.interval.setValue(l.line_interval)
        self.overscan.setValue(l.overscan)
        self.angle.setValue(float(l.scan_angle))
        self.bidir.setChecked(l.bidir_scan)
        self.cross.setChecked(l.cross_hatch)
        self.air.setValue(l.air_assist.value)
        self.air_power.setValue(l.air_power)
        self.air_cut.setChecked(l.air_cut_only)
        self.z_offset.setValue(l.z_offset)
        self.power_scale.setValue(l.power_scale)
        self.kerf.setValue(l.kerf_offset)
        self.dwell.setValue(l.dwell_time)
        self.tabs.setValue(l.tab_count)
        self.tab_w.setValue(l.tab_width)
        self.ramp.setValue(l.ramp_length)
        self.delay.setValue(l.start_delay)
        self.z_step.setValue(l.z_step)
        self.dot.setChecked(l.dot_mode)
        for w_ in (self.speed, self.passes, self.pmax, self.pmin, self.pmax_sl, self.pmin_sl, self.mode, self.laser,
                   self.fill_box, self.img_box, self.air_box, self.adv_toggle, self.adv):
            w_.setEnabled(not l.tool)
        self.title.setText(l.name + (self.win.L("  (livello tecnico, non va al laser)", "  (tool layer, not sent to the laser)") if l.tool else ""))
        self.pass_through.setChecked(l.pass_through)
        self.flood.setChecked(l.flood_fill)
        self.lead_in.setValue(l.lead_in)
        self.lead_out.setValue(l.lead_out)
        self.overcut.setValue(l.overcut)
        self.perf.setChecked(l.perforate)
        self.perf_cut.setValue(l.perf_cut)
        self.perf_skip.setValue(l.perf_skip)
        self.sub_btn.setText(self.win.L("Sotto-layer…", "Sub-layers…") + (f" ({len(l.sublayers)})" if l.sublayers else ""))
        self.bottom_up.setChecked(l.bottom_up)
        self.dot_width.setValue(l.dot_width)
        self._loading = False
        self.update_fill_visibility()

    def update_fill_visibility(self):
        idx = self.win.doc.active_layer
        img = any(s.get("layer") == idx and s["type"] == "image" for s in M.flatten(self.win.doc.shapes))
        self.fill_box.setVisible(self.mode.value() != "line" or img)
        self.img_box.setVisible(img)

    def save(self, *_):
        if self._loading:
            return
        data = {
            "speed": self.speed.value() / self.speed_factor, "passes": self.passes.value(), "power_max": self.pmax.value(), "power_min": self.pmin.value(),
            "cut_mode": self.mode.value(), "laser_mode": self.laser.value(), "line_interval": self.interval.value(),
            "overscan": self.overscan.value(), "scan_angle": self.angle.value(), "bidir_scan": self.bidir.isChecked(),
            "cross_hatch": self.cross.isChecked(), "air_assist": self.air.value(), "air_power": self.air_power.value(),
            "air_cut_only": self.air_cut.isChecked(),
            "z_offset": self.z_offset.value(), "power_scale": self.power_scale.value(), "kerf_offset": self.kerf.value(),
            "dwell_time": self.dwell.value(), "tab_count": self.tabs.value(), "tab_width": self.tab_w.value(),
            "ramp_length": self.ramp.value(), "start_delay": self.delay.value(), "z_step": self.z_step.value(),
            "dot_mode": self.dot.isChecked(), "pass_through": self.pass_through.isChecked(),
            "bottom_up": self.bottom_up.isChecked(), "dot_width": self.dot_width.value(),
            "flood_fill": self.flood.isChecked(), "lead_in": self.lead_in.value(), "lead_out": self.lead_out.value(),
            "overcut": self.overcut.value(), "perforate": self.perf.isChecked(), "perf_cut": self.perf_cut.value(),
            "perf_skip": self.perf_skip.value(),
        }
        self.win.save_layer_settings({k: v for k, v in data.items() if v is not None})
        self.update_fill_visibility()


# ====================================================================== LASER
ORIGIN_POS = [(0, 0), (0.5, 0), (1, 0), (0, 0.5), (0.5, 0.5), (1, 0.5), (0, 1), (0.5, 1), (1, 1)]


class LaserPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(6)
        dev_row = QHBoxLayout()
        self.dev_label = QLabel()
        self.device = QComboBox()
        self.device.activated.connect(self._device_picked)
        self.dev_btn = QToolButton()
        self.dev_btn.setText("…")
        self.dev_btn.clicked.connect(lambda: win.run("devices"))
        dev_row.addWidget(self.dev_label)
        dev_row.addWidget(self.device, 1)
        dev_row.addWidget(self.dev_btn)
        lay.addLayout(dev_row)
        conn = QHBoxLayout()
        self.port = QComboBox()
        self.port.addItem("Simulatore", "SIM")
        self.refresh_btn = QToolButton()
        self.refresh_btn.setText("↻")
        self.refresh_btn.clicked.connect(self.refresh_ports)
        self.port.activated.connect(self._port_picked)
        self.connect_btn = QPushButton()
        self.connect_btn.setMinimumWidth(110)
        self.connect_btn.setProperty("role", "primary")
        self.connect_btn.clicked.connect(win.toggle_connection)
        conn.addWidget(self.port, 1)
        conn.addWidget(self.refresh_btn)
        conn.addWidget(self.connect_btn)
        lay.addLayout(conn)

        grid = QGridLayout()
        grid.setSpacing(4)
        self.btns = {}
        spec = [("startLaser", "start", "primary"), ("pauseLaser", "pause", "warn"), ("stopLaser", "stop", "danger"),
                ("frame", "frame", None), ("resumeLaser", "resume", None), ("homeLaser", "home", None),
                ("gotoOrigin", "gotoOrigin", None), ("saveGcode", "saveGcode", None), ("preview", "preview", None)]
        for n, (act, key, role) in enumerate(spec):
            bt = QPushButton()
            if role:
                bt.setProperty("role", role)
            bt.clicked.connect(lambda _=False, a=act: win.run(a))
            grid.addWidget(bt, n // 3, n % 3)
            self.btns[key] = bt
        lay.addLayout(grid)
        self.alarm_box = QWidget()
        ab = QHBoxLayout(self.alarm_box)
        ab.setContentsMargins(0, 0, 0, 0)
        self.alarm_label = QLabel()
        self.alarm_label.setWordWrap(True)
        self.alarm_label.setStyleSheet("color:#ff9800; font-weight:600;")
        self.unlock_btn = QPushButton()
        self.unlock_btn.setProperty("role", "warn")
        self.unlock_btn.clicked.connect(lambda: win.run("unlock"))
        self.alarm_home = QPushButton()
        self.alarm_home.clicked.connect(lambda: win.run("homeLaser"))
        ab.addWidget(self.alarm_label, 1)
        ab.addWidget(self.unlock_btn)
        ab.addWidget(self.alarm_home)
        self.alarm_box.setVisible(False)
        lay.addWidget(self.alarm_box)

        # "Start from" on its own full-width row; job origin grid and the two buttons below it (no clipping)
        self.sf_label = QLabel()
        self.sf_label.setStyleSheet("color:#a8aab1; font-size:11px;")
        self.start_from = Segmented([("absolute", ""), ("current", ""), ("user", "")])
        self.start_from.changed.connect(win.set_start_from)
        lay.addWidget(self.sf_label)
        lay.addWidget(self.start_from)
        sf = QHBoxLayout()
        right = QVBoxLayout()
        self.origin_label = QLabel()
        self.origin_label.setStyleSheet("color:#a8aab1; font-size:11px;")
        right.addWidget(self.origin_label)
        og = QGridLayout()
        og.setSpacing(2)
        self.origin_btns = []
        for i in range(9):
            b = QToolButton()
            b.setCheckable(True)
            b.setFixedSize(18, 18)
            b.setStyleSheet("QToolButton { background: transparent; border:1px solid #8a8c93; border-radius:9px; }"
                            "QToolButton:checked { background:#2f7bf5; border-color:#2f7bf5; }"
                            "QToolButton:disabled { border-color: rgba(138,140,147,90); }")
            b.clicked.connect(lambda _=False, k=i: win.set_job_origin(k))
            og.addWidget(b, i // 3, i % 3)
            self.origin_btns.append(b)
        right.addLayout(og)
        right.addStretch(1)
        sf.addLayout(right)
        sf.addSpacing(10)
        bl = QVBoxLayout()
        self.set_origin_btn = QPushButton()
        self.set_origin_btn.clicked.connect(lambda: win.run("setUserOrigin"))
        self.opt_btn = QPushButton()
        self.opt_btn.clicked.connect(lambda: win.run("optimizeSettings"))
        bl.addWidget(self.set_origin_btn)
        bl.addWidget(self.opt_btn)
        bl.addStretch(1)
        sf.addLayout(bl, 1)
        lay.addLayout(sf)

        # job progress and live overrides (GRBL real-time feed / laser power)
        from PySide6.QtWidgets import QProgressBar
        self.progress = QProgressBar()
        self.progress.setTextVisible(True)
        self.progress.setVisible(False)
        lay.addWidget(self.progress)
        ovr = QGridLayout()
        ovr.setSpacing(3)
        self.ovr_labels = {}
        self.ovr_titles = {}
        for row_i, kind in enumerate(("feed", "power")):
            title = QLabel()
            self.ovr_titles[kind] = title
            ovr.addWidget(title, row_i, 0)
            for col, (txt, delta) in enumerate((("−10", -10), ("−1", -1), ("100%", 0), ("+1", 1), ("+10", 10))):
                b = QPushButton(txt)
                b.setFixedHeight(24)
                b.clicked.connect(lambda _=False, k=kind, dl=delta: self._override(k, dl))
                ovr.addWidget(b, row_i, col + 1)
            val = QLabel("100%")
            val.setMinimumWidth(42)
            val.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
            self.ovr_labels[kind] = val
            ovr.addWidget(val, row_i, 6)
        lay.addLayout(ovr)

        self.estimate = QLabel("")
        self.estimate.setStyleSheet("background:#1f2a3a; border-radius:4px; padding:4px 6px; color:#9ec5ff;")
        self.estimate.setVisible(False)
        lay.addWidget(self.estimate)
        self.gcode = QPlainTextEdit()
        self.gcode.setReadOnly(True)
        self.gcode.setPlaceholderText("G-code")
        self.gcode.setFont(QFont("Menlo", 10))
        self.gcode.setMaximumBlockCount(20000)
        lay.addWidget(self.gcode, 1)
        self.retranslate()

    def retranslate(self):
        t = self.win.tr
        for key, bt in self.btns.items():
            bt.setText(t(key))
        self.sf_label.setText(t("startFrom"))
        self.alarm_label.setText(self.win.L("GRBL in allarme (motivo nella Console)", "GRBL in alarm (reason in the Console)"))
        self.unlock_btn.setText(self.win.L("Sblocca ($X)", "Unlock ($X)"))
        self.alarm_home.setText("Home ($H)")
        self.dev_label.setText(self.win.L("Dispositivo", "Device"))
        self.refresh_devices()
        self.start_from.setLabels({"absolute": t("sfAbsolute"), "current": t("sfCurrent"), "user": t("sfUser")})
        self.set_origin_btn.setText(t("setOrigin"))
        self.opt_btn.setText(t("optimization"))
        self.origin_label.setText(t("jobOrigin"))
        self.ovr_titles["feed"].setText(self.win.L("Velocità", "Speed"))
        self.ovr_titles["power"].setText(self.win.L("Potenza", "Power"))
        self.port.setItemText(0, self.win.L("Simulatore", "Simulator"))
        self.set_connected(self.win.connected)

    def _override(self, kind, delta):
        if not self.win.connected:
            self.win.toast(self.win.L("Connetti prima il laser", "Connect a laser first"))
            return
        v = self.win.engine.override(kind, delta)
        self.ovr_labels[kind].setText(f"{v}%")

    def show_progress(self, sent, total, busy):
        if total and (busy or 0 < sent < total):
            self.progress.setVisible(True)
            self.progress.setRange(0, total)
            self.progress.setValue(min(sent, total))
            self.progress.setFormat(f"{sent}/{total}  (%p%)")
        elif self.progress.isVisible() and (not total or sent >= total):
            self.progress.setVisible(False)

    def show_alarm(self, on):
        if self.alarm_box.isVisible() != on:
            self.alarm_box.setVisible(on)

    def refresh_devices(self):
        from .devices import load_devices
        cur = self.win.settings.value("device", "")
        self.device.blockSignals(True)
        self.device.clear()
        self.device.addItem(self.win.L("(impostazioni attuali)", "(current settings)"), None)
        for dv in load_devices(self.win):
            self.device.addItem(dv["name"], dv["name"])
        i = self.device.findData(cur) if cur else 0
        self.device.setCurrentIndex(max(0, i))
        self.device.blockSignals(False)

    def _device_picked(self, _i):
        from .devices import apply_device, load_devices
        name = self.device.currentData()
        dv = next((x for x in load_devices(self.win) if x["name"] == name), None)
        if dv:
            apply_device(self.win, dv)

    def refresh_ports(self):
        cur = self.port.currentData()
        self.port.clear()
        self.port.addItem(self.win.L("Simulatore", "Simulator"), "SIM")
        for p in self.win.engine.list_ports():
            name = p.get("device", p) if isinstance(p, dict) else p
            desc = p.get("description", "") if isinstance(p, dict) else ""
            self.port.addItem(f"{name}  {desc}".strip(), name)
        for addr in self.win._load_json("netPorts", []):
            self.port.addItem(self.win.L("Rete ", "Network ") + addr[6:], addr)
        self.port.addItem(self.win.L("Rete (TCP)…", "Network (TCP)…"), "__tcp__")
        self.port.addItem(self.win.L("Rileva controller…", "Detect controller…"), "__detect__")
        self.port.addItem(self.win.L("Cerca laser in rete…", "Find network lasers…"), "__mdns__")
        i = self.port.findData(cur)
        if i >= 0:
            self.port.setCurrentIndex(i)

    def select_port(self, address):
        if address and address.startswith("tcp://"):
            nets = self.win._load_json("netPorts", [])
            if address not in nets:
                self.win._store_json("netPorts", nets + [address])
        self.refresh_ports()
        i = self.port.findData(address)
        if i >= 0:
            self.port.setCurrentIndex(i)

    def _port_picked(self, _i):
        from PySide6.QtWidgets import QInputDialog
        data = self.port.currentData()
        if data == "__tcp__":
            text, ok = QInputDialog.getText(self, self.win.L("Laser in rete", "Network laser"),
                                            self.win.L("Indirizzo e porta (es. 192.168.1.50:23):", "Address and port (e.g. 192.168.1.50:23):"), text="192.168.1.50:23")
            if ok and text.strip():
                self.select_port("tcp://" + text.strip().replace("tcp://", ""))
            else:
                self.port.setCurrentIndex(0)
        elif data == "__mdns__":
            from .serial_comm import discover_network
            from PySide6.QtWidgets import QApplication
            QApplication.setOverrideCursor(Qt.WaitCursor)
            try:
                res = discover_network()
            finally:
                QApplication.restoreOverrideCursor()
            if not res:
                self.port.setCurrentIndex(0)
                self.win.toast(self.win.L("Nessun laser trovato in rete (Bonjour/mDNS)", "No network laser found (Bonjour/mDNS)"))
                return
            labels = [f"{n} — {h}:{p}" for n, h, p in res]
            pick, ok = QInputDialog.getItem(self, self.win.L("Laser in rete", "Network lasers"), self.win.L("Trovati:", "Found:"), labels, 0, False)
            if ok:
                n, h, p = res[labels.index(pick)]
                self.select_port(f"tcp://{h}:{p}")
                self.win.toast(self.win.L(f"Selezionato {n}", f"Selected {n}"))
            else:
                self.port.setCurrentIndex(0)
        elif data == "__detect__":
            from .serial_comm import detect_controllers
            from PySide6.QtWidgets import QApplication
            QApplication.setOverrideCursor(Qt.WaitCursor)
            try:
                res = detect_controllers(self.win.engine.machine.baud_rate)
            finally:
                QApplication.restoreOverrideCursor()
            if res:
                dev, fl, banner = res[0]
                self.win.engine.update_machine({"flavor": fl})
                self.select_port(dev)
                self.win.toast(self.win.L(f"Trovato {banner} su {dev}", f"Found {banner} on {dev}"))
            else:
                self.port.setCurrentIndex(0)
                self.win.toast(self.win.L("Nessun controller trovato", "No controller found"))

    def set_connected(self, on):
        self.connect_btn.setText(self.win.tr("disconnect") if on else self.win.tr("connect"))
        self.connect_btn.setProperty("role", "danger" if on else "primary")
        self.connect_btn.style().unpolish(self.connect_btn)
        self.connect_btn.style().polish(self.connect_btn)

    def refresh_origin(self):
        absolute = self.win.start_from == "absolute"
        for i, b in enumerate(self.origin_btns):
            b.setChecked(i == self.win.job_origin)
            b.setEnabled(not absolute)
        self.start_from.setValue(self.win.start_from)

    def show_gcode(self, gcode, estimate=None, lines=None):
        self.gcode.setPlainText(gcode if len(gcode) < 400000 else gcode[:400000] + "\n...")
        if estimate:
            self.estimate.setText(f"⏱ {estimate['estimated']}   ·   {estimate.get('cut_distance_mm', 0)} mm   ·   {lines} {self.win.L('righe', 'lines')}")
            self.estimate.setVisible(True)


# ====================================================================== MOVE
class MovePanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        pad = QGridLayout()
        pad.setSpacing(4)
        for (dx, dy, txt, r, c) in ((0, -1, "▲", 0, 1), (-1, 0, "◀", 1, 0), (1, 0, "▶", 1, 2), (0, 1, "▼", 2, 1),
                                     (-1, -1, "◤", 0, 0), (1, -1, "◥", 0, 2), (-1, 1, "◣", 2, 0), (1, 1, "◢", 2, 2)):
            b = QPushButton(txt)
            b.setFixedSize(44, 36)
            b.clicked.connect(lambda _=False, x=dx, y=dy: win.jog(x, y))
            pad.addWidget(b, r, c)
        home = QPushButton("⌂")
        home.setFixedSize(44, 36)
        home.clicked.connect(lambda: win.run("homeLaser"))
        pad.addWidget(home, 1, 1)
        top = QHBoxLayout()
        top.addLayout(pad)
        side = QVBoxLayout()
        self.step_label = QLabel()
        self.step_label.setStyleSheet("color:#a8aab1; font-size:11px;")
        self.step = Segmented([(0.1, "0.1"), (1.0, "1"), (10.0, "10"), (50.0, "50")])
        self.step.setValue(1.0)
        self.speed = dspin(1000, 10, 20000, 100, 0, " mm/min")
        side.addWidget(self.step_label)
        side.addWidget(self.step)
        side.addWidget(self.speed)
        self.zbox = QWidget()
        zrow = QHBoxLayout(self.zbox)
        zrow.setContentsMargins(0, 0, 0, 0)
        for txt, dz in (("Z+", 1), ("Z−", -1)):
            b = QPushButton(txt)
            b.clicked.connect(lambda _=False, z=dz: win.jog(0, 0, z))
            zrow.addWidget(b)
        side.addWidget(self.zbox)
        # LightBurn "Fire": the laser stays on at low power only while the button is held, to see where it points
        self.fire = QPushButton("Fire")
        self.fire.setStyleSheet("QPushButton { background:#8b1d1d; color:white; font-weight:600; } QPushButton:pressed { background:#d32f2f; }")
        self.fire.pressed.connect(lambda: win.fire_laser(True))
        self.fire.released.connect(lambda: win.fire_laser(False))
        side.addWidget(self.fire)
        side.addStretch(1)
        top.addLayout(side, 1)
        lay.addLayout(top)
        # LightBurn: Get Position, set work zero here, clear it
        prow = QHBoxLayout()
        self.getpos_btn, self.zero_btn, self.clrzero_btn = QPushButton(), QPushButton(), QPushButton()
        self.getpos_btn.clicked.connect(win.show_position)
        self.zero_btn.clicked.connect(win.zero_here)
        self.clrzero_btn.clicked.connect(win.clear_zero)
        for b in (self.getpos_btn, self.zero_btn, self.clrzero_btn):
            prow.addWidget(b)
        lay.addLayout(prow)

        self.ws_x = dspin(400, 10, 3000, 10, 1)
        self.ws_y = dspin(400, 10, 3000, 10, 1)
        self.w_x, self.w_y = labeled("", self.ws_x), labeled("", self.ws_y)
        lay.addWidget(row(self.w_x, self.w_y))
        self.home_start = checkbox("")
        self.ret_origin = checkbox("")
        lay.addWidget(self.home_start)
        lay.addWidget(self.ret_origin)
        # saved positions (LightBurn: Stored Coordinates)
        self.pos_title = QLabel()
        self.pos_title.setStyleSheet("font-weight:600; margin-top:6px;")
        lay.addWidget(self.pos_title)
        self.pos_list = QListWidget()
        self.pos_list.setMaximumHeight(120)
        self.pos_list.itemDoubleClicked.connect(lambda _it: self.goto_pos())
        lay.addWidget(self.pos_list)
        ph = QHBoxLayout()
        self.pos_save, self.pos_go, self.pos_ren, self.pos_del = QPushButton(), QPushButton(), QPushButton(), QPushButton()
        self.pos_save.clicked.connect(self.save_pos)
        self.pos_go.clicked.connect(self.goto_pos)
        self.pos_ren.clicked.connect(self.rename_pos)
        self.pos_del.clicked.connect(self.delete_pos)
        for b in (self.pos_save, self.pos_go, self.pos_ren, self.pos_del):
            ph.addWidget(b)
        lay.addLayout(ph)
        self.positions = self.win._load_json("positions", [])
        self._fill_pos()
        lay.addStretch(1)
        for w in (self.ws_x, self.ws_y):
            w.valueChanged.connect(self._save)
        for w in (self.home_start, self.ret_origin):
            w.toggled.connect(self._save)
        self._loading = False
        self.retranslate()

    def _fill_pos(self):
        self.pos_list.clear()
        for p in self.positions:
            self.pos_list.addItem(f"{p['name']}   (X {p['x']:.2f}  Y {p['y']:.2f})")

    def _store_pos(self):
        self.win._store_json("positions", self.positions)
        self._fill_pos()

    def save_pos(self):
        from PySide6.QtWidgets import QInputDialog
        x, y = self.win.machine_pos
        name, ok = QInputDialog.getText(self, self.win.L("Salva posizione", "Save position"), self.win.L("Nome:", "Name:"),
                                        text=self.win.L(f"Posizione {len(self.positions) + 1}", f"Position {len(self.positions) + 1}"))
        if ok:
            self.positions.append({"name": name.strip() or "?", "x": x, "y": y})
            self._store_pos()

    def goto_pos(self):
        i = self.pos_list.currentRow()
        if i < 0:
            return
        if not self.win.connected:
            self.win.toast(self.win.L("Connetti prima il laser", "Connect a laser first"))
            return
        p = self.positions[i]
        self.win.goto_canvas(p["x"], p["y"])

    def rename_pos(self):
        from PySide6.QtWidgets import QInputDialog
        i = self.pos_list.currentRow()
        if i < 0:
            return
        name, ok = QInputDialog.getText(self, self.win.L("Rinomina", "Rename"), self.win.L("Nome:", "Name:"), text=self.positions[i]["name"])
        if ok and name.strip():
            self.positions[i]["name"] = name.strip()
            self._store_pos()

    def delete_pos(self):
        i = self.pos_list.currentRow()
        if i >= 0:
            self.positions.pop(i)
            self._store_pos()

    def retranslate(self):
        t = self.win.tr
        L = self.win.L
        self.pos_title.setText(L("Posizioni salvate", "Saved positions"))
        self.getpos_btn.setText(L("Leggi posizione", "Get Position"))
        self.zero_btn.setText(L("Azzera X/Y qui", "Zero X/Y here"))
        self.zero_btn.setToolTip(L("Rende la posizione attuale della testa lo zero di lavoro (G10 L20). Con «Assolute» il lavoro parte da qui.",
                                   "Makes the head's current position the work zero (G10 L20). With “Absolute” the job is placed from here."))
        self.clrzero_btn.setText(L("Cancella zero", "Clear zero"))
        self.clrzero_btn.setToolTip(L("Riporta lo zero di lavoro sullo zero macchina (G10 L2)", "Puts the work zero back on machine zero (G10 L2)"))
        self.pos_save.setText(L("Salva attuale", "Save current"))
        self.pos_go.setText(L("Vai", "Go"))
        self.pos_ren.setText(L("Rinomina", "Rename"))
        self.pos_del.setText(L("Elimina", "Delete"))
        self.step_label.setText(t("step") + " (mm)")
        self.w_x.label.setText(t("areaW"))
        self.w_y.label.setText(t("areaH"))
        self.home_start.setText(t("homeOnStart"))
        self.ret_origin.setText(t("returnOrigin"))

    def load(self):
        m = self.win.engine.machine
        self._loading = True
        self.ws_x.setValue(m.workspace_x)
        self.ws_y.setValue(m.workspace_y)
        self.home_start.setChecked(m.home_on_start)
        self.ret_origin.setChecked(m.return_to_origin)
        self.fire.setVisible(bool(m.fire_enabled))
        self.zbox.setVisible(bool(m.z_enabled))
        self.fire.setToolTip(self.win.L(f"Tieni premuto: laser acceso al {m.fire_power:g}% finché non rilasci",
                                        f"Hold: laser on at {m.fire_power:g}% until released"))
        self._loading = False

    def _save(self, *_):
        if self._loading:
            return
        self.win.engine.update_machine({"workspace_x": self.ws_x.value(), "workspace_y": self.ws_y.value(),
                                        "home_on_start": self.home_start.isChecked(), "return_to_origin": self.ret_origin.isChecked()})
        self.win.invalidate_gcode()
        self.win.canvas.update()


# ====================================================================== CONSOLE
LEVEL_COLORS = {"ok": "#7bd88f", "error": "#ff6b6b", "status": "#9ec5ff", "plain": "#c9cbd1", "info": "#c9cbd1"}


class ConsolePanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.out = QTextEdit()
        self.out.setReadOnly(True)
        self.out.setFont(QFont("Menlo", 10))
        lay.addWidget(self.out, 1)
        h = QHBoxLayout()
        self.inp = QLineEdit()
        self.inp.setPlaceholderText("GRBL… ($$, $I, G0 X10 …)")
        self.inp.returnPressed.connect(self.send)
        self.send_btn = QPushButton()
        self.send_btn.clicked.connect(self.send)
        self.clear_btn = QPushButton()
        self.clear_btn.clicked.connect(self.clear)
        h.addWidget(self.inp, 1)
        h.addWidget(self.send_btn)
        h.addWidget(self.clear_btn)
        lay.addLayout(h)
        self._count = -1
        self.retranslate()

    def retranslate(self):
        self.send_btn.setText(self.win.tr("send"))
        self.clear_btn.setText(self.win.tr("clear"))

    def send(self):
        cmd = self.inp.text().strip()
        if cmd:
            self.win.engine.command(cmd)
            self.inp.clear()
            self.refresh()

    def clear(self):
        self.win.engine.clear_console()
        self.out.clear()
        self._count = 0

    def refresh(self):
        lines = self.win.engine.console_lines()
        sig = (len(lines), lines[-1]["time"] + lines[-1]["msg"] if lines else "")
        if sig == self._count:
            return
        self._count = sig
        body = "".join(f'<div><span style="color:#6b6d74">{l["time"]}</span>&nbsp; '
                       f'<span style="color:{LEVEL_COLORS.get(l["level"], "#c9cbd1")}">{html.escape(l["msg"])}</span></div>'
                       for l in lines[-400:])
        self.out.setHtml(body)
        self.out.moveCursor(QTextCursor.End)


# ====================================================================== OBJECTS / LIBRARY
class ObjectsPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.list = QListWidget()
        self.list.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.list.itemSelectionChanged.connect(self._picked)
        lay.addWidget(self.list)
        self._sync = False

    def refresh(self):
        self._sync = True
        self.list.clear()
        sel = set(self.win.doc.selected)
        for s in self.win.doc.shapes:
            name = f'"{s["text"]}"' if s["type"] == "text" else s.get("name", "image") if s["type"] == "image" else s["type"]
            it = QListWidgetItem(f"■  {name} #{s['id']}" + ("  🔒" if s.get("locked") else ""))
            it.setForeground(QBrush(QColor(self.win.layer_color(s.get("layer", 0)))))
            it.setData(Qt.UserRole, s["id"])
            self.list.addItem(it)
            it.setSelected(s["id"] in sel)
        self._sync = False

    def _picked(self):
        if self._sync:
            return
        ids = [it.data(Qt.UserRole) for it in self.list.selectedItems()]
        self.win.doc.set_selection(ids)


class LibraryPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.list = QListWidget()
        for name, p in MATERIAL_PRESETS.items():
            it = QListWidgetItem(f"{name}    —    {p['speed']} mm/min · {p['power_max']}% · {p['air_assist'].upper()}")
            it.setData(Qt.UserRole, name)
            self.list.addItem(it)
        self.list.itemDoubleClicked.connect(lambda it: win.apply_preset(it.data(Qt.UserRole)))
        lay.addWidget(self.list, 1)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self.retranslate()

    def retranslate(self):
        self.hint.setText(self.win.tr("libraryHint"))
