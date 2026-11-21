//! Preference bundle (.lfbundle, zip): every preference, the material library and the art library, in the same
//! layout as the Python app so a bundle moves between the two. Ported from laserforge/prefs.py.

use crate::app::LaserApp;
use crate::l;
use serde_json::{json, Value};
use std::io::{Read, Write};

impl LaserApp {
    pub fn export_bundle(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("LaserForge bundle", &["lfbundle"]).set_file_name("laserforge.lfbundle").save_file() else { return };
        let s = |v: Value| json!(v.to_string());
        let values = json!({
            "prefs": s(json!(self.prefs)), "opt": s(json!(self.opt)), "machine": s(json!(self.machine)), "layers": s(json!(self.doc.layers)),
            "devices": s(json!(self.devices)),
            "macros": s(json!(self.macros.iter().map(|(n, g)| json!({"name": n, "gcode": g})).collect::<Vec<_>>())),
            "positions": s(json!(self.positions.iter().map(|(n, x, y)| json!({"name": n, "x": x, "y": y})).collect::<Vec<_>>())),
            "shortcuts": s(json!(self.shortcut_overrides)), "checklist": s(json!(self.checklist)), "cylinder": s(self.cylinder.clone()),
            "lang": if crate::defs::english() { "en" } else { "it" }, "startFrom": self.start_from, "origin": self.job_origin.to_string(),
            "userOrigin": s(json!(self.user_origin)),
        });
        let res = (|| -> zip::result::ZipResult<()> {
            let f = std::fs::File::create(&path)?;
            let mut z = zip::ZipWriter::new(f);
            let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("settings.json", opt)?;
            z.write_all(serde_json::to_string_pretty(&json!({"app": "LaserForge", "version": 1, "values": values})).unwrap().as_bytes())?;
            if let Ok(b) = std::fs::read(crate::library::lib_path()) {
                z.start_file("materials.json", opt)?;
                z.write_all(&b)?;
            }
            if let Ok(rd) = std::fs::read_dir(crate::library::art_dir()) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "json") {
                        if let Ok(b) = std::fs::read(&p) {
                            z.start_file(format!("art/{}", p.file_name().unwrap().to_string_lossy()), opt)?;
                            z.write_all(&b)?;
                        }
                    }
                }
            }
            z.finish()?;
            Ok(())
        })();
        match res {
            Ok(_) => self.say(path.file_name().unwrap().to_string_lossy().to_string()),
            Err(e) => self.say(format!("Errore: {e}")),
        }
    }

    pub fn import_bundle(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("LaserForge bundle", &["lfbundle", "zip"]).pick_file() else { return };
        let res = (|| -> Result<(), String> {
            let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
            let mut text = String::new();
            z.by_name("settings.json").map_err(|e| e.to_string())?.read_to_string(&mut text).map_err(|e| e.to_string())?;
            let values = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?["values"].clone();
            let get = |k: &str| values.get(k).and_then(|v| v.as_str()).and_then(|t| serde_json::from_str::<Value>(t).ok());
            if let Some(v) = get("prefs") {
                let mut base = json!(self.prefs);
                if let (Value::Object(b), Value::Object(n)) = (&mut base, v) {
                    b.extend(n);
                }
                if let Ok(p) = serde_json::from_value(base) {
                    self.prefs = p;
                }
            }
            if let Some(v) = get("machine") {
                let mut base = json!(self.machine);
                if let (Value::Object(b), Value::Object(n)) = (&mut base, v) {
                    for (k, x) in n {
                        if b.contains_key(&k) && !x.is_null() {
                            b.insert(k, x);
                        }
                    }
                }
                if let Ok(m) = serde_json::from_value(base) {
                    self.machine = m;
                }
            }
            if let Some(Value::Array(ls)) = get("layers") {
                for (i, lv) in ls.iter().enumerate().take(self.doc.layers.len()) {
                    if let Some(m) = lv.as_object() {
                        crate::project::apply_layer_dict(&mut self.doc.layers[i], m);
                    }
                }
            }
            if let Some(v) = get("opt") {
                let mut base = json!(self.opt);
                if let (Value::Object(b), Value::Object(n)) = (&mut base, v) {
                    b.extend(n);
                }
                if let Ok(o) = serde_json::from_value(base) {
                    self.opt = o;
                }
            }
            if let Some(Value::Array(d)) = get("devices") {
                self.devices = d;
            }
            if let Some(Value::Array(m)) = get("macros") {
                self.macros = m.iter().map(|x| (x["name"].as_str().unwrap_or("").to_string(), x["gcode"].as_str().unwrap_or("").to_string())).collect();
            }
            if let Some(Value::Array(p)) = get("positions") {
                self.positions = p.iter().map(|x| (x["name"].as_str().unwrap_or("").to_string(), x["x"].as_f64().unwrap_or(0.0), x["y"].as_f64().unwrap_or(0.0))).collect();
            }
            if let Some(Value::Object(s)) = get("shortcuts") {
                self.shortcut_overrides = s.into_iter().filter_map(|(k, v)| v.as_str().map(|x| (k, x.to_string()))).collect();
            }
            if let Some(Value::Array(c)) = get("checklist") {
                self.checklist = c.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
            }
            if let Some(c) = get("cylinder") {
                self.cylinder = c;
            }
            if let Ok(mut f) = z.by_name("materials.json") {
                let mut b = Vec::new();
                f.read_to_end(&mut b).map_err(|e| e.to_string())?;
                let p = crate::library::lib_path();
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                std::fs::write(p, b).map_err(|e| e.to_string())?;
            }
            let dir = crate::library::art_dir();
            let _ = std::fs::create_dir_all(&dir);
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(|e| e.to_string())?;
                let n = f.name().to_string();
                if let Some(rest) = n.strip_prefix("art/") {
                    if rest.ends_with(".json") && !rest.contains('/') && !rest.contains("..") {
                        let mut b = Vec::new();
                        f.read_to_end(&mut b).map_err(|e| e.to_string())?;
                        std::fs::write(dir.join(rest), b).map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(())
        })();
        match res {
            Ok(_) => {
                self.library = Default::default();
                self.art = Default::default();
                self.snap_on = self.prefs.grid_snap;
                self.invalidate();
                self.say(l("Pacchetto importato", "Bundle imported"));
            }
            Err(e) => self.say(format!("{}: {e}", l("Pacchetto non valido", "Invalid bundle"))),
        }
    }
}
