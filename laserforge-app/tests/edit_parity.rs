//! Edit / Arrange commands against the Python app (fixtures made by the Python app of v0.4.0-beta): same shapes and selection.

use laserforge_app::edit::Place;
use laserforge_app::project::Doc;
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
        (Value::Number(x), Value::Bool(y)) | (Value::Bool(y), Value::Number(x)) if (x.as_f64() == Some(1.0)) == *y => Ok(()),
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: rust {a} python {b}")),
    }
}

fn area(s: &Shape) -> f64 {
    s.paths().iter().map(|p| (1..p.len()).map(|i| p[i - 1][0] * p[i][1] - p[i][0] * p[i - 1][1]).sum::<f64>() / 2.0).sum::<f64>().abs()
}

#[test]
fn edits_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures5");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures missing: they were made by the Python app (v0.4.0-beta)").map(|e| e.unwrap().path()).collect();
    names.sort();
    let (mut ok, mut fails) = (0, Vec::new());
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let mut d = Doc::default();
        d.shapes = serde_json::from_value(fx["base"].clone()).unwrap();
        d.next_id = 100;
        d.selected = fx["selected_before"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        let ws = (fx["workspace"][0].as_f64().unwrap(), fx["workspace"][1].as_f64().unwrap());
        let sp = &fx["spec"];
        let f = |k: &str| sp[k].as_f64().unwrap_or(0.0);
        let b = |k: &str| sp[k].as_bool().unwrap_or(false);
        let st = |k: &str| sp[k].as_str().unwrap_or("").to_string();
        let r = match sp["op"].as_str().unwrap() {
            "mirror" => d.mirror_sel(&st("axis")),
            "rotate" => d.rotate_selection(f("deg")),
            "align" => d.align_sel(&st("mode")),
            "distribute" => d.distribute_sel(b("h"), b("centers")),
            "move_to" => d.move_sel_to(match st("place").as_str() { "C" => Place::C, "DR" => Place::DR, _ => Place::UL }, ws, [0.0, 0.0]),
            "same_size" => d.same_size(b("width")),
            "group" => d.group_sel(),
            "ungroup" => d.ungroup_sel(),
            "reorder" => d.reorder(&st("how")),
            "convert_to_path" => d.convert_to_path(),
            "auto_join" => d.auto_join(f("tol")),
            "close_tol" => d.close_with_tolerance(f("tol")),
            "break_apart" => d.break_apart(),
            "mirror_line" => d.mirror_across_line(),
            "close_path" => d.close_path(),
            "duplicate" => d.duplicate_sel(),
            "delete" => { d.delete_selected(); Ok(None) }
            "lock" => d.set_locked(b("v")),
            "invert" => { d.invert_selection(); Ok(None) }
            "select_contained" => d.select_contained(),
            "select_smaller" => d.select_smaller(),
            "field" => d.apply_transform_field(&st("key"), f("v"), b("lock")),
            "insert" => { d.insert_shape(&st("kind"), ws); Ok(None) }
            "nesting" => d.nesting(ws),
            "geometry" => d.geometry_op(&st("name"), &sp["params"], sp["replace"].as_bool().unwrap_or(true), sp["minimum"].as_u64().unwrap_or(1) as usize),
            "dup_then_delete" => {
                let mut c = d.shapes[0].clone();
                c.0.insert("id".into(), 99.into());
                d.shapes.push(c);
                d.delete_duplicates()
            }
            o => panic!("unknown op {o}"),
        };
        let _ = r;
        let want: Vec<Shape> = serde_json::from_value(fx["shapes"].clone()).unwrap();
        let mut errs = Vec::new();
        if sp["op"] == "geometry" {
            // shapely and geo: same regions, different vertices
            let (ga, wa): (Vec<f64>, Vec<f64>) = (d.shapes.iter().map(area).collect(), want.iter().map(area).collect());
            if ga.len() != wa.len() || ga.iter().zip(&wa).any(|(x, y)| (x - y).abs() > 0.01 * y.max(1.0)) {
                errs.push(format!("areas rust {ga:?} python {wa:?}"));
            }
        } else if let Err(e) = close(&serde_json::to_value(&d.shapes).unwrap(), &fx["shapes"], "shapes") {
            errs.push(e);
        }
        let wsel: Vec<i64> = fx["selected"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        if sp["op"] != "geometry" && d.selected != wsel {
            errs.push(format!("selection rust {:?} python {wsel:?}", d.selected));
        }
        if errs.is_empty() { ok += 1 } else { fails.push(format!("{name}: {}", errs.join("; "))) }
    }
    eprintln!("edits: {ok}/{} match Python", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
