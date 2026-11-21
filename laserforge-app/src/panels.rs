//! Side panels: Cuts / layers (with the full layer editor), Laser, Move, Console, Objects, Properties, Pages,
//! Macros. Ported from laserforge/panels.py and laserforge/props.py.

use crate::app::{color, fmt_time, LaserApp};
use crate::connection::{list_ports, Link};
use crate::dialogs::Dialog;
use crate::shapes::{flatten, multi_bounds};
use crate::{l, tr};
use egui::{vec2, Color32, Key, RichText, Sense};
use laserforge_core::grbl::problems;
use laserforge_core::model::{AirAssist, CutMode, LaserMode};
use serde_json::{json, Value};

const AIR: Color32 = Color32::from_rgb(0x4f, 0xc3, 0xf7);

fn num(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64, suffix: &str) -> bool {
    ui.label(label);
    let r = ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix).max_decimals(3)).changed();
    ui.end_row();
    r
}

/// Panels docked in the lower group (machine side); the others go in the upper group (design side).
pub fn is_lower_panel(id: &str) -> bool {
    matches!(id, "winLaser" | "winLibrary" | "winConsole" | "winArt" | "winMacros")
}

pub fn panel_name(id: &str) -> String {
    match id {
        "winCuts" => l("Tagli / Livelli", "Cuts / Layers"), "winLaser" => "Laser".into(), "winMove" => l("Sposta", "Move"),
        "winConsole" => "Console".into(), "winObjects" => l("Oggetti", "Objects"), "winProps" => l("Proprietà", "Properties"),
        "winPages" => l("Pagine", "Pages"), "winLibrary" => l("Libreria dei materiali", "Material Library"), "winArt" => l("Libreria grafica", "Art Library"),
        "winCamera" => l("Camera", "Camera"), "winMacros" => "Macro".into(), _ => id.to_string(),
    }
}

impl LaserApp {
    pub fn side_panels(&mut self, ui: &mut egui::Ui) {
        let upper: Vec<String> = self.panels.iter().filter(|p| !is_lower_panel(p)).cloned().collect();
        let lower: Vec<String> = self.panels.iter().filter(|p| is_lower_panel(p)).cloned().collect();
        if !upper.is_empty() && !lower.is_empty() {
            let h = ui.available_height();
            egui::Panel::bottom("side_lower").resizable(true).default_size(h * 0.52).min_size(160.0).show(ui, |ui| self.panel_group(ui, &lower, true));
            self.panel_group(ui, &upper, false);
        } else if !upper.is_empty() {
            self.panel_group(ui, &upper, false);
        } else {
            self.panel_group(ui, &lower, true);
        }
    }

    /// One group of docked panels: a title strip, the open panel, and its tabs along the bottom.
    fn panel_group(&mut self, ui: &mut egui::Ui, ids: &[String], lower: bool) {
        let cur = if lower { self.side_tab2.clone() } else { self.side_tab.clone() };
        let cur = if ids.contains(&cur) { cur } else { ids[0].clone() };
        if lower { self.side_tab2 = cur.clone() } else { self.side_tab = cur.clone() }
        // title strip
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        let accent = ui.visuals().selection.bg_fill;
        ui.painter().rect_filled(r, 2.0, accent.gamma_multiply(0.55));
        ui.painter().text(r.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, panel_name(&cur), egui::FontId::proportional(12.5), Color32::WHITE);
        // tabs at the bottom
        egui::Panel::bottom(egui::Id::new(("tabs", lower))).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for id in ids {
                    if ui.selectable_label(*id == cur, panel_name(id)).clicked() {
                        if lower { self.side_tab2 = id.clone() } else { self.side_tab = id.clone() }
                    }
                }
            });
        });
        egui::ScrollArea::vertical().id_salt(("panel", lower)).auto_shrink([false, false]).show(ui, |ui| self.panel_body(ui, &cur));
    }

    fn panel_body(&mut self, ui: &mut egui::Ui, id: &str) {
        match id {
            "winCuts" => self.cuts_panel(ui),
            "winLaser" => self.laser_panel(ui),
            "winMove" => self.move_panel(ui),
            "winConsole" => self.console_panel(ui),
            "winObjects" => self.objects_panel(ui),
            "winProps" => self.props_panel(ui),
            "winPages" => self.pages_panel(ui),
            "winMacros" => self.macros_panel(ui),
            "winLibrary" => self.library_panel(ui),
            "winArt" => self.art_panel(ui),
            "winCamera" => self.camera_panel(ui),
            _ => {
                ui.weak(l("Questo pannello non è ancora disponibile nella versione Rust.", "This panel is not available in the Rust version yet."));
            }
        }
    }

    // ------------------------------------------------------------------ cuts / layers
    fn cuts_panel(&mut self, ui: &mut egui::Ui) {
        let used = self.doc.used_layers();
        let mut show = used.clone();
        if !show.contains(&self.doc.active_layer) {
            show.push(self.doc.active_layer);
            show.sort();
        }
        let flat = flatten(&self.doc.shapes);
        let sf = self.prefs.speed_factor();
        let mut open_layer = None;
        egui::Grid::new("cuts").striped(true).num_columns(6).spacing([8.0, 4.0]).show(ui, |ui| {
            for h in ["#", &tr("layer"), &tr("mode"), "Vel/Pot", &tr("air"), &tr("output")] {
                ui.weak(h);
            }
            ui.end_row();
            for i in show {
                let has_img = flat.iter().any(|s| s.layer() == i as i64 && s.kind() == "image");
                let lay = self.doc.layers[i].clone();
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 16.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, color(&lay.color));
                let name = format!("{}{}", lay.name, if used.contains(&i) { "" } else { " •" });
                let r = ui.selectable_label(i == self.doc.active_layer, name).on_hover_text(l("Doppio clic: impostazioni del livello", "Double-click: layer settings"));
                if r.clicked() {
                    self.doc.active_layer = i;
                }
                if r.double_clicked() {
                    open_layer = Some(i);
                }
                let mode = if lay.tool { l("Tecnico", "Tool") } else if has_img { l("Immagine", "Image") } else {
                    match lay.cut_mode { CutMode::Line => tr("modeLine"), CutMode::Fill => tr("modeFill"), CutMode::OffsetFill => tr("modeOffsetFill"), CutMode::LineFill => l("Lin+Riem", "Line+Fill") }
                };
                ui.label(mode);
                ui.label(if lay.tool { "—".to_string() } else { format!("{:.0}/{}%", lay.speed * sf, lay.power_max.round()) });
                if lay.tool || lay.air_assist == AirAssist::Off {
                    ui.label("—");
                } else {
                    ui.label(RichText::new(&self.machine.air_command).color(AIR));
                }
                let out = if lay.tool { "T" } else if lay.enabled { "◉" } else { "○" };
                let tip = if lay.tool { l("Livello tecnico: mai inviato al laser", "Tool layer: never sent to the laser") } else { tr("output") };
                let col = if lay.enabled && !lay.tool { Color32::from_rgb(0x4c, 0xaf, 0x50) } else { Color32::GRAY };
                if ui.add(egui::Label::new(RichText::new(out).color(col)).sense(Sense::click())).on_hover_text(tip).clicked() {
                    if lay.tool {
                        self.say(l("T1/T2 sono livelli tecnici: non vanno mai al laser", "T1/T2 are tool layers: never sent to the laser"));
                    } else {
                        self.doc.layers[i].enabled = !lay.enabled;
                        self.invalidate();
                    }
                }
                ui.end_row();
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(l("Impostazioni livello…", "Layer settings…")).clicked() {
                open_layer = Some(self.doc.active_layer);
            }
        });
        ui.add(egui::Label::new(RichText::new(l("Doppio clic su un livello per aprirne le impostazioni (velocità, potenza, riempimento, kerf…).",
                                                 "Double-click a layer to open its settings (speed, power, fill, kerf…).")).weak().size(11.0)).wrap());
        if let Some(i) = open_layer {
            self.doc.active_layer = i;
            let orig = Box::new(self.doc.layers[i].clone());
            self.dialogs.push(Dialog::Layer { idx: i, orig });
        }
    }

    /// Every setting of one layer (shown in the layer window).
    pub fn layer_editor(&mut self, ui: &mut egui::Ui, i: usize) {
        let flat = flatten(&self.doc.shapes);
        let sf = self.prefs.speed_factor();
        let img = flat.iter().any(|s| s.layer() == i as i64 && s.kind() == "image");
        let air_cmd = self.machine.air_command.clone();
        let before = serde_json::to_value(&self.doc.layers[i]).unwrap();
        let mut air_test = None;
        let mut sub = false;
        {
            let lay = &mut self.doc.layers[i];
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, color(&lay.color));
                ui.strong(&lay.name);
                if lay.tool {
                    ui.weak(l("(livello tecnico, non va al laser)", "(tool layer, not sent to the laser)"));
                }
            });
            ui.add_enabled_ui(!lay.tool, |ui| {
                egui::Grid::new("layer_main").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                    let mut sp = lay.speed * sf;
                    ui.label(format!("{} ({})", tr("speed").split('(').next().unwrap_or("").trim(), self.prefs.speed_units));
                    if ui.add(egui::DragValue::new(&mut sp).range(0.01..=100000.0 * sf).speed(10.0 * sf)).changed() {
                        lay.speed = sp / sf;
                    }
                    ui.end_row();
                    ui.label(tr("passes"));
                    ui.add(egui::DragValue::new(&mut lay.passes).range(1..=100));
                    ui.end_row();
                    ui.label(tr("powerMax"));
                    ui.add(egui::Slider::new(&mut lay.power_max, 0.0..=100.0).suffix(" %"));
                    ui.end_row();
                    ui.label(tr("powerMin"));
                    ui.add(egui::Slider::new(&mut lay.power_min, 0.0..=100.0).suffix(" %"));
                    ui.end_row();
                    ui.label(tr("mode"));
                    ui.horizontal(|ui| {
                        for (m, k) in [(CutMode::Line, "modeLine"), (CutMode::Fill, "modeFill"), (CutMode::OffsetFill, "modeOffsetFill"), (CutMode::LineFill, "modeLineFill")] {
                            ui.selectable_value(&mut lay.cut_mode, m, tr(k));
                        }
                    });
                    ui.end_row();
                    ui.label("Laser");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut lay.laser_mode, LaserMode::Constant, tr("laserConst"));
                        ui.selectable_value(&mut lay.laser_mode, LaserMode::Dynamic, tr("laserDyn"));
                    });
                    ui.end_row();
                });
                if lay.cut_mode != CutMode::Line || img {
                    ui.group(|ui| {
                        ui.strong(l("Riempimento", "Fill"));
                        egui::Grid::new("layer_fill").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                            num(ui, &tr("interval"), &mut lay.line_interval, 0.01..=5.0, 0.005, " mm");
                            num(ui, "Overscan (mm)", &mut lay.overscan, 0.0..=50.0, 0.1, "");
                            ui.label(tr("scanAngle"));
                            ui.horizontal(|ui| {
                                for a in [0.0, 45.0, 90.0, 135.0] {
                                    ui.selectable_value(&mut lay.scan_angle, a, format!("{a:.0}°"));
                                }
                                ui.add(egui::DragValue::new(&mut lay.scan_angle).range(-180.0..=180.0).suffix("°"));
                            });
                            ui.end_row();
                        });
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut lay.bidir_scan, tr("bidir"));
                            ui.checkbox(&mut lay.cross_hatch, "Cross-hatch");
                        });
                        ui.checkbox(&mut lay.flood_fill, "Flood Fill").on_hover_text(l("Riempie un'area alla volta invece di attraversare tutto il disegno a ogni riga", "Fills one area at a time instead of scanning across the whole design"));
                    });
                }
                if img {
                    ui.group(|ui| {
                        ui.strong(l("Immagine", "Image"));
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut lay.pass_through, "Pass-Through").on_hover_text(l("Usa i pixel dell'immagine così come sono (già retinata): niente ricampionamento né regolazioni", "Use the image pixels as they are (already dithered): no resampling, no adjustments"));
                            ui.checkbox(&mut lay.bottom_up, l("Dal basso in alto", "Bottom-up"));
                        });
                        egui::Grid::new("layer_img").num_columns(2).show(ui, |ui| {
                            num(ui, &l("Correzione larghezza punto (mm)", "Dot width correction (mm)"), &mut lay.dot_width, 0.0..=1.0, 0.005, "");
                        });
                    });
                }
                ui.group(|ui| {
                    ui.label(RichText::new(tr("airAssist")).color(AIR).strong());
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut lay.air_assist, AirAssist::Off, tr("airOff"));
                        ui.selectable_value(&mut lay.air_assist, AirAssist::On, format!("{} ({air_cmd})", l("Accesa", "On")));
                        ui.add(egui::DragValue::new(&mut lay.air_power).range(0.0..=100.0).suffix(" %"));
                    });
                    ui.checkbox(&mut lay.air_cut_only, l("Solo durante il taglio (spenta negli spostamenti)", "Only while cutting (off during travel)"));
                    ui.horizontal(|ui| {
                        if ui.button(format!("{} {air_cmd} ON", l("Prova", "Test"))).clicked() {
                            air_test = Some(true);
                        }
                        if ui.button(format!("{} M9 OFF", l("Prova", "Test"))).clicked() {
                            air_test = Some(false);
                        }
                    });
                });
                egui::CollapsingHeader::new(tr("advanced")).show(ui, |ui| {
                    egui::Grid::new("layer_adv").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
                        num(ui, "Z offset (mm)", &mut lay.z_offset, -50.0..=50.0, 0.1, "");
                        num(ui, &tr("powerScale"), &mut lay.power_scale, 1.0..=100.0, 1.0, " %");
                        num(ui, "Kerf (mm)", &mut lay.kerf_offset, -2.0..=2.0, 0.005, "");
                        num(ui, "Dwell (ms)", &mut lay.dwell_time, 0.0..=1000.0, 1.0, "");
                        ui.label(tr("tabs"));
                        ui.add(egui::DragValue::new(&mut lay.tab_count).range(0..=20));
                        ui.end_row();
                        num(ui, &tr("tabWidth"), &mut lay.tab_width, 0.5..=20.0, 0.1, " mm");
                        num(ui, &tr("ramp"), &mut lay.ramp_length, 0.0..=50.0, 0.1, " mm");
                        num(ui, &tr("startDelay"), &mut lay.start_delay, 0.0..=1000.0, 1.0, " ms");
                        num(ui, &tr("zStep"), &mut lay.z_step, 0.0..=20.0, 0.05, " mm");
                        ui.label("");
                        ui.checkbox(&mut lay.dot_mode, tr("dotMode"));
                        ui.end_row();
                        num(ui, "Lead-in (mm)", &mut lay.lead_in, 0.0..=50.0, 0.1, "");
                        num(ui, "Lead-out (mm)", &mut lay.lead_out, 0.0..=50.0, 0.1, "");
                        num(ui, &l("Overcut (mm, forme chiuse)", "Overcut (mm, closed shapes)"), &mut lay.overcut, 0.0..=50.0, 0.1, "");
                        ui.label("");
                        ui.checkbox(&mut lay.perforate, l("Perforazione (taglio tratteggiato)", "Perforation (dashed cut)"));
                        ui.end_row();
                        num(ui, &l("Taglio (mm)", "Cut (mm)"), &mut lay.perf_cut, 0.1..=100.0, 0.05, "");
                        num(ui, &l("Salto (mm)", "Skip (mm)"), &mut lay.perf_skip, 0.1..=100.0, 0.05, "");
                    });
                    let n = lay.sublayers.len();
                    if ui.button(format!("{}{}", l("Sotto-layer…", "Sub-layers…"), if n > 0 { format!(" ({n})") } else { String::new() })).clicked() {
                        sub = true;
                    }
                });
            });
        }
        if sub {
            let d = Dialog::sublayers(i, &self.doc.layers[i]);
            self.dialogs.push(d);
        }
        if let Some(on) = air_test {
            let cmd = if on { self.machine.air_command.clone() } else { "M9".into() };
            self.send(&cmd);
        }
        if serde_json::to_value(&self.doc.layers[i]).unwrap() != before {
            self.invalidate();
        }
    }

    // ------------------------------------------------------------------ laser
    fn laser_panel(&mut self, ui: &mut egui::Ui) {
        self.laser_actions(ui);
        ui.separator();
        self.laser_connection(ui);
    }

    /// Job buttons, "Start from", job origin and job options (top of the Laser panel).
    fn laser_actions(&mut self, ui: &mut egui::Ui) {
        let state = self.conn.as_ref().map(|c| c.shared.lock().unwrap().status.state.clone()).unwrap_or_default();
        ui.label(if !self.connected() { l("Non connesso", "Not connected") } else if state.is_empty() { tr("ready") } else { state });
        if self.job.is_some() {
            self.job_progress(ui);
        }
        let busy = self.busy();
        let btn = |ui: &mut egui::Ui, text: String, enabled: bool| -> bool {
            ui.add_enabled_ui(enabled, |ui| ui.add_sized([ui.available_width(), 28.0], egui::Button::new(text))).inner.clicked()
        };
        let mut act: Option<&str> = None;
        ui.columns(3, |c| {
            if btn(&mut c[0], l("Pausa", "Pause"), true) { act = Some("pauseLaser"); }
            if btn(&mut c[1], "■ Stop".into(), true) { act = Some("stopLaser"); }
            if btn(&mut c[2], format!("▶ {}", l("Avvia", "Start")), !busy) { act = Some("startLaser"); }
        });
        ui.columns(4, |c| {
            if btn(&mut c[0], "Frame".to_string(), !busy) { act = Some("frame"); }
            if btn(&mut c[1], l("Frame elastico", "Rubber frame"), !busy) { act = Some("frameRubber"); }
            if btn(&mut c[2], l("Salva G-code", "Save G-code"), true) { act = Some("saveGcode"); }
            if btn(&mut c[3], l("Anteprima", "Preview"), true) { act = Some("preview"); }
        });
        ui.columns(3, |c| {
            if btn(&mut c[0], "Home".into(), !busy) { act = Some("homeLaser"); }
            if btn(&mut c[1], l("Vai all'origine", "Go to origin"), !busy) { act = Some("gotoOrigin"); }
            if btn(&mut c[2], l("Riprendi", "Resume"), true) { act = Some("resumeLaser"); }
        });
        if let Some(a) = act.take() {
            self.run(a);
        }
        egui::Grid::new("laser_opts").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(l("Inizia da:", "Start from:"));
            let names = [("absolute", l("Coordinate assolute", "Absolute coords")), ("current", l("Posizione attuale", "Current position")), ("user", l("Origine utente", "User origin"))];
            let cur = names.iter().find(|n| n.0 == self.start_from).map(|n| n.1.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("start").selected_text(cur).width(200.0).show_ui(ui, |ui| {
                for (k, t) in names {
                    if ui.selectable_label(self.start_from == k, t).clicked() {
                        if k == "user" && !self.user_origin_set {
                            self.say(l("Imposta prima l'origine utente: porta il laser nel punto voluto e premi «Imposta origine»", "Set the user origin first: jog the laser to the spot and press “Set origin”"));
                        } else {
                            self.start_from = k.into();
                            self.invalidate();
                        }
                    }
                }
            });
            ui.end_row();
            ui.label(l("Origine lavoro", "Job origin")).on_hover_text(l("Usata con «Posizione attuale» e «Origine utente»", "Used with “Current position” and “User origin”"));
            ui.add_enabled_ui(self.start_from != "absolute", |ui| {
                egui::Grid::new("job_origin").spacing([18.0, 2.0]).show(ui, |ui| {
                    for k in 0..9 {
                        if ui.radio(self.job_origin == k, "").clicked() {
                            self.job_origin = k;
                            self.invalidate();
                        }
                        if k % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            });
            ui.end_row();
        });
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut self.prefs.cut_selected, l("Taglia la grafica selezionata", "Cut selected graphics")).changed() {
                self.invalidate();
            }
            if ui.checkbox(&mut self.prefs.origin_from_selection, l("Usa origine selezione", "Use selection origin")).changed() {
                self.invalidate();
            }
        });
        ui.columns(2, |c| {
            if c[0].add_sized([c[0].available_width(), 24.0], egui::Button::new(l("Impostazioni ottimizzazione", "Optimization settings"))).clicked() {
                act = Some("optimizeSettings");
            }
            if c[1].add_sized([c[1].available_width(), 24.0], egui::Button::new(l("Impostazioni macchina", "Machine settings"))).clicked() {
                act = Some("deviceSettings");
            }
        });
        if let Some(a) = act.take() {
            self.run(a);
        }
        if let Some(o) = &self.gcode {
            ui.weak(format!("G-code: {} {} · {}", o.gcode.lines().count(), l("righe", "lines"), fmt_time(o.seconds)));
        }
    }

    /// Device, port and connection state (bottom of the Laser panel).
    fn laser_connection(&mut self, ui: &mut egui::Ui) {
        if !self.devices.is_empty() {
            ui.horizontal(|ui| {
                ui.label(l("Dispositivo", "Device"));
                let cur = if self.device.is_empty() { l("(impostazioni attuali)", "(current settings)") } else { self.device.clone() };
                let mut pick: Option<Value> = None;
                egui::ComboBox::from_id_salt("device").width(200.0).selected_text(cur).show_ui(ui, |ui| {
                    for dv in self.devices.clone() {
                        let name = dv["name"].as_str().unwrap_or("?").to_string();
                        if ui.selectable_label(self.device == name, &name).clicked() {
                            pick = Some(dv);
                        }
                    }
                });
                if let Some(dv) = pick {
                    self.apply_device(&dv);
                }
            });
        }
        if !self.connected() {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("port").width(210.0)
                    .selected_text(if self.port.is_empty() { l("Porta…", "Port…") } else { self.port.clone() })
                    .show_ui(ui, |ui| {
                        for p in self.ports.clone() {
                            ui.selectable_value(&mut self.port, p.clone(), p);
                        }
                        for (n, h, p) in self.net_found.clone() {
                            ui.selectable_value(&mut self.port, format!("tcp://{h}:{p}"), format!("{n} — {h}:{p}"));
                        }
                        ui.selectable_value(&mut self.port, "SIM".to_string(), l("Simulatore (senza laser)", "Simulator (no laser)"));
                    });
                if ui.button("⟳").on_hover_text(l("Aggiorna porte", "Refresh ports")).clicked() {
                    self.ports = list_ports();
                }
                if ui.button("🔍").on_hover_text(l("Rileva controller sulle porte USB", "Detect controller on the USB ports")).clicked() {
                    let found = crate::connection::detect_controllers(self.machine.baud_rate.max(1) as u32);
                    if let Some((dev, fl, banner)) = found.first() {
                        self.port = dev.clone();
                        self.machine.flavor = fl.clone();
                        self.say(format!("{} {dev}: {banner}", l("Trovato", "Found")));
                    } else {
                        self.say(l("Nessun controller trovato", "No controller found"));
                    }
                }
                if self.net_rx.is_some() {
                    ui.spinner();
                } else if ui.button("📡").on_hover_text(l("Cerca laser in rete (Bonjour/mDNS)", "Find network lasers (Bonjour/mDNS)")).clicked() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(crate::connection::discover_network(std::time::Duration::from_secs(2)));
                    });
                    self.net_rx = Some(rx);
                }
            });
            if let Some(rx) = &self.net_rx {
                if let Ok(found) = rx.try_recv() {
                    self.net_rx = None;
                    if let Some((n, h, p)) = found.first() {
                        self.port = format!("tcp://{h}:{p}");
                        self.say(format!("{} {n}", l("Selezionato", "Selected")));
                    } else {
                        self.say(l("Nessun laser trovato in rete (Bonjour/mDNS)", "No network laser found (Bonjour/mDNS)"));
                    }
                    self.net_found = found;
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
            }
            ui.horizontal(|ui| {
                ui.label(l("Indirizzo", "Address"));
                ui.add(egui::TextEdit::singleline(&mut self.port).hint_text("/dev/cu.… o tcp://ip:porta").desired_width(220.0));
            });
            ui.horizontal(|ui| {
                ui.label("Baud");
                egui::ComboBox::from_id_salt("baud").selected_text(self.machine.baud_rate.to_string()).show_ui(ui, |ui| {
                    for b in [9600i64, 19200, 38400, 57600, 115200, 230400, 250000, 460800, 921600] {
                        ui.selectable_value(&mut self.machine.baud_rate, b, b.to_string());
                    }
                });
                if ui.button(l("Connetti", "Connect")).clicked() {
                    self.connect();
                }
            });
        } else {
            let (state, x, y, alarm, settings, busy, acked, total, banner) = {
                let s = self.conn.as_ref().unwrap().shared.lock().unwrap();
                (s.status.state.clone(), s.status.x, s.status.y, s.link == Some(Link::Alarm), s.settings.clone(), s.busy, s.acked, s.total, s.banner.clone())
            };
            ui.horizontal(|ui| {
                let c = if alarm { Color32::from_rgb(220, 50, 50) } else if busy { Color32::from_rgb(230, 150, 20) } else { Color32::from_rgb(40, 170, 80) };
                ui.label(RichText::new("●").color(c));
                ui.strong(if state.is_empty() { "…".to_string() } else { state.clone() });
                ui.monospace(format!("X {x:.2}  Y {y:.2}"));
                if ui.button(l("Disconnetti", "Disconnect")).clicked() {
                    if let Some(c) = self.conn.take() {
                        c.close();
                    }
                }
            });
            if !banner.is_empty() {
                ui.weak(banner);
            }
            if alarm {
                ui.colored_label(Color32::from_rgb(220, 60, 60), l("GRBL è in ALLARME: guarda la Console, poi premi Sblocca ($X) o Home", "GRBL is in ALARM: check the Console, then press Unlock ($X) or Home"));
            }
            for p in problems(&settings, self.machine.max_power) {
                ui.colored_label(Color32::from_rgb(220, 140, 0), format!("⚠ {p}"));
            }
            if total > 0 {
                let f = acked as f32 / total as f32;
                let mut text = format!("{acked} / {total} {}", l("righe", "lines"));
                if let Some(o) = &self.gcode {
                    text += &format!(" — {} {}", l("stima", "estimate"), fmt_time(o.seconds));
                }
                ui.add(egui::ProgressBar::new(f).text(text));
            }
            ui.horizontal(|ui| {
                ui.label(l("Velocità", "Feed"));
                let rt = crate::connection::grbl_family(&self.machine.flavor);
                for (t, b, d) in [("−10%", 0x92u8, -10i64), ("100%", 0x90, 0), ("+10%", 0x91, 10)] {
                    if ui.small_button(t).clicked() {
                        if rt {
                            if let Some(c) = &self.conn { c.realtime(b); }
                        } else {
                            // Marlin / Smoothie: feed-rate percentage
                            self.feed_ovr = if d == 0 { 100 } else { (self.feed_ovr + d).clamp(10, 200) };
                            let cmd = format!("M220 S{}", self.feed_ovr);
                            self.send(&cmd);
                        }
                    }
                }
                ui.label(l("Potenza", "Power"));
                for (t, b) in [("−10%", 0x9Bu8), ("100%", 0x99), ("+10%", 0x9A)] {
                    if ui.small_button(t).clicked() {
                        if rt {
                            if let Some(c) = &self.conn { c.realtime(b); }
                        } else {
                            self.say(l("Override di potenza non disponibile su questo controller", "Power override not available on this controller"));
                        }
                    }
                }
            });
        }
    }

    // ------------------------------------------------------------------ move
    fn move_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                egui::Grid::new("jog").spacing([4.0, 4.0]).show(ui, |ui| {
                    let b = |ui: &mut egui::Ui, t: &str| ui.add_sized([36.0, 32.0], egui::Button::new(t)).clicked();
                    if b(ui, "↖") { self.jog(-1.0, -1.0); }
                    if b(ui, "▲") { self.jog(0.0, -1.0); }
                    if b(ui, "↗") { self.jog(1.0, -1.0); }
                    ui.end_row();
                    if b(ui, "◀") { self.jog(-1.0, 0.0); }
                    if b(ui, "⌂") { self.send("G90 G0 X0 Y0"); }
                    if b(ui, "▶") { self.jog(1.0, 0.0); }
                    ui.end_row();
                    if b(ui, "↙") { self.jog(-1.0, 1.0); }
                    if b(ui, "▼") { self.jog(0.0, 1.0); }
                    if b(ui, "↘") { self.jog(1.0, 1.0); }
                    ui.end_row();
                });
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} (mm)", tr("step")));
                        ui.add(egui::DragValue::new(&mut self.jog_step).range(0.01..=500.0).speed(0.5));
                    });
                    ui.horizontal(|ui| {
                        for s in [0.1, 1.0, 10.0, 50.0] {
                            if ui.selectable_label(self.jog_step == s, format!("{s}")).clicked() {
                                self.jog_step = s;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(l("Velocità", "Speed"));
                        ui.add(egui::DragValue::new(&mut self.jog_speed).range(10.0..=50000.0).speed(50.0).suffix(" mm/min"));
                    });
                    if self.machine.z_enabled {
                        ui.horizontal(|ui| {
                            if ui.button("Z+").clicked() { self.jog_xyz(0.0, 0.0, 1.0); }
                            if ui.button("Z−").clicked() { self.jog_xyz(0.0, 0.0, -1.0); }
                        });
                    }
                    if self.machine.fire_enabled {
                        let r = ui.add(egui::Button::new(egui::RichText::new("Fire").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0x8b, 0x1d, 0x1d)))
                            .on_hover_text(l(&format!("Tieni premuto: laser acceso al {}% finché non rilasci", self.machine.fire_power), &format!("Hold: laser on at {}% until released", self.machine.fire_power)));
                        if r.is_pointer_button_down_on() && !self.firing {
                            self.firing = true;
                            self.fire(true);
                        }
                        if self.firing && !r.is_pointer_button_down_on() {
                            self.firing = false;
                            self.fire(false);
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Home").on_hover_text("$H").clicked() { self.run("homeLaser"); }
                if ui.button(l("Sblocca", "Unlock")).on_hover_text("$X").clicked() { self.send("$X"); }
                if ui.button(l("Azzera X/Y qui", "Zero X/Y here")).on_hover_text(l("Rende la posizione attuale della testa lo zero di lavoro (G10 L20). Con «Assolute» il lavoro parte da qui.", "Makes the head's current position the work zero (G10 L20). With “Absolute” the job is placed from here.")).clicked() {
                    self.send("G10 L20 P1 X0 Y0");
                    self.say(l("Zero di lavoro impostato qui (G10 L20)", "Work zero set here (G10 L20)"));
                }
                if ui.button(l("Cancella zero", "Clear zero")).on_hover_text(l("Riporta lo zero di lavoro sullo zero macchina (G10 L2)", "Puts the work zero back on machine zero (G10 L2)")).clicked() {
                    self.send("G10 L2 P1 X0 Y0");
                    self.say(l("Zero di lavoro cancellato", "Work zero cleared"));
                }
                if ui.button(l("Imposta origine", "Set origin")).clicked() { self.run("setUserOrigin"); }
            });
        });
        ui.separator();
        egui::Grid::new("ws").num_columns(2).show(ui, |ui| {
            let mut ch = num(ui, &tr("areaW"), &mut self.machine.workspace_x, 10.0..=5000.0, 1.0, " mm");
            ch |= num(ui, &tr("areaH"), &mut self.machine.workspace_y, 10.0..=5000.0, 1.0, " mm");
            if ch {
                self.invalidate();
            }
        });
        ui.separator();
        ui.strong(l("Posizioni salvate", "Saved positions"));
        let mut go = None;
        let mut del = None;
        for (i, (name, x, y)) in self.positions.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{name}  X {x:.2} Y {y:.2}"));
                if ui.small_button(l("Vai", "Go")).clicked() {
                    go = Some((*x, *y));
                }
                if ui.small_button("✕").clicked() {
                    del = Some(i);
                }
            });
        }
        if let Some((x, y)) = go {
            self.send(&format!("G90 G0 X{x:.3} Y{y:.3}"));
        }
        if let Some(i) = del {
            self.positions.remove(i);
        }
        if ui.button(l("Salva attuale", "Save current")).clicked() {
            match self.head() {
                Some(h) => {
                    let n = self.positions.len() + 1;
                    self.positions.push((l(&format!("Posizione {n}"), &format!("Position {n}")), h[0], h[1]));
                }
                None => self.say(l("Connetti prima il laser", "Connect a laser first")),
            }
        }
    }

    // ------------------------------------------------------------------ console
    fn console_panel(&mut self, ui: &mut egui::Ui) {
        let lines: Vec<(String, u8)> = match &self.conn {
            Some(c) => c.shared.lock().unwrap().console.iter().rev().take(500).rev().map(|l| (l.text.clone(), l.kind)).collect(),
            None => vec![(l("Non connesso", "Not connected"), 3)],
        };
        egui::ScrollArea::vertical().id_salt("console").max_height(ui.available_height() - 40.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for (t, k) in lines {
                let c = match k {
                    0 => ui.visuals().weak_text_color(),
                    2 => Color32::from_rgb(220, 60, 60),
                    3 => Color32::from_rgb(40, 140, 230),
                    _ => ui.visuals().text_color(),
                };
                ui.label(RichText::new(t).monospace().size(11.0).color(c));
            }
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.console_input).hint_text(l("Comando G-code / $…", "G-code / $ command")).desired_width(ui.available_width() - 70.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button(l("Invia", "Send")).clicked() || enter) && !self.console_input.trim().is_empty() {
                let c = std::mem::take(&mut self.console_input);
                self.send(c.trim());
                r.request_focus();
            }
            if ui.button("⌫").on_hover_text(l("Pulisci", "Clear")).clicked() {
                if let Some(c) = &self.conn {
                    c.shared.lock().unwrap().console.clear();
                }
            }
        });
    }

    // ------------------------------------------------------------------ objects
    fn objects_panel(&mut self, ui: &mut egui::Ui) {
        let mut pick = None;
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        for s in &self.doc.shapes {
            let name = match s.kind() {
                "text" => format!("\"{}\"", s.0.get("text").and_then(|v| v.as_str()).unwrap_or("")),
                "image" => s.0.get("name").and_then(|v| v.as_str()).unwrap_or("image").to_string(),
                k => k.to_string(),
            };
            let col = self.doc.layers.get(s.layer().max(0) as usize).map_or(Color32::GRAY, |l| color(&l.color));
            let text = RichText::new(format!("■  {name} #{}{}", s.id(), if s.locked() { "  🔒" } else { "" })).color(col);
            if ui.selectable_label(self.doc.is_selected(s.id()), text).clicked() {
                pick = Some(s.id());
            }
        }
        if let Some(id) = pick {
            if shift {
                let mut sel = self.doc.selected.clone();
                if let Some(k) = sel.iter().position(|x| *x == id) {
                    sel.remove(k);
                } else {
                    sel.push(id);
                }
                self.doc.set_selection(sel);
            } else {
                self.doc.set_selection(vec![id]);
            }
        }
    }

    // ------------------------------------------------------------------ properties
    fn props_panel(&mut self, ui: &mut egui::Ui) {
        let sel = self.doc.selected_shapes();
        if sel.len() != 1 {
            ui.add(egui::Label::new(egui::RichText::new(l("Seleziona una sola forma per vederne le proprietà: raggio degli angoli dei rettangoli, lati dei poligoni, testo (font, spaziatura, curvatura) e parametri degli array dinamici.",
                                                          "Select a single shape to see its properties: rectangle corner radius, polygon sides, text (font, spacing, bend) and live array parameters.")).weak()).wrap());
            if let Some(b) = multi_bounds(&sel) {
                ui.label(format!("{} · {:.2} × {:.2} mm", l(&format!("{} oggetti", sel.len()), &format!("{} objects", sel.len())), b[2], b[3]));
            }
            return;
        }
        let s = sel[0].clone();
        let mut m = s.clone();
        let mut regen_array = false;
        let mut flatten = false;
        let mut want_var = false;
        let title = match s.kind() {
            "rect" => l("Rettangolo", "Rectangle"),
            "circle" => l("Ellisse", "Ellipse"),
            "text" => l("Testo", "Text"),
            "image" => l("Immagine", "Image"),
            "group" => l("Gruppo", "Group"),
            "poly" if s.0.get("regular").is_some_and(|r| r["inner"].as_f64().unwrap_or(0.0) > 0.0) => l("Stella", "Star"),
            "poly" if s.0.contains_key("regular") => l("Poligono", "Polygon"),
            "poly" => l("Tracciato", "Path"),
            k => k.to_string(),
        };
        ui.strong(format!("{title} #{}", s.id()));
        egui::Grid::new("props").num_columns(2).spacing([10.0, 5.0]).show(ui, |ui| {
            let fnum = |ui: &mut egui::Ui, m: &mut crate::shapes::Shape, k: &str, label: &str, speed: f64, range: std::ops::RangeInclusive<f64>| {
                let mut v = m.f(k);
                ui.label(label);
                if ui.add(egui::DragValue::new(&mut v).speed(speed).range(range).max_decimals(3).suffix(" mm")).changed() {
                    m.set(k, v);
                }
                ui.end_row();
            };
            if s.0.contains_key("array") {
                let mut meta = s.0["array"].clone();
                let grid = meta["kind"] == "grid";
                ui.label("");
                ui.label(if grid { l("Array a griglia", "Grid array") } else { l("Array circolare", "Circular array") });
                ui.end_row();
                let mut f = |ui: &mut egui::Ui, k: &str, label: &str, range: std::ops::RangeInclusive<f64>, speed: f64| {
                    let mut v = meta[k].as_f64().unwrap_or(0.0);
                    ui.label(label);
                    if ui.add(egui::DragValue::new(&mut v).range(range).speed(speed)).changed() {
                        meta[k] = json!(v);
                        regen_array = true;
                    }
                    ui.end_row();
                };
                if grid {
                    f(ui, "cols", &l("Colonne", "Columns"), 1.0..=500.0, 0.1);
                    f(ui, "rows", &l("Righe", "Rows"), 1.0..=500.0, 0.1);
                    f(ui, "dx", &l("Spazio orizz.", "H spacing"), -1000.0..=1000.0, 0.5);
                    f(ui, "dy", &l("Spazio vert.", "V spacing"), -1000.0..=1000.0, 0.5);
                } else {
                    f(ui, "n", &l("Copie", "Copies"), 2.0..=720.0, 0.1);
                    f(ui, "angle", &l("Angolo totale", "Total angle"), -360.0..=360.0, 1.0);
                    let mut rot = meta["rotate"].as_bool().unwrap_or(true);
                    ui.label("");
                    if ui.checkbox(&mut rot, l("Ruota le copie", "Rotate copies")).changed() {
                        meta["rotate"] = json!(rot);
                        regen_array = true;
                    }
                    ui.end_row();
                }
                for k in ["cols", "rows", "n"] {
                    if let Some(v) = meta.get(k).and_then(|v| v.as_f64()) {
                        meta[k] = json!(v.round() as i64);
                    }
                }
                m.0.insert("array".into(), meta);
                ui.label("");
                if ui.button(l("Appiattisci array (copie indipendenti)", "Flatten array (independent copies)")).clicked() {
                    flatten = true;
                }
                ui.end_row();
            } else {
                match s.kind() {
                    "rect" => {
                        for (k, lab) in [("x", "X"), ("y", "Y"), ("w", &*tr("width")), ("h", &*tr("height"))] {
                            fnum(ui, &mut m, k, lab, 0.1, -10000.0..=10000.0);
                        }
                        fnum(ui, &mut m, "radius", &l("Raggio angoli", "Corner radius"), 0.1, 0.0..=10000.0);
                    }
                    "circle" => {
                        fnum(ui, &mut m, "cx", "X", 0.1, -10000.0..=10000.0);
                        fnum(ui, &mut m, "cy", "Y", 0.1, -10000.0..=10000.0);
                        fnum(ui, &mut m, "rx", &l("Raggio X", "Radius X"), 0.1, 0.01..=10000.0);
                        fnum(ui, &mut m, "ry", &l("Raggio Y", "Radius Y"), 0.1, 0.01..=10000.0);
                    }
                    "image" => {
                        for (k, lab) in [("x", "X"), ("y", "Y"), ("w", &*tr("width")), ("h", &*tr("height"))] {
                            fnum(ui, &mut m, k, lab, 0.1, -10000.0..=10000.0);
                        }
                    }
                    "text" => {
                        let mut t = m.0.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        ui.label(l("Testo", "Text"));
                        if ui.text_edit_singleline(&mut t).changed() {
                            m.0.insert("text".into(), json!(t));
                        }
                        ui.end_row();
                        let font = m.0.get("font").and_then(|v| v.as_str()).unwrap_or("Helvetica").to_string();
                        let single = crate::text::is_single_line(&m);
                        ui.label("Font");
                        let mut f2 = if single { "Helvetica".to_string() } else { font.clone() };
                        ui.add_enabled_ui(!single, |ui| {
                            egui::ComboBox::from_id_salt("pfont").selected_text(f2.clone()).width(200.0).height(400.0).show_ui(ui, |ui| {
                                for f in crate::text::families() {
                                    ui.selectable_value(&mut f2, f.clone(), f);
                                }
                            });
                        });
                        ui.end_row();
                        let mut sl = if single { font.clone() } else { String::new() };
                        ui.label(l("Tratto singolo", "Single-line")).on_hover_text(l("Font Hershey a tratto singolo: il laser incide una sola linea invece del contorno delle lettere (più veloce, ideale per numeri di serie e scritte piccole).",
                                                                                   "Hershey single-line font: the laser burns one stroke instead of the letter outlines (faster, ideal for serial numbers and small text)."));
                        egui::ComboBox::from_id_salt("psingle").selected_text(if sl.is_empty() { "—".to_string() } else { sl.split(':').last().unwrap_or("").to_string() }).width(200.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut sl, String::new(), "—");
                            for (fid, lab) in crate::shx::single_line_fonts() {
                                ui.selectable_value(&mut sl, fid, lab);
                            }
                        });
                        ui.end_row();
                        let nf = if sl.is_empty() { f2 } else { sl };
                        if nf != font {
                            m.0.insert("font".into(), json!(nf));
                        }
                        fnum(ui, &mut m, "size", &l("Altezza", "Height"), 0.1, 0.5..=1000.0);
                        ui.label("");
                        ui.horizontal(|ui| {
                            for (k, lab) in [("bold", l("Grassetto", "Bold")), ("italic", l("Corsivo", "Italic"))] {
                                let mut b = m.flag(k);
                                if ui.checkbox(&mut b, lab).changed() {
                                    m.0.insert(k.into(), json!(b));
                                }
                            }
                        });
                        ui.end_row();
                        fnum(ui, &mut m, "spacing", &l("Spaziatura lettere", "Letter spacing"), 0.05, -50.0..=200.0);
                        fnum(ui, &mut m, "bend", &l("Curvatura (raggio)", "Bend (radius)"), 1.0, -100000.0..=100000.0);
                        ui.label("");
                        ui.label(egui::RichText::new(l("Curvatura 0 = dritto; positivo = arco verso l'alto, negativo = verso il basso.",
                                                       "Bend 0 = straight; positive = arch upward, negative = downward.")).weak().size(11.0));
                        ui.end_row();
                        ui.label("");
                        if ui.button(l("Testo variabile...", "Variable text...")).clicked() {
                            want_var = true;
                        }
                        ui.end_row();
                    }
                    "poly" => {
                        ui.label(l("Nodi", "Nodes"));
                        ui.label(format!("{}", s.nodes().len()));
                        ui.end_row();
                        ui.label(l("Chiusa", "Closed"));
                        let mut c = s.flag("closed");
                        if ui.checkbox(&mut c, "").changed() {
                            m.0.insert("closed".into(), json!(c));
                        }
                        ui.end_row();
                        if let Some(r) = s.0.get("regular").cloned() {
                            let mut r2 = r.clone();
                            let mut n = r["n"].as_f64().unwrap_or(6.0) as i64;
                            let mut rad = r["r"].as_f64().unwrap_or(10.0);
                            let mut inner = r["inner"].as_f64().unwrap_or(0.0) * 100.0;
                            ui.label(l("Lati / punte", "Sides / points"));
                            let c1 = ui.add(egui::DragValue::new(&mut n).range(3..=200)).changed();
                            ui.end_row();
                            ui.label(l("Raggio", "Radius"));
                            let c2 = ui.add(egui::DragValue::new(&mut rad).range(0.01..=10000.0).speed(0.1).suffix(" mm")).changed();
                            ui.end_row();
                            let mut c3 = false;
                            if r.get("inner").is_some() {
                                ui.label(l("Raggio interno (stelle)", "Inner radius (stars)"));
                                c3 = ui.add(egui::DragValue::new(&mut inner).range(1.0..=99.0).suffix(" %")).changed();
                                ui.end_row();
                            }
                            if c1 || c2 || c3 {
                                r2["n"] = json!(n);
                                r2["r"] = json!(rad);
                                if r.get("inner").is_some() {
                                    r2["inner"] = json!(inner / 100.0);
                                }
                                m.0.insert("points".into(), json!(crate::edit::regular_meta_points(&r2)));
                                m.0.insert("regular".into(), r2);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ui.label(l("Priorità di taglio", "Cut priority"));
            let mut pr = m.f("priority") as i64;
            if ui.add(egui::DragValue::new(&mut pr).range(-999..=999)).on_hover_text(l("Con Ottimizzazione › Per priorità le forme con il valore più basso vengono tagliate per prime.", "With Optimization › By priority, lower values are cut first.")).changed() {
                m.0.insert("priority".into(), json!(pr));
            }
            ui.end_row();
        });
        if s.kind() == "image" && ui.button(l("Regola immagine…", "Adjust image…")).clicked() {
            self.dialogs.push(Dialog::adjust_image(&s));
        }
        if want_var {
            self.run("varText");
        }
        if flatten {
            self.doc.save_undo("");
            if let Some(x) = self.doc.find_mut(s.id()) {
                x.0.remove("array");
            }
            self.invalidate();
            return;
        }
        if m != s {
            if self.doc.undo.last().is_none_or(|u| u.1 != "props") {
                self.doc.save_undo("props");
            }
            if let Some(x) = self.doc.find_mut(s.id()) {
                *x = m;
            }
            if regen_array {
                crate::dialogs::regenerate_array(&mut self.doc, s.id());
            }
            self.invalidate();
        }
    }

    // ------------------------------------------------------------------ pages
    fn pages_panel(&mut self, ui: &mut egui::Ui) {
        let mut goto = None;
        let mut del = None;
        for i in 0..self.doc.pages.len() {
            ui.horizontal(|ui| {
                let mut name = self.doc.page_name(i);
                if ui.selectable_label(i == self.doc.page, format!("{}.", i + 1)).clicked() {
                    goto = Some(i);
                }
                if ui.text_edit_singleline(&mut name).changed() {
                    self.doc.rename_page(i, &name);
                }
                if self.doc.pages.len() > 1 && ui.small_button("✕").clicked() {
                    del = Some(i);
                }
            });
        }
        if let Some(i) = goto {
            self.doc.set_page(i);
            self.invalidate();
        }
        if let Some(i) = del {
            self.doc.remove_page(i);
            self.invalidate();
        }
        if ui.button(l("Nuova pagina", "New page")).clicked() {
            self.doc.add_page(None);
            self.invalidate();
        }
        ui.add(egui::Label::new(RichText::new(l("Ogni pagina è un lavoro separato nello stesso progetto: viene inviata al laser solo la pagina aperta.",
                                                 "Each page is a separate job in the same project: only the open page is sent to the laser.")).weak().size(11.0)).wrap());
    }

    // ------------------------------------------------------------------ macros
    fn macros_panel(&mut self, ui: &mut egui::Ui) {
        let mut run = None;
        let mut del = None;
        for (i, (name, code)) in self.macros.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(name.as_str()).clicked() {
                    run = Some(code.clone());
                }
                ui.text_edit_singleline(name);
                if ui.small_button("✕").clicked() {
                    del = Some(i);
                }
            });
            ui.add(egui::TextEdit::multiline(code).desired_rows(2).code_editor());
        }
        if let Some(c) = run {
            for line in c.lines().filter(|l| !l.trim().is_empty()) {
                self.send(line.trim());
            }
        }
        if let Some(i) = del {
            self.macros.remove(i);
        }
        if ui.button(l("Nuova macro", "New macro")).clicked() {
            let n = self.macros.len() + 1;
            self.macros.push((format!("Macro {n}"), "$H".into()));
        }
        ui.add(egui::Label::new(RichText::new(l("Clic su un pulsante per inviare il suo G-code al laser. Puoi usare più righe.",
                                                 "Click a button to send its G-code to the laser. Multiple lines are allowed.")).weak().size(11.0)).wrap());
    }
}
