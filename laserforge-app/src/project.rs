//! The design document (same JSON as the Python app's `.lfproj`) and how it becomes a job: clipping to the
//! work area, start points, path order, origin corner, images. Ported from laserforge/mainwindow.py
//! (build_job, optimize_paths, _rasters_to_machine) and document.py.

use crate::shapes::{flatten, is_closed_path, multi_bounds, path_box, rotate_closed_path, Shape, P};
use laserforge_core::geometry::clip_paths;
use laserforge_core::model::{CutMode, LayerSettings, MachineSettings, Pt, Raster};
use laserforge_core::{Job, JobLayer, JobRaster};
use serde_json::{json, Map, Value};

pub const LAYER_COLORS: [&str; 30] = [
    "#000000", "#0000FF", "#FF0000", "#00FF00", "#FF8000", "#00FFFF", "#FF00FF", "#FFFF00",
    "#800080", "#008080", "#808000", "#FF69B4", "#4169E1", "#32CD32", "#FF4500", "#9400D3",
    "#8B4513", "#2E8B57", "#B22222", "#1E90FF", "#DAA520", "#20B2AA", "#C71585", "#6B8E23",
    "#483D8B", "#CD853F", "#00CED1", "#DC143C", "#7B68EE", "#556B2F",
];
pub const TOOL_LAYERS: [(&str, &str); 2] = [("T1", "#E07A10"), ("T2", "#3C8DBC")];
pub const ORIGIN_POS: [(f64, f64); 9] = [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (0.5, 0.5), (1.0, 0.5), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)];

pub fn default_layers() -> Vec<LayerSettings> {
    let mut v: Vec<LayerSettings> = LAYER_COLORS.iter().enumerate()
        .map(|(i, c)| LayerSettings { name: format!("C{i:02}"), color: (*c).into(), ..Default::default() }).collect();
    for (n, c) in TOOL_LAYERS {
        v.push(LayerSettings { name: n.into(), color: c.into(), tool: true, enabled: false, ..Default::default() });
    }
    v
}

/// engine.update_layer: unknown, null or wrongly typed keys are skipped; tool layers keep their identity.
pub fn apply_layer_dict(layer: &mut LayerSettings, data: &Map<String, Value>) {
    let was_tool = layer.tool;
    let (name, color) = (layer.name.clone(), layer.color.clone());
    let mut base = serde_json::to_value(&*layer).unwrap();
    if let Value::Object(b) = &mut base {
        for (k, v) in data {
            if k == "index" || k == "tool" || v.is_null() || !b.contains_key(k) {
                continue;
            }
            let old = b[k].clone();
            b.insert(k.clone(), v.clone());
            if serde_json::from_value::<LayerSettings>(Value::Object(b.clone())).is_err() {
                b.insert(k.clone(), old);
            }
        }
    }
    if let Ok(l) = serde_json::from_value::<LayerSettings>(base) {
        *layer = l;
    }
    if was_tool {
        layer.tool = true;
        layer.enabled = false;
        layer.name = name;
        layer.color = color;
    }
}

/// Path optimisation settings (the Python app's `opt` dict, same keys).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Opt {
    pub inner_first: bool,
    pub nearest: bool,
    pub best_start: bool,
    pub best_direction: bool,
    /// "keep", "cw" or "ccw"
    pub direction: String,
    /// "layer", "group" or "priority"
    pub order: String,
    pub remove_overlap: bool,
}

impl Default for Opt {
    fn default() -> Self {
        Opt { inner_first: true, nearest: true, best_start: true, best_direction: true, direction: "keep".into(),
              order: "layer".into(), remove_overlap: false }
    }
}

/// Where the job starts and what is sent: everything build_job reads from the window besides the document.
#[derive(Debug, Clone, Default)]
pub struct JobOpts {
    pub opt: Opt,
    /// Only these top-level shape ids (cut selected only).
    pub only: Option<Vec<i64>>,
    /// Shapes whose box anchors the job ("origin from selection"); None = the shapes being cut.
    pub origin_ids: Option<Vec<i64>>,
    /// "absolute", "current" or "user"
    pub start_from: String,
    /// 0..8, which point of the job box goes to the head / user origin.
    pub anchor: usize,
    /// Head position and user origin in machine coordinates.
    pub head: P,
    pub user_origin: P,
    /// Cylinder correction settings (enabled, diameter, axis, centre, mirror).
    pub cylinder: Option<Value>,
    /// Print & cut registration (enabled, src: [p1, p2], dst: [p1, p2]).
    pub print_cut: Option<Value>,
}

fn pair(v: &Value) -> Option<P> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// mainwindow.cylinder_transform
fn cylinder_transform(c: &Value) -> Option<impl Fn(P) -> P> {
    let num = |k: &str, d: f64| c.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
    if !c.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) || num("diameter", 0.0) <= 0.0 {
        return None;
    }
    let r = num("diameter", 0.0) / 2.0;
    let axis_x = c.get("axis").and_then(|v| v.as_str()).unwrap_or("x") == "x";
    let centre = num("centre", 0.0);
    let mirror = num("mirror", 0.0);
    let lim = r * std::f64::consts::PI / 2.0;
    let across = move |dist: f64| {
        let th = dist.clamp(-lim, lim) / r;
        let mut lateral = r * th.sin();
        if mirror > 0.0 {
            lateral *= mirror / (mirror + r * (1.0 - th.cos()));
        }
        lateral
    };
    Some(move |p: P| if axis_x { [p[0], centre + across(p[1] - centre)] } else { [centre + across(p[0] - centre), p[1]] })
}

/// model.crop_mask: 1 inside the crop rings (image-relative 0..1, even-odd), tested at pixel centres.
pub fn crop_mask(rings: &[Vec<P>], cols: usize, rows: usize) -> Vec<u8> {
    let mut m = vec![0u8; cols * rows];
    for ring in rings.iter().filter(|r| r.len() >= 3) {
        let pts: Vec<P> = ring.iter().map(|q| [q[0] * cols as f64, q[1] * rows as f64]).collect();
        for r in 0..rows {
            let y = r as f64 + 0.5;
            let mut xs: Vec<f64> = Vec::new();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a[1] <= y) != (b[1] <= y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for seg in xs.chunks(2) {
                if seg.len() < 2 {
                    continue;
                }
                for c in 0..cols {
                    let x = c as f64 + 0.5;
                    if x >= seg[0] && x < seg[1] {
                        m[r * cols + c] ^= 1;
                    }
                }
            }
        }
    }
    m
}

/// Polygon clipping in shapely (GEOS) returns shells with negative signed area and holes with positive area
/// (in these Y-down coordinates); match it so start points and directions come out the same.
fn orient_like_geos(rings: &mut [Vec<P>]) {
    let inside = |pt: P, ring: &[P]| {
        let mut c = false;
        for i in 1..ring.len() {
            let (a, b) = (ring[i - 1], ring[i]);
            if (a[1] > pt[1]) != (b[1] > pt[1]) && pt[0] < a[0] + (pt[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                c = !c;
            }
        }
        c
    };
    let n = rings.len();
    let holes: Vec<bool> = (0..n).map(|i| {
        if !is_closed_path(&rings[i]) {
            return false;
        }
        let r = &rings[i];
        let probe = [(r[0][0] + r[1][0]) / 2.0, (r[0][1] + r[1][1]) / 2.0];
        (0..n).filter(|&j| j != i && is_closed_path(&rings[j]) && inside(probe, &rings[j])).count() % 2 == 1
    }).collect();
    for (r, hole) in rings.iter_mut().zip(holes) {
        if is_closed_path(r) && (signed_area(r) > 0.0) != hole {
            r.reverse();
        }
    }
}

fn signed_area(p: &[P]) -> f64 {
    (1..p.len()).map(|i| p[i - 1][0] * p[i][1] - p[i][0] * p[i - 1][1]).sum::<f64>() / 2.0
}

pub fn to_machine(m: &MachineSettings, p: P) -> P {
    let (w, h) = (m.workspace_x, m.workspace_y);
    let o = m.origin.as_str();
    [if matches!(o, "fl" | "rl") { p[0] } else { w - p[0] }, if matches!(o, "fl" | "fr") { h - p[1] } else { p[1] }]
}

#[derive(Debug, Clone)]
struct JobPath {
    pts: Vec<P>,
    tabs: Option<Vec<f64>>,
    fixed: bool,
}

fn jp(pts: Vec<P>, src: &JobPath, reverse: bool, shift: f64) -> JobPath {
    let tabs = src.tabs.as_ref().filter(|t| !t.is_empty()).map(|t| {
        let t: Vec<f64> = t.iter().map(|v| (v - shift).rem_euclid(1.0)).collect();
        if reverse { t.iter().map(|v| 1.0 - v).collect() } else { t }
    });
    JobPath { pts, tabs: tabs.or_else(|| src.tabs.clone()), fixed: src.fixed }
}

/// mainwindow.optimize_paths: inner shapes first, nearest next, best start point and direction.
fn optimize_paths(lst: Vec<JobPath>, mut cur: P, o: &Opt) -> (Vec<JobPath>, P) {
    let boxes: Vec<[f64; 4]> = lst.iter().map(|p| path_box(&p.pts)).collect();
    let groups: Vec<Vec<JobPath>> = if o.inner_first && lst.len() < 4000 {
        let depth = |i: usize| {
            let b = boxes[i];
            boxes.iter().enumerate().filter(|(j, q)| *j != i && q[0] <= b[0] && q[1] <= b[1] && q[0] + q[2] >= b[0] + b[2]
                && q[1] + q[3] >= b[1] + b[3] && q[2] * q[3] > b[2] * b[3]).count()
        };
        let depths: Vec<usize> = (0..lst.len()).map(depth).collect();
        let mut order: Vec<usize> = (0..lst.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(depths[i]));
        let (mut groups, mut band, mut last) = (Vec::new(), Vec::new(), None);
        for i in order {
            if last.is_some() && Some(depths[i]) != last {
                groups.push(std::mem::take(&mut band));
            }
            band.push(lst[i].clone());
            last = Some(depths[i]);
        }
        if !band.is_empty() {
            groups.push(band);
        }
        groups
    } else {
        vec![lst.clone()]
    };
    if !o.nearest {
        let out: Vec<JobPath> = groups.into_iter().flatten().collect();
        let end = out.last().and_then(|p| p.pts.last().copied()).unwrap_or(cur);
        return (out, end);
    }
    let heavy = lst.iter().map(|p| p.pts.len()).sum::<usize>() * lst.len() > 4_000_000;
    let d = |a: P, b: P| (a[0] - b[0]).hypot(a[1] - b[1]);
    let mut out = Vec::new();
    for mut pool in groups {
        while !pool.is_empty() {
            let (mut best, mut best_d, mut mode) = (0usize, f64::INFINITY, (0u8, 0usize));
            for (i, p) in pool.iter().enumerate() {
                let closed = is_closed_path(&p.pts);
                if closed && o.best_start && !p.fixed && !heavy && p.pts.len() <= 400 {
                    let (mut k, mut dk) = (0usize, f64::INFINITY);
                    for (j, q) in p.pts[..p.pts.len() - 1].iter().enumerate() {
                        let dd = d(*q, cur);
                        if dd < dk {
                            k = j;
                            dk = dd;
                        }
                    }
                    if dk < best_d {
                        best = i;
                        best_d = dk;
                        mode = (2, k);
                    }
                    continue;
                }
                let d0 = d(p.pts[0], cur);
                if d0 < best_d {
                    best = i;
                    best_d = d0;
                    mode = (0, 0);
                }
                if !closed && o.best_direction {
                    let d1 = d(*p.pts.last().unwrap(), cur);
                    if d1 < best_d {
                        best = i;
                        best_d = d1;
                        mode = (1, 0);
                    }
                }
            }
            let mut p = pool.remove(best);
            if mode.0 == 1 {
                let mut r = p.pts.clone();
                r.reverse();
                p = jp(r, &p, true, 0.0);
            } else if mode.0 == 2 && mode.1 > 0 {
                let (k, n) = (mode.1, p.pts.len());
                let mut r: Vec<P> = p.pts[k..n - 1].to_vec();
                r.extend_from_slice(&p.pts[..k]);
                r.push(p.pts[k]);
                p = jp(r, &p, false, k as f64 / (n - 1).max(1) as f64);
            }
            cur = *p.pts.last().unwrap();
            out.push(p);
        }
    }
    (out, cur)
}

pub struct BuildReport {
    pub n_out: usize,
    pub n_clip: usize,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Doc {
    pub shapes: Vec<Shape>,
    pub layers: Vec<LayerSettings>,
    pub pages: Vec<Value>,
    pub page: usize,
    pub next_id: i64,
    pub notes: Value,
    /// Selected top-level shape ids, in click order.
    pub selected: Vec<i64>,
    /// Layer new shapes go to.
    pub active_layer: usize,
    pub undo: Vec<(Vec<Shape>, String)>,
    pub redo: Vec<(Vec<Shape>, String)>,
    pub dirty: bool,
}

impl Default for Doc {
    fn default() -> Self {
        Doc { shapes: Vec::new(), layers: default_layers(), pages: vec![json!({"name": "Pagina 1", "shapes": []})], page: 0,
              next_id: 1, notes: json!({}), selected: Vec::new(), active_layer: 0, undo: Vec::new(), redo: Vec::new(), dirty: false }
    }
}

fn area_state(s: &Shape, w: f64, h: f64) -> u8 {
    let b = s.bounds();
    let eps = 1e-6;
    if b[0] > w + eps || b[1] > h + eps || b[0] + b[2] < -eps || b[1] + b[3] < -eps {
        2
    } else if b[0] < -eps || b[1] < -eps || b[0] + b[2] > w + eps || b[1] + b[3] > h + eps {
        1
    } else {
        0
    }
}

impl Doc {
    pub fn new_id(&mut self) -> i64 {
        let i = self.next_id;
        self.next_id += 1;
        i
    }

    fn output_layer(&self, l: i64) -> bool {
        l >= 0 && (l as usize) < self.layers.len() && !self.layers[l as usize].tool
    }

    pub fn layer_interval(&self, l: usize) -> f64 {
        self.layers.get(l).map_or(0.1, |x| x.line_interval).max(0.05)
    }

    /// mainwindow.build_job. `raster` processes an image shape into (burn, cols, rows) for an interval and pass-through.
    pub fn build_job(&self, m: &MachineSettings, jo: &JobOpts,
                     raster: &mut dyn FnMut(&Shape, f64, bool) -> Option<(Vec<u8>, usize, usize)>) -> (Job, BuildReport) {
        let (w, h) = (m.workspace_x, m.workspace_y);
        let opt = &jo.opt;
        let pick = |ids: &Option<Vec<i64>>| -> Vec<&Shape> {
            self.shapes.iter().filter(|s| ids.as_ref().map_or(true, |o| o.contains(&s.id()))).collect()
        };
        let src = pick(&jo.only);
        // (group index, priority, layer, path)
        let mut entries: Vec<(usize, i64, usize, JobPath)> = Vec::new();
        let mut rasters: Vec<(usize, Raster)> = Vec::new();
        let mut rep = BuildReport { n_out: 0, n_clip: 0, skipped: Vec::new() };
        for (gi, top) in src.iter().enumerate() {
            for s in flatten(std::slice::from_ref(*top)) {
                if !self.output_layer(s.layer()) {
                    continue;
                }
                let layer = s.layer() as usize;
                let area = area_state(&s, w, h);
                if area == 2 {
                    rep.n_out += 1;
                    continue;
                }
                rep.n_clip += (area == 1) as usize;
                if s.kind() == "image" {
                    if let Some((mut burn, cols, rows)) = raster(&s, self.layer_interval(layer), self.layers[layer].pass_through) {
                        let (x, y, sw, sh) = (s.f("x"), s.f("y"), s.f("w"), s.f("h"));
                        if let Some(rings) = s.0.get("crop").and_then(|c| serde_json::from_value::<Vec<Vec<P>>>(c.clone()).ok()) {
                            let mask = crop_mask(&rings, cols, rows);
                            for (b, k) in burn.iter_mut().zip(mask) {
                                *b *= k;
                            }
                        }
                        if area == 1 {
                            for r in 0..rows {
                                let yy = y + (r as f64 + 0.5) * sh / rows as f64;
                                for c in 0..cols {
                                    let xx = x + (c as f64 + 0.5) * sw / cols as f64;
                                    if xx < 0.0 || xx > w || yy < 0.0 || yy > h {
                                        burn[r * cols + c] = 0;
                                    }
                                }
                            }
                        }
                        let adj = s.0.get("adj");
                        let mode = adj.and_then(|a| a.get("mode")).and_then(|v| v.as_str()).unwrap_or("dither");
                        let mk = |data: Vec<u8>| Raster { x, y, w: sw, h: sh, cols, rows, data, angle: None };
                        if mode == "slices3d" {
                            let n = adj.and_then(|a| a.get("slices")).and_then(|v| v.as_f64()).unwrap_or(10.0).max(2.0) as usize;
                            for k in 0..n {
                                let thr = (k as f64 + 0.5) * 255.0 / n as f64;
                                let pass: Vec<u8> = burn.iter().map(|&b| if b as f64 > thr { 255 } else { 0 }).collect();
                                if pass.iter().any(|&b| b > 0) {
                                    rasters.push((layer, mk(pass)));
                                }
                            }
                        } else {
                            rasters.push((layer, mk(burn)));
                        }
                    }
                    continue;
                }
                let prio = s.0.get("priority").or_else(|| top.0.get("priority")).and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
                let mut raw: Vec<Vec<P>> = s.paths().into_iter().filter(|p| p.len() > 1).collect();
                if area == 1 {
                    let fill = self.layers[layer].cut_mode != CutMode::Line;
                    let pts: Vec<Vec<Pt>> = raw.iter().map(|p| p.iter().map(|q| Pt { x: q[0], y: q[1] }).collect()).collect();
                    raw = clip_paths(&pts, w, h, fill).into_iter().map(|p| p.iter().map(|q| [q.x, q.y]).collect::<Vec<P>>())
                        .filter(|p| p.len() > 1).collect();
                    if fill {
                        orient_like_geos(&mut raw);
                    }
                }
                let (start, reverse) = (s.f("start"), s.flag("reverse"));
                let tabs: Vec<f64> = s.0.get("tabs").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                for (k, mut pts) in raw.into_iter().enumerate() {
                    if area == 0 {
                        if is_closed_path(&pts) && start != 0.0 {
                            pts = rotate_closed_path(&pts, start);
                        }
                        if reverse {
                            pts.reverse();
                        }
                    }
                    let mut j = JobPath { pts, tabs: None, fixed: area == 0 && (start != 0.0 || s.flag("tabs")) };
                    if area == 0 && k == 0 && s.flag("tabs") && is_closed_path(&j.pts) {
                        let fr: Vec<f64> = tabs.iter().map(|t| (t - start).rem_euclid(1.0)).collect();
                        j.tabs = Some(if reverse { fr.iter().map(|t| 1.0 - t).collect() } else { fr });
                    }
                    entries.push((gi, prio, layer, j));
                }
            }
        }
        // job_transform: print & cut registration, or the job box anchored on the head / user origin
        let pc = jo.print_cut.as_ref().filter(|p| p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false));
        let pc_pts = pc.and_then(|p| {
            let s = p.get("src")?.as_array()?;
            let d = p.get("dst")?.as_array()?;
            Some((pair(s.first()?)?, pair(s.get(1)?)?, pair(d.first()?)?, pair(d.get(1)?)?))
        });
        let mut tf: Option<Box<dyn Fn(P) -> P>> = None;
        if let Some((s1, s2, d1, d2)) = pc_pts {
            let (vs, vd) = ([s2[0] - s1[0], s2[1] - s1[1]], [d2[0] - d1[0], d2[1] - d1[1]]);
            let hs = vs[0].hypot(vs[1]);
            let k = vd[0].hypot(vd[1]) / if hs == 0.0 { 1.0 } else { hs };
            let a = vd[1].atan2(vd[0]) - vs[1].atan2(vs[0]);
            let (c, sn) = (a.cos() * k, a.sin() * k);
            tf = Some(Box::new(move |p: P| [d1[0] + (p[0] - s1[0]) * c - (p[1] - s1[1]) * sn,
                                             d1[1] + (p[0] - s1[0]) * sn + (p[1] - s1[1]) * c]));
        } else if jo.start_from == "current" || jo.start_from == "user" {
            let origin_src = if jo.origin_ids.is_some() { pick(&jo.origin_ids) } else { src.clone() };
            let outs: Vec<Shape> = origin_src.iter().flat_map(|t| flatten(std::slice::from_ref(*t)))
                .filter(|s| self.output_layer(s.layer()) && area_state(s, w, h) != 2).collect();
            if let Some(b) = multi_bounds(&outs) {
                let (x0, y0, x1, y1) = (b[0].max(0.0), b[1].max(0.0), (b[0] + b[2]).min(w), (b[1] + b[3]).min(h));
                if x1 > x0 && y1 > y0 {
                    let (fx, fy) = ORIGIN_POS[jo.anchor.min(8)];
                    let t = to_machine(m, if jo.start_from == "current" { jo.head } else { jo.user_origin });
                    let (dx, dy) = (t[0] - (x0 + (x1 - x0) * fx), t[1] - (y0 + (y1 - y0) * fy));
                    tf = Some(Box::new(move |p: P| [p[0] + dx, p[1] + dy]));
                }
            }
        }
        let cyl = jo.cylinder.as_ref().and_then(cylinder_transform);
        let mut fixed: Vec<(usize, i64, usize, JobPath)> = Vec::new();
        for (gi, prio, layer, j) in entries {
            let mut pts = j.pts.clone();
            if let Some(f) = &tf {
                pts = pts.into_iter().map(|p| f(p)).collect();
            }
            if let Some(f) = &cyl {
                pts = pts.into_iter().map(|p| f(p)).collect();
            }
            pts = pts.into_iter().map(|p| to_machine(m, p)).collect();
            let mut new = jp(pts.clone(), &j, false, 0.0);
            if opt.direction != "keep" && is_closed_path(&pts) && !j.fixed {
                let cw = signed_area(&pts) > 0.0;
                if (opt.direction == "cw") != cw {
                    let mut r = pts;
                    r.reverse();
                    new = jp(r, &new, true, 0.0);
                }
            }
            fixed.push((gi, prio, layer, new));
        }
        // buckets in the chosen order: (first key, layer)
        let key = |e: &(usize, i64, usize, JobPath)| -> (i64, usize) {
            match opt.order.as_str() {
                "group" => (e.0 as i64, e.2),
                "priority" => (e.1, e.2),
                _ => (0, e.2),
            }
        };
        let mut buckets: std::collections::BTreeMap<(i64, usize), Vec<JobPath>> = Default::default();
        for e in fixed {
            let k = key(&e);
            buckets.entry(k).or_default().push(e.3);
        }
        let mut layers = Vec::new();
        let mut cur: P = [0.0, 0.0];
        for ((_, li), mut paths) in buckets {
            if opt.remove_overlap {
                // mainwindow._remove_overlaps: lines without tabs merged so each overlapping piece is cut once
                let (keep, lines): (Vec<JobPath>, Vec<JobPath>) = paths.into_iter().partition(|p| p.tabs.as_ref().is_some_and(|t| !t.is_empty()));
                if lines.iter().filter(|p| p.pts.len() >= 2).count() >= 2 {
                    let reference = lines[0].clone();
                    let merged = crate::edit::merge_lines(&lines.iter().map(|p| p.pts.clone()).collect::<Vec<_>>());
                    paths = keep;
                    for m in merged {
                        let pts: Vec<P> = m.iter().map(|q| [(q[0] * 1e6).round() / 1e6, (q[1] * 1e6).round() / 1e6]).collect();
                        if pts.len() >= 2 {
                            paths.push(jp(pts, &reference, false, 0.0));
                        }
                    }
                } else {
                    paths = keep.into_iter().chain(lines).collect();
                }
            }
            let (ordered, end) = optimize_paths(paths, cur, opt);
            cur = end;
            layers.push(JobLayer { settings: self.layers[li].clone(), paths: ordered.iter().map(|p| p.pts.clone()).collect(),
                                   tabs: ordered.iter().map(|p| p.tabs.clone()).collect() });
        }
        let o = m.origin.as_str();
        let (flip_x, flip_y) = (matches!(o, "rr" | "fr"), matches!(o, "fl" | "fr"));
        let job_rasters = rasters.into_iter().map(|(li, mut r)| {
            if let Some(f) = &tf {
                let q = f([r.x, r.y]);
                r.x = q[0];
                r.y = q[1];
            }
            if flip_x {
                r.x = w - (r.x + r.w);
                for row in r.data.chunks_mut(r.cols) {
                    row.reverse();
                }
            }
            if flip_y {
                r.y = h - (r.y + r.h);
                r.data = r.data.chunks(r.cols).rev().flatten().copied().collect();
            }
            JobRaster { settings: self.layers[li].clone(), raster: r }
        }).collect();
        let mut job = Job { machine: m.clone(), layers, rasters: job_rasters, origin_mode: None, start: None };
        if tf.is_some() && pc_pts.is_none() && (jo.start_from == "current" || jo.start_from == "user") {
            job.origin_mode = Some(jo.start_from.clone());
            job.start = Some(if jo.start_from == "current" { jo.head } else { jo.user_origin });
        }
        (job, rep)
    }

    /// Read a `.lfproj`; returns the work area it was saved with.
    pub fn from_project(text: &str) -> Result<(Doc, Option<(f64, f64)>), String> {
        let d: Value = serde_json::from_str(text).map_err(|e| format!("JSON non valido: {e}"))?;
        let mut doc = Doc::default();
        if let Some(ls) = d.get("layers").and_then(|v| v.as_array()) {
            for (i, l) in ls.iter().enumerate().take(doc.layers.len()) {
                if let Some(m) = l.as_object() {
                    apply_layer_dict(&mut doc.layers[i], m);
                }
            }
        }
        doc.pages = match d.get("pages").and_then(|v| v.as_array()) {
            Some(p) if !p.is_empty() => p.clone(),
            _ => vec![json!({"name": "Pagina 1", "shapes": d.get("shapes").cloned().unwrap_or(json!([]))})],
        };
        doc.load_page(0);
        let max_id = doc.pages.iter().flat_map(|p| p.get("shapes").and_then(|s| s.as_array()).cloned().unwrap_or_default())
            .filter_map(|s| s.get("id").and_then(|v| v.as_i64())).max().unwrap_or(0);
        doc.next_id = d.get("nextId").and_then(|v| v.as_i64()).filter(|&n| n > 0).unwrap_or(max_id + 1);
        doc.notes = d.get("notes").cloned().unwrap_or(json!({}));
        let ws = d.get("workspace").and_then(|w| Some((w.get("x")?.as_f64()?, w.get("y")?.as_f64()?)));
        Ok((doc, ws))
    }

    pub fn load_page(&mut self, i: usize) {
        self.page = i.min(self.pages.len().saturating_sub(1));
        self.shapes = self.pages.get(self.page).and_then(|p| p.get("shapes"))
            .and_then(|s| serde_json::from_value(s.clone()).ok()).unwrap_or_default();
    }

    /// Same layout the Python app writes (version 4).
    pub fn to_project(&mut self, w: f64, h: f64) -> String {
        let shapes = serde_json::to_value(&self.shapes).unwrap();
        if let Some(Value::Object(p)) = self.pages.get_mut(self.page) {
            p.insert("shapes".into(), shapes);
        }
        let layers: Vec<Value> = self.layers.iter().map(|l| serde_json::to_value(l).unwrap()).collect();
        let first = self.pages.first().and_then(|p| p.get("shapes")).cloned().unwrap_or(json!([]));
        serde_json::to_string(&json!({"app": "LaserForge", "version": 4, "workspace": {"x": w, "y": h}, "layers": layers,
                                      "shapes": first, "pages": self.pages, "nextId": self.next_id, "notes": self.notes})).unwrap()
    }
}

/// model.MAX_RASTER_PX: safety cap on raster size per side.
pub const MAX_RASTER_PX: usize = 16000;

/// model.process_image without the cache: decode, resample to the interval, adjust, dither, alpha mask.
pub fn process_image(s: &Shape, interval: f64, pass_through: bool) -> Option<(Vec<u8>, usize, usize)> {
    use laserforge_core::imaging;
    let pic = imaging::decode(s.0.get("src")?.as_str()?)?;
    let adj: imaging::Adj = s.0.get("adj").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default();
    Some(imaging::process(&pic, s.f("w"), s.f("h"), interval, MAX_RASTER_PX, pass_through, s.flag("flipX"), s.flag("flipY"), &adj))
}

/// Cache of processed images keyed on everything that changes the result.
#[derive(Default)]
pub struct ImageCache(std::collections::HashMap<String, Option<(Vec<u8>, usize, usize)>>);

impl ImageCache {
    pub fn get(&mut self, s: &Shape, interval: f64, pass_through: bool) -> Option<(Vec<u8>, usize, usize)> {
        let src = s.0.get("src").and_then(|v| v.as_str()).unwrap_or("");
        let key = format!("{}|{}|{}|{}|{}|{}|{}|{}|{}", src.len(), &src[src.len().saturating_sub(64)..], s.f("w"), s.f("h"),
                          interval, pass_through, s.flag("flipX"), s.flag("flipY"), s.0.get("adj").map(|a| a.to_string()).unwrap_or_default());
        if self.0.len() > 40 {
            self.0.clear();
        }
        self.0.entry(key).or_insert_with(|| process_image(s, interval, pass_through)).clone()
    }
}
