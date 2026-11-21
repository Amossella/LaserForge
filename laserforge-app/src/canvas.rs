//! The design canvas: view (zoom / pan), drawing of the work area, grid, rulers, shapes and images, selection
//! handles, snapping, and the mouse tools (select / move / resize / marquee, rectangle, ellipse, line, polygon,
//! star, text, measure, position, pan) plus the plug-in tools of tools.rs. Ported from laserforge/canvas.py.

use crate::edit::{center, regular_polygon, scale_shape, star_points};
use crate::prefs::Prefs;
use crate::project::Doc;
use crate::shapes::{is_closed_path, multi_bounds, Shape, P};
use egui::{pos2, vec2, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use laserforge_core::model::LayerSettings;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const RULER: f32 = 18.0;
const MAX_ZOOM: f32 = 120.0;
const BLUE: Color32 = Color32::from_rgb(0x2f, 0x7b, 0xf5);

/// Everything the canvas reads from the window each frame.
pub struct Env<'a> {
    pub layers: &'a [LayerSettings],
    pub ws: (f64, f64),
    /// Laser head on the canvas (when connected).
    pub head: Option<P>,
    pub prefs: &'a Prefs,
    pub snap_on: bool,
    pub lock_aspect: bool,
    pub filled: bool,
    pub grid: bool,
    pub rulers: bool,
    pub polygon_sides: usize,
    pub dark: bool,
    /// A dialog is open: the canvas ignores keys.
    pub modal: bool,
    /// Camera image of the work area and its opacity.
    pub overlay: Option<(egui::TextureId, f32)>,
}

/// Requests the canvas makes to the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Text(P),
    PositionLaser(P),
    EditShape(i64),
    Status(String),
    Changed,
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Pan { from: Pos2, pan: Vec2 },
    Move { start: P, moved: bool },
    Resize { handle: &'static str },
    Marquee { a: P, b: P },
    Draw { a: P, b: P },
    Measure,
    Handler,
}

pub fn handle_positions(b: [f64; 4]) -> [(&'static str, f64, f64); 8] {
    let [x, y, w, h] = b;
    [("nw", x, y), ("n", x + w / 2.0, y), ("ne", x + w, y), ("e", x + w, y + h / 2.0),
     ("se", x + w, y + h), ("s", x + w / 2.0, y + h), ("sw", x, y + h), ("w", x, y + h / 2.0)]
}

pub fn color(hex: &str) -> Color32 {
    Color32::from_hex(hex).unwrap_or(Color32::BLACK)
}

fn in_bounds(p: P, b: [f64; 4], pad: f64) -> bool {
    p[0] >= b[0] - pad && p[0] <= b[0] + b[2] + pad && p[1] >= b[1] - pad && p[1] <= b[1] + b[3] + pad
}

/// A shape's geometry key: everything but the bitmap payload (summarised) and the fields that don't change paths.
fn geo_key(s: &Shape) -> String {
    let mut out = String::new();
    for (k, v) in &s.0 {
        if matches!(k.as_str(), "id" | "name" | "locked" | "layer" | "priority") {
            continue;
        }
        out.push_str(k);
        out.push('=');
        match v {
            Value::String(src) if k == "src" => {
                let n = src.len();
                out.push_str(&format!("{n}:{}", &src[n.saturating_sub(48)..]));
            }
            _ => out.push_str(&v.to_string()),
        }
        out.push(';');
    }
    out
}

/// Plug-in mouse tools (node editor, scissors, tabs, start point, ...), see tools.rs.
pub trait ToolHandler {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P, shift: bool);
    fn drag(&mut self, _c: &mut Canvas, _doc: &mut Doc, _env: &Env, _pt: P) {}
    fn hover(&mut self, _c: &mut Canvas, _doc: &mut Doc, _env: &Env, _pt: P) {}
    fn release(&mut self, _c: &mut Canvas, _doc: &mut Doc, _env: &Env) {}
    fn draw(&self, _c: &Canvas, _doc: &Doc, _env: &Env, _p: &egui::Painter) {}
    /// A typed character; true when used.
    fn key(&mut self, _c: &mut Canvas, _doc: &mut Doc, _ch: char) -> bool {
        false
    }
    fn reset(&mut self) {}
    /// Nodes selected with the node editor: (shape id, node index).
    fn selected_nodes(&self) -> Vec<(i64, usize)> {
        Vec::new()
    }
}

pub struct Canvas {
    pub zoom: f32,
    pub pan: Vec2,
    pub rect: Rect,
    pub tool: String,
    mode: Option<Mode>,
    orig: Vec<Shape>,
    start_b: [f64; 4],
    pub points: Vec<P>,
    pub last: Option<P>,
    measure: Option<(P, P)>,
    pub last_mouse: Option<P>,
    pub snap_hit: Option<P>,
    space: bool,
    pub events: Vec<Event>,
    paths: HashMap<i64, (String, Vec<Vec<P>>)>,
    textures: HashMap<String, egui::TextureHandle>,
    pub fit_pending: u32,
    pub handlers: HashMap<String, Box<dyn ToolHandler>>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas { zoom: 1.0, pan: vec2(40.0, 40.0), rect: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), tool: "select".into(),
                 mode: None, orig: Vec::new(), start_b: [0.0; 4], points: Vec::new(), last: None, measure: None, last_mouse: None,
                 snap_hit: None, space: false, events: Vec::new(), paths: HashMap::new(), textures: HashMap::new(), fit_pending: 4,
                 handlers: HashMap::new() }
    }
}

impl Canvas {
    // ------------------------------------------------------------ coordinates
    pub fn to_world(&self, s: Pos2) -> P {
        let v = s - self.rect.min - self.pan;
        [(v.x / self.zoom) as f64, (v.y / self.zoom) as f64]
    }

    pub fn to_screen(&self, p: P) -> Pos2 {
        self.rect.min + self.pan + vec2(p[0] as f32 * self.zoom, p[1] as f32 * self.zoom)
    }

    pub fn fit_zoom(&self, ws: (f64, f64)) -> f32 {
        let pad = 30.0;
        if self.rect.width() < 10.0 || ws.0 <= 0.0 || ws.1 <= 0.0 {
            return 1.0;
        }
        ((self.rect.width() - pad * 2.0) / ws.0 as f32).min((self.rect.height() - pad * 2.0) / ws.1 as f32).max(1e-3)
    }

    pub fn zoom_to_box(&mut self, b: [f64; 4]) {
        let pad = 30.0;
        let z = ((self.rect.width() - 2.0 * pad) / b[2].max(1e-3) as f32).min((self.rect.height() - 2.0 * pad) / b[3].max(1e-3) as f32);
        self.zoom = z.clamp(1e-3, MAX_ZOOM);
        let c = center(b);
        self.pan = self.rect.size() / 2.0 - vec2(c[0] as f32, c[1] as f32) * self.zoom;
    }

    pub fn zoom_page(&mut self, ws: (f64, f64)) {
        self.zoom_to_box([0.0, 0.0, ws.0, ws.1]);
    }

    pub fn zoom_at(&mut self, at: Pos2, factor: f32, ws: (f64, f64)) {
        let w = self.to_world(at);
        self.zoom = (self.zoom * factor).clamp(self.fit_zoom(ws) * 0.2, MAX_ZOOM);
        self.pan = at - self.rect.min - vec2(w[0] as f32, w[1] as f32) * self.zoom;
    }

    /// Screen points for one real millimetre, from the main display's physical size (macOS), else 96 dpi:
    /// 100 % shows the design at its true size.
    pub fn real_px_per_mm() -> f32 {
        static V: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        *V.get_or_init(|| screen_points_per_mm().filter(|v| (1.0..20.0).contains(v)).unwrap_or(96.0 / 25.4))
    }

    pub fn zoom_percent(&self) -> f32 {
        self.zoom / Self::real_px_per_mm() * 100.0
    }

    /// Zoom to a fraction of the true size (1.0 = 100 %) around the view centre.
    pub fn zoom_real(&mut self, factor: f32, ws: (f64, f64)) {
        let f = Self::real_px_per_mm() * factor / self.zoom;
        self.zoom_step(f, ws);
    }

    pub fn zoom_step(&mut self, factor: f32, ws: (f64, f64)) {
        let c = self.rect.center();
        self.zoom_at(c, factor, ws);
    }

    pub fn set_tool(&mut self, name: &str) {
        self.tool = name.to_string();
        self.points.clear();
        self.last = None;
        self.measure = None;
        self.mode = None;
        if let Some(h) = self.handlers.get_mut(name) {
            h.reset();
        }
    }

    /// Esc: drop what's being drawn; true when there was something.
    pub fn cancel_in_progress(&mut self) -> bool {
        if !self.points.is_empty() || self.measure.is_some() || self.mode.is_some() {
            self.points.clear();
            self.last = None;
            self.measure = None;
            self.mode = None;
            return true;
        }
        false
    }

    // ------------------------------------------------------------ snapping and hit tests
    pub fn snap(&mut self, doc: &Doc, env: &Env, p: P, exclude: Option<&[i64]>) -> P {
        self.snap_hit = None;
        if env.prefs.obj_snap {
            if let Some(q) = self.object_snap(doc, env, p, exclude) {
                self.snap_hit = Some(q);
                return q;
            }
        }
        if !env.snap_on {
            return p;
        }
        let g = env.prefs.snap_step.max(1e-6);
        [(p[0] / g).round() * g, (p[1] / g).round() * g]
    }

    fn object_snap(&self, doc: &Doc, env: &Env, p: P, exclude: Option<&[i64]>) -> Option<P> {
        let tol = env.prefs.obj_snap_px / self.zoom as f64;
        let moving = matches!(self.mode, Some(Mode::Move { .. }) | Some(Mode::Resize { .. }));
        let excl: Vec<i64> = exclude.map(|e| e.to_vec()).unwrap_or_else(|| if moving { doc.selected.clone() } else { Vec::new() });
        let (mut best, mut best_d) = (None, tol);
        for s in &doc.shapes {
            if excl.contains(&s.id()) {
                continue;
            }
            let b = s.bounds();
            if !in_bounds(p, b, tol) {
                continue;
            }
            let mut c = vec![center(b)];
            match s.kind() {
                "poly" => {
                    let n = s.nodes();
                    if n.len() <= 400 {
                        c.extend(n.iter().map(|q| q.p));
                        let k = n.len();
                        let segs = if s.flag("closed") { k } else { k.saturating_sub(1) };
                        for i in 0..segs {
                            let (a, d) = (n[i], n[(i + 1) % k]);
                            if a.c2.is_none() && d.c1.is_none() {
                                c.push([(a.p[0] + d.p[0]) / 2.0, (a.p[1] + d.p[1]) / 2.0]);
                            }
                        }
                    }
                }
                "rect" | "image" => {
                    let [x, y, w, h] = b;
                    c.extend([[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x + w / 2.0, y], [x + w, y + h / 2.0], [x + w / 2.0, y + h], [x, y + h / 2.0]]);
                }
                "circle" => {
                    let (cx, cy, rx, ry) = (s.f("cx"), s.f("cy"), s.f("rx"), s.f("ry"));
                    c.extend([[cx + rx, cy], [cx - rx, cy], [cx, cy + ry], [cx, cy - ry]]);
                }
                _ => {}
            }
            for q in c {
                let d = (q[0] - p[0]).hypot(q[1] - p[1]);
                if d < best_d {
                    best = Some(q);
                    best_d = d;
                }
            }
        }
        best
    }

    pub fn hit_test(&self, doc: &Doc, env: &Env, pt: P) -> Option<i64> {
        let tol = env.prefs.click_tol / self.zoom as f64;
        for s in doc.shapes.iter().rev() {
            if s.locked() {
                continue;
            }
            if matches!(s.kind(), "image" | "text") {
                if in_bounds(pt, s.bounds(), tol) {
                    return Some(s.id());
                }
                continue;
            }
            if s.distance(pt[0], pt[1]) < tol {
                return Some(s.id());
            }
        }
        let mut best = None;
        let mut best_area = f64::INFINITY;
        for s in &doc.shapes {
            if s.locked() {
                continue;
            }
            let b = s.bounds();
            if in_bounds(pt, b, 0.0) && b[2] * b[3] < best_area {
                best = Some(s.id());
                best_area = b[2] * b[3];
            }
        }
        best
    }

    fn hit_handle(&self, doc: &Doc, env: &Env, pt: P) -> Option<&'static str> {
        let sel = doc.selected_shapes();
        if sel.is_empty() || self.tool != "select" {
            return None;
        }
        let tol = (env.prefs.click_tol + 1.0) / self.zoom as f64;
        handle_positions(multi_bounds(&sel)?).into_iter().find(|(_, hx, hy)| (hx - pt[0]).abs() <= tol && (hy - pt[1]).abs() <= tol).map(|h| h.0)
    }

    /// Box of the smallest shape containing the selection (for snapping to its centre).
    fn center_container(&self, doc: &Doc, sb: [f64; 4]) -> Option<[f64; 4]> {
        let (mut best, mut best_area) = (None, f64::INFINITY);
        let eps = 0.01;
        for s in &doc.shapes {
            if doc.selected.contains(&s.id()) {
                continue;
            }
            let b = s.bounds();
            if b[0] <= sb[0] + eps && b[1] <= sb[1] + eps && b[0] + b[2] >= sb[0] + sb[2] - eps && b[1] + b[3] >= sb[1] + sb[3] - eps {
                let area = b[2] * b[3];
                if sb[2] * sb[3] + eps < area && area < best_area {
                    best = Some(b);
                    best_area = area;
                }
            }
        }
        best
    }

    fn restore_orig(&self, doc: &mut Doc) {
        for o in &self.orig {
            if let Some(s) = doc.shapes.iter_mut().find(|s| s.id() == o.id()) {
                *s = o.clone();
            }
        }
    }

    fn apply_from_orig(&self, doc: &mut Doc, f: impl Fn(&mut Shape)) {
        self.restore_orig(doc);
        let sel = doc.selected.clone();
        for s in doc.shapes.iter_mut().filter(|s| sel.contains(&s.id())) {
            f(s);
        }
    }

    fn constrain_square(a: P, b: P) -> P {
        let d = (b[0] - a[0]).abs().max((b[1] - a[1]).abs());
        let sx = if b[0] - a[0] < 0.0 { -1.0 } else { 1.0 };
        let sy = if b[1] - a[1] < 0.0 { -1.0 } else { 1.0 };
        [a[0] + sx * d, a[1] + sy * d]
    }

    fn resize_from_handle(&mut self, doc: &mut Doc, env: &Env, hd: &str, pt: P, free: bool) {
        let [x, y, w, h] = self.start_b;
        let ax = if hd.contains('w') { x + w } else if hd.contains('e') { x } else { x + w / 2.0 };
        let ay = if hd.contains('n') { y + h } else if hd.contains('s') { y } else { y + h / 2.0 };
        let nz = |v: f64| if v == 0.0 { 1e-9 } else { v };
        let mut sx = if hd.contains('e') || hd.contains('w') { (pt[0] - ax) / nz((if hd.contains('w') { x } else { x + w }) - ax) } else { 1.0 };
        let mut sy = if hd.contains('n') || hd.contains('s') { (pt[1] - ay) / nz((if hd.contains('n') { y } else { y + h }) - ay) } else { 1.0 };
        if hd.len() == 2 && env.lock_aspect != free {
            let k = sx.abs().max(sy.abs());
            sx = k.copysign(if sx == 0.0 { 1.0 } else { sx });
            sy = k.copysign(if sy == 0.0 { 1.0 } else { sy });
        }
        let (sx, sy) = (sx.abs().max(0.001), sy.abs().max(0.001));
        self.apply_from_orig(doc, |s| scale_shape(s, ax, ay, sx, sy));
    }

    fn finish_draw(&mut self, doc: &mut Doc, env: &Env, a: P, b: P) {
        let mut props = Map::new();
        let kind = match self.tool.as_str() {
            "rect" => {
                let (w, h) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
                if w < 0.5 || h < 0.5 {
                    return;
                }
                for (k, v) in [("x", a[0].min(b[0])), ("y", a[1].min(b[1])), ("w", w), ("h", h)] {
                    props.insert(k.into(), json!(v));
                }
                "rect"
            }
            "ellipse" => {
                let (rx, ry) = ((b[0] - a[0]).abs() / 2.0, (b[1] - a[1]).abs() / 2.0);
                if rx < 0.25 || ry < 0.25 {
                    return;
                }
                for (k, v) in [("cx", (a[0] + b[0]) / 2.0), ("cy", (a[1] + b[1]) / 2.0), ("rx", rx), ("ry", ry)] {
                    props.insert(k.into(), json!(v));
                }
                "circle"
            }
            "polygon" | "star" => {
                let r = (b[0] - a[0]).hypot(b[1] - a[1]);
                if r < 0.5 {
                    return;
                }
                let star = self.tool == "star";
                let n = if star { 5 } else { env.polygon_sides };
                let pts = if star { star_points(a[0], a[1], r, r * 0.45, 5) } else { regular_polygon(a[0], a[1], r, n, -std::f64::consts::PI / 2.0) };
                let mut regular = json!({"cx": a[0], "cy": a[1], "r": r, "n": n, "rot": -std::f64::consts::PI / 2.0});
                if star {
                    regular["inner"] = json!(0.45);
                }
                props.insert("points".into(), json!(pts));
                props.insert("closed".into(), json!(true));
                props.insert("regular".into(), regular);
                "poly"
            }
            _ => return,
        };
        let s = doc.create(kind, props);
        doc.add_shapes(vec![s], true);
        self.events.push(Event::Changed);
    }

    pub fn finish_poly(&mut self, doc: &mut Doc, closed: bool) {
        let pts = std::mem::take(&mut self.points);
        self.last = None;
        if pts.len() >= 2 {
            let mut props = Map::new();
            props.insert("points".into(), json!(pts.iter().map(|p| json!({"x": p[0], "y": p[1]})).collect::<Vec<_>>()));
            props.insert("closed".into(), json!(closed));
            let s = doc.create("poly", props);
            doc.add_shapes(vec![s], true);
            self.events.push(Event::Changed);
        }
    }

    // ------------------------------------------------------------ frame
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &mut Doc, env: &Env) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let full = resp.rect;
        self.rect = if env.rulers { Rect::from_min_max(full.min + vec2(RULER, RULER), full.max) } else { full };
        if self.fit_pending > 0 {
            self.fit_pending -= 1;
            self.zoom_page(env.ws);
        }
        self.input(ui, &resp, doc, env);
        let painter = painter.with_clip_rect(self.rect);
        self.paint(ui, &painter, doc, env);
        if env.rulers {
            self.draw_rulers(&ui.painter().with_clip_rect(full), full, env);
        }
    }

    fn input(&mut self, ui: &egui::Ui, resp: &egui::Response, doc: &mut Doc, env: &Env) {
        let (hover, scroll, zoom_d, mods, pressed_l, pressed_m, released, down_l, down_m) = ui.input(|i| (
            i.pointer.hover_pos(), i.smooth_scroll_delta, i.zoom_delta(), i.modifiers,
            i.pointer.button_pressed(PointerButton::Primary), i.pointer.button_pressed(PointerButton::Middle),
            i.pointer.any_released(), i.pointer.button_down(PointerButton::Primary), i.pointer.button_down(PointerButton::Middle)));
        // only when the canvas itself is under the pointer (not a menu, popup or dialog drawn over it)
        let inside = hover.is_some_and(|p| self.rect.contains(p)) && resp.hovered();
        let focused_text = ui.ctx().memory(|m| m.focused().is_some()) || env.modal;
        if !focused_text {
            self.space = ui.input(|i| i.key_down(egui::Key::Space));
        }
        // zoom and pan with the wheel / trackpad
        if inside {
            if zoom_d != 1.0 {
                self.zoom_at(hover.unwrap(), zoom_d, env.ws);
            } else if scroll != Vec2::ZERO {
                let pan_mode = env.prefs.trackpad_scroll == "pan" || mods.shift;
                if mods.command || !pan_mode {
                    self.zoom_at(hover.unwrap(), (scroll.y * 0.006).exp(), env.ws);
                } else {
                    self.pan += scroll;
                }
            }
        }
        let Some(pos) = hover else { return };
        let pt = self.to_world(pos);
        if inside {
            let sp = self.snap(doc, env, pt, None);
            self.last_mouse = Some(sp);
        }
        // press
        if inside && (pressed_l || pressed_m) {
            if pressed_m || self.space || (self.tool == "pan" && pressed_l) {
                self.mode = Some(Mode::Pan { from: pos, pan: self.pan });
            } else {
                self.press(doc, env, pt, mods.shift);
            }
        }
        // drag
        if down_l || down_m {
            self.drag(doc, env, pos, pt, mods.shift, mods.alt);
        } else if inside && self.mode.is_none() {
            let sp = self.snap(doc, env, pt, None);
            if self.tool == "poly" && !self.points.is_empty() {
                self.last = Some(sp);
            }
            if let Some(mut h) = self.handlers.remove(&self.tool) {
                h.hover(self, doc, env, pt);
                self.handlers.insert(self.tool.clone(), h);
            }
            if self.tool == "select" {
                if let Some(h) = self.hit_handle(doc, env, pt) {
                    ui.ctx().set_cursor_icon(match h {
                        "nw" | "se" => egui::CursorIcon::ResizeNwSe,
                        "ne" | "sw" => egui::CursorIcon::ResizeNeSw,
                        "n" | "s" => egui::CursorIcon::ResizeVertical,
                        _ => egui::CursorIcon::ResizeHorizontal,
                    });
                }
            } else if self.tool != "pan" {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
        if self.space || self.tool == "pan" || matches!(self.mode, Some(Mode::Pan { .. })) {
            ui.ctx().set_cursor_icon(if matches!(self.mode, Some(Mode::Pan { .. })) { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        if released && self.mode.is_some() {
            self.release(doc, env);
        }
        if resp.double_clicked() && inside {
            if self.tool == "poly" && self.points.len() > 1 {
                // the double click's second press added a point at the same place
                if self.points.len() > 2 {
                    self.points.pop();
                }
                self.finish_poly(doc, false);
            } else if self.tool == "select" {
                if let Some(s) = doc.selected_shapes().first() {
                    self.events.push(Event::EditShape(s.id()));
                }
            }
        }
        // keys handled by the canvas itself
        if !focused_text {
            let (enter, arrows, g, typed) = ui.input(|i| (
                i.key_pressed(egui::Key::Enter),
                [i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)],
                i.key_pressed(egui::Key::G) && i.modifiers.is_none(),
                i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect::<String>()));
            if enter && self.tool == "poly" && !self.points.is_empty() {
                self.finish_poly(doc, false);
            }
            if arrows.iter().any(|a| *a) && !doc.selected.is_empty() && !mods.command {
                let p = env.prefs;
                let d = if mods.shift { p.nudge_shift } else if mods.alt { p.nudge_fine } else { p.nudge };
                let dx = if arrows[0] { -d } else if arrows[1] { d } else { 0.0 };
                let dy = if arrows[2] { -d } else if arrows[3] { d } else { 0.0 };
                let _ = doc.edit_selected(&crate::l("Sposta", "Move"), |s| s.translate(dx, dy));
                self.events.push(Event::Changed);
            }
            let mut used = false;
            if !mods.command {
                if let Some(mut h) = self.handlers.remove(&self.tool) {
                    for ch in typed.chars() {
                        used |= h.key(self, doc, ch);
                    }
                    self.handlers.insert(self.tool.clone(), h);
                }
            }
            if g && !used {
                self.events.push(Event::Status("toggle_snap".into()));
            }
        }
    }

    fn press(&mut self, doc: &mut Doc, env: &Env, pt: P, shift: bool) {
        let sp = self.snap(doc, env, pt, None);
        if self.tool == "select" {
            if let Some(h) = self.hit_handle(doc, env, pt) {
                self.mode = Some(Mode::Resize { handle: h });
                self.orig = doc.selected_shapes();
                self.start_b = multi_bounds(&self.orig).unwrap_or([0.0; 4]);
                return;
            }
            if let Some(hit) = self.hit_test(doc, env, pt) {
                if shift {
                    let ids: Vec<i64> = if doc.is_selected(hit) { doc.selected.iter().copied().filter(|i| *i != hit).collect() } else { doc.selected.iter().copied().chain([hit]).collect() };
                    doc.set_selection(ids);
                } else if !doc.is_selected(hit) {
                    doc.set_selection(vec![hit]);
                }
                self.mode = Some(Mode::Move { start: pt, moved: false });
                self.orig = doc.selected_shapes();
                self.start_b = multi_bounds(&self.orig).unwrap_or([0.0; 4]);
            } else {
                if !shift {
                    doc.set_selection(Vec::new());
                }
                self.mode = Some(Mode::Marquee { a: pt, b: pt });
            }
            return;
        }
        if let Some(mut h) = self.handlers.remove(&self.tool) {
            self.mode = Some(Mode::Handler);
            h.press(self, doc, env, pt, shift);
            self.handlers.insert(self.tool.clone(), h);
            return;
        }
        match self.tool.as_str() {
            "poly" => {
                if self.points.is_empty() {
                    self.points = vec![sp];
                } else {
                    let f = self.points[0];
                    if self.points.len() > 2 && (sp[0] - f[0]).hypot(sp[1] - f[1]) < 6.0 / self.zoom as f64 {
                        self.finish_poly(doc, true);
                        return;
                    }
                    self.points.push(sp);
                }
            }
            "text" => self.events.push(Event::Text(sp)),
            "measure" => {
                self.mode = Some(Mode::Measure);
                self.measure = Some((sp, sp));
            }
            "position" => self.events.push(Event::PositionLaser(sp)),
            "rect" | "ellipse" | "polygon" | "star" => self.mode = Some(Mode::Draw { a: sp, b: sp }),
            _ => {}
        }
    }

    fn drag(&mut self, doc: &mut Doc, env: &Env, pos: Pos2, pt: P, shift: bool, alt: bool) {
        let Some(mode) = self.mode.clone() else { return };
        match mode {
            Mode::Pan { from, pan } => self.pan = pan + (pos - from),
            Mode::Move { start, moved } => {
                let (mut dx, mut dy) = (pt[0] - start[0], pt[1] - start[1]);
                if !moved && (dx.hypot(dy) * self.zoom as f64) < 3.0 {
                    return;
                }
                self.mode = Some(Mode::Move { start, moved: true });
                if env.snap_on && !alt {
                    let (nx, ny) = (self.start_b[0] + dx, self.start_b[1] + dy);
                    let sel = doc.selected.clone();
                    let s = self.snap(doc, env, [nx, ny], Some(&sel));
                    dx += s[0] - nx;
                    dy += s[1] - ny;
                }
                self.apply_from_orig(doc, |s| s.translate(dx, dy));
                let sel = doc.selected_shapes();
                if let Some(b) = multi_bounds(&sel) {
                    if let (Some(bx), false) = (self.center_container(doc, b), alt) {
                        let (c, s) = (center(bx), center(b));
                        let tol = 8.0 / self.zoom as f64;
                        if (c[0] - s[0]).abs() < tol && (c[1] - s[1]).abs() < tol {
                            let ids = doc.selected.clone();
                            for sh in doc.shapes.iter_mut().filter(|x| ids.contains(&x.id())) {
                                sh.translate(c[0] - s[0], c[1] - s[1]);
                            }
                            self.events.push(Event::Status(crate::tr("centered")));
                        }
                    }
                }
            }
            Mode::Resize { handle } => self.resize_from_handle(doc, env, handle, pt, shift),
            Mode::Marquee { a, .. } => self.mode = Some(Mode::Marquee { a, b: pt }),
            Mode::Measure => {
                let sp = self.snap(doc, env, pt, None);
                if let Some(m) = &mut self.measure {
                    m.1 = sp;
                }
            }
            Mode::Draw { a, .. } => {
                let sp = self.snap(doc, env, pt, None);
                self.mode = Some(Mode::Draw { a, b: if shift { Self::constrain_square(a, sp) } else { sp } });
            }
            Mode::Handler => {
                if let Some(mut h) = self.handlers.remove(&self.tool) {
                    h.drag(self, doc, env, pt);
                    self.handlers.insert(self.tool.clone(), h);
                }
            }
        }
    }

    fn release(&mut self, doc: &mut Doc, env: &Env) {
        let Some(mode) = self.mode.take() else { return };
        match mode {
            Mode::Pan { .. } => {}
            Mode::Move { moved: true, .. } | Mode::Resize { .. } => {
                let after = doc.shapes.clone();
                self.restore_orig(doc);
                doc.save_undo(&if matches!(mode, Mode::Resize { .. }) { crate::l("Ridimensiona", "Resize") } else { crate::l("Sposta", "Move") });
                doc.shapes = after;
                self.orig.clear();
                self.events.push(Event::Changed);
            }
            Mode::Move { .. } => self.orig.clear(),
            Mode::Marquee { a, b } => {
                let bx = [a[0].min(b[0]), a[1].min(b[1]), (b[0] - a[0]).abs(), (b[1] - a[1]).abs()];
                let mut ids = doc.selected.clone();
                if bx[2] * self.zoom as f64 > 2.0 || bx[3] * self.zoom as f64 > 2.0 {
                    let contain = b[0] >= a[0];
                    for s in &doc.shapes {
                        if s.locked() || ids.contains(&s.id()) {
                            continue;
                        }
                        let sb = s.bounds();
                        let ok = if contain {
                            sb[0] >= bx[0] && sb[1] >= bx[1] && sb[0] + sb[2] <= bx[0] + bx[2] && sb[1] + sb[3] <= bx[1] + bx[3]
                        } else {
                            sb[0] <= bx[0] + bx[2] && sb[0] + sb[2] >= bx[0] && sb[1] <= bx[1] + bx[3] && sb[1] + sb[3] >= bx[1]
                        };
                        if ok {
                            ids.push(s.id());
                        }
                    }
                }
                doc.set_selection(ids);
            }
            Mode::Measure => {
                if let Some((a, b)) = self.measure {
                    self.events.push(Event::Status(format!("{:.2} mm", (b[0] - a[0]).hypot(b[1] - a[1]))));
                }
            }
            Mode::Draw { a, b } => self.finish_draw(doc, env, a, b),
            Mode::Handler => {
                if let Some(mut h) = self.handlers.remove(&self.tool) {
                    h.release(self, doc, env);
                    self.handlers.insert(self.tool.clone(), h);
                }
            }
        }
    }

    // ------------------------------------------------------------ painting
    pub fn stroke(&self, c: Color32, w: f32) -> Stroke {
        Stroke::new(w, c)
    }

    /// Draw a polyline in world coordinates (dashed when `dash`).
    pub fn polyline(&self, p: &egui::Painter, pts: &[P], stroke: Stroke, dash: bool) {
        if pts.len() < 2 {
            return;
        }
        let sp: Vec<Pos2> = pts.iter().map(|q| self.to_screen(*q)).collect();
        if dash {
            p.extend(egui::Shape::dashed_line(&sp, stroke, 5.0, 3.0));
        } else {
            p.add(egui::Shape::line(sp, stroke));
        }
    }

    /// Even-odd fill of closed paths, as horizontal spans one screen pixel apart.
    fn fill_paths(&self, p: &egui::Painter, paths: &[Vec<P>], col: Color32) {
        let closed: Vec<Vec<Pos2>> = paths.iter().filter(|q| is_closed_path(q)).map(|q| q.iter().map(|v| self.to_screen(*v)).collect()).collect();
        if closed.is_empty() {
            return;
        }
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for r in &closed {
            for q in r {
                y0 = y0.min(q.y);
                y1 = y1.max(q.y);
            }
        }
        let clip = p.clip_rect();
        let (y0, y1) = (y0.max(clip.min.y), y1.min(clip.max.y));
        let mut y = y0.floor() + 0.5;
        let mut mesh = egui::Mesh::default();
        while y < y1 {
            let mut xs: Vec<f32> = Vec::new();
            for r in &closed {
                for w in r.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if (a.y <= y) != (b.y <= y) {
                        xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                    }
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for s in xs.chunks(2) {
                if s.len() == 2 && s[1] > s[0] {
                    mesh.add_colored_rect(Rect::from_min_max(pos2(s[0], y - 0.5), pos2(s[1], y + 0.5)), col);
                }
            }
            y += 1.0;
        }
        p.add(mesh);
    }

    fn shape_paths(&mut self, s: &Shape) -> Vec<Vec<P>> {
        let key = geo_key(s);
        if let Some((k, v)) = self.paths.get(&s.id()) {
            if *k == key {
                return v.clone();
            }
        }
        let v = s.paths();
        if self.paths.len() > 5000 {
            self.paths.clear();
        }
        self.paths.insert(s.id(), (key, v.clone()));
        v
    }

    fn image_texture(&mut self, ctx: &egui::Context, s: &Shape, env: &Env) -> Option<egui::TextureId> {
        let layer = s.layer().max(0) as usize;
        let interval = env.layers.get(layer).map_or(0.1, |l| l.line_interval).max(0.05).max(s.f("w") / 900.0);
        let key = format!("{}|{interval}|{}|{}", geo_key(s), s.f("w"), s.f("h"));
        if let Some(t) = self.textures.get(&key) {
            return Some(t.id());
        }
        let pic = laserforge_core::imaging::decode(s.0.get("src")?.as_str()?)?;
        let adj: laserforge_core::imaging::Adj = s.0.get("adj").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default();
        let pass = env.layers.get(layer).is_some_and(|l| l.pass_through);
        let (mut burn, cols, rows) = laserforge_core::imaging::process(&pic, s.f("w"), s.f("h"), interval, 900, pass, s.flag("flipX"), s.flag("flipY"), &adj);
        if let Some(rings) = s.0.get("crop").and_then(|c| serde_json::from_value::<Vec<Vec<P>>>(c.clone()).ok()) {
            for (b, m) in burn.iter_mut().zip(crate::edit::mask(&rings, cols, rows)) {
                *b *= m;
            }
        }
        let grey: Vec<u8> = burn.iter().map(|b| 255 - b).collect();
        let img = egui::ColorImage::from_gray([cols, rows], &grey);
        if self.textures.len() > 40 {
            self.textures.clear();
        }
        let t = ctx.load_texture(format!("img{}", s.id()), img, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.textures.insert(key, t);
        Some(id)
    }

    fn draw_shape(&mut self, ctx: &egui::Context, p: &egui::Painter, s: &Shape, layer_idx: i64, selected: bool, env: &Env) {
        let li = s.layer_opt().filter(|_| s.0.get("layer").is_some_and(|v| !v.is_null())).unwrap_or(layer_idx);
        let lay = env.layers.get(li.max(0) as usize);
        let col = lay.map_or(Color32::BLACK, |l| color(&l.color));
        let is_tool = lay.is_some_and(|l| l.tool);
        let (ws_w, ws_h) = env.ws;
        let outside = |b: [f64; 4]| b[0] > ws_w + 1e-6 || b[1] > ws_h + 1e-6 || b[0] + b[2] < -1e-6 || b[1] + b[3] < -1e-6;
        let fade = |c: Color32, b: [f64; 4]| if outside(b) { c.gamma_multiply(0.35) } else { c };
        let stroke = Stroke::new(if selected { 1.6 } else { 1.0 }, col);
        match s.kind() {
            "group" => {
                for c in s.children() {
                    self.draw_shape(ctx, p, &c, li, selected, env);
                }
            }
            "image" => {
                let b = s.bounds();
                let r = Rect::from_min_max(self.to_screen([b[0], b[1]]), self.to_screen([b[0] + b[2], b[1] + b[3]]));
                match self.image_texture(ctx, s, env) {
                    Some(t) => {
                        p.image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), fade(Color32::WHITE, b));
                    }
                    None => {
                        p.rect_filled(r, 0.0, Color32::from_gray(0xdd));
                    }
                }
                let rect_pts = [[b[0], b[1]], [b[0] + b[2], b[1]], [b[0] + b[2], b[1] + b[3]], [b[0], b[1] + b[3]], [b[0], b[1]]];
                self.polyline(p, &rect_pts, Stroke::new(stroke.width, fade(col, b)), selected || is_tool);
                if let Some(rings) = s.0.get("crop").and_then(|c| serde_json::from_value::<Vec<Vec<P>>>(c.clone()).ok()) {
                    for r in rings {
                        let mut pts: Vec<P> = r.iter().map(|q| [b[0] + q[0] * b[2], b[1] + q[1] * b[3]]).collect();
                        if let Some(f) = pts.first().copied() {
                            pts.push(f);
                        }
                        self.polyline(p, &pts, Stroke::new(1.2, Color32::from_rgb(0x8e, 0x44, 0xad)), true);
                    }
                }
            }
            _ => {
                let paths = self.shape_paths(s);
                let all: Vec<P> = paths.iter().flatten().copied().collect();
                let b = crate::shapes::path_box(&all);
                let single = s.kind() == "text" && crate::text::is_single_line(s);
                if (s.kind() == "text" && !single) || (env.filled && !single) {
                    let a = if s.kind() == "text" && !env.filled { 0.85 } else { 0.35 };
                    self.fill_paths(p, &paths, fade(col, b).gamma_multiply(a));
                }
                let st = Stroke::new(stroke.width, fade(col, b));
                for path in &paths {
                    self.polyline(p, path, st, selected || is_tool);
                }
            }
        }
        if s.locked() {
            let b = s.bounds();
            let c = self.to_screen([b[0] + b[2], b[1]]);
            p.rect_filled(Rect::from_center_size(c, vec2(8.0, 8.0)), 1.0, Color32::from_rgba_unmultiplied(243, 156, 18, 230));
        }
    }

    fn draw_grid(&self, p: &egui::Painter, env: &Env) {
        let (wsx, wsy) = env.ws;
        let g = env.prefs.grid_spacing;
        let a = self.to_world(self.rect.min);
        let b = self.to_world(self.rect.max);
        let (x_lo, x_hi, y_lo, y_hi) = (a[0].max(0.0), b[0].min(wsx), a[1].max(0.0), b[1].min(wsy));
        if x_lo >= x_hi || y_lo >= y_hi {
            return;
        }
        let levels: Vec<f64> = if g > 0.0 { vec![g, g * 10.0] } else { vec![0.1, 1.0, 10.0, 100.0, 1000.0] };
        for step in levels {
            let px = step as f32 * self.zoom;
            if px < 6.0 || step - 1e-9 > wsx.max(wsy) {
                continue;
            }
            let a = ((px - 6.0) / 34.0).clamp(0.0, 1.0);
            let alpha = ((0.10 + 0.14 * (px / 200.0).min(1.0)) * 255.0 * a) as u8;
            if alpha < 3 {
                continue;
            }
            let st = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, alpha));
            let mut x = step.max((x_lo / step).floor() * step);
            while x < x_hi {
                if x > 0.0 {
                    p.line_segment([self.to_screen([x, y_lo]), self.to_screen([x, y_hi])], st);
                }
                x += step;
            }
            let mut y = step.max((y_lo / step).floor() * step);
            while y < y_hi {
                if y > 0.0 {
                    p.line_segment([self.to_screen([x_lo, y]), self.to_screen([x_hi, y])], st);
                }
                y += step;
            }
        }
    }

    fn paint(&mut self, ui: &egui::Ui, p: &egui::Painter, doc: &mut Doc, env: &Env) {
        let ctx = ui.ctx().clone();
        p.rect_filled(self.rect, 0.0, if env.dark { Color32::from_rgb(0x2b, 0x2c, 0x30) } else { Color32::from_rgb(0xe7, 0xe8, 0xec) });
        let area = Rect::from_min_max(self.to_screen([0.0, 0.0]), self.to_screen([env.ws.0, env.ws.1]));
        p.rect_filled(area, 0.0, Color32::WHITE);
        if let Some((t, op)) = env.overlay {
            p.image(t, area, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(op));
        }
        if env.grid {
            self.draw_grid(p, env);
        }
        p.rect_stroke(area, 0.0, Stroke::new(1.0, Color32::from_gray(0x55)), StrokeKind::Outside);
        let shapes = doc.shapes.clone();
        for s in &shapes {
            self.draw_shape(&ctx, p, s, s.layer(), doc.is_selected(s.id()), env);
        }
        let sel = doc.selected_shapes();
        if let Some(b) = multi_bounds(&sel) {
            let r = Rect::from_min_max(self.to_screen([b[0], b[1]]), self.to_screen([b[0] + b[2], b[1] + b[3]]));
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, BLUE), 4.0, 3.0));
            if self.tool == "select" {
                for (_, hx, hy) in handle_positions(b) {
                    let c = self.to_screen([hx, hy]);
                    let hr = Rect::from_center_size(c, vec2(8.0, 8.0));
                    p.rect_filled(hr, 0.0, Color32::WHITE);
                    p.rect_stroke(hr, 0.0, Stroke::new(1.0, BLUE), StrokeKind::Inside);
                }
            }
            if let Some(bx) = self.center_container(doc, b) {
                let (c, s) = (center(bx), center(b));
                if (c[0] - s[0]).abs() <= 0.05 && (c[1] - s[1]).abs() <= 0.05 {
                    let red = Color32::from_rgba_unmultiplied(230, 20, 20, 140);
                    p.extend(egui::Shape::dashed_line(&[self.to_screen([bx[0], c[1]]), self.to_screen([bx[0] + bx[2], c[1]])], Stroke::new(1.0, red), 3.0, 3.0));
                    p.extend(egui::Shape::dashed_line(&[self.to_screen([c[0], bx[1]]), self.to_screen([c[0], bx[1] + bx[3]])], Stroke::new(1.0, red), 3.0, 3.0));
                    p.circle_filled(self.to_screen(c), 5.0, Color32::from_rgb(255, 26, 26));
                    p.circle_stroke(self.to_screen(c), 5.0, Stroke::new(1.5, Color32::WHITE));
                }
            }
        }
        // plug-in tool overlay, then start points / tabs markers
        if let Some(h) = self.handlers.remove(&self.tool) {
            h.draw(self, doc, env, p);
            self.handlers.insert(self.tool.clone(), h);
        }
        if self.tool != "tabs" {
            crate::tools::draw_tabs(self, doc, p);
        }
        let show_snap = matches!(self.mode, Some(Mode::Move { .. }) | Some(Mode::Draw { .. }) | Some(Mode::Resize { .. }) | Some(Mode::Handler))
            || !matches!(self.tool.as_str(), "select" | "pan");
        if let (Some(q), true) = (self.snap_hit, show_snap) {
            p.rect_stroke(Rect::from_center_size(self.to_screen(q), vec2(10.0, 10.0)), 0.0, Stroke::new(1.5, Color32::from_rgb(0xe0, 0x21, 0x8a)), StrokeKind::Middle);
        }
        // in progress
        let lay_col = env.layers.get(doc.active_layer).map_or(Color32::BLACK, |l| color(&l.color));
        let dash = Stroke::new(1.0, lay_col);
        if let Some(Mode::Draw { a, b }) = self.mode {
            let pts: Vec<P> = match self.tool.as_str() {
                "rect" => vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]], [a[0], a[1]]],
                "ellipse" => {
                    let (cx, cy, rx, ry) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (b[0] - a[0]).abs() / 2.0, (b[1] - a[1]).abs() / 2.0);
                    (0..=72).map(|i| { let t = i as f64 / 72.0 * std::f64::consts::TAU; [cx + rx * t.cos(), cy + ry * t.sin()] }).collect()
                }
                _ => {
                    let r = (b[0] - a[0]).hypot(b[1] - a[1]);
                    let v = if self.tool == "star" { star_points(a[0], a[1], r, r * 0.45, 5) } else { regular_polygon(a[0], a[1], r, env.polygon_sides, -std::f64::consts::PI / 2.0) };
                    let mut pts: Vec<P> = v.iter().map(|q| [q["x"].as_f64().unwrap(), q["y"].as_f64().unwrap()]).collect();
                    if let Some(f) = pts.first().copied() {
                        pts.push(f);
                    }
                    pts
                }
            };
            self.polyline(p, &pts, dash, true);
        }
        if self.tool == "poly" && !self.points.is_empty() {
            let mut pts = self.points.clone();
            if let Some(l) = self.last {
                pts.push(l);
            }
            self.polyline(p, &pts, dash, true);
        }
        if let Some((m1, m2)) = self.measure {
            let red = Color32::from_rgb(0xe0, 0x1b, 0x1b);
            p.line_segment([self.to_screen(m1), self.to_screen(m2)], Stroke::new(1.0, red));
            let d = (m2[0] - m1[0]).hypot(m2[1] - m1[1]);
            let ang = (-(m2[1] - m1[1])).atan2(m2[0] - m1[0]).to_degrees();
            p.text(self.to_screen(m2) + vec2(8.0, -8.0), Align2::LEFT_BOTTOM, format!("{}  {ang:.1}°", env.prefs.fmt_len(d)), FontId::proportional(12.0), red);
        }
        if let Some(h) = env.head {
            let c = self.to_screen(h);
            let red = Stroke::new(1.5, Color32::from_rgb(0xe0, 0x1b, 0x1b));
            p.circle_stroke(c, 6.0, red);
            p.line_segment([c - vec2(11.0, 0.0), c + vec2(11.0, 0.0)], red);
            p.line_segment([c - vec2(0.0, 11.0), c + vec2(0.0, 11.0)], red);
        }
        if let Some(Mode::Marquee { a, b }) = self.mode {
            let right = b[0] >= a[0];
            let r = Rect::from_two_pos(self.to_screen(a), self.to_screen(b));
            let (c, fill) = if right { (BLUE, Color32::from_rgba_unmultiplied(47, 123, 245, 20)) } else { (Color32::from_rgb(0x1a, 0x9d, 0x4a), Color32::from_rgba_unmultiplied(26, 157, 74, 20)) };
            p.rect_filled(r, 0.0, fill);
            if right {
                p.rect_stroke(r, 0.0, Stroke::new(1.0, c), StrokeKind::Inside);
            } else {
                p.extend(egui::Shape::dashed_line(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], Stroke::new(1.0, c), 5.0, 3.0));
            }
        }
    }

    fn draw_rulers(&self, p: &egui::Painter, full: Rect, env: &Env) {
        let (bg, line, text) = if env.dark {
            (Color32::from_rgb(0x26, 0x27, 0x2b), Color32::from_rgb(0x6b, 0x6d, 0x74), Color32::from_rgb(0xa8, 0xaa, 0xb1))
        } else {
            (Color32::from_rgb(0xf7, 0xf7, 0xf9), Color32::from_rgb(0x9a, 0x9c, 0xa3), Color32::from_rgb(0x55, 0x57, 0x5d))
        };
        let u = env.prefs.unit_factor();
        let inch = env.prefs.units == "in";
        let steps: &[f64] = if inch { &[1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0] } else { &[1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0] };
        let major = steps.iter().copied().find(|st| (st / u) as f32 * self.zoom >= 50.0).unwrap_or(*steps.last().unwrap());
        let minor = major / if inch && major < 1.0 { 4.0 } else if (major * 1000.0) % 5000.0 == 0.0 || major == 1.0 { 5.0 } else { 4.0 };
        p.rect_filled(Rect::from_min_max(full.min, pos2(full.max.x, full.min.y + RULER)), 0.0, bg);
        p.rect_filled(Rect::from_min_max(full.min, pos2(full.min.x + RULER, full.max.y)), 0.0, bg);
        let is_major = |v: f64| (v / major - (v / major).round()).abs() < 1e-6;
        let label = |v: f64| if inch { format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_string() } else { format!("{}", v.round()) };
        let font = FontId::monospace(9.0);
        let mut v = (self.to_world(pos2(self.rect.min.x, 0.0))[0] * u / minor).floor() * minor;
        let vend = self.to_world(pos2(full.max.x, 0.0))[0] * u;
        while v <= vend {
            let sx = self.to_screen([v / u, 0.0]).x.round();
            if sx >= self.rect.min.x {
                let big = is_major(v);
                p.line_segment([pos2(sx, full.min.y + RULER), pos2(sx, full.min.y + if big { 3.0 } else { RULER - 5.0 })], Stroke::new(1.0, line));
                if big {
                    p.text(pos2(sx + 3.0, full.min.y + 1.0), Align2::LEFT_TOP, label(v), font.clone(), text);
                }
            }
            v += minor;
        }
        let mut v = (self.to_world(pos2(0.0, self.rect.min.y))[1] * u / minor).floor() * minor;
        let vend = self.to_world(pos2(0.0, full.max.y))[1] * u;
        while v <= vend {
            let sy = self.to_screen([0.0, v / u]).y.round();
            if sy >= self.rect.min.y {
                let big = is_major(v);
                p.line_segment([pos2(full.min.x + RULER, sy), pos2(full.min.x + if big { 3.0 } else { RULER - 5.0 }, sy)], Stroke::new(1.0, line));
                if big {
                    let g = p.layout_no_wrap(label(v), font.clone(), text);
                    p.add(egui::epaint::TextShape::new(pos2(full.min.x + 1.0, sy - 3.0), g, text).with_angle(-std::f32::consts::FRAC_PI_2));
                }
            }
            v += minor;
        }
        if let Some(m) = self.last_mouse {
            let s = self.to_screen(m);
            let red = Stroke::new(1.0, Color32::from_rgb(0xe9, 0x45, 0x60));
            p.line_segment([pos2(s.x, full.min.y), pos2(s.x, full.min.y + RULER)], red);
            p.line_segment([pos2(full.min.x, s.y), pos2(full.min.x + RULER, s.y)], red);
        }
        p.rect_filled(Rect::from_min_size(full.min, vec2(RULER, RULER)), 0.0, bg);
    }
}

#[cfg(target_os = "macos")]
fn screen_points_per_mm() -> Option<f32> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        w: f64,
        h: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        x: f64,
        y: f64,
        size: CGSize,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayScreenSize(display: u32) -> CGSize;
        fn CGDisplayBounds(display: u32) -> CGRect;
    }
    // SAFETY: plain CoreGraphics queries on the main display; no pointers are passed
    let (mm, pts) = unsafe {
        let d = CGMainDisplayID();
        (CGDisplayScreenSize(d), CGDisplayBounds(d))
    };
    (mm.w > 1.0).then(|| (pts.size.w / mm.w) as f32)
}

#[cfg(not(target_os = "macos"))]
fn screen_points_per_mm() -> Option<f32> {
    None
}
