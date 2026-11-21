//! Shape transforms (laserforge/model.py) and the document's editing commands (laserforge/document.py and the
//! Edit / Arrange functions of laserforge/mainwindow.py): selection, undo, clipboard, groups, mirror, rotate,
//! align, distribute, order, joins, crops, boolean operations.

use crate::project::{crop_mask, Doc};
use crate::shapes::{flatten, is_closed_path, multi_bounds, Shape, P};
use crate::{l, tr};
use serde_json::{json, Map, Value};
use std::f64::consts::PI;

pub const KAPPA: f64 = 0.5522847498;

/// Result of a command: Ok(optional message) or Err(message), both shown as a toast.
pub type Msg = Result<Option<String>, String>;

fn num(v: Option<&Value>) -> f64 {
    v.and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn pt_of(v: &Value) -> Option<P> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

pub fn center(b: [f64; 4]) -> P {
    [b[0] + b[2] / 2.0, b[1] + b[3] / 2.0]
}

/// model.map_point_dict: a node and its handles through f.
pub fn map_point(p: &mut Map<String, Value>, f: &dyn Fn(f64, f64) -> P) {
    let q = f(num(p.get("x")), num(p.get("y")));
    p.insert("x".into(), json!(q[0]));
    p.insert("y".into(), json!(q[1]));
    for k in ["c1", "c2"] {
        if let Some(c) = p.get(k).and_then(pt_of) {
            let q = f(c[0], c[1]);
            p.insert(k.into(), json!([q[0], q[1]]));
        }
    }
}

fn map_points(s: &mut Shape, f: &dyn Fn(f64, f64) -> P) {
    if let Some(Value::Array(pts)) = s.0.get_mut("points") {
        for p in pts.iter_mut() {
            if let Value::Object(m) = p {
                map_point(m, f);
            }
        }
    }
}

fn for_children(s: &mut Shape, f: &mut dyn FnMut(&mut Shape)) {
    if let Some(Value::Array(kids)) = s.0.get_mut("children") {
        for k in kids {
            if let Value::Object(m) = k {
                let mut c = Shape(std::mem::take(m));
                f(&mut c);
                *m = c.0;
            }
        }
    }
}

pub fn regular_polygon(cx: f64, cy: f64, r: f64, n: usize, rot: f64) -> Vec<Value> {
    (0..n).map(|i| {
        let a = rot + i as f64 / n as f64 * std::f64::consts::TAU;
        json!({"x": cx + r * a.cos(), "y": cy + r * a.sin()})
    }).collect()
}

pub fn star_points(cx: f64, cy: f64, outer: f64, inner: f64, n: usize) -> Vec<Value> {
    (0..n * 2).map(|i| {
        let a = -PI / 2.0 + i as f64 * PI / n as f64;
        let r = if i % 2 == 0 { outer } else { inner };
        json!({"x": cx + r * a.cos(), "y": cy + r * a.sin()})
    }).collect()
}

/// model.regular_meta_points
pub fn regular_meta_points(m: &Value) -> Vec<Value> {
    let g = |k: &str| m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let n = g("n").max(3.0) as usize;
    let rot = m.get("rot").and_then(|v| v.as_f64()).unwrap_or(-PI / 2.0);
    let inner = g("inner");
    if inner != 0.0 {
        return (0..n * 2).map(|i| {
            let a = rot + i as f64 * PI / n as f64;
            let r = if i % 2 == 0 { g("r") } else { g("r") * inner };
            json!({"x": g("cx") + r * a.cos(), "y": g("cy") + r * a.sin()})
        }).collect();
    }
    regular_polygon(g("cx"), g("cy"), g("r"), n, rot)
}

/// model.scale_shape
pub fn scale_shape(s: &mut Shape, ox: f64, oy: f64, sx: f64, sy: f64) {
    let fx = move |v: f64| ox + (v - ox) * sx;
    let fy = move |v: f64| oy + (v - oy) * sy;
    match s.kind() {
        "rect" | "image" => {
            let (x1, x2, y1, y2) = (fx(s.f("x")), fx(s.f("x") + s.f("w")), fy(s.f("y")), fy(s.f("y") + s.f("h")));
            s.set("x", x1.min(x2));
            s.set("w", (x2 - x1).abs());
            s.set("y", y1.min(y2));
            s.set("h", (y2 - y1).abs());
            if s.f("radius") != 0.0 {
                let r = s.f("radius") * sx.abs().min(sy.abs());
                s.set("radius", r);
            }
        }
        "circle" => {
            let (cx, cy, rx, ry) = (fx(s.f("cx")), fy(s.f("cy")), s.f("rx") * sx.abs(), s.f("ry") * sy.abs());
            s.set("cx", cx);
            s.set("cy", cy);
            s.set("rx", rx);
            s.set("ry", ry);
        }
        "poly" => {
            map_points(s, &|x, y| [fx(x), fy(y)]);
            if s.0.contains_key("regular") {
                if (sx.abs() - sy.abs()).abs() < 1e-9 {
                    if let Some(Value::Object(m)) = s.0.get_mut("regular") {
                        let (cx, cy, r) = (fx(num(m.get("cx"))), fy(num(m.get("cy"))), num(m.get("r")) * sx.abs());
                        m.insert("cx".into(), json!(cx));
                        m.insert("cy".into(), json!(cy));
                        m.insert("r".into(), json!(r));
                    }
                } else {
                    s.0.remove("regular");
                }
            }
        }
        "text" => {
            let (x, y, size) = (fx(s.f("x")), fy(s.f("y")), s.f("size") * sy.abs());
            s.set("x", x);
            s.set("y", y);
            s.set("size", size);
            if s.f("spacing") != 0.0 {
                let v = s.f("spacing") * sx.abs();
                s.set("spacing", v);
            }
            if s.f("bend") != 0.0 {
                let v = s.f("bend") * sy.abs();
                s.set("bend", v);
            }
        }
        "group" => for_children(s, &mut |c| scale_shape(c, ox, oy, sx, sy)),
        _ => {}
    }
}

pub fn rotate_point(p: P, cx: f64, cy: f64, deg: f64) -> P {
    let a = deg * (PI / 180.0);
    let (c, sn) = (a.cos(), a.sin());
    let (dx, dy) = (p[0] - cx, p[1] - cy);
    [cx + dx * c - dy * sn, cy + dx * sn + dy * c]
}

/// model.to_poly: editable path (rect corners, rounded corners and ellipses as Bézier nodes).
pub fn to_poly(s: &Shape) -> Shape {
    if matches!(s.kind(), "poly" | "group" | "image" | "text") {
        return s.clone();
    }
    let pts: Vec<Value> = match s.kind() {
        "circle" => {
            let (cx, cy, rx, ry) = (s.f("cx"), s.f("cy"), s.f("rx"), s.f("ry"));
            let (kx, ky) = (rx * KAPPA, ry * KAPPA);
            vec![json!({"x": cx + rx, "y": cy, "c1": [cx + rx, cy - ky], "c2": [cx + rx, cy + ky]}),
                 json!({"x": cx, "y": cy + ry, "c1": [cx + kx, cy + ry], "c2": [cx - kx, cy + ry]}),
                 json!({"x": cx - rx, "y": cy, "c1": [cx - rx, cy + ky], "c2": [cx - rx, cy - ky]}),
                 json!({"x": cx, "y": cy - ry, "c1": [cx - kx, cy - ry], "c2": [cx + kx, cy - ry]})]
        }
        "rect" => {
            let (x, y, w, h) = (s.f("x"), s.f("y"), s.f("w"), s.f("h"));
            let r = s.f("radius").min(w / 2.0).min(h / 2.0);
            if r > 1e-6 {
                let k = r * KAPPA;
                vec![json!({"x": x + r, "y": y, "c1": [x + r - k, y]}), json!({"x": x + w - r, "y": y, "c2": [x + w - r + k, y]}),
                     json!({"x": x + w, "y": y + r, "c1": [x + w, y + r - k]}), json!({"x": x + w, "y": y + h - r, "c2": [x + w, y + h - r + k]}),
                     json!({"x": x + w - r, "y": y + h, "c1": [x + w - r + k, y + h]}), json!({"x": x + r, "y": y + h, "c2": [x + r - k, y + h]}),
                     json!({"x": x, "y": y + h - r, "c1": [x, y + h - r + k]}), json!({"x": x, "y": y + r, "c2": [x, y + r - k]})]
            } else {
                vec![json!({"x": x, "y": y}), json!({"x": x + w, "y": y}), json!({"x": x + w, "y": y + h}), json!({"x": x, "y": y + h})]
            }
        }
        _ => {
            let p = s.paths().into_iter().next().unwrap_or_default();
            p[..p.len().saturating_sub(1)].iter().map(|q| json!({"x": q[0], "y": q[1]})).collect()
        }
    };
    let layer = s.0.get("layer").cloned().unwrap_or(json!(0));
    Shape(serde_json::from_value(json!({"id": s.0.get("id").cloned().unwrap_or(Value::Null), "type": "poly", "layer": layer,
                                        "locked": s.locked(), "points": pts, "closed": true})).unwrap())
}

/// A text shape as a group of closed outline polygons (children get ids from `ids`).
pub fn text_to_group(s: &Shape, ids: &mut dyn FnMut() -> Option<i64>) -> Shape {
    let kids: Vec<Value> = crate::text::text_paths(s).into_iter().map(|p| {
        let pts: Vec<Value> = p[..p.len().saturating_sub(1)].iter().map(|q| json!({"x": q[0], "y": q[1]})).collect();
        let closed = !crate::text::is_single_line(s);
        json!({"id": ids(), "type": "poly", "layer": s.layer(), "points": if closed { pts } else { p.iter().map(|q| json!({"x": q[0], "y": q[1]})).collect() }, "closed": closed})
    }).collect();
    Shape(serde_json::from_value(json!({"id": s.id(), "type": "group", "layer": s.layer(), "children": kids})).unwrap())
}

/// model.rotate_shape (in place). Text becomes outlines; images are re-rendered unless the turn is 180°.
pub fn rotate_shape(s: &mut Shape, cx: f64, cy: f64, deg: f64) -> bool {
    match s.kind() {
        "image" => {
            if deg.rem_euclid(180.0) == 0.0 && deg.rem_euclid(360.0) != 0.0 {
                let b = s.bounds();
                let n = rotate_point(center(b), cx, cy, deg);
                s.set("x", n[0] - b[2] / 2.0);
                s.set("y", n[1] - b[3] / 2.0);
                let (fx, fy) = (s.flag("flipX"), s.flag("flipY"));
                s.0.insert("flipX".into(), json!(!fx));
                s.0.insert("flipY".into(), json!(!fy));
                return true;
            }
            crate::bitmap::rotate_image(s, cx, cy, deg);
            return true;
        }
        "text" => {
            *s = text_to_group(s, &mut || None);
        }
        _ => {}
    }
    if s.kind() == "circle" && ((s.f("rx") - s.f("ry")).abs() < 1e-9 || deg.rem_euclid(180.0) == 0.0) {
        let c = rotate_point([s.f("cx"), s.f("cy")], cx, cy, deg);
        s.set("cx", c[0]);
        s.set("cy", c[1]);
        return true;
    }
    if s.kind() == "group" {
        let mut ok = true;
        for_children(s, &mut |c| ok &= rotate_shape(c, cx, cy, deg));
        return ok;
    }
    *s = to_poly(s);
    map_points(s, &|x, y| rotate_point([x, y], cx, cy, deg));
    if let Some(Value::Object(m)) = s.0.get_mut("regular") {
        let c = rotate_point([num(m.get("cx")), num(m.get("cy"))], cx, cy, deg);
        let rot = m.get("rot").and_then(|v| v.as_f64()).unwrap_or(-PI / 2.0) + deg * (PI / 180.0);
        m.insert("cx".into(), json!(c[0]));
        m.insert("cy".into(), json!(c[1]));
        m.insert("rot".into(), json!(rot));
    }
    true
}

/// model.mirror_shape: "h" mirrors left-right around cx, "v" top-bottom around cy.
pub fn mirror_shape(s: &mut Shape, axis: &str, cx: f64, cy: f64) {
    let h = axis == "h";
    match s.kind() {
        "rect" | "image" | "text" => {
            let b = s.bounds();
            let image = s.kind() == "image";
            if h {
                s.set("x", 2.0 * cx - (b[0] + b[2]));
            } else {
                s.set("y", 2.0 * cy - (b[1] + b[3]));
            }
            if image {
                let k = if h { "flipX" } else { "flipY" };
                let v = s.flag(k);
                s.0.insert(k.into(), json!(!v));
                if let Some(Value::Array(rings)) = s.0.get_mut("crop") {
                    for r in rings.iter_mut() {
                        if let Value::Array(pts) = r {
                            for p in pts.iter_mut() {
                                if let Some(q) = pt_of(p) {
                                    *p = if h { json!([1.0 - q[0], q[1]]) } else { json!([q[0], 1.0 - q[1]]) };
                                }
                            }
                        }
                    }
                }
            }
        }
        "circle" => {
            if h {
                let v = 2.0 * cx - s.f("cx");
                s.set("cx", v);
            } else {
                let v = 2.0 * cy - s.f("cy");
                s.set("cy", v);
            }
        }
        "poly" => {
            if h {
                map_points(s, &|x, y| [2.0 * cx - x, y]);
            } else {
                map_points(s, &|x, y| [x, 2.0 * cy - y]);
            }
            s.0.remove("regular");
        }
        "group" => for_children(s, &mut |c| mirror_shape(c, axis, cx, cy)),
        _ => {}
    }
}

/// model.set_live_crop: attach a closed shape as a crop path, relative to the image box.
pub fn set_live_crop(img: &mut Shape, crop: &Shape) -> bool {
    let (x, y, w, h) = (img.f("x"), img.f("y"), img.f("w"), img.f("h"));
    let rings: Vec<Value> = crop.paths().into_iter().filter(|p| p.len() > 3 && is_closed_path(p))
        .map(|p| json!(p.iter().map(|q| [(q[0] - x) / w, (q[1] - y) / h]).collect::<Vec<_>>())).collect();
    if rings.is_empty() {
        return false;
    }
    img.0.insert("crop".into(), json!(rings));
    true
}

fn reid_tree(s: &mut Shape, next: &mut i64) {
    s.0.insert("id".into(), json!(*next));
    *next += 1;
    for_children(s, &mut |c| reid_tree(c, next));
}

fn fix_child_ids(s: &mut Shape, next: &mut i64) {
    for_children(s, &mut |c| {
        if c.0.get("id").is_none_or(|v| v.is_null()) {
            c.0.insert("id".into(), json!(*next));
            *next += 1;
        }
        fix_child_ids(c, next);
    });
}

/// Where "move selection to" puts it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    UL, UR, DL, DR, C, L, R, U, D, Laser,
}

impl Doc {
    // ---------------------------------------------------------------- ids, selection, undo
    pub fn create(&mut self, kind: &str, props: Map<String, Value>) -> Shape {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.new_id()));
        m.insert("type".into(), json!(kind));
        m.insert("layer".into(), json!(self.active_layer));
        m.extend(props);
        Shape(m)
    }

    /// A shape from importer properties (which include "type").
    pub fn create_from(&mut self, mut props: Map<String, Value>) -> Shape {
        let kind = props.remove("type").and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_else(|| "poly".into());
        self.create(&kind, props)
    }

    pub fn reid(&mut self, mut s: Shape) -> Shape {
        let mut n = self.next_id;
        reid_tree(&mut s, &mut n);
        self.next_id = n;
        s
    }

    pub fn find(&self, id: i64) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id() == id)
    }

    pub fn find_mut(&mut self, id: i64) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id() == id)
    }

    pub fn selected_shapes(&self) -> Vec<Shape> {
        self.shapes.iter().filter(|s| self.selected.contains(&s.id())).cloned().collect()
    }

    /// Selection in click order (alignment uses the last one clicked).
    pub fn selected_in_order(&self) -> Vec<Shape> {
        self.selected.iter().filter_map(|i| self.find(*i).cloned()).collect()
    }

    pub fn is_selected(&self, id: i64) -> bool {
        self.selected.contains(&id)
    }

    pub fn set_selection(&mut self, ids: Vec<i64>) {
        let mut seen = Vec::new();
        for i in ids {
            if !seen.contains(&i) {
                seen.push(i);
            }
        }
        self.selected = seen;
    }

    pub fn save_undo(&mut self, label: &str) {
        self.undo.push((self.shapes.clone(), if label.is_empty() { l("Modifica", "Edit") } else { label.to_string() }));
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some((s, label)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.shapes, s), label));
            self.prune_selection();
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some((s, label)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.shapes, s), label));
            self.prune_selection();
            self.dirty = true;
        }
    }

    fn prune_selection(&mut self) {
        let ids: Vec<i64> = self.shapes.iter().map(|s| s.id()).collect();
        self.selected.retain(|i| ids.contains(i));
    }

    pub fn add_shapes(&mut self, shapes: Vec<Shape>, select: bool) {
        self.save_undo("");
        if select {
            self.selected = shapes.iter().map(|s| s.id()).collect();
        }
        self.shapes.extend(shapes);
    }

    /// The selection, or an error when fewer than `minimum` shapes are selected.
    pub fn need_sel(&self, minimum: usize) -> Result<Vec<Shape>, String> {
        let sel = self.selected_shapes();
        if sel.len() < minimum.max(1) {
            return Err(if minimum <= 1 { tr("selectFirst") } else { l(&format!("Seleziona almeno {minimum} oggetti"), &format!("Select at least {minimum} objects")) });
        }
        Ok(sel)
    }

    /// Apply `f` to every selected shape (in document order), with one undo step.
    pub fn edit_selected(&mut self, label: &str, mut f: impl FnMut(&mut Shape)) -> Msg {
        self.need_sel(1)?;
        self.save_undo(label);
        let sel = self.selected.clone();
        for s in self.shapes.iter_mut().filter(|s| sel.contains(&s.id())) {
            f(s);
        }
        Ok(None)
    }

    // ---------------------------------------------------------------- pages
    pub fn sync_page(&mut self) {
        let shapes = serde_json::to_value(&self.shapes).unwrap();
        if let Some(Value::Object(p)) = self.pages.get_mut(self.page) {
            p.insert("shapes".into(), shapes);
        }
    }

    pub fn set_page(&mut self, i: usize) {
        if i < self.pages.len() && i != self.page {
            self.sync_page();
            self.load_page(i);
            self.selected.clear();
            self.undo.clear();
            self.redo.clear();
        }
    }

    pub fn add_page(&mut self, name: Option<String>) {
        self.sync_page();
        let n = self.pages.len() + 1;
        self.pages.push(json!({"name": name.unwrap_or_else(|| format!("Pagina {n}")), "shapes": []}));
        self.dirty = true;
        let last = self.pages.len() - 1;
        self.set_page(last);
    }

    pub fn remove_page(&mut self, i: usize) {
        if self.pages.len() <= 1 || i >= self.pages.len() {
            return;
        }
        self.sync_page();
        self.pages.remove(i);
        let p = self.page.min(self.pages.len() - 1);
        self.load_page(p);
        self.selected.clear();
        self.undo.clear();
        self.redo.clear();
        self.dirty = true;
    }

    pub fn page_name(&self, i: usize) -> String {
        self.pages.get(i).and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    pub fn rename_page(&mut self, i: usize, name: &str) {
        if let Some(Value::Object(p)) = self.pages.get_mut(i) {
            p.insert("name".into(), json!(name));
            self.dirty = true;
        }
    }

    pub fn used_layers(&self) -> Vec<usize> {
        let mut v: Vec<usize> = flatten(&self.shapes).iter().map(|s| s.layer().max(0) as usize).collect();
        v.sort();
        v.dedup();
        v
    }

    // ---------------------------------------------------------------- selection commands
    pub fn select_by(&mut self, pred: impl Fn(&Shape) -> bool) {
        let ids = self.shapes.iter().filter(|s| !s.locked() && pred(s)).map(|s| s.id()).collect();
        self.set_selection(ids);
    }

    pub fn invert_selection(&mut self) {
        let cur = self.selected.clone();
        self.select_by(|s| !cur.contains(&s.id()));
    }

    pub fn select_contained(&mut self) -> Msg {
        let sel = self.need_sel(1)?;
        let boxes: Vec<[f64; 4]> = sel.iter().map(|s| s.bounds()).collect();
        let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
        self.select_by(|s| {
            let sb = s.bounds();
            !ids.contains(&s.id()) && boxes.iter().any(|b| sb[0] >= b[0] && sb[1] >= b[1] && sb[0] + sb[2] <= b[0] + b[2] && sb[1] + sb[3] <= b[1] + b[3])
        });
        Ok(None)
    }

    /// Every shape whose box lies inside another closed shape.
    pub fn select_nested(&mut self) -> Msg {
        let polys: Vec<(i64, Vec<P>)> = self.shapes.iter().filter_map(|s| {
            let p = s.paths().into_iter().next()?;
            (p.len() > 3 && is_closed_path(&p)).then_some((s.id(), p))
        }).collect();
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
        let ids: Vec<i64> = self.shapes.iter().filter(|s| {
            let b = s.bounds();
            let corners = [[b[0], b[1]], [b[0] + b[2], b[1]], [b[0] + b[2], b[1] + b[3]], [b[0], b[1] + b[3]]];
            polys.iter().any(|(pid, pg)| *pid != s.id() && corners.iter().all(|c| inside(*c, pg))
                && !pg.windows(2).any(|w| w[0][0] > b[0] && w[0][0] < b[0] + b[2] && w[0][1] > b[1] && w[0][1] < b[1] + b[3]))
        }).map(|s| s.id()).collect();
        let n = ids.len();
        self.set_selection(ids);
        Ok(Some(l(&format!("{n} forme annidate"), &format!("{n} nested shapes"))))
    }

    pub fn select_same_circles(&mut self) -> Msg {
        let r = self.selected_shapes().into_iter().find(|s| s.kind() == "circle").ok_or_else(|| l("Seleziona un cerchio", "Select a circle"))?;
        let (rx, ry) = (r.f("rx"), r.f("ry"));
        self.select_by(|s| s.kind() == "circle" && (s.f("rx") - rx).abs() <= 0.05 && (s.f("ry") - ry).abs() <= 0.05);
        Ok(None)
    }

    pub fn select_smaller(&mut self) -> Msg {
        let sel = self.need_sel(1)?;
        let limit = sel.iter().map(|s| { let b = s.bounds(); b[2] * b[3] }).fold(f64::MIN, f64::max);
        self.select_by(|s| { let b = s.bounds(); b[2] * b[3] < limit });
        Ok(None)
    }

    // ---------------------------------------------------------------- clipboard and basic edits
    pub fn delete_selected(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        self.save_undo(&l("Elimina", "Delete"));
        let sel = std::mem::take(&mut self.selected);
        self.shapes.retain(|s| !sel.contains(&s.id()) || s.locked());
    }

    pub fn paste(&mut self, clip: &[Shape], at: Option<P>) {
        if clip.is_empty() {
            return;
        }
        let items: Vec<Shape> = clip.iter().map(|s| self.reid(s.clone())).collect();
        let mut items = items;
        if let Some(b) = multi_bounds(&items) {
            let target = at.unwrap_or([b[0] + 10.0 + b[2] / 2.0, b[1] + 10.0 + b[3] / 2.0]);
            for s in items.iter_mut() {
                s.translate(target[0] - b[0] - b[2] / 2.0, target[1] - b[1] - b[3] / 2.0);
            }
        }
        self.add_shapes(items, true);
    }

    pub fn paste_in_place(&mut self, clip: &[Shape]) {
        let items: Vec<Shape> = clip.iter().map(|s| self.reid(s.clone())).collect();
        if !items.is_empty() {
            self.add_shapes(items, true);
        }
    }

    pub fn duplicate_sel(&mut self) -> Msg {
        let sel = self.need_sel(1)?;
        let items: Vec<Shape> = sel.into_iter().map(|s| self.reid(s)).collect();
        self.add_shapes(items, true);
        Ok(None)
    }

    pub fn group_sel(&mut self) -> Msg {
        let sel = self.need_sel(2)?;
        self.save_undo(&l("Raggruppa", "Group"));
        let layer = sel[0].layer();
        let mut props = Map::new();
        props.insert("children".into(), serde_json::to_value(&sel).unwrap());
        let mut g = self.create("group", props);
        g.0.insert("layer".into(), json!(layer));
        let first = self.shapes.iter().position(|s| self.selected.contains(&s.id())).unwrap_or(0);
        let ids = self.selected.clone();
        self.shapes.retain(|s| !ids.contains(&s.id()));
        let gid = g.id();
        self.shapes.insert(first.min(self.shapes.len()), g);
        self.selected = vec![gid];
        Ok(None)
    }

    pub fn ungroup_sel(&mut self) -> Msg {
        let groups: Vec<i64> = self.selected_shapes().iter().filter(|s| s.kind() == "group").map(|s| s.id()).collect();
        if groups.is_empty() {
            return Ok(None);
        }
        self.save_undo(&l("Separa", "Ungroup"));
        let mut ids = Vec::new();
        for gid in groups {
            let Some(idx) = self.shapes.iter().position(|s| s.id() == gid) else { continue };
            let g = self.shapes[idx].clone();
            let mut kids = Vec::new();
            for mut c in g.children() {
                if c.layer_opt().is_none() || c.0.get("layer").is_none_or(|v| v.is_null()) {
                    c.0.insert("layer".into(), json!(g.layer()));
                }
                if c.0.get("id").is_none_or(|v| v.is_null()) {
                    c.0.insert("id".into(), json!(self.new_id()));
                }
                ids.push(c.id());
                kids.push(c);
            }
            self.shapes.splice(idx..idx + 1, kids);
        }
        self.selected = ids;
        Ok(None)
    }

    pub fn mirror_sel(&mut self, axis: &str) -> Msg {
        let sel = self.need_sel(1)?;
        let c = center(multi_bounds(&sel).unwrap());
        let a = axis.to_string();
        self.edit_selected(&l("Specchia", "Mirror"), |s| mirror_shape(s, &a, c[0], c[1]))
    }

    pub fn rotate_selection(&mut self, deg: f64) -> Msg {
        let sel = self.need_sel(1)?;
        let c = center(multi_bounds(&sel).unwrap());
        self.save_undo(&l("Ruota", "Rotate"));
        let ids = self.selected.clone();
        let mut next = self.next_id;
        let mut skipped = 0;
        for s in self.shapes.iter_mut().filter(|s| ids.contains(&s.id())) {
            if !rotate_shape(s, c[0], c[1], deg) {
                skipped += 1;
            }
            fix_child_ids(s, &mut next);
        }
        self.next_id = next;
        if skipped > 0 {
            return Ok(Some(tr("rotateImageNA")));
        }
        Ok(None)
    }

    pub fn align_sel(&mut self, mode: &str) -> Msg {
        self.need_sel(2)?;
        let r = self.selected_in_order().last().unwrap().bounds();
        let mode = mode.to_string();
        self.edit_selected(&l("Allinea", "Align"), |s| {
            let b = s.bounds();
            let (mut dx, mut dy) = (0.0, 0.0);
            match mode.as_str() {
                "left" => dx = r[0] - b[0],
                "right" => dx = r[0] + r[2] - b[0] - b[2],
                "top" => dy = r[1] - b[1],
                "bottom" => dy = r[1] + r[3] - b[1] - b[3],
                _ => {}
            }
            if mode == "centerH" || mode == "centers" {
                dx = r[0] + r[2] / 2.0 - b[0] - b[2] / 2.0;
            }
            if mode == "centerV" || mode == "centers" {
                dy = r[1] + r[3] / 2.0 - b[1] - b[3] / 2.0;
            }
            s.translate(dx, dy);
        })
    }

    pub fn distribute_sel(&mut self, horizontal: bool, centers: bool) -> Msg {
        let sel = self.need_sel(3)?;
        let (k, sz) = if horizontal { (0, 2) } else { (1, 3) };
        let mut items: Vec<(i64, [f64; 4])> = sel.iter().map(|s| (s.id(), s.bounds())).collect();
        items.sort_by(|a, b| (a.1[k] + a.1[sz] / 2.0).total_cmp(&(b.1[k] + b.1[sz] / 2.0)));
        self.save_undo(&l("Distribuisci", "Distribute"));
        let (first, last, n) = (items[0].1, items[items.len() - 1].1, items.len());
        let mut moves: Vec<(i64, f64)> = Vec::new();
        if centers {
            let (c0, c1) = (first[k] + first[sz] / 2.0, last[k] + last[sz] / 2.0);
            for (i, (id, b)) in items.iter().enumerate() {
                moves.push((*id, c0 + (c1 - c0) * i as f64 / (n - 1) as f64 - (b[k] + b[sz] / 2.0)));
            }
        } else {
            let total: f64 = items.iter().map(|(_, b)| b[sz]).sum();
            let gap = (last[k] + last[sz] - first[k] - total) / (n - 1) as f64;
            let mut pos = first[k];
            for (id, b) in &items {
                moves.push((*id, pos - b[k]));
                pos += b[sz] + gap;
            }
        }
        for (id, d) in moves {
            if let Some(s) = self.find_mut(id) {
                if horizontal { s.translate(d, 0.0) } else { s.translate(0.0, d) }
            }
        }
        Ok(None)
    }

    pub fn move_sel_to(&mut self, place: Place, ws: (f64, f64), head: P) -> Msg {
        let sel = self.need_sel(1)?;
        let b = multi_bounds(&sel).unwrap();
        let (w, h) = ws;
        let (mut tx, mut ty) = (b[0], b[1]);
        match place {
            Place::UL => { tx = 0.0; ty = 0.0; }
            Place::UR => { tx = w - b[2]; ty = 0.0; }
            Place::DL => { tx = 0.0; ty = h - b[3]; }
            Place::DR => { tx = w - b[2]; ty = h - b[3]; }
            Place::L => tx = 0.0,
            Place::R => tx = w - b[2],
            Place::U => ty = 0.0,
            Place::D => ty = h - b[3],
            Place::C => { tx = (w - b[2]) / 2.0; ty = (h - b[3]) / 2.0; }
            Place::Laser => { tx = head[0] - b[2] / 2.0; ty = head[1] - b[3] / 2.0; }
        }
        self.edit_selected(&l("Sposta", "Move"), |s| s.translate(tx - b[0], ty - b[1]))
    }

    pub fn same_size(&mut self, width: bool) -> Msg {
        self.need_sel(2)?;
        let r = self.selected_in_order().last().unwrap().bounds();
        self.edit_selected(&l("Stessa dimensione", "Same size"), |s| {
            let b = s.bounds();
            if width && b[2] != 0.0 {
                scale_shape(s, b[0], b[1], r[2] / b[2], 1.0);
            }
            if !width && b[3] != 0.0 {
                scale_shape(s, b[0], b[1], 1.0, r[3] / b[3]);
            }
        })
    }

    pub fn reorder(&mut self, how: &str) -> Msg {
        self.need_sel(1)?;
        self.save_undo(&l("Ordine", "Order"));
        let sel = self.selected.clone();
        let is_sel = |s: &Shape| sel.contains(&s.id());
        match how {
            "front" | "back" => {
                let (a, b): (Vec<Shape>, Vec<Shape>) = self.shapes.drain(..).partition(|s| is_sel(s));
                self.shapes = if how == "front" { b.into_iter().chain(a).collect() } else { a.into_iter().chain(b).collect() };
            }
            _ => {
                let n = self.shapes.len();
                let fwd = how == "forward";
                let order: Vec<usize> = if fwd { (0..n).rev().collect() } else { (0..n).collect() };
                for i in order {
                    let j = if fwd { i as i64 + 1 } else { i as i64 - 1 };
                    if j >= 0 && (j as usize) < n && is_sel(&self.shapes[i]) && !is_sel(&self.shapes[j as usize]) {
                        self.shapes.swap(i, j as usize);
                    }
                }
            }
        }
        Ok(None)
    }

    pub fn delete_duplicates(&mut self) -> Msg {
        let mut seen = std::collections::HashSet::new();
        let mut keep = Vec::new();
        let mut removed = 0;
        for s in &self.shapes {
            let mut c = s.clone();
            c.0.insert("id".into(), json!(0));
            let sig = serde_json::to_string(&c.0).unwrap();
            if seen.insert(sig) {
                keep.push(s.clone());
            } else {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(Some(l("Nessun duplicato", "No duplicates")));
        }
        self.save_undo(&l("Elimina duplicati", "Delete duplicates"));
        self.shapes = keep;
        Ok(Some(l(&format!("{removed} duplicati rimossi"), &format!("{removed} duplicates removed"))))
    }

    pub fn close_path(&mut self) -> Msg {
        let sel = self.need_sel(1)?;
        if !sel.iter().any(|s| s.kind() == "poly" && !s.flag("closed")) {
            return Ok(None);
        }
        self.edit_selected(&l("Chiudi tracciato", "Close path"), |s| {
            if s.kind() == "poly" {
                s.0.insert("closed".into(), json!(true));
            }
        })
    }

    pub fn convert_to_path(&mut self) -> Msg {
        self.need_sel(1)?;
        self.save_undo(&l("Converti in tracciato", "Convert to path"));
        let sel = self.selected.clone();
        let mut next = self.next_id;
        for s in self.shapes.iter_mut().filter(|s| sel.contains(&s.id())) {
            *s = if s.kind() == "text" {
                text_to_group(s, &mut || { next += 1; Some(next - 1) })
            } else {
                to_poly(s)
            };
        }
        self.next_id = next;
        Ok(None)
    }

    pub fn set_locked(&mut self, v: bool) -> Msg {
        self.edit_selected(&l("Blocca", "Lock"), |s| { s.0.insert("locked".into(), json!(v)); })?;
        if v {
            self.selected.clear();
        }
        Ok(None)
    }

    pub fn set_shape_attr(&mut self, key: &str, value: Option<Value>) -> Msg {
        let k = key.to_string();
        self.edit_selected("", |s| match &value {
            None => { s.0.remove(&k); }
            Some(v) => { s.0.insert(k.clone(), v.clone()); }
        })
    }

    pub fn reset_start_points(&mut self) -> Msg {
        self.edit_selected(&l("Reimposta inizio", "Reset start"), |s| {
            s.0.remove("start");
            s.0.remove("reverse");
        })
    }

    /// Draw-shape menu: triangle, pentagon, octagon, star, dual star, centred on the work area.
    pub fn insert_shape(&mut self, kind: &str, ws: (f64, f64)) {
        let (cx, cy, r) = (ws.0 / 2.0, ws.1 / 2.0, 25.0);
        let (n, inner) = match kind { "triangle" => (3, 0.0), "pentagon" => (5, 0.0), "octagon" => (8, 0.0), "star" => (5, 0.45), _ => (10, 0.75) };
        let rot = -PI / 2.0 + if kind == "octagon" { PI / 8.0 } else { 0.0 };
        let pts = if inner > 0.0 { star_points(cx, cy, r, r * inner, n) } else { regular_polygon(cx, cy, r, n, rot) };
        let mut regular = json!({"cx": cx, "cy": cy, "r": r, "n": n, "rot": rot});
        if inner > 0.0 {
            regular["inner"] = json!(inner);
        }
        let mut props = Map::new();
        props.insert("points".into(), json!(pts));
        props.insert("closed".into(), json!(true));
        props.insert("regular".into(), regular);
        let s = self.create("poly", props);
        self.add_shapes(vec![s], true);
    }

    pub fn flatten_arrays(&mut self) -> Msg {
        let arrays: Vec<i64> = self.selected_shapes().iter().filter(|s| s.kind() == "group" && s.0.contains_key("array")).map(|s| s.id()).collect();
        if arrays.is_empty() {
            return Err(l("Seleziona un array dinamico", "Select a live array"));
        }
        self.save_undo("");
        for id in arrays {
            if let Some(s) = self.find_mut(id) {
                s.0.remove("array");
            }
        }
        Ok(None)
    }

    /// Join open paths whose ends are within `tol`; paths that end where they start become closed.
    pub fn auto_join(&mut self, tol: f64) -> Msg {
        let sel: Vec<Shape> = self.need_sel(1)?.into_iter().filter(|s| s.kind() == "poly" && !s.flag("closed")).collect();
        if sel.is_empty() {
            return Ok(None);
        }
        let near = |a: &Value, b: &Value| (num(a.get("x")) - num(b.get("x"))).hypot(num(a.get("y")) - num(b.get("y"))) <= tol;
        let pts = |s: &Shape| s.0.get("points").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let mut pool: Vec<Shape> = sel.clone();
        let mut done = Vec::new();
        while !pool.is_empty() {
            let mut cur = pool.remove(0);
            let mut grew = true;
            while grew {
                grew = false;
                for i in 0..pool.len() {
                    let (a, b) = (pts(&cur), pts(&pool[i]));
                    if a.is_empty() || b.is_empty() {
                        continue;
                    }
                    let rev = |v: &[Value]| v.iter().rev().cloned().collect::<Vec<_>>();
                    let merged: Option<Vec<Value>> = if near(a.last().unwrap(), &b[0]) {
                        Some(a.iter().chain(&b[1..]).cloned().collect())
                    } else if near(a.last().unwrap(), b.last().unwrap()) {
                        Some(a.iter().cloned().chain(rev(&b[..b.len() - 1])).collect())
                    } else if near(&a[0], b.last().unwrap()) {
                        Some(b.iter().chain(&a[1..]).cloned().collect())
                    } else if near(&a[0], &b[0]) {
                        Some(rev(&b).into_iter().chain(a[1..].iter().cloned()).collect())
                    } else {
                        None
                    };
                    if let Some(m) = merged {
                        cur.0.insert("points".into(), json!(m));
                        pool.remove(i);
                        grew = true;
                        break;
                    }
                }
            }
            let mut p = pts(&cur);
            if p.len() > 2 && near(&p[0], p.last().unwrap()) {
                p.pop();
                cur.0.insert("points".into(), json!(p));
                cur.0.insert("closed".into(), json!(true));
            }
            done.push(cur);
        }
        self.save_undo(&l("Unisci automaticamente", "Auto join"));
        let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
        self.shapes.retain(|s| !ids.contains(&s.id()));
        self.selected = done.iter().map(|s| s.id()).collect();
        let (a, b) = (sel.len(), done.len());
        self.shapes.extend(done);
        Ok(Some(format!("{a} → {b}")))
    }

    pub fn close_with_tolerance(&mut self, tol: f64) -> Msg {
        self.auto_join(tol)?;
        let mut closed = 0;
        let sel = self.selected.clone();
        for s in self.shapes.iter_mut().filter(|s| sel.contains(&s.id())) {
            if s.kind() == "poly" && !s.flag("closed") {
                if let Some(Value::Array(p)) = s.0.get_mut("points") {
                    if p.len() > 2 {
                        let d = (num(p[0].get("x")) - num(p[p.len() - 1].get("x"))).hypot(num(p[0].get("y")) - num(p[p.len() - 1].get("y")));
                        if d <= tol {
                            if d < 1e-9 {
                                p.pop();
                            }
                            s.0.insert("closed".into(), json!(true));
                            closed += 1;
                        }
                    }
                }
            }
        }
        Ok(Some(l(&format!("{closed} forme chiuse"), &format!("{closed} shapes closed"))))
    }

    pub fn break_apart(&mut self) -> Msg {
        let sel = self.need_sel(1)?;
        self.save_undo(&l("Scomponi", "Break apart"));
        let mut out = Vec::new();
        let mut next = self.next_id;
        fn explode(s: Shape, out: &mut Vec<Shape>, next: &mut i64) {
            match s.kind() {
                "group" => {
                    for mut c in s.children() {
                        if c.0.get("layer").is_none_or(|v| v.is_null()) {
                            c.0.insert("layer".into(), json!(s.layer()));
                        }
                        explode(c, out, next);
                    }
                }
                "text" => out.extend(text_to_group(&s, &mut || { *next += 1; Some(*next - 1) }).children()),
                _ => {
                    let mut s = s;
                    if s.0.get("id").is_none_or(|v| v.is_null()) {
                        s.0.insert("id".into(), json!(*next));
                        *next += 1;
                    }
                    out.push(s);
                }
            }
        }
        for s in &sel {
            explode(s.clone(), &mut out, &mut next);
        }
        self.next_id = next;
        let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
        self.shapes.retain(|s| !ids.contains(&s.id()));
        self.selected = out.iter().map(|s| s.id()).collect();
        self.shapes.extend(out);
        Ok(None)
    }

    /// Mirror copies of the selection across the straight line selected last.
    pub fn mirror_across_line(&mut self) -> Msg {
        let sel = self.selected_in_order();
        let line = sel.last().filter(|s| s.kind() == "poly" && s.nodes().len() == 2 && sel.len() >= 2)
            .ok_or_else(|| l("Seleziona gli oggetti e per ultima una linea retta", "Select the objects, then a straight line last"))?;
        let n = line.nodes();
        let (a, b) = (n[0].p, n[1].p);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let refl = move |x: f64, y: f64| -> P {
            let t = ((x - a[0]) * dx + (y - a[1]) * dy) / l2;
            let (fx, fy) = (a[0] + t * dx, a[1] + t * dy);
            [2.0 * fx - x, 2.0 * fy - y]
        };
        fn mirror(s: &mut Shape, refl: &dyn Fn(f64, f64) -> P) -> bool {
            match s.kind() {
                "group" => {
                    let mut ok = true;
                    for_children(s, &mut |c| ok &= mirror(c, refl));
                    ok
                }
                "text" | "image" => false,
                "circle" if (s.f("rx") - s.f("ry")).abs() < 1e-9 => {
                    let c = refl(s.f("cx"), s.f("cy"));
                    s.set("cx", c[0]);
                    s.set("cy", c[1]);
                    true
                }
                _ => {
                    *s = to_poly(s);
                    map_points(s, refl);
                    s.0.remove("regular");
                    true
                }
            }
        }
        let mut copies = Vec::new();
        for s in &sel[..sel.len() - 1] {
            let mut c = self.reid(s.clone());
            if mirror(&mut c, &refl) {
                copies.push(c);
            }
        }
        if !copies.is_empty() {
            self.add_shapes(copies, true);
        }
        Ok(None)
    }

    /// Replace the selection with the same lines, each overlapping piece kept once (shapely unary_union + linemerge).
    pub fn delete_overlapping(&mut self) -> Msg {
        let sel: Vec<Shape> = self.selected_shapes().into_iter().filter(|s| s.kind() != "image").collect();
        if sel.is_empty() {
            return Err(tr("selectFirst"));
        }
        let paths: Vec<Vec<P>> = sel.iter().flat_map(|s| s.paths()).filter(|p| p.len() >= 2).collect();
        let before: f64 = paths.iter().map(|p| crate::tools::path_length(p)).sum();
        let merged = merge_lines(&paths);
        let layer = sel[0].layer();
        let mut new = Vec::new();
        for mut pts in merged.clone() {
            let closed = pts.len() > 3 && (pts[0][0] - pts[pts.len() - 1][0]).hypot(pts[0][1] - pts[pts.len() - 1][1]) < 1e-6;
            if closed {
                pts.pop();
            }
            new.push(Shape(serde_json::from_value(json!({"id": self.new_id(), "type": "poly", "layer": layer, "closed": closed,
                "points": pts.iter().map(|q| json!({"x": q[0], "y": q[1]})).collect::<Vec<_>>()})).unwrap()));
        }
        let after: f64 = merged.iter().map(|p| crate::tools::path_length(p)).sum();
        self.save_undo(&l("Elimina sovrapposizioni", "Delete overlaps"));
        let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
        self.shapes.retain(|s| !ids.contains(&s.id()));
        self.selected = new.iter().map(|n| n.id()).collect();
        self.shapes.extend(new);
        Ok(Some(l(&format!("Rimossi {:.1} mm di linee sovrapposte", before - after), &format!("Removed {:.1} mm of overlapping lines", before - after))))
    }

    // ---------------------------------------------------------------- images
    pub fn selected_image_id(&self) -> Result<i64, String> {
        self.selected_shapes().iter().find(|s| s.kind() == "image").map(|s| s.id()).ok_or_else(|| tr("selectImage"))
    }

    pub fn crop_image(&mut self) -> Msg {
        let sel = self.selected_shapes();
        let (Some(img), Some(crop)) = (sel.iter().find(|s| s.kind() == "image"), sel.iter().find(|s| s.kind() != "image")) else {
            return Err(l("Seleziona un'immagine e una forma di ritaglio", "Select an image and a crop shape"));
        };
        let mut im = img.clone();
        if !set_live_crop(&mut im, crop) {
            return Err(l("La forma di ritaglio deve essere chiusa", "The crop shape must be closed"));
        }
        self.save_undo(&l("Ritaglia immagine", "Crop image"));
        let (iid, cid) = (img.id(), crop.id());
        self.shapes.retain(|s| s.id() != cid);
        if let Some(s) = self.find_mut(iid) {
            *s = im;
        }
        self.selected = vec![iid];
        Ok(None)
    }

    pub fn remove_crop(&mut self) -> Msg {
        let id = self.selected_image_id()?;
        if self.find(id).is_some_and(|s| s.0.contains_key("crop")) {
            self.save_undo("");
            self.find_mut(id).unwrap().0.remove("crop");
        }
        Ok(None)
    }

    pub fn flatten_crop(&mut self) -> Msg {
        let id = self.selected_image_id()?;
        if self.find(id).is_some_and(|s| s.0.contains_key("crop")) {
            self.save_undo("");
            crate::bitmap::flatten_crop(self.find_mut(id).unwrap());
        }
        Ok(None)
    }

    // ---------------------------------------------------------------- geometry
    /// Boolean / offset / hull operations on the selection (engine.geometry); results replace it unless `replace` is false.
    pub fn geometry_op(&mut self, op: &str, params: &Value, replace: bool, minimum: usize) -> Msg {
        let sel: Vec<Shape> = self.selected_in_order().into_iter().filter(|s| s.kind() != "image").collect();
        if sel.len() < minimum.max(1) {
            return Err(if minimum > 1 { l(&format!("Seleziona almeno {minimum} forme vettoriali"), &format!("Select at least {minimum} vector shapes")) } else { tr("selectFirst") });
        }
        let shapes: Vec<Vec<Vec<P>>> = sel.iter().map(|s| s.paths()).collect();
        let results = laserforge_core::geometry::geometry_op(op, &shapes.iter().map(|s| s.iter().map(|p| p.iter().map(|q| laserforge_core::model::Pt { x: q[0], y: q[1] }).collect()).collect()).collect::<Vec<_>>(), params)?;
        let layer = sel[0].layer();
        let mut created = Vec::new();
        for rings in results {
            let polys: Vec<Shape> = rings.iter().filter(|r| r.len() > 3).map(|r| {
                let pts: Vec<Value> = r[..r.len() - 1].iter().map(|q| json!({"x": q.x, "y": q.y})).collect();
                Shape(serde_json::from_value(json!({"id": self.new_id(), "type": "poly", "layer": layer, "points": pts, "closed": true})).unwrap())
            }).collect();
            if polys.len() == 1 {
                created.push(polys.into_iter().next().unwrap());
            } else if !polys.is_empty() {
                created.push(Shape(serde_json::from_value(json!({"id": self.new_id(), "type": "group", "layer": layer, "children": polys})).unwrap()));
            }
        }
        if created.is_empty() {
            return Err(l("Risultato vuoto", "Empty result"));
        }
        self.save_undo("");
        if replace {
            let ids: Vec<i64> = sel.iter().map(|s| s.id()).collect();
            self.shapes.retain(|s| !ids.contains(&s.id()));
        }
        self.selected = created.iter().map(|s| s.id()).collect();
        self.shapes.extend(created);
        Ok(None)
    }

    /// Simple shelf nesting: largest first, left to right in rows.
    pub fn nesting(&mut self, ws: (f64, f64)) -> Msg {
        let items: Vec<Shape> = if self.selected.is_empty() { self.shapes.clone() } else { self.selected_shapes() }
            .into_iter().filter(|s| !s.locked()).collect();
        if items.is_empty() {
            return Ok(None);
        }
        let gap = 3.0;
        self.save_undo(&l("Disponi", "Nest"));
        let (mut x, mut y, mut row_h, mut overflow) = (gap, gap, 0.0f64, false);
        let mut list: Vec<(i64, [f64; 4])> = items.iter().map(|s| (s.id(), s.bounds())).collect();
        list.sort_by(|a, b| b.1[3].total_cmp(&a.1[3]));
        for (id, b) in list {
            if x + b[2] > ws.0 - gap && x > gap {
                x = gap;
                y += row_h + gap;
                row_h = 0.0;
            }
            if y + b[3] > ws.1 {
                overflow = true;
            }
            if let Some(s) = self.find_mut(id) {
                s.translate(x - b[0], y - b[1]);
            }
            x += b[2] + gap;
            row_h = row_h.max(b[3]);
        }
        Ok(overflow.then(|| l("Non tutto entra nell'area di lavoro", "Not everything fits in the work area")))
    }

    /// Selected shapes moved by (dx, dy) and/or set to a box, from the transform fields (X, Y, W, H, rotation).
    pub fn apply_transform_field(&mut self, key: &str, v: f64, lock_aspect: bool) -> Msg {
        let sel = self.need_sel(1)?;
        let b = multi_bounds(&sel).unwrap();
        if key == "r" {
            if v != 0.0 {
                return self.rotate_selection(v);
            }
            return Ok(None);
        }
        let cur = match key { "x" => b[0], "y" => b[1], "w" => b[2], _ => b[3] };
        if (cur - v).abs() < 1e-4 {
            return Ok(None);
        }
        match key {
            "x" => self.edit_selected("", |s| s.translate(v - b[0], 0.0)),
            "y" => self.edit_selected("", |s| s.translate(0.0, v - b[1])),
            _ if v > 0.0 => {
                let mut sx = if key == "w" { v / if b[2] != 0.0 { b[2] } else { 1.0 } } else { 1.0 };
                let mut sy = if key == "h" { v / if b[3] != 0.0 { b[3] } else { 1.0 } } else { 1.0 };
                if lock_aspect {
                    if key == "w" { sy = sx } else { sx = sy }
                }
                self.edit_selected("", |s| scale_shape(s, b[0], b[1], sx, sy))
            }
            _ => Ok(None),
        }
    }

    /// Move every selected shape to a layer.
    pub fn set_layer_sel(&mut self, layer: usize) -> Msg {
        self.edit_selected(&l("Cambia livello", "Change layer"), |s| s.set_layer(layer))
    }
}

/// Union of line segments with overlaps kept once, merged into the longest possible polylines.
pub fn merge_lines(paths: &[Vec<P>]) -> Vec<Vec<P>> {
    let q = |v: f64| (v * 1e6).round() / 1e6;
    let mut segs: Vec<(P, P)> = Vec::new();
    for p in paths {
        for w in p.windows(2) {
            let (a, b) = ([q(w[0][0]), q(w[0][1])], [q(w[1][0]), q(w[1][1])]);
            if a != b {
                segs.push((a, b));
            }
        }
    }
    // split every segment at points of the others that lie on it (overlaps and crossings)
    let mut pieces: Vec<(P, P)> = Vec::new();
    for (i, &(a, b)) in segs.iter().enumerate() {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let mut ts = vec![0.0, 1.0];
        for (j, &(c, d)) in segs.iter().enumerate() {
            if i == j {
                continue;
            }
            for p in [c, d] {
                let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2;
                if t > 1e-9 && t < 1.0 - 1e-9 && crate::shapes::dist_to_segment(p, a, b) < 1e-6 {
                    ts.push(t);
                }
            }
            let r = [d[0] - c[0], d[1] - c[1]];
            let den = dx * r[1] - dy * r[0];
            if den.abs() > 1e-12 {
                let t = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
                let u = ((c[0] - a[0]) * dy - (c[1] - a[1]) * dx) / den;
                if t > 1e-9 && t < 1.0 - 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&u) {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(|x, y| x.total_cmp(y));
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        for w in ts.windows(2) {
            let p0 = [q(a[0] + dx * w[0]), q(a[1] + dy * w[0])];
            let p1 = [q(a[0] + dx * w[1]), q(a[1] + dy * w[1])];
            if p0 != p1 {
                pieces.push((p0, p1));
            }
        }
    }
    let key = |p: P| ((p[0] * 1e5).round() as i64, (p[1] * 1e5).round() as i64);
    let mut seen = std::collections::HashSet::new();
    let mut uniq: Vec<(P, P)> = Vec::new();
    for (a, b) in pieces {
        let (ka, kb) = (key(a), key(b));
        if seen.insert(if ka <= kb { (ka, kb) } else { (kb, ka) }) {
            uniq.push((a, b));
        }
    }
    // linemerge: walk chains through nodes with exactly two segments
    let mut by_node: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (i, (a, b)) in uniq.iter().enumerate() {
        by_node.entry(key(*a)).or_default().push(i);
        by_node.entry(key(*b)).or_default().push(i);
    }
    let mut used = vec![false; uniq.len()];
    let mut out = Vec::new();
    let walk = |start: usize, from: P, used: &mut Vec<bool>| -> Vec<P> {
        let mut line = vec![from];
        let mut cur = start;
        let mut at = from;
        loop {
            used[cur] = true;
            let (a, b) = uniq[cur];
            let next_pt = if key(a) == key(at) { b } else { a };
            line.push(next_pt);
            at = next_pt;
            let nb = &by_node[&key(at)];
            if nb.len() != 2 {
                break;
            }
            match nb.iter().find(|&&k| !used[k]) {
                Some(&k) => cur = k,
                None => break,
            }
        }
        line
    };
    // start from line ends first (degree != 2), then the remaining closed loops
    for (k, segs_at) in by_node.clone() {
        if segs_at.len() != 2 {
            for &i in &segs_at {
                if !used[i] {
                    let (a, b) = uniq[i];
                    let from = if key(a) == k { a } else { b };
                    out.push(walk(i, from, &mut used));
                }
            }
        }
    }
    for i in 0..uniq.len() {
        if !used[i] {
            out.push(walk(i, uniq[i].0, &mut used));
        }
    }
    out
}

/// The crop mask exposed for the bitmap module.
pub fn mask(rings: &[Vec<P>], cols: usize, rows: usize) -> Vec<u8> {
    crop_mask(rings, cols, rows)
}
