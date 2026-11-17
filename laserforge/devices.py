"""Device profiles (several lasers), pre-job checklist, project notes and 1D barcodes."""

import json
from pathlib import Path

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox, QDialog, QDialogButtonBox, QFileDialog, QHBoxLayout, QInputDialog, QLabel, QListWidget, QMessageBox,
    QPlainTextEdit, QPushButton, QVBoxLayout,
)

from .ui_common import checkbox, hint


# ====================================================================== device profiles
def load_devices(win):
    return win._load_json("devices", [])


def save_devices(win, devices):
    win._store_json("devices", devices)


def apply_device(win, dev):
    win.engine.update_machine(dev["machine"])
    if dev.get("port"):
        win.laser_panel.select_port(dev["port"])
    win._store_json("machine", win.engine.machine_dict())
    win.settings.setValue("device", dev["name"])
    win.move_panel.load()
    win.invalidate_gcode()
    win.canvas.update()
    win.laser_panel.refresh_devices()
    win.status(win.L("Dispositivo: ", "Device: ") + dev["name"])


def devices_dialog(win):
    L = win.L
    d = QDialog(win)
    d.setWindowTitle(L("Dispositivi", "Devices"))
    d.setMinimumSize(420, 380)
    lay = QVBoxLayout(d)
    lst = QListWidget()
    lay.addWidget(lst, 1)
    devices = load_devices(win)
    current = win.settings.value("device", "")

    def fill():
        lst.clear()
        for dv in devices:
            m = dv["machine"]
            lst.addItem(f"{'● ' if dv['name'] == current else '   '}{dv['name']}   —   {m.get('workspace_x', 0):.0f}×{m.get('workspace_y', 0):.0f} mm, "
                        f"S max {m.get('max_power', 1000)}, {m.get('baud_rate', 115200)} baud")

    def row():
        return lst.currentRow()

    def new():
        name, ok = QInputDialog.getText(d, L("Nuovo dispositivo", "New device"), L("Nome:", "Name:"), text=L("Il mio laser", "My laser"))
        if ok and name.strip():
            devices.append({"name": name.strip(), "machine": win.engine.machine_dict()})
            save_devices(win, devices)
            fill()

    def use():
        nonlocal current
        i = row()
        if i >= 0:
            apply_device(win, devices[i])
            current = devices[i]["name"]
            fill()

    def update():
        i = row()
        if i >= 0:
            devices[i]["machine"] = win.engine.machine_dict()
            save_devices(win, devices)
            fill()

    def rename():
        i = row()
        if i < 0:
            return
        name, ok = QInputDialog.getText(d, L("Rinomina", "Rename"), L("Nome:", "Name:"), text=devices[i]["name"])
        if ok and name.strip():
            devices[i]["name"] = name.strip()
            save_devices(win, devices)
            fill()

    def delete():
        i = row()
        if i >= 0 and QMessageBox.question(d, "LaserForge", L(f"Eliminare «{devices[i]['name']}»?", f"Delete “{devices[i]['name']}”?")) == QMessageBox.Yes:
            devices.pop(i)
            save_devices(win, devices)
            fill()

    def export():
        i = row()
        if i < 0:
            return
        path, _ = QFileDialog.getSaveFileName(d, L("Esporta dispositivo", "Export device"), str(Path.home() / f"{devices[i]['name']}.lfdevice"),
                                              "LaserForge device (*.lfdevice)")
        if path:
            Path(path).write_text(json.dumps(devices[i], indent=1), encoding="utf-8")

    def import_():
        path, _ = QFileDialog.getOpenFileName(d, L("Importa dispositivo", "Import device"), str(Path.home()), "LaserForge device (*.lfdevice *.json)")
        if not path:
            return
        try:
            dv = json.loads(Path(path).read_text(encoding="utf-8"))
            assert isinstance(dv, dict) and "machine" in dv
        except (OSError, ValueError, AssertionError):
            win.toast(L("File non valido", "Invalid file"))
            return
        dv.setdefault("name", Path(path).stem)
        devices.append(dv)
        save_devices(win, devices)
        fill()

    h1, h2 = QHBoxLayout(), QHBoxLayout()
    for text, fn, box in ((L("Nuovo dalle impostazioni attuali", "New from current settings"), new, h1), (L("Usa", "Use"), use, h1),
                          (L("Aggiorna", "Update"), update, h1), (L("Rinomina", "Rename"), rename, h2), (L("Elimina", "Delete"), delete, h2),
                          (L("Esporta…", "Export…"), export, h2), (L("Importa…", "Import…"), import_, h2)):
        b = QPushButton(text)
        b.clicked.connect(fn)
        box.addWidget(b)
    lay.addLayout(h1)
    lay.addLayout(h2)
    lay.addWidget(hint(L("Ogni dispositivo salva le impostazioni della macchina: area di lavoro, velocità e potenza massime, baud, "
                         "G-code iniziale e finale, rotativo. 'Aggiorna' salva le impostazioni attuali nel dispositivo selezionato.",
                         "Each device stores the machine settings: work area, max speed and power, baud, start/end G-code, rotary. "
                         "'Update' saves the current settings into the selected device.")))
    lst.itemDoubleClicked.connect(lambda _it: use())
    bb = QDialogButtonBox(QDialogButtonBox.Close)
    bb.rejected.connect(d.reject)
    lay.addWidget(bb)
    fill()
    d.exec()


# ====================================================================== pre-job checklist
DEFAULT_CHECKLIST = ["Aspirazione / ventilazione accesa", "Aria compressa aperta", "Messa a fuoco controllata",
                     "Materiale fissato e piano", "Occhiali protettivi indossati"]


def checklist_editor(win):
    L = win.L
    d = QDialog(win)
    d.setWindowTitle(L("Checklist prima del lavoro", "Pre-job checklist"))
    d.setMinimumWidth(420)
    lay = QVBoxLayout(d)
    on = checkbox(L("Chiedi la checklist prima di ogni lavoro", "Ask for the checklist before every job"), bool(win.prefs.get("checklist_enabled")))
    lay.addWidget(on)
    lay.addWidget(QLabel(L("Una voce per riga:", "One item per line:")))
    ed = QPlainTextEdit("\n".join(win._load_json("checklist", None) or DEFAULT_CHECKLIST))
    ed.setMinimumHeight(160)
    lay.addWidget(ed)
    bb = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
    bb.accepted.connect(d.accept)
    bb.rejected.connect(d.reject)
    lay.addWidget(bb)
    if d.exec():
        win._store_json("checklist", [ln.strip() for ln in ed.toPlainText().splitlines() if ln.strip()])
        win.prefs["checklist_enabled"] = on.isChecked()
        from .prefs import save_prefs
        save_prefs(win.settings, win.prefs)


def run_checklist(win):
    """True when the job may start (checklist disabled, or every item ticked)."""
    if not win.prefs.get("checklist_enabled"):
        return True
    items = win._load_json("checklist", None) or DEFAULT_CHECKLIST
    if not items:
        return True
    L = win.L
    d = QDialog(win)
    d.setWindowTitle(L("Prima di avviare", "Before starting"))
    lay = QVBoxLayout(d)
    boxes = []
    for it in items:
        c = QCheckBox(it)
        lay.addWidget(c)
        boxes.append(c)
    bb = QDialogButtonBox()
    start = bb.addButton(L("Avvia", "Start"), QDialogButtonBox.AcceptRole)
    start.setProperty("role", "primary")
    bb.addButton(win.tr("cancel"), QDialogButtonBox.RejectRole)
    start.setEnabled(False)
    for c in boxes:
        c.toggled.connect(lambda _v: start.setEnabled(all(b.isChecked() for b in boxes)))
    bb.accepted.connect(d.accept)
    bb.rejected.connect(d.reject)
    lay.addWidget(bb)
    return bool(d.exec())


# ====================================================================== project notes
def notes_dialog(win):
    L = win.L
    d = QDialog(win)
    d.setWindowTitle(L("Note del progetto", "Project notes"))
    d.setMinimumSize(460, 320)
    lay = QVBoxLayout(d)
    ed = QPlainTextEdit(win.project_notes.get("text", ""))
    ed.setPlaceholderText(L("Materiale, impostazioni provate, istruzioni per il cliente…", "Material, tested settings, instructions…"))
    lay.addWidget(ed, 1)
    show = checkbox(L("Mostra le note quando apro il progetto", "Show notes when the project is opened"), bool(win.project_notes.get("show_on_load")))
    lay.addWidget(show)
    bb = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
    bb.accepted.connect(d.accept)
    bb.rejected.connect(d.reject)
    lay.addWidget(bb)
    if d.exec():
        new = {"text": ed.toPlainText(), "show_on_load": show.isChecked()}
        if new != win.project_notes:
            win.project_notes = new
            win.doc.set_dirty(True)


# ====================================================================== 1D barcodes
CODE128 = ["212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213",
           "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221", "223211", "221132",
           "221231", "213212", "223112", "312131", "311222", "321122", "321221", "312212", "322112", "322211",
           "212123", "212321", "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313",
           "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121", "313121", "211331",
           "231131", "213113", "213311", "213131", "311123", "311321", "331121", "312113", "312311", "332111",
           "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214",
           "112412", "122114", "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111",
           "111242", "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
           "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311", "113141",
           "114131", "311141", "411131", "211412", "211214", "211232", "2331112"]

EAN_L = ["0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011", "0110111", "0001011"]
EAN_R = ["".join("1" if c == "0" else "0" for c in code) for code in EAN_L]
EAN_G = [code[::-1] for code in EAN_R]
EAN_PARITY = ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"]


def code128_modules(text):
    """Module string ('1' = bar) for Code 128, set B (printable ASCII 32–126) with checksum."""
    if not text or any(not (32 <= ord(c) <= 126) for c in text):
        raise ValueError("Code 128 B: solo caratteri ASCII stampabili")
    values = [104] + [ord(c) - 32 for c in text]
    check = (values[0] + sum(v * (i + 1) for i, v in enumerate(values[1:]))) % 103
    out = ""
    for v in values + [check, 106]:
        bar = True
        for w in CODE128[v]:
            out += ("1" if bar else "0") * int(w)
            bar = not bar
    return out


def ean_check_digit(digits):
    """Check digit for EAN-13 (12 digits) or EAN-8 (7 digits): weights 3,1 from the right."""
    total = sum(int(d) * (3 if i % 2 == 0 else 1) for i, d in enumerate(reversed(digits)))
    return str((10 - total % 10) % 10)


def ean13_modules(data):
    digits = "".join(c for c in data if c.isdigit())
    if len(digits) == 12:
        digits += ean_check_digit(digits)
    if len(digits) != 13 or ean_check_digit(digits[:12]) != digits[12]:
        raise ValueError("EAN-13: servono 12 cifre (o 13 con cifra di controllo corretta)")
    parity = EAN_PARITY[int(digits[0])]
    left = "".join((EAN_L if p == "L" else EAN_G)[int(d)] for p, d in zip(parity, digits[1:7]))
    right = "".join(EAN_R[int(d)] for d in digits[7:])
    return "101" + left + "01010" + right + "101", digits


def ean8_modules(data):
    digits = "".join(c for c in data if c.isdigit())
    if len(digits) == 7:
        digits += ean_check_digit(digits)
    if len(digits) != 8 or ean_check_digit(digits[:7]) != digits[7]:
        raise ValueError("EAN-8: servono 7 cifre (o 8 con cifra di controllo corretta)")
    return "101" + "".join(EAN_L[int(d)] for d in digits[:4]) + "01010" + "".join(EAN_R[int(d)] for d in digits[4:]) + "101", digits


def barcode_modules(kind, data):
    """(module string, human-readable text)."""
    if kind == "code128":
        return code128_modules(data), data
    if kind == "ean13":
        return ean13_modules(data)
    if kind == "upca":
        digits = "".join(c for c in data if c.isdigit())
        mods, full = ean13_modules("0" + digits)
        return mods, full[1:]
    if kind == "ean8":
        return ean8_modules(data)
    raise ValueError(kind)


def barcode_rects(modules, x0, y0, module_w, height):
    """Merge runs of bar modules into rectangles (x, y, w, h)."""
    rects, i, n = [], 0, len(modules)
    while i < n:
        if modules[i] != "1":
            i += 1
            continue
        j = i
        while j < n and modules[j] == "1":
            j += 1
        rects.append((x0 + i * module_w, y0, (j - i) * module_w, height))
        i = j
    return rects
