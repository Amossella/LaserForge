//! Camera panel and camera dialogs: preview, overlay of the work area on the canvas, import as image, alignment
//! by 4 corners, lens correction and alignment with burned targets. Ported from laserforge/extras.py
//! (CameraPanel) and laserforge/dialogs.py. A still image can be loaded as the frame (no camera needed).

use crate::app::LaserApp;
use crate::camera::{self, Capture};
use crate::dialogs2::Extra;
use crate::{l, tr};
use egui::{pos2, vec2, Color32, Rect, Sense, Stroke};
use image::RgbImage;
use serde_json::{json, Value};

#[derive(Default)]
pub struct CamState {
    pub cal: Value,
    pub capture: Option<Capture>,
    pub devices: Vec<String>,
    pub index: usize,
    pub frame: Option<RgbImage>,
    pub preview: Option<egui::TextureHandle>,
    pub overlay: Option<egui::TextureHandle>,
    pub show_overlay: bool,
    /// Rebuild the overlay from the current frame on the next poll.
    pub want_overlay: bool,
    /// Capture the overlay once a frame arrives (camera just started).
    pub capture_when_ready: bool,
    pub frame_dirty: bool,
}

impl CamState {
    pub fn opacity(&self) -> f32 {
        self.cal.get("opacity").and_then(|v| v.as_f64()).unwrap_or(70.0) as f32 / 100.0
    }
}

fn to_color(img: &RgbImage) -> egui::ColorImage {
    egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw())
}

impl LaserApp {
    fn corrected(&self) -> Option<RgbImage> {
        let f = self.cam.frame.as_ref()?;
        Some(camera::warp_to_workspace(f, &self.cam.cal, self.ws(), self.prefs.camera_quality.max(200.0)))
    }

    pub fn poll_camera(&mut self, ctx: &egui::Context) {
        if let Some(c) = &self.cam.capture {
            if let Some(f) = c.frame.lock().unwrap().take() {
                self.cam.frame = Some(f);
                self.cam.frame_dirty = true;
            }
            let err = c.error.lock().unwrap().take();
            if let Some(e) = err {
                self.say(format!("Camera: {e}"));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.cam.frame_dirty {
            self.cam.frame_dirty = false;
            if let Some(f) = &self.cam.frame {
                let small = image::imageops::thumbnail(f, 480.min(f.width()), (480 * f.height() / f.width().max(1)).max(1));
                self.cam.preview = Some(ctx.load_texture("campreview", to_color(&small), egui::TextureOptions::LINEAR));
            }
            if std::mem::take(&mut self.cam.capture_when_ready) {
                self.cam.want_overlay = true;
            }
        }
        if std::mem::take(&mut self.cam.want_overlay) {
            match self.corrected() {
                Some(img) => {
                    self.cam.overlay = Some(ctx.load_texture("camoverlay", to_color(&img), egui::TextureOptions::LINEAR));
                    self.cam.show_overlay = true;
                }
                None => self.say(l("Avvia prima la camera", "Start the camera first")),
            }
        }
    }

    pub fn camera_capture(&mut self) {
        if self.cam.capture.is_none() && self.cam.frame.is_none() {
            self.show_panel("winCamera");
            self.start_camera();
            self.cam.capture_when_ready = true;
        } else {
            self.cam.want_overlay = true;
        }
    }

    fn start_camera(&mut self) {
        if self.cam.devices.is_empty() {
            self.cam.devices = camera::list_cameras();
        }
        if self.cam.devices.is_empty() {
            return self.say(l("Nessuna camera trovata", "No camera found"));
        }
        self.cam.capture = Some(Capture::start(self.cam.index as u32));
    }

    pub fn camera_import(&mut self) {
        let Some(img) = self.corrected() else { return self.say(l("Avvia prima la camera", "Start the camera first")) };
        let (w, h) = self.ws();
        let src = crate::importers::encode_image(&image::DynamicImage::ImageRgb8(img));
        let mut p = serde_json::Map::new();
        for (k, v) in [("src", json!(src)), ("name", json!("camera.png")), ("x", json!(0.0)), ("y", json!(0.0)), ("w", json!(w)), ("h", json!(h)),
                       ("adj", crate::shapes::default_adj())] {
            p.insert(k.into(), v);
        }
        let s = self.doc.create("image", p);
        self.doc.add_shapes(vec![s], true);
        self.invalidate();
    }

    pub fn camera_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let name = self.cam.devices.get(self.cam.index).cloned().unwrap_or_else(|| l("Nessuna camera trovata", "No camera found"));
            egui::ComboBox::from_id_salt("camdev").selected_text(name).width(200.0).show_ui(ui, |ui| {
                for (i, d) in self.cam.devices.clone().iter().enumerate() {
                    ui.selectable_value(&mut self.cam.index, i, d);
                }
            });
            if ui.button("↻").clicked() {
                self.cam.devices = camera::list_cameras();
            }
            let running = self.cam.capture.is_some();
            if ui.button(if running { l("Ferma", "Stop") } else { l("Avvia camera", "Start camera") }).clicked() {
                if running {
                    self.cam.capture = None;
                } else {
                    self.start_camera();
                }
            }
        });
        if self.cam.devices.is_empty() && self.cam.capture.is_none() {
            self.cam.devices = camera::list_cameras();
        }
        match &self.cam.preview {
            Some(t) => {
                let sz = t.size_vec2();
                let k = (ui.available_width() / sz.x).min(260.0 / sz.y);
                ui.image((t.id(), sz * k));
            }
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, Color32::from_gray(0x11));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, l("Nessuna immagine", "No image"), egui::FontId::proportional(13.0), Color32::GRAY);
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(egui::RichText::new(l("Aggiorna sovrapposizione", "Update overlay")).strong()).clicked() {
                self.cam.want_overlay = true;
            }
            if ui.button(l("Importa come immagine", "Import as image")).clicked() {
                self.camera_import();
            }
            if ui.button(l("Carica foto come fotogramma…", "Load photo as frame…")).on_hover_text(l("Usa una foto del piano al posto della camera (per allineare senza camera collegata)", "Use a photo of the bed instead of the camera (to align without a camera)")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Immagini", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file() {
                    match std::fs::read(p).ok().and_then(|b| image::load_from_memory(&b).ok()) {
                        Some(img) => {
                            self.cam.capture = None;
                            self.cam.frame = Some(img.to_rgb8());
                            self.cam.frame_dirty = true;
                        }
                        None => self.say(l("Immagine non supportata", "Unsupported image")),
                    }
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(l("Allinea (4 angoli)…", "Align (4 corners)…")).clicked() {
                self.run("cameraAlign");
            }
            if ui.button(l("Correzione lente…", "Lens correction…")).clicked() {
                self.run("cameraLens");
            }
            if ui.button(l("Allinea con segni incisi…", "Align with burned targets…")).clicked() {
                self.run("cameraTargets");
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.cam.show_overlay, l("Mostra sull'area di lavoro · opacità", "Show on work area · opacity"));
            let mut op = self.cam.cal.get("opacity").and_then(|v| v.as_f64()).unwrap_or(70.0);
            if ui.add(egui::Slider::new(&mut op, 10.0..=100.0).show_value(false)).changed() {
                self.cam.cal["opacity"] = json!(op.round());
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(l("Allinea la camera una volta (Strumenti laser › Allinea camera) indicando dove cadono gli angoli dell'area di lavoro nell'immagine.",
                                                      "Align the camera once (Laser Tools › Camera Alignment) by marking where the work-area corners fall in the image.")).weak().size(11.0)).wrap());
    }
}

/// An image the user clicks points on; returns the clicked points in image pixels.
fn picker(ui: &mut egui::Ui, tex: &egui::TextureHandle, img_size: (f64, f64), points: &mut Vec<[f64; 2]>, max: usize, labels: &[&str], grid: bool, closed: bool) {
    let avail = vec2(ui.available_width().max(600.0), 440.0);
    let (rect, resp) = ui.allocate_exact_size(avail, Sense::click());
    let k = (rect.width() as f64 / img_size.0).min(rect.height() as f64 / img_size.1);
    let o = rect.min + vec2(((rect.width() as f64 - img_size.0 * k) / 2.0) as f32, ((rect.height() as f64 - img_size.1 * k) / 2.0) as f32);
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, Color32::from_gray(0x11));
    let ir = Rect::from_min_size(o, vec2((img_size.0 * k) as f32, (img_size.1 * k) as f32));
    p.image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    if grid {
        for i in 1..8 {
            let x = ir.left() + ir.width() * i as f32 / 8.0;
            let y = ir.top() + ir.height() * i as f32 / 8.0;
            p.line_segment([pos2(x, ir.top()), pos2(x, ir.bottom())], Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 255, 0, 70)));
            p.line_segment([pos2(ir.left(), y), pos2(ir.right(), y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 255, 0, 70)));
        }
    }
    if resp.clicked() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let (x, y) = (((pp.x - o.x) as f64) / k, ((pp.y - o.y) as f64) / k);
            if (0.0..=img_size.0).contains(&x) && (0.0..=img_size.1).contains(&y) {
                if points.len() >= max {
                    points.clear();
                }
                points.push([x, y]);
            }
        }
    }
    let red = Color32::from_rgb(0xff, 0x2a, 0x2a);
    let sp: Vec<egui::Pos2> = points.iter().map(|q| o + vec2((q[0] * k) as f32, (q[1] * k) as f32)).collect();
    for (i, q) in sp.iter().enumerate() {
        p.circle_stroke(*q, 6.0, Stroke::new(2.0, red));
        let lab = labels.get(i).map(|s| s.to_string()).unwrap_or_else(|| format!("{}", i + 1));
        p.text(*q + vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, lab, egui::FontId::proportional(12.0), red);
        if i > 0 {
            p.line_segment([sp[i - 1], *q], Stroke::new(2.0, red));
        }
    }
    if closed && sp.len() == max && max > 2 {
        p.line_segment([sp[max - 1], sp[0]], Stroke::new(2.0, red));
    }
}

struct CameraAlign {
    tex: Option<egui::TextureHandle>,
    size: (f64, f64),
    corners: Vec<[f64; 2]>,
}

impl Extra for CameraAlign {
    fn title(&self) -> String {
        l("Allinea camera", "Camera Alignment")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        ui.add(egui::Label::new(egui::RichText::new(l("Clicca nell'immagine i 4 angoli dell'area di lavoro in quest'ordine: in alto a sinistra, in alto a destra, in basso a destra, in basso a sinistra. Suggerimento: incidi prima un rettangolo grande come l'area di lavoro (o segna gli angoli portando il laser).",
                                                      "Click the 4 corners of the work area in this order: top-left, top-right, bottom-right, bottom-left. Tip: first burn a rectangle the size of the work area (or mark the corners by jogging the laser).")).weak().size(11.0)).wrap());
        if self.tex.is_none() {
            let f = app.cam.frame.clone()?;
            self.size = (f.width() as f64, f.height() as f64);
            self.tex = Some(ui.ctx().load_texture("camalign", to_color(&f), egui::TextureOptions::LINEAR));
        }
        picker(ui, self.tex.as_ref().unwrap(), self.size, &mut self.corners, 4, &["TL", "TR", "BR", "BL"], false, true);
        if ui.button(l("Azzera punti", "Clear points")).clicked() {
            self.corners.clear();
        }
        ui.separator();
        let mut r = None;
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("OK").strong()).clicked() {
                r = Some(true);
            }
            if ui.button(tr("cancel")).clicked() {
                r = Some(false);
            }
        });
        r
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.corners.len() != 4 {
            app.say(l("Servono 4 punti", "4 points are needed"));
            return false;
        }
        app.cam.cal["corners"] = json!(self.corners);
        if let Some(o) = app.cam.cal.as_object_mut() {
            o.remove("points");
        }
        app.cam.want_overlay = true;
        true
    }
}

struct CameraLens {
    small: RgbImage,
    tex: Option<egui::TextureHandle>,
    shown: (f64, f64),
    k1: f64,
    k2: f64,
    points: Vec<[f64; 2]>,
}

impl Extra for CameraLens {
    fn title(&self) -> String {
        l("Correzione lente camera", "Camera Lens Correction")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        ui.add(egui::Label::new(egui::RichText::new(l("Muovi i cursori finché i bordi dritti dell'area di lavoro appaiono dritti (la griglia aiuta). Oppure clicca almeno 4 punti lungo un bordo che deve essere dritto e premi 'Stima dalla linea'.",
                                                      "Move the sliders until straight edges of the work area look straight (the grid helps). Or click at least 4 points along an edge that should be straight and press 'Estimate from line'.")).weak().size(11.0)).wrap());
        if self.tex.is_none() || self.shown != (self.k1, self.k2) {
            self.shown = (self.k1, self.k2);
            let img = camera::undistort(&self.small, self.k1, self.k2);
            self.tex = Some(ui.ctx().load_texture("camlens", to_color(&img), egui::TextureOptions::LINEAR));
        }
        let size = (self.small.width() as f64, self.small.height() as f64);
        picker(ui, self.tex.as_ref().unwrap(), size, &mut self.points, 64, &[], true, false);
        ui.horizontal(|ui| {
            ui.label("k1");
            ui.add(egui::Slider::new(&mut self.k1, -0.6..=0.6).step_by(0.001));
            ui.label("k2");
            ui.add(egui::Slider::new(&mut self.k2, -0.6..=0.6).step_by(0.001));
        });
        ui.horizontal(|ui| {
            if ui.button(l("Stima dalla linea", "Estimate from line")).clicked() {
                if self.points.len() < 4 {
                    app.say(l("Clicca almeno 4 punti lungo un bordo dritto", "Click at least 4 points along a straight edge"));
                } else {
                    self.k1 = camera::fit_k1(&self.points, size);
                    self.k2 = 0.0;
                    self.points.clear();
                }
            }
            if ui.button(l("Azzera punti", "Clear points")).clicked() {
                self.points.clear();
            }
        });
        ui.separator();
        let mut r = None;
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("OK").strong()).clicked() {
                r = Some(true);
            }
            if ui.button(tr("cancel")).clicked() {
                r = Some(false);
            }
        });
        r
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        app.cam.cal["k1"] = json!((self.k1 * 1000.0).round() / 1000.0);
        app.cam.cal["k2"] = json!((self.k2 * 1000.0).round() / 1000.0);
        app.cam.want_overlay = true;
        true
    }
}

struct CameraTargets {
    power: f64,
    speed: f64,
    targets: Vec<[f64; 2]>,
    tex: Option<egui::TextureHandle>,
    size: (f64, f64),
    points: Vec<[f64; 2]>,
}

impl Extra for CameraTargets {
    fn title(&self) -> String {
        l("Allinea camera con segni incisi", "Camera alignment with burned targets")
    }
    fn body(&mut self, ui: &mut egui::Ui, app: &mut LaserApp) -> Option<bool> {
        ui.add(egui::Label::new(egui::RichText::new(l("1) Metti un foglio di cartone sul piano e premi 'Incidi i segni': il laser incide 9 crocette in posizioni note. 2) Premi 'Cattura' e clicca il centro di ogni crocetta nell'ordine dei numeri (da sinistra a destra, dall'alto in basso).",
                                                      "1) Put scrap card on the bed and press 'Burn targets': the laser burns 9 crosses at known positions. 2) Press 'Capture' and click the centre of each cross in number order (left to right, top to bottom).")).weak().size(11.0)).wrap());
        egui::Grid::new("ct").num_columns(2).show(ui, |ui| {
            ui.label(l("Potenza segni %", "Target power %"));
            ui.add(egui::DragValue::new(&mut self.power).range(1.0..=100.0));
            ui.end_row();
            ui.label(l("Velocità segni (mm/min)", "Target speed (mm/min)"));
            ui.add(egui::DragValue::new(&mut self.speed).range(50.0..=20000.0).speed(10.0));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button(l("Incidi i segni", "Burn targets")).clicked() {
                if !app.connected() {
                    app.say(l("Connetti prima il laser", "Connect a laser first"));
                } else {
                    let s_val = (self.power / 100.0 * app.machine.max_power as f64).round();
                    let mut g = vec!["; camera alignment targets".to_string(), "G21".into(), "G90".into(), "M4 S0".into()];
                    for t in &self.targets {
                        let (x, y) = (t[0], t[1]);
                        for (a, b) in [([x - 4.0, y], [x + 4.0, y]), ([x, y - 4.0], [x, y + 4.0])] {
                            let (a, b) = (crate::project::to_machine(&app.machine, a), crate::project::to_machine(&app.machine, b));
                            g.push(format!("G0 X{:.3} Y{:.3}", a[0], a[1]));
                            g.push(format!("G1 X{:.3} Y{:.3} F{:.0} S{s_val}", b[0], b[1], self.speed));
                            g.push("S0".into());
                        }
                    }
                    g.push("M5".into());
                    if let Some(c) = &app.conn {
                        c.stream(g.join("\n"));
                    }
                }
            }
            if ui.button(l("Cattura", "Capture")).clicked() {
                match &app.cam.frame {
                    Some(f) => {
                        let k = |key: &str| app.cam.cal.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let raw = camera::undistort(f, k("k1"), k("k2"));
                        self.size = (raw.width() as f64, raw.height() as f64);
                        self.tex = Some(ui.ctx().load_texture("camtargets", to_color(&raw), egui::TextureOptions::LINEAR));
                        self.points.clear();
                    }
                    None => app.say(l("Avvia prima la camera", "Start the camera first")),
                }
            }
            if ui.button(l("Azzera punti", "Clear points")).clicked() {
                self.points.clear();
            }
        });
        if let Some(t) = self.tex.clone() {
            picker(ui, &t, self.size, &mut self.points, self.targets.len(), &[], false, false);
        }
        ui.separator();
        let mut r = None;
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("OK").strong()).clicked() {
                r = Some(true);
            }
            if ui.button(tr("cancel")).clicked() {
                r = Some(false);
            }
        });
        r
    }
    fn apply(&mut self, app: &mut LaserApp) -> bool {
        if self.points.len() < 4 {
            app.say(l("Clicca almeno 4 segni (meglio tutti e 9)", "Click at least 4 targets (all 9 is best)"));
            return false;
        }
        app.cam.cal["points"] = json!(self.points.iter().zip(&self.targets).map(|(p, t)| json!([p, t])).collect::<Vec<_>>());
        if let Some(o) = app.cam.cal.as_object_mut() {
            o.remove("corners");
        }
        app.cam.want_overlay = true;
        true
    }
}

pub fn for_action(aid: &str, app: &mut LaserApp) -> Option<Box<dyn Extra>> {
    let need_frame = |app: &mut LaserApp| -> Option<RgbImage> {
        let f = app.cam.frame.clone();
        if f.is_none() {
            app.say(l("Avvia prima la camera (Finestra › Controllo camera)", "Start the camera first (Window › Camera Control)"));
            app.show_panel("winCamera");
        }
        f
    };
    Some(match aid {
        "cameraAlign" => {
            need_frame(app)?;
            let corners = app.cam.cal.get("corners").and_then(|c| serde_json::from_value(c.clone()).ok()).unwrap_or_default();
            Box::new(CameraAlign { tex: None, size: (1.0, 1.0), corners })
        }
        "cameraLens" => {
            let f = need_frame(app)?;
            let small = image::imageops::thumbnail(&f, 900.min(f.width()), (900 * f.height() / f.width().max(1)).min(700).max(1));
            let k = |key: &str| app.cam.cal.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Box::new(CameraLens { small, tex: None, shown: (f64::NAN, f64::NAN), k1: k("k1"), k2: k("k2"), points: Vec::new() })
        }
        "cameraTargets" => Box::new(CameraTargets { power: 20.0, speed: 1000.0, targets: camera::alignment_targets(app.ws(), 3, 0.15), tex: None, size: (1.0, 1.0), points: Vec::new() }),
        _ => return None,
    })
}
