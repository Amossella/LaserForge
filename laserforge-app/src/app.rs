//! The main window: menus and tool bars built from the same tables as the Python app (defs.json), the canvas,
//! the layer palette, the side panels and the dialogs; every command goes through `run(action id)`.
//! Ported from laserforge/mainwindow.py.

use crate::canvas::{Canvas, Env, Event};
use crate::connection::{list_ports, Connection, Link};
use crate::defs::{self, MenuItem};
use crate::dialogs::Dialog;
use crate::edit::Place;
use crate::importers;
use crate::prefs::Prefs;
use crate::project::{default_layers, to_machine, BuildReport, Doc, ImageCache, JobOpts, Opt};
use crate::shapes::{multi_bounds, path_box, Shape, P};
use crate::{l, tr};
use egui::{vec2, Color32, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke};
use laserforge_core::gcode::Generator;
use laserforge_core::model::{AirAssist, LayerSettings, MachineSettings};
use laserforge_core::post::{apply_rotary_axis, translate_flavor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What survives a restart (eframe storage).
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub machine: MachineSettings,
    pub prefs: Prefs,
    pub layers: Vec<LayerSettings>,
    pub opt: Opt,
    pub port: String,
    pub baud: u32,
    pub jog_step: f64,
    pub jog_speed: f64,
    pub start_from: String,
    pub job_origin: usize,
    pub user_origin: P,
    pub user_origin_set: bool,
    pub recent: Vec<String>,
    pub lang: String,
    pub filled: bool,
    pub grid: bool,
    pub rulers: bool,
    pub cylinder: Value,
    pub positions: Vec<(String, f64, f64)>,
    pub macros: Vec<(String, String)>,
    pub shortcuts: std::collections::HashMap<String, String>,
    pub project: String,
    pub panels: Vec<String>,
    pub devices: Vec<Value>,
    pub device: String,
    pub checklist: Vec<String>,
    pub image_presets: serde_json::Map<String, Value>,
    pub camera: Value,
}

impl Default for Saved {
    fn default() -> Self {
        Saved {
            machine: MachineSettings::default(), prefs: Prefs::default(), layers: default_layers(), opt: Opt::default(),
            port: String::new(), baud: 115200, jog_step: 10.0, jog_speed: 3000.0, start_from: "absolute".into(), job_origin: 0,
            user_origin: [0.0, 0.0], user_origin_set: false, recent: Vec::new(), lang: "it".into(), filled: false, grid: true, rulers: true,
            cylinder: json!({}), positions: Vec::new(), macros: Vec::new(), shortcuts: Default::default(), project: String::new(),
            panels: default_panels(),
            devices: Vec::new(), device: String::new(), checklist: Vec::new(), image_presets: Default::default(), camera: json!({"opacity": 70}),
        }
    }
}

pub struct LaserApp {
    pub doc: Doc,
    pub machine: MachineSettings,
    pub prefs: Prefs,
    pub opt: Opt,
    pub canvas: Canvas,
    pub clipboard: Vec<Shape>,
    pub conn: Option<Connection>,
    pub port: String,
    pub ports: Vec<String>,
    pub baud: u32,
    pub jog_step: f64,
    pub jog_speed: f64,
    pub start_from: String,
    pub job_origin: usize,
    pub user_origin: P,
    pub user_origin_set: bool,
    pub print_cut: Option<Value>,
    pub cylinder: Value,
    pub lock_aspect: bool,
    pub filled: bool,
    pub grid: bool,
    pub rulers: bool,
    pub snap_on: bool,
    pub polygon_sides: usize,
    pub panels: Vec<String>,
    pub side_tab: String,
    pub recent: Vec<String>,
    pub positions: Vec<(String, f64, f64)>,
    pub macros: Vec<(String, String)>,
    pub shortcut_overrides: std::collections::HashMap<String, String>,
    pub console_input: String,
    pub toast: Option<(String, Instant)>,
    pub status: String,
    pub file: Option<PathBuf>,
    pub dialogs: Vec<Dialog>,
    pub gcode: Option<laserforge_core::Output>,
    pub gcode_origin: (Option<String>, Option<P>),
    pub images: ImageCache,
    /// Text in the property bar: X, Y, W, H, rotation, W %, H %, text height.
    pub tx: [String; 8],
    /// Property bar reference point (0..8: top-left … bottom-right) for X / Y and resizing.
    pub anchor: usize,
    /// Menus in the macOS menu bar (None elsewhere, or before the first frame).
    pub native_menu: Option<crate::native_menu::NativeMenu>,
    /// Tab shown in the lower panel group (Laser, Library, Console…).
    pub side_tab2: String,
    /// The job being burned, for the progress window.
    pub job: Option<JobRun>,
    pub frame_continuous: bool,
    pub last_frame: Option<Instant>,
    /// Bonjour search for network lasers running in the background, and what it found.
    pub net_rx: Option<std::sync::mpsc::Receiver<Vec<(String, String, u16)>>>,
    pub net_found: Vec<(String, String, u16)>,
    /// Feed override in % for controllers without real-time overrides (sent as M220).
    pub feed_ovr: i64,
    pub no_save: bool,
    pub frames: u64,
    pub shot: Option<(PathBuf, Arc<AtomicBool>)>,
    pub last_area_msg: (usize, usize),
    pub devices: Vec<Value>,
    pub device: String,
    pub checklist: Vec<String>,
    pub checklist_passed: bool,
    pub last_backup: Instant,
    pub pending_cmd: Option<egui::ViewportCommand>,
    pub fullscreen: bool,
    pub library: crate::library::LibState,
    pub art: crate::library::ArtState,
    pub remote: Option<crate::remote::Remote>,
    pub hidden_bars: Vec<String>,
    pub closing: bool,
    pub area_ok: bool,
    pub firing: bool,
    pub image_presets: serde_json::Map<String, Value>,
    pub checked_grbl: bool,
    pub cam: crate::camera_ui::CamState,
}

pub fn color(hex: &str) -> Color32 {
    Color32::from_hex(hex).unwrap_or(Color32::BLACK)
}

/// The bundled egui fonts have no arrows or geometric symbols: fall back to the system's symbol fonts.
fn add_symbol_fonts(ctx: &egui::Context) {
    let mut fd = egui::FontDefinitions::default();
    for (name, path) in [("apple_symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
                         ("arial_unicode", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fd.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fd.families.entry(fam).or_default().push(name.into());
            }
        }
    }
    ctx.set_fonts(fd);
}

pub fn fmt_time(s: f64) -> String {
    let s = s.round() as i64;
    if s >= 3600 { format!("{}h {:02}m", s / 3600, s % 3600 / 60) } else { format!("{}m {:02}s", s / 60, s % 60) }
}

/// "Ctrl+Shift+Z" → shortcut (Ctrl and Cmd both mean the command key, like Qt on macOS).
pub fn parse_shortcut(text: &str) -> Option<KeyboardShortcut> {
    if text.is_empty() {
        return None;
    }
    let mut mods = Modifiers::NONE;
    let mut key = None;
    let parts: Vec<&str> = if text.ends_with("++") { vec![&text[..text.len() - 2], "+"] } else { text.split('+').collect() };
    for p in parts {
        match p.trim() {
            "Ctrl" | "Cmd" | "Meta" => mods = mods | Modifiers::COMMAND,
            "Shift" => mods = mods | Modifiers::SHIFT,
            "Alt" | "Option" => mods = mods | Modifiers::ALT,
            "" => {}
            k => {
                key = match k {
                    "Del" | "Delete" => Some(Key::Delete),
                    "Esc" => Some(Key::Escape),
                    "`" => Some(Key::Backtick),
                    "." => Some(Key::Period),
                    "," => Some(Key::Comma),
                    "=" => Some(Key::Equals),
                    "-" => Some(Key::Minus),
                    "+" => Some(Key::Plus),
                    "Up" => Some(Key::ArrowUp),
                    "Down" => Some(Key::ArrowDown),
                    "Left" => Some(Key::ArrowLeft),
                    "Right" => Some(Key::ArrowRight),
                    _ => Key::from_name(k),
                }
            }
        }
    }
    Some(KeyboardShortcut::new(mods, key?))
}

/// Tool name of a tool action.
pub fn tool_of(aid: &str) -> Option<&'static str> {
    Some(match aid {
        "toolSelect" => "select", "toolNodes" => "nodes", "toolPoly" => "poly", "toolRect" => "rect", "toolEllipse" => "ellipse",
        "toolStar" => "star", "toolText" => "text", "toolPan" => "pan", "toolPosition" => "position", "measure" => "measure",
        "toolScissors" => "scissors", "toolTrim" => "trim", "toolTabs" => "tabs", "toolStartPoints" => "start",
        "toolTwoPoint" => "twopoint", "toolDeform" => "deform", "toolWarp" => "warp",
        _ => return None,
    })
}

impl LaserApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        add_symbol_fonts(&cc.egui_ctx);
        let no_save = ["LF_DEMO", "LF_CONNECT", "LF_SCREENSHOT", "LF_OPEN", "LF_RUN"].iter().any(|k| std::env::var_os(k).is_some());
        let saved: Saved = if no_save { Saved::default() } else { cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or_default() };
        defs::set_english(saved.lang == "en");
        if let Ok(mut f) = crate::shx::SHX_FOLDER.lock() {
            *f = saved.prefs.shx_folder.clone();
        }
        let mut doc = Doc::default();
        if !saved.layers.is_empty() {
            for (i, l) in saved.layers.iter().enumerate().take(doc.layers.len()) {
                doc.layers[i] = l.clone();
            }
        }
        if !saved.project.is_empty() {
            if let Ok((d, _)) = Doc::from_project(&saved.project) {
                doc.shapes = d.shapes;
                doc.pages = d.pages;
                doc.next_id = d.next_id;
                doc.notes = d.notes;
            }
        }
        // read the system fonts in the background: the first text draw would otherwise stall the window
        std::thread::spawn(|| {
            crate::text::faces();
        });
        let mut canvas = Canvas::default();
        canvas.handlers = crate::tools::handlers();
        let mut app = LaserApp {
            doc,
            machine: saved.machine,
            snap_on: saved.prefs.grid_snap,
            prefs: saved.prefs,
            opt: saved.opt,
            canvas,
            clipboard: Vec::new(),
            conn: None,
            port: saved.port,
            ports: list_ports(),
            baud: saved.baud,
            jog_step: saved.jog_step,
            jog_speed: saved.jog_speed,
            start_from: if saved.start_from == "user" && !saved.user_origin_set { "absolute".into() } else { saved.start_from },
            job_origin: saved.job_origin.min(8),
            user_origin: saved.user_origin,
            user_origin_set: saved.user_origin_set,
            print_cut: None,
            cylinder: saved.cylinder,
            lock_aspect: true,
            filled: saved.filled,
            grid: saved.grid,
            rulers: saved.rulers,
            polygon_sides: 6,
            panels: saved.panels,
            side_tab: "winCuts".into(),
            recent: saved.recent,
            positions: saved.positions,
            macros: saved.macros,
            shortcut_overrides: saved.shortcuts,
            console_input: String::new(),
            toast: None,
            status: String::new(),
            file: None,
            dialogs: Vec::new(),
            gcode: None,
            gcode_origin: (None, None),
            images: ImageCache::default(),
            tx: Default::default(),
            anchor: 0,
            native_menu: None,
            side_tab2: "winLaser".into(),
            job: None,
            frame_continuous: false,
            last_frame: None,
            net_rx: None,
            net_found: Vec::new(),
            feed_ovr: 100,
            no_save,
            frames: 0,
            shot: std::env::var_os("LF_SCREENSHOT").map(|p| (PathBuf::from(p), Arc::new(AtomicBool::new(false)))),
            last_area_msg: (0, 0),
            devices: saved.devices,
            device: saved.device,
            checklist: saved.checklist,
            checklist_passed: false,
            last_backup: Instant::now(),
            pending_cmd: None,
            fullscreen: false,
            library: Default::default(),
            art: Default::default(),
            remote: None,
            hidden_bars: Vec::new(),
            closing: false,
            area_ok: false,
            firing: false,
            image_presets: saved.image_presets,
            checked_grbl: false,
            cam: crate::camera_ui::CamState { cal: saved.camera.clone(), show_overlay: true, ..Default::default() },
        };
        app.apply_startup_prefs(&cc.egui_ctx);
        app.sync_remote();
        if let Ok(p) = std::env::var("LF_OPEN") {
            app.open_dropped(&PathBuf::from(p));
        }
        if std::env::var_os("LF_DEMO").is_some() {
            app.demo();
        }
        if let Ok(p) = std::env::var("LF_CONNECT") {
            app.port = p;
            app.connect();
        }
        // LF_RUN=action: run a menu action at startup (visual checks of dialogs)
        if let Ok(a) = std::env::var("LF_RUN") {
            app.run(&a);
        }
        app
    }

    /// Settings that act at startup: curve quality, grid, default "Start from" / job origin, interface text and scale.
    fn apply_startup_prefs(&mut self, ctx: &egui::Context) {
        crate::shapes::set_curve_tol(self.prefs.curve_quality);
        self.grid = self.prefs.grid_show;
        let sf = self.prefs.default_start_from.clone();
        if sf != "last" && !sf.is_empty() && (sf != "user" || self.user_origin_set) {
            self.start_from = sf;
        }
        if self.prefs.default_origin >= 0 {
            self.job_origin = (self.prefs.default_origin as usize).min(8);
        }
        let k = (self.prefs.ui_font as f32 / 12.0).clamp(0.6, 2.0);
        if (k - 1.0).abs() > 1e-3 {
            ctx.all_styles_mut(|s| {
                for f in s.text_styles.values_mut() {
                    f.size *= k;
                }
            });
        }
        let z = (self.prefs.ui_scale as f32 / 100.0).clamp(0.5, 3.0);
        if (z - 1.0).abs() > 1e-3 {
            ctx.set_zoom_factor(z);
        }
    }

    fn demo(&mut self) {
        let d = &mut self.doc;
        d.shapes.clear();
        let add = |d: &mut Doc, s: Value| {
            let mut m: serde_json::Map<String, Value> = serde_json::from_value(s).unwrap();
            let kind = m.remove("type").unwrap();
            let layer = m.remove("layer").unwrap_or(json!(0));
            let mut sh = d.create(kind.as_str().unwrap(), m);
            sh.0.insert("layer".into(), layer);
            d.shapes.push(sh);
        };
        add(d, json!({"type": "rect", "x": 40, "y": 40, "w": 120, "h": 80, "radius": 8}));
        add(d, json!({"type": "circle", "cx": 250, "cy": 110, "rx": 50, "ry": 50, "layer": 2}));
        add(d, json!({"type": "text", "x": 40, "y": 160, "text": "LaserForge", "size": 22, "font": "Helvetica", "bold": true, "layer": 1}));
        add(d, json!({"type": "poly", "closed": true, "layer": 5, "points": [{"x": 60, "y": 240}, {"x": 160, "y": 230, "c1": [100, 190]},
                      {"x": 150, "y": 330}, {"x": 70, "y": 320, "c1": [110, 360]}]}));
        add(d, json!({"type": "text", "x": 200, "y": 260, "text": "Hershey", "size": 14, "font": "Hershey:scripts", "layer": 3}));
        add(d, json!({"type": "rect", "x": 20, "y": 20, "w": 360, "h": 360, "layer": 30}));
        d.layers[5].cut_mode = laserforge_core::model::CutMode::Fill;
        d.selected = vec![2];
        self.side_tab = "winCuts".into();
    }

    // ------------------------------------------------------------------ helpers
    pub fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn msg(&mut self, r: crate::edit::Msg) {
        match r {
            Ok(Some(m)) | Err(m) => self.say(m),
            Ok(None) => {}
        }
        self.invalidate();
    }

    /// The design changed: the generated G-code is stale.
    pub fn invalidate(&mut self) {
        self.gcode = None;
    }

    pub fn ws(&self) -> (f64, f64) {
        (self.machine.workspace_x, self.machine.workspace_y)
    }

    pub fn connected(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| c.shared.lock().unwrap().link != Some(Link::Disconnected))
    }

    pub fn busy(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| c.shared.lock().unwrap().busy)
    }

    /// Head position in machine coordinates.
    pub fn head(&self) -> Option<P> {
        let c = self.conn.as_ref()?;
        let s = c.shared.lock().unwrap();
        if s.status.state.is_empty() { None } else { Some([s.status.x, s.status.y]) }
    }

    pub fn head_canvas(&self) -> Option<P> {
        self.head().filter(|_| self.connected()).map(|h| to_machine(&self.machine, h))
    }

    pub fn send(&mut self, line: &str) {
        if self.connected() {
            if let Some(c) = &self.conn {
                c.send(line.to_string());
            }
        } else {
            self.say(l("Connetti prima il laser (pannello Laser)", "Connect a laser first (Laser panel)"));
        }
    }

    pub fn connect(&mut self) {
        let addr = self.port.trim().to_string();
        if addr.is_empty() {
            return self.say(l("Scegli una porta", "Choose a port"));
        }
        match Connection::open_flavor(&addr, self.machine.baud_rate.max(1) as u32, self.machine.transfer_mode == "synchronous", &self.machine.flavor) {
            Ok(c) => {
                self.conn = Some(c);
                // the controller check reads `$$`: GRBL-style controllers only, and never the simulator
                self.checked_grbl = addr.eq_ignore_ascii_case("SIM") || !crate::connection::grbl_family(&self.machine.flavor);
                self.feed_ovr = 100;
                self.say(format!("{} {addr}", l("Connesso a", "Connected to")));
            }
            Err(e) => self.say(format!("{}: {e}", l("Connessione fallita", "Connection failed"))),
        }
    }

    pub fn job_opts(&self, only_selection: bool) -> JobOpts {
        let sel = (!self.doc.selected.is_empty()).then(|| self.doc.selected.clone());
        JobOpts {
            opt: self.opt.clone(),
            only: if only_selection { sel.clone() } else { None },
            origin_ids: if self.prefs.origin_from_selection { sel } else { None },
            start_from: self.start_from.clone(),
            anchor: self.job_origin,
            head: self.head().unwrap_or([0.0, 0.0]),
            user_origin: self.user_origin,
            cylinder: Some(self.cylinder.clone()),
            print_cut: self.print_cut.clone(),
        }
    }

    pub fn build_job(&mut self, only_selection: bool) -> (laserforge_core::Job, BuildReport) {
        let jo = self.job_opts(only_selection);
        let images = &mut self.images;
        let (job, rep) = self.doc.build_job(&self.machine, &jo, &mut |s, iv, pt| images.get(s, iv, pt));
        let msg = (rep.n_out, rep.n_clip);
        if msg != (0, 0) && msg != self.last_area_msg {
            let mut parts = Vec::new();
            if rep.n_out > 0 {
                parts.push(l(&format!("{} forme fuori dall'area ignorate", rep.n_out), &format!("{} shapes outside the area ignored", rep.n_out)));
            }
            if rep.n_clip > 0 {
                parts.push(l(&format!("{} tagliate al bordo", rep.n_clip), &format!("{} cut at the edge", rep.n_clip)));
            }
            parts.extend(rep.skipped.iter().cloned());
            self.say(parts.join(" · "));
        }
        self.last_area_msg = msg;
        (job, rep)
    }

    pub fn generate_gcode(&mut self) -> bool {
        let (job, _) = self.build_job(self.prefs.cut_selected);
        if job.layers.is_empty() && job.rasters.is_empty() {
            self.say(tr("noLayers"));
            return false;
        }
        // images are capped at MAX_RASTER_PX per side: say so when the interval asks for more
        let cap = crate::project::MAX_RASTER_PX as f64 + 1.0;
        let too_big = crate::shapes::flatten(&self.doc.shapes).iter().any(|s| {
            s.kind() == "image" && {
                let li = s.layer().max(0) as usize;
                let pt = self.doc.layers.get(li).is_some_and(|x| x.pass_through);
                !pt && s.f("w").max(s.f("h")) / self.doc.layer_interval(li) > cap
            }
        });
        if too_big {
            self.say(l(&format!("Immagine troppo grande per l'intervallo: ridotta a {} px per lato", crate::project::MAX_RASTER_PX),
                       &format!("Image too large for the interval: reduced to {} px per side", crate::project::MAX_RASTER_PX)));
        }
        let out = laserforge_core::generate(&job);
        let lines = out.gcode.lines().count();
        self.status = format!("G-code: {lines} {}, {}", l("righe", "lines"), fmt_time(out.seconds));
        self.gcode_origin = (job.origin_mode.clone(), job.start);
        self.gcode = Some(out);
        true
    }

    /// Bounding box (machine coordinates) of everything the job burns.
    pub fn job_bounds(job: &laserforge_core::Job) -> Option<[f64; 4]> {
        let mut pts: Vec<P> = job.layers.iter().flat_map(|l| l.paths.iter().flatten().copied()).collect();
        for r in &job.rasters {
            pts.push([r.raster.x, r.raster.y]);
            pts.push([r.raster.x + r.raster.w, r.raster.y + r.raster.h]);
        }
        (!pts.is_empty()).then(|| path_box(&pts))
    }

    pub fn start_laser(&mut self) {
        if self.gcode.is_none() && !self.generate_gcode() {
            return;
        }
        if !self.connected() {
            self.say(l("Connetti prima il laser (pannello Laser)", "Connect a laser first (Laser panel)"));
            self.show_panel("winLaser");
            return;
        }
        if self.busy() {
            return self.say(l("Il laser sta già lavorando", "The laser is already running"));
        }
        if self.prefs.checklist_enabled && !self.checklist_passed {
            self.dialogs.push(Dialog::Generic(Box::new(crate::dialogs2::RunChecklist { ticks: Vec::new() })));
            return;
        }
        self.checklist_passed = false;
        let (job, _) = self.build_job(self.prefs.cut_selected);
        if !self.confirm_in_area(Self::job_bounds(&job), "start") {
            return;
        }
        let out = self.gcode.as_ref().unwrap();
        let send = laserforge_core::post::apply_feeder(&out.send, &self.machine);
        let est = out.seconds;
        if let Some(c) = &self.conn {
            c.stream(send);
        }
        self.job = Some(JobRun { started: Instant::now(), est, done_at: None, paused: false, stopped: false });
        self.show_panel("winLaser");
        self.say(l("Lavoro inviato", "Job sent"));
        self.advance_variable_text();
    }

    /// confirm_in_area: a job or frame (machine box) leaving the work area asks first; false = wait for the answer.
    pub fn confirm_in_area(&mut self, b: Option<[f64; 4]>, then: &str) -> bool {
        if std::mem::take(&mut self.area_ok) {
            return true;
        }
        let Some(b) = b else { return true };
        let (w, h) = self.ws();
        let over = 0f64.max(-b[0]).max(-b[1]).max(b[0] + b[2] - w).max(b[1] + b[3] - h);
        if over < 0.05 {
            return true;
        }
        self.dialogs.push(Dialog::ConfirmArea { over, then: then.into(), to_abs: false });
        false
    }

    fn advance_variable_text(&mut self) {
        let mut changed = false;
        fn walk(s: &mut Shape, changed: &mut bool) {
            if let Some(Value::Object(var)) = s.0.get_mut("var") {
                if var.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let serial = var.get("serial").and_then(|v| v.as_i64()).unwrap_or(1) + var.get("step").and_then(|v| v.as_i64()).unwrap_or(1);
                    var.insert("serial".into(), json!(serial));
                    let n = var.get("csv").and_then(|v| v.as_array()).map_or(0, |a| a.len());
                    if n > 0 {
                        let row = (var.get("row").and_then(|v| v.as_u64()).unwrap_or(0) as usize + 1) % n;
                        var.insert("row".into(), json!(row));
                    }
                    *changed = true;
                }
            }
            if let Some(Value::Array(kids)) = s.0.get_mut("children") {
                for k in kids {
                    if let Value::Object(m) = k {
                        let mut c = Shape(std::mem::take(m));
                        walk(&mut c, changed);
                        *m = c.0;
                    }
                }
            }
        }
        for s in self.doc.shapes.iter_mut() {
            walk(s, &mut changed);
        }
        if changed {
            self.invalidate();
        }
    }

    pub fn frame_job(&mut self, laser_on: Option<bool>) {
        let src: Vec<Shape> = if self.doc.selected.is_empty() { self.doc.shapes.clone() } else { self.doc.selected_shapes() };
        let (w, h) = self.ws();
        let outs: Vec<Shape> = crate::shapes::flatten(&src).into_iter().filter(|s| {
            let l = s.layer();
            l >= 0 && (l as usize) < self.doc.layers.len() && !self.doc.layers[l as usize].tool && self.doc.layers[l as usize].enabled && {
                let b = s.bounds();
                !(b[0] > w || b[1] > h || b[0] + b[2] < 0.0 || b[1] + b[3] < 0.0)
            }
        }).collect();
        let Some(b) = multi_bounds(&outs) else { return self.say(tr("noLayers")) };
        let (x0, y0, x1, y1) = (b[0].max(0.0), b[1].max(0.0), (b[0] + b[2]).min(w), (b[1] + b[3]).min(h));
        if x1 <= x0 || y1 <= y0 {
            return self.say(tr("noLayers"));
        }
        let mut corners = vec![[x0, y0], [x1, y0], [x0, y1], [x1, y1]];
        if self.start_from == "current" || self.start_from == "user" {
            let (fx, fy) = crate::project::ORIGIN_POS[self.job_origin];
            let t = to_machine(&self.machine, if self.start_from == "current" { self.head().unwrap_or([0.0, 0.0]) } else { self.user_origin });
            let (dx, dy) = (t[0] - (x0 + (x1 - x0) * fx), t[1] - (y0 + (y1 - y0) * fy));
            for c in corners.iter_mut() {
                c[0] += dx;
                c[1] += dy;
            }
        }
        if !self.connected() {
            return self.say(l("Connetti prima il laser", "Connect a laser first"));
        }
        let mb = path_box(&corners.iter().map(|c| to_machine(&self.machine, *c)).collect::<Vec<_>>());
        if !self.frame_continuous && !self.confirm_in_area(Some(mb), "frame") {
            return;
        }
        let on = laser_on.unwrap_or(self.prefs.frame_laser_on);
        let head = self.head().map(|h| (h[0], h[1]));
        let rel = if self.start_from == "current" { head } else { None };
        let m = &self.machine;
        let g = Generator::new(m).generate_frame((mb[0], mb[1], mb[0] + mb[2], mb[1] + mb[3]), on, Some(self.prefs.frame_power), self.jog_speed, rel, head);
        let g = translate_flavor(&apply_rotary_axis(&g, m), m);
        if let Some(c) = &self.conn {
            c.stream(g);
        }
        self.last_frame = Some(Instant::now());
    }

    pub fn jog(&mut self, dx: f64, dy: f64) {
        self.jog_xyz(dx, dy, 0.0);
    }

    /// engine.jog: X/Y in screen directions (mapped through the origin corner), Z, rotary on A.
    pub fn jog_xyz(&mut self, dx: f64, dy: f64, dz: f64) {
        let m = self.machine.clone();
        let o = m.origin.as_str();
        let (sx, sy) = (if matches!(o, "fl" | "rl") { 1.0 } else { -1.0 }, if matches!(o, "fl" | "fr") { -1.0 } else { 1.0 });
        let (x, mut y) = (dx * self.jog_step * sx, dy * self.jog_step * sy);
        let mut z = dz * self.jog_step;
        if z != 0.0 && !m.z_enabled {
            self.say(l("Asse Z non abilitato (Impostazioni dispositivo → Movimento)", "Z axis not enabled (Device Settings → Motion)"));
            z = 0.0;
        }
        let mut words = vec![format!("X{x:.3}")];
        if m.rotary_enabled && m.rotary_axis == "A" && y != 0.0 {
            let a = if m.rotary_diameter > 0.0 { y * m.rotary_mm_per_rev / (std::f64::consts::PI * m.rotary_diameter) } else { y };
            y = 0.0;
            words.push(format!("Y{y:.3}"));
            words.push(format!("A{a:.3}"));
        } else {
            words.push(format!("Y{y:.3}"));
        }
        if z != 0.0 {
            words.push(format!("Z{z:.3}"));
        }
        if x == 0.0 && y == 0.0 && z == 0.0 && words.len() < 3 {
            return;
        }
        let f = self.jog_speed;
        if matches!(m.flavor.as_str(), "marlin" | "linuxcnc") {
            // no jog command: a relative rapid move, then back to absolute
            let axes: Vec<&String> = words.iter().filter(|w| w[1..].parse::<f64>().map(|v| v != 0.0).unwrap_or(false)).collect();
            if !axes.is_empty() {
                let axes = axes.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
                self.send("G91");
                self.send(&format!("G0 {axes} F{f:.0}"));
                self.send("G90");
            }
            return;
        }
        self.send(&format!("$J=G91 G21 {} F{f:.0}", words.join(" ")));
    }

    /// Fire button: the beam on at low power only while held (G1 without moving in GRBL laser mode).
    pub fn fire(&mut self, on: bool) {
        if !self.machine.fire_enabled {
            return;
        }
        if on && !self.connected() {
            return self.say(l("Connetti prima il laser", "Connect a laser first"));
        }
        if on && self.busy() {
            return;
        }
        if on {
            let s = self.machine.fire_power / 100.0 * self.machine.max_power as f64;
            let t = format!("{s:.1}");
            let t = t.trim_end_matches('0').trim_end_matches('.');
            self.send("M3");
            self.send(&format!("G1 F100 S{t}"));
        } else {
            for c in ["S0", "G0", "M5"] {
                self.send(c);
            }
        }
    }

    /// Move the head to a canvas point.
    pub fn goto_canvas(&mut self, p: P) {
        let m = to_machine(&self.machine, p);
        self.send(&format!("G90 G0 X{:.3} Y{:.3}", m[0], m[1]));
    }

    pub fn show_panel(&mut self, id: &str) {
        if !self.panels.iter().any(|p| p == id) {
            self.panels.push(id.into());
        }
        if crate::panels::is_lower_panel(id) {
            self.side_tab2 = id.into();
        } else {
            self.side_tab = id.into();
        }
    }

    // ------------------------------------------------------------------ files
    pub fn base_name(&self) -> String {
        self.file.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tr("untitled"))
    }

    pub fn add_recent(&mut self, p: &str) {
        self.recent.retain(|x| x != p);
        self.recent.insert(0, p.to_string());
        self.recent.truncate(10);
    }

    pub fn new_project(&mut self) {
        self.doc = Doc { layers: default_layers(), ..Doc::default() };
        self.file = None;
        self.print_cut = None;
        self.invalidate();
    }

    pub fn open_path(&mut self, path: &PathBuf) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                self.recent.retain(|x| *x != path.to_string_lossy());
                return self.say(l("File progetto non valido o mancante", "Invalid or missing project file"));
            }
        };
        match Doc::from_project(&text) {
            Ok((d, ws)) => {
                if let Some((x, y)) = ws {
                    self.machine.workspace_x = x;
                    self.machine.workspace_y = y;
                }
                self.doc = d;
                self.file = Some(path.clone());
                self.add_recent(&path.to_string_lossy());
                self.canvas.fit_pending = 3;
                self.invalidate();
                let notes = self.doc.notes.clone();
                if notes.get("show_on_load").and_then(|v| v.as_bool()).unwrap_or(false) {
                    if let Some(t) = notes.get("text").and_then(|v| v.as_str()) {
                        self.dialogs.push(Dialog::Message { title: l("Note del progetto", "Project notes"), text: t.to_string() });
                    }
                }
            }
            Err(e) => self.say(format!("{}: {e}", l("Impossibile aprire", "Cannot open"))),
        }
    }

    pub fn save_project(&mut self, as_new: bool) -> bool {
        let path = match (&self.file, as_new) {
            (Some(p), false) => p.clone(),
            _ => match rfd::FileDialog::new().add_filter("LaserForge", &["lfproj"]).set_file_name(format!("{}.lfproj", self.base_name())).save_file() {
                Some(p) => if p.extension().is_some_and(|e| e == "lfproj") { p } else { p.with_extension("lfproj") },
                None => return false,
            },
        };
        let (w, h) = self.ws();
        match std::fs::write(&path, self.doc.to_project(w, h)) {
            Ok(_) => {
                self.doc.dirty = false;
                self.add_recent(&path.to_string_lossy());
                self.say(format!("{}: {}", l("Salvato", "Saved"), path.file_name().unwrap().to_string_lossy()));
                self.file = Some(path);
                true
            }
            Err(e) => {
                self.say(format!("Errore: {e}"));
                false
            }
        }
    }

    pub fn save_text_file(&mut self, text: &str, name: &str, filter: (&str, &[&str])) {
        if let Some(p) = rfd::FileDialog::new().add_filter(filter.0, filter.1).set_file_name(name).save_file() {
            match std::fs::write(&p, text) {
                Ok(_) => self.say(p.file_name().unwrap().to_string_lossy().to_string()),
                Err(e) => self.say(format!("Errore: {e}")),
            }
        }
    }

    pub fn export_svg(&mut self) {
        let (w, h) = self.ws();
        let mut parts = Vec::new();
        for s in crate::shapes::flatten(&self.doc.shapes) {
            let col = self.doc.layers.get(s.layer().max(0) as usize).map_or("#000000".to_string(), |l| l.color.clone());
            if s.kind() == "image" {
                parts.push(format!("<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\" preserveAspectRatio=\"none\"/>",
                                   s.f("x"), s.f("y"), s.f("w"), s.f("h"), s.0.get("src").and_then(|v| v.as_str()).unwrap_or("")));
                continue;
            }
            for p in s.paths() {
                let pts: Vec<String> = p.iter().map(|q| format!("{:.3},{:.3}", q[0], q[1])).collect();
                parts.push(format!("<polyline fill=\"none\" stroke=\"{col}\" stroke-width=\"0.1\" points=\"{}\"/>", pts.join(" ")));
            }
        }
        let svg = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">\n{}\n</svg>\n", parts.join("\n"));
        let name = format!("{}.svg", self.base_name());
        self.save_text_file(&svg, &name, ("SVG", &["svg"]));
    }

    pub fn save_gcode(&mut self) {
        if self.gcode.is_none() && !self.generate_gcode() {
            return;
        }
        let text = self.gcode.as_ref().unwrap().send.clone();
        let name = format!("{}.gcode", self.base_name());
        self.save_text_file(&text, &name, ("G-code", &["gcode", "nc"]));
    }

    pub fn import_file(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter(&l("Immagini, vettori, progetti", "Images, vectors, projects"),
                        &["svg", "dxf", "pdf", "ai", "png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff", "lfproj", "gbr", "ger", "gtl", "gbl", "gto", "gbo", "gts", "gbs", "gko", "drl", "xln", "stl"])
            .add_filter(&l("Tutti", "All"), &["*"]).pick_file() else { return };
        self.open_dropped(&p);
    }

    pub fn open_dropped(&mut self, path: &PathBuf) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let add = |app: &mut LaserApp, shapes: Vec<importers::Props>, what: &str| {
            if shapes.is_empty() {
                return app.say(l("Nessun vettore trovato", "No vectors found"));
            }
            let n = shapes.len();
            let created: Vec<Shape> = shapes.into_iter().map(|p| app.doc.create_from(p)).collect();
            app.doc.add_shapes(created, true);
            app.invalidate();
            app.say(l(&format!("{n} forme importate{what}"), &format!("{n} shapes imported{what}")));
        };
        match ext.as_str() {
            "lfproj" | "json" => self.open_path(path),
            "svg" => match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| importers::svg_shapes(&t)) {
                Ok(s) => add(self, s, ""),
                Err(e) => self.say(e),
            },
            "dxf" => match std::fs::read(path) {
                Ok(b) => add(self, importers::vector_shapes(importers::dxf_to_paths(&String::from_utf8_lossy(&b))), &format!(" ({name})")),
                Err(e) => self.say(e.to_string()),
            },
            "pdf" | "ai" => {
                let Ok(data) = std::fs::read(path) else { return self.say(l("File non leggibile", "Cannot read the file")) };
                let pages = importers::pdf_page_count(&data);
                if pages > 1 {
                    self.dialogs.push(Dialog::PdfPage { path: path.clone(), pages, page: 1 });
                } else {
                    self.import_pdf(&data, 0, &name);
                }
            }
            "gbr" | "ger" | "gtl" | "gbl" | "gto" | "gbo" | "gts" | "gbs" | "gko" | "gm1" | "drl" | "xln" | "nc_drill" | "txt" => self.import_pcb(path),
            "stl" | "obj" | "ply" | "3mf" | "amf" => self.dialogs.push(Dialog::Import3d { path: path.clone(), px_mm: 0.1, invert: false }),
            _ => match std::fs::read(path) {
                Ok(b) => match importers::image_shape(&b, &name, self.ws(), Some(&path.to_string_lossy())) {
                    Some(p) => {
                        let (w, h) = (p["w"].as_f64().unwrap_or(0.0), p["h"].as_f64().unwrap_or(0.0));
                        let s = self.doc.create_from(p);
                        self.doc.add_shapes(vec![s], true);
                        self.invalidate();
                        self.say(format!("{name}: {w:.1} × {h:.1} mm"));
                    }
                    None => self.say(l("Immagine non supportata", "Unsupported image")),
                },
                Err(e) => self.say(e.to_string()),
            },
        }
    }

    pub fn import_pdf(&mut self, data: &[u8], page: usize, name: &str) {
        let mut imgs = Vec::new();
        match importers::pdf_to_paths(data, page, Some(&mut imgs)) {
            Ok(paths) => {
                let mut shapes = importers::pdf_image_shapes(&imgs, name);
                shapes.extend(importers::vector_shapes(paths));
                if shapes.is_empty() {
                    return self.say(l("Nessun vettore trovato (testo e immagini nei PDF non vengono importati)", "No vectors found (text and images inside PDFs are not imported)"));
                }
                let n = shapes.len();
                let created: Vec<Shape> = shapes.into_iter().map(|p| self.doc.create_from(p)).collect();
                self.doc.add_shapes(created, true);
                self.invalidate();
                self.say(l(&format!("{n} forme importate da {name}"), &format!("{n} shapes imported from {name}")));
            }
            Err(e) => self.say(format!("{}{e}", l("Impossibile importare: ", "Cannot import: "))),
        }
    }

    pub fn import_pcb(&mut self, path: &PathBuf) {
        let Ok(bytes) = std::fs::read(path) else { return self.say(l("File PCB non leggibile", "Cannot read PCB file")) };
        let text: String = bytes.iter().map(|&b| b as char).collect();
        let lower = path.to_string_lossy().to_lowercase();
        let drill = text[..text.len().min(2000)].contains("M48") || lower.ends_with(".drl") || lower.ends_with(".xln");
        let rings = crate::pcb::rings(&text, drill, self.ws().1);
        if rings.is_empty() {
            return self.say(l("Nessuna geometria nel file PCB", "No geometry in the PCB file"));
        }
        let layer = self.doc.active_layer;
        let kids: Vec<Value> = rings.iter().filter(|r| r.len() > 3).map(|r| {
            let pts = crate::shapes::simplify(&r[..r.len() - 1], 0.003);
            json!({"id": self.doc.new_id(), "type": "poly", "layer": layer, "closed": true, "points": pts.iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()})
        }).collect();
        let n = kids.len();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut props = serde_json::Map::new();
        props.insert("children".into(), json!(kids));
        props.insert("name".into(), json!(name.clone()));
        let g = self.doc.create("group", props);
        self.doc.add_shapes(vec![g], true);
        self.invalidate();
        self.say(format!("{name}: {n} {}", l(if drill { "contorni (fori)" } else { "contorni (rame/serigrafia)" }, if drill { "outlines (drills)" } else { "outlines (copper/silk)" })));
    }

    pub fn apply_device(&mut self, dev: &Value) {
        if let Some(m) = dev.get("machine") {
            let mut base = serde_json::to_value(&self.machine).unwrap();
            if let (Value::Object(b), Value::Object(n)) = (&mut base, m) {
                for (k, v) in n {
                    if b.contains_key(k) && !v.is_null() {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Ok(mm) = serde_json::from_value(base) {
                self.machine = mm;
            }
        }
        if let Some(p) = dev.get("port").and_then(|v| v.as_str()).filter(|p| !p.is_empty()) {
            self.port = p.to_string();
        }
        self.device = dev.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        self.canvas.fit_pending = 2;
        self.invalidate();
        self.say(format!("{}{}", l("Dispositivo: ", "Device: "), self.device));
    }

    /// Automatic backup of the open design every few minutes (Settings › Backup).
    fn backup(&mut self) {
        if !self.prefs.backup_enabled || self.no_save || self.doc.shapes.is_empty()
            || self.last_backup.elapsed() < Duration::from_secs_f64(self.prefs.backup_minutes.max(1.0) * 60.0) {
            return;
        }
        self.last_backup = Instant::now();
        let dir = crate::dialogs2::backup_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = format!("{}-{secs}.lfproj", self.base_name());
        let text = self.persist_doc();
        let _ = std::fs::write(dir.join(name), text);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        while files.len() > self.prefs.backup_keep.max(1) {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }

    /// Start or stop the phone remote to match the settings.
    pub fn sync_remote(&mut self) {
        if self.prefs.remote_enabled && !self.no_save {
            if self.prefs.remote_pin.is_empty() {
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1234);
                self.prefs.remote_pin = format!("{:04}", n % 10000);
            }
            if self.remote.is_none() {
                match crate::remote::Remote::start(self.prefs.remote_port, &self.prefs.remote_pin) {
                    Ok(r) => {
                        self.say(format!("{}{}", l("Controllo remoto attivo: ", "Remote control on: "), r.url));
                        self.remote = Some(r);
                    }
                    Err(e) => self.say(format!("{}{e}", l("Controllo remoto non avviato: ", "Remote control not started: "))),
                }
            }
        } else {
            self.remote = None;
        }
    }

    fn poll_remote(&mut self) {
        let Some(r) = &self.remote else { return };
        {
            let mut st = r.status.lock().unwrap();
            let (state, x, y, sent, total) = match &self.conn {
                Some(c) => { let s = c.shared.lock().unwrap(); (s.status.state.clone(), s.status.x, s.status.y, s.acked, s.total) }
                None => ("Disconnected".into(), 0.0, 0.0, 0, 0),
            };
            *st = crate::remote::Snapshot { state, x, y, sent, total, checklist: self.prefs.checklist_enabled };
        }
        let cmds: Vec<(String, f64, f64)> = r.rx.try_iter().collect();
        for (a, x, y) in cmds {
            if !self.connected() && a != "stop" {
                self.say(l("Comando remoto ignorato: laser non connesso", "Remote command ignored: laser not connected"));
                continue;
            }
            match a.as_str() {
                "start" => self.run("startLaser"),
                "pause" => self.run("pauseLaser"),
                "resume" => self.run("resumeLaser"),
                "stop" => self.run("stopLaser"),
                "frame" => self.run("frame"),
                "home" => self.run("homeLaser"),
                "jog" => {
                    let o = self.machine.origin.as_str();
                    let (sx, sy) = (if matches!(o, "fl" | "rl") { 1.0 } else { -1.0 }, if matches!(o, "fl" | "fr") { -1.0 } else { 1.0 });
                    let f = self.jog_speed;
                    self.send(&format!("$J=G91 G21 X{:.3} Y{:.3} F{f:.0}", x * sx, y * sy));
                }
                "air_on" => { let c = self.machine.air_command.clone(); self.send(&c); }
                "air_off" => self.send("M9"),
                _ => {}
            }
            self.status = format!("{}{a}", l("Comando dal telefono: ", "Remote command: "));
        }
    }

    pub fn persist_doc(&mut self) -> String {
        let (w, h) = self.ws();
        self.doc.to_project(w, h)
    }

    pub fn palette_clicked(&mut self, idx: usize) {
        if !self.doc.selected.is_empty() {
            let r = self.doc.set_layer_sel(idx);
            self.msg(r);
        }
        self.doc.active_layer = idx;
        self.side_tab = "winCuts".into();
    }

    pub fn set_all_layers(&mut self, f: impl Fn(&LayerSettings) -> Option<Value>) {
        for i in 0..self.doc.layers.len() {
            if let Some(Value::Object(patch)) = f(&self.doc.layers[i]) {
                crate::project::apply_layer_dict(&mut self.doc.layers[i], &patch);
            }
        }
        self.invalidate();
    }

    // ------------------------------------------------------------------ commands
    pub fn run(&mut self, aid: &str) {
        if let Some(t) = tool_of(aid) {
            if aid == "toolSelect" {
                if self.canvas.cancel_in_progress() {
                    return;
                }
                if self.canvas.tool == "select" {
                    self.doc.set_selection(Vec::new());
                    return;
                }
            }
            self.canvas.set_tool(t);
            return;
        }
        let ws = (self.machine.workspace_x, self.machine.workspace_y);
        let head = self.head_canvas().unwrap_or([0.0, 0.0]);
        let dirty = self.doc.dirty;
        let d = &mut self.doc;
        let r: crate::edit::Msg = match aid {
            "open" if dirty => { self.dialogs.push(Dialog::ConfirmDiscard { then: "open".into() }); Ok(None) }
            "new" => {
                if self.doc.dirty {
                    self.dialogs.push(Dialog::ConfirmDiscard { then: "new".into() });
                } else {
                    self.new_project();
                }
                Ok(None)
            }
            "open" => {
                if let Some(p) = rfd::FileDialog::new().add_filter("LaserForge", &["lfproj", "json"]).pick_file() {
                    self.open_path(&p);
                }
                Ok(None)
            }
            "import" => { self.import_file(); Ok(None) }
            "save" => { self.save_project(false); Ok(None) }
            "saveAs" => { self.save_project(true); Ok(None) }
            "exportSvg" => { self.export_svg(); Ok(None) }
            "saveGcode" => { self.save_gcode(); Ok(None) }
            "exportPrefs" => {
                let t = serde_json::to_string_pretty(&json!({"layers": self.doc.layers, "machine": self.machine})).unwrap();
                self.save_text_file(&t, "laserforge-prefs.json", ("JSON", &["json"]));
                Ok(None)
            }
            "importPrefs" => { self.import_prefs(); Ok(None) }
            "exportBundle" => { self.export_bundle(); Ok(None) }
            "importBundle" => { self.import_bundle(); Ok(None) }
            "exit" => {
                if self.doc.dirty {
                    self.dialogs.push(Dialog::ConfirmDiscard { then: "quit".into() });
                } else {
                    self.pending_cmd = Some(egui::ViewportCommand::Close);
                }
                Ok(None)
            }
            "undo" => { d.undo(); Ok(None) }
            "redo" => { d.redo(); Ok(None) }
            "selectAll" => { d.select_by(|_| true); Ok(None) }
            "invertSelection" => { d.invert_selection(); Ok(None) }
            "selectOpen" => { d.select_by(|s| s.kind() == "poly" && !s.flag("closed")); Ok(None) }
            "selectContained" => d.select_contained(),
            "selectNested" => d.select_nested(),
            "selectCircles" => d.select_same_circles(),
            "selectSmaller" => d.select_smaller(),
            "selectLayer" => { let a = d.active_layer as i64; d.select_by(|s| s.layer() == a); Ok(None) }
            "cut" => {
                match d.need_sel(1) {
                    Ok(sel) => { self.clipboard = sel; self.doc.delete_selected(); Ok(None) }
                    Err(e) => Err(e),
                }
            }
            "copy" => match d.need_sel(1) {
                Ok(sel) => { let n = sel.len(); self.clipboard = sel; Ok(Some(l(&format!("{n} copiati"), &format!("{n} copied")))) }
                Err(e) => Err(e),
            },
            "duplicate" => d.duplicate_sel(),
            "paste" => { let c = self.clipboard.clone(); let at = self.canvas.last_mouse; self.doc.paste(&c, at); Ok(None) }
            "pasteInPlace" => { let c = self.clipboard.clone(); self.doc.paste_in_place(&c); Ok(None) }
            "delete" => { d.delete_selected(); Ok(None) }
            "deleteDuplicates" => d.delete_duplicates(),
            "closePath" => d.close_path(),
            "autoJoin" => d.auto_join(0.1),
            "breakApart" => d.break_apart(),
            "convertToPath" => d.convert_to_path(),
            "lockSel" => d.set_locked(true),
            "unlockSel" => d.set_locked(false),
            "group" => d.group_sel(),
            "ungroup" => d.ungroup_sel(),
            "flattenArray" => d.flatten_arrays(),
            "mirrorH" => d.mirror_sel("h"),
            "mirrorV" => d.mirror_sel("v"),
            "rotate90cw" => d.rotate_selection(90.0),
            "rotate90ccw" => d.rotate_selection(-90.0),
            "alignLeft" => d.align_sel("left"),
            "alignRight" => d.align_sel("right"),
            "alignTop" => d.align_sel("top"),
            "alignBottom" => d.align_sel("bottom"),
            "alignCenterH" => d.align_sel("centerH"),
            "alignCenterV" => d.align_sel("centerV"),
            "alignCenters" => d.align_sel("centers"),
            "distH" => d.distribute_sel(true, false),
            "distV" => d.distribute_sel(false, false),
            "distHC" => d.distribute_sel(true, true),
            "distVC" => d.distribute_sel(false, true),
            "moveUL" => d.move_sel_to(Place::UL, ws, head),
            "moveUR" => d.move_sel_to(Place::UR, ws, head),
            "moveLL" => d.move_sel_to(Place::DL, ws, head),
            "moveLR" => d.move_sel_to(Place::DR, ws, head),
            "moveCenter" => d.move_sel_to(Place::C, ws, head),
            "moveLeft" => d.move_sel_to(Place::L, ws, head),
            "moveRight" => d.move_sel_to(Place::R, ws, head),
            "moveTop" => d.move_sel_to(Place::U, ws, head),
            "moveBottom" => d.move_sel_to(Place::D, ws, head),
            "moveToLaser" => d.move_sel_to(Place::Laser, ws, head),
            "sameWidth" => d.same_size(true),
            "sameHeight" => d.same_size(false),
            "pushFront" => d.reorder("front"),
            "pushForward" => d.reorder("forward"),
            "pushBackward" => d.reorder("backward"),
            "pushBack" => d.reorder("back"),
            "weld" => d.geometry_op("weld", &json!({}), true, 2),
            "union" => d.geometry_op("union", &json!({}), true, 2),
            "subtract" => d.geometry_op("subtract", &json!({}), true, 2),
            "intersect" => d.geometry_op("intersect", &json!({}), true, 2),
            "rubberBand" => d.geometry_op("hull", &json!({}), false, 1),
            "mirrorLine" => d.mirror_across_line(),
            "cropImage" | "applyMask" => d.crop_image(),
            "removeCrop" => d.remove_crop(),
            "flattenCrop" => d.flatten_crop(),
            "deleteOverlaps" => d.delete_overlapping(),
            "clearTabs" => d.set_shape_attr("tabs", None),
            "resetStart" => d.reset_start_points(),
            "shapeTriangle" => { d.insert_shape("triangle", ws); Ok(None) }
            "shapePentagon" => { d.insert_shape("pentagon", ws); Ok(None) }
            "shapeOctagon" => { d.insert_shape("octagon", ws); Ok(None) }
            "shapeStar" => { d.insert_shape("star", ws); Ok(None) }
            "shapeDualStar" => { d.insert_shape("dualStar", ws); Ok(None) }
            "nesting" => { self.dialogs.push(Dialog::nesting()); Ok(None) }
            "lockAspect" => { self.lock_aspect = !self.lock_aspect; Ok(None) }
            "replaceImage" | "replaceImageFit" => { self.replace_image(aid == "replaceImageFit"); Ok(None) }
            "reloadImage" => { self.reload_image(); Ok(None) }
            // laser
            "frame" => { self.frame_job(None); Ok(None) }
            "frameContinuous" => {
                self.frame_continuous = !self.frame_continuous;
                self.status = if self.frame_continuous { l("Frame continuo attivo: Stop per annullare", "Continuous framing on: Stop to cancel") } else { tr("ready") };
                Ok(None)
            }
            "frameRubber" => { self.rubber_band_frame(); Ok(None) }
            "homeLaser" => {
                // GRBL refuses $H when homing is off: say why instead of an "error:5"
                let homing_off = crate::connection::grbl_family(&self.machine.flavor)
                    && self.conn.as_ref().is_some_and(|c| c.shared.lock().unwrap().settings.get(&22) == Some(&0.0));
                if homing_off {
                    Err(l("Homing non abilitato in GRBL ($22=0): attivalo in Laser → Impostazioni macchina",
                          "Homing is not enabled in GRBL ($22=0): enable it in Laser → Machine Settings"))
                } else {
                    let cmd = match self.machine.flavor.as_str() { "marlin" => "G28 X Y", "smoothie" => "G28.2 X0 Y0", "linuxcnc" => "G28", _ => "$H" };
                    self.send(cmd);
                    Ok(None)
                }
            }
            "unlock" => { self.send("$X"); Ok(None) }
            "gotoOrigin" => { self.send("G90 G0 X0 Y0"); Ok(None) }
            "setUserOrigin" => { self.set_user_origin(); Ok(None) }
            "laserUL" | "laserUR" | "laserLL" | "laserLR" | "laserCenter" => {
                match self.doc.need_sel(1) {
                    Ok(sel) => {
                        let b = multi_bounds(&sel).unwrap();
                        let x = if aid.ends_with("UL") || aid.ends_with("LL") { b[0] } else if aid == "laserCenter" { b[0] + b[2] / 2.0 } else { b[0] + b[2] };
                        let y = if aid.ends_with("UL") || aid.ends_with("UR") { b[1] } else if aid == "laserCenter" { b[1] + b[3] / 2.0 } else { b[1] + b[3] };
                        self.goto_canvas([x, y]);
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }
            "startLaser" => { self.start_laser(); Ok(None) }
            "pauseLaser" => { if let Some(c) = &self.conn { c.pause(); } if let Some(j) = &mut self.job { j.paused = true; } Ok(None) }
            "resumeLaser" => { if let Some(c) = &self.conn { c.resume(); } if let Some(j) = &mut self.job { j.paused = false; } Ok(None) }
            "stopLaser" => {
                self.frame_continuous = false;
                if let Some(c) = &self.conn { c.stop(); }
                if let Some(j) = &mut self.job { j.stopped = true; }
                Ok(None)
            }
            "controllerInfo" => { self.send("$I"); self.show_panel("winConsole"); Ok(None) }
            "airAllOn" => { self.set_all_layers(|l| (l.air_assist == AirAssist::Off).then(|| json!({"air_assist": "on"}))); Ok(None) }
            "airAllOff" => { self.set_all_layers(|_| Some(json!({"air_assist": "off"}))); Ok(None) }
            "airInvert" => { self.set_all_layers(|l| Some(json!({"air_assist": if l.air_assist == AirAssist::Off { "on" } else { "off" }}))); Ok(None) }
            "layersAllOn" => { self.set_all_layers(|_| Some(json!({"enabled": true}))); Ok(None) }
            "layersAllOff" => { self.set_all_layers(|_| Some(json!({"enabled": false}))); Ok(None) }
            "layersInvert" => { self.set_all_layers(|l| Some(json!({"enabled": !l.enabled}))); Ok(None) }
            // view
            "viewFilled" => { self.filled = !self.filled; Ok(None) }
            "showGrid" => { self.grid = !self.grid; self.prefs.grid_show = self.grid; Ok(None) }
            "showRulers" => { self.rulers = !self.rulers; Ok(None) }
            "zoomIn" => { self.canvas.zoom_step(1.25, ws); Ok(None) }
            "zoomOut" => { self.canvas.zoom_step(0.8, ws); Ok(None) }
            "zoomPage" => { self.canvas.zoom_page(ws); Ok(None) }
            "zoomSel" => {
                let sel = self.doc.selected_shapes();
                match multi_bounds(&sel) { Some(b) => { self.canvas.zoom_to_box(b); Ok(None) } None => Err(tr("selectFirst")) }
            }
            "resetLayout" => { self.panels = default_panels(); self.side_tab = "winCuts".into(); self.side_tab2 = "winLaser".into(); self.hidden_bars.clear(); Ok(None) }
            "winCuts" | "winLaser" | "winMove" | "winConsole" | "winObjects" | "winLibrary" | "winArt" | "winCamera" | "winProps" | "winPages" | "winMacros" => {
                if let Some(i) = self.panels.iter().position(|p| p == aid) {
                    self.panels.remove(i);
                } else {
                    self.show_panel(aid);
                }
                Ok(None)
            }
            "winMainToolbar" | "winModbar" | "winToolbox" | "winPalette" => {
                if let Some(i) = self.hidden_bars.iter().position(|p| p == aid) {
                    self.hidden_bars.remove(i);
                } else {
                    self.hidden_bars.push(aid.to_string());
                }
                Ok(None)
            }
            "cameraCapture" => { self.camera_capture(); Ok(None) }
            "cameraImport" => { self.camera_import(); Ok(None) }
            "langOther" => Err(l("Altre lingue in arrivo", "More languages coming")),
            "langIt" => { defs::set_english(false); Ok(None) }
            "langEn" => { defs::set_english(true); Ok(None) }
            "minimize" => { self.pending_cmd = Some(egui::ViewportCommand::Minimized(true)); Ok(None) }
            "fullscreen" => { self.fullscreen = !self.fullscreen; self.pending_cmd = Some(egui::ViewportCommand::Fullscreen(self.fullscreen)); Ok(None) }
            "docs" => { let _ = open_url("https://github.com/Amossella/LaserForge#readme"); Ok(None) }
            _ => {
                if let Some(dl) = Dialog::for_action(aid, self) {
                    self.dialogs.push(dl);
                    Ok(None)
                } else {
                    Err(tr("notAvailable"))
                }
            }
        };
        self.msg(r);
    }

    fn import_prefs(&mut self) {
        let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() else { return };
        let Ok(d) = std::fs::read_to_string(&p).map_err(|_| ()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ())) else {
            return self.say(l("JSON non valido", "Invalid JSON"));
        };
        if let Some(m) = d.get("machine") {
            let mut base = serde_json::to_value(&self.machine).unwrap();
            if let (Value::Object(b), Value::Object(n)) = (&mut base, m) {
                for (k, v) in n {
                    if b.contains_key(k) && !v.is_null() {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Ok(mm) = serde_json::from_value(base) {
                self.machine = mm;
            }
        }
        if let Some(ls) = d.get("layers").and_then(|v| v.as_array()) {
            for (i, lv) in ls.iter().enumerate().take(self.doc.layers.len()) {
                if let Some(m) = lv.as_object() {
                    crate::project::apply_layer_dict(&mut self.doc.layers[i], m);
                }
            }
        }
        self.invalidate();
    }

    fn set_user_origin(&mut self) {
        let Some(h) = self.head().filter(|_| self.connected()) else {
            return self.say(l("Connetti il laser: l'origine utente è la posizione attuale della testa", "Connect the laser: the user origin is the head's current position"));
        };
        self.user_origin = h;
        self.user_origin_set = true;
        self.invalidate();
        self.say(format!("{}: X {:.2} Y {:.2}", l("Origine utente", "User origin"), h[0], h[1]));
    }

    fn rubber_band_frame(&mut self) {
        let (job, _) = self.build_job(!self.doc.selected.is_empty());
        let mut pts: Vec<laserforge_core::model::Pt> = job.layers.iter().flat_map(|l| l.paths.iter().flatten().map(|q| laserforge_core::model::Pt { x: q[0], y: q[1] })).collect();
        for r in &job.rasters {
            let r = &r.raster;
            for (x, y) in [(r.x, r.y), (r.x + r.w, r.y), (r.x + r.w, r.y + r.h), (r.x, r.y + r.h)] {
                pts.push(laserforge_core::model::Pt { x, y });
            }
        }
        if pts.len() < 3 {
            return self.say(tr("noLayers"));
        }
        if !self.connected() {
            return self.say(l("Connetti prima il laser", "Connect a laser first"));
        }
        let hull = laserforge_core::geometry::geometry_op("hull", &[vec![pts.clone(), vec![pts[0]]]], &json!({})).ok()
            .and_then(|r| r.into_iter().next()).and_then(|r| r.into_iter().next()).unwrap_or_default();
        if hull.len() < 3 {
            return;
        }
        let mut g = vec!["G90".to_string(), "M5".into(), format!("G0 X{:.3} Y{:.3} F{:.0}", hull[0].x, hull[0].y, self.jog_speed)];
        for p in &hull[1..] {
            g.push(format!("G1 X{:.3} Y{:.3} S0 F{:.0}", p.x, p.y, self.jog_speed));
        }
        let gc = translate_flavor(&apply_rotary_axis(&g.join("\n"), &self.machine), &self.machine);
        if let Some(c) = &self.conn {
            c.stream(gc);
        }
    }

    fn replace_image(&mut self, fit: bool) {
        let Ok(id) = self.doc.selected_image_id() else { return self.say(tr("selectImage")) };
        let Some(p) = rfd::FileDialog::new().add_filter("Immagini", &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tif", "tiff"]).pick_file() else { return };
        let Some(img) = std::fs::read(&p).ok().and_then(|b| image::load_from_memory(&b).ok()) else { return self.say(l("Immagine non supportata", "Unsupported image")) };
        self.doc.save_undo("");
        let s = self.doc.find_mut(id).unwrap();
        s.0.insert("src".into(), json!(importers::encode_image(&img)));
        s.0.insert("name".into(), json!(p.file_name().unwrap().to_string_lossy()));
        s.0.insert("path".into(), json!(p.to_string_lossy()));
        s.0.insert("flipX".into(), json!(false));
        s.0.insert("flipY".into(), json!(false));
        s.0.remove("crop");
        if !fit {
            let w = s.f("w");
            s.set("h", w * img.height() as f64 / img.width() as f64);
        }
        self.invalidate();
    }

    fn reload_image(&mut self) {
        let Ok(id) = self.doc.selected_image_id() else { return self.say(tr("selectImage")) };
        let path = self.doc.find(id).and_then(|s| s.0.get("path")).and_then(|v| v.as_str()).map(PathBuf::from);
        let Some(img) = path.filter(|p| p.exists()).and_then(|p| std::fs::read(p).ok()).and_then(|b| image::load_from_memory(&b).ok()) else {
            return self.say(l("File originale non trovato: usa Sostituisci immagine", "Original file not found: use Replace Image"));
        };
        self.doc.save_undo("");
        self.doc.find_mut(id).unwrap().0.insert("src".into(), json!(importers::encode_image(&img)));
        self.invalidate();
        self.say(l("Immagine ricaricata", "Image reloaded"));
    }

    pub fn checked(&self, aid: &str) -> Option<bool> {
        Some(match aid {
            "lockAspect" => self.lock_aspect,
            "viewFilled" => self.filled,
            "showGrid" => self.grid,
            "showRulers" => self.rulers,
            "frameContinuous" => self.frame_continuous,
            "langIt" => !defs::english(),
            "langEn" => defs::english(),
            "winMainToolbar" | "winModbar" | "winToolbox" | "winPalette" => !self.hidden_bars.iter().any(|p| p == aid),
            a if a.starts_with("win") => self.panels.iter().any(|p| p == a),
            a => return tool_of(a).map(|t| self.canvas.tool == t),
        })
    }

    pub fn shortcut_of(&self, aid: &str) -> String {
        self.shortcut_overrides.get(aid).cloned().unwrap_or_else(|| defs::shortcut(aid))
    }

    // ------------------------------------------------------------------ menus and tool bars
    fn menu_items(&mut self, ui: &mut egui::Ui, items: &[MenuItem]) {
        for it in items {
            match it {
                MenuItem::Sep => {
                    ui.separator();
                }
                MenuItem::Action { id, .. } => {
                    let label = defs::label(id).replace("...", "…");
                    let key = self.shortcut_of(id);
                    let mut b = match self.checked(id) {
                        Some(on) => egui::Button::new(format!("{}{}", if on { "✓ " } else { "    " }, label)),
                        None => egui::Button::new(format!("    {label}")),
                    };
                    if !key.is_empty() {
                        b = b.shortcut_text(key.replace("Ctrl", "⌘").replace("Shift", "⇧").replace("Alt", "⌥").replace('+', ""));
                    }
                    if ui.add(b).clicked() {
                        let id = id.clone();
                        ui.close();
                        self.run(&id);
                    }
                }
                MenuItem::Sub { it, en, items } => {
                    let items = items.clone();
                    ui.menu_button(if defs::english() { en } else { it }, |ui| self.menu_items(ui, &items));
                }
                MenuItem::Recent { it, en } => {
                    let recent = self.recent.clone();
                    ui.menu_button(if defs::english() { en } else { it }, |ui| {
                        if recent.is_empty() {
                            ui.weak(l("(vuoto)", "(empty)"));
                        }
                        for p in recent {
                            let name = std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
                            if ui.button(name).on_hover_text(&p).clicked() {
                                ui.close();
                                self.open_path(&PathBuf::from(&p));
                            }
                        }
                    });
                }
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for m in defs::menus() {
                let items = m.items.clone();
                ui.menu_button(if defs::english() { &m.en } else { &m.it }, |ui| {
                    ui.set_min_width(240.0);
                    self.menu_items(ui, &items);
                });
            }
        });
    }

    /// A tool-bar button with the LaserForge icon.
    pub fn icon_button(&mut self, ui: &mut egui::Ui, aid: &str, icon: &str) -> bool {
        let size = self.prefs.icon_size.clamp(14.0, 32.0) as f32;
        let on = self.checked(aid).unwrap_or(false);
        let (rect, resp) = ui.allocate_exact_size(vec2(size + 8.0, size + 8.0), Sense::click());
        let vis = ui.style().interact_selectable(&resp, on);
        if on || resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, vis.weak_bg_fill);
        }
        let col = ui.visuals().text_color();
        let r = Rect::from_center_size(rect.center(), vec2(size, size));
        let k = size / 24.0;
        for (poly, _) in defs::icon(icon) {
            let pts: Vec<egui::Pos2> = poly.iter().map(|q| r.min + vec2(q[0] as f32 * k, q[1] as f32 * k)).collect();
            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, col)));
        }
        if defs::icon(icon).is_empty() {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &defs::label(aid).chars().next().unwrap_or('?').to_string(), egui::FontId::proportional(size * 0.6), col);
        }
        let key = self.shortcut_of(aid);
        let tip = if key.is_empty() { defs::label(aid) } else { format!("{}  ({key})", defs::label(aid)) };
        let clicked = resp.clicked();
        resp.on_hover_text(tip);
        if clicked {
            self.run(aid);
        }
        clicked
    }

    fn main_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let mut all = defs::toolbar("MAIN_TOOLBAR");
            all.push(None);
            all.extend(defs::toolbar("MODBAR"));
            for b in all {
                match b {
                    Some((aid, icon)) => {
                        self.icon_button(ui, &aid, &icon);
                    }
                    None => {
                        ui.separator();
                    }
                }
            }
        });
    }

    /// Rebuild the macOS menu bar when the language, recent files or shortcuts change.
    fn update_native_menu(&mut self, ctx: &egui::Context) {
        if !crate::native_menu::SYSTEM_MENU_BAR {
            return;
        }
        let sig = format!("{}|{:?}|{:?}", defs::english(), self.recent, {
            let mut v: Vec<_> = self.shortcut_overrides.iter().collect();
            v.sort();
            v
        });
        if self.native_menu.as_ref().is_some_and(|m| m.sig == sig) {
            return;
        }
        let recent = self.recent.clone();
        let menu = {
            let me = &*self;
            crate::native_menu::build(&recent, &|aid: &str| me.shortcut_of(aid), sig, ctx)
        };
        self.native_menu = menu;
    }

    /// Property bar: position, size and scale of the selection around a reference point, plus text options.
    fn property_bar(&mut self, ui: &mut egui::Ui) {
        let sel = self.doc.selected_shapes();
        let b = multi_bounds(&sel);
        let u = self.prefs.unit_factor();
        let unit = if self.prefs.units == "in" { "in" } else { "mm" };
        let (ax, ay) = ((self.anchor % 3) as f64 * 0.5, (self.anchor / 3) as f64 * 0.5);
        // shown values: the reference point, the size, 100 %
        let shown = |k: usize| -> f64 {
            match (k, b) {
                (0, Some(b)) => (b[0] + ax * b[2]) * u,
                (1, Some(b)) => (b[1] + ay * b[3]) * u,
                (2, Some(b)) => b[2] * u,
                (3, Some(b)) => b[3] * u,
                (5, _) | (6, _) => 100.0,
                _ => 0.0,
            }
        };
        let mut commit: Option<(usize, f64)> = None;
        let lock = self.lock_aspect;
        ui.horizontal(|ui| {
            egui::Grid::new("propgrid").num_columns(7).spacing([6.0, 4.0]).show(ui, |ui| {
                for row in 0..2 {
                    for (k, label, suffix, w) in [(row, if row == 0 { "PosX" } else { "PosY" }, unit, 70.0),
                                                  (2 + row, if row == 0 { "width_l" } else { "height_l" }, unit, 70.0),
                                                  (5 + row, "", "%", 72.0)] {
                        let lab = match label {
                            "width_l" => l("Larghezza", "Width"),
                            "height_l" => l("Altezza", "Height"),
                            x => x.to_string(),
                        };
                        ui.label(lab);
                        let id = egui::Id::new(("ptx", k));
                        let focused = ui.ctx().memory(|m| m.has_focus(id));
                        if !focused {
                            self.tx[k] = if k >= 5 { "100.000".into() } else { format!("{:.3}", shown(k)) };
                        }
                        ui.horizontal(|ui| {
                            let r = ui.add_enabled(b.is_some(), egui::TextEdit::singleline(&mut self.tx[k]).id(id).desired_width(w));
                            ui.weak(suffix);
                            if r.lost_focus() {
                                if let Ok(v) = self.tx[k].replace(',', ".").trim().parse::<f64>() {
                                    commit = Some((k, v));
                                }
                            }
                        });
                    }
                    if row == 0 {
                        ui.end_row();
                    }
                }
                ui.end_row();
            });
            // lock aspect + reference point
            ui.vertical(|ui| {
                self.lock_button(ui);
            });
            egui::Grid::new("anchor").spacing([1.0, 1.0]).show(ui, |ui| {
                for k in 0..9 {
                    if ui.radio(self.anchor == k, "").on_hover_text(l("Punto di riferimento per posizione e dimensioni", "Reference point for position and size")).clicked() {
                        self.anchor = k;
                    }
                    if k % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("rotate"));
                    let id = egui::Id::new(("ptx", 4));
                    if !ui.ctx().memory(|m| m.has_focus(id)) {
                        self.tx[4] = "0".into();
                    }
                    let r = ui.add_enabled(b.is_some(), egui::TextEdit::singleline(&mut self.tx[4]).id(id).desired_width(50.0));
                    ui.weak("°");
                    if r.lost_focus() {
                        if let Ok(v) = self.tx[4].replace(',', ".").trim().parse::<f64>() {
                            commit = Some((4, v));
                        }
                    }
                });
            });
            // text options when one text is selected
            if sel.len() == 1 && sel[0].kind() == "text" {
                ui.separator();
                self.text_options(ui, &sel[0]);
            }
        });
        let Some((k, v)) = commit else { return };
        let Some(b) = b else { return };
        let anchor_pt = [b[0] + ax * b[2], b[1] + ay * b[3]];
        let r = match k {
            0 => self.doc.apply_transform_field("x", v / u - ax * b[2], lock),
            1 => self.doc.apply_transform_field("y", v / u - ay * b[3], lock),
            4 => self.doc.apply_transform_field("r", v, lock),
            _ => {
                let (key, size) = match k {
                    2 => ("w", v / u),
                    3 => ("h", v / u),
                    5 => ("w", b[2] * v / 100.0),
                    _ => ("h", b[3] * v / 100.0),
                };
                let r = self.doc.apply_transform_field(key, size, lock);
                // keep the reference point where it was
                if let Some(nb) = multi_bounds(&self.doc.selected_shapes()) {
                    let _ = self.doc.apply_transform_field("x", anchor_pt[0] - ax * nb[2], lock);
                    let _ = self.doc.apply_transform_field("y", anchor_pt[1] - ay * nb[3], lock);
                }
                r
            }
        };
        self.msg(r);
        self.invalidate();
    }

    /// Padlock for "lock aspect ratio": closed and highlighted when locked, open when free.
    fn lock_button(&mut self, ui: &mut egui::Ui) {
        let on = self.lock_aspect;
        let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
        let vis = ui.style().interact_selectable(&resp, on);
        if on || resp.hovered() {
            ui.painter().rect_filled(rect, 5.0, if on { ui.visuals().selection.bg_fill } else { vis.weak_bg_fill });
        }
        let col = if on { Color32::WHITE } else { ui.visuals().text_color() };
        let c = rect.center();
        let p = ui.painter();
        // body
        let body = Rect::from_center_size(c + vec2(0.0, 4.0), vec2(14.0, 10.0));
        p.rect_filled(body, 2.0, col);
        // keyhole
        let bg = if on { ui.visuals().selection.bg_fill } else if resp.hovered() { vis.weak_bg_fill } else { ui.visuals().panel_fill };
        p.circle_filled(body.center() + vec2(0.0, -1.0), 1.6, bg);
        p.line_segment([body.center() + vec2(0.0, -1.0), body.center() + vec2(0.0, 2.5)], Stroke::new(1.4, bg));
        // shackle: a half ring over the body, lifted and shifted when open
        let (lift, shift) = if on { (0.0, 0.0) } else { (3.5, 4.0) };
        let top = body.top() - lift;
        let ring_c = egui::pos2(c.x + shift, top - 1.0);
        let r = 4.5;
        let mut pts = Vec::new();
        for i in 0..=16 {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
            pts.push(ring_c + vec2(r * a.cos(), r * a.sin()));
        }
        // legs down to the body (the right one stays up when open)
        let mut path = vec![egui::pos2(ring_c.x - r, body.top())];
        path.extend(pts);
        path.push(egui::pos2(ring_c.x + r, if on { body.top() } else { top + 1.0 }));
        p.add(egui::Shape::line(path, Stroke::new(2.0, col)));
        let tip = if on { l("Proporzioni bloccate (clic per sbloccare)", "Aspect ratio locked (click to unlock)") } else { l("Proporzioni libere (clic per bloccare)", "Aspect ratio free (click to lock)") };
        if resp.on_hover_text(tip).clicked() {
            self.lock_aspect = !on;
        }
    }

    /// Font, height, bold and italic of the selected text (in the property bar).
    fn text_options(&mut self, ui: &mut egui::Ui, s: &crate::shapes::Shape) {
        let id = s.id();
        let font = s.0.get("font").and_then(|v| v.as_str()).unwrap_or("Helvetica").to_string();
        let mut set: Option<(&str, Value)> = None;
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(l("Carattere", "Font"));
                let mut f2 = font.clone();
                egui::ComboBox::from_id_salt("pbfont").selected_text(f2.split(':').last().unwrap_or("").to_string()).width(150.0).height(400.0).show_ui(ui, |ui| {
                    for (fid, lab) in crate::shx::single_line_fonts() {
                        ui.selectable_value(&mut f2, fid, format!("✎ {lab}"));
                    }
                    for f in crate::text::families() {
                        ui.selectable_value(&mut f2, f.clone(), f);
                    }
                });
                if f2 != font {
                    set = Some(("font", json!(f2)));
                }
                ui.label(l("Altezza", "Height"));
                let hid = egui::Id::new("ptx_size");
                if !ui.ctx().memory(|m| m.has_focus(hid)) {
                    self.tx[7] = format!("{:.2}", s.f("size"));
                }
                if ui.add(egui::TextEdit::singleline(&mut self.tx[7]).id(hid).desired_width(50.0)).lost_focus() {
                    if let Ok(v) = self.tx[7].replace(',', ".").trim().parse::<f64>() {
                        if v > 0.0 && (v - s.f("size")).abs() > 1e-6 {
                            set = Some(("size", json!(v)));
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                for (k, lab) in [("bold", l("Grassetto", "Bold")), ("italic", l("Corsivo", "Italic"))] {
                    let mut v = s.flag(k);
                    if ui.checkbox(&mut v, lab).changed() {
                        set = Some((k, json!(v)));
                    }
                }
            });
        });
        if let Some((k, v)) = set {
            self.doc.save_undo(&l("Modifica testo", "Edit text"));
            if let Some(sh) = self.doc.find_mut(id) {
                sh.0.insert(k.into(), v);
            }
            self.invalidate();
        }
    }

    /// While a job runs (and after it ends, until closed): progress, elapsed and remaining time, pause / stop.
    /// Job progress shown above the Pause / Stop / Start buttons: bar with %, elapsed and remaining time.
    pub fn job_progress(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(job) = self.job.as_ref() else { return };
        let (busy, acked, total, alarm) = match &self.conn {
            Some(c) => {
                let s = c.shared.lock().unwrap();
                (s.busy, s.acked, s.total, s.link == Some(Link::Alarm))
            }
            None => (false, 0, 0, false),
        };
        let mut job = job.clone();
        if !busy && job.done_at.is_none() && job.started.elapsed() > Duration::from_millis(500) {
            job.done_at = Some(Instant::now());
        }
        let elapsed = job.done_at.unwrap_or_else(Instant::now).duration_since(job.started).as_secs_f64();
        let frac = if total > 0 { (acked as f64 / total as f64).clamp(0.0, 1.0) } else { 0.0 };
        // the estimate first, then what the real pace says once enough of the job is done
        let remaining = if job.done_at.is_some() { 0.0 } else if frac > 0.05 {
            let pace = elapsed / frac - elapsed;
            let est = (job.est - elapsed).max(0.0);
            let w = ((frac - 0.05) / 0.25).clamp(0.0, 1.0);
            est * (1.0 - w) + pace * w
        } else {
            (job.est - elapsed).max(0.0)
        };
        let mut close = false;
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let title = if job.done_at.is_some() {
                    if job.stopped || alarm { l("Lavoro interrotto", "Job stopped") } else { l("Lavoro completato", "Job finished") }
                } else if job.paused { l("In pausa", "Paused") } else { l("Taglio in corso…", "Burning…") };
                ui.strong(title);
                if job.done_at.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text(tr("close")).clicked() {
                            close = true;
                        }
                    });
                }
            });
            ui.add(egui::ProgressBar::new(frac as f32).text(format!("{:.1} %  ·  {acked} / {total} {}", frac * 100.0, l("righe", "lines"))));
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} {}", l("Trascorso", "Elapsed"), fmt_time(elapsed)));
                ui.separator();
                ui.label(format!("{} {}", l("Rimanente", "Remaining"), fmt_time(remaining)));
                ui.separator();
                ui.weak(format!("{} {}", l("Stima", "Estimate"), fmt_time(job.est)));
            });
        });
        if job.done_at.is_none() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.job = if close { None } else { Some(job) };
    }

    fn toolbox(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            for b in defs::toolbar("TOOLBOX") {
                if let Some((aid, icon)) = b {
                    self.icon_button(ui, &aid, &icon);
                }
            }
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..self.doc.layers.len() {
                let lay = &self.doc.layers[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
                ui.painter().rect_filled(rect, 3.0, color(&lay.color));
                if i == self.doc.active_layer {
                    ui.painter().rect_stroke(rect.expand(1.5), 3.0, Stroke::new(2.0, ui.visuals().strong_text_color()), egui::StrokeKind::Outside);
                }
                let txt = if lay.tool { lay.name.clone() } else { format!("{i:02}") };
                let fg = if color(&lay.color).intensity() > 0.55 { Color32::BLACK } else { Color32::WHITE };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, txt, egui::FontId::proportional(10.0), fg);
                let tip = format!("{} — {}", lay.name, if lay.tool { l("livello tecnico (non va al laser)", "tool layer (not sent to the laser)") } else { l("clic: livello attivo / sposta la selezione", "click: active layer / move the selection") });
                if resp.on_hover_text(tip).clicked() {
                    self.palette_clicked(i);
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let msg = match &self.toast {
                Some((m, t)) if t.elapsed() < Duration::from_secs(5) => m.clone(),
                _ => self.status.clone(),
            };
            ui.label(msg);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (state, link) = match &self.conn {
                    Some(c) => {
                        let s = c.shared.lock().unwrap();
                        (s.status.state.clone(), s.link)
                    }
                    None => (String::new(), None),
                };
                let (dot, txt) = match link {
                    Some(Link::Alarm) => (Color32::from_rgb(220, 50, 50), "ALARM".to_string()),
                    Some(Link::Connected) => (Color32::from_rgb(40, 170, 80), if state.is_empty() { l("Connesso", "Connected") } else { state }),
                    _ => (Color32::GRAY, l("Non connesso", "Not connected")),
                };
                ui.label(txt);
                ui.label(egui::RichText::new("●").color(dot));
                if let Some(h) = self.head() {
                    ui.monospace(format!("X {:.3} Y {:.3}", h[0], h[1]));
                }
                if let Some(m) = self.canvas.last_mouse {
                    let u = self.prefs.unit_factor();
                    ui.monospace(format!("x {:.2} y {:.2}", m[0] * u, m[1] * u));
                }
                self.zoom_menu(ui);
                if ui.selectable_label(self.snap_on, l("Aggancio", "Snap")).on_hover_text(l("Aggancio alla griglia (G)", "Grid snap (G)")).clicked() {
                    self.snap_on = !self.snap_on;
                }
                let sel = self.doc.selected_shapes();
                if sel.len() == 1 {
                    ui.weak(format!("{} #{}", sel[0].kind(), sel[0].id()));
                } else if sel.len() > 1 {
                    ui.weak(format!("{} obj", sel.len()));
                }
            });
        });
    }

    /// Zoom percentage (100 % = true size on screen) with a menu of fixed zooms.
    fn zoom_menu(&mut self, ui: &mut egui::Ui) {
        let ws = (self.machine.workspace_x, self.machine.workspace_y);
        ui.menu_button(format!("{:.0}%", self.canvas.zoom_percent()), |ui| {
            if ui.button(l("Adatta alla pagina", "Zoom to page")).clicked() {
                self.canvas.zoom_page(ws);
                ui.close();
            }
            if ui.button(l("Adatta alla selezione", "Zoom to selection")).clicked() {
                self.run("zoomSel");
                ui.close();
            }
            ui.separator();
            for pct in [25.0f32, 50.0, 100.0, 200.0, 400.0, 800.0] {
                let txt = if pct == 100.0 { format!("100%  {}", l("(dimensione reale)", "(true size)")) } else { format!("{pct:.0}%") };
                if ui.button(txt).clicked() {
                    self.canvas.zoom_real(pct / 100.0, ws);
                    ui.close();
                }
            }
        }).response.on_hover_text(l("Zoom: 100% = dimensione reale sullo schermo", "Zoom: 100% = true size on screen"));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || !self.dialogs.is_empty() {
            return;
        }
        let mut all: Vec<(String, KeyboardShortcut)> = Vec::new();
        fn walk(items: &[MenuItem], out: &mut Vec<String>) {
            for it in items {
                match it {
                    MenuItem::Action { id, .. } => out.push(id.clone()),
                    MenuItem::Sub { items, .. } => walk(items, out),
                    _ => {}
                }
            }
        }
        let mut ids = Vec::new();
        for m in defs::menus() {
            walk(&m.items, &mut ids);
        }
        for id in ids {
            if let Some(k) = parse_shortcut(&self.shortcut_of(&id)) {
                all.push((id, k));
            }
        }
        // longest modifier sets first, so Ctrl+Shift+Z wins over Ctrl+Z
        all.sort_by_key(|(_, k)| std::cmp::Reverse(k.modifiers.command as u8 + k.modifiers.shift as u8 + k.modifiers.alt as u8));
        let mut fired = None;
        ctx.input_mut(|i| {
            for (id, k) in &all {
                if i.consume_shortcut(k) {
                    fired = Some(id.clone());
                    break;
                }
            }
            if fired.is_none() && i.key_pressed(Key::Backspace) && i.modifiers.is_none() {
                fired = Some("delete".into());
            }
        });
        if let Some(id) = fired {
            self.run(&id);
        }
    }

    fn handle_canvas_events(&mut self) {
        for e in std::mem::take(&mut self.canvas.events) {
            match e {
                Event::Changed => self.invalidate(),
                Event::Status(s) if s == "toggle_snap" => self.snap_on = !self.snap_on,
                Event::Status(s) => self.status = s,
                Event::Text(p) => self.dialogs.push(Dialog::new_text(p, self.doc.active_layer)),
                Event::PositionLaser(p) => {
                    self.goto_canvas(p);
                    let m = to_machine(&self.machine, p);
                    self.status = format!("G0 X{:.2} Y{:.2}", m[0], m[1]);
                }
                Event::EditShape(id) => {
                    if let Some(s) = self.doc.find(id) {
                        match s.kind() {
                            "text" => self.dialogs.push(Dialog::edit_text(s)),
                            "image" => self.dialogs.push(Dialog::adjust_image(s)),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

/// A job being burned (for the progress window).
#[derive(Clone, Debug)]
pub struct JobRun {
    pub started: Instant,
    /// Estimated duration (s).
    pub est: f64,
    pub done_at: Option<Instant>,
    pub paused: bool,
    pub stopped: bool,
}

/// Panels shown by default: the upper group (layers, move, camera, pages) and the lower one (laser, library, console).
pub fn default_panels() -> Vec<String> {
    ["winCuts", "winMove", "winCamera", "winPages", "winLaser", "winLibrary", "winConsole"].iter().map(|s| s.to_string()).collect()
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new(if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(url).spawn().map(|_| ())
}

impl eframe::App for LaserApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        ctx.set_visuals(if self.prefs.theme == "light" { egui::Visuals::light() } else { egui::Visuals::dark() });
        self.shortcuts(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.dirty && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.dialogs.iter().any(|d| matches!(d, Dialog::ConfirmDiscard { .. })) {
                self.dialogs.push(Dialog::ConfirmDiscard { then: "quit".into() });
            }
        }
        // dropped files
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for p in dropped {
            self.open_dropped(&p);
        }
        self.update_native_menu(&ctx);
        for id in crate::native_menu::take_events() {
            if let Some(p) = id.strip_prefix("recent:") {
                if !p.is_empty() {
                    self.open_path(&PathBuf::from(p));
                }
            } else {
                self.run(&id);
            }
        }
        egui::Panel::top("menu").show(ui, |ui| {
            if self.native_menu.is_none() {
                self.menu(ui);
            }
            if !self.hidden_bars.iter().any(|b| b == "winMainToolbar") {
                self.main_toolbar(ui);
            }
        });
        if !self.hidden_bars.iter().any(|b| b == "winModbar") {
            egui::Panel::top("propbar").show(ui, |ui| self.property_bar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            if !self.hidden_bars.iter().any(|b| b == "winPalette") {
                self.palette(ui);
            }
            self.status_bar(ui);
        });
        if !self.hidden_bars.iter().any(|b| b == "winToolbox") {
            egui::Panel::left("toolbox").resizable(false).show(ui, |ui| self.toolbox(ui));
        }
        if !self.panels.is_empty() {
            egui::Panel::right("side").default_size(380.0).resizable(true).show(ui, |ui| self.side_panels(ui));
        }
        let env_layers = self.doc.layers.clone();
        let head = self.head_canvas();
        let prefs = self.prefs.clone();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let env = Env { layers: &env_layers, ws: (self.machine.workspace_x, self.machine.workspace_y), head, prefs: &prefs, snap_on: self.snap_on,
                            lock_aspect: self.lock_aspect, filled: self.filled, grid: self.grid, rulers: self.rulers,
                            polygon_sides: self.polygon_sides, dark: self.prefs.theme != "light", modal: !self.dialogs.is_empty(),
                            overlay: self.cam.overlay.as_ref().filter(|_| self.cam.show_overlay).map(|t| (t.id(), self.cam.opacity())) };
            self.canvas.ui(ui, &mut self.doc, &env);
        });
        self.handle_canvas_events();
        self.show_dialogs(&ctx);
        self.backup();
        self.poll_remote();
        self.poll_camera(&ctx);
        // after connecting, once the $$ values are in: warn about settings that break laser jobs
        if !self.checked_grbl && self.connected() {
            let st = self.conn.as_ref().map(|c| c.shared.lock().unwrap().settings.clone()).unwrap_or_default();
            if st.len() > 10 {
                self.checked_grbl = true;
                let probs = laserforge_core::grbl::problems(&st, self.machine.max_power);
                if !probs.is_empty() {
                    let laser_off = st.get(&32) == Some(&0.0);
                    self.dialogs.push(Dialog::Generic(Box::new(crate::dialogs2::GrblCheck { probs, laser_off, open_settings: false })));
                }
            }
        }
        if self.remote.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if let Some(c) = self.pending_cmd.take() {
            ctx.send_viewport_cmd(c);
        }
        // continuous framing
        if self.frame_continuous && self.connected() && !self.busy() && self.last_frame.is_none_or(|t| t.elapsed() > Duration::from_millis(300)) {
            self.frame_job(None);
        }
        if self.conn.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let title = format!("LaserForge — {}{}", self.base_name(), if self.doc.dirty { " •" } else { "" });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        // LF_SCREENSHOT=path: capture the window after it settles, then quit (visual checks)
        if let Some((path, done)) = &self.shot {
            if done.load(Ordering::SeqCst) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if self.frames == 25 {
                let (path, done) = (path.clone(), done.clone());
                ctx.request_screenshot(move |img| {
                    let [w, h] = img.size;
                    let mut buf = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        buf.extend_from_slice(&p.to_array());
                    }
                    if let Some(im) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                        let _ = im.save(&path);
                    }
                    done.store(true, Ordering::SeqCst);
                });
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.no_save {
            return;
        }
        let project = self.persist_doc();
        let saved = Saved {
            machine: self.machine.clone(), prefs: self.prefs.clone(), layers: self.doc.layers.clone(), opt: self.opt.clone(),
            port: self.port.clone(), baud: self.baud, jog_step: self.jog_step, jog_speed: self.jog_speed, start_from: self.start_from.clone(),
            job_origin: self.job_origin, user_origin: self.user_origin, user_origin_set: self.user_origin_set, recent: self.recent.clone(),
            lang: if defs::english() { "en".into() } else { "it".into() }, filled: self.filled, grid: self.grid, rulers: self.rulers,
            cylinder: self.cylinder.clone(), positions: self.positions.clone(), macros: self.macros.clone(),
            shortcuts: self.shortcut_overrides.clone(), project, panels: self.panels.clone(),
            devices: self.devices.clone(), device: self.device.clone(), checklist: self.checklist.clone(),
            image_presets: self.image_presets.clone(),
            camera: self.cam.cal.clone(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &saved);
    }
}
