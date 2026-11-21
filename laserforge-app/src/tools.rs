//! Interactive canvas tools and node operations: node editor, scissors, trim at intersections, manual tabs,
//! start point / direction, two-point rotate & scale, deform (4 corners) and warp (3×3 grid), plus the path
//! helpers they use. Ported from laserforge/tools.py, laserforge/nodes.py and the node functions of model.py.

use crate::canvas::{Canvas, Env, ToolHandler};
use crate::edit::{map_point, rotate_shape, scale_shape, to_poly};
use crate::project::Doc;
use crate::shapes::{is_closed_path, multi_bounds, rotate_closed_path, simplify, Shape, P};
use crate::{l, tr};
use egui::{vec2, Color32, Rect, Stroke, StrokeKind};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const PURPLE: Color32 = Color32::from_rgb(0x8e, 0x44, 0xad);
const ORANGE: Color32 = Color32::from_rgb(0xff, 0x98, 0x00);
const BLUE: Color32 = Color32::from_rgb(0x2f, 0x7b, 0xf5);
const RED: Color32 = Color32::from_rgb(0xff, 0x1a, 0x1a);

// ====================================================================== path helpers
pub fn path_length(p: &[P]) -> f64 {
    p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// Distance along the path of the point nearest to pt (shapely project).
pub fn project(p: &[P], pt: P) -> f64 {
    let (mut best, mut best_d, mut acc) = (0.0, f64::INFINITY, 0.0);
    for w in p.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 { (((pt[0] - w[0][0]) * dx + (pt[1] - w[0][1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [w[0][0] + dx * t, w[0][1] + dy * t];
        let d = (q[0] - pt[0]).hypot(q[1] - pt[1]);
        if d < best_d {
            best_d = d;
            best = acc + t * l2.sqrt();
        }
        acc += l2.sqrt();
    }
    best
}

pub fn fraction_at(p: &[P], pt: P) -> f64 {
    let l = path_length(p);
    if l > 0.0 { project(p, pt) / l } else { 0.0 }
}

pub fn point_at(p: &[P], d: f64) -> P {
    let mut d = d.max(0.0);
    for w in p.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        if d <= l && l > 0.0 {
            let k = d / l;
            return [w[0][0] + (w[1][0] - w[0][0]) * k, w[0][1] + (w[1][1] - w[0][1]) * k];
        }
        d -= l;
    }
    *p.last().unwrap_or(&[0.0, 0.0])
}

pub fn point_at_fraction(p: &[P], f: f64) -> P {
    point_at(p, f * path_length(p))
}

/// The part of the path between distances a and b (shapely substring).
pub fn substring(p: &[P], a: f64, b: f64) -> Vec<P> {
    let (a, b) = (a.max(0.0), b.min(path_length(p)));
    if b <= a {
        return vec![point_at(p, a)];
    }
    let mut out = vec![point_at(p, a)];
    let mut acc = 0.0;
    for w in p.windows(2) {
        let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        acc += l;
        if acc > a && acc < b {
            out.push(w[1]);
        }
    }
    out.push(point_at(p, b));
    out.dedup();
    out
}

pub fn densify(p: &[P], step: f64) -> Vec<P> {
    let mut out = vec![p[0]];
    for w in p.windows(2) {
        let n = 1.max(((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) / step) as usize);
        for k in 1..=n {
            out.push([w[0][0] + (w[1][0] - w[0][0]) * k as f64 / n as f64, w[0][1] + (w[1][1] - w[0][1]) * k as f64 / n as f64]);
        }
    }
    out
}

fn dist_path(p: &[P], pt: P) -> f64 {
    p.windows(2).map(|w| crate::shapes::dist_to_segment(pt, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// Nearest top-level shape outline within 6 px → (shape id, path).
pub fn shape_hit(c: &Canvas, doc: &Doc, pt: P, kinds: &[&str]) -> Option<(i64, Vec<P>)> {
    let tol = 6.0 / c.zoom as f64;
    let mut best: Option<(i64, Vec<P>, f64)> = None;
    for s in doc.shapes.iter().rev() {
        if s.locked() || !kinds.contains(&s.kind()) {
            continue;
        }
        for path in s.paths() {
            if path.len() < 2 {
                continue;
            }
            let d = dist_path(&path, pt);
            if d < tol && best.as_ref().is_none_or(|b| d < b.2) {
                best = Some((s.id(), path, d));
            }
        }
    }
    best.map(|b| (b.0, b.1))
}

/// tools.job_path: a shape's start point and direction applied to one of its paths.
pub fn job_path(s: &Shape, path: &[P]) -> Vec<P> {
    let mut p = path.to_vec();
    if is_closed_path(&p) && s.f("start") != 0.0 {
        p = rotate_closed_path(&p, s.f("start"));
    }
    if s.flag("reverse") {
        p.reverse();
    }
    p
}

// ====================================================================== node operations (model.py)
fn num(v: Option<&Value>) -> f64 {
    v.and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn pt2(v: Option<&Value>) -> Option<P> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

pub fn pts(s: &Shape) -> Vec<Map<String, Value>> {
    s.0.get("points").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|p| p.as_object().cloned()).collect()).unwrap_or_default()
}

pub fn set_pts(s: &mut Shape, p: Vec<Map<String, Value>>) {
    s.0.insert("points".into(), Value::Array(p.into_iter().map(Value::Object).collect()));
}

fn xy(p: &Map<String, Value>) -> P {
    [num(p.get("x")), num(p.get("y"))]
}

pub fn seg_count(s: &Shape) -> usize {
    let n = pts(s).len();
    if s.flag("closed") && n > 2 { n } else { n.saturating_sub(1) }
}

fn ends(p: &[Map<String, Value>], i: usize) -> (usize, usize) {
    (i, (i + 1) % p.len())
}

pub fn is_curve_seg(s: &Shape, i: usize) -> bool {
    let p = pts(s);
    let (a, b) = ends(&p, i);
    p[a].contains_key("c2") || p[b].contains_key("c1")
}

fn lerp(p: P, q: P, t: f64) -> P {
    [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
}

pub fn seg_point(s: &Shape, i: usize, t: f64) -> P {
    let p = pts(s);
    let (a, b) = ends(&p, i);
    let (p0, p3) = (xy(&p[a]), xy(&p[b]));
    if !is_curve_seg(s, i) {
        return lerp(p0, p3, t);
    }
    let (p1, p2) = (pt2(p[a].get("c2")).unwrap_or(p0), pt2(p[b].get("c1")).unwrap_or(p3));
    let u = 1.0 - t;
    let f = |a: f64, b: f64, c: f64, d: f64| u.powf(3.0) * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t.powf(3.0) * d;
    [f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])]
}

/// (distance, t, point) of the closest point of segment i to pt.
pub fn nearest_on_segment(s: &Shape, i: usize, pt: P) -> (f64, f64, P) {
    let n = if is_curve_seg(s, i) { 48 } else { 1 };
    let mut best = (f64::INFINITY, 0.0, pt);
    let mut prev = seg_point(s, i, 0.0);
    for k in 1..=n {
        let cur = seg_point(s, i, k as f64 / n as f64);
        let (dx, dy) = (cur[0] - prev[0], cur[1] - prev[1]);
        let l2 = dx * dx + dy * dy;
        let u = if l2 > 0.0 { (((pt[0] - prev[0]) * dx + (pt[1] - prev[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let q = [prev[0] + dx * u, prev[1] + dy * u];
        let d = (pt[0] - q[0]).hypot(pt[1] - q[1]);
        if d < best.0 {
            best = (d, (k as f64 - 1.0 + u) / n as f64, q);
        }
        prev = cur;
    }
    best
}

/// Insert a node on segment i at t (exact de Casteljau split for curves); returns its index.
pub fn split_segment(s: &mut Shape, i: usize, t: f64) -> usize {
    let mut p = pts(s);
    let (a, b) = ends(&p, i);
    let j = i + 1;
    let (p0, p3) = (xy(&p[a]), xy(&p[b]));
    if !is_curve_seg(s, i) {
        let m = lerp(p0, p3, t);
        p.insert(j, serde_json::from_value(json!({"x": m[0], "y": m[1]})).unwrap());
    } else {
        let (p1, p2) = (pt2(p[a].get("c2")).unwrap_or(p0), pt2(p[b].get("c1")).unwrap_or(p3));
        let (q0, q1, q2) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
        let (r0, r1) = (lerp(q0, q1, t), lerp(q1, q2, t));
        let m = lerp(r0, r1, t);
        p[a].insert("c2".into(), json!(q0));
        p[b].insert("c1".into(), json!(q2));
        p.insert(j, serde_json::from_value(json!({"x": m[0], "y": m[1], "c1": r0, "c2": r1, "smooth": true})).unwrap());
    }
    set_pts(s, p);
    j
}

pub fn delete_nodes(s: &mut Shape, idx: &[usize]) -> bool {
    let keep: Vec<Map<String, Value>> = pts(s).into_iter().enumerate().filter(|(k, _)| !idx.contains(k)).map(|x| x.1).collect();
    let ok = keep.len() >= 2;
    set_pts(s, keep);
    s.0.remove("regular");
    ok
}

pub fn make_curve(s: &mut Shape, i: usize) {
    let mut p = pts(s);
    let (a, b) = ends(&p, i);
    let (p0, p3) = (xy(&p[a]), xy(&p[b]));
    p[a].entry("c2").or_insert(json!(lerp(p0, p3, 1.0 / 3.0)));
    p[b].entry("c1").or_insert(json!(lerp(p0, p3, 2.0 / 3.0)));
    set_pts(s, p);
    s.0.remove("regular");
}

pub fn make_line(s: &mut Shape, i: usize) {
    let mut p = pts(s);
    let (a, b) = ends(&p, i);
    p[a].remove("c2");
    p[b].remove("c1");
    set_pts(s, p);
    s.0.remove("regular");
}

pub fn set_smooth(s: &mut Shape, idx: usize, smooth: bool) {
    let mut p = pts(s);
    s.0.remove("regular");
    if !smooth {
        p[idx].remove("smooth");
        set_pts(s, p);
        return;
    }
    let n = p.len();
    let closed = s.flag("closed") && n > 2;
    let prev = if idx > 0 || closed { Some(xy(&p[(idx + n - 1) % n])) } else { None };
    let nxt = if idx < n - 1 || closed { Some(xy(&p[(idx + 1) % n])) } else { None };
    let c = xy(&p[idx]);
    let (dx, dy) = match (prev, nxt) {
        (None, None) => return,
        (Some(a), Some(b)) => (b[0] - a[0], b[1] - a[1]),
        (None, Some(b)) => (b[0] - c[0], b[1] - c[1]),
        (Some(a), None) => (c[0] - a[0], c[1] - a[1]),
    };
    let ln = { let l = dx.hypot(dy); if l == 0.0 { 1.0 } else { l } };
    let (ux, uy) = (dx / ln, dy / ln);
    if let Some(a) = prev {
        let lin = (c[0] - a[0]).hypot(c[1] - a[1]) / 3.0;
        p[idx].insert("c1".into(), json!([c[0] - ux * lin, c[1] - uy * lin]));
    }
    if let Some(b) = nxt {
        let lout = (b[0] - c[0]).hypot(b[1] - c[1]) / 3.0;
        p[idx].insert("c2".into(), json!([c[0] + ux * lout, c[1] + uy * lout]));
    }
    p[idx].insert("smooth".into(), json!(true));
    set_pts(s, p);
}

/// A smooth node's opposite handle stays collinear (keeping its own length).
pub fn mirror_handle(p: &mut Map<String, Value>, moved: &str) {
    let other = if moved == "c2" { "c1" } else { "c2" };
    if !p.get("smooth").and_then(|v| v.as_bool()).unwrap_or(false) || !p.contains_key(other) {
        return;
    }
    let c = xy(p);
    let (Some(m), Some(o)) = (pt2(p.get(moved)), pt2(p.get(other))) else { return };
    let (hx, hy) = (m[0] - c[0], m[1] - c[1]);
    let ln = hx.hypot(hy);
    if ln < 1e-9 {
        return;
    }
    let ol = (o[0] - c[0]).hypot(o[1] - c[1]);
    p.insert(other.into(), json!([c[0] - hx / ln * ol, c[1] - hy / ln * ol]));
}

/// Closed path: opened at the node. Open path: split in two. Returns the point lists.
pub fn break_at_node(s: &Shape, idx: usize) -> Vec<Vec<Map<String, Value>>> {
    let p = pts(s);
    if s.flag("closed") && p.len() > 2 {
        let mut rot: Vec<Map<String, Value>> = p[idx..].iter().chain(&p[..idx]).cloned().collect();
        rot[0].remove("c1");
        let mut last = p[idx].clone();
        last.remove("c2");
        rot.push(last);
        return vec![rot];
    }
    if idx == 0 || idx >= p.len() - 1 {
        return vec![p];
    }
    let mut a = p[..=idx].to_vec();
    let mut b: Vec<Map<String, Value>> = std::iter::once(p[idx].clone()).chain(p[idx + 1..].iter().cloned()).collect();
    a.last_mut().unwrap().remove("c2");
    b[0].remove("c1");
    vec![a, b]
}

pub fn delete_segment(s: &Shape, i: usize) -> Vec<Vec<Map<String, Value>>> {
    let p = pts(s);
    let n = p.len();
    if s.flag("closed") && n > 2 {
        let mut rot: Vec<Map<String, Value>> = p[i + 1..].iter().chain(&p[..=i]).cloned().collect();
        rot[0].remove("c1");
        rot.last_mut().unwrap().remove("c2");
        return vec![rot];
    }
    let mut a = p[..=i.min(n - 1)].to_vec();
    let mut b = p[(i + 1).min(n)..].to_vec();
    if let Some(x) = a.last_mut() {
        x.remove("c2");
    }
    if let Some(x) = b.first_mut() {
        x.remove("c1");
    }
    [a, b].into_iter().filter(|x| x.len() >= 2).collect()
}

/// Replace a shape by open paths made from `parts` (first keeps the id).
fn replace_with_parts(doc: &mut Doc, sid: i64, parts: Vec<Vec<Map<String, Value>>>, label: &str) {
    if doc.shapes.iter().any(|s| s.id() == sid) {
        doc.save_undo(label);
    }
    replace_parts_no_undo(doc, sid, parts);
}

fn replace_parts_no_undo(doc: &mut Doc, sid: i64, parts: Vec<Vec<Map<String, Value>>>) {
    let Some(idx) = doc.shapes.iter().position(|s| s.id() == sid) else { return };
    let s = doc.shapes[idx].clone();
    let mut new = Vec::new();
    for (k, p) in parts.into_iter().enumerate() {
        let mut ns = s.clone();
        for key in ["points", "regular", "tabs", "start"] {
            ns.0.remove(key);
        }
        set_pts(&mut ns, p);
        ns.0.insert("closed".into(), json!(false));
        if k > 0 {
            ns.0.insert("id".into(), json!(doc.new_id()));
        }
        new.push(ns);
    }
    doc.selected = new.iter().map(|n| n.id()).collect();
    doc.shapes.splice(idx..idx + 1, new);
}

// ====================================================================== tabs and start markers
pub fn draw_tabs(c: &Canvas, doc: &Doc, p: &egui::Painter) {
    draw_tabs_of(c, doc, p, false);
}

fn draw_tabs_of(c: &Canvas, doc: &Doc, p: &egui::Painter, all: bool) {
    for s in &doc.shapes {
        let Some(tabs) = s.0.get("tabs").and_then(|v| v.as_array()) else { continue };
        if tabs.is_empty() || (!all && !doc.is_selected(s.id())) {
            continue;
        }
        let Some(path) = s.paths().into_iter().next() else { continue };
        let l = path_length(&path);
        let width = doc.layers.get(s.layer().max(0) as usize).map_or(2.0, |x| x.tab_width);
        for f in tabs.iter().filter_map(|v| v.as_f64()) {
            let seg = substring(&path, (f * l - width / 2.0).max(0.0), (f * l + width / 2.0).min(l));
            if seg.len() >= 2 {
                c.polyline(p, &seg, Stroke::new(4.0, ORANGE), false);
            }
        }
    }
}

// ====================================================================== scissors
pub struct Scissors;

impl ToolHandler for Scissors {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env, pt: P, _shift: bool) {
        let Some((sid, _)) = shape_hit(c, doc, pt, &["rect", "circle", "poly"]) else { return };
        let idx = doc.shapes.iter().position(|s| s.id() == sid).unwrap();
        let mut s = to_poly(&doc.shapes[idx]);
        let n = seg_count(&s);
        let mut best = (f64::INFINITY, 0, 0.0);
        for i in 0..n {
            let (d, t, _) = nearest_on_segment(&s, i, pt);
            if d < best.0 {
                best = (d, i, t);
            }
        }
        let (_, i, t) = best;
        let np = pts(&s).len();
        let j = if t < 1e-6 { i } else if t > 1.0 - 1e-6 { (i + 1) % np } else { split_segment(&mut s, i, t) };
        doc.save_undo(&l("Forbici", "Scissors"));
        doc.shapes[idx] = s.clone();
        replace_parts_no_undo(doc, sid, break_at_node(&s, j));
        c.events.push(crate::canvas::Event::Status(l("Forma tagliata", "Shape cut")));
        c.events.push(crate::canvas::Event::Changed);
    }
}

// ====================================================================== trim
#[derive(Default)]
pub struct Trim {
    preview: Vec<Vec<P>>,
}

fn seg_intersections(a: P, b: P, c: P, d: P) -> Option<f64> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
    let u = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

type Piece = (i64, Vec<P>, Option<(Vec<Vec<P>>, Vec<Vec<P>>)>);

impl Trim {
    /// (shape, path, Some((kept pieces, removed pieces))) for the piece under the cursor.
    fn piece(&self, c: &Canvas, doc: &Doc, pt: P) -> Option<Piece> {
        let (sid, path) = shape_hit(c, doc, pt, &["rect", "circle", "poly", "group"])?;
        let len = path_length(&path);
        let mut cuts: Vec<f64> = Vec::new();
        for o in &doc.shapes {
            for op in o.paths() {
                if op.len() < 2 || (o.id() == sid && op == path) {
                    continue;
                }
                let mut acc = 0.0;
                for w in path.windows(2) {
                    let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                    for v in op.windows(2) {
                        if let Some(t) = seg_intersections(w[0], w[1], v[0], v[1]) {
                            let d = ((acc + t * l) * 1e6).round() / 1e6;
                            if d > 1e-6 && d < len - 1e-6 && !cuts.contains(&d) {
                                cuts.push(d);
                            }
                        }
                    }
                    acc += l;
                }
            }
        }
        cuts.sort_by(|a, b| a.total_cmp(b));
        let d0 = project(&path, pt);
        if is_closed_path(&path) {
            if cuts.len() < 2 {
                return Some((sid, path, None));
            }
            let a = cuts.iter().rev().find(|c| **c <= d0).copied().unwrap_or(*cuts.last().unwrap());
            let b = cuts.iter().find(|c| **c > d0).copied().unwrap_or(cuts[0]);
            let (keep, remove) = if a < b {
                let mut k = substring(&path, b, len);
                k.extend(substring(&path, 0.0, a).into_iter().skip(1));
                (vec![k], vec![substring(&path, a, b)])
            } else {
                (vec![substring(&path, b, a)], vec![substring(&path, a, len), substring(&path, 0.0, b)])
            };
            return Some((sid, path, Some((keep.into_iter().filter(|k| k.len() > 1).collect(), remove))));
        }
        let a = cuts.iter().rev().find(|c| **c <= d0).copied().unwrap_or(0.0);
        let b = cuts.iter().find(|c| **c > d0).copied().unwrap_or(len);
        let mut keep = Vec::new();
        if a > 1e-6 {
            keep.push(substring(&path, 0.0, a));
        }
        if b < len - 1e-6 {
            keep.push(substring(&path, b, len));
        }
        Some((sid, path.clone(), Some((keep.into_iter().filter(|k| k.len() > 1).collect(), vec![substring(&path, a, b)]))))
    }
}

impl ToolHandler for Trim {
    fn hover(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env, pt: P) {
        self.preview = self.piece(c, doc, pt).and_then(|p| p.2).map(|x| x.1).unwrap_or_default();
    }
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env, pt: P, _shift: bool) {
        let Some((sid, path, Some((keep, _)))) = self.piece(c, doc, pt) else {
            c.events.push(crate::canvas::Event::Status(l("Nessuna intersezione su questa linea", "No intersections on this line")));
            return;
        };
        let Some(idx) = doc.shapes.iter().position(|s| s.id() == sid) else { return };
        doc.save_undo(&l("Taglia", "Trim"));
        let s = doc.shapes[idx].clone();
        let mut rest = s.paths();
        if let Some(k) = rest.iter().position(|r| *r == path) {
            rest.remove(k);
        }
        let layer = s.layer();
        let mut new = Vec::new();
        for k in keep {
            new.push(Shape(serde_json::from_value(json!({"id": doc.new_id(), "type": "poly", "layer": layer, "closed": false,
                "points": k.iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()})).unwrap()));
        }
        for r in rest {
            let closed = is_closed_path(&r);
            let pp: &[P] = if closed { &r[..r.len() - 1] } else { &r };
            new.push(Shape(serde_json::from_value(json!({"id": doc.new_id(), "type": "poly", "layer": layer, "closed": closed,
                "points": pp.iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()})).unwrap()));
        }
        doc.selected = new.iter().map(|n| n.id()).collect();
        doc.shapes.splice(idx..idx + 1, new);
        self.preview.clear();
        c.events.push(crate::canvas::Event::Changed);
    }
    fn draw(&self, c: &Canvas, _doc: &Doc, _env: &Env, p: &egui::Painter) {
        for g in &self.preview {
            c.polyline(p, g, Stroke::new(3.0, RED), false);
        }
    }
    fn reset(&mut self) {
        self.preview.clear();
    }
}

// ====================================================================== manual tabs
pub struct Tabs;

impl ToolHandler for Tabs {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env, pt: P, _shift: bool) {
        let Some((sid, path)) = shape_hit(c, doc, pt, &["rect", "circle", "poly"]) else { return };
        if !is_closed_path(&path) {
            c.events.push(crate::canvas::Event::Status(l("I ponticelli servono sulle forme chiuse", "Tabs need closed shapes")));
            return;
        }
        let f = fraction_at(&path, pt);
        let len = path_length(&path);
        let zoom = c.zoom as f64;
        doc.save_undo(&l("Ponticelli", "Tabs"));
        let s = doc.find_mut(sid).unwrap();
        let mut tabs: Vec<f64> = s.0.get("tabs").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
        let tol = (8.0 / zoom) / len;
        if let Some(k) = tabs.iter().position(|t| (t - f).abs().min(1.0 - (t - f).abs()) < tol) {
            tabs.remove(k);
        } else {
            tabs.push((f * 1e6).round() / 1e6);
        }
        tabs.sort_by(|a, b| a.total_cmp(b));
        if tabs.is_empty() {
            s.0.remove("tabs");
        } else {
            s.0.insert("tabs".into(), json!(tabs));
        }
        c.events.push(crate::canvas::Event::Changed);
    }
    fn draw(&self, c: &Canvas, doc: &Doc, _env: &Env, p: &egui::Painter) {
        draw_tabs_of(c, doc, p, true);
    }
}

// ====================================================================== start point & direction
pub struct StartPoint;

impl ToolHandler for StartPoint {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env, pt: P, shift: bool) {
        let Some((sid, path)) = shape_hit(c, doc, pt, &["rect", "circle", "poly"]) else { return };
        doc.save_undo(&l("Punto di inizio", "Start point"));
        let s = doc.find_mut(sid).unwrap();
        if shift {
            if s.flag("reverse") {
                s.0.remove("reverse");
            } else {
                s.0.insert("reverse".into(), json!(true));
            }
        } else if is_closed_path(&path) {
            s.set("start", (fraction_at(&path, pt) * 1e6).round() / 1e6);
        }
        c.events.push(crate::canvas::Event::Changed);
    }
    fn draw(&self, c: &Canvas, doc: &Doc, _env: &Env, p: &egui::Painter) {
        let z = c.zoom as f64;
        let green = Color32::from_rgb(0x1a, 0x9d, 0x4a);
        for s in &doc.shapes {
            if !matches!(s.kind(), "rect" | "circle" | "poly") {
                continue;
            }
            let Some(path) = s.paths().into_iter().next() else { continue };
            if path.len() < 2 {
                continue;
            }
            let path = job_path(s, &path);
            p.circle_filled(c.to_screen(path[0]), 4.0, green);
            let len = path_length(&path);
            if len <= 0.0 {
                continue;
            }
            let a = point_at_fraction(&path, (12.0 / z / len).min(0.999));
            let b = point_at_fraction(&path, (18.0 / z / len).min(1.0));
            let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
            let sz = 6.0 / z;
            let tri = [b, [b[0] - sz * (ang - 0.5).cos(), b[1] - sz * (ang - 0.5).sin()], [b[0] - sz * (ang + 0.5).cos(), b[1] - sz * (ang + 0.5).sin()]];
            p.add(egui::Shape::convex_polygon(tri.iter().map(|q| c.to_screen(*q)).collect(), green, Stroke::NONE));
        }
    }
}

// ====================================================================== two-point rotate & scale
#[derive(Default)]
pub struct TwoPoint {
    pts: Vec<P>,
    cur: Option<P>,
}

impl ToolHandler for TwoPoint {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P, shift: bool) {
        if doc.selected.is_empty() {
            c.events.push(crate::canvas::Event::Status(tr("selectFirst")));
            return;
        }
        let sp = c.snap(doc, env, pt, None);
        self.pts.push(sp);
        if self.pts.len() < 3 {
            let msg = if self.pts.len() == 1 { l("Clic sul punto di riferimento", "Click the reference point") } else { l("Clic sulla destinazione (⇧ = anche scala)", "Click the destination (⇧ also scales)") };
            c.events.push(crate::canvas::Event::Status(msg));
            return;
        }
        let (a, b, d) = (self.pts[0], self.pts[1], self.pts[2]);
        self.pts.clear();
        let ang = ((d[1] - a[1]).atan2(d[0] - a[0]) - (b[1] - a[1]).atan2(b[0] - a[0])).to_degrees();
        let den = (b[0] - a[0]).hypot(b[1] - a[1]);
        let k = (d[0] - a[0]).hypot(d[1] - a[1]) / if den == 0.0 { 1.0 } else { den };
        let _ = doc.edit_selected(&l("Ruota", "Rotate"), |s| {
            rotate_shape(s, a[0], a[1], ang);
            if shift {
                scale_shape(s, a[0], a[1], k, k);
            }
        });
        let mut msg = l(&format!("Ruotato di {ang:.2}°"), &format!("Rotated {ang:.2}°"));
        if shift {
            msg += &format!(" · ×{k:.4}");
        }
        c.events.push(crate::canvas::Event::Status(msg));
        c.events.push(crate::canvas::Event::Changed);
    }
    fn hover(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P) {
        self.cur = Some(c.snap(doc, env, pt, None));
    }
    fn draw(&self, c: &Canvas, _doc: &Doc, _env: &Env, p: &egui::Painter) {
        if self.pts.is_empty() {
            return;
        }
        let red = Color32::from_rgb(0xe0, 0x1b, 0x1b);
        let mut all = self.pts.clone();
        all.extend(self.cur);
        for q in &all[1..] {
            c.polyline(p, &[all[0], *q], Stroke::new(1.0, red), true);
        }
        for q in &self.pts {
            p.circle_filled(c.to_screen(*q), 3.5, red);
        }
    }
    fn reset(&mut self) {
        self.pts.clear();
        self.cur = None;
    }
}

// ====================================================================== deform / warp
pub fn bilinear(c: &[P], u: f64, v: f64) -> P {
    let (a, b, cc, d) = (c[0], c[1], c[2], c[3]);
    [(1.0 - u) * (1.0 - v) * a[0] + u * (1.0 - v) * b[0] + u * v * cc[0] + (1.0 - u) * v * d[0],
     (1.0 - u) * (1.0 - v) * a[1] + u * (1.0 - v) * b[1] + u * v * cc[1] + (1.0 - u) * v * d[1]]
}

pub fn quad_patch(g: &[P], u: f64, v: f64) -> P {
    let bu = [(1.0 - u).powi(2), 2.0 * u * (1.0 - u), u * u];
    let bv = [(1.0 - v).powi(2), 2.0 * v * (1.0 - v), v * v];
    let (mut x, mut y) = (0.0, 0.0);
    for j in 0..3 {
        for i in 0..3 {
            let w = bu[i] * bv[j];
            x += w * g[j * 3 + i][0];
            y += w * g[j * 3 + i][1];
        }
    }
    [x, y]
}

/// tools.warp_shapes: densified copies of the shapes mapped through f(u, v) over `bx`.
pub fn warp_shapes(next_id: &mut i64, shapes: &[Shape], bx: [f64; 4], f: &dyn Fn(f64, f64) -> P) -> Vec<Shape> {
    let [x0, y0, w, h] = bx;
    let mut out = Vec::new();
    for s in shapes {
        if s.kind() == "image" {
            out.push(s.clone());
            continue;
        }
        let mut polys = Vec::new();
        for path in s.paths() {
            if path.len() < 2 {
                continue;
            }
            let closed = is_closed_path(&path);
            let mut mapped: Vec<P> = densify(&path, 0.8).iter().map(|q| f((q[0] - x0) / if w != 0.0 { w } else { 1.0 }, (q[1] - y0) / if h != 0.0 { h } else { 1.0 })).collect();
            if closed {
                mapped.pop();
            }
            let id = *next_id;
            *next_id += 1;
            polys.push(json!({"id": id, "type": "poly", "layer": s.layer(), "closed": closed,
                              "points": simplify(&mapped, 0.02).iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()}));
        }
        if polys.len() == 1 {
            polys[0]["id"] = json!(s.id());
            out.push(Shape(serde_json::from_value(polys.remove(0)).unwrap()));
        } else if !polys.is_empty() {
            out.push(Shape(serde_json::from_value(json!({"id": s.id(), "type": "group", "layer": s.layer(), "children": polys})).unwrap()));
        }
    }
    out
}

pub fn taper(next_id: &mut i64, shapes: &[Shape], top_pct: f64, bottom_pct: f64) -> Vec<Shape> {
    let Some(b) = multi_bounds(shapes) else { return Vec::new() };
    let cx = b[0] + b[2] / 2.0;
    let (t, bo) = (top_pct / 100.0, bottom_pct / 100.0);
    let corners = [[cx - b[2] / 2.0 * t, b[1]], [cx + b[2] / 2.0 * t, b[1]], [cx + b[2] / 2.0 * bo, b[1] + b[3]], [cx - b[2] / 2.0 * bo, b[1] + b[3]]];
    warp_shapes(next_id, shapes, b, &|u, v| bilinear(&corners, u, v))
}

pub struct Deform {
    pub warp: bool,
    bx: Option<[f64; 4]>,
    grid: Vec<P>,
    orig: Vec<Shape>,
    drag: Option<usize>,
}

impl Deform {
    pub fn new(warp: bool) -> Self {
        Deform { warp, bx: None, grid: Vec::new(), orig: Vec::new(), drag: None }
    }
    fn ensure(&mut self, doc: &Doc) -> bool {
        let sel = doc.selected_shapes();
        if sel.is_empty() {
            self.reset();
            return false;
        }
        if self.bx.is_none() {
            let b = multi_bounds(&sel).unwrap();
            let bx = [b[0], b[1], b[2].max(1e-3), b[3].max(1e-3)];
            self.bx = Some(bx);
            let [x0, y0, w, h] = bx;
            self.grid = if self.warp {
                (0..3).flat_map(|j| (0..3).map(move |i| [x0 + w * i as f64 / 2.0, y0 + h * j as f64 / 2.0])).collect()
            } else {
                vec![[x0, y0], [x0 + w, y0], [x0 + w, y0 + h], [x0, y0 + h]]
            };
        }
        true
    }
}

impl ToolHandler for Deform {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P, _shift: bool) {
        if !self.ensure(doc) {
            if let Some(hit) = c.hit_test(doc, env, pt) {
                doc.set_selection(vec![hit]);
                self.ensure(doc);
            }
            return;
        }
        let tol = 7.0 / c.zoom as f64;
        if let Some(i) = self.grid.iter().position(|q| (q[0] - pt[0]).abs() < tol && (q[1] - pt[1]).abs() < tol) {
            doc.save_undo(&l(if self.warp { "Distorci" } else { "Deforma" }, if self.warp { "Warp" } else { "Deform" }));
            self.orig = doc.selected_shapes();
            self.drag = Some(i);
        }
    }
    fn hover(&mut self, _c: &mut Canvas, doc: &mut Doc, _env: &Env, _pt: P) {
        self.ensure(doc);
    }
    fn drag(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P) {
        let Some(i) = self.drag else { return };
        self.grid[i] = c.snap(doc, env, pt, None);
        let g = self.grid.clone();
        let warp = self.warp;
        let f = move |u: f64, v: f64| if warp { quad_patch(&g, u, v) } else { bilinear(&g, u, v) };
        let mut next = doc.next_id;
        let new = warp_shapes(&mut next, &self.orig, self.bx.unwrap(), &f);
        doc.next_id = next;
        for n in new {
            if let Some(s) = doc.find_mut(n.id()) {
                *s = n;
            }
        }
    }
    fn release(&mut self, c: &mut Canvas, _doc: &mut Doc, _env: &Env) {
        if self.drag.take().is_some() {
            c.events.push(crate::canvas::Event::Changed);
        }
    }
    fn draw(&self, c: &Canvas, doc: &Doc, _env: &Env, p: &egui::Painter) {
        if doc.selected.is_empty() || self.grid.is_empty() {
            return;
        }
        let g = &self.grid;
        let st = Stroke::new(1.0, PURPLE);
        if self.warp {
            for j in 0..3 {
                for i in 0..2 {
                    c.polyline(p, &[g[j * 3 + i], g[j * 3 + i + 1]], st, true);
                }
            }
            for i in 0..3 {
                for j in 0..2 {
                    c.polyline(p, &[g[j * 3 + i], g[(j + 1) * 3 + i]], st, true);
                }
            }
        } else {
            for i in 0..4 {
                c.polyline(p, &[g[i], g[(i + 1) % 4]], st, true);
            }
        }
        for q in g {
            let r = Rect::from_center_size(c.to_screen(*q), vec2(9.0, 9.0));
            p.rect_filled(r, 0.0, Color32::WHITE);
            p.rect_stroke(r, 0.0, st, StrokeKind::Inside);
        }
    }
    fn reset(&mut self) {
        self.bx = None;
        self.grid.clear();
        self.orig.clear();
        self.drag = None;
    }
}

// ====================================================================== node editor
#[derive(Clone)]
enum NodeDrag {
    Handle { sid: i64, idx: usize, key: &'static str },
    Nodes { start: P, orig: Vec<((i64, usize), Map<String, Value>)> },
    Bend { sid: i64, seg: usize, t: f64, start: P, c2: P, c1: P },
}

#[derive(Default)]
pub struct NodeEditor {
    sel: BTreeSet<(i64, usize)>,
    hover_node: Option<(i64, usize)>,
    hover_seg: Option<(i64, usize, f64, P)>,
    drag: Option<NodeDrag>,
    marquee: Option<(P, P)>,
}

impl NodeEditor {
    fn polys(doc: &Doc) -> Vec<Shape> {
        doc.selected_shapes().into_iter().filter(|s| s.kind() == "poly").collect()
    }
    fn tol(c: &Canvas, env: &Env) -> f64 {
        (env.prefs.click_tol + 1.0) / c.zoom as f64
    }
    fn neighbour_selected(&self, s: &Shape, i: usize) -> bool {
        let n = pts(s).len();
        n <= 12 || [i + n - 1, i + 1].iter().any(|j| self.sel.contains(&(s.id(), j % n)))
    }
    fn hit_handle(&self, c: &Canvas, doc: &Doc, env: &Env, pt: P) -> Option<(i64, usize, &'static str)> {
        let tol = Self::tol(c, env);
        for s in Self::polys(doc) {
            for (i, p) in pts(&s).iter().enumerate() {
                if !self.sel.contains(&(s.id(), i)) && !self.neighbour_selected(&s, i) {
                    continue;
                }
                for k in ["c1", "c2"] {
                    if let Some(h) = pt2(p.get(k)) {
                        if (h[0] - pt[0]).abs() < tol && (h[1] - pt[1]).abs() < tol {
                            return Some((s.id(), i, k));
                        }
                    }
                }
            }
        }
        None
    }
    fn hit_node(c: &Canvas, doc: &Doc, env: &Env, pt: P) -> Option<(i64, usize)> {
        let tol = Self::tol(c, env);
        for s in Self::polys(doc) {
            for (i, p) in pts(&s).iter().enumerate() {
                let q = xy(p);
                if (q[0] - pt[0]).abs() < tol && (q[1] - pt[1]).abs() < tol {
                    return Some((s.id(), i));
                }
            }
        }
        None
    }
    fn hit_segment(c: &Canvas, doc: &Doc, env: &Env, pt: P) -> Option<(f64, i64, usize, f64, P)> {
        let tol = Self::tol(c, env);
        let mut best: Option<(f64, i64, usize, f64, P)> = None;
        for s in Self::polys(doc) {
            for i in 0..seg_count(&s) {
                let (d, t, q) = nearest_on_segment(&s, i, pt);
                if d < tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, s.id(), i, t, q));
                }
            }
        }
        best
    }
}

impl ToolHandler for NodeEditor {
    fn press(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P, shift: bool) {
        if let Some((sid, idx, key)) = self.hit_handle(c, doc, env, pt) {
            doc.save_undo(&l("Modifica nodi", "Edit nodes"));
            self.drag = Some(NodeDrag::Handle { sid, idx, key });
            return;
        }
        if let Some(key) = Self::hit_node(c, doc, env, pt) {
            if shift {
                if !self.sel.remove(&key) {
                    self.sel.insert(key);
                }
            } else if !self.sel.contains(&key) {
                self.sel = [key].into_iter().collect();
            }
            doc.save_undo(&l("Modifica nodi", "Edit nodes"));
            let orig = self.sel.iter().filter_map(|k| doc.find(k.0).and_then(|s| pts(s).get(k.1).cloned()).map(|p| (*k, p))).collect();
            self.drag = Some(NodeDrag::Nodes { start: pt, orig });
            return;
        }
        if let Some((_, sid, i, t, _)) = Self::hit_segment(c, doc, env, pt) {
            let s = doc.find(sid).unwrap().clone();
            if is_curve_seg(&s, i) {
                doc.save_undo(&l("Modifica nodi", "Edit nodes"));
                let p = pts(&s);
                let (a, b) = ends(&p, i);
                self.drag = Some(NodeDrag::Bend { sid, seg: i, t, start: pt, c2: pt2(p[a].get("c2")).unwrap_or(xy(&p[a])), c1: pt2(p[b].get("c1")).unwrap_or(xy(&p[b])) });
            } else if !shift {
                self.sel.clear();
            }
            return;
        }
        let hit = c.hit_test(doc, env, pt);
        if let Some(h) = hit {
            if doc.find(h).is_some_and(|s| matches!(s.kind(), "rect" | "circle")) {
                doc.save_undo(&l("Converti in tracciato", "Convert to path"));
                let s = doc.find_mut(h).unwrap();
                *s = to_poly(s);
                c.events.push(crate::canvas::Event::Changed);
            }
            if !doc.is_selected(h) {
                doc.set_selection(vec![h]);
                self.sel.clear();
            }
        } else {
            if !shift {
                self.sel.clear();
            }
            self.marquee = Some((pt, pt));
        }
    }
    fn hover(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P) {
        self.hover_node = Self::hit_node(c, doc, env, pt);
        self.hover_seg = if self.hover_node.is_some() { None } else { Self::hit_segment(c, doc, env, pt).map(|b| (b.1, b.2, b.3, b.4)) };
    }
    fn drag(&mut self, c: &mut Canvas, doc: &mut Doc, env: &Env, pt: P) {
        if let Some(m) = &mut self.marquee {
            m.1 = pt;
            return;
        }
        let Some(d) = self.drag.clone() else { return };
        let sp = c.snap(doc, env, pt, None);
        match d {
            NodeDrag::Handle { sid, idx, key } => {
                let Some(s) = doc.find_mut(sid) else { return };
                let mut p = pts(s);
                p[idx].insert(key.into(), json!([sp[0], sp[1]]));
                mirror_handle(&mut p[idx], key);
                set_pts(s, p);
                s.0.remove("regular");
            }
            NodeDrag::Nodes { start, orig } => {
                let s0 = c.snap(doc, env, start, None);
                let (dx, dy) = (sp[0] - s0[0], sp[1] - s0[1]);
                for ((sid, idx), o) in orig {
                    if let Some(s) = doc.find_mut(sid) {
                        let mut p = pts(s);
                        let mut q = o.clone();
                        map_point(&mut q, &|x, y| [x + dx, y + dy]);
                        if idx < p.len() {
                            p[idx] = q;
                        }
                        set_pts(s, p);
                        s.0.remove("regular");
                    }
                }
            }
            NodeDrag::Bend { sid, seg, t, start, c2, c1 } => {
                let Some(s) = doc.find_mut(sid) else { return };
                let mut p = pts(s);
                let (a, b) = ends(&p, seg);
                let t = t.clamp(0.1, 0.9);
                let (dx, dy) = (pt[0] - start[0], pt[1] - start[1]);
                let (w1, w2) = (3.0 * (1.0 - t).powi(2) * t, 3.0 * (1.0 - t) * t * t);
                let k = 1.0 / (w1 * w1 + w2 * w2);
                p[a].insert("c2".into(), json!([c2[0] + dx * w1 * k, c2[1] + dy * w1 * k]));
                p[b].insert("c1".into(), json!([c1[0] + dx * w2 * k, c1[1] + dy * w2 * k]));
                mirror_handle(&mut p[a], "c2");
                mirror_handle(&mut p[b], "c1");
                set_pts(s, p);
                s.0.remove("regular");
            }
        }
    }
    fn release(&mut self, c: &mut Canvas, doc: &mut Doc, _env: &Env) {
        if let Some((a, b)) = self.marquee.take() {
            let (x0, x1, y0, y1) = (a[0].min(b[0]), a[0].max(b[0]), a[1].min(b[1]), a[1].max(b[1]));
            for s in Self::polys(doc) {
                for (i, p) in pts(&s).iter().enumerate() {
                    let q = xy(p);
                    if q[0] >= x0 && q[0] <= x1 && q[1] >= y0 && q[1] <= y1 {
                        self.sel.insert((s.id(), i));
                    }
                }
            }
        }
        if self.drag.take().is_some() {
            c.events.push(crate::canvas::Event::Changed);
        }
    }
    fn key(&mut self, c: &mut Canvas, doc: &mut Doc, ch: char) -> bool {
        let k = ch.to_ascii_uppercase();
        if !"SLCDIMB".contains(k) {
            return false;
        }
        let label = l("Modifica nodi", "Edit nodes");
        let mut done = false;
        if (k == 'I' || k == 'M') && self.hover_seg.is_some() {
            let (sid, i, t, _) = self.hover_seg.take().unwrap();
            doc.save_undo(&label);
            if let Some(s) = doc.find_mut(sid) {
                let j = split_segment(s, i, if k == 'M' { 0.5 } else { t });
                self.sel = [(sid, j)].into_iter().collect();
            }
            done = true;
        } else if k == 'S' && !self.sel.is_empty() {
            doc.save_undo(&label);
            for (sid, idx) in self.sel.clone() {
                if let Some(s) = doc.find_mut(sid) {
                    let sm = pts(s).get(idx).and_then(|p| p.get("smooth")).and_then(|v| v.as_bool()).unwrap_or(false);
                    set_smooth(s, idx, !sm);
                }
            }
            done = true;
        } else if k == 'S' && self.hover_seg.is_some() {
            let (sid, i, _, _) = self.hover_seg.unwrap();
            doc.save_undo(&label);
            if let Some(s) = doc.find_mut(sid) {
                make_curve(s, i);
            }
            done = true;
        } else if k == 'L' && self.hover_seg.is_some() {
            let (sid, i, _, _) = self.hover_seg.unwrap();
            doc.save_undo(&label);
            if let Some(s) = doc.find_mut(sid) {
                make_line(s, i);
            }
            done = true;
        } else if k == 'C' && !self.sel.is_empty() {
            doc.save_undo(&label);
            for (sid, idx) in self.sel.clone() {
                if let Some(s) = doc.find_mut(sid) {
                    set_smooth(s, idx, false);
                }
            }
            done = true;
        } else if k == 'D' && !self.sel.is_empty() {
            doc.save_undo(&label);
            let mut by: std::collections::BTreeMap<i64, Vec<usize>> = Default::default();
            for (sid, idx) in &self.sel {
                by.entry(*sid).or_default().push(*idx);
            }
            for (sid, idxs) in by {
                let keep = doc.find_mut(sid).map(|s| delete_nodes(s, &idxs)).unwrap_or(true);
                if !keep {
                    doc.shapes.retain(|x| x.id() != sid);
                }
            }
            self.sel.clear();
            done = true;
        } else if k == 'D' && self.hover_seg.is_some() {
            let (sid, i, _, _) = self.hover_seg.take().unwrap();
            if let Some(s) = doc.find(sid).cloned() {
                replace_with_parts(doc, sid, delete_segment(&s, i), &label);
                self.sel.clear();
            }
            done = true;
        } else if k == 'B' {
            let target = self.hover_node.or_else(|| if self.sel.len() == 1 { self.sel.iter().next().copied() } else { None });
            if let Some((sid, idx)) = target {
                if let Some(s) = doc.find(sid).cloned() {
                    replace_with_parts(doc, sid, break_at_node(&s, idx), &label);
                    self.sel.clear();
                    self.hover_node = None;
                }
                done = true;
            }
        }
        if done {
            c.events.push(crate::canvas::Event::Changed);
        }
        done
    }
    fn draw(&self, c: &Canvas, doc: &Doc, env: &Env, p: &egui::Painter) {
        let hs = (env.prefs.node_size / 2.0) as f32;
        if let Some((sid, i, _, q)) = self.hover_seg {
            if let Some(s) = doc.find(sid) {
                let n = if is_curve_seg(s, i) { 24 } else { 1 };
                let seg: Vec<P> = (0..=n).map(|k| seg_point(s, i, k as f64 / n as f64)).collect();
                c.polyline(p, &seg, Stroke::new(3.0, ORANGE), false);
                p.circle_filled(c.to_screen(q), hs, ORANGE);
            }
        }
        for s in Self::polys(doc) {
            for (i, pt) in pts(&s).iter().enumerate() {
                let key = (s.id(), i);
                let at = c.to_screen(xy(pt));
                if self.sel.contains(&key) || self.neighbour_selected(&s, i) {
                    for k in ["c1", "c2"] {
                        if let Some(h) = pt2(pt.get(k)) {
                            p.line_segment([at, c.to_screen(h)], Stroke::new(1.0, PURPLE));
                            p.circle_filled(c.to_screen(h), hs * 0.8, PURPLE);
                        }
                    }
                }
                let col = if self.sel.contains(&key) { RED } else if self.hover_node == Some(key) { ORANGE } else { BLUE };
                if pt.get("smooth").and_then(|v| v.as_bool()).unwrap_or(false) {
                    p.circle_filled(at, hs, col);
                } else {
                    p.rect_filled(Rect::from_center_size(at, vec2(hs * 2.0, hs * 2.0)), 0.0, col);
                }
            }
        }
        if let Some((a, b)) = self.marquee {
            let r = Rect::from_two_pos(c.to_screen(a), c.to_screen(b));
            p.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(47, 123, 245, 20));
            p.rect_stroke(r, 0.0, Stroke::new(1.0, BLUE), StrokeKind::Inside);
        }
    }
    fn reset(&mut self) {
        *self = NodeEditor::default();
    }
    fn selected_nodes(&self) -> Vec<(i64, usize)> {
        self.sel.iter().copied().collect()
    }
}

/// All plug-in tools, keyed by tool name.
pub fn handlers() -> std::collections::HashMap<String, Box<dyn ToolHandler>> {
    let mut m: std::collections::HashMap<String, Box<dyn ToolHandler>> = Default::default();
    m.insert("nodes".into(), Box::new(NodeEditor::default()));
    m.insert("scissors".into(), Box::new(Scissors));
    m.insert("trim".into(), Box::new(Trim::default()));
    m.insert("tabs".into(), Box::new(Tabs));
    m.insert("start".into(), Box::new(StartPoint));
    m.insert("twopoint".into(), Box::new(TwoPoint::default()));
    m.insert("deform".into(), Box::new(Deform::new(false)));
    m.insert("warp".into(), Box::new(Deform::new(true)));
    m
}
