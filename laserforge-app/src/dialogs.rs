//! Dialogs (egui windows). Ported from laserforge/dialogs.py, props.py and devices.py: text, polygon, arrays,
//! offset, round corners, copy along path, image adjust / trace, device settings, optimisation, rotary,
//! cylinder, G-code preview with resume, shortcuts, about, undo history, project notes and more.

use crate::app::{fmt_time, LaserApp};
use crate::edit::{center, rotate_shape};
use crate::shapes::{multi_bounds, Shape, P};
use crate::{l, tr};
use egui::{vec2, Color32, Rect, Sense, Stroke};
use laserforge_core::model::{AirAssist, CutMode};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub const DITHER_MODES: [&str; 12] = ["dither", "jarvis", "stucki", "atkinson", "sierra", "bayer", "halftone", "newsprint", "sketch", "gray", "threshold", "slices3d"];

fn dither_name(k: &str) -> String {
    match k {
        "dither" => "Floyd–Steinberg".into(), "jarvis" => "Jarvis".into(), "stucki" => "Stucki".into(), "atkinson" => "Atkinson".into(),
        "sierra" => "Sierra".into(), "bayer" => l("Bayer (ordinato)", "Bayer (ordered)"), "halftone" => l("Mezzitoni", "Halftone"),
        "newsprint" => l("Giornale", "Newsprint"), "sketch" => l("Schizzo (contorni)", "Sketch (edges)"), "gray" => l("Scala di grigi", "Grayscale"),
        "threshold" => l("Soglia (bianco/nero)", "Threshold (black/white)"), "slices3d" => l("3D a strati (mappa di profondità)", "3D slices (depth map)"),
        _ => k.into(),
    }
}

#[derive(Clone)]
pub struct SubRow {
    enabled: bool,
    name: String,
    mode: String,
    speed: f64,
    power: f64,
    passes: i64,
}

pub enum Dialog {
    Message { title: String, text: String },
    ConfirmDiscard { then: String },
    ConfirmArea { over: f64, then: String, to_abs: bool },
    Quit,
    PdfPage { path: PathBuf, pages: usize, page: usize },
    Import3d { path: PathBuf, px_mm: f64, invert: bool },
    Text { id: Option<i64>, at: P, text: String, size: f64, font: String, single: String, bold: bool, italic: bool },
    Polygon { sides: usize },
    GridArray { cols: i64, rows: i64, dx: f64, dy: f64, live: bool },
    CircularArray { n: i64, angle: f64, cx: f64, cy: f64, rotate: bool, live: bool },
    Offset { distance: f64, direction: String, join: String, delete: bool },
    RoundCorners { radius: f64 },
    CopyAlongPath { n: i64, rotate: bool, delete: bool },
    AdjustImage { id: i64, adj: Map<String, Value>, preview: Option<egui::TextureHandle>, key: String, preset: String },
    TraceImage { id: i64, threshold: i64, smooth: f64, res: i64, delete: bool },
    Device { m: laserforge_core::model::MachineSettings, tab: usize },
    Optimize { o: crate::project::Opt },
    Rotary { m: laserforge_core::model::MachineSettings },
    Cylinder { c: Value },
    /// G-code simulation: moves, the time each one ends (s), the simulated time, playback speed (×).
    Preview { segs: Vec<(P, P, bool, f64, usize)>, times: Vec<f64>, t: f64, speed: f64, rapids: bool, playing: bool, last: Option<f64>, gcode: String },
    /// All the settings of one layer; `orig` is restored on Cancel.
    Layer { idx: usize, orig: Box<laserforge_core::model::LayerSettings> },
    Shortcuts,
    About,
    UndoHistory,
    Notes { text: String, show: bool },
    CloseTolerance { tol: f64 },
    ConvertBitmap { px: f64, fill: bool, stroke: f64, keep: bool },
    Nesting { true_shape: bool, spacing: f64, margin: f64, rot: i64 },
    Sublayers { layer: usize, rows: Vec<SubRow> },
    Settings { p: crate::prefs::Prefs },
    Generic(Box<dyn crate::dialogs2::Extra>),
}

impl Dialog {
    pub fn new_text(at: P, _layer: usize) -> Dialog {
        Dialog::Text { id: None, at, text: "LaserForge".into(), size: 20.0, font: "Helvetica".into(), single: String::new(), bold: false, italic: false }
    }
    pub fn edit_text(s: &Shape) -> Dialog {
        let font = s.0.get("font").and_then(|v| v.as_str()).unwrap_or("Helvetica").to_string();
        let single = if crate::text::is_single_line(s) { font.clone() } else { String::new() };
        Dialog::Text { id: Some(s.id()), at: [s.f("x"), s.f("y")], text: s.0.get("text").and_then(|v| v.as_str()).unwrap_or("").into(), size: s.f("size"),
                       font, single, bold: s.flag("bold"), italic: s.flag("italic") }
    }
    pub fn adjust_image(s: &Shape) -> Dialog {
        let mut adj: Map<String, Value> = serde_json::from_value(crate::shapes::default_adj()).unwrap();
        if let Some(Value::Object(a)) = s.0.get("adj") {
            adj.extend(a.clone());
        }
        Dialog::AdjustImage { id: s.id(), adj, preview: None, key: String::new(), preset: String::new() }
    }
    pub fn nesting() -> Dialog {
        Dialog::Nesting { true_shape: true, spacing: 2.0, margin: 3.0, rot: 90 }
    }
    pub fn sublayers(layer: usize, lay: &laserforge_core::model::LayerSettings) -> Dialog {
        let rows = lay.sublayers.iter().enumerate().map(|(k, s)| SubRow {
            enabled: s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            name: s.get("name").and_then(|v| v.as_str()).map(|x| x.to_string()).unwrap_or(format!("{}.{}", lay.name, k + 1)),
            mode: s.get("cut_mode").and_then(|v| v.as_str()).unwrap_or(lay.cut_mode.as_str()).to_string(),
            speed: s.get("speed").and_then(|v| v.as_f64()).unwrap_or(lay.speed),
            power: s.get("power_max").and_then(|v| v.as_f64()).unwrap_or(lay.power_max),
            passes: s.get("passes").and_then(|v| v.as_i64()).unwrap_or(1),
        }).collect();
        Dialog::Sublayers { layer, rows }
    }

    /// The dialog behind a menu action, when there is one.
    pub fn for_action(aid: &str, app: &mut LaserApp) -> Option<Dialog> {
        let need = |app: &mut LaserApp| -> bool {
            if app.doc.selected_shapes().iter().any(|s| s.kind() != "image") {
                true
            } else {
                app.say(tr("selectFirst"));
                false
            }
        };
        Some(match aid {
            "toolPolygon" => Dialog::Polygon { sides: app.polygon_sides },
            "gridArray" => {
                app.doc.need_sel(1).map_err(|e| app.say(e)).ok()?;
                Dialog::GridArray { cols: 3, rows: 2, dx: 5.0, dy: 5.0, live: true }
            }
            "circularArray" => {
                let sel = app.doc.need_sel(1).map_err(|e| app.say(e)).ok()?;
                let b = multi_bounds(&sel).unwrap();
                Dialog::CircularArray { n: 6, angle: 360.0, cx: b[0] + b[2] / 2.0, cy: b[1] + b[3] + 20.0, rotate: true, live: true }
            }
            "offset" => {
                if !need(app) { return None; }
                Dialog::Offset { distance: 2.0, direction: "out".into(), join: "round".into(), delete: false }
            }
            "roundCorners" => {
                if !need(app) { return None; }
                Dialog::RoundCorners { radius: 3.0 }
            }
            "copyAlongPath" => {
                if app.doc.selected.len() < 2 {
                    app.say(l("Seleziona gli oggetti e per ultimo il percorso", "Select the objects, then the path last"));
                    return None;
                }
                Dialog::CopyAlongPath { n: 8, rotate: true, delete: false }
            }
            "adjustImage" => {
                let id = app.doc.selected_image_id().map_err(|e| app.say(e)).ok()?;
                Dialog::adjust_image(app.doc.find(id).unwrap())
            }
            "traceImage" => {
                let id = app.doc.selected_image_id().map_err(|e| app.say(e)).ok()?;
                Dialog::TraceImage { id, threshold: 128, smooth: 0.6, res: 600, delete: false }
            }
            "deviceSettings" => Dialog::Device { m: app.machine.clone(), tab: 0 },
            "optimizeSettings" => Dialog::Optimize { o: app.opt.clone() },
            "rotarySetup" => Dialog::Rotary { m: app.machine.clone() },
            "cylinder" => {
                let mut c = json!({"enabled": false, "diameter": 80.0, "axis": "x", "centre": app.machine.workspace_y / 2.0});
                if let Value::Object(m) = &app.cylinder {
                    for (k, v) in m {
                        c[k] = v.clone();
                    }
                }
                Dialog::Cylinder { c }
            }
            "preview" => {
                if app.gcode.is_none() && !app.generate_gcode() {
                    return None;
                }
                let g = app.gcode.as_ref().unwrap().gcode.clone();
                let segs = parse_gcode(&g).into_iter().map(|(a, b, r, s, ln)| (crate::project::to_machine(&app.machine, a), crate::project::to_machine(&app.machine, b), r, s, ln)).collect::<Vec<_>>();
                let est = app.gcode.as_ref().map(|o| o.seconds).unwrap_or(0.0);
                let times = seg_times(&g, &segs, app.machine.max_speed.max(600.0), est);
                let total = times.last().copied().unwrap_or(0.0);
                // a whole simulation lasts about 30 s at the starting speed
                let speed = [1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0].into_iter().find(|v| total / v <= 30.0).unwrap_or(1000.0);
                Dialog::Preview { segs, times, t: total, speed, rapids: true, playing: false, last: None, gcode: g }
            }
            "shortcuts" => Dialog::Shortcuts,
            "about" => Dialog::About,
            "undoHistory" => Dialog::UndoHistory,
            "projectNotes" => Dialog::Notes { text: app.doc.notes.get("text").and_then(|v| v.as_str()).unwrap_or("").into(),
                                              show: app.doc.notes.get("show_on_load").and_then(|v| v.as_bool()).unwrap_or(false) },
            "closeTolerance" => {
                app.doc.need_sel(1).map_err(|e| app.say(e)).ok()?;
                Dialog::CloseTolerance { tol: 0.5 }
            }
            "convertBitmap" => {
                if !need(app) { return None; }
                Dialog::ConvertBitmap { px: app.doc.layer_interval(app.doc.active_layer), fill: true, stroke: 0.3, keep: false }
            }
            "settings" => Dialog::Settings { p: app.prefs.clone() },
            _ => return crate::dialogs2::for_action(aid, app).map(Dialog::Generic),
        })
    }

    fn title(&self) -> String {
        match self {
            Dialog::Message { title, .. } => title.clone(),
            Dialog::ConfirmDiscard { .. } | Dialog::Quit | Dialog::ConfirmArea { .. } => "LaserForge".into(),
            Dialog::PdfPage { .. } => l("Importa PDF", "Import PDF"),
            Dialog::Import3d { .. } => l("Importa modello 3D", "Import 3D model"),
            Dialog::Text { id, .. } => if id.is_some() { l("Modifica testo", "Edit Text") } else { l("Testo", "Text") },
            Dialog::Polygon { .. } => l("Poligono", "Polygon"),
            Dialog::GridArray { .. } => l("Copia a griglia", "Grid Array"),
            Dialog::CircularArray { .. } => l("Copia circolare", "Circular Array"),
            Dialog::Offset { .. } => l("Offset forme", "Offset Shapes"),
            Dialog::RoundCorners { .. } => crate::defs::label("roundCorners").replace("...", ""),
            Dialog::CopyAlongPath { .. } => crate::defs::label("copyAlongPath").replace("...", ""),
            Dialog::AdjustImage { .. } => l("Regola immagine", "Adjust Image"),
            Dialog::TraceImage { .. } => l("Traccia immagine", "Trace Image"),
            Dialog::Device { .. } => l("Impostazioni dispositivo", "Device Settings"),
            Dialog::Optimize { .. } => l("Impostazioni ottimizzazione", "Optimization Settings"),
            Dialog::Rotary { .. } => l("Configurazione rotativo", "Rotary Setup"),
            Dialog::Cylinder { .. } => l("Correzione cilindrica", "Cylinder Correction"),
            Dialog::Preview { .. } => tr("preview"),
            Dialog::Layer { idx, .. } => l(&format!("Impostazioni livello C{idx:02}"), &format!("Layer settings C{idx:02}")),
            Dialog::Shortcuts => crate::defs::label("shortcuts"),
            Dialog::About => crate::defs::label("about"),
            Dialog::UndoHistory => l("Cronologia annulla", "Undo History"),
            Dialog::Notes { .. } => l("Note del progetto", "Project notes"),
            Dialog::CloseTolerance { .. } => l("Chiudi forme con tolleranza", "Close Shapes with Tolerance"),
            Dialog::ConvertBitmap { .. } => l("Converti in bitmap", "Convert to Bitmap"),
            Dialog::Nesting { .. } => l("Disponi automaticamente (nesting)", "Nesting"),
            Dialog::Sublayers { layer, .. } => l(&format!("Sotto-layer di C{layer:02}"), &format!("Sub-layers of C{layer:02}")),
            Dialog::Settings { .. } => l("Impostazioni", "Settings"),
            Dialog::Generic(g) => g.title(),
        }
    }
}

/// OK / Cancel row; returns Some(true) for OK, Some(false) for Cancel.
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
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        r = Some(false);
    }
    r
}

fn hint(ui: &mut egui::Ui, t: &str) {
    ui.add(egui::Label::new(egui::RichText::new(t).weak().size(11.0)).wrap());
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

/// dialogs.parse_gcode: (from, to, rapid, S, line index) per move, machine coordinates.
pub fn parse_gcode(g: &str) -> Vec<(P, P, bool, f64, usize)> {
    let (mut x, mut y, mut s, mut rel) = (0.0f64, 0.0f64, 0.0f64, false);
    let mut out = Vec::new();
    for (ln, line) in g.split('\n').enumerate() {
        let t = line.split(';').next().unwrap_or("").trim();
        if t.is_empty() {
            continue;
        }
        let up = t.to_uppercase();
        if up.contains("G91") {
            rel = true;
        }
        if up.contains("G90") {
            rel = false;
        }
        let mut words: std::collections::HashMap<char, String> = Default::default();
        let mut cur: Option<char> = None;
        for ch in up.chars() {
            if ch.is_ascii_alphabetic() {
                cur = Some(ch);
                words.insert(ch, String::new());
            } else if let Some(c) = cur {
                if ch.is_ascii_digit() || ch == '.' || ch == '-' {
                    words.get_mut(&c).unwrap().push(ch);
                } else if !ch.is_whitespace() {
                    cur = None;
                }
            }
        }
        let f = |c: char| words.get(&c).and_then(|v| v.parse::<f64>().ok());
        if let Some(v) = f('S') {
            s = v;
        }
        if t.starts_with("M5") || t.starts_with("M2") {
            s = 0.0;
        }
        let gnum = words.get(&'G').and_then(|v| v.parse::<f64>().ok());
        if !matches!(gnum, Some(v) if v == 0.0 || v == 1.0 || v == 2.0 || v == 3.0) {
            continue;
        }
        let (nx, ny) = if rel { (x + f('X').unwrap_or(0.0), y + f('Y').unwrap_or(0.0)) } else { (f('X').unwrap_or(x), f('Y').unwrap_or(y)) };
        if nx != x || ny != y {
            out.push(([x, y], [nx, ny], gnum == Some(0.0) || s == 0.0, s, ln));
        }
        x = nx;
        y = ny;
    }
    out
}

/// Time (s) at which each move ends: distance at the programmed feed (F), rapids at `rapid` mm/min,
/// then scaled so the whole job matches the generator's estimate `est` (which knows accelerations).
pub fn seg_times(gcode: &str, segs: &[(P, P, bool, f64, usize)], rapid: f64, est: f64) -> Vec<f64> {
    let lines: Vec<&str> = gcode.split('\n').collect();
    let mut feed_at = vec![1000.0f64; lines.len() + 1];
    let mut f = 1000.0f64;
    for (i, line) in lines.iter().enumerate() {
        let t = line.split(';').next().unwrap_or("").to_ascii_uppercase();
        if let Some(pos) = t.find('F') {
            let num: String = t[pos + 1..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(v) = num.parse::<f64>() {
                if v > 0.0 {
                    f = v;
                }
            }
        }
        feed_at[i] = f;
    }
    let mut out = Vec::with_capacity(segs.len());
    let mut acc = 0.0;
    for s in segs {
        let d = (s.1[0] - s.0[0]).hypot(s.1[1] - s.0[1]);
        let feed = if s.2 { rapid } else { feed_at.get(s.4).copied().unwrap_or(f) };
        acc += d / (feed.max(1.0) / 60.0);
        out.push(acc);
    }
    if est > 0.0 && acc > 0.0 {
        let k = est / acc;
        for v in &mut out {
            *v *= k;
        }
    }
    out
}

impl Dialog {
    /// Draw the dialog; false when it closed.
    fn show(&mut self, ctx: &egui::Context, app: &mut LaserApp) -> bool {
        let title = self.title();
        let mut open = true;
        let mut res: Option<bool> = None;
        let wide = matches!(self, Dialog::AdjustImage { .. } | Dialog::Preview { .. } | Dialog::Layer { .. } | Dialog::Shortcuts | Dialog::Device { .. } | Dialog::Sublayers { .. });
        egui::Window::new(title).collapsible(false).resizable(true)
            .default_width(if wide { 720.0 } else { 380.0 }).open(&mut open).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                res = self.body(ui, app);
            });
        if !open {
            return false;
        }
        match res {
            Some(true) => !self.apply(app),
            Some(false) => false,
            None => true,
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        match self {
            Dialog::Message { text, .. } => {
                ui.add(egui::Label::new(text.as_str()).wrap());
                ui.separator();
                ui.button(tr("close")).clicked().then_some(false)
            }
            Dialog::ConfirmDiscard { .. } | Dialog::Quit => {
                ui.label(tr("unsaved"));
                ui.separator();
                let mut r = None;
                ui.horizontal(|ui| {
                    if ui.button(l("Salva", "Save")).clicked() {
                        r = Some(app.save_project(false));
                    }
                    if ui.button(l("Non salvare", "Don't save")).clicked() {
                        r = Some(true);
                    }
                    if ui.button(tr("cancel")).clicked() {
                        r = Some(false);
                    }
                });
                r
            }
            Dialog::ConfirmArea { over, to_abs, .. } => {
                ui.label(l(&format!("Il lavoro esce dall'area di lavoro di {over:.1} mm: la testa andrebbe contro i finecorsa."),
                           &format!("The job goes {over:.1} mm outside the work area: the head would hit the limit switches.")));
                let sf = app.start_from.clone();
                let name = match sf.as_str() { "current" => tr("sfCurrent"), "user" => tr("sfUser"), _ => tr("sfAbsolute") };
                ui.label(l(&format!("«Avvia da» è impostato su «{name}». Con «Assolute» il lavoro va esattamente dove l'hai messo sulla griglia."),
                           &format!("“Start from” is set to “{name}”. With “Absolute” the job goes exactly where you placed it on the grid.")));
                ui.separator();
                let mut r = None;
                ui.horizontal(|ui| {
                    if sf != "absolute" && ui.button(egui::RichText::new(l("Usa «Assolute»", "Use “Absolute”")).strong()).clicked() {
                        *to_abs = true;
                        r = Some(true);
                    }
                    if ui.button(l("Continua comunque", "Continue anyway")).clicked() {
                        r = Some(true);
                    }
                    if ui.button(tr("cancel")).clicked() {
                        r = Some(false);
                    }
                });
                r
            }
            Dialog::PdfPage { pages, page, .. } => {
                ui.label(l(&format!("Il file ha {pages} pagine. Quale importare?"), &format!("The file has {pages} pages. Which one?")));
                ui.add(egui::DragValue::new(page).range(1..=*pages));
                buttons(ui, "OK")
            }
            Dialog::Import3d { px_mm, invert, .. } => {
                egui::Grid::new("3d").num_columns(2).show(ui, |ui| {
                    dv(ui, &l("Risoluzione (mm/pixel)", "Resolution (mm/pixel)"), px_mm, 0.02..=2.0, 0.01);
                    ui.label("");
                    ui.checkbox(invert, l("Inverti profondità", "Invert depth"));
                    ui.end_row();
                });
                hint(ui, &l("Il modello diventa una mappa di profondità (immagine in scala di grigi) da incidere con la modalità 3D a strati o Scala di grigi.",
                            "The model becomes a depth map (grayscale image) to engrave with 3D slices or Grayscale mode."));
                buttons(ui, &l("Importa", "Import"))
            }
            Dialog::Text { id, text, size, font, single, bold, italic, .. } => {
                egui::Grid::new("text").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label(l("Testo", "Text"));
                    let r = ui.text_edit_singleline(text);
                    if id.is_none() && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.end_row();
                    dv(ui, &l("Altezza (mm)", "Height (mm)"), size, 0.5..=500.0, 0.5);
                    ui.label(l("Tratto singolo", "Single-line"));
                    let lab = if single.is_empty() { l("— no (contorno lettere) —", "— no (letter outlines) —") } else { single.split(':').last().unwrap_or("").to_string() };
                    egui::ComboBox::from_id_salt("single").selected_text(lab).width(220.0).show_ui(ui, |ui| {
                        ui.selectable_value(single, String::new(), l("— no (contorno lettere) —", "— no (letter outlines) —"));
                        for (fid, lab) in crate::shx::single_line_fonts() {
                            ui.selectable_value(single, fid, lab);
                        }
                    });
                    ui.end_row();
                    ui.label("Font");
                    ui.add_enabled_ui(single.is_empty(), |ui| {
                        egui::ComboBox::from_id_salt("font").selected_text(font.as_str()).width(220.0).height(400.0).show_ui(ui, |ui| {
                            for f in crate::text::families() {
                                ui.selectable_value(font, f.clone(), f);
                            }
                        });
                    });
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.checkbox(bold, l("Grassetto", "Bold"));
                        ui.checkbox(italic, l("Corsivo", "Italic"));
                    });
                    ui.end_row();
                });
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                buttons(ui, &if id.is_some() { tr("apply") } else { tr("create") }).or(enter.then_some(true))
            }
            Dialog::Polygon { sides } => {
                ui.horizontal(|ui| {
                    ui.label(l("Lati", "Sides"));
                    ui.add(egui::DragValue::new(sides).range(3..=64));
                });
                buttons(ui, "OK")
            }
            Dialog::GridArray { cols, rows, dx, dy, live } => {
                egui::Grid::new("ga").num_columns(2).show(ui, |ui| {
                    iv(ui, &l("Colonne", "Columns"), cols, 1..=100);
                    iv(ui, &l("Righe", "Rows"), rows, 1..=100);
                    dv(ui, &l("Spaziatura orizz. (mm)", "Horizontal spacing (mm)"), dx, -1000.0..=1000.0, 0.5);
                    dv(ui, &l("Spaziatura vert. (mm)", "Vertical spacing (mm)"), dy, -1000.0..=1000.0, 0.5);
                });
                ui.checkbox(live, l("Array dinamico (modificabile dal pannello Proprietà forma)", "Live array (editable from Shape Properties)"));
                buttons(ui, &tr("create"))
            }
            Dialog::CircularArray { n, angle, cx, cy, rotate, live } => {
                egui::Grid::new("ca").num_columns(2).show(ui, |ui| {
                    iv(ui, &l("Copie", "Copies"), n, 2..=360);
                    dv(ui, &l("Angolo totale", "Total angle"), angle, -360.0..=360.0, 1.0);
                    dv(ui, &l("Centro X", "Center X"), cx, -10000.0..=10000.0, 1.0);
                    dv(ui, &l("Centro Y", "Center Y"), cy, -10000.0..=10000.0, 1.0);
                });
                ui.checkbox(rotate, l("Ruota le copie", "Rotate copies"));
                ui.checkbox(live, l("Array dinamico (modificabile dal pannello Proprietà forma)", "Live array (editable from Shape Properties)"));
                buttons(ui, &tr("create"))
            }
            Dialog::Offset { distance, direction, join, delete } => {
                egui::Grid::new("off").num_columns(2).show(ui, |ui| {
                    dv(ui, &l("Distanza (mm)", "Distance (mm)"), distance, 0.01..=500.0, 0.1);
                    ui.label(l("Direzione", "Direction"));
                    ui.horizontal(|ui| {
                        for (k, t) in [("out", l("Esterno", "Outward")), ("in", l("Interno", "Inward")), ("both", l("Entrambi", "Both"))] {
                            ui.selectable_value(direction, k.to_string(), t);
                        }
                    });
                    ui.end_row();
                    ui.label(l("Angoli", "Corners"));
                    ui.horizontal(|ui| {
                        for (k, t) in [("round", l("Arrotondati", "Round")), ("mitre", l("Vivi", "Sharp")), ("bevel", l("Smussati", "Bevel"))] {
                            ui.selectable_value(join, k.to_string(), t);
                        }
                    });
                    ui.end_row();
                });
                ui.checkbox(delete, l("Elimina le forme originali", "Delete original shapes"));
                buttons(ui, "OK")
            }
            Dialog::RoundCorners { radius } => {
                egui::Grid::new("rc").num_columns(2).show(ui, |ui| dv(ui, &l("Raggio (mm)", "Radius (mm)"), radius, 0.1..=500.0, 0.1));
                buttons(ui, "OK")
            }
            Dialog::CopyAlongPath { n, rotate, delete } => {
                egui::Grid::new("cap").num_columns(2).show(ui, |ui| iv(ui, &l("Copie", "Copies"), n, 2..=500));
                ui.checkbox(rotate, l("Ruota seguendo il percorso", "Rotate to follow the path"));
                ui.checkbox(delete, l("Elimina gli originali", "Delete the originals"));
                buttons(ui, &tr("create"))
            }
            Dialog::AdjustImage { id, adj, preview, key, preset } => {
                let Some(s) = app.doc.find(*id).cloned() else { return Some(false) };
                ui.horizontal_top(|ui| {
                    let k = serde_json::to_string(adj).unwrap();
                    if *key != k || preview.is_none() {
                        *key = k;
                        let mut s2 = s.clone();
                        s2.0.insert("adj".into(), Value::Object(adj.clone()));
                        let interval = app.doc.layer_interval(s.layer().max(0) as usize).max(s.f("w") / 500.0);
                        if let Some(pic) = s2.0.get("src").and_then(|v| v.as_str()).and_then(laserforge_core::imaging::decode) {
                            let a: laserforge_core::imaging::Adj = serde_json::from_value(Value::Object(adj.clone())).unwrap_or_default();
                            let (burn, c, r) = laserforge_core::imaging::process(&pic, s.f("w"), s.f("h"), interval, 500, false, s.flag("flipX"), s.flag("flipY"), &a);
                            let grey: Vec<u8> = burn.iter().map(|b| 255 - b).collect();
                            *preview = Some(ui.ctx().load_texture("adjprev", egui::ColorImage::from_gray([c, r], &grey), egui::TextureOptions::LINEAR));
                        }
                    }
                    if let Some(t) = preview {
                        let sz = t.size_vec2();
                        let k = (320.0 / sz.x).min(320.0 / sz.y);
                        ui.image((t.id(), sz * k));
                    }
                    ui.vertical(|ui| {
                        let mut mode = adj["mode"].as_str().unwrap_or("dither").to_string();
                        egui::Grid::new("adj").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                            ui.label(l("Modalità", "Mode"));
                            egui::ComboBox::from_id_salt("mode").selected_text(dither_name(&mode)).width(220.0).show_ui(ui, |ui| {
                                for k in DITHER_MODES {
                                    ui.selectable_value(&mut mode, k.to_string(), dither_name(k));
                                }
                            });
                            ui.end_row();
                            let mut f = |ui: &mut egui::Ui, k: &str, label: &str, range: std::ops::RangeInclusive<f64>, speed: f64, enabled: bool| {
                                let mut v = adj.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                                ui.label(label);
                                if ui.add_enabled(enabled, egui::DragValue::new(&mut v).range(range).speed(speed).max_decimals(2)).changed() {
                                    adj.insert(k.into(), json!(v));
                                }
                                ui.end_row();
                            };
                            let m = mode.as_str();
                            f(ui, "cells_per_inch", &l("Celle per pollice", "Cells per inch"), 5.0..=200.0, 1.0, matches!(m, "halftone" | "newsprint"));
                            f(ui, "halftone_angle", &l("Angolo retino", "Screen angle"), 0.0..=90.0, 1.0, m == "halftone");
                            f(ui, "slices", &l("Strati 3D", "3D slices"), 2.0..=100.0, 1.0, m == "slices3d");
                            f(ui, "brightness", &l("Luminosità", "Brightness"), -100.0..=100.0, 1.0, true);
                            f(ui, "contrast", &l("Contrasto", "Contrast"), -100.0..=100.0, 1.0, true);
                            f(ui, "gamma", "Gamma", 0.1..=5.0, 0.02, true);
                            f(ui, "enhance_amount", &l("Nitidezza: quantità %", "Enhance amount %"), 0.0..=500.0, 1.0, true);
                            f(ui, "enhance_radius", &l("Nitidezza: raggio", "Enhance radius"), 0.0..=10.0, 0.05, true);
                            f(ui, "enhance_denoise", &l("Nitidezza: riduzione rumore", "Enhance denoise"), 0.0..=50.0, 1.0, true);
                            f(ui, "threshold", &l("Soglia", "Threshold"), 1.0..=254.0, 1.0, matches!(m, "threshold" | "sketch"));
                            ui.label("");
                            let mut inv = adj.get("invert").and_then(|v| v.as_bool()).unwrap_or(false);
                            if ui.checkbox(&mut inv, l("Inverti", "Invert")).changed() {
                                adj.insert("invert".into(), json!(inv));
                            }
                            ui.end_row();
                        });
                        adj.insert("mode".into(), json!(mode));
                        ui.horizontal(|ui| {
                            ui.label("Preset");
                            let names: Vec<String> = app.image_presets.keys().cloned().collect();
                            egui::ComboBox::from_id_salt("ipreset").selected_text(if preset.is_empty() { "— preset —".to_string() } else { preset.clone() }).show_ui(ui, |ui| {
                                for n in names {
                                    if ui.selectable_label(*preset == n, &n).clicked() {
                                        *preset = n.clone();
                                        if let Some(Value::Object(v)) = app.image_presets.get(&n) {
                                            adj.extend(v.clone());
                                        }
                                    }
                                }
                            });
                            ui.add(egui::TextEdit::singleline(preset).desired_width(100.0).hint_text(l("Nome", "Name")));
                            if ui.button(l("Salva", "Save")).clicked() && !preset.trim().is_empty() {
                                app.image_presets.insert(preset.trim().to_string(), Value::Object(adj.clone()));
                            }
                            if ui.button(l("Elimina", "Delete")).clicked() {
                                app.image_presets.remove(preset.trim());
                                preset.clear();
                            }
                        });
                        let iv = app.doc.layer_interval(s.layer().max(0) as usize);
                        hint(ui, &l(&format!("La risoluzione segue l'intervallo del layer ({iv} mm)."), &format!("Resolution follows the layer interval ({iv} mm).")));
                    });
                });
                buttons(ui, &tr("apply"))
            }
            Dialog::TraceImage { threshold, smooth, res, delete, .. } => {
                egui::Grid::new("tr").num_columns(2).show(ui, |ui| {
                    iv(ui, &l("Soglia", "Cutoff"), threshold, 1..=254);
                    dv(ui, &l("Smussatura", "Smoothness"), smooth, 0.0..=5.0, 0.05);
                    iv(ui, &l("Risoluzione max (px)", "Max resolution (px)"), res, 100..=2000);
                });
                ui.checkbox(delete, l("Elimina immagine dopo la traccia", "Delete image after tracing"));
                buttons(ui, &l("Traccia", "Trace"))
            }
            Dialog::Device { m, tab } => {
                ui.horizontal(|ui| {
                    for (i, t) in [l("Generale", "General"), l("Aria", "Air"), l("Movimento", "Motion"), l("Immagini", "Images"), "G-code".to_string()].iter().enumerate() {
                        ui.selectable_value(tab, i, t);
                    }
                });
                ui.separator();
                device_tab(ui, m, *tab);
                buttons(ui, "OK")
            }
            Dialog::Optimize { o } => {
                egui::Grid::new("opt").num_columns(2).show(ui, |ui| {
                    ui.label(l("Ordine", "Order"));
                    ui.horizontal(|ui| {
                        for (k, t) in [("layer", l("Per layer", "By layer")), ("group", l("Per gruppo", "By group")), ("priority", l("Per priorità", "By priority"))] {
                            ui.selectable_value(&mut o.order, k.to_string(), t);
                        }
                    });
                    ui.end_row();
                });
                ui.checkbox(&mut o.inner_first, l("Taglia prima le forme interne", "Cut inner shapes first"));
                ui.checkbox(&mut o.nearest, l("Ordina per vicinanza (riduce gli spostamenti)", "Order by nearest (reduce travel)"));
                ui.checkbox(&mut o.best_start, l("Scegli il punto d'inizio migliore (forme chiuse)", "Choose best starting point (closed shapes)"));
                ui.checkbox(&mut o.best_direction, l("Scegli la direzione migliore (linee aperte)", "Choose best direction (open lines)"));
                ui.horizontal(|ui| {
                    ui.label(l("Direzione forme chiuse", "Closed shape direction"));
                    for (k, t) in [("keep", l("Originale", "As drawn")), ("cw", l("Oraria", "Clockwise")), ("ccw", l("Antioraria", "Counter-CW"))] {
                        ui.selectable_value(&mut o.direction, k.to_string(), t);
                    }
                });
                ui.checkbox(&mut o.remove_overlap, l("Rimuovi le linee sovrapposte", "Remove overlapping lines"));
                hint(ui, &l("Per layer: un layer alla volta (C00, C01…). Per gruppo: ogni oggetto completo prima del successivo. Per priorità: le forme con priorità più bassa per prime (la priorità si imposta in Proprietà forma). Le immagini vengono sempre incise per prime.",
                            "By layer: one layer at a time (C00, C01…). By group: each object complete before the next. By priority: lower priority first (set it in Shape Properties). Images are always engraved first."));
                buttons(ui, "OK")
            }
            Dialog::Rotary { m } => {
                ui.checkbox(&mut m.rotary_enabled, l("Abilita rotativo", "Enable rotary"));
                egui::Grid::new("rot").num_columns(2).show(ui, |ui| {
                    ui.label(l("Asse del rotativo", "Rotary axis"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut m.rotary_axis, "Y".to_string(), l("Al posto di Y", "Replaces Y"));
                        ui.selectable_value(&mut m.rotary_axis, "A".to_string(), l("Asse A", "A axis"));
                    });
                    ui.end_row();
                    ui.label(l("Tipo", "Type"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut m.rotary_type, "roller".to_string(), l("A rulli", "Roller"));
                        ui.selectable_value(&mut m.rotary_type, "chuck".to_string(), l("Mandrino", "Chuck"));
                    });
                    ui.end_row();
                    dv(ui, &l("Movimento Y per giro (mm)", "Travel per rotation (mm)"), &mut m.rotary_mm_per_rev, 0.1..=10000.0, 0.1);
                    dv(ui, &l("Diametro oggetto (mm)", "Object diameter (mm)"), &mut m.rotary_diameter, 0.1..=2000.0, 0.1);
                });
                hint(ui, &l("Gli spostamenti Y vengono scalati: una circonferenza dell'oggetto corrisponde a un giro.", "Y moves are scaled so one object circumference equals one rotation."));
                buttons(ui, "OK")
            }
            Dialog::Cylinder { c } => {
                let mut on = c["enabled"].as_bool().unwrap_or(false);
                ui.checkbox(&mut on, l("Abilita correzione cilindrica", "Enable cylinder correction"));
                c["enabled"] = json!(on);
                let g = |k: &str, d: f64| c.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
                let (mut dia, mut centre, mut mirror) = (g("diameter", 80.0), g("centre", 0.0), g("mirror", 0.0));
                let mut axis = c["axis"].as_str().unwrap_or("x").to_string();
                egui::Grid::new("cyl").num_columns(2).show(ui, |ui| {
                    dv(ui, &l("Diametro oggetto (mm)", "Object diameter (mm)"), &mut dia, 1.0..=1000.0, 0.5);
                    ui.label(l("Asse del cilindro", "Cylinder axis"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut axis, "x".into(), l("Orizzontale (lungo X)", "Horizontal (along X)"));
                        ui.selectable_value(&mut axis, "y".into(), l("Verticale (lungo Y)", "Vertical (along Y)"));
                    });
                    ui.end_row();
                    dv(ui, &l("Linea più alta del cilindro (mm)", "Cylinder top line (mm)"), &mut centre, -5000.0..=5000.0, 0.5);
                    dv(ui, &l("Distanza specchio (galvo, 0 = portale)", "Mirror distance (galvo, 0 = gantry)"), &mut mirror, 0.0..=2000.0, 1.0);
                });
                if ui.button(l("Usa la posizione del laser", "Use laser position")).clicked() {
                    if let Some(h) = app.head_canvas() {
                        centre = if axis == "x" { h[1] } else { h[0] };
                    }
                }
                c["diameter"] = json!(dia);
                c["centre"] = json!(centre);
                c["mirror"] = json!(mirror);
                c["axis"] = json!(axis);
                hint(ui, &l("Per incidere su bicchieri, bottiglie e tubi appoggiati sul piano, senza rotativo: la grafica viene compressa verso i bordi in modo che sulla superficie curva mantenga la sua misura. Le immagini (raster) non vengono corrette.",
                            "For engraving tumblers, bottles and tubes lying on the bed, without a rotary: the artwork is compressed toward the edges so it keeps its size on the curved surface. Raster images are not corrected."));
                buttons(ui, "OK")
            }
            Dialog::Layer { idx, orig } => {
                let i = *idx;
                egui::ScrollArea::vertical().max_height(600.0).show(ui, |ui| app.layer_editor(ui, i));
                let r = buttons(ui, "OK");
                if r == Some(false) {
                    app.doc.layers[i] = (**orig).clone();
                    app.invalidate();
                }
                r
            }
            Dialog::Preview { segs, times, t, speed, rapids, playing, last, gcode } => {
                let total_t = times.last().copied().unwrap_or(0.0);
                // advance the simulated time with the real clock
                let now = ui.input(|i| i.time);
                if *playing {
                    if let Some(prev) = *last {
                        *t = (*t + (now - prev) * *speed).min(total_t);
                    }
                    *last = Some(now);
                    if *t >= total_t {
                        *playing = false;
                    }
                    ui.ctx().request_repaint();
                } else {
                    *last = None;
                }
                let n = times.partition_point(|&e| e <= *t);
                let (w, h) = app.ws();
                let size = vec2(ui.available_width().max(680.0), 440.0);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 0.0, Color32::from_rgb(0xe9, 0xe9, 0xec));
                let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, w, h);
                for s in segs.iter().filter(|s| !s.2) {
                    for q in [s.0, s.1] {
                        x0 = x0.min(q[0]);
                        y0 = y0.min(q[1]);
                        x1 = x1.max(q[0]);
                        y1 = y1.max(q[1]);
                    }
                }
                let k = ((rect.width() - 20.0) as f64 / (x1 - x0).max(1e-6)).min((rect.height() - 20.0) as f64 / (y1 - y0).max(1e-6));
                let o = vec2(((rect.width() as f64 - (x1 - x0) * k) / 2.0 - x0 * k) as f32, ((rect.height() as f64 - (y1 - y0) * k) / 2.0 - y0 * k) as f32);
                let tp = |q: P| rect.min + o + vec2((q[0] * k) as f32, (q[1] * k) as f32);
                p.rect_filled(Rect::from_min_max(tp([0.0, 0.0]), tp([w, h])), 0.0, Color32::WHITE);
                p.rect_stroke(Rect::from_min_max(tp([0.0, 0.0]), tp([w, h])), 0.0, Stroke::new(1.5, Color32::from_gray(0x66)), egui::StrokeKind::Middle);
                let max_s = segs.iter().map(|s| s.3).fold(1.0, f64::max);
                let inside = |a: P, b: P| a[0].min(b[0]) >= -0.01 && a[0].max(b[0]) <= w + 0.01 && a[1].min(b[1]) >= -0.01 && a[1].max(b[1]) <= h + 0.01;
                let mut outside = 0;
                let mut head: Option<P> = None;
                for (i, s) in segs.iter().enumerate() {
                    let ok = inside(s.0, s.1);
                    if !s.2 && !ok {
                        outside += 1;
                    }
                    let start = if i == 0 { 0.0 } else { times[i - 1] };
                    if start > *t {
                        continue;
                    }
                    // the move in progress is drawn up to where the head is now
                    let end = if times[i] <= *t { s.1 } else {
                        let f = ((*t - start) / (times[i] - start).max(1e-9)).clamp(0.0, 1.0);
                        let q = [s.0[0] + (s.1[0] - s.0[0]) * f, s.0[1] + (s.1[1] - s.0[1]) * f];
                        head = Some(q);
                        q
                    };
                    if s.2 {
                        if *rapids {
                            p.extend(egui::Shape::dashed_line(&[tp(s.0), tp(end)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(230, 40, 40, 90)), 3.0, 3.0));
                        }
                    } else if !ok {
                        p.line_segment([tp(s.0), tp(end)], Stroke::new(2.0, Color32::from_rgb(0xe0, 0x1b, 0x1b)));
                    } else {
                        let g = (200.0 - 200.0 * s.3 / max_s).round() as u8;
                        p.line_segment([tp(s.0), tp(end)], Stroke::new(1.0, Color32::from_gray(g)));
                    }
                }
                if head.is_none() && n > 0 && n < segs.len() {
                    head = Some(segs[n - 1].1);
                }
                if let Some(q) = head.filter(|_| *t < total_t) {
                    p.circle_filled(tp(q), 4.5, Color32::from_rgb(0xe0, 0x1b, 0x1b));
                    p.circle_stroke(tp(q), 7.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(224, 27, 27, 120)));
                }
                if outside > 0 {
                    p.text(rect.min + vec2(12.0, 12.0), egui::Align2::LEFT_TOP, l("Parte del lavoro è FUORI dall'area di lavoro (in rosso)", "Part of the job is OUTSIDE the work area (in red)"),
                           egui::FontId::proportional(13.0), Color32::from_rgb(0xc6, 0x28, 0x28));
                }
                let total = segs.len();
                ui.horizontal(|ui| {
                    if *playing {
                        if ui.button(l("Pausa", "Pause")).clicked() {
                            *playing = false;
                        }
                    } else if ui.button(format!("▶ {}", l("Simula", "Simulate"))).clicked() {
                        if *t >= total_t {
                            *t = 0.0;
                        }
                        *playing = true;
                    }
                    if ui.button("|◀").on_hover_text(l("Dall'inizio", "From the start")).clicked() {
                        *t = 0.0;
                    }
                    ui.add(egui::Slider::new(t, 0.0..=total_t.max(1e-6)).show_value(false));
                    ui.label(l("Velocità", "Speed"));
                    egui::ComboBox::from_id_salt("simspeed").selected_text(format!("{speed}×")).width(70.0).show_ui(ui, |ui| {
                        for v in [1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0] {
                            ui.selectable_value(speed, v, format!("{v}×"));
                        }
                    });
                    ui.checkbox(rapids, l("Mostra spostamenti", "Show rapids"));
                });
                let dist: f64 = segs[..n].iter().filter(|s| !s.2).map(|s| (s.1[0] - s.0[0]).hypot(s.1[1] - s.0[1])).sum();
                ui.horizontal(|ui| {
                    ui.strong(format!("{} {}", l("Trascorso", "Elapsed"), fmt_time(*t)));
                    ui.separator();
                    ui.strong(format!("{} {}", l("Rimanente", "Remaining"), fmt_time((total_t - *t).max(0.0))));
                    ui.separator();
                    ui.label(format!("{} {}", l("Totale", "Total"), fmt_time(total_t)));
                    ui.separator();
                    ui.weak(format!("{n}/{total} {} · {dist:.0} mm {}", l("movimenti", "moves"), l("di taglio", "burned")));
                });
                let n = n.max(1);
                let mut close = None;
                ui.horizontal(|ui| {
                    let from_here = |n: usize| (!segs.is_empty()).then(|| laserforge_core::post::resume_from(gcode, segs[n.min(segs.len() - 1)].4));
                    if ui.button(format!("▶ {}", l("Avvia da qui", "Start from here"))).on_hover_text(l("Riprende il lavoro dal punto del cursore: utile dopo un'interruzione.", "Resumes the job from the slider position: useful after an interruption.")).clicked() {
                        if let Some(g) = from_here(n) {
                            if app.connected() {
                                if let Some(c) = &app.conn {
                                    c.stream(g);
                                }
                                close = Some(false);
                            } else {
                                app.say(l("Connetti prima il laser", "Connect a laser first"));
                            }
                        }
                    }
                    if ui.button(l("Salva G-code da qui...", "Save G-code from here...")).clicked() {
                        if let Some(g) = from_here(n) {
                            let name = format!("{}-da-qui.gcode", app.base_name());
                            app.save_text_file(&g, &name, ("G-code", &["gcode", "nc"]));
                        }
                    }
                    if ui.button(tr("close")).clicked() {
                        close = Some(false);
                    }
                });
                close
            }
            Dialog::Shortcuts => {
                let mut rows: Vec<(String, String)> = Vec::new();
                fn walk(items: &[crate::defs::MenuItem], app: &LaserApp, rows: &mut Vec<(String, String)>) {
                    for it in items {
                        match it {
                            crate::defs::MenuItem::Action { id, .. } => {
                                let k = app.shortcut_of(id);
                                if !k.is_empty() {
                                    rows.push((crate::defs::label(id).replace("...", "…"), k));
                                }
                            }
                            crate::defs::MenuItem::Sub { items, .. } => walk(items, app, rows),
                            _ => {}
                        }
                    }
                }
                for m in crate::defs::menus() {
                    walk(&m.items, app, &mut rows);
                }
                rows.extend([(l("Sposta vista", "Pan view"), l("Spazio + trascina / tasto centrale", "Space + drag / middle button")), ("Zoom".into(), l("Rotella / pizzico", "Wheel / pinch")),
                             (l("Sposta selezione", "Nudge selection"), l("Frecce (Shift = 10 mm)", "Arrows (Shift = 10 mm)")), (l("Attiva/disattiva snap", "Toggle snap"), "G".into()),
                             (l("Sposta senza aggancio", "Move without snapping"), l("tieni premuto ⌥", "hold ⌥"))]);
                egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                    egui::Grid::new("sc").striped(true).num_columns(2).show(ui, |ui| {
                        for (a, b) in rows {
                            ui.label(a);
                            ui.monospace(b);
                            ui.end_row();
                        }
                    });
                });
                ui.button(tr("close")).clicked().then_some(false)
            }
            Dialog::About => {
                ui.heading(format!("LaserForge {}", env!("CARGO_PKG_VERSION")));
                hint(ui, &l("Software di taglio e incisione laser per controller GRBL con aria compressa nativa (M7/M8/M9). Versione Rust (egui).",
                            "Laser cutting and engraving software for GRBL controllers with native air assist (M7/M8/M9). Rust version (egui)."));
                ui.hyperlink_to(l("Icona dell'app: \"CNC Machine\" di Icons8", "App icon: \"CNC Machine\" by Icons8"), "https://icons8.com");
                hint(ui, &l("Font Hershey: pacchetto Hershey-Fonts (licenza MIT).", "Hershey fonts: Hershey-Fonts package (MIT licence)."));
                ui.button(tr("close")).clicked().then_some(false)
            }
            Dialog::UndoHistory => {
                let undo: Vec<String> = app.doc.undo.iter().map(|u| u.1.clone()).collect();
                let redo: Vec<String> = app.doc.redo.iter().rev().map(|u| u.1.clone()).collect();
                let mut jump: i64 = 0;
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    if ui.selectable_label(false, l("— Inizio —", "— Start —")).double_clicked() {
                        jump = -(undo.len() as i64);
                    }
                    for (k, lab) in undo.iter().enumerate() {
                        if ui.selectable_label(k + 1 == undo.len(), format!("{}. {lab}", k + 1)).double_clicked() {
                            jump = k as i64 + 1 - undo.len() as i64;
                        }
                    }
                    for (k, lab) in redo.iter().enumerate() {
                        if ui.add(egui::Button::new(egui::RichText::new(format!("{}. {lab}", undo.len() + k + 1)).weak()).frame(false)).double_clicked() {
                            jump = k as i64 + 1;
                        }
                    }
                });
                for _ in 0..jump.unsigned_abs() {
                    if jump < 0 { app.doc.undo() } else { app.doc.redo() }
                }
                if jump != 0 {
                    app.invalidate();
                }
                hint(ui, &l("Doppio clic su un passaggio per tornare a quel punto. Le voci in grigio si possono ripetere.", "Double-click a step to return to that point. Grey entries can be redone."));
                ui.button(tr("close")).clicked().then_some(false)
            }
            Dialog::Notes { text, show } => {
                ui.add(egui::TextEdit::multiline(text).desired_rows(10).desired_width(440.0)
                    .hint_text(l("Materiale, impostazioni provate, istruzioni per il cliente…", "Material, tested settings, instructions…")));
                ui.checkbox(show, l("Mostra le note quando apro il progetto", "Show notes when the project is opened"));
                buttons(ui, "OK")
            }
            Dialog::CloseTolerance { tol } => {
                egui::Grid::new("ct").num_columns(2).show(ui, |ui| dv(ui, &l("Tolleranza (mm)", "Tolerance (mm)"), tol, 0.001..=50.0, 0.01));
                hint(ui, &l("Unisce le linee aperte le cui estremità distano meno della tolleranza e chiude le forme che finiscono dove iniziano. Utile per i file DXF e PDF con contorni spezzati.",
                            "Joins open lines whose ends are closer than the tolerance and closes shapes that end where they start. Useful for DXF and PDF files with broken outlines."));
                buttons(ui, "OK")
            }
            Dialog::ConvertBitmap { px, fill, stroke, keep } => {
                egui::Grid::new("cb").num_columns(2).show(ui, |ui| {
                    dv(ui, &l("Dimensione pixel (mm)", "Pixel size (mm)"), px, 0.02..=2.0, 0.005);
                    ui.label(l("Contenuto", "Content"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(fill, true, l("Forme piene", "Filled shapes"));
                        ui.selectable_value(fill, false, l("Solo contorni", "Outlines only"));
                    });
                    ui.end_row();
                    dv(ui, &l("Spessore contorni (mm)", "Outline width (mm)"), stroke, 0.02..=10.0, 0.01);
                });
                ui.checkbox(keep, l("Mantieni le forme originali", "Keep the original shapes"));
                hint(ui, &l("Utile per incidere una grafica vettoriale come immagine (riempimento a scansione, dithering).", "Useful to engrave vector artwork as an image (raster fill, dithering)."));
                buttons(ui, "OK")
            }
            Dialog::Nesting { true_shape, spacing, margin, rot } => {
                egui::Grid::new("nest").num_columns(2).show(ui, |ui| {
                    ui.label(l("Metodo", "Method"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(true_shape, true, l("Forma reale", "True shape"));
                        ui.selectable_value(true_shape, false, l("Rettangoli", "Bounding boxes"));
                    });
                    ui.end_row();
                    dv(ui, &l("Distanza tra i pezzi (mm)", "Part spacing (mm)"), spacing, 0.0..=50.0, 0.1);
                    dv(ui, &l("Margine del foglio (mm)", "Sheet margin (mm)"), margin, 0.0..=100.0, 0.5);
                    ui.label(l("Rotazioni", "Rotations"));
                    ui.horizontal(|ui| {
                        for (v, t) in [(0, l("Nessuna", "None")), (90, "90°".into()), (45, "45°".into()), (15, "15°".into())] {
                            ui.selectable_value(rot, v, t);
                        }
                    });
                    ui.end_row();
                });
                hint(ui, &l("Usa le forme selezionate (o tutte). Forma reale: i pezzi si incastrano seguendo il contorno, i più grandi per primi; con passi di rotazione piccoli è più lento ma più compatto. L'area utile è l'area di lavoro meno il margine.",
                            "Uses the selected shapes (or all). True shape: parts interlock following their outline, largest first; small rotation steps are slower but tighter. The usable area is the work area minus the margin."));
                buttons(ui, "OK")
            }
            Dialog::Sublayers { layer, rows } => {
                let lay = app.doc.layers[*layer].clone();
                hint(ui, &l(&format!("Il layer {} viene eseguito con le sue impostazioni, poi ogni sotto-layer attivo ripassa le stesse forme con le impostazioni qui sotto.", lay.name),
                            &format!("Layer {} runs with its own settings, then every enabled sub-layer goes over the same shapes with the settings below.", lay.name)));
                let mut del = None;
                egui::Grid::new("subs").striped(true).num_columns(7).show(ui, |ui| {
                    for h in [l("Attivo", "On"), l("Nome", "Name"), l("Modalità", "Mode"), "mm/min".into(), l("Potenza %", "Power %"), l("Passate", "Passes"), String::new()] {
                        ui.weak(h);
                    }
                    ui.end_row();
                    for (r, row) in rows.iter_mut().enumerate() {
                        ui.checkbox(&mut row.enabled, "");
                        ui.text_edit_singleline(&mut row.name);
                        egui::ComboBox::from_id_salt(("sm", r)).selected_text(row.mode.clone()).show_ui(ui, |ui| {
                            for (k, t) in [("line", l("Linea", "Line")), ("fill", l("Riempi", "Fill")), ("offset_fill", "Offset".into()), ("line_fill", l("Linea+Riempi", "Line+Fill"))] {
                                ui.selectable_value(&mut row.mode, k.to_string(), t);
                            }
                        });
                        ui.add(egui::DragValue::new(&mut row.speed).range(1.0..=100000.0).speed(10.0));
                        ui.add(egui::DragValue::new(&mut row.power).range(0.0..=100.0));
                        ui.add(egui::DragValue::new(&mut row.passes).range(1..=100));
                        if ui.small_button("✕").clicked() {
                            del = Some(r);
                        }
                        ui.end_row();
                    }
                });
                if let Some(r) = del {
                    rows.remove(r);
                }
                if ui.button(l("Aggiungi", "Add")).clicked() {
                    rows.push(SubRow { enabled: true, name: format!("{}.{}", lay.name, rows.len() + 1), mode: lay.cut_mode.as_str().into(), speed: lay.speed, power: lay.power_max, passes: 1 });
                }
                buttons(ui, "OK")
            }
            Dialog::Settings { p } => {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| settings_body(ui, p));
                buttons(ui, "OK")
            }
            Dialog::Generic(g) => g.body(ui, app),
        }
    }

    /// Apply on OK; returns true to close.
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        match self {
            Dialog::Message { .. } => {}
            Dialog::ConfirmDiscard { then } => match then.as_str() {
                "new" => app.new_project(),
                "open" => {
                    app.doc.dirty = false;
                    app.run("open");
                }
                "quit" => {
                    app.closing = true;
                    app.pending_cmd = Some(egui::ViewportCommand::Close);
                }
                _ => {}
            },
            Dialog::ConfirmArea { then, to_abs, .. } => {
                if *to_abs {
                    // place the job where it is on the grid, then check the area again
                    app.start_from = "absolute".into();
                    app.invalidate();
                } else {
                    app.area_ok = true;
                }
                if then == "start" {
                    app.start_laser();
                } else {
                    app.frame_job(None);
                }
            }
            Dialog::Quit => {
                app.closing = true;
                app.pending_cmd = Some(egui::ViewportCommand::Close);
            }
            Dialog::PdfPage { path, page, .. } => {
                if let Ok(d) = std::fs::read(&*path) {
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    app.import_pdf(&d, *page - 1, &name);
                }
            }
            Dialog::Import3d { path, px_mm, invert } => {
                match crate::mesh3d::depth_map(path, *px_mm, *invert) {
                    Ok((src, w, h)) => {
                        let (ww, wh) = app.ws();
                        let mut p = Map::new();
                        for (k, v) in [("src", json!(src)), ("name", json!(path.file_name().unwrap().to_string_lossy())), ("x", json!((ww - w) / 2.0)), ("y", json!((wh - h) / 2.0)),
                                       ("w", json!(w)), ("h", json!(h))] {
                            p.insert(k.into(), v);
                        }
                        let mut adj: Map<String, Value> = serde_json::from_value(crate::shapes::default_adj()).unwrap();
                        adj.insert("mode".into(), json!("slices3d"));
                        p.insert("adj".into(), Value::Object(adj));
                        let s = app.doc.create("image", p);
                        app.doc.add_shapes(vec![s], true);
                        app.invalidate();
                    }
                    Err(e) => app.say(e),
                }
            }
            Dialog::Text { id, at, text, size, font, single, bold, italic } => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    return false;
                }
                let fam = if single.is_empty() { font.clone() } else { single.clone() };
                match id {
                    Some(id) => {
                        app.doc.save_undo(&l("Modifica testo", "Edit text"));
                        if let Some(s) = app.doc.find_mut(*id) {
                            s.0.insert("text".into(), json!(t));
                            s.set("size", *size);
                            s.0.insert("font".into(), json!(fam));
                            s.0.insert("bold".into(), json!(*bold));
                            s.0.insert("italic".into(), json!(*italic));
                        }
                    }
                    None => {
                        let mut p = Map::new();
                        for (k, v) in [("x", json!(at[0])), ("y", json!(at[1])), ("text", json!(t)), ("size", json!(*size)), ("font", json!(fam))] {
                            p.insert(k.into(), v);
                        }
                        if *bold {
                            p.insert("bold".into(), json!(true));
                        }
                        if *italic {
                            p.insert("italic".into(), json!(true));
                        }
                        let s = app.doc.create("text", p);
                        app.doc.add_shapes(vec![s], true);
                        app.canvas.set_tool("select");
                    }
                }
                app.invalidate();
            }
            Dialog::Polygon { sides } => {
                app.polygon_sides = *sides;
                app.canvas.set_tool("polygon");
            }
            Dialog::GridArray { cols, rows, dx, dy, live } => {
                let meta = json!({"kind": "grid", "cols": cols, "rows": rows, "dx": dx, "dy": dy});
                make_array_cmd(app, meta, *live);
            }
            Dialog::CircularArray { n, angle, cx, cy, rotate, live } => {
                let meta = json!({"kind": "circular", "n": n, "angle": angle, "cx": cx, "cy": cy, "rotate": rotate});
                make_array_cmd(app, meta, *live);
            }
            Dialog::Offset { distance, direction, join, delete } => {
                let r = app.doc.geometry_op("offset", &json!({"distance": distance, "direction": direction, "join": join}), *delete, 1);
                app.msg(r);
            }
            Dialog::RoundCorners { radius } => {
                let r = app.doc.geometry_op("round", &json!({"radius": radius}), true, 1);
                app.msg(r);
            }
            Dialog::CopyAlongPath { n, rotate, delete } => {
                let sel = app.doc.selected_in_order();
                let (path, objs) = (sel[sel.len() - 1].clone(), sel[..sel.len() - 1].to_vec());
                let pts = path.paths().into_iter().next().unwrap_or_default();
                let total = crate::tools::path_length(&pts);
                let closed = matches!(path.kind(), "rect" | "circle") || path.flag("closed");
                let cnt = *n as usize;
                let step = if closed { total / cnt as f64 } else { total / (cnt - 1) as f64 };
                let c0 = center(multi_bounds(&objs).unwrap());
                let mut out = Vec::new();
                for i in 0..cnt {
                    let d = step * i as f64;
                    let p = crate::tools::point_at(&pts, d);
                    let p2 = crate::tools::point_at(&pts, (d + 0.01).min(total));
                    let ang = (p2[1] - p[1]).atan2(p2[0] - p[0]).to_degrees();
                    for s in &objs {
                        let mut c = app.doc.reid(s.clone());
                        c.translate(p[0] - c0[0], p[1] - c0[1]);
                        if *rotate {
                            rotate_shape(&mut c, p[0], p[1], ang);
                        }
                        out.push(c);
                    }
                }
                app.doc.save_undo("");
                if *delete {
                    let ids: Vec<i64> = objs.iter().map(|s| s.id()).collect();
                    app.doc.shapes.retain(|s| !ids.contains(&s.id()));
                }
                app.doc.selected = out.iter().map(|s| s.id()).collect();
                app.doc.shapes.extend(out);
                app.invalidate();
            }
            Dialog::AdjustImage { id, adj, .. } => {
                app.doc.save_undo(&l("Regola immagine", "Adjust image"));
                if let Some(s) = app.doc.find_mut(*id) {
                    s.0.insert("adj".into(), Value::Object(adj.clone()));
                }
                app.invalidate();
            }
            Dialog::TraceImage { id, threshold, smooth, res, delete } => {
                let Some(s) = app.doc.find(*id).cloned() else { return true };
                let Some(pic) = s.0.get("src").and_then(|v| v.as_str()).and_then(laserforge_core::imaging::decode) else { return true };
                let mut a: laserforge_core::imaging::Adj = s.0.get("adj").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default();
                a.mode = "threshold".into();
                a.threshold = *threshold as f64;
                let r = *res as f64;
                let (burn, cols, rows) = laserforge_core::imaging::process(&pic, s.f("w"), s.f("h"), s.f("w").max(s.f("h")) / r, *res as usize, false, s.flag("flipX"), s.flag("flipY"), &a);
                let on: Vec<bool> = burn.iter().map(|b| *b > 127).collect();
                let loops = crate::bitmap::trace_binary(&on, cols, rows);
                let (kx, ky) = (s.f("w") / cols as f64, s.f("h") / rows as f64);
                let mut shapes = Vec::new();
                for lp in loops {
                    let pts = crate::shapes::simplify(&lp, *smooth);
                    if pts.len() > 3 {
                        shapes.push(json!({"id": app.doc.new_id(), "type": "poly", "layer": s.layer(), "closed": true,
                            "points": pts[..pts.len() - 1].iter().map(|q| json!({"x": s.f("x") + q[0] * kx, "y": s.f("y") + q[1] * ky})).collect::<Vec<_>>()}));
                    }
                }
                if shapes.is_empty() {
                    app.say(l("Niente da tracciare", "Nothing traced"));
                    return false;
                }
                let n = shapes.len();
                app.doc.save_undo(&l("Traccia immagine", "Trace image"));
                if *delete {
                    app.doc.shapes.retain(|x| x.id() != s.id());
                }
                let mut p = Map::new();
                p.insert("children".into(), json!(shapes));
                let mut g = app.doc.create("group", p);
                g.0.insert("layer".into(), json!(s.layer()));
                app.doc.selected = vec![g.id()];
                app.doc.shapes.push(g);
                app.invalidate();
                app.say(l(&format!("{n} percorsi tracciati"), &format!("{n} paths traced")));
            }
            Dialog::Device { m, .. } => {
                app.machine = m.clone();
                app.invalidate();
            }
            Dialog::Optimize { o } => {
                app.opt = o.clone();
                app.invalidate();
            }
            Dialog::Rotary { m } => {
                app.machine = m.clone();
                app.invalidate();
            }
            Dialog::Cylinder { c } => {
                app.cylinder = c.clone();
                app.invalidate();
            }
            Dialog::Notes { text, show } => {
                let new = json!({"text": text, "show_on_load": show});
                if new != app.doc.notes {
                    app.doc.notes = new;
                    app.doc.dirty = true;
                }
            }
            Dialog::CloseTolerance { tol } => {
                let r = app.doc.close_with_tolerance(*tol);
                app.msg(r);
            }
            Dialog::ConvertBitmap { px, fill, stroke, keep } => {
                let sel: Vec<Shape> = app.doc.selected_shapes().into_iter().filter(|s| s.kind() != "image").collect();
                match crate::bitmap::vectors_to_bitmap(&sel, *px, *fill, *stroke) {
                    Ok((src, x, y, w, h)) => {
                        app.doc.save_undo("");
                        if !*keep {
                            let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
                            app.doc.shapes.retain(|s| !ids.contains(&s.id()));
                        }
                        let mut p = Map::new();
                        let mut adj: Map<String, Value> = serde_json::from_value(crate::shapes::default_adj()).unwrap();
                        adj.insert("mode".into(), json!("threshold"));
                        for (k, v) in [("src", json!(src)), ("name", json!("bitmap.png")), ("x", json!(x)), ("y", json!(y)), ("w", json!(w)), ("h", json!(h)), ("adj", Value::Object(adj))] {
                            p.insert(k.into(), v);
                        }
                        let mut im = app.doc.create("image", p);
                        im.0.insert("layer".into(), json!(sel[0].layer()));
                        app.doc.selected = vec![im.id()];
                        app.doc.shapes.push(im);
                        app.invalidate();
                    }
                    Err(e) => app.say(e),
                }
            }
            Dialog::Nesting { true_shape, spacing, margin, rot } => {
                let ws = app.ws();
                let r = if *true_shape {
                    let rotations: Vec<f64> = if *rot == 0 { vec![0.0] } else { (0..360).step_by(*rot as usize).map(|v| v as f64).collect() };
                    crate::nest::nest_true_shape(&mut app.doc, ws, *spacing, &rotations, *margin)
                } else {
                    app.doc.nesting(ws)
                };
                app.msg(r);
            }
            Dialog::Sublayers { layer, rows } => {
                app.doc.layers[*layer].sublayers = rows.iter().map(|r| json!({"enabled": r.enabled, "name": r.name.trim(), "cut_mode": r.mode, "speed": r.speed, "power_max": r.power, "passes": r.passes})).collect();
                app.invalidate();
            }
            Dialog::Settings { p } => {
                app.prefs = p.clone();
                app.snap_on = p.grid_snap;
                app.grid = p.grid_show;
                crate::shapes::set_curve_tol(p.curve_quality);
                if let Ok(mut f) = crate::shx::SHX_FOLDER.lock() {
                    *f = p.shx_folder.clone();
                }
                app.remote = None;
                app.sync_remote();
                app.invalidate();
            }
            Dialog::Generic(g) => return g.apply(app),
            Dialog::Preview { .. } | Dialog::Shortcuts | Dialog::About | Dialog::UndoHistory | Dialog::Layer { .. } => {}
        }
        true
    }
}

fn device_tab(ui: &mut egui::Ui, m: &mut laserforge_core::model::MachineSettings, tab: usize) {
    egui::Grid::new(("dev", tab)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| match tab {
        0 => {
            dv(ui, &l("Area di lavoro larghezza (mm)", "Work area width (mm)"), &mut m.workspace_x, 10.0..=3000.0, 1.0);
            dv(ui, &l("Area di lavoro altezza (mm)", "Work area height (mm)"), &mut m.workspace_y, 10.0..=3000.0, 1.0);
            ui.label(l("Origine (0,0 della macchina)", "Origin (machine 0,0)"));
            egui::Grid::new("origin").show(ui, |ui| {
                ui.selectable_value(&mut m.origin, "rl".into(), l("↖ Dietro sx", "↖ Rear left"));
                ui.selectable_value(&mut m.origin, "rr".into(), l("Dietro dx ↗", "Rear right ↗"));
                ui.end_row();
                ui.selectable_value(&mut m.origin, "fl".into(), l("↙ Davanti sx", "↙ Front left"));
                ui.selectable_value(&mut m.origin, "fr".into(), l("Davanti dx ↘", "Front right ↘"));
                ui.end_row();
            });
            ui.end_row();
            ui.label("Controller");
            egui::ComboBox::from_id_salt("flavor").selected_text(m.flavor.clone()).show_ui(ui, |ui| {
                for (k, t) in [("grbl", "GRBL"), ("grblhal", "grblHAL"), ("fluidnc", "FluidNC"), ("marlin", "Marlin"), ("smoothie", "Smoothieware"), ("linuxcnc", "LinuxCNC")] {
                    ui.selectable_value(&mut m.flavor, k.to_string(), t);
                }
            });
            ui.end_row();
            ui.label("Baud rate");
            egui::ComboBox::from_id_salt("dbaud").selected_text(m.baud_rate.to_string()).show_ui(ui, |ui| {
                for b in [9600i64, 19200, 38400, 57600, 115200, 230400, 250000, 460800] {
                    ui.selectable_value(&mut m.baud_rate, b, b.to_string());
                }
            });
            ui.end_row();
            dv(ui, &l("Velocità max (mm/min)", "Max speed (mm/min)"), &mut m.max_speed, 100.0..=100000.0, 10.0);
            iv(ui, &l("Valore S max (GRBL $30)", "S-value max (GRBL $30)"), &mut m.max_power, 1..=100000);
            ui.label("");
            ui.checkbox(&mut m.home_on_start, tr("homeOnStart"));
            ui.end_row();
            dv(ui, &l("Potenza cornice (%)", "Frame power (%)"), &mut m.frame_power, 0.0..=20.0, 0.1);
        }
        1 => {
            ui.label(l("Comando aria compressa", "Air assist command"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut m.air_command, "M8".into(), l("M8 (flood) — consigliato", "M8 (flood) — recommended"));
                ui.selectable_value(&mut m.air_command, "M7".into(), "M7 (mist)");
            });
            ui.end_row();
            if m.air_command == "M7" {
                ui.label("");
                ui.colored_label(Color32::from_rgb(0xe0, 0xa0, 0x30), l("⚠ GRBL di serie non supporta M7: usalo solo se il firmware è compilato con ENABLE_M7.", "⚠ Stock GRBL does not support M7: use it only if the firmware was built with ENABLE_M7."));
                ui.end_row();
            }
            ui.label(l("Aria sui nuovi layer", "Air on new layers"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut m.air_assist_default, AirAssist::Off, tr("airOff"));
                ui.selectable_value(&mut m.air_assist_default, AirAssist::On, l("Accesa", "On"));
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.marlin_air_fan, l("Marlin: aria compressa sulla ventola (M106/M107)", "Marlin: air assist on the part fan (M106/M107)"));
            ui.end_row();
        }
        2 => {
            ui.label(l("Modalità di invio", "Transfer mode"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut m.transfer_mode, "buffered".into(), l("Bufferizzata — consigliata", "Buffered — recommended"));
                ui.selectable_value(&mut m.transfer_mode, "synchronous".into(), l("Sincrona", "Synchronous"));
            });
            ui.end_row();
            dv(ui, &l("Accelerazione (mm/s², $120)", "Acceleration (mm/s², $120)"), &mut m.acceleration, 10.0..=50000.0, 10.0);
            dv(ui, &l("Gioco asse X (mm)", "X backlash (mm)"), &mut m.backlash_x, 0.0..=5.0, 0.005);
            dv(ui, &l("Gioco asse Y (mm)", "Y backlash (mm)"), &mut m.backlash_y, 0.0..=5.0, 0.005);
            ui.label("");
            ui.checkbox(&mut m.return_to_origin, tr("returnOrigin"));
            ui.end_row();
            ui.label(l("Posizione di fine lavoro", "Finish position"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut m.finish_position, "origin".into(), l("Origine (0,0)", "Origin (0,0)"));
                ui.selectable_value(&mut m.finish_position, "custom".into(), l("Punto personalizzato", "Custom point"));
            });
            ui.end_row();
            dv(ui, &l("Fine lavoro X (mm)", "Finish X (mm)"), &mut m.finish_x, -3000.0..=3000.0, 1.0);
            dv(ui, &l("Fine lavoro Y (mm)", "Finish Y (mm)"), &mut m.finish_y, -3000.0..=3000.0, 1.0);
            ui.label("");
            ui.checkbox(&mut m.z_enabled, l("Abilita asse Z", "Enable Z axis"));
            ui.end_row();
            dv(ui, &l("Velocità asse Z (mm/s)", "Z axis speed (mm/s)"), &mut m.z_speed, 0.1..=200.0, 0.1);
            ui.label("");
            ui.checkbox(&mut m.z_relative_only, l("Solo movimenti Z relativi", "Relative Z moves only"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.fire_enabled, l("Abilita pulsante Fire", "Enable Fire button"));
            ui.end_row();
            dv(ui, &l("Potenza Fire (%)", "Fire power (%)"), &mut m.fire_power, 0.1..=100.0, 0.1);
        }
        3 => {
            ui.label("");
            ui.checkbox(&mut m.fast_whitespace, l("Scansione veloce degli spazi bianchi", "Fast whitespace scan"));
            ui.end_row();
            dv(ui, &l("Velocità minima sul bianco (mm/min)", "Min speed over blank (mm/min)"), &mut m.whitespace_speed, 100.0..=100000.0, 10.0);
            ui.label("");
            ui.checkbox(&mut m.use_g0_overscan, l("Usa G0 per l'overscan", "Use G0 for overscan"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.raster_relative, l("Immagini in coordinate relative (G91)", "Images in relative coordinates (G91)"));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut m.scan_offset_enabled, l("Correzione offset di scansione (bidirezionale)", "Scanning offset adjustment (bidirectional)"));
            ui.end_row();
            ui.label(l("velocità:offset (mm/min:mm)", "speed:offset (mm/min:mm)"));
            ui.add(egui::TextEdit::multiline(&mut m.scan_offsets).desired_rows(4).code_editor());
            ui.end_row();
        }
        _ => {
            ui.label(l("G-code iniziale", "Start G-code"));
            ui.add(egui::TextEdit::multiline(&mut m.start_gcode).desired_rows(4).code_editor());
            ui.end_row();
            ui.label(l("G-code finale", "End G-code"));
            ui.add(egui::TextEdit::multiline(&mut m.end_gcode).desired_rows(4).code_editor());
            ui.end_row();
        }
    });
}

fn settings_body(ui: &mut egui::Ui, p: &mut crate::prefs::Prefs) {
    egui::Grid::new("prefs").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(l("Unità", "Units"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut p.units, "mm".into(), "mm");
            ui.selectable_value(&mut p.units, "in".into(), "in");
        });
        ui.end_row();
        ui.label(l("Velocità", "Speed units"));
        ui.horizontal(|ui| {
            for u in ["mm/min", "mm/s", "in/min"] {
                ui.selectable_value(&mut p.speed_units, u.into(), u);
            }
        });
        ui.end_row();
        ui.label(l("Tema", "Theme"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut p.theme, "dark".into(), l("Scuro", "Dark"));
            ui.selectable_value(&mut p.theme, "light".into(), l("Chiaro", "Light"));
        });
        ui.end_row();
        dv(ui, &l("Dimensione icone (px)", "Icon size (px)"), &mut p.icon_size, 14.0..=32.0, 1.0);
        dv(ui, &l("Testo interfaccia (pt, al prossimo avvio)", "Interface text (pt, next start)"), &mut p.ui_font, 9.0..=20.0, 0.2);
        dv(ui, &l("Scala interfaccia (%, al prossimo avvio)", "Interface scale (%, next start)"), &mut p.ui_scale, 75.0..=250.0, 1.0);
        ui.label("");
        ui.checkbox(&mut p.grid_show, l("Mostra griglia", "Show grid"));
        ui.end_row();
        dv(ui, &l("Spaziatura griglia (0 = auto)", "Grid spacing (0 = auto)"), &mut p.grid_spacing, 0.0..=100.0, 0.5);
        ui.label("");
        ui.checkbox(&mut p.grid_snap, l("Aggancio alla griglia", "Snap to grid"));
        ui.end_row();
        dv(ui, &l("Passo aggancio (mm)", "Snap step (mm)"), &mut p.snap_step, 0.01..=50.0, 0.05);
        ui.label("");
        ui.checkbox(&mut p.obj_snap, l("Aggancio agli oggetti", "Snap to objects"));
        ui.end_row();
        dv(ui, &l("Distanza aggancio (px)", "Snap distance (px)"), &mut p.obj_snap_px, 2.0..=30.0, 0.5);
        dv(ui, &l("Tolleranza clic (px)", "Click tolerance (px)"), &mut p.click_tol, 1.0..=20.0, 0.5);
        ui.label(l("Scorrimento a due dita", "Two-finger scroll"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut p.trackpad_scroll, "zoom".into(), "Zoom");
            ui.selectable_value(&mut p.trackpad_scroll, "pan".into(), l("Sposta", "Pan"));
        });
        ui.end_row();
        dv(ui, &l("Dimensione nodi (px)", "Node size (px)"), &mut p.node_size, 3.0..=15.0, 0.5);
        ui.label(l("Qualità curve", "Curve quality"));
        ui.horizontal(|ui| {
            // 1 (fast, 0.5 mm chord error) … 10 (smooth, 0.05 mm), like the Python slider
            let mut q = (11.0 - p.curve_quality / 0.05).round().clamp(1.0, 10.0);
            ui.label(l("Veloce", "Fast"));
            if ui.add(egui::Slider::new(&mut q, 1.0..=10.0).step_by(1.0).show_value(false)).changed() {
                p.curve_quality = ((11.0 - q) * 0.05 * 1000.0).round() / 1000.0;
            }
            ui.label(l("Precisa", "Smooth"));
        });
        ui.end_row();
        dv(ui, &l("Spostamento frecce (mm)", "Arrow nudge (mm)"), &mut p.nudge, 0.01..=50.0, 0.05);
        dv(ui, &l("Spostamento con Shift (mm)", "Shift nudge (mm)"), &mut p.nudge_shift, 0.1..=200.0, 0.5);
        dv(ui, &l("Spostamento fine con ⌥ (mm)", "Fine nudge with ⌥ (mm)"), &mut p.nudge_fine, 0.001..=5.0, 0.01);
        ui.label("");
        ui.checkbox(&mut p.frame_laser_on, l("Laser acceso durante la cornice", "Laser on when framing"));
        ui.end_row();
        dv(ui, &l("Potenza cornice (%)", "Frame power (%)"), &mut p.frame_power, 0.0..=20.0, 0.1);
        ui.label(l("Cartella font SHX", "SHX font folder"));
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut p.shx_folder);
            if ui.button("…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    p.shx_folder = d.to_string_lossy().to_string();
                }
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.backup_enabled, l("Backup automatico", "Automatic backup"));
        ui.end_row();
        dv(ui, &l("Ogni (minuti)", "Every (minutes)"), &mut p.backup_minutes, 1.0..=120.0, 1.0);
        ui.label(l("Copie da tenere", "Copies to keep"));
        ui.add(egui::DragValue::new(&mut p.backup_keep).range(1..=500));
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.cut_selected, l("Lavora solo le forme selezionate", "Cut selected graphics only"));
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.checklist_enabled, l("Chiedi la checklist prima di ogni lavoro", "Ask the checklist before every job"));
        ui.end_row();
        ui.label(l("Avvia da (all'apertura)", "Start from (at startup)"));
        egui::ComboBox::from_id_salt("defstart").selected_text(match p.default_start_from.as_str() {
            "absolute" => tr("sfAbsolute"), "current" => tr("sfCurrent"), "user" => tr("sfUser"), _ => l("Come l'ultima volta", "As last time") })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.default_start_from, "last".into(), l("Come l'ultima volta", "As last time"));
                for k in ["absolute", "current", "user"] {
                    let lab = match k { "absolute" => tr("sfAbsolute"), "current" => tr("sfCurrent"), _ => tr("sfUser") };
                    ui.selectable_value(&mut p.default_start_from, k.into(), lab);
                }
            });
        ui.end_row();
        let origins = [("In alto a sinistra", "Top left"), ("In alto al centro", "Top centre"), ("In alto a destra", "Top right"),
                       ("Al centro a sinistra", "Middle left"), ("Centro", "Centre"), ("Al centro a destra", "Middle right"),
                       ("In basso a sinistra", "Bottom left"), ("In basso al centro", "Bottom centre"), ("In basso a destra", "Bottom right")];
        ui.label(l("Origine lavoro (all'apertura)", "Job origin (at startup)"));
        let cur = if p.default_origin < 0 { l("Come l'ultima volta", "As last time") } else {
            let (a, b) = origins[(p.default_origin as usize).min(8)];
            l(a, b)
        };
        egui::ComboBox::from_id_salt("deforigin").selected_text(cur).show_ui(ui, |ui| {
            ui.selectable_value(&mut p.default_origin, -1, l("Come l'ultima volta", "As last time"));
            for (i, (a, b)) in origins.iter().enumerate() {
                ui.selectable_value(&mut p.default_origin, i as i64, l(a, b));
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.origin_from_selection, l("Calcola l'origine del lavoro sulla selezione", "Use the selection for the job origin"));
        ui.end_row();
        ui.label(l("Qualità immagine camera", "Camera image quality"));
        ui.horizontal(|ui| {
            for (v, a, b) in [(800.0, "Bassa", "Low"), (1600.0, "Media", "Medium"), (2400.0, "Alta", "High")] {
                ui.selectable_value(&mut p.camera_quality, v, l(a, b));
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut p.remote_enabled, l("Controllo remoto dal telefono (rete locale)", "Remote control from a phone (local network)"));
        ui.end_row();
        ui.label(l("Porta remota", "Remote port"));
        ui.add(egui::DragValue::new(&mut p.remote_port).range(1024..=65535));
        ui.end_row();
        ui.label("PIN");
        ui.text_edit_singleline(&mut p.remote_pin);
        ui.end_row();
        if p.remote_enabled {
            ui.label("");
            ui.label(format!("http://{}:{}", crate::remote::local_ip(), p.remote_port));
            ui.end_row();
        }
    });
    ui.add(egui::Label::new(egui::RichText::new(format!("{}{}{}", l("I backup si trovano in ", "Backups are stored in "), crate::dialogs2::backup_dir().display(),
                                                        l(" e si aprono da File › Apri backup.", " and open from File › Open Backup."))).weak().size(11.0)).wrap());
    if ui.button(l("Ripristina predefiniti", "Restore defaults")).clicked() {
        let pin = p.remote_pin.clone();
        *p = crate::prefs::Prefs::default();
        p.remote_pin = pin;
    }
}

/// Grid / circular copies (props.array_copies): copies of `source` described by `meta`, ids from the document.
pub fn array_copies(doc: &mut crate::project::Doc, source: &[Shape], meta: &Value) -> Vec<Shape> {
    let mut out = Vec::new();
    let g = |k: &str| meta.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    if meta["kind"] == "grid" {
        let b = multi_bounds(source).unwrap();
        for r in 0..g("rows") as i64 {
            for c in 0..g("cols") as i64 {
                if r == 0 && c == 0 {
                    continue;
                }
                for s in source {
                    let mut x = doc.reid(s.clone());
                    x.translate(c as f64 * (b[2] + g("dx")), r as f64 * (b[3] + g("dy")));
                    out.push(x);
                }
            }
        }
    } else {
        let (n, total, cx, cy) = (g("n") as i64, g("angle"), g("cx"), g("cy"));
        let step = if total.abs() >= 360.0 { total / n as f64 } else { total / (n - 1).max(1) as f64 };
        let rot = meta.get("rotate").and_then(|v| v.as_bool()).unwrap_or(true);
        for i in 1..n {
            let deg = step * i as f64;
            let a = deg.to_radians();
            for s in source {
                let mut x = doc.reid(s.clone());
                if !(rot && rotate_shape(&mut x, cx, cy, deg)) {
                    let c0 = center(x.bounds());
                    let (dx, dy) = (c0[0] - cx, c0[1] - cy);
                    x.translate(cx + dx * a.cos() - dy * a.sin() - c0[0], cy + dx * a.sin() + dy * a.cos() - c0[1]);
                }
                out.push(x);
            }
        }
    }
    out
}

fn make_array_cmd(app: &mut LaserApp, meta: Value, live: bool) {
    let sel = app.doc.selected_shapes();
    if sel.is_empty() {
        return;
    }
    if live {
        let mut meta = meta;
        meta["count"] = json!(sel.len());
        meta["src_c"] = json!(center(multi_bounds(&sel).unwrap()));
        let copies = array_copies(&mut app.doc, &sel, &meta);
        let mut kids = sel.clone();
        kids.extend(copies);
        let mut p = Map::new();
        p.insert("children".into(), serde_json::to_value(&kids).unwrap());
        let mut g = app.doc.create("group", p);
        g.0.insert("layer".into(), json!(sel[0].layer()));
        g.0.insert("array".into(), meta);
        app.doc.save_undo(&l("Array", "Array"));
        let first = app.doc.shapes.iter().position(|s| app.doc.selected.contains(&s.id())).unwrap_or(0);
        let ids = app.doc.selected.clone();
        app.doc.shapes.retain(|s| !ids.contains(&s.id()));
        let gid = g.id();
        app.doc.shapes.insert(first.min(app.doc.shapes.len()), g);
        app.doc.selected = vec![gid];
    } else {
        let copies = array_copies(&mut app.doc, &sel, &meta);
        app.doc.add_shapes(copies, false);
    }
    app.invalidate();
}

/// props.regenerate_array: rebuild a live array's copies from its first `count` children.
pub fn regenerate_array(doc: &mut crate::project::Doc, id: i64) {
    let Some(g) = doc.find(id).cloned() else { return };
    let Some(mut meta) = g.0.get("array").cloned() else { return };
    let count = meta["count"].as_u64().unwrap_or(1) as usize;
    let source: Vec<Shape> = g.children().into_iter().take(count).collect();
    let Some(b) = multi_bounds(&source) else { return };
    let cur = center(b);
    if meta["kind"] == "circular" {
        if let Some(sc) = meta.get("src_c").and_then(|v| v.as_array()).map(|a| [a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)]) {
            meta["cx"] = json!(meta["cx"].as_f64().unwrap_or(0.0) + cur[0] - sc[0]);
            meta["cy"] = json!(meta["cy"].as_f64().unwrap_or(0.0) + cur[1] - sc[1]);
        }
    }
    meta["src_c"] = json!(cur);
    let copies = array_copies(doc, &source, &meta);
    let mut kids = source;
    kids.extend(copies);
    if let Some(s) = doc.find_mut(id) {
        s.0.insert("children".into(), serde_json::to_value(&kids).unwrap());
        s.0.insert("array".into(), meta);
    }
}

impl LaserApp {
    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        let mut ds = std::mem::take(&mut self.dialogs);
        ds.retain_mut(|d| d.show(ctx, self));
        ds.append(&mut self.dialogs);
        self.dialogs = ds;
    }
}

#[allow(dead_code)]
fn unused(_: CutMode) {}
