# LaserForge

> **⚠️ Beta (0.5.0-beta)** — LaserForge is under active development. Test every new feature on scrap
> material first, keep a hand near the emergency stop, and never leave the laser unattended.

LaserForge is a native macOS desktop application for designing and running jobs on **GRBL-family laser
engravers and cutters** (GRBL, grblHAL, FluidNC, plus Marlin and Smoothieware), with **native air assist**
support.

From 0.5.0 LaserForge is written in **Rust** (egui). The earlier Python / Qt version is in the git
history and under the [v0.4.0-beta tag](https://github.com/Amossella/LaserForge/tree/v0.4.0-beta);
projects (`.lfproj`), the material library and the art library are shared with it.

The interface is in Italian and English.

## Download

Get the zip from the [Releases](https://github.com/Amossella/LaserForge/releases) page, unzip it and move
`LaserForge.app` to Applications. The app is not notarised by Apple: the first time, right-click it and
choose **Open**.

## Features

**Design**
- Shapes, text (including single-line Hershey / SHX fonts), Bézier node editing, booleans with a preview
  assistant, offset, weld, fillet, dogbone / T-bone, arrays, true-shape nesting, barcodes and QR codes
- Images: dithering (Floyd–Steinberg, Jarvis, Stucki, Atkinson, Sierra, Bayer, halftone, newsprint,
  sketch, grayscale, 3D slices), brightness / contrast / gamma / sharpening, Pass-Through, masks
- Import: SVG, DXF, PDF, PNG/JPG, Gerber / Excellon, STL / OBJ / 3MF / AMF (as depth maps)
- 30 colour layers plus T1 / T2 tool layers (never sent to the laser); layer settings open with a
  double-click on the layer
- Property bar with position, size and scale around a chosen reference point

**Cutting and engraving**
- Line, fill, offset fill, line+fill; flood fill, cross-hatch, bidirectional scanning, overscan
- Lead-in / lead-out, overcut, perforation, tabs, kerf, ramps, sub-layers
- Air assist M8 (or M7), per layer, optionally only while cutting
- Dot width correction, fast whitespace scan, scanning offset adjustment, backlash compensation

**Machine control**
- Buffered (character-counting) or synchronous streaming, real-time speed / power overrides
- Job progress with percentage, elapsed and remaining time in the Laser panel
- Frame (rectangle, rubber-band, continuous), start from absolute / current position / user origin
- Machine Settings editor (`$$` read / write / save / load) with checks for laser mode (`$32`), homing
  and soft limits; GRBL errors and alarms explained in the console
- Calibrate axis, Fire button, Z axis, rotary on Y or A, work zero (`G10 L20`), saved positions
- USB serial, TCP (Wi-Fi / Ethernet), Bonjour discovery of network controllers, USB auto-detect
- Built-in GRBL simulator (port **SIM**) to try everything without a laser
- Preview played in real time with elapsed / remaining time and "start from here"
- Camera overlay with lens correction and alignment; phone / tablet remote on the local network

## Build from source

Needs [Rust](https://rustup.rs) (stable).

```bash
git clone https://github.com/Amossella/LaserForge.git
cd LaserForge
cargo run --release -p laserforge-app
```

Build the macOS app and the release zip:

```bash
tools/make_app.sh
```

## Tests

```bash
cargo test --release
```

The test suites compare the Rust code with fixtures produced by the former Python app (G-code byte for
byte, imports, editing commands, tools, camera maths) and stream jobs to an emulated GRBL controller
(`tools/fake_grbl.py`, needs Python 3) over a pseudo-terminal and TCP. `PORTING.md` records what was ported
and how it was checked.

Test modes for the app: `LF_DEMO=1` (demo drawing), `LF_OPEN=file.lfproj`, `LF_CONNECT=port`
(`LF_CONNECT=SIM` for the simulator), `LF_RUN=action` (run a menu action at startup),
`LF_SCREENSHOT=out.png` (capture the window and quit). None of them saves settings.

## Command line

```bash
./target/release/laserforge gcode job.json -o job.gcode
./target/release/laserforge estimate job.json
```

`job.json` holds `machine` (same keys as the device settings), `layers` (`settings` + `paths` in machine
coordinates, optional `tabs`), `rasters` (`settings` + `x y w h cols rows data`), and optionally
`origin_mode` (`"current"` / `"user"`) with `start`.

## Credits

- App icon: "CNC Machine" by [Icons8](https://icons8.com). The icon (`assets/icon.png`) is Icons8's work
  and is **not** covered by the GPL licence of this repository.
- Hershey font data from the Hershey-Fonts package (MIT, `laserforge-app/assets/hershey-LICENSE.txt`).

LaserForge is an independent project. It is not affiliated with, endorsed by or connected to LightBurn
Software or any laser manufacturer. Product names are used only to describe compatibility.

## Licence

LaserForge is free software, released under the [GNU General Public License v3.0](LICENSE).
Dependencies (egui/eframe, serialport, geo, image, rfd, quick-xml, flate2, ttf-parser, qrcode, zip, muda,
nokhwa and others) are under MIT, Apache-2.0, BSD, ISC, Zlib, Unlicense or MPL-2.0 licences, all compatible
with GPL-3.0.
