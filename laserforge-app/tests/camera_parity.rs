//! Camera maths against laserforge/extras.py (fixtures made by the Python app of v0.4.0-beta).

use image::RgbImage;
use laserforge_app::camera;
use serde_json::Value;
use std::path::Path;

fn to_img(v: &Value) -> RgbImage {
    let rows = v.as_array().unwrap();
    let (h, w) = (rows.len(), rows[0].as_array().unwrap().len());
    let mut img = RgbImage::new(w as u32, h as u32);
    for (y, r) in rows.iter().enumerate() {
        for (x, p) in r.as_array().unwrap().iter().enumerate() {
            let p = p.as_array().unwrap();
            img.put_pixel(x as u32, y as u32, image::Rgb([p[0].as_u64().unwrap() as u8, p[1].as_u64().unwrap() as u8, p[2].as_u64().unwrap() as u8]));
        }
    }
    img
}

/// (pixels off by more than `tol`, total)
fn diff(a: &RgbImage, b: &RgbImage, tol: i32) -> (usize, usize) {
    let n = a.pixels().zip(b.pixels()).filter(|(p, q)| (0..3).any(|k| (p.0[k] as i32 - q.0[k] as i32).abs() > tol)).count();
    (n, (a.width() * a.height()) as usize)
}

#[test]
fn camera_matches_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures8");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let mut fails = Vec::new();
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let r: Result<(), String> = match fx["kind"].as_str().unwrap() {
            "undistort" => {
                let out = camera::undistort(&to_img(&fx["src"]), fx["k1"].as_f64().unwrap(), fx["k2"].as_f64().unwrap());
                let (n, t) = diff(&out, &to_img(&fx["out"]), 1);
                if n == 0 { Ok(()) } else { Err(format!("{n}/{t} pixels differ")) }
            }
            "homography" => {
                let src: Vec<[f64; 2]> = serde_json::from_value(fx["src"].clone()).unwrap();
                let dst: Vec<[f64; 2]> = serde_json::from_value(fx["dst"].clone()).unwrap();
                let h = camera::homography(&src, &dst).unwrap();
                let want: [[f64; 3]; 3] = serde_json::from_value(fx["h"].clone()).unwrap();
                // compare where the matrices send the source points (least-squares variants differ slightly)
                let worst = src.iter().map(|s| { let (a, b) = (camera::apply_h(&h, s[0], s[1]), camera::apply_h(&want, s[0], s[1])); (a[0] - b[0]).hypot(a[1] - b[1]) }).fold(0.0, f64::max);
                if worst < 1e-6 { Ok(()) } else { Err(format!("points differ by {worst}")) }
            }
            "warp" => {
                let ws = (fx["ws"][0].as_f64().unwrap(), fx["ws"][1].as_f64().unwrap());
                let out = camera::warp_to_workspace(&to_img(&fx["src"]), &fx["cal"], ws, fx["max"].as_f64().unwrap());
                let want = to_img(&fx["out"]);
                if (out.width(), out.height()) != (want.width(), want.height()) { Err(format!("size {:?} vs {:?}", out.dimensions(), want.dimensions())) } else {
                    let (n, t) = diff(&out, &want, 40);
                    if n * 20 <= t { Ok(()) } else { Err(format!("{n}/{t} pixels differ by more than 40")) }
                }
            }
            "detect" => {
                let ws = (fx["ws"][0].as_f64().unwrap(), fx["ws"][1].as_f64().unwrap());
                let m = camera::detect_mark(&to_img(&fx["img"]), ws, fx["at"][0].as_f64().unwrap(), fx["at"][1].as_f64().unwrap(), 15.0).unwrap();
                let w = [fx["out"][0].as_f64().unwrap(), fx["out"][1].as_f64().unwrap()];
                if (m[0] - w[0]).abs() < 1e-6 && (m[1] - w[1]).abs() < 1e-6 { Ok(()) } else { Err(format!("{m:?} vs {w:?}")) }
            }
            "fitk1" => {
                let pts: Vec<[f64; 2]> = serde_json::from_value(fx["points"].clone()).unwrap();
                let k = camera::fit_k1(&pts, (640.0, 480.0));
                let w = fx["out"].as_f64().unwrap();
                if (k - w).abs() < 1e-9 { Ok(()) } else { Err(format!("{k} vs {w}")) }
            }
            k => Err(format!("unknown {k}")),
        };
        if let Err(e) = r {
            fails.push(format!("{name}: {e}"));
        }
    }
    eprintln!("camera: {}/{} match Python", names.len() - fails.len(), names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
