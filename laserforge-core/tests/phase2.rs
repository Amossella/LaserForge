//! Phase-2 checks against the Python engine (fixtures made by the Python app of v0.4.0-beta): dithering and adjustments
//! exactly, rotation pixel by pixel within a tiny margin, geometry and whole jobs on area / burned paths.

use laserforge_core::{geometry, imaging, model::Pt, Job};
use serde_json::Value;
use std::path::Path;

fn pts(v: &Value) -> Vec<Vec<Pt>> {
    v.as_array().unwrap().iter().map(|p| p.as_array().unwrap().iter()
        .map(|q| Pt { x: q[0].as_f64().unwrap(), y: q[1].as_f64().unwrap() }).collect()).collect()
}

/// Burned segments of a G-code program: total length and bounding box.
fn burned(g: &str) -> (f64, [f64; 4]) {
    let (mut x, mut y, mut rel, mut s, mut g1) = (0.0f64, 0.0f64, false, 0.0f64, false);
    let mut len = 0.0;
    let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for raw in g.lines() {
        let ln = raw.split(';').next().unwrap().trim().to_uppercase();
        if ln.is_empty() { continue; }
        if ln.contains("G91") { rel = true; }
        if ln.contains("G90") { rel = false; }
        let mut gx = None; let mut gy = None;
        for w in ln.split_whitespace() {
            let (k, v) = w.split_at(1);
            let v: f64 = match v.parse() { Ok(v) => v, Err(_) => continue };
            match k { "G" => { if v == 0.0 { g1 = false } else if v == 1.0 { g1 = true } }, "S" => s = v, "X" => gx = Some(v), "Y" => gy = Some(v), _ => {} }
        }
        if ln.starts_with("M5") { s = 0.0; }
        if gx.is_none() && gy.is_none() { continue; }
        let nx = if rel { x + gx.unwrap_or(0.0) } else { gx.unwrap_or(x) };
        let ny = if rel { y + gy.unwrap_or(0.0) } else { gy.unwrap_or(y) };
        if g1 && s > 0.0 {
            len += (nx - x).hypot(ny - y);
            for (px, py) in [(x, y), (nx, ny)] { bb = [bb[0].min(px), bb[1].min(py), bb[2].max(px), bb[3].max(py)]; }
        }
        x = nx; y = ny;
    }
    (len, bb)
}

/// A burned move that starts in one flood-fill area and ends in another (the fixture's areas are separated
/// in X: square 10..60, triangle 70..120, star 130..170).
fn crosses_gap(g: &str) -> bool {
    let zone = |x: f64| if x <= 60.5 { 0 } else if x <= 120.5 { 1 } else { 2 };
    let (mut x, mut s) = (0.0f64, 0.0f64);
    for raw in g.lines() {
        let ln = raw.split(';').next().unwrap().trim();
        let mut nx = None;
        for w in ln.split_whitespace() {
            if let Some(v) = w.strip_prefix('X') { nx = v.parse().ok(); }
            if let Some(v) = w.strip_prefix('S') { s = v.parse().unwrap_or(s); }
        }
        if let Some(nx) = nx {
            if ln.starts_with("G1") && s > 0.0 && zone(x) != zone(nx) { return true; }
            x = nx;
        }
    }
    false
}

#[test]
fn phase2_matches_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures2");
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    let mut fails = Vec::new();
    let mut ok = 0;
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let err: Option<String> = match fx["kind"].as_str().unwrap() {
            "dither" => {
                let (c, r) = (fx["cols"].as_u64().unwrap() as usize, fx["rows"].as_u64().unwrap() as usize);
                let input: Vec<f32> = fx["input"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
                let adj: imaging::Adj = serde_json::from_value(fx["adj"].clone()).unwrap();
                let mut g = imaging::Grey { cols: c, rows: r, v: input };
                imaging::adjust(&mut g, &adj);
                let got = imaging::dither(&g, &adj, fx["px_mm"].as_f64().unwrap());
                let want: Vec<u8> = fx["expected"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
                let diff = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                (diff != 0).then(|| format!("{name}: {diff}/{} pixels differ", want.len()))
            }
            "rotate" => {
                let (c, r) = (fx["cols"].as_u64().unwrap() as usize, fx["rows"].as_u64().unwrap() as usize);
                let input: Vec<u8> = fx["input"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
                let (got, gc, gr) = imaging::rotate_nearest(&input, c, r, fx["angle"].as_f64().unwrap());
                let want: Vec<u8> = fx["expected"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
                let (ec, er) = (fx["ecols"].as_u64().unwrap() as usize, fx["erows"].as_u64().unwrap() as usize);
                if (gc, gr) != (ec, er) {
                    Some(format!("{name}: size {gc}x{gr} vs {ec}x{er}"))
                } else {
                    let diff = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                    (diff as f64 > 0.02 * want.len() as f64).then(|| format!("{name}: {diff}/{} pixels differ", want.len()))
                }
            }
            "process" => {
                let pic = imaging::decode(fx["src"].as_str().unwrap()).expect("decode");
                let adj: imaging::Adj = serde_json::from_value(fx["adj"].clone()).unwrap();
                let (got, c, r) = imaging::process(&pic, fx["w"].as_f64().unwrap(), fx["h"].as_f64().unwrap(),
                                                   fx["interval"].as_f64().unwrap(), 16000, false, false, false, &adj);
                let want: Vec<u8> = fx["expected"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
                let (ec, er) = (fx["cols"].as_u64().unwrap() as usize, fx["rows"].as_u64().unwrap() as usize);
                if (c, r) != (ec, er) {
                    Some(format!("{name}: size {c}x{r} vs {ec}x{er}"))
                } else {
                    let n = want.len() as f64;
                    let mean = |v: &[u8]| v.iter().map(|&x| x as f64).sum::<f64>() / n;
                    let mad = got.iter().zip(&want).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / n;
                    let outside = got.iter().zip(&want).filter(|(a, b)| (**a == 0) != (**b == 0) && (**a == 0 || **b == 0) && (*a.max(b) > &40)).count();
                    let gray = adj.mode == "gray";
                    let bad = (mean(&got) - mean(&want)).abs() > 3.0 || (gray && mad > 4.0) || outside as f64 > 0.01 * n && gray;
                    bad.then(|| format!("{name}: mean {:.1} vs {:.1}, mean abs diff {mad:.2}, mask mismatches {outside}", mean(&got), mean(&want)))
                }
            }
            "clip" => {
                let (w, h) = (fx["w"].as_f64().unwrap(), fx["h"].as_f64().unwrap());
                let res = geometry::clip_paths(&pts(&fx["paths"]), w, h, fx["fill"].as_bool().unwrap());
                let len: f64 = res.iter().map(|r| r.windows(2).map(|s| (s[1].x - s[0].x).hypot(s[1].y - s[0].y)).sum::<f64>()).sum();
                let want = fx["length"].as_f64().unwrap();
                let outside = res.iter().flatten().any(|p| p.x < -1e-9 || p.y < -1e-9 || p.x > w + 1e-9 || p.y > h + 1e-9);
                ((len - want).abs() > 0.01 * want.max(1.0) || outside)
                    .then(|| format!("{name}: length {len:.3} vs {want:.3}, point outside: {outside}"))
            }
            "kerf" => {
                let res = geometry::offset_paths(&pts(&fx["paths"]), fx["distance"].as_f64().unwrap());
                let area: f64 = res.iter().map(|r| geometry::area(&[r.clone()])).sum();
                let want = fx["area"].as_f64().unwrap();
                ((area - want).abs() > 0.01 * want.max(1.0) || res.len() as u64 != fx["count"].as_u64().unwrap())
                    .then(|| format!("{name}: area {area:.3} vs {want:.3}, rings {} vs {}", res.len(), fx["count"]))
            }
            "bool" => {
                let res = geometry::boolean(fx["op"].as_str().unwrap(), &[pts(&fx["a"]), pts(&fx["b"])]);
                let area = geometry::area(&res);
                let want = fx["area"].as_f64().unwrap();
                ((area - want).abs() > 0.01 * want.max(1.0)).then(|| format!("{name}: area {area:.3} vs {want:.3}"))
            }
            "job" => {
                let job: Job = serde_json::from_value(fx["job"].clone()).unwrap();
                let out = laserforge_core::generate(&job);
                let (lr, br) = burned(&out.gcode);
                let (lp, bp) = burned(fx["gcode"].as_str().unwrap());
                // GEOS (shapely) leaves the mitre off on the vertex where a ring starts; geo mitres every corner the
                // same way, so a kerfed sharp tip can reach ~0.4 mm further in Rust
                let tol = if name.starts_with("job_kerf") { 0.7 } else { 0.25 };
                let bad_bb = br.iter().zip(bp.iter()).any(|(a, b)| (a - b).abs() > tol);
                let areas = |g: &str| g.lines().find(|l| l.starts_with("; Flood fill:")).map(|l| l.to_string());
                let flood_bad = name.contains("flood") && (areas(&out.gcode).is_none() || areas(&out.gcode) != areas(fx["gcode"].as_str().unwrap())
                    || crosses_gap(&out.gcode));
                ((lr - lp).abs() > 0.03 * lp.max(1.0) || bad_bb || flood_bad)
                    .then(|| format!("{name}: burned {lr:.2} vs {lp:.2} mm, box {br:?} vs {bp:?}, flood {flood_bad} ({:?} vs {:?})",
                                     areas(&out.gcode), areas(fx["gcode"].as_str().unwrap())))
            }
            k => Some(format!("{name}: unknown kind {k}")),
        };
        match err { Some(e) => fails.push(e), None => ok += 1 }
    }
    eprintln!("phase 2: {ok}/{} fixtures within tolerance", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
