"""Shape Properties panel, Pages panel, Undo History dialog and live (editable) arrays."""

import math

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QAbstractItemView, QDialog, QDialogButtonBox, QFontComboBox, QFormLayout, QHBoxLayout, QInputDialog, QLabel,
    QLineEdit, QListWidget, QListWidgetItem, QPushButton, QStackedWidget, QVBoxLayout, QWidget,
)

from . import model as M
from .ui_common import Segmented, checkbox, dspin, hint, ispin


# ====================================================================== live arrays
def array_copies(win, source, meta):
    """Copies (excluding the originals) for an array description."""
    out = []
    if meta["kind"] == "grid":
        b = M.multi_bounds(source)
        for r in range(meta["rows"]):
            for c in range(meta["cols"]):
                if r == 0 and c == 0:
                    continue
                for s in M.clone(source):
                    win.doc.reid(s)
                    M.move_shape(s, c * (b[2] + meta["dx"]), r * (b[3] + meta["dy"]))
                    out.append(s)
    else:
        n, total = meta["n"], meta["angle"]
        cx, cy = meta["cx"], meta["cy"]
        step = total / n if abs(total) >= 360 else total / max(1, n - 1)
        for i in range(1, n):
            deg = step * i
            a = math.radians(deg)
            for s in M.clone(source):
                win.doc.reid(s)
                if meta.get("rotate", True) and M.rotate_shape(s, cx, cy, deg):
                    win._fix_child_ids(s)
                else:
                    c0 = M.center(M.bounds(s))
                    dx, dy = c0[0] - cx, c0[1] - cy
                    M.move_shape(s, cx + dx * math.cos(a) - dy * math.sin(a) - c0[0], cy + dx * math.sin(a) + dy * math.cos(a) - c0[1])
                out.append(s)
    return out


def make_array(win, source, meta):
    """A group whose first children are the originals and the rest are regenerated copies."""
    meta = dict(meta, count=len(source), src_c=list(M.center(M.multi_bounds(source))))
    g = win.doc.create("group", children=M.clone(source) + array_copies(win, source, meta), layer=source[0].get("layer", 0))
    g["array"] = meta
    return g


def regenerate_array(win, g):
    meta = g["array"]
    source = g["children"][:meta["count"]]
    cur = M.center(M.multi_bounds(source))
    if meta["kind"] == "circular" and "src_c" in meta:
        # the array was moved: move the rotation centre with it
        meta["cx"] += cur[0] - meta["src_c"][0]
        meta["cy"] += cur[1] - meta["src_c"][1]
    meta["src_c"] = list(cur)
    g["children"] = source + array_copies(win, source, meta)


def flatten_array(g):
    g.pop("array", None)


# ====================================================================== undo history
def undo_history_dialog(win):
    doc = win.doc
    d = QDialog(win)
    d.setWindowTitle(win.L("Cronologia annulla", "Undo History"))
    d.setMinimumSize(380, 420)
    lay = QVBoxLayout(d)
    lst = QListWidget()
    lay.addWidget(lst, 1)
    lay.addWidget(hint(win.L("Doppio clic su un passaggio per tornare a quel punto. Le voci in grigio si possono ripetere.",
                             "Double-click a step to return to that point. Grey entries can be redone.")))

    def fill():
        lst.clear()
        it = QListWidgetItem(win.L("— Inizio —", "— Start —"))
        it.setData(Qt.UserRole, -len(doc.undo_labels))
        lst.addItem(it)
        for k, lab in enumerate(doc.undo_labels):
            it = QListWidgetItem(f"{k + 1}. {lab}")
            it.setData(Qt.UserRole, k + 1 - len(doc.undo_labels))
            lst.addItem(it)
        cur = lst.count() - 1
        for k, lab in enumerate(reversed(doc.redo_labels)):
            it = QListWidgetItem(f"{len(doc.undo_labels) + k + 1}. {lab}")
            it.setForeground(Qt.gray)
            it.setData(Qt.UserRole, k + 1)
            lst.addItem(it)
        lst.setCurrentRow(cur)

    def jump(it):
        delta = it.data(Qt.UserRole)
        if delta < 0:
            doc.undo_to(-delta)
        elif delta > 0:
            doc.redo_to(delta)
        fill()
    lst.itemDoubleClicked.connect(jump)
    bb = QDialogButtonBox(QDialogButtonBox.Close)
    bb.rejected.connect(d.reject)
    lay.addWidget(bb)
    fill()
    d.exec()


# ====================================================================== pages
class PagesPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.list = QListWidget()
        self.list.currentRowChanged.connect(self._picked)
        self.list.itemDoubleClicked.connect(lambda _it: self.rename())
        lay.addWidget(self.list, 1)
        h = QHBoxLayout()
        self.add_btn, self.dup_btn, self.ren_btn, self.del_btn = QPushButton(), QPushButton(), QPushButton(), QPushButton()
        self.add_btn.clicked.connect(lambda: self.win.doc.add_page())
        self.dup_btn.clicked.connect(self.duplicate)
        self.ren_btn.clicked.connect(self.rename)
        self.del_btn.clicked.connect(self.delete)
        for b in (self.add_btn, self.dup_btn, self.ren_btn, self.del_btn):
            h.addWidget(b)
        lay.addLayout(h)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self._sync = False
        win.doc.pagesChanged.connect(self.refresh)
        self.retranslate()
        self.refresh()

    def retranslate(self):
        L = self.win.L
        self.add_btn.setText(L("Nuova", "New"))
        self.dup_btn.setText(L("Duplica", "Duplicate"))
        self.ren_btn.setText(L("Rinomina", "Rename"))
        self.del_btn.setText(L("Elimina", "Delete"))
        self.hint.setText(L("Ogni pagina è un lavoro separato nello stesso progetto: viene inviata al laser solo la pagina aperta.",
                            "Each page is a separate job in the same project: only the open page is sent to the laser."))

    def refresh(self):
        self._sync = True
        self.list.clear()
        for i, pg in enumerate(self.win.doc.pages):
            self.list.addItem(f"{pg['name']}   ({len(pg['shapes'])})")
        self.list.setCurrentRow(self.win.doc.page)
        self._sync = False

    def _picked(self, row):
        if not self._sync and row >= 0:
            self.win.doc.set_page(row)

    def duplicate(self):
        doc = self.win.doc
        shapes = [doc.reid(s) for s in M.clone(doc.shapes)]
        doc.add_page(doc.pages[doc.page]["name"] + self.win.L(" (copia)", " (copy)"), shapes)

    def rename(self):
        doc = self.win.doc
        name, ok = QInputDialog.getText(self, self.win.L("Rinomina pagina", "Rename page"), self.win.L("Nome:", "Name:"),
                                        text=doc.pages[doc.page]["name"])
        if ok and name.strip():
            doc.pages[doc.page]["name"] = name.strip()
            doc.set_dirty(True)
            self.refresh()

    def delete(self):
        self.win.doc.remove_page(self.win.doc.page)


# ====================================================================== shape properties
class PropertiesPanel(QWidget):
    """Edits the parameters of the single selected shape (like LightBurn's Shape Properties)."""

    def __init__(self, win):
        super().__init__()
        self.win = win
        self._loading = False
        self.shape_id = None
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.title = QLabel()
        self.title.setStyleSheet("font-weight:600; font-size:13px;")
        lay.addWidget(self.title)
        self.stack = QStackedWidget()
        lay.addWidget(self.stack)
        # cut priority (used by Optimization › Order by priority)
        self.prio_row = QWidget()
        pf = QFormLayout(self.prio_row)
        pf.setContentsMargins(0, 8, 0, 0)
        self.prio = ispin(0, -999, 999)
        self.prio.valueChanged.connect(lambda v: self._set("priority", v))
        self.l_prio = QLabel()
        pf.addRow(self.l_prio, self.prio)
        lay.addWidget(self.prio_row)
        lay.addStretch(1)

        # empty
        self.p_empty = hint("")
        self.stack.addWidget(self.p_empty)

        # rectangle
        self.p_rect = QWidget()
        f = QFormLayout(self.p_rect)
        self.r_radius = dspin(0, 0, 10000, 0.5, 2, " mm")
        self.r_radius.valueChanged.connect(lambda v: self._set("radius", v))
        self.l_radius = QLabel()
        f.addRow(self.l_radius, self.r_radius)
        self.stack.addWidget(self.p_rect)

        # ellipse
        self.p_ell = QWidget()
        f = QFormLayout(self.p_ell)
        self.e_rx, self.e_ry = dspin(10, 0.01, 10000, 0.5, 3, " mm"), dspin(10, 0.01, 10000, 0.5, 3, " mm")
        self.e_rx.valueChanged.connect(lambda v: self._set("rx", v))
        self.e_ry.valueChanged.connect(lambda v: self._set("ry", v))
        self.l_rx, self.l_ry = QLabel(), QLabel()
        f.addRow(self.l_rx, self.e_rx)
        f.addRow(self.l_ry, self.e_ry)
        self.stack.addWidget(self.p_ell)

        # regular polygon / star
        self.p_poly = QWidget()
        f = QFormLayout(self.p_poly)
        self.g_n = ispin(6, 3, 200)
        self.g_r = dspin(10, 0.01, 10000, 0.5, 3, " mm")
        self.g_inner = dspin(45, 1, 99, 1, 0, " %")
        self.g_n.valueChanged.connect(lambda v: self._set_regular("n", v))
        self.g_r.valueChanged.connect(lambda v: self._set_regular("r", v))
        self.g_inner.valueChanged.connect(lambda v: self._set_regular("inner", v / 100))
        self.l_n, self.l_r, self.l_inner = QLabel(), QLabel(), QLabel()
        f.addRow(self.l_n, self.g_n)
        f.addRow(self.l_r, self.g_r)
        f.addRow(self.l_inner, self.g_inner)
        self.stack.addWidget(self.p_poly)

        # text
        self.p_text = QWidget()
        f = QFormLayout(self.p_text)
        self.t_text = QLineEdit()
        self.t_text.editingFinished.connect(lambda: self._set("text", self.t_text.text()))
        self.t_font = QFontComboBox()
        self.t_font.currentFontChanged.connect(lambda font: None if self.t_single.currentData() else self._set("font", font.family()))
        from PySide6.QtWidgets import QComboBox
        self.t_single = QComboBox()
        self.t_single.addItem("—", None)
        for fid, label in M.single_line_fonts():
            self.t_single.addItem(label, fid)
        self.t_single.currentIndexChanged.connect(self._single_changed)
        self.t_size = dspin(10, 0.5, 1000, 0.5, 2, " mm")
        self.t_size.valueChanged.connect(lambda v: self._set("size", v))
        self.t_bold, self.t_italic = checkbox(""), checkbox("")
        self.t_bold.toggled.connect(lambda v: self._set("bold", v))
        self.t_italic.toggled.connect(lambda v: self._set("italic", v))
        self.t_spacing = dspin(0, -50, 200, 0.1, 2, " mm")
        self.t_spacing.valueChanged.connect(lambda v: self._set("spacing", v))
        self.t_bend = dspin(0, -100000, 100000, 5, 1, " mm")
        self.t_bend.valueChanged.connect(lambda v: self._set("bend", v))
        self.t_var = QPushButton()
        self.t_var.clicked.connect(lambda: self.win.run("varText"))
        self.l_text, self.l_font, self.l_size, self.l_spacing, self.l_bend = QLabel(), QLabel("Font"), QLabel(), QLabel(), QLabel()
        bi = QWidget()
        h = QHBoxLayout(bi)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(self.t_bold)
        h.addWidget(self.t_italic)
        h.addStretch(1)
        self.l_single = QLabel()
        for lab, w in ((self.l_text, self.t_text), (self.l_font, self.t_font), (self.l_single, self.t_single), (self.l_size, self.t_size), ("", bi),
                       (self.l_spacing, self.t_spacing), (self.l_bend, self.t_bend), ("", self.t_var)):
            f.addRow(lab, w)
        self.t_hint = hint("")
        f.addRow("", self.t_hint)
        self.stack.addWidget(self.p_text)

        # array
        self.p_arr = QWidget()
        f = QFormLayout(self.p_arr)
        self.a_kind = QLabel()
        self.a_cols, self.a_rows = ispin(3, 1, 500), ispin(2, 1, 500)
        self.a_dx, self.a_dy = dspin(5, -1000, 1000, 0.5, 2, " mm"), dspin(5, -1000, 1000, 0.5, 2, " mm")
        self.a_n = ispin(6, 2, 720)
        self.a_angle = dspin(360, -360, 360, 15, 1, " °")
        self.a_rot = checkbox("", True)
        for w, key in ((self.a_cols, "cols"), (self.a_rows, "rows"), (self.a_dx, "dx"), (self.a_dy, "dy"), (self.a_n, "n"), (self.a_angle, "angle")):
            w.valueChanged.connect(lambda v, k=key: self._set_array(k, v))
        self.a_rot.toggled.connect(lambda v: self._set_array("rotate", v))
        self.a_flat = QPushButton()
        self.a_flat.clicked.connect(self._flatten_array)
        self.l_cols, self.l_rows, self.l_dx, self.l_dy, self.l_an, self.l_angle = QLabel(), QLabel(), QLabel(), QLabel(), QLabel(), QLabel()
        self.arr_rows = [(self.l_cols, self.a_cols), (self.l_rows, self.a_rows), (self.l_dx, self.a_dx), (self.l_dy, self.a_dy),
                         (self.l_an, self.a_n), (self.l_angle, self.a_angle), ("", self.a_rot)]
        f.addRow("", self.a_kind)
        for lab, w in self.arr_rows:
            f.addRow(lab, w)
        f.addRow("", self.a_flat)
        self.arr_form = f
        self.stack.addWidget(self.p_arr)

        win.doc.selectionChanged.connect(self.refresh)
        win.doc.changed.connect(self.refresh)
        self.retranslate()
        self.refresh()

    def retranslate(self):
        L = self.win.L
        self.p_empty.setText(L("Seleziona una sola forma per vederne le proprietà: raggio degli angoli dei rettangoli, lati dei poligoni, testo (font, spaziatura, curvatura) e parametri degli array dinamici.",
                               "Select a single shape to see its properties: rectangle corner radius, polygon sides, text (font, spacing, bend) and live array parameters."))
        self.l_radius.setText(L("Raggio angoli", "Corner radius"))
        self.l_prio.setText(L("Priorità di taglio", "Cut priority"))
        self.prio.setToolTip(L("Con Ottimizzazione › Per priorità le forme con il valore più basso vengono tagliate per prime.",
                               "With Optimization › By priority, lower values are cut first."))
        self.l_rx.setText(L("Raggio X", "Radius X"))
        self.l_ry.setText(L("Raggio Y", "Radius Y"))
        self.l_n.setText(L("Lati / punte", "Sides / points"))
        self.l_r.setText(L("Raggio", "Radius"))
        self.l_inner.setText(L("Raggio interno (stelle)", "Inner radius (stars)"))
        self.l_text.setText(L("Testo", "Text"))
        self.l_single.setText(L("Tratto singolo", "Single-line"))
        self.t_single.setToolTip(L("Font Hershey a tratto singolo: il laser incide una sola linea invece del contorno delle lettere (più veloce, ideale per numeri di serie e scritte piccole).",
                                   "Hershey single-line font: the laser burns one stroke instead of the letter outlines (faster, ideal for serial numbers and small text)."))
        self.l_size.setText(L("Altezza", "Height"))
        self.t_bold.setText(L("Grassetto", "Bold"))
        self.t_italic.setText(L("Corsivo", "Italic"))
        self.l_spacing.setText(L("Spaziatura lettere", "Letter spacing"))
        self.l_bend.setText(L("Curvatura (raggio)", "Bend (radius)"))
        self.t_var.setText(L("Testo variabile...", "Variable text..."))
        self.t_hint.setText(L("Curvatura 0 = dritto; positivo = arco verso l'alto, negativo = verso il basso.",
                              "Bend 0 = straight; positive = arch upward, negative = downward."))
        self.l_cols.setText(L("Colonne", "Columns"))
        self.l_rows.setText(L("Righe", "Rows"))
        self.l_dx.setText(L("Spazio orizz.", "H spacing"))
        self.l_dy.setText(L("Spazio vert.", "V spacing"))
        self.l_an.setText(L("Copie", "Copies"))
        self.l_angle.setText(L("Angolo totale", "Total angle"))
        self.a_rot.setText(L("Ruota le copie", "Rotate copies"))
        self.a_flat.setText(L("Appiattisci array (copie indipendenti)", "Flatten array (independent copies)"))
        self.refresh()

    def _shape(self):
        sel = self.win.doc.selected_shapes()
        return sel[0] if len(sel) == 1 else None

    def refresh(self, *_):
        s = self._shape()
        self._loading = True
        L = self.win.L
        if s is None:
            self.title.setText(L("Proprietà forma", "Shape properties"))
            self.stack.setCurrentWidget(self.p_empty)
        elif s["type"] == "rect":
            self.title.setText(L("Rettangolo", "Rectangle"))
            self.r_radius.setValue(s.get("radius", 0.0))
            self.stack.setCurrentWidget(self.p_rect)
        elif s["type"] == "circle":
            self.title.setText(L("Ellisse", "Ellipse"))
            self.e_rx.setValue(s["rx"])
            self.e_ry.setValue(s["ry"])
            self.stack.setCurrentWidget(self.p_ell)
        elif s["type"] == "poly" and "regular" in s:
            m = s["regular"]
            self.title.setText(L("Stella", "Star") if m.get("inner") else L("Poligono", "Polygon"))
            self.g_n.setValue(m["n"])
            self.g_r.setValue(m["r"])
            self.g_inner.setValue(round((m.get("inner") or 0.45) * 100))
            self.g_inner.setEnabled(bool(m.get("inner")))
            self.stack.setCurrentWidget(self.p_poly)
        elif s["type"] == "text":
            self.title.setText(L("Testo", "Text"))
            self.t_text.setText(s["text"])
            single = M.is_single_line(s)
            self.t_single.setCurrentIndex(max(0, self.t_single.findData(s["font"])) if single else 0)
            self.t_font.setEnabled(not single)
            if not single:
                self.t_font.setCurrentText(s.get("font") or "Helvetica")
            self.t_bold.setEnabled(not single)
            self.t_italic.setEnabled(not single)
            self.t_spacing.setEnabled(not single)
            self.t_bend.setEnabled(not single)
            self.t_size.setValue(s["size"])
            self.t_bold.setChecked(bool(s.get("bold")))
            self.t_italic.setChecked(bool(s.get("italic")))
            self.t_spacing.setValue(s.get("spacing", 0.0))
            self.t_bend.setValue(s.get("bend", 0.0))
            self.stack.setCurrentWidget(self.p_text)
        elif s["type"] == "group" and "array" in s:
            m = s["array"]
            grid = m["kind"] == "grid"
            self.title.setText(L("Array dinamico", "Live array"))
            self.a_kind.setText(L("Griglia", "Grid") if grid else L("Circolare", "Circular"))
            for w, key in ((self.a_cols, "cols"), (self.a_rows, "rows"), (self.a_dx, "dx"), (self.a_dy, "dy")):
                if grid:
                    w.setValue(m[key])
            if not grid:
                self.a_n.setValue(m["n"])
                self.a_angle.setValue(m["angle"])
                self.a_rot.setChecked(m.get("rotate", True))
            for lab, w in self.arr_rows:
                visible = (w in (self.a_cols, self.a_rows, self.a_dx, self.a_dy)) == grid
                w.setVisible(visible)
                lw = self.arr_form.labelForField(w)
                if lw:
                    lw.setVisible(visible)
            self.stack.setCurrentWidget(self.p_arr)
        else:
            self.title.setText(L("Proprietà forma", "Shape properties"))
            self.stack.setCurrentWidget(self.p_empty)
        self.prio_row.setVisible(s is not None)
        if s is not None:
            self.prio.setValue(int(s.get("priority", 0) or 0))
        self.shape_id = s["id"] if s else None
        self._loading = False

    def _apply(self, fn):
        if self._loading:
            return
        s = self._shape()
        if s is None:
            return
        doc = self.win.doc
        doc.save_undo(self.win.L("Proprietà forma", "Shape properties"))
        fn(s)
        doc.changed.emit()

    def _set(self, key, value):
        def fn(s):
            if value in (0, 0.0, False, "") and key in ("radius", "spacing", "bend", "bold", "italic", "priority"):
                s.pop(key, None)
            else:
                s[key] = value
        self._apply(fn)

    def _single_changed(self, _i):
        fid = self.t_single.currentData()
        self._set("font", fid if fid else self.t_font.currentFont().family())

    def _set_regular(self, key, value):
        def fn(s):
            s["regular"][key] = value
            s["points"] = M.regular_meta_points(s["regular"])
        self._apply(fn)

    def _set_array(self, key, value):
        def fn(s):
            s["array"][key] = value
            regenerate_array(self.win, s)
        self._apply(fn)

    def _flatten_array(self):
        self._apply(flatten_array)
