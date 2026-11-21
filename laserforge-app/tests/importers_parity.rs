//! Importers against the Python app (fixtures made by the Python app of v0.4.0-beta): the same shapes from the same SVG, DXF, PDF
//! and image files.

use base64::Engine;
use laserforge_app::importers::{dxf_to_paths, image_shape, pdf_image_shapes, pdf_to_paths, svg_shapes, vector_shapes, Props};
use serde_json::Value;
use std::path::Path;

/// Same JSON up to float rounding (1e-9 relative).
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

#[test]
fn importers_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures4");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let (mut ok, mut fails) = (0, Vec::new());
    let b64 = |v: &Value| base64::engine::general_purpose::STANDARD.decode(v.as_str().unwrap()).unwrap();
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        if fx["kind"] == "text" {
            let sh: laserforge_app::shapes::Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
            let got = sh.paths();
            let want: Vec<Vec<[f64; 2]>> = serde_json::from_value(fx["paths"].clone()).unwrap();
            let single = sh.0["font"].as_str().unwrap().starts_with("Hershey:");
            let err = if single {
                close(&serde_json::to_value(&got).unwrap(), &fx["paths"], &name).err()
            } else {
                // Qt and ttf-parser: same glyph contours, curves flattened differently, Qt kerns
                let bb = |p: &[Vec<[f64; 2]>]| laserforge_app::shapes::path_box(&p.iter().flatten().copied().collect::<Vec<_>>());
                let (a, b) = (bb(&got), bb(&want));
                let size = sh.f("size");
                let off = a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
                let (wr, wp) = (laserforge_app::text::text_width(&sh), fx["width"].as_f64().unwrap());
                (got.len() != want.len() || off > 0.04 * size + 0.05 || (wr - wp).abs() > 0.03 * wp.max(1.0))
                    .then(|| format!("{name}: rings {} vs {}, box {a:?} vs {b:?}, width {wr:.3} vs {wp:.3}", got.len(), want.len()))
            };
            match err { None => ok += 1, Some(e) => fails.push(e) }
            continue;
        }
        let shapes: Vec<Props> = match fx["kind"].as_str().unwrap() {
            "svg" => svg_shapes(fx["input"].as_str().unwrap()).unwrap(),
            "dxf" => vector_shapes(dxf_to_paths(fx["input"].as_str().unwrap())),
            "pdf" => {
                let data = b64(&fx["input"]);
                let mut imgs = Vec::new();
                let paths = pdf_to_paths(&data, 0, Some(&mut imgs)).unwrap();
                let mut s = pdf_image_shapes(&imgs, "a.pdf");
                s.extend(vector_shapes(paths));
                s
            }
            "image" => {
                let ws = &fx["workspace"];
                vec![image_shape(&b64(&fx["input"]), fx["name"].as_str().unwrap(), (ws[0].as_f64().unwrap(), ws[1].as_f64().unwrap()), None).unwrap()]
            }
            k => panic!("unknown kind {k}"),
        };
        let got = Value::Array(shapes.into_iter().map(|mut s| {
            s.remove("src");
            Value::Object(s)
        }).collect());
        match close(&got, &fx["shapes"], &name) {
            Ok(()) => ok += 1,
            Err(e) => fails.push(e),
        }
    }
    eprintln!("importers: {ok}/{} match Python", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
