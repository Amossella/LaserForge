"""LaserForge — native desktop laser software (Qt / PySide6)."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from PySide6.QtCore import QLibraryInfo, QSettings, Qt, QTranslator
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QApplication

from laserforge.mainwindow import MainWindow
from laserforge.ui_common import apply_dark_theme


def set_bundle_name():
    """Name the macOS app menu 'LaserForge' instead of 'Python' when launched through the interpreter."""
    try:
        from Foundation import NSBundle
        info = NSBundle.mainBundle().localizedInfoDictionary() or NSBundle.mainBundle().infoDictionary()
        info["CFBundleName"] = "LaserForge"
    except Exception:
        pass


def install_translations(app, lang):
    """Qt's own strings (app menu, standard dialogs) in the chosen language."""
    tr = QTranslator(app)
    if tr.load(f"qtbase_{lang}", QLibraryInfo.path(QLibraryInfo.TranslationsPath)):
        app.installTranslator(tr)
    return tr


def apply_ui_scale():
    """Interface scale from Settings (needs to be set before the application starts)."""
    import json
    s = QSettings(os.environ["LASERFORGE_SETTINGS"], QSettings.IniFormat) if os.environ.get("LASERFORGE_SETTINGS") else QSettings("LaserForge", "LaserForge")
    try:
        scale = int(json.loads(s.value("prefs") or "{}").get("ui_scale", 100))
    except (TypeError, ValueError):
        scale = 100
    if scale != 100 and "QT_SCALE_FACTOR" not in os.environ:
        os.environ["QT_SCALE_FACTOR"] = f"{scale / 100:.2f}"


def main():
    set_bundle_name()
    apply_ui_scale()
    QApplication.setHighDpiScaleFactorRoundingPolicy(Qt.HighDpiScaleFactorRoundingPolicy.PassThrough)
    app = QApplication(sys.argv)
    lang = (QSettings(os.environ["LASERFORGE_SETTINGS"], QSettings.IniFormat) if os.environ.get("LASERFORGE_SETTINGS")
            else QSettings("LaserForge", "LaserForge")).value("lang", "it")
    app._qt_tr = install_translations(app, lang)
    app.setApplicationName("LaserForge")
    app.setApplicationDisplayName("LaserForge")
    app.setOrganizationName("LaserForge")
    icon = os.path.join(os.path.dirname(os.path.abspath(__file__)), "LaserForge.app", "Contents", "Resources", "LaserForge.icns")
    if os.path.exists(icon):
        app.setWindowIcon(QIcon(icon))
    apply_dark_theme(app)
    win = MainWindow()
    win.show()
    for arg in sys.argv[1:]:
        if os.path.isfile(arg):
            win.open_dropped(arg)
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
