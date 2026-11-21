//! Material library panel: layer settings saved by material, thickness and description, in the same JSON file
//! as the Python app (shared between the two), with apply / new / update / rename / delete / import / export.
//! Ported from laserforge/library.py.

use crate::app::LaserApp;
use crate::l;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const FIELDS: [&str; 33] = ["speed", "power_min", "power_max", "passes", "cut_mode", "air_assist", "air_power", "air_cut_only", "lead_in", "lead_out",
    "overcut", "perforate", "perf_cut", "perf_skip", "flood_fill", "sublayers", "z_offset", "laser_mode", "line_interval", "overscan", "scan_angle",
    "cross_hatch", "bidir_scan", "dwell_time", "kerf_offset", "power_scale", "dot_mode", "dot_time", "ramp_length", "tab_count", "tab_width",
    "start_delay", "z_step"];

pub fn lib_path() -> PathBuf {
    if let Some(p) = std::env::var_os("LASERFORGE_MATERIALS") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Application Support/LaserForge/materials.json")
}

fn seed() -> Vec<Value> {
    let presets: Map<String, Value> = serde_json::from_str(include_str!("../assets/material_presets.json")).unwrap_or_default();
    presets.into_iter().map(|(name, p)| {
        let parts: Vec<&str> = name.rsplitn(2, ' ').collect();
        let (mat, thick) = if parts.len() == 2 && parts[0].ends_with("mm") { (parts[1].to_string(), parts[0].to_string()) } else { (name.clone(), String::new()) };
        let desc = if p.get("cut_mode").and_then(|v| v.as_str()) == Some("line") { "Taglio" } else { "Incisione" };
        json!({"material": mat, "thickness": thick, "name": desc, "settings": p})
    }).collect()
}

pub fn load_library() -> Vec<Value> {
    match std::fs::read_to_string(lib_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(v) => {
            let list = if v.is_object() { v.get("materials").cloned().unwrap_or(json!([])) } else { v };
            list.as_array().map(|a| a.iter().filter(|e| e.get("settings").is_some()).cloned().collect()).unwrap_or_default()
        }
        None => seed(),
    }
}

fn save_library(entries: &[Value]) {
    let p = lib_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(&json!({"app": "LaserForge", "version": 1, "materials": entries})).unwrap()).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

#[derive(Default)]
pub struct LibState {
    pub entries: Option<Vec<Value>>,
    pub search: String,
    pub sel: Option<usize>,
    pub meta: [String; 3],
}

impl LaserApp {
    fn layer_material(&self) -> Value {
        let d = serde_json::to_value(&self.doc.layers[self.doc.active_layer]).unwrap();
        Value::Object(FIELDS.iter().filter_map(|k| d.get(*k).map(|v| (k.to_string(), v.clone()))).collect())
    }

    pub fn library_panel(&mut self, ui: &mut egui::Ui) {
        if self.library.entries.is_none() {
            self.library.entries = Some(load_library());
        }
        ui.add(egui::TextEdit::singleline(&mut self.library.search).hint_text(l("Cerca materiale…", "Search material…")).desired_width(f32::INFINITY));
        let q = self.library.search.trim().to_lowercase();
        let entries = self.library.entries.clone().unwrap_or_default();
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let key = format!("{} {}", e["material"].as_str().unwrap_or("?"), e["thickness"].as_str().unwrap_or("")).trim().to_string();
            if !q.is_empty() && !key.to_lowercase().contains(&q) && !e["name"].as_str().unwrap_or("").to_lowercase().contains(&q) {
                continue;
            }
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(i),
                None => groups.push((key, vec![i])),
            }
        }
        let mut apply = None;
        egui::ScrollArea::vertical().id_salt("lib").max_height(320.0).show(ui, |ui| {
            for (key, idx) in &groups {
                ui.strong(key);
                for &i in idx {
                    let e = &entries[i];
                    let s = &e["settings"];
                    let label = format!("   {}   ·   {} mm/min · {}% · {}", e["name"].as_str().unwrap_or("—"), s["speed"].as_f64().unwrap_or(0.0).round(),
                                        s["power_max"].as_f64().unwrap_or(0.0).round(), s["air_assist"].as_str().unwrap_or("off").to_uppercase());
                    let r = ui.selectable_label(self.library.sel == Some(i), label);
                    if r.clicked() {
                        self.library.sel = Some(i);
                        self.library.meta = [e["material"].as_str().unwrap_or("").into(), e["thickness"].as_str().unwrap_or("").into(), e["name"].as_str().unwrap_or("").into()];
                    }
                    if r.double_clicked() {
                        apply = Some(i);
                    }
                }
            }
        });
        egui::Grid::new("libmeta").num_columns(2).show(ui, |ui| {
            for (k, (it, en)) in [("Materiale", "Material"), ("Spessore", "Thickness"), ("Descrizione", "Description")].iter().enumerate() {
                ui.label(l(it, en));
                ui.text_edit_singleline(&mut self.library.meta[k]);
                ui.end_row();
            }
        });
        let mut list = entries;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button(egui::RichText::new(l("Applica al layer", "Apply to layer")).strong()).clicked() {
                apply = self.library.sel;
                if apply.is_none() {
                    self.say(l("Scegli un materiale", "Pick a material"));
                }
            }
            if ui.button(l("Nuovo dal layer", "New from layer")).clicked() && !self.library.meta[0].trim().is_empty() {
                let m = &self.library.meta;
                list.push(json!({"material": m[0].trim(), "thickness": m[1].trim(), "name": m[2].trim(), "settings": self.layer_material()}));
                changed = true;
            }
            if let Some(i) = self.library.sel.filter(|i| *i < list.len()) {
                if ui.button(l("Aggiorna dal layer", "Update from layer")).clicked() {
                    list[i]["settings"] = self.layer_material();
                    changed = true;
                    self.say(l("Materiale aggiornato", "Material updated"));
                }
                if ui.button(l("Rinomina", "Rename")).clicked() {
                    let m = self.library.meta.clone();
                    list[i]["material"] = json!(m[0].trim());
                    list[i]["thickness"] = json!(m[1].trim());
                    list[i]["name"] = json!(m[2].trim());
                    changed = true;
                }
                if ui.button(l("Elimina", "Delete")).clicked() {
                    list.remove(i);
                    self.library.sel = None;
                    changed = true;
                }
            }
            if ui.button(l("Importa/unisci…", "Import/merge…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json", "lflib"]).pick_file() {
                    match std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                        Some(data) => {
                            let incoming = if data.is_object() { data.get("materials").cloned().unwrap_or(json!([])) } else { data };
                            let key = |e: &Value| (e["material"].clone(), e["thickness"].clone(), e["name"].clone());
                            let (mut added, mut replaced) = (0, 0);
                            for e in incoming.as_array().cloned().unwrap_or_default().into_iter().filter(|e| e.get("settings").is_some()) {
                                match list.iter().position(|x| key(x) == key(&e)) {
                                    Some(k) => { list[k] = e; replaced += 1; }
                                    None => { list.push(e); added += 1; }
                                }
                            }
                            changed = true;
                            self.say(l(&format!("{added} aggiunti, {replaced} aggiornati"), &format!("{added} added, {replaced} updated")));
                        }
                        None => self.say(l("File non valido", "Invalid file")),
                    }
                }
            }
            if ui.button(l("Esporta…", "Export…")).clicked() {
                let t = serde_json::to_string_pretty(&json!({"app": "LaserForge", "version": 1, "materials": list})).unwrap();
                self.save_text_file(&t, "materiali-laserforge.json", ("JSON", &["json"]));
            }
        });
        if let Some(i) = apply.filter(|i| *i < list.len()) {
            let e = list[i].clone();
            let idx = self.doc.active_layer;
            if let Some(m) = e["settings"].as_object() {
                crate::project::apply_layer_dict(&mut self.doc.layers[idx], m);
            }
            self.invalidate();
            self.say(format!("{} {} · {} → {}", e["material"].as_str().unwrap_or(""), e["thickness"].as_str().unwrap_or(""), e["name"].as_str().unwrap_or(""), self.doc.layers[idx].name));
        }
        if changed {
            save_library(&list);
        }
        self.library.entries = Some(list);
        ui.add(egui::Label::new(egui::RichText::new(format!("{}{}", l("Doppio clic su una voce per applicarla al layer attivo. La libreria si salva in ", "Double-click an entry to apply it to the active layer. The library is saved in "), lib_path().display())).weak().size(11.0)).wrap());
    }
}

// ====================================================================== art library
/// Folder with saved artwork (same as the Python app, shared).
pub fn art_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("LASERFORGE_ART_DIR") {
        return PathBuf::from(p);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Application Support/LaserForge/ArtLibrary")
}

#[derive(Default)]
pub struct ArtState {
    pub items: Option<Vec<(PathBuf, String, Vec<crate::shapes::Shape>)>>,
    pub sel: Vec<usize>,
    pub name: String,
}

impl LaserApp {
    fn art_reload(&mut self) {
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(art_dir()).map(|r| r.flatten().map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json")).map(|p| { let t = p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH); (p, t) }).collect()).unwrap_or_default();
        files.sort_by(|a, b| b.1.cmp(&a.1));
        self.art.items = Some(files.into_iter().filter_map(|(p, _)| {
            let d: Value = serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            let name = d["name"].as_str().map(|s| s.to_string()).unwrap_or_else(|| p.file_stem().unwrap().to_string_lossy().to_string());
            let shapes = serde_json::from_value(d["shapes"].clone()).unwrap_or_default();
            Some((p, name, shapes))
        }).collect());
        self.art.sel.clear();
    }

    fn art_save(&mut self, name: &str, shapes: &[crate::shapes::Shape]) {
        let dir = art_dir();
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' || c == '_' { c } else { '_' }).collect();
        let safe = if safe.trim().is_empty() { "art".to_string() } else { safe.trim().to_string() };
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let _ = std::fs::write(dir.join(format!("{safe}-{ms}.json")), serde_json::to_string(&json!({"name": name, "shapes": shapes})).unwrap());
    }

    fn art_insert(&mut self, i: usize) {
        let Some((_, _, shapes)) = self.art.items.as_ref().and_then(|v| v.get(i)).cloned() else { return };
        let mut shapes: Vec<crate::shapes::Shape> = shapes.into_iter().map(|s| self.doc.reid(s)).collect();
        let Some(b) = crate::shapes::multi_bounds(&shapes) else { return };
        let (w, h) = self.ws();
        for s in shapes.iter_mut() {
            s.translate(w / 2.0 - b[0] - b[2] / 2.0, h / 2.0 - b[1] - b[3] / 2.0);
        }
        self.doc.add_shapes(shapes, true);
        self.invalidate();
    }

    pub fn art_panel(&mut self, ui: &mut egui::Ui) {
        if self.art.items.is_none() {
            self.art_reload();
        }
        let items = self.art.items.clone().unwrap_or_default();
        let layers = self.doc.layers.clone();
        let mut insert = None;
        egui::ScrollArea::vertical().id_salt("art").max_height(360.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, (_, name, shapes)) in items.iter().enumerate() {
                    ui.vertical(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::click());
                        let p = ui.painter_at(rect);
                        p.rect_filled(rect, 4.0, egui::Color32::WHITE);
                        if self.art.sel.contains(&i) {
                            p.rect_stroke(rect, 4.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(0x2f, 0x7b, 0xf5)), egui::StrokeKind::Inside);
                        }
                        if let Some(b) = crate::shapes::multi_bounds(shapes) {
                            let k = (80.0 / b[2].max(1e-6)).min(80.0 / b[3].max(1e-6));
                            let o = egui::vec2(((96.0 - b[2] * k) / 2.0 - b[0] * k) as f32, ((96.0 - b[3] * k) / 2.0 - b[1] * k) as f32);
                            for s in crate::shapes::flatten(shapes) {
                                let col = layers.get(s.layer().max(0) as usize).map_or(egui::Color32::BLACK, |l| crate::app::color(&l.color));
                                for path in s.paths() {
                                    let pts: Vec<egui::Pos2> = path.iter().map(|q| rect.min + o + egui::vec2((q[0] * k) as f32, (q[1] * k) as f32)).collect();
                                    p.add(egui::Shape::line(pts, egui::Stroke::new(1.2, col)));
                                }
                            }
                        }
                        if resp.clicked() {
                            if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
                                if let Some(k) = self.art.sel.iter().position(|x| *x == i) { self.art.sel.remove(k); } else { self.art.sel.push(i); }
                            } else {
                                self.art.sel = vec![i];
                            }
                        }
                        if resp.double_clicked() {
                            insert = Some(i);
                        }
                        ui.add_sized([96.0, 14.0], egui::Label::new(egui::RichText::new(name).size(10.0)).truncate());
                    });
                }
            });
        });
        if let Some(i) = insert {
            self.art_insert(i);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.art.name).hint_text(l("Nome", "Name")).desired_width(120.0));
            if ui.button(l("Aggiungi selezione", "Add selection")).clicked() {
                let sel = self.doc.selected_shapes();
                if sel.is_empty() {
                    self.say(crate::tr("selectFirst"));
                } else {
                    let name = if self.art.name.trim().is_empty() { sel[0].name().is_empty().then(|| sel[0].kind().to_string()).unwrap_or(sel[0].name()) } else { self.art.name.trim().to_string() };
                    self.art_save(&name, &sel);
                    self.art_reload();
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(l("Inserisci", "Insert")).clicked() {
                for i in self.art.sel.clone() {
                    self.art_insert(i);
                }
            }
            if ui.button(l("Importa file...", "Import files...")).clicked() {
                if let Some(paths) = rfd::FileDialog::new().add_filter("SVG / LaserForge", &["svg", "lfproj"]).pick_files() {
                    for p in paths {
                        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                        let shapes: Vec<crate::shapes::Shape> = if p.extension().is_some_and(|e| e == "lfproj") {
                            std::fs::read_to_string(&p).ok().and_then(|t| crate::project::Doc::from_project(&t).ok()).map(|d| d.0.shapes).unwrap_or_default()
                        } else {
                            std::fs::read_to_string(&p).ok().and_then(|t| crate::importers::svg_shapes(&t).ok()).map(|v| v.into_iter().map(|pr| self.doc.create_from(pr)).collect()).unwrap_or_default()
                        };
                        if !shapes.is_empty() {
                            self.art_save(&stem, &shapes);
                        }
                    }
                    self.art_reload();
                }
            }
            if ui.button(l("Elimina", "Delete")).clicked() {
                for i in self.art.sel.clone() {
                    if let Some((p, _, _)) = items.get(i) {
                        let _ = std::fs::remove_file(p);
                    }
                }
                self.art_reload();
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(format!("{}{}", l("Doppio clic per inserire un elemento al centro dell'area di lavoro. Gli elementi sono salvati in ", "Double-click to insert an item at the centre of the work area. Items are stored in "), art_dir().display())).weak().size(11.0)).wrap());
    }
}
