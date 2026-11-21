//! Corner and shape tools: fillet, dogbone / T-bone reliefs, slot resizing, Apollonius tangent circles, box and
//! gear generators, 1D barcodes. Ported from laserforge/corners.py, tools.py, dialogs.py and devices.py.

use crate::shapes::{Shape, P};
use crate::tools::{pts, set_pts};
use serde_json::{json, Map, Value};
use std::f64::consts::{PI, TAU};

fn unit(x: f64, y: f64) -> P {
    let l = x.hypot(y);
    if l > 1e-12 { [x / l, y / l] } else { [0.0, 0.0] }
}

fn xy(p: &Map<String, Value>) -> P {
    [p.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0), p.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0)]
}

/// (A, P, B) for node i when both neighbouring segments are straight.
fn corner(p: &[Map<String, Value>], i: usize, closed: bool) -> Option<(P, P, P)> {
    let n = p.len();
    if !closed && (i == 0 || i == n - 1) {
        return None;
    }
    let (a, c, b) = (&p[(i + n - 1) % n], &p[i], &p[(i + 1) % n]);
    if a.contains_key("c2") || c.contains_key("c1") || c.contains_key("c2") || b.contains_key("c1") {
        return None;
    }
    Some((xy(a), xy(c), xy(b)))
}

/// Bézier nodes for the arc of circle (c, r) from e1 to e2 through `through`.
fn arc_nodes(c: P, r: f64, e1: P, e2: P, through: P) -> Vec<Map<String, Value>> {
    let a1 = (e1[1] - c[1]).atan2(e1[0] - c[0]);
    let a2 = (e2[1] - c[1]).atan2(e2[0] - c[0]);
    let ap = (through[1] - c[1]).atan2(through[0] - c[0]);
    let ccw = (a2 - a1).rem_euclid(TAU);
    let sweep = if (ap - a1).rem_euclid(TAU) <= ccw { ccw } else { ccw - TAU };
    let segs = ((sweep.abs() / (PI / 2.0)).ceil() as usize).max(1);
    let step = sweep / segs as f64;
    let h = 4.0 / 3.0 * (step / 4.0).tan() * r;
    (0..=segs).map(|k| {
        let ang = a1 + step * k as f64;
        let (x, y) = (c[0] + r * ang.cos(), c[1] + r * ang.sin());
        let (tx, ty) = (-ang.sin(), ang.cos());
        let mut m = Map::new();
        m.insert("x".into(), json!(x));
        m.insert("y".into(), json!(y));
        if k > 0 {
            m.insert("c1".into(), json!([x - tx * h, y - ty * h]));
        }
        if k < segs {
            m.insert("c2".into(), json!([x + tx * h, y + ty * h]));
        }
        m
    }).collect()
}

/// corners.fillet: round the straight corners of a poly (all, or the node indices in `only`).
pub fn fillet(s: &mut Shape, radius: f64, only: Option<&[usize]>) -> usize {
    let p = pts(s);
    let closed = s.flag("closed");
    let mut out = Vec::new();
    let mut changed = 0;
    for i in 0..p.len() {
        let cor = if only.is_some_and(|o| !o.contains(&i)) { None } else { corner(&p, i, closed) };
        let Some((a, c, b)) = cor else {
            out.push(p[i].clone());
            continue;
        };
        let (u1, u2) = (unit(a[0] - c[0], a[1] - c[1]), unit(b[0] - c[0], b[1] - c[1]));
        let theta = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0).acos();
        if !(1e-3..=PI - 1e-3).contains(&theta) {
            out.push(p[i].clone());
            continue;
        }
        let mut d = radius / (theta / 2.0).tan();
        let lim = 0.5 * (a[0] - c[0]).hypot(a[1] - c[1]).min((b[0] - c[0]).hypot(b[1] - c[1]));
        let mut r = radius;
        if d > lim {
            d = lim;
            r = d * (theta / 2.0).tan();
        }
        let t1 = [c[0] + u1[0] * d, c[1] + u1[1] * d];
        let t2 = [c[0] + u2[0] * d, c[1] + u2[1] * d];
        let bi = unit(u1[0] + u2[0], u1[1] + u2[1]);
        let cd = r / (theta / 2.0).sin();
        let cc = [c[0] + bi[0] * cd, c[1] + bi[1] * cd];
        let mid = [cc[0] - bi[0] * r, cc[1] - bi[1] * r];
        out.extend(arc_nodes(cc, r, t1, t2, mid));
        changed += 1;
    }
    set_pts(s, out);
    s.0.remove("regular");
    changed
}

fn inside(pt: P, ring: &[P]) -> bool {
    let mut c = false;
    for i in 1..ring.len() {
        let (a, b) = (ring[i - 1], ring[i]);
        if (a[1] > pt[1]) != (b[1] > pt[1]) && pt[0] < a[0] + (pt[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

/// corners.relief: dogbone or T-bone reliefs on the material-inner corners of closed polys.
pub fn relief(shapes: &mut [Shape], diameter: f64, tbone: bool, near_90: bool) -> usize {
    let r = diameter / 2.0;
    let rings: Vec<(i64, Vec<P>)> = shapes.iter().filter_map(|s| s.paths().into_iter().next().filter(|p| p.len() > 3).map(|p| (s.id(), p))).collect();
    let depth: Vec<usize> = shapes.iter().map(|s| {
        let first = pts(s).first().map(xy).unwrap_or([0.0, 0.0]);
        rings.iter().filter(|(id, ring)| *id != s.id() && inside(first, ring)).count()
    }).collect();
    let mut total = 0;
    for (k, s) in shapes.iter_mut().enumerate() {
        let p = pts(s);
        if !s.flag("closed") || p.len() < 3 {
            continue;
        }
        let n = p.len();
        let area: f64 = (0..n).map(|i| { let (a, b) = (xy(&p[(i + n - 1) % n]), xy(&p[i])); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() / 2.0;
        let orient = if area > 0.0 { 1.0 } else { -1.0 };
        let hole = depth[k] % 2 == 1;
        let mut out = Vec::new();
        for i in 0..n {
            let Some((a, c, b)) = corner(&p, i, true) else {
                out.push(p[i].clone());
                continue;
            };
            let cross = (c[0] - a[0]) * (b[1] - c[1]) - (c[1] - a[1]) * (b[0] - c[0]);
            let concave = cross * orient < 0.0;
            if concave == hole || cross.abs() < 1e-9 {
                out.push(p[i].clone());
                continue;
            }
            let (u1, u2) = (unit(a[0] - c[0], a[1] - c[1]), unit(b[0] - c[0], b[1] - c[1]));
            let theta = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0).acos();
            if near_90 && (theta.to_degrees() - 90.0).abs() > 15.0 {
                out.push(p[i].clone());
                continue;
            }
            let (la, lb) = ((a[0] - c[0]).hypot(a[1] - c[1]), (b[0] - c[0]).hypot(b[1] - c[1]));
            let u = if lb >= la { u2 } else { u1 };
            let cc = if tbone {
                [c[0] + u[0] * r, c[1] + u[1] * r]
            } else {
                let bi = unit(u1[0] + u2[0], u1[1] + u2[1]);
                [c[0] + bi[0] * r, c[1] + bi[1] * r]
            };
            let t1 = 2.0 * (u1[0] * (cc[0] - c[0]) + u1[1] * (cc[1] - c[1]));
            let t2 = 2.0 * (u2[0] * (cc[0] - c[0]) + u2[1] * (cc[1] - c[1]));
            let e1 = [c[0] + u1[0] * t1.max(0.0), c[1] + u1[1] * t1.max(0.0)];
            let e2 = [c[0] + u2[0] * t2.max(0.0), c[1] + u2[1] * t2.max(0.0)];
            let through = if tbone {
                let o = if u == u2 { u1 } else { u2 };
                let dot = o[0] * u[0] + o[1] * u[1];
                let op = unit(o[0] - dot * u[0], o[1] - dot * u[1]);
                [cc[0] - op[0] * r, cc[1] - op[1] * r]
            } else {
                c
            };
            out.extend(arc_nodes(cc, r, e1, e2, through));
            total += 1;
        }
        set_pts(s, out);
        s.0.remove("regular");
    }
    total
}

/// tools.find_slots: slots (edges of length ≈ width between two parallel opposite walls) in a straight poly.
fn find_slots(p: &[Map<String, Value>], width: f64, tol: f64) -> Vec<usize> {
    let n = p.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (a, b) = (&p[i], &p[(i + 1) % n]);
        if ["c1", "c2"].iter().any(|k| a.contains_key(*k) || b.contains_key(*k)) {
            continue;
        }
        let (pa, pb) = (xy(a), xy(b));
        let l = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
        if (l - width).abs() > tol {
            continue;
        }
        let (pp, nb) = (xy(&p[(i + n - 1) % n]), xy(&p[(i + 2) % n]));
        let u = [(pb[0] - pa[0]) / l, (pb[1] - pa[1]) / l];
        let (da, db) = ([pa[0] - pp[0], pa[1] - pp[1]], [nb[0] - pb[0], nb[1] - pb[1]]);
        let (la, lb) = (da[0].hypot(da[1]), db[0].hypot(db[1]));
        if la < 1e-6 || lb < 1e-6 {
            continue;
        }
        if (da[0] * u[0] + da[1] * u[1]).abs() / la > 0.02 || (db[0] * u[0] + db[1] * u[1]).abs() / lb > 0.02 {
            continue;
        }
        if (da[0] * db[0] + da[1] * db[1]) / (la * lb) > -0.98 {
            continue;
        }
        out.push(i);
    }
    out
}

/// tools.resize_slots: widen / narrow every slot of width `old` to `new`, centred.
pub fn resize_slots(s: &mut Shape, old: f64, new: f64, tol: f64) -> usize {
    let mut p = pts(s);
    let slots = find_slots(&p, old, tol);
    let n = p.len();
    let mut moved: std::collections::BTreeMap<usize, [f64; 2]> = Default::default();
    for &i in &slots {
        let (ia, ib) = (i, (i + 1) % n);
        let (a, b) = (xy(&p[ia]), xy(&p[ib]));
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        let d = (new - l) / 2.0;
        for (idx, sign) in [(ia, -1.0), ((i + n - 1) % n, -1.0), (ib, 1.0), ((i + 2) % n, 1.0)] {
            let m = moved.entry(idx).or_insert([0.0, 0.0]);
            m[0] += sign * u[0] * d;
            m[1] += sign * u[1] * d;
        }
    }
    for (idx, d) in moved {
        let q = xy(&p[idx]);
        p[idx].insert("x".into(), json!(q[0] + d[0]));
        p[idx].insert("y".into(), json!(q[1] + d[1]));
    }
    if !slots.is_empty() {
        set_pts(s, p);
        s.0.remove("regular");
    }
    slots.len()
}

/// tools.apollonius: every circle tangent to three circles (x, y, r).
pub fn apollonius(c: [[f64; 3]; 3]) -> Vec<[f64; 3]> {
    let mut sols: Vec<[f64; 3]> = Vec::new();
    for s1 in [-1.0, 1.0] {
        for s2 in [-1.0, 1.0] {
            for s3 in [-1.0, 1.0] {
                let ([x1, y1, r1], [x2, y2, r2], [x3, y3, r3]) = (c[0], c[1], c[2]);
                let (a2, b2, c2, d2) = (2.0 * (x2 - x1), 2.0 * (y2 - y1), 2.0 * (s2 * r2 - s1 * r1), (x2 * x2 + y2 * y2 - r2 * r2) - (x1 * x1 + y1 * y1 - r1 * r1));
                let (a3, b3, c3, d3) = (2.0 * (x3 - x1), 2.0 * (y3 - y1), 2.0 * (s3 * r3 - s1 * r1), (x3 * x3 + y3 * y3 - r3 * r3) - (x1 * x1 + y1 * y1 - r1 * r1));
                let det = a2 * b3 - a3 * b2;
                if det.abs() < 1e-12 {
                    continue;
                }
                let (xd, xr) = ((d2 * b3 - d3 * b2) / det, -(c2 * b3 - c3 * b2) / det);
                let (yd, yr) = ((a2 * d3 - a3 * d2) / det, -(a2 * c3 - a3 * c2) / det);
                let (ex, ey) = (xd - x1, yd - y1);
                let a = xr * xr + yr * yr - 1.0;
                let b = 2.0 * (ex * xr + ey * yr) - 2.0 * s1 * r1;
                let cc = ex * ex + ey * ey - r1 * r1;
                let rs: Vec<f64> = if a.abs() < 1e-12 {
                    if b.abs() > 1e-12 { vec![-cc / b] } else { continue }
                } else {
                    let disc = b * b - 4.0 * a * cc;
                    if disc < 0.0 {
                        continue;
                    }
                    vec![(-b + disc.sqrt()) / (2.0 * a), (-b - disc.sqrt()) / (2.0 * a)]
                };
                for r in rs {
                    if r > 1e-6 {
                        let cand = [xd + xr * r, yd + yr * r, r];
                        if !sols.iter().any(|o| (cand[0] - o[0]).abs() < 1e-6 && (cand[1] - o[1]).abs() < 1e-6 && (cand[2] - o[2]).abs() < 1e-6) {
                            sols.push(cand);
                        }
                    }
                }
            }
        }
    }
    sols
}

pub fn circle_of(s: &Shape) -> [f64; 3] {
    if s.kind() == "circle" {
        return [s.f("cx"), s.f("cy"), (s.f("rx") + s.f("ry")) / 2.0];
    }
    let b = s.bounds();
    [b[0] + b[2] / 2.0, b[1] + b[3] / 2.0, (b[2] + b[3]) / 4.0]
}

// ---------------------------------------------------------------- box maker
fn finger_edge(length: f64, t: f64, fw: f64, kind: &str) -> Vec<P> {
    if kind == "flat" {
        return vec![[0.0, 0.0], [length, 0.0]];
    }
    let mut n = ((length / fw).floor() as usize).max(3);
    if n % 2 == 0 {
        n -= 1;
    }
    let seg = length / n as f64;
    let mut out = Vec::new();
    for i in 0..n {
        let ind = if kind == "out" { i % 2 == 0 } else { i % 2 == 1 };
        let d = if ind { t } else { 0.0 };
        out.push([i as f64 * seg, d]);
        out.push([(i + 1) as f64 * seg, d]);
    }
    out
}

pub fn box_panel(x0: f64, y0: f64, w: f64, h: f64, edges: [&str; 4], t: f64, fw: f64) -> Vec<Value> {
    let lens = [w, h, w, h];
    let map = |i: usize, a: f64, d: f64| -> P {
        match i {
            0 => [x0 + a, y0 + d],
            1 => [x0 + w - d, y0 + a],
            2 => [x0 + w - a, y0 + h - d],
            _ => [x0 + d, y0 + h - a],
        }
    };
    let lists: Vec<Vec<P>> = (0..4).map(|i| finger_edge(lens[i], t, fw, edges[i])).collect();
    let mut out: Vec<P> = Vec::new();
    for i in 0..4 {
        let lst = &lists[i];
        let (prev_d, next_d) = (lists[(i + 3) % 4].last().unwrap()[1], lists[(i + 1) % 4][0][1]);
        for (k, q) in lst.iter().enumerate() {
            let mut a = q[0];
            if k == 0 {
                a = a.max(prev_d);
            }
            if k == lst.len() - 1 {
                a = a.min(lens[i] - next_d);
            }
            let p = map(i, a, q[1]);
            if out.last().is_none_or(|l| (l[0] - p[0]).abs() > 1e-6 || (l[1] - p[1]).abs() > 1e-6) {
                out.push(p);
            }
        }
    }
    out.iter().map(|p| json!({"x": p[0], "y": p[1]})).collect()
}

pub fn gear_points(cx: f64, cy: f64, n: usize, module: f64) -> Vec<Value> {
    let rp = module * n as f64 / 2.0;
    let (ra, rf) = (rp + module, (rp - 1.25 * module).max(0.5));
    let step = TAU / n as f64;
    let at = |r: f64, a: f64| json!({"x": cx + r * a.cos(), "y": cy + r * a.sin()});
    let mut out = Vec::new();
    for i in 0..n {
        let a = i as f64 * step;
        out.extend([at(rf, a), at(rf, a + step * 0.12), at(ra, a + step * 0.3), at(ra, a + step * 0.45), at(rf, a + step * 0.63)]);
    }
    out
}

// ---------------------------------------------------------------- barcodes
const CODE128: [&str; 107] = ["212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213",
    "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221", "223211", "221132",
    "221231", "213212", "223112", "312131", "311222", "321122", "321221", "312212", "322112", "322211",
    "212123", "212321", "232121", "111323", "131123", "131321", "112313", "132113", "132311", "211313",
    "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121", "313121", "211331",
    "231131", "213113", "213311", "213131", "311123", "311321", "331121", "312113", "312311", "332111",
    "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214",
    "112412", "122114", "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111",
    "111242", "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311", "113141",
    "114131", "311141", "411131", "211412", "211214", "211232", "2331112"];
const EAN_L: [&str; 10] = ["0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011", "0110111", "0001011"];
const EAN_PARITY: [&str; 10] = ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"];

fn ean_r(d: usize) -> String {
    EAN_L[d].chars().map(|c| if c == '0' { '1' } else { '0' }).collect()
}

fn ean_g(d: usize) -> String {
    ean_r(d).chars().rev().collect()
}

pub fn code128(text: &str) -> Result<String, String> {
    if text.is_empty() || text.chars().any(|c| !(32..=126).contains(&(c as u32))) {
        return Err("Code 128 B: solo caratteri ASCII stampabili".into());
    }
    let mut values = vec![104usize];
    values.extend(text.chars().map(|c| c as usize - 32));
    let check = (values[0] + values[1..].iter().enumerate().map(|(i, v)| v * (i + 1)).sum::<usize>()) % 103;
    values.push(check);
    values.push(106);
    let mut out = String::new();
    for v in values {
        let mut bar = true;
        for w in CODE128[v].chars() {
            for _ in 0..w.to_digit(10).unwrap() {
                out.push(if bar { '1' } else { '0' });
            }
            bar = !bar;
        }
    }
    Ok(out)
}

pub fn ean_check(digits: &str) -> char {
    let total: u32 = digits.chars().rev().enumerate().map(|(i, d)| d.to_digit(10).unwrap() * if i % 2 == 0 { 3 } else { 1 }).sum();
    char::from_digit((10 - total % 10) % 10, 10).unwrap()
}

pub fn ean13(data: &str) -> Result<(String, String), String> {
    let mut d: String = data.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() == 12 {
        d.push(ean_check(&d));
    }
    if d.len() != 13 || ean_check(&d[..12]) != d.chars().nth(12).unwrap() {
        return Err("EAN-13: servono 12 cifre (o 13 con cifra di controllo corretta)".into());
    }
    let dig: Vec<usize> = d.chars().map(|c| c.to_digit(10).unwrap() as usize).collect();
    let parity = EAN_PARITY[dig[0]];
    let left: String = parity.chars().zip(&dig[1..7]).map(|(p, &x)| if p == 'L' { EAN_L[x].to_string() } else { ean_g(x) }).collect();
    let right: String = dig[7..].iter().map(|&x| ean_r(x)).collect();
    Ok((format!("101{left}01010{right}101"), d))
}

pub fn ean8(data: &str) -> Result<(String, String), String> {
    let mut d: String = data.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() == 7 {
        d.push(ean_check(&d));
    }
    if d.len() != 8 || ean_check(&d[..7]) != d.chars().nth(7).unwrap() {
        return Err("EAN-8: servono 7 cifre (o 8 con cifra di controllo corretta)".into());
    }
    let dig: Vec<usize> = d.chars().map(|c| c.to_digit(10).unwrap() as usize).collect();
    let l: String = dig[..4].iter().map(|&x| EAN_L[x]).collect();
    let r: String = dig[4..].iter().map(|&x| ean_r(x)).collect();
    Ok((format!("101{l}01010{r}101"), d))
}

/// devices.barcode_modules: (module string, human-readable text).
pub fn barcode_modules(kind: &str, data: &str) -> Result<(String, String), String> {
    match kind {
        "code128" => Ok((code128(data)?, data.to_string())),
        "ean13" => ean13(data),
        "upca" => {
            let digits: String = data.chars().filter(|c| c.is_ascii_digit()).collect();
            let (m, full) = ean13(&format!("0{digits}"))?;
            Ok((m, full[1..].to_string()))
        }
        "ean8" => ean8(data),
        k => Err(k.to_string()),
    }
}

/// Runs of bar modules merged into rectangles (x, y, w, h).
pub fn barcode_rects(modules: &str, x0: f64, y0: f64, mw: f64, h: f64) -> Vec<[f64; 4]> {
    let b: Vec<char> = modules.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i] != '1' {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < b.len() && b[j] == '1' {
            j += 1;
        }
        out.push([x0 + i as f64 * mw, y0, (j - i) as f64 * mw, h]);
        i = j;
    }
    out
}

/// QR code module matrix (true = dark), with the given error correction level (l, m, q, h).
pub fn qr_matrix(text: &str, level: &str) -> Result<Vec<Vec<bool>>, String> {
    use qrcode::{EcLevel, QrCode};
    let ec = match level { "l" => EcLevel::L, "q" => EcLevel::Q, "h" => EcLevel::H, _ => EcLevel::M };
    let code = QrCode::with_error_correction_level(text.as_bytes(), ec).map_err(|e| e.to_string())?;
    let w = code.width();
    let colors = code.to_colors();
    Ok((0..w).map(|r| (0..w).map(|c| colors[r * w + c] == qrcode::Color::Dark).collect()).collect())
}
