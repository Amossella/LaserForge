//! G-code generation for GRBL-family laser controllers, ported from LaserForge's Python engine
//! (laserforge/gcode.py). Output is meant to be identical line for line.

use crate::model::*;
use crate::util::{f0, f1, f3, py_float, py_round};

pub const JOB_HEADER: &str = "G00 G17 G40 G21 G54";

pub fn is_closed(path: &[Pt]) -> bool {
    let tol = 1e-3;
    path.len() > 2 && (path[0].x - path[path.len() - 1].x).abs() < tol && (path[0].y - path[path.len() - 1].y).abs() < tol
}

/// Even-odd scanline fill: rows = [(y, [(x0, x1)...])] in scan space, plus the rotation back.
pub struct FillRows {
    pub rows: Vec<(f64, Vec<(f64, f64)>)>,
    ca: f64,
    sa: f64,
}

impl FillRows {
    pub fn unrotate(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.ca - y * self.sa, x * self.sa + y * self.ca)
    }
}

pub fn fill_rows(paths: &[Vec<Pt>], interval: f64, angle_deg: f64) -> FillRows {
    let a = angle_deg.to_radians();
    let (ca, sa) = (a.cos(), a.sin());
    // (ymin, ymax, x1, y1, x2, y2)
    let mut edges: Vec<[f64; 6]> = Vec::new();
    for path in paths {
        if path.len() < 3 {
            continue;
        }
        let mut pts: Vec<(f64, f64)> = path.iter().map(|p| (p.x * ca + p.y * sa, -p.x * sa + p.y * ca)).collect();
        if pts[0] != pts[pts.len() - 1] {
            pts.push(pts[0]);
        }
        for w in pts.windows(2) {
            let ((x1, y1), (x2, y2)) = (w[0], w[1]);
            if y1 != y2 {
                edges.push([y1.min(y2), y1.max(y2), x1, y1, x2, y2]);
            }
        }
    }
    let mut out = FillRows { rows: Vec::new(), ca, sa };
    if edges.is_empty() || interval <= 0.0 {
        return out;
    }
    edges.sort_by(|a, b| {
        for i in 0..6 {
            match a[i].partial_cmp(&b[i]).unwrap_or(std::cmp::Ordering::Equal) {
                std::cmp::Ordering::Equal => continue,
                o => return o,
            }
        }
        std::cmp::Ordering::Equal
    });
    let ymin = edges[0][0];
    let ymax = edges.iter().map(|e| e[1]).fold(f64::NEG_INFINITY, f64::max);
    let mut active: Vec<[f64; 6]> = Vec::new();
    let mut idx = 0;
    let mut y = ymin + interval / 2.0;
    while y < ymax {
        while idx < edges.len() && edges[idx][0] <= y {
            active.push(edges[idx]);
            idx += 1;
        }
        active.retain(|e| e[1] > y);
        let mut xs: Vec<f64> = active.iter().map(|e| e[2] + (y - e[3]) * (e[4] - e[2]) / (e[5] - e[3])).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut segs = Vec::new();
        let mut i = 0;
        while i + 1 < xs.len() {
            if xs[i + 1] - xs[i] > 1e-6 {
                segs.push((xs[i], xs[i + 1]));
            }
            i += 2;
        }
        if !segs.is_empty() {
            out.rows.push((y, segs));
        }
        y += interval;
    }
    out
}

pub fn air_on_command(m: &MachineSettings) -> &'static str {
    if m.air_command.to_uppercase() == "M7" {
        "M7"
    } else {
        "M8"
    }
}

/// Raster block in relative mode (G91 between G90s): after the first rapid every X/Y is a delta. The position
/// follows the rounded deltas actually written, so rounding never accumulates.
pub fn relativize(lines: Vec<String>) -> Vec<String> {
    // Raster lines are single-space separated words ("G1 X1.000 Y2.000 F1000 S300"): tokens are rewritten
    // directly instead of with a regex, which matters for images with hundreds of thousands of lines.
    let mut out = Vec::with_capacity(lines.len() + 2);
    let mut pos = [0.0f64; 2];
    let mut started = false;
    let num = |t: &str| -> Option<f64> { if t.len() > 1 { t[1..].parse::<f64>().ok() } else { None } };
    for ln in lines {
        if !started && ln.starts_with("G0 ") && ln.contains('X') && ln.contains('Y') {
            for t in ln.split(' ') {
                match (t.as_bytes().first(), num(t)) {
                    (Some(b'X'), Some(v)) => pos[0] = v,
                    (Some(b'Y'), Some(v)) => pos[1] = v,
                    _ => {}
                }
            }
            out.push(ln);
            out.push("G91".to_string());
            started = true;
            continue;
        }
        let is_motion = started && (ln.starts_with("G0") || ln.starts_with("G1"))
            && ln.as_bytes().get(2).map_or(true, |c| !c.is_ascii_alphanumeric() && *c != b'_');
        if is_motion {
            use std::fmt::Write as _;
            let mut line = String::with_capacity(ln.len() + 4);
            for (i, t) in ln.split(' ').enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                let ax = match t.as_bytes().first() { Some(b'X') => Some(0), Some(b'Y') => Some(1), _ => None };
                match (ax, num(t)) {
                    (Some(a), Some(v)) => {
                        let dv = py_round(v - pos[a], 3);
                        pos[a] = py_round(pos[a] + dv, 6);
                        let _ = write!(line, "{}{:.3}", &t[..1], dv);
                    }
                    _ => line.push_str(t),
                }
            }
            out.push(line);
            continue;
        }
        out.push(ln);
    }
    if started {
        out.push("G90".to_string());
    }
    out
}

pub struct Generator<'a> {
    pub machine: &'a MachineSettings,
    pub lines: Vec<String>,
    current_air: Option<AirAssist>,
    z_now: f64,
}

impl<'a> Generator<'a> {
    pub fn new(machine: &'a MachineSettings) -> Self {
        Generator { machine, lines: Vec::new(), current_air: None, z_now: 0.0 }
    }

    // ---- primitives ----
    pub fn s(&self, percent: f64) -> i64 {
        (percent.max(0.0) / 100.0 * self.machine.max_power as f64).round_ties_even() as i64
    }

    pub fn add<S: Into<String>>(&mut self, line: S) {
        self.lines.push(line.into());
    }

    fn y(&self, y: f64) -> f64 {
        let m = self.machine;
        if m.rotary_enabled && m.rotary_diameter > 0.0 {
            y * m.rotary_mm_per_rev / (std::f64::consts::PI * m.rotary_diameter)
        } else {
            y
        }
    }

    pub fn set_air(&mut self, mode: AirAssist) {
        if Some(mode) == self.current_air {
            return;
        }
        if mode == AirAssist::Off {
            self.add("M9 ; Air assist OFF");
        } else {
            let c = air_on_command(self.machine);
            self.add(format!("{c} ; Air assist ON"));
        }
        self.current_air = Some(mode);
    }

    pub fn rapid(&mut self, x: f64, y: f64) {
        let yy = self.y(y);
        self.add(format!("G0 X{} Y{}", f3(x), f3(yy)));
    }

    pub fn cut(&mut self, x: f64, y: f64, speed: f64, power: f64) {
        let (yy, s) = (self.y(y), self.s(power));
        self.add(format!("G1 X{} Y{} F{} S{}", f3(x), f3(yy), f0(speed), s));
    }

    fn dwell(&mut self, ms: f64) {
        if ms > 0.0 {
            self.add(format!("G4 P{}", f3(ms / 1000.0)));
        }
    }

    pub fn set_z(&mut self, z: f64) {
        let m = self.machine;
        if !m.z_enabled {
            self.add(format!("; Z {} skipped (Z axis not enabled)", f3(z)));
            return;
        }
        let feed = m.z_speed.max(1.0) * 60.0;
        if m.z_relative_only {
            let dz = z - self.z_now;
            self.z_now = z;
            if dz.abs() > 1e-6 {
                self.add(format!("G91 G1 Z{} F{} S0", f3(dz), f0(feed)));
                self.add("G90");
            }
        } else {
            self.add(format!("G1 Z{} F{} S0", f3(z), f0(feed)));
        }
    }

    fn power(&self, power: f64, layer: &LayerSettings) -> f64 {
        (power * layer.power_scale / 100.0).min(100.0)
    }

    fn in_bed(&self, x: f64, y: f64) -> (f64, f64) {
        let m = self.machine;
        let x = x.max(0.0).min(m.workspace_x);
        let y = if m.rotary_enabled { y } else { y.max(0.0).min(m.workspace_y) };
        (x, y)
    }

    /// Scanning offset: lag (mm) at this speed, linearly interpolated between "speed:offset" entries.
    pub fn scan_offset(&self, speed: f64) -> f64 {
        let m = self.machine;
        if !m.scan_offset_enabled || m.scan_offsets.trim().is_empty() {
            return 0.0;
        }
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for ln in m.scan_offsets.replace(';', "\n").lines() {
            let s = ln.replace(',', " ").replace(':', " ");
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(a), Ok(b)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                    pts.push((a, b));
                }
            }
        }
        if pts.is_empty() {
            return 0.0;
        }
        pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if speed <= pts[0].0 {
            return pts[0].1;
        }
        for w in pts.windows(2) {
            let ((s0, o0), (s1, o1)) = (w[0], w[1]);
            if speed <= s1 {
                let den = if s1 - s0 != 0.0 { s1 - s0 } else { 1.0 };
                return o0 + (o1 - o0) * (speed - s0) / den;
            }
        }
        pts[pts.len() - 1].1
    }

    fn overscan(&self, layer: &LayerSettings) -> f64 {
        if layer.overscan_percent {
            layer.speed / 60.0 * layer.overscan / 100.0
        } else {
            layer.overscan
        }
    }

    // ---- header ----
    fn header(&mut self, layer: &LayerSettings) {
        self.add(format!("; Layer: {}", layer.name));
        self.add(format!("; Speed: {} mm/min | Power: {}-{}%", py_float(layer.speed), py_float(layer.power_min), py_float(layer.power_max)));
        self.add(format!("; Air Assist: {} | Passes: {}", layer.air_assist.as_str(), layer.passes));
        self.add(format!("; Mode: {} | Laser: {}", layer.cut_mode.as_str(), layer.laser_mode.as_str()));
        if layer.cut_mode != CutMode::Line {
            self.add(format!("; Interval: {}mm | Angle: {} | Overscan: {}mm", py_float(layer.line_interval), py_float(layer.scan_angle), py_float(layer.overscan)));
        }
        if layer.kerf_offset != 0.0 {
            self.add(format!("; Kerf offset: {}mm", py_float(layer.kerf_offset)));
        }
        if layer.tab_count != 0 {
            self.add(format!("; Tabs: {} x {}mm", layer.tab_count, py_float(layer.tab_width)));
        }
        if self.machine.rotary_enabled {
            self.add(format!("; Rotary: {}mm/rev, object diameter {}mm", py_float(self.machine.rotary_mm_per_rev), py_float(self.machine.rotary_diameter)));
        }
        self.add("G21 ; mm");
        self.add("G90 ; absolute");
        self.add(format!("{} S0", layer.laser_mode.as_str()));
    }

    // ---- fill ----
    fn emit_fill(&mut self, layer: &LayerSettings, paths: &[GPath], angle: f64) {
        let closed: Vec<Vec<Pt>> = paths.iter().filter(|p| is_closed(&p.pts)).map(|p| p.pts.clone()).collect();
        if layer.flood_fill && closed.len() > 1 {
            // flood fill: each separate area on its own, nearest first, so the head never scans the gaps
            let mut parts = crate::geometry::flood_areas(&closed);
            if parts.len() > 1 {
                let mut cur = (0.0, 0.0);
                self.add(format!("; Flood fill: {} areas", parts.len()));
                while !parts.is_empty() {
                    let mut k = 0;
                    let mut best = f64::MAX;
                    for (i, (r, _)) in parts.iter().enumerate() {
                        let d = crate::geometry::distance_to(r, cur.0, cur.1);
                        if d < best {
                            best = d;
                            k = i;
                        }
                    }
                    let (rings, b) = parts.remove(k);
                    self.emit_fill_rows(layer, &rings, angle);
                    cur = (b[2], b[3]);
                }
                return;
            }
        }
        self.emit_fill_rows(layer, &closed, angle);
    }

    fn emit_fill_rows(&mut self, layer: &LayerSettings, closed: &[Vec<Pt>], angle: f64) {
        let fr = fill_rows(closed, layer.line_interval.max(0.01), angle);
        if fr.rows.is_empty() {
            return;
        }
        let power = self.power(layer.power_max, layer);
        let over = self.overscan(layer);
        let lag = self.scan_offset(layer.speed);
        self.add(format!("; Fill {} lines @ {} deg", fr.rows.len(), f0(angle)));
        for (n, (y, segs)) in fr.rows.iter().enumerate() {
            let forward = !layer.bidir_scan || n % 2 == 0;
            let mut segs: Vec<(f64, f64)> = if forward { segs.clone() } else { segs.iter().rev().map(|&(a, b)| (b, a)).collect() };
            let sign = if forward { 1.0 } else { -1.0 };
            if lag != 0.0 {
                segs = segs.iter().map(|&(a, b)| (a - sign * lag, b - sign * lag)).collect();
            }
            let (ux, uy) = fr.unrotate(segs[0].0 - sign * over, *y);
            let (sx, sy) = self.in_bed(ux, uy);
            self.rapid(sx, sy);
            for &(a, b) in &segs {
                let (ax, ay) = fr.unrotate(a, *y);
                let (bx, by) = fr.unrotate(b, *y);
                self.cut(ax, ay, layer.speed, 0.0);
                self.cut(bx, by, layer.speed, power);
            }
            let (ux, uy) = fr.unrotate(segs[segs.len() - 1].1 + sign * over, *y);
            let (ex, ey) = self.in_bed(ux, uy);
            self.cut(ex, ey, layer.speed, 0.0);
        }
    }

    fn emit_offset_fill(&mut self, layer: &LayerSettings, paths: &[GPath]) {
        let closed: Vec<Vec<Pt>> = paths.iter().filter(|p| is_closed(&p.pts)).map(|p| p.pts.clone()).collect();
        if closed.is_empty() {
            return;
        }
        let power = self.power(layer.power_max, layer);
        let rings = crate::geometry::offset_fill_rings(&closed, layer.line_interval.max(0.02));
        for r in &rings {
            if r.is_empty() {
                continue;
            }
            self.travel(layer, r[0].x, r[0].y);
            for p in &r[1..] {
                self.cut(p.x, p.y, layer.speed, power);
            }
        }
        self.add(format!("; Offset fill: {} rings", rings.len()));
    }

    // ---- line ----
    fn tab_gaps(&self, path: &GPath, layer: &LayerSettings) -> Vec<(f64, f64)> {
        let length = |p: &[Pt]| p.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum::<f64>();
        if let Some(manual) = &path.tabs {
            if !manual.is_empty() {
                let total = length(&path.pts);
                let half = layer.tab_width / 2.0;
                let mut m = manual.clone();
                m.sort_by(|a, b| a.partial_cmp(b).unwrap());
                return m.iter().map(|f| ((f * total - half).max(0.0), (f * total + half).min(total))).collect();
            }
        }
        if layer.tab_count <= 0 || !is_closed(&path.pts) {
            return Vec::new();
        }
        let total = length(&path.pts);
        let spacing = total / layer.tab_count as f64;
        let half = layer.tab_width / 2.0;
        (0..layer.tab_count).map(|i| (spacing * (i as f64 + 0.5) - half, spacing * (i as f64 + 0.5) + half)).collect()
    }

    fn air_cut_only(&self, layer: &LayerSettings) -> bool {
        layer.air_cut_only && layer.air_assist != AirAssist::Off
    }

    fn travel(&mut self, layer: &LayerSettings, x: f64, y: f64) {
        if self.air_cut_only(layer) {
            self.set_air(AirAssist::Off);
        }
        self.rapid(x, y);
        if self.air_cut_only(layer) {
            self.set_air(layer.air_assist);
        }
    }

    fn lead_len(&self, x: f64, y: f64, ux: f64, uy: f64, length: f64) -> f64 {
        let m = self.machine;
        let mut t = length;
        if ux > 1e-9 {
            t = t.min((m.workspace_x - x) / ux);
        } else if ux < -1e-9 {
            t = t.min((0.0 - x) / ux);
        }
        if !m.rotary_enabled {
            if uy > 1e-9 {
                t = t.min((m.workspace_y - y) / uy);
            } else if uy < -1e-9 {
                t = t.min((0.0 - y) / uy);
            }
        }
        t.max(0.0)
    }

    fn with_leads(&self, path: &[Pt], layer: &LayerSettings) -> Vec<Pt> {
        let mut pts: Vec<Pt> = path.to_vec();
        if is_closed(&pts) && layer.overcut > 0.0 {
            let (mut left, mut i) = (layer.overcut, 1usize);
            let mut extra = Vec::new();
            let n = pts.len();
            while left > 1e-6 && n > 2 {
                let a = pts[(i - 1) % (n - 1)];
                let b = pts[i % (n - 1)];
                let seg = (b.x - a.x).hypot(b.y - a.y);
                if seg >= left {
                    let t = if seg != 0.0 { left / seg } else { 1.0 };
                    extra.push(Pt { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
                    break;
                }
                extra.push(b);
                left -= seg;
                i += 1;
                if i > 10 * n {
                    break;
                }
            }
            pts.extend(extra);
        }
        let unit = |a: Pt, b: Pt| {
            let d = (b.x - a.x).hypot(b.y - a.y);
            let d = if d != 0.0 { d } else { 1.0 };
            ((b.x - a.x) / d, (b.y - a.y) / d)
        };
        if layer.lead_in > 0.0 && pts.len() > 1 {
            let (ux, uy) = unit(pts[0], pts[1]);
            let ln = self.lead_len(pts[0].x, pts[0].y, -ux, -uy, layer.lead_in);
            if ln > 1e-3 {
                let p0 = pts[0];
                pts.insert(0, Pt { x: p0.x - ux * ln, y: p0.y - uy * ln });
            }
        }
        if layer.lead_out > 0.0 && pts.len() > 1 {
            let k = pts.len();
            let (ux, uy) = unit(pts[k - 2], pts[k - 1]);
            let ln = self.lead_len(pts[k - 1].x, pts[k - 1].y, ux, uy, layer.lead_out);
            if ln > 1e-3 {
                let pl = pts[k - 1];
                pts.push(Pt { x: pl.x + ux * ln, y: pl.y + uy * ln });
            }
        }
        pts
    }

    fn emit_line_path(&mut self, layer: &LayerSettings, gpath: &GPath) {
        let power = self.power(layer.power_max, layer);
        let tab_gaps = self.tab_gaps(gpath, layer);
        let path: Vec<Pt> = if tab_gaps.is_empty() && !layer.dot_mode && (layer.lead_in > 0.0 || layer.lead_out > 0.0 || layer.overcut > 0.0) {
            self.with_leads(&gpath.pts, layer)
        } else {
            gpath.pts.clone()
        };
        self.travel(layer, path[0].x, path[0].y);
        self.dwell(layer.start_delay);
        if layer.dot_mode {
            for p in &path[1..] {
                self.rapid(p.x, p.y);
                let s = self.s(power);
                self.add(format!("G1 S{} F{}", s, f0(layer.speed)));
                self.dwell(layer.dot_time);
                self.add("S0");
            }
            return;
        }
        let mut gaps = tab_gaps;
        if layer.perforate && layer.perf_cut > 0.0 && layer.perf_skip > 0.0 {
            let total: f64 = path.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum();
            let period = layer.perf_cut + layer.perf_skip;
            let mut k = 0.0;
            while k * period < total {
                gaps.push((k * period + layer.perf_cut, ((k + 1.0) * period).min(total)));
                k += 1.0;
            }
        }
        let mut dist = 0.0;
        for i in 1..path.len() {
            let (a, b) = (path[i - 1], path[i]);
            let seg = (b.x - a.x).hypot(b.y - a.y);
            let mut cuts = vec![0.0, seg];
            for &(g0, g1) in &gaps {
                for g in [g0, g1] {
                    if dist < g && g < dist + seg {
                        cuts.push(g - dist);
                    }
                }
            }
            if layer.ramp_length > 0.0 && dist < layer.ramp_length {
                let reach = seg.min(layer.ramp_length - dist);
                let steps = ((reach / 0.5) as i64).max(1);
                for k in 1..=steps {
                    cuts.push(reach * k as f64 / steps as f64);
                }
            }
            let mut cuts: Vec<f64> = cuts.iter().map(|c| py_round(*c, 6)).collect();
            cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
            cuts.dedup();
            for w in cuts.windows(2) {
                let (c0, c1) = (w[0], w[1]);
                let mid = dist + (c0 + c1) / 2.0;
                let t = if seg != 0.0 { c1 / seg } else { 1.0 };
                let (x, y) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                let in_gap = gaps.iter().any(|&(g0, g1)| g0 <= mid && mid <= g1);
                let mut p = power;
                if layer.ramp_length > 0.0 && dist + c1 <= layer.ramp_length {
                    p = layer.power_min + (power - layer.power_min) * (dist + c1) / layer.ramp_length;
                }
                if in_gap {
                    self.rapid(x, y);
                } else {
                    self.cut(x, y, layer.speed, p);
                }
            }
            dist += seg;
            if layer.dwell_time > 0.0 && i < path.len() - 1 {
                self.dwell(layer.dwell_time);
            }
        }
        self.dwell(layer.end_delay);
    }

    // ---- layers ----
    pub fn generate_layer(&mut self, layer: &LayerSettings, paths: &[GPath]) -> String {
        self.lines.clear();
        self.header(layer);
        if !layer.enabled {
            self.add("; Layer disabled");
            return self.lines.join("\n");
        }
        let kerfed: Vec<GPath>;
        let line_paths: &[GPath] = if layer.kerf_offset != 0.0 && matches!(layer.cut_mode, CutMode::Line | CutMode::LineFill) {
            let raw: Vec<Vec<Pt>> = paths.iter().map(|p| p.pts.clone()).collect();
            kerfed = crate::geometry::offset_paths(&raw, layer.kerf_offset).into_iter().map(GPath::new).collect();
            &kerfed
        } else {
            paths
        };
        let air = if self.air_cut_only(layer) { AirAssist::Off } else { layer.air_assist };
        self.set_air(air);
        for n in 0..layer.passes {
            if layer.passes > 1 {
                self.add(format!("; Pass {}/{}", n + 1, layer.passes));
            }
            let z = layer.z_offset - layer.z_step * n as f64;
            if z != 0.0 {
                self.set_z(z);
            }
            if matches!(layer.cut_mode, CutMode::Fill | CutMode::LineFill) {
                self.emit_fill(layer, paths, layer.scan_angle);
                if layer.cross_hatch {
                    self.emit_fill(layer, paths, layer.scan_angle + 90.0);
                }
            }
            if layer.cut_mode == CutMode::OffsetFill {
                self.emit_offset_fill(layer, paths);
            }
            if matches!(layer.cut_mode, CutMode::Line | CutMode::LineFill) {
                for path in line_paths {
                    if path.pts.len() >= 2 {
                        self.emit_line_path(layer, path);
                    }
                }
            }
        }
        self.add("S0");
        if layer.z_offset != 0.0 || layer.z_step != 0.0 {
            self.set_z(0.0);
        }
        self.lines.join("\n")
    }

    /// Image engraving. Scan-angle rotation of the bitmap is phase 2 (images are scanned at 0°).
    pub fn generate_raster(&mut self, layer: &LayerSettings, r: &Raster) -> String {
        self.lines.clear();
        let (x0, y0, w, h, cols, rows) = (r.x, r.y, r.w, r.h, r.cols, r.rows);
        if cols < 1 || rows < 1 || r.data.len() < cols * rows {
            return "; empty image".into();
        }
        let (mut cols, mut rows) = (cols, rows);
        let (mut px_w, px_h) = (w / cols as f64, h / rows as f64);
        let angle = r.angle.unwrap_or(layer.scan_angle).rem_euclid(180.0);
        let (cx, cy) = (x0 + w / 2.0, y0 + h / 2.0);
        let mut rot: Option<(f64, f64)> = None;
        let rotated: Vec<u8>;
        let a: &[u8] = if angle.abs() > 1e-6 {
            // the bitmap is turned so the scan lines run at the angle, then mapped back onto the bed
            let mut d = r.data[..cols * rows].to_vec();
            if (px_w - px_h).abs() > 1e-3 * px_h {
                let nc = ((w / px_h).round_ties_even() as usize).max(1);
                d = crate::imaging::resize_nearest(&d, cols, rows, nc, rows);
                cols = nc;
            }
            let (dd, nc, nr) = crate::imaging::rotate_nearest(&d, cols, rows, angle);
            rotated = dd;
            cols = nc;
            rows = nr;
            px_w = px_h;
            let t = angle.to_radians();
            rot = Some((t.cos(), t.sin()));
            &rotated
        } else {
            &r.data
        };
        let (u0, v0) = match rot {
            Some(_) => (-(cols as f64) * px_w / 2.0, -(rows as f64) * px_h / 2.0),
            None => (x0, y0),
        };
        let place = move |u: f64, v: f64| match rot {
            Some((c, s)) => (cx + u * c - v * s, cy + u * s + v * c),
            None => (u, v),
        };
        let over = self.overscan(layer);
        let span = layer.power_max - layer.power_min;
        let lag = self.scan_offset(layer.speed);
        let dw = layer.dot_width.max(0.0);
        let m = self.machine;
        let ws_speed = if m.fast_whitespace { layer.speed.max(m.whitespace_speed) } else { layer.speed };
        self.add(format!("; Image {}x{}px, {}x{}mm, interval {}mm, angle {}", cols, rows, f1(w), f1(h), f3(px_h), f1(angle)));
        self.add("G21");
        self.add("G90");
        self.add("M4 S0 ; dynamic power for raster");
        self.set_air(layer.air_assist);

        struct St {
            x: Option<f64>,
            y: Option<f64>,
            f: Option<f64>,
            s: Option<i64>,
            merge: Option<usize>,
            prev: (Option<f64>, Option<f64>, Option<f64>, Option<i64>),
        }
        let mut st = St { x: None, y: None, f: None, s: None, merge: None, prev: (None, None, None, None) };

        let mut mv = |g: &mut Generator, u: f64, v: f64, s_val: i64, feed: f64, rapid: bool| {
            let (pu, pv) = place(u, v);
            let (x, y) = g.in_bed(pu, pv);
            let mut dx = st.x.map_or(true, |px| (x - px).abs() >= 0.0025);
            let mut dy = st.y.map_or(true, |py| (y - py).abs() >= 0.0025);
            if !dx && !dy {
                return;
            }
            if rapid {
                let yy = g.y(y);
                g.add(format!("G0 X{} Y{}", f3(x), f3(yy)));
                st.x = Some(x);
                st.y = Some(y);
                st.merge = None;
                return;
            }
            if s_val == 0 && st.s == Some(0) && Some(feed) == st.f && st.merge == Some(g.lines.len().wrapping_sub(1)) {
                g.lines.pop();
                st.x = st.prev.0;
                st.y = st.prev.1;
                st.f = st.prev.2;
                st.s = st.prev.3;
                dx = st.x.map_or(true, |px| (x - px).abs() >= 0.0025);
                dy = st.y.map_or(true, |py| (y - py).abs() >= 0.0025);
            }
            let prev = (st.x, st.y, st.f, st.s);
            use std::fmt::Write as _;
            let mut line = String::with_capacity(40);
            line.push_str("G1");
            if dx {
                let _ = write!(line, " X{:.3}", x);
            }
            if dy {
                let _ = write!(line, " Y{:.3}", g.y(y));
            }
            if Some(feed) != st.f {
                let _ = write!(line, " F{:.0}", feed);
                st.f = Some(feed);
            }
            let _ = write!(line, " S{}", s_val);
            st.s = Some(s_val);
            g.lines.push(line);
            st.x = Some(x);
            st.y = Some(y);
            st.prev = prev;
            st.merge = if s_val == 0 { Some(g.lines.len() - 1) } else { None };
        };

        let power_s = |g: &Generator, v: u8| -> i64 {
            if v == 0 {
                0
            } else {
                g.s(g.power(layer.power_min + span * v as f64 / 255.0, layer))
            }
        };

        let mut order: Vec<usize> = (0..rows).collect();
        if (m.origin == "fl" || m.origin == "fr") != layer.bottom_up {
            order.reverse();
        }
        for _ in 0..layer.passes {
            let mut k = 0usize;
            for &rr in &order {
                let row = &a[rr * cols..(rr + 1) * cols];
                let first = match row.iter().position(|&v| v != 0) {
                    Some(f) => f,
                    None => continue,
                };
                let last = row.iter().rposition(|&v| v != 0).unwrap();
                let seg = &row[first..=last];
                let mut bounds = vec![0usize];
                for i in 1..seg.len() {
                    if seg[i] != seg[i - 1] {
                        bounds.push(i);
                    }
                }
                bounds.push(seg.len());
                let mut runs: Vec<(usize, usize, u8)> =
                    (0..bounds.len() - 1).map(|i| (first + bounds[i], first + bounds[i + 1], seg[bounds[i]])).collect();
                let forward = !layer.bidir_scan || k % 2 == 0;
                k += 1;
                let d = if forward { 1.0 } else { -1.0 };
                let shift = -d * lag;
                let v = v0 + (rr as f64 + 0.5) * px_h;
                if !forward {
                    runs = runs.iter().rev().map(|&(s0, e, val)| (e, s0, val)).collect();
                }
                let ustart = u0 + runs[0].0 as f64 * px_w + shift;
                mv(self, ustart - d * over, v, 0, layer.speed, true);
                mv(self, ustart, v, 0, layer.speed, false);
                for &(s0, e, val) in &runs {
                    let (us, ue) = (u0 + s0 as f64 * px_w + shift, u0 + e as f64 * px_w + shift);
                    if val == 0 {
                        let f = if (ue - us).abs() > 1.0 { ws_speed } else { layer.speed };
                        mv(self, ue, v, 0, f, false);
                        continue;
                    }
                    let ps = power_s(self, val);
                    if dw > 0.0 {
                        let length = (ue - us).abs();
                        let cut = dw.min(length * 0.8) / 2.0;
                        mv(self, us + d * cut, v, 0, layer.speed, false);
                        mv(self, ue - d * cut, v, ps, layer.speed, false);
                        mv(self, ue, v, 0, layer.speed, false);
                    } else {
                        mv(self, ue, v, ps, layer.speed, false);
                    }
                }
                let uend = u0 + runs[runs.len() - 1].1 as f64 * px_w + shift;
                mv(self, uend + d * over, v, 0, layer.speed, false);
            }
        }
        if m.raster_relative {
            let lines = std::mem::take(&mut self.lines);
            self.lines = relativize(lines);
        }
        self.add("S0");
        self.lines.join("\n")
    }

    /// Frame around the job: preamble, trace from the corner nearest the head; relative (G91) with `relative_to`.
    pub fn generate_frame(&mut self, bounds: (f64, f64, f64, f64), laser_on: bool, power: Option<f64>, speed: f64,
                          relative_to: Option<(f64, f64)>, head: Option<(f64, f64)>) -> String {
        self.lines.clear();
        let (min_x, min_y, max_x, max_y) = bounds;
        let ring = [(min_x, min_y), (max_x, min_y), (max_x, max_y), (min_x, max_y)];
        let here = relative_to.or(head);
        let k = match here {
            Some((hx, hy)) => {
                let mut best = 0;
                for i in 1..4 {
                    if (ring[i].0 - hx).hypot(ring[i].1 - hy) < (ring[best].0 - hx).hypot(ring[best].1 - hy) {
                        best = i;
                    }
                }
                best
            }
            None => 0,
        };
        let mut trace: Vec<(f64, f64)> = ring[k..].iter().chain(ring[..k].iter()).cloned().collect();
        trace.push(ring[k]);
        let p = power.unwrap_or(self.machine.frame_power);
        let s_on = if laser_on { self.s(p) } else { 0 };
        let mut title = String::from("; Frame trace");
        if relative_to.is_some() {
            title.push_str(" from current position");
        }
        title.push_str(if laser_on { " (laser on)" } else { " (beam off)" });
        self.add(title);
        self.add(JOB_HEADER);
        self.add(if laser_on { "M4 S0" } else { "M5" });
        if let Some((rx, ry)) = relative_to {
            self.add("G91");
            let (mut cx, mut cy) = (rx, ry);
            let mut pts = trace.clone();
            pts.push((rx, ry));
            let n = pts.len();
            for (i, &(x, y)) in pts.iter().enumerate() {
                let (dx, dy) = (x - cx, y - cy);
                cx = x;
                cy = y;
                if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
                    continue;
                }
                let yy = self.y(dy);
                if i == 0 || i == n - 1 {
                    self.add(format!("G0 X{} Y{}", f3(dx), f3(yy)));
                } else {
                    self.add(format!("G1 X{} Y{} F{} S{}", f3(dx), f3(yy), f0(speed), s_on));
                }
            }
        } else {
            self.add("G90");
            self.rapid(trace[0].0, trace[0].1);
            for &(x, y) in &trace[1..] {
                let yy = self.y(y);
                self.add(format!("G1 X{} Y{} F{} S{}", f3(x), f3(yy), f0(speed), s_on));
            }
        }
        self.add("G0");
        self.add("G90");
        self.add("S0");
        self.add("M5");
        self.lines.join("\n")
    }

    pub fn generate_job(&mut self, layers: &[(LayerSettings, Vec<GPath>)], rasters: &[(LayerSettings, Raster)],
                        finish: Option<(f64, f64)>) -> String {
        let m = self.machine;
        let mut out: Vec<String> = vec![
            "; === LaserForge Job ===".into(),
            format!("; Layers: {}  Images: {}", layers.len(), rasters.len()),
            String::new(),
        ];
        if !m.start_gcode.trim().is_empty() {
            out.extend([";USER START SCRIPT".into(), m.start_gcode.trim().to_string(), ";USER START SCRIPT".into(), String::new()]);
        }
        out.push(JOB_HEADER.into());
        out.push("G90".into());
        self.current_air = None;
        for (layer, raster) in rasters {
            out.push(format!("; ======== Image on {} ========", layer.name));
            out.push(self.generate_raster(layer, raster));
            out.push(String::new());
        }
        for (layer, paths) in layers {
            out.push(format!("; ======== {} ========", layer.name));
            out.push(self.generate_layer(layer, paths));
            out.push(String::new());
        }
        out.extend(["; --- job end ---".into(), "M5 ; laser off".into(), "M9 ; air assist off".into()]);
        if m.return_to_origin {
            if let Some((fx, fy)) = finish {
                out.push(format!("G0 X{} Y{} ; return to start", f3(fx), f3(self.y(fy))));
            } else if m.finish_position == "custom" {
                out.push(format!("G0 X{} Y{} ; finish position", f3(m.finish_x), f3(m.finish_y)));
            } else {
                out.push("G0 X0 Y0 ; return to origin".into());
            }
        }
        if !m.end_gcode.trim().is_empty() {
            out.extend([";USER END SCRIPT".into(), m.end_gcode.trim().to_string(), ";USER END SCRIPT".into()]);
        }
        out.extend(["; === Job Complete ===".into(), "M2".into()]);
        self.current_air = None;
        out.join("\n")
    }
}
