//! Polygon work: kerf offset, offset fill, flood-fill areas, boolean operations and clipping to the work area.
//! Python LaserForge does this with shapely; here the `geo` crate is used, so vertex order and start points
//! can differ while the shapes are the same (tests compare areas and burned paths within a tolerance).

use crate::model::Pt;
use geo::{BooleanOps, Buffer, Contains, Coord, LineString, MultiPolygon, Point, Polygon};
use geo::algorithm::buffer::{BufferStyle, LineJoin};

pub fn closed(path: &[Pt], tol: f64) -> bool {
    path.len() > 2 && (path[0].x - path[path.len() - 1].x).abs() < tol && (path[0].y - path[path.len() - 1].y).abs() < tol
}

pub fn polygon(path: &[Pt]) -> Polygon<f64> {
    Polygon::new(LineString::from(path.iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>()), vec![])
}

fn ring_pts(ls: &LineString<f64>) -> Vec<Pt> {
    ls.coords().map(|c| Pt { x: c.x, y: c.y }).collect()
}

/// All rings (exterior then holes) of every polygon.
pub fn rings(mp: &MultiPolygon<f64>) -> Vec<Vec<Pt>> {
    let mut out = Vec::new();
    for p in mp.iter() {
        if p.exterior().0.len() < 3 {
            continue;
        }
        out.push(ring_pts(p.exterior()));
        for h in p.interiors() {
            out.push(ring_pts(h));
        }
    }
    out
}

/// Even-odd region of several closed paths (each one toggles the area inside it), like LaserForge's
/// shapely symmetric_difference chain. Self-intersecting paths are cleaned by the boolean engine.
pub fn even_odd(paths: &[Vec<Pt>]) -> MultiPolygon<f64> {
    let mut acc: Option<MultiPolygon<f64>> = None;
    for p in paths {
        let poly = MultiPolygon::new(vec![polygon(p)]);
        let clean = poly.union(&MultiPolygon::new(vec![]));
        acc = Some(match acc {
            None => clean,
            Some(a) => a.xor(&clean),
        });
    }
    acc.unwrap_or_else(|| MultiPolygon::new(vec![]))
}

fn miter() -> LineJoin<f64> {
    // shapely mitre_limit=4: corners sharper than 2·asin(1/4) are bevelled
    LineJoin::Miter(2.0 * (0.25f64).asin())
}

/// Kerf compensation: outlines grow by `distance`, holes shrink; open paths are unchanged.
pub fn offset_paths(paths: &[Vec<Pt>], distance: f64) -> Vec<Vec<Pt>> {
    let closed_p: Vec<&Vec<Pt>> = paths.iter().filter(|p| closed(p, 1e-3)).collect();
    let open_p: Vec<Vec<Pt>> = paths.iter().filter(|p| !closed(p, 1e-3)).cloned().collect();
    let polys: Vec<Polygon<f64>> = closed_p.iter().map(|p| polygon(p)).collect();
    let mut out = Vec::new();
    for (i, poly) in polys.iter().enumerate() {
        let first = poly.exterior().0[0];
        let depth = polys.iter().enumerate().filter(|(j, q)| *j != i && q.contains(&Point::from(first))).count();
        let d = if depth % 2 == 0 { distance } else { -distance };
        let g = MultiPolygon::new(vec![poly.clone()]).union(&MultiPolygon::new(vec![]));
        let buf = g.buffer_with_style(BufferStyle::new(d).line_join(miter()));
        for part in buf.iter() {
            if part.exterior().0.len() >= 3 {
                out.push(ring_pts(part.exterior()));
            }
        }
    }
    out.extend(open_p);
    out
}

/// Offset fill: the region, then the region shrunk by `step` again and again (round joins), ring by ring.
pub fn offset_fill_rings(paths: &[Vec<Pt>], step: f64) -> Vec<Vec<Pt>> {
    let closed_p: Vec<Vec<Pt>> = paths.iter().filter(|p| closed(p, 1e-3)).cloned().collect();
    let mut out = Vec::new();
    if closed_p.is_empty() {
        return out;
    }
    let mut current = even_odd(&closed_p);
    while !current.0.is_empty() && out.len() < 5000 {
        let before = out.len();
        out.extend(rings(&current));
        if out.len() == before {
            break;
        }
        current = current.buffer_with_style(BufferStyle::new(-step).line_join(geo::algorithm::buffer::LineJoin::Round(std::f64::consts::PI / 32.0)));
    }
    out
}

/// Separate areas of the even-odd region (for flood fill), each as its rings. Empty when there is only one.
pub fn flood_areas(paths: &[Vec<Pt>]) -> Vec<(Vec<Vec<Pt>>, [f64; 4])> {
    let region = even_odd(paths);
    let parts: Vec<&Polygon<f64>> = region.iter().filter(|p| p.exterior().0.len() >= 3).collect();
    if parts.len() < 2 {
        return Vec::new();
    }
    parts
        .into_iter()
        .map(|p| {
            let mut r = vec![ring_pts(p.exterior())];
            r.extend(p.interiors().iter().map(ring_pts));
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for c in p.exterior().coords() {
                x0 = x0.min(c.x);
                y0 = y0.min(c.y);
                x1 = x1.max(c.x);
                y1 = y1.max(c.y);
            }
            (r, [x0, y0, x1, y1])
        })
        .collect()
}

/// Distance from a point to an area (0 inside), used to visit flood-fill areas nearest first.
pub fn distance_to(rings_: &[Vec<Pt>], x: f64, y: f64) -> f64 {
    let poly = Polygon::new(
        LineString::from(rings_[0].iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>()),
        rings_[1..].iter().map(|r| LineString::from(r.iter().map(|p| Coord { x: p.x, y: p.y }).collect::<Vec<_>>())).collect(),
    );
    if poly.contains(&Point::new(x, y)) {
        return 0.0;
    }
    let mut best = f64::MAX;
    for r in rings_ {
        for w in r.windows(2) {
            best = best.min(seg_dist(x, y, w[0], w[1]));
        }
    }
    best
}

fn seg_dist(px: f64, py: f64, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((px - a.x) * dx + (py - a.y) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (px - (a.x + t * dx)).hypot(py - (a.y + t * dy))
}

/// Boolean operations on shapes (each shape = its paths): union, subtract (A − rest), subtract_rev
/// (last − rest), intersect, xor. Returns the rings of the result.
pub fn boolean(op: &str, shapes: &[Vec<Vec<Pt>>]) -> Vec<Vec<Pt>> {
    let regions: Vec<MultiPolygon<f64>> = shapes
        .iter()
        .map(|s| even_odd(&s.iter().filter(|p| closed(p, 1e-6) && p.len() > 3).cloned().collect::<Vec<_>>()))
        .collect();
    if regions.is_empty() {
        return Vec::new();
    }
    let union_all = |rs: &[MultiPolygon<f64>]| rs.iter().fold(MultiPolygon::new(vec![]), |a, b| a.union(b));
    let res = match op {
        "union" | "weld" => union_all(&regions),
        "subtract" => regions[0].difference(&union_all(&regions[1..])),
        "subtract_rev" => regions[regions.len() - 1].difference(&union_all(&regions[..regions.len() - 1])),
        "intersect" => regions[1..].iter().fold(regions[0].clone(), |a, b| a.intersection(b)),
        "xor" => regions[1..].iter().fold(regions[0].clone(), |a, b| a.xor(b)),
        _ => MultiPolygon::new(vec![]),
    };
    rings(&res)
}

/// Area of an even-odd region (for tests and checks).
pub fn area(paths: &[Vec<Pt>]) -> f64 {
    use geo::Area;
    even_odd(paths).unsigned_area()
}

/// Cut paths at the edge of the work area [0,W]×[0,H]. Closed paths in fill modes are clipped as areas,
/// everything else as lines (Liang–Barsky per segment).
pub fn clip_paths(paths: &[Vec<Pt>], w: f64, h: f64, fill: bool) -> Vec<Vec<Pt>> {
    let mut out = Vec::new();
    for path in paths {
        if path.iter().all(|p| p.x >= 0.0 && p.x <= w && p.y >= 0.0 && p.y <= h) {
            out.push(path.clone());
            continue;
        }
        if fill && closed(path, 1e-6) && path.len() >= 4 {
            let bx = polygon(&[Pt { x: 0.0, y: 0.0 }, Pt { x: w, y: 0.0 }, Pt { x: w, y: h }, Pt { x: 0.0, y: h }, Pt { x: 0.0, y: 0.0 }]);
            let r = MultiPolygon::new(vec![polygon(path)]).union(&MultiPolygon::new(vec![])).intersection(&MultiPolygon::new(vec![bx]));
            out.extend(rings(&r));
            continue;
        }
        let mut cur: Vec<Pt> = Vec::new();
        for s in path.windows(2) {
            match clip_segment(s[0], s[1], w, h) {
                Some((a, b)) => {
                    if cur.last().map_or(true, |l| (l.x - a.x).abs() > 1e-9 || (l.y - a.y).abs() > 1e-9) {
                        if cur.len() > 1 {
                            out.push(std::mem::take(&mut cur));
                        }
                        cur = vec![a];
                    }
                    cur.push(b);
                }
                None => {
                    if cur.len() > 1 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
        }
        if cur.len() > 1 {
            out.push(cur);
        }
    }
    out
}

fn clip_segment(a: Pt, b: Pt, w: f64, h: f64) -> Option<(Pt, Pt)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.x), (dx, w - a.x), (-dy, a.y), (dy, h - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    let at = |t: f64| Pt { x: a.x + dx * t, y: a.y + dy * t };
    if t1 - t0 < 1e-12 {
        return None;
    }
    Some((at(t0), at(t1)))
}

fn join_of(name: &str) -> LineJoin<f64> {
    match name {
        "mitre" => miter(),
        "bevel" => LineJoin::Bevel,
        _ => LineJoin::Round(std::f64::consts::PI / 32.0),
    }
}

/// engine.geometry: offset (out / in / both, round · mitre · bevel joins), weld / union, subtract,
/// subtract_rev, intersect, xor, round (corner rounding) and hull. Each shape is its list of paths;
/// returns one list of rings per result.
pub fn geometry_op(op: &str, shapes: &[Vec<Vec<Pt>>], params: &serde_json::Value) -> Result<Vec<Vec<Vec<Pt>>>, String> {
    use geo::ConvexHull;
    let regions: Vec<MultiPolygon<f64>> = shapes.iter()
        .map(|s| even_odd(&s.iter().filter(|p| p.len() > 3 && closed(p, 1e-6)).cloned().collect::<Vec<_>>())).collect();
    let lines: Vec<Vec<LineString<f64>>> = shapes.iter().map(|s| s.iter().filter(|p| p.len() >= 2 && !(p.len() > 3 && closed(p, 1e-6)))
        .map(|p| LineString::from(p.iter().map(|q| Coord { x: q.x, y: q.y }).collect::<Vec<_>>())).collect()).collect();
    let join = params.get("join").and_then(|v| v.as_str()).unwrap_or("round");
    let num = |k: &str, d: f64| params.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
    let mut results: Vec<Vec<Vec<Pt>>> = Vec::new();
    let has = |m: &MultiPolygon<f64>| !m.0.is_empty();
    match op {
        "offset" => {
            let dist = num("distance", 2.0);
            let dir = params.get("direction").and_then(|v| v.as_str()).unwrap_or("out");
            for (g, ls) in regions.iter().zip(&lines) {
                if has(g) {
                    if dir == "out" || dir == "both" {
                        results.push(rings(&g.buffer_with_style(BufferStyle::new(dist).line_join(join_of(join)))));
                    }
                    if dir == "in" || dir == "both" {
                        results.push(rings(&g.buffer_with_style(BufferStyle::new(-dist).line_join(join_of(join)))));
                    }
                }
                for l in ls {
                    if dir == "out" || dir == "both" {
                        results.push(rings(&l.buffer_with_style(BufferStyle::new(dist).line_join(join_of(join)))));
                    }
                }
            }
        }
        "weld" | "union" | "subtract" | "subtract_rev" | "intersect" | "xor" => {
            results.push(boolean(op, shapes));
        }
        "round" => {
            let r = num("radius", 2.0);
            let st = |d: f64| BufferStyle::new(d).line_join(LineJoin::Round(std::f64::consts::PI / 32.0));
            for g in regions.iter().filter(|g| has(g)) {
                results.push(rings(&g.buffer_with_style(st(r)).buffer_with_style(st(-2.0 * r)).buffer_with_style(st(r))));
            }
        }
        "hull" => {
            let mut pts: Vec<Coord<f64>> = Vec::new();
            for g in &regions {
                for p in g.iter() {
                    pts.extend(p.exterior().coords().copied());
                }
            }
            for ls in &lines {
                for l in ls {
                    pts.extend(l.coords().copied());
                }
            }
            if pts.len() >= 3 {
                let hull = geo::MultiPoint::from(pts.into_iter().map(Point::from).collect::<Vec<_>>()).convex_hull();
                results.push(rings(&MultiPolygon::new(vec![hull])));
            }
        }
        _ => return Err(format!("operazione sconosciuta {op}")),
    }
    Ok(results.into_iter().filter(|r| !r.is_empty()).collect())
}
