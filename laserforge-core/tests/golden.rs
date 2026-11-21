//! Golden tests: every fixture was produced by the Python engine (fixtures made by the Python app of v0.4.0-beta);
//! the Rust engine must give exactly the same G-code.

use laserforge_core::{gcode::Generator, model::MachineSettings, post, Job};
use serde_json::Value;
use std::path::Path;

fn first_diff(name: &str, what: &str, got: &str, want: &str) -> Option<String> {
    if got == want {
        return None;
    }
    let (g, w): (Vec<&str>, Vec<&str>) = (got.split('\n').collect(), want.split('\n').collect());
    let i = (0..g.len().max(w.len())).find(|&i| g.get(i) != w.get(i)).unwrap_or(0);
    Some(format!("{name} [{what}] line {}: rust {:?} python {:?} (rust {} lines, python {})",
                 i + 1, g.get(i), w.get(i), g.len(), w.len()))
}

#[test]
fn golden_fixtures_match_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    assert!(!names.is_empty(), "no fixtures: fixtures missing: they were made by the Python app (v0.4.0-beta)");
    let (mut ok, mut fails) = (0, Vec::new());
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let mut errs: Vec<String> = Vec::new();
        match fx["kind"].as_str().unwrap() {
            "job" => {
                let job: Job = serde_json::from_value(fx["job"].clone()).unwrap();
                let out = laserforge_core::generate(&job);
                errs.extend(first_diff(&name, "gcode", &out.gcode, fx["gcode"].as_str().unwrap()));
                errs.extend(first_diff(&name, "send", &out.send, fx["send"].as_str().unwrap()));
                let want = fx["seconds"].as_f64().unwrap();
                if (out.seconds - want).abs() > 1e-6 * want.max(1.0) {
                    errs.push(format!("{name} [seconds] rust {} python {}", out.seconds, want));
                }
            }
            "frame" => {
                let m: MachineSettings = serde_json::from_value(fx["machine"].clone()).unwrap();
                let b: Vec<f64> = serde_json::from_value(fx["bounds"].clone()).unwrap();
                let pair = |v: &Value| v.as_array().map(|a| (a[0].as_f64().unwrap(), a[1].as_f64().unwrap()));
                let out = Generator::new(&m).generate_frame((b[0], b[1], b[2], b[3]), fx["laser_on"].as_bool().unwrap(),
                    fx["power"].as_f64(), fx["speed"].as_f64().unwrap(), pair(&fx["relative_to"]), pair(&fx["head"]));
                errs.extend(first_diff(&name, "frame", &out, fx["expected"].as_str().unwrap()));
            }
            "resume" => {
                let out = post::resume_from(fx["gcode"].as_str().unwrap(), fx["line"].as_u64().unwrap() as usize);
                errs.extend(first_diff(&name, "resume", &out, fx["expected"].as_str().unwrap()));
            }
            "feeder" => {
                let m: MachineSettings = serde_json::from_value(fx["machine"].clone()).unwrap();
                let out = post::apply_feeder(fx["gcode"].as_str().unwrap(), &m);
                errs.extend(first_diff(&name, "feeder", &out, fx["expected"].as_str().unwrap()));
            }
            k => errs.push(format!("{name}: unknown kind {k}")),
        }
        if errs.is_empty() { ok += 1 } else { fails.extend(errs) }
    }
    eprintln!("golden: {ok}/{} fixtures identical", names.len());
    assert!(fails.is_empty(), "\n{}", fails.join("\n"));
}
