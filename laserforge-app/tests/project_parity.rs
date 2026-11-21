//! Block 1: whole projects produced by the Python window (fixtures made by the Python app of v0.4.0-beta) must give the same job and
//! G-code in Rust: exactly for vectors, within a tolerance for images (Pillow and the image crate resample differently).

use laserforge_app::project::{process_image, Doc, JobOpts, Opt};
use laserforge_core::model::MachineSettings;
use serde_json::Value;
use std::path::Path;

fn first_diff(got: &str, want: &str) -> Option<String> {
    if got == want {
        return None;
    }
    let (g, w): (Vec<&str>, Vec<&str>) = (got.split('\n').collect(), want.split('\n').collect());
    let i = (0..g.len().max(w.len())).find(|&i| g.get(i) != w.get(i)).unwrap_or(0);
    Some(format!("line {}: rust {:?} python {:?} (rust {} lines, python {})", i + 1, g.get(i), w.get(i), g.len(), w.len()))
}

/// Burned length of a G-code program (G1 moves with S > 0), absolute or relative.
fn burned(g: &str) -> f64 {
    let (mut x, mut y, mut rel, mut s, mut g1, mut len) = (0.0f64, 0.0f64, false, 0.0f64, false, 0.0);
    for raw in g.lines() {
        let ln = raw.split(';').next().unwrap().trim().to_uppercase();
        if ln.contains("G91") { rel = true; }
        if ln.contains("G90") { rel = false; }
        let (mut gx, mut gy) = (None, None);
        for w in ln.split_whitespace() {
            let (k, v) = w.split_at(1);
            let Ok(v) = v.parse::<f64>() else { continue };
            match k { "G" if v == 0.0 => g1 = false, "G" if v == 1.0 => g1 = true, "S" => s = v, "X" => gx = Some(v), "Y" => gy = Some(v), _ => {} }
        }
        if ln.starts_with("M5") { s = 0.0; }
        if gx.is_none() && gy.is_none() { continue; }
        let nx = if rel { x + gx.unwrap_or(0.0) } else { gx.unwrap_or(x) };
        let ny = if rel { y + gy.unwrap_or(0.0) } else { gy.unwrap_or(y) };
        if g1 && s > 0.0 { len += (nx - x).hypot(ny - y); }
        x = nx;
        y = ny;
    }
    len
}

fn pair(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0)]
}

#[test]
fn projects_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures3");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let (mut ok, mut fails) = (0, Vec::new());
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let (doc, ws) = Doc::from_project(fx["project"].as_str().unwrap()).unwrap();
        let win = &fx["window"];
        let mut m = MachineSettings::default();
        if let Some((x, y)) = ws {
            m.workspace_x = x;
            m.workspace_y = y;
        }
        m.origin = win["machine"]["origin"].as_str().unwrap().into();
        let ids = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect::<Vec<i64>>());
        let jo = JobOpts {
            opt: serde_json::from_value::<Opt>(win["opt"].clone()).unwrap(),
            only: None,
            origin_ids: ids(&win["origin_ids"]),
            start_from: win["start_from"].as_str().unwrap().into(),
            anchor: win["anchor"].as_u64().unwrap() as usize,
            head: pair(&win["head"]),
            user_origin: pair(&win["user_origin"]),
            cylinder: Some(win["cylinder"].clone()).filter(|v| !v.is_null()),
            print_cut: Some(win["print_cut"].clone()).filter(|v| !v.is_null()),
        };
        let (job, _rep) = doc.build_job(&m, &jo, &mut |s, iv, pt| process_image(s, iv, pt));
        let mut errs = Vec::new();
        if job.origin_mode.as_deref() != fx["origin_mode"].as_str() {
            errs.push(format!("origin_mode rust {:?} python {:?}", job.origin_mode, fx["origin_mode"]));
        }
        let want_r = fx["rasters"].as_array().unwrap();
        let out = if job.layers.is_empty() && job.rasters.is_empty() { None } else { Some(laserforge_core::generate(&job)) };
        let (gcode, send) = out.map(|o| (o.gcode, o.send)).unwrap_or_default();
        if name.starts_with("offset_fill") {
            // shapely (GEOS) and geo start the offset rings at different vertices: same rings, same length
            let (gl, wl) = (gcode.lines().count(), fx["gcode"].as_str().unwrap().lines().count());
            let (lr, lp) = (burned(&gcode), burned(fx["gcode"].as_str().unwrap()));
            if gl != wl || (lr - lp).abs() > 0.005 * lp {
                errs.push(format!("offset fill: {gl} vs {wl} lines, burned {lr:.2} vs {lp:.2} mm"));
            }
        } else if want_r.is_empty() {
            errs.extend(first_diff(&gcode, fx["gcode"].as_str().unwrap()).map(|e| format!("gcode {e}")));
            errs.extend(first_diff(&send, fx["send"].as_str().unwrap()).map(|e| format!("send {e}")));
        } else {
            if job.rasters.len() != want_r.len() {
                errs.push(format!("rasters rust {} python {}", job.rasters.len(), want_r.len()));
            }
            for (r, w) in job.rasters.iter().zip(want_r) {
                let r = &r.raster;
                let dims = (r.cols as u64, r.rows as u64);
                if dims != (w["cols"].as_u64().unwrap(), w["rows"].as_u64().unwrap()) {
                    errs.push(format!("raster size rust {dims:?} python {}x{}", w["cols"], w["rows"]));
                    continue;
                }
                for (k, v) in [("x", r.x), ("y", r.y), ("w", r.w), ("h", r.h)] {
                    if (v - w[k].as_f64().unwrap()).abs() > 1e-9 {
                        errs.push(format!("raster {k} rust {v} python {}", w[k]));
                    }
                }
                let wd: Vec<u8> = w["data"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u8).collect();
                let diff = r.data.iter().zip(&wd).filter(|(a, b)| a != b).count();
                if diff as f64 > 0.06 * wd.len() as f64 {
                    errs.push(format!("raster pixels differ: {diff}/{}", wd.len()));
                }
            }
            let (gl, wl) = (gcode.lines().count() as f64, fx["gcode"].as_str().unwrap().lines().count() as f64);
            if (gl - wl).abs() > 0.1 * wl {
                errs.push(format!("gcode lines rust {gl} python {wl}"));
            }
        }
        if errs.is_empty() {
            ok += 1;
        } else {
            fails.push(format!("{name}: {}", errs.join("; ")));
        }
    }
    eprintln!("projects: {ok}/{} match Python", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}

#[test]
fn project_round_trip_keeps_everything() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures3");
    let text = std::fs::read_to_string(dir.join("basic.json")).unwrap();
    let fx: Value = serde_json::from_str(&text).unwrap();
    let src: Value = serde_json::from_str(fx["project"].as_str().unwrap()).unwrap();
    let (mut doc, ws) = Doc::from_project(fx["project"].as_str().unwrap()).unwrap();
    let (w, h) = ws.unwrap();
    let back: Value = serde_json::from_str(&doc.to_project(w, h)).unwrap();
    for k in ["shapes", "pages", "nextId", "notes", "workspace", "version"] {
        assert_eq!(back[k], src[k], "{k} changed in the round trip");
    }
    let (la, lb) = (src["layers"].as_array().unwrap(), back["layers"].as_array().unwrap());
    assert_eq!(la.len(), lb.len());
    for (a, b) in la.iter().zip(lb) {
        for (k, v) in a.as_object().unwrap() {
            assert_eq!(&b[k], v, "layer key {k}");
        }
    }
}

#[test]
#[ignore]
fn debug_layers() {
    let name = std::env::var("FX").unwrap_or("basic".into());
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures3/{name}.json"));
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let (doc, ws) = Doc::from_project(fx["project"].as_str().unwrap()).unwrap();
    let win = &fx["window"];
    let mut m = MachineSettings::default();
    m.workspace_x = ws.unwrap().0;
    m.workspace_y = ws.unwrap().1;
    m.origin = win["machine"]["origin"].as_str().unwrap().into();
    let jo = JobOpts { opt: serde_json::from_value(win["opt"].clone()).unwrap(), start_from: win["start_from"].as_str().unwrap().into(),
        anchor: win["anchor"].as_u64().unwrap() as usize, head: pair(&win["head"]), user_origin: pair(&win["user_origin"]),
        origin_ids: win["origin_ids"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()),
        cylinder: Some(win["cylinder"].clone()).filter(|v| !v.is_null()), print_cut: Some(win["print_cut"].clone()).filter(|v| !v.is_null()), only: None };
    let (job, _) = doc.build_job(&m, &jo, &mut |s, iv, pt| process_image(s, iv, pt));
    let want = fx["layers"].as_array().unwrap();
    eprintln!("layers rust {} python {}", job.layers.len(), want.len());
    for (k, (r, w)) in job.layers.iter().zip(want).enumerate() {
        let wp = w["paths"].as_array().unwrap();
        eprintln!("layer {k}: rust {} paths python {} paths (python index {})", r.paths.len(), wp.len(), w["layer_index"]);
        for (i, (a, b)) in r.paths.iter().zip(wp).enumerate() {
            let bv: Vec<[f64; 2]> = serde_json::from_value(b.clone()).unwrap();
            let same = a.len() == bv.len() && a.iter().zip(&bv).all(|(p, q)| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6);
            if !same {
                eprintln!("  path {i}: rust {} pts {:?}..  python {} pts {:?}..", a.len(), &a[..a.len().min(3)], bv.len(), &bv[..bv.len().min(3)]);
            }
        }
    }
}

#[test]
#[ignore]
fn debug_generator_on_python_job() {
    let name = std::env::var("FX").unwrap_or("basic".into());
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures3/{name}.json"));
    let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let (doc, ws) = Doc::from_project(fx["project"].as_str().unwrap()).unwrap();
    let mut m = MachineSettings::default();
    m.workspace_x = ws.unwrap().0;
    m.workspace_y = ws.unwrap().1;
    m.origin = fx["window"]["machine"]["origin"].as_str().unwrap().into();
    let layers = fx["layers"].as_array().unwrap().iter().map(|l| laserforge_core::JobLayer {
        settings: doc.layers[l["layer_index"].as_u64().unwrap() as usize].clone(),
        paths: serde_json::from_value(l["paths"].clone()).unwrap(),
        tabs: serde_json::from_value(l["tabs"].clone()).unwrap(),
    }).collect();
    let job = laserforge_core::Job { machine: m, layers, rasters: vec![], origin_mode: None, start: None };
    let out = laserforge_core::generate(&job);
    eprintln!("{:?}", first_diff(&out.gcode, fx["gcode"].as_str().unwrap()));
    for l in out.gcode.lines().filter(|l| l.contains("ring") || l.contains("Fill")) { eprintln!("rust: {l}"); }
}
