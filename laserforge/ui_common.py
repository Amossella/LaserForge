"""Shared Qt helpers: dark theme, SVG icons, segmented control, numeric fields."""

from PySide6.QtCore import QByteArray, QRectF, QSize, Qt, Signal
from PySide6.QtGui import QColor, QIcon, QPainter, QPalette, QPixmap
from PySide6.QtSvg import QSvgRenderer
from PySide6.QtWidgets import (
    QButtonGroup, QCheckBox, QDoubleSpinBox, QFormLayout, QFrame, QHBoxLayout, QLabel, QSpinBox, QToolButton,
    QWidget,
)

from .defs import ICON_PATHS

ACCENT = "#2f7bf5"
AIR = "#22b8cf"

STYLE = f"""
QMainWindow, QDialog {{ background: #2b2c30; }}
QWidget {{ color: #dfe1e5; font-size: 12px; }}
QToolBar {{ background: #303136; border: none; spacing: 2px; padding: 2px; }}
QToolBar::separator {{ background: #45474d; width: 1px; height: 1px; margin: 4px; }}
QToolButton {{ border: 1px solid transparent; border-radius: 4px; padding: 3px; }}
QToolButton:hover {{ background: #3d3f45; }}
QToolButton:checked {{ background: #24426f; border-color: {ACCENT}; }}
QDockWidget {{ titlebar-close-icon: none; color: #c9cbd1; }}
QDockWidget::title {{ background: #34353a; padding: 5px 8px; text-align: left; }}
QStatusBar {{ background: #232427; color: #a8aab1; }}
QStatusBar QLabel {{ padding: 0 6px; }}
QLineEdit, QPlainTextEdit, QTextEdit, QComboBox {{
    background: #1f2023; border: 1px solid #45474d; border-radius: 4px; padding: 3px 5px; selection-background-color: {ACCENT};
}}
QAbstractSpinBox {{ background: #1f2023; border: 1px solid #45474d; border-radius: 4px; padding-left: 4px; min-height: 22px;
    selection-background-color: {ACCENT}; }}
QAbstractSpinBox:focus {{ border-color: {ACCENT}; }}
QAbstractSpinBox::up-button, QAbstractSpinBox::down-button {{ width: 0px; border: none; }}
QAbstractSpinBox::up-arrow, QAbstractSpinBox::down-arrow {{ width: 0px; height: 0px; }}
QLineEdit:focus, QComboBox:focus {{ border-color: {ACCENT}; }}
QComboBox::drop-down {{ border: none; width: 18px; }}
QComboBox QAbstractItemView {{ background: #26272b; border: 1px solid #45474d; selection-background-color: {ACCENT}; }}
QPushButton {{ background: #3a3c42; border: 1px solid #4a4c53; border-radius: 4px; padding: 5px 10px; }}
QPushButton:hover {{ background: #45474e; }}
QPushButton:pressed {{ background: #2f3136; }}
QPushButton:checked {{ background: #24426f; border-color: {ACCENT}; }}
QPushButton[role="primary"] {{ background: {ACCENT}; border-color: {ACCENT}; color: white; }}
QPushButton[role="danger"] {{ background: #b3261e; border-color: #c62828; color: white; }}
QPushButton[role="warn"] {{ background: #8a6d1a; border-color: #a7841f; color: white; }}
QTableWidget, QListWidget, QTreeWidget {{ background: #232427; border: 1px solid #3a3c42; gridline-color: #34353a;
    selection-background-color: #24426f; alternate-background-color: #26272b; }}
QHeaderView::section {{ background: #2f3034; border: none; border-right: 1px solid #3a3c42; padding: 4px; color: #a8aab1; }}
QTabWidget::pane {{ border: 1px solid #3a3c42; top: -1px; }}
QTabBar::tab {{ background: #2b2c30; padding: 5px 10px; border: 1px solid #3a3c42; border-bottom: none; }}
QTabBar::tab:selected {{ background: #34353a; color: white; }}
QGroupBox {{ border: 1px solid #3a3c42; border-radius: 6px; margin-top: 10px; padding-top: 6px; }}
QGroupBox::title {{ subcontrol-origin: margin; left: 8px; padding: 0 4px; color: #a8aab1; }}
QSlider::groove:horizontal {{ height: 4px; background: #45474d; border-radius: 2px; }}
QSlider::handle:horizontal {{ background: {ACCENT}; width: 14px; margin: -6px 0; border-radius: 7px; }}
QSlider::sub-page:horizontal {{ background: {ACCENT}; border-radius: 2px; }}
QScrollArea {{ border: none; }}
QMenu {{ background: #2b2c30; border: 1px solid #45474d; }}
QMenu::item:selected {{ background: {ACCENT}; }}
QLabel[hint="1"] {{ color: #8e9097; font-size: 11px; }}
/* segmented control */
QFrame#seg {{ background: #1f2023; border: 1px solid #45474d; border-radius: 6px; }}
QFrame#seg QToolButton {{ border: none; border-radius: 4px; padding: 4px 8px; color: #b9bbc1; }}
QFrame#seg QToolButton:hover {{ background: #33353a; color: white; }}
QFrame#seg QToolButton:checked {{ background: {ACCENT}; color: white; }}
QFrame#seg[air="1"] QToolButton:checked {{ background: {AIR}; color: #0b1a1d; }}
"""


def apply_dark_theme(app):
    app.setStyle("Fusion")
    pal = QPalette()
    for role, col in [
        (QPalette.Window, "#2b2c30"), (QPalette.WindowText, "#dfe1e5"), (QPalette.Base, "#1f2023"),
        (QPalette.AlternateBase, "#26272b"), (QPalette.ToolTipBase, "#1f2023"), (QPalette.ToolTipText, "#dfe1e5"),
        (QPalette.Text, "#dfe1e5"), (QPalette.Button, "#3a3c42"), (QPalette.ButtonText, "#dfe1e5"),
        (QPalette.Highlight, ACCENT), (QPalette.HighlightedText, "#ffffff"), (QPalette.PlaceholderText, "#777980"),
    ]:
        pal.setColor(role, QColor(col))
    pal.setColor(QPalette.Disabled, QPalette.Text, QColor("#6b6d74"))
    pal.setColor(QPalette.Disabled, QPalette.ButtonText, QColor("#6b6d74"))
    pal.setColor(QPalette.Disabled, QPalette.WindowText, QColor("#6b6d74"))
    app.setPalette(pal)
    app.setStyleSheet(STYLE)


_icon_cache: dict = {}
ICON_COLOR = "#dfe1e5"


def svg_icon(name: str, color: str = None, size: int = 22) -> QIcon:
    color = color or ICON_COLOR
    key = (name, color, size)
    if key in _icon_cache:
        return _icon_cache[key]
    body = ICON_PATHS.get(name, "")
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{color}" color="{color}" '
           f'stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{body}</svg>').replace("currentColor", color)
    renderer = QSvgRenderer(QByteArray(svg.encode()))
    icon = QIcon()
    for scale in (1, 2):
        pm = QPixmap(size * scale, size * scale)
        pm.fill(Qt.transparent)
        p = QPainter(pm)
        renderer.render(p, QRectF(0, 0, size * scale, size * scale))
        p.end()
        pm.setDevicePixelRatio(scale)
        icon.addPixmap(pm)
    _icon_cache[key] = icon
    return icon


class Segmented(QFrame):
    """Segmented control (like macOS NSSegmentedControl) replacing combo boxes for 2-4 choices."""
    changed = Signal(object)

    def __init__(self, options, air=False, parent=None):
        super().__init__(parent)
        self.setObjectName("seg")
        if air:
            self.setProperty("air", "1")
        lay = QHBoxLayout(self)
        lay.setContentsMargins(2, 2, 2, 2)
        lay.setSpacing(2)
        self.group = QButtonGroup(self)
        self.group.setExclusive(True)
        self.buttons = {}
        for value, label in options:
            b = QToolButton()
            b.setText(label)
            b.setCheckable(True)
            b.setToolButtonStyle(Qt.ToolButtonTextOnly)
            b.setSizePolicy(b.sizePolicy().horizontalPolicy().Expanding, b.sizePolicy().verticalPolicy())
            b.clicked.connect(lambda _=False, v=value: self._clicked(v))
            self.group.addButton(b)
            lay.addWidget(b, 1)
            self.buttons[value] = b
        self._value = None

    def _clicked(self, v):
        if v != self._value:
            self._value = v
            self.changed.emit(v)

    def value(self):
        return self._value

    def setValue(self, v):
        self._value = v
        for k, b in self.buttons.items():
            b.setChecked(k == v)

    def setLabels(self, labels: dict):
        for k, txt in labels.items():
            if k in self.buttons:
                self.buttons[k].setText(txt)


def dspin(value=0.0, lo=-1e6, hi=1e6, step=1.0, decimals=2, suffix="", width=None):
    w = QDoubleSpinBox()
    w.setRange(lo, hi)
    w.setDecimals(decimals)
    w.setSingleStep(step)
    w.setValue(value)
    w.setKeyboardTracking(False)
    if suffix:
        w.setSuffix(suffix)
    if width:
        w.setFixedWidth(width)
    return w


def ispin(value=0, lo=0, hi=100000, step=1, suffix="", width=None):
    w = QSpinBox()
    w.setRange(lo, hi)
    w.setSingleStep(step)
    w.setValue(value)
    w.setKeyboardTracking(False)
    if suffix:
        w.setSuffix(suffix)
    if width:
        w.setFixedWidth(width)
    return w


def hint(text):
    lab = QLabel(text)
    lab.setProperty("hint", "1")
    lab.setWordWrap(True)
    return lab


def checkbox(text, checked=False):
    c = QCheckBox(text)
    c.setChecked(checked)
    return c


def form(*rows):
    w = QWidget()
    f = QFormLayout(w)
    f.setContentsMargins(0, 0, 0, 0)
    f.setLabelAlignment(Qt.AlignRight | Qt.AlignVCenter)
    for label, widget in rows:
        f.addRow(label, widget)
    return w


ICON_SIZE = QSize(20, 20)
