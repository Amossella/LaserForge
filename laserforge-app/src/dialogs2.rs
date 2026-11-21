//! More dialogs, each its own type implementing `Extra`: laser tests, centre finder, print & cut, generators
//! (box, gear, QR, barcode), text on path, variable text, corner tools, devices, checklist, feeder, GRBL
//! settings, axis calibration, controller files, shortcuts editor, boolean assistant, backups and bundles.
//! Ported from laserforge/dialogs.py and devices.py.

use crate::app::LaserApp;
use crate::corners;
use crate::edit::center;
use crate::shapes::{multi_bounds, Shape, P};
use crate::{l, tr};
use egui::Color32;
use serde_json::{json, Map, Value};

pub trait Extra {
    fn title(&self) -> String;
    /// Draw; Some(true) = OK, Some(false) = cancel / close.
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool>;
    /// Apply on OK; true to close.
    fn apply(&mut self, app: &mut LaserApp) -> bool;
}

fn buttons(ui: &mut egui::Ui, ok: &str) -> Option<bool> {
    ui.separator();
    let mut r = None;
    ui.horizontal(|ui| {
        if ui.button(egui::RichText::new(ok).strong()).clicked() {
            r = Some(true);
        }
        if ui.button(tr("cancel")).clicked() {
            r = Some(false);
        }
    });
    r
}

fn close_button(ui: &mut egui::Ui) -> Option<bool> {
    ui.separator();
    ui.button(tr("close")).clicked().then_some(false)
}

fn hint(ui: &mut egui::Ui, t: &str) {
    ui.add(egui::Label::new(egui::RichText::new(t).weak().size(11.0)).wrap());
}

fn grid(ui: &mut egui::Ui, id: &str, f: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([10.0, 6.0]).show(ui, f);
}

fn dv(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64) {
    ui.label(label);
    ui.add(egui::DragValue::new(v).range(range).speed(speed).max_decimals(3));
    ui.end_row();
}

fn iv(ui: &mut egui::Ui, label: &str, v: &mut i64, range: std::ops::RangeInclusive<i64>) {
    ui.label(label);
    ui.add(egui::DragValue::new(v).range(range));
    ui.end_row();
}

fn props(v: Value) -> Map<String, Value> {
    serde_json::from_value(v).unwrap_or_default()
}

/// Add a group of shapes made of child objects; switches the layer to Fill when asked.
fn add_group(app: &mut LaserApp, kids: Vec<Value>, name: Option<String>, fill: bool) {
    let mut p = Map::new();
    p.insert("children".into(), json!(kids));
    if let Some(n) = name {
        p.insert("name".into(), json!(n));
    }
    let g = app.doc.create("group", p);
    app.doc.add_shapes(vec![g], true);
    let a = app.doc.active_layer;
    if fill && app.doc.layers[a].cut_mode == laserforge_core::model::CutMode::Line {
        app.doc.layers[a].cut_mode = laserforge_core::model::CutMode::Fill;
    }
    app.invalidate();
}

// ====================================================================== laser tests
struct Pattern {
    kind: &'static str,
    v: Map<String, Value>,
}

impl Extra for Pattern {
    fn title(&self) -> String {
        match self.kind {
            "material" => l("Test materiale", "Material Test"),
            "focus" => l("Test di messa a fuoco", "Focus Test"),
            _ => l("Test intervallo", "Interval Test"),
        }
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        let v = &mut self.v;
        let mut f = |ui: &mut egui::Ui, k: &str, label: &str, range: std::ops::RangeInclusive<f64>, speed: f64| {
            let mut x = v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
            dv(ui, label, &mut x, range, speed);
            v.insert(k.into(), json!(x));
        };
        match self.kind {
            "material" => {
                hint(ui, &l("Le righe variano la potenza, le colonne la velocità.", "Rows vary power, columns vary speed."));
                grid(ui, "mt", |ui| {
                    f(ui, "rows", &l("Righe (potenza)", "Rows (power)"), 1.0..=20.0, 1.0);
                    f(ui, "cols", &l("Colonne (velocità)", "Columns (speed)"), 1.0..=20.0, 1.0);
                    f(ui, "power_min", &l("Potenza min %", "Power min %"), 0.0..=100.0, 1.0);
                    f(ui, "power_max", &l("Potenza max %", "Power max %"), 0.0..=100.0, 1.0);
                    f(ui, "speed_min", &l("Velocità min", "Speed min"), 1.0..=100000.0, 10.0);
                    f(ui, "speed_max", &l("Velocità max", "Speed max"), 1.0..=100000.0, 10.0);
                    f(ui, "size", &l("Lato cella (mm)", "Cell size (mm)"), 1.0..=100.0, 0.5);
                    f(ui, "gap", &l("Spazio (mm)", "Gap (mm)"), 0.0..=50.0, 0.5);
                });
                let mut mode = v["mode"].as_str().unwrap_or("fill").to_string();
                let mut air = v["air_assist"].as_str().unwrap_or("on").to_string();
                ui.horizontal(|ui| {
                    ui.label(tr("mode"));
                    ui.selectable_value(&mut mode, "fill".into(), tr("modeFill"));
                    ui.selectable_value(&mut mode, "line".into(), tr("modeLine"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("airAssist"));
                    ui.selectable_value(&mut air, "on".into(), "On");
                    ui.selectable_value(&mut air, "off".into(), "Off");
                });
                v.insert("mode".into(), json!(mode));
                v.insert("air_assist".into(), json!(air));
            }
            "focus" => {
                hint(ui, &l("Incide una linea per ogni altezza Z: la linea più sottile indica il fuoco migliore.", "Burns one line per Z height. The thinnest line is the best focus."));
                grid(ui, "ft", |ui| {
                    f(ui, "count", &l("Linee", "Lines"), 2.0..=30.0, 1.0);
                    f(ui, "z_start", &l("Z iniziale (mm)", "Start Z (mm)"), -50.0..=50.0, 0.1);
                    f(ui, "z_step", &l("Passo Z (mm)", "Z step (mm)"), 0.05..=10.0, 0.05);
                    f(ui, "speed", &l("Velocità", "Speed"), 1.0..=100000.0, 10.0);
                    f(ui, "power", &l("Potenza %", "Power %"), 0.0..=100.0, 1.0);
                });
            }
            _ => {
                hint(ui, &l("Incide quadrati con intervallo di linea crescente.", "Engraves squares with increasing line interval."));
                grid(ui, "it", |ui| {
                    f(ui, "count", &l("Celle", "Cells"), 2.0..=20.0, 1.0);
                    f(ui, "interval_min", &l("Intervallo min", "Min interval"), 0.01..=2.0, 0.005);
                    f(ui, "interval_max", &l("Intervallo max", "Max interval"), 0.01..=2.0, 0.005);
                    f(ui, "speed", &l("Velocità", "Speed"), 1.0..=100000.0, 10.0);
                    f(ui, "power", &l("Potenza %", "Power %"), 0.0..=100.0, 1.0);
                });
            }
        }
        buttons(ui, &l("Genera", "Generate"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        let v = Value::Object(self.v.clone());
        let g = match self.kind {
            "material" => laserforge_core::patterns::material_test(&app.machine, &v),
            "focus" => laserforge_core::patterns::focus_test(&app.machine, &v),
            _ => laserforge_core::patterns::interval_test(&app.machine, &v),
        };
        let secs = laserforge_core::post::estimate_gcode_time(&g, &app.machine);
        app.gcode = Some(laserforge_core::Output { gcode: g.clone(), send: g, seconds: secs });
        app.gcode_origin = (None, None);
        if let Some(d) = crate::dialogs::Dialog::for_action("preview", app) {
            app.dialogs.push(d);
        }
        true
    }
}

// ====================================================================== centre finder
struct CenterFinder {
    pts: [Option<P>; 3],
    res: Option<[f64; 3]>,
}

impl Extra for CenterFinder {
    fn title(&self) -> String {
        l("Trova centro", "Center Finder")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        hint(ui, &l("Porta il laser su tre punti del bordo dell'oggetto rotondo e registra ciascun punto.", "Jog the laser to three points on the edge of the round object and record each one."));
        let head = app.head_canvas();
        egui::Grid::new("cf").num_columns(3).show(ui, |ui| {
            for i in 0..3 {
                ui.label(format!("{} {}", l("Punto", "Point"), i + 1));
                ui.monospace(self.pts[i].map_or("--".into(), |p| format!("X {:.3} Y {:.3}", p[0], p[1])));
                if ui.button(l("Usa posizione laser", "Use laser position")).clicked() {
                    match head {
                        Some(h) => self.pts[i] = Some(h),
                        None => app.say(l("Connetti prima il laser", "Connect a laser first")),
                    }
                }
                ui.end_row();
            }
        });
        if let [Some(a), Some(b), Some(c)] = self.pts {
            let dd = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
            if dd.abs() < 1e-9 {
                ui.label(l("I punti sono allineati", "Points are aligned"));
                self.res = None;
            } else {
                let sq = |p: P| p[0] * p[0] + p[1] * p[1];
                let ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / dd;
                let uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / dd;
                let r = (a[0] - ux).hypot(a[1] - uy);
                self.res = Some([ux, uy, r]);
                ui.label(format!("{}: X {ux:.3} Y {uy:.3} · Ø {:.2} mm", l("Centro", "Center"), r * 2.0));
            }
        }
        ui.horizontal(|ui| {
            if let Some([x, y, r]) = self.res {
                if ui.button(l("Sposta laser al centro", "Move laser to center")).clicked() {
                    app.goto_canvas([x, y]);
                }
                if ui.button(l("Disegna cerchio", "Draw circle")).clicked() {
                    let s = app.doc.create("circle", props(json!({"cx": x, "cy": y, "rx": r, "ry": r})));
                    app.doc.add_shapes(vec![s], true);
                    app.invalidate();
                }
            }
        });
        close_button(ui)
    }
    fn apply(&mut self, _app: &mut LaserApp) -> bool {
        true
    }
}

// ====================================================================== print and cut
struct PrintCut {
    src: Option<[P; 2]>,
    dst: [Option<P>; 2],
    enabled: bool,
}

impl Extra for PrintCut {
    fn title(&self) -> String {
        l("Stampa e taglia", "Print and Cut")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        hint(ui, &l("1) Seleziona due segni di registro nel disegno (il primo e l'ultimo selezionato). 2) Porta il laser su ciascun segno stampato e imposta i target. 3) Abilita l'allineamento: il lavoro viene spostato, ruotato e scalato per combaciare.",
                    "1) Select two registration marks in the design (first and last selected). 2) Jog the laser onto each printed mark and set the targets. 3) Enable alignment: the job is moved, rotated and scaled to match."));
        let fmt = |p: Option<P>| p.map_or("--".into(), |p| format!("X {:.2} Y {:.2}", p[0], p[1]));
        egui::Grid::new("pc").num_columns(3).show(ui, |ui| {
            ui.label(l("Segni nel disegno", "Design marks"));
            ui.label(self.src.map_or("--".into(), |s| format!("{} / {}", fmt(Some(s[0])), fmt(Some(s[1])))));
            if ui.button(l("Usa selezione", "Use selection")).clicked() {
                let sel = app.doc.selected_in_order();
                if sel.len() < 2 {
                    app.say(l("Seleziona due segni", "Select two marks"));
                } else {
                    self.src = Some([center(sel[0].bounds()), center(sel[sel.len() - 1].bounds())]);
                }
            }
            ui.end_row();
            for k in 0..2 {
                ui.label(if k == 0 { l("Primo target", "First target") } else { l("Secondo target", "Second target") });
                ui.label(fmt(self.dst[k]));
                if ui.button(l("Imposta dal laser", "Set from laser")).clicked() {
                    match app.head_canvas() {
                        Some(h) => self.dst[k] = Some(h),
                        None => app.say(l("Connetti prima il laser", "Connect a laser first")),
                    }
                }
                ui.end_row();
            }
        });
        ui.checkbox(&mut self.enabled, l("Allinea l'output ai target", "Align output to targets"));
        buttons(ui, "OK")
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.enabled && (self.src.is_none() || self.dst.iter().any(|d| d.is_none())) {
            app.say(l("Imposta prima i segni e entrambi i target", "Set the marks and both targets first"));
            return false;
        }
        app.print_cut = Some(json!({"enabled": self.enabled, "src": self.src, "dst": self.dst}));
        app.invalidate();
        true
    }
}

// ====================================================================== generators
struct BoxMaker {
    w: f64,
    d: f64,
    h: f64,
    t: f64,
    f: f64,
    lid: bool,
}

impl Extra for BoxMaker {
    fn title(&self) -> String {
        l("Generatore scatole a incastro", "Box Maker (finger joints)")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        grid(ui, "box", |ui| {
            dv(ui, &l("Larghezza (mm)", "Width (mm)"), &mut self.w, 10.0..=2000.0, 1.0);
            dv(ui, &l("Profondità (mm)", "Depth (mm)"), &mut self.d, 10.0..=2000.0, 1.0);
            dv(ui, &l("Altezza (mm)", "Height (mm)"), &mut self.h, 10.0..=2000.0, 1.0);
            dv(ui, &l("Spessore materiale", "Material thickness"), &mut self.t, 0.5..=30.0, 0.1);
            dv(ui, &l("Larghezza dente", "Finger width"), &mut self.f, 2.0..=200.0, 0.5);
        });
        ui.checkbox(&mut self.lid, l("Scatola chiusa (aggiungi coperchio)", "Closed box (add lid)"));
        buttons(ui, &tr("create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        let (w, d, h, t, f) = (self.w, self.d, self.h, self.t, self.f);
        let top = if self.lid { "in" } else { "flat" };
        let (g, x, y) = (6.0, 10.0, 10.0);
        let mut panels = vec![
            corners::box_panel(x, y, w, d, ["out"; 4], t, f),
            corners::box_panel(x, y + d + g, w, h, [top, "out", "in", "out"], t, f),
            corners::box_panel(x + w + g, y + d + g, w, h, [top, "out", "in", "out"], t, f),
            corners::box_panel(x, y + d + h + 2.0 * g, d, h, [top, "in", "in", "in"], t, f),
            corners::box_panel(x + d + g, y + d + h + 2.0 * g, d, h, [top, "in", "in", "in"], t, f),
        ];
        if self.lid {
            panels.push(corners::box_panel(x + w + g, y, w, d, ["out"; 4], t, f));
        }
        let layer = app.doc.active_layer;
        let kids: Vec<Value> = panels.into_iter().map(|p| json!({"id": app.doc.new_id(), "type": "poly", "layer": layer, "points": p, "closed": true})).collect();
        add_group(app, kids, None, false);
        true
    }
}

struct GearMaker {
    n: i64,
    module: f64,
    bore: f64,
}

impl Extra for GearMaker {
    fn title(&self) -> String {
        l("Generatore ingranaggi", "Gear Maker")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        grid(ui, "gear", |ui| {
            iv(ui, &l("Denti", "Teeth"), &mut self.n, 6..=200);
            dv(ui, &l("Modulo (mm)", "Module (mm)"), &mut self.module, 0.2..=20.0, 0.1);
            dv(ui, &l("Foro Ø (mm)", "Bore Ø (mm)"), &mut self.bore, 0.0..=200.0, 0.5);
        });
        buttons(ui, &tr("create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        let (w, h) = app.ws();
        let (cx, cy) = (w / 2.0, h / 2.0);
        let layer = app.doc.active_layer;
        let mut kids = vec![json!({"id": app.doc.new_id(), "type": "poly", "layer": layer, "points": corners::gear_points(cx, cy, self.n as usize, self.module), "closed": true})];
        if self.bore > 0.0 {
            kids.push(json!({"id": app.doc.new_id(), "type": "circle", "layer": layer, "cx": cx, "cy": cy, "rx": self.bore / 2.0, "ry": self.bore / 2.0}));
        }
        add_group(app, kids, None, false);
        true
    }
}

struct Qr {
    text: String,
    size: f64,
    err: String,
}

impl Extra for Qr {
    fn title(&self) -> String {
        l("Crea codice QR", "Create QR Code")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        grid(ui, "qr", |ui| {
            ui.label(l("Testo o URL", "Text or URL"));
            ui.add(egui::TextEdit::singleline(&mut self.text).hint_text("https://"));
            ui.end_row();
            dv(ui, &l("Dimensione (mm)", "Size (mm)"), &mut self.size, 5.0..=400.0, 0.5);
            ui.label(l("Correzione errori", "Error correction"));
            ui.horizontal(|ui| {
                for (k, t) in [("l", "L 7%"), ("m", "M 15%"), ("q", "Q 25%"), ("h", "H 30%")] {
                    ui.selectable_value(&mut self.err, k.to_string(), t);
                }
            });
            ui.end_row();
        });
        hint(ui, &l("Il layer attivo viene impostato su Riempi per incidere i moduli.", "The active layer is switched to Fill so the modules get engraved."));
        buttons(ui, &tr("create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.text.is_empty() {
            return false;
        }
        let m = match corners::qr_matrix(&self.text, &self.err) {
            Ok(m) => m,
            Err(e) => {
                app.say(e);
                return false;
            }
        };
        let n = m.len();
        let k = self.size / n as f64;
        let (w, h) = app.ws();
        let (x0, y0) = ((w - self.size) / 2.0, (h - self.size) / 2.0);
        let layer = app.doc.active_layer;
        let mut kids = Vec::new();
        for (r, row) in m.iter().enumerate() {
            let mut c = 0;
            while c < n {
                if !row[c] {
                    c += 1;
                    continue;
                }
                let start = c;
                while c < n && row[c] {
                    c += 1;
                }
                kids.push(json!({"id": app.doc.new_id(), "type": "rect", "layer": layer, "x": x0 + start as f64 * k, "y": y0 + r as f64 * k, "w": (c - start) as f64 * k, "h": k}));
            }
        }
        add_group(app, kids, None, true);
        true
    }
}

struct Barcode {
    kind: String,
    data: String,
    module: f64,
    height: f64,
    human: bool,
}

impl Extra for Barcode {
    fn title(&self) -> String {
        l("Crea codice a barre", "Create Barcode")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        grid(ui, "bc", |ui| {
            ui.label(l("Tipo", "Type"));
            egui::ComboBox::from_id_salt("bck").selected_text(self.kind.clone()).show_ui(ui, |ui| {
                for (k, t) in [("code128", "Code 128"), ("ean13", "EAN-13"), ("ean8", "EAN-8"), ("upca", "UPC-A"), ("qr", "QR")] {
                    ui.selectable_value(&mut self.kind, k.to_string(), t);
                }
            });
            ui.end_row();
            ui.label(l("Dati", "Data"));
            ui.text_edit_singleline(&mut self.data);
            ui.end_row();
            dv(ui, &l("Larghezza modulo (mm)", "Module width (mm)"), &mut self.module, 0.1..=3.0, 0.01);
            dv(ui, &l("Altezza barre (mm)", "Bar height (mm)"), &mut self.height, 2.0..=200.0, 0.5);
        });
        ui.checkbox(&mut self.human, l("Testo leggibile sotto le barre", "Human-readable text under the bars"));
        hint(ui, &l("Code 128: testo e numeri. EAN-13/EAN-8/UPC-A: solo cifre, la cifra di controllo si aggiunge da sola. Il layer attivo passa a Riempi per incidere le barre.",
                    "Code 128: text and digits. EAN-13/EAN-8/UPC-A: digits only, the check digit is added automatically. The active layer is switched to Fill to engrave the bars."));
        buttons(ui, &tr("create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.kind == "qr" {
            app.dialogs.push(crate::dialogs::Dialog::Generic(Box::new(Qr { text: self.data.clone(), size: 30.0, err: "m".into() })));
            return true;
        }
        let (mods, text) = match corners::barcode_modules(&self.kind, self.data.trim()) {
            Ok(x) => x,
            Err(e) => {
                app.say(e);
                return false;
            }
        };
        let (mw, h) = (self.module, self.height);
        let (ww, wh) = app.ws();
        let total = mods.len() as f64 * mw;
        let (x0, y0) = ((ww - total) / 2.0, (wh - h) / 2.0);
        let layer = app.doc.active_layer;
        let mut kids: Vec<Value> = corners::barcode_rects(&mods, x0, y0, mw, h).iter()
            .map(|r| json!({"id": app.doc.new_id(), "type": "rect", "layer": layer, "x": r[0], "y": r[1], "w": r[2], "h": r[3]})).collect();
        if self.human {
            let size = (h * 0.18).clamp(1.5, 6.0);
            let mut t = Shape(props(json!({"id": app.doc.new_id(), "type": "text", "layer": layer, "x": x0, "y": y0 + h + 0.8, "text": text, "size": size, "font": "Helvetica"})));
            let tw = crate::text::text_width(&t);
            t.set("x", x0 + (total - tw) / 2.0);
            kids.push(Value::Object(t.0));
        }
        let name = format!("{} {}", self.kind, text);
        add_group(app, kids, Some(name), true);
        app.say(l(&format!("{text} · {:.1} mm con i margini", total + 20.0 * mw), &format!("{text} · {:.1} mm with quiet zones", total + 20.0 * mw)));
        true
    }
}

// ====================================================================== text tools
struct TextOnPath {
    start: f64,
    dist: f64,
    spacing: f64,
    flip: bool,
    centre: bool,
    keep: bool,
}

impl Extra for TextOnPath {
    fn title(&self) -> String {
        l("Testo su percorso", "Text on Path")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        grid(ui, "top", |ui| {
            dv(ui, &l("Inizio lungo il percorso (mm)", "Start along path (mm)"), &mut self.start, 0.0..=100000.0, 0.5);
            dv(ui, &l("Distanza dal percorso (mm)", "Distance from path (mm)"), &mut self.dist, -500.0..=500.0, 0.1);
            dv(ui, &l("Spaziatura lettere (mm)", "Letter spacing (mm)"), &mut self.spacing, -50.0..=50.0, 0.05);
        });
        ui.checkbox(&mut self.flip, l("Inverti direzione / lato", "Flip direction / side"));
        ui.checkbox(&mut self.centre, l("Centra sul percorso", "Center on path"));
        ui.checkbox(&mut self.keep, l("Mantieni il testo originale", "Keep the original text"));
        buttons(ui, &tr("create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        let sel = app.doc.selected_in_order();
        let (Some(t), Some(path)) = (sel.iter().find(|s| s.kind() == "text").cloned(), sel.last().cloned()) else { return true };
        let pts = path.paths().into_iter().next().unwrap_or_default();
        let total = crate::tools::path_length(&pts);
        let mut st = self.start;
        if self.centre {
            let n = crate::text::resolve_text(&t).chars().count();
            let tw = crate::text::text_width(&t) + self.spacing * n.saturating_sub(1) as f64;
            st = ((total - tw) / 2.0).max(0.0);
        }
        let polys = crate::text::text_on_path(&t, &pts, st, self.dist, self.spacing, self.flip);
        if polys.is_empty() {
            return false;
        }
        let kids: Vec<Value> = polys.iter().map(|p| json!({"id": app.doc.new_id(), "type": "poly", "layer": t.layer(), "closed": true,
            "points": p[..p.len() - 1].iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()})).collect();
        let mut g = app.doc.create("group", props(json!({"children": kids})));
        g.0.insert("layer".into(), json!(t.layer()));
        app.doc.save_undo("");
        if !self.keep {
            app.doc.shapes.retain(|s| s.id() != t.id());
        }
        app.doc.selected = vec![g.id()];
        app.doc.shapes.push(g);
        app.invalidate();
        true
    }
}

struct VarText {
    id: i64,
    template: String,
    var: Map<String, Value>,
}

fn parse_csv(text: &str) -> Vec<Map<String, Value>> {
    let first = text.lines().next().unwrap_or("");
    let delim = [',', ';', '\t'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',');
    let split = |line: &str| -> Vec<String> {
        let (mut out, mut cur, mut q) = (Vec::new(), String::new(), false);
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                if q && chars.get(i + 1) == Some(&'"') {
                    cur.push('"');
                    i += 1;
                } else {
                    q = !q;
                }
            } else if c == delim && !q {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
            i += 1;
        }
        out.push(cur);
        out
    };
    let mut lines = text.trim_start_matches('\u{feff}').lines().filter(|l| !l.trim().is_empty());
    let Some(h) = lines.next() else { return Vec::new() };
    let header = split(h);
    lines.map(|l| {
        let vals = split(l);
        header.iter().enumerate().map(|(i, k)| (k.clone(), json!(vals.get(i).cloned().unwrap_or_default()))).collect()
    }).collect()
}

impl Extra for VarText {
    fn title(&self) -> String {
        l("Testo variabile", "Variable Text")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        hint(ui, &l("Segnaposto nel testo: {serial} numero di serie · {data} · {ora} · {NomeColonna} dal file CSV. Dopo ogni lavoro inviato il numero avanza e si passa alla riga CSV successiva.",
                    "Placeholders: {serial} serial number · {date} · {time} · {ColumnName} from the CSV file. After each job sent the number advances and the next CSV row is used."));
        let v = &mut self.var;
        let mut on = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
        let gi = |k: &str, d: i64| v.get(k).and_then(|x| x.as_i64()).unwrap_or(d);
        let (mut serial, mut step, mut digits, mut row) = (gi("serial", 1), gi("step", 1), gi("digits", 3), gi("row", 0) + 1);
        let rows = v.get("csv").and_then(|x| x.as_array()).map_or(0, |a| a.len());
        grid(ui, "vt", |ui| {
            ui.label(l("Testo / modello", "Text / template"));
            ui.text_edit_singleline(&mut self.template);
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut on, l("Abilita testo variabile", "Enable variable text"));
            ui.end_row();
            iv(ui, &l("Numero attuale", "Current number"), &mut serial, -999999..=999999);
            iv(ui, &l("Incremento", "Advance by"), &mut step, -1000..=1000);
            iv(ui, &l("Cifre minime", "Minimum digits"), &mut digits, 1..=12);
            ui.label("CSV");
            ui.horizontal(|ui| {
                if ui.button(l("Carica CSV...", "Load CSV...")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv", "txt"]).pick_file() {
                        if let Ok(t) = std::fs::read(&p) {
                            v.insert("csv".into(), json!(parse_csv(&String::from_utf8_lossy(&t))));
                        }
                    }
                }
                ui.label(if rows > 0 { l(&format!("{rows} righe"), &format!("{rows} rows")) } else { "--".into() });
                ui.label(l("Riga", "Row"));
                ui.add(egui::DragValue::new(&mut row).range(1..=rows.max(1) as i64));
            });
            ui.end_row();
        });
        for (k, x) in [("serial", serial), ("step", step), ("digits", digits), ("row", row - 1)] {
            v.insert(k.into(), json!(x));
        }
        v.insert("enabled".into(), json!(on));
        let preview = Shape(props(json!({"text": self.template, "var": v})));
        ui.label(egui::RichText::new(crate::text::resolve_text(&preview)).color(Color32::from_rgb(0x9e, 0xc5, 0xff)).size(14.0));
        buttons(ui, &tr("apply"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.doc.save_undo("");
        if let Some(s) = app.doc.find_mut(self.id) {
            s.0.insert("text".into(), json!(self.template));
            s.0.insert("var".into(), Value::Object(self.var.clone()));
        }
        app.invalidate();
        true
    }
}

// ====================================================================== corner and shape tools
/// Selected rect / circle / poly shapes as editable paths (in place); ids of the polys to work on.
fn closed_polys(app: &mut LaserApp) -> Vec<i64> {
    let sel = app.doc.selected.clone();
    let mut out = Vec::new();
    for s in app.doc.shapes.iter_mut().filter(|s| sel.contains(&s.id())) {
        if matches!(s.kind(), "rect" | "circle") {
            *s = crate::edit::to_poly(s);
        }
        if s.kind() == "poly" {
            out.push(s.id());
        }
    }
    out
}

struct CornerTool {
    nodes: Vec<(i64, usize)>,
    kind: &'static str,
    a: f64,
    b: f64,
    c: f64,
    flag: bool,
    tbone: bool,
}

impl Extra for CornerTool {
    fn title(&self) -> String {
        match self.kind {
            "fillet" => l("Raccorda angoli", "Fillet Corners"),
            "dogbone" => "Dogbone / T-bone".into(),
            "slot" => l("Ridimensiona fessure", "Resize Slots"),
            _ => l("Rastrema", "Taper Warp"),
        }
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        match self.kind {
            "fillet" => {
                grid(ui, "fil", |ui| dv(ui, &l("Raggio (mm)", "Radius (mm)"), &mut self.a, 0.05..=500.0, 0.1));
                let n = self.nodes.len();
                hint(ui, &if n > 0 { l(&format!("{n} nodi selezionati: verranno raccordati solo quelli."), &format!("{n} selected nodes: only those are filleted.")) }
                          else { l("Nessun nodo selezionato: vengono raccordati tutti gli angoli dritti delle forme selezionate (seleziona i nodi con Modifica nodi per scegliere).",
                                   "No nodes selected: every straight corner of the selected shapes is filleted (select nodes with Edit Nodes to choose).") });
            }
            "dogbone" => {
                ui.horizontal(|ui| {
                    ui.label(l("Tipo", "Type"));
                    ui.selectable_value(&mut self.tbone, false, "Dogbone");
                    ui.selectable_value(&mut self.tbone, true, "T-bone");
                });
                grid(ui, "dog", |ui| dv(ui, &l("Diametro (mm)", "Diameter (mm)"), &mut self.a, 0.1..=50.0, 0.1));
                ui.checkbox(&mut self.flag, l("Solo angoli di circa 90°", "Only corners near 90°"));
                hint(ui, &l("Aggiunge uno scarico circolare agli angoli interni del materiale (fessure e fori), così un pezzo ad angolo vivo entra fino in fondo.",
                            "Adds a circular relief to the material's inner corners (slots and holes) so a square-cornered part seats fully."));
            }
            "slot" => {
                grid(ui, "slot", |ui| {
                    dv(ui, &l("Larghezza attuale (mm)", "Current width (mm)"), &mut self.a, 0.1..=100.0, 0.01);
                    dv(ui, &l("Nuova larghezza (mm)", "New width (mm)"), &mut self.b, 0.1..=100.0, 0.01);
                    dv(ui, &l("Tolleranza (mm)", "Tolerance (mm)"), &mut self.c, 0.001..=2.0, 0.005);
                });
                hint(ui, &l("Trova le fessure larghe come lo spessore del vecchio materiale e le adatta al nuovo spessore, mantenendole centrate.",
                            "Finds slots matching the old material thickness and resizes them to the new thickness, keeping them centred."));
            }
            _ => {
                grid(ui, "tap", |ui| {
                    dv(ui, &l("Larghezza in alto %", "Top width %"), &mut self.a, 1.0..=500.0, 1.0);
                    dv(ui, &l("Larghezza in basso %", "Bottom width %"), &mut self.b, 1.0..=500.0, 1.0);
                });
                hint(ui, &l("Utile per incidere su oggetti conici (bicchieri, tazze): stringe la grafica in alto o in basso.",
                            "Useful for engraving on tapered objects (tumblers, cups): narrows the artwork at the top or bottom."));
            }
        }
        buttons(ui, "OK")
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.doc.save_undo(&self.title());
        match self.kind {
            "taper" => {
                let sel = app.doc.selected_shapes();
                let mut next = app.doc.next_id;
                let new = crate::tools::taper(&mut next, &sel, self.a, self.b);
                app.doc.next_id = next;
                for n in new {
                    if let Some(s) = app.doc.find_mut(n.id()) {
                        *s = n;
                    }
                }
            }
            _ => {
                let ids = closed_polys(app);
                let mut n = 0;
                if self.kind == "dogbone" {
                    let mut shapes: Vec<Shape> = ids.iter().filter_map(|i| app.doc.find(*i).cloned()).collect();
                    n = corners::relief(&mut shapes, self.a, self.tbone, self.flag);
                    for s in shapes {
                        if let Some(x) = app.doc.find_mut(s.id()) {
                            *x = s;
                        }
                    }
                } else {
                    for id in ids {
                        let only: Vec<usize> = self.nodes.iter().filter(|(sid, _)| *sid == id).map(|x| x.1).collect();
                        if !self.nodes.is_empty() && only.is_empty() && self.kind == "fillet" {
                            continue;
                        }
                        if let Some(s) = app.doc.find_mut(id) {
                            n += if self.kind == "fillet" { corners::fillet(s, self.a, (!self.nodes.is_empty()).then_some(only.as_slice())) } else { corners::resize_slots(s, self.a, self.b, self.c) };
                        }
                    }
                    if let Some(h) = app.canvas.handlers.get_mut("nodes") {
                        h.reset();
                    }
                }
                app.say(match self.kind {
                    "fillet" => l(&format!("{n} angoli raccordati"), &format!("{n} corners filleted")),
                    "dogbone" => l(&format!("{n} angoli modificati"), &format!("{n} corners relieved")),
                    _ => l(&format!("{n} fessure ridimensionate"), &format!("{n} slots resized")),
                });
            }
        }
        app.invalidate();
        true
    }
}

struct BoolAssistant {
    ops: Vec<(String, String, Vec<Vec<P>>)>,
    pick: Option<String>,
}

impl Extra for BoolAssistant {
    fn title(&self) -> String {
        l("Assistente booleano", "Boolean Assistant")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        hint(ui, &l("Clicca il risultato che vuoi: le due forme vengono sostituite.", "Click the result you want: the two shapes are replaced."));
        let sel: Vec<Shape> = app.doc.selected_in_order();
        let all: Vec<P> = sel.iter().flat_map(|s| s.paths()).flatten().collect();
        let b = crate::shapes::path_box(&all);
        ui.horizontal(|ui| {
            for (op, label, rings) in &self.ops {
                ui.vertical(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(150.0, 150.0), egui::Sense::click());
                    let p = ui.painter_at(rect);
                    p.rect_filled(rect, 4.0, Color32::WHITE);
                    let k = 130.0 / b[2].max(b[3]).max(1e-6);
                    let tp = |q: P| rect.min + egui::vec2((10.0 + (q[0] - b[0]) * k) as f32, (10.0 + (q[1] - b[1]) * k) as f32);
                    for s in &sel {
                        for path in s.paths() {
                            p.extend(egui::Shape::dashed_line(&path.iter().map(|q| tp(*q)).collect::<Vec<_>>(), egui::Stroke::new(1.0, Color32::from_gray(200)), 3.0, 3.0));
                        }
                    }
                    for r in rings {
                        p.add(egui::Shape::line(r.iter().map(|q| tp(*q)).collect(), egui::Stroke::new(1.6, Color32::from_rgb(0x2f, 0x7b, 0xf5))));
                    }
                    if resp.hovered() {
                        p.rect_stroke(rect, 4.0, egui::Stroke::new(2.0, Color32::from_rgb(0x2f, 0x7b, 0xf5)), egui::StrokeKind::Inside);
                    }
                    if resp.clicked() && !rings.is_empty() {
                        self.pick = Some(op.clone());
                    }
                    ui.label(label);
                });
            }
        });
        if self.pick.is_some() {
            return Some(true);
        }
        close_button(ui)
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if let Some(op) = self.pick.take() {
            let r = app.doc.geometry_op(&op, &json!({}), true, 2);
            app.msg(r);
        }
        true
    }
}

// ====================================================================== machine
struct Feeder {
    m: laserforge_core::model::MachineSettings,
}

impl Extra for Feeder {
    fn title(&self) -> String {
        l("Alimentatore automatico", "Feeder Setup")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        let m = &mut self.m;
        ui.checkbox(&mut m.feeder_enabled, l("Ripeti il lavoro facendo avanzare il materiale", "Repeat the job, advancing the material"));
        grid(ui, "feed", |ui| {
            ui.label(l("Asse dell'alimentatore", "Feeder axis"));
            ui.horizontal(|ui| {
                for k in ["A", "Y", "Z", "custom"] {
                    ui.selectable_value(&mut m.feeder_axis, k.to_string(), if k == "custom" { "G-code".to_string() } else { k.to_string() });
                }
            });
            ui.end_row();
            dv(ui, &l("Avanzamento (mm)", "Advance (mm)"), &mut m.feeder_distance, 0.1..=10000.0, 1.0);
            iv(ui, &l("Ripetizioni", "Repetitions"), &mut m.feeder_repeats, 1..=1000);
            dv(ui, &l("Velocità avanzamento (mm/min)", "Feed speed (mm/min)"), &mut m.feeder_speed, 10.0..=50000.0, 10.0);
            ui.label(l("G-code personalizzato", "Custom G-code"));
            ui.add(egui::TextEdit::multiline(&mut m.feeder_gcode).desired_rows(3).code_editor());
            ui.end_row();
        });
        hint(ui, &l("Per macchine con nastro o rullo trascinatore: dopo ogni lavoro l'asse scelto fa avanzare il materiale e il lavoro riparte, per il numero di ripetizioni indicato.",
                    "For machines with a conveyor or feed roller: after each run the chosen axis advances the material and the job runs again, for the given number of repetitions."));
        buttons(ui, "OK")
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.machine = self.m.clone();
        true
    }
}

const GRBL_NAMES: [(u32, &str, &str); 41] = [
    (0, "Impulso passo (µs)", "Step pulse (µs)"), (1, "Ritardo inattività passo (ms)", "Step idle delay (ms)"),
    (2, "Inverti impulso passo (maschera)", "Step port invert (mask)"), (3, "Inverti direzione (maschera)", "Direction port invert (mask)"),
    (4, "Inverti abilitazione passi", "Step enable invert"), (5, "Inverti pin finecorsa", "Limit pins invert"),
    (6, "Inverti pin sonda", "Probe pin invert"), (10, "Rapporto di stato (maschera)", "Status report (mask)"),
    (11, "Deviazione agli angoli (mm)", "Junction deviation (mm)"), (12, "Tolleranza archi (mm)", "Arc tolerance (mm)"),
    (13, "Rapporti in pollici", "Report inches"), (20, "Limiti software", "Soft limits"), (21, "Limiti hardware (finecorsa)", "Hard limits"),
    (22, "Ciclo di homing", "Homing cycle"), (23, "Direzione homing (maschera)", "Homing direction invert (mask)"),
    (24, "Velocità homing lenta (mm/min)", "Homing feed (mm/min)"), (25, "Velocità homing ricerca (mm/min)", "Homing seek (mm/min)"),
    (26, "Antirimbalzo homing (ms)", "Homing debounce (ms)"), (27, "Distacco dopo homing (mm)", "Homing pull-off (mm)"),
    (30, "Valore S max (potenza 100%)", "Max spindle / S-value"), (31, "Valore S min", "Min spindle / S-value"),
    (32, "Modalità laser", "Laser mode enable"),
    (100, "Passi/mm X", "X steps/mm"), (101, "Passi/mm Y", "Y steps/mm"), (102, "Passi/mm Z", "Z steps/mm"), (103, "Passi/mm A", "A steps/mm"),
    (110, "Velocità max X (mm/min)", "X max rate (mm/min)"), (111, "Velocità max Y (mm/min)", "Y max rate (mm/min)"),
    (112, "Velocità max Z (mm/min)", "Z max rate (mm/min)"), (113, "Velocità max A", "A max rate"),
    (120, "Accelerazione X (mm/s²)", "X acceleration (mm/s²)"), (121, "Accelerazione Y (mm/s²)", "Y acceleration (mm/s²)"),
    (122, "Accelerazione Z (mm/s²)", "Z acceleration (mm/s²)"), (123, "Accelerazione A", "A acceleration"),
    (130, "Corsa max X (mm)", "X max travel (mm)"), (131, "Corsa max Y (mm)", "Y max travel (mm)"),
    (132, "Corsa max Z (mm)", "Z max travel (mm)"), (133, "Corsa max A", "A max travel"),
    (14, "", ""), (15, "", ""), (16, "", ""),
];

/// The $$ values GRBL has reported on this connection.
fn grbl_settings(app: &LaserApp) -> std::collections::BTreeMap<u32, f64> {
    app.conn.as_ref().map(|c| c.shared.lock().unwrap().settings.clone()).unwrap_or_default()
}

struct GrblSettings {
    vals: Vec<(u32, String)>,
    loaded: std::collections::BTreeMap<u32, f64>,
}

impl Extra for GrblSettings {
    fn title(&self) -> String {
        l("Impostazioni macchina (GRBL)", "Machine Settings (GRBL)")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        let live = grbl_settings(app);
        if self.vals.is_empty() && !live.is_empty() {
            self.loaded = live.clone();
            self.vals = live.iter().map(|(k, v)| (*k, format!("{v}"))).collect();
        }
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            egui::Grid::new("grbl").striped(true).num_columns(3).show(ui, |ui| {
                ui.weak("$");
                ui.weak(l("Impostazione", "Setting"));
                ui.weak(l("Valore", "Value"));
                ui.end_row();
                for (k, v) in self.vals.iter_mut() {
                    ui.monospace(format!("${k}"));
                    let name = GRBL_NAMES.iter().find(|n| n.0 == *k).map(|n| if crate::defs::english() { n.2 } else { n.1 }).unwrap_or("");
                    ui.label(name);
                    ui.add(egui::TextEdit::singleline(v).desired_width(90.0));
                    ui.end_row();
                }
            });
        });
        let current: std::collections::BTreeMap<u32, f64> = self.vals.iter().filter_map(|(k, v)| v.replace(',', ".").trim().parse::<f64>().ok().map(|x| (*k, x))).collect();
        let probs = laserforge_core::grbl::problems(&current, app.machine.max_power);
        if probs.is_empty() {
            ui.colored_label(Color32::from_rgb(0x7b, 0xd8, 0x8f), l("Nessun problema nelle impostazioni lette.", "No problems in the settings read."));
        }
        for p in probs {
            ui.colored_label(Color32::from_rgb(0xe0, 0xa0, 0x30), format!("⚠ {p}"));
        }
        ui.horizontal(|ui| {
            if ui.button(l("Leggi", "Read")).clicked() {
                self.vals.clear();
                app.send("$$");
            }
            if ui.button(l("Scrivi", "Write")).clicked() {
                let changes: Vec<(u32, f64)> = current.iter().filter(|(k, v)| self.loaded.get(k).is_none_or(|o| (o - *v).abs() > 1e-9)).map(|(k, v)| (*k, *v)).collect();
                if changes.is_empty() {
                    app.say(l("Nessuna modifica da scrivere", "Nothing changed"));
                } else if !app.connected() {
                    app.say(l("Connetti il laser per scrivere le impostazioni", "Connect the laser to write the settings"));
                } else {
                    for (k, v) in &changes {
                        app.send(&format!("${k}={v}"));
                        self.loaded.insert(*k, *v);
                    }
                    app.say(l(&format!("Scritte {} impostazioni", changes.len()), &format!("Wrote {} settings", changes.len())));
                }
            }
            if ui.button(l("Salva su file", "Save to File")).clicked() {
                let t: String = current.iter().map(|(k, v)| format!("${k}={v}\n")).collect();
                app.save_text_file(&t, "grbl-settings.txt", ("GRBL", &["txt", "nc"]));
            }
            if ui.button(l("Carica da file", "Load from File")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("GRBL", &["txt", "nc"]).pick_file() {
                    let re = regex::Regex::new(r"^\s*\$(\d+)\s*=\s*(-?[\d.]+)").unwrap();
                    let text = std::fs::read_to_string(p).unwrap_or_default();
                    for line in text.lines() {
                        if let Some(c) = re.captures(line) {
                            let k: u32 = c[1].parse().unwrap_or(0);
                            match self.vals.iter_mut().find(|x| x.0 == k) {
                                Some(x) => x.1 = c[2].to_string(),
                                None => self.vals.push((k, c[2].to_string())),
                            }
                        }
                    }
                    self.vals.sort_by_key(|x| x.0);
                    app.say(l("Caricate: premi «Scrivi» per mandarle al controller", "Loaded: press “Write” to send them to the controller"));
                }
            }
        });
        close_button(ui)
    }
    fn apply(&mut self, _app: &mut LaserApp) -> bool {
        true
    }
}

struct Calibrate {
    axis: u32,
    steps: f64,
    want: f64,
    got: f64,
}

impl Extra for Calibrate {
    fn title(&self) -> String {
        l("Calibra asse", "Calibrate Axis")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        let live = grbl_settings(app);
        grid(ui, "cal", |ui| {
            ui.label(l("Asse", "Axis"));
            ui.horizontal(|ui| {
                for (k, t) in [(100u32, "X ($100)"), (101, "Y ($101)")] {
                    if ui.selectable_value(&mut self.axis, k, t).clicked() {
                        if let Some(v) = live.get(&k) {
                            self.steps = *v;
                        }
                    }
                }
            });
            ui.end_row();
            dv(ui, &l("Passi/mm attuali", "Current steps/mm"), &mut self.steps, 1.0..=10000.0, 0.1);
            dv(ui, &l("Lunghezza disegnata (mm)", "Designed length (mm)"), &mut self.want, 1.0..=3000.0, 0.5);
            dv(ui, &l("Lunghezza misurata (mm)", "Measured length (mm)"), &mut self.got, 1.0..=3000.0, 0.05);
        });
        let nv = self.steps * self.want / self.got.max(0.001);
        ui.label(egui::RichText::new(l(&format!("Nuovo valore: {nv:.3} passi/mm"), &format!("New value: {nv:.3} steps/mm"))).strong());
        hint(ui, &l("1) Incidi una linea o un quadrato della lunghezza indicata.  2) Misurala con un calibro.  3) Inserisci la misura: il nuovo valore viene scritto in GRBL.",
                    "1) Engrave a line or square of the designed length.  2) Measure it with a caliper.  3) Enter the measurement: the new value is written to GRBL."));
        if ui.button(l("Leggi valori dal controller", "Read values from controller")).clicked() {
            app.send("$$");
            if let Some(v) = live.get(&self.axis) {
                self.steps = *v;
            }
        }
        buttons(ui, &l("Scrivi sul controller", "Write to controller"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if !app.connected() {
            app.say(l("Connetti il laser per scrivere il valore", "Connect the laser to write the value"));
            return false;
        }
        let cmd = format!("${}={:.3}", self.axis, self.steps * self.want / self.got.max(0.001));
        app.send(&cmd);
        app.say(l(&format!("Scritto {cmd}"), &format!("Wrote {cmd}")));
        true
    }
}

struct ControllerFiles {
    asked: usize,
}

impl Extra for ControllerFiles {
    fn title(&self) -> String {
        l("File sul controller", "Laser files")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        let fluid = app.machine.flavor == "fluidnc";
        let lines: Vec<String> = app.conn.as_ref().map(|c| c.shared.lock().unwrap().console.iter().skip(self.asked).map(|l| l.text.clone()).collect()).unwrap_or_default();
        let re = regex::Regex::new(r"\[FILE:\s*([^|\]]+)(?:\|SIZE:(\d+))?").unwrap();
        let files: Vec<(String, Option<u64>)> = lines.iter().filter_map(|l| re.captures(l).map(|c| (c[1].trim().to_string(), c.get(2).and_then(|s| s.as_str().parse().ok())))).collect();
        let mut run = None;
        let mut del = None;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            if files.is_empty() {
                ui.weak(l("(nessun file o risposta non riconosciuta: guarda la Console)", "(no files or unrecognised reply: see the Console)"));
            }
            for (name, size) in &files {
                ui.horizontal(|ui| {
                    ui.label(format!("{name}{}", size.map_or(String::new(), |s| format!("   ({} KB)", s / 1024))));
                    if ui.small_button(l("Avvia", "Run")).clicked() {
                        run = Some(name.clone());
                    }
                    if ui.small_button(l("Elimina", "Delete")).clicked() {
                        del = Some(name.clone());
                    }
                });
            }
        });
        if ui.button(l("Aggiorna", "Refresh")).clicked() {
            self.asked = app.conn.as_ref().map_or(0, |c| c.shared.lock().unwrap().console.len());
            app.send(if fluid { "$SD/List" } else { "$F" });
        }
        if let Some(n) = run {
            app.send(&if fluid { format!("$SD/Run={n}") } else { format!("$F={n}") });
        }
        if let Some(n) = del {
            app.send(&if fluid { format!("$SD/Delete={n}") } else { format!("$FD={n}") });
        }
        hint(ui, &l("Per caricare un file sulla scheda usa l'interfaccia web di FluidNC o la scheda SD: LaserForge salva il G-code con File › Salva G-code.",
                    "To put a file on the card use FluidNC's web interface or the SD card: LaserForge saves G-code with File › Save GCode."));
        close_button(ui)
    }
    fn apply(&mut self, _app: &mut LaserApp) -> bool {
        true
    }
}

// ====================================================================== GRBL check after connecting
pub struct GrblCheck {
    pub probs: Vec<String>,
    pub laser_off: bool,
    pub open_settings: bool,
}

impl Extra for GrblCheck {
    fn title(&self) -> String {
        l("Controlla le impostazioni di GRBL", "Check the GRBL settings")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        for p in &self.probs {
            ui.colored_label(Color32::from_rgb(0xe0, 0xa0, 0x30), format!("⚠ {p}"));
            ui.add_space(4.0);
        }
        ui.separator();
        let mut r = None;
        ui.horizontal(|ui| {
            if self.laser_off && ui.button(l("Attiva modalità laser ($32=1)", "Enable laser mode ($32=1)")).clicked() {
                app.send("$32=1");
                if let Some(c) = &app.conn {
                    c.shared.lock().unwrap().settings.insert(32, 1.0);
                }
                app.say(l("Modalità laser attivata ($32=1)", "Laser mode enabled ($32=1)"));
                r = Some(false);
            }
            if ui.button(l("Apri Impostazioni macchina", "Open Machine Settings")).clicked() {
                self.open_settings = true;
                r = Some(true);
            }
            if ui.button(tr("close")).clicked() {
                r = Some(false);
            }
        });
        r
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.open_settings {
            app.run("machineSettings");
        }
        true
    }
}

// ====================================================================== devices and checklist
pub const DEFAULT_CHECKLIST: [&str; 5] = ["Aspirazione / ventilazione accesa", "Aria compressa aperta", "Messa a fuoco controllata", "Materiale fissato e piano", "Occhiali protettivi indossati"];

struct Devices {
    sel: Option<usize>,
    name: String,
}

impl Extra for Devices {
    fn title(&self) -> String {
        l("Dispositivi", "Devices")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        let list = app.devices.clone();
        for (i, d) in list.iter().enumerate() {
            let m = &d["machine"];
            let cur = d["name"].as_str() == Some(app.device.as_str());
            let t = format!("{}{}   —   {:.0}×{:.0} mm, S max {}, {} baud", if cur { "● " } else { "   " }, d["name"].as_str().unwrap_or(""),
                            m["workspace_x"].as_f64().unwrap_or(0.0), m["workspace_y"].as_f64().unwrap_or(0.0), m["max_power"], m["baud_rate"]);
            let r = ui.selectable_label(self.sel == Some(i), t);
            if r.clicked() {
                self.sel = Some(i);
                self.name = d["name"].as_str().unwrap_or("").to_string();
            }
            if r.double_clicked() {
                app.apply_device(&list[i]);
            }
        }
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.name);
            if ui.button(l("Nuovo dalle impostazioni attuali", "New from current settings")).clicked() && !self.name.trim().is_empty() {
                app.devices.push(json!({"name": self.name.trim(), "machine": app.machine, "port": app.port}));
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(i) = self.sel.filter(|i| *i < app.devices.len()) {
                if ui.button(l("Usa", "Use")).clicked() {
                    let d = app.devices[i].clone();
                    app.apply_device(&d);
                }
                if ui.button(l("Aggiorna", "Update")).clicked() {
                    app.devices[i]["machine"] = json!(app.machine);
                }
                if ui.button(l("Rinomina", "Rename")).clicked() && !self.name.trim().is_empty() {
                    app.devices[i]["name"] = json!(self.name.trim());
                }
                if ui.button(l("Elimina", "Delete")).clicked() {
                    app.devices.remove(i);
                    self.sel = None;
                }
                if ui.button(l("Esporta…", "Export…")).clicked() {
                    let t = serde_json::to_string_pretty(&app.devices[i]).unwrap();
                    let name = format!("{}.lfdevice", app.devices[i]["name"].as_str().unwrap_or("laser"));
                    app.save_text_file(&t, &name, ("LaserForge device", &["lfdevice"]));
                }
            }
            if ui.button(l("Importa…", "Import…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("LaserForge device", &["lfdevice", "json"]).pick_file() {
                    match std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|v| v.get("machine").is_some()) {
                        Some(mut d) => {
                            if d.get("name").is_none() {
                                d["name"] = json!(p.file_stem().unwrap().to_string_lossy());
                            }
                            app.devices.push(d);
                        }
                        None => app.say(l("File non valido", "Invalid file")),
                    }
                }
            }
        });
        hint(ui, &l("Ogni dispositivo salva le impostazioni della macchina: area di lavoro, velocità e potenza massime, baud, G-code iniziale e finale, rotativo. 'Aggiorna' salva le impostazioni attuali nel dispositivo selezionato.",
                    "Each device stores the machine settings: work area, max speed and power, baud, start/end G-code, rotary. 'Update' saves the current settings into the selected device."));
        close_button(ui)
    }
    fn apply(&mut self, _app: &mut LaserApp) -> bool {
        true
    }
}

struct NewDevice {
    detect: Option<std::sync::mpsc::Receiver<Vec<(String, String, String)>>>,
    found: String,
    name: String,
    m: laserforge_core::model::MachineSettings,
    usb: bool,
    port: String,
    host: String,
}

impl Extra for NewDevice {
    fn title(&self) -> String {
        l("Nuovo dispositivo", "New device")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        let m = &mut self.m;
        grid(ui, "nd", |ui| {
            ui.label(l("Nome", "Name"));
            ui.text_edit_singleline(&mut self.name);
            ui.end_row();
            ui.label("Controller");
            egui::ComboBox::from_id_salt("ndfl").selected_text(m.flavor.clone()).show_ui(ui, |ui| {
                for (k, t) in [("grbl", "GRBL"), ("grblhal", "grblHAL"), ("fluidnc", "FluidNC"), ("marlin", "Marlin"), ("smoothie", "Smoothieware"), ("linuxcnc", "LinuxCNC")] {
                    if ui.selectable_value(&mut m.flavor, k.to_string(), t).clicked() {
                        m.max_power = match k { "marlin" => 255, "smoothie" => 1, _ if m.max_power > 1 => m.max_power, _ => 1000 };
                    }
                }
            });
            ui.end_row();
            ui.label(l("Collegamento", "Connection"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.usb, true, "USB");
                ui.selectable_value(&mut self.usb, false, l("Rete", "Network"));
            });
            ui.end_row();
            if self.usb {
                ui.label("");
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.detect.is_none(), egui::Button::new(l("Rileva automaticamente", "Detect automatically"))).clicked() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let baud = m.baud_rate.max(1) as u32;
                        std::thread::spawn(move || { let _ = tx.send(crate::connection::detect_controllers(baud)); });
                        self.detect = Some(rx);
                        self.found = l("Ricerca in corso…", "Searching…");
                    }
                    ui.label(&self.found);
                });
                ui.end_row();
                if let Some(rx) = &self.detect {
                    if let Ok(res) = rx.try_recv() {
                        self.detect = None;
                        match res.first() {
                            Some((dev, fl, banner)) => {
                                self.port = dev.clone();
                                m.flavor = fl.clone();
                                self.found = l(&format!("Trovato: {banner} su {dev}"), &format!("Found: {banner} on {dev}"));
                            }
                            None => self.found = l("Nessun controller trovato sulle porte USB.", "No controller found on the USB ports."),
                        }
                    } else {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                    }
                }
                ui.label(l("Porta USB", "USB port"));
                egui::ComboBox::from_id_salt("ndport").selected_text(self.port.clone()).show_ui(ui, |ui| {
                    for p in crate::connection::list_ports() {
                        ui.selectable_value(&mut self.port, p.clone(), p);
                    }
                });
            } else {
                ui.label(l("Indirizzo di rete", "Network address"));
                ui.text_edit_singleline(&mut self.host);
            }
            ui.end_row();
            dv(ui, &l("Area di lavoro larghezza (mm)", "Work area width (mm)"), &mut m.workspace_x, 10.0..=3000.0, 1.0);
            dv(ui, &l("Area di lavoro altezza (mm)", "Work area height (mm)"), &mut m.workspace_y, 10.0..=3000.0, 1.0);
            iv(ui, &l("Valore S massimo", "Max S value"), &mut m.max_power, 1..=100000);
            ui.label("Baud");
            egui::ComboBox::from_id_salt("ndb").selected_text(m.baud_rate.to_string()).show_ui(ui, |ui| {
                for b in [115200i64, 250000, 230400, 57600, 9600] {
                    ui.selectable_value(&mut m.baud_rate, b, b.to_string());
                }
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.home_on_start, l("La macchina ha i finecorsa (home all'avvio)", "The machine has limit switches (home at start)"));
            ui.end_row();
            ui.label(l("Origine (0,0 della macchina)", "Origin (machine 0,0)"));
            egui::Grid::new("ndo").show(ui, |ui| {
                ui.selectable_value(&mut m.origin, "rl".into(), l("↖ Dietro sx", "↖ Rear left"));
                ui.selectable_value(&mut m.origin, "rr".into(), l("Dietro dx ↗", "Rear right ↗"));
                ui.end_row();
                ui.selectable_value(&mut m.origin, "fl".into(), l("↙ Davanti sx", "↙ Front left"));
                ui.selectable_value(&mut m.origin, "fr".into(), l("Davanti dx ↘", "Front right ↘"));
                ui.end_row();
            });
            ui.end_row();
        });
        buttons(ui, &l("Crea", "Create"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        let address = if self.usb { self.port.clone() } else { format!("tcp://{}", self.host.trim()) };
        let name = if self.name.trim().is_empty() { "Laser".to_string() } else { self.name.trim().to_string() };
        let dev = json!({"name": name, "machine": self.m, "port": address});
        app.devices.retain(|d| d["name"] != dev["name"]);
        app.devices.push(dev.clone());
        app.apply_device(&dev);
        true
    }
}

struct ChecklistEditor {
    on: bool,
    text: String,
}

impl Extra for ChecklistEditor {
    fn title(&self) -> String {
        l("Checklist prima del lavoro", "Pre-job checklist")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        ui.checkbox(&mut self.on, l("Chiedi la checklist prima di ogni lavoro", "Ask for the checklist before every job"));
        ui.label(l("Una voce per riga:", "One item per line:"));
        ui.add(egui::TextEdit::multiline(&mut self.text).desired_rows(8).desired_width(420.0));
        buttons(ui, "OK")
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.checklist = self.text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        app.prefs.checklist_enabled = self.on;
        true
    }
}

pub struct RunChecklist {
    pub ticks: Vec<bool>,
}

impl Extra for RunChecklist {
    fn title(&self) -> String {
        l("Prima di avviare", "Before starting")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        let items: Vec<String> = if app.checklist.is_empty() { DEFAULT_CHECKLIST.iter().map(|s| s.to_string()).collect() } else { app.checklist.clone() };
        self.ticks.resize(items.len(), false);
        for (i, it) in items.iter().enumerate() {
            ui.checkbox(&mut self.ticks[i], it);
        }
        ui.separator();
        let mut r = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.ticks.iter().all(|t| *t), egui::Button::new(egui::RichText::new(l("Avvia", "Start")).strong())).clicked() {
                r = Some(true);
            }
            if ui.button(tr("cancel")).clicked() {
                r = Some(false);
            }
        });
        r
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.checklist_passed = true;
        app.start_laser();
        true
    }
}

// ====================================================================== keyboard shortcuts editor
struct ShortcutsEditor {
    rows: Vec<(String, String, String)>,
    filter: String,
}

impl Extra for ShortcutsEditor {
    fn title(&self) -> String {
        l("Scorciatoie da tastiera", "Keyboard Shortcuts")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        ui.horizontal(|ui| {
            ui.label("🔎");
            ui.text_edit_singleline(&mut self.filter);
        });
        hint(ui, &l("Scrivi la combinazione, per esempio Ctrl+Shift+K, Alt+P o F5. Lascia vuoto per nessuna scorciatoia.", "Type the combination, for example Ctrl+Shift+K, Alt+P or F5. Leave empty for none."));
        let f = self.filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            egui::Grid::new("sce").striped(true).num_columns(3).show(ui, |ui| {
                for (id, label, key) in self.rows.iter_mut() {
                    if !f.is_empty() && !label.to_lowercase().contains(&f) {
                        continue;
                    }
                    ui.label(label.as_str());
                    let bad = !key.is_empty() && crate::app::parse_shortcut(key).is_none();
                    ui.add(egui::TextEdit::singleline(key).desired_width(140.0).text_color_opt(bad.then_some(Color32::from_rgb(220, 60, 60))));
                    if ui.small_button("↺").on_hover_text(l("Predefinita", "Default")).clicked() {
                        *key = crate::defs::shortcut(id);
                    }
                    ui.end_row();
                }
            });
        });
        buttons(ui, "OK")
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.shortcut_overrides = self.rows.iter().filter(|(id, _, k)| *k != crate::defs::shortcut(id)).map(|(id, _, k)| (id.clone(), k.clone())).collect();
        true
    }
}

// ====================================================================== backups and bundles
struct OpenBackup {
    files: Vec<std::path::PathBuf>,
    pick: Option<std::path::PathBuf>,
}

impl Extra for OpenBackup {
    fn title(&self) -> String {
        l("Apri backup", "Open Backup")
    }
    fn body(&mut self, ui: &mut egui::Ui, _app: &mut LaserApp) -> Option<bool> {
        if self.files.is_empty() {
            ui.weak(l("Nessun backup ancora.", "No backups yet."));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for f in &self.files {
                let name = f.file_name().unwrap().to_string_lossy().to_string();
                if ui.selectable_label(self.pick.as_ref() == Some(f), name).clicked() {
                    self.pick = Some(f.clone());
                }
            }
        });
        buttons(ui, &l("Apri", "Open"))
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if let Some(p) = self.pick.clone() {
            app.open_path(&p);
            app.file = None;
        }
        true
    }
}

/// Folder for automatic backups.
pub fn backup_dir() -> std::path::PathBuf {
    let base = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    base.join("Library/Application Support/LaserForge-rs/backups")
}

pub fn for_action(aid: &str, app: &mut LaserApp) -> Option<Box<dyn Extra>> {
    let sel_or_say = |app: &mut LaserApp| -> bool {
        if app.doc.selected.is_empty() {
            app.say(tr("selectFirst"));
            false
        } else {
            true
        }
    };
    Some(match aid {
        "materialTest" => Box::new(Pattern { kind: "material", v: props(json!({"rows": 5, "cols": 5, "power_min": 20, "power_max": 100, "speed_min": 500, "speed_max": 3000, "size": 10, "gap": 3, "mode": "fill", "air_assist": "on", "x": 10, "y": 10})) }),
        "intervalTest" => Box::new(Pattern { kind: "interval", v: props(json!({"count": 6, "interval_min": 0.05, "interval_max": 0.3, "speed": 3000, "power": 40})) }),
        "focusTest" => Box::new(Pattern { kind: "focus", v: props(json!({"count": 11, "z_start": -5, "z_step": 1, "speed": 1000, "power": 30})) }),
        "centerFinder" => Box::new(CenterFinder { pts: [None; 3], res: None }),
        "printAndCut" => {
            let pc = app.print_cut.clone().unwrap_or(json!({}));
            let p2 = |v: &Value| v.as_array().and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
            let src = pc.get("src").and_then(|s| s.as_array()).and_then(|a| Some([p2(a.first()?)?, p2(a.get(1)?)?]));
            let dst = pc.get("dst").and_then(|s| s.as_array()).map(|a| [a.first().and_then(p2), a.get(1).and_then(p2)]).unwrap_or([None, None]);
            Box::new(PrintCut { src, dst, enabled: pc.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) })
        }
        "boxMaker" => Box::new(BoxMaker { w: 100.0, d: 60.0, h: 50.0, t: 3.0, f: 10.0, lid: false }),
        "gearMaker" => Box::new(GearMaker { n: 20, module: 2.0, bore: 6.0 }),
        "createQR" => Box::new(Qr { text: String::new(), size: 30.0, err: "m".into() }),
        "createBarcode" => Box::new(Barcode { kind: "code128".into(), data: "LF-0001".into(), module: 0.33, height: 15.0, human: true }),
        "textOnPath" => {
            let sel = app.doc.selected_in_order();
            if !sel.iter().any(|s| s.kind() == "text") || sel.last().is_none_or(|p| matches!(p.kind(), "text" | "image")) {
                app.say(l("Seleziona un testo e per ultimo il percorso", "Select a text, then the path last"));
                return None;
            }
            Box::new(TextOnPath { start: 0.0, dist: 1.0, spacing: 0.0, flip: false, centre: true, keep: false })
        }
        "varText" => {
            let Some(t) = app.doc.selected_shapes().into_iter().find(|s| s.kind() == "text") else {
                app.say(l("Seleziona un testo", "Select a text"));
                return None;
            };
            let text = t.0.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let var: Map<String, Value> = t.0.get("var").and_then(|v| v.as_object()).cloned()
                .unwrap_or_else(|| props(json!({"enabled": true, "serial": 1, "step": 1, "digits": 3, "csv": [], "row": 0})));
            Box::new(VarText { id: t.id(), template: if text.contains('{') { text } else { format!("{text} {{serial}}") }, var })
        }
        "fillet" => {
            if !sel_or_say(app) { return None; }
            let nodes = if app.canvas.tool == "nodes" { app.canvas.handlers.get("nodes").map(|h| h.selected_nodes()).unwrap_or_default() } else { Vec::new() };
            Box::new(CornerTool { nodes, kind: "fillet", a: 2.0, b: 0.0, c: 0.0, flag: false, tbone: false })
        }
        "dogbone" => {
            if !sel_or_say(app) { return None; }
            Box::new(CornerTool { nodes: Vec::new(), kind: "dogbone", a: 3.0, b: 0.0, c: 0.0, flag: true, tbone: false })
        }
        "slotResize" => {
            if !sel_or_say(app) { return None; }
            Box::new(CornerTool { nodes: Vec::new(), kind: "slot", a: 3.0, b: 3.2, c: 0.05, flag: false, tbone: false })
        }
        "taper" => {
            if !sel_or_say(app) { return None; }
            Box::new(CornerTool { nodes: Vec::new(), kind: "taper", a: 70.0, b: 100.0, c: 0.0, flag: false, tbone: false })
        }
        "tangentCircles" => {
            let sel = app.doc.selected_in_order();
            if sel.len() != 3 {
                app.say(l("Seleziona esattamente 3 cerchi", "Select exactly 3 circles"));
                return None;
            }
            let sols = corners::apollonius([corners::circle_of(&sel[0]), corners::circle_of(&sel[1]), corners::circle_of(&sel[2])]);
            if sols.is_empty() {
                app.say(l("Nessun cerchio tangente possibile", "No tangent circle exists"));
                return None;
            }
            let layer = sel[0].layer();
            let circles: Vec<Shape> = sols.iter().map(|c| Shape(props(json!({"id": app.doc.new_id(), "type": "circle", "layer": layer, "cx": c[0], "cy": c[1], "rx": c[2], "ry": c[2]})))).collect();
            let n = circles.len();
            app.doc.add_shapes(circles, true);
            app.invalidate();
            app.say(l(&format!("{n} cerchi tangenti creati: elimina quelli che non servono"), &format!("{n} tangent circles created: delete the ones you don't need")));
            return None;
        }
        "booleanAssistant" => {
            let sel: Vec<Shape> = app.doc.selected_in_order().into_iter().filter(|s| s.kind() != "image").collect();
            if sel.len() != 2 {
                app.say(l("Seleziona esattamente 2 forme vettoriali (la prima è A)", "Select exactly 2 vector shapes (the first is A)"));
                return None;
            }
            let shapes: Vec<Vec<Vec<laserforge_core::model::Pt>>> = sel.iter().map(|s| s.paths().iter().map(|p| p.iter().map(|q| laserforge_core::model::Pt { x: q[0], y: q[1] }).collect()).collect()).collect();
            let ops = [("union", l("Unione", "Union")), ("subtract", "A − B".into()), ("subtract_rev", "B − A".into()), ("intersect", l("Intersezione", "Intersection")), ("xor", l("Esclusione", "Exclusion"))];
            let ops = ops.into_iter().map(|(op, label)| {
                let rings = laserforge_core::geometry::geometry_op(op, &shapes, &json!({})).unwrap_or_default().into_iter().flatten()
                    .map(|r| r.iter().map(|q| [q.x, q.y]).collect()).collect();
                (op.to_string(), label, rings)
            }).collect();
            Box::new(BoolAssistant { ops, pick: None })
        }
        "feeder" => Box::new(Feeder { m: app.machine.clone() }),
        "machineSettings" => Box::new(GrblSettings { vals: Vec::new(), loaded: Default::default() }),
        "calibrateAxis" => Box::new(Calibrate { axis: 100, steps: grbl_settings(app).get(&100).copied().unwrap_or(80.0), want: 100.0, got: 100.0 }),
        "controllerFiles" => {
            if !matches!(app.machine.flavor.as_str(), "fluidnc" | "grblhal") {
                app.say(l("I file sulla scheda sono disponibili con controller FluidNC o grblHAL (Impostazioni dispositivo › Controller)", "Controller files need a FluidNC or grblHAL controller (Device Settings › Controller)"));
                return None;
            }
            if !app.connected() {
                app.say(l("Connetti prima il laser", "Connect a laser first"));
                return None;
            }
            let asked = app.conn.as_ref().map_or(0, |c| c.shared.lock().unwrap().console.len());
            app.send(if app.machine.flavor == "fluidnc" { "$SD/List" } else { "$F" });
            Box::new(ControllerFiles { asked })
        }
        "devices" => Box::new(Devices { sel: None, name: l("Il mio laser", "My laser") }),
        "newDevice" => Box::new(NewDevice { detect: None, found: String::new(), name: l("Il mio laser", "My laser"), m: app.machine.clone(), usb: true, port: app.port.clone(), host: "192.168.0.1:23".into() }),
        "checklist" => Box::new(ChecklistEditor { on: app.prefs.checklist_enabled,
            text: if app.checklist.is_empty() { DEFAULT_CHECKLIST.join("\n") } else { app.checklist.join("\n") } }),
        "editHotkeys" => {
            let mut rows = Vec::new();
            fn walk(items: &[crate::defs::MenuItem], app: &LaserApp, rows: &mut Vec<(String, String, String)>) {
                for it in items {
                    match it {
                        crate::defs::MenuItem::Action { id, .. } => rows.push((id.clone(), crate::defs::label(id).replace("...", "…"), app.shortcut_of(id))),
                        crate::defs::MenuItem::Sub { items, .. } => walk(items, app, rows),
                        _ => {}
                    }
                }
            }
            for m in crate::defs::menus() {
                walk(&m.items, app, &mut rows);
            }
            Box::new(ShortcutsEditor { rows, filter: String::new() })
        }
        "openBackup" => {
            let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(backup_dir()).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "lfproj")).collect()).unwrap_or_default();
            files.sort();
            files.reverse();
            Box::new(OpenBackup { files, pick: None })
        }
        _ => return crate::camera_ui::for_action(aid, app),
    })
}

#[allow(dead_code)]
fn _unused() -> Option<[f64; 4]> {
    multi_bounds(&[])
}
