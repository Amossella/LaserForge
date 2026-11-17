"""Editable material library (like LightBurn's Cut Library): materials grouped by name/thickness,
saved to a JSON file, with import/merge and export."""

import json
import os
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QAbstractItemView, QDialog, QDialogButtonBox, QFileDialog, QFormLayout, QHBoxLayout, QLineEdit, QMessageBox,
    QPushButton, QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget,
)

from .engine import MATERIAL_PRESETS
from .ui_common import dspin, hint

LIB_PATH = Path(os.environ.get("LASERFORGE_MATERIALS") or Path.home() / "Library" / "Application Support" / "LaserForge" / "materials.json")

# layer fields a material stores (name and colour stay with the layer)
MATERIAL_FIELDS = ["speed", "power_min", "power_max", "passes", "cut_mode", "air_assist", "air_power", "air_cut_only", "lead_in", "lead_out", "overcut", "perforate", "perf_cut", "perf_skip", "flood_fill", "sublayers", "z_offset", "laser_mode",
                   "line_interval", "overscan", "scan_angle", "cross_hatch", "bidir_scan", "dwell_time", "kerf_offset",
                   "power_scale", "dot_mode", "dot_time", "ramp_length", "tab_count", "tab_width", "start_delay", "z_step"]


def _seed():
    out = []
    for name, p in MATERIAL_PRESETS.items():
        parts = name.rsplit(" ", 1)
        thick = parts[1] if len(parts) == 2 and parts[1].endswith("mm") else ""
        mat = parts[0] if thick else name
        out.append({"material": mat, "thickness": thick, "name": "Taglio" if p.get("cut_mode") == "line" else "Incisione",
                    "settings": dict(p)})
    return out


def load_library():
    try:
        data = json.loads(LIB_PATH.read_text(encoding="utf-8"))
        if isinstance(data, dict):
            data = data.get("materials", [])
        return [e for e in data if isinstance(e, dict) and "settings" in e]
    except (OSError, ValueError):
        return _seed()


def save_library(entries):
    LIB_PATH.parent.mkdir(parents=True, exist_ok=True)
    tmp = LIB_PATH.with_suffix(".tmp")
    tmp.write_text(json.dumps({"app": "LaserForge", "version": 1, "materials": entries}, indent=1), encoding="utf-8")
    os.replace(tmp, LIB_PATH)


def entry_label(e):
    return f"{e.get('name') or '—'}   ·   {round(e['settings'].get('speed', 0))} mm/min · {round(e['settings'].get('power_max', 0))}%" \
           f" · {str(e['settings'].get('air_assist', 'off')).upper()}"


class LibraryPanel(QWidget):
    def __init__(self, win):
        super().__init__()
        self.win = win
        self.entries = load_library()
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        self.search = QLineEdit()
        self.search.textChanged.connect(self.refresh)
        lay.addWidget(self.search)
        self.tree = QTreeWidget()
        self.tree.setHeaderHidden(True)
        self.tree.setSelectionMode(QAbstractItemView.SingleSelection)
        self.tree.itemDoubleClicked.connect(lambda it, _c: self.apply())
        lay.addWidget(self.tree, 1)
        row1, row2 = QHBoxLayout(), QHBoxLayout()
        self.b_apply, self.b_new, self.b_update = QPushButton(), QPushButton(), QPushButton()
        self.b_edit, self.b_del, self.b_import, self.b_export = QPushButton(), QPushButton(), QPushButton(), QPushButton()
        self.b_apply.setProperty("role", "primary")
        self.b_apply.clicked.connect(self.apply)
        self.b_new.clicked.connect(self.new_from_layer)
        self.b_update.clicked.connect(self.update_from_layer)
        self.b_edit.clicked.connect(self.edit)
        self.b_del.clicked.connect(self.delete)
        self.b_import.clicked.connect(self.import_merge)
        self.b_export.clicked.connect(self.export)
        for b in (self.b_apply, self.b_new, self.b_update):
            row1.addWidget(b)
        for b in (self.b_edit, self.b_del, self.b_import, self.b_export):
            row2.addWidget(b)
        lay.addLayout(row1)
        lay.addLayout(row2)
        self.hint = hint("")
        lay.addWidget(self.hint)
        self.retranslate()

    def retranslate(self):
        L = self.win.L
        self.search.setPlaceholderText(L("Cerca materiale…", "Search material…"))
        self.b_apply.setText(L("Applica al layer", "Apply to layer"))
        self.b_new.setText(L("Nuovo dal layer", "New from layer"))
        self.b_update.setText(L("Aggiorna dal layer", "Update from layer"))
        self.b_edit.setText(L("Rinomina", "Rename"))
        self.b_del.setText(L("Elimina", "Delete"))
        self.b_import.setText(L("Importa/unisci…", "Import/merge…"))
        self.b_export.setText(L("Esporta…", "Export…"))
        self.hint.setText(L("Doppio clic su una voce per applicarla al layer attivo. La libreria si salva in ",
                            "Double-click an entry to apply it to the active layer. The library is saved in ") + str(LIB_PATH))
        self.refresh()

    # ---------- tree
    def refresh(self, *_):
        q = self.search.text().strip().lower() if hasattr(self, "search") else ""
        self.tree.clear()
        groups = {}
        for i, e in enumerate(self.entries):
            key = f"{e.get('material', '?')} {e.get('thickness', '')}".strip()
            if q and q not in key.lower() and q not in (e.get("name") or "").lower():
                continue
            parent = groups.get(key)
            if parent is None:
                parent = QTreeWidgetItem([key])
                f = parent.font(0)
                f.setBold(True)
                parent.setFont(0, f)
                self.tree.addTopLevelItem(parent)
                groups[key] = parent
            child = QTreeWidgetItem([entry_label(e)])
            child.setData(0, Qt.UserRole, i)
            parent.addChild(child)
        self.tree.expandAll()

    def _current(self):
        it = self.tree.currentItem()
        if it is None:
            return None
        i = it.data(0, Qt.UserRole)
        if i is None and it.childCount():
            i = it.child(0).data(0, Qt.UserRole)
        return i

    def _layer_settings(self):
        d = self.win.engine.layer_dicts()[self.win.doc.active_layer]
        return {k: d[k] for k in MATERIAL_FIELDS if k in d}

    def _save(self):
        save_library(self.entries)
        self.refresh()

    # ---------- actions
    def apply(self):
        i = self._current()
        if i is None:
            self.win.toast(self.win.L("Scegli un materiale", "Pick a material"))
            return
        e = self.entries[i]
        idx = self.win.doc.active_layer
        self.win.engine.update_layer(idx, e["settings"])
        self.win.persist_layers()
        self.win.cuts.load(idx)
        self.win.cuts.refresh()
        self.win.invalidate_gcode()
        self.win.toast(f"{e.get('material')} {e.get('thickness', '')} · {e.get('name')} → {self.win.engine.layers[idx].name}")

    def _ask(self, title, e):
        L = self.win.L
        d = QDialog(self)
        d.setWindowTitle(title)
        f = QFormLayout(d)
        mat, thick, name = QLineEdit(e.get("material", "")), QLineEdit(e.get("thickness", "")), QLineEdit(e.get("name", ""))
        thick.setPlaceholderText("3mm")
        name.setPlaceholderText(L("es. Taglio, Incisione", "e.g. Cut, Engrave"))
        f.addRow(L("Materiale", "Material"), mat)
        f.addRow(L("Spessore", "Thickness"), thick)
        f.addRow(L("Descrizione", "Description"), name)
        bb = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        bb.accepted.connect(d.accept)
        bb.rejected.connect(d.reject)
        f.addRow(bb)
        if d.exec() and mat.text().strip():
            return {"material": mat.text().strip(), "thickness": thick.text().strip(), "name": name.text().strip()}
        return None

    def new_from_layer(self):
        meta = self._ask(self.win.L("Nuovo materiale", "New material"), {"name": self.win.L("Taglio", "Cut")})
        if meta:
            self.entries.append({**meta, "settings": self._layer_settings()})
            self._save()

    def update_from_layer(self):
        i = self._current()
        if i is None:
            return
        self.entries[i]["settings"] = self._layer_settings()
        self._save()
        self.win.toast(self.win.L("Materiale aggiornato", "Material updated"))

    def edit(self):
        i = self._current()
        if i is None:
            return
        meta = self._ask(self.win.L("Rinomina materiale", "Rename material"), self.entries[i])
        if meta:
            self.entries[i].update(meta)
            self._save()

    def delete(self):
        i = self._current()
        if i is None:
            return
        e = self.entries[i]
        if QMessageBox.question(self, "LaserForge", self.win.L(f"Eliminare «{e.get('material')} {e.get('thickness', '')} · {e.get('name')}»?",
                                                              f"Delete “{e.get('material')} {e.get('thickness', '')} · {e.get('name')}”?")) == QMessageBox.Yes:
            self.entries.pop(i)
            self._save()

    def import_merge(self):
        path, _ = QFileDialog.getOpenFileName(self, self.win.L("Importa libreria", "Import library"), str(Path.home()), "JSON (*.json *.lflib)")
        if not path:
            return
        try:
            data = json.loads(Path(path).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            self.win.toast(self.win.L("File non valido", "Invalid file"))
            return
        incoming = data.get("materials", data) if isinstance(data, dict) else data
        key = lambda e: (e.get("material"), e.get("thickness"), e.get("name"))
        existing = {key(e): k for k, e in enumerate(self.entries)}
        added = replaced = 0
        for e in incoming:
            if not isinstance(e, dict) or "settings" not in e:
                continue
            if key(e) in existing:
                self.entries[existing[key(e)]] = e
                replaced += 1
            else:
                self.entries.append(e)
                added += 1
        self._save()
        self.win.toast(self.win.L(f"{added} aggiunti, {replaced} aggiornati", f"{added} added, {replaced} updated"))

    def export(self):
        path, _ = QFileDialog.getSaveFileName(self, self.win.L("Esporta libreria", "Export library"), str(Path.home() / "materiali-laserforge.json"), "JSON (*.json)")
        if path:
            Path(path).write_text(json.dumps({"app": "LaserForge", "version": 1, "materials": self.entries}, indent=1), encoding="utf-8")
            self.win.toast(os.path.basename(path))
