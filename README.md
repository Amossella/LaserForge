# LaserForge

> **⚠️ Beta (0.4.0-beta)** — LaserForge is under active development. Test every new feature on scrap
> material first, keep a hand near the emergency stop, and never leave the laser unattended.

LaserForge is a native macOS desktop application (Python + Qt / PySide6) for designing and running jobs
on **GRBL-family laser engravers and cutters** (GRBL, grblHAL, FluidNC, plus Marlin and Smoothieware),
with **native air assist** support.

The interface is in Italian and English.

## Features

**Design**
- Shapes, text (including single-line Hershey / SHX fonts), Bézier node editing, booleans with a preview
  assistant, offset, weld, fillet, dogbone / T-bone, arrays, true-shape nesting, barcodes and QR codes
- Images: dithering (Floyd–Steinberg, Jarvis, Stucki, Atkinson, Sierra, Bayer, halftone, newsprint,
  sketch, grayscale, 3D slices), brightness / contrast / gamma / sharpening, Pass-Through, masks
- Import: SVG, DXF, PDF, PNG/JPG, Gerber / Excellon, STL / OBJ / 3MF (as depth maps)
- 30 colour layers plus T1 / T2 tool layers (never sent to the laser); shapes outside the work area are
  ignored or clipped at the edge

**Cutting and engraving**
- Line, fill, offset fill, line+fill; flood fill, cross-hatch, bidirectional scanning, overscan
- Lead-in / lead-out, overcut, perforation, tabs, kerf, ramps, sub-layers
- Air assist M8 (or M7), per layer, optionally only while cutting
- Dot width correction, fast whitespace scan, scanning offset adjustment, backlash compensation

**Machine control (GRBL)**
- Buffered (character-counting) or synchronous streaming, real-time speed / power overrides
- Frame (rectangle, rubber-band, continuous), start from absolute / current position / user origin
- Machine Settings editor (`$$` read / write / save / load) with checks for laser mode (`$32`), homing
  and soft limits; auto-home on connect
- Calibrate axis, Fire button, Z axis, rotary on Y or A, work zero (`G10 L20`), saved positions
- USB serial, TCP (Wi-Fi / Ethernet) and Bonjour discovery of network controllers
- Job time estimate with acceleration; preview with "start from here"

## Requirements

- macOS (Apple Silicon or Intel)
- Python 3.11 or newer

## Run from source

```bash
git clone https://github.com/<you>/LaserForge.git
cd LaserForge
python3 -m pip install -r requirements.txt
python3 main.py
```

## Build the macOS app

```bash
python3 build_app.py
```

This creates `LaserForge.app`, a launcher that runs `main.py` with your Python.

## Tests

```bash
QT_QPA_PLATFORM=offscreen python3 tests/selftest.py
```

The self-test drives the real window offscreen and talks to an emulated GRBL controller
(`tests/fake_grbl.py`) over a pseudo-terminal.

## Credits

- App icon: "CNC Machine" by [Icons8](https://icons8.com). The icon (`assets/icon.png`) is Icons8's work
  and is **not** covered by the GPL licence of this repository.

LaserForge is an independent project. It is not affiliated with, endorsed by or connected to LightBurn
Software or any laser manufacturer. Product names are used only to describe compatibility.

## Licence

LaserForge is free software, released under the [GNU General Public License v3.0](LICENSE).
