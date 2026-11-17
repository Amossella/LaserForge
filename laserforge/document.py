"""Design document: shapes, selection, undo/redo, layer assignment."""

from PySide6.QtCore import QObject, Signal

from . import model as M


class Document(QObject):
    changed = Signal()            # geometry / content changed
    selectionChanged = Signal()
    dirtyChanged = Signal(bool)

    pagesChanged = Signal()

    def __init__(self):
        super().__init__()
        self.pages: list = [{"name": "Pagina 1", "shapes": []}]
        self.page = 0
        self.selected: list = []
        self.active_layer = 0
        self.undo_stack: list = []
        self.redo_stack: list = []
        self.undo_labels: list = []
        self.redo_labels: list = []
        self.current_label = None
        self.next_id = 1
        self.dirty = False

    # the shapes of the current page
    @property
    def shapes(self):
        return self.pages[self.page]["shapes"]

    @shapes.setter
    def shapes(self, value):
        self.pages[self.page]["shapes"] = value

    # ---------- pages ----------
    def set_page(self, i):
        if 0 <= i < len(self.pages) and i != self.page:
            self.page = i
            self.selected = []
            self._clear_history()
            self.pagesChanged.emit()
            self.notify()

    def add_page(self, name=None, shapes=None):
        self.pages.append({"name": name or f"Pagina {len(self.pages) + 1}", "shapes": shapes or []})
        self.set_dirty(True)
        self.set_page(len(self.pages) - 1)
        self.pagesChanged.emit()

    def remove_page(self, i):
        if len(self.pages) <= 1:
            return
        self.pages.pop(i)
        self.page = min(self.page, len(self.pages) - 1)
        self.selected = []
        self._clear_history()
        self.set_dirty(True)
        self.pagesChanged.emit()
        self.notify()

    def _clear_history(self):
        self.undo_stack.clear()
        self.redo_stack.clear()
        self.undo_labels.clear()
        self.redo_labels.clear()

    # ---------- ids / creation ----------
    def new_id(self):
        i = self.next_id
        self.next_id += 1
        return i

    def create(self, type_, **props):
        s = {"id": self.new_id(), "type": type_, "layer": self.active_layer}
        s.update(props)
        return s

    def reid(self, s):
        s["id"] = self.new_id()
        for c in s.get("children", []):
            self.reid(c)
        return s

    def find(self, sid):
        return next((s for s in self.shapes if s["id"] == sid), None)

    def selected_shapes(self):
        return [s for s in self.shapes if s["id"] in self.selected]

    def selected_in_order(self):
        """Selection in click order (LightBurn aligns to the last selected)."""
        return [s for s in (self.find(i) for i in self.selected) if s]

    # ---------- undo ----------
    def set_dirty(self, d=True):
        if self.dirty != d:
            self.dirty = d
            self.dirtyChanged.emit(d)

    def save_undo(self, label=None):
        self.push_undo_snapshot(M.clone(self.shapes), label)

    def push_undo_snapshot(self, snapshot, label=None):
        self.undo_stack.append(snapshot)
        self.undo_labels.append(label or self.current_label or "Modifica")
        if len(self.undo_stack) > 100:
            self.undo_stack.pop(0)
            self.undo_labels.pop(0)
        self.redo_stack.clear()
        self.redo_labels.clear()
        self.set_dirty(True)

    def undo(self):
        if not self.undo_stack:
            return
        self.redo_stack.append(M.clone(self.shapes))
        self.redo_labels.append(self.undo_labels.pop() if self.undo_labels else "Modifica")
        self.shapes = self.undo_stack.pop()
        self._prune_selection()
        self.set_dirty(True)
        self.notify()

    def redo(self):
        if not self.redo_stack:
            return
        self.undo_stack.append(M.clone(self.shapes))
        self.undo_labels.append(self.redo_labels.pop() if self.redo_labels else "Modifica")
        self.shapes = self.redo_stack.pop()
        self._prune_selection()
        self.set_dirty(True)
        self.notify()

    def _prune_selection(self):
        ids = {s["id"] for s in self.shapes}
        self.selected = [i for i in self.selected if i in ids]

    # ---------- edits ----------
    def notify(self):
        self.changed.emit()
        self.selectionChanged.emit()

    def add_shapes(self, shapes, select=True):
        self.save_undo()
        self.shapes.extend(shapes)
        if select:
            self.selected = [s["id"] for s in shapes]
        self.notify()

    def set_selection(self, ids):
        self.selected = list(dict.fromkeys(ids))
        self.selectionChanged.emit()

    def undo_to(self, steps):
        for _ in range(steps):
            self.undo()

    def redo_to(self, steps):
        for _ in range(steps):
            self.redo()

    def reset(self):
        self.pages = [{"name": "Pagina 1", "shapes": []}]
        self.page = 0
        self.selected = []
        self._clear_history()
        self.next_id = 1
        self.pagesChanged.emit()
        self.set_dirty(False)
        self.notify()

    def load(self, shapes, next_id=None, pages=None):
        self.pages = pages if pages else [{"name": "Pagina 1", "shapes": shapes}]
        self.page = 0
        self.selected = []
        self._clear_history()
        all_ids = [s["id"] for pg in self.pages for s in pg["shapes"]]
        self.next_id = next_id or (max(all_ids, default=0) + 1)
        self.pagesChanged.emit()
        self.set_dirty(False)
        self.notify()

    def used_layers(self):
        return sorted({s.get("layer", 0) for s in M.flatten(self.shapes)})
