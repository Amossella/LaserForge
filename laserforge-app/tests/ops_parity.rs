//! Node operations, warp / taper, live array copies and text on path against the Python app
//! (fixtures made by the Python app of v0.4.0-beta).

use laserforge_app::shapes::{Shape, P};
use laserforge_app::tools;
use serde_json::{json, Value};
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

#[test]
fn ops_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures7");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let (mut ok, mut fails) = (0, Vec::new());
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let r: Result<(), String> = match fx["kind"].as_str().unwrap() {
            "node" => {
                let mut s: Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
                let a = &fx["args"];
                let res: Value = match fx["op"].as_str().unwrap() {
                    "split" => json!(tools::split_segment(&mut s, a[0].as_u64().unwrap() as usize, a[1].as_f64().unwrap())),
                    "smooth" => { tools::set_smooth(&mut s, a[0].as_u64().unwrap() as usize, a[1].as_bool().unwrap()); Value::Null }
                    "curve" => { tools::make_curve(&mut s, a[0].as_u64().unwrap() as usize); Value::Null }
                    "line" => { tools::make_line(&mut s, a[0].as_u64().unwrap() as usize); Value::Null }
                    "delete" => json!(tools::delete_nodes(&mut s, &a[0].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect::<Vec<_>>())),
                    "break" => json!(tools::break_at_node(&s, a[0].as_u64().unwrap() as usize)),
                    _ => json!(tools::delete_segment(&s, a[0].as_u64().unwrap() as usize)),
                };
                close(&serde_json::to_value(&s).unwrap(), &fx["out"], "shape").and(close(&res, &fx["result"], "result"))
            }
            "nearest" => {
                let s: Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
                let pt: P = serde_json::from_value(fx["pt"].clone()).unwrap();
                let out: Vec<Value> = (0..tools::seg_count(&s)).map(|i| { let (d, t, q) = tools::nearest_on_segment(&s, i, pt); json!([d, t, q]) }).collect();
                close(&Value::Array(out), &fx["out"], "nearest")
            }
            "warp" => {
                let shapes: Vec<Shape> = serde_json::from_value(fx["shapes"].clone()).unwrap();
                let c: Vec<P> = serde_json::from_value(fx["corners"].clone()).unwrap();
                let bx: [f64; 4] = serde_json::from_value(fx["box"].clone()).unwrap();
                let mut next = fx["next_id"].as_i64().unwrap();
                let out = tools::warp_shapes(&mut next, &shapes, bx, &|u, v| tools::bilinear(&c, u, v));
                close(&serde_json::to_value(&out).unwrap(), &fx["out"], "shapes")
            }
            "taper" => {
                let shapes: Vec<Shape> = serde_json::from_value(fx["shapes"].clone()).unwrap();
                let mut next = fx["next_id"].as_i64().unwrap();
                let out = tools::taper(&mut next, &shapes, fx["top"].as_f64().unwrap(), fx["bottom"].as_f64().unwrap());
                close(&serde_json::to_value(&out).unwrap(), &fx["out"], "shapes")
            }
            "array" => {
                let shapes: Vec<Shape> = serde_json::from_value(fx["shapes"].clone()).unwrap();
                let mut d = laserforge_app::project::Doc::default();
                d.next_id = fx["next_id"].as_i64().unwrap();
                let out = laserforge_app::dialogs::array_copies(&mut d, &shapes, &fx["meta"]);
                close(&serde_json::to_value(&out).unwrap(), &fx["out"], "copies")
            }
            "top" => {
                let s: Shape = serde_json::from_value(fx["shape"].clone()).unwrap();
                let path: Vec<P> = serde_json::from_value(fx["path"].clone()).unwrap();
                let got = laserforge_app::text::text_on_path(&s, &path, fx["start"].as_f64().unwrap(), fx["dist"].as_f64().unwrap(), fx["spacing"].as_f64().unwrap(), false);
                let want: Vec<Vec<P>> = serde_json::from_value(fx["out"].clone()).unwrap();
                let bb = |v: &[Vec<P>]| laserforge_app::shapes::path_box(&v.iter().flatten().copied().collect::<Vec<_>>());
                let (a, b) = (bb(&got), bb(&want));
                let off = a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
                if got.len() == want.len() && off < 0.5 { Ok(()) } else { Err(format!("rings {} vs {}, box {a:?} vs {b:?}", got.len(), want.len())) }
            }
            k => Err(format!("unknown kind {k}")),
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("{name}: {e}")),
        }
    }
    eprintln!("ops: {ok}/{} match Python", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
