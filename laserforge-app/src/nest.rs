//! True-shape nesting: parts placed by their real outline (grown by half the spacing) on a raster of the sheet,
//! largest first, at the top-most then left-most free spot, trying each rotation. Same method as
//! laserforge/nest.py; the collision search scans bit rows instead of an FFT correlation.

use crate::edit::{rotate_shape, Msg};
use crate::project::Doc;
use crate::shapes::{is_closed_path, Shape, P};
use crate::l;
use laserforge_core::model::Pt;
use serde_json::json;

/// Filled outline rings of a part grown by `grow` (outer rings only: holes are not used for nesting).
fn outline(s: &Shape, grow: f64) -> Vec<Vec<P>> {
    let paths = s.paths();
    let closed: Vec<Vec<Pt>> = paths.iter().filter(|p| p.len() > 3 && is_closed_path(p)).map(|p| p.iter().map(|q| Pt { x: q[0], y: q[1] }).collect()).collect();
    let open: Vec<Vec<Pt>> = paths.iter().filter(|p| p.len() >= 2 && !(p.len() > 3 && is_closed_path(p))).map(|p| p.iter().map(|q| Pt { x: q[0], y: q[1] }).collect()).collect();
    let mut shapes = Vec::new();
    if !closed.is_empty() {
        shapes.push(closed);
    }
    for o in open {
        shapes.push(vec![o]);
    }
    let rings = laserforge_core::geometry::geometry_op("offset", &shapes, &json!({"distance": grow.max(0.05), "direction": "out", "join": "mitre"}))
        .unwrap_or_default().into_iter().flatten().collect::<Vec<_>>();
    let mut out: Vec<Vec<P>> = rings.into_iter().map(|r| r.iter().map(|q| [q.x, q.y]).collect()).collect();
    if out.is_empty() {
        let b = s.bounds();
        out.push(vec![[b[0], b[1]], [b[0] + b[2].max(0.1), b[1]], [b[0] + b[2].max(0.1), b[1] + b[3].max(0.1)], [b[0], b[1] + b[3].max(0.1)], [b[0], b[1]]]);
    }
    out
}

struct Grid {
    w: usize,
    h: usize,
    words: usize,
    bits: Vec<u64>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Grid {
        let words = w.div_ceil(64) + 1;
        Grid { w, h, words, bits: vec![0; words * h] }
    }
    fn set(&mut self, x: usize, y: usize) {
        self.bits[y * self.words + x / 64] |= 1 << (x % 64);
    }
    fn get(&self, x: usize, y: usize) -> bool {
        self.bits[y * self.words + x / 64] >> (x % 64) & 1 == 1
    }
    /// Row y of the grid shifted so that bit `x0` comes first, `n` words.
    fn row_from(&self, y: usize, x0: usize, k: usize) -> u64 {
        let w = x0 / 64 + k;
        let s = x0 % 64;
        let lo = self.bits.get(y * self.words + w).copied().unwrap_or(0) >> s;
        let hi = if s == 0 { 0 } else { self.bits.get(y * self.words + w + 1).copied().unwrap_or(0) << (64 - s) };
        if w + 1 >= self.words && s != 0 { lo } else { lo | hi }
    }
}

fn raster(rings: &[Vec<P>], res: f64, ox: f64, oy: f64, w: usize, h: usize) -> Grid {
    let mut g = Grid::new(w, h);
    let rel: Vec<Vec<P>> = rings.iter().map(|r| r.iter().map(|q| [(q[0] - ox) / res / w as f64, (q[1] - oy) / res / h as f64]).collect()).collect();
    for ring in rel {
        let m = crate::project::crop_mask(&[ring], w, h);
        for y in 0..h {
            for x in 0..w {
                if m[y * w + x] > 0 {
                    g.set(x, y);
                }
            }
        }
    }
    g
}

/// Nest the selection (or all unlocked shapes) inside the work area minus `margin`.
pub fn nest_true_shape(doc: &mut Doc, ws: (f64, f64), spacing: f64, rotations: &[f64], margin: f64) -> Msg {
    let items: Vec<Shape> = if doc.selected.is_empty() { doc.shapes.clone() } else { doc.selected_shapes() }.into_iter().filter(|s| !s.locked()).collect();
    if items.is_empty() {
        return Ok(None);
    }
    let (sx, sy, sw, sh) = (margin, margin, ws.0 - 2.0 * margin, ws.1 - 2.0 * margin);
    let res = 0.25f64.max(sw.max(sh) / 700.0);
    let (gw, gh) = ((sw / res).ceil() as usize, (sh / res).ceil() as usize);
    let mut occ = Grid::new(gw, gh);
    let mut parts: Vec<(Shape, f64)> = items.iter().map(|s| {
        let o = outline(s, spacing / 2.0);
        let area: f64 = o.iter().map(|r| (1..r.len()).map(|i| r[i - 1][0] * r[i][1] - r[i][0] * r[i - 1][1]).sum::<f64>().abs() / 2.0).sum();
        (s.clone(), area)
    }).collect();
    parts.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut placed: Vec<(i64, f64, f64, f64)> = Vec::new();
    let mut unplaced = 0;
    for (s, _) in &parts {
        let mut best: Option<(usize, usize, f64, f64, f64, Grid)> = None;
        for &ang in rotations {
            let mut r = s.clone();
            rotate_shape(&mut r, 0.0, 0.0, ang);
            let rings = outline(&r, spacing / 2.0);
            let pts: Vec<P> = rings.iter().flatten().copied().collect();
            let b = crate::shapes::path_box(&pts);
            let (pw, ph) = ((b[2] / res).ceil() as usize + 1, (b[3] / res).ceil() as usize + 1);
            if pw > gw || ph > gh {
                continue;
            }
            let mask = raster(&rings, res, b[0], b[1], pw, ph);
            let mwords = pw.div_ceil(64);
            let mut found = None;
            'scan: for py in 0..=(gh - ph) {
                if let Some((by, _, ..)) = &best {
                    if py > *by {
                        break;
                    }
                }
                for px in 0..=(gw - pw) {
                    let mut hit = false;
                    for my in 0..ph {
                        for k in 0..mwords {
                            if mask.bits[my * mask.words + k] & occ.row_from(py + my, px, k) != 0 {
                                hit = true;
                                break;
                            }
                        }
                        if hit {
                            break;
                        }
                    }
                    if !hit {
                        found = Some((py, px));
                        break 'scan;
                    }
                }
            }
            if let Some((py, px)) = found {
                if best.as_ref().is_none_or(|bb| (py, px) < (bb.0, bb.1)) {
                    best = Some((py, px, ang, b[0], b[1], mask));
                }
            }
        }
        let Some((py, px, ang, minx, miny, mask)) = best else {
            unplaced += 1;
            continue;
        };
        for y in 0..mask.h {
            for x in 0..mask.w {
                if mask.get(x, y) && py + y < gh && px + x < gw {
                    occ.set(px + x, py + y);
                }
            }
        }
        placed.push((s.id(), ang, sx + px as f64 * res - minx, sy + py as f64 * res - miny));
    }
    doc.save_undo(&l("Disponi", "Nest"));
    let mut next = doc.next_id;
    for (id, ang, dx, dy) in &placed {
        if let Some(s) = doc.find_mut(*id) {
            if *ang != 0.0 {
                rotate_shape(s, 0.0, 0.0, *ang);
            }
            s.translate(*dx, *dy);
        }
        let _ = &mut next;
    }
    Ok(Some(if unplaced > 0 {
        l(&format!("{unplaced} pezzi non entrano nell'area di lavoro"), &format!("{unplaced} parts don't fit in the work area"))
    } else {
        l(&format!("{} pezzi disposti", placed.len()), &format!("{} parts nested", placed.len()))
    }))
}
