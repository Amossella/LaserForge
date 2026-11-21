# Porting status (Python → Rust)

How to resume: read this file, then `PORTING-symbols.md` (every Python symbol, from the graphify graph).
Verification: `cargo test` runs the Python-parity suites (fixtures from the Python app of v0.4.0-beta).

Legend: `[x]` ported and verified against Python · `[~]` ported, not verified / approximate · `[ ]` missing · `[-]` not ported on purpose

## Engine and data

- [x] G-code generation, post-processing, GRBL protocol (phases 1–2: golden + phase2 tests)
- [x] `.lfproj` open/save, same JSON as Python (project_parity round trip)
- [x] build_job: clipping, start points, tabs, order layer/group/priority, direction, current/user origin, origin from selection, cylinder, print & cut, image crop, 3D slices (24 projects vs Python, project_parity)
- [x] Import SVG, DXF, PDF/AI (vectors + images), bitmaps (importers_parity)
- [x] Text: system fonts (ttf-parser, kerning), Hershey and SHX single-line fonts, letter spacing, bend, variable text (importers_parity text_*)
- [x] Edit / Arrange commands: 44 cases vs Python (edit_parity)
- [x] Test patterns, barcodes, QR (same structure; mask may differ), fillet, dogbone/T-bone, slots, tangent circles, box maker, overlap removal, image trace, PCB, 3D depth map (tools_parity)
- [~] removeOverlap option inside build_job (applied per layer bucket with the same segment merge as Edit › Delete overlaps; not compared with Python)
- [~] True-shape nesting (bit-row search instead of FFT; same placement rule, not compared with Python)
- [~] Image rotation by any angle (bicubic, white fill), crop flatten: not compared pixel by pixel
- [~] 3MF and zipped AMF models

## Interface

- [x] Menus, tool bars, tool box, palette, status bar from the Python tables (defs.json)
- [x] Canvas: zoom/pan, grid, rulers, shapes, filled view, images (dithered preview), crop path, selection handles (move/resize), marquee, snapping (grid + objects), centre snap, nudging
- [x] Tools: select, nodes, line/poly, rect, ellipse, polygon, star, text, pan, measure, position, scissors, trim, tabs, start point, two-point, deform, warp
- [x] Panels: cuts/layers + full layer editor, laser, move (jog, positions), console, objects, properties (incl. live arrays), pages, macros, material library, art library
- [x] Dialogs: see the action list below (one per menu entry)
- [x] Devices, checklist, backups, preference bundles (.lfbundle, same zip layout), phone remote (HTTP + PIN), keyboard shortcuts editor
- [~] Camera panel and camera dialogs (maths verified against Python; live capture not tried on hardware)
- [x] Connection extras: SIM simulator port, GRBL error/alarm explanations in the console, jog cancel, controller families (Marlin / LinuxCNC: no reset or `?` polling, synchronous streaming, own home/jog/override commands) (tests/connection.rs)
- [~] Settings dialog with every Python preference (interface text and scale at the next start, grid, curve quality, backups kept, cut selected, checklist, start-from / job origin at startup, selection origin, camera quality, restore defaults)
- [~] Out-of-area warning offers “Use Absolute”; Home warns when GRBL homing is off ($22=0); notice when an image is reduced to the raster cap
- [~] Rust-only interface changes: menus in the macOS menu bar; property bar (position / size / scale % around a 3×3 reference point, text font options); two docked panel groups with tabs at the bottom; layer settings in a window (double-click a layer); preview played in real time with elapsed / remaining time; job progress window
- [~] Bonjour search for network lasers (not tried on a real network); device selector and USB detect in the Laser panel; zoom % with 100 % = true size from the display's physical size (macOS)

## Menu actions (from laserforge/defs.py)

### File
- [x] `new` Nuovo
- [x] `open` Apri...
- [x] `openBackup` Apri backup...
- [x] `import` Importa...
- [x] `save` Salva
- [x] `saveAs` Salva con nome...
- [x] `exportSvg` Esporta SVG...
- [x] `saveGcode` Salva G-code...
- [x] `projectNotes` Note del progetto...
- Preferenze
  - [x] `exportPrefs` Esporta preferenze...
  - [x] `importPrefs` Importa preferenze...
  - [x] `exportBundle` Esporta pacchetto completo...
  - [x] `importBundle` Importa pacchetto completo...
- [x] `exit` Esci

### Modifica
- [x] `undo` Annulla
- [x] `redo` Ripeti
- [x] `undoHistory` Cronologia annulla...
- [x] `selectAll` Seleziona tutto
- [x] `invertSelection` Inverti selezione
- [x] `selectOpen` Seleziona forme aperte
- [x] `selectContained` Seleziona forme contenute
- [x] `selectNested` Seleziona forme annidate
- [x] `selectCircles` Seleziona cerchi uguali
- [x] `selectSmaller` Seleziona forme più piccole della selezione
- [x] `selectLayer` Seleziona tutto nel layer
- [x] `cut` Taglia
- [x] `copy` Copia
- [x] `duplicate` Duplica
- [x] `paste` Incolla
- [x] `pasteInPlace` Incolla sul posto
- [x] `delete` Elimina
- [x] `deleteDuplicates` Elimina forme duplicate
- [x] `closePath` Chiudi percorso
- [x] `closeTolerance` Chiudi forme con tolleranza...
- [x] `deleteOverlaps` Elimina linee sovrapposte
- [x] `autoJoin` Unisci automaticamente forme
- [x] `breakApart` Spezza in forme separate
- [x] `convertToPath` Converti in tracciato
- [x] `lockSel` Blocca forme selezionate
- [x] `unlockSel` Sblocca forme selezionate
- [x] `settings` Impostazioni...
- [x] `editHotkeys` Scorciatoie da tastiera...
- [x] `deviceSettings` Impostazioni dispositivo
- [x] `devices` Dispositivi...
- [x] `newDevice` Nuovo dispositivo (procedura guidata)...

### Strumenti
- [x] `toolSelect` Seleziona
- [x] `toolNodes` Modifica nodi
- [x] `toolPoly` Disegna linee
- [x] `toolRect` Rettangolo
- [x] `toolEllipse` Ellisse
- [x] `toolPolygon` Poligono
- [x] `toolText` Testo
- [x] `textOnPath` Testo su percorso...
- [x] `varText` Testo variabile...
- Disegna forma
  - [x] `shapeTriangle` Triangolo
  - [x] `shapePentagon` Pentagono
  - [x] `shapeOctagon` Ottagono
  - [x] `shapeStar` Stella
  - [x] `shapeDualStar` Stella doppia
- [x] `offset` Offset forme...
- [x] `weld` Salda forme
- Strumenti booleani
  - [x] `union` Unione
  - [x] `subtract` Sottrai
  - [x] `intersect` Intersezione
  - [x] `booleanAssistant` Assistente booleano...
- [x] `roundCorners` Arrotonda angoli...
- Modifica forme
  - [x] `toolScissors` Forbici (taglia in un punto)
  - [x] `toolTrim` Taglia linee alle intersezioni
  - [x] `fillet` Raccorda angoli selezionati...
  - [x] `dogbone` Dogbone / T-bone...
  - [x] `slotResize` Ridimensiona fessure...
  - [x] `toolTwoPoint` Ruota e scala a 2 punti
  - [x] `toolDeform` Deforma (4 angoli)
  - [x] `toolWarp` Curva (griglia 3×3)
  - [x] `taper` Rastrema...
  - [x] `tangentCircles` Cerchi tangenti a 3 cerchi
- [x] `rubberBand` Contorno elastico dalla selezione
- [x] `gridArray` Copia a griglia
- [x] `circularArray` Copia circolare
- [x] `copyAlongPath` Copia lungo percorso...
- [x] `mirrorLine` Specchia lungo una linea
- [x] `createQR` Crea codice QR...
- [x] `createBarcode` Crea codice a barre...
- [x] `measure` Misura
- [x] `traceImage` Traccia immagine
- [x] `adjustImage` Regola immagine
- [x] `cropImage` Applica ritaglio all'immagine
- [x] `applyMask` Applica maschera all'immagine
- [x] `removeCrop` Rimuovi ritaglio dall'immagine
- [x] `flattenCrop` Appiattisci ritaglio dell'immagine
- [x] `convertBitmap` Converti in bitmap...
- [x] `replaceImage` Sostituisci immagine...
- [x] `replaceImageFit` Sostituisci immagine adattando...
- [x] `reloadImage` Ricarica immagine dal file

### Disponi
- [x] `group` Raggruppa
- [x] `ungroup` Separa
- [x] `flattenArray` Appiattisci array dinamico
- [x] `mirrorH` Specchia orizzontalmente
- [x] `mirrorV` Specchia verticalmente
- [x] `rotate90cw` Ruota 90° orario
- [x] `rotate90ccw` Ruota 90° antiorario
- Allinea
  - [x] `alignLeft` Bordi sinistri
  - [x] `alignRight` Bordi destri
  - [x] `alignTop` Bordi superiori
  - [x] `alignBottom` Bordi inferiori
  - [x] `alignCenterH` Centri orizzontali
  - [x] `alignCenterV` Centri verticali
  - [x] `alignCenters` Centri
- Distribuisci
  - [x] `distH` Orizzontalmente (spaziatura)
  - [x] `distV` Verticalmente (spaziatura)
  - [x] `distHC` Centri orizzontali
  - [x] `distVC` Centri verticali
- Sposta oggetti selezionati
  - [x] `moveUL` In alto a sinistra
  - [x] `moveUR` In alto a destra
  - [x] `moveLL` In basso a sinistra
  - [x] `moveLR` In basso a destra
  - [x] `moveCenter` Centro pagina
  - [x] `moveLeft` Bordo sinistro
  - [x] `moveRight` Bordo destro
  - [x] `moveTop` Bordo superiore
  - [x] `moveBottom` Bordo inferiore
  - [x] `moveToLaser` Posizione del laser
- [x] `sameWidth` Stessa larghezza
- [x] `sameHeight` Stessa altezza
- [x] `nesting` Disponi automaticamente (nesting)
- [x] `pushFront` Porta in primo piano
- [x] `pushForward` Porta avanti
- [x] `pushBackward` Porta indietro
- [x] `pushBack` Porta in fondo
- [x] `lockAspect` Blocca proporzioni

### Strumenti laser
- [x] `materialTest` Test materiale...
- [x] `intervalTest` Test intervallo...
- [x] `focusTest` Test di messa a fuoco...
- Sposta laser alla selezione
  - [x] `laserUL` In alto a sinistra
  - [x] `laserUR` In alto a destra
  - [x] `laserLL` In basso a sinistra
  - [x] `laserLR` In basso a destra
  - [x] `laserCenter` Centro
- [x] `frame` Traccia contorno (frame)
- [x] `frameRubber` Frame elastico (contorno, laser spento)
- [x] `frameContinuous` Frame continuo
- [x] `homeLaser` Home
- [x] `unlock` Sblocca allarme ($X)
- [x] `gotoOrigin` Vai all'origine
- [x] `setUserOrigin` Imposta origine utente qui
- [x] `optimizeSettings` Impostazioni ottimizzazione...
- [x] `toolTabs` Ponticelli manuali
- [x] `clearTabs` Rimuovi ponticelli dalla selezione
- [x] `toolStartPoints` Punti di inizio e direzione
- [x] `resetStart` Ripristina punti di inizio
- [x] `airAllOn` Attiva aria su tutti i layer
- [x] `airAllOff` Disattiva aria su tutti i layer
- [x] `airInvert` Inverti aria su tutti i layer
- [x] `layersAllOn` Abilita tutti i layer
- [x] `layersAllOff` Disabilita tutti i layer
- [x] `layersInvert` Inverti layer abilitati
- [x] `rotarySetup` Configurazione rotativo...
- [x] `cylinder` Correzione cilindrica...
- [x] `feeder` Alimentatore automatico...
- [x] `centerFinder` Trova centro...
- [x] `printAndCut` Stampa e taglia...
- [x] `checklist` Checklist prima del lavoro...
- [x] `cameraCapture` Aggiorna immagine camera
- [x] `cameraAlign` Allinea camera...
- [x] `cameraTargets` Allinea camera con segni incisi...
- [x] `cameraLens` Correzione lente camera...
- [x] `cameraImport` Importa immagine camera nel progetto
- [x] `machineSettings` Impostazioni macchina (GRBL $$)
- [x] `calibrateAxis` Calibra asse...
- [x] `controllerInfo` Info controller ($I)
- [x] `controllerFiles` File sul controller (SD)...

### Strumenti web
- [x] `boxMaker` Generatore scatole a incastro...
- [x] `gearMaker` Generatore ingranaggi...
- [x] `docs` Documentazione LaserForge (online)

### Finestra
- [x] `winCuts` Tagli / Layer
- [x] `winLaser` Laser
- [x] `winMove` Sposta
- [x] `winConsole` Console
- [x] `winObjects` Oggetti
- [x] `winLibrary` Libreria materiali
- [x] `winArt` Libreria grafica
- [x] `winCamera` Controllo camera
- [x] `winProps` Proprietà forma
- [x] `winPages` Pagine
- [x] `winMacros` Macro
- [x] `winMainToolbar` Barra principale
- [x] `winModbar` Barra di modifica
- [x] `winToolbox` Barra strumenti
- [x] `winPalette` Palette colori
- [x] `resetLayout` Ripristina disposizione finestre
- [x] `viewFilled` Vista piena (riempi forme)
- [x] `showGrid` Mostra griglia
- [x] `showRulers` Mostra righelli
- [x] `zoomIn` Zoom avanti
- [x] `zoomOut` Zoom indietro
- [x] `zoomPage` Adatta alla pagina
- [x] `zoomSel` Adatta alla selezione
- [x] `minimize` Riduci a icona
- [x] `fullscreen` Schermo intero
- [x] `resetLayout` Ripristina layout predefinito

### Lingua
- [x] `langIt` Italiano
- [x] `langEn` English
- [x] `langOther` Altre lingue (in arrivo)

### Aiuto
- [x] `shortcuts` Scorciatoie da tastiera
- [x] `docs` Documentazione LaserForge (online)
- [x] `about` Informazioni su LaserForge

### Extra (toolbar)
- [x] `toolPan` Sposta vista
- [x] `toolStar` Stella
- [x] `toolPosition` Posiziona laser (clic)
- [x] `preview` Anteprima
- [x] `startLaser` Avvia
- [x] `pauseLaser` Pausa
- [x] `stopLaser` Stop
- [x] `resumeLaser` Riprendi

## Panels

- [~] Tagli/Livelli (cuts) with full layer editor
- [~] Laser
- [~] Movimento (move/jog)
- [~] Console
- [~] Oggetti
- [~] Libreria materiali
- [~] Libreria arte
- [~] Camera
- [~] Proprietà forma
- [~] Pagine
- [~] Macro

## Not verifiable here (implemented, not exercised)

- Mouse-driven canvas behaviour, real controllers, USB reconnect, camera
