//! LaserForge engine in Rust (phase 1 of the rewrite): data model, G-code generation, post-processing
//! and the GRBL protocol logic. Ported from LaserForge's own Python engine.

pub mod gcode;
pub mod geometry;
pub mod grbl;
pub mod imaging;
pub mod model;
pub mod patterns;
pub mod post;
pub mod util;

use model::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct JobLayer {
    #[serde(default)]
    pub settings: LayerSettings,
    #[serde(default)]
    pub paths: Vec<Vec<[f64; 2]>>,
    /// Manual tab positions per path (fractions of its length), or null.
    #[serde(default)]
    pub tabs: Vec<Option<Vec<f64>>>,
}

#[derive(Debug, Deserialize)]
pub struct JobRaster {
    #[serde(default)]
    pub settings: LayerSettings,
    #[serde(flatten)]
    pub raster: Raster,
}

/// A job as the CLI reads it: machine settings, vector layers (machine coordinates) and rasters.
#[derive(Debug, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub machine: MachineSettings,
    #[serde(default)]
    pub layers: Vec<JobLayer>,
    #[serde(default)]
    pub rasters: Vec<JobRaster>,
    /// "current" or "user": sent relative to the head (G91), starting at `start`.
    #[serde(default)]
    pub origin_mode: Option<String>,
    #[serde(default)]
    pub start: Option<[f64; 2]>,
}

/// A layer, then each enabled sub-layer: the same shapes again with its own settings.
pub fn expand_sublayers(layer: &LayerSettings) -> Vec<LayerSettings> {
    let mut out = vec![layer.clone()];
    for (k, sub) in layer.sublayers.iter().enumerate() {
        let Value::Object(map) = sub else { continue };
        if map.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let mut base = serde_json::to_value(layer).unwrap();
        if let Value::Object(b) = &mut base {
            for (kk, v) in map {
                if kk != "name" && kk != "enabled" && kk != "sublayers" {
                    b.insert(kk.clone(), v.clone());
                }
            }
            b.insert("sublayers".into(), Value::Array(vec![]));
            let name = map.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from)
                .unwrap_or_else(|| format!("{}.{}", layer.name, k + 1));
            b.insert("name".into(), Value::String(name));
        }
        if let Ok(l) = serde_json::from_value::<LayerSettings>(base) {
            out.push(l);
        }
    }
    out
}

/// Features of the Python engine not ported yet: a job using one would come out wrong, so callers refuse it.
pub fn unported_features(_job: &Job) -> Vec<String> {
    // Everything the Python engine does to a job is ported (phase 2 added kerf, offset fill, flood fill and
    // image rotation). Kept so callers keep refusing jobs if a future feature lands in Python first.
    Vec::new()
}

pub struct Output {
    /// Absolute G-code (for preview / resume).
    pub gcode: String,
    /// What goes to the laser or to a file (relative for current-position / user-origin jobs).
    pub send: String,
    pub seconds: f64,
}

/// The whole pipeline: layers → G-code, backlash, rotary A, controller flavour, relative modes.
pub fn generate(job: &Job) -> Output {
    let m = &job.machine;
    let mut layers: Vec<(LayerSettings, Vec<GPath>)> = Vec::new();
    for jl in &job.layers {
        if !jl.settings.enabled || jl.settings.tool {
            continue;
        }
        let paths: Vec<GPath> = jl.paths.iter().enumerate().filter(|(_, p)| !p.is_empty()).map(|(i, p)| GPath {
            pts: p.iter().map(|q| Pt { x: q[0], y: q[1] }).collect(),
            tabs: jl.tabs.get(i).cloned().flatten(),
        }).collect();
        if paths.is_empty() {
            continue;
        }
        for l in expand_sublayers(&jl.settings) {
            layers.push((l, paths.clone()));
        }
    }
    let rasters: Vec<(LayerSettings, Raster)> = job.rasters.iter()
        .filter(|r| r.settings.enabled && !r.settings.tool)
        .map(|r| (r.settings.clone(), r.raster.clone())).collect();
    let relative = matches!(job.origin_mode.as_deref(), Some("current") | Some("user"));
    let finish = if relative { job.start.map(|s| (s[0], s[1])) } else { None };
    let timing = std::env::var("LF_TIMING").is_ok();
    let t0 = std::time::Instant::now();
    let mut g = gcode::Generator::new(m);
    let job_g = g.generate_job(&layers, &rasters, finish);
    if timing { eprintln!("generate: {:?}", t0.elapsed()); }
    let raw = post::apply_backlash(&job_g, m.backlash_x, m.backlash_y);
    if timing { eprintln!("backlash: {:?}", t0.elapsed()); }
    let seconds = post::estimate_gcode_time(&raw, m);
    if timing { eprintln!("estimate: {:?}", t0.elapsed()); }
    let gcode = post::translate_flavor(&post::apply_rotary_axis(&raw, m), m);
    if timing { eprintln!("post: {:?}", t0.elapsed()); }
    let send = match (relative, job.start) {
        (true, Some(s)) => post::to_relative(&gcode, (s[0], s[1]), job.origin_mode.as_deref() == Some("user")),
        _ => gcode.clone(),
    };
    Output { gcode, send, seconds }
}
