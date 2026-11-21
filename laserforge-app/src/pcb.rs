//! PCB files: Gerber RS-274X (copper / silk / mask) and Excellon drill files, converted to filled outlines in
//! mm. Y is flipped with a fixed reference (the work-area height) so several files of the same board stay
//! aligned. Ported from laserforge/pcb.py.

use crate::shapes::P;
use laserforge_core::geometry::boolean;
use laserforge_core::model::Pt;
use regex::Regex;
use std::f64::consts::TAU;

fn num(token: &str, fmt: (usize, usize), zeros: char) -> f64 {
    if token.contains('.') {
        return token.parse().unwrap_or(0.0);
    }
    let neg = token.starts_with('-');
    let mut t = token.trim_start_matches(['+', '-']).to_string();
    if zeros == 'T' {
        while t.len() < fmt.0 + fmt.1 {
            t.push('0');
        }
    }
    let v = t.parse::<f64>().unwrap_or(0.0) / 10f64.powi(fmt.1 as i32);
    if neg { -v } else { v }
}

fn circle(x: f64, y: f64, r: f64, n: usize) -> Vec<P> {
    let mut v: Vec<P> = (0..n).map(|i| [x + r * (i as f64 / n as f64 * TAU).cos(), y + r * (i as f64 / n as f64 * TAU).sin()]).collect();
    v.push(v[0]);
    v
}

/// Capsule around a polyline (shapely LineString.buffer with round caps).
fn capsule(pts: &[P], r: f64) -> Vec<Vec<P>> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        if l < 1e-12 {
            out.push(circle(a[0], a[1], r, 48));
            continue;
        }
        let ang = dy.atan2(dx);
        let mut ring = Vec::new();
        for k in 0..=24 {
            let t = ang - std::f64::consts::FRAC_PI_2 + k as f64 / 24.0 * std::f64::consts::PI;
            ring.push([b[0] + r * t.cos(), b[1] + r * t.sin()]);
        }
        for k in 0..=24 {
            let t = ang + std::f64::consts::FRAC_PI_2 + k as f64 / 24.0 * std::f64::consts::PI;
            ring.push([a[0] + r * t.cos(), a[1] + r * t.sin()]);
        }
        ring.push(ring[0]);
        out.push(ring);
    }
    out
}

fn flash(kind: char, p: &[f64], x: f64, y: f64) -> Vec<Vec<P>> {
    match kind {
        'C' => vec![circle(x, y, p[0] / 2.0, 96)],
        'R' => {
            let (w, h) = (p[0] / 2.0, p.get(1).copied().unwrap_or(p[0]) / 2.0);
            vec![vec![[x - w, y - h], [x + w, y - h], [x + w, y + h], [x - w, y + h], [x - w, y - h]]]
        }
        'O' => {
            let (w, h) = (p[0], p.get(1).copied().unwrap_or(p[0]));
            if w > h {
                capsule(&[[x - (w - h) / 2.0, y], [x + (w - h) / 2.0, y]], h / 2.0)
            } else if h > w {
                capsule(&[[x, y - (h - w) / 2.0], [x, y + (h - w) / 2.0]], w / 2.0)
            } else {
                vec![circle(x, y, w / 2.0, 96)]
            }
        }
        'P' => {
            let (d, n) = (p[0], p.get(1).copied().unwrap_or(6.0) as usize);
            let rot = p.get(2).copied().unwrap_or(0.0).to_radians();
            let mut v: Vec<P> = (0..n).map(|i| [x + d / 2.0 * (rot + i as f64 * TAU / n as f64).cos(), y + d / 2.0 * (rot + i as f64 * TAU / n as f64).sin()]).collect();
            v.push(v[0]);
            vec![v]
        }
        _ => vec![circle(x, y, 0.1, 24)],
    }
}

fn arc_points(x0: f64, y0: f64, x1: f64, y1: f64, cx: f64, cy: f64, cw: bool) -> Vec<P> {
    let a0 = (y0 - cy).atan2(x0 - cx);
    let mut a1 = (y1 - cy).atan2(x1 - cx);
    let r = (x0 - cx).hypot(y0 - cy);
    if cw {
        while a1 >= a0 {
            a1 -= TAU;
        }
    } else {
        while a1 <= a0 {
            a1 += TAU;
        }
    }
    if (x0 - x1).abs() < 1e-9 && (y0 - y1).abs() < 1e-9 {
        a1 = if cw { a0 - TAU } else { a0 + TAU };
    }
    let n = 4.max(((a1 - a0).abs() * r / 0.2) as usize);
    (1..=n).map(|k| [cx + r * (a0 + (a1 - a0) * k as f64 / n as f64).cos(), cy + r * (a0 + (a1 - a0) * k as f64 / n as f64).sin()]).collect()
}

type Items = Vec<(Vec<Vec<P>>, bool)>;

fn gerber_items(text: &str) -> Items {
    let text = text.replace('\r', "");
    let (mut fmt, mut zeros, mut scale) = ((2usize, 4usize), 'L', 1.0);
    let ext_re = Regex::new(r"(?s)%([^%]*)%").unwrap();
    let xy_re = Regex::new(r"X(\d)(\d)").unwrap();
    for m in ext_re.captures_iter(&text) {
        let body = m[1].replace('\n', "");
        if let Some(fs) = body.strip_prefix("FS") {
            zeros = if fs.get(..2).is_some_and(|s| s.contains('T')) { 'T' } else { 'L' };
            if let Some(c) = xy_re.captures(&body) {
                fmt = (c[1].parse().unwrap_or(2), c[2].parse().unwrap_or(4));
            }
        } else if body.starts_with("MOIN") {
            scale = 25.4;
        } else if body.starts_with("MOMM") {
            scale = 1.0;
        }
    }
    let ad_re = Regex::new(r"ADD(\d+)([A-Za-z_][\w.$]*),?([\dX.]*)").unwrap();
    let g_re = Regex::new(r"^G0?(\d+)").unwrap();
    let d_re = Regex::new(r"^D0?(\d+)$").unwrap();
    let c_re = Regex::new(r"([XYIJ])([+-]?[\d.]+)").unwrap();
    let op_re = Regex::new(r"D0?([123])\b").unwrap();
    const EXT: [&str; 18] = ["FS", "MO", "AD", "AM", "LP", "SR", "TF", "TA", "TO", "TD", "IP", "OF", "SF", "LN", "IN", "AS", "IR", "MI"];
    let mut aps: std::collections::HashMap<i64, (char, Vec<f64>)> = Default::default();
    let mut cur: Option<(char, Vec<f64>)> = None;
    let (mut x, mut y) = (0.0, 0.0);
    let mut interp = 1;
    let mut region: Option<Vec<P>> = None;
    let mut dark = true;
    let mut items: Items = Vec::new();
    let splitter = Regex::new(r"[*%\n]+").unwrap();
    for raw in splitter.split(&text) {
        let mut cmd = raw.trim().to_string();
        if cmd.is_empty() {
            continue;
        }
        if EXT.iter().any(|e| cmd.starts_with(e)) || cmd.starts_with(|c: char| c.is_ascii_digit() || c == '$' || c == ',') {
            if let Some(ad) = ad_re.captures(&cmd) {
                let mut params: Vec<f64> = ad[3].split('X').filter(|v| !v.is_empty()).filter_map(|v| v.parse::<f64>().ok()).map(|v| v * scale).collect();
                if params.is_empty() {
                    params.push(0.1);
                }
                if &ad[2] == "P" && params.len() > 1 {
                    params[1] /= scale;
                    if params.len() > 2 {
                        params[2] /= scale;
                    }
                }
                let kind = match &ad[2] { "C" => 'C', "R" => 'R', "O" => 'O', "P" => 'P', _ => 'C' };
                aps.insert(ad[1].parse().unwrap_or(0), (kind, params));
            } else if cmd.starts_with("LPD") {
                dark = true;
            } else if cmd.starts_with("LPC") {
                dark = false;
            }
            continue;
        }
        while !cmd.is_empty() {
            if let Some(g) = g_re.captures(&cmd) {
                let code: i64 = g[1].parse().unwrap_or(0);
                let end = g.get(0).unwrap().end();
                cmd = cmd[end..].to_string();
                match code {
                    1..=3 => interp = code,
                    36 => region = Some(Vec::new()),
                    37 => {
                        if let Some(r) = region.take() {
                            if r.len() > 2 {
                                let mut r = r;
                                r.push(r[0]);
                                items.push((vec![r], dark));
                            }
                        }
                    }
                    4 => cmd.clear(),
                    _ => {}
                }
                continue;
            }
            if !cmd.contains(['X', 'Y', 'I', 'J']) {
                if let Some(d) = d_re.captures(&cmd) {
                    let n: i64 = d[1].parse().unwrap_or(0);
                    if n >= 10 {
                        cur = aps.get(&n).cloned();
                        cmd.clear();
                        continue;
                    }
                }
            }
            if cmd.starts_with("M02") || cmd.starts_with("M00") {
                cmd.clear();
                continue;
            }
            let coords: std::collections::HashMap<String, String> = c_re.captures_iter(&cmd).map(|c| (c[1].to_string(), c[2].to_string())).collect();
            let dcode: i64 = op_re.captures(&cmd).map(|c| c[1].parse().unwrap_or(1)).unwrap_or(1);
            let nx = coords.get("X").map_or(x, |v| num(v, fmt, zeros) * scale);
            let ny = coords.get("Y").map_or(y, |v| num(v, fmt, zeros) * scale);
            let i = coords.get("I").map_or(0.0, |v| num(v, fmt, zeros) * scale);
            let j = coords.get("J").map_or(0.0, |v| num(v, fmt, zeros) * scale);
            match dcode {
                1 => {
                    let seg = if interp == 1 { vec![[nx, ny]] } else { arc_points(x, y, nx, ny, x + i, y + j, interp == 2) };
                    if let Some(r) = region.as_mut() {
                        if r.is_empty() {
                            r.push([x, y]);
                        }
                        r.extend(seg);
                    } else if let Some((k, p)) = &cur {
                        let width = if *k == 'C' || p.len() < 2 { p[0] } else { p[0].min(p[1]) };
                        let mut line = vec![[x, y]];
                        line.extend(seg);
                        items.push((capsule(&line, width / 2.0), dark));
                    }
                }
                2 => {
                    if let Some(r) = region.as_mut() {
                        if r.len() > 2 {
                            let mut rr = std::mem::take(r);
                            rr.push(rr[0]);
                            items.push((vec![rr], dark));
                        }
                    }
                }
                3 => {
                    if let Some((k, p)) = &cur {
                        items.push((flash(*k, p, nx, ny), dark));
                    }
                }
                _ => {}
            }
            x = nx;
            y = ny;
            cmd.clear();
        }
    }
    items
}

fn excellon_items(text: &str) -> Items {
    let (mut scale, mut dec, mut zeros) = (1.0, 3usize, 'L');
    let mut tools: std::collections::HashMap<i64, f64> = Default::default();
    let mut cur: Option<i64> = None;
    let mut holes: Vec<(f64, f64, f64)> = Vec::new();
    let mut header = false;
    let tdef = Regex::new(r"^T(\d+)(?:F\d+)?(?:S\d+)?C([\d.]+)").unwrap();
    let tsel = Regex::new(r"^T(\d+)$").unwrap();
    let c_re = Regex::new(r"([XY])([+-]?[\d.]+)").unwrap();
    for line in text.replace('\r', "").split('\n') {
        let ln = line.trim();
        if ln.is_empty() || ln.starts_with(';') {
            continue;
        }
        if ln == "M48" {
            header = true;
            continue;
        }
        if ln == "%" || ln == "M95" {
            header = false;
            continue;
        }
        if ln.starts_with("INCH") || ln == "M72" {
            scale = 25.4;
            dec = 4;
            zeros = if ln.contains("TZ") { 'T' } else { 'L' };
            continue;
        }
        if ln.starts_with("METRIC") || ln == "M71" {
            scale = 1.0;
            dec = 3;
            zeros = if ln.contains("TZ") { 'T' } else { 'L' };
            continue;
        }
        if let Some(t) = tdef.captures(ln) {
            tools.insert(t[1].parse().unwrap_or(0), t[2].parse::<f64>().unwrap_or(0.8) * scale);
            continue;
        }
        if let Some(t) = tsel.captures(ln) {
            if !header {
                cur = t[1].parse().ok();
                continue;
            }
        }
        let c: std::collections::HashMap<String, String> = c_re.captures_iter(ln).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        if !c.is_empty() && cur.is_some() && !header {
            let last = holes.last().map(|h| (h.0, h.1)).unwrap_or((0.0, 0.0));
            let px = c.get("X").map_or(last.0, |v| num(v, (6 - dec, dec), zeros) * scale);
            let py = c.get("Y").map_or(last.1, |v| num(v, (6 - dec, dec), zeros) * scale);
            holes.push((px, py, tools.get(&cur.unwrap()).copied().unwrap_or(0.8)));
        }
    }
    holes.into_iter().map(|(x, y, d)| (vec![circle(x, y, d / 2.0, 96)], true)).collect()
}

/// Closed rings in canvas coordinates (y' = ref_height − y) of a Gerber or drill file.
pub fn rings(text: &str, drill: bool, ref_height: f64) -> Vec<Vec<P>> {
    let items = if drill { excellon_items(text) } else { gerber_items(text) };
    let to_pt = |rs: &Vec<Vec<P>>| rs.iter().map(|r| r.iter().map(|q| Pt { x: q[0], y: q[1] }).collect::<Vec<Pt>>()).collect::<Vec<_>>();
    let mut acc: Vec<Vec<Pt>> = Vec::new();
    // union in batches of dark items; clear items are subtracted from what came before
    let mut batch: Vec<Vec<Vec<Pt>>> = Vec::new();
    let flush = |acc: &mut Vec<Vec<Pt>>, batch: &mut Vec<Vec<Vec<Pt>>>| {
        if batch.is_empty() {
            return;
        }
        let mut shapes = vec![std::mem::take(acc)];
        shapes.retain(|s| !s.is_empty());
        shapes.extend(batch.drain(..));
        *acc = boolean("union", &shapes);
    };
    for (rs, dark) in &items {
        if *dark {
            // each capsule ring of a trace is its own shape (they overlap)
            for r in to_pt(rs) {
                batch.push(vec![r]);
            }
            if batch.len() >= 200 {
                flush(&mut acc, &mut batch);
            }
        } else {
            flush(&mut acc, &mut batch);
            if !acc.is_empty() {
                acc = boolean("subtract", &[acc.clone(), to_pt(rs)]);
            }
        }
    }
    flush(&mut acc, &mut batch);
    acc.into_iter().map(|r| r.iter().map(|q| [q.x, ref_height - q.y]).collect()).collect()
}
