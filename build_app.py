#!/usr/bin/env python3
"""Build LaserForge.app, a double-clickable macOS launcher for main.py (native Qt app)."""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent
APP = ROOT / "LaserForge.app"


def make_icon(dest: Path):
    size = 1024
    custom = ROOT / "assets" / "icon.png"      # chosen app icon (CNC machine, Icons8 "liquid glass")
    if custom.exists():
        img = Image.open(custom).convert("RGBA").resize((size, size), Image.LANCZOS)
        _write_icns(img, dest)
        return
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((40, 40, size - 40, size - 40), radius=200, fill=(31, 32, 35, 255))
    d.rectangle((250, 250, 774, 774), outline=(233, 69, 96, 255), width=36)
    d.line((512, 120, 512, 512), fill=(255, 60, 60, 255), width=28)
    d.ellipse((462, 462, 562, 562), fill=(255, 40, 40, 255))
    d.ellipse((440, 440, 584, 584), outline=(255, 255, 255, 200), width=10)
    _write_icns(img, dest)


def _write_icns(img, dest: Path):
    with tempfile.TemporaryDirectory() as tmp:
        iconset = Path(tmp) / "LaserForge.iconset"
        iconset.mkdir()
        for s in (16, 32, 128, 256, 512):
            img.resize((s, s), Image.LANCZOS).save(iconset / f"icon_{s}x{s}.png")
            img.resize((s * 2, s * 2), Image.LANCZOS).save(iconset / f"icon_{s}x{s}@2x.png")
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(dest)], check=True)


def main():
    if APP.exists():
        shutil.rmtree(APP)
    macos = APP / "Contents" / "MacOS"
    res = APP / "Contents" / "Resources"
    macos.mkdir(parents=True)
    res.mkdir(parents=True)

    launcher = macos / "LaserForge"
    launcher.write_text(
        "#!/bin/bash\n"
        f'exec "{sys.executable}" "{ROOT / "main.py"}" "$@"\n'
    )
    os.chmod(launcher, 0o755)

    (APP / "Contents" / "Info.plist").write_text("""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>LaserForge</string>
    <key>CFBundleDisplayName</key><string>LaserForge</string>
    <key>CFBundleIdentifier</key><string>local.laserforge.app</string>
    <key>CFBundleVersion</key><string>0.3</string>
    <key>CFBundleShortVersionString</key><string>0.3</string>
    <key>CFBundleExecutable</key><string>LaserForge</string>
    <key>CFBundleIconFile</key><string>LaserForge</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSCameraUsageDescription</key><string>LaserForge usa la camera per mostrare il materiale sull'area di lavoro.</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>CFBundleDocumentTypes</key>
    <array><dict>
        <key>CFBundleTypeName</key><string>LaserForge Project</string>
        <key>CFBundleTypeExtensions</key><array><string>lfproj</string></array>
        <key>CFBundleTypeRole</key><string>Editor</string>
    </dict></array>
</dict>
</plist>
""")
    make_icon(res / "LaserForge.icns")
    print(f"Creato {APP}")


if __name__ == "__main__":
    main()
