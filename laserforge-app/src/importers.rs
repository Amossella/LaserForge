//! Importers: SVG, DXF (ASCII), PDF / Illustrator (.ai, PDF-compatible) and bitmap images.
//! Ported from laserforge/importers.py and the import functions of laserforge/mainwindow.py.
//! Vector importers return (points, closed) polylines in mm with Y pointing down, like the canvas;
//! shape builders return shape property maps without `id` / `layer` (the document assigns them).

use crate::shapes::{simplify, P};
use regex::bytes::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::LazyLock;

pub type Props = Map<String, Value>;

fn props(v: Value) -> Props {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn pt(x: f64, y: f64) -> Value {
    json!({"x": x, "y": y})
}

// ====================================================================== SVG

/// model.parse_svg_path: sub-paths of an SVG `d` attribute (curves sampled, arcs as a straight line).
pub fn parse_svg_path(d: &str) -> Vec<(Vec<P>, bool)> {
    static TOK: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[a-zA-Z]|-?\d*\.?\d+(?:[eE][-+]?\d+)?").unwrap());
    let tokens: Vec<&str> = TOK.find_iter(d).map(|m| m.as_str()).collect();
    let mut subs: Vec<(Vec<P>, bool)> = Vec::new();
    let mut cur: Option<usize> = None;
    let (mut x, mut y, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut last_c: Option<P> = None;
    let mut i = 0usize;
    let mut cmd = String::new();
    let is_alpha = |t: &str| t.chars().all(|c| c.is_ascii_alphabetic());
    while i < tokens.len() {
        if is_alpha(tokens[i]) {
            cmd = tokens[i].to_string();
            i += 1;
            if cmd == "z" || cmd == "Z" {
                if let Some(c) = cur {
                    subs[c].1 = true;
                    x = sx;
                    y = sy;
                }
                last_c = None;
                continue;
            }
        }
        if i >= tokens.len() {
            break;
        }
        let rel = cmd.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let up = cmd.to_ascii_uppercase();
        // one command's worth of numbers; None = ran out / bad number (Python breaks out of the loop)
        let step: Option<()> = (|| {
            macro_rules! n {
                () => {{
                    let v = tokens.get(i)?.parse::<f64>().ok()?;
                    i += 1;
                    v
                }};
            }
            match up.as_str() {
                "M" => {
                    x = ox + n!();
                    y = oy + n!();
                    sx = x;
                    sy = y;
                    subs.push((vec![[x, y]], false));
                    cur = Some(subs.len() - 1);
                    cmd = if rel { "l".into() } else { "L".into() };
                    last_c = None;
                }
                _ if cur.is_none() => {
                    subs.push((vec![[x, y]], false));
                    cur = Some(subs.len() - 1);
                }
                "L" => {
                    x = ox + n!();
                    y = oy + n!();
                    subs[cur.unwrap()].0.push([x, y]);
                    last_c = None;
                }
                "H" => {
                    x = (if rel { x } else { 0.0 }) + n!();
                    subs[cur.unwrap()].0.push([x, y]);
                    last_c = None;
                }
                "V" => {
                    y = (if rel { y } else { 0.0 }) + n!();
                    subs[cur.unwrap()].0.push([x, y]);
                    last_c = None;
                }
                "C" | "S" => {
                    let (x1, y1) = if up == "C" {
                        (ox + n!(), oy + n!())
                    } else {
                        last_c.map_or((x, y), |c| (2.0 * x - c[0], 2.0 * y - c[1]))
                    };
                    let (x2, y2, ex, ey) = (ox + n!(), oy + n!(), ox + n!(), oy + n!());
                    for k in 1..13 {
                        let t = k as f64 / 12.0;
                        let u = 1.0 - t;
                        subs[cur.unwrap()].0.push([
                            u.powf(3.0) * x + 3.0 * u * u * t * x1 + 3.0 * u * t * t * x2 + t.powf(3.0) * ex,
                            u.powf(3.0) * y + 3.0 * u * u * t * y1 + 3.0 * u * t * t * y2 + t.powf(3.0) * ey,
                        ]);
                    }
                    last_c = Some([x2, y2]);
                    x = ex;
                    y = ey;
                }
                "Q" | "T" => {
                    let (x1, y1) = if up == "Q" {
                        (ox + n!(), oy + n!())
                    } else {
                        last_c.map_or((x, y), |c| (2.0 * x - c[0], 2.0 * y - c[1]))
                    };
                    let (ex, ey) = (ox + n!(), oy + n!());
                    for k in 1..11 {
                        let t = k as f64 / 10.0;
                        let u = 1.0 - t;
                        subs[cur.unwrap()].0.push([u * u * x + 2.0 * u * t * x1 + t * t * ex, u * u * y + 2.0 * u * t * y1 + t * t * ey]);
                    }
                    last_c = Some([x1, y1]);
                    x = ex;
                    y = ey;
                }
                "A" => {
                    for _ in 0..5 {
                        let _ = n!();
                    }
                    x = ox + n!();
                    y = oy + n!();
                    subs[cur.unwrap()].0.push([x, y]);
                    last_c = None;
                }
                _ => i += 1,
            }
            Some(())
        })();
        if step.is_none() {
            break;
        }
    }
    subs
}

/// mainwindow.import_svg: shapes (without id / layer) from SVG text.
pub fn svg_shapes(text: &str) -> Result<Vec<Props>, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut elems: Vec<(String, HashMap<String, String>)> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let mut attrs = HashMap::new();
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
                    #[allow(deprecated)]
                    let v = a.unescape_value().map(|v| v.to_string()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).to_string());
                    attrs.insert(k, v);
                }
                elems.push((name, attrs));
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err("SVG non valido".into()),
            _ => {}
        }
    }
    let Some((_, root)) = elems.first() else { return Err("SVG non valido".into()) };
    let mut unit = 25.4 / 96.0;
    let wattr = root.get("width").cloned().unwrap_or_default();
    let vb: Vec<f64> = root.get("viewBox").map(|v| v.replace(',', " ").split_whitespace().filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_default();
    if vb.len() == 4 && vb[2] != 0.0 {
        for (suf, mul) in [("mm", 1.0), ("cm", 10.0), ("in", 25.4)] {
            if let Some(num) = wattr.strip_suffix(suf) {
                if let Ok(v) = num.trim().parse::<f64>() {
                    unit = v * mul / vb[2];
                }
            }
        }
    }
    let mut shapes = Vec::new();
    for (tag, a) in &elems {
        let num = |k: &str| a.get(k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
        let sc = |x: f64, y: f64| pt(x * unit, y * unit);
        match tag.as_str() {
            "line" => shapes.push(props(json!({"type": "poly", "points": [sc(num("x1"), num("y1")), sc(num("x2"), num("y2"))], "closed": false}))),
            "rect" => shapes.push(props(json!({"type": "rect", "x": num("x") * unit, "y": num("y") * unit, "w": num("width") * unit,
                                               "h": num("height") * unit}))),
            "circle" => {
                let r = num("r") * unit;
                shapes.push(props(json!({"type": "circle", "cx": num("cx") * unit, "cy": num("cy") * unit, "rx": r, "ry": r})));
            }
            "ellipse" => shapes.push(props(json!({"type": "circle", "cx": num("cx") * unit, "cy": num("cy") * unit,
                                                  "rx": num("rx") * unit, "ry": num("ry") * unit}))),
            "polyline" | "polygon" => {
                let n: Vec<f64> = a.get("points").map(|v| v.replace(',', " ").split_whitespace().filter_map(|t| t.parse().ok()).collect())
                    .unwrap_or_default();
                let pts: Vec<Value> = n.chunks_exact(2).map(|c| sc(c[0], c[1])).collect();
                if pts.len() > 1 {
                    shapes.push(props(json!({"type": "poly", "points": pts, "closed": tag == "polygon"})));
                }
            }
            "path" => {
                for (p, closed) in parse_svg_path(a.get("d").map(|s| s.as_str()).unwrap_or("")) {
                    if p.len() > 1 {
                        shapes.push(props(json!({"type": "poly", "points": p.iter().map(|q| sc(q[0], q[1])).collect::<Vec<_>>(),
                                                 "closed": closed})));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(shapes)
}

// ====================================================================== DXF

const DXF_UNITS: [(i64, f64); 8] = [(1, 25.4), (2, 304.8), (4, 1.0), (5, 10.0), (6, 1000.0), (8, 0.0000254), (9, 0.0254), (10, 914.4)];

fn dxf_pairs(text: &str) -> Vec<(i64, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if let Ok(c) = lines[i].trim().parse::<i64>() {
            out.push((c, lines[i + 1].trim().to_string()));
        }
        i += 2;
    }
    out
}

fn bulge_arc(p0: P, p1: P, bulge: f64, step: f64) -> Vec<P> {
    if bulge.abs() < 1e-9 {
        return vec![p1];
    }
    let theta = 4.0 * bulge.atan();
    let chord = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
    if chord < 1e-12 {
        return vec![p1];
    }
    let r = chord / (2.0 * (theta / 2.0).sin());
    let (mx, my) = ((p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0);
    let d = r * (theta / 2.0).cos();
    let (nx, ny) = (-(p1[1] - p0[1]) / chord, (p1[0] - p0[0]) / chord);
    let (cx, cy) = (mx + nx * d, my + ny * d);
    let a0 = (p0[1] - cy).atan2(p0[0] - cx);
    let n = 2i64.max((theta.abs() / std::f64::consts::TAU * 48.0) as i64).max(((theta * r).abs() / step) as i64);
    (1..=n).map(|k| {
        let a = a0 + theta * k as f64 / n as f64;
        [cx + r.abs() * a.cos(), cy + r.abs() * a.sin()]
    }).collect()
}

fn arc(cx: f64, cy: f64, r: f64, a0: f64, mut a1: f64, step: f64) -> Vec<P> {
    if a1 <= a0 {
        a1 += 360.0;
    }
    let span = (a1 - a0) * (std::f64::consts::PI / 180.0);
    let n = 4i64.max((span / std::f64::consts::TAU * 64.0) as i64).max((span * r / step) as i64);
    let ar = a0 * (std::f64::consts::PI / 180.0);
    (0..=n).map(|k| [cx + r * (ar + span * k as f64 / n as f64).cos(), cy + r * (ar + span * k as f64 / n as f64).sin()]).collect()
}

fn bspline(ctrl: &[P], degree: i64, knots: &[f64]) -> Vec<P> {
    let n = ctrl.len();
    if n < 2 {
        return ctrl.to_vec();
    }
    let degree = degree.clamp(1, n as i64 - 1) as usize;
    let knots: Vec<f64> = if knots.len() == n + degree + 1 {
        knots.to_vec()
    } else {
        let mut k = vec![0.0; degree + 1];
        k.extend((1..(n - degree)).map(|v| v as f64));
        k.extend(std::iter::repeat((n - degree) as f64).take(degree + 1));
        k
    };
    let (lo, hi) = (knots[degree], knots[n]);
    let samples = 20.max(n * 12);
    let deboor = |t: f64| -> P {
        let mut k = degree;
        while k < n - 1 && t >= knots[k + 1] {
            k += 1;
        }
        let mut d: Vec<P> = (0..=degree).map(|j| ctrl[j + k - degree]).collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = j + k - degree;
                let den = knots[i + degree - r + 1] - knots[i];
                let a = if den != 0.0 { (t - knots[i]) / den } else { 0.0 };
                d[j] = [(1.0 - a) * d[j - 1][0] + a * d[j][0], (1.0 - a) * d[j - 1][1] + a * d[j][1]];
            }
        }
        d[degree]
    };
    (0..=samples).map(|s| deboor(lo + (hi - lo) * s as f64 / samples as f64)).collect()
}

type Entity = (String, Vec<(i64, String)>);

fn parse_entities(pairs: &[(i64, String)]) -> Vec<Entity> {
    let mut ents = Vec::new();
    let mut cur: Option<Entity> = None;
    for (code, val) in pairs {
        if *code == 0 {
            if let Some(c) = cur.take() {
                ents.push(c);
            }
            cur = Some((val.clone(), Vec::new()));
        } else if let Some(c) = cur.as_mut() {
            c.1.push((*code, val.clone()));
        }
    }
    ents.extend(cur);
    ents
}

fn getf(data: &[(i64, String)], c: i64, d: f64) -> f64 {
    data.iter().find(|(k, _)| *k == c).map(|(_, v)| v.parse::<f64>().unwrap_or(d)).unwrap_or(d)
}

fn entity_paths(etype: &str, data: &[(i64, String)], step: f64) -> Vec<(Vec<P>, bool)> {
    let get = |c: i64| getf(data, c, 0.0);
    let mut out = Vec::new();
    match etype {
        "LINE" => out.push((vec![[get(10), get(20)], [get(11), get(21)]], false)),
        "CIRCLE" => out.push((arc(get(10), get(20), get(40), 0.0, 360.0, step), true)),
        "ARC" => out.push((arc(get(10), get(20), get(40), get(50), get(51), step), false)),
        "ELLIPSE" => {
            let (cx, cy, mx, my, ratio) = (get(10), get(20), get(11), get(21), getf(data, 40, 1.0));
            let (t0, mut t1) = (getf(data, 41, 0.0), getf(data, 42, std::f64::consts::TAU));
            if t1 <= t0 {
                t1 += std::f64::consts::TAU;
            }
            let a = mx.hypot(my);
            let rot = my.atan2(mx);
            let n = 48i64.max(((t1 - t0) * a / step) as i64);
            let pts = (0..=n).map(|k| {
                let t = t0 + (t1 - t0) * k as f64 / n as f64;
                let (x, y) = (a * t.cos(), a * ratio * t.sin());
                [cx + x * rot.cos() - y * rot.sin(), cy + x * rot.sin() + y * rot.cos()]
            }).collect();
            out.push((pts, (t1 - t0 - std::f64::consts::TAU).abs() < 1e-6));
        }
        "LWPOLYLINE" => {
            let closed = (getf(data, 70, 0.0) as i64) & 1 == 1;
            let (mut verts, mut bulges): (Vec<P>, Vec<f64>) = (Vec::new(), Vec::new());
            let mut x: Option<f64> = None;
            for (k, v) in data {
                match *k {
                    10 => x = v.parse().ok(),
                    20 if x.is_some() => {
                        if let Ok(y) = v.parse() {
                            verts.push([x.unwrap(), y]);
                            bulges.push(0.0);
                        }
                    }
                    42 if !bulges.is_empty() => *bulges.last_mut().unwrap() = v.parse().unwrap_or(0.0),
                    _ => {}
                }
            }
            if verts.len() >= 2 {
                let n = verts.len();
                let mut pts = vec![verts[0]];
                for i in 0..(n - 1 + closed as usize) {
                    pts.extend(bulge_arc(verts[i], verts[(i + 1) % n], bulges[i], step));
                }
                out.push((pts, closed));
            }
        }
        "SPLINE" => {
            let degree = getf(data, 71, 3.0) as i64;
            let closed = (getf(data, 70, 0.0) as i64) & 1 == 1;
            let (mut ctrl, mut fit, mut knots) = (Vec::new(), Vec::new(), Vec::new());
            let (mut cx, mut fx): (Option<f64>, Option<f64>) = (None, None);
            for (k, v) in data {
                match *k {
                    40 => knots.extend(v.parse::<f64>().ok()),
                    10 => cx = v.parse().ok(),
                    20 if cx.is_some() => ctrl.extend(v.parse::<f64>().ok().map(|y| [cx.unwrap(), y])),
                    11 => fx = v.parse().ok(),
                    21 if fx.is_some() => fit.extend(v.parse::<f64>().ok().map(|y| [fx.unwrap(), y])),
                    _ => {}
                }
            }
            if !ctrl.is_empty() {
                out.push((bspline(&ctrl, degree, &knots), closed));
            } else if fit.len() >= 2 {
                out.push((fit, closed));
            }
        }
        _ => {}
    }
    out
}

/// importers.dxf_to_paths
pub fn dxf_to_paths(text: &str) -> Vec<(Vec<P>, bool)> {
    let pairs = dxf_pairs(text);
    let mut scale = 1.0;
    for (i, (code, val)) in pairs.iter().enumerate() {
        if *code == 9 && val == "$INSUNITS" && i + 1 < pairs.len() {
            let u = pairs[i + 1].1.parse::<i64>().unwrap_or(0);
            scale = DXF_UNITS.iter().find(|(k, _)| *k == u).map_or(1.0, |x| x.1);
            break;
        }
    }
    let step = 0.3 / scale;
    let mut sections: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let (mut cur, mut name): (Option<Vec<(i64, String)>>, Option<String>) = (None, None);
    for (code, val) in &pairs {
        if *code == 0 && val == "SECTION" {
            cur = Some(Vec::new());
            name = None;
        } else if *code == 2 && cur.is_some() && name.is_none() {
            name = Some(val.clone());
        } else if *code == 0 && val == "ENDSEC" {
            if let (Some(n), Some(c)) = (name.take(), cur.take()) {
                sections.insert(n, c);
            }
            cur = None;
            name = None;
        } else if let (Some(c), Some(_)) = (cur.as_mut(), name.as_ref()) {
            c.push((*code, val.clone()));
        }
    }
    let mut blocks: HashMap<String, (P, Vec<Entity>)> = HashMap::new();
    let (mut bname, mut bbase, mut blist): (Option<String>, P, Option<Vec<Entity>>) = (None, [0.0, 0.0], None);
    for (etype, data) in parse_entities(sections.get("BLOCKS").map(|v| v.as_slice()).unwrap_or(&[])) {
        if etype == "BLOCK" {
            bname = data.iter().find(|(k, _)| *k == 2).map(|(_, v)| v.clone());
            bbase = [getf(&data, 10, 0.0), getf(&data, 20, 0.0)];
            blist = Some(Vec::new());
        } else if etype == "ENDBLK" {
            if let (Some(n), Some(l)) = (bname.take(), blist.take()) {
                blocks.insert(n, (bbase, l));
            }
            bname = None;
            blist = None;
        } else if let Some(l) = blist.as_mut() {
            l.push((etype, data));
        }
    }
    let mut paths: Vec<(Vec<P>, bool)> = Vec::new();
    fn emit(ents: &[Entity], tf: &dyn Fn(P) -> P, depth: usize, blocks: &HashMap<String, (P, Vec<Entity>)>, step: f64,
            paths: &mut Vec<(Vec<P>, bool)>) {
        struct Poly {
            closed: bool,
            verts: Vec<P>,
            bulges: Vec<f64>,
        }
        let mut polyline: Option<Poly> = None;
        for (etype, data) in ents {
            match etype.as_str() {
                "POLYLINE" => {
                    let closed = data.iter().find(|(k, _)| *k == 70).map(|(_, v)| v.parse::<i64>().unwrap_or(0)).unwrap_or(0) & 1 == 1;
                    polyline = Some(Poly { closed, verts: Vec::new(), bulges: Vec::new() });
                    continue;
                }
                "VERTEX" if polyline.is_some() => {
                    let p = polyline.as_mut().unwrap();
                    p.verts.push([getf(data, 10, 0.0), getf(data, 20, 0.0)]);
                    p.bulges.push(getf(data, 42, 0.0));
                    continue;
                }
                "SEQEND" if polyline.is_some() => {
                    let p = polyline.take().unwrap();
                    if p.verts.len() >= 2 {
                        let mut pts = vec![p.verts[0]];
                        for i in 0..(p.verts.len() - 1 + p.closed as usize) {
                            pts.extend(bulge_arc(p.verts[i], p.verts[(i + 1) % p.verts.len()], p.bulges[i], step));
                        }
                        paths.push((pts.into_iter().map(tf).collect(), p.closed));
                    }
                    continue;
                }
                "INSERT" if depth < 8 => {
                    let name = data.iter().find(|(k, _)| *k == 2).map(|(_, v)| v.clone());
                    if let Some((base, bl)) = name.and_then(|n| blocks.get(&n)) {
                        let (ix, iy) = (getf(data, 10, 0.0), getf(data, 20, 0.0));
                        let sx = getf(data, 41, 1.0);
                        let sy = getf(data, 42, sx);
                        let rot = getf(data, 50, 0.0) * (std::f64::consts::PI / 180.0);
                        let (c, s) = (rot.cos(), rot.sin());
                        let b = *base;
                        let sub = move |p: P| -> P {
                            let (x, y) = ((p[0] - b[0]) * sx, (p[1] - b[1]) * sy);
                            tf([ix + x * c - y * s, iy + x * s + y * c])
                        };
                        emit(bl, &sub, depth + 1, blocks, step, paths);
                    }
                    continue;
                }
                "INSERT" => continue,
                _ => {}
            }
            for (pts, closed) in entity_paths(etype, data, step) {
                paths.push((pts.into_iter().map(tf).collect(), closed));
            }
        }
    }
    let ents = parse_entities(sections.get("ENTITIES").map(|v| v.as_slice()).unwrap_or(&[]));
    emit(&ents, &|p| p, 0, &blocks, step, &mut paths);
    if paths.is_empty() {
        return Vec::new();
    }
    let maxy = paths.iter().flat_map(|(p, _)| p.iter().map(|q| q[1])).fold(f64::MIN, f64::max);
    let minx = paths.iter().flat_map(|(p, _)| p.iter().map(|q| q[0])).fold(f64::MAX, f64::min);
    paths.into_iter().map(|(p, c)| (p.into_iter().map(|q| [(q[0] - minx) * scale, (maxy - q[1]) * scale]).collect(), c)).collect()
}

// ====================================================================== PDF / AI

static OBJ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s+(\d+)\s+obj\b").unwrap());
static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s+(\d+)\s+R").unwrap());

struct Pdf {
    /// num -> (dictionary bytes, decoded stream)
    objs: HashMap<i64, (Vec<u8>, Option<Vec<u8>>)>,
    /// num -> undecoded stream bytes (JPEG images)
    raw: HashMap<i64, Vec<u8>>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    find(hay, needle, 0).is_some()
}

fn inflate(raw: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    let _ = flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out);
    out
}

fn decode_stream(head: &[u8], raw: &[u8]) -> Vec<u8> {
    let h = String::from_utf8_lossy(head);
    if h.contains("/FlateDecode") || h.contains("/Fl ") || h.trim_end().ends_with("/Fl") {
        return inflate(raw);
    }
    if h.contains("/Filter") {
        return Vec::new();
    }
    raw.to_vec()
}

fn int_after(re: &str, body: &[u8]) -> Option<i64> {
    let r = Regex::new(re).ok()?;
    std::str::from_utf8(r.captures(body)?.get(1)?.as_bytes()).ok()?.parse().ok()
}

fn pdf_objects(data: &[u8]) -> Pdf {
    let mut objs = HashMap::new();
    let mut raws = HashMap::new();
    for c in OBJ.captures_iter(data) {
        let num: i64 = std::str::from_utf8(&c[1]).unwrap().parse().unwrap_or(-1);
        let start = c.get(0).unwrap().end();
        let Some(end) = find(data, b"endobj", start) else { continue };
        let mut body = data[start..end].to_vec();
        let mut stream = None;
        if let Some(si) = find(&body, b"stream", 0) {
            let mut raw = &body[si + 6..];
            raw = if raw.starts_with(b"\r\n") { &raw[2..] } else if raw.first().is_some_and(|b| *b == b'\n' || *b == b'\r') { &raw[1..] } else { raw };
            let raw = match rfind(raw, b"endstream") { Some(ei) => &raw[..ei], None => raw };
            let head = body[..si].to_vec();
            raws.insert(num, raw.to_vec());
            stream = Some(decode_stream(&head, raw));
            body = head;
        }
        objs.insert(num, (body, stream));
    }
    let keys: Vec<i64> = objs.keys().copied().collect();
    for num in keys {
        let (head, stream) = objs[&num].clone();
        let Some(stream) = stream else { continue };
        if !contains(&head, b"/ObjStm") {
            continue;
        }
        let (Some(n), Some(first)) = (int_after(r"/N\s+(\d+)", &head), int_after(r"/First\s+(\d+)", &head)) else { continue };
        let first = first as usize;
        let nums: Vec<i64> = String::from_utf8_lossy(&stream[..first.min(stream.len())]).split_whitespace().filter_map(|t| t.parse().ok()).collect();
        let pairs: Vec<(i64, usize)> = (0..(n as usize).min(nums.len() / 2)).map(|i| (nums[2 * i], nums[2 * i + 1] as usize)).collect();
        for (i, (onum, off)) in pairs.iter().enumerate() {
            let end = if i + 1 < pairs.len() { pairs[i + 1].1 } else { stream.len().saturating_sub(first) };
            if !objs.contains_key(onum) {
                let (a, b) = ((first + off).min(stream.len()), (first + end).min(stream.len()));
                objs.insert(*onum, (stream[a..b.max(a)].to_vec(), None));
            }
        }
    }
    Pdf { objs, raw: raws }
}

fn refs(body: &[u8], key: &str) -> Vec<i64> {
    let re = Regex::new(&format!(r"/{key}\s*(\[[^\]]*\]|\d+\s+\d+\s+R)")).unwrap();
    let Some(m) = re.captures(body) else { return Vec::new() };
    REF.captures_iter(&m[1]).filter_map(|r| std::str::from_utf8(&r[1]).ok()?.parse().ok()).collect()
}

fn pdf_image(num: i64, pdf: &Pdf) -> Option<image::DynamicImage> {
    let (head, stream) = pdf.objs.get(&num)?;
    let raw = pdf.raw.get(&num).map(|v| v.as_slice()).unwrap_or(&[]);
    if contains(head, b"/DCTDecode") {
        return image::load_from_memory(raw).ok().map(|i| image::DynamicImage::ImageRgb8(i.to_rgb8()));
    }
    if contains(head, b"/JPXDecode") {
        return None; // JPEG 2000 is not supported by the image crate
    }
    let w = int_after(r"/Width\s+(\d+)", head)? as u32;
    let h = int_after(r"/Height\s+(\d+)", head)? as u32;
    let bpc = int_after(r"/BitsPerComponent\s+(\d+)", head).unwrap_or(8);
    let stream = stream.as_ref()?;
    if stream.is_empty() || bpc != 8 {
        return None;
    }
    let n = (w * h) as usize;
    if contains(head, b"/DeviceRGB") && stream.len() >= n * 3 {
        return image::RgbImage::from_raw(w, h, stream[..n * 3].to_vec()).map(image::DynamicImage::ImageRgb8);
    }
    if contains(head, b"/DeviceCMYK") && stream.len() >= n * 4 {
        // PIL's CMYK -> RGB: r = 255 - min(255, c + k) per channel
        let rgb: Vec<u8> = stream[..n * 4].chunks(4).flat_map(|p| {
            let k = p[3] as u16;
            [255 - (p[0] as u16 + k).min(255) as u8, 255 - (p[1] as u16 + k).min(255) as u8, 255 - (p[2] as u16 + k).min(255) as u8]
        }).collect();
        return image::RgbImage::from_raw(w, h, rgb).map(image::DynamicImage::ImageRgb8);
    }
    if stream.len() >= n {
        let g = image::GrayImage::from_raw(w, h, stream[..n].to_vec())?;
        return Some(image::DynamicImage::ImageRgb8(image::DynamicImage::ImageLuma8(g).to_rgb8()));
    }
    None
}

type Ctm = [f64; 6];

fn mul(m: &[f64], c: &Ctm) -> Ctm {
    let (a, b, cc, d, e, f) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let [aa, bb, ccc, dd, ee, ff] = *c;
    [a * aa + b * ccc, a * bb + b * dd, cc * aa + d * ccc, cc * bb + d * dd, e * aa + f * ccc + ee, e * bb + f * dd + ff]
}

enum Item {
    Num(f64),
    Str(String),
}

fn xobjects(body: &[u8], pdf: &Pdf) -> HashMap<Vec<u8>, i64> {
    static XO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/XObject\s*<<(.*?)>>").unwrap());
    static RES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Resources\s+(\d+)\s+\d+\s+R").unwrap());
    static XR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/XObject\s+(\d+)\s+\d+\s+R").unwrap());
    static KV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/([^\s/<>]+)\s+(\d+)\s+\d+\s+R").unwrap());
    let num = |c: &[u8]| std::str::from_utf8(c).ok().and_then(|s| s.parse::<i64>().ok());
    let Some(m) = XO.captures(body) else {
        if let Some(r) = RES.captures(body).and_then(|r| num(&r[1])) {
            if let Some((b, _)) = pdf.objs.get(&r) {
                return xobjects(&b.clone(), pdf);
            }
        }
        if let Some(x) = XR.captures(body).and_then(|r| num(&r[1])) {
            if let Some((b, _)) = pdf.objs.get(&x) {
                let mut wrapped = b"/XObject <<".to_vec();
                wrapped.extend_from_slice(b);
                wrapped.extend_from_slice(b">>");
                return xobjects(&wrapped, pdf);
            }
        }
        return HashMap::new();
    };
    KV.captures_iter(&m[1]).filter_map(|c| Some((c[1].to_vec(), num(&c[2])?))).collect()
}

fn content_paths(content: &[u8], mut ctm: Ctm, pdf: &Pdf, resources: &HashMap<Vec<u8>, i64>, depth: usize,
                 images: &mut Vec<(image::DynamicImage, Ctm)>) -> Vec<(Vec<P>, bool)> {
    static TOK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"/[^\s/\[\]()<>]+|\((?:\\.|[^\\)])*\)|<[0-9A-Fa-f\s]*>|\[|\]|[^\s\[\]()<>/]+").unwrap()
    });
    static MATRIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Matrix\s*\[([^\]]*)\]").unwrap());
    let mut stack: Vec<Item> = Vec::new();
    let mut out: Vec<(Vec<P>, bool)> = Vec::new();
    let (mut sub, mut path): (Vec<P>, Vec<(Vec<P>, bool)>) = (Vec::new(), Vec::new());
    let mut gstack: Vec<Ctm> = Vec::new();
    let (mut cur, mut start): (P, P) = ([0.0, 0.0], [0.0, 0.0]);
    let t = |p: P, m: &Ctm| [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]];
    for tok in TOK.find_iter(content) {
        let tok = tok.as_bytes();
        let s = String::from_utf8_lossy(tok).to_string();
        if let Some(v) = py_float(&s) {
            stack.push(Item::Num(v));
            continue;
        }
        if s == "[" || s == "]" || s.starts_with('(') || s.starts_with('<') || s.starts_with('/') {
            stack.push(Item::Str(s));
            continue;
        }
        let nums: Vec<f64> = stack.iter().filter_map(|i| if let Item::Num(v) = i { Some(*v) } else { None }).collect();
        let l = nums.len();
        let flush = |sub: &mut Vec<P>, path: &mut Vec<(Vec<P>, bool)>, closed: bool| {
            if sub.len() >= 2 {
                path.push((std::mem::take(sub), closed));
            }
            sub.clear();
        };
        let cubic = |sub: &mut Vec<P>, cur: &mut P, p1: P, p2: P, p3: P, ctm: &Ctm| {
            let p0 = *cur;
            for k in 1..13 {
                let tt = k as f64 / 12.0;
                let u = 1.0 - tt;
                let x = u.powf(3.0) * p0[0] + 3.0 * u * u * tt * p1[0] + 3.0 * u * tt * tt * p2[0] + tt.powf(3.0) * p3[0];
                let y = u.powf(3.0) * p0[1] + 3.0 * u * u * tt * p1[1] + 3.0 * u * tt * tt * p2[1] + tt.powf(3.0) * p3[1];
                sub.push(t([x, y], ctm));
            }
            *cur = p3;
        };
        match s.as_str() {
            "q" => gstack.push(ctm),
            "Q" => ctm = gstack.pop().unwrap_or(ctm),
            "cm" if l >= 6 => ctm = mul(&nums[l - 6..], &ctm),
            "m" if l >= 2 => {
                flush(&mut sub, &mut path, false);
                cur = [nums[l - 2], nums[l - 1]];
                start = cur;
                sub = vec![t(cur, &ctm)];
            }
            "l" if l >= 2 => {
                cur = [nums[l - 2], nums[l - 1]];
                if sub.is_empty() {
                    sub = vec![t(start, &ctm)];
                }
                sub.push(t(cur, &ctm));
            }
            "c" if l >= 6 => cubic(&mut sub, &mut cur, [nums[l - 6], nums[l - 5]], [nums[l - 4], nums[l - 3]], [nums[l - 2], nums[l - 1]], &ctm),
            "v" if l >= 4 => {
                let c0 = cur;
                cubic(&mut sub, &mut cur, c0, [nums[l - 4], nums[l - 3]], [nums[l - 2], nums[l - 1]], &ctm)
            }
            "y" if l >= 4 => cubic(&mut sub, &mut cur, [nums[l - 4], nums[l - 3]], [nums[l - 2], nums[l - 1]], [nums[l - 2], nums[l - 1]], &ctm),
            "h" => {
                if !sub.is_empty() {
                    sub.push(t(start, &ctm));
                }
                flush(&mut sub, &mut path, true);
                cur = start;
            }
            "re" if l >= 4 => {
                flush(&mut sub, &mut path, false);
                let (x, y, w, h) = (nums[l - 4], nums[l - 3], nums[l - 2], nums[l - 1]);
                path.push((vec![t([x, y], &ctm), t([x + w, y], &ctm), t([x + w, y + h], &ctm), t([x, y + h], &ctm), t([x, y], &ctm)], true));
                cur = [x, y];
                start = cur;
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                flush(&mut sub, &mut path, matches!(s.as_str(), "s" | "b" | "b*"));
                out.append(&mut path);
            }
            "n" => {
                sub.clear();
                path.clear();
            }
            "Do" if depth < 6 => {
                let name = stack.iter().rev().find_map(|i| if let Item::Str(v) = i { v.starts_with('/').then(|| v.clone()) } else { None });
                if let Some(r) = name.and_then(|n| resources.get(n[1..].as_bytes()).copied()) {
                    if let Some((head, stream)) = pdf.objs.get(&r) {
                        if contains(head, b"/Image") {
                            if let Some(im) = pdf_image(r, pdf) {
                                images.push((im, ctm));
                            }
                        } else if stream.is_some() && contains(head, b"/Form") {
                            let fm: Vec<f64> = MATRIX.captures(head).map(|m| String::from_utf8_lossy(&m[1]).split_whitespace()
                                .filter_map(|v| v.parse().ok()).collect()).unwrap_or_else(|| vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                            if fm.len() >= 6 {
                                let fctm = mul(&fm, &ctm);
                                let mut res = resources.clone();
                                res.extend(xobjects(head, pdf));
                                out.extend(content_paths(stream.as_ref().unwrap(), fctm, pdf, &res, depth + 1, images));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        stack.clear();
    }
    out
}

/// Python float(): plain decimal / exponent numbers (and inf / nan), with an optional sign.
fn py_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_digit() || matches!(b[0], b'+' | b'-' | b'.' | b'i' | b'I' | b'n' | b'N')) {
        return None;
    }
    s.parse::<f64>().ok()
}

fn page_list(pdf: &Pdf) -> Vec<i64> {
    static PAGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Type\s*/Pages\b").unwrap());
    static PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/Type\s*/Page\b").unwrap());
    let mut nums: Vec<i64> = pdf.objs.keys().copied().collect();
    nums.sort();
    let root = nums.iter().copied().find(|n| { let b = &pdf.objs[n].0; PAGES.is_match(b) && !contains(b, b"/Parent") });
    let mut out = Vec::new();
    fn walk(n: i64, depth: usize, pdf: &Pdf, out: &mut Vec<i64>) {
        let Some((b, _)) = pdf.objs.get(&n) else { return };
        if depth > 32 {
            return;
        }
        if PAGES.is_match(b) {
            for k in refs(b, "Kids") {
                walk(k, depth + 1, pdf, out);
            }
        } else if PAGE.is_match(b) {
            out.push(n);
        }
    }
    if let Some(r) = root {
        walk(r, 0, pdf, &mut out);
    }
    if out.is_empty() {
        out = nums.into_iter().filter(|n| { let b = &pdf.objs[n].0; PAGE.is_match(b) && !PAGES.is_match(b) }).collect();
    }
    out
}

fn is_pdf(data: &[u8]) -> bool {
    contains(&data[..data.len().min(1024)], b"%PDF")
}

pub fn pdf_page_count(data: &[u8]) -> usize {
    if !is_pdf(data) {
        return 0;
    }
    page_list(&pdf_objects(data)).len()
}

/// Distance from point p to the polyline q (GEOS discrete Hausdorff measures vertices against segments).
fn point_line_dist(p: P, q: &[P]) -> f64 {
    if q.len() == 1 {
        return (p[0] - q[0][0]).hypot(p[1] - q[0][1]);
    }
    q.windows(2).map(|s| crate::shapes::dist_to_segment(p, s[0], s[1])).fold(f64::INFINITY, f64::min)
}

fn hausdorff(a: &[P], b: &[P]) -> f64 {
    let ab = a.iter().map(|p| point_line_dist(*p, b)).fold(0.0, f64::max);
    let ba = b.iter().map(|p| point_line_dist(*p, a)).fold(0.0, f64::max);
    ab.max(ba)
}

/// A placed PDF image: picture and its box in mm (Y down).
pub struct PlacedImage {
    pub img: image::DynamicImage,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// importers.pdf_to_paths: vector paths of one page in mm (Y down), plus embedded images.
pub fn pdf_to_paths(data: &[u8], page_index: usize, images: Option<&mut Vec<PlacedImage>>) -> Result<Vec<(Vec<P>, bool)>, String> {
    if !data.starts_with(b"%PDF") && !is_pdf(data) {
        return Err("non è un PDF (i vecchi file .ai PostScript non sono supportati)".into());
    }
    let pdf = pdf_objects(data);
    let pages = page_list(&pdf);
    if pages.is_empty() {
        return Ok(Vec::new());
    }
    let body = pdf.objs[&pages[page_index.min(pages.len() - 1)]].0.clone();
    static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(?:CropBox|MediaBox)\s*\[([^\]]*)\]").unwrap());
    let bx: Vec<f64> = BOX.captures(&body).map(|m| String::from_utf8_lossy(&m[1]).split_whitespace().filter_map(|v| v.parse().ok()).collect())
        .filter(|v: &Vec<f64>| v.len() >= 4).unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
    let mut content = Vec::new();
    for r in refs(&body, "Contents") {
        match pdf.objs.get(&r) {
            Some((_, Some(st))) => {
                content.extend_from_slice(st);
                content.push(b'\n');
            }
            Some((b, None)) => {
                for r2 in REF.captures_iter(b) {
                    if let Some(n) = std::str::from_utf8(&r2[1]).ok().and_then(|s| s.parse::<i64>().ok()) {
                        if let Some((_, Some(st))) = pdf.objs.get(&n) {
                            content.extend_from_slice(st);
                            content.push(b'\n');
                        }
                    }
                }
            }
            None => {}
        }
    }
    let mut placed = Vec::new();
    let paths = content_paths(&content, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], &pdf, &xobjects(&body, &pdf), 0, &mut placed);
    let k = 25.4 / 72.0;
    let top = bx[3];
    if let Some(images) = images {
        for (mut im, m) in placed {
            let [a, b, c, d, e, f] = m;
            let xs = [e, e + a, e + c, e + a + c];
            let ys = [f, f + b, f + d, f + b + d];
            if a < 0.0 {
                im = im.fliph();
            }
            if d < 0.0 {
                im = im.flipv();
            }
            let (x0, x1) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
            let (y0, y1) = (ys.iter().copied().fold(f64::MAX, f64::min), ys.iter().copied().fold(f64::MIN, f64::max));
            images.push(PlacedImage { img: im, x: (x0 - bx[0]) * k, y: (top - y1) * k, w: (x1 - x0) * k, h: (y1 - y0) * k });
        }
    }
    let mut out = Vec::new();
    let mut kept: Vec<([f64; 4], Vec<P>)> = Vec::new();
    for (pts, closed) in paths {
        if pts.len() < 2 {
            continue;
        }
        let mm: Vec<P> = pts.iter().map(|p| [(p[0] - bx[0]) * k, (top - p[1]) * k]).collect();
        let bb = [mm.iter().map(|p| p[0]).fold(f64::MAX, f64::min), mm.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
                  mm.iter().map(|p| p[0]).fold(f64::MIN, f64::max), mm.iter().map(|p| p[1]).fold(f64::MIN, f64::max)];
        // the same outline painted twice (fill + stroke), or a hairline saved as two rings a hair apart: keep one
        if kept.iter().any(|(kb, kl)| bb.iter().zip(kb).all(|(a, b)| (a - b).abs() < 0.15) && hausdorff(&mm, kl) < 0.15) {
            continue;
        }
        kept.push((bb, mm.clone()));
        out.push((mm, closed));
    }
    Ok(out)
}

/// mainwindow.import_vector_file: polylines -> poly shapes (closing point dropped, simplified to 0.005 mm).
pub fn vector_shapes(paths: Vec<(Vec<P>, bool)>) -> Vec<Props> {
    let mut shapes = Vec::new();
    for (mut pts, closed) in paths {
        if closed && pts.len() > 2 && (pts[0][0] - pts[pts.len() - 1][0]).hypot(pts[0][1] - pts[pts.len() - 1][1]) < 1e-6 {
            pts.pop();
        }
        if pts.len() >= 2 {
            let pts: Vec<Value> = simplify(&pts, 0.005).iter().map(|p| pt(p[0], p[1])).collect();
            shapes.push(props(json!({"type": "poly", "points": pts, "closed": closed})));
        }
    }
    shapes
}

// ====================================================================== images

/// model.image_from_bytes + encode_image: any bitmap -> PNG data URL (transparency kept when present).
pub fn encode_image(img: &image::DynamicImage) -> String {
    use base64::Engine;
    let rgba = img.to_rgba8();
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 255);
    let mut buf = std::io::Cursor::new(Vec::new());
    let ok = if has_alpha {
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut buf, image::ImageFormat::Png)
    } else {
        image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut buf, image::ImageFormat::Png)
    };
    if ok.is_err() {
        return String::new();
    }
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// mainwindow.add_image_bytes: an image shape centred on the work area, 254 px per inch, at most 60 % of the area.
pub fn image_shape(data: &[u8], name: &str, ws: (f64, f64), path: Option<&str>) -> Option<Props> {
    let img = image::load_from_memory(data).ok()?;
    let (w_area, h_area) = ws;
    let (mut w, mut h) = (img.width() as f64 * 25.4 / 254.0, img.height() as f64 * 25.4 / 254.0);
    let k = 1f64.min(w_area * 0.6 / w).min(h_area * 0.6 / h);
    w *= k;
    h *= k;
    let mut s = props(json!({"type": "image", "src": encode_image(&img), "name": name, "x": (w_area - w) / 2.0, "y": (h_area - h) / 2.0,
                             "w": w, "h": h, "adj": crate::shapes::default_adj()}));
    if let Some(p) = path {
        s.insert("path".into(), json!(p));
    }
    Some(s)
}

/// Shapes for the images placed in a PDF (tiny ones are dropped).
pub fn pdf_image_shapes(images: &[PlacedImage], name: &str) -> Vec<Props> {
    images.iter().filter(|im| im.w > 0.1 && im.h > 0.1).map(|im| props(json!({
        "type": "image", "src": encode_image(&im.img), "name": name, "x": im.x, "y": im.y, "w": im.w, "h": im.h,
        "adj": crate::shapes::default_adj()}))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_path_commands() {
        let subs = parse_svg_path("M10 10 h20 v10 H10 z m5 5 l1 1");
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].0, vec![[10.0, 10.0], [30.0, 10.0], [30.0, 20.0], [10.0, 20.0]]);
        assert!(subs[0].1);
        assert_eq!(subs[1].0, vec![[15.0, 15.0], [16.0, 16.0]]);
    }

    #[test]
    fn dxf_line_and_units() {
        let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n5\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n3\n21\n4\n0\nENDSEC\n0\nEOF\n";
        let p = dxf_to_paths(dxf);
        assert_eq!(p, vec![(vec![[0.0, 40.0], [30.0, 0.0]], false)]);
    }
}
