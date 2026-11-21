//! Tools against the Python app (fixtures made by the Python app of v0.4.0-beta): test patterns, barcodes, QR codes, corner tools,
//! box generator, overlap removal, image tracing, PCB import and 3D depth maps.

use laserforge_app::shapes::Shape;
use serde_json::Value;
use std::path::Path;

fn close(a: &Value, b: &Value, path: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0) { Ok(()) } else { Err(format!("{path}: rust {x} python {y}")) }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{path}: rust {} items python {}", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().try_for_each(|(i, (p, q))| close(p, q, &format!("{path}[{i}]")))
        }
        (Value::Object(x), Value::Object(y)) => {
            let mut kx: Vec<&String> = x.keys().collect();
            let mut ky: Vec<&String> = y.keys().collect();
            kx.sort();
            ky.sort();
            if kx != ky {
                return Err(format!("{path}: keys rust {kx:?} python {ky:?}"));
            }
            x.iter().try_for_each(|(k, v)| close(v, &y[k], &format!("{path}.{k}")))
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: rust {a} python {b}")),
    }
}

fn length(paths: &[Vec<[f64; 2]>]) -> f64 {
    paths.iter().map(|p| p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f64>()).sum()
}

#[test]
fn tools_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures6");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let (mut ok, mut fails) = (0, Vec::new());
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let r: Result<(), String> = match fx["kind"].as_str().unwrap() {
            "pattern" => {
                let m: laserforge_core::model::MachineSettings = serde_json::from_value(fx["machine"].clone()).unwrap();
                let g = match fx["which"].as_str().unwrap() {
                    "material" => laserforge_core::patterns::material_test(&m, &fx["params"]),
                    "interval" => laserforge_core::patterns::interval_test(&m, &fx["params"]),
                    _ => laserforge_core::patterns::focus_test(&m, &fx["params"]),
                };
                let want = fx["gcode"].as_str().unwrap();
                if g == want { Ok(()) } else {
                    let i = g.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
                    Err(format!("line {}: rust {:?} python {:?}", i + 1, g.lines().nth(i), want.lines().nth(i)))
                }
            }
            "barcode" => {
                let (m, t) = laserforge_app::corners::barcode_modules(fx["type"].as_str().unwrap(), fx["data"].as_str().unwrap()).unwrap();
                if m == fx["modules"].as_str().unwrap() && t == fx["text"].as_str().unwrap() { Ok(()) } else { Err(format!("modules or text differ ({t})")) }
            }
            "qr" => {
                let m = laserforge_app::corners::qr_matrix(fx["text"].as_str().unwrap(), fx["level"].as_str().unwrap()).unwrap();
                let want: Vec<Vec<bool>> = fx["matrix"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(|v| v.as_i64() == Some(1)).collect()).collect();
                // segno and the qrcode crate may pick a different (equally valid) mask pattern: same version and
                // the same finder / timing patterns are required, the data area may differ
                let n = m.len();
                let fixed = |r: usize, c: usize| (r < 8 && c < 8) || (r < 8 && c >= n - 8) || (r >= n - 8 && c < 8) || r == 6 || c == 6;
                let bad = (0..n).flat_map(|r| (0..n).map(move |c| (r, c))).filter(|&(r, c)| fixed(r, c) && m[r][c] != want[r][c]).count();
                if n == want.len() && bad == 0 { Ok(()) } else { Err(format!("matrix {n}x{n} vs {}x{}, {bad} fixed modules differ", want.len(), want.len())) }
            }
            "fillet" => {
                let mut s: Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
                let only: Option<Vec<usize>> = fx["only"].as_array().map(|a| a.iter().map(|v| v.as_u64().unwrap() as usize).collect());
                let n = laserforge_app::corners::fillet(&mut s, fx["radius"].as_f64().unwrap(), only.as_deref());
                close(&serde_json::to_value(&s).unwrap(), &fx["out"], "shape").and(if n as u64 == fx["n"].as_u64().unwrap() { Ok(()) } else { Err(format!("n {n}")) })
            }
            "relief" => {
                let mut shapes: Vec<Shape> = serde_json::from_value(fx["shapes"].clone()).unwrap();
                let n = laserforge_app::corners::relief(&mut shapes, fx["diameter"].as_f64().unwrap(), fx["tbone"].as_bool().unwrap(), true);
                close(&serde_json::to_value(&shapes).unwrap(), &fx["out"], "shapes").and(if n as u64 == fx["n"].as_u64().unwrap() { Ok(()) } else { Err(format!("n {n}")) })
            }
            "slots" => {
                let mut s: Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
                let n = laserforge_app::corners::resize_slots(&mut s, fx["old"].as_f64().unwrap(), fx["new"].as_f64().unwrap(), fx["tol"].as_f64().unwrap());
                close(&serde_json::to_value(&s).unwrap(), &fx["out"], "shape").and(if n as u64 == fx["n"].as_u64().unwrap() { Ok(()) } else { Err(format!("n {n}")) })
            }
            "apollonius" => {
                let c: Vec<[f64; 3]> = serde_json::from_value(fx["circles"].clone()).unwrap();
                let out = laserforge_app::corners::apollonius([c[0], c[1], c[2]]);
                close(&serde_json::to_value(&out).unwrap(), &fx["out"], "circles")
            }
            "box" => {
                let a = &fx["args"];
                let e: Vec<String> = serde_json::from_value(a[4].clone()).unwrap();
                let out = laserforge_app::corners::box_panel(a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap(), a[3].as_f64().unwrap(),
                    [&e[0], &e[1], &e[2], &e[3]], a[5].as_f64().unwrap(), a[6].as_f64().unwrap());
                close(&Value::Array(out), &fx["out"], "points")
            }
            "overlaps" => {
                let mut d = laserforge_app::project::Doc::default();
                d.shapes = vec![Shape::rect(1, 0, 0.0, 0.0, 20.0, 20.0), Shape::rect(2, 0, 20.0, 0.0, 20.0, 20.0), Shape::poly(3, 0, &[[5.0, 20.0], [35.0, 20.0]], false)];
                d.next_id = 10;
                d.selected = vec![1, 2, 3];
                let _ = d.delete_overlapping();
                let want: Vec<Shape> = serde_json::from_value(fx["out"].clone()).unwrap();
                let (lr, lp) = (length(&d.shapes.iter().flat_map(|s| s.paths()).collect::<Vec<_>>()), length(&want.iter().flat_map(|s| s.paths()).collect::<Vec<_>>()));
                if (lr - lp).abs() < 1e-6 { Ok(()) } else { Err(format!("length rust {lr} python {lp} ({} vs {} shapes)", d.shapes.len(), want.len())) }
            }
            "trace" => {
                let rows: Vec<Vec<i64>> = serde_json::from_value(fx["on"].clone()).unwrap();
                let (h, w) = (rows.len(), rows[0].len());
                let on: Vec<bool> = rows.iter().flatten().map(|v| *v == 1).collect();
                let got = laserforge_app::bitmap::trace_binary(&on, w, h);
                let want: Vec<Vec<[f64; 2]>> = serde_json::from_value(fx["loops"].clone()).unwrap();
                if got.len() == want.len() && (length(&got) - length(&want)).abs() < 1e-9 { Ok(()) } else { Err(format!("{} loops / {} vs {} / {}", got.len(), length(&got), want.len(), length(&want))) }
            }
            "pcb" => {
                let rings = laserforge_app::pcb::rings(fx["text"].as_str().unwrap(), fx["drill"].as_bool().unwrap(), 100.0);
                let pts: Vec<Vec<laserforge_core::model::Pt>> = rings.iter().map(|r| r.iter().map(|q| laserforge_core::model::Pt { x: q[0], y: q[1] }).collect()).collect();
                let area = laserforge_core::geometry::area(&pts);
                let want = fx["area"].as_f64().unwrap();
                if (area - want).abs() < 0.01 * want && rings.len() as u64 == fx["rings"].as_u64().unwrap() { Ok(()) } else { Err(format!("area {area:.3} vs {want:.3}, rings {} vs {}", rings.len(), fx["rings"])) }
            }
            "mesh" => {
                let p = std::env::temp_dir().join(format!("lf-mesh-{}.stl", std::process::id()));
                std::fs::write(&p, fx["stl"].as_str().unwrap()).unwrap();
                let tris = laserforge_app::mesh3d::load_mesh(&p).unwrap();
                let (grey, w, h, wmm, hmm) = laserforge_app::mesh3d::depth_map_tris(&tris, fx["px"].as_f64().unwrap(), false, 1600.0).unwrap();
                let want: Vec<Vec<i64>> = serde_json::from_value(fx["grey"].clone()).unwrap();
                let diff = grey.iter().zip(want.iter().flatten()).filter(|(a, b)| (**a as i64 - **b).abs() > 1).count();
                if want.len() == h && want[0].len() == w && diff == 0 && (wmm - fx["w"].as_f64().unwrap()).abs() < 1e-9 && (hmm - fx["h"].as_f64().unwrap()).abs() < 1e-9 { Ok(()) }
                else { Err(format!("{w}x{h} vs {}x{}, {diff} pixels differ", want[0].len(), want.len())) }
            }
            k => Err(format!("unknown kind {k}")),
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("{name}: {e}")),
        }
    }
    eprintln!("tools: {ok}/{} match Python", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
