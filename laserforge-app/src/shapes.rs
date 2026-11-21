//! Design shapes, stored exactly like the Python app's `.lfproj` files (plain JSON objects), so projects open
//! in both apps. Types: rect {x,y,w,h,radius}, circle {cx,cy,rx,ry}, poly {points:[{x,y,c1?,c2?}], closed},
//! text {x,y,text,size,font}, image {x,y,w,h,src,adj,flipX,flipY}, group {children}; all carry id and layer.
//! Geometry ported from laserforge/model.py.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::f64::consts::{PI, TAU};

/// Max chord error (mm) when flattening curves (the Python default "curve quality").
/// Default maximum chord error (mm) when curves become line segments.
pub const CURVE_TOL: f64 = 0.25;
static CURVE_TOL_BITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x3FD0_0000_0000_0000); // 0.25

/// Current chord error, from Settings › Curve quality.
pub fn curve_tol() -> f64 {
    f64::from_bits(CURVE_TOL_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_curve_tol(v: f64) {
    let v = if v.is_finite() && v > 0.0 { v.clamp(0.01, 2.0) } else { CURVE_TOL };
    CURVE_TOL_BITS.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(transparent)]
pub struct Shape(pub Map<String, Value>);

pub type P = [f64; 2];

fn num(v: Option<&Value>) -> f64 {
    v.and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn pt_of(v: &Value) -> Option<P> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// A poly node: position plus optional Bézier handles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub p: P,
    pub c1: Option<P>,
    pub c2: Option<P>,
}

impl Shape {
    pub fn new(kind: &str, id: i64, layer: usize, fields: Value) -> Shape {
        let mut m = Map::new();
        m.insert("id".into(), json!(id));
        m.insert("type".into(), json!(kind));
        m.insert("layer".into(), json!(layer));
        if let Value::Object(f) = fields {
            m.extend(f);
        }
        Shape(m)
    }
    pub fn rect(id: i64, layer: usize, x: f64, y: f64, w: f64, h: f64) -> Shape {
        Shape::new("rect", id, layer, json!({"x": x, "y": y, "w": w, "h": h}))
    }
    pub fn circle(id: i64, layer: usize, cx: f64, cy: f64, rx: f64, ry: f64) -> Shape {
        Shape::new("circle", id, layer, json!({"cx": cx, "cy": cy, "rx": rx, "ry": ry}))
    }
    pub fn poly(id: i64, layer: usize, pts: &[P], closed: bool) -> Shape {
        let points: Vec<Value> = pts.iter().map(|p| json!({"x": p[0], "y": p[1]})).collect();
        Shape::new("poly", id, layer, json!({"points": points, "closed": closed}))
    }

    pub fn kind(&self) -> &str {
        self.0.get("type").and_then(|v| v.as_str()).unwrap_or("")
    }
    pub fn id(&self) -> i64 {
        self.0.get("id").and_then(|v| v.as_i64()).unwrap_or(0)
    }
    pub fn f(&self, k: &str) -> f64 {
        num(self.0.get(k))
    }
    pub fn set(&mut self, k: &str, v: f64) {
        self.0.insert(k.into(), json!(v));
    }
    pub fn flag(&self, k: &str) -> bool {
        match self.0.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Object(o)) => !o.is_empty(),
            _ => false,
        }
    }
    /// `s.get("layer", 0)`; None when the key holds null (group children inheriting the group's layer).
    pub fn layer_opt(&self) -> Option<i64> {
        match self.0.get("layer") {
            None => Some(0),
            Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)),
        }
    }
    pub fn layer(&self) -> i64 {
        self.layer_opt().unwrap_or(0)
    }
    pub fn set_layer(&mut self, l: usize) {
        self.0.insert("layer".into(), json!(l));
        if let Some(Value::Array(kids)) = self.0.get_mut("children") {
            for k in kids {
                if let Value::Object(m) = k {
                    let mut s = Shape(std::mem::take(m));
                    s.set_layer(l);
                    *m = s.0;
                }
            }
        }
    }
    pub fn locked(&self) -> bool {
        self.flag("locked")
    }
    pub fn children(&self) -> Vec<Shape> {
        match self.0.get("children") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_object().map(|m| Shape(m.clone()))).collect(),
            _ => Vec::new(),
        }
    }
    pub fn nodes(&self) -> Vec<Node> {
        match self.0.get("points") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| {
                let m = v.as_object()?;
                Some(Node { p: [num(m.get("x")), num(m.get("y"))], c1: m.get("c1").and_then(pt_of), c2: m.get("c2").and_then(pt_of) })
            }).collect(),
            _ => Vec::new(),
        }
    }
    pub fn name(&self) -> String {
        self.0.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    /// Bounding box (x, y, w, h) on the canvas.
    pub fn bounds(&self) -> [f64; 4] {
        match self.kind() {
            "rect" | "image" => [self.f("x"), self.f("y"), self.f("w"), self.f("h")],
            "circle" => [self.f("cx") - self.f("rx"), self.f("cy") - self.f("ry"), self.f("rx") * 2.0, self.f("ry") * 2.0],
            "text" => crate::text::text_bounds(self),
            "poly" => {
                let nodes = self.nodes();
                if nodes.is_empty() {
                    return [0.0; 4];
                }
                if nodes.iter().any(|n| n.c1.is_some() || n.c2.is_some()) {
                    return path_box(&self.paths()[0]);
                }
                path_box(&nodes.iter().map(|n| n.p).collect::<Vec<_>>())
            }
            "group" => multi_bounds(&self.children()).unwrap_or([0.0; 4]),
            _ => [0.0; 4],
        }
    }

    /// Outline paths; closed paths repeat the first point (model.shape_paths).
    pub fn paths(&self) -> Vec<Vec<P>> {
        match self.kind() {
            "rect" => {
                let (x, y, w, h) = (self.f("x"), self.f("y"), self.f("w"), self.f("h"));
                let r = self.f("radius").min(w / 2.0).min(h / 2.0);
                if r > 1e-6 {
                    return vec![rounded_rect(x, y, w, h, r)];
                }
                vec![vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]]]
            }
            "circle" => {
                let (cx, cy, rx, ry) = (self.f("cx"), self.f("cy"), self.f("rx"), self.f("ry"));
                let r = rx.max(ry).max(1e-6);
                let n = ((TAU / (8.0 * curve_tol().min(r) / r).sqrt()) as i64 + 1).clamp(24, 720) as usize;
                vec![(0..=n).map(|i| {
                    let a = i as f64 / n as f64 * TAU;
                    [cx + rx * a.cos(), cy + ry * a.sin()]
                }).collect()]
            }
            "poly" => vec![poly_flatten(&self.nodes(), self.flag("closed"))],
            "group" => self.children().iter().flat_map(|c| c.paths()).collect(),
            "text" => crate::text::text_paths(self),
            _ => Vec::new(),
        }
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        match self.kind() {
            "rect" | "text" | "image" => {
                let (x, y) = (self.f("x"), self.f("y"));
                self.set("x", x + dx);
                self.set("y", y + dy);
            }
            "circle" => {
                let (x, y) = (self.f("cx"), self.f("cy"));
                self.set("cx", x + dx);
                self.set("cy", y + dy);
            }
            "poly" => {
                if let Some(Value::Array(pts)) = self.0.get_mut("points") {
                    for p in pts.iter_mut() {
                        if let Value::Object(m) = p {
                            for k in ["x", "y"] {
                                let d = if k == "x" { dx } else { dy };
                                let v = num(m.get(k)) + d;
                                m.insert(k.into(), json!(v));
                            }
                            for k in ["c1", "c2"] {
                                if let Some(c) = m.get(k).and_then(pt_of) {
                                    m.insert(k.into(), json!([c[0] + dx, c[1] + dy]));
                                }
                            }
                        }
                    }
                }
                if let Some(Value::Object(r)) = self.0.get_mut("regular") {
                    for (k, d) in [("cx", dx), ("cy", dy)] {
                        let v = num(r.get(k)) + d;
                        r.insert(k.into(), json!(v));
                    }
                }
            }
            "group" => {
                if let Some(Value::Array(kids)) = self.0.get_mut("children") {
                    for k in kids {
                        if let Value::Object(m) = k {
                            let mut s = Shape(std::mem::take(m));
                            s.translate(dx, dy);
                            *m = s.0;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Distance from a point to the outline (mm), for picking; images and text pick inside their box.
    pub fn distance(&self, x: f64, y: f64) -> f64 {
        if matches!(self.kind(), "image" | "text") {
            let b = self.bounds();
            let dx = (b[0] - x).max(x - (b[0] + b[2])).max(0.0);
            let dy = (b[1] - y).max(y - (b[1] + b[3])).max(0.0);
            return (dx * dx + dy * dy).sqrt();
        }
        let mut best = f64::INFINITY;
        for path in self.paths() {
            for w in path.windows(2) {
                best = best.min(dist_to_segment([x, y], w[0], w[1]));
            }
        }
        best
    }
}

pub fn dist_to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 != 0.0 { ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2 } else { 0.0 };
    let t = t.clamp(0.0, 1.0);
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

pub fn path_box(pts: &[P]) -> [f64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in pts {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

pub fn multi_bounds(shapes: &[Shape]) -> Option<[f64; 4]> {
    if shapes.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for s in shapes {
        let b = s.bounds();
        x0 = x0.min(b[0]);
        y0 = y0.min(b[1]);
        x1 = x1.max(b[0] + b[2]);
        y1 = y1.max(b[1] + b[3]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

pub fn is_closed_path(p: &[P]) -> bool {
    p.len() > 2 && (p[0][0] - p[p.len() - 1][0]).hypot(p[0][1] - p[p.len() - 1][1]) < 1e-6
}

/// Groups opened up; children without a layer take the group's.
pub fn flatten(shapes: &[Shape]) -> Vec<Shape> {
    let mut out = Vec::new();
    for s in shapes {
        if s.kind() == "group" {
            let kids: Vec<Shape> = s.children().into_iter().map(|mut c| {
                if c.0.get("layer").map_or(true, |v| v.is_null()) {
                    c.0.insert("layer".into(), s.0.get("layer").cloned().unwrap_or(json!(0)));
                }
                c
            }).collect();
            out.extend(flatten(&kids));
        } else {
            out.push(s.clone());
        }
    }
    out
}

fn pow3(v: f64) -> f64 {
    v.powf(3.0) // Python's `u ** 3` calls C pow(), not repeated multiplication
}

pub fn cubic_points(p0: P, p1: P, p2: P, p3: P) -> Vec<P> {
    let est = (p1[0] - p0[0]).hypot(p1[1] - p0[1]) + (p2[0] - p1[0]).hypot(p2[1] - p1[1]) + (p3[0] - p2[0]).hypot(p3[1] - p2[1]);
    let n = ((est / curve_tol() / 2.0) as i64 + 1).clamp(4, 96);
    (1..=n).map(|i| {
        let t = i as f64 / n as f64;
        let u = 1.0 - t;
        let c = |k: usize| pow3(u) * p0[k] + 3.0 * u * u * t * p1[k] + 3.0 * u * t * t * p2[k] + pow3(t) * p3[k];
        [c(0), c(1)]
    }).collect()
}

fn segment_points(a: &Node, b: &Node) -> Vec<P> {
    if a.c2.is_some() || b.c1.is_some() {
        return cubic_points(a.p, a.c2.unwrap_or(a.p), b.c1.unwrap_or(b.p), b.p);
    }
    vec![b.p]
}

pub fn poly_flatten(nodes: &[Node], closed: bool) -> Vec<P> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let mut out = vec![nodes[0].p];
    for i in 1..nodes.len() {
        out.extend(segment_points(&nodes[i - 1], &nodes[i]));
    }
    if closed && nodes.len() > 2 {
        out.extend(segment_points(&nodes[nodes.len() - 1], &nodes[0]));
        let first = out[0];
        *out.last_mut().unwrap() = first;
    }
    out
}

pub fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> Vec<P> {
    let mut pts = Vec::new();
    let k = 16;
    for (cx, cy, a0) in [(x + w - r, y + r, -90.0), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)] {
        for i in 0..=k {
            let a = (a0 + (90 * i) as f64 / k as f64) * (PI / 180.0);
            pts.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    }
    pts.push(pts[0]);
    pts
}

/// Douglas–Peucker (model.simplify).
pub fn simplify(pts: &[P], tol: f64) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    *keep.last_mut().unwrap() = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut maxd, mut idx) = (0.0, 0usize);
        let mut found = false;
        for i in a + 1..b {
            let d = dist_to_segment(pts[i], pts[a], pts[b]);
            if d > maxd {
                maxd = d;
                idx = i;
                found = true;
            }
        }
        if found && maxd > tol && idx > 0 {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Closed path starting at fraction `f` of its length (exact start point inserted), as tools.rotate_closed_path.
pub fn rotate_closed_path(path: &[P], f: f64) -> Vec<P> {
    if !is_closed_path(path) || f <= 0.0 || f >= 1.0 {
        return path.to_vec();
    }
    let seg: Vec<f64> = path.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).collect();
    let total: f64 = seg.iter().sum();
    let target = f * total;
    let mut acc = 0.0;
    for (i, l) in seg.iter().enumerate() {
        if acc + l >= target && *l > 0.0 {
            let t = (target - acc) / l;
            let a = path[i];
            let b = path[i + 1];
            let s = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let mut out = vec![s];
            let start = if t >= 1.0 - 1e-12 { i + 2 } else { i + 1 };
            out.extend_from_slice(&path[start.min(path.len())..]);
            out.extend_from_slice(&path[1..=i]);
            if t > 1e-12 || i > 0 {
                out.push(s);
            }
            return out;
        }
        acc += l;
    }
    path.to_vec()
}

pub fn default_adj() -> Value {
    json!({"mode": "dither", "brightness": 0, "contrast": 0, "gamma": 1.0, "invert": false, "threshold": 128,
           "cells_per_inch": 30, "halftone_angle": 22.5, "slices": 10, "enhance_radius": 0.0, "enhance_amount": 0.0,
           "enhance_denoise": 0.0})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_children_inherit_layer() {
        let g: Shape = serde_json::from_value(json!({"id": 1, "type": "group", "layer": 4, "children": [
            {"id": 2, "type": "rect", "x": 0, "y": 0, "w": 5, "h": 5, "layer": null},
            {"id": 3, "type": "rect", "x": 0, "y": 0, "w": 5, "h": 5, "layer": 2}]})).unwrap();
        let f = flatten(&[g]);
        assert_eq!(f.iter().map(|s| s.layer()).collect::<Vec<_>>(), vec![4, 2]);
    }

    #[test]
    fn rotate_closed_path_starts_at_fraction() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let r = rotate_closed_path(&sq, 0.125);
        assert_eq!(r[0], [5.0, 0.0]);
        assert_eq!(*r.last().unwrap(), [5.0, 0.0]);
        assert_eq!(r.len(), 6);
        let r = rotate_closed_path(&sq, 0.25);
        assert_eq!(r, vec![[10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0], [10.0, 0.0]]);
    }
}
